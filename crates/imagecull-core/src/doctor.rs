//! `imagecull doctor`: is this server able to score photos at all?
//!
//! The run-killing mistakes are all made before the first frame: oMLX is not
//! running, it is older than the release that serves decision models, the alias
//! in the rubric does not exist, or the alias is a plain chat/VLM checkpoint
//! that will answer with generated text instead of `/v1/systemone` scores.
//! Discovering any of those as 400 identical HTTP failures halfway through a
//! shoot is the failure this module exists to prevent.
//!
//! [`diagnose`] never fails: it turns transport errors and mismatches into the
//! one sentence that tells the photographer what to do, and an exit
//! classification for the CLI.
//!
//! [`check_file`] does the same for one of the photographer's own files: it runs
//! the preview pipeline end to end on it, so a camera or lens whose output the
//! extractor cannot handle is found before a 400-frame run, not during it.

use std::path::{Path, PathBuf};

use crate::error::{Error, ErrorKind, ExitCode};
use crate::omlx::{Client, MIN_DECISION_VERSION, ModelKind};
use crate::preprocess::{self, Params};
use crate::preview;
use crate::version::Version;

/// What `doctor` concluded about an endpoint and an alias.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The endpoint that was checked.
    pub endpoint: String,
    /// The model alias that was checked.
    pub alias: String,
    /// The conclusion.
    pub verdict: Verdict,
}

impl Report {
    /// Whether photos can be scored with this endpoint and alias.
    ///
    /// [`Verdict::TypesUnavailable`] counts as ready: the alias is served, and
    /// only the optional type report is missing. A server that keys that one
    /// endpoint still answers `/v1/systemone` — refusing to score against it
    /// would be the tool calling a working server broken.
    pub fn is_ready(&self) -> bool {
        matches!(
            self.verdict,
            Verdict::Ready { .. } | Verdict::TypesUnavailable { .. }
        )
    }

    /// The exit classification: ready, or fatal.
    pub fn exit(&self) -> ExitCode {
        if self.is_ready() {
            ExitCode::Ok
        } else {
            ExitCode::Fatal
        }
    }

    /// Whether a run may start against this endpoint and alias.
    ///
    /// Looser than [`Report::is_ready`] on purpose, because the two answer
    /// different questions. `doctor` reports only what it can *confirm*, and the
    /// front end needs that strictness to explain an install. A run, though,
    /// only refuses what it can *prove* unfit: nothing answering, something that
    /// is not oMLX, a server demanding an API key, a checkpoint still loading, an
    /// alias the server does not serve, or a server that reports model types and
    /// says the alias is not a decision model (or that it predates decision
    /// models altogether). A server that simply never says what its models are
    /// has not been shown to be unfit — its `/v1/systemone` may answer
    /// perfectly — so preflight lets it through (spec §51: "if the server
    /// exposes per-model status, confirm the model is a decision model").
    pub fn can_score(&self) -> bool {
        match &self.verdict {
            Verdict::Ready { .. } | Verdict::TypesUnavailable { .. } => true,
            // An absent type report is not a negative one: an install without
            // `/v1/models/status` has simply never been asked, and it serves the
            // alias this run wants.
            Verdict::NoModelTypes { .. } | Verdict::TooOld { found: None } => true,
            // A version below the floor is the one report that says the endpoint
            // this run depends on does not exist there.
            Verdict::TooOld { found: Some(_) }
            | Verdict::Unreachable { .. }
            | Verdict::NotReady { .. }
            | Verdict::Unauthorized { .. }
            | Verdict::NotOmlx { .. }
            | Verdict::AliasMissing { .. }
            | Verdict::PlainModel { .. } => false,
        }
    }

