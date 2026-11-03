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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use image::RgbImage;
use imagecull_core::cull::{self, Culled, Event, Watcher};
use imagecull_core::error::Error;
use imagecull_core::ingest::ScanOptions;
use imagecull_core::preprocess::Params;
use imagecull_core::scoring::{Decider, Request, Response, RetryPolicy};
use imagecull_core::sidecar::{Prior, Writer, Written};
use imagecull_core::{ErrorKind, group, keeper, sidecar};
use support::ExifFacts;
use support::decider::{Guarded, Scripted, keeper};
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
        keeper::default_preview(),
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
    cull_with(
        dir,
        decider,
        &sidecar::Filesystem,
        recorded,
        budget,
        keeper::default_preview(),
        false,
    )
}

fn cull_with(
    dir: &Path,
    decider: &Scripted,
    writer: &dyn Writer,
    recorded: &Recorded,
    budget: keeper::Budget,
    preview: Params,
    allow_foreign: bool,
) -> Culled {
    let plan = plan(dir);
    let config = cull::Config {
        model: "clef-flash-oQ4e",
        shoot_context: None,
        budget,
        preview,
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
fn the_comparisons_preview_size_is_the_runs_to_set() {
    let shoot = two_frames();
    let decider = Scripted::answering(Vec::new()).comparing(vec![Ok(keeper("1", 0.88))]);
    let recorded = quiet();

    let culled = cull_with(
        shoot.path(),
        &decider,
        &sidecar::Filesystem,
        &recorded,
        keeper::Budget::default(),
        Params {
            max_edge: 256,
            jpeg_quality: 40,
        },
        false,
    );

    let compared = decider.compared();
    assert_eq!(
        compared[0].previews[0].width, 256,
        "a comparison is rendered at the edge the run asked for, not at the measured 512"
    );
    assert_eq!(compared[0].previews[0].height, 171, "and keeps the shape");
    assert_eq!(
        culled.green, 1,
        "the chosen frame is marked and the other is not"
    );
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
        keeper.contains("xmp:Label=\"Green\"")
            && keeper.contains("imagecull:labeledBy=\"imagecull\""),
        "the keeper is green: the comparison's answer is the mark: {keeper}"
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
            green: 1,
            rounds: 1,
            verdicts: vec![cull::Verdict {
                number: 1,
                start: "DSC_0001.NEF".to_string(),
                size: 3,
                keeper: Some("DSC_0002.NEF".to_string()),
                red: 2,
                yellow: 0,
                green: 1,
                unmarked: 0,
                keeper_unmarked: 0,
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
    // Two frames per request over five frames: batches of two, two and one. The
    // one-frame remainder is carried, not asked about, so the pass spends four
    // requests: two in the first round, one in the second, one in the final.
    let decider = Scripted::answering(Vec::new()).comparing(vec![
        Ok(keeper("1", 0.90)),
        Ok(keeper("1", 0.80)),
        Ok(keeper("1", 0.70)),
        Ok(keeper("2", 0.60)),
    ]);
    let recorded = quiet();
    let budget = keeper::Budget {
        max: 2,
        ..keeper::Budget::default()
    };

    let culled = cull_through(shoot.path(), &decider, &recorded, budget);

    let compared = decider.compared();
    assert_eq!(compared.len(), 4, "four requests: {compared:?}");
    assert!(
        compared.iter().all(|request| request.previews.len() == 2),
        "no request carries a single frame, because a question with one answer is not a \
         comparison: {:?}",
        compared
            .iter()
            .map(|request| request.previews.len())
            .collect::<Vec<_>>()
    );
    let verdict = recorded.verdicts().remove(0);
    assert_eq!(verdict.rounds, 4, "four requests stood behind the verdict");
    assert_eq!(
        verdict.confidence, 0.60,
        "the shakiest call on the winner's path, not the final call's certainty"
    );
    assert_eq!(
        verdict.keeper.as_deref(),
        Some("DSC_0005.NEF"),
        "the survivor of the final round"
    );
    assert_eq!(culled.rounds, 4);
    assert_eq!(culled.red, 4, "every frame that is not the keeper is red");

    // Every frame of the group carries the verdict, including the frames a
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
            text.contains("imagecull:pickRounds=\"4\""),
            "{name}: {text}"
        );
    }
}

#[test]
fn a_burst_of_twelve_against_a_four_image_cap_is_decided_in_batches_of_four() {
    // Workers AI takes four images per request. A twelve-frame burst is three
    // batches of four, and their three winners are one more batch: four
    // comparative requests, one decided group, and not a single frame left
    // uncompared.
    let names = [
        "DSC_0001.NEF",
        "DSC_0002.NEF",
        "DSC_0003.NEF",
        "DSC_0004.NEF",
        "DSC_0005.NEF",
        "DSC_0006.NEF",
        "DSC_0007.NEF",
        "DSC_0008.NEF",
        "DSC_0009.NEF",
        "DSC_0010.NEF",
        "DSC_0011.NEF",
        "DSC_0012.NEF",
    ];
    let shoot = shoot_of(&[
        frame(names[0], 0.00, 0, "10"),
        frame(names[1], 0.05, 0, "11"),
        frame(names[2], 0.10, 0, "12"),
        frame(names[3], 0.15, 0, "13"),
        frame(names[4], 0.20, 0, "14"),
        frame(names[5], 0.25, 0, "15"),
        frame(names[6], 0.30, 0, "16"),
        frame(names[7], 0.35, 0, "17"),
        frame(names[8], 0.40, 0, "18"),
        frame(names[9], 0.45, 0, "19"),
        frame(names[10], 0.50, 0, "20"),
        frame(names[11], 0.55, 0, "21"),
    ]);
    let decider = Scripted::answering(Vec::new()).comparing(vec![
        Ok(keeper("1", 0.91)),
        Ok(keeper("1", 0.88)),
        Ok(keeper("1", 0.86)),
        Ok(keeper("2", 0.80)),
    ]);
    let recorded = quiet();
    let budget = keeper::Budget {
        max: 4,
        ..keeper::Budget::default()
    };

    let culled = cull_through(shoot.path(), &decider, &recorded, budget);

    let compared = decider.compared();
    assert_eq!(
        compared
            .iter()
            .map(|request| request.previews.len())
            .collect::<Vec<_>>(),
        [4, 4, 4, 3],
        "three batches of four, then the three winners: no request carries a lone frame and \
         none exceeds the cap"
    );
    let verdict = recorded.verdicts().remove(0);
    assert_eq!(
        verdict.keeper.as_deref(),
        Some("DSC_0005.NEF"),
        "the survivor of the final batch, one decided keeper for the whole group"
    );
    assert_eq!(
        verdict.confidence, 0.80,
        "the final call is the shakiest step"
    );
    assert_eq!(culled.undecided, 0, "the cap costs requests, not groups");
    assert_eq!(culled.rounds, 4, "three batches and the final one");
    assert_eq!(culled.red, 11, "every frame that is not the keeper is red");
    assert_eq!(culled.green, 1);
}

#[test]
fn a_losing_batchs_coin_flip_does_not_shadow_the_winners_path() {
    let shoot = shoot_of(&[
        frame("DSC_0001.NEF", 0.0, 0, "10"),
        frame("DSC_0002.NEF", 0.2, 0, "15"),
        frame("DSC_0003.NEF", 0.4, 0, "20"),
        frame("DSC_0004.NEF", 0.6, 0, "25"),
        frame("DSC_0005.NEF", 0.8, 0, "30"),
    ]);
    // Three frames per request: batches of three and two. The first batch is a
    // coin flip whose winner then loses; the second is sure, and so is the
    // final. The verdict depended on 0.90 and 0.95, not on 0.40.
    let decider = Scripted::answering(Vec::new()).comparing(vec![
        Ok(keeper("3", 0.40)),
        Ok(keeper("1", 0.90)),
        Ok(keeper("2", 0.95)),
    ]);
    let recorded = quiet();
    let budget = keeper::Budget {
        max: 3,
        ..keeper::Budget::default()
    };

    let culled = cull_through(shoot.path(), &decider, &recorded, budget);

    let verdict = recorded.verdicts().remove(0);
    assert_eq!(
        verdict.confidence, 0.90,
        "the shakiest call on the winner's own path: a batch it did not survive is not evidence \
         about it"
    );
    assert_eq!(
        culled.low_confidence, 0,
        "and the group is not called unsure"
    );
    assert_eq!(
        verdict.keeper.as_deref(),
        Some("DSC_0004.NEF"),
        "the second batch's winner took the final"
    );
}

#[test]
fn a_group_the_model_was_unsure_about_marks_its_losers_yellow_and_counts_it() {
    let shoot = two_frames();
    let decider = Scripted::answering(Vec::new()).comparing(vec![Ok(keeper("1", 0.42))]);
    let recorded = quiet();

    let culled = cull(shoot.path(), &decider, &sidecar::Filesystem, &recorded);

    assert_eq!(
        culled.low_confidence, 1,
        "a near-tie is the common case for a group, and it is reported rather than suppressed"
    );
    assert_eq!(
        culled.yellow, 1,
        "the non-keeper is marked anyway, but yellow: a near-tie is worth a look before deleting"
    );
    assert_eq!(culled.red, 0, "a low-confidence group marks no frame red");
    assert!(
        sidecar_of(shoot.path(), "DSC_0002.NEF").contains("xmp:Label=\"Yellow\""),
        "yellow says the comparison was unsure, and the frame is still the one it did not keep"
    );
    assert!(
        sidecar_of(shoot.path(), "DSC_0001.NEF").contains("xmp:Label=\"Green\""),
        "the pick is green whatever the confidence, because it is still the pick"
    );
}

#[test]
fn a_confident_run_rewrites_the_yellow_an_unsure_run_wrote() {
    let shoot = two_frames();
    let unsure = Scripted::answering(Vec::new()).comparing(vec![Ok(keeper("1", 0.42))]);
    cull(shoot.path(), &unsure, &sidecar::Filesystem, &quiet());
    assert!(
        sidecar_of(shoot.path(), "DSC_0002.NEF").contains("xmp:Label=\"Yellow\""),
        "the first run was a near-tie, so it marked the loser yellow"
    );

    let sure = Scripted::answering(Vec::new()).comparing(vec![Ok(keeper("1", 0.82))]);
    let culled = cull(shoot.path(), &sure, &sidecar::Filesystem, &quiet());

    assert!(
        sidecar_of(shoot.path(), "DSC_0002.NEF").contains("xmp:Label=\"Red\""),
        "a later, confident run is free to rewrite the yellow it wrote as red"
    );
    assert!(
        sidecar_of(shoot.path(), "DSC_0001.NEF").contains("xmp:Label=\"Green\""),
        "and the keeper stays green"
    );
    assert_eq!(culled.yellow, 0, "this run marked no frame yellow");
    assert_eq!(culled.red, 1, "it marked one red");
    assert_eq!(culled.green, 1);
}

#[test]
fn a_keepers_photographer_label_is_left_alone_and_counted() {
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
    let decider = Scripted::answering(Vec::new()).comparing(vec![Ok(keeper("1", 0.80))]);
    let culled = cull(shoot.path(), &decider, &sidecar::Filesystem, &quiet());

    let keeper = sidecar_of(shoot.path(), "DSC_0001.NEF");
    assert!(
        keeper.contains("xmp:Label=\"Yellow\"") && !keeper.contains("imagecull:labeledBy"),
        "a photographer's colour is never overwritten, not even to make the pick green: {keeper}"
    );
    assert_eq!(
        culled.keeper_unmarked, 1,
        "the keeper the run could not colour is counted, so a grid without a green is explained"
    );
    assert_eq!(culled.green, 0);
    assert!(
        sidecar_of(shoot.path(), "DSC_0002.NEF").contains("xmp:Label=\"Red\""),
        "and the frame that lost is still marked"
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
fn a_promoted_keeper_turns_green_and_the_frame_that_lost_turns_red() {
    let shoot = two_frames();
    let first = Scripted::answering(Vec::new()).comparing(vec![Ok(keeper("2", 0.62))]);
    cull(shoot.path(), &first, &sidecar::Filesystem, &quiet());
    assert!(
        sidecar_of(shoot.path(), "DSC_0001.NEF").contains("xmp:Label=\"Red\""),
        "the first cull made the first frame red"
    );
    assert!(
        sidecar_of(shoot.path(), "DSC_0002.NEF").contains("xmp:Label=\"Green\""),
        "and the first keeper green"
    );

    let second = Scripted::answering(Vec::new()).comparing(vec![Ok(keeper("1", 0.68))]);
    let recorded = quiet();
    let culled = cull(shoot.path(), &second, &sidecar::Filesystem, &recorded);

    let promoted = sidecar_of(shoot.path(), "DSC_0001.NEF");
    assert!(
        promoted.contains("xmp:Label=\"Green\"")
            && promoted.contains("imagecull:labeledBy=\"imagecull\""),
        "the red this tool wrote is rewritten as green, so the keeper is the frame a cull keeps: \
         {promoted}"
    );
    assert!(
        promoted.contains("imagecull:groupKeeper=\"true\""),
        "and the verdict says what this run decided: {promoted}"
    );
    assert_eq!(culled.green, 1, "the promotion is counted green");
    assert_eq!(
        culled.retired, 0,
        "a decided group rewrites a colour, it does not remove one"
    );
    assert!(
        sidecar_of(shoot.path(), "DSC_0002.NEF").contains("xmp:Label=\"Red\""),
        "and the frame that lost this time is red now"
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

/// A cull config for a grouping hash, with the measured budget and no waiting.
fn stream_config(hash: &str) -> cull::Config<'_> {
    cull::Config {
        model: "clef-flash-oQ4e",
        shoot_context: None,
        budget: keeper::Budget::default(),
        preview: Params::default(),
        grouping_hash: hash,
        allow_foreign: false,
        retry: RetryPolicy::without_waiting(),
    }
}

#[test]
fn a_streamed_cull_judges_every_group_and_writes_its_verdicts() {
    let shoot = shoot_of(&[
        frame("DSC_0001.NEF", 0.0, 0, "10"),
        frame("DSC_0002.NEF", 0.2, 0, "20"),
        frame("DSC_0003.NEF", 0.0, 5, "10"),
        frame("DSC_0004.NEF", 0.2, 5, "20"),
    ]);
    let decider = Guarded::new(
        Scripted::answering(Vec::new()).comparing(vec![Ok(keeper("1", 0.9)), Ok(keeper("1", 0.9))]),
    );
    let recorded = quiet();
    let hash = group::grouping_hash(&group::Thresholds::default());
    let config = stream_config(&hash);

    let streamed = cull::run_streaming(
        &config,
        &decider,
        &sidecar::Filesystem,
        &recorded,
        2,
        |sink| {
            group::stream_with(
                shoot.path(),
                &group::Options {
                    jobs: Some(1),
                    ..group::Options::default()
                },
                &imagecull_core::progress::Silent,
                sink,
            )
        },
    )
    .expect("a streamed cull");

    assert_eq!(streamed.plan.groups, 2);
    assert_eq!(streamed.culled.groups, 2, "both groups were judged");
    assert_eq!(streamed.culled.red, 2, "one non-keeper per group");
    let verdicts = &streamed.culled.verdicts;
    assert_eq!(verdicts.len(), 2);
    assert_eq!(
        verdicts
            .iter()
            .map(|verdict| verdict.number)
            .collect::<Vec<_>>(),
        vec![1, 2],
        "the report's verdicts are in group order however the workers finished"
    );
    assert_eq!(decider.get().compared().len(), 2, "one request per group");
    assert!(
        sidecar_of(shoot.path(), "DSC_0002.NEF").contains("xmp:Label=\"Red\""),
        "the non-keeper of the first group wears the red"
    );
    assert!(
        sidecar_of(shoot.path(), "DSC_0004.NEF").contains("xmp:Label=\"Red\""),
        "the non-keeper of the second group wears the red"
    );
}

/// A decider that answers every group with the same choice, slowly enough that
/// two requests can be in flight together, and counts how many were.
struct Slow {
    in_flight: AtomicUsize,
    most: AtomicUsize,
}

impl Slow {
    fn new() -> Slow {
        Slow {
            in_flight: AtomicUsize::new(0),
            most: AtomicUsize::new(0),
        }
    }
}

impl Decider for Slow {
    fn decide(&self, _request: &Request<'_>) -> imagecull_core::Result<Response> {
        panic!("a cull never rates a frame");
    }

    fn compare(
        &self,
        _comparison: &imagecull_core::keeper::Comparison,
    ) -> imagecull_core::Result<Response> {
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.most.fetch_max(now, Ordering::SeqCst);
        thread::sleep(Duration::from_millis(300));
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        Ok(Response {
            document: keeper("1", 0.9),
            client_latency_ms: 0,
        })
    }
}

#[test]
fn a_streamed_cull_has_several_groups_in_flight_and_waits_for_every_one() {
    let shoot = shoot_of(&[
        frame("DSC_0001.NEF", 0.0, 0, "10"),
        frame("DSC_0002.NEF", 0.2, 0, "20"),
        frame("DSC_0003.NEF", 0.0, 5, "10"),
        frame("DSC_0004.NEF", 0.2, 5, "20"),
        frame("DSC_0005.NEF", 0.0, 10, "10"),
        frame("DSC_0006.NEF", 0.2, 10, "20"),
        frame("DSC_0007.NEF", 0.0, 15, "10"),
        frame("DSC_0008.NEF", 0.2, 15, "20"),
    ]);
    let decider = Slow::new();
    let recorded = quiet();
    let hash = group::grouping_hash(&group::Thresholds::default());
    let config = stream_config(&hash);

    let streamed = cull::run_streaming(
        &config,
        &decider,
        &sidecar::Filesystem,
        &recorded,
        4,
        |sink| {
            group::stream_with(
                shoot.path(),
                &group::Options {
                    jobs: Some(4),
                    ..group::Options::default()
                },
                &imagecull_core::progress::Silent,
                sink,
            )
        },
    )
    .expect("a streamed cull");

    assert_eq!(streamed.culled.groups, 4, "every group was judged");
    assert!(
        decider.most.load(Ordering::SeqCst) >= 2,
        "two groups' comparisons were in flight at once, not one after another"
    );
    assert_eq!(
        decider.in_flight.load(Ordering::SeqCst),
        0,
        "the pass returned only after every comparison finished"
    );
}

#[test]
fn a_streamed_cull_retires_a_singletons_red_when_it_is_regrouped() {
    // A first run decides the pair in `shoot` and leaves a red on its first frame.
    let shoot = two_frames();
    let first = Scripted::answering(Vec::new()).comparing(vec![Ok(keeper("2", 0.62))]);
    cull(shoot.path(), &first, &sidecar::Filesystem, &quiet());
    assert!(
        sidecar_of(shoot.path(), "DSC_0001.NEF").contains("xmp:Label=\"Red\""),
        "the first cull made the first frame red"
    );

    // The folder is regrouped: DSC_0001 is now alone, and DSC_0002 and DSC_0003
    // are one moment. The lone frame is emitted before the group, so its
    // retirement is judged alongside it.
    let regrouped = shoot_of(&[
        frame("DSC_0001.NEF", 0.0, 0, "10"),
        frame("DSC_0002.NEF", 0.0, 10, "10"),
        frame("DSC_0003.NEF", 0.2, 10, "20"),
    ]);
    fs::copy(
        sidecar::path_for(&shoot.path().join("DSC_0001.NEF")),
        sidecar::path_for(&regrouped.path().join("DSC_0001.NEF")),
    )
    .expect("copy the verdict");
    let decider =
        Guarded::new(Scripted::answering(Vec::new()).comparing(vec![Ok(keeper("1", 0.9))]));
    let recorded = quiet();
    let hash = group::grouping_hash(&group::Thresholds::default());
    let config = stream_config(&hash);

    let streamed = cull::run_streaming(
        &config,
        &decider,
        &sidecar::Filesystem,
        &recorded,
        2,
        |sink| {
            group::stream_with(
                regrouped.path(),
                &group::Options {
                    jobs: Some(1),
                    ..group::Options::default()
                },
                &imagecull_core::progress::Silent,
                sink,
            )
        },
    )
    .expect("a streamed cull");

    assert_eq!(streamed.culled.groups, 1, "only the pair was judged");
    assert_eq!(streamed.culled.retired, 1, "the lone frame lost our red");
    let text = sidecar_of(regrouped.path(), "DSC_0001.NEF");
    assert!(
        !text.contains("xmp:Label") && !text.contains("imagecull:groupKeeper"),
        "a red for a comparison that no longer exists is retired: {text}"
    );
    assert!(
        sidecar_of(regrouped.path(), "DSC_0003.NEF").contains("xmp:Label=\"Red\""),
        "the pair's non-keeper still wears the red"
    );
}
