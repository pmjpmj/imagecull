//! The oMLX HTTP surface imagecull depends on: `GET /health`,
//! `GET /v1/models`, and `GET /v1/models/status`.
//!
//! Three facts matter before a scoring run, and only the two model endpoints
//! together answer them: `/v1/models` lists the aliases the server serves but
//! says nothing about what they are, while `/v1/models/status` reports each
//! model's `model_type`/`engine_type` and is where a decision model
//! (`"decision"`) can be told apart from a plain chat or VLM checkpoint.
//! `/v1/models/status` only exists from oMLX `0.7.1.dev1`, so its absence is
//! itself the "too old" signal.
//!
//! Everything here is typed and synchronous: no retries, no preflight policy,
//! no printing. Callers decide what a failure means.

use std::fmt;
use std::time::Duration;

use serde::Deserialize;

use crate::error::{Error, ErrorKind, Result};

/// Where oMLX listens by default.
pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:8000";

/// The alias used when nothing else says otherwise.
///
/// The rubric will own this once rubrics exist; until then it is the reference
/// server's Clef-Flash alias (spec §7.1).
pub const DEFAULT_MODEL: &str = "clef-flash-oQ4e";

/// The first oMLX release that serves decision models at `/v1/systemone`
/// (`jundot/omlx#4315`). Before it, Clef checkpoints load as plain VLMs.
pub const MIN_DECISION_VERSION: &str = "0.7.1.dev1";

/// How long to wait on the socket, in the two places a run can stall.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    /// Connect timeout. A local server should answer in milliseconds.
    pub connect: Duration,
    /// Read timeout: a 27B Clef model on a long prompt takes tens of seconds.
    pub read: Duration,
}

impl Default for Timeouts {
    fn default() -> Timeouts {
        Timeouts {
            connect: Duration::from_secs(5),
            read: Duration::from_secs(120),
        }
    }
}

impl Timeouts {
    /// Defaults, with the read timeout taken from `--timeout`.
    pub fn with_read_seconds(read_secs: u64) -> Timeouts {
        Timeouts {
            read: Duration::from_secs(read_secs),
            ..Timeouts::default()
        }
    }
}

/// A validated base URL, such as `http://127.0.0.1:8000`, with no trailing
/// slash, so paths can be appended without doubling it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint(String);

impl Endpoint {
    /// Validate a user-supplied base URL.
    ///
    /// Only the shape is checked: a URL that parses can still point at nothing,
    /// which is [`Client::health`]'s job to report.
    pub fn parse(raw: &str) -> Result<Endpoint> {
        let trimmed = raw.trim();
        let rest = trimmed
            .strip_prefix("http://")
            .or_else(|| trimmed.strip_prefix("https://"));
        let Some(rest) = rest else {
            return Err(Error::fatal(format!(
                "--endpoint must be an http:// or https:// URL, got '{raw}'"
            )));
        };
        if rest.is_empty() || rest.starts_with('/') {
            return Err(Error::fatal(format!("--endpoint '{raw}' has no host")));
        }
        Ok(Endpoint(trimmed.trim_end_matches('/').to_string()))
    }

    /// The base URL, without a trailing slash.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.0, path)
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What `GET /health` said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Health {
    /// The HTTP status. oMLX answers 503 with `"status": "loading"` while its
    /// pinned models are still being preloaded.
    pub http_status: u16,
    /// `status` from the body, e.g. `"ok"` or `"loading"`.
    pub status: Option<String>,
    /// Server version, e.g. `"0.7.1.dev1"`. Older builds may omit it.
    pub version: Option<String>,
    /// The server's default model, when it reports one.
    pub default_model: Option<String>,
}

impl Health {
    /// Whether the server answered that it is ready to serve.
    pub fn is_ready(&self) -> bool {
        if self.http_status != 200 {
            return false;
        }
        !matches!(self.status.as_deref(), Some("loading"))
    }
}

/// What a served model is, as far as the server will say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelKind {
    /// A decision model: answers typed questions in one forward pass.
    Decision,
    /// A chat or vision-language model: generates text, cannot score.
    Chat,
    /// The server did not report a type (pre-`0.7.1.dev1`).
    Unreported,
}

impl ModelKind {
    /// The label the CLI prints.
    pub fn as_str(self) -> &'static str {
        match self {
            ModelKind::Decision => "decision",
            ModelKind::Chat => "chat",
            ModelKind::Unreported => "unknown",
        }
    }
}

/// One alias the server serves, with its type when reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServedModel {
    /// The alias to send in a request (oMLX lists models under their alias).
    pub alias: String,
    /// What the server says it is.
    pub kind: ModelKind,
    /// The raw `model_type`/`engine_type` string, for messages.
    pub model_type: Option<String>,
}

