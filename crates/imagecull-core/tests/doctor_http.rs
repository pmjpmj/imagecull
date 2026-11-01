//! `doctor` against a real socket: the four verdicts the ticket names, each
//! produced by a stub server that answers like the oMLX version in question.
//!
//! The stub is a hand-rolled `TcpListener` rather than a mocking crate: the
//! whole point is to exercise the real `ureq` client and our JSON parsing, and
//! the contract is three GETs with canned bodies.

mod support;

use std::net::TcpListener;

use imagecull_core::ExitCode;
use imagecull_core::doctor::{self, Verdict};
use imagecull_core::omlx::{Client, Endpoint, Timeouts};
use imagecull_core::version::Version;

use support::stub::{Route, Stub};

const MODELS_ONE: &str = r#"{"object":"list","data":[{"id":"clef-flash-oQ4e","object":"model"}]}"#;
const MODELS_OTHER: &str = r#"{"object":"list","data":[{"id":"clef-27b-4bit","object":"model"}]}"#;
const STATUS_DECISION: &str = r#"{"models":[
    {"id":"models--clef-flash","model_alias":"clef-flash-oQ4e",
     "model_type":"decision","engine_type":"decision","loaded":true}
]}"#;
const STATUS_PLAIN: &str = r#"{"models":[
    {"id":"models--clef-flash","model_alias":"clef-flash-oQ4e",
     "model_type":"vlm","engine_type":"vlm","loaded":true}
]}"#;
const STATUS_OTHER: &str =
    r#"{"models":[{"id":"clef-27b-4bit","model_type":"decision","loaded":true}]}"#;

fn health(version: &str) -> String {
    format!(
        r#"{{"status":"ok","version":"{version}","uptime_seconds":42.0,
            "models_loaded":1,"default_model":"clef-flash-oQ4e"}}"#
    )
}

fn route(path: &'static str, status: u16, body: String) -> Route {
    Route::new(path, status, body)
}

fn client(endpoint: Endpoint) -> Client {
    Client::new(endpoint, Timeouts::with_read_seconds(5))
}

/// A port that was free a moment ago: connection refused rather than a timeout.
fn dead_endpoint() -> Endpoint {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    drop(listener);
    Endpoint::parse(&format!("http://{addr}")).expect("valid endpoint")
}

#[test]
fn a_current_server_serving_a_decision_model_is_ready() {
    let server = Stub::routed(vec![
        route("/health", 200, health("0.7.1.dev1")),
        route("/v1/models", 200, MODELS_ONE.to_string()),
        route("/v1/models/status", 200, STATUS_DECISION.to_string()),
    ]);
    let report = doctor::diagnose(&client(server.endpoint()), "clef-flash-oQ4e");

    assert_eq!(
        report.verdict,
        Verdict::Ready {
            version: Version::parse("0.7.1.dev1")
        }
    );
    assert!(report.is_ready());
    assert_eq!(report.exit(), ExitCode::Ok);
    assert!(report.message().contains("ready to score"));
}

#[test]
fn an_old_server_is_recognised_by_its_version_and_missing_status_endpoint() {
    let server = Stub::routed(vec![
        route("/health", 200, health("0.7.0")),
        route("/v1/models", 200, MODELS_ONE.to_string()),
        // pre-0.7.1.dev1 has no /v1/models/status: the stub 404s, as oMLX does
    ]);
    let report = doctor::diagnose(&client(server.endpoint()), "clef-flash-oQ4e");

    assert_eq!(
        report.verdict,
        Verdict::TooOld {
            found: Version::parse("0.7.0")
        }
    );
    assert_eq!(report.exit(), ExitCode::Fatal);
    let message = report.message();
    assert!(message.contains("predates decision models"));
    assert!(message.contains("0.7.1.dev1"));
    assert!(message.contains("plain VLM"));
}

