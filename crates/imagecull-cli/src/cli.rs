//! The command line itself: what a photographer types.
//!
//! Only arguments live here — no rendering, no decisions. Every flag maps to a
//! core value, so nothing about the pipeline can be reachable only from the
//! terminal.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use imagecull_core::ingest::ScanOptions;
use imagecull_core::ledger::LedgerMode;
use imagecull_core::omlx::{self, Client, Endpoint, Timeouts};
use imagecull_core::preprocess::Params;
use imagecull_core::report::Outputs;
use imagecull_core::rubric::Rubric;

/// Where `--dry-run` writes its previews, relative to the working directory.
pub const DEFAULT_PREVIEWS_DIR: &str = "previews";

/// The CSV a run writes when `--out` is not given (spec §11: `./scores.csv`).
pub const DEFAULT_OUTPUT: &str = "scores.csv";

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
    Models(ServerArgs),
    /// Check that the local oMLX can score photos, and that a file's preview
    /// survives the pipeline
    Doctor(DoctorArgs),
    /// Score a shoot folder
    Score(ScoreArgs),
}

/// The options the commands that talk to a server share.
#[derive(Args, Debug)]
pub struct ServerArgs {
    /// Base URL of the oMLX server
    #[arg(long, value_name = "URL", default_value = omlx::DEFAULT_ENDPOINT)]
    pub endpoint: String,

    /// Model alias to check or score with (score defaults to the rubric's alias,
    /// doctor and models to clef-flash-oQ4e)
    #[arg(short = 'm', long, value_name = "ALIAS")]
    pub model: Option<String>,

    /// Read timeout in seconds
    #[arg(long, value_name = "SECS", default_value_t = 120)]
    pub timeout: u64,
}

impl ServerArgs {
    /// The alias to use when nothing more specific applies: the configured one,
    /// or the reference server's default. `score` prefers the rubric's alias.
    pub fn model(&self) -> &str {
        self.model.as_deref().unwrap_or(omlx::DEFAULT_MODEL)
    }

    /// Build the client these options describe, or the message explaining why
    /// they describe nothing.
    pub fn client(&self) -> Result<Client, String> {
        let endpoint = Endpoint::parse(&self.endpoint).map_err(|error| error.to_string())?;
        Ok(Client::new(
            endpoint,
            Timeouts::with_read_seconds(self.timeout),
        ))
    }
}

#[derive(Args, Debug)]
pub struct DoctorArgs {
    #[command(flatten)]
    pub server: ServerArgs,

    /// A RAW file to run the preview pipeline on, as a self-test
    #[arg(value_name = "FILE")]
    pub file: Option<PathBuf>,

    /// Cap on the longest edge of the sent preview, in pixels
    #[arg(
        long,
        value_name = "PX",
        default_value_t = Params::DEFAULT_MAX_EDGE,
        value_parser = clap::value_parser!(u32).range(1..)
    )]
    pub max_edge: u32,

    /// JPEG quality of the sent preview, 1-100
    #[arg(
        long,
        value_name = "QUALITY",
        default_value_t = Params::DEFAULT_JPEG_QUALITY,
        value_parser = clap::value_parser!(u8).range(1..=100)
    )]
    pub jpeg_quality: u8,
}

impl DoctorArgs {
    /// How the preview should be rendered — the same knobs a run would use, so
    /// the self-test proves the pipeline the photographer is about to use.
    pub fn params(&self) -> Params {
        Params {
            max_edge: self.max_edge,
            jpeg_quality: self.jpeg_quality,
        }
    }
}

#[derive(Args, Debug)]
pub struct ScoreArgs {
    /// The shoot folder to scan
    #[arg(value_name = "DIR")]
    pub dir: PathBuf,

    #[command(flatten)]
    pub server: ServerArgs,

    /// A rubric file to use instead of the one built into imagecull
    #[arg(long, value_name = "FILE")]
    pub rubric: Option<PathBuf>,

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

