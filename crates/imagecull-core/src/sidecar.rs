//! The XMP sidecar beside a frame: the rating, the provenance of it, and the
//! group verdict that marks a frame as one it is not keeping.
//!
//! One job, one function per write: an existing document (or none) plus what
//! this run decided in, the text to write out — or a classified refusal.
//! Nothing here opens a file: the writer is a port and the run owns it. A run
//! writes a frame twice — its rating as it is judged, and the group's verdict
//! once the group is complete — and the two writes never touch each other's
//! properties.
//!
//! The merge is a targeted text edit rather than a parse and re-serialise,
//! because the requirement is that every byte we do not own survives unchanged:
//! develop settings, keywords, GPS and another application's history are not
//! ours to reorder, re-quote or drop. So there is no XML dependency here, only
//! enough scanning to find the first `rdf:Description` opening tag, the
//! attributes in it, and where its attributes end.

use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

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

/// The label written on a frame that lost its group.
///
/// Red is the only label this tool ever writes: it says "not this group's
/// keeper", never "bad". A 4★ frame that lost to a 5★ is correctly red.
pub const RED: &str = "Red";

/// The note a frame carries when it was left out of its group's comparison:
/// the request could not carry every frame, and this one was not compared.
pub const NOT_COMPARED: &str = "not_compared";

/// The value of `imagecull:labeledBy` on a frame whose label this tool wrote:
/// the proof a later run needs before it may touch that label, and the only
/// thing that makes a red attributable to us rather than to the photographer.
pub const LABELED_BY: &str = "imagecull";

/// The name the proof that a label is ours is written under.
const LABELED_BY_NAME: &str = "imagecull:labeledBy";

/// One property a write owns: the name it is written under, and the value it
/// carries.
type Property = (&'static str, String);

/// Every property name this build writes, learned from the two writers over a
/// value that is only a shape — never written, never merged, only asked what it
/// would write.
///
/// A merge retires the `imagecull:` names missing from it — ours are ours to
/// retire — and leaves every name in it that the write in hand does not own
/// exactly as it found it. That is what keeps a re-rating from erasing a verdict,
/// and a verdict from inventing a rating, and deriving it here is what keeps a
/// property added to a writer from needing a second list beside it.
fn written() -> &'static [&'static str] {
    static NAMES: OnceLock<Vec<&'static str>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let (rating, verdict) = shape();
        names_of(rating).chain(names_of(verdict)).collect()
    })
}

/// The verdict names a mark write may retire: the ones it owns, except
/// `imagecull:labeledBy`, which is written whenever a label is written and left
/// in place whenever the label is left in place — a red without it can never be
/// attributed to us again.
fn retirable_by_mark() -> &'static [&'static str] {
    static NAMES: OnceLock<Vec<&'static str>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let (_, verdict) = shape();
        names_of(verdict)
            .filter(|name| name.starts_with("imagecull:") && *name != LABELED_BY_NAME)
            .collect()
    })
}

/// A rating and a verdict as a value that is only a shape: every property the
/// writers produce, with nothing in the values. It is what asks the writers for
/// their names without a second list that could drift from them.
fn shape() -> (Vec<Property>, Vec<Property>) {
    let properties = Properties {
        rating: 0,
        conf_stars: 0.0,
        model: String::new(),
        rubric_hash: String::new(),
        scored_at: String::new(),
        version: String::new(),
    };
    let mark = Mark {
        // A label and a note are set, so the properties only a marked or a
        // narrowed frame carries are named too.
        label: Some(RED),
        keeper: false,
        group_start: String::new(),
        group_size: 0,
        grouping_hash: String::new(),
        pick_by: String::new(),
        pick_conf: 0.0,
        picked_at: String::new(),
        note: Some(NOT_COMPARED),
    };
    (our_properties(&properties), mark_properties(&mark))
}

/// The names of a write's properties, in the order the write lists them.
fn names_of(properties: Vec<Property>) -> impl Iterator<Item = &'static str> {
    properties.into_iter().map(|(name, _)| name)
}

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

/// One frame's place in its group's verdict, as the sidecar records it.
///
/// The verdict is not a copy of the label: the label can be missing (the frame's
/// colour is not ours to touch) or carry the photographer's own choice, and the
/// verdict still exists. That is what lets a summary say `12 red, 9 marked, 3
/// unmarked` about one grouping.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    /// The label this write puts on the frame, or `None` when it writes none:
    /// on the keeper, whose rating is its mark, and on a frame whose colour
    /// belongs to the photographer.
    pub label: Option<&'static str>,
    /// Whether this frame is the group's keeper.
    pub keeper: bool,
    /// When the group's first frame was taken, as the camera's clock read it.
    pub group_start: String,
    /// How many frames the group holds.
    pub group_size: usize,
    /// The grouping thresholds the group was computed under.
    pub grouping_hash: String,
    /// What chose the keeper.
    pub pick_by: String,
    /// How sure it was: the top-option probability of the comparison's answer.
    pub pick_conf: f64,
    /// When the verdict was reached, RFC 3339 in UTC.
    pub picked_at: String,
    /// Why this frame was not compared, when it was not.
    pub note: Option<&'static str>,
}

