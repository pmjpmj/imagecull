//! The `groups` command as a photographer runs it: the binary, a folder, an exit
//! code.
//!
//! The cull itself is asserted where it can be observed properly —
//! `groups_marking.rs` for the pass, `systemone_http.rs` for the request a real
//! socket receives. What is left, and what these two tests are for, is the
//! command's shape: `--no-cull` still runs the whole technical pass with no
//! server anywhere, and a folder with no group asks no server for a question it
//! would not ask.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The binary under test, run over `dir`, with every config file refused so the
/// machine's own `~/.config/imagecull.toml` cannot change what these tests see.
fn groups(dir: &Path, extra: &[&str]) -> (Output, PathBuf) {
    let csv = dir.join("groups.csv");
    let output = Command::new(env!("CARGO_BIN_EXE_imagecull"))
        .arg("groups")
        .arg(dir)
        .arg("--no-config")
        .arg("--out")
        .arg(&csv)
        .args(extra)
        .output()
        .expect("the binary runs");
    (output, csv)
}

#[test]
fn no_cull_runs_the_technical_pass_with_no_server_anywhere() {
    let shoot = tempfile::tempdir().expect("a temp dir");
    fs::write(shoot.path().join("DSC_0001.NEF"), b"not a NEF").expect("a broken frame");

    let (output, csv) = groups(shoot.path(), &["--no-cull"]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "a frame that cannot be read is a failed row, not a failed run: {output:?}"
    );
    assert!(
        csv.exists(),
        "the measures CSV is written whatever the cull does: {output:?}"
    );
    let report = String::from_utf8_lossy(&output.stdout);
    assert!(
        report.contains("reading 1 frame") && report.contains("1 without measures"),
        "the technical pass narrates itself and accounts for the frame: {report}"
    );
    assert!(
        !report.contains("culling"),
        "and says nothing about a cull it was told to leave out: {report}"
    );
    assert!(
        !shoot.path().join("DSC_0001.xmp").exists(),
        "a pass that asks nothing writes no verdict"
    );
}

#[test]
fn a_folder_with_no_group_asks_no_server_for_a_question_it_cannot_ask() {
    let shoot = tempfile::tempdir().expect("a temp dir");
    fs::write(shoot.path().join("notes.txt"), b"no raw files here").expect("a note");

    // No `--no-cull`: the cull runs, and with no group to compare it must not
    // need a server — nothing is listening on the default endpoint here.
    let (output, csv) = groups(shoot.path(), &[]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "a folder with nothing to compare is a clean run with no server: {output:?}"
    );
    assert!(csv.exists(), "{output:?}");
    let report = String::from_utf8_lossy(&output.stdout);
    assert!(
        report.contains("culling 0 groups") && report.contains("0 groups judged"),
        "the cull says it had no group to judge, and the summary counts it: {report}"
    );
}

#[test]
fn a_shoot_configs_scan_section_reaches_the_groups_scan() {
    let shoot = tempfile::tempdir().expect("a temp dir");
    let sub = shoot.path().join("sub");
    fs::create_dir(&sub).expect("a subdirectory");
    fs::write(sub.join("DSC_0002.NEF"), b"not a NEF").expect("a broken frame");
    fs::write(
        shoot.path().join("imagecull.toml"),
        "[scan]\nrecursive = true\n",
    )
    .expect("a shoot config");

    // `./imagecull.toml` is discovered in the working directory, so the shoot
    // folder is the working directory here.
    let csv = shoot.path().join("groups.csv");
    let run = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_imagecull"))
            .arg("groups")
            .arg(shoot.path())
            .arg("--out")
            .arg(&csv)
            .arg("--no-cull")
            .args(extra)
            .current_dir(shoot.path())
            .output()
            .expect("the binary runs")
    };
    let relative = Path::new("sub")
        .join("DSC_0002.NEF")
        .to_string_lossy()
        .into_owned();

    let output = run(&[]);
    let measures = fs::read_to_string(&csv).expect("the CSV is written");
    assert!(
        measures.contains(&relative),
        "the file's [scan] recursive has to reach the scan: {} {measures}",
        String::from_utf8_lossy(&output.stdout)
    );

    let output = run(&["--no-config"]);
    let measures = fs::read_to_string(&csv).expect("the CSV is written");
    assert!(
        !measures.contains(&relative) && !measures.contains("DSC_0002.NEF"),
        "--no-config leaves the folder alone, which is what proves the file did it: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}
