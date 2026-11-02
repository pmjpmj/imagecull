//! The config file: the values that stay true across runs, where a run looks
//! for them, and how they merge with the flags (spec §11.1, ADR 0003, ADR 0010,
//! ADR 0014, ADR 0016).
//!
//! A config file is a *source* of config, never the effective config itself:
//! [`Resolved`] holds only the keys a file actually set, so "the file did not
//! say" stays distinguishable from "the file said the default value".
//! [`Resolved::override_with`] then merges in what the flags said and
//! [`Resolved::settle`] applies the built-in defaults, which is the same order
//! every other setting in this tool resolves in: flag, then file, then default.
//!
//! Nothing here prints and nothing exits; a bad file is an [`Error`] the front
//! end renders.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::cull;
use crate::error::{Error, Result};
use crate::group::{self, Thresholds};
use crate::ingest::ScanOptions;
use crate::keeper;
use crate::omlx;
use crate::preprocess::Params;
use crate::rubric::RubricSource;
use crate::sidecar::Outputs;
use crate::systemone;

/// The config file that travels with a shoot folder.
pub const SHOOT_CONFIG: &str = "imagecull.toml";

/// The directory under the XDG config home that holds the user-level file.
pub const USER_CONFIG_DIR: &str = "imagecull";

/// The user-level config file's name, inside [`USER_CONFIG_DIR`].
pub const USER_CONFIG_FILE: &str = "config.toml";

/// The read timeout a run uses when neither a flag nor a file names another
/// one, in seconds.
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 120;

/// Where a run looks for a config file, as paths rather than as a process.
///
/// Discovery takes this as an argument instead of reading `std::env` itself so
/// the whole search order is a pure function of paths: the tests run in threads,
/// and a test that set `XDG_CONFIG_HOME` to prove the order would race every
/// other test in the binary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env {
    /// The working directory, which a relative `--config` and the shoot config
    /// are resolved against.
    pub cwd: PathBuf,
    /// `XDG_CONFIG_HOME`, when it is set and absolute (per the XDG spec).
    pub xdg_config_home: Option<PathBuf>,
    /// `HOME`, used for the XDG default of `~/.config` when the variable is
    /// unset.
    pub home: Option<PathBuf>,
}

impl Env {
    /// The paths a real run looks through, read from the process environment.
    pub fn of_process() -> Result<Env> {
        let cwd = std::env::current_dir().map_err(|error| {
            Error::fatal(format!("cannot determine the working directory: {error}"))
        })?;
        Ok(Env {
            cwd,
            // A relative `XDG_CONFIG_HOME` is ignored rather than joined to the
            // working directory, as the XDG base-directory spec requires.
            xdg_config_home: variable("XDG_CONFIG_HOME").filter(|path| path.is_absolute()),
            home: variable("HOME"),
        })
    }
}

/// A non-empty environment variable, as a path.
fn variable(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// A config file that was found, and the path it was found at.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    /// The absolute path that was read.
    pub path: PathBuf,
    /// What it contained.
    pub file: ConfigFile,
}

impl Loaded {
    /// The keys this file sets, with its relative paths anchored to its own
    /// directory, so a shoot config keeps working wherever the folder is
    /// mounted.
    pub fn resolve(&self) -> Resolved {
        self.file.resolve(self.path.parent())
    }
}

/// Find the config file a run should read, at most one of them (spec §11.1).
///
/// `--config` wins when given, and a `--config` that names nothing is a fatal
/// error rather than a silent fall-through: a run that quietly ignored the file
/// it was told to use would report numbers nobody can attribute. Otherwise the
/// shoot config, then the user config, then nothing.
pub fn discover(explicit: Option<&Path>, env: &Env) -> Result<Option<Loaded>> {
    if let Some(path) = explicit {
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            env.cwd.join(path)
        };
        return Ok(Some(Loaded {
            file: read(&path)?,
            path,
        }));
    }

    let mut candidates = vec![env.cwd.join(SHOOT_CONFIG)];
    candidates.extend(user_config(env));
    for candidate in candidates {
        if candidate.is_file() {
            return Ok(Some(Loaded {
                file: read(&candidate)?,
                path: candidate,
            }));
        }
    }
    Ok(None)
}

