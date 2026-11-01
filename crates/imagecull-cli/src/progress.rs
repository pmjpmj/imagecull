//! How a run looks on a terminal, rendered from the core's events.
//!
//! Rendering is a pure function of an [`Event`], a verbosity and whether the
//! output is a terminal, so every presentation decision is assertable without a
//! terminal: a test folds the events of interest through [`render`] and reads
//! the strings. [`Terminal`] is the reporter that does nothing but write them.
//!
//! Three ways to watch the same run: `--quiet` says only what the run did, the
//! default draws a line that overwrites itself and then the summary, and
//! `--verbose` writes a line per frame with the bytes that went out, so a
//! surprising result can be traced without re-running with a debugger.

use std::io::{IsTerminal, Write};

use imagecull_core::ingest::ScanCounts;
use imagecull_core::progress::{
    DISAGREEMENT_RULE, Event, FrameEvent, Judgement, Reporter, Started, Summary, Warning,
};
use imagecull_core::scoring::Ending;

/// How much of a run the photographer wants to see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verbosity {
    /// `--quiet`: the outcome and nothing else, so a script's output stays clean.
    Quiet,
    /// The default: a progress line while it runs, the summary when it ends.
    Normal,
    /// `--verbose`: a line per frame, with what was sent and what came back.
    Verbose,
}

/// Which stream an event's text belongs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    /// The result: progress and the summary.
    Out,
    /// Diagnostics: warnings, and how a stopped run ended.
    Err,
}

/// One event as the terminal sees it: what goes to stdout, and what goes to
/// stderr.
///
/// Two strings rather than one because a single event can say two things — a
/// quiet run's summary is the outcome, and the reason it stopped is a warning on
/// the other stream.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rendering {
    /// Text for stdout, empty when this event says nothing there.
    pub out: String,
    /// Text for stderr, empty when there is nothing to warn about.
    pub err: String,
}

/// Render one event for a terminal.
///
/// `live` is whether stdout is a terminal. The running commentary — the header
/// and the self-overwriting progress line — is the one thing that depends on it,
/// because a redirected stdout must stay clean (spec §11).
pub fn render(event: &Event, verbosity: Verbosity, live: bool) -> Rendering {
    match event {
        Event::Started(started) => Rendering {
            out: started_line(started, verbosity, live).unwrap_or_default(),
            err: String::new(),
        },
        Event::Frame(frame) => Rendering {
            out: frame_text(frame, verbosity, live).unwrap_or_default(),
            err: String::new(),
        },
        Event::Warning(warning) => Rendering {
            out: String::new(),
            err: warning_line(warning),
        },
        Event::Finished(finished) => {
            if verbosity == Verbosity::Quiet {
                Rendering {
                    out: format!("{}\n", outcome_line(&finished.summary)),
                    err: ending_line(&finished.summary.ending).unwrap_or_default(),
                }
            } else {
                Rendering {
                    // The progress line is drawn with a carriage return and no
                    // newline, so it has to be ended before the block starts.
                    out: format!(
                        "{}{}",
                        if live && verbosity == Verbosity::Normal {
                            "\n"
                        } else {
                            ""
                        },
                        summary_block(&finished.summary)
                    ),
                    err: String::new(),
                }
            }
        }
    }
}

/// The reporter a run writes through: it renders and writes, and decides
/// nothing.
pub struct Terminal {
    verbosity: Verbosity,
    live: bool,
}

impl Terminal {
    /// A reporter for a run whose stdout may be a terminal.
    pub fn attached(verbosity: Verbosity) -> Terminal {
        Terminal {
            verbosity,
            live: std::io::stdout().is_terminal(),
        }
    }
}

impl Reporter for Terminal {
    fn event(&self, event: Event) {
        let rendering = render(&event, self.verbosity, self.live);
        write(Stream::Out, &rendering.out);
        write(Stream::Err, &rendering.err);
    }
}

/// Write `text` to `stream` and flush it, so a progress line appears when it is
/// drawn rather than when the buffer fills.
fn write(stream: Stream, text: &str) {
    if text.is_empty() {
        return;
    }
    // A closed pipe is the reader's decision, not an error worth a panic: a
    // script that stopped listening should not turn into a failed run.
    match stream {
        Stream::Out => {
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(text.as_bytes());
            let _ = out.flush();
        }
        Stream::Err => {
            let mut err = std::io::stderr().lock();
            let _ = err.write_all(text.as_bytes());
            let _ = err.flush();
        }
    }
}

