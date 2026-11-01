//! The deliverable: a CSV one row per frame, and the raw answers behind it.
//!
//! The columns are the spec's table verbatim (spec §9.2), in its order, so a
//! photographer who has read the spec reads the file without a legend. A row is
//! rendered from a ledger record as that record is produced rather than rebuilt
//! from the finished run, so a long run can be flushed mid-flight and the CSV is
//! always the ledger's own account of itself.
//!
//! RFC 4180 above all: a filename may contain a comma, a quote or a line break,
//! and every cell is UTF-8. Values are the model's own numbers at the precision
//! it gave them — never rounded, never rescaled — and a failed row leaves its
//! value cells empty rather than zero, because "no answer" and "a score of
//! zero" are different facts.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::error::{Error, ErrorKind, Result};
use crate::ingest::SourceFile;
use crate::reading::Answers;
use crate::scoring::Usage;

/// The `status` a row carries when nothing went wrong (spec §10).
pub const OK: &str = "ok";

/// The columns, in the order the spec's table lists them.
pub const COLUMNS: [&str; 17] = [
    "file",
    "overall",
    "technical",
    "composition",
    "impact",
    "stars",
    "verdict",
    "conf_overall",
    "conf_stars",
    "conf_verdict",
    "latency_ms",
    "input_tokens",
    "status",
    "error",
    "duplicate_of",
    "rubric_hash",
    "model",
];

/// Where a run writes its report, and how it spells the paths in it.
#[derive(Debug, Clone)]
pub struct Outputs {
    /// The CSV of scores (`--out`).
    pub csv: PathBuf,
    /// The raw answer document of every frame (`--json-scores`), when asked
    /// for.
    pub json_scores: Option<PathBuf>,
    /// Write absolute paths in the `file` column (`--absolute`) instead of
    /// paths relative to the shoot folder.
    pub absolute: bool,
}

/// One line of the CSV: the unit a run accumulates as it goes, and the unit a
/// ledger record will turn back into (spec §9.3).
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// The file, as the scan found it. The `file` column picks the absolute or
    /// the in-folder spelling from it.
    pub source: SourceFile,
    /// The six answers at the precision the model gave them; `None` on a failed
    /// row, whose value cells stay empty.
    pub answers: Option<Answers>,
    /// What the request cost, when it got far enough to cost anything.
    pub usage: Option<Usage>,
    /// The raw answer document, kept for `--json-scores`. A `bad_response` row
    /// keeps its document even though its answers could not be read.
    pub document: Option<Value>,
    /// Why the row has no answers; `None` means it has them.
    pub failure: Option<Failure>,
    /// The first frame whose identical preview this row reuses (spec §9.3): the
    /// frame that paid for the request this row's numbers came from.
    pub duplicate_of: Option<String>,
    /// The rubric's short hash: the row's provenance at a glance.
    pub rubric_hash: String,
    /// The alias actually used.
    pub model: String,
}

impl Row {
    /// The `status` column: `ok`, or the kind that cost the frame its values.
    pub fn status(&self) -> &'static str {
        self.failure
            .as_ref()
            .map_or(OK, |failure| failure.kind.as_str())
    }
}

// The rows are built from ledger records (`ledger::Record::row`) rather than
// from whatever the run happens to hold in memory, so a resumed frame and a
// freshly judged one are rendered by the same code.

/// A failed row's `status` and `error` columns, kept together so they cannot
/// disagree about whether the row failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The taxonomy kind (spec §10).
    pub kind: ErrorKind,
    /// A short message that explains the row without a log file.
    pub message: String,
}

impl Failure {
    /// The status and message for a frame's error.
    ///
    /// Every error a frame can carry is classified: an unclassified one is
    /// run-level, and stops the run before it could become a row.
    pub fn of(error: &Error) -> Failure {
        Failure {
            kind: error.kind().unwrap_or(ErrorKind::Unreadable),
            message: error.message().to_string(),
        }
    }
}

