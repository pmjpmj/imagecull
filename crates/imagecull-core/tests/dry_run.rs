mod support;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use imagecull_core::dry_run;
use imagecull_core::error::ErrorKind;
use imagecull_core::ingest::ScanOptions;
use imagecull_core::preprocess::Params;
use tempfile::TempDir;

/// A shoot folder, and a previews directory beside it (never inside it).
fn shoot() -> (TempDir, TempDir) {
    (
        tempfile::tempdir().expect("a shoot dir"),
        tempfile::tempdir().expect("a previews dir"),
    )
}

fn write(dir: &Path, name: &str, bytes: &[u8]) {
    let path = dir.join(name);
    fs::create_dir_all(path.parent().expect("a parent")).expect("create parent");
    fs::write(path, bytes).expect("write fixture");
}

/// A NEF-shaped fixture whose embedded preview is `width` by `height`.
fn nef_with_preview(orientation: u16, width: u32, height: u32) -> Vec<u8> {
    support::nef(orientation, &[support::jpeg(width, height, 80)])
}

/// The (size, modification time) of everything under `dir`, recursively.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, (u64, Option<SystemTime>)> {
    fn walk(dir: &Path, into: &mut BTreeMap<PathBuf, (u64, Option<SystemTime>)>) {
        for entry in fs::read_dir(dir).expect("read dir") {
            let entry = entry.expect("entry");
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).expect("metadata");
            if metadata.is_dir() {
                into.insert(path.clone(), (0, metadata.modified().ok()));
                walk(&path, into);
            } else {
                into.insert(path, (metadata.len(), metadata.modified().ok()));
            }
        }
    }
    let mut into = BTreeMap::new();
    walk(dir, &mut into);
    into
}

#[test]
fn every_accepted_file_gets_exactly_one_preview_and_the_folder_shape_is_mirrored() {
    let (shoot_dir, previews) = shoot();
    write(
        shoot_dir.path(),
        "DSC_0001.NEF",
        &nef_with_preview(1, 64, 48),
    );
    write(
        shoot_dir.path(),
        "sub/DSC_0002.NEF",
        &nef_with_preview(1, 32, 24),
    );
    write(shoot_dir.path(), "ignored.jpg", b"not a raw file");

    let export = dry_run::render_previews(
        shoot_dir.path(),
        previews.path(),
        &ScanOptions {
            recursive: true,
            ..ScanOptions::default()
        },
        &Params::default(),
    )
    .expect("dry run");

    assert_eq!(export.scan.files.len(), 2, "one row per accepted file");
    assert_eq!(export.frames.len(), 2);
    assert_eq!(export.written(), 2);
    assert_eq!(export.scan.skipped_extension, 1);

    let dsc1 = previews.path().join("DSC_0001.jpg");
    let dsc2 = previews.path().join("sub/DSC_0002.jpg");
    assert!(dsc1.is_file(), "{} was not written", dsc1.display());
    assert!(dsc2.is_file(), "{} was not written", dsc2.display());

    let first = export.frames[0].outcome.as_ref().expect("a preview");
    assert_eq!((first.width, first.height), (64, 48));
    assert_eq!(first.preview, dsc1);
    assert_eq!(first.jpeg_bytes, fs::metadata(&dsc1).expect("metadata").len() as usize);
}

#[test]
fn a_file_without_an_embedded_preview_is_classified_and_the_run_continues() {
    let (shoot_dir, previews) = shoot();
    write(shoot_dir.path(), "DSC_0001.NEF", &support::nef(1, &[]));
    write(
        shoot_dir.path(),
        "DSC_0002.NEF",
        &nef_with_preview(1, 32, 24),
    );

    let export = dry_run::render_previews(
        shoot_dir.path(),
        previews.path(),
        &ScanOptions::default(),
        &Params::default(),
    )
    .expect("a missing preview is a row, not a stopped run");

    assert_eq!(export.written(), 1);

    let failures: Vec<&dry_run::Frame> = export.failures().collect();
    assert_eq!(failures.len(), 1);
    let error = failures[0].outcome.as_ref().expect_err("a classified error");
    assert_eq!(error.kind(), Some(ErrorKind::NoPreview));
    assert!(
        error.message().contains("DSC_0001.NEF"),
        "the message names the file: {}",
        error.message()
    );
}

