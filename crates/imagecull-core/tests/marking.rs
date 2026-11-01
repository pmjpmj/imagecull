//! The group's keeper, and the red mark on everyone else: a folder in, sidecars
//! with a verdict out.
//!
//! Every test here runs a real `scoring::run` over a synthetic group — NEF-shaped
//! files with a real capture time, one body, one exposure and one scene — with a
//! scripted [`Decider`] behind the same seam every per-frame request goes through.
//! Nothing here needs a server, and nothing here asserts on an internal helper:
//! the assertions are the sidecars a photographer opens, the events a reporter
//! receives, and the counts the summary prints.
//!
//! [`Decider`]: imagecull_core::scoring::Decider

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use image::RgbImage;
use imagecull_core::ErrorKind;
use imagecull_core::error::Error;
use imagecull_core::ingest::ScanOptions;
use imagecull_core::preprocess::Params;
use imagecull_core::progress::Silent;
use imagecull_core::rubric::Builtin;
use imagecull_core::scoring::{self, Cancel, Config, Groups, Run};
use imagecull_core::sidecar::{self, Outputs, Prior, Properties, Writer, Written};
use support::decider::{Scripted, document, keeper};
use support::scenes;
use support::{ExifFacts, Warnings};
use tempfile::TempDir;

/// The size the group tests draw at: above the comparison edge, so a comparison
/// preview really is rendered down, and small enough that a decode costs
/// milliseconds.
const WIDTH: u32 = 640;
const HEIGHT: u32 = 427;

/// One frame of a synthetic group.
struct Frame {
    name: &'static str,
    image: RgbImage,
    /// The second of `10:00:0s` it was taken in.
    second: u8,
    /// The fraction of that second, as Exif writes it.
    subsecond: &'static str,
}

/// A frame of the default body at the default exposure, taken at `second`
/// (hundredths `subsecond`) of the moment, showing `stops` brighter a version of
/// the scene than the frame before it.
///
/// The brightness is what makes two frames different pictures of one moment: the
/// bytes differ, so dedupe does not merge them, and the grouping signature is
/// mean-subtracted, so they are still the same scene.
fn frame(name: &'static str, stops: f32, second: u8, subsecond: &'static str) -> Frame {
    let scene = scenes::scene(WIDTH, HEIGHT);
    Frame {
        name,
        image: if stops == 0.0 {
            scene
        } else {
            scenes::brighter(&scene, stops)
        },
        second,
        subsecond,
    }
}

/// Write `frames` into a fresh folder as NEFs of one moment.
fn group(frames: &[Frame]) -> TempDir {
    let dir = tempfile::tempdir().expect("a temp dir");
    for frame in frames {
        let jpeg = support::jpeg_of(&frame.image, 92);
        let facts = ExifFacts {
            captured: Some(format!("2026:11:02 10:00:{:02}", frame.second)),
            subsecond: Some(frame.subsecond.to_string()),
            serial: Some("3001".to_string()),
            iso: Some(200),
            exposure_time: Some((1, 500)),
            f_number: Some((28, 10)),
            focal_length: Some((50, 1)),
        };
        fs::write(
            dir.path().join(frame.name),
            support::nef_with(1, &[jpeg], &facts),
        )
        .expect("write a NEF");
    }
    dir
}

/// The rating and alias every test scores with, and the defaults for the group
/// pass unless a test changes one.
fn config() -> Config {
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

/// Run `config` over `dir`, writing sidecars to disk.
fn run(dir: &Path, config: &Config, decider: &Scripted) -> Run {
    scoring::run(
        dir,
        config,
        decider,
        &sidecar::Filesystem,
        &Silent,
        &Cancel::new(),
    )
    .expect("a run")
}

/// Run `config` over `dir` through `writer`, keeping no sidecar on disk.
fn run_through(dir: &Path, config: &Config, decider: &Scripted, writer: &dyn Writer) -> Run {
    scoring::run(dir, config, decider, writer, &Silent, &Cancel::new()).expect("a run")
}

/// The sidecar text beside `name`.
fn sidecar_of(dir: &Path, name: &str) -> String {
    let path = sidecar::path_for(&dir.join(name));
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("{} has no sidecar: {error}", name))
}

/// The names in `dir`, sorted: "nothing else was written" is assertable.
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
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

