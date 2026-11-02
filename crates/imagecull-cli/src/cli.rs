//! The command line itself: what a photographer types.
//!
//! Only arguments live here — no rendering, no decisions. Every flag maps to a
//! core value, so nothing about the pipeline can be reachable only from the
//! terminal; and no default is filled here either, because a config file may
//! supply it. Each command asks [`CommonArgs::effective`] for the values it will
//! actually use, which is why the flags are `Option<T>`: this layer has to say
//! "the photographer did not say" without guessing what the answer is.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use imagecull_core::config::{self, Effective, Loaded, Resolved};
use imagecull_core::group;
use imagecull_core::ingest::ScanOptions;
use imagecull_core::omlx::{self, Client, Endpoint, Timeouts};
use imagecull_core::rubric::{Builtin, RubricSource};

/// Where `--dry-run` writes its previews, relative to the working directory.
pub const DEFAULT_PREVIEWS_DIR: &str = "previews";

/// Where `groups` writes its measures, relative to the working directory.
pub const DEFAULT_GROUPS_CSV: &str = "groups.csv";

#[derive(Parser, Debug)]
#[command(
    name = "imagecull",
    version,
    about = "Score a folder of Nikon mirrorless RAW files with a local Clef decision model",
    subcommand_required = true,
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// List what the local oMLX serves, flagging decision models
    Models(CommonArgs),
    /// Check that the local oMLX can score photos, and that a file's preview
    /// survives the pipeline
    Doctor(DoctorArgs),
    /// Score a shoot folder
    Score(ScoreArgs),
    /// Group a shoot folder by moment and cull each group with clef
    Groups(GroupsArgs),
}

/// The options every command shares: which server to talk to, and where the
/// values come from.
#[derive(Args, Debug)]
pub struct CommonArgs {
    /// Base URL of the oMLX server (default http://127.0.0.1:8000)
    #[arg(long, value_name = "URL")]
    pub endpoint: Option<String>,

    /// Model alias to ask: score defaults to the rubric's alias, doctor, groups
    /// and models to clef-flash-oQ4e
    #[arg(short = 'm', long, value_name = "ALIAS")]
    pub model: Option<String>,

    /// Read timeout in seconds (default 120)
    #[arg(long, value_name = "SECS", value_parser = clap::value_parser!(u64).range(1..))]
    pub timeout: Option<u64>,

    /// Read values from this config file instead of the discovered one
    #[arg(long, value_name = "FILE", conflicts_with = "no_config")]
    pub config: Option<PathBuf>,

    /// Ignore any config file and use the built-in defaults
    #[arg(long)]
    pub no_config: bool,
}

impl CommonArgs {
    /// The values this run actually uses: the config file's, under the flags',
    /// under the built-in defaults.
    ///
    /// `flags` is what this command's own flags said; the shared ones come from
    /// here. Any error is the message a photographer reads, already naming the
    /// file and the key that has to change.
    pub fn effective(&self, flags: Resolved) -> Result<Effective, String> {
        let loaded = self.load()?;
        let source = loaded.as_ref().map(|loaded| loaded.path.clone());
        let file = loaded.map(|loaded| loaded.resolve()).unwrap_or_default();
        Ok(file.override_with(flags).settle(source))
    }

    /// The shared flags as one source of config, before the file is consulted.
    pub fn overrides(&self) -> Resolved {
        Resolved {
            endpoint: self.endpoint.clone(),
            model: self.model.clone(),
            timeout: self.timeout,
            ..Resolved::default()
        }
    }

    /// Find the config file this run reads, if any: `--config`, the shoot
    /// config, or the user config, and nothing at all under `--no-config`.
    fn load(&self) -> Result<Option<Loaded>, String> {
        if self.no_config {
            return Ok(None);
        }
        let env = config::Env::of_process().map_err(|error| error.to_string())?;
        config::discover(self.config.as_deref(), &env).map_err(|error| error.to_string())
    }

    /// The alias `doctor` and `models` work with: the configured one, or the
    /// reference server's default. A run prefers the rubric's alias instead.
    pub fn alias<'a>(&self, effective: &'a Effective) -> &'a str {
        effective.model.as_deref().unwrap_or(omlx::DEFAULT_MODEL)
    }

    /// Build the client the effective config describes, or the message
    /// explaining why it describes nothing.
    pub fn client(&self, effective: &Effective) -> Result<Client, String> {
        let endpoint = Endpoint::parse(&effective.endpoint).map_err(|error| error.to_string())?;
        Ok(Client::new(
            endpoint,
            Timeouts::with_read_seconds(effective.timeout),
        ))
    }
}

#[derive(Args, Debug)]
pub struct DoctorArgs {
    #[command(flatten)]
    pub common: CommonArgs,

    /// A RAW file to run the preview pipeline on, as a self-test
    #[arg(value_name = "FILE")]
    pub file: Option<PathBuf>,

