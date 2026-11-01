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
//! answer, even though it is not what a naive "same second, same group" detector
//! would say.
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
//! every verdict computed under the old hash is invalid, exactly as changing
//! rubric wording invalidates scores.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use image::RgbImage;
use raw_files::{DateTime, Metadata};
use sha2::{Digest, Sha256};

use crate::csv;
use crate::error::{Error, Result};
use crate::ingest::{self, ScanCounts, ScanOptions, SourceFile};
use crate::measure::{self, AfPoint, Measures};
use crate::preprocess;
use crate::preview;
use crate::progress;
use crate::rubric::hex;
use crate::sidecar::{self, Failure};

/// The identity of the algorithm, part of every [`grouping_hash`]. Bump it when
/// a gate, its order or the signature changes: two runs under different versions
/// did not group the same way.
///
/// The word is *group*, not *burst*: this string is hashed into every plan, and
/// a persisted identifier outlives the prose around it.
pub const ALGORITHM: &str = "grouping/1";

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
/// Ordering is by the clock's own fields, which is the order a group happened
/// in: one body's frames share a clock, and two bodies' clocks are separated by
/// the serial gate long before their times are compared. The zone offset is not
/// part of the order, because a group cannot change zone.
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

impl fmt::Display for Capture {
    /// The clock reading as ISO 8601 without a zone: the camera's own local time,
    /// which is what the file recorded and all a group's start can honestly say.
    ///
    /// The fraction is written with the digits it needs — none when there is
    /// none, else three, six or nine — so two groups inside one second stay
    /// distinguishable in a sidecar read a year later.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )?;
        let digits = match self.nanosecond {
            0 => return Ok(()),
            nanos if nanos % 1_000_000 == 0 => 3,
            nanos if nanos % 1_000 == 0 => 6,
            _ => 9,
        };
        let nanos = format!("{:09}", self.nanosecond);
        write!(f, ".{}", &nanos[..digits])
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
/// Equality *is* the gate — a [`Fraction`] compares by its value, so two
/// settings are one exposure exactly when they are equal.
/// That is the same treatment the serial number gets, and the opposite of what a
/// *measure* does with a missing value: a gate asks "may these be one moment",
/// a measure asks "what is true".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    /// The sensor sensitivity the camera set.
    pub iso: Option<u32>,
    /// The exposure time.
    pub exposure_time: Option<Fraction>,
    /// The aperture.
    pub f_number: Option<Fraction>,
    /// The focal length.
    pub focal_length: Option<Fraction>,
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
            exposure_time: exposure.exposure_time().map(Fraction::of),
            f_number: exposure.f_number().map(Fraction::of),
            focal_length: exposure.focal_length().map(Fraction::of),
            orientation: metadata.orientation().to_exif(),
        }
    }
}

/// A fraction as the file stored it, compared by its **value**: `1/500` and
/// `2/1000` are one exposure as a photographer means it, though a camera may
/// write either.
///
/// Equality is the number, so [`Settings`] needs no comparison method of its
/// own — two settings are the same exposure exactly when they are equal.
#[derive(Debug, Clone, Copy)]
pub struct Fraction {
    /// The number above the line, as stored.
    pub numerator: u32,
    /// The number below the line, as stored.
    pub denominator: u32,
}

impl Fraction {
    /// The fraction a file recorded.
    pub fn of(value: raw_files::Rational) -> Fraction {
        Fraction {
            numerator: value.numerator,
            denominator: value.denominator,
        }
    }
}

impl PartialEq for Fraction {
    fn eq(&self, other: &Fraction) -> bool {
        // Cross-multiplied in `u64`: `u32 * u32` overflows.
        u64::from(self.numerator) * u64::from(other.denominator)
            == u64::from(other.numerator) * u64::from(self.denominator)
    }
}

