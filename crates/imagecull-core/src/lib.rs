//! Scoring core for imagecull.
//!
//! This crate holds everything the eventual macOS GUI needs, and nothing a
//! terminal owns: no stdout, no TTY checks, no `process::exit` (the lints below
//! keep it that way). The CLI in `imagecull-cli` is a thin shell over it.
//!
//! Landed so far (M0): the failure taxonomy ([`error`]), server version
//! comparison ([`version`]), a typed oMLX client ([`omlx`]), and the `doctor`
//! diagnosis ([`doctor`]).
#![forbid(unsafe_code)]
#![deny(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::dbg_macro,
    clippy::exit
)]

pub mod doctor;
pub mod error;
pub mod omlx;
pub mod version;

pub use error::{Error, ErrorKind, ExitCode, Recovery, Result};
pub use omlx::{Client, Endpoint, ModelKind, Timeouts};
