//! `[server] token_env` as the boundary that resolves it: the binary, a real
//! environment, a real socket.
//!
//! `config.rs` proves the resolution rules over an injected lookup, which is
//! deliberately not the process environment: the tests run in threads, and one
//! that set a variable would race every other. That leaves exactly one thing
//! unproven, and it is the thing that matters to a photographer — that the front
//! end actually reads `std::env` and hands the value to the client. A regression
//! to `|_| None` here would otherwise leave the whole suite green while making
//! every `token_env` config fatal.

#[path = "../../imagecull-core/tests/support/mod.rs"]
mod support;

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::json;
use support::stub::{Received, Stub};

/// The alias the frame's rating asks, and the one the stub serves.
const ALIAS: &str = "clef-flash-oQ4e";

/// The variable the shoot config names.
const VARIABLE: &str = "IMAGECULL_TEST_BEARER";

/// The token the test exports, distinctive enough that a rendering would show it.
const TOKEN: &str = "cf-cli-boundary-token";

/// A shoot of one frame, with a shoot config that names `VARIABLE` and no
/// literal: the shape this test exists for.
fn shoot(stub: &Stub) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temp dir");
    fs::write(
        dir.path().join("DSC_0001.NEF"),
        support::nef(1, &[support::jpeg(48, 32, 80)]),
    )
    .expect("write a NEF");
    fs::write(
        dir.path().join("imagecull.toml"),
        format!(
            "[server]\nendpoint = \"{}\"\nmodel = \"{ALIAS}\"\ntoken_env = \"{VARIABLE}\"\n",
            stub.url()
        ),
    )
    .expect("a shoot config");
    dir
}

/// A complete rating answer, so a scored frame exits clean.
fn answer_document() -> String {
    json!({
        "model": ALIAS,
        "answers": {
            "stars": {"type": "choice", "choice": "4", "confidence": 0.63,
                      "probabilities": {"1": 0.01, "2": 0.03, "3": 0.12,
                                        "4": 0.63, "5": 0.21}}
        },
        "usage": {"input_tokens": 1327, "output_tokens": 0},
    })
    .to_string()
}

/// Score the shoot with the binary, with `VARIABLE` set to `value` or removed.
fn score(shoot: &Path, value: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_imagecull"));
    command
        .arg("score")
        .arg(shoot)
        .current_dir(shoot)
        .env_remove(VARIABLE);
    if let Some(value) = value {
        command.env(VARIABLE, value);
    }
    command.output().expect("the binary runs")
}

/// The rating requests the stub received: a run's preflight GETs are not them.
fn ratings(stub: &Stub) -> Vec<Received> {
    stub.requests()
        .into_iter()
        .filter(|request| request.path == "/v1/systemone")
        .collect()
}

#[test]
fn a_token_the_config_names_is_read_from_the_process_environment() {
    let stub = Stub::serving(vec![(200, answer_document())], &[ALIAS]);
    let shoot = shoot(&stub);

    let output = score(shoot.path(), Some(TOKEN));

    assert_eq!(
        output.status.code(),
        Some(0),
        "a resolved token is a clean run: {output:?}"
    );
    let ratings = ratings(&stub);
    assert_eq!(ratings.len(), 1, "one frame, one request: {output:?}");
    assert_eq!(
        ratings[0].authorization.as_deref(),
        Some("Bearer cf-cli-boundary-token"),
        "the value the variable held is what the request carried"
    );
}

#[test]
fn a_named_variable_with_nothing_behind_it_stops_the_run_before_a_request() {
    let stub = Stub::serving(vec![(200, answer_document())], &[ALIAS]);
    let shoot = shoot(&stub);

    let output = score(shoot.path(), None);

    assert_eq!(
        output.status.code(),
        Some(2),
        "a config that describes no credential is fatal: {output:?}"
    );
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(
        message.contains(VARIABLE),
        "the row has to name the variable to export: {message}"
    );
    assert!(
        stub.requests().is_empty(),
        "and the run must stop before paying for a request: {:?}",
        stub.requests()
    );
}
