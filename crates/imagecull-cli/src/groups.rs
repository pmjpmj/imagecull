//! `imagecull groups`: a shoot folder, grouped by moment, measured from its own
//! pixels, and culled by clef.
//!
//! Two passes, in order. The technical pass is local CPU and needs nothing: every
//! frame is grouped with the moment it was taken in and measured from its own
//! pixels, and each frame gets a line as it is read. The cull asks one
//! comparative question per group of two or more — which frame to keep — and
//! marks every other frame of that group red. `--no-cull` stops after the first
//! pass, so the whole technical pass still runs on a machine with no model
//! running.
//!
//! Rendering is all this module does — the grouping and the numbers are
//! `imagecull_core::group`, and the verdict is `imagecull_core::cull`.

use std::path::Path;

use imagecull_core::cull::{self, Culled};
use imagecull_core::group::{self, Plan};
use imagecull_core::keeper;
use imagecull_core::omlx;
use imagecull_core::scoring::RetryPolicy;
use imagecull_core::sidecar;
use imagecull_core::systemone::Adapter;
use imagecull_core::{ExitCode, cull::Verdict};

use crate::cli::GroupsArgs;
use crate::printer::Printer;
use crate::progress::{Stream, duration_text, left_out, plural, write};

/// Run the command and return the process exit code.
pub fn run(args: &GroupsArgs) -> u8 {
    let effective = match args.common.effective(args.overrides()) {
        Ok(effective) => effective,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::Fatal.code();
        }
    };
    let console = Console {
        quiet: args.quiet,
        verbose: args.verbose,
    };
    let plan = match group::run_with(&args.dir, &args.options(), &console) {
        Ok(plan) => plan,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::Fatal.code();
        }
    };
    let csv = group::render_csv(&plan);
    if let Err(error) = std::fs::write(&args.out, csv.as_bytes()) {
        eprintln!("error: cannot write {}: {error}", args.out.display());
        return ExitCode::Fatal.code();
    }
    // The cull is the pass that talks to a server, and the one a machine with no
    // model running can leave out.
    let culled = if args.no_cull {
        None
    } else {
        match cull_for(args, &effective, &plan, &console) {
            Ok(culled) => Some(culled),
            Err(message) => {
                eprintln!("error: {message}");
                return ExitCode::Fatal.code();
            }
        }
    };
    // Through the same write as the progress lines: a reader that stopped at
    // `head` is not a failed run.
    write(
        Stream::Out,
        &report(&plan, culled.as_ref(), &args.out, args.quiet),
    );
    exit_code(&plan, culled.as_ref())
}

/// Run the cull over a plan: the server the effective config names, the model
/// alias it resolved, and the writer the preview flag chose.
fn cull_for(
    args: &GroupsArgs,
    effective: &imagecull_core::config::Effective,
    plan: &Plan,
    console: &Console,
) -> Result<Culled, String> {
    let alias = effective
        .model
        .clone()
        .unwrap_or_else(|| omlx::DEFAULT_MODEL.to_string());
    let client = args.common.client(effective)?;
    let budget = keeper::Budget::default();
    let config = cull::Config {
        model: &alias,
        shoot_context: effective.shoot_context.clone(),
        budget,
        // The comparison renders its own previews at the budget's edge, and
        // encodes them at the quality this run would send.
        jpeg_quality: effective.params.jpeg_quality,
        grouping_hash: &plan.hash,
        allow_foreign: args.allow_foreign,
        retry: RetryPolicy::default(),
    };
    // A preview scores and prints; a run writes. The port is the whole
    // difference between the two, which is why there is no second pipeline.
    let writer: &dyn sidecar::Writer = if args.print_sidecars {
        &Printer
    } else {
        &sidecar::Filesystem
    };
    cull::run(
        plan,
        &config,
        &Adapter::new(client, &alias),
        writer,
        console,
    )
    .map_err(|error| error.to_string())
}

/// The reporter a pass writes through: it renders and writes, and decides
/// nothing. It renders on the pass's own thread, so the lines arrive in the
/// pass's order and no lock is needed between the readers.
struct Console {
    quiet: bool,
    verbose: bool,
}

