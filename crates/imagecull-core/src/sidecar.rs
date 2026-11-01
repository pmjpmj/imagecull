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

use crate::error::{Error, ErrorKind, Result};

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
