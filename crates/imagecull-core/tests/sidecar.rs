//! The rating's home: a run over a shoot folder that leaves sidecars, and the
//! rules that decide what it may touch.
//!
//! Everything is asserted at the entry point a photographer uses — a real folder
//! of synthetic NEFs, a scripted [`Decider`], and the files the run leaves beside
//! the raws — so the skip rule, the merge, the backup and the abort are pinned as
//! behaviour rather than as internals.
//!
//! [`Decider`]: imagecull_core::scoring::Decider

mod support;

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use imagecull_core::Error;
use imagecull_core::error::ErrorKind;
use imagecull_core::ingest::ScanOptions;
use imagecull_core::preprocess::Params;
use imagecull_core::progress::{Event, Judgement, Reporter};
use imagecull_core::rubric::Builtin;
use imagecull_core::scoring::{self, Cancel, Config, Ending};
use imagecull_core::sidecar::{self, Outputs, Writer, Written};
use serde_json::json;
use support::decider::{Scripted, document};
use tempfile::TempDir;

/// A shoot folder, and the run over it.
struct Shoot {
    dir: TempDir,
}

impl Shoot {
    fn of(files: &[(&str, Vec<u8>)]) -> Shoot {
        Shoot {
            dir: support::shoot(files),
        }
    }

    /// One frame, with a decoded preview of its own.
    fn one() -> Shoot {
        Shoot::of(&[("DSC_0001.NEF", nef(support::jpeg(64, 32, 80)))])
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    /// The frame's own path.
    fn frame(&self, name: &str) -> PathBuf {
        self.path().join(name)
    }

    /// The sidecar beside a frame.
    fn sidecar(&self, name: &str) -> PathBuf {
        sidecar::path_for(&self.frame(name))
    }

    fn config(&self) -> Config {
        let mut config = Config::new(
            Builtin::General
                .rubric()
                .expect("the default built-in parses"),
            None,
            Params::default(),
            ScanOptions::default(),
            None,
            Outputs::default(),
        );
        config.retry = scoring::RetryPolicy::without_waiting();
        config
    }

    /// The names in the folder, sorted: "nothing else was written" is assertable.
    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.path())
            .expect("read the folder")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    fn sidecar_text(&self, name: &str) -> String {
        fs::read_to_string(self.sidecar(name)).unwrap_or_else(|error| {
            panic!("{} has no sidecar: {error}", name);
        })
    }
}

/// A frame built out of one embedded JPEG.
fn nef(frame: Vec<u8>) -> Vec<u8> {
    support::nef(1, &[frame])
}

/// A sidecar another application would have written: a rating, and content that
/// is not ours.
fn rated(rating: u8) -> String {
    format!(
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n\
         <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
         <rdf:Description rdf:about=\"\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" \
         xmlns:crs=\"http://ns.adobe.com/camera-raw-settings/1.0/\" \
         crs:Exposure2012=\"+0.35\" xmp:Rating=\"{rating}\"/>\n\
         </rdf:RDF>\n\
         </x:xmpmeta>\n"
    )
}

/// The rating in a sidecar, as the reader this tool uses would see it.
fn rating_in(text: &str) -> Option<u8> {
    sidecar::existing_rating(text)
}

/// A reporter that keeps every frame event.
#[derive(Default)]
struct Collected {
    frames: Mutex<Vec<Judgement>>,
}

impl Reporter for Collected {
    fn event(&self, event: Event) {
        if let Event::Frame(frame) = event {
            self.frames.lock().expect("a lock").push(frame.judgement);
        }
    }
}

impl Collected {
    fn judgements(&self) -> Vec<Judgement> {
        self.frames.lock().expect("a lock").clone()
    }
}

/// A reporter that keeps every warning.
#[derive(Default)]
struct Warnings {
    messages: Mutex<Vec<String>>,
}

impl Reporter for Warnings {
    fn event(&self, event: Event) {
        if let Event::Warning(warning) = event {
            self.messages.lock().expect("a lock").push(warning.message);
        }
    }
}

impl Warnings {
    fn messages(&self) -> Vec<String> {
        self.messages.lock().expect("a lock").clone()
    }
}

/// A writer that refuses everything, so the abort path is reachable without
/// fighting the filesystem's permissions.
#[derive(Default)]
struct Refusing {
    asked: RefCell<Vec<PathBuf>>,
}

impl Writer for Refusing {
    fn write(
        &self,
        target: &Path,
        _document: &str,
        _existing: Option<&str>,
    ) -> imagecull_core::Result<Written> {
        self.asked.borrow_mut().push(target.to_path_buf());
        Err(Error::new(
            ErrorKind::XmpWriteFailed,
            "cannot write a sidecar in this folder",
        ))
    }
}