impl ServedModel {
    /// Whether this is a decision model.
    pub fn is_decision(&self) -> bool {
        self.kind == ModelKind::Decision
    }
}

/// Every alias the server serves, plus whether it would tell us their types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    models: Vec<ServedModel>,
    types_reported: bool,
    types_unavailable: Option<u16>,
}

impl Catalog {
    /// Build a catalog from an already-joined listing (used by the join below
    /// and by tests).
    pub fn new(models: Vec<ServedModel>, types_reported: bool) -> Catalog {
        Catalog {
            models,
            types_reported,
            types_unavailable: None,
        }
    }

    /// Note that `/v1/models/status` answered, but not with a listing: a server
    /// that guards it (or does not have it) still lists its aliases, and that is
    /// the part a run cannot do without.
    pub fn with_types_unavailable(mut self, http_status: u16) -> Catalog {
        self.types_unavailable = Some(http_status);
        self
    }

    /// The served aliases, in the order the server listed them.
    pub fn models(&self) -> &[ServedModel] {
        &self.models
    }

    /// Whether `/v1/models/status` answered with a listing. When false, every
    /// model is [`ModelKind::Unreported`], either because the server predates
    /// that endpoint or because it refused to show it.
    pub fn types_reported(&self) -> bool {
        self.types_reported
    }

    /// The status `/v1/models/status` answered when it refused to show its
    /// listing: `401` or `403`, i.e. it wants a key. `None` when it answered
    /// with a listing, or when it has no such endpoint to show.
    ///
    /// This is the difference between "this install is too old to tell me" and
    /// "this install will not tell me", which is the difference between
    /// "upgrade oMLX" and "scoring may well work anyway".
    pub fn types_unavailable(&self) -> Option<u16> {
        self.types_unavailable
    }

    /// The model served under `alias`, if any.
    pub fn find(&self, alias: &str) -> Option<&ServedModel> {
        self.models.iter().find(|model| model.alias == alias)
    }

    /// The number of decision models the server reports.
    pub fn decision_count(&self) -> usize {
        self.models
            .iter()
            .filter(|model| model.is_decision())
            .count()
    }

    /// The aliases, for "did you mean" lists.
    pub fn aliases(&self) -> Vec<String> {
        self.models
            .iter()
            .map(|model| model.alias.clone())
            .collect()
    }
}

/// A blocking client for the endpoints above. One client per run; requests are
/// made one at a time, because oMLX serializes them anyway.
#[derive(Debug, Clone)]
pub struct Client {
    endpoint: Endpoint,
    agent: ureq::Agent,
    timeouts: Timeouts,
}

impl Client {
    /// Build a client for `endpoint` with the given timeouts.
    pub fn new(endpoint: Endpoint, timeouts: Timeouts) -> Client {
        let config = ureq::Agent::config_builder()
            .timeout_connect(Some(timeouts.connect))
            .timeout_global(Some(timeouts.read))
            // Statuses are read, not thrown: "too old" and "wrong alias" are
            // diagnosis inputs, not transport failures.
            .http_status_as_error(false)
            .build();
        Client {
            endpoint,
            agent: config.new_agent(),
            timeouts,
        }
    }

    /// The endpoint this client talks to.
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// `GET /health`.
    ///
    /// A 503 (models still loading) is a successful *read*, not an error: the
    /// caller needs the body and the status to explain it.
    pub fn health(&self) -> Result<Health> {
        let fetched = self.get("/health")?;
        let body: HealthBody = parse_json("/health", &fetched.body)?;
        Ok(Health {
            http_status: fetched.http_status,
            status: body.status,
            version: body.version,
            default_model: body.default_model,
        })
    }

