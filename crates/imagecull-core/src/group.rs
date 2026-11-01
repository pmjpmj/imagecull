//! Which frames of a shoot are the same moment.
//!
//! A group is the set of frames taken within a moment of each other, by one
//! body, at one exposure, of one scene. The four gates run in that order and a
//! candidate is out as soon as one fails: capture time within
//! [`Thresholds::gap`] of the previous frame in the group, the same body serial,
//! the same exposure settings and orientation, and a preview signature within
//! [`Thresholds::similarity`] of the group's first frame.
//!
//! The similarity gate is what makes this safe. Without it, "a second apart"
//! merges a subject who stepped behind a tree, a hat that came off, a pan. With
//! it, a pan splits into the compositions it actually produced — the right
//! answer, even though it is not what a naive burst detector would say.
//!
//! Nothing here consults a model. Grouping and measures are a fact about the
//! shoot, computable with no server running, which is why they ship first as
//! [`run`] and `imagecull groups` rather than as columns on a scored report: a
//! number that silently decided which frames deserved a request would turn a
//! plausible wrong measure into missing scores.
//!
//! # What a group is not
//!
//! A group is not a duplicate. Dedupe is a cache over byte-identical previews;
//! a group is a fact about the shoot. Neither replaces the other: a group's
//! frames are different pictures of one moment, and a duplicate is usually one
//! of them. Grouping never drops a frame from the report — the plan has one row
//! per frame, always.
//!
//! # Determinism
//!
//! The plan is a pure function of the files' metadata, their preview pixels and
//! the two thresholds. [`grouping_hash`] covers the algorithm version, the
//! signature parameters and the thresholds, and nothing else — never a
//! filename, never the order the scan happened to list. Change a threshold and
//! every group rank computed under the old hash is invalid, exactly as changing
//! rubric wording invalidates scores.

use std::path::Path;
use std::time::{Duration, Instant};

use image::RgbImage;
use raw_files::{DateTime, Metadata};
use sha2::{Digest, Sha256};

use crate::error::Result;
use crate::ingest::{self, ScanCounts, ScanOptions, SourceFile};
use crate::measure::{self, AfPoint, Measures};
use crate::preprocess;
use crate::preview;
use crate::report::{self, Failure};
use crate::rubric::hex;

/// The identity of the algorithm, part of every [`grouping_hash`]. Bump it when
/// a gate, its order or the signature changes: two runs under different
/// versions did not group the same way.
pub const ALGORITHM: &str = "burst-grouping/1";

/// The side of a [`Signature`], in pixels.
pub const SIGNATURE_EDGE: u32 = 32;

/// The default gap: frames within two seconds of each other are one moment.
pub const DEFAULT_GAP: Duration = Duration::from_secs(2);

/// The default similarity: a cosine of 0.90 between two preview signatures.
pub const DEFAULT_SIMILARITY: f64 = 0.90;

/// How close two frames have to be to be the same moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thresholds {
    /// The most time between two frames of one group, measured from the
    /// previous frame in the group rather than from its first.
    pub gap: Duration,
    /// The least similarity between a frame's signature and the group's first
    /// frame's. A cosine, so `1.0` is the same scene and `0.0` is unrelated.
    pub similarity: f64,
}

impl Default for Thresholds {
    fn default() -> Thresholds {
        Thresholds {
            gap: DEFAULT_GAP,
            similarity: DEFAULT_SIMILARITY,
        }
    }
}

/// When a frame was taken, to subseconds, as the camera's clock recorded it.
///
/// Ordering is by the clock's own fields, which is the order a burst happened
/// in: one body's frames share a clock, and two bodies' clocks are separated by
/// the serial gate long before their times are compared. The zone offset is not
/// part of the order, because a burst cannot change zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Capture {
    /// The year the camera's clock showed.
    pub year: u16,
    /// The month, 1–12.
    pub month: u8,
    /// The day of the month.
    pub day: u8,
    /// The hour.
    pub hour: u8,
    /// The minute.
    pub minute: u8,
    /// The second.
    pub second: u8,
    /// The fraction of a second, in nanoseconds.
    pub nanosecond: u32,
}

impl Capture {
    /// The capture time a file records.
    pub fn of(time: DateTime) -> Capture {
        Capture {
            year: time.year(),
            month: time.month(),
            day: time.day(),
            hour: time.hour(),
            minute: time.minute(),
            second: time.second(),
            nanosecond: time.nanosecond(),
        }
    }