#[test]
fn a_scored_frame_leaves_a_sidecar_beside_it_holding_its_rating_and_its_provenance() {
    let shoot = Shoot::one();
    let decider = Scripted::answering(vec![Ok(document("4"))]);

    let run = scoring::run(
        shoot.path(),
        &shoot.config(),
        &decider,
        &sidecar::Filesystem,
        &imagecull_core::progress::Silent,
        &Cancel::new(),
    )
    .expect("a run");

    assert_eq!(run.scored(), 1);
    assert_eq!(run.kept(), 0);
    let text = shoot.sidecar_text("DSC_0001.NEF");
    assert_eq!(rating_in(&text), Some(4));
    assert!(text.contains("imagecull:confStars=\"0.63\""), "{text}");
    assert!(
        text.contains("imagecull:model=\"clef-flash-oQ4e\""),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "imagecull:rubricHash=\"{}\"",
            run.rubric_hash.short()
        )),
        "the sidecar names the rubric that judged it: {text}"
    );
    assert!(text.contains("imagecull:scoredAt=\""), "{text}");
    assert_eq!(
        shoot.names(),
        ["DSC_0001.NEF", "DSC_0001.xmp"],
        "one sidecar, no backup, nothing else"
    );
}

#[test]
fn a_frame_the_model_never_answered_leaves_no_sidecar_at_all() {
    let shoot = Shoot::of(&[
        ("DSC_0001.NEF", b"this is not a RAW file".to_vec()),
        ("DSC_0002.NEF", nef(support::jpeg(64, 32, 80))),
    ]);
    let decider = Scripted::always_reading(1);

    let run = scoring::run(
        shoot.path(),
        &shoot.config(),
        &decider,
        &sidecar::Filesystem,
        &imagecull_core::progress::Silent,
        &Cancel::new(),
    )
    .expect("a run");

    assert_eq!(run.failed(), 1);
    assert_eq!(
        run.frames[0].failure().expect("a failure").kind,
        ErrorKind::Unreadable
    );
    assert!(
        !shoot.sidecar("DSC_0001.NEF").exists(),
        "no answer must never reach the disk as a rating of zero or an empty file"
    );
    assert!(shoot.sidecar("DSC_0002.NEF").exists());
}

#[test]
fn a_frame_with_a_rating_is_kept_even_when_its_preview_could_not_be_read() {
    let shoot = Shoot::of(&[("DSC_0001.NEF", b"this is not a RAW file".to_vec())]);
    fs::write(shoot.sidecar("DSC_0001.NEF"), rated(3)).expect("a sidecar");

    let run = scoring::run(
        shoot.path(),
        &shoot.config(),
        &Scripted::answering(vec![]),
        &sidecar::Filesystem,
        &imagecull_core::progress::Silent,
        &Cancel::new(),
    )
    .expect("a run");

    assert_eq!(run.kept(), 1, "the skip check comes before any extraction");
    assert_eq!(run.failed(), 0, "a frame that needs nothing cannot fail");
}

#[test]
fn a_second_run_over_the_same_folder_asks_nothing_and_writes_nothing() {
    let shoot = Shoot::one();
    let decider = Scripted::answering(vec![Ok(document("4"))]);
    scoring::run(
        shoot.path(),
        &shoot.config(),
        &decider,
        &sidecar::Filesystem,
        &imagecull_core::progress::Silent,
        &Cancel::new(),
    )
    .expect("a run");
    let first = shoot.sidecar_text("DSC_0001.NEF");

    let reporter = Collected::default();
    let run = scoring::run(
        shoot.path(),
        &shoot.config(),
        &Scripted::answering(vec![]),
        &sidecar::Filesystem,
        &reporter,
        &Cancel::new(),
    )
    .expect("a second run");

    assert_eq!(decider.seen().len(), 1, "the second run asks nothing");
    assert_eq!(shoot.sidecar_text("DSC_0001.NEF"), first, "byte-identical");
    assert_eq!(run.scored(), 0, "nothing was scored this run");
    assert_eq!(run.kept(), 1);
    assert_eq!(reporter.judgements(), [Judgement::Skipped { rating: 4 }]);
    assert_eq!(shoot.names(), ["DSC_0001.NEF", "DSC_0001.xmp"], "no churn");
}

