//! The scoring entry point through its one seam: a fake [`Decider`] records
//! exactly what the pipeline would send, and the run reports what came back.
//!
//! Orientation, the size cap, the state, the questions, the model alias and the
//! order of requests are all asserted here — at the boundary the spec names
//! (`Decider`), on synthetic NEFs, without a server.

mod support;

use std::fs;
use std::path::Path;

use imagecull_core::dry_run;
use imagecull_core::error::{Error, ErrorKind};
use imagecull_core::ingest::ScanOptions;
use imagecull_core::preprocess::Params;
use imagecull_core::reading::Verdict;
use imagecull_core::rubric::Rubric;
use imagecull_core::scoring::{self, Config};
use serde_json::json;
use support::decider::{Scripted, document};
use tempfile::TempDir;

fn config(params: Params, context: Option<&str>) -> Config {
    Config::new(
        Rubric::embedded().expect("the embedded rubric parses"),
        None,
        params,
        ScanOptions::default(),
        context.map(str::to_string),
        support::unwatched(),
    )
}

fn limited(limit: usize) -> Config {
    Config::new(
        Rubric::embedded().expect("the embedded rubric parses"),
        None,
        Params::default(),
        ScanOptions {
            recursive: false,
            limit: Some(limit),
        },
        None,
        support::unwatched(),
    )
}

fn shoot(files: &[(&str, Vec<u8>)]) -> TempDir {
    let dir = tempfile::tempdir().expect("a temp dir");
    for (name, bytes) in files {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create a subfolder");
        }
        fs::write(path, bytes).expect("write fixture");
    }
    dir
}

fn nef(frame: Vec<u8>) -> Vec<u8> {
    support::nef(1, &[frame])
}

#[test]
fn every_frame_reaches_the_decider_with_its_questions_state_and_alias() {
    let shoot = shoot(&[
        ("DSC_0001.NEF", nef(support::jpeg(640, 480, 80))),
        ("sub/DSC_0002.NEF", nef(support::jpeg(320, 240, 80))),
    ]);
    let decider = Scripted::always_reading(2);
    let mut config = config(Params::default(), Some("street at dusk, high ISO"));
    config.scan.recursive = true;
    let run = scoring::run(shoot.path(), &config, &decider).expect("a run");

    assert_eq!(run.frames.len(), 2);
    assert_eq!(run.scored(), 2);
    assert_eq!(run.failed(), 0);
    assert_eq!(run.model, "clef-flash-oQ4e");

    let seen = decider.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].model, "clef-flash-oQ4e");
    assert_eq!(
        seen[0].question_ids,
        [
            "overall",
            "technical",
            "composition",
            "impact",
            "stars",
            "verdict"
        ],
        "the rubric's questions go out in declaration order"
    );
    assert_eq!(seen[0].state.task, "photo_cull");
    assert_eq!(seen[0].state.filename, "DSC_0001.NEF");
    assert_eq!(
        seen[0].state.shoot_context.as_deref(),
        Some("street at dusk, high ISO"),
        "the photographer's framing reaches the model"
    );
    assert_eq!(seen[0].width, 640);
    assert_eq!(seen[0].height, 480);
    assert_eq!(seen[1].state.filename, "sub/DSC_0002.NEF");
}

#[test]
fn a_portrait_frame_reaches_the_decider_upright_and_capped() {
    // Stored 40x20 with the left half black, EXIF orientation 6: upright means
    // 20x40, the black edge on top, and the long edge at the cap.
    let shoot = shoot(&[(
        "DSC_0001.NEF",
        support::nef(6, &[support::left_black_right_white(80, 40)]),
    )]);
    let decider = Scripted::always_reading(1);
    let params = Params {
        max_edge: 40,
        ..Params::default()
    };

    scoring::run(shoot.path(), &config(params, None), &decider).expect("a run");

    let seen = &decider.seen()[0];
    assert_eq!(
        (seen.width, seen.height),
        (20, 40),
        "orientation is applied before the cap, so a portrait frame stays portrait"
    );
    let image = image::load_from_memory(&seen.jpeg)
        .expect("decode")
        .to_rgb8();
    assert!(
        image.get_pixel(10, 4).0[0] < 40,
        "the stored left edge is the displayed top: the decider sees an upright frame"
    );
    assert!(image.get_pixel(10, 36).0[0] > 215);
}