impl Console {
    /// What one reading event looks like, or nothing at all under `--quiet`.
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

impl cull::Watcher for Console {
    /// The cull's own lines: how much work the pass faces, which group found
    /// which keeper, and — always, quiet or not — every verdict that could not
    /// reach its sidecar and every group nothing could decide. A missing mark is
    /// a hole in the delete set, and silence about it reads as a complete one.
    fn event(&self, event: &cull::Event) {
        match event {
            cull::Event::Started { groups } => {
                if !self.quiet {
                    write(
                        Stream::Out,
                        &format!("culling {groups} {}\n", plural(*groups, "group", "groups")),
                    );
                }
            }
            cull::Event::Group(verdict) => {
                if let Some(failure) = &verdict.failure {
                    write(
                        Stream::Err,
                        &format!(
                            "warning: no keeper for the group of {} starting at {}: {}: {} — \
                             nothing was marked\n",
                            verdict.size,
                            verdict.start,
                            failure.kind.as_str(),
                            failure.message
                        ),
                    );
                } else if self.verbose && !self.quiet {
                    write(Stream::Out, &verdict_text(verdict));
                }
            }
            cull::Event::Unwritten { filename, message } => {
                write(
                    Stream::Err,
                    &format!(
                        "warning: the verdict on {filename} could not be written: {message}\n"
                    ),
                );
            }
        }
    }
}

/// One group's verdict as a progress line: where the pass is, the group's first
/// frame, and what the comparison decided.
fn verdict_text(verdict: &Verdict) -> String {
    let keeper = verdict.keeper.as_deref().unwrap_or("-");
    let mut line = format!(
        "  group {:>3}  {:>3} {}  keeper {keeper}",
        verdict.number,
        verdict.size,
        plural(verdict.size, "frame", "frames"),
    );
    if verdict.red > 0 {
        line.push_str(&format!("  {} red", verdict.red));
    }
    if verdict.unmarked > 0 {
        line.push_str(&format!("  {} unmarked", verdict.unmarked));
    }
    line.push_str(&format!("  {:.2}", verdict.confidence));
    line.push('\n');
    line
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
fn report(plan: &Plan, culled: Option<&Culled>, written: &Path, quiet: bool) -> String {
    if quiet {
        summary(plan, culled, written)
    } else {
        render(plan, culled, written)
    }
}

/// The exit classification: every frame measured and every group decided, or
/// some left behind.
///
/// A group nothing could decide and a verdict that could not be written are
/// failed rows, not a clean run: the delete set is missing frames and the exit
/// code has to say so.
pub fn exit_code(plan: &Plan, culled: Option<&Culled>) -> u8 {
    let failed = plan.failures().count() > 0
        || culled.is_some_and(|culled| culled.undecided + culled.unwritten > 0);
    if failed {
        ExitCode::FailedRows.code()
    } else {
        ExitCode::Ok.code()
    }
}

/// The report: one line per group, then the summary. Pure, so it can be asserted
/// on without a terminal.
pub fn render(plan: &Plan, culled: Option<&Culled>, written: &Path) -> String {
    let mut out = String::new();
    for row in plan.grouped() {
        let verdict = culled.and_then(|culled| {
            culled
                .verdicts
                .iter()
                .find(|verdict| verdict.number == row.number)
        });
        out.push_str(&group_line(&row, verdict));
        out.push('\n');
    }
    if !plan.frames.is_empty() {
        out.push('\n');
    }
    out.push_str(&summary(plan, culled, written));
    out
}

/// One group: its number, its size, the frames it spans, and — when the group
/// was judged — who kept it and how many frames wear the red.
///
/// The span rather than every name: a photographer reading a 34-frame group
/// wants to know where it starts and ends, and the CSV has the rest.
fn group_line(row: &group::GroupRow<'_>, verdict: Option<&Verdict>) -> String {
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
    if let Some(verdict) = verdict {
        match (&verdict.keeper, &verdict.failure) {
            (Some(keeper), _) => {
                line.push_str(&format!("  keeper {keeper}"));
                if verdict.red > 0 {
                    line.push_str(&format!(" · {} red", verdict.red));
                }
                if verdict.unmarked > 0 {
                    line.push_str(&format!(" · {} unmarked", verdict.unmarked));
                }
            }
            (None, Some(failure)) => {
                line.push_str(&format!("  undecided: {}", failure.kind.as_str()))
            }
            (None, None) => {}
        }
    }
    line
}

/// The one-line summary: how the shoot grouped, what both passes cost, what the
/// cull decided, the hash a later run would be compared against, and where the
/// CSV went.
fn summary(plan: &Plan, culled: Option<&Culled>, written: &Path) -> String {
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
    if let Some(culled) = culled {
        parts.push(format!(
            "{} {} judged",
            culled.groups,
            plural(culled.groups, "group", "groups")
        ));
        if culled.non_keepers > 0 {
            parts.push(format!(
                "{} {}",
                culled.non_keepers,
                plural(culled.non_keepers, "non-keeper", "non-keepers")
            ));
        }
        if culled.red > 0 {
            parts.push(format!("{} red", culled.red));
        }
        if culled.unmarked > 0 {
            parts.push(format!("{} unmarked (label not ours)", culled.unmarked));
        }
        if culled.retired > 0 {
            parts.push(format!("{} red retired", culled.retired));
        }
        if culled.stale_reds > 0 {
            parts.push(format!(
                "{} red now the keeper (not ours)",
                culled.stale_reds
            ));
        }
        if culled.undecided > 0 {
            parts.push(format!("{} undecided", culled.undecided));
        }
        if culled.unwritten > 0 {
            parts.push(format!("{} marks failed", culled.unwritten));
        }
        if culled.low_confidence > 0 {
            parts.push(format!(
                "{} low confidence {}",
                culled.low_confidence,
                plural(culled.low_confidence, "group", "groups")
            ));
        }
        if culled.rounds > 0 {
            parts.push(format!(
                "{} comparative {}",
                culled.rounds,
                plural(culled.rounds, "request", "requests")
            ));
        }
    }
    let failed = plan.failures().count();
    if failed > 0 {
        parts.push(format!("{failed} without measures"));
    }
    parts.push(format!(
        "{} measured, {per_frame:.0} ms/frame",
        duration_text(plan.elapsed)
    ));
    if plan.jobs > 1 {
        parts.push(format!("{} frames at a time", plan.jobs));
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

    use imagecull_core::cull::Verdict;
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
            captured: None,
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
            captured: None,
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

    fn verdict(number: usize, start: &str, size: usize, keeper: &str, red: usize) -> Verdict {
        Verdict {
            number,
            start: start.to_string(),
            size,
            keeper: Some(keeper.to_string()),
            red,
            unmarked: 0,
            confidence: 0.83,
            rounds: 1,
            failure: None,
        }
    }

    fn culled(verdicts: Vec<Verdict>) -> Culled {
        let red = verdicts.iter().map(|verdict| verdict.red).sum();
        let groups = verdicts.len();
        Culled {
            groups,
            non_keepers: red,
            red,
            rounds: groups,
            undecided: verdicts
                .iter()
                .filter(|verdict| verdict.failure.is_some())
                .count(),
            verdicts,
            ..Culled::default()
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

        let rendered = render(&plan, None, Path::new("groups.csv"));

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
    fn a_group_line_names_the_keeper_and_the_reds() {
        let plan = plan(
            vec![
                measured("DSC_0001.NEF", 1, 1, 2),
                measured("DSC_0002.NEF", 1, 2, 2),
                measured("DSC_0003.NEF", 2, 1, 2),
                measured("DSC_0004.NEF", 2, 2, 2),
            ],
            2,
        );
        let culled = culled(vec![
            verdict(1, "DSC_0001.NEF", 2, "DSC_0002.NEF", 1),
            Verdict {
                keeper: None,
                failure: Some(Failure::of(&Error::new(ErrorKind::Timeout, "timed out"))),
                ..verdict(2, "DSC_0003.NEF", 2, "-", 0)
            },
        ]);

        let rendered = render(&plan, Some(&culled), Path::new("groups.csv"));

        assert!(
            rendered.contains(
                "group   1    2 frames  DSC_0001.NEF … DSC_0002.NEF  keeper DSC_0002.NEF · 1 red\n"
            ),
            "{rendered}"
        );
        assert!(
            rendered.contains(
                "group   2    2 frames  DSC_0003.NEF … DSC_0004.NEF  undecided: \
                               timeout\n"
            ),
            "a group nothing could decide says so where its keeper would be: {rendered}"
        );
    }

    #[test]
    fn the_summary_names_the_cost_the_cull_the_hash_and_the_file() {
        let plan = plan(
            vec![
                measured("DSC_0001.NEF", 1, 1, 1),
                measured("DSC_0002.NEF", 1, 2, 1),
            ],
            1,
        );
        let culled = culled(vec![verdict(1, "DSC_0001.NEF", 2, "DSC_0001.NEF", 1)]);

        let rendered = render(&plan, Some(&culled), Path::new("out/groups.csv"));

        assert!(
            rendered.contains(
                "groups: 1 group in 2 frames (mean 2.0, largest 2) · 1 group judged · \
                 1 non-keeper · 1 red · 1 comparative request · 2s measured, 1000 ms/frame · \
                 grouping hash 01234567 · wrote out/groups.csv\n"
            ),
            "{rendered}"
        );
        assert_eq!(exit_code(&plan, Some(&culled)), ExitCode::Ok.code());
    }

    #[test]
    fn a_group_nothing_could_decide_fails_the_run() {
        let plan = plan(
            vec![
                measured("DSC_0001.NEF", 1, 1, 2),
                measured("DSC_0002.NEF", 1, 2, 2),
            ],
            1,
        );
        let culled = culled(vec![Verdict {
            keeper: None,
            failure: Some(Failure::of(&Error::new(ErrorKind::Timeout, "timed out"))),
            ..verdict(1, "DSC_0001.NEF", 2, "-", 0)
        }]);

        let rendered = summary(&plan, Some(&culled), Path::new("groups.csv"));

        assert!(rendered.contains("1 undecided"), "{rendered}");
        assert_eq!(exit_code(&plan, Some(&culled)), ExitCode::FailedRows.code());
    }

    #[test]
    fn a_pass_that_left_the_cull_out_says_nothing_about_one() {
        let plan = plan(vec![measured("DSC_0001.NEF", 1, 1, 1)], 1);

        let rendered = render(&plan, None, Path::new("groups.csv"));

        assert!(!rendered.contains("judged"), "{rendered}");
        assert!(!rendered.contains("non-keeper"), "{rendered}");
        assert!(!rendered.contains("keeper"), "{rendered}");
        assert_eq!(exit_code(&plan, None), ExitCode::Ok.code());
    }

    #[test]
    fn a_pass_that_read_frames_together_says_how_many() {
        let mut plan = plan(vec![measured("DSC_0001.NEF", 1, 1, 1)], 1);
        plan.jobs = 8;

        let rendered = render(&plan, None, Path::new("groups.csv"));

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
        let culled = culled(vec![verdict(1, "DSC_0001.NEF", 2, "DSC_0001.NEF", 1)]);

        let quiet = report(&plan, Some(&culled), Path::new("groups.csv"), true);

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

        let quiet = Console {
            quiet: true,
            verbose: true,
        };
        assert!(quiet.text(&started).is_none());
        assert!(quiet.text(&frame).is_none());
        let console = Console {
            quiet: false,
            verbose: false,
        };
        assert!(console.text(&started).is_some());
        assert_eq!(
            console.text(&frame).as_deref(),
            Some(" 1/40  DSC_0001.NEF  ok\n")
        );
    }

    #[test]
    fn a_verdict_line_names_the_group_the_keeper_and_the_reds() {
        let line = verdict_text(&verdict(3, "DSC_0100.NEF", 12, "DSC_0104.NEF", 11));

        assert_eq!(
            line, "  group   3   12 frames  keeper DSC_0104.NEF  11 red  0.83\n",
            "a cull's progress is per group, so one line says what that group decided"
        );
    }

    #[test]
    fn a_frame_without_measures_is_named_and_fails_the_run() {
        let plan = plan(
            vec![measured("DSC_0001.NEF", 1, 1, 2), failed("DSC_0002.NEF", 1)],
            1,
        );

        let rendered = render(&plan, None, Path::new("groups.csv"));

        assert!(
            rendered.contains("(1 without measures)"),
            "the group says a frame of it is unmeasured: {rendered}"
        );
        assert!(rendered.contains("1 without measures ·"));
        assert_eq!(exit_code(&plan, None), ExitCode::FailedRows.code());
    }

    #[test]
    fn an_empty_folder_still_says_what_it_did() {
        let plan = plan(Vec::new(), 0);

        let rendered = render(&plan, None, Path::new("groups.csv"));

        assert!(
            rendered.starts_with("groups: 0 groups in 0 frames"),
            "{rendered}"
        );
        assert_eq!(exit_code(&plan, None), ExitCode::Ok.code());
    }

    #[test]
    fn the_scan_left_out_reaches_the_summary() {
        let mut plan = plan(vec![measured("DSC_0001.NEF", 1, 1, 1)], 1);
        plan.counts = ScanCounts {
            skipped_extension: 3,
            ignored: 2,
            hidden_directories: 1,
        };

        let rendered = render(&plan, None, Path::new("groups.csv"));

        assert!(rendered.contains("3 skipped (not RAW)"), "{rendered}");
        assert!(rendered.contains("2 ignored (Finder noise)"), "{rendered}");
        assert!(
            rendered.contains("1 hidden directory not scanned"),
            "{rendered}"
        );
    }
}