#[test]
fn garbage_and_truncated_files_are_classified_failures_not_panics() {
    let (shoot_dir, previews) = shoot();
    write(shoot_dir.path(), "garbage.NEF", b"nothing like a raw file");
    let full = nef_with_preview(1, 64, 48);
    // Cut before the directory can be read, then inside the preview's bytes.
    write(shoot_dir.path(), "headless.NEF", &full[..6]);
    write(shoot_dir.path(), "truncated.NEF", &full[..full.len() / 2]);
    write(
        shoot_dir.path(),
        "DSC_0003.NEF",
        &nef_with_preview(1, 32, 24),
    );

    let export = dry_run::render_previews(
        shoot_dir.path(),
        previews.path(),
        &ScanOptions::default(),
        &Params::default(),
    )
    .expect("damaged files are rows, not a stopped run");

    assert_eq!(export.written(), 1, "the good frame is still scored");
    let kinds: Vec<ErrorKind> = export
        .failures()
        .filter_map(|frame| frame.outcome.as_ref().err().and_then(|error| error.kind()))
        .collect();
    assert_eq!(
        kinds,
        [
            ErrorKind::Unreadable,
            ErrorKind::Unreadable,
            // `raw-files` drops a preview that runs past the end of the file, so
            // a truncated card copy looks like a file with no preview rather
            // than failing to parse. Still a classified row, still no panic.
            ErrorKind::NoPreview,
        ],
        "in file order: garbage, a cut header, a cut preview"
    );
}

#[test]
fn the_shoot_folder_is_not_touched() {
    let (shoot_dir, previews) = shoot();
    write(
        shoot_dir.path(),
        "DSC_0001.NEF",
        &nef_with_preview(6, 64, 48),
    );
    write(shoot_dir.path(), "DSC_0002.NEF", &support::nef(1, &[]));
    write(shoot_dir.path(), "notes.txt", b"keep out");

    let before = snapshot(shoot_dir.path());
    dry_run::render_previews(
        shoot_dir.path(),
        previews.path(),
        &ScanOptions::default(),
        &Params::default(),
    )
    .expect("dry run");
    let after = snapshot(shoot_dir.path());

    assert_eq!(
        before, after,
        "a dry run writes nothing into the folder it scored"
    );
}

#[test]
fn a_previews_directory_inside_the_shoot_folder_is_refused() {
    let (shoot_dir, _) = shoot();
    write(
        shoot_dir.path(),
        "DSC_0001.NEF",
        &nef_with_preview(1, 64, 48),
    );

    let error = dry_run::render_previews(
        shoot_dir.path(),
        &shoot_dir.path().join("previews"),
        &ScanOptions::default(),
        &Params::default(),
    )
    .expect_err("writing previews into the shot is never allowed");

    assert!(error.is_fatal());
    assert!(
        error.message().contains("inside the shoot folder"),
        "{}",
        error.message()
    );
    assert!(!shoot_dir.path().join("previews").exists());
}

#[test]
fn the_limit_shortens_the_run() {
    let (shoot_dir, previews) = shoot();
    for name in ["DSC_0001.NEF", "DSC_0002.NEF", "DSC_0003.NEF"] {
        write(shoot_dir.path(), name, &nef_with_preview(1, 32, 24));
    }

    let export = dry_run::render_previews(
        shoot_dir.path(),
        previews.path(),
        &ScanOptions {
            limit: Some(1),
            ..ScanOptions::default()
        },
        &Params::default(),
    )
    .expect("dry run");

    assert_eq!(export.written(), 1);
    assert!(previews.path().join("DSC_0001.jpg").is_file());
    assert!(!previews.path().join("DSC_0002.jpg").exists());
}

#[test]
fn the_cap_and_quality_flags_reach_the_previews() {
    let (shoot_dir, previews) = shoot();
    write(
        shoot_dir.path(),
        "DSC_0001.NEF",
        &support::nef(1, &[support::jpeg(400, 200, 80)]),
    );

    let export = dry_run::render_previews(
        shoot_dir.path(),
        previews.path(),
        &ScanOptions::default(),
        &Params {
            max_edge: 100,
            jpeg_quality: 40,
        },
    )
    .expect("dry run");

    let written = export.frames[0].outcome.as_ref().expect("a preview");
    assert_eq!((written.width, written.height), (100, 50));

    let decoded = image::load_from_memory(&fs::read(&written.preview).expect("read"))
        .expect("decode")
        .to_rgb8();
    assert_eq!(decoded.dimensions(), (100, 50), "the file on disk is capped too");
}
