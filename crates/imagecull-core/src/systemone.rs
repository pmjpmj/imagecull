//! `POST /v1/systemone`: the request body the rubric becomes, and the one
//! production [`Decider`].
//!
//! The endpoint is new surface (spec §7), so the body is built in one place and
//! kept inspectable: exactly one image, a non-null state, `truncate: true`, and
//! the rubric's questions in declaration order. The adapter classifies
//! transport and status failures into the taxonomy and hands the raw document
//! on untouched; reading it is [`reading`](crate::reading)'s job.

use std::time::Instant;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Map, Value, json};

use crate::doctor;
use crate::error::{Error, ErrorKind, Result};
use crate::omlx::Client;
use crate::scoring::{Decider, Request, Response, State};

/// The path a decision model is served on (spec §7.1).
pub const PATH: &str = "/v1/systemone";

/// The oMLX HTTP adapter: the only [`Decider`] that opens a socket.
#[derive(Debug, Clone)]
pub struct Adapter {
    client: Client,
    model: String,
}

impl Adapter {
    /// Wrap a client for one alias. The client's timeouts are the request's
    /// timeouts, and the alias is what the preflight looks for on the server.
    pub fn new(client: Client, model: impl Into<String>) -> Adapter {
        Adapter {
            client,
            model: model.into(),
        }
    }
}

impl Decider for Adapter {
    /// Ask `doctor`'s question before the first frame: is something answering,
    /// and is the alias served as a decision model? Both failures arrive here
    /// as one actionable sentence, so a broken server costs seconds instead of
    /// 400 frames of the same 400 (spec §10).
    ///
    /// The run is the lenient reader of that diagnosis: it refuses only what the
    /// server has said is unfit, never what the server did not say at all
    /// ([`crate::doctor::Report::can_score`] has the list). `imagecull doctor`
    /// stays strict, so the strictness is available where it is useful —
    /// explaining a server — and does not stop a working one from scoring.
    fn preflight(&self) -> Result<()> {
        let report = doctor::diagnose(&self.client, &self.model);
        if report.can_score() {
            Ok(())
        } else {
            Err(Error::fatal(report.message()))
        }
    }

    fn decide(&self, request: &Request<'_>) -> Result<Response> {
        let payload = body(request);
        let started = Instant::now();
        let fetched = self.client.post_json(PATH, &payload)?;
        let client_latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

        if !(200..300).contains(&fetched.http_status) {
            return Err(failure(fetched.http_status, &fetched.body));
        }

        // A body that is not JSON is still kept as the document (as a string),
        // so `--json-scores` can show what arrived when the reading fails.
        let document = serde_json::from_str(&fetched.body)
            .unwrap_or_else(|_| Value::String(fetched.body.clone()));

        Ok(Response {
            document,
            client_latency_ms,
        })
    }
}

/// The JSON body of one request, exactly as it goes on the wire.
///
/// Crate-internal on purpose: the one seam is [`Decider`], and contract tests
/// assert on this document by recording what a real request carried rather than
/// by calling the renderer behind the adapter's back.
pub(crate) fn body(request: &Request<'_>) -> Value {
    json!({
        "model": request.model,
        "state": state(&request.state),
        "images": [data_uri(&request.preview.jpeg)],
        "truncate": true,
        "questions": request.rubric.questions_json(),
    })
}

/// The request's state. Never null: a cull always has a task and a filename
/// (spec §7.1), and the photographer's context when they gave one.
fn state(state: &State) -> Value {
    let mut state_json = Map::new();
    state_json.insert("task".to_string(), Value::String(state.task.clone()));
    state_json.insert(
        "filename".to_string(),
        Value::String(state.filename.clone()),
    );
    if let Some(context) = &state.shoot_context {
        state_json.insert("shoot_context".to_string(), Value::String(context.clone()));
    }
    Value::Object(state_json)
}

/// The preview as the full data URI oMLX takes (spec §6.6).
fn data_uri(jpeg: &[u8]) -> String {
    format!("data:image/jpeg;base64,{}", BASE64.encode(jpeg))
}

/// Classify a non-2xx answer: the status picks the taxonomy kind, and the
/// server's own `detail` rides along because a rejected rubric should say which
/// criteria it rejected (spec §10).
///
/// The row is the only thing the photographer reads after a run, so a status
/// with a known remedy says what to change: a context that is too long asks for
/// a smaller preview.
fn failure(status: u16, body: &str) -> Error {
    let kind = ErrorKind::from_status_code(status).unwrap_or(ErrorKind::BadResponse);
    let mut message = format!("POST {PATH} returned HTTP {status}");
    let detail = detail(body);
    if !detail.is_empty() {
        message.push_str(": ");
        message.push_str(&detail);
    }
    if kind == ErrorKind::Http413 {
        message.push_str(" — the preview is too big for the model's context; lower --max-edge");
    }
    Error::new(kind, message).with_http_status(status)
}

/// The server's `detail` field, or the body itself, cut to one short line.
fn detail(body: &str) -> String {
    const LIMIT: usize = 200;
    let text = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|document| {
            document
                .get("detail")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.trim().to_string());
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > LIMIT {
        let cut: String = one_line.chars().take(LIMIT).collect();
        format!("{cut}…")
    } else {
        one_line
    }
}