/// The document to write for one frame's rating: `existing` when there is one to
/// merge into, `None` when the sidecar is new.
pub fn render(existing: Option<&str>, properties: &Properties) -> Result<String> {
    let owned = our_properties(properties);
    match existing {
        None => Ok(new_document(&owned)),
        // A rating write owns every property it produces, so it has nothing of
        // its own to retire — and the verdict beside it is not its to touch.
        Some(text) => merge(text, &owned, &[]),
    }
}

/// The document to write for one frame's group verdict, merged into the document
/// its rating was written to.
///
/// The rating document is an argument rather than an option: a verdict is only
/// ever added to a frame this run has just rated, so a merge with nothing to
/// merge into is a caller's bug and not a document this module has to invent.
pub fn render_mark(rated: &str, mark: &Mark) -> Result<String> {
    merge(rated, &mark_properties(mark), retirable_by_mark())
}

/// A whole new sidecar for a frame that has none, holding `owned` and nothing
/// else.
fn new_document(owned: &[Property]) -> String {
    let mut document = format!(
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"imagecull {version}\">\n\
         \x20<rdf:RDF xmlns:rdf=\"{RDF_NAMESPACE}\">\n\
         \x20 <rdf:Description rdf:about=\"\"\n\
         \x20   xmlns:xmp=\"{XMP_NAMESPACE}\"\n\
         \x20   xmlns:imagecull=\"{IMAGECULL_NAMESPACE}\"",
        version = env!("CARGO_PKG_VERSION"),
    );
    for (name, value) in owned {
        document.push_str(&format!("\n    {name}=\"{}\"", escape(value)));
    }
    document.push_str("/>\n </rdf:RDF>\n</x:xmpmeta>\n");
    document
}

