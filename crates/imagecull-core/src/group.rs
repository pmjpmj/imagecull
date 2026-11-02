//! Which frames of a shoot are the same moment.
//!
//! A group is the set of frames taken within a moment of each other, by one
//! body, at one exposure, of one scene. The four gates run in that order and a
//! candidate is out as soon as one fails: capture time within
//! [`Thresholds::gap`] of the previous frame in the group, the same body serial,
//! the same optics and frame at an exposure within
//! [`Thresholds::exposure_stops`] of the group's first, and a preview signature
//! within [`Thresholds::similarity`] of it.
//!
//! The settings gate tolerates a drifted exposure on purpose. A burst whose auto
//! ISO walked two stops, or that was bracketed, is one moment: the frames are
//! the same picture brighter or darker, which the signature is built to survive,
//! and refusing to group them costs the photographer the comparison that would
//! have answered them (ADR 0009). What is *not* tolerated is the picture moving:
//! orientation and focal length are asked for exactly.
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
//! the thresholds. [`grouping_hash`] covers the algorithm version, the
//! signature parameters and the thresholds, and nothing else — never a
//! filename, never the order the scan happened to list. Change a threshold and
//! every verdict computed under the old hash is invalid, exactly as changing
//! rubric wording invalidates scores.

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
pub const ALGORITHM: &str = "grouping/2";

/// The side of a [`Signature`], in pixels.
pub const SIGNATURE_EDGE: u32 = 32;

/// The default gap: frames within two seconds of each other are one moment.
pub const DEFAULT_GAP: Duration = Duration::from_secs(2);

/// The default similarity: a cosine of 0.90 between two preview signatures.
pub const DEFAULT_SIMILARITY: f64 = 0.90;

/// The default exposure tolerance: two stops of brightness drift between two
/// frames of one group.
///
/// Two stops covers the auto-ISO walk and the one-stop-per-frame bracket a
/// photographer actually shoots, and stops well short of a scene lit differently
/// (a flash going off, a light switched on) counting as the same exposure. Zero
/// asks for one exposure exactly.
pub const DEFAULT_EXPOSURE_STOPS: f64 = 2.0;

/// What two brightnesses may differ by and still be one exposure: a sliver of a
/// stop, so that `--group-exposure-stops 0` means "the same exposure" rather
/// than "bit-identical logarithms".
///
/// `log2` of two equal products can differ in the last place, and a tolerance is
/// a photographer's number — two stops, not two stops and four ulps.
const BRIGHTNESS_EPSILON: f64 = 1e-9;

/// How close two frames have to be to be the same moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thresholds {
    /// The most time between two frames of one group, measured from the
    /// previous frame in the group rather than from its first.
    pub gap: Duration,
    /// The least similarity between a frame's signature and the group's first
    /// frame's. A cosine, so `1.0` is the same scene and `0.0` is unrelated.
    pub similarity: f64,
    /// The most brightness drift between two frames of one group, in stops.
    ///
    /// A tolerance rather than an equality: see [`Settings::within`]. `0.0` asks
    /// for one exposure exactly, and a very large value stops the gate having an
    /// opinion about exposure at all.
    pub exposure_stops: f64,
}