impl Eq for Fraction {}

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
    ///
    /// Exactly `SIGNATURE_EDGE` by `SIGNATURE_EDGE`, whatever shape the preview
    /// is: the frame is resampled onto a square grid, so a 3:2 frame and a 16:9
    /// frame are compared value for value rather than one being truncated
    /// against the other.
    pub fn of_rgb(image: &RgbImage) -> Result<Signature> {
        let square = preprocess::resize_to(image, SIGNATURE_EDGE, SIGNATURE_EDGE)?;
        let grey: Vec<f32> = square.pixels().map(preprocess::luma).collect();
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
///
/// Two frames whose recorded times are equal have no *fact* to order them by, so
/// they keep the order they were given — the scan's order, which is the folder's
/// names. Which group they land in does not depend on that: equal times are
/// inside any gap, and the serial, settings and scene gates are blind to names.
pub fn assign(facts: &[Facts], thresholds: &Thresholds) -> Vec<Grouping> {
    // A frame is timed when it can be placed in a moment at all: no capture
    // time or no signature and there is nothing to compare, so it is its own
    // group. The mask is read once per frame in the loop below, which is what
    // keeps the undated tail linear on a shoot of thousands.
    let timed: Vec<bool> = facts
        .iter()
        .map(|frame| frame.captured.is_some() && frame.signature.is_some())
        .collect();
    let mut ordered: Vec<usize> = (0..facts.len()).filter(|&index| timed[index]).collect();
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

    for (index, &timed) in timed.iter().enumerate() {
        if !timed {
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
    if first.serial != candidate.serial || first.settings != candidate.settings {
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
    /// How many frames to read at once: `None` is one per core the machine has.
    ///
    /// A scheduling decision, not a semantic one. A reading is a pure function
    /// of one path, so the degree changes how long the pass takes and nothing it
    /// says — the same frames under the same thresholds give the same plan, the
    /// same CSV and the same [`grouping_hash`] at any degree.
    pub jobs: Option<usize>,
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
    /// When the camera's clock says it was taken: the group's start when this
    /// frame opens one, and what a verdict records.
    pub captured: Option<Capture>,
    /// What its pixels said; all `None` when they could not be read.
    pub measures: Measures,
    /// Why there are no measures, when there are none.
    pub failure: Option<Failure>,
}

impl Planned {
    /// The `status` cell: `ok`, or the kind that cost the frame its measures.
    pub fn status(&self) -> &'static str {
        status_of(self.failure.as_ref())
    }
}

/// The `status` cell a failure produces: `ok` when there is none. One function
/// because the progress line and the CSV row must never spell a frame's outcome
/// two ways.
fn status_of(failure: Option<&Failure>) -> &'static str {
    failure.map_or(sidecar::OK, |failure| failure.kind.as_str())
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
    ///
    /// Wall clock, so with more than one job it is the shoot's price rather
    /// than one frame's, and [`Plan::jobs`] is what makes the per-frame figure
    /// readable as the price it is.
    pub elapsed: Duration,
    /// How many frames were read at once.
    pub jobs: usize,
}

impl Plan {
    /// The per-frame failures, in plan order.
    pub fn failures(&self) -> impl Iterator<Item = &Planned> {
        self.frames.iter().filter(|frame| frame.failure.is_some())
    }

    /// The groups, in order, each with the frames it holds.
    pub fn grouped(&self) -> Vec<GroupRow<'_>> {
        let mut rows = Vec::new();
        let mut start = 0;
        for (index, frame) in self.frames.iter().enumerate() {
            if frame.group != self.frames[start].group {
                rows.push(GroupRow {
                    number: self.frames[start].group,
                    frames: &self.frames[start..index],
                });
                start = index;
            }
        }
        if let Some(first) = self.frames.get(start) {
            rows.push(GroupRow {
                number: first.group,
                frames: &self.frames[start..],
            });
        }
        rows
    }
}

/// One group of a [`Plan`].
#[derive(Debug, Clone)]
pub struct GroupRow<'a> {
    /// The group's number, from 1.
    pub number: usize,
    /// Its frames, in capture order.
    pub frames: &'a [Planned],
}

/// What a pass says while it runs, as plain data.
///
/// The same seam [`progress`] describes for `score`, for the pass that asks no
/// model: the core decides when an event happens and in what order, the front
/// end decides what, if anything, to draw. A terminal writes a line per frame, a
/// window will draw a bar, a test collects them — and the core learns nothing
/// about any of them.
pub trait Watcher {
    /// One event from the pass.
    fn event(&self, event: Event);
}

/// One thing a pass tells the front end.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The folder is scanned and the pass knows how much work there is, before
    /// the first frame is read.
    Started(Started),
    /// One frame is read, whichever way it went.
    Frame(FrameEvent),
}

/// The pass beginning: how many frames it faces, and how many at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Started {
    /// How many files the scan accepted: the denominator of every index.
    pub total: usize,
    /// How many frames are read at once.
    pub jobs: usize,
}

/// One frame read: the unit the progress line shows.
///
/// It comes in the pass's own order — the scan's, which is the folder's names —
/// rather than the plan's capture order, because the groups are not known until
/// every frame has been read and being told about a frame after the run is over
/// would not be progress. The CSV is where capture order lives.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameEvent {
    /// The frame's position in the pass, counting from 1, so a line reads
    /// `12/400`.
    pub index: usize,
    /// How many frames the pass faces: the total [`Started`] announced.
    pub total: usize,
    /// The name the CSV will use for this frame.
    pub filename: String,
    /// The `status` cell its row will carry: `ok`, or the kind that cost it its
    /// measures.
    pub status: &'static str,
    /// Why there are no measures, when there are none.
    pub failure: Option<Failure>,
}

