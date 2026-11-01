//! How a run looks on a terminal, rendered from the core's events.
//!
//! Rendering is a pure function of an [`Event`] and a verbosity, so every
//! presentation decision is assertable without a terminal: a test folds the
//! events of interest through [`render`] and reads the strings. [`Terminal`] is
//! the reporter that does nothing but write them.
//!
//! Three ways to watch the same run: `--quiet` says only what the run did, the
//! default writes a line per frame and then the summary, and `--verbose` writes
//! that line with the bytes that went out and the answers at full precision, so
//! a surprising result can be traced without re-running with a debugger. Nothing
//! checks where stdout points: a frame line ends in a newline, so a redirected
//! log is the same text a terminal received, and `--quiet` is how a script asks
//! for silence.

use std::io::Write;

use imagecull_core::ingest::ScanCounts;
use imagecull_core::progress::{Event, FrameEvent, Judgement, Reporter, Started, Summary, Warning};
use imagecull_core::rubric::{Builtin, RubricHash};
use imagecull_core::scoring::Ending;

/// How much of a run the photographer wants to see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verbosity {
    /// `--quiet`: the outcome and nothing else, so a script's output stays clean.
    Quiet,
    /// The default: a line per frame, then the summary.
    Normal,
    /// `--verbose`: that line plus what was sent and what came back.
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
/// A frame becomes one line, terminated by a newline, in every mode that says
/// anything about it: a line you can no longer read after it scrolls is a line
/// that was not worth printing.
pub fn render(event: &Event, verbosity: Verbosity) -> Rendering {
    match event {
        Event::Started(started) => Rendering {
            out: started_line(started, verbosity).unwrap_or_default(),
            err: String::new(),
        },
        Event::Frame(frame) => Rendering {
            out: frame_text(frame, verbosity).unwrap_or_default(),
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
                    out: summary_block(&finished.summary),
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
}

impl Terminal {
    /// A reporter for a run.
    pub fn new(verbosity: Verbosity) -> Terminal {
        Terminal { verbosity }
    }
}

impl Reporter for Terminal {
    fn event(&self, event: Event) {
        let rendering = render(&event, self.verbosity);
        write(Stream::Out, &rendering.out);
        write(Stream::Err, &rendering.err);
    }
}

/// Write `text` to `stream` and flush it, so a line appears when it is drawn
/// rather than when the buffer fills.
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
fn started_line(started: &Started, verbosity: Verbosity) -> Option<String> {
    match verbosity {
        Verbosity::Quiet => None,
        Verbosity::Normal | Verbosity::Verbose => Some(format!(
            "scoring {} {} · rubric {} · model {}\n",
            started.total,
            plural(started.total, "frame", "frames"),
            rubric_name(started.builtin, &started.rubric_hash),
            started.model,
        )),
    }
}

/// One frame: the line, or the verbose account of it.
fn frame_text(frame: &FrameEvent, verbosity: Verbosity) -> Option<String> {
    match verbosity {
        Verbosity::Quiet => None,
        Verbosity::Verbose => Some(verbose_line(frame)),
        Verbosity::Normal => Some(format!("{}\n", frame_line(frame))),
    }
}

/// The per-frame line: index, name, the rating, what it cost, whether an
/// identical preview paid for it, and when the run will be done.
fn frame_line(frame: &FrameEvent) -> String {
    let mut parts = vec![
        format!("{}/{}", frame.index, frame.total),
        frame.filename.clone(),
    ];
    match &frame.judgement {
        Judgement::Answered(answers) => {
            parts.push(format!("{}★", answers.stars));
        }
        Judgement::Skipped { rating } => parts.push(format!("{rating}★ kept")),
        Judgement::Failed(failure) => parts.push(failure.kind.as_str().to_string()),
    }
    if let Some(latency_ms) = frame.latency_ms {
        parts.push(format!("{latency_ms}ms"));
    }
    if let Some(original) = &frame.duplicate_of {
        parts.push(format!("duplicate of {original}"));
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
                "{}★  confidence {:.2}",
                answers.stars, answers.confidence
            ));
        }
        Judgement::Skipped { rating } => {
            parts.push(format!("kept {rating}★"));
            parts.push("the sidecar already had a rating".to_string());
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

/// `general/69139540` for a rubric selected by name, `69139540` for one loaded
/// from a file. The name says how the rubric was asked for and the hash says
/// what was judged, so a file — which has no name to give — prints the hash
/// alone rather than inventing one.
pub(crate) fn rubric_name(builtin: Option<Builtin>, hash: &RubricHash) -> String {
    match builtin {
        Some(builtin) => format!("{builtin}/{}", hash.short()),
        None => hash.short().to_string(),
    }
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
    if let Some(rating) = &summary.rating {
        out.push_str(&format!(
            "rating: {:.2} mean / {:.2} median\n",
            rating.mean, rating.median,
        ));
        out.push_str(&format!("stars: {}\n", histogram(&summary.stars)));
    }
    if summary.skipped > 0 || summary.duplicates > 0 {
        let mut parts = Vec::new();
        if summary.skipped > 0 {
            parts.push(format!("skipped: {}", summary.skipped));
        }
        if summary.duplicates > 0 {
            parts.push(format!("deduplicated: {}", summary.duplicates));
        }
        out.push_str(&format!("{}\n", parts.join(" · ")));
    }
    let left_out = left_out(&summary.left_out);
    if !left_out.is_empty() {
        out.push_str(&format!("left out: {}\n", left_out.join(" · ")));
    }
    out.push_str(&format!(
        "rubric {} · model {}",
        rubric_name(summary.builtin, &summary.rubric_hash),
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
    if summary.skipped > 0 {
        parts.push(format!("{} skipped", summary.skipped));
    }
    if summary.duplicates > 0 {
        parts.push(format!("{} deduplicated", summary.duplicates));
    }
    let left_out = left_out(&summary.left_out);
    if !left_out.is_empty() {
        parts.push(format!("left out: {}", left_out.join(" · ")));
    }
    parts.push(format!(
        "rubric {}",
        rubric_name(summary.builtin, &summary.rubric_hash)
    ));
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

/// A duration as a photographer reads it: `42s`, `8m12s`, `1h02m`.
pub(crate) fn duration_text(duration: std::time::Duration) -> String {
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
    use imagecull_core::progress::{MeanMedian, PreviewShape, StatusCount, Warning};
    use imagecull_core::reading::Answers;
    use imagecull_core::rubric::Builtin;
    use imagecull_core::scoring::Ending;
    use imagecull_core::sidecar::Failure;

    use super::*;

    fn answers() -> Answers {
        Answers {
            stars: 4,
            confidence: 0.63,
        }
    }

    fn started(total: usize) -> Event {
        Event::Started(Started {
            total,
            builtin: Some(Builtin::General),
            rubric_hash: Builtin::General
                .rubric()
                .expect("parse")
                .hash("clef-flash-oQ4e"),
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
        let rubric = Builtin::General
            .rubric()
            .expect("parse")
            .hash("clef-flash-oQ4e");
        Summary {
            total: 400,
            judged: 400,
            scored: 397,
            failed: 3,
            skipped: 3,
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
            rating: Some(MeanMedian {
                mean: 4.54,
                median: 5.0,
            }),
            stars: [0, 4, 51, 68, 274],
            builtin: Some(Builtin::General),
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
    fn transcript(events: &[Event], verbosity: Verbosity) -> Rendering {
        events.iter().fold(Rendering::default(), |mut seen, event| {
            let one = render(event, verbosity);
            seen.out.push_str(&one.out);
            seen.err.push_str(&one.err);
            seen
        })
    }

    #[test]
    fn the_frame_line_shows_index_name_rating_latency_and_eta() {
        let rendered = transcript(&[answered(12)], Verbosity::Normal);

        assert_eq!(
            rendered.out, "12/400  DSC_0012.NEF  4★  812ms  eta 7m41s\n",
            "one line per frame, ended by a newline"
        );
        assert!(rendered.err.is_empty());
    }

    #[test]
    fn a_failed_frame_keeps_its_place_on_a_line_of_its_own() {
        let rendered = transcript(&[failed(13)], Verbosity::Normal);

        assert_eq!(
            rendered.out, "13/400  DSC_0013.NEF  timeout  eta 7m39s\n",
            "the row's status, and no latency for a frame that never answered"
        );
    }

    #[test]
    fn normal_mode_writes_a_line_per_frame_and_never_a_carriage_return() {
        let events = [started(400), answered(12), failed(13)];

        let rendered = transcript(&events, Verbosity::Normal);

        assert!(
            rendered.out.starts_with("scoring 400 frames · rubric "),
            "a run says what it is about to do: {}",
            rendered.out
        );
        assert!(
            rendered.out.contains(
                "12/400  DSC_0012.NEF  4★  812ms  eta 7m41s\n\
                 13/400  DSC_0013.NEF  timeout  eta 7m39s\n"
            ),
            "every frame gets its own line: {}",
            rendered.out
        );
        assert!(
            !rendered.out.contains(['\r', '\x1b']),
            "nothing overwrites itself, so a redirected log is the same text: {:?}",
            rendered.out
        );
    }

    #[test]
    fn the_summary_block_reports_the_counts_the_rating_and_the_histogram() {
        let rendered = transcript(&[finished(Ending::Completed)], Verbosity::Normal);
        let short = summary().rubric_hash.short().to_string();

        let expected = format!(
            "scored: 397 of 400 in 8m12s\n\
             status: ok 397 · no_preview 1 · timeout 2\n\
             rating: 4.54 mean / 5.00 median\n\
             stars: 1★ 0 · 2★ 4 · 3★ 51 · 4★ 68 · 5★ 274\n\
             skipped: 3 · deduplicated: 2\n\
             left out: 3 skipped (not RAW) · 2 ignored (Finder noise) · \
             1 hidden directory not scanned\n\
             rubric general/{short} · model clef-flash-oQ4e\n"
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
        );

        assert!(
            rendered.out.contains(&format!(
                "rubric general/{short} · model clef-flash-oQ4e · config /Volumes/Shoot A/imagecull.toml\n"
            )),
            "the numbers have to be attributable to the file that made them: {}",
            rendered.out
        );
    }

    #[test]
    fn a_file_selected_rubric_prints_its_hash_without_a_name() {
        let mut summary = summary();
        summary.builtin = None;
        let short = summary.rubric_hash.short().to_string();

        let rendered = transcript(
            &[Event::Finished(imagecull_core::progress::Finished {
                summary,
            })],
            Verbosity::Normal,
        );

        assert!(
            rendered
                .out
                .ends_with(&format!("rubric {short} · model clef-flash-oQ4e\n")),
            "a rubric that was not selected by name has no name to print, and the hash \
             already identifies it: {}",
            rendered.out
        );
    }

    #[test]
    fn the_summary_follows_the_last_frame_line_on_its_own_line() {
        let rendered = transcript(
            &[started(400), failed(13), finished(Ending::Completed)],
            Verbosity::Normal,
        );

        assert!(
            rendered.out.contains("eta 7m39s\nscored: 397 of 400"),
            "the block needs no separator of its own: {}",
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

        let rendered = transcript(&events, Verbosity::Quiet);

        assert_eq!(
            rendered.out,
            "scored 397 of 400 in 8m12s · ok 397 · no_preview 1 · timeout 2 · 3 skipped · \
             2 deduplicated · left out: 3 skipped (not RAW) · 2 ignored (Finder noise) · \
             1 hidden directory not scanned · rubric general/3f2a1b0c · model clef-flash-oQ4e\n"
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
        );
        assert_eq!(stopped.err, "stopped: http_400: invalid criteria\n");

        let interrupted = transcript(&[finished(Ending::Interrupted)], Verbosity::Quiet);
        assert!(
            interrupted.err.starts_with("interrupted: "),
            "{}",
            interrupted.err
        );

        let completed = transcript(&[finished(Ending::Completed)], Verbosity::Quiet);
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

        let rendered = transcript(&events, Verbosity::Verbose);

        assert!(
            rendered.out.starts_with("scoring 400 frames · rubric "),
            "{}",
            rendered.out
        );
        assert!(
            rendered
                .out
                .contains("DSC_0012.NEF  ok  1568x1045  215 KB  812ms  4★  confidence 0.63\n"),
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
        let short = summary().rubric_hash.short().to_string();
        assert!(
            rendered
                .out
                .ends_with(&format!("rubric general/{short} · model clef-flash-oQ4e\n"))
                && rendered.out.contains("scored: 397 of 400 in 8m12s"),
            "the summary follows and names the built-in beside its hash: {}",
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

        let rendered = transcript(&[duplicate], Verbosity::Verbose);

        assert!(
            rendered
                .out
                .ends_with("4★  confidence 0.63  duplicate of DSC_0001.NEF\n"),
            "{}",
            rendered.out
        );
    }

    #[test]
    fn the_default_line_names_the_frame_a_duplicate_reused() {
        let mut duplicate = answered(13);
        if let Event::Frame(frame) = &mut duplicate {
            frame.duplicate_of = Some("DSC_0001.NEF".to_string());
        }

        let rendered = transcript(&[duplicate], Verbosity::Normal);

        assert_eq!(
            rendered.out, "13/400  DSC_0012.NEF  4★  812ms  duplicate of DSC_0001.NEF  eta 7m41s\n",
            "a reused rating says which frame paid for it, before the clock does"
        );
    }

    #[test]
    fn warnings_reach_stderr_in_every_mode() {
        let warning = Event::Warning(Warning {
            message: "scores.ledger.jsonl ends mid-record".to_string(),
        });

        for verbosity in [Verbosity::Quiet, Verbosity::Normal, Verbosity::Verbose] {
            let rendered = transcript(std::slice::from_ref(&warning), verbosity);
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
