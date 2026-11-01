//! The ledger: the file that lets a run be interrupted, and the burst dedupe
//! that keeps a run from asking the same question twice.
//!
//! Everything here enters through the scoring entry point and is asserted on
//! what the run left behind — the CSV, the JSONL ledger beside it, the
//! `duplicate_of` column — and on the requests the scripted [`Decider`] saw,
//! which is the only honest measure of "this frame was not scored again".
//!
//! [`Decider`]: imagecull_core::scoring::Decider

mod support;

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use imagecull_core::Result;
use imagecull_core::ingest::ScanOptions;
use imagecull_core::ledger::{LedgerMode, VERSION};
use imagecull_core::preprocess::Params;
use imagecull_core::progress::Silent;
use imagecull_core::report::Outputs;
use imagecull_core::rubric::Rubric;
use imagecull_core::scoring::{
    self, Cancel, Config, DEFAULT_CSV_REWRITE_EVERY, Decider, Ending, Request, Response,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use support::decider::{Scripted, document};
use tempfile::TempDir;

/// A shoot of `count` synthetic NEFs, each with its own preview, so nothing
/// deduplicates unless a test means it to.
fn shoot(count: usize) -> TempDir {
    let files: Vec<(String, Vec<u8>)> = (1..=count)
        .map(|index| {
            let name = format!("DSC_{index:04}.NEF");
            let preview = support::jpeg(48 + 8 * index as u32, 32, 80);
            (name, support::nef(1, &[preview]))
        })
        .collect();
    let borrowed: Vec<(&str, Vec<u8>)> = files
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.clone()))
        .collect();
    support::shoot(&borrowed)
}

/// A shoot of two frames whose previews are byte-identical, and one that is not:
/// a burst, as the camera writes it.
fn burst() -> TempDir {
    let same = support::jpeg(640, 480, 80);
    let other = support::jpeg(320, 240, 80);
    support::shoot(&[
        ("DSC_0001.NEF", support::nef(1, std::slice::from_ref(&same))),
        ("DSC_0002.NEF", support::nef(1, &[other])),
        ("DSC_0003.NEF", support::nef(1, &[same])),
    ])
}

/// A folder a report goes in, so a run never writes into the folder it scores.
fn out() -> TempDir {
    tempfile::tempdir().expect("a temp dir")
}

fn csv_in(out: &Path) -> PathBuf {
    out.join("scores.csv")
}

fn ledger_in(out: &Path) -> PathBuf {
    out.join("scores.ledger.jsonl")
}

/// The config of a run whose report lands in `out`: the embedded rubric, the
/// default preview parameters, and a plain `score`'s ledger mode.
fn config(out: &Path) -> Config {
    Config::new(
        Rubric::embedded().expect("the embedded rubric parses"),
        None,
        Params::default(),
        ScanOptions::default(),
        None,
        Outputs {
            csv: csv_in(out),
            json_scores: None,
            absolute: false,
        },
    )
}

/// Run `dir` with `config` and `decider`, nobody at the stop button.
fn run(dir: &Path, config: &Config, decider: &dyn Decider) -> scoring::Run {
    scoring::run(dir, config, decider, &Silent, &Cancel::new()).expect("a run")
}

/// The CSV a run wrote, as text.
fn csv_text(out: &Path) -> String {
    fs::read_to_string(csv_in(out)).expect("the run writes its CSV")
}

/// The CSV as rows of cells, header included.
///
/// The fixtures here have no commas, quotes or line breaks in their names, so a
/// plain split reads them back; RFC 4180 escaping is `tests/report.rs`'s job.
fn csv_rows(out: &Path) -> Vec<Vec<String>> {
    csv_text(out)
        .lines()
        .map(|line| line.split(',').map(str::to_string).collect())
        .collect()
}

/// One column of the report, header excluded.
fn column(out: &Path, name: &str) -> Vec<String> {
    let rows = csv_rows(out);
    let index = rows[0]
        .iter()
        .position(|column| column == name)
        .unwrap_or_else(|| panic!("the CSV has a {name} column"));
    rows[1..].iter().map(|row| row[index].clone()).collect()
}

/// A ledger read back as JSON: its header line, then its records.
struct LedgerText {
    header: Value,
    records: Vec<Value>,
}