    /// Cap on the longest edge of the sent preview, in pixels (default 1568)
    #[arg(long, value_name = "PX", value_parser = clap::value_parser!(u32).range(1..))]
    pub max_edge: Option<u32>,

    /// JPEG quality of the sent preview, 1-100 (default 88)
    #[arg(long, value_name = "QUALITY", value_parser = clap::value_parser!(u8).range(1..=100))]
    pub jpeg_quality: Option<u8>,
}

impl DoctorArgs {
    /// What this command's own flags said. The self-test proves the pipeline the
    /// photographer is about to use, so it takes the same knobs a run would.
    pub fn overrides(&self) -> Resolved {
        Resolved {
            max_edge: self.max_edge,
            jpeg_quality: self.jpeg_quality,
            ..self.common.overrides()
        }
    }
}

#[derive(Args, Debug)]
pub struct ScoreArgs {
    /// The shoot folder to scan
    #[arg(value_name = "DIR")]
    pub dir: PathBuf,

    #[command(flatten)]
    pub common: CommonArgs,

    /// The rubric to judge by: `general`, `press` or `holiday` for a built-in,
    /// or a path to a rubric file (default: general)
    #[arg(long, value_name = "NAME|FILE", value_parser = RubricSource::parse)]
    pub rubric: Option<RubricSource>,

    /// Free text about the shoot, passed to the model as state.shoot_context,
    /// so a dim, high-ISO street shoot is judged as that
    #[arg(long, value_name = "TEXT")]
    pub shoot_context: Option<String>,

    /// Write the exact previews a scoring run would send, and make no requests
    #[arg(long)]
    pub dry_run: bool,

    /// Where --dry-run writes its previews
    #[arg(long, value_name = "DIR", default_value = DEFAULT_PREVIEWS_DIR)]
    pub previews_dir: PathBuf,

    /// Cap on the longest edge of the sent preview, in pixels (default 1568)
    #[arg(long, value_name = "PX", value_parser = clap::value_parser!(u32).range(1..))]
    pub max_edge: Option<u32>,

    /// JPEG quality of the sent preview, 1-100 (default 88)
    #[arg(long, value_name = "QUALITY", value_parser = clap::value_parser!(u8).range(1..=100))]
    pub jpeg_quality: Option<u8>,

    /// Also scan subdirectories; hidden directories are never scanned.
    /// Takes an optional true/false, so a config file that turns it on can be
    /// turned off for one run
    #[arg(
        long,
        value_name = "BOOL",
        num_args = 0..=1,
        default_missing_value = "true"
    )]
    pub recursive: Option<bool>,

    /// Score only the first N files, as a smoke test
    #[arg(long, value_name = "N", value_parser = positive_count)]
    pub limit: Option<usize>,

    /// Also write every raw model answer to this JSON file, so a surprising
    /// score loses nothing
    #[arg(long, value_name = "FILE")]
    pub json_scores: Option<PathBuf>,

    /// Score every frame independently instead of reusing the outcome of an
    /// identical preview
    #[arg(long)]
    pub no_dedupe: bool,

    /// Rate frames again even though their sidecars already carry a rating
    #[arg(long)]
    pub force: bool,

    /// Write into a sidecar another application owns (darktable), which will
    /// rewrite it from its own history
    #[arg(long)]
    pub allow_foreign: bool,

    /// Score the folder and print what each sidecar would become, writing
    /// nothing
    #[arg(long, conflicts_with_all = ["quiet", "verbose"])]
    pub print_sidecars: bool,

    /// Suppress the per-frame line; the outcome and a one-line summary remain,
    /// so a script can read them
    #[arg(long, conflicts_with = "verbose")]
    pub quiet: bool,

    /// One line per frame with the bytes that were sent, the answers at full
    /// precision and the status of every frame
    #[arg(long)]
    pub verbose: bool,
}

/// The cull: which frames are the same moment, which frame clef chose to keep,
/// and the red mark on the rest.
///
/// The technical pass runs first and needs nothing: grouping and measuring are
/// local CPU. The comparison is the part that asks a server, and `--no-cull`
/// leaves it out, so the whole model-free pass is still one command on a machine
/// with no model running.
#[derive(Args, Debug)]
pub struct GroupsArgs {
    /// The shoot folder to scan
    #[arg(value_name = "DIR")]
    pub dir: PathBuf,

    #[command(flatten)]
    pub common: CommonArgs,

    /// Where to write the measures CSV
    #[arg(long, value_name = "FILE", default_value = DEFAULT_GROUPS_CSV)]
    pub out: PathBuf,

