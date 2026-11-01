//! Writing a comma-separated row, for the one report this tool still writes.
//!
//! Ratings go into sidecars; the measures CSV `imagecull groups` writes is the
//! remaining tabular output, and it needs the same RFC 4180 care a filename with
//! a comma has always needed. One writer, so a name is quoted the same way
//! wherever it appears.

/// Append one CSV record, escaping the cells that need it and ending the line
/// with `\n`.
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