    /// One sentence: what is wrong (or right) and what to do about it.
    pub fn message(&self) -> String {
        let endpoint = &self.endpoint;
        let alias = &self.alias;
        match &self.verdict {
            Verdict::Ready { version } => {
                let server = match version {
                    Some(version) => format!("oMLX {version} at {endpoint}"),
                    None => format!("oMLX at {endpoint}"),
                };
                format!("{server} serves '{alias}' as a decision model — ready to score")
            }
            Verdict::Unreachable { detail } => format!(
                "{detail} — start oMLX (omlx serve), pass --endpoint if it listens elsewhere, \
                 or raise --timeout if it is still loading"
            ),
            Verdict::NotReady { http_status } => format!(
                "oMLX at {endpoint} is up but not serving yet (GET /health returned {http_status}) \
                 — wait for the checkpoint to finish loading, then run doctor again"
            ),
            Verdict::Unauthorized { http_status } => format!(
                "oMLX at {endpoint} refused the request (HTTP {http_status}) — it wants an API key, \
                 which imagecull v1 does not send; turn auth off for a loopback bind or point \
                 --endpoint at a keyless server"
            ),
            Verdict::NotOmlx { detail } => format!(
                "{detail} — something is answering at {endpoint} but not like oMLX; \
                 check that --endpoint points at omlx serve"
            ),
            Verdict::TooOld { found: Some(found) } => format!(
                "oMLX {found} at {endpoint} predates decision models, which arrived in \
                 {MIN_DECISION_VERSION} (jundot/omlx#4315) — upgrade oMLX; until then a Clef \
                 checkpoint loads as a plain VLM and returns text instead of scores"
            ),
            Verdict::TooOld { found: None } => format!(
                "oMLX at {endpoint} has no GET /v1/models/status, so it predates decision models \
                 ({MIN_DECISION_VERSION}, jundot/omlx#4315) — upgrade oMLX; until then a Clef \
                 checkpoint loads as a plain VLM and returns text instead of scores"
            ),
            Verdict::NoModelTypes { version } => {
                let server = match version {
                    Some(version) => format!("oMLX {version} at {endpoint}"),
                    None => format!("oMLX at {endpoint}"),
                };
                format!(
                    "{server} does not report model types (GET /v1/models/status is missing), so it \
                     cannot confirm '{alias}' is a decision model — reinstall oMLX \
                     {MIN_DECISION_VERSION} or later; a checkpoint served as a plain VLM returns \
                     text instead of scores"
                )
            }
            Verdict::AliasMissing { available } if available.is_empty() => format!(
                "no models are served at {endpoint} — load a Clef or Clef-Flash checkpoint in oMLX \
                 and run imagecull models to see its alias"
            ),
            Verdict::TypesUnavailable { http_status } => format!(
                "oMLX at {endpoint} serves '{alias}', but GET /v1/models/status returned HTTP \
                 {http_status}, so which models are decision models cannot be confirmed — scoring \
                 may still work: check it with `imagecull score <DIR> --limit 1` before a long run, \
                 or turn auth off for the loopback bind"
            ),
            Verdict::AliasMissing { available } => format!(
                "'{alias}' is not served at {endpoint} — served: {}; run imagecull models to see \
                 them, or pass --model",
                available.join(", ")
            ),
            Verdict::PlainModel { served_as } => format!(
                "'{alias}' is served as type '{served_as}', not a decision model — load a Clef or \
                 OpenJev checkpoint so oMLX detects it as type decision; a plain VLM generates text \
                 instead of answering /v1/systemone"
            ),
        }
    }
}

/// What `doctor` found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Reachable, new enough, alias present, served as a decision model.
    Ready {
        /// The server version, when it reported one that parses.
        version: Option<Version>,
    },
    /// Nothing answered at the endpoint.
    Unreachable {
        /// The transport failure, as the client described it.
        detail: String,
    },
    /// Something answered, but it is still starting.
    NotReady {
        /// The status `/health` returned (503 while pinned models load).
        http_status: u16,
    },
    /// The server demanded credentials.
    Unauthorized {
        /// 401 or 403.
        http_status: u16,
    },
    /// Something answered, but not as oMLX does.
    NotOmlx {
        /// What did not look like oMLX.
        detail: String,
    },
    /// oMLX, but from before decision models existed.
    TooOld {
        /// The version, when it could be read (or was reported at all).
        found: Option<Version>,
    },
    /// The server claims a new-enough version but does not say what its models
    /// are, so decision-ness cannot be confirmed.
    NoModelTypes {
        /// The version `/health` reported, verbatim.
        version: Option<String>,
    },
    /// The alias is served, but `/v1/models/status` would not say what it is.
    ///
    /// A server can guard that endpoint with an API key while `/v1/systemone`
    /// stays open, so this is a caveat rather than a failure: the alias exists,
    /// which is the part a run cannot do without.
    TypesUnavailable {
        /// What `/v1/models/status` answered (401, 403, 404, …).
        http_status: u16,
    },
    /// The endpoint does not serve the configured alias.
    AliasMissing {
        /// The aliases it does serve.
        available: Vec<String>,
    },
    /// The alias exists, but oMLX serves it as a chat/VLM model.
    PlainModel {
        /// The type the server reported, e.g. `vlm`.
        served_as: String,
    },
}

/// What `doctor <FILE>` found out about one of the photographer's files.
#[derive(Debug)]
pub struct FileCheck {
    /// The file that was tested.
    pub path: PathBuf,
    /// What the preview pipeline made of it.
    pub outcome: Result<FileFacts, Error>,
}

impl FileCheck {
    /// Whether the file went through the whole pipeline.
    pub fn is_ok(&self) -> bool {
        self.outcome.is_ok()
    }