    /// The clock reading in nanoseconds since 1970-01-01: an integer, so that
    /// the gap between two readings is exact, however many seconds apart they
    /// are and however small the fraction between them.
    fn instant(&self) -> i128 {
        let days = i128::from(days_from_civil(i64::from(self.year), self.month, self.day));
        days * 86_400_000_000_000
            + i128::from(self.hour) * 3_600_000_000_000
            + i128::from(self.minute) * 60_000_000_000
            + i128::from(self.second) * 1_000_000_000
            + i128::from(self.nanosecond)
    }

    /// The seconds from `self` to `later`, exact to the nanosecond.
    fn seconds_after(&self, later: &Capture) -> f64 {
        (later.instant() - self.instant()) as f64 / 1e9
    }
}

/// Days from 1970-01-01 to `year - month - day`, proleptic Gregorian.
///
/// Howard Hinnant's `days_from_civil`: it is exact for every year a camera can
/// write, and it keeps the conversion out of a `chrono`-shaped dependency the
/// crate does not need for one subtraction.
fn days_from_civil(year: i64, month: u8, day: u8) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day = i64::from(day);
    let shifted = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The exposure settings two frames must share to be one moment.
///
/// Every field is optional because a file may record none of them; two frames
/// that both record nothing have not been shown to differ, so the gate passes.
/// That is the same treatment the serial number gets, and the opposite of what a
/// *measure* does with a missing value: a gate asks "may these be one moment",
/// a measure asks "what is true".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    /// The sensor sensitivity the camera set.
    pub iso: Option<u32>,
    /// The exposure time, as the numerator and denominator the file stored.
    pub exposure_time: Option<(u32, u32)>,
    /// The aperture, as a fraction.
    pub f_number: Option<(u32, u32)>,
    /// The focal length, as a fraction.
    pub focal_length: Option<(u32, u32)>,
    /// The EXIF orientation: a frame turned on its side is not the same
    /// composition as the one before it.
    pub orientation: u16,
}

impl Settings {
    /// The settings a file records.
    pub fn of(metadata: &Metadata) -> Settings {
        let exposure = metadata.exposure();
        Settings {
            iso: exposure.iso(),
            exposure_time: fraction(exposure.exposure_time()),
            f_number: fraction(exposure.f_number()),
            focal_length: fraction(exposure.focal_length()),
            orientation: metadata.orientation().to_exif(),
        }
    }

    /// Whether these settings are the same exposure. Fractions are compared by
    /// their values, so `1/500` and `2/1000` are one exposure as a photographer
    /// means it.
    pub fn same(&self, other: &Settings) -> bool {
        self.orientation == other.orientation
            && self.iso == other.iso
            && same_fraction(self.exposure_time, other.exposure_time)
            && same_fraction(self.f_number, other.f_number)
            && same_fraction(self.focal_length, other.focal_length)
    }
}

/// A `Rational` as its stored numerator and denominator.
fn fraction(value: Option<raw_files::Rational>) -> Option<(u32, u32)> {
    value.map(|value| (value.numerator, value.denominator))
}

/// Two fractions of unknown originals: equal when their values are, or when
/// neither was recorded. Cross-multiplied in `u64`, since `u32 * u32` overflows.
fn same_fraction(left: Option<(u32, u32)>, right: Option<(u32, u32)>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            u64::from(left.0) * u64::from(right.1) == u64::from(right.0) * u64::from(left.1)
        }
        _ => false,
    }
}

/// A frame's preview at 32×32 greyscale, mean subtracted and scaled to unit
/// length: the fact two frames are compared on to decide they show one scene.
///
/// Mean subtraction takes out the exposure and the overall brightness, so a
/// frame a stop darker is still the same scene; unit length makes the
/// comparison a dot product. A frame with no variation at all carries no scene
/// and has no direction to compare, which is why its similarity is zero rather
/// than one.
#[derive(Debug, Clone, PartialEq)]
pub struct Signature(Vec<f32>);