    /// Cap on the longest edge of the sent preview, in pixels
    #[arg(
        long,
        value_name = "PX",
        default_value_t = Params::DEFAULT_MAX_EDGE,
        value_parser = clap::value_parser!(u32).range(1..)
    )]
    pub max_edge: u32,

    /// JPEG quality of the sent preview, 1-100
    #[arg(
        long,
        value_name = "QUALITY",
        default_value_t = Params::DEFAULT_JPEG_QUALITY,
        value_parser = clap::value_parser!(u8).range(1..=100)
    )]
    pub jpeg_quality: u8,

    /// Also scan subdirectories; hidden directories are never scanned
    #[arg(long)]
    pub recursive: bool,

    /// Score only the first N files, as a smoke test
    #[arg(long, value_name = "N", value_parser = positive_count)]
    pub limit: Option<usize>,

    /// Where to write the CSV of scores
    #[arg(long, value_name = "FILE", default_value = DEFAULT_OUTPUT)]
    pub out: PathBuf,

    /// Also write every raw model answer to this JSON file, so a surprising
    /// score loses nothing
    #[arg(long, value_name = "FILE")]
    pub json_scores: Option<PathBuf>,

    /// Write absolute paths in the CSV instead of paths inside the shoot folder
    #[arg(long)]
    pub absolute: bool,

    /// Pick up an interrupted run: frames the ledger beside the CSV already
    /// holds are not scored again
    #[arg(long, conflicts_with = "fresh")]
    pub resume: bool,

    /// Throw the ledger beside the CSV away and score every frame again
    #[arg(long)]
    pub fresh: bool,

    /// Score every frame independently instead of reusing the outcome of an
    /// identical preview
    #[arg(long)]
    pub no_dedupe: bool,

    /// Suppress the per-frame progress line; the outcome and a one-line summary
    /// remain, so a script can read them
    #[arg(long, conflicts_with = "verbose")]
    pub quiet: bool,

    /// One line per frame with the bytes that were sent, the answers at full
    /// precision and the status of every frame
    #[arg(long)]
    pub verbose: bool,
}

