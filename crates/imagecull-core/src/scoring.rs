//! One frame in, one answer out: the scoring entry point, and the one seam
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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, ErrorKind, ExitCode, Result};
use crate::ingest::{self, ScanCounts, ScanOptions, SourceFile};
use crate::preprocess::{self, Encoded, Params};
use crate::preview;
use crate::progress::{
    Event, Finished, FrameEvent, Judgement, PreviewShape, Reporter, Started, Summary, Warning,
};
use crate::reading::{self, Answers, Reading};
use crate::rubric::{Builtin, Rubric, RubricHash, hex};
use crate::sidecar::{self, Failure, Outputs, Properties, Row};

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
///
/// Recorded in the ledger as they were read, so a resumed frame costs nothing to
/// report and a duplicate row reports what its request actually cost.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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
/// folder, how to prepare each frame, and what to do with a sidecar.
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
    /// Where the raw answers are written, when they are.
    pub outputs: Outputs,
    /// Whether a frame whose sidecar already carries a rating is rated again
    /// (`--force`).
    pub force: bool,
    /// Whether a sidecar another application owns may be written into
    /// (`--allow-foreign`).
    pub allow_foreign: bool,
    /// Whether an identical preview may reuse an earlier frame's outcome
    /// (`--no-dedupe` turns it off).
    pub dedupe: bool,
    /// The server this run talks to, recorded for provenance. `None` when the
    /// decider has no endpoint of its own, as every test's fake has.
    pub endpoint: Option<String>,
    /// The config file this run's values came from, when one did. Provenance for
    /// the summary; `None` when the flags and the built-in defaults were the
    /// whole story.
    pub source: Option<PathBuf>,
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
            force: false,
            allow_foreign: false,
            dedupe: true,
            endpoint: None,
            source: None,
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
    /// The built-in rubric a name selected, when one did.
    pub builtin: Option<Builtin>,
    /// Which rubric and alias every frame was scored under.
    pub rubric_hash: RubricHash,
    /// The alias actually used.
    pub model: String,
    /// What the scan left out, and why.
    pub counts: ScanCounts,
    /// How many files the scan accepted. [`Run::frames`] holds the ones the run
    /// got to, which is fewer when it stopped or was interrupted early.
    pub total: usize,
    /// One entry per file the run got to, in ingest order.
    pub frames: Vec<Frame>,
    /// How many frames reused an identical preview's outcome instead of costing
    /// a request.
    pub duplicates: usize,
    /// From the scan to the last write: the wall time the summary reports.
    pub elapsed: Duration,
    /// How the run ended, which is also what it exits as.
    pub ending: Ending,
    /// The config file the run's values came from, when one did.
    pub source: Option<PathBuf>,
}

impl Run {
    /// How many frames this run produced a rating for.
    pub fn scored(&self) -> usize {
        self.frames
            .iter()
            .filter(|frame| frame.answers().is_some())
            .count()
    }

    /// How many frames carried a rating already and were left alone.
    pub fn kept(&self) -> usize {
        self.frames
            .iter()
            .filter(|frame| frame.kept().is_some())
            .count()
    }

    /// How many frames did not produce a rating this run could stand behind.
    pub fn failed(&self) -> usize {
        self.frames
            .iter()
            .filter(|frame| frame.failure().is_some())
            .count()
    }

    /// The run as a summary: the numbers a photographer judges it by, which is
    /// also what the reporter receives as the run finishes.
    pub fn summary(&self) -> Summary {
        Summary::of(self)
    }

    /// What the process exits with (spec §10).
    ///
    /// A pure function of the run: an interruption outranks a failed row
    /// (the run did not finish, it was stopped), and a run that stopped at a
    /// frame is fatal however many ratings it had already written.
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
    /// The answer, or the rating the sidecar already carried.
    pub outcome: Outcome,
    /// What became of the sidecar, when the run got that far.
    pub persisted: Persisted,
}