impl Signature {
    /// The signature of an upright, decoded preview.
    pub fn of_rgb(image: &RgbImage) -> Result<Signature> {
        let fitted = preprocess::fit(image, SIGNATURE_EDGE)?;
        let grey: Vec<f32> = fitted
            .pixels()
            .map(|pixel| {
                0.299 * f32::from(pixel.0[0])
                    + 0.587 * f32::from(pixel.0[1])
                    + 0.114 * f32::from(pixel.0[2])
            })
            .collect();
        if grey.is_empty() {
            return Ok(Signature(Vec::new()));
        }
        let mean = grey.iter().sum::<f32>() / grey.len() as f32;
        let mut values: Vec<f32> = grey.iter().map(|value| value - mean).collect();
        let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
        if norm > f32::EPSILON {
            for value in &mut values {
                *value /= norm;
            }
        }
        Ok(Signature(values))
    }

    /// Whether this signature has a direction at all: a frame with no variation
    /// normalises to nothing, and nothing is not a scene.
    fn carries_a_scene(&self) -> bool {
        self.0.iter().any(|value| *value != 0.0)
    }

    /// How alike two signatures are, as a cosine: `1.0` the same picture,
    /// `0.0` unrelated, and `0.0` for a frame with nothing in it.
    ///
    /// Two equal signatures are exactly `1.0` rather than `1.0` less a rounding
    /// error: an algorithm run twice on one file produces the same signature,
    /// and "the same scene" is not something a threshold should miss by a bit.
    /// Every other pair is the cosine the arithmetic gives, clamped.
    pub fn similarity(&self, other: &Signature) -> f64 {
        if !self.carries_a_scene() || !other.carries_a_scene() {
            return 0.0;
        }
        if self == other {
            return 1.0;
        }
        let dot: f32 = self
            .0
            .iter()
            .zip(&other.0)
            .map(|(left, right)| left * right)
            .sum();
        f64::from(dot).clamp(-1.0, 1.0)
    }
}

/// What grouping reads about one frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Facts {
    /// When it was taken; `None` when the file records no capture time, which
    /// makes the frame a group of its own.
    pub captured: Option<Capture>,
    /// The body that took it.
    pub serial: Option<String>,
    /// The exposure settings it was taken at.
    pub settings: Settings,
    /// What its preview looks like; `None` when the preview could not be read,
    /// which also makes the frame a group of its own.
    pub signature: Option<Signature>,
}

impl Facts {
    /// What a file records, with the signature of its preview when there is one.
    pub fn of(metadata: &Metadata, signature: Option<Signature>) -> Facts {
        Facts {
            captured: metadata.capture_time().map(Capture::of),
            serial: metadata.serial_number().map(str::to_string),
            settings: Settings::of(metadata),
            signature,
        }
    }
}

/// One group: the frames that are the same moment, as indices into the facts
/// they were assigned from, in capture order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grouping {
    /// The indices of this group's frames, in the order they were captured.
    pub members: Vec<usize>,
}

/// Assign every frame to a group.
///
/// Groups come back in capture order, and so do their members. A frame with no
/// capture time, or with no signature to compare, is a group of its own and
/// comes after the timed ones, in the order it was given: a missing fact must
/// never invent a decision.
pub fn assign(facts: &[Facts], thresholds: &Thresholds) -> Vec<Grouping> {
    let timed: Vec<usize> = (0..facts.len())
        .filter(|&index| facts[index].captured.is_some() && facts[index].signature.is_some())
        .collect();
    let mut ordered = timed.clone();
    ordered.sort_by_key(|&index| (facts[index].captured, index));

    let mut groups: Vec<Grouping> = Vec::new();
    for index in ordered {
        if let Some(group) = groups.last_mut()
            && same_moment(facts, group, index, thresholds)
        {
            group.members.push(index);
            continue;
        }
        groups.push(Grouping {
            members: vec![index],
        });
    }

    for index in 0..facts.len() {
        if !timed.contains(&index) {
            groups.push(Grouping {
                members: vec![index],
            });
        }
    }
    groups
}

/// Whether `candidate` belongs to `group`: all four gates, in order, against the
/// group's first frame — and the time gate against its last.
fn same_moment(
    facts: &[Facts],
    group: &Grouping,
    candidate: usize,
    thresholds: &Thresholds,
) -> bool {
    let first = &facts[group.members[0]];
    let last = facts[group.members[group.members.len() - 1]].captured;
    let next = facts[candidate].captured;
    let (Some(previous), Some(next)) = (last, next) else {
        return false;
    };
    if previous.seconds_after(&next) > thresholds.gap.as_secs_f64() {
        return false;
    }
    let candidate = &facts[candidate];
    if first.serial != candidate.serial || !first.settings.same(&candidate.settings) {
        return false;
    }
    let (Some(seen), Some(fresh)) = (&first.signature, &candidate.signature) else {
        return false;
    };
    seen.similarity(fresh) >= thresholds.similarity
}

