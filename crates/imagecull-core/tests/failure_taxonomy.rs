//! How a run fails, at the scale it fails: the taxonomy's ten statuses, the
//! retries that keep a hiccup from costing the shoot, the failures that stop the
//! run instead, and the code the process exits with.
//!
//! Everything enters through the scoring entry point with a scripted [`Decider`]
//! and is asserted on what the run left behind: the CSV it wrote, the frames it
//! judged, and the number of requests each frame cost — which is the attempt
//! count, observed at the seam rather than through a counter inside the loop.

mod support;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use imagecull_core::Result;
use imagecull_core::error::{Error, ErrorKind, ExitCode};
use imagecull_core::ingest::ScanOptions;
use imagecull_core::preprocess::Params;
use imagecull_core::progress::Silent;
use imagecull_core::report::Outputs;
use imagecull_core::rubric::Rubric;
use imagecull_core::scoring::{self, Cancel, Config, Ending, RetryPolicy};
use serde_json::json;
use support::decider::{Scripted, document};

/// A run over `csv`'s folder, with the spec's retry schedule shortened to
/// nothing: the attempt count is what these tests assert, not the waiting.
fn config(csv: PathBuf) -> Config {
    let mut config = Config::new(
        Rubric::embedded().expect("the embedded rubric parses"),
        None,
        Params::default(),
        ScanOptions::default(),
        None,
        Outputs {
            csv,
            json_scores: None,
            absolute: false,
        },
    );
    config.retry = RetryPolicy::without_waiting();
    config
}

/// One synthetic NEF per `count`, each with its own small preview.
fn shoot(count: usize) -> tempfile::TempDir {
    let names: Vec<String> = (1..=count)
        .map(|index| format!("DSC_{index:04}.NEF"))
        .collect();
    let files: Vec<(&str, Vec<u8>)> = names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let preview = support::jpeg(48 + index as u32, 32, 80);
            (name.as_str(), support::nef(1, &[preview]))
        })
        .collect();
    support::shoot(&files)
}

/// A failure the taxonomy classifies, as the adapter would hand it over.
fn failure(kind: ErrorKind, message: &str) -> Error {
    Error::new(kind, message)
}

/// The CSV a run wrote, as rows of cells, parsed the way RFC 4180 asks: a cell
/// may be quoted, may hold commas, and may hold a line break. Escaping in
/// general is `tests/report.rs`'s job; this is only enough to read a row back.
struct Csv {
    header: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl Csv {
    fn read(path: &Path) -> Csv {
        let text = fs::read_to_string(path).expect("the run wrote a CSV");
        let mut records = records(&text).into_iter();
        Csv {
            header: records.next().expect("a header row"),
            rows: records.collect(),
        }
    }

    /// One column of the report, row by row.
    fn column(&self, column: &str) -> Vec<&str> {
        (0..self.rows.len())
            .map(|row| self.cell(row, column))
            .collect()
    }

    /// One column of one row, by column name.
    fn cell(&self, row: usize, column: &str) -> &str {
        let index = self
            .header
            .iter()
            .position(|name| name == column)
            .unwrap_or_else(|| panic!("the CSV has a {column} column: {:?}", self.header));
        &self.rows[row][index]
    }
}

/// The text as records of cells: `"` toggles quoting, `""` inside a quoted cell
/// is one quote, and `\n` ends a record only outside quotes.
fn records(text: &str) -> Vec<Vec<String>> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '"' if quoted && characters.peek() == Some(&'"') => {
                cell.push('"');
                characters.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => record.push(std::mem::take(&mut cell)),
            '\n' if !quoted => {
                record.push(std::mem::take(&mut cell));
                records.push(std::mem::take(&mut record));
            }
            '\r' if !quoted => {}
            other => cell.push(other),
        }
    }
    records
}

#[test]
fn the_retry_schedule_is_the_specs_three_waits() {
    // Pinned so a change to the schedule is a deliberate edit: the spec gives
    // three waits, which is three retries after the first attempt.
    assert_eq!(
        RetryPolicy::default().backoff,
        [
            Duration::from_millis(500),
            Duration::from_secs(1),
            Duration::from_secs(2),
        ]
    );
}

