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
    // `--version` carries the embedded rubric's hash, which clap cannot know at
    // compile time, so it is answered before the parser runs.
    if std::env::args()
        .skip(1)
        .any(|arg| arg == "--version" || arg == "-V")
    {
        println!("{}", cli::version_line());
        return ExitCode::SUCCESS;
    }

    let cli = Cli::parse();
    let code = match cli.command {
        Command::Models(args) => models::run(&args),
        Command::Doctor(args) => doctor::run(&args),
        Command::Score(args) => score::run(&args),
    };
    ExitCode::from(code)
}