/// Where the user-level config lives: `$XDG_CONFIG_HOME/imagecull/config.toml`,
/// or `~/.config/imagecull/config.toml` when the variable is unset.
fn user_config(env: &Env) -> Option<PathBuf> {
    let base = match &env.xdg_config_home {
        Some(path) => path.clone(),
        None => env.home.as_ref()?.join(".config"),
    };
    Some(base.join(USER_CONFIG_DIR).join(USER_CONFIG_FILE))
}

/// Read one config file, or explain why it cannot be read.
fn read(path: &Path) -> Result<ConfigFile> {
    if !path.exists() {
        return Err(Error::fatal(format!(
            "the config file {} does not exist",
            path.display()
        )));
    }
    let text = fs::read_to_string(path).map_err(|error| {
        Error::fatal(format!(
            "cannot read the config file {}: {error}",
            path.display()
        ))
    })?;
    let file: ConfigFile = toml::from_str(&text).map_err(|error| {
        Error::fatal(format!(
            "the config file {} is not usable: {error}",
            path.display()
        ))
    })?;
    file.validate()?;
    Ok(file)
}

/// The TOML file as written, before validation.
///
/// Every section and every key is optional, and an unknown one is an error
/// rather than a warning: a typo'd `max_edeg` would otherwise cost a whole run
/// at the wrong resolution.
///
/// The four live sections split by who reads them: the two command sections,
/// `[score]` and `[group]`, own what one command needs; `[server]` and `[scan]`
/// hold what every command shares (ADR 0016). The three after them are retired
/// sections kept only so a file that still writes one is refused by name.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConfigFile {
    server: ServerSection,
    scan: ScanSection,
    score: ScoreSection,
    group: GroupSection,
    /// Retired by ADR 0016: every key moved to `[score]`.
    preview: RetiredPreview,
    /// Retired by ADR 0016: `json_scores` moved to `[score]`; `csv` and
    /// `absolute` died with the report.
    outputs: RetiredOutputs,
    /// Retired by ADR 0016: every key moved to `[score]`.
    run: RetiredRun,
}

impl ConfigFile {
    /// Parse and validate a config file's text.
    pub fn from_toml(text: &str) -> Result<ConfigFile> {
        let file: ConfigFile = toml::from_str(text)
            .map_err(|error| Error::fatal(format!("the config is not valid TOML: {error}")))?;
        file.validate()?;
        Ok(file)
    }

    /// The keys this file sets, with relative paths anchored to `base`.
    pub fn resolve(&self, base: Option<&Path>) -> Resolved {
        Resolved {
            endpoint: self.server.endpoint.clone(),
            path: self.server.path.clone(),
            model: self.server.model.clone(),
            selector: self.server.selector.clone(),
            token_env: self.server.token_env.clone(),
            token: self.server.token.clone(),
            truncate: self.server.truncate,
            timeout: self.server.timeout,
            score_max_edge: self.score.max_edge,
            score_jpeg_quality: self.score.jpeg_quality,
            recursive: self.scan.recursive,
            json_scores: anchor(&self.score.json_scores, base),
            rubric: self
                .score
                .rubric
                .clone()
                .map(|source| source.anchored(base)),
            shoot_context: self.score.shoot_context.clone(),
            dedupe: self.score.dedupe,
            group_gap: self.group.gap,
            group_similarity: self.group.similarity,
            group_exposure_stops: self.group.exposure_stops,
            group_max_edge: self.group.max_edge,
            group_jpeg_quality: self.group.jpeg_quality,
            group_cull_jobs: self.group.cull_jobs,
        }
    }

