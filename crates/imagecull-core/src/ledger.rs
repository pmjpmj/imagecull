//! The ledger: what a run has already done, and therefore what a resumed run
//! may skip.
//!
//! A JSONL file beside the CSV. Its first line is a versioned header naming the
//! rubric, the model, the endpoint and the start time; every line after it is
//! one frame — its identity on disk, the SHA-256 of the bytes that were actually
//! sent, the status, the whole outcome, and when it was recorded (spec §9.2).
//!
//! Three things make the file worth trusting:
//!
//! - each record is appended and flushed as it is produced, so a `SIGKILL` costs
//!   at most the frame in flight;
//! - recovery keeps the last line that parses and truncates whatever follows it,
//!   so a half-written record cannot poison the file;
//! - a header written under a different rubric or model is a fatal, actionable
//!   error, so two rubrics never end up in one CSV (spec §28).
//!
//! The CSV is a rendering of this file — [`Record::row`] is the only thing that
//! turns a record into a line of the report — so a resumed frame's row and a
//! freshly judged one's cannot drift apart.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, ErrorKind, Result};
use crate::ingest::SourceFile;
use crate::reading::{Answers, Reading};
use crate::report::{self, Failure, Row};
use crate::rubric::RubricHash;
use crate::scoring::{Frame, Usage};

/// The ledger format this build writes. A file written by another version is a
/// mismatch, not a guess: the fields a resume depends on may have moved.
pub const VERSION: u32 = 1;

/// The suffix a ledger carries, appended to the CSV's stem: `scores.csv` →
/// `scores.ledger.jsonl`.
const SUFFIX: &str = "ledger.jsonl";

/// What to do with a ledger that is already there (spec §9.2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LedgerMode {
    /// Neither `--resume` nor `--fresh`. An existing ledger must have been
    /// written under this run's rubric and model — otherwise it is fatal, the
    /// same check `--resume` makes — and is then started over: a plain `score`
    /// is a new run, not a silent continuation.
    #[default]
    Restart,
    /// `--resume`: keep every frame that has not changed and score the rest.
    Resume,
    /// `--fresh`: throw the ledger away without looking at it.
    Discard,
}

/// A frame's identity on disk: the part of the resume key that says whether the
/// file is still the file that was scored.
///
/// A re-export or an edit changes the size or the modification time, so the
/// frame is scored again rather than silently recycled (spec §32).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileIdentity {
    /// The file's size in bytes.
    pub size: u64,
    /// The file's modification time in nanoseconds since the Unix epoch.
    pub mtime_ns: u64,
}

impl FileIdentity {
    /// Read a file's identity, or the classified reason it has none.
    pub fn of(path: &Path) -> Result<FileIdentity> {
        let metadata = fs::metadata(path).map_err(|error| {
            Error::new(
                ErrorKind::Unreadable,
                format!("{}: {error}", path.display()),
            )
        })?;
        Ok(FileIdentity {
            size: metadata.len(),
            mtime_ns: metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |since| {
                    u64::try_from(since.as_nanos()).unwrap_or(u64::MAX)
                }),
        })
    }
}

/// The key a resumed run matches a recorded frame on: the file, its size, and
/// its modification time in nanoseconds (spec §9.2).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResumeKey {
    file: String,
    size: u64,
    mtime_ns: u64,
}

impl ResumeKey {
    /// The resume key of `file` at `identity`.
    pub fn new(file: impl Into<String>, identity: FileIdentity) -> ResumeKey {
        ResumeKey {
            file: file.into(),
            size: identity.size,
            mtime_ns: identity.mtime_ns,
        }
    }
}

/// The first line of a ledger: the identity of the run that wrote it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    /// The format version ([`VERSION`]).
    pub version: u32,
    /// The full rubric hash, alias included, every record was scored under.
    pub rubric_hash: String,
    /// The alias every record was scored with.
    pub model: String,
    /// The server the run talked to, when the caller knows one: a `Decider`
    /// with no endpoint (a test's fake) records none.
    #[serde(default)]
    pub endpoint: Option<String>,
    /// When the run began, RFC 3339 in UTC.
    pub started: String,
}

impl Header {
    /// The header of a run that is starting now.
    pub fn new(rubric_hash: &RubricHash, model: &str, endpoint: Option<&str>) -> Header {
        Header {
            version: VERSION,
            rubric_hash: rubric_hash.as_str().to_string(),
            model: model.to_string(),
            endpoint: endpoint.map(str::to_string),
            started: now(),
        }
    }
}