#[test]
fn a_timeout_is_retried_and_then_recorded() {
    let shoot = shoot(2);
    let csv = shoot.path().join("scores.csv");
    let decider = Scripted::answering(vec![
        Err(failure(ErrorKind::Timeout, "POST /v1/systemone timed out")),
        Err(failure(ErrorKind::Timeout, "POST /v1/systemone timed out")),
        Err(failure(ErrorKind::Timeout, "POST /v1/systemone timed out")),
        Err(failure(ErrorKind::Timeout, "POST /v1/systemone timed out")),
        Ok(document(5.0, "3")),
    ]);

    let run = scoring::run(
        shoot.path(),
        &config(csv.clone()),
        &decider,
        &Silent,
        &Cancel::new(),
    )
    .expect("a run");

    let attempts: Vec<String> = decider
        .seen()
        .iter()
        .map(|seen| seen.state.filename.clone())
        .collect();
    assert_eq!(
        attempts,
        [
            "DSC_0001.NEF",
            "DSC_0001.NEF",
            "DSC_0001.NEF",
            "DSC_0001.NEF",
            "DSC_0002.NEF"
        ],
        "a timeout costs four attempts, and the next frame is a fresh start"
    );
    assert_eq!(run.scored(), 1, "the run carried on after the timeout");
    let csv = Csv::read(&csv);
    assert_eq!(csv.cell(0, "status"), "timeout");
    assert!(
        csv.cell(0, "error").ends_with("4 attempts"),
        "the row says the retries happened: {}",
        csv.cell(0, "error")
    );
    assert_eq!(csv.cell(0, "overall"), "", "a failed row has no score");
}

#[test]
fn a_5xx_that_never_clears_is_recorded_after_its_retries() {
    let shoot = shoot(2);
    let csv = shoot.path().join("scores.csv");
    let hiccup = || failure(ErrorKind::Http5xx, "POST /v1/systemone returned HTTP 503");
    let decider = Scripted::answering(vec![
        Err(hiccup()),
        Err(hiccup()),
        Err(hiccup()),
        Err(hiccup()),
        Ok(document(6.0, "4")),
    ]);

    let run = scoring::run(
        shoot.path(),
        &config(csv.clone()),
        &decider,
        &Silent,
        &Cancel::new(),
    )
    .expect("a run");

    assert_eq!(decider.seen().len(), 5);
    assert_eq!(run.scored(), 1);
    let csv = Csv::read(&csv);
    assert_eq!(csv.cell(0, "status"), "http_5xx");
    assert!(
        csv.cell(0, "error").contains("503"),
        "{}",
        csv.cell(0, "error")
    );
    assert_eq!(csv.cell(1, "status"), "ok");
}

#[test]
fn a_4xx_is_recorded_without_being_retried() {
    let shoot = shoot(2);
    let csv = shoot.path().join("scores.csv");
    let decider = Scripted::answering(vec![
        Err(failure(
            ErrorKind::Http413,
            "POST /v1/systemone returned HTTP 413 — lower --max-edge",
        )),
        Ok(document(6.0, "4")),
    ]);

    let run = scoring::run(
        shoot.path(),
        &config(csv.clone()),
        &decider,
        &Silent,
        &Cancel::new(),
    )
    .expect("a run");

    assert_eq!(
        decider.seen().len(),
        2,
        "a 413 is a 4xx: one attempt, then the next frame"
    );
    assert_eq!(run.scored(), 1);
    let csv = Csv::read(&csv);
    assert_eq!(csv.cell(0, "status"), "http_413");
    assert!(
        csv.cell(0, "error").contains("--max-edge"),
        "the row names the remedy: {}",
        csv.cell(0, "error")
    );
    assert!(
        !csv.cell(0, "error").contains("attempts"),
        "nothing was retried, so nothing says otherwise"
    );
}

#[test]
fn a_bad_response_is_recorded_and_the_run_continues() {
    let shoot = shoot(2);
    let csv = shoot.path().join("scores.csv");
    let decider = Scripted::answering(vec![
        Ok(json!({"answers": {"overall": {"score": 6.0}}})),
        Ok(document(6.0, "4")),
    ]);

    let run = scoring::run(
        shoot.path(),
        &config(csv.clone()),
        &decider,
        &Silent,
        &Cancel::new(),
    )
    .expect("a run");

    assert_eq!(
        decider.seen().len(),
        2,
        "an unreadable answer is not a hiccup"
    );
    assert_eq!(run.scored(), 1);
    assert_eq!(Csv::read(&csv).cell(0, "status"), "bad_response");
}