    /// Reject what the flags would reject, with the key that has to change
    /// named in the message. Nothing is clamped: a quality of 0 is a mistake,
    /// not a preference.
    fn validate(&self) -> Result<()> {
        // The retired sections first: a file built for the old shape is told to
        // move a key, not told that a number it is about to move is out of range.
        self.refuse_the_retired_sections()?;
        if let Some(edge) = self.score.max_edge
            && edge == 0
        {
            return Err(Error::fatal(
                "score.max_edge must be at least 1 px, or nothing would be sent",
            ));
        }
        if let Some(quality) = self.score.jpeg_quality
            && !(1..=100).contains(&quality)
        {
            return Err(Error::fatal(format!(
                "score.jpeg_quality must be between 1 and 100, not {quality}"
            )));
        }
        if let Some(0) = self.server.timeout {
            return Err(Error::fatal(
                "server.timeout must be at least 1 second, or no request could be waited for",
            ));
        }
        if let Some(edge) = self.group.max_edge
            && edge == 0
        {
            return Err(Error::fatal(
                "group.max_edge must be at least 1 px, or nothing would be sent",
            ));
        }
        if let Some(quality) = self.group.jpeg_quality
            && !(1..=100).contains(&quality)
        {
            return Err(Error::fatal(format!(
                "group.jpeg_quality must be between 1 and 100, not {quality}"
            )));
        }
        if let Some(0) = self.group.cull_jobs {
            return Err(Error::fatal(
                "group.cull_jobs must be at least 1, or no group would be culled",
            ));
        }
        if let Some(gap) = self.group.gap
            && !(gap > 0.0 && gap.is_finite())
        {
            return Err(Error::fatal(
                "group.gap must be a positive number of seconds, or a group could never hold \
                 two frames",
            ));
        }
        if let Some(similarity) = self.group.similarity
            && !(0.0..=1.0).contains(&similarity)
        {
            return Err(Error::fatal(format!(
                "group.similarity is a cosine between 0 and 1, not {similarity}"
            )));
        }
        if let Some(stops) = self.group.exposure_stops
            && !(stops.is_finite() && stops >= 0.0)
        {
            return Err(Error::fatal(format!(
                "group.exposure_stops must be zero or more stops, not {stops}"
            )));
        }
        for (key, value, remedy) in [
            ("server.endpoint", self.server.endpoint.as_deref(), None),
            ("server.model", self.server.model.as_deref(), None),
            (
                "server.selector",
                self.server.selector.as_deref(),
                Some("delete the key to send the model instead"),
            ),
            (
                "server.token",
                self.server.token.as_deref(),
                Some("delete the key or put the secret there"),
            ),
        ] {
            refuse_empty(key, value, remedy)?;
        }
        // The wire's four facts: what a request addresses, what it says it is,
        // and what it carries to be let in. Each refusal names the key and the
        // remedy, because none of them has a range a photographer could guess.
        if let Some(path) = &self.server.path {
            if !path.starts_with('/') {
                return Err(Error::fatal(format!(
                    "server.path must be a request path beginning with '/', not {path}"
                )));
            }
            if has_a_bad_placeholder(path) {
                return Err(Error::fatal(format!(
                    "server.path '{path}' has a placeholder that is not {{model}}"
                )));
            }
        }
        if let Some(name) = &self.server.token_env
            && !names_a_variable(name)
        {
            return Err(Error::fatal(format!(
                "server.token_env names an environment variable, not '{name}'"
            )));
        }
        Ok(())
    }