/// One frame, as the ledger keeps it.
///
/// Everything the CSV needs and everything a resume needs: the frame's identity
/// on disk, the hash of the bytes that went to the model, the status, the whole
/// outcome, and when it was recorded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// The frame's name inside the shoot folder: the CSV's `file` column.
    pub file: String,
    /// The file's size in bytes, absent when it could not be read.
    #[serde(default)]
    pub size: Option<u64>,
    /// The file's modification time in nanoseconds since the Unix epoch, absent
    /// when it could not be read.
    #[serde(default)]
    pub mtime_ns: Option<u64>,
    /// The SHA-256 of the bytes actually sent, when the frame got as far as a
    /// request.
    #[serde(default)]
    pub preview_sha256: Option<String>,
    /// The key an identical preview may reuse this outcome under (ADR 0002),
    /// when the frame got as far as a request.
    #[serde(default)]
    pub dedupe_key: Option<String>,
    /// [`report::OK`], or the taxonomy status that cost the frame its values.
    pub status: String,
    /// A short message, on a frame that has no answers.
    #[serde(default)]
    pub error: Option<String>,
    /// The answer, on a frame the model answered.
    #[serde(default)]
    pub answers: Option<Answers>,
    /// What the request cost, when it got far enough to cost anything.
    #[serde(default)]
    pub usage: Option<Usage>,
    /// The raw answer document, kept whole — also what `--json-scores` writes.
    #[serde(default)]
    pub document: Option<Value>,
    /// The first frame whose identical preview this one reused.
    #[serde(default)]
    pub duplicate_of: Option<String>,
    /// When the frame was recorded, RFC 3339 in UTC.
    pub recorded: String,
}

/// What the ledger records about how a frame's outcome was produced.
///
/// The three fields travel together — a frame either reached the model or it did
/// not — and two of them are `Option<&str>`, so they are passed as one value
/// rather than as three positional arguments a caller could transpose.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Provenance<'a> {
    /// The SHA-256 of the bytes actually sent, when the frame got as far as a
    /// request.
    pub preview_sha256: Option<&'a str>,
    /// The key an identical preview may reuse this outcome under (ADR 0002).
    pub dedupe_key: Option<&'a str>,
    /// The earlier frame whose outcome this one reused, if it reused one.
    pub duplicate_of: Option<&'a str>,
}

impl Record {
    /// The record of a frame the run just judged.
    ///
    /// `identity` is what the filesystem said about the file; `provenance` is
    /// what the run did with it — the bytes that went out, the key an identical
    /// preview could reuse the outcome under, and the frame it was reused from.
    pub fn of(frame: &Frame, identity: Option<FileIdentity>, provenance: Provenance<'_>) -> Record {
        let mut record = Record {
            file: crate::scoring::display_name(&frame.source),
            size: identity.map(|identity| identity.size),
            mtime_ns: identity.map(|identity| identity.mtime_ns),
            preview_sha256: provenance.preview_sha256.map(str::to_string),
            dedupe_key: provenance.dedupe_key.map(str::to_string),
            status: report::OK.to_string(),
            error: None,
            answers: None,
            usage: None,
            document: None,
            duplicate_of: provenance.duplicate_of.map(str::to_string),
            recorded: now(),
        };
        match &frame.outcome {
            Err(error) => {
                let failure = Failure::of(error);
                record.status = failure.kind.as_str().to_string();
                record.error = Some(failure.message);
            }
            Ok(reading) => {
                record.usage = Some(reading.usage);
                record.document = Some(reading.document.clone());
                match &reading.answers {
                    Ok(answers) => record.answers = Some(answers.clone()),
                    Err(error) => {
                        let failure = Failure::of(error);
                        record.status = failure.kind.as_str().to_string();
                        record.error = Some(failure.message);
                    }
                }
            }
        }
        record
    }

    /// The resume key of this record, when the file's identity was recorded.
    pub fn resume_key(&self) -> Option<ResumeKey> {
        Some(ResumeKey {
            file: self.file.clone(),
            size: self.size?,
            mtime_ns: self.mtime_ns?,
        })
    }

