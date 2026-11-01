//! The XMP sidecar beside a frame: the rating, and the provenance of it.
//!
//! One job, one function: an existing document (or none) plus one frame's
//! properties in, the text to write out — or a classified refusal. Nothing here
//! opens a file: the writer is a port and the run owns it.
//!
//! The merge is a targeted text edit rather than a parse and re-serialise,
//! because the requirement is that every byte we do not own survives unchanged:
//! develop settings, keywords, GPS and another application's history are not
//! ours to reorder, re-quote or drop. So there is no XML dependency here, only
//! enough scanning to find the first `rdf:Description` opening tag, the
//! attributes in it, and where its attributes end.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use crate::error::{Error, ErrorKind, Result};
use crate::ingest::SourceFile;
use crate::reading::Answers;
use crate::scoring::Usage;

/// The namespace our own properties live in, outside the reach of any other
/// application.
pub const IMAGECULL_NAMESPACE: &str = "https://ns.imagecull.app/1.0/";

/// The XMP namespace, where the rating lives.
pub const XMP_NAMESPACE: &str = "http://ns.adobe.com/xap/1.0/";

/// The RDF namespace, where a description of the frame lives.
pub const RDF_NAMESPACE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";

/// The opening tag whose attributes we read and extend.
const DESCRIPTION: &str = "<rdf:Description";

/// One frame's rating and the provenance of it: everything this tool owns in a
/// sidecar.
#[derive(Debug, Clone, PartialEq)]
pub struct Properties {
    /// The model's rating, 1–5. Never `-1`: the destructive reject is the
    /// photographer's, not the model's.
    pub rating: u8,
    /// The top-option probability of that rating, at the precision the model
    /// gave it.
    pub conf_stars: f64,
    /// The alias the rating was produced by.
    pub model: String,
    /// The rubric's short hash: which wording judged this frame.
    pub rubric_hash: String,
    /// When, RFC 3339 in UTC.
    pub scored_at: String,
    /// The binary that wrote it.
    pub version: String,
}

/// The document to write for one frame: `existing` when there is one to merge
/// into, `None` when the sidecar is new.
pub fn render(existing: Option<&str>, properties: &Properties) -> Result<String> {
    match existing {
        None => Ok(new_document(properties)),
        Some(text) => merge(text, properties),
    }
}

/// A whole new sidecar for a frame that has none.
fn new_document(properties: &Properties) -> String {
    let Properties {
        rating,
        conf_stars,
        model,
        rubric_hash,
        scored_at,
        version,
    } = properties;
    format!(
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"imagecull {version}\">\n\
         \x20<rdf:RDF xmlns:rdf=\"{RDF_NAMESPACE}\">\n\
         \x20 <rdf:Description rdf:about=\"\"\n\
         \x20   xmlns:xmp=\"{XMP_NAMESPACE}\"\n\
         \x20   xmlns:imagecull=\"{IMAGECULL_NAMESPACE}\"\n\
         \x20   xmp:Rating=\"{rating}\"\n\
         \x20   imagecull:confStars=\"{conf}\"\n\
         \x20   imagecull:model=\"{model}\"\n\
         \x20   imagecull:rubricHash=\"{rubric_hash}\"\n\
         \x20   imagecull:scoredAt=\"{scored_at}\"\n\
         \x20   imagecull:version=\"{version}\"/>\n\
         \x20</rdf:RDF>\n\
         </x:xmpmeta>\n",
        conf = escape(&conf_stars.to_string()),
        model = escape(model),
        rubric_hash = escape(rubric_hash),
        scored_at = escape(scored_at),
        version = escape(version),
    )
}

/// Merge our properties into an existing document, leaving everything else
/// exactly as it was.
fn merge(text: &str, properties: &Properties) -> Result<String> {
    let start = text.find(DESCRIPTION).ok_or_else(no_description)?;
    let end = tag_end(text, start).ok_or_else(no_description)?;
    let tag = &text[start..end];
    let merged = merge_tag(tag, properties);
    Ok(format!("{}{}{}", &text[..start], merged, &text[end..]))
}