    /// One sentence: what the pipeline produced, or why it could not.
    pub fn message(&self) -> String {
        let name = preview::file_name(&self.path);
        match &self.outcome {
            Ok(facts) => format!(
                "{name}: embedded preview {}x{} shown upright by orientation {}, sent as {}x{} {}, \
                 no EXIF — the preview pipeline works on this file",
                facts.stored_width,
                facts.stored_height,
                facts.orientation,
                facts.width,
                facts.height,
                kilobytes(facts.jpeg_bytes),
            ),
            Err(error) => format!("{name}: {error}"),
        }
    }
}

/// What the preview pipeline produced from one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFacts {
    /// The preview's stored width, before the orientation is applied.
    pub stored_width: u32,
    /// The preview's stored height, before the orientation is applied.
    pub stored_height: u32,
    /// The EXIF orientation the camera recorded, 1–8.
    pub orientation: u16,
    /// The width that would be sent.
    pub width: u32,
    /// The height that would be sent.
    pub height: u32,
    /// The size of the JPEG that would be sent.
    pub jpeg_bytes: usize,
}

/// Run the preview pipeline over `path` with `params` and report what came out.
///
/// This never fails: a file the pipeline cannot handle is the answer, in the
/// file's own classification (`unreadable`, `no_preview`, `preview_broken`).
pub fn check_file(path: &Path, params: &Params) -> FileCheck {
    let outcome = preview::extract(path).and_then(|embedded| {
        let encoded = preprocess::render(&embedded, params)?;
        Ok(FileFacts {
            stored_width: embedded.width,
            stored_height: embedded.height,
            orientation: embedded.orientation,
            width: encoded.width,
            height: encoded.height,
            jpeg_bytes: encoded.jpeg.len(),
        })
    });
    FileCheck {
        path: path.to_path_buf(),
        outcome,
    }
}

/// `bytes` as a rounded number of kilobytes, for a sentence a person reads.
fn kilobytes(bytes: usize) -> String {
    format!("{} KB", bytes.div_ceil(1024))
}

/// Check `alias` against the server behind `client`, and explain the result.
pub fn diagnose(client: &Client, alias: &str) -> Report {
    let endpoint = client.endpoint().to_string();
    let verdict = match client.health() {
        Err(error) => classify_health_error(error),
        Ok(health) if health.http_status == 200 => check_models(client, alias, &health.version),
        Ok(health) => match health.http_status {
            401 | 403 => Verdict::Unauthorized {
                http_status: health.http_status,
            },
            503 => Verdict::NotReady {
                http_status: health.http_status,
            },
            other => Verdict::NotOmlx {
                detail: format!("GET /health returned HTTP {other}"),
            },
        },
    };
    Report {
        endpoint,
        alias: alias.to_string(),
        verdict,
    }
}

/// A transport failure on `/health` means either nothing is listening or
/// something other than oMLX is.
fn classify_health_error(error: Error) -> Verdict {
    match error.kind() {
        Some(ErrorKind::ServerUnreachable) | Some(ErrorKind::Timeout) => Verdict::Unreachable {
            detail: error.message().to_string(),
        },
        _ => Verdict::NotOmlx {
            detail: error.message().to_string(),
        },
    }
}