    /// Refuse a config file written for the three-section shape, naming the key
    /// that replaced the one it set.
    ///
    /// The move is reported per key rather than per section: a file with three
    /// keys under `[preview]` is three lines to edit, and a message that only
    /// said "`[preview]` is gone" would leave the photographer to work out
    /// where. `outputs.csv` and `outputs.absolute` are the exceptions, because
    /// they did not move — they died with the CSV report and say so.
    fn refuse_the_retired_sections(&self) -> Result<()> {
        for (old, set, new) in [
            (
                "preview.max_edge",
                self.preview.max_edge.is_some(),
                "score.max_edge",
            ),
            (
                "preview.jpeg_quality",
                self.preview.jpeg_quality.is_some(),
                "score.jpeg_quality",
            ),
            (
                "outputs.json_scores",
                self.outputs.json_scores.is_some(),
                "score.json_scores",
            ),
            ("run.rubric", self.run.rubric.is_some(), "score.rubric"),
            (
                "run.shoot_context",
                self.run.shoot_context.is_some(),
                "score.shoot_context",
            ),
            ("run.dedupe", self.run.dedupe.is_some(), "score.dedupe"),
        ] {
            if set {
                return Err(Error::fatal(format!(
                    "{old} has moved to {new}: [score] holds what a rating needs, the way \
                     [group] holds what a cull needs — move the key"
                )));
            }
        }
        if self.outputs.csv.is_some() {
            return Err(Error::fatal(
                "outputs.csv is gone: a run writes each rating into an XMP sidecar beside its \
                 frame, so there is no report path to set — delete the key",
            ));
        }
        if self.outputs.absolute.is_some() {
            return Err(Error::fatal(
                "outputs.absolute is gone with the report: there is no file column to spell \
                 absolutely — delete the key",
            ));
        }
        Ok(())
    }
}

/// `[server]`: what every command shares. Seven keys are the wire — where a
/// request goes, what it says it is, and what credential it carries — and
/// `timeout` is how long a run waits for the answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ServerSection {
    endpoint: Option<String>,
    /// The request path, as a template: `{model}` becomes the run's model id.
    path: Option<String>,
    model: Option<String>,
    /// The body's `model` value, when it differs from the model id.
    selector: Option<String>,
    /// The environment variable a bearer token is read from.
    token_env: Option<String>,
    /// A literal bearer token, for a machine whose environment cannot hold one.
    token: Option<String>,
    /// Whether the body carries oMLX's `truncate: true`.
    truncate: Option<bool>,
    timeout: Option<u64>,
}

/// `[score]`: what a rating needs — the payload shape it sends, where its raw
/// answers go, and how it judges.
///
/// `score` reads all of it, and `doctor` reads the preview pair because its file
/// check renders the preview a rating would send. A comparison has its own pair
/// under [`GroupSection`], because a request carrying twenty frames of one moment
/// is a different payload problem from a request carrying one; the one key
/// `groups` borrows from here is `shoot_context`, since a shoot's framing is a
/// property of the shoot rather than of the pass (ADR 0016).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ScoreSection {
    /// Cap on the longest edge of the preview sent for one frame's rating.
    max_edge: Option<u32>,
    /// JPEG quality of that preview, 1–100.
    jpeg_quality: Option<u8>,
    /// Where every raw model answer is written, when it is asked for.
    json_scores: Option<PathBuf>,
    /// A built-in rubric's name or a path to a rubric file: the same value
    /// `--rubric` takes, so the two can never disagree about what a name is.
    #[serde(deserialize_with = "rubric_source")]
    rubric: Option<RubricSource>,
    /// The photographer's framing of the shoot.
    shoot_context: Option<String>,
    /// Whether an identical preview may reuse an earlier frame's outcome.
    dedupe: Option<bool>,
}

/// `[scan]`: which files a run covers, as far as a file may decide it.
///
/// `limit` is deliberately absent: a smoke test on the first N files describes
/// the invocation, not the shoot.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ScanSection {
    recursive: Option<bool>,
}

/// `[preview]`, retired by ADR 0016: named only so the refusal can point at
/// `[score] max_edge` / `[score] jpeg_quality` instead of leaving a photographer
/// guessing at an unknown field.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RetiredPreview {
    max_edge: Option<toml::Value>,
    jpeg_quality: Option<toml::Value>,
}

/// `[outputs]`, retired by ADR 0016: `json_scores` moved to `[score]`, and `csv`
/// and `absolute` are gone with the CSV report.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RetiredOutputs {
    /// Gone with the report: there is no `file` column to spell absolutely.
    absolute: Option<toml::Value>,
    /// Gone: named only so the refusal can say where ratings go instead.
    csv: Option<toml::Value>,
    json_scores: Option<toml::Value>,
}

/// `[run]`, retired by ADR 0016: every key moved to `[score]`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RetiredRun {
    rubric: Option<toml::Value>,
    shoot_context: Option<toml::Value>,
    dedupe: Option<toml::Value>,
}