    /// List the served aliases and, when the server reports them, their types.
    ///
    /// `/v1/models` is required; `/v1/models/status` is optional, because only
    /// `0.7.1.dev1` and later have it.
    pub fn catalog(&self) -> Result<Catalog> {
        let listing: ModelsEnvelope = parse_json("/v1/models", &self.get_success("/v1/models")?)?;

        // `/v1/models` answered, so the aliases are known. The optional type
        // endpoint is decoration on top: a server that guards it (401), never
        // had it (404) or dropped the connection still lists what it serves, and
        // refusing the whole catalog over the decoration is how a working server
        // gets called broken. Why it was unavailable is kept, so `doctor` can
        // tell "too old to say" from "will not say" (spec §11, ticket 01).
        let mut unavailable = None;
        let statuses = match self.get("/v1/models/status") {
            Ok(fetched) if (200..300).contains(&fetched.http_status) => {
                Some(parse_json::<StatusEnvelope>("/v1/models/status", &fetched.body)?.models)
            }
            // Guarded: the endpoint exists and we are not allowed to read it.
            // That is `doctor`'s "will not say", not its "too old to say".
            Ok(fetched) if matches!(fetched.http_status, 401 | 403) => {
                unavailable = Some(fetched.http_status);
                None
            }
            // Absent (404) or broken: treated as a server without the endpoint,
            // which is the version verdict's job to explain.
            Ok(_) => None,
            Err(_) => None,
        };

        let models = listing
            .data
            .iter()
            .filter(|entry| !entry.id.is_empty())
            .map(|entry| {
                let status = statuses
                    .as_ref()
                    .and_then(|entries| entries.iter().find(|status| status.serves(&entry.id)));
                match status {
                    None => ServedModel {
                        alias: entry.id.clone(),
                        kind: ModelKind::Unreported,
                        model_type: None,
                    },
                    Some(status) => ServedModel {
                        alias: entry.id.clone(),
                        kind: if status.is_decision() {
                            ModelKind::Decision
                        } else {
                            ModelKind::Chat
                        },
                        model_type: status
                            .model_type
                            .clone()
                            .or_else(|| status.engine_type.clone()),
                    },
                }
            })
            .collect();

        let catalog = Catalog::new(models, statuses.is_some());
        Ok(match unavailable {
            Some(http_status) => catalog.with_types_unavailable(http_status),
            None => catalog,
        })
    }

    /// `POST path` with a JSON body, returning the status and the body
    /// whatever the status is.
    ///
    /// Classifying the answer is the caller's job: a 400 names a rejected
    /// rubric, a 5xx names a hiccup, and both are decisions rather than
    /// transport failures.
    pub fn post_json(&self, path: &str, body: &serde_json::Value) -> Result<Fetched> {
        let url = self.endpoint.url(path);
        let mut response = self
            .agent
            .post(&url)
            .send_json(body)
            .map_err(|error| self.transport_error("POST", path, error))?;
        let http_status = response.status().as_u16();
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|error| self.transport_error("POST", path, error))?;
        Ok(Fetched { http_status, body })
    }

    /// Fetch `path`, requiring a 2xx and returning its body.
    fn get_success(&self, path: &str) -> Result<String> {
        let fetched = self.get(path)?;
        if !(200..300).contains(&fetched.http_status) {
            return Err(status_error(path, fetched.http_status));
        }
        Ok(fetched.body)
    }

    /// Fetch `path` and return the status and body whatever the status is.
    fn get(&self, path: &str) -> Result<Fetched> {
        let url = self.endpoint.url(path);
        let mut response = self
            .agent
            .get(&url)
            .call()
            .map_err(|error| self.transport_error("GET", path, error))?;
        let http_status = response.status().as_u16();
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|error| self.transport_error("GET", path, error))?;
        Ok(Fetched { http_status, body })
    }

    fn transport_error(&self, method: &str, path: &str, error: ureq::Error) -> Error {
        match error {
            ureq::Error::Timeout(_) => Error::new(
                ErrorKind::Timeout,
                format!(
                    "{method} {path} at {} timed out after {}s",
                    self.endpoint,
                    self.timeouts.read.as_secs()
                ),
            ),
            ureq::Error::StatusCode(status) => status_error(path, status),
            other => Error::new(
                ErrorKind::ServerUnreachable,
                format!("cannot reach {}: {other}", self.endpoint),
            ),
        }
    }
}

/// A request that finished at the transport level, whatever its status.
#[derive(Debug)]
pub struct Fetched {
    /// The HTTP status.
    pub http_status: u16,
    /// The response body, unparsed.
    pub body: String,
}

/// `/health`, reduced to what imagecull reads. Unknown fields are ignored: the
/// real payload carries uptime, load counts and more.
#[derive(Debug, Deserialize)]
struct HealthBody {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    default_model: Option<String>,
}

/// `/v1/models`: an OpenAI-shaped list, where `id` is the alias when the server
/// has one.
#[derive(Debug, Deserialize)]
struct ModelsEnvelope {
    #[serde(default)]
    data: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    #[serde(default)]
    id: String,
}

/// `/v1/models/status`: the extended listing, where types live.
#[derive(Debug, Deserialize)]
struct StatusEnvelope {
    #[serde(default)]
    models: Vec<StatusEntry>,
}

#[derive(Debug, Deserialize)]
struct StatusEntry {
    #[serde(default)]
    id: String,
    /// The user-facing alias, present only for models the server renamed.
    #[serde(default)]
    model_alias: Option<String>,
    #[serde(default)]
    model_type: Option<String>,
    #[serde(default)]
    engine_type: Option<String>,
}

