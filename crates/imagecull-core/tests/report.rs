//! The report: the CSV the photographer opens in a spreadsheet, and the raw
//! answers kept beside it.
//!
//! Everything here is asserted at the entry point the spec names — a real
//! folder of synthetic NEFs, a scripted [`Decider`], and the files the run
//! leaves behind — so column order, number formatting, escaping, empty cells
//! and path spelling are pinned as the deliverable rather than as internals.
//!
//! [`Decider`]: imagecull_core::scoring::Decider

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use imagecull_core::ingest::ScanOptions;
use imagecull_core::preprocess::Params;
use imagecull_core::report::Outputs;
use imagecull_core::rubric::Rubric;
use imagecull_core::scoring::{self, Cancel, Config};
use serde_json::json;
use support::decider::{Scripted, document};
use tempfile::TempDir;

/// A shoot folder and the folder its report goes in, both temporary.
///
/// The report never goes inside the shoot: a run must not write into the folder
/// it scores, so every test here writes to a second temp dir on purpose.
struct Shoot {
    dir: TempDir,
    out: TempDir,
}

impl Shoot {
    fn new(files: &[(&str, Vec<u8>)]) -> Shoot {
        let dir = tempfile::tempdir().expect("a temp dir");
        for (name, bytes) in files {
            let path = dir.path().join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create a subfolder");
            }
            fs::write(path, bytes).expect("write fixture");
        }
        Shoot {
            dir,
            out: tempfile::tempdir().expect("a temp dir"),
        }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn csv_path(&self) -> PathBuf {
        self.out.path().join("scores.csv")
    }

    /// The config of a run over this folder: the embedded rubric, the default
    /// preview parameters, and the report in `out`.
    fn config(&self, scan: ScanOptions) -> Config {
        Config::new(
            Rubric::embedded().expect("the embedded rubric parses"),
            None,
            Params::default(),
            scan,
            None,
            Outputs {
                csv: self.csv_path(),
                json_scores: None,
                absolute: false,
            },
        )
    }

    /// The CSV the run wrote, as text.
    fn csv(&self) -> String {
        fs::read_to_string(self.csv_path()).expect("the run writes its CSV")
    }

    /// The `file` cells of the CSV, header excluded, for the tests whose names
    /// hold no comma.
    fn files(&self) -> Vec<String> {
        self.csv()
            .lines()
            .skip(1)
            .map(|line| {
                line.split(',')
                    .next()
                    .expect("every line has a first cell")
                    .to_string()
            })
            .collect()
    }
}

/// The header the spec's column table spells, in its order.
const HEADER: &str = "file,overall,technical,composition,impact,stars,verdict,conf_overall,\
conf_stars,conf_verdict,latency_ms,input_tokens,status,error,duplicate_of,rubric_hash,model\n";

fn nef(frame: Vec<u8>) -> Vec<u8> {
    support::nef(1, &[frame])
}

/// The short rubric hash every row of a default run carries, so the golden text
/// below pins the columns rather than the hash itself.
fn short_hash() -> String {
    Rubric::embedded()
        .expect("the embedded rubric parses")
        .hash("clef-flash-oQ4e")
        .short()
        .to_string()
}

#[test]
fn a_folder_with_no_raw_files_still_gets_the_header_row() {
    let shoot = Shoot::new(&[("notes.txt", b"nothing to score".to_vec())]);
    let decider = Scripted::answering(vec![]);

    scoring::run(
        shoot.path(),
        &shoot.config(ScanOptions::default()),
        &decider,
        &Cancel::new(),
    )
    .expect("an empty run");

    assert_eq!(shoot.csv(), HEADER);
}

#[test]
fn a_scored_frame_is_one_row_with_its_answers_at_full_precision() {
    let shoot = Shoot::new(&[("DSC_0001.NEF", nef(support::jpeg(64, 32, 80)))]);
    let decider = Scripted::answering(vec![Ok(document(6.4237, "4"))]);

    scoring::run(
        shoot.path(),
        &shoot.config(ScanOptions::default()),
        &decider,
        &Cancel::new(),
    )
    .expect("a run");

    assert_eq!(
        shoot.csv(),
        format!(
            "{HEADER}DSC_0001.NEF,6.4237,7.8125,5.3398,4.9601,4,like,0.41,0.63,0.88,812,1327,ok,,,{},clef-flash-oQ4e\n",
            short_hash()
        )
    );
}

