//! `POST {endpoint}`: the request body the rubric becomes, and the one production
//! [`Decider`].
//!
//! The endpoint is new surface (spec §7), so the body is built in one place and
//! kept inspectable: exactly one image, a non-null state, the rubric's questions
//! in declaration order, and the two facts a `[server]` section decides — the
//! address a request goes to, with `{model}` filled in, and whether the body
//! carries oMLX's `truncate`. The adapter classifies transport and status
//! failures into the taxonomy and hands the raw document on untouched; reading
//! it is [`reading`](crate::reading)'s job.

use std::time::Instant;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Map, Value, json};

use crate::error::{Error, ErrorKind, Result};
use crate::keeper::{self, Comparison};
use crate::omlx::{self, Client, MODEL_PLACEHOLDER};
use crate::scoring::{Decider, Request, Response, State};

/// The path a decision model is served on by default (spec §7.1).
///
/// It is oMLX's, which is why it is what [`DEFAULT_ENDPOINT`] ends with and what
/// `config.rs` settles to. [`is_omlx`] is the one place that decides whether an
/// address names this server.
pub const PATH: &str = "/v1/systemone";

/// The address a run addresses when nothing says otherwise: oMLX on the port it
/// listens on, and nothing else — a config with no `[server]` key posts here.
pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:8000/v1/systemone";

/// Whether an address is oMLX's own decision path, the one whose probes `models`
/// reads.
///
/// An address with `{model}` in it is some other server's addressing: the model
/// name is part of its path, which no oMLX install does, and `models` can say
/// nothing about it. Anything else has to *end* with the decision path, so an
/// oMLX behind a prefix (`https://box.local/omlx/v1/systemone`) is still oMLX to
/// that command — its probes hang off the same prefix.
///
/// Asked by [`crate::config::Effective::asks_omlx`], which decides whether the
/// command runs at all.
pub fn is_omlx(endpoint: &str) -> bool {
    !endpoint.contains(MODEL_PLACEHOLDER) && omlx::path_of(endpoint).ends_with(PATH)
}

/// The HTTP wire a run addresses: the two facts a `[server]` section decides.
///
/// Built in `cli.rs` from the effective config and handed to [`Adapter::new`],
/// which fills the address's `{model}` per request. Owned values rather than a
/// borrowed `Effective`, because the adapter is `Clone` and shared across a
/// cull's workers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wire {
    /// The request address, as a template: `{model}` becomes the run's model
    /// name, verbatim and unencoded.
    pub endpoint: String,
    /// Whether the body carries oMLX's `truncate: true`. Workers AI's schema
    /// lists no such field, and sends it absent rather than false.
    pub truncate: bool,
}

impl Default for Wire {
    /// oMLX's wire, which is what a config that names no `[server]` key
    /// describes: the fixed address, and `truncate`.
    fn default() -> Wire {
        Wire {
            endpoint: DEFAULT_ENDPOINT.to_string(),
            truncate: true,
        }
    }
}

/// The HTTP adapter: the only [`Decider`] that opens a socket.
#[derive(Debug, Clone)]
pub struct Adapter {
    client: Client,
    /// The model name: interpolated into the address, and the body's `model`.
    model: String,
    /// The address template, unresolved: [`Adapter::address`] fills it per
    /// request, because only the request knows the address it posts to.
    endpoint: String,
    /// Whether the body carries oMLX's `truncate: true`.
    truncate: bool,
}

impl Adapter {
    /// Wrap a client and a [`Wire`] for one model. The client's timeouts are the
    /// request's timeouts, the model is what the body carries, and the wire is
    /// where each request goes and whether it carries `truncate`.
    ///
    /// An address that needs a model, with no model to fill it, is fatal here
    /// rather than at the first request: a config can be legal and still describe
    /// nothing, and a template with a brace in it would post nowhere once per
    /// frame.
    pub fn new(client: Client, model: impl Into<String>, wire: Wire) -> Result<Adapter> {
        let model = model.into();
        if model.trim().is_empty() && wire.endpoint.contains(MODEL_PLACEHOLDER) {
            return Err(Error::fatal(
                "the configured endpoint needs a model — pass --model or set [server] model",
            ));
        }
        // The model name goes into the address verbatim, so anything that cannot
        // be a path segment or a body value has to be refused by name rather than
        // turned into a 404 nobody can read.
        if model.contains(['{', '}']) || model.chars().any(char::is_whitespace) {
            return Err(Error::fatal(format!(
                "the model name '{model}' cannot contain whitespace or braces — it is both the \
                 body's `model` and the value `{{model}}` takes in the address"
            )));
        }
        Ok(Adapter {
            client,
            model,
            endpoint: wire.endpoint,
            truncate: wire.truncate,
        })
    }

