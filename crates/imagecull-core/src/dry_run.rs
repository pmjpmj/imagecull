//! `score <DIR> --dry-run`: render the bytes a scoring run would send, and
//! nothing else.
//!
//! This is the whole pipeline minus the model: scan the folder, extract each
//! file's largest embedded preview, orient it, downscale it, encode it, and
//! write it into a previews directory so the photographer can eyeball
//! orientation, crop and detail loss before committing an hour to a real run
//! (spec §"What the model sees"). No HTTP, no ledger, no CSV, and nothing
//! written inside the shoot folder.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::ingest::{self, Scan, ScanOptions, SourceFile};
use crate::preprocess::{self, Params};
use crate::preview;

/// What a dry run did.
#[derive(Debug)]
pub struct Export {
    /// Where the previews were written.
    pub previews: PathBuf,
    /// What the scan found, including what it ignored and skipped.
    pub scan: Scan,
    /// One entry per accepted file, in ingest order.
    pub frames: Vec<Frame>,
}

impl Export {
    /// How many previews were written.
    pub fn written(&self) -> usize {
        self.frames
            .iter()
            .filter(|frame| frame.outcome.is_ok())
            .count()
    }

    /// The per-file failures, in ingest order.
    pub fn failures(&self) -> impl Iterator<Item = &Frame> {
        self.frames
            .iter()
            .filter(|frame| frame.outcome.is_err())
    }
}

/// What happened to one file.
#[derive(Debug)]
pub struct Frame {
    /// The file it happened to.
    pub source: SourceFile,
    /// The preview written, or the classified reason there is none.
    pub outcome: Result<Written>,
}

/// A preview that reached the previews directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    /// Where it was written.
    pub preview: PathBuf,
    /// Its width, after the orientation was applied and it was downscaled.
    pub width: u32,
    /// Its height, after the orientation was applied and it was downscaled.
    pub height: u32,
    /// How many bytes the JPEG takes.
    pub jpeg_bytes: usize,
}

/// Render every accepted file of `shoot` into `previews`.
///
/// The run continues past a file that cannot be read or that carries no
/// preview: one odd file must not cost the photographer the other 399.
pub fn render_previews(
    shoot: &Path,
    previews: &Path,
    options: &ScanOptions,
    params: &Params,
) -> Result<Export> {
    let scan = ingest::scan(shoot, options)?;
    guard_outside_shoot(shoot, previews)?;
    fs::create_dir_all(previews).map_err(|error| {
        Error::fatal(format!(
            "cannot create the previews directory {}: {error}",
            previews.display()
        ))
    })?;

    let mut frames = Vec::with_capacity(scan.files.len());
    for source in &scan.files {
        let outcome = render_one(source, previews, params);
        frames.push(Frame {
            source: source.clone(),
            outcome,
        });
    }
    Ok(Export {
        previews: previews.to_path_buf(),
        scan,
        frames,
    })
}

/// Extract, orient, downscale, encode and write one file's preview.
fn render_one(source: &SourceFile, previews: &Path, params: &Params) -> Result<Written> {
    let embedded = preview::extract(&source.path)?;
    let encoded = preprocess::render(&embedded, params)?;

    let jpeg = encoded.jpeg;
    let target = preview_path(previews, &source.relative);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            Error::fatal(format!(
                "cannot create the previews directory {}: {error}",
                parent.display()
            ))
        })?;
    }
    fs::write(&target, &jpeg).map_err(|error| {
        Error::fatal(format!(
            "cannot write the preview {}: {error}",
            target.display()
        ))
    })?;

    Ok(Written {
        preview: target,
        width: encoded.width,
        height: encoded.height,
        jpeg_bytes: jpeg.len(),
    })
}

/// Where a file's preview goes: the same name under the previews directory,
/// with the RAW extension replaced, so subfolders stay apart and the
/// photographer can find the frame the numbers are about.
fn preview_path(previews: &Path, relative: &Path) -> PathBuf {
    let mut target = previews.join(relative);
    target.set_extension("jpg");
    target
}

/// Refuse a previews directory that lies inside the shoot folder: a dry run
/// must not write into the shot, not even sneakily.
fn guard_outside_shoot(shoot: &Path, previews: &Path) -> Result<()> {
    let shoot = shoot.canonicalize().map_err(|error| {
        Error::fatal(format!(
            "cannot read the shoot folder {}: {error}",
            shoot.display()
        ))
    })?;
    let previews = absolute(previews)?;
    if previews.starts_with(&shoot) {
        return Err(Error::fatal(format!(
            "the previews directory {} is inside the shoot folder {} — imagecull never writes \
             into the folder it scores; pass a previews directory outside it",
            previews.display(),
            shoot.display()
        )));
    }
    Ok(())
}

/// `path` as an absolute path, with its nearest existing ancestor resolved, so
/// that a directory that does not exist yet can still be compared.
fn absolute(path: &Path) -> Result<PathBuf> {
    let mut existing = path;
    loop {
        if let Ok(mut resolved) = existing.canonicalize() {
            let rest = path.strip_prefix(existing).unwrap_or(Path::new(""));
            resolved.push(rest);
            return Ok(resolved);
        }
        existing = existing.parent().ok_or_else(|| {
            Error::fatal(format!(
                "cannot resolve the previews directory {}",
                path.display()
            ))
        })?;
    }
}