#[test]
fn a_failed_row_carries_its_status_and_message_and_leaves_the_value_cells_empty() {
    let shoot = Shoot::new(&[
        ("DSC_0001.NEF", nef(support::jpeg(64, 32, 80))),
        // A NEF-shaped file the camera would never write: no embedded JPEG.
        ("DSC_0002.NEF", support::nef(1, &[])),
        ("DSC_0003.NEF", nef(support::jpeg(48, 24, 80))),
    ]);
    // The second document arrived but cannot be read: its `overall` answer has
    // a score and no confidence.
    let unreadable = json!({
        "answers": {"overall": {"score": 6.0}},
        "usage": {"input_tokens": 987}
    });
    let decider = Scripted::answering(vec![Ok(document(6.4237, "4")), Ok(unreadable)]).taking(654);

    scoring::run(
        shoot.path(),
        &shoot.config(ScanOptions::default()),
        &decider,
        &Cancel::new(),
    )
    .expect("a run");

    assert_eq!(
        shoot.csv(),
        format!(
            "{HEADER}DSC_0001.NEF,6.4237,7.8125,5.3398,4.9601,4,like,0.41,0.63,0.88,654,1327,ok,,,{},clef-flash-oQ4e\nDSC_0002.NEF,,,,,,,,,,,,no_preview,DSC_0002.NEF embeds no JPEG preview,,{},clef-flash-oQ4e\nDSC_0003.NEF,,,,,,,,,,654,987,bad_response,the `overall` answer has no numeric `confidence`,,{},clef-flash-oQ4e\n",
            short_hash(),
            short_hash(),
            short_hash()
        )
    );
}

#[test]
fn a_filename_with_a_comma_a_quote_a_line_break_or_non_ascii_survives_the_round_trip() {
    let shoot = Shoot::new(&[
        ("Café, \"le pick\".NEF", nef(support::jpeg(64, 32, 80))),
        ("line\nbreak.NEF", nef(support::jpeg(48, 24, 80))),
    ]);
    let decider = Scripted::always_reading(2);

    scoring::run(
        shoot.path(),
        &shoot.config(ScanOptions::default()),
        &decider,
        &Cancel::new(),
    )
    .expect("a run");

    assert_eq!(
        shoot.csv(),
        format!(
            "{HEADER}\"Café, \"\"le pick\"\".NEF\",6.4237,7.8125,5.3398,4.9601,4,like,0.41,0.63,0.88,812,1327,ok,,,{},clef-flash-oQ4e\n\"line\nbreak.NEF\",6.4237,7.8125,5.3398,4.9601,4,like,0.41,0.63,0.88,812,1327,ok,,,{},clef-flash-oQ4e\n",
            short_hash(),
            short_hash()
        )
    );
}

#[test]
fn the_golden_report_pins_the_columns_the_numbers_and_the_failures() {
    let shoot = Shoot::new(&[
        ("DSC_0001.NEF", nef(support::jpeg(64, 32, 80))),
        ("DSC_0002, \"Café\".NEF", nef(support::jpeg(56, 28, 80))),
        // A NEF-shaped file with no embedded JPEG.
        ("DSC_0003.NEF", support::nef(1, &[])),
        ("DSC_0004.NEF", nef(support::jpeg(48, 24, 80))),
        // Not RAW: counted as skipped, never a row.
        ("notes.txt", b"nothing to score".to_vec()),
    ]);
    let second = json!({
        "answers": {
            "overall": {"score": 9.125, "confidence": 0.77},
            "technical": {"score": 8.0, "confidence": 0.5},
            "composition": {"score": 7.5, "confidence": 0.25},
            "impact": {"score": 9.875, "confidence": 0.9},
            "stars": {"choice": "5", "confidence": 0.91},
            "verdict": {"choice": "like", "confidence": 0.66}
        },
        "usage": {"input_tokens": 1402, "latency_ms": 731}
    });
    let unreadable = json!({
        "answers": {"overall": {"score": 6.0}},
        "usage": {"input_tokens": 987}
    });
    let decider = Scripted::answering(vec![Ok(document(6.4237, "4")), Ok(second), Ok(unreadable)])
        .taking(654);

    scoring::run(
        shoot.path(),
        &shoot.config(ScanOptions::default()),
        &decider,
        &Cancel::new(),
    )
    .expect("a run");

    assert_eq!(
        shoot.csv(),
        format!(
            "{HEADER}DSC_0001.NEF,6.4237,7.8125,5.3398,4.9601,4,like,0.41,0.63,0.88,654,1327,ok,,,{},clef-flash-oQ4e\n\"DSC_0002, \"\"Café\"\".NEF\",9.125,8,7.5,9.875,5,like,0.77,0.91,0.66,731,1402,ok,,,{},clef-flash-oQ4e\nDSC_0003.NEF,,,,,,,,,,,,no_preview,DSC_0003.NEF embeds no JPEG preview,,{},clef-flash-oQ4e\nDSC_0004.NEF,,,,,,,,,,654,987,bad_response,the `overall` answer has no numeric `confidence`,,{},clef-flash-oQ4e\n",
            short_hash(),
            short_hash(),
            short_hash(),
            short_hash()
        )
    );
}

