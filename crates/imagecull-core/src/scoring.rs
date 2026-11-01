//! One frame in, six answers out: the scoring entry point, and the one seam
//! the pipeline has.
//!
//! Everything the model would see is decided here and passed across
//! [`Decider`]: the preview bytes extraction oriented and downscaled, the
//! rubric's questions in declaration order, the state the frame is judged in,
//! and the alias. Tests drive this module with a recording decider; the real
//! [`systemone::Adapter`](crate::systemone::Adapter) is the only other
//! implementation, and the only thing that opens a socket.
//!
//! Requests are strictly sequential. oMLX serialises decision requests anyway,
//! so concurrency would only manufacture timeouts (spec §7.1).

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::error::{ExitCode, Result};
use crate::ingest::{self, ScanCounts, ScanOptions, SourceFile};
use crate::preprocess::{self, Encoded, Params};
use crate::preview;
use crate::reading::{self, Reading};
use crate::report::{self, Outputs, Row};
use crate::rubric::{Rubric, RubricHash};

/// The task framing every request's state carries (spec §7.1).
pub const TASK: &str = "photo_cull";

/// What the model is asked about, besides the picture.
///
/// Never the scores: the state is the setup, not the answer. `filename` is the
/// name the photographer will see in the CSV, and `shoot_context` is their own
/// framing of the shoot ("street at dusk, high ISO expected"), so a dim frame
/// is judged as that and not as a failed studio frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    /// The task framing, [`TASK`] for a cull.
    pub task: String,
    /// The frame's name, as reported.
    pub filename: String,
    /// The photographer's `--shoot-context`, when they gave one.
    pub shoot_context: Option<String>,
}

impl State {
    /// The state for one frame: fixed task framing, the frame's name, and the
    /// photographer's optional context.
    pub fn for_frame(filename: impl Into<String>, shoot_context: Option<String>) -> State {
        State {
            task: TASK.to_string(),
            filename: filename.into(),
            shoot_context,
        }
    }
}

/// One frame's question to the model.
#[derive(Debug)]
pub struct Request<'a> {
    /// The alias the rubric hash covers and the request asks for.
    pub model: &'a str,
    /// The questions, in declaration order.
    pub rubric: &'a Rubric,
    /// The framing and the frame's name.
    pub state: State,
    /// The exact bytes to send: oriented, downscaled, encoded, no EXIF.
    pub preview: &'a Encoded,
}

/// What the server answered: the raw document, kept whole, and how long the
/// round trip took on this side of the socket.
///
/// The token and server-latency numbers live inside the document, so reading it
/// is one job in one place ([`reading`]).
#[derive(Debug, Clone)]
pub struct Response {
    /// The full answer document, for [`reading`] and later for `--json-scores`.
    pub document: serde_json::Value,
    /// The wall time this side measured around the request.
    pub client_latency_ms: u64,
}

/// Token and latency numbers for one request (spec §9.1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    /// Prompt size, when the server reported it.
    pub input_tokens: Option<u64>,
    /// Server-reported latency, when the server reports one.
    pub server_latency_ms: Option<u64>,
    /// Latency measured on this side of the socket.
    pub client_latency_ms: u64,
}

impl Usage {
    /// Usage with only the client-measured latency known.
    pub fn measured(client_latency_ms: u64) -> Usage {
        Usage {
            client_latency_ms,
            ..Usage::default()
        }
    }

    /// The latency to report: the server's when there is one, this side's
    /// otherwise (spec §9.1).
    pub fn latency_ms(&self) -> u64 {
        self.server_latency_ms.unwrap_or(self.client_latency_ms)
    }
}

/// The seam: an image plus a rubric in, raw answers out.
///
/// The oMLX HTTP adapter is the only production implementation; tests supply
/// fixed and adversarial answers. Nothing below this trait knows about HTTP.
pub trait Decider {
    /// Ask the model one frame's questions and return its answer document.
    fn decide(&self, request: &Request<'_>) -> Result<Response>;

    /// Confirm, before the first frame, that this decider can answer at all.
    ///
    /// A run over 400 frames that would fail identically 400 times is worse
    /// than a run that never started, so the preflight runs once, up front, and
    /// its failure stops the run as a fatal error (spec §10). A decider with
    /// nothing to check — every fake — is ready by default.
    fn preflight(&self) -> Result<()> {
        Ok(())
    }
}

/// The photographer's stop button, as a flag the run reads between files.
///
/// Cancellation is cooperative and lives outside the pipeline's types so a
/// terminal, a GUI and a test can all pull the same lever: the run looks at it
/// before each frame and never in the middle of one, so a cancelled run leaves
/// its rows whole (spec §6).
#[derive(Debug, Default)]
pub struct Cancel {
    cancelled: AtomicBool,
}

impl Cancel {
    /// A flag nothing has pulled yet.
    pub fn new() -> Cancel {
        Cancel::default()
    }