impl StatusEntry {
    /// Whether this entry describes the model listed as `alias` in
    /// `/v1/models`, which is the alias when one is set and the directory name
    /// otherwise.
    fn serves(&self, alias: &str) -> bool {
        self.id == alias || self.model_alias.as_deref() == Some(alias)
    }

    /// oMLX reports decision models as `model_type` or `engine_type`
    /// `"decision"`, depending on where the checkpoint was detected.
    fn is_decision(&self) -> bool {
        let is = |field: &Option<String>| field.as_deref() == Some("decision");
        is(&self.model_type) || is(&self.engine_type)
    }
}

fn status_error(path: &str, status: u16) -> Error {
    Error::new(
        ErrorKind::from_status_code(status).unwrap_or(ErrorKind::BadResponse),
        format!("GET {path} returned HTTP {status}"),
    )
    .with_http_status(status)
}

fn parse_json<T: for<'de> Deserialize<'de>>(path: &str, body: &str) -> Result<T> {
    serde_json::from_str(body).map_err(|error| {
        Error::new(
            ErrorKind::BadResponse,
            format!("GET {path} did not answer the JSON imagecull expects: {error}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_normalise_trailing_slashes() {
        let endpoint = Endpoint::parse("http://127.0.0.1:8000/").unwrap();
        assert_eq!(endpoint.as_str(), "http://127.0.0.1:8000");
        assert_eq!(endpoint.url("/health"), "http://127.0.0.1:8000/health");

        let with_path = Endpoint::parse("https://box.local:8443/omlx").unwrap();
        assert_eq!(
            with_path.url("/v1/models"),
            "https://box.local:8443/omlx/v1/models"
        );
    }

    #[test]
    fn endpoints_reject_what_is_not_a_base_url() {
        for bad in [
            "",
            "127.0.0.1:8000",
            "ftp://example.com",
            "http://",
            "http:///v1",
        ] {
            let error = Endpoint::parse(bad).unwrap_err();
            assert!(error.is_fatal(), "{bad} should be a fatal argument error");
        }
    }

    #[test]
    fn a_loading_server_is_not_ready() {
        let loading = Health {
            http_status: 503,
            status: Some("loading".into()),
            version: Some("0.7.1.dev1".into()),
            default_model: None,
        };
        assert!(!loading.is_ready());

        let ok = Health {
            http_status: 200,
            status: Some("ok".into()),
            version: Some("0.7.1.dev1".into()),
            default_model: Some("clef-flash-oQ4e".into()),
        };
        assert!(ok.is_ready());
        assert!(
            !Health {
                http_status: 500,
                ..ok
            }
            .is_ready()
        );
    }

    #[test]
    fn health_parses_the_fields_it_reads_and_ignores_the_rest() {
        let body: HealthBody = serde_json::from_str(
            r#"{"status":"ok","version":"0.7.1.dev1","uptime_seconds":12.5,
                "models_loaded":1,"default_model":"clef-flash-oQ4e"}"#,
        )
        .unwrap();
        assert_eq!(body.version.as_deref(), Some("0.7.1.dev1"));
        assert_eq!(body.default_model.as_deref(), Some("clef-flash-oQ4e"));
        assert_eq!(body.status.as_deref(), Some("ok"));

        let minimal: HealthBody = serde_json::from_str("{}").unwrap();
        assert_eq!(minimal.version, None);
    }

    #[test]
    fn status_entries_match_by_id_or_alias_and_report_decision() {
        let decision: StatusEntry = serde_json::from_str(
            r#"{"id":"models--clef-flash","model_alias":"clef-flash-oQ4e",
                "model_type":"decision","engine_type":"decision"}"#,
        )
        .unwrap();
        assert!(decision.serves("clef-flash-oQ4e"));
        assert!(decision.serves("models--clef-flash"));
        assert!(!decision.serves("clef-27b"));
        assert!(decision.is_decision());

        let by_engine: StatusEntry =
            serde_json::from_str(r#"{"id":"clef","engine_type":"decision"}"#).unwrap();
        assert!(by_engine.is_decision());

        let chat: StatusEntry =
            serde_json::from_str(r#"{"id":"qwen","model_type":"qwen3_5","engine_type":"vlm"}"#)
                .unwrap();
        assert!(!chat.is_decision());
    }

    #[test]
    fn unreported_kind_is_labelled_unknown() {
        assert_eq!(ModelKind::Unreported.as_str(), "unknown");
        assert_eq!(ModelKind::Decision.as_str(), "decision");
        assert_eq!(ModelKind::Chat.as_str(), "chat");
    }
}