/// The hash that makes a plan's groups comparable with another run's.
///
/// The algorithm's identity, the signature's parameters and the two thresholds:
/// nothing about the folder, and nothing about a filename. Two runs of the same
/// shoot under the same hash grouped the same way by construction; a threshold
/// change replaces the hash, so a stored rank can never be read as if it had
/// been computed under the new one.
pub fn grouping_hash(thresholds: &Thresholds) -> String {
    let mut hasher = Sha256::new();
    hasher.update(ALGORITHM.as_bytes());
    hasher.update(
        format!(
            "\nsignature=grey{SIGNATURE_EDGE}x{SIGNATURE_EDGE}-mean-subtracted-unit-length-lanczos3"
        )
        .as_bytes(),
    );
    hasher.update(format!("\ngap={:.6}", thresholds.gap.as_secs_f64()).as_bytes());
    hasher.update(format!("\nsimilarity={:.6}", thresholds.similarity).as_bytes());
    hex(hasher.finalize())
}

/// How to group one shoot folder.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Which files the scan accepts.
    pub scan: ScanOptions,
    /// How close two frames must be.
    pub thresholds: Thresholds,
}

/// One frame of the plan: where it is, which group it is in, and what the pixels
/// said about it.
#[derive(Debug, Clone, PartialEq)]
pub struct Planned {
    /// The file, as the scan found it.
    pub source: SourceFile,
    /// The group's number, from 1, in capture order.
    pub group: usize,
    /// The frame's place in its group, from 1, in capture order.
    pub group_index: usize,
    /// How many frames the group holds.
    pub group_size: usize,
    /// What its pixels said; all `None` when they could not be read.
    pub measures: Measures,
    /// Why there are no measures, when there are none.
    pub failure: Option<Failure>,
}

impl Planned {
    /// The `status` cell: `ok`, or the kind that cost the frame its measures.
    pub fn status(&self) -> &'static str {
        self.failure
            .as_ref()
            .map_or(report::OK, |failure| failure.kind.as_str())
    }
}

/// What one pass over a shoot folder found.
#[derive(Debug, Clone)]
pub struct Plan {
    /// One entry per accepted file, in capture order: groups in the order they
    /// happened, and each group's frames in the order they were taken.
    pub frames: Vec<Planned>,
    /// How many groups there are.
    pub groups: usize,
    /// The hash the groups were computed under.
    pub hash: String,
    /// What the scan left out, and why.
    pub counts: ScanCounts,
    /// How long the whole pass took: the measured cost, which is evidence, not
    /// an estimate.
    pub elapsed: Duration,
}

impl Plan {
    /// The per-frame failures, in plan order.
    pub fn failures(&self) -> impl Iterator<Item = &Planned> {
        self.frames.iter().filter(|frame| frame.failure.is_some())
    }

    /// The groups, in order, each with the frames it holds.
    pub fn grouped(&self) -> Vec<Row<'_>> {
        let mut rows = Vec::new();
        let mut start = 0;
        for (index, frame) in self.frames.iter().enumerate() {
            if frame.group != self.frames[start].group {
                rows.push(Row {
                    number: self.frames[start].group,
                    frames: &self.frames[start..index],
                });
                start = index;
            }
        }
        if let Some(first) = self.frames.get(start) {
            rows.push(Row {
                number: first.group,
                frames: &self.frames[start..],
            });
        }
        rows
    }
}

/// One group of a [`Plan`].
#[derive(Debug, Clone)]
pub struct Row<'a> {
    /// The group's number, from 1.
    pub number: usize,
    /// Its frames, in capture order.
    pub frames: &'a [Planned],
}