    /// Ask the run to stop before its next file. Safe to call from a signal
    /// handler and from another thread.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Whether a stop has been asked for.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// How a transient failure is retried (spec §10).
///
/// Three retries after the first attempt, waiting 0.5 s, 1 s and 2 s: a hiccup
/// costs one frame a few seconds instead of the shoot. A 4xx is never retried,
/// because a rejected rubric fails identically however many times it is asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryPolicy {
    /// The wait after each failed attempt that will be retried, in order.
    pub backoff: Vec<Duration>,
}

impl Default for RetryPolicy {
    fn default() -> RetryPolicy {
        RetryPolicy {
            backoff: vec![
                Duration::from_millis(500),
                Duration::from_secs(1),
                Duration::from_secs(2),
            ],
        }
    }
}

impl RetryPolicy {
    /// The same retries with no waiting at all: what a test wants when it
    /// counts attempts rather than feels them.
    pub fn without_waiting() -> RetryPolicy {
        RetryPolicy {
            backoff: vec![Duration::ZERO; RetryPolicy::default().backoff.len()],
        }
    }
}

/// An effective scoring run: which rubric, which alias, how to look at the
/// folder, how to prepare each frame, and where the report goes.
#[derive(Debug, Clone)]
pub struct Config {
    /// The questions.
    pub rubric: Rubric,
    /// The alias actually used: `--model`, or the rubric's own.
    pub model: String,
    /// How much picture to send.
    pub params: Params,
    /// Which files a run covers.
    pub scan: ScanOptions,
    /// The photographer's framing of the shoot, when they gave one.
    pub shoot_context: Option<String>,
    /// How hard to retry a transient failure. The spec's schedule by default;
    /// tests replace it so an attempt count costs no wall time.
    pub retry: RetryPolicy,
    /// Where the CSV and the raw answers are written.
    pub outputs: Outputs,
}

impl Config {
    /// Resolve the effective config.
    ///
    /// The model alias is part of the rubric's identity, so an explicit
    /// `--model` overrides the rubric's alias and changes the hash; leaving it
    /// out uses the alias the rubric was written for (spec §8.3, §11).
    pub fn new(
        rubric: Rubric,
        model: Option<String>,
        params: Params,
        scan: ScanOptions,
        shoot_context: Option<String>,
        outputs: Outputs,
    ) -> Config {
        let model = model.unwrap_or_else(|| rubric.model().to_string());
        Config {
            rubric,
            model,
            params,
            scan,
            shoot_context,
            retry: RetryPolicy::default(),
            outputs,
        }
    }

    /// The effective rubric hash: schema plus the alias in use.
    pub fn rubric_hash(&self) -> RubricHash {
        self.rubric.hash(&self.model)
    }
}

/// What a run did, frame by frame.
#[derive(Debug)]
pub struct Run {
    /// Which rubric and alias every frame was scored under.
    pub rubric_hash: RubricHash,
    /// The alias actually used.
    pub model: String,
    /// What the scan left out, and why.
    pub counts: ScanCounts,
    /// How many files the scan accepted. [`Run::frames`] holds the ones the run
    /// got to, which is fewer when it stopped or was interrupted early.
    pub total: usize,
    /// One entry per file the run got to, in ingest order. A run that stopped
    /// early has one entry for the frame that stopped it and nothing after.
    pub frames: Vec<Frame>,
    /// How the run ended, which is also what it exits as.
    pub ending: Ending,
}

impl Run {
    /// How many frames produced six readable answers.
    pub fn scored(&self) -> usize {
        self.frames
            .iter()
            .filter(|frame| matches!(&frame.outcome, Ok(reading) if reading.answers.is_ok()))
            .count()
    }

    /// How many frames did not, for any reason.
    pub fn failed(&self) -> usize {
        self.frames.len() - self.scored()
    }

    /// What the process exits with (spec §10).
    ///
    /// A pure function of the run: an interruption outranks a failed row
    /// (the run did not finish, it was stopped), and a run that stopped at a
    /// frame is fatal however many rows it had already written.
    pub fn exit_code(&self) -> ExitCode {
        match &self.ending {
            Ending::Interrupted => ExitCode::Interrupted,
            Ending::Aborted { .. } => ExitCode::Fatal,
            Ending::Completed if self.failed() > 0 => ExitCode::FailedRows,
            Ending::Completed => ExitCode::Ok,
        }
    }
}

/// How a run ended (spec §10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ending {
    /// Every file the scan accepted was judged.
    Completed,
    /// The run stopped at a frame, because its failure is one a retry cannot
    /// fix: a rejected rubric, a malformed body, or a server that never
    /// answered. The rows judged so far are still written.
    Aborted {
        /// The failure that stopped it, as the photographer reads it.
        reason: String,
    },
    /// The photographer cancelled, between files. The rows judged so far are
    /// still written, so nothing scored is lost.
    Interrupted,
}