/// The two ways a frame leaves the model: asked, or left alone.
#[derive(Debug)]
pub enum Outcome {
    /// The run asked, and this is what came back — including a document that
    /// arrived but could not be read, whose failure is classified here and whose
    /// document survives for `--json-scores`.
    Asked(Result<Reading>),
    /// The frame's sidecar already carried a rating, so the run spent nothing on
    /// it and left it exactly as it found it.
    Kept {
        /// The rating that was already there.
        rating: u8,
    },
    /// The run refused the frame before asking: its sidecar could not be read or
    /// written into, or belonged to another application.
    Refused(Error),
}

/// What became of a frame's sidecar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Persisted {
    /// Nothing was written: the frame had no answer, or it was kept.
    NotAttempted,
    /// The sidecar now holds this run's rating.
    Written,
    /// The write failed, which makes the frame a failure: the rating's only home
    /// is the document beside the frame.
    Failed(Failure),
}

impl Frame {
    /// The model's reading, when this frame reached the model and came back.
    pub fn reading(&self) -> Option<&Reading> {
        match &self.outcome {
            Outcome::Asked(Ok(reading)) => Some(reading),
            _ => None,
        }
    }

    /// The answer, when this frame has one the run can stand behind.
    pub fn answers(&self) -> Option<&Answers> {
        match (&self.outcome, &self.persisted) {
            // A rating whose sidecar could not be written is nowhere: the run
            // scored nothing it can stand behind, so it does not claim to have.
            (Outcome::Asked(_), Persisted::Failed(_)) => None,
            (Outcome::Asked(Ok(reading)), _) => reading.answers.as_ref().ok(),
            (Outcome::Kept { .. }, _) | (Outcome::Asked(Err(_)), _) | (Outcome::Refused(_), _) => {
                None
            }
        }
    }

    /// The rating that was already in the sidecar, when the run left it alone.
    pub fn kept(&self) -> Option<u8> {
        match self.outcome {
            Outcome::Kept { rating } => Some(rating),
            Outcome::Asked(_) | Outcome::Refused(_) => None,
        }
    }

    /// The failure a row's status carries, when the frame failed.
    ///
    /// A sidecar that could not be written outranks a model that did not answer:
    /// the frame's rating is nowhere, which is the fact worth reporting.
    pub fn failure(&self) -> Option<Failure> {
        if let Persisted::Failed(failure) = &self.persisted {
            return Some(failure.clone());
        }
        match &self.outcome {
            Outcome::Kept { .. } => None,
            Outcome::Refused(error) | Outcome::Asked(Err(error)) => Some(Failure::of(error)),
            Outcome::Asked(Ok(reading)) => reading.answers.as_ref().err().map(Failure::of),
        }
    }