/// The index of the `>` that closes the tag starting at `start`, skipping any
/// `>` inside a quoted attribute value.
fn tag_end(text: &str, start: usize) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (offset, character) in text[start..].char_indices() {
        match (quote, character) {
            (None, '"' | '\'') => quote = Some(character),
            (Some(open), character) if character == open => quote = None,
            (None, '>') => return Some(start + offset),
            _ => {}
        }
    }
    None
}

/// The tag with our properties merged in: our own attributes replaced, the
/// rating rewritten in place, and anything missing appended.
///
/// Appended attributes go on one line before the tag's `>` (or `/`), separated
/// by single spaces: the one piece of formatting this tool invents inside
/// someone else's document, kept as small as it can be.
fn merge_tag(tag: &str, properties: &Properties) -> String {
    let attributes = attributes(tag);
    // Nothing to do when the document already says exactly this: a forced run
    // over an unchanged rating must not churn the file.
    if is_current(&attributes, properties) {
        return tag.to_string();
    }

    let ours = our_properties(properties);
    let mut edits: Vec<Edit> = Vec::new();
    let mut appended = String::new();

    let declared = |name: &str| attributes.iter().any(|attribute| attribute.name == name);
    if !declared("xmlns:xmp") {
        appended.push_str(&format!(" xmlns:xmp=\"{XMP_NAMESPACE}\""));
    }
    if !declared("xmlns:imagecull") {
        appended.push_str(&format!(" xmlns:imagecull=\"{IMAGECULL_NAMESPACE}\""));
    }

    // A property this run knows and the document already has is rewritten in
    // place, so its formatting survives; one it lacks is appended.
    for (name, value) in &ours {
        match attributes.iter().find(|attribute| attribute.name == *name) {
            Some(attribute) => edits.push(Edit::replace(
                attribute.value_start,
                attribute.value_end,
                escape(value),
            )),
            None => appended.push_str(&format!(" {name}=\"{}\"", escape(value))),
        }
    }

    // A property of ours this build no longer writes is not left behind: ours
    // are ours to retire.
    for attribute in &attributes {
        if attribute.name.starts_with("imagecull:")
            && !ours.iter().any(|(name, _)| *name == attribute.name)
        {
            edits.push(Edit::remove(
                whitespace_before(tag, attribute.name_start),
                attribute.end,
            ));
        }
    }

    if !appended.is_empty() {
        edits.push(Edit::insert(attributes_end(tag), appended));
    }
    apply(tag, edits)
}

