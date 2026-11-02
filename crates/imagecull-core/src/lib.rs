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
//! from M2, with the CSV deliverable ([`report`]) on top of them. M3 adds the
//! rules that keep a long run honest: the taxonomy's retries and its aborting
//! kinds ([`error`]), the preflight that runs before the first frame, the
//! cancellation flag between files ([`scoring::Cancel`]) and the run's own exit
//! classification ([`scoring::Ending`]). The ledger beside the CSV ([`ledger`])
//! is what lets a run be interrupted: each frame is recorded and flushed as it
//! is judged, `--resume` skips what is already there, and the CSV is rendered
//! from those records. Ticket 07 adds the run's own account of itself
//! ([`progress`]): the events a front end draws a frame line and a summary
//! from, still with no terminal in the core. The config file ([`config`]) is the
//! last source of values a run reads: what stays true across runs, resolved into
//! the same effective config the flags produce. `imagecull groups` (ticket 01 of
//! the burst-grouping slice) adds the model-free technical pass: the grouping
//! ([`group`]) that puts a shoot's frames into the moments they were taken in,
//! and the [`measure`]s taken from their own pixels — acutance, shake, roll,
//! clipping and the camera's focus report. Neither consults a model, and neither
//! changes a score. The pass reports as it runs through the same kind of seam as
//! [`progress`] ([`group::Watcher`]), so a front end can draw its frames without
//! the core knowing what a screen is. Ticket 02 of the same slice added the
//! group's keeper ([`keeper`]); ticket 03 gave the cull its own pass ([`cull`]):
//! `imagecull groups` asks the comparative question itself, one request per
//! round, and marks every non-keeper red. `score` rates frames and nothing else.
//! The Cloudflare slice makes the wire configurable: `[server]` gains a request
//! path, a body selector and a token source, and the token is a [`Secret`] —
//! resolved from the variable that source names ([`config::Effective`]'s
//! `resolve_token`), carried and compared, never printed.
#![forbid(unsafe_code)]
#![deny(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::dbg_macro,
    clippy::exit
)]

pub mod config;
pub(crate) mod csv;
pub mod cull;
pub mod doctor;
pub mod dry_run;
pub mod error;
pub mod group;
pub mod ingest;
pub mod keeper;
pub mod measure;
pub mod omlx;
pub mod preprocess;
pub mod preview;
pub mod progress;
pub mod reading;
pub mod rubric;
pub mod scoring;
pub mod secret;
pub mod sidecar;
pub mod systemone;
pub mod time;
pub mod version;

pub use config::{ConfigFile, Effective, Env, Loaded, Resolved};
pub use cull::{Culled, Verdict};
pub use error::{Error, ErrorKind, ExitCode, Recovery, Result};
pub use group::{
    Capture, Facts, Grouping, Options as GroupOptions, Plan, Planned, Settings, Signature,
    Thresholds,
};
pub use ingest::{Scan, ScanCounts, ScanOptions, SourceFile};
pub use keeper::{Budget, Choice, Comparison};
pub use omlx::{Client, Endpoint, ModelKind, Timeouts};
pub use progress::{
    Event, Finished, FrameEvent, Judgement, MeanMedian, PreviewShape, Reporter, Silent, Started,
    StatusCount, Summary, Warning,
};
pub use reading::{Answers, Reading};
pub use rubric::{Builtin, Question, Rubric, RubricHash, RubricSource};
pub use scoring::{
    Cancel, Config, Decider, Ending, Outcome, Persisted, Request, Response, RetryPolicy, Run,
    State, Usage,
};
pub use secret::Secret;
pub use sidecar::{ExistingLabel, Failure, Mark, Outputs, Prior, Properties, Row, Written};