impl FrameEvent {
    /// The event one finished reading is: the same status and the same failure
    /// the row carries, so a line and a CSV cell cannot disagree.
    fn of(index: usize, total: usize, source: &SourceFile, reading: &Reading) -> FrameEvent {
        FrameEvent {
            index: index + 1,
            total,
            filename: source.relative.to_string_lossy().into_owned(),
            status: status_of(reading.failure.as_ref()),
            failure: reading.failure.clone(),
        }
    }
}

impl Watcher for progress::Silent {
    fn event(&self, _event: Event) {}
}

/// Group a shoot folder and measure every frame in it.
///
/// One pass, one decode per frame, no requests. A frame that cannot be read is a
/// row with empty measures and a status, and a group of its own: one odd file
/// never costs the photographer the other 399.
///
/// The frames are independent of each other — a reading is a pure function of
/// one path, and grouping is a pass over what the readings said — so they are
/// read [`Options::jobs`] at a time, and the plan is assembled in the scan's
/// order whatever order they finished in. Everything the pass costs is local
/// CPU, which is why this command is worth threading where `score` is not: no
/// server is serialising anything here.
///
/// What the pass says while it runs goes to a [`Watcher`]; this is the entry
/// point for a caller that has nothing to say about it.
pub fn run(shoot: &Path, options: &Options) -> Result<Plan> {
    run_with(shoot, options, &progress::Silent)
}

/// [`run`], telling `watcher` about each frame as it is read.
///
/// The events arrive in the pass's order however many jobs the pass uses: a
/// watcher sees frame 3 before frame 4 even when 4 finished first, so what a
/// photographer reads is the same text at any degree, and a redirected log is
/// the same log a serial run would have written.
pub fn run_with(shoot: &Path, options: &Options, watcher: &dyn Watcher) -> Result<Plan> {
    check(&options.thresholds)?;
    let started = Instant::now();
    let scan = ingest::scan(shoot, &options.scan)?;

    let jobs = degree(options.jobs, scan.files.len());
    watcher.event(Event::Started(Started {
        total: scan.files.len(),
        jobs,
    }));
    let readings = read_all(&scan.files, jobs, watcher);
    let facts: Vec<Facts> = readings
        .iter()
        .map(|reading| reading.facts.clone())
        .collect();

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
                captured: reading.facts.captured,
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
        jobs,
    })
}

/// Refuse a similarity that is not a cosine before a single file is read: a
/// front end may take the number from a flag, a file or a GUI, and the range is
/// the algorithm's, not one front end's.
fn check(thresholds: &Thresholds) -> Result<()> {
    if !(0.0..=1.0).contains(&thresholds.similarity) {
        return Err(Error::fatal(format!(
            "a group similarity of {} is not a cosine; it has to lie between 0 and 1",
            thresholds.similarity
        )));
    }
    if thresholds.gap.is_zero() {
        return Err(Error::fatal(
            "a group gap of zero seconds would make every frame its own moment",
        ));
    }
    Ok(())
}

/// How many frames to read at once: what was asked for, or one per core, and
/// never more workers than there are frames to read.
fn degree(asked: Option<usize>, frames: usize) -> usize {
    asked
        .unwrap_or_else(|| thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get))
        .clamp(1, frames.max(1))
}

/// Read every frame, `jobs` of them at a time, and return them in the scan's
/// order, telling `watcher` about each one as it arrives.
///
/// A worker claims the next index from a cursor and sends its reading back; the
/// pass's own thread buffers whatever arrived out of order and says each frame
/// exactly once, in index order. So the plan, the CSV, the hash and the log are
/// the same at any degree. Below two jobs this is a plain loop: a shoot of one
/// file has nothing to overlap, and keeping the serial path exact is what lets a
/// slow run be reproduced.
fn read_all(sources: &[SourceFile], jobs: usize, watcher: &dyn Watcher) -> Vec<Reading> {
    let total = sources.len();
    let mut readings: Vec<Reading> = Vec::with_capacity(total);
    let tell = |readings: &mut Vec<Reading>, index: usize, reading: Reading| {
        watcher.event(Event::Frame(FrameEvent::of(
            index,
            total,
            &sources[index],
            &reading,
        )));
        readings.push(reading);
    };
    if jobs < 2 || total < 2 {
        for (index, source) in sources.iter().enumerate() {
            tell(&mut readings, index, read(source));
        }
        return readings;
    }

    let cursor = AtomicUsize::new(0);
    let (sender, receiver) = mpsc::channel::<(usize, Reading)>();
    let cursor = &cursor;
    thread::scope(|scope| {
        for _ in 0..jobs {
            let sender = sender.clone();
            scope.spawn(move || {
                while let Some(index) = next(cursor, total) {
                    let reading = read(&sources[index]);
                    // The receiver lives until every sender is gone, which is
                    // the end of this scope: a send cannot be refused here.
                    sender
                        .send((index, reading))
                        .expect("the pass is still collecting readings");
                }
            });
        }
        // Drop the pass's own handle, or the channel would never close.
        drop(sender);
        let mut pending: BTreeMap<usize, Reading> = BTreeMap::new();
        let mut due = 0;
        for (index, reading) in receiver {
            pending.insert(index, reading);
            while let Some(reading) = pending.remove(&due) {
                tell(&mut readings, due, reading);
                due += 1;
            }
        }
    });
    readings
}

