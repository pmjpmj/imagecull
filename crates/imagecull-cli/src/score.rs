//! `imagecull score`: a shoot folder, scored by the local decision model.
//!
//! Without `--dry-run` this is the real run: the folder is scanned, every
//! accepted frame goes to `/v1/systemone` one at a time, and what the run is
//! doing reaches the terminal through the core's reporter — this module hands it
//! [`Terminal`], which renders the events, and reads the exit code off the run.
//! With `--dry-run` the same per-frame step runs but the bytes are written into
//! a previews directory and no request is made, so that command keeps its own
//! small report. Rendering is all here; the decisions are in `imagecull_core`.

use std::sync::Arc;

use imagecull_core::config::Effective;
use imagecull_core::rubric::Rubric;
use imagecull_core::scoring::{self, Cancel};
use imagecull_core::systemone::Adapter;
use imagecull_core::{ExitCode, dry_run};

use crate::cli::ScoreArgs;
use crate::progress::{Terminal, Verbosity, kilobytes, left_out, plural};

/// Run the command and return the process exit code.
pub fn run(args: &ScoreArgs) -> u8 {
    // The effective config first: a bad config file, or a `--config` that names
    // nothing, must stop the run before a single frame is read.
    let effective = match args.common.effective(args.overrides()) {
        Ok(effective) => effective,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::Fatal.code();
        }
    };

    if args.dry_run {
        return dry_run_command(args, &effective);
    }

    let rubric = match Rubric::load(effective.rubric.as_deref()) {
        Ok(rubric) => rubric,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::Fatal.code();
        }
    };
    let mut config = scoring::Config::new(
        rubric,
        effective.model.clone(),
        effective.params,
        effective.scan(args.limit),
        effective.shoot_context.clone(),
        effective.outputs.clone(),
    );
    // What the run does with the ledger beside its CSV, whether an identical
    // preview may reuse an outcome, which server to record in the header, and
    // which config file these values came from.
    config.ledger = args.ledger_mode();
    config.dedupe = effective.dedupe;
    config.endpoint = Some(effective.endpoint.clone());
    config.source = effective.source.clone();
    let client = match args.common.client(&effective) {
        Ok(client) => client,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::Fatal.code();
        }
    };

    let cancel = Arc::new(Cancel::new());
    install_cancel_handler(&cancel);

    // The run narrates itself; the terminal draws it. The exit code is the
    // run's own classification, so a script can act on the outcome.
    let reporter = Terminal::new(verbosity(args));
    match scoring::run(
        &args.dir,
        &config,
        &Adapter::new(client, &config.model),
        &reporter,
        &cancel,
    ) {
        Ok(run) => run.exit_code().code(),
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::Fatal.code()
        }
    }
}

/// How much of the run the photographer asked to see.
fn verbosity(args: &ScoreArgs) -> Verbosity {
    if args.quiet {
        Verbosity::Quiet
    } else if args.verbose {
        Verbosity::Verbose
    } else {
        Verbosity::Normal
    }
}

/// The CLI's half of cooperative cancellation: Ctrl-C asks the run to stop
/// between files rather than killing the process mid-request, so nothing already
/// scored is lost and the run still writes its CSV and exits `3` (spec §10).
fn install_cancel_handler(cancel: &Arc<Cancel>) {
    let cancel = Arc::clone(cancel);
    if let Err(error) = ctrlc::set_handler(move || cancel.cancel()) {
        // A process that cannot take the signal still runs; Ctrl-C then kills it
        // the usual way, without the tidy exit.
        eprintln!(
            "warning: Ctrl-C cannot be caught here, so it will not stop a run cleanly: {error}"
        );
    }
}

/// `--dry-run`: the bytes a run would send, written out, no HTTP at all. It uses
/// the same effective config, so a dry run previews what the real run would.
fn dry_run_command(args: &ScoreArgs, effective: &Effective) -> u8 {
    match dry_run::render_previews(
        &args.dir,
        &args.previews_dir,
        &effective.scan(args.limit),
        &effective.params,
    ) {
        Ok(export) => {
            print!("{}", render(&export));
            exit_code(&export)
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::Fatal.code()
        }
    }
}

