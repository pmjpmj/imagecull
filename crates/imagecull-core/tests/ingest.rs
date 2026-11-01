//! What a shoot folder scan accepts, ignores, and in what order.
//!
//! Every test builds a throwaway directory, runs the scan over it, and asserts
//! on the files and counts a run would see — nothing about how the scan walks.

use std::fs;
use std::path::Path;

use imagecull_core::ingest::{self, ScanOptions};
use tempfile::TempDir;

/// An empty shoot folder.
fn shoot() -> TempDir {
    tempfile::tempdir().expect("a temp dir")
}

/// A file of one byte at `relative`, creating parents as needed.
fn touch(dir: &Path, relative: &str) {
    let path = dir.join(relative);
    let parent = path.parent().expect("a parent directory");
    fs::create_dir_all(parent).expect("create parent");
    fs::write(&path, b"x").expect("write file");
}

/// The accepted files, as relative paths in scan order.
fn accepted(scan: &ingest::Scan) -> Vec<String> {
    scan.files
        .iter()
        .map(|file| file.relative.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn raw_extensions_are_accepted_in_any_case_and_the_rest_are_counted() {
    let dir = shoot();
    for name in [
        "a.NEF", "b.nrw", "c.DnG", "d.nef.bak", "e.jpg", "f.MOV", "notes.txt",
    ] {
        touch(dir.path(), name);
    }

    let scan = ingest::scan(dir.path(), &ScanOptions::default()).expect("scan");

    assert_eq!(accepted(&scan), ["a.NEF", "b.nrw", "c.DnG"]);
    assert_eq!(
        scan.counts.skipped_extension, 4,
        "every non-RAW file is skipped and counted, never an error"
    );
}

#[test]
fn finder_noise_is_ignored_and_counted_and_hidden_directories_are_never_descended() {
    let dir = shoot();
    for name in [
        "keep.NEF",
        "._keep.NEF",
        ".DS_Store",
        "__MACOSX/._keep.NEF",
        "__MACOSX/keep.NEF",
        ".trash/cached.NEF",
        "sub/nested.NEF",
    ] {
        touch(dir.path(), name);
    }

    let scan = ingest::scan(dir.path(), &ScanOptions::default()).expect("scan");

    assert_eq!(accepted(&scan), ["keep.NEF"]);
    assert_eq!(
        scan.counts.ignored, 4,
        "AppleDouble, .DS_Store and the two files under __MACOSX are ignored and counted"
    );
    assert_eq!(
        scan.counts.hidden_directories, 1,
        ".trash is hidden and never descended; __MACOSX is counted as ignored instead"
    );
    assert_eq!(
        scan.counts.skipped_extension, 0,
        "Finder noise is ignored, not counted as a foreign extension"
    );
}

#[test]
fn recursion_is_opt_in_and_still_never_descends_into_hidden_directories() {
    let dir = shoot();
    for name in ["a.NEF", "sub/nested.NEF", ".cache/deep.NEF", "sub/.hidden/x.NEF"] {
        touch(dir.path(), name);
    }

    let shallow = ingest::scan(dir.path(), &ScanOptions::default()).expect("scan");
    assert_eq!(accepted(&shallow), ["a.NEF"]);

    let deep = ingest::scan(
        dir.path(),
        &ScanOptions {
            recursive: true,
            ..ScanOptions::default()
        },
    )
    .expect("scan");

    assert_eq!(accepted(&deep), ["a.NEF", "sub/nested.NEF"]);
    assert_eq!(deep.counts.hidden_directories, 2, ".cache and sub/.hidden hold no shoot");
}

#[test]
fn the_limit_keeps_the_first_files_in_scan_order() {
    let dir = shoot();
    for name in ["c.NEF", "a.NEF", "b.NEF", "ignored.jpg"] {
        touch(dir.path(), name);
    }

    let scan = ingest::scan(
        dir.path(),
        &ScanOptions {
            limit: Some(2),
            ..ScanOptions::default()
        },
    )
    .expect("scan");

    assert_eq!(accepted(&scan), ["a.NEF", "b.NEF"]);
    assert_eq!(
        scan.counts.skipped_extension, 1,
        "the limit applies to accepted files, not to what the scan counts"
    );
}

#[test]
fn numbers_in_names_are_compared_as_numbers() {
    let dir = shoot();
    for name in ["DSC_10.NEF", "DSC_2.NEF", "DSC_100.NEF", "DSC_1.NEF"] {
        touch(dir.path(), name);
    }

    let scan = ingest::scan(dir.path(), &ScanOptions::default()).expect("scan");

    assert_eq!(
        accepted(&scan),
        ["DSC_1.NEF", "DSC_2.NEF", "DSC_10.NEF", "DSC_100.NEF"],
        "a frame's crop of 400 sorts between 99 and 500, not between 3 and 4"
    );
}

#[test]
fn names_are_compared_after_unicode_normalisation() {
    let dir = shoot();
    // macOS hands out decomposed names (NFD); a card reader hands out the
    // precomposed ones. Both spell the same filename, so both must sort alike.
    for name in ["A\u{301}.NEF", "\u{c0}.NEF"] {
        touch(dir.path(), name);
    }

    let scan = ingest::scan(dir.path(), &ScanOptions::default()).expect("scan");

    assert_eq!(
        accepted(&scan),
        ["\u{c0}.NEF", "A\u{301}.NEF"],
        "U+00C0 sorts before U+00C1; comparing raw bytes would put the decomposed Á first"
    );
}