#[test]
fn a_rating_the_photographer_chose_is_left_alone_until_force_is_given() {
    let shoot = Shoot::one();
    fs::write(shoot.sidecar("DSC_0001.NEF"), rated(5)).expect("a sidecar");
    let decider = Scripted::answering(vec![Ok(document("2"))]);

    let mut config = shoot.config();
    config.force = true;
    let run = scoring::run(
        shoot.path(),
        &config,
        &decider,
        &sidecar::Filesystem,
        &imagecull_core::progress::Silent,
        &Cancel::new(),
    )
    .expect("a forced run");

    assert_eq!(run.kept(), 0, "force means rate it again");
    let text = shoot.sidecar_text("DSC_0001.NEF");
    assert_eq!(rating_in(&text), Some(2));
    assert!(
        text.contains("crs:Exposure2012=\"+0.35\""),
        "a forced merge keeps the develop settings: {text}"
    );
    assert_eq!(
        fs::read_to_string(shoot.path().join("DSC_0001.xmp.bak")).expect("a backup"),
        rated(5),
        "what was there before survives in the backup"
    );
}

#[test]
fn a_darktable_sidecar_is_left_alone_unless_foreign_writes_are_allowed() {
    let history = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n\
                   <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
                   <rdf:Description rdf:about=\"\" xmlns:darktable=\"http://darktable.sf.net/\" \
                   darktable:history_end=\"4\"/>\n\
                   </rdf:RDF>\n\
                   </x:xmpmeta>\n";

    let shoot = Shoot::one();
    fs::write(shoot.sidecar("DSC_0001.NEF"), history).expect("a sidecar");
    let decider = Scripted::answering(vec![Ok(document("4"))]);

    let run = scoring::run(
        shoot.path(),
        &shoot.config(),
        &decider,
        &sidecar::Filesystem,
        &imagecull_core::progress::Silent,
        &Cancel::new(),
    )
    .expect("a run");

    assert_eq!(
        run.frames[0].failure().expect("a refusal").kind,
        ErrorKind::XmpForeign
    );
    assert_eq!(shoot.sidecar_text("DSC_0001.NEF"), history, "untouched");
    assert_eq!(decider.seen().len(), 0, "and no request was paid for");

    let mut allowed = shoot.config();
    allowed.allow_foreign = true;
    scoring::run(
        shoot.path(),
        &allowed,
        &Scripted::answering(vec![Ok(document("4"))]),
        &sidecar::Filesystem,
        &imagecull_core::progress::Silent,
        &Cancel::new(),
    )
    .expect("an allowed run");

    let text = shoot.sidecar_text("DSC_0001.NEF");
    assert_eq!(rating_in(&text), Some(4));
    assert!(
        text.contains("darktable:history_end=\"4\""),
        "the other application's history survives: {text}"
    );
}

#[test]
fn a_sidecar_with_nothing_to_write_into_fails_the_frame_before_a_request_is_paid() {
    let shoot = Shoot::one();
    fs::write(
        shoot.sidecar("DSC_0001.NEF"),
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/>",
    )
    .expect("a sidecar");
    let decider = Scripted::answering(vec![Ok(document("4"))]);

    let run = scoring::run(
        shoot.path(),
        &shoot.config(),
        &decider,
        &sidecar::Filesystem,
        &imagecull_core::progress::Silent,
        &Cancel::new(),
    )
    .expect("a run");

    assert_eq!(
        run.frames[0].failure().expect("a refusal").kind,
        ErrorKind::XmpUnreadable
    );
    assert_eq!(
        decider.seen().len(),
        0,
        "no request for a frame we cannot record"
    );
    assert_eq!(
        shoot.sidecar_text("DSC_0001.NEF"),
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/>"
    );
}

#[test]
fn a_sidecar_that_cannot_be_written_stops_the_run_at_that_frame() {
    let shoot = Shoot::of(&[
        ("DSC_0001.NEF", nef(support::jpeg(64, 32, 80))),
        ("DSC_0002.NEF", nef(support::jpeg(48, 24, 80))),
    ]);
    let decider = Scripted::always_reading(2);
    let refusing = Refusing::default();

    let run = scoring::run(
        shoot.path(),
        &shoot.config(),
        &decider,
        &refusing,
        &imagecull_core::progress::Silent,
        &Cancel::new(),
    )
    .expect("a run");

    assert_eq!(
        run.frames.len(),
        1,
        "the run stopped at the frame that failed"
    );
    assert_eq!(
        run.frames[0].failure().expect("a write failure").kind,
        ErrorKind::XmpWriteFailed
    );
    assert_eq!(
        decider.seen().len(),
        1,
        "the second frame was never paid for"
    );
    assert_eq!(
        run.ending,
        Ending::Aborted {
            reason: run.frames[0].failure().expect("a failure").message
        }
    );
    assert!(!shoot.sidecar("DSC_0001.NEF").exists(), "nothing landed");
}

