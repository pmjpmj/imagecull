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

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{ExitCode, Result};
use crate::ingest::{self, ScanCounts, ScanOptions, SourceFile};
use crate::ledger::{
    self, FileIdentity, Header, Ledger, LedgerMode, Provenance, Record, ResumeKey,
};
use crate::preprocess::{self, Encoded, Params};
use crate::preview;
use crate::progress::{
    Event, Finished, FrameEvent, Judgement, PreviewShape, Reporter, Started, Summary, Warning,
};
use crate::reading::{self, Answers, Reading};
use crate::report::{self, Failure, Outputs, Row};
use crate::rubric::{Rubric, RubricHash, hex};

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

/// How many rows accumulate between CSV rewrites: the spec's 25 (spec §9.3).
///
/// A run rewrites the CSV from what it has judged, so a killed run leaves a
/// readable spreadsheet behind instead of nothing.
pub const DEFAULT_CSV_REWRITE_EVERY: usize = 25;

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
    /// What to do with a ledger that is already beside the CSV (spec §9.3).
    pub ledger: LedgerMode,
    /// Whether an identical preview may reuse an earlier frame's outcome
    /// (spec §9.3); `--no-dedupe` turns it off.
    pub dedupe: bool,
    /// How many rows accumulate between CSV rewrites.
    pub csv_rewrite_every: usize,
    /// The server this run talks to, recorded in the ledger's header for
    /// provenance. `None` when the decider has no endpoint of its own, as every
    /// test's fake has.
    pub endpoint: Option<String>,
    /// The config file this run's values came from, when one did. Provenance for
    /// the summary, the way `endpoint` is provenance for the ledger's header;
    /// `None` when the flags and the built-in defaults were the whole story.
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
            ledger: LedgerMode::default(),
            dedupe: true,
            csv_rewrite_every: DEFAULT_CSV_REWRITE_EVERY,
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
    /// How many frames the resumed ledger already held: judged, never re-asked.
    pub resumed: usize,
    /// How many frames reused an identical preview's outcome instead of costing
    /// a request.
    pub duplicates: usize,
    /// From the scan to the last write: the wall time the summary reports.
    pub elapsed: Duration,
    /// How the run ended, which is also what it exits as.
    pub ending: Ending,
    /// The config file the run's values came from, when one did: the summary
    /// prints it beside the rubric hash so the numbers are attributable.
    pub source: Option<PathBuf>,
}

impl Run {
    /// How many frames produced six readable answers.
    pub fn scored(&self) -> usize {
        self.frames
            .iter()
            .filter(|frame| frame.answers().is_some())
            .count()
    }

