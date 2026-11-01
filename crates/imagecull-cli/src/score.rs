//! `imagecull score`: a shoot folder, scored by the local decision model.
//!
//! Without `--dry-run` this is the real run: the folder is scanned, every
//! accepted frame goes to `/v1/systemone` one at a time, and each frame's six
//! answers are printed as they arrive. With `--dry-run` the same per-frame step
//! runs but the bytes are written into a previews directory and no request is
//! made. Rendering is all here; the decisions are in `imagecull_core`.

use std::sync::Arc;

use imagecull_core::ingest::ScanCounts;
use imagecull_core::rubric::Rubric;
use imagecull_core::scoring::{self, Cancel, Ending, Run};
use imagecull_core::systemone::Adapter;
use imagecull_core::{ExitCode, dry_run};

use crate::cli::ScoreArgs;

/// Run the command and return the process exit code.
pub fn run(args: &ScoreArgs) -> u8 {
    if args.dry_run {
        return dry_run_command(args);
    }

    let rubric = match Rubric::load(args.rubric.as_deref()) {
        Ok(rubric) => rubric,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::Fatal.code();
        }
    };
    let config = scoring::Config::new(
        rubric,
        args.server.model.clone(),
        args.params(),
        args.scan_options(),
        args.shoot_context.clone(),
        args.outputs(),
    );
    let client = match args.server.client() {
        Ok(client) => client,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::Fatal.code();
        }
    };

    let cancel = Arc::new(Cancel::new());
    install_cancel_handler(&cancel);

    match scoring::run(
        &args.dir,
        &config,
        &Adapter::new(client, &config.model),
        &cancel,
    ) {
        Ok(run) => {
            print!("{}", render_run(&run));
            exit_code_for_run(&run)
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::Fatal.code()
        }
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

/// `--dry-run`: the bytes a run would send, written out, no HTTP at all.
fn dry_run_command(args: &ScoreArgs) -> u8 {
    match dry_run::render_previews(
        &args.dir,
        &args.previews_dir,
        &args.scan_options(),
        &args.params(),
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

/// The process exit code for a real run: the run already knows what it is —
/// all ok, completed with failed rows, stopped, or interrupted.
pub fn exit_code_for_run(run: &Run) -> u8 {
    run.exit_code().code()
}

/// The report of a real run: one line per frame with its six answers, then a
/// summary. Pure, so it can be asserted on without a terminal.
pub fn render_run(run: &Run) -> String {
    let mut out = String::new();
    for frame in &run.frames {
        let name = frame.source.relative.display();
        match &frame.outcome {
            Err(error) => out.push_str(&format!("{name}  {error}\n")),
            Ok(reading) => match &reading.answers {
                Err(error) => out.push_str(&format!("{name}  {error}\n")),
                Ok(answers) => out.push_str(&format!(
                    "{name}  overall {:.2}  technical {:.2}  composition {:.2}  impact {:.2}  \
                     {}★ {}  {}ms\n",
                    answers.overall,
                    answers.technical,
                    answers.composition,
                    answers.impact,
                    answers.stars,
                    answers.verdict.as_str(),
                    reading.usage.latency_ms(),
                )),
            },
        }
    }
    out.push('\n');
    out.push_str(&summary(run));
    out
}

/// The one-line summary: how much of the shoot was scored, under which rubric,
/// and — when it did not finish — what stopped it.
fn summary(run: &Run) -> String {
    let mut parts = vec![format!("{} of {} scored", run.scored(), run.total)];
    if run.failed() > 0 {
        parts.push(format!("{} failed", run.failed()));
    }
    match &run.ending {
        Ending::Aborted { reason } => parts.push(format!("stopped: {reason}")),
        Ending::Interrupted => parts.push("interrupted".to_string()),
        Ending::Completed => {}
    }
    parts.extend(left_out(&run.counts));
    parts.push(format!("rubric {}", run.rubric_hash.short()));
    parts.push(format!("model {}", run.model));
    format!("scored: {}\n", parts.join(" · "))
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

/// What the scan left out, and why, in the words a photographer reads: the same
/// counts in both reports, because a run and a dry run look at the same folder.
fn left_out(counts: &ScanCounts) -> Vec<String> {
    let mut parts = Vec::new();
    if counts.skipped_extension > 0 {
        parts.push(format!("{} skipped (not RAW)", counts.skipped_extension));
    }
    if counts.ignored > 0 {
        parts.push(format!("{} ignored (Finder noise)", counts.ignored));
    }
    if counts.hidden_directories > 0 {
        parts.push(format!(
            "{} {} not scanned",
            counts.hidden_directories,
            plural(
                counts.hidden_directories,
                "hidden directory",
                "hidden directories"
            )
        ));
    }
    parts
}

fn plural<'a>(count: usize, one: &'a str, many: &'a str) -> &'a str {
    if count == 1 { one } else { many }
}

/// `bytes` as a whole number of kilobytes.
fn kilobytes(bytes: usize) -> String {
    format!("{} KB", bytes.div_ceil(1024))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use imagecull_core::error::{Error, ErrorKind};
    use imagecull_core::ingest::{ScanCounts, SourceFile};
    use imagecull_core::reading;
    use imagecull_core::rubric::Rubric;
    use imagecull_core::scoring::{Frame, Response};
    use serde_json::Value;

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

    fn document(document: Value) -> reading::Reading {
        reading::read(
            &Rubric::embedded().expect("parse"),
            Response {
                document,
                client_latency_ms: 812,
            },
        )
    }

    fn reading() -> reading::Reading {
        document(serde_json::json!({
            "answers": {
                "overall": {"score": 6.4237, "confidence": 0.41},
                "technical": {"score": 7.8125, "confidence": 0.55},
                "composition": {"score": 5.3398, "confidence": 0.33},
                "impact": {"score": 4.9601, "confidence": 0.62},
                "stars": {"choice": "4", "confidence": 0.63},
                "verdict": {"choice": "like", "confidence": 0.88}
            },
            "usage": {"input_tokens": 1327}
        }))
    }

    fn run(frames: Vec<Frame>) -> Run {
        Run {
            rubric_hash: Rubric::embedded().expect("parse").hash("clef-flash-oQ4e"),
            model: "clef-flash-oQ4e".to_string(),
            counts: ScanCounts::default(),
            total: frames.len(),
            frames,
            ending: Ending::Completed,
        }
    }

    fn frame(name: &str, outcome: imagecull_core::Result<reading::Reading>) -> Frame {
        Frame {
            source: source(name),
            outcome,
        }
    }

    #[test]
    fn a_scored_run_prints_the_six_answers_per_frame_and_its_identity() {
        let run = run(vec![frame("DSC_0001.NEF", Ok(reading()))]);

        let rendered = render_run(&run);

        assert!(
            rendered.starts_with(
                "DSC_0001.NEF  overall 6.42  technical 7.81  composition 5.34  impact 4.96  \
                 4★ like  812ms\n"
            ),
            "{rendered}"
        );
        let hash = run.rubric_hash.short();
        assert!(
            rendered.contains(&format!(
                "scored: 1 of 1 scored · rubric {hash} · model clef-flash-oQ4e\n"
            )),
            "{rendered}"
        );
        assert_eq!(exit_code_for_run(&run), ExitCode::Ok.code());
    }

    #[test]
    fn a_failed_frame_is_named_with_its_status_and_changes_the_exit_code() {
        let run = run(vec![
            frame("DSC_0001.NEF", Ok(reading())),
            frame(
                "DSC_0002.NEF",
                Err(Error::new(
                    ErrorKind::NoPreview,
                    "DSC_0002.NEF embeds no JPEG preview",
                )),
            ),
            frame(
                "DSC_0003.NEF",
                Ok(document(
                    serde_json::json!({"answers": {"overall": {"score": 6.0}}}),
                )),
            ),
        ]);

        let rendered = render_run(&run);

        assert!(
            rendered.contains("DSC_0002.NEF  no_preview: DSC_0002.NEF embeds no JPEG preview\n"),
            "{rendered}"
        );
        assert!(
            rendered.contains("DSC_0003.NEF  bad_response:"),
            "an unreadable document is a failed row too: {rendered}"
        );
        assert!(
            rendered.contains("scored: 1 of 3 scored · 2 failed ·"),
            "{rendered}"
        );
        assert_eq!(exit_code_for_run(&run), ExitCode::FailedRows.code());
    }

    #[test]
    fn an_interrupted_run_says_so_and_exits_three() {
        let mut run = run(vec![frame("DSC_0001.NEF", Ok(reading()))]);
        run.total = 400;
        run.ending = Ending::Interrupted;

        assert!(
            render_run(&run).contains("scored: 1 of 400 scored · interrupted ·"),
            "an interrupted run does not look like a finished one: {}",
            render_run(&run)
        );
        assert_eq!(exit_code_for_run(&run), ExitCode::Interrupted.code());
    }

    #[test]
    fn a_stopped_run_prints_why_and_exits_two() {
        let mut run = run(vec![frame(
            "DSC_0001.NEF",
            Err(Error::new(
                ErrorKind::Http400,
                "POST /v1/systemone returned HTTP 400: invalid criteria",
            )),
        )]);
        run.ending = Ending::Aborted {
            reason: "http_400: POST /v1/systemone returned HTTP 400: invalid criteria".into(),
        };

        let rendered = render_run(&run);
        assert!(
            rendered.contains(
                "stopped: http_400: POST /v1/systemone returned HTTP 400: invalid criteria"
            ),
            "{rendered}"
        );
        assert_eq!(exit_code_for_run(&run), ExitCode::Fatal.code());
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