/// A writer that refuses every write, so the abort path is reachable without
/// fighting the filesystem.
struct Refusing;

impl Writer for Refusing {
    fn write(
        &self,
        _frame: &Path,
        _document: &str,
        _prior: Prior<'_>,
    ) -> imagecull_core::Result<Written> {
        Err(Error::new(
            ErrorKind::XmpWriteFailed,
            "cannot write a sidecar in this folder",
        ))
    }
}

/// A writer that writes nothing and keeps what it was given, in order: the
/// `--print-sidecars` port, minus the terminal.
#[derive(Default)]
struct Recorder {
    written: Mutex<Vec<(PathBuf, String)>>,
}

impl Writer for Recorder {
    fn write(
        &self,
        frame: &Path,
        document: &str,
        prior: Prior<'_>,
    ) -> imagecull_core::Result<Written> {
        self.written
            .lock()
            .expect("a lock")
            .push((frame.to_path_buf(), document.to_string()));
        Ok(if prior.text().is_some() {
            Written::Replaced
        } else {
            Written::Created
        })
    }
}

impl Recorder {
    fn documents(&self) -> Vec<(PathBuf, String)> {
        self.written.lock().expect("a lock").clone()
    }
}

#[test]
fn a_group_of_two_or_more_gets_one_comparative_request_and_a_lone_frame_gets_none() {
    let shoot = group(&[
        frame("DSC_0001.NEF", 0.0, 0, "10"),
        frame("DSC_0002.NEF", 0.2, 0, "20"),
        // Forty seconds later: a different moment, and a group of one.
        frame("DSC_0003.NEF", 0.4, 40, "00"),
    ]);
    let decider = Scripted::answering(vec![
        Ok(document("5")),
        Ok(document("4")),
        Ok(document("3")),
    ])
    .comparing(vec![Ok(keeper("1", 0.83))]);

    let run = run(shoot.path(), &config(), &decider);

    assert_eq!(decider.seen().len(), 3, "every frame is still scored alone");
    let compared = decider.compared();
    assert_eq!(
        compared.len(),
        1,
        "one request for the group, and none for the frame of its own"
    );
    assert_eq!(compared[0].previews.len(), 2, "both frames of the group");
    assert_eq!(
        compared[0].group(),
        "DSC_0001.NEF",
        "the group is named by the frame it opens with"
    );
    assert_eq!(
        run.groups.groups, 1,
        "the lone frame is a group of one and is not judged"
    );

    let lone = sidecar_of(shoot.path(), "DSC_0003.NEF");
    assert!(
        !lone.contains("imagecull:groupKeeper") && !lone.contains("xmp:Label"),
        "a frame nothing was compared against carries no verdict and no label: {lone}"
    );
    assert!(lone.contains("xmp:Rating=\"3\""), "{lone}");
}

#[test]
fn the_keeper_wears_no_label_and_every_other_frame_of_its_group_wears_red() {
    let shoot = group(&[
        frame("DSC_0001.NEF", 0.0, 0, "10"),
        frame("DSC_0002.NEF", 0.2, 0, "20"),
        frame("DSC_0003.NEF", 0.4, 0, "30"),
    ]);
    let decider = Scripted::answering(vec![
        Ok(document("5")),
        Ok(document("4")),
        Ok(document("3")),
    ])
    .comparing(vec![Ok(keeper("2", 0.71))]);

    let run = run(shoot.path(), &config(), &decider);

    let keeper = sidecar_of(shoot.path(), "DSC_0002.NEF");
    assert!(
        !keeper.contains("xmp:Label"),
        "the keeper's mark is its rating: this run writes no label on it, not even Green: {keeper}"
    );
    assert!(
        keeper.contains("imagecull:groupKeeper=\"true\""),
        "and the verdict is still recorded, because it is not a copy of the label: {keeper}"
    );
    assert!(
        keeper.contains("imagecull:pickConf=\"0.71\""),
        "the comparison's confidence is recorded on every member, the keeper included: {keeper}"
    );

    for name in ["DSC_0001.NEF", "DSC_0003.NEF"] {
        let non_keeper = sidecar_of(shoot.path(), name);
        assert!(
            non_keeper.contains("xmp:Label=\"Red\""),
            "every other frame of the group is red: {name}: {non_keeper}"
        );
        assert!(
            non_keeper.contains("imagecull:labeledBy=\"imagecull\""),
            "a red this tool wrote says so, so a later run may rewrite it: {non_keeper}"
        );
        assert!(
            non_keeper.contains("imagecull:groupKeeper=\"false\""),
            "{non_keeper}"
        );
        assert!(
            non_keeper.contains("imagecull:pickBy=\"comparison\""),
            "a red is explained by what chose the keeper: {non_keeper}"
        );
        assert!(
            non_keeper.contains("imagecull:pickConf=\"0.71\""),
            "{non_keeper}"
        );
        assert!(
            non_keeper.contains("imagecull:groupSize=\"3\"")
                && non_keeper.contains("imagecull:groupStart=\"2026-11-02T10:00:00.100\""),
            "the group it lost is readable without the folder: {non_keeper}"
        );
        assert!(
            non_keeper.contains(&format!(
                "imagecull:groupingHash=\"{}\"",
                imagecull_core::group::grouping_hash(&imagecull_core::group::Thresholds::default())
            )),
            "the thresholds it was decided under travel with it: {non_keeper}"
        );
        assert!(non_keeper.contains("imagecull:pickedAt=\""), "{non_keeper}");
    }

    assert_eq!(
        run.groups,
        Groups {
            groups: 1,
            non_keepers: 2,
            red: 2,
            ..Groups::default()
        }
    );
}

