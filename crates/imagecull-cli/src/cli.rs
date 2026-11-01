//! The command line itself: what a photographer types.
//!
//! Only arguments live here — no rendering, no decisions. Every flag maps to a
//! core value, so nothing about the pipeline can be reachable only from the
//! terminal.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use imagecull_core::ingest::ScanOptions;
use imagecull_core::omlx::{self, Client, Endpoint, Timeouts};
use imagecull_core::preprocess::Params;

/// Where `--dry-run` writes its previews, relative to the working directory.
pub const DEFAULT_PREVIEWS_DIR: &str = "previews";

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

    /// Model alias to check or score with
    #[arg(short = 'm', long, value_name = "ALIAS", default_value = omlx::DEFAULT_MODEL)]
    pub model: String,

    /// Read timeout in seconds
    #[arg(long, value_name = "SECS", default_value_t = 120)]
    pub timeout: u64,
}

impl ServerArgs {
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
}

#[derive(Args, Debug)]
pub struct ScoreArgs {
    /// The shoot folder to scan
    #[arg(value_name = "DIR")]
    pub dir: PathBuf,

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
        assert_eq!(args.previews_dir, PathBuf::from(DEFAULT_PREVIEWS_DIR));
        assert_eq!(args.max_edge, Params::DEFAULT_MAX_EDGE);
        assert_eq!(args.jpeg_quality, Params::DEFAULT_JPEG_QUALITY);
        assert!(!args.recursive);
        assert_eq!(args.limit, None);
        assert!(!args.scan_options().recursive);
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
    fn doctor_takes_an_optional_file_and_the_server_flags() {
        let Command::Doctor(args) = parse(&["doctor"]) else {
            panic!("not a doctor command");
        };
        assert_eq!(args.file, None);
        assert_eq!(args.server.endpoint, omlx::DEFAULT_ENDPOINT);
        assert_eq!(args.server.model, omlx::DEFAULT_MODEL);

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
        assert_eq!(args.server.model, "clef-27b");
    }

    #[test]
    fn score_needs_a_directory() {
        assert!(parse_result(&["score"]).is_err());
    }
}
