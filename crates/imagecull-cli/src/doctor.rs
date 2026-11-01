//! `imagecull doctor`: one sentence about whether this server can score.
//!
//! The diagnosis itself is in `imagecull_core::doctor`; this module only picks
//! the stream and the exit code.

use imagecull_core::doctor;

use crate::{EXIT_FATAL, ServerArgs};

/// Run the command and return the process exit code.
pub fn run(args: &ServerArgs) -> u8 {
    let client = match args.client() {
        Ok(client) => client,
        Err(message) => {
            eprintln!("error: {message}");
            return EXIT_FATAL;
        }
    };

    let report = doctor::diagnose(&client, &args.model);
    if report.is_ready() {
        println!("ok: {}", report.message());
    } else {
        eprintln!("error: {}", report.message());
    }
    report.exit().code()
}