/// The properties this run writes, in the order a new document lists them.
fn our_properties(properties: &Properties) -> [(&'static str, String); 6] {
    [
        ("xmp:Rating", properties.rating.to_string()),
        ("imagecull:confStars", properties.conf_stars.to_string()),
        ("imagecull:model", properties.model.clone()),
        ("imagecull:rubricHash", properties.rubric_hash.clone()),
        ("imagecull:scoredAt", properties.scored_at.clone()),
        ("imagecull:version", properties.version.clone()),
    ]
}

/// Whether the tag already carries exactly the properties this run would write.
fn is_current(attributes: &[Attribute], properties: &Properties) -> bool {
    let value = |name: &str| {
        attributes
            .iter()
            .find(|attribute| attribute.name == name)
            .map(|attribute| attribute.value.clone())
    };
    value("xmp:Rating").as_deref() == Some(properties.rating.to_string().as_str())
        && value("imagecull:confStars").as_deref()
            == Some(properties.conf_stars.to_string().as_str())
        && value("imagecull:model").as_deref() == Some(properties.model.as_str())
        && value("imagecull:rubricHash").as_deref() == Some(properties.rubric_hash.as_str())
        && value("imagecull:scoredAt").as_deref() == Some(properties.scored_at.as_str())
        && value("imagecull:version").as_deref() == Some(properties.version.as_str())
}

/// One attribute of an opening tag, as the spans a text edit needs.
struct Attribute {
    name: String,
    /// The value as text, with XML entities decoded.
    value: String,
    /// Where the name starts.
    name_start: usize,
    /// Where the value starts, after the opening quote.
    value_start: usize,
    /// Where the value ends, before the closing quote.
    value_end: usize,
    /// Where the attribute ends, after the closing quote.
    end: usize,
}

/// The attributes of an opening tag, in document order.
fn attributes(tag: &str) -> Vec<Attribute> {
    let bytes = tag.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    // The tag's own name is not an attribute.
    while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    while index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] == b'/' || bytes[index] == b'>' {
            break;
        }
        let name_start = index;
        while index < bytes.len()
            && bytes[index] != b'='
            && !bytes[index].is_ascii_whitespace()
            && bytes[index] != b'/'
        {
            index += 1;
        }
        let name = tag[name_start..index].to_string();
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] != b'=' {
            continue;
        }
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }
        let quote = bytes[index];
        if quote != b'"' && quote != b'\'' {
            continue;
        }
        index += 1;
        let value_start = index;
        while index < bytes.len() && bytes[index] != quote {
            index += 1;
        }
        let value_end = index;
        if index < bytes.len() {
            index += 1;
        }
        out.push(Attribute {
            name,
            value: unescape(&tag[value_start..value_end]),
            name_start,
            value_start,
            value_end,
            end: index,
        });
    }
    out
}

/// Where a tag's attributes end: before a self-closing `/`, else at the end.
fn attributes_end(tag: &str) -> usize {
    let trimmed = tag.trim_end();
    if trimmed.ends_with('/') {
        trimmed.len() - 1
    } else {
        trimmed.len()
    }
}

/// Where the whitespace before `index` starts, so removing an attribute does not
/// leave a run of spaces behind.
fn whitespace_before(text: &str, index: usize) -> usize {
    let mut start = index;
    for (offset, character) in text[..index].char_indices().rev() {
        if !character.is_whitespace() {
            break;
        }
        start = offset;
    }
    start
}

/// One text edit: a replacement of `start..end`, or an insertion at `start`.
struct Edit {
    start: usize,
    end: usize,
    replacement: String,
}

impl Edit {
    fn remove(start: usize, end: usize) -> Edit {
        Edit {
            start,
            end,
            replacement: String::new(),
        }
    }

    fn replace(start: usize, end: usize, replacement: String) -> Edit {
        Edit {
            start,
            end,
            replacement,
        }
    }

    fn insert(at: usize, replacement: String) -> Edit {
        Edit {
            start: at,
            end: at,
            replacement,
        }
    }
}

/// Apply edits to `text`, last first so earlier indices stay valid.
fn apply(text: &str, mut edits: Vec<Edit>) -> String {
    edits.sort_by_key(|edit| std::cmp::Reverse((edit.start, edit.end)));
    let mut out = text.to_string();
    for edit in edits {
        out.replace_range(edit.start..edit.end, &edit.replacement);
    }
    out
}

/// An attribute value, escaped for XML: a filename or an alias may contain any
/// of these, and an unescaped one would break the document.
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            other => out.push(other),
        }
    }
    out
}

/// The refusal when there is no `rdf:Description` we can safely extend: another
/// tool's document is not ours to guess at, so the frame is left alone.
fn no_description() -> Error {
    Error::new(
        ErrorKind::XmpUnreadable,
        "the sidecar has no rdf:Description to write into, so it was left alone",
    )
}

/// An attribute value with the entities this tool writes decoded again, so two
/// spellings of the same text compare equal.
fn unescape(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        // Last, so `&amp;quot;` does not become a quote.
        .replace("&amp;", "&")
}

/// Where a frame's sidecar lives: beside it, with the extension replaced.
///
/// `DSC_0001.NEF` → `DSC_0001.xmp`, never `DSC_0001.NEF.xmp`: the doubled name
/// belongs to darktable, and writing it would put our rating inside another
/// application's history file.
pub fn path_for(frame: &std::path::Path) -> std::path::PathBuf {
    frame.with_extension("xmp")
}