#[test]
fn a_group_larger_than_the_request_can_carry_is_narrowed_and_the_rest_are_red_and_not_compared() {
    let shoot = group(&[
        frame("DSC_0001.NEF", 0.0, 0, "10"),
        frame("DSC_0002.NEF", 0.2, 0, "20"),
        frame("DSC_0003.NEF", 0.4, 0, "30"),
    ]);
    let decider = Scripted::answering(vec![
        Ok(document("5")),
        Ok(document("3")),
        Ok(document("4")),
    ])
    .comparing(vec![Ok(keeper("2", 0.66))]);
    let mut config = config();
    // The same rule the measured 20-frame cap applies, at a size a test can pay
    // for: two frames enter the comparison, the weakest rating does not.
    config.comparison.max = 2;

    let run = run(shoot.path(), &config, &decider);

    let compared = &decider.compared()[0];
    assert_eq!(
        compared.previews.len(),
        2,
        "exactly what the budget carries"
    );
    assert_eq!(
        compared.previews[0].width, 512,
        "previews are rendered at the measured comparison edge"
    );

    let left_out = sidecar_of(shoot.path(), "DSC_0002.NEF");
    assert!(
        left_out.contains("imagecull:groupNote=\"not_compared\"")
            && left_out.contains("imagecull:groupKeeper=\"false\""),
        "a frame the request could not carry says so, and is red like any other non-keeper: {left_out}"
    );
    assert!(
        left_out.contains("xmp:Label=\"Red\"") && left_out.contains("xmp:Rating=\"3\""),
        "it keeps the rating it was given, and still lost its group: {left_out}"
    );
    assert_eq!(run.groups.not_compared, 1);
    assert_eq!(run.groups.red, 2);
}