/// The server is up; now the version, the alias, and its type.
fn check_models(client: &Client, alias: &str, raw_version: &Option<String>) -> Verdict {
    let catalog = match client.catalog() {
        Ok(catalog) => catalog,
        Err(error) => {
            if let Some(http_status) = error.http_status()
                && matches!(http_status, 401 | 403)
            {
                return Verdict::Unauthorized { http_status };
            }
            return match error.kind() {
                Some(ErrorKind::ServerUnreachable) | Some(ErrorKind::Timeout) => {
                    Verdict::Unreachable {
                        detail: error.message().to_string(),
                    }
                }
                _ => Verdict::NotOmlx {
                    detail: error.message().to_string(),
                },
            };
        }
    };

    let parsed = raw_version.as_deref().and_then(Version::parse);
    if let Some(found) = &parsed
        && let Some(floor) = Version::parse(MIN_DECISION_VERSION)
        && *found < floor
    {
        return Verdict::TooOld {
            found: Some(found.clone()),
        };
    }

    // The alias comes first: an endpoint that does not serve it cannot score,
    // however much else is or is not known about the server.
    let Some(model) = catalog.find(alias) else {
        return Verdict::AliasMissing {
            available: catalog.aliases(),
        };
    };
    let model = model.clone();

    if !catalog.types_reported() {
        // No `/v1/models/status` listing. A server that guarded the endpoint or
        // dropped the call has not said whether it is new enough, and its aliases
        // are known, so the honest verdict is a caveat rather than a diagnosis.
        if let Some(http_status) = catalog.types_unavailable() {
            return Verdict::TypesUnavailable { http_status };
        }
        // Never had it: on a server that predates it, that is the "too old"
        // verdict; on one that claims a new version, the install is lying and
        // types cannot be confirmed either way.
        return match parsed {
            Some(found) => Verdict::NoModelTypes {
                version: Some(found.to_string()),
            },
            None => Verdict::TooOld { found: None },
        };
    }

    match model.kind {
        ModelKind::Decision => Verdict::Ready { version: parsed },
        ModelKind::Chat => Verdict::PlainModel {
            served_as: model
                .model_type
                .clone()
                .unwrap_or_else(|| "chat".to_string()),
        },
        ModelKind::Unreported => Verdict::TooOld { found: parsed },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(verdict: Verdict) -> Report {
        Report {
            endpoint: "http://127.0.0.1:8000".to_string(),
            alias: "clef-flash-oQ4e".to_string(),
            verdict,
        }
    }

    #[test]
    fn ready_is_the_only_non_fatal_verdict() {
        let ready = report(Verdict::Ready {
            version: Version::parse("0.7.1.dev1"),
        });
        assert!(ready.is_ready());
        assert_eq!(ready.exit(), ExitCode::Ok);
        assert_eq!(
            ready.message(),
            "oMLX 0.7.1.dev1 at http://127.0.0.1:8000 serves 'clef-flash-oQ4e' as a decision \
             model — ready to score"
        );

        let caveat = report(Verdict::TypesUnavailable { http_status: 401 });
        assert!(
            caveat.is_ready(),
            "a server that guards only its type report can still answer the questions"
        );
        assert_eq!(caveat.exit(), ExitCode::Ok);
        assert!(caveat.message().contains("HTTP 401"));
        assert!(caveat.message().contains("clef-flash-oQ4e"));

        for verdict in [
            Verdict::Unreachable {
                detail: "cannot reach http://127.0.0.1:8000: Connection refused".into(),
            },
            Verdict::NotReady { http_status: 503 },
            Verdict::Unauthorized { http_status: 401 },
            Verdict::NotOmlx {
                detail: "GET /health returned HTTP 404".into(),
            },
            Verdict::TooOld { found: None },
            Verdict::NoModelTypes { version: None },
            Verdict::AliasMissing {
                available: vec!["clef-27b".into()],
            },
            Verdict::PlainModel {
                served_as: "vlm".into(),
            },
        ] {
            let report = report(verdict);
            assert!(!report.is_ready());
            assert_eq!(report.exit(), ExitCode::Fatal);
        }
    }

    #[test]
    fn an_old_server_names_the_release_it_needs() {
        let message = report(Verdict::TooOld {
            found: Version::parse("0.7.0"),
        })
        .message();
        assert!(
            message.starts_with("oMLX 0.7.0 at http://127.0.0.1:8000 predates decision models")
        );
        assert!(message.contains("0.7.1.dev1"));
        assert!(message.contains("plain VLM"));

        let unknown = report(Verdict::TooOld { found: None }).message();
        assert!(unknown.contains("/v1/models/status"));
    }

    #[test]
    fn a_missing_alias_lists_what_is_served() {
        let message = report(Verdict::AliasMissing {
            available: vec!["clef-27b-4bit".into(), "qwen3.5-4b".into()],
        })
        .message();
        assert!(message.contains("'clef-flash-oQ4e' is not served"));
        assert!(message.contains("served: clef-27b-4bit, qwen3.5-4b"));
        assert!(message.contains("imagecull models"));

        let empty = report(Verdict::AliasMissing { available: vec![] }).message();
        assert!(empty.contains("no models are served"));
    }

    #[test]
    fn a_plain_model_explains_why_it_cannot_score() {
        let message = report(Verdict::PlainModel {
            served_as: "vlm".into(),
        })
        .message();
        assert!(
            message.contains("'clef-flash-oQ4e' is served as type 'vlm', not a decision model")
        );
        assert!(message.contains("/v1/systemone"));
    }

    #[test]
    fn every_failure_says_what_to_do() {
        let hints = [
            report(Verdict::Unreachable {
                detail: "cannot reach http://127.0.0.1:8000: Connection refused".into(),
            })
            .message(),
            report(Verdict::NotReady { http_status: 503 }).message(),
            report(Verdict::Unauthorized { http_status: 401 }).message(),
            report(Verdict::NotOmlx {
                detail: "GET /health returned HTTP 404".into(),
            })
            .message(),
        ];
        assert!(hints[0].contains("start oMLX"));
        assert!(hints[1].contains("finish loading"));
        assert!(hints[2].contains("API key"));
        assert!(hints[3].contains("--endpoint"));
    }
}
