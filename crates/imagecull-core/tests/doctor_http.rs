//! `doctor` against a real socket: the four verdicts the ticket names, each
//! produced by a stub server that answers like the oMLX version in question.
//!
//! The stub is a hand-rolled `TcpListener` rather than a mocking crate: the
//! whole point is to exercise the real `ureq` client and our JSON parsing, and
//! the contract is three GETs with canned bodies.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use imagecull_core::ExitCode;
use imagecull_core::doctor::{self, Verdict};
use imagecull_core::omlx::{Client, Endpoint, Timeouts};
use imagecull_core::version::Version;

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

/// One canned response, matched on the request path.
struct Route {
    path: &'static str,
    status: u16,
    body: String,
}

fn route(path: &'static str, status: u16, body: String) -> Route {
    Route { path, status, body }
}

/// A single-threaded HTTP stub that closes after each response.
struct StubServer {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl StubServer {
    fn start(routes: Vec<Route>) -> StubServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let addr = listener.local_addr().expect("local addr");
        listener
            .set_nonblocking(true)
            .expect("non-blocking listener");
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => respond(stream, &routes),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        StubServer {
            addr,
            stop,
            handle: Some(handle),
        }
    }

    fn endpoint(&self) -> Endpoint {
        Endpoint::parse(&format!("http://{}", self.addr)).expect("valid endpoint")
    }
}

impl Drop for StubServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn respond(mut stream: TcpStream, routes: &[Route]) {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
    let mut path = String::from("/");
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) if line.trim().is_empty() => break,
            Ok(_) => {}
            Err(_) => return,
        }
        if path == "/" {
            path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
        }
    }

    let (status, body) = match routes.iter().find(|route| route.path == path) {
        Some(route) => (route.status, route.body.clone()),
        None => (404, "{\"detail\":\"Not Found\"}".to_string()),
    };
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
    let _ = stream.flush();
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
    let server = StubServer::start(vec![
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
    let server = StubServer::start(vec![
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
    let server = StubServer::start(vec![
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
    let server = StubServer::start(vec![
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
    let server = StubServer::start(vec![
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
    let server = StubServer::start(vec![route("/health", 404, "{}".to_string())]);
    let report = doctor::diagnose(&client(server.endpoint()), "clef-flash-oQ4e");

    assert!(matches!(report.verdict, Verdict::NotOmlx { .. }));
    assert!(report.message().contains("not like oMLX"));
}

#[test]
fn a_server_still_loading_its_models_is_not_ready() {
    let server = StubServer::start(vec![route(
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
    let server = StubServer::start(vec![
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