#[test]
fn a_later_groups_left_out_frame_is_the_one_that_says_so() {
    let shoot = group(&[
        // One moment at 10:00:00, and a second one forty seconds later: two
        // groups, and the second one starts above frame 0 of the run.
        frame("DSC_0001.NEF", 0.0, 0, "10"),
        frame("DSC_0002.NEF", 0.2, 0, "20"),
        frame("DSC_0003.NEF", 0.4, 40, "00"),
        frame("DSC_0004.NEF", 0.6, 40, "10"),
        frame("DSC_0005.NEF", 0.8, 40, "20"),
    ]);
    let decider = Scripted::answering(vec![
        Ok(document("4")),
        Ok(document("4")),
        Ok(document("3")),
        Ok(document("5")),
        Ok(document("4")),
    ])
    .comparing(vec![Ok(keeper("2", 0.90)), Ok(keeper("2", 0.80))]);
    let mut config = config();
    config.comparison.max = 2;

    let run = run(shoot.path(), &config, &decider);

    assert_eq!(
        decider.compared().len(),
        2,
        "the second group is judged like the first"
    );
    let left_out = sidecar_of(shoot.path(), "DSC_0003.NEF");
    assert!(
        left_out.contains("imagecull:groupNote=\"not_compared\"")
            && left_out.contains("imagecull:groupKeeper=\"false\"")
            && left_out.contains("xmp:Label=\"Red\""),
        "the weakest frame of the second group was left out, and is the one that says so: \
         {left_out}"
    );
    for name in ["DSC_0004.NEF", "DSC_0005.NEF"] {
        let compared = sidecar_of(shoot.path(), name);
        assert!(
            !compared.contains("imagecull:groupNote"),
            "a frame the request did carry says nothing about being left out: {name}: {compared}"
        );
    }
    let noted = [
        "DSC_0001.NEF",
        "DSC_0002.NEF",
        "DSC_0003.NEF",
        "DSC_0004.NEF",
        "DSC_0005.NEF",
    ]
    .iter()
    .filter(|name| sidecar_of(shoot.path(), name).contains("imagecull:groupNote"))
    .count();
    assert_eq!(
        noted, run.groups.not_compared,
        "the summary's count and the notes on disk are the same comparison"
    );
    assert_eq!(run.groups.not_compared, 1);
}

#[test]
fn a_group_the_model_was_unsure_about_is_marked_anyway_and_counted() {
    let shoot = group(&[
        frame("DSC_0001.NEF", 0.0, 0, "10"),
        frame("DSC_0002.NEF", 0.2, 0, "20"),
    ]);
    let decider = Scripted::answering(vec![Ok(document("4")), Ok(document("4"))])
        .comparing(vec![Ok(keeper("1", 0.42))]);

    let run = run(shoot.path(), &config(), &decider);

    assert_eq!(
        run.groups.low_confidence, 1,
        "a near-tie is the common case for a group, and it is reported rather than suppressed"
    );
    assert_eq!(run.groups.red, 1, "and the non-keeper is marked anyway");
    assert!(
        sidecar_of(shoot.path(), "DSC_0002.NEF").contains("xmp:Label=\"Red\""),
        "the mark is what a cull filters on, so 0.42 would have to be very wrong to withhold it"
    );
}

#[test]
fn a_comparison_that_fails_marks_nothing_and_reports_the_group_as_undecided() {
    let shoot = group(&[
        frame("DSC_0001.NEF", 0.0, 0, "10"),
        frame("DSC_0002.NEF", 0.2, 0, "20"),
    ]);
    let decider =
        Scripted::answering(vec![Ok(document("4")), Ok(document("4"))]).comparing(vec![Err(
            Error::new(
                ErrorKind::Timeout,
                "POST /v1/systemone timed out after 120s — 4 attempts",
            ),
        )]);
    let warnings = Warnings::default();

    let run = scoring::run(
        shoot.path(),
        &config(),
        &decider,
        &sidecar::Filesystem,
        &warnings,
        &Cancel::new(),
    )
    .expect("a run");

    for name in ["DSC_0001.NEF", "DSC_0002.NEF"] {
        let text = sidecar_of(shoot.path(), name);
        assert!(
            !text.contains("xmp:Label") && !text.contains("imagecull:groupKeeper"),
            "nothing is marked on no answer: {name}: {text}"
        );
        assert!(
            text.contains("xmp:Rating=\"4\""),
            "the rating stands: {text}"
        );
    }
    assert_eq!(run.groups.undecided, 1);
    assert_eq!(run.groups.non_keepers, 0);
    let messages = warnings.messages();
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert!(
        messages[0].contains("timeout") && messages[0].contains("nothing was marked"),
        "the status is reported, and the fold has to know nothing was filtered: {}",
        messages[0]
    );
}

#[test]
fn a_comparison_that_answers_nothing_is_undecided_rather_than_a_guess() {
    let shoot = group(&[
        frame("DSC_0001.NEF", 0.0, 0, "10"),
        frame("DSC_0002.NEF", 0.2, 0, "20"),
    ]);
    let decider = Scripted::answering(vec![Ok(document("4")), Ok(document("4"))])
        .comparing(vec![Ok(serde_json::json!({"answers": {}}))]);

    let run = run(shoot.path(), &config(), &decider);

    assert_eq!(run.groups.undecided, 1);
    assert_eq!(run.groups.non_keepers, 0);
    assert!(
        !sidecar_of(shoot.path(), "DSC_0001.NEF").contains("xmp:Label"),
        "an unreadable answer is not a keeper, and not a red either"
    );
}