#[test]
fn a_rejected_rubric_stops_the_run_at_the_first_frame() {
    let shoot = shoot(3);
    let csv = shoot.path().join("scores.csv");
    let decider = Scripted::answering(vec![Err(failure(
        ErrorKind::Http400,
        "POST /v1/systemone returned HTTP 400: invalid criteria for 'overall'",
    ))]);

    let run = scoring::run(
        shoot.path(),
        &config(csv.clone()),
        &decider,
        &Silent,
        &Cancel::new(),
    )
    .expect("a run");

    assert_eq!(
        decider.seen().len(),
        1,
        "a wrong rubric fails identically 400 times: stop at the first"
    );
    assert_eq!(run.frames.len(), 1);
    match &run.ending {
        Ending::Aborted { reason } => assert!(
            reason.contains("HTTP 400") && reason.contains("invalid criteria"),
            "the run says why it stopped: {reason}"
        ),
        other => panic!("a rejected rubric aborts the run, got {other:?}"),
    }
    assert_eq!(run.exit_code(), ExitCode::Fatal);
    let csv = Csv::read(&csv);
    assert_eq!(csv.rows.len(), 1, "the frame that stopped the run is a row");
    assert_eq!(csv.cell(0, "status"), "http_400");
}

#[test]
fn a_malformed_body_stops_the_run_too() {
    let shoot = shoot(2);
    let csv = shoot.path().join("scores.csv");
    let decider = Scripted::answering(vec![Err(failure(
        ErrorKind::Http422,
        "POST /v1/systemone returned HTTP 422: state must not be null",
    ))]);

    let run = scoring::run(
        shoot.path(),
        &config(csv.clone()),
        &decider,
        &Silent,
        &Cancel::new(),
    )
    .expect("a run");

    assert_eq!(decider.seen().len(), 1, "a client bug is not retried");
    assert_eq!(run.exit_code(), ExitCode::Fatal);
    assert_eq!(Csv::read(&csv).cell(0, "status"), "http_422");
}

#[test]
fn an_unreachable_server_stops_the_run_after_its_retries() {
    let shoot = shoot(3);
    let csv = shoot.path().join("scores.csv");
    let unreachable = || {
        failure(
            ErrorKind::ServerUnreachable,
            "cannot reach http://127.0.0.1:8000: Connection refused",
        )
    };
    let decider = Scripted::answering(vec![
        Err(unreachable()),
        Err(unreachable()),
        Err(unreachable()),
        Err(unreachable()),
    ]);

    let run = scoring::run(
        shoot.path(),
        &config(csv.clone()),
        &decider,
        &Silent,
        &Cancel::new(),
    )
    .expect("a run");

    assert_eq!(
        decider.seen().len(),
        4,
        "four attempts at the first frame, then no more requests"
    );
    assert_eq!(
        run.frames.len(),
        1,
        "the other two frames were never judged"
    );
    assert!(matches!(run.ending, Ending::Aborted { .. }));
    assert_eq!(run.exit_code(), ExitCode::Fatal);
    let csv = Csv::read(&csv);
    assert_eq!(csv.cell(0, "status"), "server_unreachable");
    assert!(
        csv.cell(0, "error").ends_with("4 attempts"),
        "{}",
        csv.cell(0, "error")
    );
}

#[test]
fn the_exit_code_is_a_function_of_the_run() {
    let shoot = shoot(2);
    let decider = Scripted::answering(vec![
        Ok(document(6.0, "4")),
        Err(failure(
            ErrorKind::NoPreview,
            "DSC_0002.NEF embeds no JPEG preview",
        )),
    ]);
    let completed = scoring::run(
        shoot.path(),
        &config(shoot.path().join("a.csv")),
        &decider,
        &Silent,
        &Cancel::new(),
    )
    .expect("a run");
    assert_eq!(completed.ending, Ending::Completed);
    assert_eq!(
        completed.exit_code(),
        ExitCode::FailedRows,
        "a completed run with a failed row is not a failure of the run"
    );

    let mut all_ok = scoring::run(
        shoot.path(),
        &config(shoot.path().join("b.csv")),
        &Scripted::always_reading(2),
        &Silent,
        &Cancel::new(),
    )
    .expect("a run");
    assert_eq!(all_ok.exit_code(), ExitCode::Ok);

    // The other two endings cannot be produced from this folder, so they are
    // built: the classification is a pure function of the run, and that is what
    // the process exits with.
    all_ok.ending = Ending::Aborted {
        reason: "POST /v1/systemone returned HTTP 400".into(),
    };
    assert_eq!(all_ok.exit_code(), ExitCode::Fatal);
    all_ok.ending = Ending::Interrupted;
    assert_eq!(
        all_ok.exit_code(),
        ExitCode::Interrupted,
        "an interrupted run is not a failure, however many rows it left"
    );
    assert_eq!(ExitCode::Interrupted.code(), 3);
}

