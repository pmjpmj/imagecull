//! The cull: a folder in, the group's verdict and its red marks out.
//!
//! Every test here runs the real model-free pass over a synthetic group —
//! NEF-shaped files with a real capture time, one body, one exposure and one
//! scene — and then the cull over the plan it produces, with a scripted
//! [`Decider`] behind the same seam the comparative request goes through.
//! Nothing here needs a server, and nothing here asserts on an internal helper:
//! the assertions are the sidecars a photographer opens, the events a watcher
//! receives, and the counts the summary prints.
//!
//! [`Decider`]: imagecull_core::scoring::Decider

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use image::RgbImage;
use imagecull_core::cull::{self, Culled, Event, Watcher};
use imagecull_core::error::Error;
use imagecull_core::ingest::ScanOptions;
use imagecull_core::preprocess::Params;
use imagecull_core::scoring::RetryPolicy;
use imagecull_core::sidecar::{Prior, Writer, Written};
use imagecull_core::{ErrorKind, group, keeper, sidecar};
use support::ExifFacts;
use support::decider::{Scripted, keeper};
use support::scenes;

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
/// bytes differ, so the frames are not identical, and the grouping signature is
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
fn shoot_of(frames: &[Frame]) -> tempfile::TempDir {
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

/// Every event a cull pass reported, in order.
#[derive(Default)]
struct Recorded {
    events: Mutex<Vec<Event>>,
}

impl Watcher for Recorded {
    fn event(&self, event: &Event) {
        self.events.lock().expect("a lock").push(event.clone());
    }
}

impl Recorded {
    fn events(&self) -> Vec<Event> {
        self.events.lock().expect("a lock").clone()
    }

    fn verdicts(&self) -> Vec<cull::Verdict> {
        self.events()
            .into_iter()
            .filter_map(|event| match event {
                Event::Group(verdict) => Some(verdict),
                _ => None,
            })
            .collect()
    }

    fn unwritten(&self) -> Vec<(String, String)> {
        self.events()
            .into_iter()
            .filter_map(|event| match event {
                Event::Unwritten { filename, message } => Some((filename, message)),
                _ => None,
            })
            .collect()
    }
}

/// The plan the model-free pass makes of `dir`, with the default thresholds.
fn plan(dir: &Path) -> group::Plan {
    group::run(
        dir,
        &group::Options {
            scan: ScanOptions::default(),
            thresholds: group::Thresholds::default(),
            jobs: Some(1),
        },
    )
    .expect("a plan")
}

/// Cull `dir` with the measured budget, reporting every event to `recorded`.
fn cull(dir: &Path, decider: &Scripted, writer: &dyn Writer, recorded: &Recorded) -> Culled {
    cull_with(
        dir,
        decider,
        writer,
        recorded,
        keeper::Budget::default(),
        false,
    )
}

/// Cull `dir` with `budget`, keeping every sidecar on disk.
fn cull_through(
    dir: &Path,
    decider: &Scripted,
    recorded: &Recorded,
    budget: keeper::Budget,
) -> Culled {
    cull_with(dir, decider, &sidecar::Filesystem, recorded, budget, false)
}

fn cull_with(
    dir: &Path,
    decider: &Scripted,
    writer: &dyn Writer,
    recorded: &Recorded,
    budget: keeper::Budget,
    allow_foreign: bool,
) -> Culled {
    let plan = plan(dir);
    let config = cull::Config {
        model: "clef-flash-oQ4e",
        shoot_context: None,
        budget,
        params: Params::default(),
        grouping_hash: &plan.hash,
        allow_foreign,
        retry: RetryPolicy::without_waiting(),
    };
    cull::run(&plan, &config, decider, writer, recorded).expect("a cull")
}

/// The run with nothing to report.
fn quiet() -> Recorded {
    Recorded::default()
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

/// A writer that refuses every write, so the unwritten path is reachable
/// without fighting the filesystem.
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

fn two_frames() -> tempfile::TempDir {
    shoot_of(&[
        frame("DSC_0001.NEF", 0.0, 0, "10"),
        frame("DSC_0002.NEF", 0.2, 0, "20"),
    ])
}

#[test]
fn one_request_decides_the_group_and_every_other_frame_wears_the_red() {
    let shoot = shoot_of(&[
        frame("DSC_0001.NEF", 0.0, 0, "10"),
        frame("DSC_0002.NEF", 0.2, 0, "20"),
        frame("DSC_0003.NEF", 0.4, 0, "30"),
    ]);
    let decider = Scripted::answering(Vec::new()).comparing(vec![Ok(keeper("2", 0.71))]);
    let recorded = quiet();

    let culled = cull(shoot.path(), &decider, &sidecar::Filesystem, &recorded);

    let compared = decider.compared();
    assert_eq!(compared.len(), 1, "one group, one request");
    assert_eq!(compared[0].previews.len(), 3, "the whole group is compared");
    assert_eq!(
        compared[0].previews[0].width, 512,
        "previews are rendered at the measured comparison edge"
    );
    assert_eq!(
        compared[0].group(),
        "DSC_0001.NEF",
        "the group is named by the frame it opens with"
    );
    assert!(
        decider.seen().is_empty(),
        "no frame is rated: the cull asks one relative question"
    );

    let keeper = sidecar_of(shoot.path(), "DSC_0002.NEF");
    assert!(
        !keeper.contains("xmp:Label"),
        "the keeper's mark is its rating: this pass writes no label on it: {keeper}"
    );
    assert!(
        keeper.contains("imagecull:groupKeeper=\"true\"")
            && keeper.contains("imagecull:pickConf=\"0.71\"")
            && keeper.contains("imagecull:pickRounds=\"1\""),
        "the verdict and its provenance are recorded: {keeper}"
    );
    assert!(
        !keeper.contains("xmp:Rating"),
        "the cull writes no rating, not even the keeper's: {keeper}"
    );

    for name in ["DSC_0001.NEF", "DSC_0003.NEF"] {
        let non_keeper = sidecar_of(shoot.path(), name);
        assert!(
            non_keeper.contains("xmp:Label=\"Red\"")
                && non_keeper.contains("imagecull:labeledBy=\"imagecull\""),
            "every other frame of the group is red, and says whose red it is: {name}: {non_keeper}"
        );
        assert!(
            non_keeper.contains("imagecull:groupKeeper=\"false\"")
                && non_keeper.contains("imagecull:groupSize=\"3\"")
                && non_keeper.contains("imagecull:groupStart=\"2026-11-02T10:00:00.100\"")
                && non_keeper.contains("imagecull:pickBy=\"comparison\""),
            "the group it lost is readable without the folder: {non_keeper}"
        );
    }
    assert_eq!(
        culled,
        Culled {
            groups: 1,
            non_keepers: 2,
            red: 2,
            rounds: 1,
            verdicts: vec![cull::Verdict {
                number: 1,
                start: "DSC_0001.NEF".to_string(),
                size: 3,
                keeper: Some("DSC_0002.NEF".to_string()),
                red: 2,
                unmarked: 0,
                confidence: 0.71,
                rounds: 1,
                failure: None,
            }],
            ..Culled::default()
        }
    );
}

#[test]
fn a_frame_of_its_own_is_compared_with_nothing_and_marked_with_nothing() {
    let shoot = shoot_of(&[
        frame("DSC_0001.NEF", 0.0, 0, "10"),
        frame("DSC_0002.NEF", 0.2, 0, "20"),
        // Forty seconds later: a different moment, and a group of one.
        frame("DSC_0003.NEF", 0.4, 40, "00"),
    ]);
    let decider = Scripted::answering(Vec::new()).comparing(vec![Ok(keeper("1", 0.83))]);
    let recorded = quiet();

    let culled = cull(shoot.path(), &decider, &sidecar::Filesystem, &recorded);

    assert_eq!(decider.compared().len(), 1, "the lone frame asks nothing");
    assert_eq!(culled.groups, 1);
    assert!(
        !shoot.path().join("DSC_0003.xmp").exists(),
        "a frame nothing was compared against gets no sidecar from a cull"
    );
    assert_eq!(
        names(shoot.path()).len(),
        5,
        "three raws and the two sidecars the group needed: nothing else"
    );
}

#[test]
fn the_cull_reads_no_rating_and_writes_none() {
    let shoot = two_frames();
    let rated = sidecar::render(
        None,
        &sidecar::Properties {
            rating: 5,
            conf_stars: 0.91,
            model: "clef-flash-oQ4e".to_string(),
            rubric_hash: "0f1e2d3c".to_string(),
            scored_at: "2026-11-01T09:00:00Z".to_string(),
            version: "0.1.0".to_string(),
        },
    )
    .expect("a rated document");
    fs::write(
        sidecar::path_for(&shoot.path().join("DSC_0001.NEF")),
        &rated,
    )
    .expect("a sidecar from score");
    let decider = Scripted::answering(Vec::new()).comparing(vec![Ok(keeper("2", 0.77))]);
    let recorded = quiet();

    let culled = cull(shoot.path(), &decider, &sidecar::Filesystem, &recorded);

    let five_star = sidecar_of(shoot.path(), "DSC_0001.NEF");
    assert!(
        five_star.contains("xmp:Rating=\"5\"")
            && five_star.contains("imagecull:confStars=\"0.91\"")
            && five_star.contains("imagecull:rubricHash=\"0f1e2d3c\""),
        "a rating and its provenance survive the cull exactly as they were: {five_star}"
    );
    assert!(
        five_star.contains("xmp:Label=\"Red\""),
        "and the frame that lost its group is marked anyway: the rating does not vote: {five_star}"
    );
    assert_eq!(culled.red, 1);
}

#[test]
fn a_group_larger_than_one_request_is_compared_in_rounds_and_drops_no_frame() {
    let shoot = shoot_of(&[
        frame("DSC_0001.NEF", 0.0, 0, "10"),
        frame("DSC_0002.NEF", 0.2, 0, "15"),
        frame("DSC_0003.NEF", 0.4, 0, "20"),
        frame("DSC_0004.NEF", 0.6, 0, "25"),
        frame("DSC_0005.NEF", 0.8, 0, "30"),
    ]);
    // Two frames per request: three first-round batches (2, 2, 1), two second
    // round batches (the three winners, 2 then 1), then the final.
    let decider = Scripted::answering(Vec::new()).comparing(vec![
        Ok(keeper("1", 0.90)),
        Ok(keeper("1", 0.80)),
        Ok(keeper("1", 0.70)),
        Ok(keeper("1", 0.60)),
        Ok(keeper("1", 0.50)),
        Ok(keeper("2", 0.40)),
    ]);
    let recorded = quiet();
    let budget = keeper::Budget {
        max: 2,
        ..keeper::Budget::default()
    };

    let culled = cull_through(shoot.path(), &decider, &recorded, budget);

    let compared = decider.compared();
    assert_eq!(compared.len(), 6, "every round is a request: {compared:?}");
    assert_eq!(
        compared[0].previews.len(),
        2,
        "the first batch carries the request's worth and no more"
    );
    let mut first_round: Vec<String> = Vec::new();
    for request in &compared[..3] {
        assert_eq!(request.previews.len().min(2), request.previews.len());
        first_round.push(request.group().to_string());
    }
    assert_eq!(
        first_round
            .iter()
            .filter(|name| *name == "DSC_0001.NEF")
            .count(),
        3,
        "each first-round batch is named by the frame it opens with, and the group is the same"
    );
    let culled_verdict = recorded.verdicts().remove(0);
    assert_eq!(
        culled_verdict.rounds, 6,
        "six requests stood behind the verdict"
    );
    assert_eq!(
        culled_verdict.confidence, 0.40,
        "the shakiest call on the winner's path, not the final call's certainty"
    );
    assert_eq!(
        culled_verdict.keeper.as_deref(),
        Some("DSC_0005.NEF"),
        "the survivor of the final round"
    );
    assert_eq!(culled.red, 4, "every frame that is not the keeper is red");
    assert_eq!(culled.unwritten, 0);

    // Every frame of the group carries the verdict, including the ones a
    // shortlist would have dropped before clef saw them.
    for name in [
        "DSC_0001.NEF",
        "DSC_0002.NEF",
        "DSC_0003.NEF",
        "DSC_0004.NEF",
        "DSC_0005.NEF",
    ] {
        let text = sidecar_of(shoot.path(), name);
        assert!(
            text.contains("imagecull:pickRounds=\"6\""),
            "{name}: {text}"
        );
    }
}

#[test]
fn a_group_the_model_was_unsure_about_is_marked_anyway_and_counted() {
    let shoot = two_frames();
    let decider = Scripted::answering(Vec::new()).comparing(vec![Ok(keeper("1", 0.42))]);
    let recorded = quiet();

    let culled = cull(shoot.path(), &decider, &sidecar::Filesystem, &recorded);

    assert_eq!(
        culled.low_confidence, 1,
        "a near-tie is the common case for a group, and it is reported rather than suppressed"
    );
    assert_eq!(culled.red, 1, "and the non-keeper is marked anyway");
    assert!(
        sidecar_of(shoot.path(), "DSC_0002.NEF").contains("xmp:Label=\"Red\""),
        "the mark is what a cull filters on, so 0.42 would have to be very wrong to withhold it"
    );
}

#[test]
fn a_comparison_that_fails_marks_nothing_and_reports_the_group_as_undecided() {
    let shoot = two_frames();
    // A timeout is retried on the spec's schedule: four attempts, and then the
    // group is undecided rather than marked on a failure.
    let timed_out = || {
        Err(Error::new(
            ErrorKind::Timeout,
            "POST /v1/systemone timed out after 120s",
        ))
    };
    let decider = Scripted::answering(Vec::new()).comparing(vec![
        timed_out(),
        timed_out(),
        timed_out(),
        timed_out(),
    ]);
    let recorded = quiet();

    let culled = cull(shoot.path(), &decider, &sidecar::Filesystem, &recorded);

    assert_eq!(culled.undecided, 1);
    assert_eq!(culled.non_keepers, 0, "nothing is marked on no answer");
    assert_eq!(names(shoot.path()).len(), 2, "no sidecar was invented");
    let verdict = recorded.verdicts().remove(0);
    assert_eq!(verdict.keeper, None);
    let failure = verdict
        .failure
        .expect("the group says why it has no keeper");
    assert_eq!(failure.kind, ErrorKind::Timeout);
    assert!(
        failure.message.contains("timed out"),
        "the status a photographer reads: {}",
        failure.message
    );
}

#[test]
fn a_comparison_that_answers_nothing_is_undecided_rather_than_a_guess() {
    let shoot = two_frames();
    let decider =
        Scripted::answering(Vec::new()).comparing(vec![Ok(serde_json::json!({"answers": {}}))]);
    let recorded = quiet();

    let culled = cull(shoot.path(), &decider, &sidecar::Filesystem, &recorded);

    assert_eq!(culled.undecided, 1);
    assert_eq!(culled.non_keepers, 0);
    assert!(
        !shoot.path().join("DSC_0001.xmp").exists(),
        "an unreadable answer is not a keeper, and not a red either"
    );
}

#[test]
fn a_label_the_photographer_wrote_is_left_alone_and_the_verdict_is_still_recorded() {
    let shoot = two_frames();
    let yellow = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\
                  <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
                  <rdf:Description rdf:about=\"\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" \
                  xmp:Label=\"Yellow\"/></rdf:RDF></x:xmpmeta>";
    fs::write(
        sidecar::path_for(&shoot.path().join("DSC_0001.NEF")),
        yellow,
    )
    .expect("a sidecar of the photographer's");
    let decider = Scripted::answering(Vec::new()).comparing(vec![Ok(keeper("2", 0.80))]);
    let recorded = quiet();

    let culled = cull(shoot.path(), &decider, &sidecar::Filesystem, &recorded);

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
        culled.unmarked, 1,
        "the delete set is incomplete and says so"
    );
    assert_eq!(culled.red, 0, "this pass marked no frame red");
    assert_eq!(
        culled.non_keepers, 1,
        "and the non-keeper still counts as one"
    );
}

#[test]
fn a_red_this_tool_did_not_write_is_the_photographers_and_is_left_alone() {
    let shoot = two_frames();
    // Red, and no `imagecull:labeledBy`: the photographer culls in red too, and a
    // pass that assumed every red was its own would rewrite their filter.
    let theirs = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\
                  <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
                  <rdf:Description rdf:about=\"\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" \
                  xmp:Label=\"Red\"/></rdf:RDF></x:xmpmeta>";
    fs::write(
        sidecar::path_for(&shoot.path().join("DSC_0001.NEF")),
        theirs,
    )
    .expect("a sidecar of the photographer's");
    let decider = Scripted::answering(Vec::new()).comparing(vec![Ok(keeper("2", 0.9))]);
    let recorded = quiet();

    let culled = cull(shoot.path(), &decider, &sidecar::Filesystem, &recorded);

    let text = sidecar_of(shoot.path(), "DSC_0001.NEF");
    assert!(
        !text.contains("imagecull:labeledBy"),
        "the cull does not claim a red it did not write: {text}"
    );
    assert!(
        text.contains("imagecull:groupKeeper=\"false\""),
        "and the verdict is recorded beside it anyway: {text}"
    );
    assert_eq!(culled.unmarked, 1);
    assert_eq!(culled.red, 0);
}

#[test]
fn a_promoted_keeper_loses_the_red_this_tool_wrote() {
    let shoot = two_frames();
    let first = Scripted::answering(Vec::new()).comparing(vec![Ok(keeper("2", 0.62))]);
    cull(shoot.path(), &first, &sidecar::Filesystem, &quiet());
    assert!(
        sidecar_of(shoot.path(), "DSC_0001.NEF").contains("xmp:Label=\"Red\""),
        "the first cull made the first frame red"
    );

    let second = Scripted::answering(Vec::new()).comparing(vec![Ok(keeper("1", 0.68))]);
    let recorded = quiet();
    let culled = cull(shoot.path(), &second, &sidecar::Filesystem, &recorded);

    let promoted = sidecar_of(shoot.path(), "DSC_0001.NEF");
    assert!(
        !promoted.contains("xmp:Label") && !promoted.contains("imagecull:labeledBy"),
        "the red this tool wrote is retired, so the keeper is not the frame a cull filters out: \
         {promoted}"
    );
    assert!(
        promoted.contains("imagecull:groupKeeper=\"true\""),
        "and the verdict says what this run decided: {promoted}"
    );
    assert_eq!(culled.retired, 1, "the retirement is counted");
    assert!(
        sidecar_of(shoot.path(), "DSC_0002.NEF").contains("xmp:Label=\"Red\""),
        "and the frame that lost this time is marked now"
    );
}

#[test]
fn a_red_without_our_proof_survives_a_promotion() {
    let shoot = two_frames();
    let theirs = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\
                  <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
                  <rdf:Description rdf:about=\"\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" \
                  xmp:Label=\"Red\"/></rdf:RDF></x:xmpmeta>";
    fs::write(
        sidecar::path_for(&shoot.path().join("DSC_0001.NEF")),
        theirs,
    )
    .expect("a sidecar of the photographer's");
    let decider = Scripted::answering(Vec::new()).comparing(vec![Ok(keeper("1", 0.68))]);
    let recorded = quiet();

    let culled = cull(shoot.path(), &decider, &sidecar::Filesystem, &recorded);

    assert!(
        sidecar_of(shoot.path(), "DSC_0001.NEF").contains("xmp:Label=\"Red\""),
        "a red this tool did not write is the photographer's, keeper or not"
    );
    assert_eq!(
        culled.stale_reds, 1,
        "and the summary says the folder disagrees"
    );
    assert_eq!(culled.retired, 0);
}

#[test]
fn a_frame_that_is_no_longer_in_a_group_loses_the_red_we_wrote() {
    let shoot = two_frames();
    let first = Scripted::answering(Vec::new()).comparing(vec![Ok(keeper("2", 0.62))]);
    cull(shoot.path(), &first, &sidecar::Filesystem, &quiet());
    assert!(
        sidecar_of(shoot.path(), "DSC_0001.NEF").contains("xmp:Label=\"Red\""),
        "the first cull made the first frame red"
    );

    // The folder is regrouped with the second frame far enough away to be a
    // moment of its own: the first frame is now alone, so the comparison it lost
    // no longer exists.
    let regrouped = shoot_of(&[
        frame("DSC_0001.NEF", 0.0, 0, "10"),
        frame("DSC_0002.NEF", 0.2, 40, "00"),
    ]);
    fs::copy(
        sidecar::path_for(&shoot.path().join("DSC_0001.NEF")),
        sidecar::path_for(&regrouped.path().join("DSC_0001.NEF")),
    )
    .expect("copy the verdict");
    let decider = Scripted::answering(Vec::new());
    let recorded = quiet();

    let culled = cull(regrouped.path(), &decider, &sidecar::Filesystem, &recorded);

    assert!(decider.compared().is_empty(), "two lone frames ask nothing");
    let text = sidecar_of(regrouped.path(), "DSC_0001.NEF");
    assert!(
        !text.contains("xmp:Label") && !text.contains("imagecull:groupKeeper"),
        "a red for a comparison that no longer exists is retired: {text}"
    );
    assert_eq!(culled.retired, 1);
}

#[test]
fn a_preview_run_asks_the_group_and_writes_nothing() {
    let shoot = two_frames();
    let decider = Scripted::answering(Vec::new()).comparing(vec![Ok(keeper("2", 0.81))]);
    let recorded = quiet();
    let recorder = Recorder::default();

    let culled = cull(shoot.path(), &decider, &recorder, &recorded);

    assert_eq!(culled.red, 1, "the pass did the work it would have written");
    let documents = recorder.documents();
    assert_eq!(documents.len(), 2, "one block per frame of the group");
    for (_, document) in &documents {
        assert!(document.contains("imagecull:groupKeeper"), "{document}");
    }
    assert!(
        !shoot.path().join("DSC_0001.xmp").exists() && !shoot.path().join("DSC_0002.xmp").exists(),
        "nothing reached the shoot folder"
    );
}

#[test]
fn a_verdict_that_cannot_be_written_is_counted_and_named() {
    let shoot = two_frames();
    let decider = Scripted::answering(Vec::new()).comparing(vec![Ok(keeper("2", 0.81))]);
    let recorded = quiet();

    let culled = cull(shoot.path(), &decider, &Refusing, &recorded);

    assert_eq!(culled.unwritten, 2);
    let unwritten = recorded.unwritten();
    assert_eq!(unwritten.len(), 2);
    assert!(
        unwritten[0].0 == "DSC_0001.NEF" && unwritten[0].1.contains("cannot write"),
        "each frame it could not mark is named with the reason: {unwritten:?}"
    );
}

#[test]
fn a_server_that_fails_the_preflight_marks_nothing() {
    let shoot = two_frames();
    let decider = Scripted::refusing_preflight(Error::new(
        ErrorKind::Http400,
        "POST /v1/systemone returned 400: this model is not a decision model",
    ));
    let recorded = quiet();

    let error = cull::run(
        &plan(shoot.path()),
        &cull::Config {
            model: "clef-flash-oQ4e",
            shoot_context: None,
            budget: keeper::Budget::default(),
            params: Params::default(),
            grouping_hash: "hash",
            allow_foreign: false,
            retry: RetryPolicy::without_waiting(),
        },
        &decider,
        &sidecar::Filesystem,
        &recorded,
    )
    .expect_err("a decider that cannot answer stops the pass");

    assert_eq!(error.kind(), Some(ErrorKind::Http400));
    assert!(
        !shoot.path().join("DSC_0001.xmp").exists(),
        "nothing is marked before the server has shown it can answer"
    );
}