/// The line a run opens with: how much work it is about to do, and under what.
fn started_line(started: &Started, verbosity: Verbosity, live: bool) -> Option<String> {
    match verbosity {
        Verbosity::Quiet => None,
        // A redirected stdout carries the result, not the running commentary.
        Verbosity::Normal if !live => None,
        Verbosity::Normal | Verbosity::Verbose => Some(format!(
            "scoring {} {} · rubric {} · model {}\n",
            started.total,
            plural(started.total, "frame", "frames"),
            started.rubric_hash.short(),
            started.model,
        )),
    }
}

/// One frame: the live line, or the verbose account of it.
fn frame_text(frame: &FrameEvent, verbosity: Verbosity, live: bool) -> Option<String> {
    match verbosity {
        Verbosity::Quiet => None,
        Verbosity::Verbose => Some(verbose_line(frame)),
        Verbosity::Normal if live => {
            // `\r` returns to the start of the line and `ESC[K` clears what the
            // previous, longer line left behind.
            Some(format!("\r{}\x1b[K", progress_line(frame)))
        }
        Verbosity::Normal => None,
    }
}

/// The live progress line: index, name, the scores and stars, latency, ETA.
fn progress_line(frame: &FrameEvent) -> String {
    let mut parts = vec![
        format!("{}/{}", frame.index, frame.total),
        frame.filename.clone(),
    ];
    match &frame.judgement {
        Judgement::Answered(answers) => {
            parts.push(format!(
                "{:.2}/{:.2}/{:.2}/{:.2}",
                answers.overall, answers.technical, answers.composition, answers.impact
            ));
            parts.push(format!("{}★ {}", answers.stars, answers.verdict.as_str()));
        }
        Judgement::Failed(failure) => parts.push(failure.kind.as_str().to_string()),
    }
    if let Some(latency_ms) = frame.latency_ms {
        parts.push(format!("{latency_ms}ms"));
    }
    if let Some(eta) = frame.eta {
        parts.push(format!("eta {}", duration_text(eta)));
    }
    parts.join("  ")
}

/// `--verbose`'s per-frame line: what was sent, what it cost, what came back —
/// the answers at the model's own precision, and a failure's whole message.
fn verbose_line(frame: &FrameEvent) -> String {
    let mut parts = vec![frame.filename.clone()];
    match &frame.judgement {
        Judgement::Answered(answers) => {
            parts.push("ok".to_string());
            if let Some(preview) = frame.preview {
                parts.push(format!(
                    "{}x{}  {}",
                    preview.width,
                    preview.height,
                    kilobytes(preview.jpeg_bytes)
                ));
            }
            if let Some(latency_ms) = frame.latency_ms {
                parts.push(format!("{latency_ms}ms"));
            }
            parts.push(format!(
                "overall {}  technical {}  composition {}  impact {}",
                answers.overall, answers.technical, answers.composition, answers.impact
            ));
            parts.push(format!("{}★ {}", answers.stars, answers.verdict.as_str()));
        }
        Judgement::Failed(failure) => {
            parts.push(failure.kind.as_str().to_string());
            if !failure.message.is_empty() {
                parts.push(failure.message.clone());
            }
        }
    }
    if let Some(original) = &frame.duplicate_of {
        parts.push(format!("duplicate of {original}"));
    }
    format!("{}\n", parts.join("  "))
}

/// A warning is a warning in every mode: it is why the run is not what the
/// photographer expected, and `--quiet` is about noise, not about silence
/// (spec §38).
fn warning_line(warning: &Warning) -> String {
    format!("warning: {}\n", warning.message)
}