    /// What the request that produced this outcome cost, when one was recorded.
    pub fn latency_ms(&self) -> Option<u64> {
        match &self.outcome {
            Outcome::Asked(Ok(reading)) => Some(reading.usage.latency_ms()),
            _ => None,
        }
    }
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
/// stops the run between files, and its rows are written too.
///
/// Every frame becomes a ledger record as it is produced, and its report row is
/// rendered from that record and appended — so the CSV on disk is always the
/// ledger's own account of itself up to the last judged frame, and a frame the
/// ledger already holds is skipped rather than asked again (spec §9.2). An
/// identical preview reuses the earlier frame's outcome and still gets its own
/// row.
///
/// `reporter` hears about every one of those frames as it goes, in ingest order,
/// and about the summary they add up to: this is the run's whole account of
/// itself, and the reason the core needs no terminal (spec §52–53).
pub fn run(
    shoot: &Path,
    config: &Config,
    decider: &dyn Decider,
    sidecars: &dyn sidecar::Writer,
    reporter: &dyn Reporter,
    cancel: &Cancel,
) -> Result<Run> {
    let began = Instant::now();
    let scan = ingest::scan(shoot, &config.scan)?;
    let rubric_hash = config.rubric_hash();
    // Before the first frame and before any request: the front end can say how
    // much work there is while the preflight is still in flight.
    reporter.event(Event::Started(Started {
        total: scan.files.len(),
        builtin: config.rubric.builtin(),
        rubric_hash: rubric_hash.clone(),
        model: config.model.clone(),
    }));
    // Before the first frame: a server that cannot answer must cost seconds,
    // not a shoot.
    decider.preflight()?;
    let total = scan.files.len();
    let mut account = Account::new(reporter, &rubric_hash, &config.model, total, began);
    // Dedupe's memory: one run's answers, keyed by the question that produced
    // them. It dies with the process, which is where bursts live anyway.
    let mut reused: HashMap<String, Reused> = HashMap::new();
    let mut ending = Ending::Completed;
    for (position, source) in scan.files.iter().enumerate() {
        if cancel.is_cancelled() {
            ending = Ending::Interrupted;
            break;
        }
        let path = sidecar::path_for(&source.path);
        let existing = match read_sidecar(&path) {
            Ok(existing) => existing,
            Err(error) => {
                // A sidecar nobody can read is a frame this run cannot write, and
                // no reason to spend a request on it.
                if let Some(reason) = account.refused(position + 1, source, error) {
                    ending = Ending::Aborted { reason };
                    break;
                }
                continue;
            }
        };
        // The frame already carries a rating a photographer would recognise: the
        // run leaves it, and its money, alone.
        if let Some(rating) = existing.as_deref().and_then(sidecar::existing_rating)
            && !config.force
        {
            account.kept(position + 1, source, rating);
            continue;
        }
        // A sidecar nothing can be written into is a frame this run cannot
        // record, and asking the model anyway would buy an answer with nowhere to
        // go.
        if existing
            .as_deref()
            .is_some_and(|document| !sidecar::can_extend(document))
        {
            let error = Error::new(
                ErrorKind::XmpUnreadable,
                format!(
                    "{} has no rdf:Description this tool can write into, so it was left alone",
                    path.display()
                ),
            );
            if let Some(reason) = account.refused(position + 1, source, error) {
                ending = Ending::Aborted { reason };
                break;
            }
            continue;
        }
        // A sidecar another application owns is not ours to write into, whatever
        // the rating: darktable has its own copy of this frame's story.
        if existing.as_deref().is_some_and(sidecar::is_foreign) && !config.allow_foreign {
            let error = Error::new(
                ErrorKind::XmpForeign,
                format!(
                    "{} belongs to another application, so it was left alone; pass \
                     --allow-foreign to write into it",
                    path.display()
                ),
            );
            if let Some(reason) = account.refused(position + 1, source, error) {
                ending = Ending::Aborted { reason };
                break;
            }
            continue;
        }

        let mut judged = judge(source, config, decider, &rubric_hash, &mut reused);
        // The sidecar is written before the frame line is reported, so the
        // terminal never claims a rating that is not yet on disk.
        persist(
            &mut judged.frame,
            existing.as_deref(),
            config,
            &rubric_hash,
            sidecars,
        );
        let stops_run = judged.stops_run();
        account.judged(position + 1, judged);
        if let Some(reason) = stops_run {
            ending = Ending::Aborted { reason };
            break;
        }
    }
    // A run that rated nothing because everything was already rated looks
    // identical to a broken one unless it says so.
    if !account.frames.is_empty() && account.frames.iter().all(|frame| frame.kept().is_some()) {
        reporter.event(Event::Warning(Warning {
            message: "every frame already had a rating, so this run scored nothing: pass \
                      --force to rate them again"
                .to_string(),
        }));
    }
    sidecar::write_json(&account.rows, &config.outputs)?;
    let Account {
        frames, duplicates, ..
    } = account;
    let run = Run {
        builtin: config.rubric.builtin(),
        rubric_hash,
        model: config.model.clone(),
        counts: scan.counts,
        total,
        frames,
        duplicates,
        elapsed: began.elapsed(),
        ending,
        source: config.source.clone(),
    };
    // The last thing a reporter hears: the run's own account of itself.
    reporter.event(Event::Finished(Finished {
        summary: run.summary(),
    }));
    Ok(run)
}

/// The sidecar as it is before this run touches it: `None` when there is none, a
/// classified refusal when there is one this run cannot read.
fn read_sidecar(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Error::new(
            ErrorKind::XmpUnreadable,
            format!("cannot read {}: {error}", path.display()),
        )),
    }
}

