//! The deliverable: a CSV one row per frame, and the raw answers behind it.
//!
//! The columns are the spec's table verbatim (spec §9.1), in its order, so a
//! photographer who has read the spec reads the file without a legend. A row is
//! appended as the ledger record behind it is produced, and the file is
//! truncated once at the start of the run and never again, so after frame N the
//! CSV holds exactly the ledger's first N rows: it is never a snapshot, and a
//! spreadsheet or `tail -f` can follow it while the run is still going.
//!
//! No row is `fsync`ed. The ledger is the durable record — it is flushed per
//! record and repaired on reopen — and the CSV is a rendering of it that a
//! `--resume` rebuilds from scratch, so paying for durability twice would buy
//! nothing.
//!
//! RFC 4180 above all: a filename may contain a comma, a quote or a line break,
//! and every cell is UTF-8. Values are the model's own answer — the integer key
//! it chose, and its confidence as the model reported it — and a failed row
//! leaves its value cells empty rather than zero, because "no answer" and "a
//! rating of one" are different facts.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::error::{Error, ErrorKind, Result};
use crate::ingest::SourceFile;
use crate::reading::Answers;
use crate::scoring::Usage;

/// The `status` a row carries when nothing went wrong (spec §10).
pub const OK: &str = "ok";

/// The columns, in the order the spec's table lists them.
pub const COLUMNS: [&str; 10] = [
    "file",
    "stars",
    "conf_stars",
    "latency_ms",
    "input_tokens",
    "status",
    "error",
    "duplicate_of",
    "rubric_hash",
    "model",
];

/// Where a run writes its report, and how it spells the paths in it.
#[derive(Debug, Clone, PartialEq, Eq)]
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
/// ledger record will turn back into (spec §9.2).
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// The file, as the scan found it. The `file` column picks the absolute or
    /// the in-folder spelling from it.
    pub source: SourceFile,
    /// The answer at the precision the model gave it; `None` on a failed row,
    /// whose value cells stay empty.
    pub answers: Option<Answers>,
    /// What the request cost, when it got far enough to cost anything.
    pub usage: Option<Usage>,
    /// The raw answer document, kept for `--json-scores`. A `bad_response` row
    /// keeps its document even though its answers could not be read.
    pub document: Option<Value>,
    /// Why the row has no answers; `None` means it has them.
    pub failure: Option<Failure>,
    /// The first frame whose identical preview this row reuses (spec §9.2): the
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

/// The CSV as a run writes it: truncated once, then one row appended per frame.
///
/// The open is what makes the file the run's: whatever was at `path` belonged to
/// an earlier run, and a report that is the ledger's own account of itself
/// cannot start by keeping someone else's rows.
pub struct Writer {
    path: PathBuf,
    file: File,
    /// How this report spells the `file` column; fixed for the whole run.
    absolute: bool,
}

impl Writer {
    /// Truncate `path` and write the header, ready to append.
    pub fn open(path: &Path, absolute: bool) -> Result<Writer> {
        let mut file = File::create(path)
            .map_err(|error| Error::fatal(format!("cannot write {}: {error}", path.display())))?;
        let mut header = String::new();
        push_record(&mut header, &COLUMNS);
        write_bytes(&mut file, path, &header)?;
        Ok(Writer {
            path: path.to_path_buf(),
            file,
            absolute,
        })
    }

    /// Append one frame's row. A file write puts it in the kernel's hands
    /// immediately, so a reader sees it without anything being flushed here.
    pub fn append(&mut self, row: &Row) -> Result<()> {
        let mut line = String::new();
        push_record(&mut line, &cells(row, self.absolute));
        write_bytes(&mut self.file, &self.path, &line)
    }
}

/// Write `--json-scores`, when the run asked for it: the raw documents, once, at
/// the end. A JSON object cannot be appended line by line, and a frame whose
/// document a killed run lost is still in the ledger.
pub fn write_json(rows: &[Row], outputs: &Outputs) -> Result<()> {
    if let Some(path) = &outputs.json_scores {
        write_file(path, json(rows, outputs.absolute).as_bytes())?;
    }
    Ok(())
}

/// One row's cells, in [`COLUMNS`] order.
fn cells(row: &Row, absolute: bool) -> Vec<String> {
    let answers =
        |cell: fn(&Answers) -> String| row.answers.as_ref().map_or_else(String::new, cell);
    let usage = row.usage;
    vec![
        file_cell(&row.source, absolute),
        answers(|answers| answers.stars.to_string()),
        answers(|answers| number(answers.confidence)),
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
///
/// Shared with the measures CSV ([`group::render_csv`](crate::group::render_csv)):
/// one writer, so a filename with a comma is quoted the same way in both reports.
pub(crate) fn push_record(out: &mut String, fields: &[impl AsRef<str>]) {
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
pub(crate) fn push_field(out: &mut String, value: &str) {
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

fn write_bytes(file: &mut File, path: &Path, bytes: &str) -> Result<()> {
    file.write_all(bytes.as_bytes())
        .map_err(|error| Error::fatal(format!("cannot write {}: {error}", path.display())))
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes)
        .map_err(|error| Error::fatal(format!("cannot write {}: {error}", path.display())))
}
