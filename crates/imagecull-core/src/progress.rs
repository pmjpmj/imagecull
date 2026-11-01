//! What a run says while it runs, as plain data.
//!
//! The pipeline has one more seam than [`Decider`](crate::scoring::Decider): it
//! describes what it is doing and hands the description to a [`Reporter`]. A
//! terminal renders it as a line that overwrites itself; a window will draw it
//! as a bar; a test collects it into a vector (spec §"The seam"). Nothing here
//! formats for a screen, asks whether stdout is a terminal, or exits — the
//! crate's lints keep the core from ever learning how (spec §52–53).
//!
//! Every event is owned data: a front end may keep it, log it, or drop it, and
//! the run does not care which.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use crate::error::ErrorKind;
use crate::ingest::ScanCounts;
use crate::reading::Answers;
use crate::report::{self, Failure};
use crate::rubric::{Builtin, RubricHash};
use crate::scoring::{Ending, Run};

/// Where a run's progress goes.
///
/// One method, carrying owned data: a front end keeps what it draws and drops
/// the rest, and the run never learns which.
pub trait Reporter {
    /// One event from the run.
    fn event(&self, event: Event);
}

/// A reporter with nothing to say: what a caller that does not care passes.
#[derive(Debug, Clone, Copy, Default)]
pub struct Silent;

impl Reporter for Silent {
    fn event(&self, _event: Event) {}
}

/// One thing a run tells the front end.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The run has scanned its folder and is about to score it.
    Started(Started),
    /// One frame is done, whichever way it went: answered, reused, or failed.
    Frame(FrameEvent),
    /// Something the photographer should know that is not a row: work that was
    /// recovered, a run that is not going to be what they expected.
    Warning(Warning),
    /// The run is over, however it ended. The last event a reporter receives.
    Finished(Finished),
}

/// A note about the run rather than about one frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// What happened, and what it means for the report.
    pub message: String,
}

/// The run ending: what it did, what it left out, and how long it took.
#[derive(Debug, Clone, PartialEq)]
pub struct Finished {
    /// The numbers a photographer judges the run and the rubric by.
    pub summary: Summary,
}

/// The rule the disagreement rate counts, in the words the summary line shows
/// it, so the number can be read without reading this file.
///
/// The spec names the metric but not its threshold (spec §29): `stars` and
/// `overall` are two different questions and will sometimes disagree, and what
/// matters is a disagreement you would notice, not the rounding noise between a
/// 7.0 overall and three stars. One star of difference is noise; two is a
/// judgement worth looking at, and it is counted, never averaged away.
pub const DISAGREEMENT_RULE: &str =
    "a frame disagrees when its stars are two or more away from round(overall / 2), clamped to 1-5";

/// The star rating an `overall` score implies: its band out of five.
///
/// Clamped in `f64` before the cast, so a score outside the legend still lands
/// in one of the five bands.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn implied_stars(overall: f64) -> u8 {
    (overall / 2.0).round().clamp(1.0, 5.0) as u8
}

/// Whether a frame's `stars` and `overall` agree, by [`DISAGREEMENT_RULE`].
pub fn agrees(stars: u8, overall: f64) -> bool {
    let stars = i32::from(stars);
    let implied = i32::from(implied_stars(overall));
    (stars - implied).abs() < 2
}

/// How many frames carry one `status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusCount {
    /// The `status` column's value: `ok`, or a taxonomy kind.
    pub status: &'static str,
    /// How many rows carry it.
    pub count: usize,
}

/// The mean and median of one score, over the frames that have answers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeanMedian {
    /// The arithmetic mean: the rubric's centre of gravity.
    pub mean: f64,
    /// The middle value, or the midpoint of the two middle values on an even
    /// count: what one bad model call cannot drag.
    pub median: f64,
}

/// Mean and median of each of the four scores, kept apart exactly as they are
/// in every row (spec §19).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scores {
    /// The keeper judgement.
    pub overall: MeanMedian,
    /// Technical quality only.
    pub technical: MeanMedian,
    /// Framing and graphic design only.
    pub composition: MeanMedian,
    /// Moment, gesture and light only.
    pub impact: MeanMedian,
}

/// What a run did, in the numbers a photographer judges it by.
///
/// A summary is computed from a [`Run`], so it cannot disagree with the frames
/// the CSV was written from, and it says how the run ended as well as how much
/// it did: a stopped run is not a finished one.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    /// How many files the scan accepted: the work the run took on.
    pub total: usize,
    /// How many frames the run got to: the sum of every status count.
    pub judged: usize,
    /// How many frames produced six readable answers.
    pub scored: usize,
    /// How many did not, for any reason.
    pub failed: usize,
    /// How many frames the resumed ledger already held.
    pub resumed: usize,
    /// How many frames reused an identical preview's outcome.
    pub duplicates: usize,
    /// Counts by `status`: `ok` first, then the taxonomy's own order, omitting
    /// the kinds no frame hit.
    pub counts: Vec<StatusCount>,
    /// The four scores' mean and median; `None` when nothing was scored.
    pub scores: Option<Scores>,
    /// The star histogram, index 0 holding one star.
    pub stars: [usize; 5],
    /// How many scored frames' `stars` and `overall` disagree.
    pub disagreements: usize,
    /// [`Summary::disagreements`] over the scored frames, `0.0` when there are
    /// none. Reported, never reconciled (spec §29).
    pub disagreement_rate: f64,
    /// The built-in rubric a name selected, when one did: provenance the hash
    /// cannot carry, because the same words are the same judgement however a
    /// run reached them.
    pub builtin: Option<Builtin>,
    /// The rubric every frame was scored under: the number's provenance.
    pub rubric_hash: RubricHash,
    /// The alias every frame was scored with.
    pub model: String,
    /// The config file the run's values came from, when one did: the provenance
    /// line prints it beside the rubric hash and the model alias.
    pub config: Option<PathBuf>,
    /// From the first scan to the last write.
    pub elapsed: Duration,
    /// What the scan left out because it was not a RAW file, was Finder noise,
    /// or was in a hidden directory.
    pub left_out: ScanCounts,
    /// How the run ended, which is also what it exits as.
    pub ending: Ending,
}