#[test]
fn a_label_the_photographer_wrote_is_left_alone_and_the_verdict_is_still_recorded() {
    let shoot = group(&[
        frame("DSC_0001.NEF", 0.0, 0, "10"),
        frame("DSC_0002.NEF", 0.2, 0, "20"),
    ]);
    let yellow = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\
                  <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
                  <rdf:Description rdf:about=\"\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" \
                  xmp:Label=\"Yellow\"/></rdf:RDF></x:xmpmeta>";
    fs::write(
        sidecar::path_for(&shoot.path().join("DSC_0001.NEF")),
        yellow,
    )
    .expect("a sidecar of the photographer's");
    let decider = Scripted::answering(vec![Ok(document("4")), Ok(document("4"))])
        .comparing(vec![Ok(keeper("2", 0.80))]);

    let run = run(shoot.path(), &config(), &decider);

    let text = sidecar_of(shoot.path(), "DSC_0001.NEF");
    assert!(
        text.contains("xmp:Label=\"Yellow\""),
        "the photographer's colour is never overwritten: {text}"
    );
    assert!(
        text.contains("imagecull:groupKeeper=\"false\"") && text.contains("imagecull:pickConf="),
        "and the verdict exists anyway, because a label is only one rendering of it: {text}"
    );
    assert_eq!(
        run.groups.unmarked, 1,
        "the delete set is incomplete and says so"
    );
    assert_eq!(run.groups.red, 0, "this run marked no frame red");
    assert_eq!(
        run.groups.non_keepers, 1,
        "and the non-keeper it could not tint still counts as one"
    );
}

#[test]
fn a_red_this_tool_did_not_write_is_the_photographers_and_is_left_alone() {
    let shoot = group(&[
        frame("DSC_0001.NEF", 0.0, 0, "10"),
        frame("DSC_0002.NEF", 0.2, 0, "20"),
    ]);
    // Red, and no `imagecull:labeledBy`: the photographer culls in red too, and a
    // run that assumed every red was its own would rewrite their filter.
    let theirs = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\
                  <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
                  <rdf:Description rdf:about=\"\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" \
                  xmp:Label=\"Red\"/></rdf:RDF></x:xmpmeta>";
    fs::write(
        sidecar::path_for(&shoot.path().join("DSC_0001.NEF")),
        theirs,
    )
    .expect("a sidecar of the photographer's");
    let decider = Scripted::answering(vec![Ok(document("4")), Ok(document("4"))])
        .comparing(vec![Ok(keeper("2", 0.9))]);

    let run = run(shoot.path(), &config(), &decider);

    let text = sidecar_of(shoot.path(), "DSC_0001.NEF");
    assert!(
        !text.contains("imagecull:labeledBy"),
        "the run does not claim a red it did not write: {text}"
    );
    assert!(
        text.contains("imagecull:groupKeeper=\"false\""),
        "and the verdict is recorded beside it anyway: {text}"
    );
    assert_eq!(run.groups.unmarked, 1);
    assert_eq!(
        run.groups.red, 0,
        "a label written by a previous run is rewritten only where that run said so"
    );
}

#[test]
fn no_comparative_answer_changes_a_rating() {
    let shoot = group(&[
        frame("DSC_0001.NEF", 0.0, 0, "10"),
        frame("DSC_0002.NEF", 0.2, 0, "20"),
        frame("DSC_0003.NEF", 0.4, 0, "30"),
    ]);
    // The model keeps the frame it rated 3, so the 5★ frame of the same group is
    // the one that goes red: the mark is a judgement about the group, and the
    // rating stays a judgement about the frame.
    let decider = Scripted::answering(vec![
        Ok(document("5")),
        Ok(document("3")),
        Ok(document("4")),
    ])
    .comparing(vec![Ok(keeper("2", 0.77))]);

    let run = run(shoot.path(), &config(), &decider);

    for (name, stars) in [
        ("DSC_0001.NEF", "5"),
        ("DSC_0002.NEF", "3"),
        ("DSC_0003.NEF", "4"),
    ] {
        assert!(
            sidecar_of(shoot.path(), name).contains(&format!("xmp:Rating=\"{stars}\"")),
            "{name} was rated {stars} and the group pass left that alone"
        );
    }
    let five_star = sidecar_of(shoot.path(), "DSC_0001.NEF");
    assert!(
        five_star.contains("xmp:Label=\"Red\"") && five_star.contains("xmp:Rating=\"5\""),
        "a 5★ frame that lost to a 3★ is correctly red, and still a 5★: {five_star}"
    );
    assert_eq!(
        run.summary().stars,
        [0, 0, 1, 1, 1],
        "the histogram is unchanged"
    );
}

