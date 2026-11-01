//! `imagecull` — the command-line front end.
//!
//! Everything here is presentation: argument parsing, rendering, exit codes.
//! The decisions themselves live in `imagecull_core`, which never prints and
//! never exits, so the same core can drive the GUI later.

use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use imagecull_core::omlx::{self, Client, Endpoint, Timeouts};

mod doctor;
mod models;

/// Exit code 2: nothing could be done (spec §10).
const EXIT_FATAL: u8 = 2;

#[derive(Parser, Debug)]
#[command(
    name = "imagecull",
    version,
    about = "Score a folder of Nikon mirrorless RAW files with a local Clef decision model",
    subcommand_required = true,
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// List what the local oMLX serves, flagging decision models
    Models(ServerArgs),
    /// Check that the local oMLX can score photos
    Doctor(ServerArgs),
}

/// The options both M0 commands share.
#[derive(Args, Debug)]
struct ServerArgs {
    /// Base URL of the oMLX server
    #[arg(long, value_name = "URL", default_value = omlx::DEFAULT_ENDPOINT)]
    endpoint: String,

    /// Model alias to check or score with
    #[arg(short = 'm', long, value_name = "ALIAS", default_value = omlx::DEFAULT_MODEL)]
    model: String,

    /// Read timeout in seconds
    #[arg(long, value_name = "SECS", default_value_t = 120)]
    timeout: u64,
}

impl ServerArgs {
    /// Build the client these options describe, or the message explaining why
    /// they describe nothing.
    fn client(&self) -> Result<Client, String> {
        let endpoint = Endpoint::parse(&self.endpoint).map_err(|error| error.to_string())?;
        Ok(Client::new(
            endpoint,
            Timeouts::with_read_seconds(self.timeout),
        ))
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let code = match cli.command {
        Command::Models(args) => models::run(&args),
        Command::Doctor(args) => doctor::run(&args),
    };
    ExitCode::from(code)
}