#[test]
fn an_identical_preview_reuses_the_answer_within_one_run_and_only_within_one() {
    let preview = support::jpeg(64, 32, 80);
    let shoot = Shoot::of(&[
        ("DSC_0001.NEF", nef(preview.clone())),
        ("DSC_0002.NEF", nef(preview)),
    ]);
    let decider = Scripted::always_reading(1);

    let run = scoring::run(
        shoot.path(),
        &shoot.config(),
        &decider,
        &sidecar::Filesystem,
        &imagecull_core::progress::Silent,
        &Cancel::new(),
    )
    .expect("a run");

    assert_eq!(decider.seen().len(), 1, "a burst costs one request");
    assert_eq!(run.duplicates, 1);
    assert_eq!(run.scored(), 2, "both frames still get a rating on disk");
    assert!(shoot.sidecar("DSC_0002.NEF").exists());

    // A second run under --force has no memory of the first one's answer: it
    // pays for the pair again. One request, not zero, is what that costs here,
    // because the two frames are still a burst inside the new run.
    let mut config = shoot.config();
    config.force = true;
    let second = Scripted::always_reading(2);
    let run = scoring::run(
        shoot.path(),
        &config,
        &second,
        &sidecar::Filesystem,
        &imagecull_core::progress::Silent,
        &Cancel::new(),
    )
    .expect("a forced run");
    assert_eq!(
        second.seen().len(),
        1,
        "no reuse of the earlier run's answer"
    );
    assert_eq!(run.duplicates, 1, "the pair still dedupes inside that run");
}

#[test]
fn json_scores_keeps_the_raw_answer_document_of_every_frame() {
    let shoot = Shoot::of(&[
        ("DSC_0001.NEF", nef(support::jpeg(64, 32, 80))),
        ("DSC_0002.NEF", support::nef(1, &[])),
        ("DSC_0003.NEF", nef(support::jpeg(48, 24, 80))),
    ]);
    let unreadable = json!({
        "answers": {"stars": {"choice": "4"}},
        "usage": {"input_tokens": 987}
    });
    let decider = Scripted::answering(vec![Ok(document("4")), Ok(unreadable.clone())]);
    let mut config = shoot.config();
    let path = shoot.path().join("answers.json");
    config.outputs.json_scores = Some(path.clone());

    scoring::run(
        shoot.path(),
        &config,
        &decider,
        &sidecar::Filesystem,
        &imagecull_core::progress::Silent,
        &Cancel::new(),
    )
    .expect("a run");

    let raw = fs::read_to_string(&path).expect("--json-scores writes its file");
    let documents: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON");
    assert_eq!(
        documents["DSC_0001.NEF"],
        document("4"),
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
fn a_run_that_rated_nothing_says_so_and_still_succeeds() {
    let shoot = Shoot::one();
    fs::write(shoot.sidecar("DSC_0001.NEF"), rated(4)).expect("a sidecar");
    let reporter = Warnings::default();

    let run = scoring::run(
        shoot.path(),
        &shoot.config(),
        &Scripted::answering(vec![]),
        &sidecar::Filesystem,
        &reporter,
        &Cancel::new(),
    )
    .expect("a run");

    assert_eq!(run.exit_code(), imagecull_core::ExitCode::Ok);
    let warnings = reporter.messages();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("--force"),
        "a run that did nothing has to say how to make it do something: {}",
        warnings[0]
    );
}

#[test]
fn the_summary_says_how_much_of_the_folder_this_run_rated() {
    let shoot = Shoot::of(&[
        ("DSC_0001.NEF", nef(support::jpeg(64, 32, 80))),
        ("DSC_0002.NEF", nef(support::jpeg(48, 24, 80))),
        ("DSC_0003.NEF", nef(support::jpeg(56, 28, 80))),
    ]);
    for name in ["DSC_0001.NEF", "DSC_0002.NEF"] {
        fs::write(shoot.sidecar(name), rated(5)).expect("a sidecar");
    }

    let run = scoring::run(
        shoot.path(),
        &shoot.config(),
        &Scripted::always_reading(1),
        &sidecar::Filesystem,
        &imagecull_core::progress::Silent,
        &Cancel::new(),
    )
    .expect("a run");

    let summary = run.summary();
    assert_eq!(summary.total, 3);
    assert_eq!(summary.scored, 1, "only what this run produced");
    assert_eq!(summary.skipped, 2);
    assert_eq!(
        summary.stars,
        [0, 0, 0, 1, 0],
        "kept ratings are not re-counted"
    );
    assert_eq!(summary.rating.map(|rating| rating.median), Some(4.0));
}