/// What a write did, which is what decides whether a `.bak` was owed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Written {
    /// There was no sidecar and there is one now.
    Created,
    /// There was a sidecar and it now holds this run's document.
    Replaced,
}

/// The port a run writes sidecars through: production writes files, tests write
/// what they are told to.
pub trait Writer {
    /// Write `document` for `target`, given the document that is there now
    /// (`None` when the sidecar is new). The existing text is passed in rather
    /// than read again: the run has already read it to decide whether to write
    /// at all, and a `.bak` has to hold exactly what was there.
    fn write(
        &self,
        target: &std::path::Path,
        document: &str,
        existing: Option<&str>,
    ) -> Result<Written>;
}

/// The writer a run uses: atomic, permission-preserving, and sparing with
/// backups.
#[derive(Debug, Clone, Copy, Default)]
pub struct Filesystem;

impl Writer for Filesystem {
    fn write(
        &self,
        target: &std::path::Path,
        document: &str,
        existing: Option<&str>,
    ) -> Result<Written> {
        if let Some(previous) = existing {
            back_up(target, previous)?;
        }
        let folder = target.parent().unwrap_or_else(|| std::path::Path::new("."));
        let mut temp = tempfile::Builder::new()
            .prefix(".imagecull-")
            .suffix(".tmp")
            .tempfile_in(folder)
            .map_err(|error| failed(target, error))?;
        {
            use std::io::Write as _;
            temp.write_all(document.as_bytes())
                .map_err(|error| failed(target, error))?;
            temp.flush().map_err(|error| failed(target, error))?;
        }
        // The file's permissions are the photographer's, not ours: a sidecar
        // that was readable only by its owner stays that way.
        let replaced = target.exists();
        if replaced
            && let Ok(metadata) = std::fs::metadata(target)
            && let Err(error) = temp.as_file().set_permissions(metadata.permissions())
        {
            return Err(failed(target, error));
        }
        temp.persist(target)
            .map_err(|error| failed(target, error.error))?;
        Ok(if replaced {
            Written::Replaced
        } else {
            Written::Created
        })
    }
}

/// Keep the previous document beside the sidecar, once: a run that rewrites a
/// sidecar owes the photographer the text it replaced, and a second run must not
/// overwrite the only copy.
fn back_up(target: &std::path::Path, previous: &str) -> Result<()> {
    let backup = backup_for(target);
    if backup.exists() {
        return Ok(());
    }
    std::fs::write(&backup, previous).map_err(|error| failed(&backup, error))
}

/// `DSC_0001.xmp` → `DSC_0001.xmp.bak`.
fn backup_for(target: &std::path::Path) -> std::path::PathBuf {
    let mut name = target.as_os_str().to_os_string();
    name.push(".bak");
    std::path::PathBuf::from(name)
}

/// A write that could not land: a frame-scoped failure with the path in it.
fn failed(target: &std::path::Path, error: std::io::Error) -> Error {
    Error::new(
        ErrorKind::XmpWriteFailed,
        format!("cannot write {}: {error}", target.display()),
    )
}

/// The `status` a frame carries when nothing went wrong.
pub const OK: &str = "ok";

/// Where a run writes what it still writes to a file: the raw answers, when
/// they are asked for. The ratings themselves are sidecars, not a report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outputs {
    /// The raw answer document of every frame (`--json-scores`), when asked for.
    pub json_scores: Option<PathBuf>,
}

