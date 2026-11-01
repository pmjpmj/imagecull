//! `imagecull` — the command-line front end.
//!
//! Everything here is presentation: argument parsing, rendering, exit codes.
//! The decisions themselves live in `imagecull_core`, which never prints and
//! never exits, so the same core can drive the GUI later.

use std::process::ExitCode;

use clap::Parser;

mod cli;
mod doctor;
mod models;
mod score;

use cli::{Cli, Command};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let code = match cli.command {
        Command::Models(args) => models::run(&args),
        Command::Doctor(args) => doctor::run(&args),
        Command::Score(args) => score::run(&args),
    };
    ExitCode::from(code)
}
