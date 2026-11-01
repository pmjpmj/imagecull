//! What a run tells the front end while it runs: the events a [`Reporter`]
//! receives, collected into a vector and asserted in order.
//!
//! This is the seam the terminal and the eventual window both sit behind, so
//! nothing here needs a terminal: the run is driven through the scoring entry
//! point with a scripted [`Decider`] and a reporter that keeps every event.

mod support;

use std::cell::RefCell;
use std::time::Duration;

use imagecull_core::error::ErrorKind;
use imagecull_core::ingest::ScanOptions;
use imagecull_core::ledger::{self, LedgerMode};
use imagecull_core::preprocess::Params;
use imagecull_core::progress::{Event, FrameEvent, Judgement, Reporter, Silent};
use imagecull_core::rubric::Rubric;
use imagecull_core::scoring::{self, Cancel, Config, RetryPolicy};
use support::decider::{Scripted, document};
use support::shoot;

/// A reporter that keeps every event, in the order the run produced them: the
/// test-side implementation of the trait, with no terminal anywhere near it.
#[derive(Default)]
struct Collected {
    events: RefCell<Vec<Event>>,
}

impl Reporter for Collected {
    fn event(&self, event: Event) {
        self.events.borrow_mut().push(event);
    }
}

impl Collected {
    fn events(&self) -> Vec<Event> {
        self.events.borrow().clone()
    }

    /// The frame events, in order: what the progress line would have drawn.
    fn frames(&self) -> Vec<FrameEvent> {
        self.events()
            .into_iter()
            .filter_map(|event| match event {
                Event::Frame(frame) => Some(frame),
                _ => None,
            })
            .collect()
    }
}

/// One synthetic NEF carrying `frame` as its embedded preview.
fn nef(frame: Vec<u8>) -> Vec<u8> {
    support::nef(1, &[frame])
}

fn config() -> Config {
    let mut config = Config::new(
        Rubric::embedded().expect("the embedded rubric parses"),
        None,
        Params::default(),
        ScanOptions::default(),
        None,
        support::unwatched(),
    );
    config.retry = RetryPolicy::without_waiting();
    config
}

/// A run over the frames in `shoot`, with `decider` answering and every event
/// collected.
fn driven(shoot: &tempfile::TempDir, decider: &Scripted) -> (Collected, scoring::Run) {
    let reporter = Collected::default();
    let run =
        scoring::run(shoot.path(), &config(), decider, &reporter, &Cancel::new()).expect("a run");
    (reporter, run)
}

#[test]
fn the_run_announces_its_total_rubric_and_model_before_the_first_frame() {
    let shoot = shoot(&[
        ("DSC_0001.NEF", nef(support::jpeg(64, 32, 80))),
        ("DSC_0002.NEF", nef(support::jpeg(64, 32, 80))),
    ]);
    let config = config();
    let reporter = Collected::default();

    scoring::run(
        shoot.path(),
        &config,
        &Scripted::always_reading(2),
        &reporter,
        &Cancel::new(),
    )
    .expect("a run");

    let events = reporter.events();
    let Event::Started(started) = &events[0] else {
        panic!("the first event must be the run starting: {events:?}");
    };
    assert_eq!(started.total, 2, "the run knows how many frames it faces");
    assert_eq!(started.rubric_hash, config.rubric_hash());
    assert_eq!(started.model, config.model);
}

#[test]
fn every_frame_is_reported_in_ingest_order_with_its_answers_and_latency() {
    let shoot = shoot(&[
        ("DSC_0002.NEF", nef(support::jpeg(64, 32, 80))),
        ("DSC_0001.NEF", nef(support::jpeg(48, 32, 80))),
        ("DSC_0003.NEF", nef(support::jpeg(32, 32, 80))),
    ]);
    let decider = Scripted::always_reading(3).taking(812);

    let (reporter, _run) = driven(&shoot, &decider);

    let frames = reporter.frames();
    let names: Vec<&str> = frames.iter().map(|frame| frame.filename.as_str()).collect();
    assert_eq!(
        names,
        ["DSC_0001.NEF", "DSC_0002.NEF", "DSC_0003.NEF"],
        "the progress line follows ingest order, like the CSV"
    );
    let positions: Vec<(usize, usize)> = frames
        .iter()
        .map(|frame| (frame.index, frame.total))
        .collect();
    assert_eq!(positions, [(1, 3), (2, 3), (3, 3)]);

    let first = &frames[0];
    let Judgement::Answered(answers) = &first.judgement else {
        panic!("the model answered this frame: {first:?}");
    };
    assert_eq!(answers.overall, 6.4237, "the model's own precision");
    assert_eq!(answers.stars, 4);
    assert_eq!(first.latency_ms, Some(812), "the request's own latency");
    let preview = first.preview.as_ref().expect("the bytes that were sent");
    assert_eq!((preview.width, preview.height), (48, 32));
    assert!(preview.jpeg_bytes > 0, "{preview:?}");
}

