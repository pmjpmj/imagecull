//! `imagecull doctor`: one sentence about whether this server can score, and,
//! given a file, one about whether that file survives the preview pipeline.
//!
//! The diagnosis itself is in `imagecull_core::doctor`; this module only picks
//! the stream and the exit code.

use imagecull_core::ExitCode;
use imagecull_core::doctor;

use crate::cli::DoctorArgs;

/// Run the command and return the process exit code.
pub fn run(args: &DoctorArgs) -> u8 {
    // An unreadable config file is itself a diagnosis, and the first one to make:
    // `doctor` reading a different config than `score` would certify a pipeline
    // nobody is about to run.
    let effective = match args.common.effective(args.overrides()) {
        Ok(effective) => effective,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::Fatal.code();
        }
    };

    let mut code = match args.common.client(&effective) {
        Ok(client) => {
            let report = doctor::diagnose(&client, args.common.alias(&effective));
            if report.is_ready() {
                println!("ok: {}", report.message());
            } else {
                eprintln!("error: {}", report.message());
            }
            report.exit().code()
        }
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::Fatal.code()
        }
    };

    // The file check is a separate proof and runs either way: a photographer
    // whose server is down still wants to know their camera's output works.
    if let Some(file) = &args.file {
        let check = doctor::check_file(file, &effective.score_params);
        if check.is_ok() {
            println!("ok: {}", check.message());
        } else {
            eprintln!("error: {}", check.message());
            code = ExitCode::Fatal.code();
        }
    }

    code
}