#[test]
fn the_file_column_is_relative_to_the_shoot_folder_unless_absolute_is_asked_for() {
    let shoot = Shoot::new(&[
        ("DSC_0001.NEF", nef(support::jpeg(64, 32, 80))),
        ("sub/DSC_0002.NEF", nef(support::jpeg(48, 24, 80))),
    ]);
    let scan = ScanOptions {
        recursive: true,
        limit: None,
    };
    let decider = Scripted::always_reading(2);

    scoring::run(
        shoot.path(),
        &shoot.config(scan.clone()),
        &decider,
        &Cancel::new(),
    )
    .expect("a run");
    assert_eq!(shoot.files(), ["DSC_0001.NEF", "sub/DSC_0002.NEF"]);

    let mut config = shoot.config(scan);
    config.outputs.absolute = true;
    let decider = Scripted::always_reading(2);
    scoring::run(shoot.path(), &config, &decider, &Cancel::new()).expect("a run");
    assert_eq!(
        shoot.files(),
        [
            shoot
                .path()
                .join("DSC_0001.NEF")
                .to_string_lossy()
                .into_owned(),
            shoot
                .path()
                .join("sub/DSC_0002.NEF")
                .to_string_lossy()
                .into_owned(),
        ],
        "the same frame, spelled from the root it sits under"
    );
}

#[test]
fn limit_reports_only_the_first_files_of_the_run() {
    let shoot = Shoot::new(&[
        ("DSC_0001.NEF", nef(support::jpeg(64, 32, 80))),
        ("DSC_0002.NEF", nef(support::jpeg(60, 30, 80))),
        ("DSC_0003.NEF", nef(support::jpeg(56, 28, 80))),
    ]);
    let scan = ScanOptions {
        recursive: false,
        limit: Some(2),
    };
    let decider = Scripted::always_reading(2);

    scoring::run(shoot.path(), &shoot.config(scan), &decider, &Cancel::new()).expect("a run");

    assert_eq!(shoot.files(), ["DSC_0001.NEF", "DSC_0002.NEF"]);
    assert_eq!(
        decider.seen().len(),
        2,
        "the frame that was left out was never asked about"
    );
}

#[test]
fn json_scores_keeps_the_raw_answer_document_of_every_frame() {
    let shoot = Shoot::new(&[
        ("DSC_0001.NEF", nef(support::jpeg(64, 32, 80))),
        ("DSC_0002.NEF", support::nef(1, &[])),
        ("DSC_0003.NEF", nef(support::jpeg(48, 24, 80))),
    ]);
    let unreadable = json!({
        "answers": {"overall": {"score": 6.0}},
        "usage": {"input_tokens": 987}
    });
    let decider = Scripted::answering(vec![Ok(document(6.4237, "4")), Ok(unreadable.clone())]);
    let mut config = shoot.config(ScanOptions::default());
    let path = shoot.out.path().join("scores.json");
    config.outputs.json_scores = Some(path.clone());

    scoring::run(shoot.path(), &config, &decider, &Cancel::new()).expect("a run");

    let raw = fs::read_to_string(&path).expect("--json-scores writes its file");
    let documents: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON");
    assert_eq!(
        documents["DSC_0001.NEF"],
        document(6.4237, "4"),
        "the whole document the server sent, not a summary of it"
    );
    assert_eq!(
        documents["DSC_0003.NEF"], unreadable,
        "a bad_response row keeps the document its scores could not be read from"
    );
    assert_eq!(
        documents["DSC_0002.NEF"],
        serde_json::Value::Null,
        "a frame the model never answered says so instead of vanishing"
    );
}

#[test]
fn a_run_writes_the_csv_with_its_ledger_beside_it_and_nothing_else() {
    let shoot = Shoot::new(&[("DSC_0001.NEF", nef(support::jpeg(64, 32, 80)))]);
    let decider = Scripted::always_reading(1);

    scoring::run(
        shoot.path(),
        &shoot.config(ScanOptions::default()),
        &decider,
        &Cancel::new(),
    )
    .expect("a run");

    let mut written: Vec<String> = fs::read_dir(shoot.out.path())
        .expect("the report folder")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    written.sort();
    assert_eq!(
        written,
        ["scores.csv", "scores.ledger.jsonl"],
        "the CSV, and the ledger a resume reads: nothing else"
    );
}