#[test]
fn a_frame_without_answers_is_reported_with_its_status_and_message() {
    let shoot = shoot(&[("DSC_0001.NEF", b"not a NEF at all".to_vec())]);

    let (reporter, _run) = driven(&shoot, &Scripted::answering(vec![]));

    let frames = reporter.frames();
    assert_eq!(frames.len(), 1);
    let Judgement::Failed(failure) = &frames[0].judgement else {
        panic!("an unreadable file has no answers: {frames:?}");
    };
    assert_eq!(failure.kind, ErrorKind::Unreadable);
    assert!(failure.message.contains("DSC_0001.NEF"), "{failure:?}");
    assert_eq!(
        frames[0].latency_ms, None,
        "no request was made, so there is no latency"
    );
}

#[test]
fn a_reading_that_could_not_be_read_is_reported_as_its_status() {
    let shoot = shoot(&[("DSC_0001.NEF", nef(support::jpeg(64, 32, 80)))]);
    let unreadable = serde_json::json!({"answers": {"overall": {"score": 6.0}}});

    let (reporter, _run) = driven(&shoot, &Scripted::answering(vec![Ok(unreadable)]));

    let frames = reporter.frames();
    let Judgement::Failed(failure) = &frames[0].judgement else {
        panic!("a document with no stars cannot be read: {frames:?}");
    };
    assert_eq!(failure.kind, ErrorKind::BadResponse);
    assert_eq!(
        frames[0].latency_ms,
        Some(812),
        "the request was made and took the time it took"
    );
}

/// A failure nobody classified cannot become a row, so it never becomes a frame
/// event either: the run returns it instead.
#[test]
fn a_fatal_error_returns_before_any_frame_is_reported() {
    let reporter = Collected::default();
    let error = scoring::run(
        std::path::Path::new("/no/such/shoot"),
        &config(),
        &Scripted::answering(vec![]),
        &reporter,
        &Cancel::new(),
    )
    .expect_err("a missing folder is fatal");

    assert!(error.is_fatal(), "{error}");
    assert!(
        reporter.frames().is_empty(),
        "nothing was scored, so there is nothing to report"
    );
}

/// Compile-time proof that a frame is ordinary data: no borrow of the run, no
/// terminal, no lifetime.
#[test]
fn a_frame_event_can_be_kept_and_compared_after_the_run_is_gone() {
    let shoot = shoot(&[("DSC_0001.NEF", nef(support::jpeg(64, 32, 80)))]);
    let kept: Vec<FrameEvent> = {
        let (reporter, _run) = driven(&shoot, &Scripted::always_reading(1));
        reporter.frames()
    };

    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].filename, "DSC_0001.NEF");
}

/// The failure type is the one the CSV uses, so a progress line and a row can
/// never disagree about what went wrong.
#[test]
fn the_reported_failure_is_the_rows_own_failure() {
    let shoot = shoot(&[("DSC_0001.NEF", b"not a NEF at all".to_vec())]);
    let (reporter, run) = driven(&shoot, &Scripted::answering(vec![]));

    let Judgement::Failed(failure) = &reporter.frames()[0].judgement else {
        panic!("this frame has no answers");
    };
    let row_failure = run.frames[0]
        .outcome
        .as_ref()
        .err()
        .map(imagecull_core::report::Failure::of)
        .expect("an unreadable frame is an error");
    assert_eq!(*failure, row_failure);
}

#[test]
fn the_eta_shrinks_to_nothing_by_the_last_frame() {
    let shoot = shoot(&[
        ("DSC_0001.NEF", nef(support::jpeg(64, 32, 80))),
        ("DSC_0002.NEF", nef(support::jpeg(64, 32, 80))),
    ]);

    let (reporter, _run) = driven(&shoot, &Scripted::always_reading(2).taking(1));

    let frames = reporter.frames();
    assert!(
        frames[0].eta.is_some(),
        "with work left, there is an estimate to give"
    );
    assert_eq!(
        frames[1].eta, None,
        "on the last frame there is nothing left to estimate"
    );
}