/// The exit classification of a dry run: every frame previewed, or some left
/// behind.
pub fn exit_code(export: &dry_run::Export) -> u8 {
    if export.failures().count() > 0 {
        ExitCode::FailedRows.code()
    } else {
        ExitCode::Ok.code()
    }
}

/// The dry-run report: one line per accepted file, then a summary line. Pure,
/// so it can be asserted on without a terminal.
pub fn render(export: &dry_run::Export) -> String {
    let mut out = String::new();
    for frame in &export.frames {
        match &frame.outcome {
            Ok(written) => out.push_str(&format!(
                "{}  {}x{}  {}  {}\n",
                frame.source.relative.display(),
                written.width,
                written.height,
                kilobytes(written.jpeg_bytes),
                written.preview.display(),
            )),
            Err(error) => {
                out.push_str(&format!("{}  {}\n", frame.source.relative.display(), error))
            }
        }
    }
    out.push('\n');
    out.push_str(&dry_run_summary(export));
    out
}

/// The one-line dry-run summary: what was written, what was left out.
fn dry_run_summary(export: &dry_run::Export) -> String {
    let written = export.written();
    let failed = export.frames.len() - written;
    let mut parts = vec![format!(
        "{written} {} in {}",
        plural(written, "preview", "previews"),
        export.previews.display(),
    )];
    if failed > 0 {
        parts.push(format!("{failed} failed"));
    }
    parts.extend(left_out(&export.counts));
    format!("dry run: {}\n", parts.join(" · "))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use imagecull_core::error::{Error, ErrorKind};
    use imagecull_core::ingest::{ScanCounts, SourceFile};

    use super::*;

    fn source(name: &str) -> SourceFile {
        SourceFile {
            path: PathBuf::from("/shoot").join(name),
            relative: PathBuf::from(name),
        }
    }

    fn export(frames: Vec<dry_run::Frame>) -> dry_run::Export {
        dry_run::Export {
            previews: PathBuf::from("/out"),
            counts: ScanCounts {
                skipped_extension: 3,
                ignored: 2,
                hidden_directories: 1,
            },
            frames,
        }
    }

    fn written(name: &str, width: u32, height: u32, jpeg_bytes: usize) -> dry_run::Frame {
        dry_run::Frame {
            source: source(name),
            outcome: Ok(dry_run::Written {
                preview: PathBuf::from("/out").join(name).with_extension("jpg"),
                width,
                height,
                jpeg_bytes,
            }),
        }
    }

    fn failed(name: &str, kind: ErrorKind) -> dry_run::Frame {
        dry_run::Frame {
            source: source(name),
            outcome: Err(Error::new(kind, format!("{name} embeds no JPEG preview"))),
        }
    }

    #[test]
    fn a_clean_dry_run_reports_every_preview_and_counts_what_it_left_out() {
        let export = export(vec![
            written("DSC_0001.NEF", 1568, 1045, 220_000),
            written("DSC_0002.NEF", 800, 600, 90_000),
        ]);

        let rendered = render(&export);

        assert!(rendered.starts_with("DSC_0001.NEF  1568x1045  215 KB  /out/DSC_0001.jpg\n"));
        assert!(rendered.contains("DSC_0002.NEF  800x600  88 KB  /out/DSC_0002.jpg\n"));
        assert!(rendered.contains(
            "dry run: 2 previews in /out · 3 skipped (not RAW) · 2 ignored (Finder noise) · \
             1 hidden directory not scanned\n"
        ));
        assert_eq!(exit_code(&export), ExitCode::Ok.code());
    }

    #[test]
    fn a_failed_dry_run_frame_is_named_with_its_status() {
        let export = export(vec![
            written("DSC_0001.NEF", 100, 50, 4096),
            failed("DSC_0002.NEF", ErrorKind::NoPreview),
        ]);

        let rendered = render(&export);

        assert!(
            rendered.contains("DSC_0002.NEF  no_preview: DSC_0002.NEF embeds no JPEG preview\n"),
            "{rendered}"
        );
        assert!(rendered.contains("dry run: 1 preview in /out · 1 failed ·"));
        assert_eq!(exit_code(&export), ExitCode::FailedRows.code());
    }
}
