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

use crate::error::Result;
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
    /// One entry per accepted file, in ingest order.
    pub frames: Vec<Frame>,
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
/// the other 399. Each frame becomes its report row as it is produced, from the
/// frame in hand rather than from the finished run, so the rows can be flushed
/// as they arrive.
pub fn run(shoot: &Path, config: &Config, decider: &dyn Decider) -> Result<Run> {
    let scan = ingest::scan(shoot, &config.scan)?;
    let rubric_hash = config.rubric_hash();
    let mut frames = Vec::with_capacity(scan.files.len());
    let mut rows = Vec::with_capacity(scan.files.len());
    for source in &scan.files {
        let frame = judge(source, config, decider);
        rows.push(Row::of(&frame, &rubric_hash, &config.model));
        frames.push(frame);
    }
    report::write(&rows, &config.outputs)?;
    Ok(Run {
        rubric_hash,
        model: config.model.clone(),
        counts: scan.counts,
        frames,
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
        decider
            .decide(&request)
            .map(|response| reading::read(&config.rubric, response))
    });
    Frame {
        source: source.clone(),
        outcome,
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
