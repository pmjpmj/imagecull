//! Getting the picture a camera already embedded out of a RAW file.
//!
//! No RAW decode, no demosaic, no colour pipeline: the camera's own JPEG is the
//! thing the model judges (spec §"What the model sees"). This module reads it,
//! chooses among the several a file may hold, and reports the EXIF orientation
//! the display must apply.

use std::fmt;
use std::path::Path;

use raw_files::{Preview, RawFile};

use crate::error::{Error, ErrorKind, Result};

/// An embedded preview, exactly as the camera wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Embedded {
    /// The JPEG bytes, still in the orientation they were stored in.
    pub jpeg: Vec<u8>,
    /// The stored width, before the orientation is applied.
    pub width: u32,
    /// The stored height, before the orientation is applied.
    pub height: u32,
    /// The EXIF orientation a viewer must apply, 1–8.
    pub orientation: u16,
}

impl Embedded {
    /// The stored width times height: how much picture the preview holds.
    pub fn pixel_area(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

/// Read the largest embedded preview of the RAW file at `path`.
///
/// The choice is by **pixel area**, not byte count: files commonly carry a
/// small full-quality preview and a large heavily-compressed one, and the
/// pixels are what the model sees.
pub fn extract(path: &Path) -> Result<Embedded> {
    Ok(read(path)?.0)
}

/// The largest embedded preview and what the camera recorded about the frame,
/// from one open of the file.
///
/// Two things a run needs come out of the same header: the preview the model is
/// sent and the pixels the [`measure`](crate::measure)s are taken from, and the
/// capture time, body and exposure the [`group`](crate::group)ing gates read.
/// Reading it once is why a grouping pass costs one extraction per frame rather
/// than two.
pub fn read(path: &Path) -> Result<(Embedded, raw_files::Metadata)> {
    let name = file_name(path);
    let mut raw = RawFile::open(path).map_err(|error| unreadable(path, &name, &error))?;

    let preview = largest_by_area(raw.previews()).ok_or_else(|| {
        Error::new(
            ErrorKind::NoPreview,
            format!("{name} embeds no JPEG preview"),
        )
    })?;

    let jpeg = raw
        .read_preview(&preview)
        .map_err(|error| Error::new(ErrorKind::Unreadable, format!("{name}: {error}")))?;

    let size = preview.size();
    let embedded = Embedded {
        jpeg,
        width: size.width,
        height: size.height,
        orientation: preview.orientation().to_exif(),
    };
    Ok((embedded, raw.metadata().clone()))
}

/// The `unreadable` error for a file the extractor cannot open.
///
/// A `.dng` is the one case that needs more than the reader's own words: the
/// ingest rule accepts the extension (spec §"Ingest") so a Z body set to DNG is
/// counted rather than skipped, but `raw-files` has no DNG reader at all, so the
/// frame can never be scored and the library's message for it ("a TIFF file, but
/// not of a format this version recognises") names neither the cause nor the
/// way out. The row's `error` cell is all a photographer reads afterwards, so a
/// DNG says what v1 cannot do and what to do instead.
fn unreadable(path: &Path, name: &str, cause: &dyn fmt::Display) -> Error {
    if is_dng(path) {
        return Error::new(
            ErrorKind::Unreadable,
            format!(
                "{name}: this build cannot read DNG's embedded preview (raw-files has no DNG \
                 reader) — export a NEF or TIFF, or score the JPEG"
            ),
        );
    }
    Error::new(ErrorKind::Unreadable, format!("{name}: {cause}"))
}

/// Whether `path` carries the `.dng` extension, in any case the camera wrote.
fn is_dng(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("dng"))
}

/// The preview with the most pixels.
///
/// `raw-files` already lists previews smallest first by pixel area, but the
/// choice is spelled out here because it is an acceptance criterion, not an
/// upstream detail: the largest-by-bytes `Medium` preview must never win.
fn largest_by_area(previews: &[Preview]) -> Option<Preview> {
    previews.iter().copied().max_by_key(|preview| {
        let size = preview.size();
        u64::from(size.width) * u64::from(size.height)
    })
}

/// The file's name alone, for messages that end up in a CSV cell.
pub(crate) fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}