/// Write the run's report: the CSV, and the raw documents when asked for.
pub fn write(rows: &[Row], outputs: &Outputs) -> Result<()> {
    write_file(&outputs.csv, csv(rows, outputs.absolute).as_bytes())?;
    if let Some(path) = &outputs.json_scores {
        write_file(path, json(rows, outputs.absolute).as_bytes())?;
    }
    Ok(())
}

/// The CSV: the header, then one record per row in ingest order.
fn csv(rows: &[Row], absolute: bool) -> String {
    let mut out = String::new();
    let header: Vec<&str> = COLUMNS.to_vec();
    push_record(&mut out, &header);
    for row in rows {
        push_record(&mut out, &cells(row, absolute));
    }
    out
}

/// One row's cells, in [`COLUMNS`] order.
fn cells(row: &Row, absolute: bool) -> Vec<String> {
    let answers =
        |cell: fn(&Answers) -> String| row.answers.as_ref().map_or_else(String::new, cell);
    let usage = row.usage;
    vec![
        file_cell(&row.source, absolute),
        answers(|answers| number(answers.overall)),
        answers(|answers| number(answers.technical)),
        answers(|answers| number(answers.composition)),
        answers(|answers| number(answers.impact)),
        answers(|answers| answers.stars.to_string()),
        answers(|answers| answers.verdict.as_str().to_string()),
        answers(|answers| number(answers.confidence.overall)),
        answers(|answers| number(answers.confidence.stars)),
        answers(|answers| number(answers.confidence.verdict)),
        // The one cell that measures the run instead of reporting the model:
        // two runs of the same folder agree on every other column (spec
        // §Determinism), and this is the reason they cannot agree on all of
        // them.
        usage.map_or_else(String::new, |usage| usage.latency_ms().to_string()),
        usage
            .and_then(|usage| usage.input_tokens)
            .map_or_else(String::new, |tokens| tokens.to_string()),
        row.status().to_string(),
        row.failure
            .as_ref()
            .map_or_else(String::new, |failure| failure.message.clone()),
        row.duplicate_of.clone().unwrap_or_default(),
        row.rubric_hash.clone(),
        row.model.clone(),
    ]
}

/// A number as the CSV spells it: the shortest text that round-trips to the
/// same `f64`. The model's `6.4237` is never rounded to `6.42`, and never
/// padded with digits it did not send.
fn number(value: f64) -> String {
    value.to_string()
}

/// The `file` cell: the path inside the shoot folder, or the absolute one.
fn file_cell(source: &SourceFile, absolute: bool) -> String {
    let path = if absolute {
        std::path::absolute(&source.path).unwrap_or_else(|_| source.path.clone())
    } else {
        source.relative.clone()
    };
    path.to_string_lossy().into_owned()
}

/// Append one CSV record, escaping the cells that need it and ending the line
/// with `\n` (the spec's ending, not RFC 4180's `\r\n`).
fn push_record(out: &mut String, fields: &[impl AsRef<str>]) {
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_field(out, field.as_ref());
    }
    out.push('\n');
}

/// Append one cell, quoted the way RFC 4180 asks when it has to be: it holds a
/// comma, a quote or a line break.
fn push_field(out: &mut String, value: &str) {
    if !value.contains([',', '"', '\n', '\r']) {
        out.push_str(value);
        return;
    }
    out.push('"');
    for character in value.chars() {
        if character == '"' {
            out.push('"');
        }
        out.push(character);
    }
    out.push('"');
}

/// The raw answer document of every row, keyed by the same spelling the CSV
/// uses, `null` for a frame the model never answered (an unreadable file, a
/// timeout): a surprising score loses nothing, and a missing one says so.
fn json(rows: &[Row], absolute: bool) -> String {
    let mut documents = Map::new();
    for row in rows {
        documents.insert(
            file_cell(&row.source, absolute),
            row.document.clone().unwrap_or(Value::Null),
        );
    }
    let mut out = serde_json::to_string_pretty(&Value::Object(documents))
        .unwrap_or_else(|_| "{}".to_string());
    out.push('\n');
    out
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes)
        .map_err(|error| Error::fatal(format!("cannot write {}: {error}", path.display())))
}
