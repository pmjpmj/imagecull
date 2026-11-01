//! The summary a run ends with: counts by status, mean and median per score, the
//! star histogram, the `stars`/`overall` disagreement rate and the wall time, as
//! data a front end renders.
//!
//! Every number is computed from a [`Run`], so a hand-built run is the whole
//! input: no folder, no decider, no clock. The expected values below are worked
//! examples, not the code's own arithmetic.

use std::path::PathBuf;
use std::time::Duration;

use imagecull_core::error::ErrorKind;
use imagecull_core::ingest::{ScanCounts, SourceFile};
use imagecull_core::progress::{DISAGREEMENT_RULE, Summary, agrees, implied_stars};
use imagecull_core::reading::{Answers, Confidences, Reading, Verdict};
use imagecull_core::rubric::Rubric;
use imagecull_core::scoring::{Ending, Frame, Run, Usage};

fn source(name: &str) -> SourceFile {
    SourceFile {
        path: PathBuf::from("/shoot").join(name),
        relative: PathBuf::from(name),
    }
}

fn answers(overall: f64, technical: f64, composition: f64, impact: f64, stars: u8) -> Answers {
    Answers {
        overall,
        technical,
        composition,
        impact,
        stars,
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

/// A scored frame: `overall` and `stars` as given, the other three fixed.
fn scored(name: &str, overall: f64, stars: u8) -> Frame {
    Frame {
        source: source(name),
        outcome: Ok(Reading {
            answers: Ok(answers(overall, 7.0, 5.0, 4.0, stars)),
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
        rubric_hash: Rubric::embedded().expect("parse").hash("clef-flash-oQ4e"),
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
        scored("DSC_0001.NEF", 6.0, 3),
        scored("DSC_0002.NEF", 4.0, 2),
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
fn the_mean_and_median_of_each_score_come_from_the_frames_that_have_answers() {
    let run = run(vec![
        scored_with("DSC_0001.NEF", [4.0, 5.0, 1.0, 0.0]),
        scored_with("DSC_0002.NEF", [6.0, 5.0, 2.0, 0.0]),
        failed("DSC_0003.NEF", ErrorKind::Timeout),
        scored_with("DSC_0004.NEF", [8.0, 5.0, 3.0, 10.0]),
        scored_with("DSC_0005.NEF", [10.0, 5.0, 4.0, 10.0]),
    ]);

    let summary = run.summary();

    let scores = summary.scores.expect("four frames were scored");
    assert_eq!((scores.overall.mean, scores.overall.median), (7.0, 7.0));
    assert_eq!(
        (scores.technical.mean, scores.technical.median),
        (5.0, 5.0),
        "a column of one value has that value for both"
    );
    assert_eq!(
        (scores.composition.mean, scores.composition.median),
        (2.5, 2.5),
        "an even count takes the midpoint of the two middle values"
    );
    assert_eq!(
        (scores.impact.mean, scores.impact.median),
        (5.0, 5.0),
        "the failed frame's empty cells are not a zero"
    );
}

/// A scored frame with all four scores given.
fn scored_with(name: &str, scores: [f64; 4]) -> Frame {
    Frame {
        source: source(name),
        outcome: Ok(Reading {
            answers: Ok(answers(scores[0], scores[1], scores[2], scores[3], 3)),
            usage: Usage::measured(812),
            document: serde_json::Value::Null,
        }),
    }
}

#[test]
fn the_star_histogram_counts_every_scored_frame_once() {
    let run = run(vec![
        scored("DSC_0001.NEF", 2.0, 1),
        scored("DSC_0002.NEF", 2.0, 1),
        scored("DSC_0003.NEF", 6.0, 3),
        failed("DSC_0004.NEF", ErrorKind::PreviewBroken),
        scored("DSC_0005.NEF", 10.0, 5),
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
fn the_disagreement_rate_counts_only_material_disagreements() {
    // (overall, stars) → the star band the overall implies:
    // 6.0 → 3 (agrees), 9.6 → 5 (agrees), 0.4 → 1 (agrees), 7.0 → 4 (agrees);
    // 6.0 with 5 stars is two away, 10.0 with 3 stars is two away, and 5.0 with
    // 1 star is two away from the 3 its overall implies.
    let cases = [
        (6.0, 3),
        (6.0, 5),
        (9.6, 5),
        (0.4, 1),
        (10.0, 3),
        (5.0, 1),
        (7.0, 4),
    ];
    let frames: Vec<Frame> = cases
        .iter()
        .enumerate()
        .map(|(index, (overall, stars))| scored(&format!("DSC_{index:04}.NEF"), *overall, *stars))
        .collect();

    let summary = run(frames).summary();

    assert_eq!(summary.disagreements, 3, "of seven");
    assert!(
        (summary.disagreement_rate - 3.0 / 7.0).abs() < 1e-12,
        "{}",
        summary.disagreement_rate
    );
    assert!(
        !DISAGREEMENT_RULE.is_empty(),
        "the rule is stated where the number is computed"
    );
}

#[test]
fn the_rule_maps_overall_to_the_star_band_it_implies() {
    assert_eq!(implied_stars(0.0), 1, "nothing clamps below one star");
    assert_eq!(implied_stars(2.0), 1);
    assert_eq!(implied_stars(3.0), 2);
    assert_eq!(implied_stars(7.0), 4);
    assert_eq!(implied_stars(9.9), 5);
    assert_eq!(implied_stars(10.0), 5, "nothing clamps above five stars");

    assert!(
        agrees(4, 7.0),
        "one band of rounding noise is not a disagreement"
    );
    assert!(agrees(2, 5.0));
    assert!(!agrees(5, 5.0), "two stars away from the band is material");
    assert!(!agrees(1, 6.0));
}

#[test]
fn a_run_with_no_answers_has_no_statistics_but_still_has_counts() {
    let run = run(vec![failed("DSC_0001.NEF", ErrorKind::NoPreview)]);

    let summary = run.summary();

    assert_eq!(summary.scores, None);
    assert_eq!(summary.stars, [0; 5]);
    assert_eq!(summary.disagreements, 0);
    assert_eq!(summary.disagreement_rate, 0.0, "0 of 0 is not a NaN");
    assert_eq!(statuses(&summary), [("no_preview", 1)]);
}

#[test]
fn the_summary_carries_the_time_it_took_and_the_frames_it_reused() {
    let mut run = run(vec![scored("DSC_0001.NEF", 6.0, 3)]);
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
    let mut stopped = run(vec![scored("DSC_0001.NEF", 6.0, 3)]);
    stopped.ending = Ending::Aborted {
        reason: "http_400: invalid criteria".into(),
    };
    let mut interrupted = run(vec![scored("DSC_0001.NEF", 6.0, 3)]);
    interrupted.ending = Ending::Interrupted;

    assert_eq!(stopped.summary().ending, stopped.ending);
    assert_eq!(interrupted.summary().ending, Ending::Interrupted);
}