/// Group a shoot folder and measure every frame in it.
///
/// One pass, one decode per frame, no requests. A frame that cannot be read is a
/// row with empty measures and a status, and a group of its own: one odd file
/// never costs the photographer the other 399.
pub fn run(shoot: &Path, options: &Options) -> Result<Plan> {
    let started = Instant::now();
    let scan = ingest::scan(shoot, &options.scan)?;

    let mut readings: Vec<Reading> = Vec::with_capacity(scan.files.len());
    let mut facts: Vec<Facts> = Vec::with_capacity(scan.files.len());
    for source in &scan.files {
        let reading = read(source);
        facts.push(reading.facts.clone());
        readings.push(reading);
    }

    let groupings = assign(&facts, &options.thresholds);
    let mut frames = Vec::with_capacity(scan.files.len());
    for (number, grouping) in groupings.iter().enumerate() {
        for (place, &index) in grouping.members.iter().enumerate() {
            let reading = &readings[index];
            frames.push(Planned {
                source: scan.files[index].clone(),
                group: number + 1,
                group_index: place + 1,
                group_size: grouping.members.len(),
                measures: reading.measures,
                failure: reading.failure.clone(),
            });
        }
    }

    Ok(Plan {
        groups: groupings.len(),
        frames,
        hash: grouping_hash(&options.thresholds),
        counts: scan.counts,
        elapsed: started.elapsed(),
    })
}

/// What one file yielded.
struct Reading {
    facts: Facts,
    measures: Measures,
    failure: Option<Failure>,
}

/// Read one file: its largest preview, its metadata, and everything derived from
/// them. The preview is decoded once and used twice — a signature for grouping,
/// a working image for the measures — because the spec's cost budget assumes one
/// extraction and one decode per frame.
fn read(source: &SourceFile) -> Reading {
    let (embedded, metadata) = match preview::read(&source.path) {
        Ok(pair) => pair,
        Err(error) => return Reading::unreadable(&error),
    };
    let af = af_point(metadata.focus().as_ref());
    let previewed = preprocess::decode(&embedded).and_then(|image| {
        let rgb = image.to_rgb8();
        let working = measure::Working::from_rgb(&rgb)?;
        Ok((rgb, working))
    });
    let (rgb, working) = match previewed {
        Ok(pair) => pair,
        Err(error) => {
            return Reading {
                facts: Facts::of(&metadata, None),
                measures: Measures::default(),
                failure: Some(Failure::of(&error)),
            };
        }
    };
    Reading {
        facts: Facts::of(&metadata, Signature::of_rgb(&rgb).ok()),
        measures: measure::of_working(&working, af.as_ref(), embedded.orientation),
        failure: None,
    }
}

impl Reading {
    /// A file nothing could be read from: no facts, no measures, and the reason.
    fn unreadable(error: &crate::error::Error) -> Reading {
        Reading {
            facts: Facts::default(),
            measures: Measures::default(),
            failure: Some(Failure::of(error)),
        }
    }
}

/// Where the camera said it focused, in the sensor frame it recorded it in.
fn af_point(focus: Option<&raw_files::Focus>) -> Option<AfPoint> {
    let focus = focus?;
    let frame = focus.frame();
    Some(AfPoint {
        frame: (frame.width, frame.height),
        center: (focus.center().x, focus.center().y),
        size: focus.size().map(|size| (size.width, size.height)),
        in_focus: focus.in_focus(),
    })
}

/// The columns of the measures CSV, in the order a photographer reads them:
/// which frame and which group first, then the numbers, then what happened.
pub const COLUMNS: [&str; 14] = [
    "file",
    "group",
    "group_index",
    "group_size",
    "acutance",
    "shake",
    "roll_deg",
    "roll_strength",
    "clip_hi",
    "clip_lo",
    "af_in_focus",
    "af_off_subject",
    "status",
    "error",
];

/// The whole plan as the CSV `imagecull groups` writes: a header and one row per
/// frame, values at the precision they were measured at.
///
/// Pure, so the columns can be asserted on without a file; the command writes
/// exactly these bytes. A failed frame keeps its row and leaves its measure cells
/// empty: "no answer" and "a measure of zero" are different facts.
pub fn render_csv(plan: &Plan) -> String {
    let mut out = String::new();
    report::push_record(&mut out, &COLUMNS);
    for frame in &plan.frames {
        let measures = frame.measures;
        let cells = [
            frame.source.relative.to_string_lossy().into_owned(),
            frame.group.to_string(),
            frame.group_index.to_string(),
            frame.group_size.to_string(),
            number(measures.acutance),
            number(measures.shake),
            number(measures.roll_deg),
            number(measures.roll_strength),
            number(measures.clip_hi),
            number(measures.clip_lo),
            flag(measures.af_in_focus),
            flag(measures.af_off_subject),
            frame.status().to_string(),
            frame
                .failure
                .as_ref()
                .map_or_else(String::new, |failure| failure.message.clone()),
        ];
        report::push_record(&mut out, &cells);
    }
    out
}