#[test]
fn every_status_an_answer_can_carry_reaches_the_csv() {
    let cases: [(ErrorKind, &str, &str); 7] = [
        (
            ErrorKind::Http400,
            "POST /v1/systemone returned HTTP 400: invalid criteria for 'overall'",
            "invalid criteria",
        ),
        (
            ErrorKind::Http413,
            "POST /v1/systemone returned HTTP 413: context too long — lower --max-edge",
            "--max-edge",
        ),
        (
            ErrorKind::Http422,
            "POST /v1/systemone returned HTTP 422: state must not be null",
            "state must not be null",
        ),
        (
            ErrorKind::Http5xx,
            "POST /v1/systemone returned HTTP 503: models reloading",
            "HTTP 503",
        ),
        (
            ErrorKind::ServerUnreachable,
            "cannot reach http://127.0.0.1:8000: Connection refused",
            "Connection refused",
        ),
        (
            ErrorKind::Timeout,
            "POST /v1/systemone at http://127.0.0.1:8000 timed out after 120s",
            "timed out",
        ),
        (
            ErrorKind::BadResponse,
            "the answer document has no `answers` object",
            "`answers`",
        ),
    ];

    for (kind, message, needle) in cases {
        let shoot = shoot(1);
        let csv = shoot.path().join("scores.csv");
        let answers: Vec<Result<serde_json::Value>> =
            (0..4).map(|_| Err(failure(kind, message))).collect();
        let run = scoring::run(
            shoot.path(),
            &config(csv.clone()),
            &Scripted::answering(answers),
            &Silent,
            &Cancel::new(),
        )
        .expect("a run");

        let csv = Csv::read(&csv);
        assert_eq!(
            csv.cell(0, "status"),
            kind.as_str(),
            "the cause names the status"
        );
        let expected = if kind.recovery().is_retried() {
            format!("{message} — 4 attempts")
        } else {
            message.to_string()
        };
        assert_eq!(
            csv.cell(0, "error"),
            expected,
            "the row is the message, once"
        );
        assert!(
            csv.cell(0, "error").contains(needle) && csv.cell(0, "error").len() < 200,
            "a failed row explains itself without a log: {}",
            csv.cell(0, "error")
        );
        assert_eq!(run.frames.len(), 1);
    }
}

#[test]
fn every_status_a_file_can_carry_reaches_the_csv() {
    let shoot = support::shoot(&[
        ("garbage.NEF", b"this is not a RAW file".to_vec()),
        ("previewless.NEF", support::nef(1, &[])),
        ("broken.NEF", support::nef(1, &[truncated_jpeg()])),
        ("fine.NEF", support::nef(1, &[support::jpeg(48, 32, 80)])),
    ]);
    let csv = shoot.path().join("scores.csv");

    scoring::run(
        shoot.path(),
        &config(csv.clone()),
        &Scripted::always_reading(1),
        &Silent,
        &Cancel::new(),
    )
    .expect("a run");

    let csv = Csv::read(&csv);
    let mut statuses: Vec<(&str, &str)> = (0..csv.rows.len())
        .map(|row| (csv.cell(row, "file"), csv.cell(row, "status")))
        .collect();
    statuses.sort_unstable();
    assert_eq!(
        statuses,
        [
            ("broken.NEF", "preview_broken"),
            ("fine.NEF", "ok"),
            ("garbage.NEF", "unreadable"),
            ("previewless.NEF", "no_preview"),
        ],
        "each file failure lands in the taxonomy, and the good file still scores"
    );
    for (file, status) in statuses {
        let row = (0..csv.rows.len())
            .find(|row| csv.cell(*row, "file") == file)
            .expect("the file has a row");
        if status != "ok" {
            assert!(
                csv.cell(row, "error").len() > 10 && csv.cell(row, "error").len() < 200,
                "{file}: {}",
                csv.cell(row, "error")
            );
        }
    }
}

#[test]
fn a_preflight_failure_stops_the_run_before_the_first_frame() {
    let shoot = shoot(3);
    let csv = shoot.path().join("scores.csv");
    let decider = Scripted::refusing_preflight(Error::fatal(
        "cannot reach http://127.0.0.1:8000: Connection refused — start oMLX (omlx serve)",
    ));

    let error = scoring::run(
        shoot.path(),
        &config(csv.clone()),
        &decider,
        &Silent,
        &Cancel::new(),
    )
    .expect_err("a broken server costs seconds, not 400 frames");

    assert!(error.is_fatal());
    assert!(
        error.message().contains("start oMLX"),
        "the message says what to do: {}",
        error.message()
    );
    assert!(
        decider.seen().is_empty(),
        "the run stops before it judges anything"
    );
    assert!(
        !csv.exists(),
        "no report is written for a run that never started"
    );
}