    /// Also scan subdirectories; hidden directories are never scanned. Takes an
    /// optional true/false, as `score` does
    #[arg(
        long,
        value_name = "BOOL",
        num_args = 0..=1,
        default_missing_value = "true"
    )]
    pub recursive: Option<bool>,

    /// Group only the first N files, as a smoke test
    #[arg(long, value_name = "N", value_parser = positive_count)]
    pub limit: Option<usize>,

    /// The most seconds between two frames of one group (default 2.0)
    #[arg(long, value_name = "SECS", value_parser = gap_seconds)]
    pub group_gap: Option<f64>,

    /// The least cosine similarity between a frame and its group's first frame
    /// (default 0.90)
    #[arg(long, value_name = "SIMILARITY", value_parser = similarity)]
    pub group_similarity: Option<f64>,

    /// The most brightness drift, in stops, between two frames of one group
    /// (default 2.0); `0` asks for one exposure exactly
    #[arg(long, value_name = "STOPS", value_parser = exposure_stops)]
    pub group_exposure_stops: Option<f64>,

    /// Cap on the longest edge of one image in a comparison, in pixels (default
    /// 512): every frame of a group is sent at this size, so it sets what detail
    /// the comparison can see and what one request costs
    #[arg(long, value_name = "PX", value_parser = clap::value_parser!(u32).range(1..))]
    pub max_edge: Option<u32>,

    /// JPEG quality of one image in a comparison, 1-100 (default 88)
    #[arg(long, value_name = "QUALITY", value_parser = clap::value_parser!(u8).range(1..=100))]
    pub jpeg_quality: Option<u8>,

    /// How many frames to read at once, on this machine's own cores
    /// (default: one per core)
    #[arg(short = 'j', long, value_name = "N", value_parser = parallel_jobs)]
    pub jobs: Option<usize>,

    /// How many groups to cull at once (default 4); different groups' requests
    /// may be in flight together
    #[arg(
        long,
        value_name = "N",
        value_parser = parallel_jobs,
        conflicts_with = "no_cull"
    )]
    pub cull_jobs: Option<usize>,

    /// Group and measure only: make no request and write no label
    #[arg(long, conflicts_with = "print_sidecars")]
    pub no_cull: bool,

    /// Ask clef, then print what each sidecar would become, writing nothing
    #[arg(long, conflicts_with_all = ["no_cull", "quiet", "verbose"])]
    pub print_sidecars: bool,

    /// Write into a sidecar another application owns (darktable), which will
    /// rewrite it from its own history
    #[arg(long)]
    pub allow_foreign: bool,

    /// Print only the summary, not the per-frame progress or the group lines
    #[arg(long, conflicts_with = "verbose")]
    pub quiet: bool,

    /// One line per group with the frames it compared and the keeper it found
    #[arg(long)]
    pub verbose: bool,
}

impl GroupsArgs {
    /// What this command's own flags said, for the config file to sit under.
    ///
    /// `--recursive` is included even though a run's breadth is the file's
    /// subject and not the invocation's: the flag takes an explicit `false`, so
    /// a shoot config that turns recursion on can still be turned off for one
    /// run. `--max-edge` and `--jpeg-quality` are the comparison's own pair, and
    /// land on the `group_` keys: `score`'s `--max-edge` does not reach a cull,
    /// and a cull's does not reach a rating. `--limit`, `--out`, `--jobs`,
    /// `--cull-jobs` and the cull/console flags describe the invocation and stay
    /// out.
    pub fn overrides(&self) -> Resolved {
        Resolved {
            recursive: self.recursive,
            group_gap: self.group_gap,
            group_similarity: self.group_similarity,
            group_exposure_stops: self.group_exposure_stops,
            group_max_edge: self.max_edge,
            group_jpeg_quality: self.jpeg_quality,
            ..self.common.overrides()
        }
    }

    /// What this command actually runs with: the effective config, with the two
    /// values that describe the invocation — the smoke-test limit and the degree
    /// — taken from the flags.
    pub fn options(&self, effective: &Effective) -> group::Options {
        group::Options {
            scan: ScanOptions {
                recursive: effective.recursive,
                limit: self.limit,
            },
            thresholds: effective.thresholds,
            jobs: self.jobs,
        }
    }
}

/// A gap in seconds: positive, because a group of zero width is not a moment.
fn gap_seconds(text: &str) -> Result<f64, String> {
    let seconds: f64 = text
        .parse()
        .map_err(|_| format!("'{text}' is not a number of seconds"))?;
    if !(seconds > 0.0 && seconds.is_finite()) {
        return Err("the gap has to be a positive number of seconds".to_string());
    }
    Ok(seconds)
}

/// A similarity threshold: a cosine, so it lies between 0 and 1.
fn similarity(text: &str) -> Result<f64, String> {
    let value: f64 = text
        .parse()
        .map_err(|_| format!("'{text}' is not a similarity"))?;
    if !(0.0..=1.0).contains(&value) {
        return Err("a similarity is a cosine between 0 and 1".to_string());
    }
    Ok(value)
}

/// An exposure tolerance in stops: any number from zero up, because "the same
/// exposure" is a legitimate answer and a negative drift is not a thing.
fn exposure_stops(text: &str) -> Result<f64, String> {
    let value: f64 = text
        .parse()
        .map_err(|_| format!("'{text}' is not a number of stops"))?;
    if !(value.is_finite() && value >= 0.0) {
        return Err("the exposure tolerance has to be zero or more stops".to_string());
    }
    Ok(value)
}