/// Write this frame's rating into its sidecar, and record how that went.
///
/// A frame with no answer is not touched at all: "no answer" must never reach
/// the disk as a rating of zero or an empty file.
fn persist(
    frame: &mut Frame,
    existing: Option<&str>,
    config: &Config,
    rubric_hash: &RubricHash,
    writer: &dyn sidecar::Writer,
) {
    let Some(answers) = frame.answers().cloned() else {
        return;
    };
    let properties = Properties::now(
        answers.stars,
        answers.confidence,
        &config.model,
        rubric_hash.short(),
    );
    let document = match sidecar::render(existing, &properties) {
        Ok(document) => document,
        Err(error) => {
            frame.persisted = Persisted::Failed(Failure::of(&error));
            return;
        }
    };
    frame.persisted = match writer.write(&frame.source.path, &document, existing) {
        Ok(_) => Persisted::Written,
        Err(error) => Persisted::Failed(Failure::of(&error)),
    };
}

/// The run's account of itself while it runs: what the reporter hears, the rows
/// `--json-scores` is written from, and the frames the summary adds up.
struct Account<'a> {
    reporter: &'a dyn Reporter,
    rubric_hash: &'a RubricHash,
    model: &'a str,
    total: usize,
    began: Instant,
    frames: Vec<Frame>,
    rows: Vec<Row>,
    duplicates: usize,
}

impl<'a> Account<'a> {
    fn new(
        reporter: &'a dyn Reporter,
        rubric_hash: &'a RubricHash,
        model: &'a str,
        total: usize,
        began: Instant,
    ) -> Account<'a> {
        Account {
            reporter,
            rubric_hash,
            model,
            total,
            began,
            frames: Vec::with_capacity(total),
            rows: Vec::with_capacity(total),
            duplicates: 0,
        }
    }

    /// A frame whose sidecar already carried a rating: reported and counted, and
    /// otherwise untouched.
    fn kept(&mut self, index: usize, source: &SourceFile, rating: u8) {
        let frame = Frame {
            source: source.clone(),
            outcome: Outcome::Kept { rating },
            persisted: Persisted::NotAttempted,
        };
        self.record(index, frame, None, None);
    }

    /// A frame the run refused before asking: an unreadable sidecar, a document
    /// nothing can be written into, another application's file.
    ///
    /// Returns the reason the run must stop, when the refusal is one that would
    /// repeat for every frame after it — one door, so the three refusals cannot
    /// drift apart about whether they stop a run.
    fn refused(&mut self, index: usize, source: &SourceFile, error: Error) -> Option<String> {
        let judged = Judged::refused(source, error);
        let stops_run = judged.stops_run();
        self.judged(index, judged);
        stops_run
    }

    /// A frame this run judged, and what became of its sidecar.
    fn judged(&mut self, index: usize, judged: Judged) {
        if judged.duplicate_of.is_some() {
            self.duplicates += 1;
        }
        let Judged {
            frame,
            preview,
            duplicate_of,
        } = judged;
        self.record(index, frame, preview, duplicate_of);
    }

    /// One frame's account: the event a reporter hears, the row the raw answers
    /// are written from, and the frame the summary adds up.
    fn record(
        &mut self,
        index: usize,
        frame: Frame,
        preview: Option<PreviewShape>,
        duplicate_of: Option<String>,
    ) {
        self.reporter.event(Event::Frame(frame_event(
            index,
            self.total,
            &frame,
            preview,
            duplicate_of.as_deref(),
            self.began.elapsed(),
        )));
        let reading = frame.reading();
        let row = Row {
            source: frame.source.clone(),
            answers: frame.answers().cloned(),
            usage: reading.map(|reading| reading.usage),
            document: reading.map(|reading| reading.document.clone()),
            failure: frame.failure(),
            duplicate_of,
            rubric_hash: self.rubric_hash.short().to_string(),
            model: self.model.to_string(),
        };
        self.rows.push(row);
        self.frames.push(frame);
    }
}

