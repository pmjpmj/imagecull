mod support;

use std::fs;
use std::path::Path;

use imagecull_core::doctor;
use imagecull_core::preprocess::Params;
use tempfile::TempDir;

fn shoot() -> TempDir {
    tempfile::tempdir().expect("a temp dir")
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = dir.join(name);
    fs::write(&path, bytes).expect("write fixture");
    path
}

#[test]
fn the_self_test_proves_the_pipeline_on_a_real_file() {
    let dir = shoot();
    let path = write(
        dir.path(),
        "DSC_0001.NEF",
        &support::nef(6, &[support::jpeg(400, 200, 80)]),
    );

    let check = doctor::check_file(&path, &Params::default());

    assert!(check.is_ok());
    let message = check.message();
    assert!(message.contains("DSC_0001.NEF"), "{message}");
    assert!(
        message.contains("400x200") && message.contains("orientation 6"),
        "the self-test reports the preview it found and how it will be turned: {message}"
    );
    assert!(
        message.contains("200x400"),
        "and the upright size that would be sent: {message}"
    );
}

#[test]
fn the_self_test_explains_what_is_wrong_with_a_bad_file() {
    let dir = shoot();
    let garbage = write(dir.path(), "garbage.NEF", b"nothing like a raw file");
    let empty = write(dir.path(), "empty.NEF", &support::nef(1, &[]));

    let garbage_check = doctor::check_file(&garbage, &Params::default());
    assert!(!garbage_check.is_ok());
    assert!(
        garbage_check.message().contains("unreadable"),
        "{}",
        garbage_check.message()
    );

    let empty_check = doctor::check_file(&empty, &Params::default());
    assert!(!empty_check.is_ok());
    assert!(
        empty_check.message().contains("no_preview"),
        "{}",
        empty_check.message()
    );
}
