//! Finding the RAW files in a shoot folder.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// One accepted RAW file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    /// Where the file is on disk.
    pub path: PathBuf,
    /// Where it is inside the scanned folder: the name a row reports.
    pub relative: PathBuf,
}

/// What one scan found.
#[derive(Debug, Clone, Default)]
pub struct Scan {
    /// The accepted files, in ingest order.
    pub files: Vec<SourceFile>,
    /// How many files were skipped because their extension is not a RAW one.
    pub skipped_extension: usize,
    /// How many files were ignored as Finder noise: `._*` AppleDouble files,
    /// `.DS_Store`, and anything under a `__MACOSX` directory.
    pub ignored: usize,
    /// How many directories were not descended into because their name starts
    /// with a dot.
    pub hidden_directories: usize,
}

/// How to scan a folder.
#[derive(Debug, Clone, Default)]
pub struct ScanOptions {
    /// Descend into subdirectories. Hidden ones are never descended either way.
    pub recursive: bool,
    /// Keep only the first `N` accepted files, after ordering: a smoke test.
    pub limit: Option<usize>,
}

/// Scan `dir` for the RAW files a run should score.
pub fn scan(dir: &Path, options: &ScanOptions) -> Result<Scan> {
    let mut scan = Scan::default();
    collect(dir, dir, options, &mut scan)?;
    order(&mut scan.files);
    if let Some(limit) = options.limit {
        scan.files.truncate(limit);
    }
    Ok(scan)
}

/// Put the files in the order every later stage reports them in: natural, so
/// `DSC_9` precedes `DSC_10`, and Unicode-normalised to NFC first, so the same
/// frame from a card reader and from the Finder lands in the same place.
fn order(files: &mut [SourceFile]) {
    let mut keyed: Vec<((String, String), SourceFile)> = files
        .iter()
        .map(|file| {
            let name = file
                .relative
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let relative = file.relative.to_string_lossy().into_owned();
            ((nfc(&name), nfc(&relative)), file.clone())
        })
        .collect();
    keyed.sort_by(|(left, _), (right, _)| {
        natural_cmp(&left.0, &right.0).then_with(|| natural_cmp(&left.1, &right.1))
    });
    for (slot, (_, file)) in files.iter_mut().zip(keyed) {
        *slot = file;
    }
}

/// `text` in Unicode Normalization Form C.
fn nfc(text: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    text.nfc().collect()
}

/// Compare two names so that a run of digits counts as the number it spells.
///
/// Digits compare by value first (so `9` precedes `10`), then by how they are
/// written (so `2` precedes `02`); everything else compares as text.
fn natural_cmp(left: &str, right: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    let left_chunks = chunks(left);
    let right_chunks = chunks(right);
    for (left_chunk, right_chunk) in left_chunks.iter().zip(right_chunks.iter()) {
        let ordering = match (left_chunk, right_chunk) {
            ((true, left_digits), (true, right_digits)) => {
                let left_value = left_digits.trim_start_matches('0');
                let right_value = right_digits.trim_start_matches('0');
                left_value
                    .len()
                    .cmp(&right_value.len())
                    .then_with(|| left_value.cmp(right_value))
                    .then_with(|| left_digits.len().cmp(&right_digits.len()))
            }
            ((_, left_text), (_, right_text)) => left_text.cmp(right_text),
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    left_chunks
        .len()
        .cmp(&right_chunks.len())
        .then_with(|| left.cmp(right))
}

/// `text` split into runs of ASCII digits and runs of everything else, each
/// marked with which it is. A multi-byte character never splits: only ASCII
/// bytes start or end a run.
fn chunks(text: &str) -> Vec<(bool, &str)> {
    let bytes = text.as_bytes();
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < bytes.len() {
        let digits = bytes[start].is_ascii_digit();
        let mut end = start + 1;
        while end < bytes.len() && bytes[end].is_ascii_digit() == digits {
            end += 1;
        }
        chunks.push((digits, &text[start..end]));
        start = end;
    }
    chunks
}

/// Add the accepted files of `dir` to `scan`, descending when asked to.
fn collect(dir: &Path, base: &Path, options: &ScanOptions, scan: &mut Scan) -> Result<()> {
    for entry in read_dir(dir)? {
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = entry.file_type().is_ok_and(|file_type| file_type.is_dir());
        if is_dir {
            if name == MACOSX_DIRECTORY {
                scan.ignored += count_files(&entry.path())?;
            } else if name.starts_with('.') {
                scan.hidden_directories += 1;
            } else if options.recursive {
                collect(&entry.path(), base, options, scan)?;
            }
            continue;
        }
        if is_finder_noise(&name) {
            scan.ignored += 1;
        } else if has_accepted_extension(&name) {
            let relative = entry.path().strip_prefix(base).map_or_else(
                |_| PathBuf::from(&name),
                std::path::Path::to_path_buf,
            );
            scan.files.push(SourceFile {
                path: entry.path(),
                relative,
            });
        } else {
            scan.skipped_extension += 1;
        }
    }
    Ok(())
}

/// How many files, at any depth, lie under `dir`.
fn count_files(dir: &Path) -> Result<usize> {
    let mut count = 0;
    for entry in read_dir(dir)? {
        if entry.file_type().is_ok_and(|file_type| file_type.is_dir()) {
            count += count_files(&entry.path())?;
        } else {
            count += 1;
        }
    }
    Ok(count)
}

/// Whether `name` is noise macOS writes into a folder rather than a file a
/// camera or an editor made.
fn is_finder_noise(name: &str) -> bool {
    name.starts_with("._") || name == ".DS_Store"
}

/// The entries of `dir`, or the fatal error explaining why there are none.
fn read_dir(dir: &Path) -> Result<Vec<fs::DirEntry>> {
    let entries = fs::read_dir(dir).map_err(|error| {
        Error::fatal(format!("cannot read {}: {error}", dir.display()))
    })?;
    entries
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| Error::fatal(format!("cannot read {}: {error}", dir.display())))
}

/// Whether the file name carries one of the accepted extensions, in any case.
fn has_accepted_extension(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ACCEPTED_EXTENSIONS
                .iter()
                .any(|accepted| extension.eq_ignore_ascii_case(accepted))
        })
}

/// The extensions a run accepts, lower case and without the dot.
pub const ACCEPTED_EXTENSIONS: [&str; 3] = ["nef", "nrw", "dng"];

/// The directory macOS copies resources into; nothing under it is a photo.
const MACOSX_DIRECTORY: &str = "__MACOSX";