#[test]
fn a_preview_smaller_than_the_cap_is_never_upscaled_on_its_way_to_the_decider() {
    let shoot = shoot(&[("DSC_0001.NEF", nef(support::jpeg(64, 32, 80)))]);
    let decider = Scripted::always_reading(1);

    scoring::run(shoot.path(), &config(Params::default(), None), &decider).expect("a run");

    let seen = &decider.seen()[0];
    assert_eq!((seen.width, seen.height), (64, 32));
    let decoded = image::load_from_memory(&seen.jpeg)
        .expect("decode")
        .to_rgb8();
    assert_eq!(decoded.dimensions(), (64, 32));
}

#[test]
fn the_run_reports_all_six_answers_to_its_caller() {
    let shoot = shoot(&[("DSC_0001.NEF", nef(support::jpeg(64, 32, 80)))]);
    let decider = Scripted::always_reading(1);

    let run =
        scoring::run(shoot.path(), &config(Params::default(), None), &decider).expect("a run");

    let frame = &run.frames[0];
    assert_eq!(frame.source.relative.to_str(), Some("DSC_0001.NEF"));
    let reading = frame
        .outcome
        .as_ref()
        .expect("the frame reached the decider");
    let answers = reading.answers.as_ref().expect("six readable answers");
    assert_eq!(answers.overall, 6.4237);
    assert_eq!(answers.technical, 7.8125);
    assert_eq!(answers.composition, 5.3398);
    assert_eq!(answers.impact, 4.9601);
    assert_eq!(answers.stars, 4);
    assert_eq!(answers.verdict, Verdict::Like);
    assert_eq!(answers.confidence.verdict, 0.88);
    assert_eq!(reading.usage.input_tokens, Some(1327));
    assert_eq!(
        reading.usage.latency_ms(),
        812,
        "client-measured when unstated"
    );
}

#[test]
fn an_unreadable_file_is_classified_and_the_run_continues() {
    let shoot = shoot(&[
        ("DSC_0001.NEF", b"this is not a RAW file".to_vec()),
        ("DSC_0002.NEF", nef(support::jpeg(64, 32, 80))),
    ]);
    let decider = Scripted::always_reading(1);

    let run = scoring::run(shoot.path(), &config(Params::default(), None), &decider)
        .expect("the folder is readable, so the run happens");

    assert_eq!(run.frames.len(), 2);
    assert_eq!(run.scored(), 1);
    assert_eq!(run.failed(), 1);
    let error = run.frames[0]
        .outcome
        .as_ref()
        .expect_err("a garbage file cannot be prepared");
    assert_eq!(error.kind(), Some(ErrorKind::Unreadable));
    assert!(
        error.message().contains("DSC_0001.NEF"),
        "the row's error names the file: {}",
        error.message()
    );
    assert_eq!(
        decider.seen().len(),
        1,
        "the model is never asked about a file that could not be prepared"
    );
}

#[test]
fn a_transport_failure_is_classified_and_does_not_stop_the_run() {
    let shoot = shoot(&[
        ("DSC_0001.NEF", nef(support::jpeg(64, 32, 80))),
        ("DSC_0002.NEF", nef(support::jpeg(48, 24, 80))),
    ]);
    let decider = Scripted::answering(vec![
        Err(Error::new(
            ErrorKind::Timeout,
            "POST /v1/systemone timed out",
        )),
        Ok(document(5.0, "3")),
    ]);

    let run =
        scoring::run(shoot.path(), &config(Params::default(), None), &decider).expect("a run");

    let timeout = run.frames[0].outcome.as_ref().expect_err("no reading");
    assert_eq!(timeout.kind(), Some(ErrorKind::Timeout));
    let second = run.frames[1].outcome.as_ref().expect("the run continued");
    assert_eq!(second.answers.as_ref().expect("readable").stars, 3);
    assert_eq!(run.scored(), 1);
    assert_eq!(run.failed(), 1);
}

#[test]
fn a_bad_response_keeps_its_document_and_the_run_continues() {
    let shoot = shoot(&[
        ("DSC_0001.NEF", nef(support::jpeg(64, 32, 80))),
        ("DSC_0002.NEF", nef(support::jpeg(48, 24, 80))),
    ]);
    let unreadable = json!({"answers": {"overall": {"score": 6.0}}});
    let decider = Scripted::answering(vec![Ok(unreadable.clone()), Ok(document(5.0, "3"))]);

    let run =
        scoring::run(shoot.path(), &config(Params::default(), None), &decider).expect("a run");

    let reading = run.frames[0]
        .outcome
        .as_ref()
        .expect("a document arrived, so the frame is not a transport failure");
    let error = reading
        .answers
        .as_ref()
        .expect_err("the document is unreadable");
    assert_eq!(error.kind(), Some(ErrorKind::BadResponse));
    assert_eq!(
        reading.document, unreadable,
        "the raw document survives for --json-scores"
    );
    assert_eq!(run.scored(), 1);
    assert_eq!(run.failed(), 1);
}