    /// The frame this record stands for, as the run would have produced it.
    ///
    /// A frame restored from the ledger is a frame like any other: a resumed
    /// run's summary counts it, and its row is the row that was already decided.
    pub fn frame(&self, source: SourceFile) -> Frame {
        let outcome = match self.failure() {
            Some(failure) => Err(Error::new(failure.kind, failure.message)),
            None => Ok(Reading {
                answers: self.answers.clone().ok_or_else(|| {
                    Error::new(
                        ErrorKind::BadResponse,
                        format!("the ledger records no answers for {}", self.file),
                    )
                }),
                usage: self.usage.unwrap_or_default(),
                document: self.document.clone().unwrap_or(Value::Null),
            }),
        };
        Frame { source, outcome }
    }

    /// The CSV row this record stands for.
    ///
    /// `source` is the row's own file, so a frame that reused another's outcome
    /// reports itself and says where the numbers came from.
    pub fn row(&self, source: SourceFile, rubric_hash: &RubricHash, model: &str) -> Row {
        Row {
            source,
            answers: self.answers.clone(),
            usage: self.usage,
            document: self.document.clone(),
            failure: self.failure(),
            duplicate_of: self.duplicate_of.clone(),
            rubric_hash: rubric_hash.short().to_string(),
            model: model.to_string(),
        }
    }

    /// The status and message of a frame that has no answers.
    ///
    /// A record that claims `ok` and carries no answers cannot be produced by a
    /// run — [`Record::is_whole`] rejects it when the ledger is read — so it is
    /// reported as what it is: a response nothing could read.
    fn failure(&self) -> Option<Failure> {
        if self.status != report::OK {
            return Some(Failure {
                kind: ErrorKind::from_status(&self.status).unwrap_or(ErrorKind::Unreadable),
                message: self.error.clone().unwrap_or_default(),
            });
        }
        self.answers.is_none().then(|| Failure {
            kind: ErrorKind::BadResponse,
            message: format!("the ledger records no answers for {}", self.file),
        })
    }

    /// Whether this record says enough for a resume to rely on it: a status the
    /// taxonomy names, answers when it claims `ok`, and an identity either whole
    /// or absent.
    fn is_whole(&self) -> bool {
        let known = self.status == report::OK || ErrorKind::from_status(&self.status).is_some();
        let answered = self.status != report::OK || self.answers.is_some();
        let matched = (self.size.is_none() && self.mtime_ns.is_none())
            || (self.size.is_some() && self.mtime_ns.is_some());
        known && answered && matched
    }
}

/// The ledger of one run: the file, the frames it holds, and the two questions a
/// run asks it — "is this frame already done?" and "may I reuse this outcome?".
pub struct Ledger {
    file: File,
    records: Vec<Record>,
    by_resume: HashMap<ResumeKey, usize>,
    by_dedupe: HashMap<String, usize>,
    truncated: usize,
    discarded: usize,
}

impl Ledger {
    /// Open the ledger that belongs beside `csv`, in the mode the photographer
    /// chose, and make it ready to append.
    ///
    /// An existing ledger written under another rubric or model, or by another
    /// version of this format, is fatal unless the mode throws it away: a
    /// mismatch is the one thing a resume must never paper over.
    pub fn open(csv: &Path, header: Header, mode: LedgerMode) -> Result<Ledger> {
        let path = path_for(csv);
        let existing = read(&path)?;
        let mut records = Vec::new();
        let mut keep = 0;
        let mut write_header = true;
        let mut truncated = 0;
        let mut discarded = 0;
        if mode != LedgerMode::Discard && !existing.is_empty() {
            let loaded = parse(&path, &existing)?;
            ensure_compatible(&path, &loaded.header, &header)?;
            if mode == LedgerMode::Resume {
                records = loaded.records;
                keep = loaded.keep;
                write_header = false;
                // What the last line that did not parse cost: a hard kill's
                // leavings, which a resumed run re-scores. Worth saying out
                // loud, because the photographer's last run died.
                truncated = existing.len().saturating_sub(keep);
            } else {
                // A restart throws these away. Nothing else knows how much work
                // that is, and starting over silently is how an hour of a
                // shoot disappears without a word.
                discarded = loaded.records.len();
            }
        }
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .map_err(|error| {
                Error::fatal(format!(
                    "cannot open the ledger {}: {error}",
                    path.display()
                ))
            })?;
        // A corrupt tail is cut here, so the next record lands on clean bytes.
        file.set_len(keep as u64).map_err(|error| {
            Error::fatal(format!(
                "cannot repair the ledger {}: {error}",
                path.display()
            ))
        })?;
        file.seek(SeekFrom::End(0)).map_err(|error| {
            Error::fatal(format!(
                "cannot reach the end of {}: {error}",
                path.display()
            ))
        })?;
        let mut ledger = Ledger {
            file,
            records: Vec::new(),
            by_resume: HashMap::new(),
            by_dedupe: HashMap::new(),
            truncated,
            discarded,
        };
        if write_header {
            ledger.write_line(&header, &path)?;
        }
        for record in records {
            ledger.remember(record);
        }
        Ok(ledger)
    }