/// One frame's account while the run goes, and the unit `--json-scores` is
/// written from.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// The file, as the scan found it.
    pub source: SourceFile,
    /// The answer at the precision the model gave it; `None` on a failed row.
    pub answers: Option<Answers>,
    /// What the request cost, when it got far enough to cost anything.
    pub usage: Option<Usage>,
    /// The raw answer document, kept for `--json-scores`. A `bad_response` row
    /// keeps its document even though its answers could not be read.
    pub document: Option<Value>,
    /// Why the row has no answers; `None` means it has them.
    pub failure: Option<Failure>,
    /// The first frame whose identical preview this row reuses: the frame that
    /// paid for the request this row's numbers came from.
    pub duplicate_of: Option<String>,
    /// The rubric's short hash: the row's provenance at a glance.
    pub rubric_hash: String,
    /// The alias actually used.
    pub model: String,
}

impl Row {
    /// The status this row would carry: `ok`, or the kind that cost it its
    /// values.
    pub fn status(&self) -> &'static str {
        self.failure
            .as_ref()
            .map_or(OK, |failure| failure.kind.as_str())
    }
}

/// A failed frame's status and message, kept together so they cannot disagree
/// about whether the frame failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The taxonomy kind.
    pub kind: ErrorKind,
    /// A short message that explains the frame without a log file.
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

/// Write `--json-scores`, when the run asked for it: the raw documents, once, at
/// the end. The documents are the debugging record now that nothing else keeps
/// the model's own words.
pub fn write_json(rows: &[Row], outputs: &Outputs) -> Result<()> {
    if let Some(path) = &outputs.json_scores {
        let text = json(rows);
        fs::write(path, text.as_bytes())
            .map_err(|error| Error::fatal(format!("cannot write {}: {error}", path.display())))?;
    }
    Ok(())
}

/// The raw answer document of every row, keyed by the frame's path inside the
/// shoot folder, `null` for a frame the model never answered.
fn json(rows: &[Row]) -> String {
    let mut documents = Map::new();
    for row in rows {
        documents.insert(
            row.source.relative.to_string_lossy().into_owned(),
            row.document.clone().unwrap_or(Value::Null),
        );
    }
    let mut out = serde_json::to_string_pretty(&Value::Object(documents))
        .unwrap_or_else(|_| "{}".to_string());
    out.push('\n');
    out
}

/// The rating a sidecar already carries, when it carries one a photographer
/// would recognise: an integer 1–5 in the first description.
///
/// Anything else — absent, empty, `0`, `-1`, a number that is not an integer —
/// is not a rating this tool defers to, and is not a rating it produced either.
pub fn existing_rating(document: &str) -> Option<u8> {
    let start = document.find(DESCRIPTION)?;
    let end = tag_end(document, start)?;
    attributes(&document[start..end])
        .into_iter()
        .find(|attribute| attribute.name == "xmp:Rating")
        .and_then(|attribute| attribute.value.trim().parse::<u8>().ok())
        .filter(|rating| (1..=5).contains(rating))
}

/// Whether this document has a description we can safely write into.
///
/// Asked before a request is spent: a sidecar no tool can extend is a frame this
/// run cannot record, and finding that out after paying for the answer would be
/// paying twice for the same refusal.
pub fn can_extend(document: &str) -> bool {
    document
        .find(DESCRIPTION)
        .and_then(|start| tag_end(document, start))
        .is_some()
}

/// Whether another application owns this sidecar: darktable records its history
/// in one, and rewrites the whole file the next time it is opened. Writing into
/// it is allowed only when the photographer says so.
pub fn is_foreign(document: &str) -> bool {
    document.contains("xmlns:darktable") || document.contains("darktable:")
}

impl Properties {
    /// This run's properties for one frame, stamped now with the alias and the
    /// rubric that produced the rating.
    pub fn now(rating: u8, conf_stars: f64, model: &str, rubric_hash: &str) -> Properties {
        Properties {
            rating,
            conf_stars,
            model: model.to_string(),
            rubric_hash: rubric_hash.to_string(),
            scored_at: scored_at_now(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

/// The current time, RFC 3339 in UTC.
pub fn scored_at_now() -> String {
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
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    (
        year,
        u64::try_from(month).unwrap_or(0),
        u64::try_from(day).unwrap_or(0),
    )
}
