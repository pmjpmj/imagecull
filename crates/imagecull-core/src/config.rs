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
use crate::preprocess::Params;
use crate::rubric::RubricSource;
use crate::secret::Secret;
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
            model: self.server.model.clone(),
            token_env: self.server.token_env.clone(),
            token: self.server.token.clone().map(Secret::new),
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
            group_max_images: self.group.max_images,
        }
    }

    /// Refuse the two `[server]` keys ADR 0019 folded into `endpoint` and `model`,
    /// by name and with the value that replaces them.
    ///
    /// Both are legal TOML and would otherwise be "unknown field" errors, which
    /// name nothing a photographer can act on.
    fn refuse_the_retired_wire_keys(&self) -> Result<()> {
        if self.server.path.is_some() {
            return Err(Error::fatal(
                "server.path is part of server.endpoint now: the endpoint is the whole address, \
                 `{model}` and all — fold the path into it and delete the key, e.g. \
                 endpoint = \"http://127.0.0.1:8000/v1/systemone\"",
            ));
        }
        if self.server.selector.is_some() {
            return Err(Error::fatal(
                "server.selector is gone: the body's `model` is server.model, and the endpoint \
                 spells the rest — put the short name in model and the id's own prefix in the \
                 address, e.g. endpoint = \"https://…/ai/run/@cf/cloudflare/{model}\", \
                 model = \"clef-flash\"",
            ));
        }
        Ok(())
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
        if let Some(max) = self.group.max_images
            && !(2..=keeper::MAX).contains(&max)
        {
            return Err(Error::fatal(format!(
                "group.max_images must be between 2 and {}, not {max} — one image is not a \
                 comparison, and {} is the batch a comparison's token budget was measured at",
                keeper::MAX,
                keeper::MAX
            )));
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
                "server.token",
                self.server.token.as_deref(),
                Some("delete the key or put the secret there"),
            ),
        ] {
            refuse_empty(key, value, remedy)?;
        }
        // The wire's two facts: the address a request goes to, with `{model}`
        // where the model name goes, and the credential it carries. Each refusal
        // names the key and the remedy, because none of them has a shape a
        // photographer could guess.
        self.refuse_the_retired_wire_keys()?;
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

/// `[server]`: what every command shares. Six keys are the wire — where a request
/// goes, what it says it is, and what credential it carries — and `timeout` is
/// how long a run waits for the answer.
///
/// `path` and `selector` are retired (ADR 0019): the address carries its own path,
/// `{model}` and all, and the model name is what the body says. Both are kept as
/// fields for one reason — so a file that still writes one is refused by name
/// with the fold-in remedy, rather than by a bare "unknown field".
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ServerSection {
    endpoint: Option<String>,
    model: Option<String>,
    /// Retired by ADR 0019: the path is part of `endpoint` now.
    path: Option<toml::Value>,
    /// Retired by ADR 0019: the body's `model` is `model`, always.
    selector: Option<toml::Value>,
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
/// `score` reads all of it. A comparison has its own pair under
/// [`GroupSection`], because a request carrying twenty frames of one moment is a
/// different payload problem from a request carrying one; the one key
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
    /// The most previews one comparative request carries.
    max_images: Option<usize>,
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
    pub model: Option<String>,
    /// The environment variable a bearer token is read from.
    pub token_env: Option<String>,
    /// A literal bearer token, wrapped so no `Debug` of this struct prints it.
    pub token: Option<Secret>,
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
    /// The most previews one comparative request carries.
    pub group_max_images: Option<usize>,
}

impl Resolved {
    /// These values, with `flags` winning wherever the flags said anything.
    ///
    /// This is the whole precedence rule, in one place and with no terminal
    /// involved: flag, then file, then (in [`Resolved::settle`]) the default.
    pub fn override_with(self, flags: Resolved) -> Resolved {
        Resolved {
            endpoint: flags.endpoint.or(self.endpoint),
            model: flags.model.or(self.model),
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
            group_max_images: flags.group_max_images.or(self.group_max_images),
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
                .unwrap_or_else(|| systemone::DEFAULT_ENDPOINT.to_string()),
            // Still optional: an absent alias defers to the rubric's own, which
            // is part of the rubric hash (spec §8.3).
            model: self.model,
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
            max_images: self.group_max_images.unwrap_or(keeper::MAX),
        }
    }
}

/// A run's effective config: what is left once every source has been resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct Effective {
    /// The config file that contributed, when one did. Provenance only.
    pub source: Option<PathBuf>,
    /// The server to talk to: the whole address a request is posted to, with
    /// `{model}` where the model name goes when the server's addressing needs it.
    pub endpoint: String,
    /// The alias the photographer named, if any: `None` defers to the rubric's.
    /// It fills the address's `{model}` and is the body's `model`.
    pub model: Option<String>,
    /// The environment variable a bearer token is read from, as written.
    pub token_env: Option<String>,
    /// A literal bearer token, as written, wrapped so it cannot be printed.
    pub token: Option<Secret>,
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
    /// The most previews one comparative request may carry, whose default is
    /// [`keeper::MAX`]. It fills [`keeper::Budget::max`]: a server that refuses
    /// more than four images per request is a scheduling fact about that server,
    /// not a change to how a group is judged, so the grouping hash does not
    /// cover it either.
    pub max_images: usize,
}

impl Effective {
    /// Whether this run addresses oMLX's own decision path, the one whose probes
    /// `models` reads. An address with `{model}` in it is some other server's
    /// addressing, and that command can say nothing about it.
    ///
    /// The comparison itself is [`systemone::is_omlx`].
    pub fn asks_omlx(&self) -> bool {
        systemone::is_omlx(&self.endpoint)
    }

    /// How the folder should be scanned: this config's breadth, the flag's limit.
    pub fn scan(&self, limit: Option<usize>) -> ScanOptions {
        ScanOptions {
            recursive: self.recursive,
            limit,
        }
    }

    /// The bearer token this config asks for, if any: the environment variable
    /// it names when that variable is set and non-empty, else the literal
    /// fallback.
    ///
    /// The environment arrives as a lookup rather than from `std::env` here,
    /// for the same reason [`Env`] takes its paths as arguments: the tests run
    /// in threads, and a test that set a process variable would race every
    /// other test in the binary. The front end passes
    /// `|name| std::env::var(name).ok()`; a test passes a closure over a map.
    ///
    /// A `token_env` that names a variable with nothing behind it, and no
    /// literal beside it, is fatal and names the variable: a typo'd name must
    /// not become an anonymous request the server then rejects 401 frames in a
    /// row for. The message names the variable, never a value.
    ///
    /// The variable's value is trimmed; the literal is not. Stray whitespace in
    /// an exported variable is a shell quoting accident nobody can see, and
    /// `Bearer  abc ` is a 401 per frame, while a literal is a value the
    /// photographer wrote out and can read — `token = " abc "` is theirs to
    /// keep.
    pub fn resolve_token(&self, lookup: impl Fn(&str) -> Option<String>) -> Result<Option<Secret>> {
        let Some(name) = &self.token_env else {
            return Ok(self.token.clone());
        };
        if let Some(value) = lookup(name).filter(|value| !value.trim().is_empty()) {
            return Ok(Some(Secret::new(value.trim())));
        }
        match &self.token {
            Some(token) => Ok(Some(token.clone())),
            None => Err(Error::fatal(format!(
                "{name} is not set and [server] token is empty — export the variable or write \
                 the key"
            ))),
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
