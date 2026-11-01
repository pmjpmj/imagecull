//! `imagecull groups`: a shoot folder, grouped by moment and measured from its
//! own pixels.
//!
//! No server, no rubric, no `Decider`: this is the technical pass on its own,
//! and it is here first so the measures can be judged against a photographer's
//! eye before anything is allowed to depend on them. Rendering is all this
//! module does — the grouping and the numbers are `imagecull_core::group`.

use std::path::Path;

use imagecull_core::ExitCode;
use imagecull_core::group::{self, Plan};

use crate::cli::GroupsArgs;
use crate::progress::{duration_text, left_out, plural};

/// Run the command and return the process exit code.
pub fn run(args: &GroupsArgs) -> u8 {
    match group::run(&args.dir, &args.options()) {
        Ok(plan) => {
            let csv = group::render_csv(&plan);
            if let Err(error) = std::fs::write(&args.out, csv.as_bytes()) {
                eprintln!("error: cannot write {}: {error}", args.out.display());
                return ExitCode::Fatal.code();
            }
            print!("{}", render(&plan, &args.out));
            exit_code(&plan)
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::Fatal.code()
        }
    }
}

/// The exit classification: every frame measured, or some left without numbers.
pub fn exit_code(plan: &Plan) -> u8 {
    if plan.failures().count() > 0 {
        ExitCode::FailedRows.code()
    } else {
        ExitCode::Ok.code()
    }
}

/// The report: one line per group, then the summary. Pure, so it can be asserted
/// on without a terminal.
pub fn render(plan: &Plan, written: &Path) -> String {
    let mut out = String::new();
    for row in plan.grouped() {
        out.push_str(&group_line(&row));
        out.push('\n');
    }
    if !plan.frames.is_empty() {
        out.push('\n');
    }
    out.push_str(&summary(plan, written));
    out
}

/// One group: its number, its size, and the frames it spans.
///
/// The span rather than every name: a photographer reading a 34-frame burst
/// wants to know where it starts and ends, and the CSV has the rest.
fn group_line(row: &group::GroupRow<'_>) -> String {
    let size = row.frames.len();
    let first = row.frames[0].source.relative.display();
    let mut line = format!(
        "group {:>3}  {:>3} {}  ",
        row.number,
        size,
        plural(size, "frame", "frames")
    );
    match row.frames.last() {
        Some(last) if size > 1 => {
            line.push_str(&format!("{first} … {}", last.source.relative.display()))
        }
        _ => line.push_str(&first.to_string()),
    }
    let without = row
        .frames
        .iter()
        .filter(|frame| frame.failure.is_some())
        .count();
    if without > 0 {
        line.push_str(&format!("  ({without} without measures)"));
    }
    line
}

/// The one-line summary: how the shoot grouped, what the pass cost, the hash a
/// later run would be compared against, and where the CSV went.
fn summary(plan: &Plan, written: &Path) -> String {
    let frames = plan.frames.len();
    let largest = plan
        .grouped()
        .iter()
        .map(|row| row.frames.len())
        .max()
        .unwrap_or(0);
    #[allow(clippy::cast_precision_loss)]
    let mean = if plan.groups > 0 {
        frames as f64 / plan.groups as f64
    } else {
        0.0
    };
    #[allow(clippy::cast_precision_loss)]
    let per_frame = if frames > 0 {
        plan.elapsed.as_secs_f64() * 1000.0 / frames as f64
    } else {
        0.0
    };

    let mut parts = vec![format!(
        "{} {} in {frames} {} (mean {mean:.1}, largest {largest})",
        plan.groups,
        plural(plan.groups, "group", "groups"),
        plural(frames, "frame", "frames"),
    )];
    let failed = plan.failures().count();
    if failed > 0 {
        parts.push(format!("{failed} without measures"));
    }
    parts.push(format!(
        "{} measured, {per_frame:.0} ms/frame",
        duration_text(plan.elapsed)
    ));
    parts.push(format!("grouping hash {}", short_hash(&plan.hash)));
    parts.push(format!("wrote {}", written.display()));
    parts.extend(left_out(&plan.counts));
    format!("groups: {}\n", parts.join(" · "))
}