#[test]
fn a_resumed_frame_is_reported_too() {
    let shoot = shoot(&[
        ("DSC_0001.NEF", nef(support::jpeg(64, 32, 80))),
        ("DSC_0002.NEF", nef(support::jpeg(48, 32, 80))),
    ]);
    let report = tempfile::tempdir().expect("a temp dir");
    let mut config = config();
    config.outputs.csv = report.path().join("scores.csv");
    scoring::run(
        shoot.path(),
        &config,
        &Scripted::always_reading(2),
        &Silent,
        &Cancel::new(),
    )
    .expect("a run");

    // The same shoot again, picking up what the ledger already holds.
    let mut resumed_config = config.clone();
    resumed_config.ledger = LedgerMode::Resume;
    let reporter = Collected::default();
    let run = scoring::run(
        shoot.path(),
        &resumed_config,
        &Scripted::answering(vec![]),
        &reporter,
        &Cancel::new(),
    )
    .expect("a resumed run");

    assert_eq!(run.resumed, 2);
    let frames = reporter.frames();
    assert_eq!(frames.len(), 2, "a resumed frame is still progress");
    assert_eq!(frames[0].latency_ms, Some(812), "what the ledger recorded");
    assert_eq!(
        frames[0].preview, None,
        "nothing was prepared, so there are no bytes to report"
    );
}

#[test]
fn a_deduplicated_frame_names_the_frame_it_reused() {
    // One preview, two files: a burst, where the second frame costs no request.
    let preview = support::jpeg(64, 32, 80);
    let shoot = shoot(&[
        (
            "DSC_0001.NEF",
            support::nef(1, std::slice::from_ref(&preview)),
        ),
        ("DSC_0002.NEF", support::nef(1, &[preview])),
    ]);
    let decider = Scripted::answering(vec![Ok(document(6.4237, "4"))]);

    let (reporter, _run) = driven(&shoot, &decider);

    assert_eq!(decider.seen().len(), 1, "one request for the burst");
    let frames = reporter.frames();
    assert_eq!(frames[0].duplicate_of, None);
    assert_eq!(frames[1].duplicate_of.as_deref(), Some("DSC_0001.NEF"));
}

#[test]
fn a_ledger_cut_short_by_a_killed_run_is_reported_as_a_warning() {
    let shoot = shoot(&[
        ("DSC_0001.NEF", nef(support::jpeg(64, 32, 80))),
        ("DSC_0002.NEF", nef(support::jpeg(48, 32, 80))),
    ]);
    let report = tempfile::tempdir().expect("a temp dir");
    let csv = report.path().join("scores.csv");
    let mut config = config();
    config.outputs.csv = csv.clone();
    scoring::run(
        shoot.path(),
        &config,
        &Scripted::always_reading(2),
        &Silent,
        &Cancel::new(),
    )
    .expect("a run");

    // A hard kill mid-write: the file ends inside a record.
    let ledger = ledger::path_for(&csv);
    let whole = std::fs::read(&ledger).expect("the ledger");
    std::fs::write(&ledger, &whole[..whole.len() - 30]).expect("a cut-short ledger");

    let mut resumed_config = config.clone();
    resumed_config.ledger = LedgerMode::Resume;
    let reporter = Collected::default();
    scoring::run(
        shoot.path(),
        &resumed_config,
        &Scripted::always_reading(1),
        &reporter,
        &Cancel::new(),
    )
    .expect("a resumed run");

    let warnings: Vec<String> = reporter
        .events()
        .into_iter()
        .filter_map(|event| match event {
            Event::Warning(warning) => Some(warning.message),
            _ => None,
        })
        .collect();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("ledger"), "{}", warnings[0]);
}

#[test]
fn the_run_finishes_with_the_summary_its_frames_add_up_to() {
    let shoot = shoot(&[
        ("DSC_0001.NEF", nef(support::jpeg(64, 32, 80))),
        ("DSC_0002.NEF", b"not a NEF at all".to_vec()),
    ]);
    let decider = Scripted::answering(vec![Ok(document(6.4237, "4"))]);

    let (reporter, run) = driven(&shoot, &decider);

    let events = reporter.events();
    let Event::Finished(finished) = events.last().expect("events") else {
        panic!("a run ends by saying how it went: {events:?}");
    };
    let summary = &finished.summary;
    assert_eq!(summary.judged, 2);
    assert_eq!(summary.scored, 1);
    assert_eq!(summary.failed, 1);
    assert_eq!(summary.ending, run.ending);
    assert_eq!(summary.total, run.total);
    assert!(
        summary.elapsed > Duration::ZERO,
        "a run measures its own wall time: {:?}",
        summary.elapsed
    );
}