/// `[group]`: how close two frames have to be to be one moment, as
/// [`Thresholds`], and how much picture one image of a comparison carries, as
/// [`Params`]. Read by `groups`, which is the only command that groups.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct GroupSection {
    gap: Option<f64>,
    similarity: Option<f64>,
    exposure_stops: Option<f64>,
    max_edge: Option<u32>,
    jpeg_quality: Option<u8>,
    /// How many groups the cull judges at once.
    cull_jobs: Option<usize>,
}

/// Refuse a key that was set to nothing, naming it and — when the key has one —
/// the remedy. A blank value is always a mistake rather than a preference: an
/// empty endpoint posts nowhere, and an empty token is a request made anonymous.
fn refuse_empty(key: &str, value: Option<&str>, remedy: Option<&str>) -> Result<()> {
    if value.is_some_and(|text| text.trim().is_empty()) {
        let remedy = remedy
            .map(|remedy| format!(" — {remedy}"))
            .unwrap_or_default();
        return Err(Error::fatal(format!("{key} cannot be empty{remedy}")));
    }
    Ok(())
}

/// Whether `path` carries a `{…}` placeholder that is not `{model}`.
///
/// A path is a template with exactly one thing to interpolate, so anything else
/// between braces is a typo that would sail onto the wire and 404, rather than a
/// value to pass through. The delimiter has to be the brace rather than a
/// separator, because a Workers AI model id contains `/` and `@` itself. A brace
/// that never closes is not a placeholder at all, so it is left alone: the path
/// validator judges placeholders, not braces.
fn has_a_bad_placeholder(path: &str) -> bool {
    let mut rest = path;
    while let Some(open) = rest.find('{') {
        let from_open = &rest[open..];
        let Some(close) = from_open.find('}') else {
            return false;
        };
        if &from_open[..=close] != "{model}" {
            return true;
        }
        rest = &from_open[close + 1..];
    }
    false
}

/// Whether `name` could be an environment variable: non-empty, and free of the
/// separators that would make it a shell assignment or two words.
fn names_a_variable(name: &str) -> bool {
    !name.is_empty() && !name.contains('=') && !name.chars().any(char::is_whitespace)
}

/// `[score] rubric` as the text a photographer wrote, read the way `--rubric`
/// reads it.
fn rubric_source<'de, D>(deserializer: D) -> std::result::Result<Option<RubricSource>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let text = String::deserialize(deserializer)?;
    RubricSource::parse(&text)
        .map(Some)
        .map_err(serde::de::Error::custom)
}

/// The values one source actually set — a config file's, or the flags'.
///
/// Every field is an `Option` on purpose: this is the only place that can tell
/// "unset" from "set to the value that happens to be the default", and the merge
/// depends on it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Resolved {
    pub endpoint: Option<String>,
    /// The request path template, `{model}` and all.
    pub path: Option<String>,
    pub model: Option<String>,
    /// The body's `model` value, when the server wants a selector rather than
    /// the id.
    pub selector: Option<String>,
    /// The environment variable a bearer token is read from.
    pub token_env: Option<String>,
    /// A literal bearer token.
    pub token: Option<String>,
    /// Whether the body carries `truncate: true`.
    pub truncate: Option<bool>,
    pub timeout: Option<u64>,
    /// The longest edge of a rating's preview, in pixels.
    pub score_max_edge: Option<u32>,
    /// The JPEG quality of a rating's preview, 1–100.
    pub score_jpeg_quality: Option<u8>,
    pub recursive: Option<bool>,
    pub json_scores: Option<PathBuf>,
    /// Which rubric to judge by: a built-in's name, or a path to a file.
    pub rubric: Option<RubricSource>,
    pub shoot_context: Option<String>,
    pub dedupe: Option<bool>,
    /// The most seconds between two frames of one group.
    pub group_gap: Option<f64>,
    /// The least cosine similarity between a frame and its group's first frame.
    pub group_similarity: Option<f64>,
    /// The most brightness drift, in stops, between two frames of one group.
    pub group_exposure_stops: Option<f64>,
    /// The longest edge of one image in a comparison, in pixels.
    pub group_max_edge: Option<u32>,
    /// The JPEG quality of one image in a comparison, 1–100.
    pub group_jpeg_quality: Option<u8>,
    /// How many groups the cull judges at once.
    pub group_cull_jobs: Option<usize>,
}