/// The next unclaimed frame index, or `None` when the shoot has been handed out.
fn next(cursor: &AtomicUsize, frames: usize) -> Option<usize> {
    let index = cursor.fetch_add(1, Ordering::Relaxed);
    (index < frames).then_some(index)
}

/// What one file yielded: the facts the grouping gates read, the measures the
/// shortlist can order two equal ratings by, and why there are none when there
/// are none.
///
/// The same reading serves [`run`], which builds a whole plan from it, and
/// [`scoring`](crate::scoring), which groups the frames it has just rated and
/// needs one frame's facts at a time.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    /// What the grouping gates compare.
    pub facts: Facts,
    /// What the frame's own pixels say.
    pub measures: Measures,
    /// Why there are no measures, when there are none.
    pub failure: Option<Failure>,
}

/// Read one file: its largest preview, its metadata, and everything derived from
/// them. The preview is decoded once and used twice — a signature for grouping,
/// a working image for the measures — because the spec's cost budget assumes one
/// extraction and one decode per frame.
pub fn read(source: &SourceFile) -> Reading {
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
    fn unreadable(error: &Error) -> Reading {
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
    csv::push_record(&mut out, &COLUMNS);
    for frame in &plan.frames {
        let measures = frame.measures;
        let cells = [
            frame.source.relative.to_string_lossy().into_owned(),
            frame.group.to_string(),
            frame.group_index.to_string(),
            frame.group_size.to_string(),
            measure_cell(measures.acutance),
            measure_cell(measures.shake),
            measure_cell(measures.roll_deg),
            measure_cell(measures.roll_strength),
            measure_cell(measures.clip_hi),
            measure_cell(measures.clip_lo),
            flag(measures.af_in_focus),
            flag(measures.af_off_subject),
            frame.status().to_string(),
            frame
                .failure
                .as_ref()
                .map_or_else(String::new, |failure| failure.message.clone()),
        ];
        csv::push_record(&mut out, &cells);
    }
    out
}

/// A measure as the CSV spells it: the shortest text that round-trips, or
/// nothing at all when the measure could not be taken.
fn measure_cell(value: Option<f64>) -> String {
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
    fn a_signature_is_a_square_grid_whatever_shape_the_frame_is() {
        // The signature is compared value for value, so every frame has to give
        // the same number of them: a 3:2 frame and a 16:9 frame must not be
        // truncated against each other.
        for (width, height) in [(120, 80), (160, 90), (80, 120), (32, 32)] {
            let image = image::ImageBuffer::from_fn(width, height, |x, y| {
                image::Rgb([
                    u8::try_from((x * 7) % 256).unwrap_or(0),
                    u8::try_from((y * 5) % 256).unwrap_or(0),
                    40,
                ])
            });
            let signature = Signature::of_rgb(&image).expect("a signature");
            assert_eq!(
                signature.0.len(),
                (SIGNATURE_EDGE * SIGNATURE_EDGE) as usize,
                "{width}×{height} did not give a square signature"
            );
        }
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

        // And an orientation that changed mid-group is not the same
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
        let half = |numerator, denominator| {
            Some(Fraction {
                numerator,
                denominator,
            })
        };
        assert_eq!(half(1, 500), half(2, 1000));
        assert_ne!(half(1, 500), half(1, 250));
        assert_eq!(None::<Fraction>, None);
        assert_ne!(half(1, 500), None);

        // And the settings they belong to compare the same way, which is what
        // the gate reads.
        let fast = Settings {
            exposure_time: half(1, 500),
            ..Settings::default()
        };
        let same = Settings {
            exposure_time: half(2, 1000),
            ..Settings::default()
        };
        assert_eq!(fast, same);
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