    /// The address this adapter posts to, with the model name interpolated
    /// verbatim.
    ///
    /// No escaping: Workers AI names contain `/` and `@`, which *are* the address
    /// being asked for, and encoding them would address something else.
    fn address(&self) -> String {
        self.endpoint.replace(MODEL_PLACEHOLDER, &self.model)
    }
}

impl Decider for Adapter {
    fn decide(&self, request: &Request<'_>) -> Result<Response> {
        self.ask(self.body(request), Asked::Frame)
    }

    fn compare(&self, comparison: &Comparison) -> Result<Response> {
        self.ask(self.comparison_body(comparison), Asked::Group)
    }
}

impl Adapter {
    /// Post one payload and read the answer document out of it.
    ///
    /// One implementation for both kinds of request, so the latency this side
    /// measures, the way a non-2xx becomes a taxonomy kind and the rule that a
    /// body which is not JSON is still kept cannot drift apart between a frame
    /// and a group.
    fn ask(&self, payload: Value, kind: Asked) -> Result<Response> {
        let url = self.address();
        let started = Instant::now();
        let fetched = self.client.post_json(&url, &payload)?;
        let client_latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

        if !(200..300).contains(&fetched.http_status) {
            return Err(failure(
                fetched.http_status,
                &fetched.body,
                kind,
                omlx::path_of(&url),
            ));
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

    /// The JSON body of one request, exactly as it goes on the wire.
    ///
    /// Private on purpose: the one seam is [`Decider`], and contract tests assert
    /// on this document by recording what a real request carried rather than by
    /// calling the renderer behind the adapter's back.
    fn body(&self, request: &Request<'_>) -> Value {
        self.envelope(
            &request.state,
            vec![data_uri(&request.preview.jpeg)],
            request.rubric.questions_json(),
        )
    }

    /// The JSON body of one comparative request: several previews of one moment,
    /// and the question which of them to keep.
    ///
    /// The shape is the one measured against the reference server (see
    /// `.scratch/burst-grouping-v1/evidence/comparison-budget.md`): `questions` is
    /// a map keyed by question id, `criteria` is a map from option key to
    /// description, and every image is a data URI. The option keys are the
    /// frames' positions, so the answer is a position in this list and nothing
    /// else.
    fn comparison_body(&self, comparison: &Comparison) -> Value {
        let mut criteria = Map::new();
        for position in 1..=comparison.previews.len() {
            criteria.insert(
                position.to_string(),
                Value::String(format!("image at position {position}")),
            );
        }
        let mut question = Map::new();
        question.insert("type".to_string(), Value::String("choice".to_string()));
        question.insert(
            "instructions".to_string(),
            Value::String(keeper::INSTRUCTIONS.to_string()),
        );
        question.insert("criteria".to_string(), Value::Object(criteria));
        let mut questions = Map::new();
        questions.insert(keeper::QUESTION.to_string(), Value::Object(question));

        self.envelope(
            &comparison.state,
            comparison
                .previews
                .iter()
                .map(|preview| data_uri(&preview.jpeg))
                .collect(),
            Value::Object(questions),
        )
    }

    /// The envelope every request in this run shares: the model the server is
    /// asked for, the state, the images, oMLX's `truncate` when the wire wants
    /// it, and the questions last.
    ///
    /// One builder, so a rating and a comparison cannot disagree about what a
    /// request is made of. `truncate` is written only when the wire asks for it —
    /// absent and `false` are different requests, and Workers AI's schema has no
    /// such field — and in its original position, because `serde_json` keeps
    /// insertion order and an oMLX body has to stay byte for byte what it was.
    fn envelope(&self, state: &State, images: Vec<String>, questions: Value) -> Value {
        let mut body = json!({
            "model": self.model,
            "state": state_json(state),
            "images": images,
        });
        if self.truncate {
            body["truncate"] = json!(true);
        }
        body["questions"] = questions;
        body
    }
}

/// The request's state as JSON. Never null: a cull always has a task and a
/// filename (spec §7.1), and the photographer's context when they gave one.
fn state_json(state: &State) -> Value {
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

/// Which of the two requests a failure came from, for the advise a status
/// carries: a frame that 413s is too big a picture, and a group that 413s is a
/// budget the server does not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Asked {
    Frame,
    Group,
}

/// Classify a non-2xx answer: the status picks the taxonomy kind, and the
/// server's own `detail` rides along because a rejected rubric should say which
/// criteria it rejected (spec §10).
///
/// The row is the only thing the photographer reads after a run, so a status
/// with a known remedy says what to change: a context that is too long asks for
/// a smaller preview, a group that does not fit is a group nothing was marked
/// for, and refused credentials name the config keys that carry them — never the
/// credential itself, which must not reach a row, a log, or a CSV. The path is
/// the one this request actually posted to, never a constant: it has to be the
/// request the photographer can reproduce with `curl`.
fn failure(status: u16, body: &str, asked: Asked, path: &str) -> Error {
    let kind = ErrorKind::from_status_code(status).unwrap_or(ErrorKind::BadResponse);
    let mut message = format!("POST {path} returned HTTP {status}");
    let detail = detail(body);
    if !detail.is_empty() {
        message.push_str(": ");
        message.push_str(&detail);
    }
    match kind {
        ErrorKind::Http413 => message.push_str(match asked {
            Asked::Frame => " — the preview is too big for the model's context; lower --max-edge",
            Asked::Group => {
                " — the group needs more image tokens than this model's context holds, so nothing \
                 was marked; the request is refused rather than quietly made smaller, and a lower \
                 --max-edge (or [group] max_edge) is how to ask for less picture"
            }
        }),
        // 401 and 403 abort the run, so this row is the one the photographer
        // reads: it says which keys hold the credential and never repeats it.
        ErrorKind::Http401 | ErrorKind::Http403 => message.push_str(
            " — the server refused this run's credentials; check [server] token_env (or \
             [server] token) and the account the endpoint names",
        ),
        _ => {}
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::omlx::{Endpoint, Timeouts};

    /// A client that never connects: these tests are about construction.
    fn client() -> Client {
        Client::new(
            Endpoint::parse(DEFAULT_ENDPOINT).expect("the default address"),
            Timeouts::with_read_seconds(1),
            None,
        )
    }

    #[test]
    fn the_default_address_is_omlx_on_its_decision_path() {
        assert_eq!(
            DEFAULT_ENDPOINT,
            format!("{}{PATH}", omlx::DEFAULT_ENDPOINT),
            "the default address and the oMLX origin constant cannot drift apart"
        );
        assert!(is_omlx(DEFAULT_ENDPOINT));
    }

    #[test]
    fn is_omlx_is_the_path_and_nothing_else() {
        assert!(is_omlx("http://127.0.0.1:9000/v1/systemone"));
        assert!(
            is_omlx("https://box.local/omlx/v1/systemone"),
            "an oMLX behind a prefix is still oMLX: its path is the decision path"
        );
        assert!(
            !is_omlx("https://host/ai/run/{model}"),
            "a model name in the address is another server's addressing"
        );
        assert!(
            !is_omlx("http://127.0.0.1:9000/v1/decisions"),
            "another fixed path is not oMLX's probe surface"
        );
        assert!(
            !is_omlx("http://127.0.0.1:9000"),
            "a path-less address addresses `/`, which oMLX does not serve a decision on"
        );
    }

    #[test]
    fn the_address_is_the_request_even_without_a_path() {
        let plain = Adapter::new(client(), "clef-oQ4e", Wire::default()).expect("a settled wire");
        assert_eq!(plain.address(), DEFAULT_ENDPOINT);
        assert_eq!(plain.model, "clef-oQ4e", "the body says the same name");

        let templated = Adapter::new(
            client(),
            "clef-flash",
            Wire {
                endpoint: "https://host/ai/run/@cf/cloudflare/{model}".to_string(),
                truncate: false,
            },
        )
        .expect("a name fills the placeholder");
        assert_eq!(
            templated.address(),
            "https://host/ai/run/@cf/cloudflare/clef-flash",
            "the name is interpolated verbatim, `/` and `@` and all"
        );
        assert_eq!(
            omlx::path_of(&templated.address()),
            "/ai/run/@cf/cloudflare/clef-flash"
        );
    }

    #[test]
    fn a_name_that_could_not_be_a_path_segment_or_a_body_value_is_refused() {
        let wire = Wire {
            endpoint: "https://host/ai/run/{model}".to_string(),
            truncate: false,
        };
        let missing = Adapter::new(client(), "", wire.clone())
            .expect_err("an address that needs a model has none");
        assert!(
            missing.message().contains("needs a model"),
            "the row says what is missing: {}",
            missing.message()
        );
        for bad in ["clef flash", "clef\tflash", "{model}", "clef}"] {
            let error = Adapter::new(client(), bad, wire.clone())
                .expect_err("a name with whitespace or braces is refused by name");
            assert!(error.is_fatal(), "'{bad}' should be fatal");
            assert!(
                error.message().contains("model name"),
                "the message has to name what is wrong: {}",
                error.message()
            );
        }
    }
}
