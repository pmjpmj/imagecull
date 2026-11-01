//! A hand-rolled HTTP stub for the tests that drive the real `ureq` client.
//!
//! Two tests need a socket — `doctor`'s three GETs and a `/v1/systemone` POST —
//! and both need the same things: a real `TcpListener` on loopback, a request
//! read off the wire, a canned answer written back, and no mocking crate. This
//! is that stub, once: routers answer by path like an old server, scripts answer
//! in order like a model, and every request is recorded so a test can assert on
//! what the client actually sent.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use imagecull_core::omlx::Endpoint;
use serde_json::{Value, json};

/// One canned answer, matched on the request path.
pub struct Route {
    pub path: &'static str,
    pub status: u16,
    pub body: String,
}

impl Route {
    pub fn new(path: &'static str, status: u16, body: impl Into<String>) -> Route {
        Route {
            path,
            status,
            body: body.into(),
        }
    }
}

/// One request the stub received.
#[derive(Debug, Clone)]
pub struct Received {
    pub path: String,
    pub content_type: Option<String>,
    /// The body, parsed as JSON when it is JSON and kept as a string otherwise.
    pub body: Value,
}

struct State {
    routes: Vec<Route>,
    script: Mutex<VecDeque<(u16, String)>>,
    requests: Mutex<Vec<Received>>,
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
    delay: Mutex<Duration>,
}

/// A loopback HTTP stub, running until it is dropped.
pub struct Stub {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    state: Arc<State>,
}

impl Stub {
    /// Answer by path; anything not routed is a 404, the way a server without
    /// that endpoint answers.
    pub fn routed(routes: Vec<Route>) -> Stub {
        Stub::start(routes, Vec::new())
    }

    /// Answer each request in turn from `script`, whatever its path.
    pub fn scripted(script: Vec<(u16, String)>) -> Stub {
        Stub::start(Vec::new(), script)
    }

    /// A server that passes a run's preflight — `GET /health` ready and every
    /// alias in `aliases` served as a decision model — and then answers the
    /// decision requests in turn from `script`.
    pub fn serving(script: Vec<(u16, String)>, aliases: &[&str]) -> Stub {
        let listed: Vec<Value> = aliases.iter().map(|alias| json!({"id": alias})).collect();
        let typed: Vec<Value> = aliases
            .iter()
            .map(|alias| json!({"id": alias, "model_type": "decision"}))
            .collect();
        Stub::start(
            vec![
                Route::new(
                    "/health",
                    200,
                    json!({"status": "ok", "version": "0.7.1.dev1"}).to_string(),
                ),
                Route::new("/v1/models", 200, json!({"data": listed}).to_string()),
                Route::new(
                    "/v1/models/status",
                    200,
                    json!({"models": typed}).to_string(),
                ),
            ],
            script,
        )
    }

    /// A server that lists its aliases but never answers `/v1/models/status`: an
    /// install older than that endpoint, or one that guards it. It is still a
    /// server a run may score against, which is why the fixture exists.
    pub fn untyped(script: Vec<(u16, String)>, aliases: &[&str]) -> Stub {
        let listed: Vec<Value> = aliases.iter().map(|alias| json!({"id": alias})).collect();
        Stub::start(
            vec![
                Route::new(
                    "/health",
                    200,
                    json!({"status": "ok", "version": "0.7.1.dev1"}).to_string(),
                ),
                Route::new("/v1/models", 200, json!({"data": listed}).to_string()),
                // The old endpoint 404s, as an install without it does. Routed
                // rather than left to fall through, so the script stays for the
                // decision requests.
                Route::new(
                    "/v1/models/status",
                    404,
                    json!({"detail": "Not Found"}).to_string(),
                ),
            ],
            script,
        )
    }

    /// Wait this long before answering, so a client timeout can be exercised.
    pub fn taking(self, delay: Duration) -> Stub {
        *self.state.delay.lock().expect("delay") = delay;
        self
    }

    fn start(routes: Vec<Route>, script: Vec<(u16, String)>) -> Stub {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let addr = listener.local_addr().expect("local addr");
        listener.set_nonblocking(true).expect("non-blocking");
        let state = Arc::new(State {
            routes,
            script: Mutex::new(script.into()),
            requests: Mutex::new(Vec::new()),
            in_flight: AtomicUsize::new(0),
            max_in_flight: AtomicUsize::new(0),
            delay: Mutex::new(Duration::ZERO),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread_state = Arc::clone(&state);
        let handle = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        // BSD `accept` inherits the listener's non-blocking
                        // flag; a handler that raced the client's first bytes
                        // would close the connection under it, which is the
                        // intermittent "Connection reset" this suite used to
                        // have.
                        stream.set_nonblocking(false).expect("blocking socket");
                        let state = Arc::clone(&thread_state);
                        std::thread::spawn(move || handle(stream, &state));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });
        Stub {
            addr,
            stop,
            handle: Some(handle),
            state,
        }
    }

    /// The base URL a client should be pointed at.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// The base URL as the client's own type.
    pub fn endpoint(&self) -> Endpoint {
        Endpoint::parse(&self.url()).expect("valid endpoint")
    }

    /// Every request received, in the order it arrived.
    pub fn requests(&self) -> Vec<Received> {
        self.state.requests.lock().expect("requests").clone()
    }

    /// The deepest overlap the stub ever saw: 1 means strictly sequential.
    pub fn max_in_flight(&self) -> usize {
        self.state.max_in_flight.load(Ordering::SeqCst)
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn handle(mut stream: TcpStream, state: &State) {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut path = String::from("/");
    let mut content_type = None;
    let mut length = 0usize;
    let mut first = true;
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) if line.trim().is_empty() => break,
            Ok(_) => {}
            Err(_) => return,
        }
        if first {
            path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
            first = false;
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-type") {
            content_type = Some(value.to_string());
        } else if name.eq_ignore_ascii_case("content-length") {
            length = value.parse().unwrap_or(0);
        }
    }

    let mut body = vec![0u8; length];
    if reader.read_exact(&mut body).is_err() {
        return;
    }
    let body = String::from_utf8(body).unwrap_or_default();
    let parsed = serde_json::from_str(&body).unwrap_or(Value::String(body));

    let in_flight = state.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
    state.max_in_flight.fetch_max(in_flight, Ordering::SeqCst);
    state.requests.lock().expect("requests").push(Received {
        path: path.clone(),
        content_type,
        body: parsed,
    });

    let delay = *state.delay.lock().expect("delay");
    if !delay.is_zero() {
        std::thread::sleep(delay);
    }
    let (status, body) = answer(state, &path);

    state.in_flight.fetch_sub(1, Ordering::SeqCst);

    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        413 => "Payload Too Large",
        422 => "Unprocessable Entity",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
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

/// A routed path wins, then the script, then the 404 an absent endpoint gives.
fn answer(state: &State, path: &str) -> (u16, String) {
    if let Some(route) = state.routes.iter().find(|route| route.path == path) {
        return (route.status, route.body.clone());
    }
    if let Some(next) = state.script.lock().expect("script").pop_front() {
        return next;
    }
    (404, r#"{"detail":"Not Found"}"#.to_string())
}