    /// The frames the ledger holds, in the order they were recorded.
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    /// How many bytes of a half-written record opening this ledger discarded: a
    /// killed run's last line, which a resume re-scores rather than trusts.
    ///
    /// Only a resume can repair anything — a restart starts the file over on
    /// purpose, so it has nothing to report.
    pub fn truncated_bytes(&self) -> usize {
        self.truncated
    }

    /// How many recorded frames opening this ledger threw away: a plain `score`
    /// starts the file over, and the frames it held are scored again.
    ///
    /// `--fresh` discards without reading, so it reports nothing: the
    /// photographer already said what they wanted.
    pub fn discarded_frames(&self) -> usize {
        self.discarded
    }

    /// The frame already recorded under `key`, if a resume may skip it.
    pub fn recorded(&self, key: &ResumeKey) -> Option<&Record> {
        self.by_resume.get(key).map(|index| &self.records[*index])
    }

    /// The frame whose identical preview was already judged, if this one may
    /// reuse its outcome.
    pub fn reusable(&self, dedupe_key: &str) -> Option<&Record> {
        self.by_dedupe
            .get(dedupe_key)
            .map(|index| &self.records[*index])
    }

    /// Append one frame's record and flush it, so a hard kill loses at most the
    /// frame in flight.
    pub fn append(&mut self, record: Record) -> Result<()> {
        let line = line(&record)?;
        self.file
            .write_all(line.as_bytes())
            .and_then(|()| self.file.sync_data())
            .map_err(|error| Error::fatal(format!("cannot append to the ledger: {error}")))?;
        self.remember(record);
        Ok(())
    }

    /// Write one line of JSONL for `value`, and flush it.
    fn write_line<T: Serialize>(&mut self, value: &T, path: &Path) -> Result<()> {
        let line = line(value)?;
        self.file
            .write_all(line.as_bytes())
            .and_then(|()| self.file.sync_data())
            .map_err(|error| {
                Error::fatal(format!(
                    "cannot write the ledger {}: {error}",
                    path.display()
                ))
            })
    }

    /// Index a record so the run's two questions answer in constant time.
    fn remember(&mut self, record: Record) {
        let index = self.records.len();
        if let Some(key) = record.resume_key() {
            self.by_resume.entry(key).or_insert(index);
        }
        if record.status == report::OK
            && let Some(key) = &record.dedupe_key
        {
            // The earliest frame with these bytes keeps the credit: a third
            // identical frame names the first, not the one that reused it.
            self.by_dedupe.entry(key.clone()).or_insert(index);
        }
        self.records.push(record);
    }
}

/// One record or header as a line of JSONL.
fn line<T: Serialize>(value: &T) -> Result<String> {
    let mut line = serde_json::to_string(value)
        .map_err(|error| Error::fatal(format!("cannot serialise a ledger line: {error}")))?;
    line.push('\n');
    Ok(line)
}

/// Where a run's ledger lives: beside its CSV, named after it.
pub fn path_for(csv: &Path) -> PathBuf {
    csv.with_extension(SUFFIX)
}

/// The ledger file's bytes, or nothing when there is no ledger yet.
fn read(path: &Path) -> Result<Vec<u8>> {
    match fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(Error::fatal(format!(
            "cannot read the ledger {}: {error}",
            path.display()
        ))),
    }
}

/// A ledger read back, with the length of the part that was whole.
struct Loaded {
    header: Header,
    records: Vec<Record>,
    /// The bytes before the first line that did not parse: everything after it
    /// is a hard kill's leavings and is truncated.
    keep: usize,
}