/// The whole summary block: what the run did, how it ended, and whether to
/// trust the rubric that produced it.
fn summary_block(summary: &Summary) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "scored: {} of {} in {}\n",
        summary.scored,
        summary.total,
        duration_text(summary.elapsed)
    ));
    out.push_str(&format!("status: {}\n", statuses(summary)));
    if let Some(scores) = &summary.scores {
        out.push_str(&format!(
            "scores: overall {:.2} mean / {:.2} median · technical {:.2} / {:.2} · \
             composition {:.2} / {:.2} · impact {:.2} / {:.2}\n",
            scores.overall.mean,
            scores.overall.median,
            scores.technical.mean,
            scores.technical.median,
            scores.composition.mean,
            scores.composition.median,
            scores.impact.mean,
            scores.impact.median,
        ));
        out.push_str(&format!("stars: {}\n", histogram(&summary.stars)));
    }
    out.push_str(&format!(
        "disagreements: {} of {} ({}%) · {} — reported, never reconciled\n",
        summary.disagreements,
        summary.scored,
        percent(summary.disagreement_rate),
        DISAGREEMENT_RULE,
    ));
    if summary.resumed > 0 || summary.duplicates > 0 {
        out.push_str(&format!(
            "resumed: {} · deduplicated: {}\n",
            summary.resumed, summary.duplicates
        ));
    }
    let left_out = left_out(&summary.left_out);
    if !left_out.is_empty() {
        out.push_str(&format!("left out: {}\n", left_out.join(" · ")));
    }
    out.push_str(&format!(
        "rubric {} · model {}",
        summary.rubric_hash.short(),
        summary.model
    ));
    // Which config file produced these numbers, when one did: the same kind of
    // provenance as the rubric hash and the alias beside it (spec §11.1).
    if let Some(config) = &summary.config {
        out.push_str(&format!(" · config {}", config.display()));
    }
    out.push('\n');
    if let Some(ending) = ending_line(&summary.ending) {
        out.push_str(&ending);
    }
    out
}

/// `--quiet`'s single line: the outcome, without the statistics. A script reads
/// this and the exit code; a photographer reads the CSV.
fn outcome_line(summary: &Summary) -> String {
    let mut parts = vec![format!(
        "scored {} of {} in {}",
        summary.scored,
        summary.total,
        duration_text(summary.elapsed)
    )];
    parts.push(statuses(summary));
    if summary.resumed > 0 {
        parts.push(format!("{} resumed", summary.resumed));
    }
    if summary.duplicates > 0 {
        parts.push(format!("{} deduplicated", summary.duplicates));
    }
    let left_out = left_out(&summary.left_out);
    if !left_out.is_empty() {
        parts.push(format!("left out: {}", left_out.join(" · ")));
    }
    parts.push(format!("rubric {}", summary.rubric_hash.short()));
    parts.push(format!("model {}", summary.model));
    parts.join(" · ")
}

/// How a run ended, when it did not run to the end: the one thing a `--quiet`
/// user must not have to guess from an exit code.
fn ending_line(ending: &Ending) -> Option<String> {
    match ending {
        Ending::Completed => None,
        Ending::Aborted { reason } => Some(format!("stopped: {reason}\n")),
        Ending::Interrupted => Some(
            "interrupted: the run was stopped between frames; the rows judged so far are in the \
             CSV, and --resume picks up where it stopped\n"
                .to_string(),
        ),
    }
}