/// What happened to one frame.
#[derive(Debug)]
pub struct Frame {
    /// The file it happened to.
    pub source: SourceFile,
    /// The reading, or the classified reason there is none. A document that
    /// arrived but could not be read is an `Ok` reading whose `answers` are an
    /// error, so the raw document survives for `--json-scores`.
    pub outcome: Result<Reading>,
}

/// Score a shoot folder: scan it, take every accepted frame through the model,
/// one request at a time, in ingest order, and write the report those frames
/// add up to.
///
/// A file that cannot be read or that carries no preview becomes a classified
/// failure and the run continues: one odd file must not cost the photographer
/// the other 399. The failures whose kind says a retry cannot help — a rejected
/// rubric, a malformed body, a server that never answered — stop the run
/// instead, and the rows judged so far are written either way. A cancellation
/// stops the run between files, and its rows are written too. Each frame
/// becomes its report row as it is produced, from the frame in hand rather than
/// from the finished run, so the rows can be flushed as they arrive.
pub fn run(shoot: &Path, config: &Config, decider: &dyn Decider, cancel: &Cancel) -> Result<Run> {
    let scan = ingest::scan(shoot, &config.scan)?;
    // Before the first frame, and before any report is written: a server that
    // cannot answer must cost seconds, not a shoot.
    decider.preflight()?;
    let rubric_hash = config.rubric_hash();
    let mut frames = Vec::with_capacity(scan.files.len());
    let mut rows = Vec::with_capacity(scan.files.len());
    let mut ending = Ending::Completed;
    for source in &scan.files {
        if cancel.is_cancelled() {
            ending = Ending::Interrupted;
            break;
        }
        let frame = judge(source, config, decider);
        let stops_run = frame
            .outcome
            .as_ref()
            .err()
            .filter(|error| error.recovery().aborts_run())
            .map(ToString::to_string);
        rows.push(Row::of(&frame, &rubric_hash, &config.model));
        frames.push(frame);
        if let Some(reason) = stops_run {
            ending = Ending::Aborted { reason };
            break;
        }
    }
    report::write(&rows, &config.outputs)?;
    Ok(Run {
        rubric_hash,
        model: config.model.clone(),
        counts: scan.counts,
        total: scan.files.len(),
        frames,
        ending,
    })
}

/// One file, all the way to a reading: prepare, ask, read.
///
/// This is the only place a frame becomes a request, which is what makes the
/// exact bytes, dimensions and questions handed to [`Decider`] assertable.
fn judge(source: &SourceFile, config: &Config, decider: &dyn Decider) -> Frame {
    let outcome = prepare(source, &config.params).and_then(|preview| {
        let request = Request {
            model: &config.model,
            rubric: &config.rubric,
            state: State::for_frame(display_name(source), config.shoot_context.clone()),
            preview: &preview,
        };
        decide(decider, &request, &config.retry)
            .map(|response| reading::read(&config.rubric, response))
    });
    Frame {
        source: source.clone(),
        outcome,
    }
}

/// Ask the model one frame's question, retrying only the transient failures
/// (spec §10): a connection failure, a timeout and a 5xx are worth another
/// attempt, with the policy's backoff between them. The failure that ends the
/// attempts names how many were spent, so the row explains itself.
fn decide(decider: &dyn Decider, request: &Request<'_>, retry: &RetryPolicy) -> Result<Response> {
    let mut waited = 0;
    loop {
        match decider.decide(request) {
            Ok(response) => return Ok(response),
            Err(error) => {
                if !error.recovery().is_retried() || waited == retry.backoff.len() {
                    return Err(error.with_attempts(waited + 1));
                }
                std::thread::sleep(retry.backoff[waited]);
                waited += 1;
            }
        }
    }
}

/// Extract, orient, downscale and encode one frame: everything a request
/// carries except the questions.
///
/// Shared by a scoring run and by `--dry-run`, so the previews a photographer
/// eyeballs are the bytes a run would actually send.
pub(crate) fn prepare(source: &SourceFile, params: &Params) -> Result<Encoded> {
    let embedded = preview::extract(&source.path)?;
    preprocess::render(&embedded, params)
}

/// The name a frame is reported under: its path inside the shoot folder, so
/// subfolder runs stay unambiguous and the CSV's `file` column is the same
/// string the model was told.
fn display_name(source: &SourceFile) -> String {
    source.relative.to_string_lossy().into_owned()
}