/// The first eight characters of the grouping hash, as a report names a rubric
/// by its short hash.
fn short_hash(hash: &str) -> &str {
    hash.get(..8).unwrap_or(hash)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use imagecull_core::error::{Error, ErrorKind};
    use imagecull_core::group::Planned;
    use imagecull_core::ingest::{ScanCounts, SourceFile};
    use imagecull_core::measure::Measures;
    use imagecull_core::sidecar::Failure;

    use super::*;

    fn source(name: &str) -> SourceFile {
        SourceFile {
            path: PathBuf::from("/shoot").join(name),
            relative: PathBuf::from(name),
        }
    }

    fn measured(name: &str, number: usize, index: usize, size: usize) -> Planned {
        Planned {
            source: source(name),
            group: number,
            group_index: index,
            group_size: size,
            measures: Measures {
                acutance: Some(3.5),
                ..Measures::default()
            },
            failure: None,
        }
    }

    fn failed(name: &str, number: usize) -> Planned {
        Planned {
            source: source(name),
            group: number,
            group_index: 1,
            group_size: 1,
            measures: Measures::default(),
            failure: Some(Failure::of(&Error::new(
                ErrorKind::NoPreview,
                format!("{name} embeds no JPEG preview"),
            ))),
        }
    }

    fn plan(frames: Vec<Planned>, groups: usize) -> Plan {
        Plan {
            frames,
            groups,
            hash: "0123456789abcdef".to_string(),
            counts: ScanCounts::default(),
            elapsed: Duration::from_secs(2),
        }
    }

    #[test]
    fn a_group_is_one_line_spanning_its_first_and_last_frame() {
        let plan = plan(
            vec![
                measured("DSC_0001.NEF", 1, 1, 2),
                measured("DSC_0002.NEF", 1, 2, 2),
                measured("DSC_0003.NEF", 2, 1, 1),
            ],
            2,
        );

        let rendered = render(&plan, Path::new("groups.csv"));

        assert!(
            rendered.starts_with("group   1    2 frames  DSC_0001.NEF … DSC_0002.NEF\n"),
            "{rendered}"
        );
        assert!(
            rendered.contains("group   2    1 frame  DSC_0003.NEF\n"),
            "a group of one names its frame once: {rendered}"
        );
    }

    #[test]
    fn the_summary_names_the_cost_the_hash_and_the_file() {
        let plan = plan(
            vec![
                measured("DSC_0001.NEF", 1, 1, 1),
                measured("DSC_0002.NEF", 1, 2, 1),
            ],
            1,
        );

        let rendered = render(&plan, Path::new("out/groups.csv"));

        assert!(rendered.contains(
            "groups: 1 group in 2 frames (mean 2.0, largest 2) · 2s measured, 1000 ms/frame · \
             grouping hash 01234567 · wrote out/groups.csv\n"
        ));
        assert_eq!(exit_code(&plan), ExitCode::Ok.code());
    }

    #[test]
    fn a_frame_without_measures_is_named_and_fails_the_run() {
        let plan = plan(
            vec![measured("DSC_0001.NEF", 1, 1, 2), failed("DSC_0002.NEF", 1)],
            1,
        );

        let rendered = render(&plan, Path::new("groups.csv"));

        assert!(
            rendered.contains("(1 without measures)"),
            "the group says a frame of it is unmeasured: {rendered}"
        );
        assert!(rendered.contains("1 without measures ·"));
        assert_eq!(exit_code(&plan), ExitCode::FailedRows.code());
    }

    #[test]
    fn an_empty_folder_still_says_what_it_did() {
        let plan = plan(Vec::new(), 0);

        let rendered = render(&plan, Path::new("groups.csv"));

        assert!(
            rendered.starts_with("groups: 0 groups in 0 frames"),
            "{rendered}"
        );
        assert_eq!(exit_code(&plan), ExitCode::Ok.code());
    }

    #[test]
    fn the_scan_left_out_reaches_the_summary() {
        let mut plan = plan(vec![measured("DSC_0001.NEF", 1, 1, 1)], 1);
        plan.counts = ScanCounts {
            skipped_extension: 3,
            ignored: 2,
            hidden_directories: 1,
        };

        let rendered = render(&plan, Path::new("groups.csv"));

        assert!(rendered.contains("3 skipped (not RAW)"), "{rendered}");
        assert!(rendered.contains("2 ignored (Finder noise)"), "{rendered}");
        assert!(
            rendered.contains("1 hidden directory not scanned"),
            "{rendered}"
        );
    }
}
