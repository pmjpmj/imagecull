//! `imagecull groups`: a shoot folder, grouped by moment and measured from its
//! own pixels.
//!
//! No server, no rubric, no `Decider`: this is the technical pass on its own,
//! and it is here first so the measures can be judged against a photographer's
//! eye before anything is allowed to depend on them. Rendering is all this
//! module does — the grouping and the numbers are `imagecull_core::group`.
//!
//! A pass is silent for as long as it takes to read a shoot: a second a frame
//! on one core, half a minute on four hundred frames, with nothing on screen to
//! say whether it is working. So each frame gets a line as it is read, the same
//! way `score` gives each frame a line as it is judged — in the pass's own
//! order, whatever order the jobs finished in, so a redirected log of a
//! threaded run is the log a serial run would have written.

use std::path::Path;

use imagecull_core::ExitCode;
use imagecull_core::group::{self, Plan};

use crate::cli::GroupsArgs;
use crate::progress::{Stream, duration_text, left_out, plural, write};

/// Run the command and return the process exit code.
pub fn run(args: &GroupsArgs) -> u8 {
    let console = Console { quiet: args.quiet };
    match group::run_with(&args.dir, &args.options(), &console) {
        Ok(plan) => {
            let csv = group::render_csv(&plan);
            if let Err(error) = std::fs::write(&args.out, csv.as_bytes()) {
                eprintln!("error: cannot write {}: {error}", args.out.display());
                return ExitCode::Fatal.code();
            }
            // Through the same write as the progress lines: a reader that
            // stopped at `head` is not a failed run.
            write(Stream::Out, &report(&plan, &args.out, args.quiet));
            exit_code(&plan)
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::Fatal.code()
        }
    }
}

/// The reporter a pass writes through: it renders and writes, and decides
/// nothing. It renders on the pass's own thread, so the lines arrive in the
/// pass's order and no lock is needed between the readers.
struct Console {
    quiet: bool,
}

impl Console {
    /// What one event looks like, or nothing at all under `--quiet`.
    fn text(&self, event: &group::Event) -> Option<String> {
        if self.quiet {
            return None;
        }
        Some(match event {
            group::Event::Started(started) => started_text(started),
            group::Event::Frame(frame) => frame_text(frame),
        })
    }
}

impl group::Watcher for Console {
    fn event(&self, event: group::Event) {
        if let Some(text) = self.text(&event) {
            write(Stream::Out, &text);
        }
    }
}

/// The line a pass opens with: how much work it faces, and how many frames at a
/// time. Said before the first frame, so a slow shoot is not a blank terminal.
fn started_text(started: &group::Started) -> String {
    let mut line = format!(
        "reading {} {}",
        started.total,
        plural(started.total, "frame", "frames")
    );
    if started.jobs > 1 {
        line.push_str(&format!(" · {} at a time", started.jobs));
    }
    line.push('\n');
    line
}

/// One frame read: where it sits in the pass, the name its row will carry, and
/// its status — the whole of what the row says in one line, with the reason
/// after it when the frame could not be measured.
fn frame_text(frame: &group::FrameEvent) -> String {
    let width = frame.total.to_string().len();
    let mut line = format!(
        "{:>width$}/{}  {}  {}",
        frame.index, frame.total, frame.filename, frame.status
    );
    if let Some(failure) = &frame.failure
        && !failure.message.is_empty()
    {
        line.push_str(&format!(": {}", failure.message));
    }
    line.push('\n');
    line
}

/// What a run leaves on stdout when it is over: every group, then the summary —
/// or, with `--quiet`, the summary alone, which is what a script wants to read.
fn report(plan: &Plan, written: &Path, quiet: bool) -> String {
    if quiet {
        summary(plan, written)
    } else {
        render(plan, written)
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
/// The span rather than every name: a photographer reading a 34-frame group
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
    if plan.jobs > 1 {
        parts.push(format!("{0} frames at a time", plan.jobs));
    }
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
    use imagecull_core::sidecar::{self, Failure};

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
            jobs: 1,
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
    fn a_pass_that_read_frames_together_says_how_many() {
        let mut plan = plan(vec![measured("DSC_0001.NEF", 1, 1, 1)], 1);
        plan.jobs = 8;

        let rendered = render(&plan, Path::new("groups.csv"));

        assert!(
            rendered.contains("8 frames at a time"),
            "the per-frame figure is wall clock, so the degree belongs beside it: {rendered}"
        );
    }

    fn frame_event(
        index: usize,
        total: usize,
        name: &str,
        status: &'static str,
    ) -> group::FrameEvent {
        group::FrameEvent {
            index,
            total,
            filename: name.to_string(),
            status,
            failure: None,
        }
    }

    #[test]
    fn the_pass_opens_by_saying_how_much_work_and_how_many_at_a_time() {
        assert_eq!(
            started_text(&group::Started { total: 40, jobs: 8 }),
            "reading 40 frames \u{00b7} 8 at a time\n"
        );
        assert_eq!(
            started_text(&group::Started { total: 1, jobs: 1 }),
            "reading 1 frame\n",
            "one frame, one job: nothing to add"
        );
    }

    #[test]
    fn a_frame_line_is_its_place_its_name_and_its_status() {
        let lined = frame_text(&frame_event(7, 40, "DSC_0007.NEF", sidecar::OK));
        assert_eq!(lined, " 7/40  DSC_0007.NEF  ok\n");

        let failed = group::FrameEvent {
            failure: Some(Failure::of(&Error::new(
                ErrorKind::NoPreview,
                "DSC_0008.NEF embeds no JPEG preview",
            ))),
            ..frame_event(8, 40, "DSC_0008.NEF", ErrorKind::NoPreview.as_str())
        };
        assert_eq!(
            frame_text(&failed),
            " 8/40  DSC_0008.NEF  no_preview: DSC_0008.NEF embeds no JPEG preview\n",
            "the reason the frame has no measures is on the line that says so"
        );
    }

    #[test]
    fn quiet_says_the_summary_and_nothing_else() {
        let plan = plan(
            vec![
                measured("DSC_0001.NEF", 1, 1, 1),
                measured("DSC_0002.NEF", 1, 2, 1),
            ],
            1,
        );

        let quiet = report(&plan, Path::new("groups.csv"), true);

        assert!(quiet.starts_with("groups: 1 group in 2 frames"), "{quiet}");
        assert!(
            !quiet.contains("group   1"),
            "a quiet run does not name a group: {quiet}"
        );
        assert_eq!(quiet.lines().count(), 1);
    }

    #[test]
    fn the_console_is_silent_under_quiet_and_draws_both_events_otherwise() {
        let started = group::Event::Started(group::Started { total: 40, jobs: 8 });
        let frame = group::Event::Frame(frame_event(1, 40, "DSC_0001.NEF", sidecar::OK));

        assert!(Console { quiet: true }.text(&started).is_none());
        assert!(Console { quiet: true }.text(&frame).is_none());
        assert!(Console { quiet: false }.text(&started).is_some());
        assert_eq!(
            Console { quiet: false }.text(&frame).as_deref(),
            Some(" 1/40  DSC_0001.NEF  ok\n")
        );
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