/// The counts by status, as one clause: `ok 397 · no_preview 1`.
fn statuses(summary: &Summary) -> String {
    if summary.counts.is_empty() {
        return "nothing to score".to_string();
    }
    summary
        .counts
        .iter()
        .map(|count| format!("{} {}", count.status, count.count))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The star histogram, as one clause: `1★ 0 · 2★ 4 · … · 5★ 274`.
fn histogram(stars: &[usize; 5]) -> String {
    stars
        .iter()
        .enumerate()
        .map(|(index, count)| format!("{}★ {count}", index + 1))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// A rate as the summary spells it: one decimal place of a percentage.
fn percent(rate: f64) -> String {
    format!("{:.1}", rate * 100.0)
}

/// A duration as a photographer reads it: `42s`, `8m12s`, `1h02m`.
fn duration_text(duration: std::time::Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m{:02}s", seconds % 60);
    }
    format!("{}h{:02}m", minutes / 60, minutes % 60)
}

/// What the scan left out, and why, in the words a photographer reads: the same
/// counts in the summary and in a dry run's report, because both look at the
/// same folder.
pub(crate) fn left_out(counts: &ScanCounts) -> Vec<String> {
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

pub(crate) fn plural<'a>(count: usize, one: &'a str, many: &'a str) -> &'a str {
    if count == 1 { one } else { many }
}

/// `bytes` as a whole number of kilobytes.
pub(crate) fn kilobytes(bytes: usize) -> String {
    format!("{} KB", bytes.div_ceil(1024))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use imagecull_core::error::ErrorKind;
    use imagecull_core::ingest::ScanCounts;
    use imagecull_core::progress::{MeanMedian, PreviewShape, Scores, StatusCount, Warning};
    use imagecull_core::reading::{Answers, Confidences, Verdict};
    use imagecull_core::report::Failure;
    use imagecull_core::rubric::Rubric;
    use imagecull_core::scoring::Ending;

    use super::*;

    fn answers() -> Answers {
        Answers {
            overall: 6.4237,
            technical: 7.8125,
            composition: 5.3398,
            impact: 4.9601,
            stars: 4,
            verdict: Verdict::Like,
            confidence: Confidences {
                overall: 0.41,
                technical: 0.55,
                composition: 0.33,
                impact: 0.62,
                stars: 0.63,
                verdict: 0.88,
            },
        }
    }

    fn started(total: usize) -> Event {
        Event::Started(Started {
            total,
            rubric_hash: Rubric::embedded().expect("parse").hash("clef-flash-oQ4e"),
            model: "clef-flash-oQ4e".to_string(),
        })
    }

    fn answered(index: usize) -> Event {
        Event::Frame(FrameEvent {
            index,
            total: 400,
            filename: "DSC_0012.NEF".to_string(),
            judgement: Judgement::Answered(answers()),
            latency_ms: Some(812),
            eta: Some(Duration::from_secs(461)),
            preview: Some(PreviewShape {
                width: 1568,
                height: 1045,
                jpeg_bytes: 220_000,
            }),
            duplicate_of: None,
        })
    }

    fn failed(index: usize) -> Event {
        Event::Frame(FrameEvent {
            index,
            total: 400,
            filename: "DSC_0013.NEF".to_string(),
            judgement: Judgement::Failed(Failure {
                kind: ErrorKind::Timeout,
                message: "POST /v1/systemone timed out after 120s — 3 attempts".to_string(),
            }),
            latency_ms: None,
            eta: Some(Duration::from_secs(459)),
            preview: None,
            duplicate_of: None,
        })
    }

    /// The summary of a 400-frame run that scored 397 of them.
    fn summary() -> Summary {
        let rubric = Rubric::embedded().expect("parse").hash("clef-flash-oQ4e");
        Summary {
            total: 400,
            judged: 400,
            scored: 397,
            failed: 3,
            resumed: 3,
            duplicates: 2,
            counts: vec![
                StatusCount {
                    status: "ok",
                    count: 397,
                },
                StatusCount {
                    status: "no_preview",
                    count: 1,
                },
                StatusCount {
                    status: "timeout",
                    count: 2,
                },
            ],
            scores: Some(Scores {
                overall: MeanMedian {
                    mean: 6.42,
                    median: 6.51,
                },
                technical: MeanMedian {
                    mean: 7.8,
                    median: 7.9,
                },
                composition: MeanMedian {
                    mean: 5.3,
                    median: 5.4,
                },
                impact: MeanMedian {
                    mean: 4.9,
                    median: 5.0,
                },
            }),
            stars: [0, 4, 51, 68, 274],
            disagreements: 6,
            disagreement_rate: 6.0 / 397.0,
            rubric_hash: rubric,
            model: "clef-flash-oQ4e".to_string(),
            config: None,
            elapsed: Duration::from_secs(8 * 60 + 12),
            left_out: ScanCounts {
                skipped_extension: 3,
                ignored: 2,
                hidden_directories: 1,
            },
            ending: Ending::Completed,
        }
    }

    fn finished(ending: Ending) -> Event {
        let mut summary = summary();
        summary.ending = ending;
        Event::Finished(imagecull_core::progress::Finished { summary })
    }

    /// The whole run as a terminal would have received it.
    fn transcript(events: &[Event], verbosity: Verbosity, live: bool) -> Rendering {
        events.iter().fold(Rendering::default(), |mut seen, event| {
            let one = render(event, verbosity, live);
            seen.out.push_str(&one.out);
            seen.err.push_str(&one.err);
            seen
        })
    }

    #[test]
    fn the_progress_line_shows_index_name_scores_stars_latency_and_eta() {
        let rendered = transcript(&[answered(12)], Verbosity::Normal, true);

        assert_eq!(
            rendered.out,
            "\r12/400  DSC_0012.NEF  6.42/7.81/5.34/4.96  4★ like  812ms  eta 7m41s\x1b[K",
            "one line, overwriting itself"
        );
        assert!(rendered.err.is_empty());
    }

    #[test]
    fn a_failed_frame_keeps_its_place_on_the_progress_line() {
        let rendered = transcript(&[failed(13)], Verbosity::Normal, true);

        assert_eq!(
            rendered.out, "\r13/400  DSC_0013.NEF  timeout  eta 7m39s\x1b[K",
            "the row's status, and no latency for a frame that never answered"
        );
    }

    #[test]
    fn normal_output_is_the_progress_line_on_a_terminal_and_silence_when_redirected() {
        let events = [started(400), answered(12)];

        let live = transcript(&events, Verbosity::Normal, true);
        assert!(
            live.out.starts_with("scoring 400 frames · rubric "),
            "a run says what it is about to do: {}",
            live.out
        );
        assert!(live.out.contains("\r12/400  DSC_0012.NEF"), "{}", live.out);

        let redirected = transcript(&events, Verbosity::Normal, false);
        assert_eq!(
            redirected,
            Rendering::default(),
            "a redirected stdout carries the result, not the running commentary"
        );
    }

    #[test]
    fn the_summary_block_reports_the_counts_the_statistics_and_the_rule() {
        let rendered = transcript(&[finished(Ending::Completed)], Verbosity::Normal, false);
        let short = summary().rubric_hash.short().to_string();

        let expected = format!(
            "scored: 397 of 400 in 8m12s\n\
             status: ok 397 · no_preview 1 · timeout 2\n\
             scores: overall 6.42 mean / 6.51 median · technical 7.80 / 7.90 · \
             composition 5.30 / 5.40 · impact 4.90 / 5.00\n\
             stars: 1★ 0 · 2★ 4 · 3★ 51 · 4★ 68 · 5★ 274\n\
             disagreements: 6 of 397 (1.5%) · {DISAGREEMENT_RULE} — reported, never reconciled\n\
             resumed: 3 · deduplicated: 2\n\
             left out: 3 skipped (not RAW) · 2 ignored (Finder noise) · \
             1 hidden directory not scanned\n\
             rubric {short} · model clef-flash-oQ4e\n"
        );
        assert_eq!(rendered.out, expected);
        assert!(rendered.err.is_empty());
    }

    #[test]
    fn the_summary_names_the_config_file_when_one_was_read() {
        let mut summary = summary();
        summary.config = Some(PathBuf::from("/Volumes/Shoot A/imagecull.toml"));
        let short = summary.rubric_hash.short().to_string();

        let rendered = transcript(
            &[Event::Finished(imagecull_core::progress::Finished {
                summary,
            })],
            Verbosity::Normal,
            false,
        );

        assert!(
            rendered.out.contains(&format!(
                "rubric {short} · model clef-flash-oQ4e · config /Volumes/Shoot A/imagecull.toml\n"
            )),
            "the numbers have to be attributable to the file that made them: {}",
            rendered.out
        );
    }

    #[test]
    fn the_summary_block_starts_on_a_fresh_line_after_the_live_progress_line() {
        let rendered = transcript(
            &[started(400), answered(12), finished(Ending::Completed)],
            Verbosity::Normal,
            true,
        );

        assert!(
            rendered.out.contains("\x1b[K\nscored: 397 of 400"),
            "the carriage-return line has to be ended first: {}",
            rendered.out
        );
    }

    #[test]
    fn quiet_prints_one_outcome_line_and_nothing_per_frame() {
        let events = [
            started(400),
            answered(12),
            failed(13),
            finished(Ending::Completed),
        ];

        let rendered = transcript(&events, Verbosity::Quiet, true);

        assert_eq!(
            rendered.out,
            "scored 397 of 400 in 8m12s · ok 397 · no_preview 1 · timeout 2 · 3 resumed · \
             2 deduplicated · left out: 3 skipped (not RAW) · 2 ignored (Finder noise) · \
             1 hidden directory not scanned · rubric 3f2a1b0c · model clef-flash-oQ4e\n"
                .replace("3f2a1b0c", summary().rubric_hash.short()),
            "a script reads this line and the exit code"
        );
        assert!(rendered.err.is_empty());
    }

    #[test]
    fn quiet_still_says_on_stderr_why_a_run_stopped() {
        let stopped = transcript(
            &[finished(Ending::Aborted {
                reason: "http_400: invalid criteria".to_string(),
            })],
            Verbosity::Quiet,
            false,
        );
        assert_eq!(stopped.err, "stopped: http_400: invalid criteria\n");

        let interrupted = transcript(&[finished(Ending::Interrupted)], Verbosity::Quiet, false);
        assert!(
            interrupted.err.starts_with("interrupted: "),
            "{}",
            interrupted.err
        );

        let completed = transcript(&[finished(Ending::Completed)], Verbosity::Quiet, false);
        assert!(
            completed.err.is_empty(),
            "a finished run warns about nothing"
        );
    }

    #[test]
    fn verbose_reports_every_frame_with_the_bytes_that_went_out() {
        let events = [
            started(400),
            answered(12),
            failed(13),
            finished(Ending::Completed),
        ];

        let rendered = transcript(&events, Verbosity::Verbose, false);

        assert!(
            rendered.out.starts_with("scoring 400 frames · rubric "),
            "{}",
            rendered.out
        );
        assert!(
            rendered.out.contains(
                "DSC_0012.NEF  ok  1568x1045  215 KB  812ms  overall 6.4237  technical 7.8125  \
                 composition 5.3398  impact 4.9601  4★ like\n"
            ),
            "{}",
            rendered.out
        );
        assert!(
            rendered.out.contains(
                "DSC_0013.NEF  timeout  POST /v1/systemone timed out after 120s — 3 attempts\n"
            ),
            "a retried failure says how many attempts it cost: {}",
            rendered.out
        );
        assert!(
            rendered
                .out
                .ends_with("rubric 69139540 · model clef-flash-oQ4e\n")
                && rendered.out.contains("scored: 397 of 400 in 8m12s"),
            "the summary follows: {}",
            rendered.out
        );
    }

    #[test]
    fn verbose_names_the_frame_a_duplicate_reused() {
        let mut duplicate = answered(13);
        if let Event::Frame(frame) = &mut duplicate {
            frame.duplicate_of = Some("DSC_0001.NEF".to_string());
            frame.preview = None;
        }

        let rendered = transcript(&[duplicate], Verbosity::Verbose, false);

        assert!(
            rendered
                .out
                .ends_with("4★ like  duplicate of DSC_0001.NEF\n"),
            "{}",
            rendered.out
        );
    }

    #[test]
    fn warnings_reach_stderr_in_every_mode() {
        let warning = Event::Warning(Warning {
            message: "scores.ledger.jsonl ends mid-record".to_string(),
        });

        for verbosity in [Verbosity::Quiet, Verbosity::Normal, Verbosity::Verbose] {
            let rendered = transcript(std::slice::from_ref(&warning), verbosity, false);
            assert_eq!(
                rendered.err, "warning: scores.ledger.jsonl ends mid-record\n",
                "{verbosity:?} must still say what went wrong"
            );
            assert!(rendered.out.is_empty(), "{verbosity:?}");
        }
    }

    #[test]
    fn durations_read_the_way_a_photographer_says_them() {
        assert_eq!(duration_text(Duration::from_secs(0)), "0s");
        assert_eq!(duration_text(Duration::from_secs(42)), "42s");
        assert_eq!(duration_text(Duration::from_secs(65)), "1m05s");
        assert_eq!(duration_text(Duration::from_secs(492)), "8m12s");
        assert_eq!(duration_text(Duration::from_secs(3720)), "1h02m");
    }
}