#[test]
fn a_frame_promoted_to_keeper_keeps_its_red_and_the_run_warns_the_folder_disagrees() {
    let shoot = group(&[
        frame("DSC_0001.NEF", 0.0, 0, "10"),
        frame("DSC_0002.NEF", 0.2, 0, "20"),
    ]);
    let first = Scripted::answering(vec![Ok(document("4")), Ok(document("4"))])
        .comparing(vec![Ok(keeper("2", 0.62))]);
    run(shoot.path(), &config(), &first);
    assert!(
        sidecar_of(shoot.path(), "DSC_0001.NEF").contains("xmp:Label=\"Red\""),
        "the first run made the first frame red"
    );

    // The same folder again, re-rated, with the other frame as the keeper.
    let mut forced = config();
    forced.force = true;
    let second = Scripted::answering(vec![Ok(document("4")), Ok(document("4"))])
        .comparing(vec![Ok(keeper("1", 0.68))]);
    let run = run(shoot.path(), &forced, &second);

    let promoted = sidecar_of(shoot.path(), "DSC_0001.NEF");
    assert!(
        promoted.contains("xmp:Label=\"Red\""),
        "no label is ever removed: the folder keeps a red this grouping disagrees with: {promoted}"
    );
    assert!(
        promoted.contains("imagecull:groupKeeper=\"true\""),
        "and the verdict says what this run actually decided: {promoted}"
    );
    assert!(
        promoted.contains("imagecull:labeledBy=\"imagecull\""),
        "the proof that the red is ours survives, so a later run can still touch it: {promoted}"
    );
    assert_eq!(
        run.groups.stale_reds, 1,
        "the summary is the only remedy the decision allows"
    );
    assert!(
        sidecar_of(shoot.path(), "DSC_0002.NEF").contains("xmp:Label=\"Red\""),
        "and the frame that lost this time is marked now"
    );
}

#[test]
fn a_forced_preview_shows_the_verdict_beside_the_rating_this_run_gave() {
    let shoot = group(&[
        frame("DSC_0001.NEF", 0.0, 0, "10"),
        frame("DSC_0002.NEF", 0.2, 0, "20"),
    ]);
    // An earlier run left a 3★ rating and no verdict, and `--force` rates the
    // folder again: the preview must show the verdict on the rating this run
    // produced, not on the one still in the folder.
    let earlier = sidecar::render(
        None,
        &Properties {
            rating: 3,
            conf_stars: 0.51,
            model: "clef-flash-oQ4e".to_string(),
            rubric_hash: "0f1e2d3c".to_string(),
            scored_at: "2026-11-01T09:00:00Z".to_string(),
            version: "0.1.0".to_string(),
        },
    )
    .expect("an earlier document");
    for name in ["DSC_0001.NEF", "DSC_0002.NEF"] {
        fs::write(sidecar::path_for(&shoot.path().join(name)), &earlier)
            .expect("an earlier sidecar");
    }
    let decider = Scripted::answering(vec![Ok(document("4")), Ok(document("4"))])
        .comparing(vec![Ok(keeper("2", 0.81))]);
    let mut config = config();
    config.force = true;
    let recorder = Recorder::default();

    run_through(shoot.path(), &config, &decider, &recorder);

    let verdict = recorder
        .documents()
        .into_iter()
        .map(|(_, document)| document)
        .find(|document| document.contains("imagecull:groupKeeper"))
        .expect("a verdict is previewed for every frame");
    assert!(
        verdict.contains("xmp:Rating=\"4\""),
        "the verdict previews on the rating this run wrote, not the one in the folder: {verdict}"
    );
    assert!(
        !verdict.contains("xmp:Rating=\"3\""),
        "and the rating it replaced is gone, exactly as a real run would leave it: {verdict}"
    );
    assert!(
        verdict.contains("imagecull:confStars=\"0.63\""),
        "the rating's own provenance travels with it: {verdict}"
    );
}