/// One frame as the front end sees it: where it sits in the run, what came back,
/// and what the request cost.
fn frame_event(
    index: usize,
    total: usize,
    frame: &Frame,
    preview: Option<PreviewShape>,
    duplicate_of: Option<&str>,
    elapsed: Duration,
) -> FrameEvent {
    FrameEvent {
        index,
        total,
        filename: display_name(&frame.source),
        judgement: judgement(frame),
        latency_ms: frame.latency_ms(),
        eta: estimate_remaining(index, total, elapsed),
        preview,
        duplicate_of: duplicate_of.map(str::to_string),
    }
}

/// How long the rest of the run will take, at the pace it has kept so far.
///
/// The last frame has nothing left to estimate, and a run with no elapsed time
/// behind it has no pace to project: both answer `None` rather than a made-up
/// zero, so a front end can leave the estimate off the line.
fn estimate_remaining(done: usize, total: usize, elapsed: Duration) -> Option<Duration> {
    let remaining = total.checked_sub(done)?;
    if remaining == 0 || done == 0 || elapsed.is_zero() {
        return None;
    }
    let pace = elapsed.as_secs_f64() / done as f64;
    Some(Duration::from_secs_f64(pace * remaining as f64))
}

/// A frame's outcome as the front end sees it.
fn judgement(frame: &Frame) -> Judgement {
    if let Some(rating) = frame.kept() {
        return Judgement::Skipped { rating };
    }
    match frame.failure() {
        Some(failure) => Judgement::Failed(failure),
        None => match frame.answers() {
            Some(answers) => Judgement::Answered(answers.clone()),
            // Unreachable: a frame with neither answer, rating nor failure
            // cannot be built. Reporting it as a failure beats panicking in a run.
            None => Judgement::Failed(Failure {
                kind: ErrorKind::BadResponse,
                message: "the frame has no answer and no reason".to_string(),
            }),
        },
    }
}

/// One frame, judged: the run's frame, the shape of the bytes that were sent, and
/// the earlier frame whose answer this one reused.
struct Judged {
    frame: Frame,
    preview: Option<PreviewShape>,
    duplicate_of: Option<String>,
}

impl Judged {
    /// A frame the run refused before asking: an unreadable sidecar, a sidecar
    /// another application owns.
    fn refused(source: &SourceFile, error: Error) -> Judged {
        Judged {
            frame: Frame {
                source: source.clone(),
                outcome: Outcome::Refused(error),
                persisted: Persisted::NotAttempted,
            },
            preview: None,
            duplicate_of: None,
        }
    }

    /// The failure that stops the run, when this frame has one that will fail
    /// identically for every frame after it.
    fn stops_run(&self) -> Option<String> {
        self.frame
            .failure()
            .filter(|failure| failure.kind.recovery().aborts_run())
            .map(|failure| failure.message)
    }
}

/// An answer one run already paid for, and the frame it paid for it on.
struct Reused {
    file: String,
    answers: Answers,
    usage: Usage,
    document: serde_json::Value,
}