#[test]
fn requests_are_strictly_sequential_in_ingest_order() {
    let shoot = shoot(&[
        ("DSC_0001.NEF", nef(support::jpeg(64, 32, 80))),
        ("DSC_0002.NEF", nef(support::jpeg(60, 30, 80))),
        ("DSC_0003.NEF", nef(support::jpeg(56, 28, 80))),
    ]);
    let decider = Scripted::always_reading(3);

    scoring::run(shoot.path(), &config(Params::default(), None), &decider).expect("a run");

    assert!(
        !decider.overlapped(),
        "a second request must never be in flight while the first is unanswered"
    );
    let names: Vec<String> = decider
        .seen()
        .iter()
        .map(|seen| seen.state.filename.clone())
        .collect();
    assert_eq!(names, ["DSC_0001.NEF", "DSC_0002.NEF", "DSC_0003.NEF"]);
}

#[test]
fn limit_scores_only_the_first_files_of_the_run() {
    let shoot = shoot(&[
        ("DSC_0001.NEF", nef(support::jpeg(64, 32, 80))),
        ("DSC_0002.NEF", nef(support::jpeg(60, 30, 80))),
        ("DSC_0003.NEF", nef(support::jpeg(56, 28, 80))),
    ]);
    let decider = Scripted::always_reading(1);

    let run = scoring::run(shoot.path(), &limited(1), &decider).expect("a run");

    assert_eq!(run.frames.len(), 1);
    assert_eq!(decider.seen().len(), 1);
}

#[test]
fn the_dry_run_writes_exactly_the_bytes_the_decider_received() {
    let shoot = shoot(&[
        (
            "DSC_0001.NEF",
            support::nef(6, &[support::left_black_right_white(40, 20)]),
        ),
        ("DSC_0002.NEF", nef(support::jpeg(600, 400, 80))),
    ]);
    let params = Params {
        max_edge: 100,
        ..Params::default()
    };
    let decider = Scripted::always_reading(2);
    scoring::run(shoot.path(), &config(params, None), &decider).expect("a run");

    let previews = tempfile::tempdir().expect("a temp dir");
    dry_run::render_previews(
        shoot.path(),
        previews.path(),
        &ScanOptions::default(),
        &params,
    )
    .expect("a dry run");

    let seen = decider.seen();
    let written = |name: &str| fs::read(previews.path().join(name)).expect("a written preview");
    assert_eq!(
        written("DSC_0001.jpg"),
        seen[0].jpeg,
        "the dry run and a scoring run share the per-frame step: same bytes, same orientation"
    );
    assert_eq!(written("DSC_0002.jpg"), seen[1].jpeg);
    assert_eq!(
        Support::dimensions(&written("DSC_0002.jpg")),
        (100, 67),
        "the dry run cap matches the scoring cap"
    );
}

/// A locally scoped helper namespace, so the test reads as one story.
struct Support;

impl Support {
    fn dimensions(jpeg: &[u8]) -> (u32, u32) {
        image::load_from_memory(jpeg)
            .expect("decode")
            .to_rgb8()
            .dimensions()
    }
}

#[test]
fn a_folder_with_no_raw_files_is_an_empty_run_not_an_error() {
    let shoot = shoot(&[("notes.txt", b"nothing to score".to_vec())]);
    let decider = Scripted::answering(vec![]);

    let run = scoring::run(shoot.path(), &config(Params::default(), None), &decider)
        .expect("an empty run");

    assert!(run.frames.is_empty());
    assert_eq!(run.counts.skipped_extension, 1);
    assert!(decider.seen().is_empty());
}

#[test]
fn the_missing_directory_is_a_fatal_error() {
    let decider = Scripted::answering(vec![]);
    let error = scoring::run(
        Path::new("/no/such/shoot"),
        &config(Params::default(), None),
        &decider,
    )
    .expect_err("a missing folder stops the run before any file");

    assert!(error.is_fatal());
}
