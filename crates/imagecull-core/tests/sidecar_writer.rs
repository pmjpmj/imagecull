//! The writer: where a sidecar goes, how it lands, and what is kept of what was
//! there before.
//!
//! Everything here is observed through the filesystem a photographer would see
//! next to their raws: the file that appeared, the file that did not, the `.bak`
//! that holds the previous text, and the temp file that must never survive.

use std::fs;
use std::path::{Path, PathBuf};

use imagecull_core::sidecar::{self, Filesystem, Writer, Written};
use tempfile::TempDir;

const DOC: &str = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/>\n";

fn shoot() -> TempDir {
    tempfile::tempdir().expect("a temp dir")
}

/// A frame in `dir`, as the scan would have found it.
fn frame(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, b"raw bytes").expect("write the frame");
    path
}

/// The names in `dir`, sorted, so "nothing else was written" is assertable.
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("read the folder")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn the_sidecar_sits_beside_the_frame_with_the_extension_replaced() {
    for (frame, expected) in [
        ("DSC_0001.NEF", "DSC_0001.xmp"),
        ("dsc_0002.nef", "dsc_0002.xmp"),
        ("DSC_0003.NRW", "DSC_0003.xmp"),
        ("DSC_0004.dng", "DSC_0004.xmp"),
    ] {
        assert_eq!(
            sidecar::path_for(Path::new(frame)),
            PathBuf::from(expected),
            "{frame}"
        );
    }
}

#[test]
fn a_new_sidecar_is_written_beside_its_frame_and_no_temp_file_survives() {
    let dir = shoot();
    let path = frame(dir.path(), "DSC_0001.NEF");

    let written = Filesystem.write(&path, DOC, None).expect("write");

    assert_eq!(written, Written::Created);
    assert_eq!(
        fs::read_to_string(dir.path().join("DSC_0001.xmp")).expect("the sidecar"),
        DOC
    );
    assert_eq!(
        names(dir.path()),
        ["DSC_0001.NEF", "DSC_0001.xmp"],
        "exactly one new file, and nothing left half-written"
    );
}

#[test]
fn replacing_a_sidecar_keeps_its_permissions() {
    let dir = shoot();
    let path = frame(dir.path(), "DSC_0001.NEF");
    let sidecar = sidecar::path_for(&path);
    fs::write(&sidecar, "old").expect("an old sidecar");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&sidecar, fs::Permissions::from_mode(0o600)).expect("a mode");
    }

    let written = Filesystem.write(&path, DOC, Some("old")).expect("write");

    assert_eq!(written, Written::Replaced);
    assert_eq!(fs::read_to_string(&sidecar).expect("the sidecar"), DOC);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&sidecar)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode, 0o600,
            "the file's permissions are the user's, not ours"
        );
    }
}

#[test]
fn an_existing_sidecar_is_backed_up_once_and_never_again() {
    let dir = shoot();
    let path = frame(dir.path(), "DSC_0001.NEF");
    let sidecar = sidecar::path_for(&path);
    fs::write(&sidecar, "the original").expect("an old sidecar");

    Filesystem
        .write(&path, DOC, Some("the original"))
        .expect("write");
    assert_eq!(
        fs::read_to_string(sidecar.with_extension("xmp.bak")).expect("a backup"),
        "the original"
    );

    Filesystem
        .write(&path, DOC, Some("the original"))
        .expect("write again");
    assert_eq!(
        fs::read_to_string(sidecar.with_extension("xmp.bak")).expect("the backup"),
        "the original",
        "a second run must not destroy the only copy of what was there first"
    );
}

#[test]
fn a_new_sidecar_is_not_backed_up() {
    let dir = shoot();
    let path = frame(dir.path(), "DSC_0001.NEF");

    Filesystem.write(&path, DOC, None).expect("write");

    assert_eq!(
        names(dir.path()),
        ["DSC_0001.NEF", "DSC_0001.xmp"],
        "a first run leaves no .bak clutter beside frames it only created"
    );
}

#[test]
fn a_write_that_cannot_land_is_an_error_and_leaves_nothing_behind() {
    let dir = shoot();
    let path = dir.path().join("no-such-folder").join("DSC_0001.NEF");

    let error = Filesystem
        .write(&path, DOC, None)
        .expect_err("a folder that is not there");

    assert!(!sidecar::path_for(&path).exists());
    assert!(
        !error.message().is_empty(),
        "the error says what went wrong"
    );
}
