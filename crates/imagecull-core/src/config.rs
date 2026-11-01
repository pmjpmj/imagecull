//! The config file: the values that stay true across runs, where a run looks
//! for them, and how they merge with the flags (spec §11.1, ADR 0003).
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

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::ingest::ScanOptions;
use crate::omlx;
use crate::preprocess::Params;
use crate::report::Outputs;

/// The config file that travels with a shoot folder.
pub const SHOOT_CONFIG: &str = "imagecull.toml";

/// The directory under the XDG config home that holds the user-level file.
pub const USER_CONFIG_DIR: &str = "imagecull";

/// The user-level config file's name, inside [`USER_CONFIG_DIR`].
pub const USER_CONFIG_FILE: &str = "config.toml";

/// The CSV a run writes when neither a flag nor a file names another one
/// (spec §11).
pub const DEFAULT_CSV: &str = "scores.csv";

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
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConfigFile {
    server: ServerSection,
    preview: PreviewSection,
    scan: ScanSection,
    outputs: OutputsSection,
    run: RunSection,
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
            timeout: self.server.timeout,
            max_edge: self.preview.max_edge,
            jpeg_quality: self.preview.jpeg_quality,
            recursive: self.scan.recursive,
            csv: anchor(&self.outputs.csv, base),
            json_scores: anchor(&self.outputs.json_scores, base),
            absolute: self.outputs.absolute,
            rubric: anchor(&self.run.rubric, base),
            shoot_context: self.run.shoot_context.clone(),
            dedupe: self.run.dedupe,
            csv_rewrite_every: self.run.csv_rewrite_every,
        }
    }

    /// Reject what the flags would reject, with the key that has to change
    /// named in the message. Nothing is clamped: a quality of 0 is a mistake,
    /// not a preference.
    fn validate(&self) -> Result<()> {
        if let Some(edge) = self.preview.max_edge
            && edge == 0
        {
            return Err(Error::fatal(
                "preview.max_edge must be at least 1 px, or nothing would be sent",
            ));
        }
        if let Some(quality) = self.preview.jpeg_quality
            && !(1..=100).contains(&quality)
        {
            return Err(Error::fatal(format!(
                "preview.jpeg_quality must be between 1 and 100, not {quality}"
            )));
        }
        if let Some(0) = self.run.csv_rewrite_every {
            return Err(Error::fatal("run.csv_rewrite_every must be at least 1 row"));
        }
        if let Some(0) = self.server.timeout {
            return Err(Error::fatal(
                "server.timeout must be at least 1 second, or no request could be waited for",
            ));
        }
        for (key, value) in [
            ("server.endpoint", self.server.endpoint.as_deref()),
            ("server.model", self.server.model.as_deref()),
        ] {
            if value.is_some_and(|text| text.trim().is_empty()) {
                return Err(Error::fatal(format!("{key} cannot be empty")));
            }
        }
        Ok(())
    }
}

/// `[server]`: the three values every command shares.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ServerSection {
    endpoint: Option<String>,
    model: Option<String>,
    timeout: Option<u64>,
}

/// `[preview]`: what gets sent, as [`Params`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct PreviewSection {
    max_edge: Option<u32>,
    jpeg_quality: Option<u8>,
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

/// `[outputs]`: where the report goes, as [`Outputs`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct OutputsSection {
    csv: Option<PathBuf>,
    json_scores: Option<PathBuf>,
    absolute: Option<bool>,
}

/// `[run]`: the rest of what a run needs to know.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RunSection {
    rubric: Option<PathBuf>,
    shoot_context: Option<String>,
    dedupe: Option<bool>,
    csv_rewrite_every: Option<usize>,
}

/// The values one source actually set — a config file's, or the flags'.
///
/// Every field is an `Option` on purpose: this is the only place that can tell
/// "unset" from "set to the value that happens to be the default", and the merge
/// depends on it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolved {
    pub endpoint: Option<String>,
    pub model: Option<String>,
    pub timeout: Option<u64>,
    pub max_edge: Option<u32>,
    pub jpeg_quality: Option<u8>,
    pub recursive: Option<bool>,
    pub csv: Option<PathBuf>,
    pub json_scores: Option<PathBuf>,
    pub absolute: Option<bool>,
    pub rubric: Option<PathBuf>,
    pub shoot_context: Option<String>,
    pub dedupe: Option<bool>,
    pub csv_rewrite_every: Option<usize>,
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
            timeout: flags.timeout.or(self.timeout),
            max_edge: flags.max_edge.or(self.max_edge),
            jpeg_quality: flags.jpeg_quality.or(self.jpeg_quality),
            recursive: flags.recursive.or(self.recursive),
            csv: flags.csv.or(self.csv),
            json_scores: flags.json_scores.or(self.json_scores),
            absolute: flags.absolute.or(self.absolute),
            rubric: flags.rubric.or(self.rubric),
            shoot_context: flags.shoot_context.or(self.shoot_context),
            dedupe: flags.dedupe.or(self.dedupe),
            csv_rewrite_every: flags.csv_rewrite_every.or(self.csv_rewrite_every),
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
            // Still optional: an absent alias defers to the rubric's own, which
            // is part of the rubric hash (spec §8.3).
            model: self.model,
            timeout: self.timeout.unwrap_or(DEFAULT_TIMEOUT_SECONDS),
            params: Params {
                max_edge: self.max_edge.unwrap_or(Params::DEFAULT_MAX_EDGE),
                jpeg_quality: self.jpeg_quality.unwrap_or(Params::DEFAULT_JPEG_QUALITY),
            },
            recursive: self.recursive.unwrap_or(false),
            outputs: Outputs {
                csv: self.csv.unwrap_or_else(|| PathBuf::from(DEFAULT_CSV)),
                json_scores: self.json_scores,
                absolute: self.absolute.unwrap_or(false),
            },
            rubric: self.rubric,
            shoot_context: self.shoot_context,
            dedupe: self.dedupe.unwrap_or(true),
            csv_rewrite_every: self
                .csv_rewrite_every
                .unwrap_or(crate::scoring::DEFAULT_CSV_REWRITE_EVERY),
        }
    }
}

/// A run's effective config: what is left once every source has been resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effective {
    /// The config file that contributed, when one did. Provenance only.
    pub source: Option<PathBuf>,
    /// The server to talk to.
    pub endpoint: String,
    /// The alias the photographer named, if any: `None` defers to the rubric's.
    pub model: Option<String>,
    /// The read timeout, in seconds.
    pub timeout: u64,
    /// How much picture to send.
    pub params: Params,
    /// Whether a run descends into subdirectories. The scan's `limit` stays a
    /// flag, because it describes the invocation.
    pub recursive: bool,
    /// Where the CSV and the raw answers go.
    pub outputs: Outputs,
    /// The rubric file to use instead of the embedded one.
    pub rubric: Option<PathBuf>,
    /// The photographer's framing of the shoot.
    pub shoot_context: Option<String>,
    /// Whether an identical preview may reuse an earlier frame's outcome.
    pub dedupe: bool,
    /// How many rows accumulate between CSV rewrites.
    pub csv_rewrite_every: usize,
}

impl Effective {
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