/// A measure as the CSV spells it: the shortest text that round-trips, or
/// nothing at all when the measure could not be taken.
fn number(value: Option<f64>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

/// A three-state flag: `true`, `false`, or empty when the camera did not say.
fn flag(value: Option<bool>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture(second: u8, nanosecond: u32) -> Capture {
        Capture {
            year: 2026,
            month: 11,
            day: 2,
            hour: 10,
            minute: 0,
            second,
            nanosecond,
        }
    }

    /// A signature of one bright block in one place: two of these are either the
    /// same picture or not, without needing a decoder.
    fn signature(at: usize) -> Signature {
        let mut values = vec![0.0f32; 32 * 32];
        values[at] = 1.0;
        Signature(values)
    }

    fn facts(second: u8, serial: &str, at: usize) -> Facts {
        Facts {
            captured: Some(capture(second, 0)),
            serial: Some(serial.to_string()),
            settings: Settings {
                iso: Some(200),
                ..Settings::default()
            },
            signature: Some(signature(at)),
        }
    }

    fn sizes(groups: &[Grouping]) -> Vec<usize> {
        groups.iter().map(|group| group.members.len()).collect()
    }

    #[test]
    fn days_from_civil_counts_from_1970_and_handles_leap_years() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(1970, 1, 2), 1);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        // 2024 is a leap year: 29 February exists and 1 March follows it by one.
        assert_eq!(
            days_from_civil(2024, 3, 1) - days_from_civil(2024, 2, 29),
            1
        );
        assert_eq!(
            days_from_civil(2100, 3, 1) - days_from_civil(2100, 2, 28),
            1,
            "2100 is not a leap year"
        );
    }

    #[test]
    fn a_capture_reads_its_subseconds_as_part_of_the_clock_reading() {
        let early = capture(0, 100_000_000);
        let late = capture(0, 900_000_000);
        assert!(early < late);
        assert!((early.seconds_after(&late) - 0.8).abs() < 1e-9);
        // Days apart, and still exact: an f64 of seconds since the epoch would
        // lose the fraction at this distance.
        let next_day = Capture { day: 3, ..early };
        assert!((early.seconds_after(&next_day) - 86_400.0).abs() < 1e-9);
    }

    #[test]
    fn the_signature_ignores_brightness_and_compares_shape() {
        let image = image::ImageBuffer::from_fn(64, 64, |x, _| {
            image::Rgb([u8::try_from(x * 4).unwrap_or(255); 3])
        });
        let darker = image::ImageBuffer::from_fn(64, 64, |x, _| {
            image::Rgb([u8::try_from(x * 2).unwrap_or(255); 3])
        });
        let other = image::ImageBuffer::from_fn(64, 64, |_, y| {
            image::Rgb([u8::try_from(y * 4).unwrap_or(255); 3])
        });

        let left = Signature::of_rgb(&image).expect("a signature");
        let dim = Signature::of_rgb(&darker).expect("a signature");
        let turned = Signature::of_rgb(&other).expect("a signature");

        assert!((left.similarity(&left) - 1.0).abs() < 1e-6);
        assert!(
            left.similarity(&dim) > 0.99,
            "a slower ramp is the same shape: {}",
            left.similarity(&dim)
        );
        assert!(
            left.similarity(&turned) < 0.5,
            "a ramp across is not a ramp down: {}",
            left.similarity(&turned)
        );
    }

    #[test]
    fn a_signature_is_exactly_one_similar_to_itself() {
        let image = image::ImageBuffer::from_fn(48, 48, |x, y| {
            image::Rgb([
                u8::try_from((x * 5) % 256).unwrap_or(0),
                u8::try_from((y * 3) % 256).unwrap_or(0),
                90,
            ])
        });
        let signature = Signature::of_rgb(&image).expect("a signature");
        assert_eq!(signature.similarity(&signature), 1.0);
    }

    #[test]
    fn a_frame_with_no_variation_has_no_scene_to_compare() {
        let flat = image::ImageBuffer::from_pixel(32, 32, image::Rgb([120, 120, 120]));
        let flat = Signature::of_rgb(&flat).expect("a signature");
        assert_eq!(flat.similarity(&flat), 0.0);
    }

    #[test]
    fn each_gate_splits_on_its_own_boundary() {
        let thresholds = Thresholds::default();

        // The time gate: measured from the previous frame, so three frames a
        // second and a half apart are one group even though the first and last
        // are three seconds apart.
        let in_time = vec![
            Facts {
                captured: Some(capture(0, 0)),
                ..facts(0, "1", 0)
            },
            Facts {
                captured: Some(capture(1, 500_000_000)),
                ..facts(1, "1", 0)
            },
            Facts {
                captured: Some(capture(3, 0)),
                ..facts(3, "1", 0)
            },
        ];
        assert_eq!(sizes(&assign(&in_time, &thresholds)), vec![3]);

        let out_of_time = vec![
            facts(0, "1", 0),
            Facts {
                captured: Some(capture(2, 500_000_000)),
                ..facts(2, "1", 0)
            },
        ];
        assert_eq!(sizes(&assign(&out_of_time, &thresholds)), vec![1, 1]);

        // The body gate.
        let two_bodies = vec![facts(0, "1", 0), facts(1, "2", 0)];
        assert_eq!(sizes(&assign(&two_bodies, &thresholds)), vec![1, 1]);

        // The settings gate: the same second and the same body, a stop apart.
        let two_exposures = vec![
            facts(0, "1", 0),
            Facts {
                settings: Settings {
                    iso: Some(800),
                    ..Settings::default()
                },
                ..facts(1, "1", 0)
            },
        ];
        assert_eq!(sizes(&assign(&two_exposures, &thresholds)), vec![1, 1]);

        // And an orientation that changed mid-burst is not the same
        // composition.
        let turned = vec![
            facts(0, "1", 0),
            Facts {
                settings: Settings {
                    orientation: 6,
                    ..Settings::default()
                },
                ..facts(1, "1", 0)
            },
        ];
        assert_eq!(sizes(&assign(&turned, &thresholds)), vec![1, 1]);

        // The scene gate.
        let two_scenes = vec![facts(0, "1", 0), facts(1, "1", 500)];
        assert_eq!(sizes(&assign(&two_scenes, &thresholds)), vec![1, 1]);
    }

    #[test]
    fn an_unrecorded_fact_gates_nothing_and_never_invents_a_decision() {
        let thresholds = Thresholds::default();
        // Two files that record neither a serial nor settings: not shown to
        // differ, so they group.
        let unknown = vec![
            Facts {
                captured: Some(capture(0, 0)),
                signature: Some(signature(0)),
                ..Facts::default()
            },
            Facts {
                captured: Some(capture(1, 0)),
                signature: Some(signature(0)),
                ..Facts::default()
            },
        ];
        assert_eq!(sizes(&assign(&unknown, &thresholds)), vec![2]);

        // A file with no capture time is its own group, wherever it sorts.
        let undated = vec![
            facts(0, "1", 0),
            Facts {
                captured: None,
                ..facts(1, "1", 0)
            },
            facts(2, "1", 0),
        ];
        let groups = assign(&undated, &thresholds);
        assert_eq!(sizes(&groups), vec![2, 1]);
        assert_eq!(groups[1].members, vec![1], "the undated frame is alone");
    }

    #[test]
    fn the_groups_come_back_in_capture_order_whatever_order_they_were_given() {
        let thresholds = Thresholds::default();
        let shuffled = vec![facts(5, "1", 0), facts(0, "1", 0), facts(1, "1", 0)];
        let groups = assign(&shuffled, &thresholds);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].members, vec![1, 2], "the 0 s and 1 s frames");
        assert_eq!(groups[1].members, vec![0], "the 5 s frame");
    }

    #[test]
    fn a_fraction_is_compared_by_its_value_and_never_across_a_missing_one() {
        assert!(same_fraction(Some((1, 500)), Some((2, 1000))));
        assert!(!same_fraction(Some((1, 500)), Some((1, 250))));
        assert!(same_fraction(None, None));
        assert!(!same_fraction(Some((1, 500)), None));
        assert!(!same_fraction(None, Some((1, 500))));
    }

    #[test]
    fn the_hash_is_the_algorithm_and_the_thresholds_and_nothing_else() {
        let base = grouping_hash(&Thresholds::default());
        assert_eq!(base.len(), 64);
        assert_eq!(base, grouping_hash(&Thresholds::default()));
        assert_ne!(
            base,
            grouping_hash(&Thresholds {
                gap: Duration::from_secs_f64(2.001),
                ..Thresholds::default()
            })
        );
        assert_ne!(
            base,
            grouping_hash(&Thresholds {
                similarity: 0.900001,
                ..Thresholds::default()
            })
        );
    }
}
