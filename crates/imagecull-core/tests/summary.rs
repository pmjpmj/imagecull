//! The summary a run ends with: counts by status, the rating's mean and median,
//! the star histogram and the wall time, as data a front end renders.
//!
//! Every number is computed from a [`Run`], so a hand-built run is the whole
//! input: no folder, no decider, no clock. The expected values below are worked
//! examples, not the code's own arithmetic.

use std::path::PathBuf;
use std::time::Duration;

use imagecull_core::error::ErrorKind;
use imagecull_core::ingest::{ScanCounts, SourceFile};
use imagecull_core::progress::{MeanMedian, Summary};
use imagecull_core::reading::{Answers, Reading};
use imagecull_core::rubric::Builtin;
use imagecull_core::scoring::{Ending, Frame, Run, Usage};

fn source(name: &str) -> SourceFile {
    SourceFile {
        path: PathBuf::from("/shoot").join(name),
        relative: PathBuf::from(name),
    }
}

fn answers(stars: u8) -> Answers {
    Answers {
        stars,
        confidence: 0.63,
    }
}

/// A scored frame with the given rating.
fn scored(name: &str, stars: u8) -> Frame {
    Frame {
        source: source(name),
        outcome: Ok(Reading {
            answers: Ok(answers(stars)),
            usage: Usage::measured(812),
            document: serde_json::Value::Null,
        }),
    }
}

fn failed(name: &str, kind: ErrorKind) -> Frame {
    Frame {
        source: source(name),
        outcome: Err(imagecull_core::Error::new(
            kind,
            format!("{name} did not answer"),
        )),
    }
}

fn run(frames: Vec<Frame>) -> Run {
    Run {
        builtin: None,
        rubric_hash: Builtin::General
            .rubric()
            .expect("parse")
            .hash("clef-flash-oQ4e"),
        model: "clef-flash-oQ4e".to_string(),
        counts: ScanCounts::default(),
        total: frames.len(),
        frames,
        resumed: 0,
        duplicates: 0,
        elapsed: Duration::from_millis(1),
        ending: Ending::Completed,
        source: None,
    }
}

#[test]
fn the_built_in_a_run_used_rides_through_the_summary() {
    let mut run = run(vec![scored("a.NEF", 3)]);
    run.builtin = Some(Builtin::General);
    assert_eq!(
        run.summary().builtin,
        Some(Builtin::General),
        "the summary has to be able to say which built-in judged the folder"
    );

    run.builtin = None;
    assert_eq!(
        run.summary().builtin,
        None,
        "a rubric loaded from a file has no built-in to name"
    );
}

fn statuses(summary: &Summary) -> Vec<(&str, usize)> {
    summary
        .counts
        .iter()
        .map(|count| (count.status, count.count))
        .collect()
}

#[test]
fn the_status_counts_add_up_to_the_frames_the_run_got_to() {
    let run = run(vec![
        scored("DSC_0001.NEF", 3),
        scored("DSC_0002.NEF", 2),
        failed("DSC_0003.NEF", ErrorKind::NoPreview),
        failed("DSC_0004.NEF", ErrorKind::Timeout),
        failed("DSC_0005.NEF", ErrorKind::Timeout),
    ]);

    let summary = run.summary();

    assert_eq!(
        statuses(&summary),
        [("ok", 2), ("no_preview", 1), ("timeout", 2)],
        "every frame is counted once, under the status its row carries"
    );
    assert_eq!(summary.judged, 5);
    assert_eq!(summary.scored, 2);
    assert_eq!(summary.failed, 3);
    assert_eq!(
        summary
            .counts
            .iter()
            .map(|count| count.count)
            .sum::<usize>(),
        summary.judged
    );
}

#[test]
fn the_mean_and_median_of_the_rating_come_from_the_frames_that_have_answers() {
    let run = run(vec![
        scored("DSC_0001.NEF", 1),
        scored("DSC_0002.NEF", 3),
        failed("DSC_0003.NEF", ErrorKind::Timeout),
        scored("DSC_0004.NEF", 4),
        scored("DSC_0005.NEF", 5),
    ]);

    let summary = run.summary();

    assert_eq!(
        summary.rating,
        Some(MeanMedian {
            mean: 3.25,
            median: 3.5,
        }),
        "the failed frame is in neither figure; an even count takes the midpoint of the two \
         middle values"
    );
}

#[test]
fn the_star_histogram_counts_every_scored_frame_once() {
    let run = run(vec![
        scored("DSC_0001.NEF", 1),
        scored("DSC_0002.NEF", 1),
        scored("DSC_0003.NEF", 3),
        failed("DSC_0004.NEF", ErrorKind::PreviewBroken),
        scored("DSC_0005.NEF", 5),
    ]);

    let summary = run.summary();

    assert_eq!(summary.stars, [2, 0, 1, 0, 1], "index 0 is one star");
    assert_eq!(
        summary.stars.iter().sum::<usize>(),
        summary.scored,
        "a failed frame is in no bucket"
    );
}

#[test]
fn a_run_with_no_answers_has_no_statistics_but_still_has_counts() {
    let run = run(vec![failed("DSC_0001.NEF", ErrorKind::NoPreview)]);

    let summary = run.summary();

    assert_eq!(summary.rating, None);
    assert_eq!(summary.stars, [0; 5]);
    assert_eq!(statuses(&summary), [("no_preview", 1)]);
}

#[test]
fn the_summary_carries_the_time_it_took_and_the_frames_it_reused() {
    let mut run = run(vec![scored("DSC_0001.NEF", 3)]);
    run.total = 400;
    run.resumed = 3;
    run.duplicates = 2;
    run.elapsed = Duration::from_secs(8 * 60 + 12);
    run.counts = ScanCounts {
        skipped_extension: 3,
        ignored: 2,
        hidden_directories: 1,
    };

    let summary = run.summary();

    assert_eq!(summary.elapsed, Duration::from_secs(492));
    assert_eq!(summary.resumed, 3);
    assert_eq!(summary.duplicates, 2);
    assert_eq!(summary.total, 400, "the folder, not the frames it got to");
    assert_eq!(summary.left_out, run.counts, "what the scan skipped");
}

#[test]
fn how_the_run_ended_is_part_of_its_summary() {
    let mut stopped = run(vec![scored("DSC_0001.NEF", 3)]);
    stopped.ending = Ending::Aborted {
        reason: "http_400: invalid criteria".into(),
    };
    let mut interrupted = run(vec![scored("DSC_0001.NEF", 3)]);
    interrupted.ending = Ending::Interrupted;

    assert_eq!(stopped.summary().ending, stopped.ending);
    assert_eq!(interrupted.summary().ending, Ending::Interrupted);
}
