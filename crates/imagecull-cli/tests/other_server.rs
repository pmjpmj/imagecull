//! `models` pointed at a server that is not oMLX: the one command that reads
//! oMLX's own probes says so, and says it without a request.
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

/// The model name a photographer would configure for Workers AI: the body's
/// `model`, and the value the address's `{model}` becomes.
const MODEL: &str = "clef-flash";

/// The part of a Workers AI address that names the model's family, `{model}` and
/// all: the address is the request, so the path lives in it.
const PATH: &str = "/@cf/cloudflare/{model}";

/// A shoot whose config addresses a Workers AI account at the stub's address.
fn shoot(stub: &Stub) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temp dir");
    fs::write(
        dir.path().join("imagecull.toml"),
        format!(
            "[server]\nendpoint = \"{}{PATH}\"\nmodel = \"{MODEL}\"\n",
            stub.url()
        ),
    )
    .expect("a shoot config");
    dir
}

/// Run `models` against the shoot's config.
fn imagecull(shoot: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_imagecull"))
        .arg("models")
        .current_dir(shoot)
        .output()
        .expect("the binary runs")
}

#[test]
fn models_refuses_a_workers_ai_config_and_makes_no_request() {
    let stub = Stub::scripted(vec![]);
    let shoot = shoot(&stub);

    let output = imagecull(shoot.path());

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
