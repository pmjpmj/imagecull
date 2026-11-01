//! Scoring core for imagecull.
//!
//! This crate holds everything the eventual macOS GUI needs, and nothing a
//! terminal owns: no stdout, no TTY checks, no `process::exit` (the lints below
//! keep it that way). The CLI in `imagecull-cli` is a thin shell over it.
//!
//! Landed so far: the failure taxonomy ([`error`]), server version comparison
//! ([`version`]), a typed oMLX client ([`omlx`]) and the `doctor` diagnosis
//! ([`doctor`]) from M0; the shoot-folder scan ([`ingest`]), embedded preview
//! extraction ([`preview`]) and its orientation/downscale/encode path
//! ([`preprocess`]), and the `--dry-run` export that puts them together
//! ([`dry_run`]) from M1; and the rubric ([`rubric`]), the `/v1/systemone`
//! client ([`systemone`]) and the one-frame scoring entry point ([`scoring`])
//! from M2.
#![forbid(unsafe_code)]
#![deny(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::dbg_macro,
    clippy::exit
)]

pub mod doctor;
pub mod dry_run;
pub mod error;
pub mod ingest;
pub mod omlx;
pub mod preprocess;
pub mod preview;
pub mod reading;
pub mod rubric;
pub mod scoring;
pub mod systemone;
pub mod version;

pub use error::{Error, ErrorKind, ExitCode, Recovery, Result};
pub use ingest::{Scan, ScanCounts, ScanOptions, SourceFile};
pub use omlx::{Client, Endpoint, ModelKind, Timeouts};
pub use reading::{Answers, Confidences, Reading, Verdict};
pub use rubric::{Question, QuestionKind, Rubric, RubricHash};
pub use scoring::{Config, Decider, Request, Response, Run, State, Usage};