/// The `--version` line: the version, and the rubric a run uses by default.
///
/// The hash is what makes the line worth printing: two builds with different
/// default rubrics are not the same tool, and a photographer comparing CSVs
/// needs the identity of the rubric that made them (spec §8.3, §11).
pub fn version_line() -> String {
    match Builtin::DEFAULT.rubric() {
        Ok(rubric) => format!(
            "imagecull {} (rubric {}, model {})",
            env!("CARGO_PKG_VERSION"),
            crate::progress::rubric_name(Some(Builtin::DEFAULT), &rubric.hash(rubric.model())),
            rubric.model(),
        ),
        // Unreachable in a tested build, but a version line must never be the
        // thing that fails.
        Err(_) => format!("imagecull {}", env!("CARGO_PKG_VERSION")),
    }
}

/// A count of files: at least one, because `--limit 0` is a typo, not a run.
fn positive_count(text: &str) -> Result<usize, String> {
    let count: usize = text
        .parse()
        .map_err(|_| format!("'{text}' is not a number of files"))?;
    if count == 0 {
        return Err("a limit of 0 files would score nothing; leave --limit out".to_string());
    }
    Ok(count)
}

/// A number of frames to read at once: at least one, because `--jobs 0` asks a
/// run to do nothing at all rather than to do it one frame at a time.
fn parallel_jobs(text: &str) -> Result<usize, String> {
    let jobs: usize = text
        .parse()
        .map_err(|_| format!("'{text}' is not a number of frames"))?;
    if jobs == 0 {
        return Err("zero jobs would read nothing; leave --jobs out for one per core".to_string());
    }
    Ok(jobs)
}