#[test]
fn a_run_cancelled_between_files_keeps_the_rows_it_judged() {
    let shoot = shoot(3);
    let csv = shoot.path().join("scores.csv");
    let cancel = Arc::new(Cancel::new());
    let decider = Scripted::always_reading(3).cancelling_after(2, Arc::clone(&cancel));

    let run = scoring::run(
        shoot.path(),
        &config(csv.clone()),
        &decider,
        &Silent,
        &cancel,
    )
    .expect("a run");

    assert_eq!(
        decider.seen().len(),
        2,
        "the flag is checked between files, so the third frame is never prepared"
    );
    assert_eq!(run.frames.len(), 2);
    assert_eq!(run.ending, Ending::Interrupted);
    assert_eq!(run.exit_code(), ExitCode::Interrupted);
    let csv = Csv::read(&csv);
    assert_eq!(
        csv.rows.len(),
        2,
        "the rows judged before the cancel are on disk"
    );
    assert_eq!(csv.column(&csv.header[12]), ["ok", "ok"]);
}

#[test]
fn a_run_cancelled_before_it_starts_writes_an_empty_report() {
    let shoot = shoot(3);
    let csv = shoot.path().join("scores.csv");
    let cancel = Cancel::new();
    cancel.cancel();
    let decider = Scripted::always_reading(0);

    let run = scoring::run(
        shoot.path(),
        &config(csv.clone()),
        &decider,
        &Silent,
        &cancel,
    )
    .expect("a run");

    assert!(decider.seen().is_empty());
    assert!(run.frames.is_empty());
    assert_eq!(run.exit_code(), ExitCode::Interrupted);
    let csv = Csv::read(&csv);
    assert!(csv.rows.is_empty(), "the header alone is still a report");
    assert_eq!(csv.header.len(), 17);
}

#[test]
fn a_synthetic_400_file_shoot_gets_a_status_on_every_row() {
    let shoot = shoot(400);
    let csv = shoot.path().join("scores.csv");
    let answers = (0..400)
        .map(|index| {
            if index % 50 == 0 {
                Ok(json!({"answers": {"overall": {"score": 6.0}}}))
            } else {
                Ok(document(6.4237, "4"))
            }
        })
        .collect();
    let decider = Scripted::answering(answers);

    let run = scoring::run(
        shoot.path(),
        &config(csv.clone()),
        &decider,
        &Silent,
        &Cancel::new(),
    )
    .expect("a run");

    assert_eq!(run.frames.len(), 400);
    let csv = Csv::read(&csv);
    assert_eq!(csv.rows.len(), 400, "one row per file, none missing");
    let vocabulary: Vec<&str> = ErrorKind::ALL.iter().map(|kind| kind.as_str()).collect();
    for row in 0..csv.rows.len() {
        let status = csv.cell(row, "status");
        assert!(
            status == "ok" || vocabulary.contains(&status),
            "row {row} has no status the taxonomy names: {status:?}"
        );
    }
    assert!(csv.column("status").contains(&"bad_response"));
}

#[test]
fn a_run_writes_nothing_into_the_shoot_folder() {
    let shoot = shoot(3);
    let before = mtimes(shoot.path());
    let report = tempfile::tempdir().expect("a temp dir");

    scoring::run(
        shoot.path(),
        &config(report.path().join("scores.csv")),
        &Scripted::always_reading(3),
        &Silent,
        &Cancel::new(),
    )
    .expect("a run");

    assert_eq!(
        mtimes(shoot.path()),
        before,
        "spec §43 allows the CSV in the shoot folder only because the photographer asked for it: \
         a run that is not asked to write there touches nothing"
    );
}

/// A preview whose header parses — so it is counted as a preview, with a size —
/// but whose pixels do not decode.
fn truncated_jpeg() -> Vec<u8> {
    let mut jpeg = support::jpeg(64, 48, 80);
    jpeg.truncate(600);
    jpeg
}

/// The mtimes (and sizes) of every file in `dir`, for the acceptance test that
/// pins "a run never writes into the shoot folder".
fn mtimes(dir: &Path) -> BTreeMap<PathBuf, (SystemTime, u64)> {
    fs::read_dir(dir)
        .expect("the shoot folder")
        .map(|entry| {
            let path = entry.expect("a directory entry").path();
            let metadata = fs::metadata(&path).expect("metadata");
            (path, (metadata.modified().expect("mtime"), metadata.len()))
        })
        .collect()
}