impl Resolved {
    /// These values, with `flags` winning wherever the flags said anything.
    ///
    /// This is the whole precedence rule, in one place and with no terminal
    /// involved: flag, then file, then (in [`Resolved::settle`]) the default.
    pub fn override_with(self, flags: Resolved) -> Resolved {
        Resolved {
            endpoint: flags.endpoint.or(self.endpoint),
            path: flags.path.or(self.path),
            model: flags.model.or(self.model),
            selector: flags.selector.or(self.selector),
            token_env: flags.token_env.or(self.token_env),
            token: flags.token.or(self.token),
            truncate: flags.truncate.or(self.truncate),
            timeout: flags.timeout.or(self.timeout),
            score_max_edge: flags.score_max_edge.or(self.score_max_edge),
            score_jpeg_quality: flags.score_jpeg_quality.or(self.score_jpeg_quality),
            recursive: flags.recursive.or(self.recursive),
            json_scores: flags.json_scores.or(self.json_scores),
            rubric: flags.rubric.or(self.rubric),
            shoot_context: flags.shoot_context.or(self.shoot_context),
            dedupe: flags.dedupe.or(self.dedupe),
            group_gap: flags.group_gap.or(self.group_gap),
            group_similarity: flags.group_similarity.or(self.group_similarity),
            group_exposure_stops: flags.group_exposure_stops.or(self.group_exposure_stops),
            group_max_edge: flags.group_max_edge.or(self.group_max_edge),
            group_jpeg_quality: flags.group_jpeg_quality.or(self.group_jpeg_quality),
            group_cull_jobs: flags.group_cull_jobs.or(self.group_cull_jobs),
        }
    }

    /// Apply the built-in defaults, giving the effective config a run uses.
    ///
    /// `source` is the config file this came from, kept for provenance: the run
    /// summary prints it beside the rubric hash, the way the ledger header
    /// already keeps the endpoint.
    pub fn settle(self, source: Option<PathBuf>) -> Effective {
        Effective {
            source,
            endpoint: self
                .endpoint
                .unwrap_or_else(|| omlx::DEFAULT_ENDPOINT.to_string()),
            // The template is kept whole: interpolating it is the client's job,
            // and it cannot be done until the model is known.
            path: self.path.unwrap_or_else(|| systemone::PATH.to_string()),
            // Still optional: an absent alias defers to the rubric's own, which
            // is part of the rubric hash (spec §8.3).
            model: self.model,
            // Left as written: a selector is spread across two settled values,
            // and [`Effective::selector`] is where the fallback lives.
            selector: self.selector,
            // Carried, not resolved: `settle` stays pure, so the environment is
            // read once, at the boundary, by whoever builds the client.
            token_env: self.token_env,
            token: self.token,
            truncate: self.truncate.unwrap_or(true),
            timeout: self.timeout.unwrap_or(DEFAULT_TIMEOUT_SECONDS),
            score_params: Params {
                max_edge: self.score_max_edge.unwrap_or(Params::DEFAULT_MAX_EDGE),
                jpeg_quality: self
                    .score_jpeg_quality
                    .unwrap_or(Params::DEFAULT_JPEG_QUALITY),
            },
            group_params: Params {
                max_edge: self
                    .group_max_edge
                    .unwrap_or(keeper::default_preview().max_edge),
                jpeg_quality: self
                    .group_jpeg_quality
                    .unwrap_or(keeper::default_preview().jpeg_quality),
            },
            recursive: self.recursive.unwrap_or(false),
            outputs: Outputs {
                json_scores: self.json_scores,
            },
            rubric: self.rubric,
            shoot_context: self.shoot_context,
            dedupe: self.dedupe.unwrap_or(true),
            thresholds: Thresholds {
                gap: self
                    .group_gap
                    .map_or(group::DEFAULT_GAP, Duration::from_secs_f64),
                similarity: self.group_similarity.unwrap_or(group::DEFAULT_SIMILARITY),
                exposure_stops: self
                    .group_exposure_stops
                    .unwrap_or(group::DEFAULT_EXPOSURE_STOPS),
            },
            cull_jobs: self.group_cull_jobs.unwrap_or(cull::DEFAULT_JOBS),
        }
    }
}