/// One file, all the way to a frame: prepare, reuse or ask, read.
///
/// This is the only place a frame becomes a request, which is what makes the
/// exact bytes, dimensions and questions handed to [`Decider`] assertable, and
/// the only place an outcome may be reused: the key covers the bytes that would
/// go out, the state they would go out in, and the rubric that would read them.
fn judge(
    source: &SourceFile,
    config: &Config,
    decider: &dyn Decider,
    rubric_hash: &RubricHash,
    reused: &mut HashMap<String, Reused>,
) -> Judged {
    let encoded = match prepare(source, &config.params) {
        Ok(encoded) => encoded,
        Err(error) => {
            return Judged {
                frame: Frame {
                    source: source.clone(),
                    outcome: Outcome::Asked(Err(error)),
                    persisted: Persisted::NotAttempted,
                },
                preview: None,
                duplicate_of: None,
            };
        }
    };
    let preview_sha256 = preview_sha256(&encoded);
    let preview = PreviewShape {
        width: encoded.width,
        height: encoded.height,
        jpeg_bytes: encoded.jpeg.len(),
    };
    let name = display_name(source);
    let state = State::for_frame(name.clone(), config.shoot_context.clone());
    let dedupe_key = config
        .dedupe
        .then(|| dedupe_key(&preview_sha256, &state, rubric_hash, &config.model));
    // A frame is never a duplicate of itself: a file whose bytes changed is
    // scored again even when it renders to the same preview.
    if let Some(reused) = dedupe_key
        .as_deref()
        .and_then(|key| reused.get(key))
        .filter(|earlier| earlier.file != name)
    {
        let frame = Frame {
            source: source.clone(),
            outcome: Outcome::Asked(Ok(Reading {
                answers: Ok(reused.answers.clone()),
                usage: reused.usage,
                document: reused.document.clone(),
            })),
            persisted: Persisted::NotAttempted,
        };
        return Judged {
            frame,
            preview: Some(preview),
            duplicate_of: Some(reused.file.clone()),
        };
    }
    let request = Request {
        model: &config.model,
        rubric: &config.rubric,
        state,
        preview: &encoded,
    };
    let outcome = decide(decider, &request, &config.retry)
        .map(|response| reading::read(&config.rubric, response));
    // Only an answer is worth reusing: a timeout is not an answer, so a burst
    // partner gets its own attempt.
    if let Some(key) = dedupe_key
        && let Ok(reading) = &outcome
        && let Ok(answers) = &reading.answers
    {
        reused.insert(
            key,
            Reused {
                file: name,
                answers: answers.clone(),
                usage: reading.usage,
                document: reading.document.clone(),
            },
        );
    }
    Judged {
        frame: Frame {
            source: source.clone(),
            outcome: Outcome::Asked(outcome),
            persisted: Persisted::NotAttempted,
        },
        preview: Some(preview),
        duplicate_of: None,
    }
}

/// The SHA-256 of the bytes actually sent, as 64 lowercase hex characters: what
/// the ledger records, and what a burst's identical previews have in common.
fn preview_sha256(preview: &Encoded) -> String {
    let mut digest = Sha256::new();
    digest.update(&preview.jpeg);
    hex(digest.finalize())
}

/// The key an identical preview is reused under: the bytes sent, the state the
/// frame is judged in, and the rubric's identity (ADR 0002).
///
/// The filename is deliberately absent. It is part of the state the model reads,
/// but it is the photographer's label and never something the picture shows, and
/// two frames of one burst always have different names: a key that covered it
/// could never fire, which is the opposite of what a dedupe key is for.
fn dedupe_key(
    preview_sha256: &str,
    state: &State,
    rubric_hash: &RubricHash,
    model: &str,
) -> String {
    let mut digest = Sha256::new();
    // Length-prefixed, so no value can forge a field boundary.
    let mut field = |value: &str| {
        digest.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
        digest.update(value.as_bytes());
    };
    field("imagecull-dedupe-v1");
    field(preview_sha256);
    field(&state.task);
    match &state.shoot_context {
        Some(context) => {
            field("with-context");
            field(context);
        }
        None => field("without-context"),
    }
    field(rubric_hash.as_str());
    field(model);
    hex(digest.finalize())
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
pub(crate) fn display_name(source: &SourceFile) -> String {
    source.relative.to_string_lossy().into_owned()
}