impl ScoreArgs {
    /// What this command's flags said, before any config file is consulted.
    ///
    /// `--no-dedupe` is the one flag whose absence is meaningful: it can only
    /// turn dedupe off, so leaving it out leaves the decision to the file.
    pub fn overrides(&self) -> Resolved {
        Resolved {
            max_edge: self.max_edge,
            jpeg_quality: self.jpeg_quality,
            recursive: self.recursive,
            json_scores: self.json_scores.clone(),
            rubric: self.rubric.clone(),
            shoot_context: self.shoot_context.clone(),
            dedupe: self.no_dedupe.then_some(false),
            ..self.common.overrides()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use imagecull_core::group::Thresholds;
    use imagecull_core::preprocess::Params;

    use super::*;

    fn parse(arguments: &[&str]) -> Command {
        parse_result(arguments).expect("valid arguments").command
    }

    /// The effective config of a parsed `groups` with no config file on disk,
    /// which is what the parser alone can prove: the file itself is proven in
    /// the core's own tests.
    fn groups_options(args: &GroupsArgs) -> group::Options {
        args.options(&args.overrides().settle(None))
    }

    fn parse_result(arguments: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("imagecull").chain(arguments.iter().copied()))
    }

    /// The effective config of a parsed `score` with no config file on disk.
    /// Parsing only: the file itself is proven in the core's own tests.
    fn effective(args: &ScoreArgs) -> Effective {
        args.overrides().settle(None)
    }

    #[test]
    fn score_defaults_are_the_specs_defaults() {
        let Command::Score(args) = parse(&["score", "shoot"]) else {
            panic!("not a score command");
        };
        assert_eq!(args.dir, PathBuf::from("shoot"));
        assert!(!args.dry_run);
        assert_eq!(
            args.common.endpoint, None,
            "the default is resolved later, so a file may supply it"
        );
        assert_eq!(
            args.common.model, None,
            "--model defaults to the rubric's alias"
        );
        assert_eq!(args.common.timeout, None);
        assert_eq!(args.rubric, None);
        assert_eq!(args.shoot_context, None);
        assert_eq!(args.previews_dir, PathBuf::from(DEFAULT_PREVIEWS_DIR));
        assert_eq!(args.max_edge, None);
        assert_eq!(args.jpeg_quality, None);
        assert_eq!(args.recursive, None);
        assert_eq!(args.limit, None);
        assert_eq!(args.json_scores, None);
        assert_eq!(args.common.config, None);
        assert!(!args.common.no_config);
        assert!(
            !args.quiet && !args.verbose,
            "a plain run shows a line per frame and the summary"
        );

        let effective = effective(&args);
        assert_eq!(effective.endpoint, omlx::DEFAULT_ENDPOINT);
        assert_eq!(effective.model, None, "no alias means the rubric's own");
        assert_eq!(effective.timeout, 120);
        assert_eq!(effective.params.max_edge, Params::DEFAULT_MAX_EDGE);
        assert_eq!(effective.params.jpeg_quality, Params::DEFAULT_JPEG_QUALITY);
        assert!(!effective.recursive);
        assert!(!effective.scan(None).recursive);
        assert_eq!(effective.outputs.json_scores, None, "no report path at all");
        assert!(effective.dedupe);
        assert_eq!(effective.source, None);
    }

    #[test]
    fn score_accepts_a_smoke_test_run() {
        let Command::Score(args) = parse(&[
            "score",
            "shoot",
            "--dry-run",
            "--recursive",
            "--limit",
            "20",
            "--max-edge",
            "1024",
            "--jpeg-quality",
            "70",
            "--previews-dir",
            "/tmp/out",
        ]) else {
            panic!("not a score command");
        };
        assert!(args.dry_run);
        assert_eq!(args.recursive, Some(true));
        assert_eq!(args.limit, Some(20));
        assert_eq!(args.previews_dir, PathBuf::from("/tmp/out"));

        let effective = effective(&args);
        assert_eq!(effective.params.max_edge, 1024);
        assert_eq!(effective.params.jpeg_quality, 70);
        assert!(effective.recursive);
        assert_eq!(effective.scan(Some(20)).limit, Some(20));
    }

    #[test]
    fn a_config_that_turns_recursion_on_can_be_turned_off_for_one_run() {
        let Command::Score(args) = parse(&["score", "shoot", "--recursive", "false"]) else {
            panic!("not a score command");
        };
        assert_eq!(
            args.recursive,
            Some(false),
            "the flag has to be able to say no, or a file could only ever add"
        );
    }

    #[test]
    fn score_takes_the_raw_answers_flag() {
        let Command::Score(args) = parse(&["score", "shoot", "--json-scores", "scores/march.json"])
        else {
            panic!("not a score command");
        };
        let outputs = effective(&args).outputs;
        assert_eq!(
            outputs.json_scores,
            Some(PathBuf::from("scores/march.json"))
        );
    }

    #[test]
    fn the_report_flags_are_gone() {
        for flag in ["--out", "--absolute", "--resume", "--fresh"] {
            assert!(
                parse_result(&["score", "shoot", flag, "x"]).is_err(),
                "{flag} belonged to the report and is not a flag any more"
            );
        }
    }

    #[test]
    fn nonsense_edges_and_qualities_are_rejected() {
        let cases: [&[&str]; 5] = [
            &["score", "shoot", "--max-edge", "0"],
            &["score", "shoot", "--jpeg-quality", "0"],
            &["score", "shoot", "--jpeg-quality", "101"],
            &["score", "shoot", "--limit", "0"],
            &["score", "shoot", "--timeout", "0"],
        ];
        for arguments in cases {
            assert!(
                parse_result(arguments).is_err(),
                "{arguments:?} should be refused"
            );
        }
    }

    #[test]
    fn score_takes_the_rubric_the_context_and_the_server() {
        let Command::Score(args) = parse(&[
            "score",
            "shoot",
            "--endpoint",
            "http://127.0.0.1:9000",
            "--model",
            "clef-27b-oQ4e",
            "--rubric",
            "mine.toml",
            "--shoot-context",
            "street at dusk, high ISO expected",
            "--timeout",
            "240",
        ]) else {
            panic!("not a score command");
        };
        assert_eq!(
            args.common.endpoint.as_deref(),
            Some("http://127.0.0.1:9000")
        );
        assert_eq!(args.common.model.as_deref(), Some("clef-27b-oQ4e"));
        assert_eq!(
            args.rubric,
            Some(RubricSource::File(PathBuf::from("mine.toml")))
        );
        assert_eq!(
            args.shoot_context.as_deref(),
            Some("street at dusk, high ISO expected")
        );
        assert_eq!(args.common.timeout, Some(240));

        let effective = effective(&args);
        assert_eq!(effective.endpoint, "http://127.0.0.1:9000");
        assert_eq!(effective.model.as_deref(), Some("clef-27b-oQ4e"));
        assert_eq!(
            effective.rubric,
            Some(RubricSource::File(PathBuf::from("mine.toml"))),
            "a path stays a path through every layer"
        );
        assert_eq!(effective.timeout, 240);
    }

    #[test]
    fn a_bare_name_selects_a_built_in_rubric() {
        let Command::Score(args) = parse(&["score", "shoot", "--rubric", "holiday"]) else {
            panic!("not a score command");
        };
        assert_eq!(
            args.rubric,
            Some(RubricSource::Builtin(Builtin::Holiday)),
            "the flag reads a bare name as a built-in, exactly as the config file does"
        );
        assert_eq!(
            effective(&args).rubric,
            Some(RubricSource::Builtin(Builtin::Holiday)),
            "and no layer under the flag turns it back into a path"
        );

        let Command::Score(args) = parse(&["score", "shoot", "--rubric", "General"]) else {
            panic!("not a score command");
        };
        assert_eq!(
            args.rubric,
            Some(RubricSource::File(PathBuf::from("General"))),
            "three lowercase words, not a fuzzy match: `General` is a path"
        );
    }

    #[test]
    fn a_rubric_that_names_nothing_is_refused_before_a_frame_is_read() {
        let error = parse_result(&["score", "shoot", "--rubric", "  "])
            .expect_err("a blank rubric must stop the command line");
        let message = error.to_string();
        for expected in ["general", "press", "holiday"] {
            assert!(
                message.contains(expected),
                "the refusal must list the built-ins: {message}"
            );
        }
    }

    #[test]
    fn score_takes_the_dedupe_flag() {
        let Command::Score(args) = parse(&["score", "shoot"]) else {
            panic!("not a score command");
        };
        assert!(!args.no_dedupe);
        assert_eq!(
            args.overrides().dedupe,
            None,
            "a run that says nothing about dedupe lets a config file decide"
        );

        let Command::Score(args) = parse(&["score", "shoot", "--no-dedupe"]) else {
            panic!("not a score command");
        };
        assert!(args.no_dedupe);
        assert_eq!(
            args.overrides().dedupe,
            Some(false),
            "--no-dedupe can only turn dedupe off"
        );
    }

    #[test]
    fn score_takes_the_sidecar_flags() {
        let Command::Score(args) = parse(&[
            "score",
            "shoot",
            "--force",
            "--allow-foreign",
            "--print-sidecars",
        ]) else {
            panic!("not a score command");
        };
        assert!(args.force, "--force is how a re-score gets to happen");
        assert!(
            args.allow_foreign,
            "--allow-foreign is the darktable override"
        );
        assert!(args.print_sidecars, "--print-sidecars writes nothing");

        let Command::Score(args) = parse(&["score", "shoot"]) else {
            panic!("not a score command");
        };
        assert!(!args.force && !args.allow_foreign && !args.print_sidecars);
    }

    #[test]
    fn a_preview_is_not_also_quiet_or_verbose() {
        for other in ["--quiet", "--verbose"] {
            assert!(
                parse_result(&["score", "shoot", "--print-sidecars", other]).is_err(),
                "--print-sidecars says its own thing about every frame"
            );
        }
    }

    #[test]
    fn score_takes_the_quiet_and_verbose_flags_but_not_both() {
        let Command::Score(args) = parse(&["score", "shoot", "--quiet"]) else {
            panic!("not a score command");
        };
        assert!(args.quiet && !args.verbose);

        let Command::Score(args) = parse(&["score", "shoot", "--verbose"]) else {
            panic!("not a score command");
        };
        assert!(args.verbose && !args.quiet);

        assert!(
            parse_result(&["score", "shoot", "--quiet", "--verbose"]).is_err(),
            "one run cannot be both silent and voluble"
        );
    }

    #[test]
    fn every_command_takes_a_config_file_and_can_refuse_one() {
        for command in ["score shoot", "groups shoot", "doctor", "models"] {
            let arguments: Vec<&str> = command
                .split(' ')
                .chain(["--config", "mine.toml"])
                .collect();
            let parsed = parse(&arguments);
            let common = match &parsed {
                Command::Score(args) => &args.common,
                Command::Doctor(args) => &args.common,
                Command::Models(args) => args,
                Command::Groups(args) => &args.common,
            };
            assert_eq!(common.config, Some(PathBuf::from("mine.toml")), "{command}");

            let arguments: Vec<&str> = command.split(' ').chain(["--no-config"]).collect();
            let parsed = parse(&arguments);
            let common = match &parsed {
                Command::Score(args) => &args.common,
                Command::Doctor(args) => &args.common,
                Command::Models(args) => args,
                Command::Groups(args) => &args.common,
            };
            assert!(common.no_config, "{command}");
        }
    }

    #[test]
    fn naming_a_config_file_and_refusing_them_all_are_opposite_answers() {
        assert!(
            parse_result(&["score", "shoot", "--config", "mine.toml", "--no-config"]).is_err(),
            "one run cannot read a file and ignore every file"
        );
    }

    #[test]
    fn doctor_takes_an_optional_file_and_the_server_flags() {
        let Command::Doctor(args) = parse(&["doctor"]) else {
            panic!("not a doctor command");
        };
        assert_eq!(args.file, None);
        assert_eq!(args.common.endpoint, None);
        let effective = args.overrides().settle(None);
        assert_eq!(effective.endpoint, omlx::DEFAULT_ENDPOINT);
        assert_eq!(args.common.alias(&effective), omlx::DEFAULT_MODEL);
        assert_eq!(effective.params.max_edge, Params::DEFAULT_MAX_EDGE);

        let Command::Doctor(args) = parse(&[
            "doctor",
            "DSC_0001.NEF",
            "--endpoint",
            "http://127.0.0.1:9000",
            "--model",
            "clef-27b",
        ]) else {
            panic!("not a doctor command");
        };
        assert_eq!(args.file, Some(PathBuf::from("DSC_0001.NEF")));
        let effective = args.overrides().settle(None);
        assert_eq!(effective.endpoint, "http://127.0.0.1:9000");
        assert_eq!(args.common.alias(&effective), "clef-27b");
    }

    #[test]
    fn doctor_takes_the_same_render_flags_as_a_run() {
        let Command::Doctor(args) = parse(&["doctor", "DSC_0001.NEF", "--max-edge", "1024"]) else {
            panic!("not a doctor command");
        };
        let effective = args.overrides().settle(None);
        assert_eq!(effective.params.max_edge, 1024);
        assert_eq!(effective.params.jpeg_quality, Params::DEFAULT_JPEG_QUALITY);
    }

    #[test]
    fn score_needs_a_directory() {
        assert!(parse_result(&["score"]).is_err());
    }

    #[test]
    fn groups_needs_a_directory_and_defaults_to_the_core_thresholds() {
        assert!(parse_result(&["groups"]).is_err());

        let Command::Groups(args) = parse(&["groups", "shoot"]) else {
            panic!("not a groups command");
        };
        assert_eq!(args.dir, PathBuf::from("shoot"));
        assert_eq!(args.out, PathBuf::from(DEFAULT_GROUPS_CSV));
        assert_eq!(args.recursive, None);
        assert_eq!(args.limit, None);
        assert_eq!(args.group_gap, None);
        assert_eq!(args.group_similarity, None);
        assert_eq!(args.group_exposure_stops, None);
        assert_eq!(args.max_edge, None);
        assert_eq!(args.jpeg_quality, None);
        assert_eq!(args.jobs, None);
        assert_eq!(args.cull_jobs, None);

        let options = groups_options(&args);
        assert_eq!(options.thresholds, Thresholds::default());
        assert_eq!(options.jobs, None, "unasked means one job per core");
        assert!(!options.scan.recursive);
        assert_eq!(options.scan.limit, None);

        let effective = args.overrides().settle(None);
        assert_eq!(
            effective.group_params,
            Params {
                max_edge: imagecull_core::keeper::EDGE,
                jpeg_quality: Params::DEFAULT_JPEG_QUALITY,
            },
            "a comparison defaults to the measured 512 px, not a rating's 1568"
        );
    }

    #[test]
    fn groups_takes_the_comparisons_preview_flags() {
        let Command::Groups(args) = parse(&[
            "groups",
            "shoot",
            "--max-edge",
            "640",
            "--jpeg-quality",
            "60",
        ]) else {
            panic!("not a groups command");
        };
        assert_eq!(args.max_edge, Some(640));
        assert_eq!(args.jpeg_quality, Some(60));

        let effective = args.overrides().settle(None);
        assert_eq!(
            effective.group_params,
            Params {
                max_edge: 640,
                jpeg_quality: 60,
            }
        );
        assert_eq!(
            effective.params,
            Params::default(),
            "a cull's --max-edge is not a rating's"
        );

        for arguments in [
            &["groups", "shoot", "--max-edge", "0"][..],
            &["groups", "shoot", "--jpeg-quality", "0"][..],
            &["groups", "shoot", "--jpeg-quality", "101"][..],
        ] {
            assert!(
                parse_result(arguments).is_err(),
                "{arguments:?} should be refused"
            );
        }
    }

    #[test]
    fn groups_takes_its_comparison_pair_from_the_config_file() {
        let file = config::ConfigFile::from_toml("[group]\nmax_edge = 700\njpeg_quality = 55\n")
            .expect("a valid config");
        let Command::Groups(args) = parse(&["groups", "shoot"]) else {
            panic!("not a groups command");
        };

        let effective = file
            .resolve(None)
            .override_with(args.overrides())
            .settle(None);
        assert_eq!(
            effective.group_params,
            Params {
                max_edge: 700,
                jpeg_quality: 55,
            },
            "[group] max_edge and jpeg_quality reach the cull"
        );
        assert_eq!(
            effective.params,
            Params::default(),
            "and [preview] stays score's"
        );
    }

    #[test]
    fn groups_takes_its_own_thresholds_and_scan_flags() {
        let Command::Groups(args) = parse(&[
            "groups",
            "shoot",
            "--out",
            "march-groups.csv",
            "--recursive",
            "--limit",
            "40",
            "--group-gap",
            "0.75",
            "--group-similarity",
            "0.96",
            "--group-exposure-stops",
            "1.5",
            "--jobs",
            "4",
            "--cull-jobs",
            "2",
        ]) else {
            panic!("not a groups command");
        };

        let options = groups_options(&args);
        assert_eq!(
            options.thresholds.gap,
            std::time::Duration::from_secs_f64(0.75)
        );
        assert_eq!(options.thresholds.similarity, 0.96);
        assert_eq!(options.thresholds.exposure_stops, 1.5);
        assert!(options.scan.recursive);
        assert_eq!(options.scan.limit, Some(40));
        assert_eq!(options.jobs, Some(4), "the degree is the flag's");
        assert_eq!(args.cull_jobs, Some(2), "the cull has its own degree");
        assert_eq!(args.out, PathBuf::from("march-groups.csv"));
    }

    #[test]
    fn groups_takes_its_grouping_keys_from_the_config_file() {
        let file = config::ConfigFile::from_toml(
            "[server]\nmodel = \"clef-27b-oQ4e\"\n\
             [scan]\nrecursive = true\n\
             [group]\ngap = 0.5\nsimilarity = 0.95\nexposure_stops = 1.0\n",
        )
        .expect("a valid config");
        let Command::Groups(args) = parse(&["groups", "shoot"]) else {
            panic!("not a groups command");
        };

        let effective = file
            .resolve(None)
            .override_with(args.overrides())
            .settle(None);
        let options = args.options(&effective);

        assert_eq!(options.thresholds.gap, Duration::from_secs_f64(0.5));
        assert_eq!(options.thresholds.similarity, 0.95);
        assert_eq!(options.thresholds.exposure_stops, 1.0);
        assert!(options.scan.recursive, "[scan] recursive reaches the scan");
        assert_eq!(
            effective.model.as_deref(),
            Some("clef-27b-oQ4e"),
            "and the alias is the one the cull will ask"
        );
    }

    #[test]
    fn a_groups_flag_beats_the_file_it_overrides() {
        let file = config::ConfigFile::from_toml(
            "[scan]\nrecursive = true\n[group]\ngap = 5.0\nsimilarity = 0.99\n",
        )
        .expect("a valid config");
        let Command::Groups(args) = parse(&[
            "groups",
            "shoot",
            "--group-gap",
            "1.5",
            "--recursive",
            "false",
        ]) else {
            panic!("not a groups command");
        };

        let effective = file
            .resolve(None)
            .override_with(args.overrides())
            .settle(None);
        let options = args.options(&effective);

        assert_eq!(options.thresholds.gap, Duration::from_secs_f64(1.5));
        assert_eq!(
            options.thresholds.similarity, 0.99,
            "a key the flags left alone keeps the file's value"
        );
        assert!(
            !options.scan.recursive,
            "--recursive false beats a file that turned it on"
        );
    }

    #[test]
    fn a_gap_and_a_similarity_outside_their_range_are_refused() {
        for arguments in [
            &["groups", "shoot", "--group-gap", "0"][..],
            &["groups", "shoot", "--group-gap", "-1"][..],
            &["groups", "shoot", "--group-similarity", "1.5"][..],
            &["groups", "shoot", "--group-similarity", "-0.1"][..],
            &["groups", "shoot", "--group-exposure-stops", "-1"][..],
            &["groups", "shoot", "--group-exposure-stops", "nan"][..],
            &["groups", "shoot", "--limit", "0"][..],
            &["groups", "shoot", "--jobs", "0"][..],
            &["groups", "shoot", "--cull-jobs", "0"][..],
        ] {
            assert!(
                parse_result(arguments).is_err(),
                "{arguments:?} should be refused"
            );
        }
    }

    #[test]
    fn groups_can_be_asked_for_the_summary_alone() {
        let Command::Groups(args) = parse(&["groups", "shoot", "--quiet"]) else {
            panic!("not a groups command");
        };
        assert!(args.quiet);
        assert!(
            !groups_options(&args).scan.recursive,
            "--quiet silences, it does not scan"
        );
    }

    #[test]
    fn groups_takes_the_server_and_the_cull_flags() {
        let Command::Groups(args) = parse(&[
            "groups",
            "shoot",
            "--endpoint",
            "http://127.0.0.1:9000",
            "--model",
            "clef-27b",
            "--allow-foreign",
            "--print-sidecars",
        ]) else {
            panic!("not a groups command");
        };
        assert_eq!(
            args.common.endpoint.as_deref(),
            Some("http://127.0.0.1:9000")
        );
        assert_eq!(args.common.model.as_deref(), Some("clef-27b"));
        assert!(args.allow_foreign);
        assert!(args.print_sidecars);
        assert!(!args.no_cull);

        let Command::Groups(args) = parse(&["groups", "shoot", "--no-cull", "--verbose"]) else {
            panic!("not a groups command");
        };
        assert!(args.no_cull);
        assert!(args.verbose);
        assert!(
            parse_result(&["groups", "shoot", "--no-cull", "--print-sidecars"]).is_err(),
            "a pass that asks nothing cannot preview what an answer would write"
        );
        assert!(
            parse_result(&["groups", "shoot", "--no-cull", "--cull-jobs", "2"]).is_err(),
            "a pass that does not cull has no cull degree"
        );
        assert!(
            parse_result(&["groups", "shoot", "--print-sidecars", "--quiet"]).is_err(),
            "a preview run cannot also be silent about the blocks it prints"
        );
    }

    #[test]
    fn the_version_line_names_the_version_and_the_default_built_in() {
        let line = version_line();
        assert!(line.starts_with(&format!("imagecull {}", env!("CARGO_PKG_VERSION"))));
        assert!(
            line.contains("rubric general/"),
            "the default built-in is named, not just hashed: {line}"
        );
        assert!(line.contains(omlx::DEFAULT_MODEL), "{line}");

        let short = Builtin::General
            .rubric()
            .expect("parse")
            .hash(omlx::DEFAULT_MODEL)
            .short()
            .to_string();
        assert!(line.contains(&short), "{line} should carry {short}");
    }
}