/// Read a ledger, keeping the last line that parses and stopping at the first
/// that does not.
fn parse(path: &Path, bytes: &[u8]) -> Result<Loaded> {
    let mut lines = lines(bytes);
    let Some((start, end)) = lines.next() else {
        return Err(Error::fatal(format!(
            "{} is an empty ledger",
            path.display()
        )));
    };
    let header: Header = serde_json::from_slice(trimmed(bytes, start, end)).map_err(|error| {
        Error::fatal(format!(
            "{} is not an imagecull ledger: its first line is not a ledger header ({error}); \
             pass --fresh to start over, or move the file aside",
            path.display()
        ))
    })?;
    let mut records = Vec::new();
    let mut keep = end;
    for (start, end) in lines {
        match serde_json::from_slice::<Record>(trimmed(bytes, start, end)) {
            Ok(record) if record.is_whole() => {
                records.push(record);
                keep = end;
            }
            // A line that was cut short, or that nothing wrote: the run died
            // mid-write, and everything from here is not a record.
            _ => break,
        }
    }
    Ok(Loaded {
        header,
        records,
        keep,
    })
}

/// The line starting at `start` and ending at `end`, without its line ending.
fn trimmed(bytes: &[u8], start: usize, end: usize) -> &[u8] {
    let mut line = &bytes[start..end];
    while let [rest @ .., last] = line
        && (*last == b'\n' || *last == b'\r')
    {
        line = rest;
    }
    line
}

/// The `(start, end)` of every line, `end` one past its `\n` when it has one.
fn lines(bytes: &[u8]) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut start = 0;
    std::iter::from_fn(move || {
        if start >= bytes.len() {
            return None;
        }
        let end = bytes[start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(bytes.len(), |index| start + index + 1);
        let line = (start, end);
        start = end;
        Some(line)
    })
}

/// Refuse a ledger this run cannot honestly continue.
fn ensure_compatible(path: &Path, found: &Header, wanted: &Header) -> Result<()> {
    if found.version != wanted.version {
        return Err(Error::fatal(format!(
            "{} is ledger version {}, and this build writes version {} — pass --fresh to start over",
            path.display(),
            found.version,
            wanted.version
        )));
    }
    if found.rubric_hash != wanted.rubric_hash || found.model != wanted.model {
        return Err(Error::fatal(format!(
            "{} was written with rubric {} (model {}), but this run uses rubric {} (model {}) — \
             pass --fresh to start over, or write this run's scores to a different --out",
            path.display(),
            short(&found.rubric_hash),
            found.model,
            short(&wanted.rubric_hash),
            wanted.model
        )));
    }
    Ok(())
}

/// The first eight characters of a hash, for an error message that has to stay
/// readable. A header written by hand may hold anything, so this never panics.
fn short(hash: &str) -> &str {
    &hash[..hash.len().min(8)]
}

/// The current time, RFC 3339 in UTC.
fn now() -> String {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    timestamp(since_epoch.as_secs())
}

/// `seconds` since the Unix epoch as `YYYY-MM-DDTHH:MM:SSZ`.
fn timestamp(seconds: u64) -> String {
    let (year, month, day) = civil_from_days(seconds / 86_400);
    let rest = seconds % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// The calendar date of a day count from 1970-01-01: Howard Hinnant's
/// `civil_from_days`, which is the whole of the calendar arithmetic a timestamp
/// needs and one dependency cheaper than a date crate.
fn civil_from_days(days: u64) -> (i64, u64, u64) {
    let days = i64::try_from(days).unwrap_or(i64::MAX);
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    (
        year,
        u64::try_from(month).unwrap_or(1),
        u64::try_from(day).unwrap_or(1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_timestamp_is_rfc_3339_utc() {
        // Known values, so the calendar arithmetic is pinned to the real
        // calendar rather than to itself.
        assert_eq!(timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(timestamp(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(timestamp(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(timestamp(4_102_444_800), "2100-01-01T00:00:00Z");
    }

    #[test]
    fn a_ledger_belongs_beside_its_csv() {
        assert_eq!(
            path_for(Path::new("/out/scores.csv")),
            PathBuf::from("/out/scores.ledger.jsonl")
        );
        assert_eq!(
            path_for(Path::new("scores/march.csv")),
            PathBuf::from("scores/march.ledger.jsonl")
        );
    }
}