impl Default for Thresholds {
    fn default() -> Thresholds {
        Thresholds {
            gap: DEFAULT_GAP,
            similarity: DEFAULT_SIMILARITY,
            exposure_stops: DEFAULT_EXPOSURE_STOPS,
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

/// What two frames must share to be one moment: their optics, their frame and
/// their exposure.
///
/// Every field is optional because a file may record none of them; two frames
/// that both record nothing have not been shown to differ, so the gate passes.
/// That is the same treatment the serial number gets, and the opposite of what a
/// *measure* does with a missing value: a gate asks "may these be one moment",
/// a measure asks "what is true".
///
/// The two halves of the gate answer different questions, which is why equality
/// is not it. [`Settings::within`] is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    /// The sensor sensitivity the camera set.
    pub iso: Option<u32>,
    /// The exposure time.
    pub exposure_time: Option<Fraction>,
    /// The aperture.
    pub f_number: Option<Fraction>,
    /// The focal length: a zoom is a different picture, so this is asked for
    /// exactly rather than tolerated.
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

    /// Whether these settings are the same moment's as `other` — the settings
    /// gate.
    ///
    /// **The picture is asked for exactly; the exposure is tolerated within
    /// `stops`.** A burst whose auto ISO walked a stop, or that was bracketed,
    /// is still one moment, and the photographer still wants the comparison that
    /// picks between its frames; a zoom, a lens change or a camera turned on its
    /// side during the burst is not one moment, and no amount of tolerance makes
    /// it one (ADR 0009).
    ///
    /// Equality is deliberately *not* this question: it is the gate that ADR 0009
    /// replaced, and reading `==` here again would silently reintroduce it.
    pub fn within(self, other: Settings, stops: f64) -> bool {
        if self.orientation != other.orientation || self.focal_length != other.focal_length {
            return false;
        }
        match (self.brightness(), other.brightness()) {
            (Some(left), Some(right)) => (left - right).abs() <= stops + BRIGHTNESS_EPSILON,
            // One side does not record what it would take to tell them apart.
            _ => true,
        }
    }

    /// The brightness these settings produce, in stops above ISO 100 at f/1 for
    /// one second: `log2(t · ISO / 100 / N²)`.
    ///
    /// Two frames with the same number are one exposure as a photographer means
    /// it, however the camera arrived at it — ISO 400 at 1/1000 and ISO 200 at
    /// 1/500 are one exposure, and the difference between two of these numbers
    /// *is* the drift in stops that [`Settings::within`] tolerates.
    ///
    /// `None` when the file does not record all three, or records a zero: a
    /// frame that cannot be placed cannot be shown to differ.
    fn brightness(self) -> Option<f64> {
        let speed = self.iso?;
        let time = self.exposure_time?.value()?;
        let aperture = self.f_number?.value()?;
        if speed == 0 || time <= 0.0 || aperture <= 0.0 {
            return None;
        }
        Some((time * (f64::from(speed) / 100.0) / (aperture * aperture)).log2())
    }
}

/// A fraction as the file stored it, compared by its **value**: `1/500` and
/// `2/1000` are one exposure as a photographer means it, though a camera may
/// write either.
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

    /// The number the fraction stands for, or `None` for a zero denominator —
    /// a corrupt tag rather than a very fast shutter.
    pub fn value(self) -> Option<f64> {
        if self.denominator == 0 {
            return None;
        }
        Some(f64::from(self.numerator) / f64::from(self.denominator))
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

/// One group the sweep is filling: its first frame's facts (every gate but the
/// gap compares against them), its last frame's capture time (the gap compares
/// against that), and the indices of the frames it holds.
#[derive(Debug, Clone)]
struct Moment {
    first: Facts,
    last: Capture,
    members: Vec<usize>,
}

impl Moment {
    /// The moment a frame opens on its own.
    fn opening(index: usize, facts: &Facts) -> Moment {
        Moment {
            first: facts.clone(),
            last: facts
                .captured
                .expect("a timed frame records its capture time"),
            members: vec![index],
        }
    }

    /// Whether `candidate` belongs to this moment: all four gates, in order,
    /// against the group's first frame — and the time gate against its last.
    fn takes(&self, candidate: &Facts, thresholds: &Thresholds) -> bool {
        let Some(next) = candidate.captured else {
            return false;
        };
        if self.last.seconds_after(&next) > thresholds.gap.as_secs_f64() {
            return false;
        }
        if self.first.serial != candidate.serial
            || !self
                .first
                .settings
                .within(candidate.settings, thresholds.exposure_stops)
        {
            return false;
        }
        let (Some(seen), Some(fresh)) = (&self.first.signature, &candidate.signature) else {
            return false;
        };
        seen.similarity(fresh) >= thresholds.similarity
    }

    /// Add a frame that [`takes`](Moment::takes) it.
    fn add(&mut self, index: usize, facts: &Facts) {
        self.members.push(index);
        self.last = facts
            .captured
            .expect("a timed frame records its capture time");
    }
}

/// The sweep that turns a capture-ordered stream of timed frames into groups.
///
/// The one implementation of the grouping rule: [`assign`] runs it over a whole
/// slice, and the streaming pass runs it over frames as their previews arrive.
/// A frame is only ever offered to the most recent group, which is what makes a
/// group final the moment a later frame starts a new one — or fails the gap
/// against its last member.
#[derive(Debug, Clone)]
struct Grouper<'a> {
    thresholds: &'a Thresholds,
    open: Option<Moment>,
}

impl<'a> Grouper<'a> {
    fn new(thresholds: &'a Thresholds) -> Grouper<'a> {
        Grouper {
            thresholds,
            open: None,
        }
    }

    /// Offer one timed frame, in capture order. The group it closes, when the
    /// frame starts a new one instead of joining the open one.
    fn push(&mut self, index: usize, facts: &Facts) -> Option<Grouping> {
        if let Some(open) = self.open.as_mut()
            && open.takes(facts, self.thresholds)
        {
            open.add(index, facts);
            return None;
        }
        let closed = self.open.take().map(Moment::closing);
        self.open = Some(Moment::opening(index, facts));
        closed
    }

    /// Close the open group when the next frame in capture order cannot reach
    /// it: the gap is the one gate a frame can fail by time alone, so a frame
    /// beyond it can never join, whatever its other facts say.
    ///
    /// This is what lets a streaming pass start culling a group before the next
    /// frame's preview exists.
    fn close_before(&mut self, next: Capture) -> Option<Grouping> {
        let open = self.open.as_ref()?;
        if open.last.seconds_after(&next) > self.thresholds.gap.as_secs_f64() {
            self.open.take().map(Moment::closing)
        } else {
            None
        }
    }

    /// Close the open group at the end of the timed frames.
    fn close(&mut self) -> Option<Grouping> {
        self.open.take().map(Moment::closing)
    }
}

impl Moment {
    /// The finished group, as the plan numbers it.
    fn closing(self) -> Grouping {
        Grouping {
            members: self.members,
        }
    }
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

    let mut grouper = Grouper::new(thresholds);
    let mut groups: Vec<Grouping> = Vec::new();
    for index in ordered {
        if let Some(closed) = grouper.push(index, &facts[index]) {
            groups.push(closed);
        }
    }
    if let Some(closed) = grouper.close() {
        groups.push(closed);
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
    hasher.update(format!("\nexposure_stops={:.6}", thresholds.exposure_stops).as_bytes());
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

/// A group the streaming pass has finished finding: it can no longer gain a
/// frame. Either the next frame in capture order is beyond its gap, or that frame
/// has opened a new group by failing one of the other gates.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// The group's number, from 1, in capture order.
    pub number: usize,
    /// Its frames, in capture order.
    pub frames: Vec<Planned>,
}

impl Found {
    /// How many frames the group holds.
    pub fn size(&self) -> usize {
        self.frames.len()
    }

    /// The group as the cull reads it.
    pub fn row(&self) -> GroupRow<'_> {
        GroupRow {
            number: self.number,
            frames: &self.frames,
        }
    }
}

/// Where a streaming pass puts each group as soon as it is final.
///
/// The pass calls [`group`](Sink::group) as soon as the sweep can prove the group
/// can no longer grow — often from the next frame's header alone, before that
/// frame's preview has been decoded — so a sink can start the group's work while
/// the rest of the shoot is still being read. An error stops the pass: a sink that
/// cannot take a group has no use for the ones after it.
pub trait Sink {
    /// One finished group, in capture order.
    fn group(&mut self, group: Found) -> Result<()>;
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
/// rather than the plan's capture order, because a photographer reads the folder
/// as they named it; the CSV is where capture order lives. A cull's verdict lines
/// may arrive before some of these when a group is finished ahead of an earlier
/// scan index, but the report at the end is in group order.
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
/// One pass over the frames, one decode per frame, no requests. A frame that
/// cannot be read is a row with empty measures and a status, and a group of its
/// own: one odd file never costs the photographer the other 399.
///
/// The frames are independent of each other — a reading is a pure function of
/// one path — so their previews are read [`Options::jobs`] at a time, and the
/// plan is assembled in the scan's order whatever order they finished in.
/// Everything this pass costs is local CPU, which is why this command is worth
/// threading where `score` is not: no server is serialising anything here.
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
    /// A pass with no interest in a group before it is all over.
    struct Ignore;

    impl Sink for Ignore {
        fn group(&mut self, _group: Found) -> Result<()> {
            Ok(())
        }
    }

    stream_with(shoot, options, watcher, &mut Ignore)
}

/// [`run_with`], handing each group to `sink` the moment it can no longer grow.
///
/// The frame events still arrive in the scan's order, exactly as [`run_with`]
/// says; the sink's groups arrive in capture order, as soon as the next frame's
/// capture time is beyond the open group's gap. The two orders are the reason a
/// group can be culled before the folder has finished being read.
///
/// The metadata pass is what makes that possible. A group cannot be final until
/// a frame's capture time is known, and a preview cannot be placed in capture
/// order until it has been decoded; reading every header first costs one extra
/// open per frame and lets the previews be handed out in the order the groups
/// need them. A frame whose header cannot be read is unreadable in exactly the
/// way [`read`] would have said so, and falls into the undated tail.
pub fn stream_with(
    shoot: &Path,
    options: &Options,
    watcher: &dyn Watcher,
    sink: &mut dyn Sink,
) -> Result<Plan> {
    check(&options.thresholds)?;
    let started = Instant::now();
    let scan = ingest::scan(shoot, &options.scan)?;
    let total = scan.files.len();
    let jobs = degree(options.jobs, total);
    watcher.event(Event::Started(Started { total, jobs }));

    // The headers first: capture times, bodies and exposures, which is what
    // places every frame in the one order grouping reads.
    let prepared = read_metadata(&scan.files, jobs);
    let captures: Vec<Option<Capture>> = prepared
        .iter()
        .map(|prepared| {
            prepared
                .as_ref()
                .ok()
                .and_then(|metadata| metadata.capture_time().map(Capture::of))
        })
        .collect();

    // The order the sweep considers frames in: the timed ones by capture time
    // (the scan's index breaking a tie, as [`assign`] does), then every frame
    // with no capture time in the scan's order. Previews are still read in the
    // scan's order; this is the order their *readings* are consumed in.
    let mut order: Vec<usize> = (0..total)
        .filter(|&index| captures[index].is_some())
        .collect();
    order.sort_by_key(|&index| (captures[index], index));
    let timed = order.len();
    order.extend((0..total).filter(|&index| captures[index].is_none()));

    let mut foundry = Foundry {
        sources: &scan.files,
        order: &order,
        captures: &captures,
        timed,
        grouper: Grouper::new(&options.thresholds),
        groups: 0,
        planned: (0..total).map(|_| None).collect(),
        untimed: Vec::new(),
        frames: Vec::with_capacity(total),
        sink,
    };
    read_previews(
        &scan.files,
        &prepared,
        &order,
        jobs,
        watcher,
        |position, index, reading| foundry.consume(position, index, reading),
    )?;
    foundry.close()?;
    let groups = foundry.groups;
    let frames = foundry.frames;

    Ok(Plan {
        groups,
        frames,
        hash: grouping_hash(&options.thresholds),
        counts: scan.counts,
        elapsed: started.elapsed(),
        jobs,
    })
}

/// The state a streamed pass fills as readings arrive and groups finish.
struct Foundry<'a> {
    /// The files, for the frame a group reports.
    sources: &'a [SourceFile],
    /// The order readings are consumed in: timed frames first, by capture time.
    order: &'a [usize],
    /// The capture time the metadata pass found, for the lookahead that closes a
    /// group before its next frame's preview exists.
    captures: &'a [Option<Capture>],
    /// How many leading entries of `order` are timed.
    timed: usize,
    /// The group being filled.
    grouper: Grouper<'a>,
    /// How many groups have been handed to the sink.
    groups: usize,
    /// One row per frame, built when its reading arrives and moved out when its
    /// group is handed over.
    planned: Vec<Option<Planned>>,
    /// Frames with no signature, each a group of its own at the end.
    untimed: Vec<usize>,
    /// The plan's rows, in group order.
    frames: Vec<Planned>,
    /// Where each finished group goes.
    sink: &'a mut dyn Sink,
}

impl Foundry<'_> {
    /// Take one reading that the sweep is due to consider, in `order`.
    fn consume(&mut self, position: usize, index: usize, reading: &Reading) -> Result<()> {
        let planned = Planned {
            source: self.sources[index].clone(),
            group: 0,
            group_index: 1,
            group_size: 1,
            captured: reading.facts.captured,
            measures: reading.measures,
            failure: reading.failure.clone(),
        };
        self.planned[index] = Some(planned);

        if position >= self.timed {
            // No capture time at all: the undated tail, in the scan's order.
            self.untimed.push(index);
            return Ok(());
        }

        if reading.facts.signature.is_none() {
            // Printable but not comparable: `assign` would never place it in a
            // timed group, so it belongs to the tail and must not interrupt the
            // open one.
            self.untimed.push(index);
        } else if let Some(closed) = self.grouper.push(index, &reading.facts) {
            self.emit(closed)?;
        }

        // The next frame's header already says whether it can reach the open
        // group at all. If it cannot, the group is final now — its next frame's
        // preview has not been decoded, and does not need to be.
        if position + 1 < self.timed
            && let Some(next) = self.captures[self.order[position + 1]]
            && let Some(closed) = self.grouper.close_before(next)
        {
            self.emit(closed)?;
        }
        Ok(())
    }

    /// Finish the timed groups, then the undated tail in the scan's order.
    fn close(&mut self) -> Result<()> {
        if let Some(closed) = self.grouper.close() {
            self.emit(closed)?;
        }
        let mut untimed = std::mem::take(&mut self.untimed);
        untimed.sort_unstable();
        for index in untimed {
            self.emit(Grouping {
                members: vec![index],
            })?;
        }
        Ok(())
    }

    /// Number one finished group, keep its rows, and hand it to the sink.
    fn emit(&mut self, grouping: Grouping) -> Result<()> {
        let number = self.groups + 1;
        let size = grouping.members.len();
        let mut frames = Vec::with_capacity(size);
        for (place, index) in grouping.members.iter().enumerate() {
            let mut frame = self.planned[*index]
                .take()
                .expect("a group is only finished after its frames are read");
            frame.group = number;
            frame.group_index = place + 1;
            frame.group_size = size;
            self.frames.push(frame.clone());
            frames.push(frame);
        }
        self.groups = number;
        self.sink.group(Found { number, frames })
    }
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

/// Read every frame's header, `jobs` at a time, in the scan's order.
///
/// A header is all the grouping gates but the signature need, and it is what
/// puts a frame in capture order before its preview exists. A frame the header
/// cannot be read from carries the failure that costs it its measures, exactly
/// as [`read`] would have reported it.
fn read_metadata(
    sources: &[SourceFile],
    jobs: usize,
) -> Vec<std::result::Result<Metadata, Failure>> {
    let total = sources.len();
    if jobs < 2 || total < 2 {
        return sources
            .iter()
            .map(|source| preview::metadata(&source.path).map_err(|error| Failure::of(&error)))
            .collect();
    }

    let cursor = AtomicUsize::new(0);
    let (sender, receiver) = mpsc::channel::<(usize, std::result::Result<Metadata, Failure>)>();
    let cursor = &cursor;
    let mut prepared: Vec<Option<std::result::Result<Metadata, Failure>>> =
        (0..total).map(|_| None).collect();
    thread::scope(|scope| {
        for _ in 0..jobs {
            let sender = sender.clone();
            scope.spawn(move || {
                while let Some(index) = next(cursor, total) {
                    let result = preview::metadata(&sources[index].path)
                        .map_err(|error| Failure::of(&error));
                    // The receiver lives until every sender is gone, which is
                    // the end of this scope: a send cannot be refused here.
                    sender
                        .send((index, result))
                        .expect("the pass is still collecting headers");
                }
            });
        }
        drop(sender);
        for (index, result) in receiver {
            prepared[index] = Some(result);
        }
    });
    prepared
        .into_iter()
        .map(|result| result.expect("every frame has a header reading"))
        .collect()
}

/// Read every frame's preview, `jobs` at a time, in the scan's order, telling
/// `watcher` about each frame as it arrives and handing each reading to
/// `consume` in `order`.
///
/// The two orders are the point: a photographer reads frame lines in the scan's
/// order, while the sweep needs the readings in capture order. A worker claims
/// the next scan index from a cursor; the pass's own thread buffers whatever
/// arrived out of order, says each frame exactly once in index order, and then
/// consumes every position of `order` that has become available. So the plan,
/// the CSV, the hash and the log are the same at any degree, and a group can be
/// finished before the folder has been fully read.
fn read_previews(
    sources: &[SourceFile],
    prepared: &[std::result::Result<Metadata, Failure>],
    order: &[usize],
    jobs: usize,
    watcher: &dyn Watcher,
    mut consume: impl FnMut(usize, usize, &Reading) -> Result<()>,
) -> Result<()> {
    let total = sources.len();
    let mut readings: Vec<Option<Reading>> = (0..total).map(|_| None).collect();
    let mut announced = 0;
    let mut due = 0;
    let mut deliver = |index: usize, reading: Reading| -> Result<()> {
        readings[index] = Some(reading);
        while announced < total {
            let Some(reading) = readings[announced].as_ref() else {
                break;
            };
            watcher.event(Event::Frame(FrameEvent::of(
                announced,
                total,
                &sources[announced],
                reading,
            )));
            announced += 1;
        }
        while due < order.len() {
            let index = order[due];
            let Some(reading) = readings[index].as_ref() else {
                break;
            };
            consume(due, index, reading)?;
            due += 1;
        }
        Ok(())
    };
    if jobs < 2 || total < 2 {
        for (index, source) in sources.iter().enumerate() {
            deliver(index, read_streamed(source, prepared[index].as_ref()))?;
        }
        debug_assert_eq!(announced, total, "every frame line was announced");
        debug_assert_eq!(due, order.len(), "every reading was consumed");
        return Ok(());
    }

    let cursor = AtomicUsize::new(0);
    let (sender, receiver) = mpsc::channel::<(usize, Reading)>();
    let cursor = &cursor;
    thread::scope(|scope| {
        for _ in 0..jobs {
            let sender = sender.clone();
            scope.spawn(move || {
                while let Some(index) = next(cursor, total) {
                    let reading = read_streamed(&sources[index], prepared[index].as_ref());
                    // A sink that stopped the pass may have dropped the receiver;
                    // a worker's reading is then simply not wanted.
                    let _ = sender.send((index, reading));
                }
            });
        }
        // Drop the pass's own handle, or the channel would never close.
        drop(sender);
        for (index, reading) in receiver {
            deliver(index, reading)?;
        }
        Ok(())
    })?;
    debug_assert_eq!(announced, total, "every frame line was announced");
    debug_assert_eq!(due, order.len(), "every reading was consumed");
    Ok(())
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
    read_embedded(&metadata, embedded)
}

/// Read one frame against a header the metadata pass already read.
///
/// The failure is the one [`read`] would have produced from a single open: a
/// header that could not be read at all leaves the frame with no facts, and a
/// preview that cannot be extracted leaves it with no facts either, while a
/// preview that was extracted but could not be decoded keeps the capture time
/// the header recorded.
fn read_streamed(
    source: &SourceFile,
    prepared: std::result::Result<&Metadata, &Failure>,
) -> Reading {
    let metadata = match prepared {
        Ok(metadata) => metadata,
        Err(failure) => {
            return Reading {
                facts: Facts::default(),
                measures: Measures::default(),
                failure: Some(failure.clone()),
            };
        }
    };
    let embedded = match preview::extract(&source.path) {
        Ok(embedded) => embedded,
        Err(error) => return Reading::unreadable(&error),
    };
    read_embedded(metadata, embedded)
}

/// The decode and the measures, from an extracted preview and the header it came
/// with: the half of [`read`] that is the same however the picture arrived.
fn read_embedded(metadata: &Metadata, embedded: preview::Embedded) -> Reading {
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
                facts: Facts::of(metadata, None),
                measures: Measures::default(),
                failure: Some(Failure::of(&error)),
            };
        }
    };
    Reading {
        facts: Facts::of(metadata, Signature::of_rgb(&rgb).ok()),
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

    /// One exposure as a camera records it, so that a test varying one field is
    /// varying that field and not silently clearing the other two.
    fn exposure() -> Settings {
        Settings {
            iso: Some(200),
            exposure_time: Some(Fraction {
                numerator: 1,
                denominator: 500,
            }),
            f_number: Some(Fraction {
                numerator: 28,
                denominator: 10,
            }),
            focal_length: Some(Fraction {
                numerator: 50,
                denominator: 1,
            }),
            orientation: 1,
        }
    }

    fn facts(second: u8, serial: &str, at: usize) -> Facts {
        Facts {
            captured: Some(capture(second, 0)),
            serial: Some(serial.to_string()),
            settings: exposure(),
            signature: Some(signature(at)),
        }
    }

    fn sizes(groups: &[Grouping]) -> Vec<usize> {
        groups.iter().map(|group| group.members.len()).collect()
    }

    /// The groups a streaming sweep finds, with the lookahead the pass uses: a
    /// group closes when the *next* frame's header cannot reach it.
    fn streamed(facts: &[Facts], thresholds: &Thresholds) -> Vec<Grouping> {
        let timed: Vec<bool> = facts
            .iter()
            .map(|frame| frame.captured.is_some() && frame.signature.is_some())
            .collect();
        let mut ordered: Vec<usize> = (0..facts.len()).filter(|&index| timed[index]).collect();
        ordered.sort_by_key(|&index| (facts[index].captured, index));

        let mut grouper = Grouper::new(thresholds);
        let mut groups = Vec::new();
        for (position, &index) in ordered.iter().enumerate() {
            if let Some(closed) = grouper.push(index, &facts[index]) {
                groups.push(closed);
            }
            if position + 1 < ordered.len()
                && let Some(next) = facts[ordered[position + 1]].captured
                && let Some(closed) = grouper.close_before(next)
            {
                groups.push(closed);
            }
        }
        if let Some(closed) = grouper.close() {
            groups.push(closed);
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

    #[test]
    fn a_streamed_sweep_does_not_close_a_group_before_the_frame_that_extends_it() {
        // The second frame is a second after the first, and the third is three
        // seconds after the second: the group's gap is measured from its *last*
        // member, so the second frame must join before the lookahead decides the
        // group is over.
        let frames = vec![
            facts(0, "3001", 0),
            facts(1, "3001", 0),
            facts(4, "3001", 0),
        ];

        assert_eq!(sizes(&assign(&frames, &Thresholds::default())), vec![2, 1]);
        assert_eq!(
            sizes(&streamed(&frames, &Thresholds::default())),
            vec![2, 1]
        );
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

        // The settings gate tolerates brightness drift up to the threshold and
        // no further: the same second, the same body, two stops apart groups at
        // the default and three stops apart does not.
        let two_stops = vec![
            facts(0, "1", 0),
            Facts {
                settings: Settings {
                    iso: Some(800),
                    ..exposure()
                },
                ..facts(1, "1", 0)
            },
        ];
        assert_eq!(sizes(&assign(&two_stops, &thresholds)), vec![2]);

        let three_stops = vec![
            facts(0, "1", 0),
            Facts {
                settings: Settings {
                    iso: Some(1600),
                    ..exposure()
                },
                ..facts(1, "1", 0)
            },
        ];
        assert_eq!(sizes(&assign(&three_stops, &thresholds)), vec![1, 1]);

        // And the threshold is the flag's: what the default tolerates, zero
        // stops refuses.
        let exact = Thresholds {
            exposure_stops: 0.0,
            ..Thresholds::default()
        };
        assert_eq!(sizes(&assign(&two_stops, &exact)), vec![1, 1]);

        // And an orientation that changed mid-group is not the same
        // composition.
        let turned = vec![
            facts(0, "1", 0),
            Facts {
                settings: Settings {
                    orientation: 6,
                    ..exposure()
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
    fn one_exposure_is_one_brightness_however_the_camera_got_there() {
        let fraction = |numerator, denominator| {
            Some(Fraction {
                numerator,
                denominator,
            })
        };
        let exposed = Settings {
            iso: Some(200),
            exposure_time: fraction(1, 500),
            f_number: fraction(28, 10),
            focal_length: fraction(50, 1),
            orientation: 1,
        };
        // A stop of ISO for a stop of shutter: the same picture, brighter by
        // nothing at all.
        let equivalent = Settings {
            iso: Some(400),
            exposure_time: fraction(1, 1000),
            ..exposed
        };
        assert!(exposed.within(equivalent, 0.0), "one exposure, exactly");

        // f/2.8 to f/5.6 is two stops darker, and the gate reads the numbers a
        // camera writes rather than the clicks a lens makes: f/2.8 to f/4 is
        // 1.03 stops, not 1.
        let stopped_down = Settings {
            f_number: fraction(56, 10),
            ..exposed
        };
        let drift = (stopped_down.brightness().expect("an exposure")
            - exposed.brightness().expect("an exposure"))
        .abs();
        assert!(
            (drift - 2.0).abs() < 1e-6,
            "f/2.8 to f/5.6 is two stops of light: {drift}"
        );
        assert!(exposed.within(stopped_down, drift));
        assert!(
            !exposed.within(stopped_down, 1.5),
            "two stops is not one and a half"
        );

        // A number the gate cannot take the log of is not a difference.
        let corrupt = Settings {
            f_number: fraction(28, 0),
            ..exposed
        };
        assert_eq!(corrupt.brightness(), None);
        assert!(
            exposed.within(corrupt, 0.0),
            "a zero denominator is unknown"
        );
    }

    #[test]
    fn the_exposure_gate_tolerates_brightness_and_nothing_else() {
        let fraction = |numerator, denominator| {
            Some(Fraction {
                numerator,
                denominator,
            })
        };
        let wide = Settings {
            iso: Some(200),
            exposure_time: fraction(1, 500),
            f_number: fraction(28, 10),
            focal_length: fraction(50, 1),
            orientation: 1,
        };

        // The tolerant half: any drift at all.
        let darker = Settings {
            iso: Some(1600),
            ..wide
        };
        assert!(
            wide.within(darker, 3.5),
            "three stops, when three and a half is allowed"
        );
        assert!(
            wide.within(darker, 1e9),
            "a gate with no opinion about exposure"
        );

        // The exact half: a zoom and a turn are different pictures, whatever the
        // tolerance says.
        let zoomed = Settings {
            focal_length: fraction(85, 1),
            ..wide
        };
        assert!(!wide.within(zoomed, 1e9), "a zoom is a different picture");
        let turned = Settings {
            orientation: 6,
            ..wide
        };
        assert!(
            !wide.within(turned, 1e9),
            "a turned frame is a different composition"
        );
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

        // A frame that records no exposure at all is not shown to differ from
        // one that does.
        let half_recorded = vec![
            Facts {
                settings: Settings {
                    iso: Some(400),
                    exposure_time: Some(Fraction {
                        numerator: 1,
                        denominator: 60,
                    }),
                    f_number: Some(Fraction {
                        numerator: 2,
                        denominator: 1,
                    }),
                    ..Settings::default()
                },
                ..facts(0, "1", 0)
            },
            Facts {
                settings: Settings::default(),
                ..facts(1, "1", 0)
            },
        ];
        assert_eq!(sizes(&assign(&half_recorded, &thresholds)), vec![2]);

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

        // And the settings they belong to read the same way: `1/500` and
        // `2/1000` are one exposure, so no threshold can tell them apart.
        let fast = Settings {
            iso: Some(200),
            exposure_time: half(1, 500),
            f_number: half(28, 10),
            ..Settings::default()
        };
        let same = Settings {
            exposure_time: half(2, 1000),
            ..fast
        };
        assert_eq!(fast, same);
        assert!(fast.within(same, 0.0));
        assert_eq!(fast.brightness(), same.brightness());
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
        assert_ne!(
            base,
            grouping_hash(&Thresholds {
                exposure_stops: 1.0,
                ..Thresholds::default()
            }),
            "two runs that tolerated different exposure drift did not group the \
             same way"
        );
    }
}
