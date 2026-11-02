//! `doctor` and `models` pointed at a server that is not oMLX: the two commands
//! that read oMLX's own probes say so, and say it without a request.
//!
//! `systemone_http.rs` proves a run against Workers AI's addressing works;
//! `config.rs` proves the predicate that decides which server a run addresses.
//! What is left is the thing a photographer sees — a refusal that names the URL
//! their config describes and points them at the commands that do work there —
//! and the one thing a refusal must never do: spend a request finding out.

#[path = "../../imagecull-core/tests/support/mod.rs"]
mod support;

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use support::stub::Stub;

/// The Workers AI model id a photographer would configure: it *is* the rest of
/// the URL, which is why the path has to be a template.
const MODEL: &str = "@cf/cloudflare/clef-flash";

/// The path template a Workers AI config describes.
const PATH: &str = "/{model}";

/// A shoot whose config addresses a Workers AI account at the stub's address.
fn shoot(stub: &Stub) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temp dir");
    fs::write(
        dir.path().join("imagecull.toml"),
        format!(
            "[server]\nendpoint = \"{}\"\npath = \"{PATH}\"\nmodel = \"{MODEL}\"\n",
            stub.url()
        ),
    )
    .expect("a shoot config");
    dir
}

/// Run one of the two oMLX-only commands against the shoot's config.
fn imagecull(shoot: &Path, command: &str, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_imagecull"))
        .arg(command)
        .args(extra)
        .current_dir(shoot)
        .output()
        .expect("the binary runs")
}

#[test]
fn doctor_refuses_a_workers_ai_config_and_makes_no_request() {
    let stub = Stub::scripted(vec![]);
    let shoot = shoot(&stub);

    let output = imagecull(shoot.path(), "doctor", &[]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "a server it cannot read is fatal, not a diagnosis: {output:?}"
    );
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(
        message.contains("doctor reads oMLX's own endpoints"),
        "the refusal has to say what doctor would have read: {message}"
    );
    assert!(
        message.contains(&format!("{}{PATH}", stub.url())),
        "and it has to name the URL this config describes, template and all: {message}"
    );
    assert!(
        message.contains("imagecull score and imagecull groups"),
        "and point at the commands that do work there: {message}"
    );
    assert!(
        stub.requests().is_empty(),
        "a refused server is never asked a question: {:?}",
        stub.requests()
    );
}

#[test]
fn doctor_refuses_the_file_check_too() {
    // The file check renders "the preview a rating would send", and that
    // rating's server is one this command does not understand — so the command
    // is refused whole rather than half-certifying a pipeline. A file that does
    // not exist is the sharpest way to say so: had the check run, it would have
    // an error of its own to report.
    let stub = Stub::scripted(vec![]);
    let shoot = shoot(&stub);

    let output = imagecull(shoot.path(), "doctor", &["missing.NEF"]);

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(
        message.contains("imagecull score and imagecull groups"),
        "the one refusal is all there is: {message}"
    );
    assert!(
        !message.contains("missing.NEF"),
        "the file check never ran, so it has nothing to say about the file: {message}"
    );
    assert!(
        output.stdout.is_empty(),
        "and nothing was certified: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(stub.requests().is_empty(), "no request was made");
}

#[test]
fn models_refuses_a_workers_ai_config_and_makes_no_request() {
    let stub = Stub::scripted(vec![]);
    let shoot = shoot(&stub);

    let output = imagecull(shoot.path(), "models", &[]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "a listing it cannot read is fatal: {output:?}"
    );
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(
        message.contains("models reads oMLX's model listing"),
        "the refusal has to name the listing models would have read: {message}"
    );
    assert!(
        message.contains("/v1/models"),
        "both listings, since it joins them: {message}"
    );
    assert!(
        message.contains(&format!("{}{PATH}", stub.url())),
        "and the URL this config describes: {message}"
    );
    assert!(
        stub.requests().is_empty(),
        "a refused server is never asked for its catalogue: {:?}",
        stub.requests()
    );
}