/// Read the ledger beside `out`'s CSV, insisting that every line is JSON: the
/// state a healthy ledger is in.
fn ledger(out: &Path) -> LedgerText {
    let text = fs::read_to_string(ledger_in(out)).expect("the run writes a ledger");
    let mut lines = text
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("every ledger line is JSON"));
    LedgerText {
        header: lines.next().expect("a ledger starts with a header"),
        records: lines.collect(),
    }
}

/// A file's size and mtime in nanoseconds, read the way a resume key expects.
fn identity(path: &Path) -> (u64, u64) {
    let metadata = fs::metadata(path).expect("metadata");
    let mtime = metadata
        .modified()
        .expect("mtime")
        .duration_since(UNIX_EPOCH)
        .expect("a time after the epoch");
    (
        metadata.len(),
        u64::try_from(mtime.as_nanos()).expect("nanoseconds fit"),
    )
}

/// The SHA-256 of `bytes`, computed here rather than by the code under test.
fn sha256(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut hex = String::new();
    for byte in Sha256::digest(bytes) {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

#[test]
fn the_ledger_lives_beside_the_csv_and_names_the_run_that_wrote_it() {
    let dir = shoot(1);
    let out = out();
    let mut config = config(out.path());
    config.endpoint = Some("http://127.0.0.1:8000".to_string());
    let decider = Scripted::always_reading(1);

    run(dir.path(), &config, &decider);

    assert_eq!(
        ledger_in(out.path()),
        out.path().join("scores.ledger.jsonl"),
        "scores.csv's ledger is scores.ledger.jsonl"
    );
    let ledger = ledger(out.path());
    assert_eq!(ledger.header["version"], Value::from(VERSION));
    assert_eq!(
        ledger.header["rubric_hash"],
        Value::from(config.rubric_hash().as_str()),
        "the full hash, so a resume compares the whole identity"
    );
    assert_eq!(ledger.header["model"], Value::from("clef-flash-oQ4e"));
    assert_eq!(
        ledger.header["endpoint"],
        Value::from("http://127.0.0.1:8000")
    );
    let started = ledger.header["started"].as_str().expect("a start time");
    assert_eq!(started.len(), "2026-11-08T10:00:00Z".len(), "{started}");
    assert!(started.ends_with('Z'), "UTC, in RFC 3339: {started}");
}

#[test]
fn a_scored_frame_is_recorded_with_its_identity_the_bytes_that_were_sent_and_the_outcome() {
    let dir = shoot(1);
    let out = out();
    let decider = Scripted::always_reading(1);

    run(dir.path(), &config(out.path()), &decider);

    let records = ledger(out.path()).records;
    assert_eq!(records.len(), 1, "one record per frame");
    let record = &records[0];
    let (size, mtime_ns) = identity(&dir.path().join("DSC_0001.NEF"));
    assert_eq!(record["file"], Value::from("DSC_0001.NEF"));
    assert_eq!(record["size"], Value::from(size));
    assert_eq!(record["mtime_ns"], Value::from(mtime_ns));
    assert_eq!(
        record["preview_sha256"],
        Value::from(sha256(&decider.seen()[0].jpeg)),
        "the hash of the bytes that actually went to the model"
    );
    assert_eq!(record["status"], Value::from("ok"));
    assert_eq!(record["answers"]["overall"], Value::from(6.4237));
    assert_eq!(record["answers"]["verdict"], Value::from("like"));
    assert_eq!(record["usage"]["input_tokens"], Value::from(1327));
    assert_eq!(
        record["document"],
        document(6.4237, "4"),
        "the raw answer, kept whole for --json-scores and the v2 sidecar"
    );
    assert_eq!(record["duplicate_of"], Value::Null);
    assert!(
        record["dedupe_key"].is_string(),
        "the key an identical preview would be reused under: {record}"
    );
    let recorded = record["recorded"].as_str().expect("a timestamp");
    assert!(recorded.ends_with('Z'), "UTC, in RFC 3339: {recorded}");
}

#[test]
fn a_frame_the_model_never_answered_is_recorded_with_its_status_and_no_digest() {
    let dir = support::shoot(&[
        ("DSC_0001.NEF", b"this is not a RAW file".to_vec()),
        ("DSC_0002.NEF", support::nef(1, &[])),
    ]);
    let out = out();

    run(
        dir.path(),
        &config(out.path()),
        &Scripted::answering(Vec::new()),
    );

    let records = ledger(out.path()).records;
    assert_eq!(records[0]["status"], Value::from("unreadable"));
    assert_eq!(records[0]["preview_sha256"], Value::Null);
    assert!(
        records[0]["error"]
            .as_str()
            .is_some_and(|message: &str| message.contains("DSC_0001.NEF")),
        "the row explains itself: {records:?}"
    );
    assert_eq!(records[1]["status"], Value::from("no_preview"));
    assert_eq!(records[1]["answers"], Value::Null);
}

#[test]
fn cancelling_mid_run_then_resuming_produces_one_uninterrupted_runs_csv() {
    let dir = shoot(5);
    let whole = out();
    let interrupted = out();

    // The run one sitting would have produced.
    run(
        dir.path(),
        &config(whole.path()),
        &Scripted::always_reading(5),
    );

    // The same shoot, stopped after the second frame (a Ctrl-C, a lunch break).
    let cancel = Arc::new(Cancel::new());
    let stopped = scoring::run(
        dir.path(),
        &config(interrupted.path()),
        &Scripted::always_reading(5).cancelling_after(2, Arc::clone(&cancel)),
        &Silent,
        &cancel,
    )
    .expect("a run");
    assert_eq!(stopped.ending, Ending::Interrupted);
    assert_eq!(stopped.frames.len(), 2);
    assert_eq!(
        ledger(interrupted.path()).records.len(),
        2,
        "the ledger holds what was judged, flushes and all"
    );

    // And picked up again: only the frames it never got to are asked for.
    let mut resumed_config = config(interrupted.path());
    resumed_config.ledger = LedgerMode::Resume;
    let decider = Scripted::always_reading(3);
    let resumed = run(dir.path(), &resumed_config, &decider);

    assert_eq!(decider.seen().len(), 3, "the ledger saved two requests");
    assert_eq!(resumed.resumed, 2);
    assert_eq!(resumed.frames.len(), 5);
    assert_eq!(
        resumed.scored(),
        5,
        "a resumed frame is a judged frame, not a gap"
    );
    assert_eq!(
        csv_text(interrupted.path()),
        csv_text(whole.path()),
        "cancelling and resuming is indistinguishable in the deliverable"
    );
    assert_eq!(
        column(interrupted.path(), "file"),
        [
            "DSC_0001.NEF",
            "DSC_0002.NEF",
            "DSC_0003.NEF",
            "DSC_0004.NEF",
            "DSC_0005.NEF"
        ],
        "one row per file, none repeated"
    );
}

/// The CSV with one column's cells emptied: what two runs still have to agree
/// on once the column that measures the run rather than the model is taken out
/// of the comparison.
///
/// Plain splitting is enough because the fixtures here have no commas, quotes
/// or line breaks in their names; escaping is `tests/report.rs`'s job.
fn without_column(csv: &str, name: &str) -> String {
    let mut rows: Vec<Vec<String>> = csv
        .lines()
        .map(|line| line.split(',').map(str::to_string).collect())
        .collect();
    let index = rows[0]
        .iter()
        .position(|column| column == name)
        .unwrap_or_else(|| panic!("the CSV has a {name} column"));
    for row in &mut rows[1..] {
        row[index].clear();
    }
    rows.iter()
        .map(|row| row.join(","))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn two_full_runs_over_the_same_folder_agree_apart_from_the_latency_they_measured() {
    // The answers are a deterministic function of the bytes, the questions and
    // the weights, so two runs must agree on every cell that reports one. The
    // exception is `latency_ms`: it measures the run, not the model, and no two
    // runs take the same time. The deciders below differ on purpose, so this
    // only passes if `latency_ms` is the one cell that moved.
    let dir = shoot(3);
    let out = out();

    run(
        dir.path(),
        &config(out.path()),
        &Scripted::always_reading(3).taking(812),
    );
    let first = csv_text(out.path());
    let first_latency = column(out.path(), "latency_ms");
    run(
        dir.path(),
        &config(out.path()),
        &Scripted::always_reading(3).taking(437),
    );
    let second = csv_text(out.path());

    assert_eq!(first_latency, ["812", "812", "812"], "the first run's pace");
    assert_eq!(
        column(out.path(), "latency_ms"),
        ["437", "437", "437"],
        "the column keeps reporting the measurement it always did"
    );
    let compared = without_column(&first, "latency_ms");
    assert_eq!(compared.lines().count(), 4, "the header and three rows");
    assert!(
        compared.contains("DSC_0002.NEF,6.4237,7.8125,5.3398,4.9601,4,like"),
        "the cells being compared are still the run's own numbers: {compared}"
    );
    assert_eq!(
        compared,
        without_column(&second, "latency_ms"),
        "every cell that reports a decision is byte-identical across runs"
    );
    assert_eq!(
        ledger(out.path()).records.len(),
        3,
        "a plain score starts the ledger over instead of appending to it"
    );
}

#[test]
fn a_ledger_with_a_truncated_final_line_is_repaired_and_the_run_reads_what_was_whole() {
    let dir = shoot(3);
    let out = out();
    run(
        dir.path(),
        &config(out.path()),
        &Scripted::always_reading(3),
    );
    let whole = csv_text(out.path());

    // A SIGKILL in the middle of the fourth frame's write: half a record, no
    // newline, nothing else.
    let mut ledger_text = fs::read_to_string(ledger_in(out.path())).expect("a ledger");
    ledger_text.push_str("{\"file\":\"DSC_0004.NEF\",\"size\":12");
    fs::write(ledger_in(out.path()), ledger_text).expect("write the damaged ledger");

    let mut config = config(out.path());
    config.ledger = LedgerMode::Resume;
    let decider = Scripted::answering(Vec::new());
    let resumed = run(dir.path(), &config, &decider);

    assert!(
        decider.seen().is_empty(),
        "all three frames were whole in the ledger: nothing to ask"
    );
    assert_eq!(resumed.resumed, 3);
    assert_eq!(csv_text(out.path()), whole);
    let repaired = fs::read_to_string(ledger_in(out.path())).expect("a ledger");
    assert!(
        !repaired.contains("DSC_0004.NEF"),
        "the half-written record is cut off, so the next append lands on clean bytes"
    );
    assert_eq!(
        ledger(out.path()).records.len(),
        3,
        "the last parseable line is kept"
    );
}

#[test]
fn a_frame_whose_bytes_changed_since_it_was_scored_is_scored_again() {
    let dir = shoot(2);
    let out = out();
    run(
        dir.path(),
        &config(out.path()),
        &Scripted::always_reading(2),
    );

    // A re-export: same name, different file inside.
    let reexported = support::nef(1, &[support::jpeg(300, 200, 80)]);
    fs::write(dir.path().join("DSC_0002.NEF"), reexported).expect("re-export");

    let mut config = config(out.path());
    config.ledger = LedgerMode::Resume;
    let decider = Scripted::answering(vec![Ok(document(9.125, "5"))]);
    let resumed = run(dir.path(), &config, &decider);

    assert_eq!(
        resumed.resumed, 1,
        "the untouched frame came from the ledger"
    );
    assert_eq!(
        decider.seen().len(),
        1,
        "the re-exported frame was asked about again"
    );
    assert_eq!(
        decider.seen()[0].state.filename,
        "DSC_0002.NEF",
        "and it was the re-exported one"
    );
    assert_eq!(column(out.path(), "stars"), ["4", "5"]);
}

#[test]
fn a_frame_whose_modification_time_changed_is_scored_again() {
    let dir = shoot(1);
    let out = out();
    run(
        dir.path(),
        &config(out.path()),
        &Scripted::always_reading(1),
    );
    let before = identity(&dir.path().join("DSC_0001.NEF"));
    let bytes = fs::read(dir.path().join("DSC_0001.NEF")).expect("the frame");

    // Identical bytes, different file: the same size, a new mtime.
    std::thread::sleep(Duration::from_millis(20));
    fs::write(dir.path().join("DSC_0001.NEF"), bytes).expect("touch");
    let after = identity(&dir.path().join("DSC_0001.NEF"));
    assert_eq!(before.0, after.0, "the size did not change");
    assert_ne!(
        before.1, after.1,
        "the filesystem must report a new modification time for this test to mean anything"
    );

    let mut config = config(out.path());
    config.ledger = LedgerMode::Resume;
    let decider = Scripted::answering(vec![Ok(document(3.5, "2"))]);
    let resumed = run(dir.path(), &config, &decider);

    assert_eq!(resumed.resumed, 0);
    assert_eq!(
        decider.seen().len(),
        1,
        "the modification time is part of the resume key"
    );
    assert_eq!(column(out.path(), "stars"), ["2"]);
}

#[test]
fn a_ledger_written_under_another_rubric_or_model_is_a_fatal_error() {
    let dir = shoot(1);
    let out = out();
    run(
        dir.path(),
        &config(out.path()),
        &Scripted::always_reading(1),
    );
    let untouched = csv_text(out.path());

    // A different model, in the plain mode and in the resuming one: neither may
    // mix two identities in one CSV.
    for mode in [LedgerMode::Restart, LedgerMode::Resume] {
        let mut config = config(out.path());
        config.ledger = mode;
        config.model = "clef-27b-oQ4e".to_string();
        let error = scoring::run(
            dir.path(),
            &config,
            &Scripted::answering(Vec::new()),
            &Silent,
            &Cancel::new(),
        )
        .expect_err("a ledger from another model cannot be continued");
        assert!(error.is_fatal());
        assert!(
            error.message().contains("--fresh"),
            "the error says how to proceed: {}",
            error.message()
        );
        assert!(
            error.message().contains("clef-flash-oQ4e")
                && error.message().contains("clef-27b-oQ4e"),
            "and which two identities collided: {}",
            error.message()
        );
    }
    assert_eq!(
        csv_text(out.path()),
        untouched,
        "a run that refused to start leaves the report alone"
    );

    // A different rubric, same model: the hash is what changed.
    let mut reworded = config(out.path());
    reworded.rubric = support::rubric("clef-flash-oQ4e", "Rate the frame as a keeper.");
    let error = scoring::run(
        dir.path(),
        &reworded,
        &Scripted::answering(Vec::new()),
        &Silent,
        &Cancel::new(),
    )
    .expect_err("a reworded rubric is a different rubric");
    assert!(error.message().contains("--fresh"), "{}", error.message());

    // Unless the photographer says so.
    reworded.ledger = LedgerMode::Discard;
    let run = run(dir.path(), &reworded, &Scripted::always_reading(1));
    assert_eq!(run.scored(), 1, "--fresh starts over deliberately");
}

#[test]
fn fresh_discards_the_ledger_and_scores_every_frame_again() {
    let dir = shoot(3);
    let out = out();
    run(
        dir.path(),
        &config(out.path()),
        &Scripted::always_reading(3),
    );

    let mut config = config(out.path());
    config.ledger = LedgerMode::Discard;
    let decider = Scripted::always_reading(3);
    let second = run(dir.path(), &config, &decider);

    assert_eq!(decider.seen().len(), 3, "--fresh asks again");
    assert_eq!(second.resumed, 0);
    assert_eq!(
        ledger(out.path()).records.len(),
        3,
        "the old records are gone, not appended to"
    );
}

#[test]
fn a_ledger_written_by_another_version_is_refused() {
    let dir = shoot(1);
    let out = out();
    run(
        dir.path(),
        &config(out.path()),
        &Scripted::always_reading(1),
    );

    let text = fs::read_to_string(ledger_in(out.path())).expect("a ledger");
    let (header, records) = text.split_once('\n').expect("a header line");
    let mut changed: Value = serde_json::from_str(header).expect("a header");
    changed["version"] = Value::from(VERSION + 1);
    fs::write(ledger_in(out.path()), format!("{changed}\n{records}"))
        .expect("write the future ledger");

    let mut config = config(out.path());
    config.ledger = LedgerMode::Resume;
    let error = scoring::run(
        dir.path(),
        &config,
        &Scripted::answering(Vec::new()),
        &Silent,
        &Cancel::new(),
    )
    .expect_err("a ledger from another format cannot be read as this one");
    assert!(
        error.message().contains("version") && error.message().contains("--fresh"),
        "{}",
        error.message()
    );
}

#[test]
fn an_identical_preview_reuses_the_earlier_outcome_and_still_gets_its_own_row() {
    let dir = burst();
    let out = out();
    let decider = Scripted::answering(vec![Ok(document(6.4237, "4")), Ok(document(2.5, "2"))]);

    let run = run(dir.path(), &config(out.path()), &decider);

    assert_eq!(
        decider.seen().len(),
        2,
        "the burst's identical preview cost nothing"
    );
    assert_eq!(run.frames.len(), 3, "every NEF is judged");
    assert_eq!(run.duplicates, 1);
    let rows = csv_rows(out.path());
    assert_eq!(rows.len(), 4, "three files, three rows");
    assert_eq!(
        column(out.path(), "duplicate_of"),
        ["", "", "DSC_0001.NEF"],
        "the third frame names the one that paid for the request"
    );
    for column_name in ["overall", "stars", "verdict", "conf_overall", "latency_ms"] {
        assert_eq!(
            column(out.path(), column_name)[0],
            column(out.path(), column_name)[2],
            "a duplicate carries the first frame's {column_name}"
        );
    }
    assert_eq!(
        column(out.path(), "stars")[1],
        "2",
        "the frame that was not a duplicate got its own answer"
    );
    let records = ledger(out.path()).records;
    assert_eq!(
        records[2]["preview_sha256"], records[0]["preview_sha256"],
        "identical bytes, one digest"
    );
}

#[test]
fn no_dedupe_scores_every_frame_independently() {
    let dir = burst();
    let out = out();
    let mut config = config(out.path());
    config.dedupe = false;
    let decider = Scripted::answering(vec![
        Ok(document(6.4237, "4")),
        Ok(document(2.5, "2")),
        Ok(document(8.25, "5")),
    ]);

    let run = run(dir.path(), &config, &decider);

    assert_eq!(
        decider.seen().len(),
        3,
        "--no-dedupe asks about every frame"
    );
    assert_eq!(run.duplicates, 0);
    assert_eq!(
        column(out.path(), "duplicate_of"),
        ["", "", ""],
        "nothing was reused, so nothing points at another frame"
    );
}

#[test]
fn dedupe_reaches_the_outcomes_a_resumed_ledger_holds() {
    let dir = burst();
    let out = out();

    // The run stops after the frame that paid for the burst's request.
    let cancel = Arc::new(Cancel::new());
    scoring::run(
        dir.path(),
        &config(out.path()),
        &Scripted::answering(vec![Ok(document(6.4237, "4"))])
            .cancelling_after(1, Arc::clone(&cancel)),
        &Silent,
        &cancel,
    )
    .expect("a run");

    // Picked up later: the burst partner reuses the recorded outcome, and only
    // the frame that is not a duplicate costs a request.
    let mut config = config(out.path());
    config.ledger = LedgerMode::Resume;
    let decider = Scripted::answering(vec![Ok(document(2.5, "2"))]);
    let resumed = run(dir.path(), &config, &decider);

    assert_eq!(decider.seen().len(), 1);
    assert_eq!(decider.seen()[0].state.filename, "DSC_0002.NEF");
    assert_eq!(resumed.resumed, 1);
    assert_eq!(resumed.duplicates, 1);
    assert_eq!(column(out.path(), "duplicate_of"), ["", "", "DSC_0001.NEF"]);
}

#[test]
fn a_file_that_is_not_a_ledger_is_refused_rather_than_read() {
    let dir = shoot(1);
    let out = out();
    fs::write(ledger_in(out.path()), "a note to self\n").expect("write something else");

    let mut config = config(out.path());
    config.ledger = LedgerMode::Resume;
    let error = scoring::run(
        dir.path(),
        &config,
        &Scripted::answering(Vec::new()),
        &Silent,
        &Cancel::new(),
    )
    .expect_err("a file that is not a ledger cannot be resumed from");
    assert!(
        error.message().contains("not an imagecull ledger") && error.message().contains("--fresh"),
        "{}",
        error.message()
    );
}

#[test]
fn a_resumed_run_keeps_the_documents_and_the_failures_the_ledger_recorded() {
    let dir = shoot(3);
    let out = out();
    let unreadable = serde_json::json!({"answers": {"overall": {"score": 6.0}}});
    let json = out.path().join("scores.json");
    let mut whole = config(out.path());
    whole.outputs.json_scores = Some(json.clone());
    let decider = Scripted::answering(vec![
        Ok(document(6.4237, "4")),
        Ok(unreadable.clone()),
        Ok(document(2.5, "2")),
    ]);
    run(dir.path(), &whole, &decider);
    let first_csv = csv_text(out.path());
    let first_json = fs::read_to_string(&json).expect("--json-scores writes its file");

    let mut resumed_config = config(out.path());
    resumed_config.outputs.json_scores = Some(json.clone());
    resumed_config.ledger = LedgerMode::Resume;
    let decider = Scripted::answering(Vec::new());
    let resumed = run(dir.path(), &resumed_config, &decider);

    assert!(
        decider.seen().is_empty(),
        "a finished ledger leaves nothing to ask"
    );
    assert_eq!(resumed.resumed, 3);
    assert_eq!(
        column(out.path(), "status"),
        ["ok", "bad_response", "ok"],
        "a failure a resumed run restores is still a failure"
    );
    assert_eq!(csv_text(out.path()), first_csv);
    assert_eq!(
        fs::read_to_string(&json).expect("json scores"),
        first_json,
        "the raw document survives the round trip through the ledger"
    );
}

/// A decider that reads the CSV as it goes, so a test can see how much of the
/// report was on disk at the moment each frame was judged.
struct Watching {
    inner: Scripted,
    csv: PathBuf,
    rows: RefCell<Vec<usize>>,
}

impl Watching {
    fn new(csv: PathBuf, inner: Scripted) -> Watching {
        Watching {
            inner,
            csv,
            rows: RefCell::new(Vec::new()),
        }
    }

    /// How many rows the CSV held at each request, in order.
    fn rows(&self) -> Vec<usize> {
        self.rows.borrow().clone()
    }
}

impl Decider for Watching {
    fn preflight(&self) -> Result<()> {
        self.inner.preflight()
    }

    fn decide(&self, request: &Request<'_>) -> Result<Response> {
        self.rows.borrow_mut().push(rows_on_disk(&self.csv));
        self.inner.decide(request)
    }
}

/// How many rows the CSV at `path` holds, header excluded; none before it exists.
fn rows_on_disk(path: &Path) -> usize {
    fs::read_to_string(path).map_or(0, |text| text.lines().count().saturating_sub(1))
}

#[test]
fn the_csv_is_rewritten_as_the_run_goes_and_again_at_exit() {
    assert_eq!(
        DEFAULT_CSV_REWRITE_EVERY, 25,
        "the spec's rewrite interval (spec §9.3)"
    );
    let dir = shoot(3);

    // The default: nothing on disk until the run ends, so a 26th row is the
    // first one that makes it visible mid-flight.
    let watched = out();
    let watching = Watching::new(csv_in(watched.path()), Scripted::always_reading(3));
    run(dir.path(), &config(watched.path()), &watching);
    assert_eq!(watching.rows(), [0, 0, 0]);
    assert_eq!(csv_rows(watched.path()).len(), 4);

    // Shrunk to every row: each frame's judgement leaves the report one row
    // longer, which is what a killed run has to rely on.
    let eager = out();
    let mut config = config(eager.path());
    config.csv_rewrite_every = 1;
    let watching = Watching::new(csv_in(eager.path()), Scripted::always_reading(3));
    run(dir.path(), &config, &watching);
    assert_eq!(
        watching.rows(),
        [0, 1, 2],
        "the CSV is rewritten from the ledger as the rows arrive"
    );
    assert_eq!(csv_rows(eager.path()).len(), 4);
}

#[test]
fn a_run_that_never_started_writes_no_ledger() {
    let dir = shoot(2);
    let out = out();
    let decider = Scripted::refusing_preflight(imagecull_core::Error::fatal(
        "cannot reach http://127.0.0.1:8000 — start oMLX",
    ));

    let error = scoring::run(
        dir.path(),
        &config(out.path()),
        &decider,
        &Silent,
        &Cancel::new(),
    )
    .expect_err("a broken server costs seconds, not a run");

    assert!(error.is_fatal());
    assert!(
        !ledger_in(out.path()).exists(),
        "a run that never judged a frame leaves no ledger behind"
    );
}