/// A run's effective config: what is left once every source has been resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct Effective {
    /// The config file that contributed, when one did. Provenance only.
    pub source: Option<PathBuf>,
    /// The server to talk to.
    pub endpoint: String,
    /// The request path template this run addresses: `{model}` becomes the
    /// model id, and the default is oMLX's fixed path.
    pub path: String,
    /// The alias the photographer named, if any: `None` defers to the rubric's.
    pub model: Option<String>,
    /// The body's `model` value when the server wants a selector: `None` sends
    /// [`Effective::model`] whole.
    pub selector: Option<String>,
    /// The environment variable a bearer token is read from, as written.
    pub token_env: Option<String>,
    /// A literal bearer token, as written. A plain string here rather than a
    /// redacting `Secret`, which is ticket 03: `Effective`'s derived `Debug`
    /// would print it, so until that ticket lands nothing may debug-print one.
    pub token: Option<String>,
    /// Whether the request body carries oMLX's `truncate: true`.
    pub truncate: bool,
    /// The read timeout, in seconds.
    pub timeout: u64,
    /// How much picture to send for one frame's rating.
    pub score_params: Params,
    /// How much picture to send, one image of a comparison at a time. Its
    /// default is the measured comparison edge ([`keeper::EDGE`]), not `score`'s
    /// cap: a comparison carries up to twenty frames of one moment.
    pub group_params: Params,
    /// Whether a run descends into subdirectories. The scan's `limit` stays a
    /// flag, because it describes the invocation.
    pub recursive: bool,
    /// Where the raw answers go, when they are asked for.
    pub outputs: Outputs,
    /// Which rubric to judge by: a built-in's name, or a path to a file. `None`
    /// is the default built-in.
    pub rubric: Option<RubricSource>,
    /// The photographer's framing of the shoot.
    pub shoot_context: Option<String>,
    /// Whether an identical preview may reuse an earlier frame's outcome.
    pub dedupe: bool,
    /// How close two frames have to be to be one moment. `groups` decides with
    /// them; `score` rates every frame alone and never asks.
    pub thresholds: Thresholds,
    /// How many groups `groups` culls at once: a cap on the comparative requests
    /// in flight, whose default is [`cull::DEFAULT_JOBS`]. It is scheduling, not
    /// algorithm, so the grouping hash does not cover it.
    pub cull_jobs: usize,
}

impl Effective {
    /// What the request body's `model` field should say: the selector when the
    /// file named one, and the model id whole otherwise.
    ///
    /// Named for the field it fills rather than `selector()`, because the two
    /// differ: [`Effective::selector`] is only what a file wrote. The fallback
    /// lives here rather than in [`Resolved::settle`], so "the file said
    /// nothing" and "the file said the model" stay distinguishable — the first
    /// defers to `--model`, the second does not.
    ///
    /// `None` means no model is settled at all: the rubric's own alias fills the
    /// field later (spec §8.3), so a caller must not read it as "send none".
    pub fn body_model(&self) -> Option<&str> {
        self.selector.as_deref().or(self.model.as_deref())
    }

    /// How the folder should be scanned: this config's breadth, the flag's limit.
    pub fn scan(&self, limit: Option<usize>) -> ScanOptions {
        ScanOptions {
            recursive: self.recursive,
            limit,
        }
    }
}

/// Anchor a relative path to the directory the config file was found in.
fn anchor(path: &Option<PathBuf>, base: Option<&Path>) -> Option<PathBuf> {
    let path = path.as_ref()?;
    match base {
        Some(base) if path.is_relative() => Some(base.join(path)),
        _ => Some(path.clone()),
    }
}