impl Summary {
    /// Summarise a run that has finished, stopped, or been interrupted.
    pub fn of(run: &Run) -> Summary {
        let mut failures: BTreeMap<ErrorKind, usize> = BTreeMap::new();
        let mut overall = Vec::new();
        let mut technical = Vec::new();
        let mut composition = Vec::new();
        let mut impact = Vec::new();
        let mut stars = [0usize; 5];
        let mut disagreements = 0;
        for frame in &run.frames {
            let answers = match frame.answers_or_failure() {
                Ok(answers) => answers,
                Err(failure) => {
                    *failures.entry(failure.kind).or_default() += 1;
                    continue;
                }
            };
            overall.push(answers.overall);
            technical.push(answers.technical);
            composition.push(answers.composition);
            impact.push(answers.impact);
            let bucket = usize::from(answers.stars.clamp(1, 5)) - 1;
            stars[bucket] += 1;
            if !agrees(answers.stars, answers.overall) {
                disagreements += 1;
            }
        }
        // How many frames have answers is the run's own count, not a second
        // tally kept here: the exit code and the printed summary must never
        // disagree about how much of the shoot was scored.
        let scored = run.scored();
        let mut counts = Vec::with_capacity(1 + ErrorKind::ALL.len());
        if scored > 0 {
            counts.push(StatusCount {
                status: report::OK,
                count: scored,
            });
        }
        for kind in ErrorKind::ALL {
            if let Some(count) = failures.get(&kind).filter(|count| **count > 0) {
                counts.push(StatusCount {
                    status: kind.as_str(),
                    count: *count,
                });
            }
        }
        let scores = (scored > 0).then(|| Scores {
            overall: mean_median(overall),
            technical: mean_median(technical),
            composition: mean_median(composition),
            impact: mean_median(impact),
        });
        #[allow(clippy::cast_precision_loss)]
        let disagreement_rate = if scored == 0 {
            0.0
        } else {
            disagreements as f64 / scored as f64
        };
        Summary {
            total: run.total,
            judged: run.frames.len(),
            scored,
            failed: run.failed(),
            resumed: run.resumed,
            duplicates: run.duplicates,
            counts,
            scores,
            stars,
            disagreements,
            disagreement_rate,
            builtin: run.builtin,
            rubric_hash: run.rubric_hash.clone(),
            model: run.model.clone(),
            config: run.source.clone(),
            elapsed: run.elapsed,
            left_out: run.counts,
            ending: run.ending.clone(),
        }
    }
}

/// The mean and the median of `values`, which must not be empty.
fn mean_median(mut values: Vec<f64>) -> MeanMedian {
    debug_assert!(!values.is_empty(), "only scored frames are measured");
    #[allow(clippy::cast_precision_loss)]
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    let median = if values.len().is_multiple_of(2) {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    };
    MeanMedian { mean, median }
}

/// The run beginning: how many frames it faces, under which rubric and alias.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Started {
    /// How many files the scan accepted. The denominator of every index.
    pub total: usize,
    /// The built-in rubric a name selected, when one did: the name is how the
    /// rubric was chosen and is printed beside the hash that identifies it.
    pub builtin: Option<Builtin>,
    /// The rubric's identity, so the front end can show what is being applied.
    pub rubric_hash: RubricHash,
    /// The alias actually in use.
    pub model: String,
}

/// One frame, done: the unit the frame line shows.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameEvent {
    /// The frame's position, counting from 1, so a line reads `12/400`.
    pub index: usize,
    /// How many frames the run faces: the same total [`Started`] announced.
    pub total: usize,
    /// The name the CSV will use for this frame.
    pub filename: String,
    /// The six answers, or the classified reason there are none.
    pub judgement: Judgement,
    /// The latency of the request that produced this frame's outcome, when one
    /// was recorded: `None` for a frame that never reached the model.
    pub latency_ms: Option<u64>,
    /// How long the whole run has left, estimated from what it has done so far,
    /// once there is something to estimate from. `None` on the last frame, and
    /// on a run that has not finished a frame yet.
    pub eta: Option<Duration>,
    /// How large the preview that was sent was, when this run prepared one.
    pub preview: Option<PreviewShape>,
    /// The earlier frame whose identical preview this one reused, if it reused
    /// one (spec §34).
    pub duplicate_of: Option<String>,
}

/// A frame's outcome, as the front end sees it.
#[derive(Debug, Clone, PartialEq)]
pub enum Judgement {
    /// Six answers, at the precision the model gave them.
    Answered(Answers),
    /// The row's `status` and `error` columns: the same failure the CSV carries.
    Failed(Failure),
}

/// The bytes that were sent, in the shape a photographer thinks about them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewShape {
    /// The width of the sent preview, after orientation and downscale.
    pub width: u32,
    /// The height of the sent preview, after orientation and downscale.
    pub height: u32,
    /// How many bytes the encoded JPEG took.
    pub jpeg_bytes: usize,
}