    /// How many frames did not, for any reason.
    pub fn failed(&self) -> usize {
        self.frames.len() - self.scored()
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

impl Frame {
    /// The six answers, or the failure the row's `status` and `error` carry.
    ///
    /// One of the two always applies: a document that arrived but could not be
    /// read is a [`Failure`] too, exactly as the CSV records it. This is the
    /// only place that decision is made, so a progress line, a summary and a
    /// row cannot disagree about what happened to a frame.
    pub fn answers_or_failure(&self) -> std::result::Result<&Answers, Failure> {
        match &self.outcome {
            Ok(reading) => reading.answers.as_ref().map_err(Failure::of),
            Err(error) => Err(Failure::of(error)),
        }
    }

    /// The six answers, when this frame has them.
    pub fn answers(&self) -> Option<&Answers> {
        self.answers_or_failure().ok()
    }

    /// What the request that produced this outcome cost, when one was recorded.
    pub fn latency_ms(&self) -> Option<u64> {
        self.outcome
            .as_ref()
            .ok()
            .map(|reading| reading.usage.latency_ms())
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
/// rendered from that record — so the CSV is a view of the ledger, the CSV can
/// be rewritten mid-run, and a frame the ledger already holds is skipped rather
/// than asked again (spec §9.3). An identical preview reuses the earlier frame's
/// outcome and still gets its own row.
///
/// `reporter` hears about every one of those frames as it goes, in ingest order,
/// and about the summary they add up to: this is the run's whole account of
/// itself, and the reason the core needs no terminal (spec §52–53).
pub fn run(
    shoot: &Path,
    config: &Config,
    decider: &dyn Decider,
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
        rubric_hash: rubric_hash.clone(),
        model: config.model.clone(),
    }));
    // Before the first frame, and before any report or ledger is written: a
    // server that cannot answer must cost seconds, not a shoot.
    decider.preflight()?;
    let header = Header::new(&rubric_hash, &config.model, config.endpoint.as_deref());
    let mut ledger = Ledger::open(&config.outputs.csv, header, config.ledger)?;
    if ledger.truncated_bytes() > 0 {
        reporter.event(Event::Warning(Warning {
            message: format!(
                "{} ends mid-record, so the run it belonged to was killed: {} bytes of a \
                 half-written record were discarded and those frames are scored again",
                ledger::path_for(&config.outputs.csv).display(),
                ledger.truncated_bytes(),
            ),
        }));
    }
    if ledger.discarded_frames() > 0 {
        reporter.event(Event::Warning(Warning {
            message: format!(
                "{} held {}, and a plain score starts the ledger over: they are scored \
                 again — pass --resume to keep them",
                ledger::path_for(&config.outputs.csv).display(),
                recorded_frames(ledger.discarded_frames()),
            ),
        }));
    }
    let total = scan.files.len();
    // The run's account of itself as it goes: the events a reporter hears, the
    // rows the CSV is written from, and the frames the summary adds up.
    let mut account = Account::new(reporter, &rubric_hash, &config.model, total, began);
    let mut ending = Ending::Completed;
    for (position, source) in scan.files.iter().enumerate() {
        if cancel.is_cancelled() {
            ending = Ending::Interrupted;
            break;
        }
        let identity = FileIdentity::of(&source.path).ok();
        let recorded = identity
            .map(|identity| ResumeKey::new(display_name(source), identity))
            .and_then(|key| ledger.recorded(&key));
        if let Some(record) = recorded {
            // Resumed frames are reported like any other: the progress line
            // counts them and the summary counts them, so an interrupted run
            // picks up where it stopped and says so.
            account.resumed(position + 1, source, record);
            continue;
        }
        let judged = judge(source, config, decider, &rubric_hash, identity, &ledger);
        // The frame's row is written first either way, so a run that stops says
        // which frame stopped it and why.
        let stops_run = judged
            .frame
            .outcome
            .as_ref()
            .err()
            .filter(|error| error.recovery().aborts_run())
            .map(ToString::to_string);
        account.judged(position + 1, judged, &mut ledger)?;
        if config.csv_rewrite_every > 0
            && account.rows.len().is_multiple_of(config.csv_rewrite_every)
        {
            report::write(&account.rows, &config.outputs)?;
        }
        if let Some(reason) = stops_run {
            ending = Ending::Aborted { reason };
            break;
        }
    }
    report::write(&account.rows, &config.outputs)?;
    let Account {
        frames,
        resumed,
        duplicates,
        ..
    } = account;
    let run = Run {
        rubric_hash,
        model: config.model.clone(),
        counts: scan.counts,
        total,
        frames,
        resumed,
        duplicates,
        elapsed: began.elapsed(),
        ending,
        source: config.source.clone(),
    };
    // The last thing a reporter hears: the run's own account of itself, which a
    // front end renders as the summary and a script reads for the outcome.
    reporter.event(Event::Finished(Finished {
        summary: run.summary(),
    }));
    Ok(run)
}

/// `count` recorded frames, for a warning that has to read like a sentence.
fn recorded_frames(count: usize) -> String {
    if count == 1 {
        "1 recorded frame".to_string()
    } else {
        format!("{count} recorded frames")
    }
}

/// The run's account of itself while it runs: what the reporter hears, the rows
/// the CSV is written from, and the frames the summary adds up.
///
/// A resumed frame and a freshly judged one reach it through two doors and are
/// recorded identically from there, so a resumed row and a fresh one cannot
/// drift apart — which is what makes an interrupted-and-resumed run write the
/// same CSV as an uninterrupted one.
struct Account<'a> {
    reporter: &'a dyn Reporter,
    rubric_hash: &'a RubricHash,
    model: &'a str,
    total: usize,
    began: Instant,
    frames: Vec<Frame>,
    rows: Vec<Row>,
    resumed: usize,
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
            resumed: 0,
            duplicates: 0,
        }
    }

    /// A frame the ledger already held: reported and counted like any other, but
    /// never asked again.
    fn resumed(&mut self, index: usize, source: &SourceFile, record: &Record) {
        self.record(index, record.frame(source.clone()), record, None);
        self.resumed += 1;
    }

    /// A frame this run prepared and judged: recorded in the ledger it will be
    /// resumed from, reported, and turned into the row it will be read as.
    fn judged(&mut self, index: usize, judged: Judged, ledger: &mut Ledger) -> Result<()> {
        if judged.record.duplicate_of.is_some() {
            self.duplicates += 1;
        }
        self.record(index, judged.frame, &judged.record, judged.preview);
        ledger.append(judged.record)
    }

    /// One frame's account: the progress event, its CSV row, and the frame the
    /// summary adds up.
    fn record(
        &mut self,
        index: usize,
        frame: Frame,
        record: &Record,
        preview: Option<PreviewShape>,
    ) {
        self.reporter.event(Event::Frame(frame_event(
            index,
            self.total,
            &frame,
            preview,
            record.duplicate_of.as_deref(),
            self.began.elapsed(),
        )));
        self.rows
            .push(record.row(frame.source.clone(), self.rubric_hash, self.model));
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

/// A frame's outcome as the front end sees it: the answers, or the failure the
/// row's `status` and `error` columns will carry.
fn judgement(frame: &Frame) -> Judgement {
    match frame.answers_or_failure() {
        Ok(answers) => Judgement::Answered(answers.clone()),
        Err(failure) => Judgement::Failed(failure),
    }
}

/// One frame, judged: the run's frame, the ledger's record of it, and the shape
/// of the bytes that were sent.
struct Judged {
    frame: Frame,
    record: Record,
    preview: Option<PreviewShape>,
}

/// One file, all the way to a record: prepare, reuse or ask, read.
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
    identity: Option<FileIdentity>,
    ledger: &Ledger,
) -> Judged {
    let encoded = match prepare(source, &config.params) {
        Ok(encoded) => encoded,
        Err(error) => {
            let frame = Frame {
                source: source.clone(),
                outcome: Err(error),
            };
            let record = Record::of(&frame, identity, Provenance::default());
            return Judged {
                frame,
                record,
                preview: None,
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
    // A frame is never a duplicate of itself: a file whose mtime changed is
    // scored again even when the bytes it renders to did not (spec §32).
    if let Some(reused) = dedupe_key
        .as_deref()
        .and_then(|key| ledger.reusable(key))
        .filter(|reused| reused.file != name)
    {
        let frame = reused.frame(source.clone());
        let record = Record::of(
            &frame,
            identity,
            Provenance {
                preview_sha256: Some(&preview_sha256),
                dedupe_key: dedupe_key.as_deref(),
                duplicate_of: Some(&reused.file),
            },
        );
        return Judged {
            frame,
            record,
            preview: Some(preview),
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
    let frame = Frame {
        source: source.clone(),
        outcome,
    };
    let record = Record::of(
        &frame,
        identity,
        Provenance {
            preview_sha256: Some(&preview_sha256),
            dedupe_key: dedupe_key.as_deref(),
            ..Provenance::default()
        },
    );
    Judged {
        frame,
        record,
        preview: Some(preview),
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