#[test]
fn every_frame_is_rated_on_disk_before_any_group_is_judged() {
    let shoot = group(&[
        frame("DSC_0001.NEF", 0.0, 0, "10"),
        frame("DSC_0002.NEF", 0.2, 0, "20"),
    ]);
    let decider = Scripted::answering(vec![Ok(document("4")), Ok(document("4"))])
        .comparing(vec![Ok(keeper("1", 0.9))]);
    let recorder = Recorder::default();

    run_through(shoot.path(), &config(), &decider, &recorder);

    assert_eq!(
        names(shoot.path()),
        ["DSC_0001.NEF", "DSC_0002.NEF"],
        "a preview run writes nothing into the shoot folder"
    );
    let written = recorder.documents();
    assert_eq!(written.len(), 4, "a rating and a verdict per frame");
    let with_verdict = written
        .iter()
        .position(|(_, document)| document.contains("imagecull:groupKeeper"));
    let last_rating = written
        .iter()
        .rposition(|(_, document)| !document.contains("imagecull:groupKeeper"));
    assert_eq!(
        (last_rating, with_verdict),
        (Some(1), Some(2)),
        "every rating is written before the first mark, so no mark is ever reported for a rating \
         that is not on disk"
    );

    let keeper = &written[2].1;
    let non_keeper = &written[3].1;
    assert!(!keeper.contains("xmp:Label"), "{keeper}");
    assert!(
        non_keeper.contains("xmp:Label=\"Red\"") && non_keeper.contains("imagecull:labeledBy"),
        "the preview shows the label line with what was there before: {non_keeper}"
    );
    assert!(
        non_keeper.contains("xmp:Rating=\"4\""),
        "and the rating it was written beside: {non_keeper}"
    );
}

#[test]
fn a_group_the_model_cannot_hold_is_reported_with_its_own_status_and_marks_nothing() {
    let shoot = group(&[
        frame("DSC_0001.NEF", 0.0, 0, "10"),
        frame("DSC_0002.NEF", 0.2, 0, "20"),
    ]);
    let decider =
        Scripted::answering(vec![Ok(document("4")), Ok(document("4"))]).comparing(vec![Err(
            Error::new(
                ErrorKind::Http413,
                "POST /v1/systemone returned HTTP 413: the schema needs 19810 tokens before the \
             state; the limit is 16384",
            ),
        )]);
    let warnings = Warnings::default();

    let run = scoring::run(
        shoot.path(),
        &config(),
        &decider,
        &sidecar::Filesystem,
        &warnings,
        &Cancel::new(),
    )
    .expect("a rejected group is not a rejected run");

    assert_eq!(run.groups.undecided, 1);
    assert_eq!(run.groups.non_keepers, 0);
    assert_eq!(
        decider.compared().len(),
        1,
        "the request is refused once: it is never retried smaller or in pieces"
    );
    let messages = warnings.messages();
    assert!(
        messages[0].contains("http_413") && messages[0].contains("nothing was marked"),
        "the frame's status vocabulary carries the failure: {}",
        messages[0]
    );
}

#[test]
fn a_run_that_stopped_writes_no_marks_for_the_frames_it_did_judge() {
    let shoot = group(&[
        frame("DSC_0001.NEF", 0.0, 0, "10"),
        frame("DSC_0002.NEF", 0.2, 0, "20"),
    ]);
    let decider = Scripted::answering(vec![Ok(document("4")), Ok(document("4"))])
        .comparing(vec![Ok(keeper("1", 0.9))]);

    let run = scoring::run(
        shoot.path(),
        &config(),
        &decider,
        &Refusing,
        &Silent,
        &Cancel::new(),
    )
    .expect("a run");

    assert_eq!(
        run.ending,
        scoring::Ending::Aborted {
            reason: run.frames[0].failure().expect("a failure").message.clone(),
        }
    );
    assert!(
        decider.compared().is_empty(),
        "a folder the run could not finish is not a folder whose groups are known"
    );
    assert!(run.groups.judged_nothing());
}