/// The `--version` line: the version, and the rubric compiled into the binary.
///
/// The hash is what makes the line worth printing: two builds with different
/// default rubrics are not the same tool, and a photographer comparing CSVs
/// needs the identity of the rubric that made them (spec §8.3, §11).
pub fn version_line() -> String {
    match Rubric::embedded() {
        Ok(rubric) => format!(
            "imagecull {} (rubric {}, model {})",
            env!("CARGO_PKG_VERSION"),
            rubric.hash(rubric.model()).short(),
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

impl ScoreArgs {
    /// How the folder should be scanned.
    pub fn scan_options(&self) -> ScanOptions {
        ScanOptions {
            recursive: self.recursive,
            limit: self.limit,
        }
    }

    /// How the preview should be rendered.
    pub fn params(&self) -> Params {
        Params {
            max_edge: self.max_edge,
            jpeg_quality: self.jpeg_quality,
        }
    }

    /// Where the run writes its report.
    pub fn outputs(&self) -> Outputs {
        Outputs {
            csv: self.out.clone(),
            json_scores: self.json_scores.clone(),
            absolute: self.absolute,
        }
    }

    /// What the run does with a ledger that is already beside its CSV.
    ///
    /// Neither flag is a deliberate choice too: a plain `score` is a new run, so
    /// it starts the ledger over — after the same rubric-and-model check
    /// `--resume` makes, because two rubrics silently sharing a ledger is the
    /// failure that check exists to prevent (spec §28, §33).
    pub fn ledger_mode(&self) -> LedgerMode {
        if self.fresh {
            LedgerMode::Discard
        } else if self.resume {
            LedgerMode::Resume
        } else {
            LedgerMode::Restart
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> Command {
        parse_result(arguments).expect("valid arguments").command
    }

    fn parse_result(arguments: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("imagecull").chain(arguments.iter().copied()))
    }

    #[test]
    fn score_defaults_are_the_specs_defaults() {
        let Command::Score(args) = parse(&["score", "shoot"]) else {
            panic!("not a score command");
        };
        assert_eq!(args.dir, PathBuf::from("shoot"));
        assert!(!args.dry_run);
        assert_eq!(args.server.endpoint, omlx::DEFAULT_ENDPOINT);
        assert_eq!(
            args.server.model, None,
            "--model defaults to the rubric's alias"
        );
        assert_eq!(args.server.timeout, 120);
        assert_eq!(args.rubric, None);
        assert_eq!(args.shoot_context, None);
        assert_eq!(args.previews_dir, PathBuf::from(DEFAULT_PREVIEWS_DIR));
        assert_eq!(args.max_edge, Params::DEFAULT_MAX_EDGE);
        assert_eq!(args.jpeg_quality, Params::DEFAULT_JPEG_QUALITY);
        assert!(!args.recursive);
        assert_eq!(args.limit, None);
        assert!(!args.scan_options().recursive);
        assert_eq!(args.out, PathBuf::from(DEFAULT_OUTPUT));
        assert_eq!(args.json_scores, None);
        assert!(!args.absolute);
        assert_eq!(args.outputs().csv, PathBuf::from(DEFAULT_OUTPUT));
        assert_eq!(args.outputs().json_scores, None);
        assert!(!args.outputs().absolute);
        assert!(
            !args.quiet && !args.verbose,
            "a plain run shows the progress line and the summary"
        );
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
        assert!(args.recursive);
        assert_eq!(args.limit, Some(20));
        assert_eq!(args.params().max_edge, 1024);
        assert_eq!(args.params().jpeg_quality, 70);
        assert_eq!(args.previews_dir, PathBuf::from("/tmp/out"));
    }

    #[test]
    fn score_takes_the_report_flags() {
        let Command::Score(args) = parse(&[
            "score",
            "shoot",
            "--out",
            "scores/march.csv",
            "--json-scores",
            "scores/march.json",
            "--absolute",
        ]) else {
            panic!("not a score command");
        };
        let outputs = args.outputs();
        assert_eq!(outputs.csv, PathBuf::from("scores/march.csv"));
        assert_eq!(
            outputs.json_scores,
            Some(PathBuf::from("scores/march.json"))
        );
        assert!(outputs.absolute);
    }

    #[test]
    fn nonsense_edges_and_qualities_are_rejected() {
        let cases: [&[&str]; 4] = [
            &["score", "shoot", "--max-edge", "0"],
            &["score", "shoot", "--jpeg-quality", "0"],
            &["score", "shoot", "--jpeg-quality", "101"],
            &["score", "shoot", "--limit", "0"],
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
        assert_eq!(args.server.endpoint, "http://127.0.0.1:9000");
        assert_eq!(args.server.model.as_deref(), Some("clef-27b-oQ4e"));
        assert_eq!(args.rubric, Some(PathBuf::from("mine.toml")));
        assert_eq!(
            args.shoot_context.as_deref(),
            Some("street at dusk, high ISO expected")
        );
        assert_eq!(args.server.timeout, 240);
    }

    /// The three flags that decide what a run does with a ledger it finds
    /// beside its CSV: neither is a new run, and the two conflict.
    #[test]
    fn score_takes_the_ledger_and_dedupe_flags() {
        let Command::Score(args) = parse(&["score", "shoot"]) else {
            panic!("not a score command");
        };
        assert!(!args.resume && !args.fresh && !args.no_dedupe);
        assert_eq!(
            args.ledger_mode(),
            LedgerMode::Restart,
            "a plain score starts the ledger over, after the mismatch check"
        );

        let Command::Score(args) = parse(&["score", "shoot", "--resume"]) else {
            panic!("not a score command");
        };
        assert_eq!(args.ledger_mode(), LedgerMode::Resume);

        let Command::Score(args) = parse(&["score", "shoot", "--fresh"]) else {
            panic!("not a score command");
        };
        assert_eq!(args.ledger_mode(), LedgerMode::Discard);

        let Command::Score(args) = parse(&["score", "shoot", "--no-dedupe"]) else {
            panic!("not a score command");
        };
        assert!(args.no_dedupe);
    }

    #[test]
    fn resuming_and_starting_fresh_are_opposite_answers() {
        assert!(parse_result(&["score", "shoot", "--resume", "--fresh"]).is_err());
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
    fn doctor_takes_an_optional_file_and_the_server_flags() {
        let Command::Doctor(args) = parse(&["doctor"]) else {
            panic!("not a doctor command");
        };
        assert_eq!(args.file, None);
        assert_eq!(args.server.endpoint, omlx::DEFAULT_ENDPOINT);
        assert_eq!(args.server.model(), omlx::DEFAULT_MODEL);

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
        assert_eq!(args.server.endpoint, "http://127.0.0.1:9000");
        assert_eq!(args.server.model(), "clef-27b");
        assert_eq!(args.params().max_edge, Params::DEFAULT_MAX_EDGE);
    }

    #[test]
    fn doctor_takes_the_same_render_flags_as_a_run() {
        let Command::Doctor(args) = parse(&["doctor", "DSC_0001.NEF", "--max-edge", "1024"]) else {
            panic!("not a doctor command");
        };
        assert_eq!(args.params().max_edge, 1024);
        assert_eq!(args.params().jpeg_quality, Params::DEFAULT_JPEG_QUALITY);
    }

    #[test]
    fn score_needs_a_directory() {
        assert!(parse_result(&["score"]).is_err());
    }

    #[test]
    fn the_version_line_names_the_version_and_the_embedded_rubric() {
        let line = version_line();
        assert!(line.starts_with(&format!("imagecull {}", env!("CARGO_PKG_VERSION"))));
        assert!(line.contains("rubric "), "{line}");
        assert!(line.contains(omlx::DEFAULT_MODEL), "{line}");

        let short = Rubric::embedded()
            .expect("parse")
            .hash(omlx::DEFAULT_MODEL)
            .short()
            .to_string();
        assert!(line.contains(&short), "{line} should carry {short}");
    }
}