#[test]
fn a_server_that_does_not_report_model_types_cannot_be_confirmed() {
    let server = Stub::routed(vec![
        // A version newer than the floor, but no status endpoint: the
        // capability check wins, because the version cannot be trusted alone.
        route("/health", 200, health("0.7.1.dev1")),
        route("/v1/models", 200, MODELS_ONE.to_string()),
    ]);
    let report = doctor::diagnose(&client(server.endpoint()), "clef-flash-oQ4e");

    assert_eq!(
        report.verdict,
        Verdict::NoModelTypes {
            version: Some("0.7.1.dev1".to_string())
        }
    );
    assert_eq!(report.exit(), ExitCode::Fatal);
    assert!(report.message().contains("/v1/models/status"));
}

#[test]
fn an_absent_alias_lists_what_the_server_does_serve() {
    let server = Stub::routed(vec![
        route("/health", 200, health("0.7.1.dev1")),
        route("/v1/models", 200, MODELS_OTHER.to_string()),
        route("/v1/models/status", 200, STATUS_OTHER.to_string()),
    ]);
    let report = doctor::diagnose(&client(server.endpoint()), "clef-flash-oQ4e");

    assert_eq!(
        report.verdict,
        Verdict::AliasMissing {
            available: vec!["clef-27b-4bit".to_string()]
        }
    );
    assert_eq!(report.exit(), ExitCode::Fatal);
    let message = report.message();
    assert!(message.contains("'clef-flash-oQ4e' is not served"));
    assert!(message.contains("clef-27b-4bit"));
    assert!(message.contains("--model"));
}

#[test]
fn an_alias_served_as_a_plain_vlm_is_not_ready() {
    let server = Stub::routed(vec![
        route("/health", 200, health("0.7.1.dev1")),
        route("/v1/models", 200, MODELS_ONE.to_string()),
        route("/v1/models/status", 200, STATUS_PLAIN.to_string()),
    ]);
    let report = doctor::diagnose(&client(server.endpoint()), "clef-flash-oQ4e");

    assert_eq!(
        report.verdict,
        Verdict::PlainModel {
            served_as: "vlm".to_string()
        }
    );
    assert_eq!(report.exit(), ExitCode::Fatal);
    assert!(report.message().contains("/v1/systemone"));
}

#[test]
fn nothing_listening_is_unreachable_and_says_what_to_start() {
    let report = doctor::diagnose(&client(dead_endpoint()), "clef-flash-oQ4e");

    assert!(matches!(report.verdict, Verdict::Unreachable { .. }));
    assert_eq!(report.exit(), ExitCode::Fatal);
    assert!(report.message().contains("start oMLX"));
}

#[test]
fn something_that_is_not_omlx_is_named_as_such() {
    let server = Stub::routed(vec![route("/health", 404, "{}".to_string())]);
    let report = doctor::diagnose(&client(server.endpoint()), "clef-flash-oQ4e");

    assert!(matches!(report.verdict, Verdict::NotOmlx { .. }));
    assert!(report.message().contains("not like oMLX"));
}

#[test]
fn a_server_still_loading_its_models_is_not_ready() {
    let server = Stub::routed(vec![route(
        "/health",
        503,
        r#"{"status":"loading","version":"0.7.1.dev1"}"#.to_string(),
    )]);
    let report = doctor::diagnose(&client(server.endpoint()), "clef-flash-oQ4e");

    assert_eq!(report.verdict, Verdict::NotReady { http_status: 503 });
    assert!(report.message().contains("not serving yet"));
}

#[test]
fn a_server_that_wants_an_api_key_is_named_as_such() {
    // oMLX guards /v1/models/status with the main API key while /v1/models can
    // be open, so an auth failure shows up on the second call.
    let server = Stub::routed(vec![
        route("/health", 200, health("0.7.1.dev1")),
        route("/v1/models", 200, MODELS_ONE.to_string()),
        route(
            "/v1/models/status",
            401,
            r#"{"detail":"API key required"}"#.to_string(),
        ),
    ]);
    let report = doctor::diagnose(&client(server.endpoint()), "clef-flash-oQ4e");

    assert_eq!(report.verdict, Verdict::Unauthorized { http_status: 401 });
    assert_eq!(report.exit(), ExitCode::Fatal);
    assert!(report.message().contains("API key"));
}