/// Merge `owned` into an existing document, leaving everything else exactly as
/// it was, and retiring the `imagecull:` names this write owns but did not
/// produce.
fn merge(text: &str, owned: &[Property], retirable: &[&str]) -> Result<String> {
    let start = text.find(DESCRIPTION).ok_or_else(no_description)?;
    let end = tag_end(text, start).ok_or_else(no_description)?;
    let tag = &text[start..end];
    let merged = merge_tag(tag, owned, retirable);
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
fn merge_tag(tag: &str, ours: &[Property], retirable: &[&str]) -> String {
    let attributes = attributes(tag);
    // Nothing to do when the document already says exactly this: a forced run
    // over an unchanged rating must not churn the file.
    if is_current(&attributes, ours, retirable) {
        return tag.to_string();
    }

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
    for (name, value) in ours {
        match attributes.iter().find(|attribute| attribute.name == *name) {
            Some(attribute) => edits.push(Edit::replace(
                attribute.value_start,
                attribute.value_end,
                escape(value),
            )),
            None => appended.push_str(&format!(" {name}=\"{}\"", escape(value))),
        }
    }

    // A property of ours this build no longer writes is not left behind, and
    // neither is one this write owns and did not produce: ours are ours to
    // retire. A property this build does write, and this write does not own, is
    // left exactly as it was.
    for attribute in &attributes {
        if retires(&attribute.name, ours, retirable) {
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

/// The rating properties this run writes, in the order a new document lists
/// them.
fn our_properties(properties: &Properties) -> Vec<Property> {
    vec![
        ("xmp:Rating", properties.rating.to_string()),
        ("imagecull:confStars", properties.conf_stars.to_string()),
        ("imagecull:model", properties.model.clone()),
        ("imagecull:rubricHash", properties.rubric_hash.clone()),
        ("imagecull:scoredAt", properties.scored_at.clone()),
        ("imagecull:version", properties.version.clone()),
    ]
}

/// The properties a group's verdict writes: the label, when it is ours to write,
/// then the facts that explain it.
fn mark_properties(mark: &Mark) -> Vec<Property> {
    let mut owned = Vec::with_capacity(10);
    if let Some(label) = mark.label {
        owned.push(("xmp:Label", label.to_string()));
        owned.push((LABELED_BY_NAME, LABELED_BY.to_string()));
    }
    owned.push(("imagecull:groupKeeper", mark.keeper.to_string()));
    owned.push(("imagecull:groupStart", mark.group_start.clone()));
    owned.push(("imagecull:groupSize", mark.group_size.to_string()));
    owned.push(("imagecull:groupingHash", mark.grouping_hash.clone()));
    owned.push(("imagecull:pickBy", mark.pick_by.clone()));
    owned.push(("imagecull:pickConf", mark.pick_conf.to_string()));
    owned.push(("imagecull:pickedAt", mark.picked_at.clone()));
    if let Some(note) = mark.note {
        owned.push(("imagecull:groupNote", note.to_string()));
    }
    owned
}

/// Whether the tag already carries exactly the properties this write would put
/// there, and none this write owns and did not produce: one list of what we own,
/// so a comparison and a write can never disagree.
fn is_current(attributes: &[Attribute], ours: &[Property], retirable: &[&str]) -> bool {
    ours.iter().all(|(name, expected)| {
        attributes
            .iter()
            .any(|attribute| attribute.name == *name && attribute.value == *expected)
    }) && !attributes
        .iter()
        .any(|attribute| retires(&attribute.name, ours, retirable))
}

/// Whether a merge retires `name`: a property of ours that this build does not
/// write at all, or one this write owns and did not produce.
fn retires(name: &str, ours: &[Property], retirable: &[&str]) -> bool {
    if !name.starts_with("imagecull:") || ours.iter().any(|(owned, _)| *owned == name) {
        return false;
    }
    retirable.contains(&name) || !written().contains(&name)
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
    /// Write `document` for `target`, given what it replaces and whose that was.
    /// The existing text is passed in rather than read again: the run has already
    /// read it to decide whether to write at all, and a `.bak` has to hold exactly
    /// what was there.
    fn write(&self, target: &std::path::Path, document: &str, prior: Prior<'_>) -> Result<Written>;
}

/// The document a write replaces, and whose it is.
///
/// A run writes a frame twice: its rating as it is judged, and its group's
/// verdict once the group is complete. The second write replaces the first, and
/// the photographer is owed a copy of neither version of that — a `.bak` beside
/// every frame of a first pass would be clutter the run itself made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prior<'a> {
    /// The frame has no sidecar yet.
    Absent,
    /// A sidecar that was there before this run touched the frame: replacing it
    /// owes the photographer a copy of what it held.
    Earlier(&'a str),
    /// A sidecar this run wrote a moment ago.
    ThisRun(&'a str),
}

impl<'a> Prior<'a> {
    /// Whatever was there when the run read it: the document this write
    /// replaces, owed a copy if it is replaced.
    pub fn earlier(existing: Option<&'a str>) -> Prior<'a> {
        match existing {
            Some(text) => Prior::Earlier(text),
            None => Prior::Absent,
        }
    }

    /// The text this write replaces, when there is any.
    pub fn text(&self) -> Option<&'a str> {
        match self {
            Prior::Absent => None,
            Prior::Earlier(text) | Prior::ThisRun(text) => Some(text),
        }
    }

    /// Whether replacing this document owes a `.bak`.
    pub fn owes_a_backup(&self) -> bool {
        matches!(self, Prior::Earlier(_))
    }
}

/// The writer a run uses: atomic, permission-preserving, and sparing with
/// backups.
#[derive(Debug, Clone, Copy, Default)]
pub struct Filesystem;

impl Writer for Filesystem {
    fn write(&self, frame: &std::path::Path, document: &str, prior: Prior<'_>) -> Result<Written> {
        let target = &path_for(frame);
        if prior.owes_a_backup()
            && let Some(previous) = prior.text()
        {
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
    description_attributes(document)
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

/// The properties this tool owns in a document, as name and text: what a preview
/// of a write shows, and what a merge compares.
pub fn properties_in(document: &str) -> Vec<(String, String)> {
    let Some(start) = document.find(DESCRIPTION) else {
        return Vec::new();
    };
    let Some(end) = tag_end(document, start) else {
        return Vec::new();
    };
    attributes(&document[start..end])
        .into_iter()
        .filter(|attribute| {
            attribute.name == "xmp:Rating"
                || attribute.name == "xmp:Label"
                || attribute.name.starts_with("imagecull:")
        })
        .map(|attribute| (attribute.name, attribute.value))
        .collect()
}

/// The label a document's first description carries, and whether this tool is the
/// one that wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExistingLabel {
    /// The value as the document spells it: `Red`, `Yellow`, a colour name a
    /// camera wrote.
    pub value: String,
    /// Whether `imagecull:labeledBy` says this tool put it there.
    pub ours: bool,
}

/// The label a document already carries, when it carries one.
pub fn existing_label(document: &str) -> Option<ExistingLabel> {
    let attributes = description_attributes(document);
    let value = attributes
        .iter()
        .find(|attribute| attribute.name == "xmp:Label")?
        .value
        .trim()
        .to_string();
    if value.is_empty() {
        return None;
    }
    let ours = attributes.iter().any(|attribute| {
        attribute.name == "imagecull:labeledBy" && attribute.value.trim() == LABELED_BY
    });
    Some(ExistingLabel { value, ours })
}

/// Whether a label may be written into this document: only when there is none
/// yet, or when the one there is this tool's own.
///
/// A photographer's Yellow is never overwritten and never removed: it is their
/// field, their eye and their cull. The run counts such a frame as unmarked and
/// says so, because a delete set that is quietly incomplete is worse than one
/// that admits it.
pub fn label_is_ours_to_write(existing: Option<&str>) -> bool {
    existing
        .and_then(existing_label)
        .is_none_or(|label| label.ours)
}

/// The attributes of a document's first description, empty when it has none.
fn description_attributes(document: &str) -> Vec<Attribute> {
    let Some(start) = document.find(DESCRIPTION) else {
        return Vec::new();
    };
    let Some(end) = tag_end(document, start) else {
        return Vec::new();
    };
    attributes(&document[start..end])
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
            scored_at: crate::time::now_rfc3339(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}
