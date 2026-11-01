//! `imagecull score`: score a shoot folder.
//!
//! Only `--dry-run` is implemented so far: it writes the exact bytes a scoring
//! run would send, and makes no requests. Rendering is all in the core; this
//! module prints and picks an exit code.

use imagecull_core::ExitCode;
use imagecull_core::dry_run;

use crate::cli::ScoreArgs;

/// Run the command and return the process exit code.
pub fn run(args: &ScoreArgs) -> u8 {
    if !args.dry_run {
        eprintln!(
            "error: scoring needs the oMLX wire adapter, which is not in this build — run with \
             --dry-run to render the previews that would be sent"
        );
        return ExitCode::Fatal.code();
    }

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

/// The exit classification: every frame previewed, or some left behind.
pub fn exit_code(export: &dry_run::Export) -> u8 {
    if export.failures().count() > 0 {
        ExitCode::FailedRows.code()
    } else {
        ExitCode::Ok.code()
    }
}

/// The report: one line per accepted file, then a summary line. Pure, so it can
/// be asserted on without a terminal.
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
            Err(error) => out.push_str(&format!(
                "{}  {}\n",
                frame.source.relative.display(),
                error
            )),
        }
    }
    out.push('\n');
    out.push_str(&summary(export));
    out
}

/// The one-line summary: what was written, what was left out, with counts.
fn summary(export: &dry_run::Export) -> String {
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
    if export.counts.skipped_extension > 0 {
        parts.push(format!(
            "{} skipped (not RAW)",
            export.counts.skipped_extension
        ));
    }
    if export.counts.ignored > 0 {
        parts.push(format!(
            "{} ignored (Finder noise)",
            export.counts.ignored
        ));
    }
    if export.counts.hidden_directories > 0 {
        parts.push(format!(
            "{} {} not scanned",
            export.counts.hidden_directories,
            plural(
                export.counts.hidden_directories,
                "hidden directory",
                "hidden directories"
            )
        ));
    }
    format!("dry run: {}\n", parts.join(" · "))
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
    use imagecull_core::ExitCode;

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
    fn a_clean_run_reports_every_preview_and_counts_what_it_left_out() {
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
    fn a_failed_frame_is_named_with_its_status_and_changes_the_exit_code() {
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
