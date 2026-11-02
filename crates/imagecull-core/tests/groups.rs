//! Grouping, at the seam that matters: a shoot folder in, groups and measures
//! out.
//!
//! Every test here writes a few synthetic frames to disk — NEF-shaped files
//! whose Exif IFD carries a real capture time, body serial and exposure, and
//! whose embedded preview is a known transform of one scene — then runs the
//! model-free entry point over the folder. Nothing here starts a server, and
//! nothing here could: `group` has no decider to call.

mod support;

use std::path::Path;

use image::RgbImage;
use imagecull_core::group::{self, Thresholds};
use imagecull_core::ingest::ScanOptions;
use imagecull_core::measure::Measures;
use support::{ExifFacts, scenes};

/// The size the grouping tests draw at: small, because a group is about times,
/// serials and settings, and every pixel here costs the suite a decode.
const WIDTH: u32 = 640;
const HEIGHT: u32 = 427;

/// What one frame of a synthetic sequence is.
struct Frame {
    name: &'static str,
    image: RgbImage,
    facts: ExifFacts,
}

/// A frame taken at `second` (and `subsecond` hundredths of it) by `serial`, at
/// `iso`, showing `image`.
fn frame(
    name: &'static str,
    image: &RgbImage,
    second: u8,
    subsecond: &str,
    serial: &str,
    iso: u16,
) -> Frame {
    Frame {
        name,
        image: image.clone(),
        facts: ExifFacts {
            captured: Some(format!("2026:11:02 10:00:{second:02}")),
            subsecond: Some(subsecond.to_string()),
            serial: Some(serial.to_string()),
            iso: Some(iso),
            exposure_time: Some((1, 500)),
            f_number: Some((28, 10)),
            focal_length: Some((50, 1)),
        },
    }
}

/// A frame of the default body at the default exposure: what almost every test
/// here needs, without the two arguments it never varies.
fn moment(name: &'static str, image: &RgbImage, second: u8, subsecond: &str) -> Frame {
    frame(name, image, second, subsecond, "3001", 200)
}

/// The scene every frame of a sequence shows, freshly drawn.
fn scene() -> RgbImage {
    scenes::scene(WIDTH, HEIGHT)
}

/// Write `frames` as NEFs into a fresh folder, in the order given.
fn shoot(frames: Vec<Frame>) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temp dir");
    for frame in &frames {
        let jpeg = support::jpeg_of(&frame.image, 92);
        let nef = support::nef_with(1, &[jpeg], &frame.facts);
        std::fs::write(dir.path().join(frame.name), nef).expect("write a NEF");
    }
    dir
}

/// Run the grouping pass over `dir`, with the default thresholds unless given.
fn plan(dir: &Path, thresholds: Thresholds) -> group::Plan {
    group::run(
        dir,
        &group::Options {
            scan: ScanOptions::default(),
            thresholds,
            jobs: None,
        },
    )
    .expect("a folder that scans")
}

/// Run the grouping pass at a fixed degree of parallelism.
fn plan_at(dir: &Path, jobs: usize) -> group::Plan {
    group::run(
        dir,
        &group::Options {
            jobs: Some(jobs),
            ..group::Options::default()
        },
    )
    .expect("a folder that scans")
}

/// The group each frame landed in, by file name, in the plan's own order.
fn groups_by_name(plan: &group::Plan) -> Vec<(String, usize, usize, usize)> {
    plan.frames
        .iter()
        .map(|frame| {
            (
                frame.source.relative.display().to_string(),
                frame.group,
                frame.group_index,
                frame.group_size,
            )
        })
        .collect()
}

/// The measures of the frame whose file name is `name`.
fn measures_of(plan: &group::Plan, name: &str) -> Measures {
    plan.frames
        .iter()
        .find(|frame| frame.source.relative == Path::new(name))
        .unwrap_or_else(|| panic!("{name} is in the plan"))
        .measures
}

#[test]
fn frames_inside_the_gap_with_one_body_and_one_scene_are_one_group() {
    let image = scene();
    let dir = shoot(vec![
        moment("DSC_0001.NEF", &image, 0, "10"),
        moment("DSC_0002.NEF", &image, 0, "20"),
        moment("DSC_0003.NEF", &image, 0, "30"),
        moment("DSC_0004.NEF", &image, 0, "40"),
    ]);

    let plan = plan(dir.path(), Thresholds::default());

    assert_eq!(plan.groups, 1);
    assert_eq!(
        groups_by_name(&plan),
        (1..=4)
            .map(|index| (format!("DSC_{index:04}.NEF"), 1, index, 4))
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_gap_longer_than_the_threshold_starts_a_new_group() {
    let image = scene();
    let dir = shoot(vec![
        moment("DSC_0001.NEF", &image, 0, "00"),
        moment("DSC_0002.NEF", &image, 0, "10"),
        moment("DSC_0003.NEF", &image, 3, "50"),
    ]);

    let plan = plan(dir.path(), Thresholds::default());

    assert_eq!(
        groups_by_name(&plan),
        vec![
            ("DSC_0001.NEF".to_string(), 1, 1, 2),
            ("DSC_0002.NEF".to_string(), 1, 2, 2),
            ("DSC_0003.NEF".to_string(), 2, 1, 1),
        ]
    );
}

#[test]
fn a_different_body_is_a_different_moment_and_so_is_a_drift_past_the_tolerance() {
    let image = scene();
    let dir = shoot(vec![
        moment("DSC_0001.NEF", &image, 0, "00"),
        frame("DSC_0002.NEF", &image, 0, "10", "3002", 200),
        frame("DSC_0003.NEF", &image, 0, "20", "3002", 1600),
    ]);

    // The body gate splits the first frame from the other two; the exposure gate
    // then splits those two, three stops apart against a two-stop tolerance.
    let strict = plan(dir.path(), Thresholds::default());

    assert_eq!(strict.groups, 3, "{:?}", groups_by_name(&strict));
    for frame in &strict.frames {
        assert_eq!(frame.group_size, 1);
    }

    // And the tolerance is the reason: widen it past three stops and the two
    // frames of one body are one moment again.
    let tolerant = plan(
        dir.path(),
        Thresholds {
            exposure_stops: 3.5,
            ..Thresholds::default()
        },
    );
    assert_eq!(tolerant.groups, 2, "{:?}", groups_by_name(&tolerant));
}

#[test]
fn a_pan_splits_the_sequence_into_the_compositions_it_produced() {
    // The camera held still, then swung: the same body, the same settings and
    // no gap, but a different picture.
    let before = scene();
    let after = scenes::noise(WIDTH, HEIGHT, 11);
    let dir = shoot(vec![
        moment("DSC_0001.NEF", &before, 0, "00"),
        moment("DSC_0002.NEF", &before, 0, "10"),
        moment("DSC_0003.NEF", &after, 0, "20"),
        moment("DSC_0004.NEF", &after, 0, "30"),
    ]);

    let plan = plan(dir.path(), Thresholds::default());

    assert_eq!(
        groups_by_name(&plan),
        vec![
            ("DSC_0001.NEF".to_string(), 1, 1, 2),
            ("DSC_0002.NEF".to_string(), 1, 2, 2),
            ("DSC_0003.NEF".to_string(), 2, 1, 2),
            ("DSC_0004.NEF".to_string(), 2, 2, 2),
        ]
    );
}

#[test]
fn a_frame_with_no_capture_time_is_a_group_of_its_own() {
    let image = scene();
    let mut undated = moment("DSC_0002.NEF", &image, 0, "10");
    undated.facts.captured = None;
    undated.facts.subsecond = None;
    let dir = shoot(vec![
        moment("DSC_0001.NEF", &image, 0, "00"),
        undated,
        moment("DSC_0003.NEF", &image, 0, "20"),
    ]);

    let plan = plan(dir.path(), Thresholds::default());

    // The two dated frames are one moment; the undated one is not guessed at by
    // its name and joins nothing.
    assert_eq!(
        groups_by_name(&plan),
        vec![
            ("DSC_0001.NEF".to_string(), 1, 1, 2),
            ("DSC_0003.NEF".to_string(), 1, 2, 2),
            ("DSC_0002.NEF".to_string(), 2, 1, 1),
        ]
    );
}

#[test]
fn groups_follow_capture_order_and_not_the_order_of_their_names() {
    // `DSC_9` sorts before `DSC_10` in ingest order, but it was taken second:
    // the plan has to read as the shoot happened.
    let image = scene();
    let dir = shoot(vec![
        moment("DSC_10.NEF", &image, 0, "00"),
        moment("DSC_9.NEF", &image, 0, "10"),
    ]);

    let plan = plan(dir.path(), Thresholds::default());

    assert_eq!(
        plan.frames
            .iter()
            .map(|frame| frame.source.relative.display().to_string())
            .collect::<Vec<_>>(),
        vec!["DSC_10.NEF".to_string(), "DSC_9.NEF".to_string()]
    );
    assert_eq!(plan.groups, 1);
}

#[test]
fn renaming_the_files_changes_nothing_about_the_groups() {
    let image = scene();
    let first = shoot(vec![
        moment("AAA_0001.NEF", &image, 0, "00"),
        moment("AAA_0002.NEF", &image, 0, "10"),
        moment("AAA_0003.NEF", &image, 5, "00"),
    ]);
    let second = shoot(vec![
        moment("ZZZ_9999.NEF", &image, 0, "00"),
        moment("ZZZ_0001.NEF", &image, 0, "10"),
        moment("MMM_0042.NEF", &image, 5, "00"),
    ]);

    let left = plan(first.path(), Thresholds::default());
    let right = plan(second.path(), Thresholds::default());

    let shape = |plan: &group::Plan| {
        plan.frames
            .iter()
            .map(|frame| (frame.group, frame.group_index, frame.group_size))
            .collect::<Vec<_>>()
    };
    assert_eq!(shape(&left), shape(&right));
    assert_eq!(
        left.hash, right.hash,
        "the hash names the algorithm and its thresholds, never a file"
    );
}

#[test]
fn two_runs_over_the_same_folder_agree_column_for_column() {
    let dir = shoot(vec![
        moment("DSC_0001.NEF", &scene(), 0, "00"),
        moment("DSC_0002.NEF", &scenes::blurred(&scene(), 1.0), 0, "10"),
        moment("DSC_0003.NEF", &scene(), 0, "20"),
    ]);

    let first = group::render_csv(&plan(dir.path(), Thresholds::default()));
    let second = group::render_csv(&plan(dir.path(), Thresholds::default()));

    assert_eq!(first, second);
}

#[test]
fn a_burst_that_drifted_its_exposure_is_still_one_moment() {
    // The same second, the same body, the same picture: only the exposure walked.
    // Two stops apart is one moment by default and two by `--group-exposure-stops
    // 0`, which is the tolerance doing exactly one thing (ADR 0009).
    let image = scene();
    let exposed = |name: &'static str, iso: u16, shutter: (u32, u32), focal: (u32, u32)| Frame {
        name,
        image: image.clone(),
        facts: ExifFacts {
            captured: Some("2026:11:02 10:00:00".to_string()),
            subsecond: Some("00".to_string()),
            serial: Some("3001".to_string()),
            iso: Some(iso),
            exposure_time: Some(shutter),
            f_number: Some((28, 10)),
            focal_length: Some(focal),
        },
    };

    let drifted = shoot(vec![
        exposed("DSC_0001.NEF", 200, (1, 500), (50, 1)),
        exposed("DSC_0002.NEF", 400, (1, 500), (50, 1)),
    ]);
    assert_eq!(
        plan(drifted.path(), Thresholds::default()).groups,
        1,
        "a stop of ISO drift is one moment at the default tolerance"
    );
    assert_eq!(
        plan(
            drifted.path(),
            Thresholds {
                exposure_stops: 0.0,
                ..Thresholds::default()
            }
        )
        .groups,
        2,
        "zero stops asks for one exposure exactly"
    );

    // And the tolerance is a tolerance, not a blindfold: a zoom is a different
    // picture however much drift is allowed.
    let zoomed = shoot(vec![
        exposed("DSC_0001.NEF", 200, (1, 500), (50, 1)),
        exposed("DSC_0002.NEF", 200, (1, 500), (85, 1)),
    ]);
    assert_eq!(
        plan(
            zoomed.path(),
            Thresholds {
                exposure_stops: 1e9,
                ..Thresholds::default()
            }
        )
        .groups,
        2,
        "a zoom is not one moment, whatever the exposure tolerance says"
    );
}

#[test]
fn the_grouping_hash_moves_with_a_threshold_and_not_with_a_name() {
    let default = Thresholds::default();
    let wider = Thresholds {
        gap: std::time::Duration::from_secs_f64(5.0),
        ..Thresholds::default()
    };
    let stricter = Thresholds {
        similarity: 0.99,
        ..Thresholds::default()
    };
    let tolerant = Thresholds {
        exposure_stops: 0.5,
        ..Thresholds::default()
    };

    assert_eq!(
        group::grouping_hash(&default),
        group::grouping_hash(&Thresholds::default())
    );
    assert_ne!(group::grouping_hash(&default), group::grouping_hash(&wider));
    assert_ne!(
        group::grouping_hash(&default),
        group::grouping_hash(&stricter)
    );
    assert_ne!(
        group::grouping_hash(&default),
        group::grouping_hash(&tolerant),
        "a sidecar verdict computed under one tolerance is not readable under \
         another"
    );
}

#[test]
fn the_synthetic_answer_key_orders_as_the_transforms_did() {
    // One group, five frames of the same moment, each a known transform of one
    // scene: the ranking issue 02 will do needs these numbers to come out in
    // the order the transforms say.
    let sharp = scene();
    let blown = scenes::brighter(&sharp, 2.0);
    let tilted = scenes::tilted_scene(WIDTH, HEIGHT, 1.5);
    let dir = shoot(vec![
        moment("DSC_0001.NEF", &sharp, 0, "00"),
        moment("DSC_0002.NEF", &scenes::blurred(&sharp, 1.0), 0, "10"),
        moment("DSC_0003.NEF", &scenes::shaken(&sharp, 5), 0, "20"),
        moment("DSC_0004.NEF", &blown, 0, "30"),
        moment("DSC_0005.NEF", &tilted, 0, "40"),
    ]);

    // The tilted frame is a different picture — a flat scene drawn at an angle,
    // because rotating this one would quantise its edges — so it is a group of
    // its own. Grouping is pinned by its own tests; what this one reads is the
    // measures, in the order the transforms put them.
    let plan = plan(dir.path(), Thresholds::default());
    assert_eq!(plan.frames.len(), 5);

    let sharp_measures = measures_of(&plan, "DSC_0001.NEF");
    let soft = measures_of(&plan, "DSC_0002.NEF");
    let shaken = measures_of(&plan, "DSC_0003.NEF");
    let blown = measures_of(&plan, "DSC_0004.NEF");
    let tilted = measures_of(&plan, "DSC_0005.NEF");

    assert!(
        sharp_measures.acutance > soft.acutance,
        "the sharp frame outranks the soft one: {:?} vs {:?}",
        sharp_measures.acutance,
        soft.acutance
    );
    assert!(
        sharp_measures.acutance > shaken.acutance,
        "and the shaken one: {:?} vs {:?}",
        sharp_measures.acutance,
        shaken.acutance
    );
    assert!(
        shaken.shake > soft.shake,
        "shake tells the moving camera from the lens that missed: {:?} vs {:?}",
        shaken.shake,
        soft.shake
    );
    assert!(
        blown.clip_hi > sharp_measures.clip_hi,
        "+2 EV clips where the sharp frame does not: {:?} vs {:?}",
        blown.clip_hi,
        sharp_measures.clip_hi
    );
    assert!(
        tilted.roll_deg.is_some_and(|degrees| degrees.abs() > 1.0),
        "the tilted frame's horizon is a number: {:?}",
        tilted.roll_deg
    );
}

#[test]
fn a_frame_with_no_readable_preview_is_a_singleton_with_empty_measures_and_a_status() {
    let image = scene();
    let dir = shoot(vec![moment("DSC_0001.NEF", &image, 0, "00")]);
    // A file whose preview the reader cannot get at: the frame is still a frame,
    // and its numbers are missing rather than zero.
    let previewless = support::nef_with(1, &[], &moment("x", &image, 0, "10").facts);
    std::fs::write(dir.path().join("DSC_0002.NEF"), previewless).expect("write a NEF");

    let plan = plan(dir.path(), Thresholds::default());
    let missing = plan
        .frames
        .iter()
        .find(|frame| frame.source.relative == Path::new("DSC_0002.NEF"))
        .expect("the previewless frame is in the plan");

    assert_eq!(
        missing.group_size, 1,
        "a frame it cannot read joins nothing"
    );
    assert_eq!(
        missing.measures,
        Measures::default(),
        "no zeros dressed as answers"
    );
    assert_eq!(
        missing.failure.as_ref().map(|failure| failure.kind),
        Some(imagecull_core::ErrorKind::NoPreview)
    );
    let csv = group::render_csv(&plan);
    assert!(
        csv.contains("DSC_0002.NEF,2,1,1,,,,,,,,,no_preview,"),
        "{csv}"
    );
}

#[test]
fn the_csv_keeps_one_row_per_frame_and_names_its_columns() {
    let image = scene();
    let dir = shoot(vec![
        moment("DSC_0001.NEF", &image, 0, "00"),
        moment("DSC_0002.NEF", &image, 0, "10"),
    ]);

    let csv = group::render_csv(&plan(dir.path(), Thresholds::default()));
    let lines: Vec<&str> = csv.lines().collect();

    assert_eq!(
        lines[0],
        group::COLUMNS.join(","),
        "the header is the columns, in order"
    );
    assert_eq!(lines.len(), 3, "a header and one row per frame");
    assert!(
        lines[1].starts_with("DSC_0001.NEF,1,1,2,"),
        "the row carries the group and the place in it: {}",
        lines[1]
    );
}

#[test]
fn the_cost_of_the_pass_is_measured_rather_than_assumed() {
    let image = scene();
    let dir = shoot(vec![
        moment("DSC_0001.NEF", &image, 0, "00"),
        moment("DSC_0002.NEF", &image, 0, "10"),
    ]);

    let plan = plan(dir.path(), Thresholds::default());

    assert_eq!(plan.frames.len(), 2);
    assert!(
        plan.elapsed > std::time::Duration::ZERO,
        "the plan records how long the pass took, so the evidence can quote it"
    );
    assert_eq!(plan.groups, 1);
}

#[test]
fn the_similarity_gate_is_the_threshold_and_not_a_property_of_the_scene() {
    // Same second, same body, same settings, two horizons a degree apart: the
    // only thing that decides whether they are one moment is the threshold, so
    // both sides of it are exercised.
    let before = scenes::tilted_scene(WIDTH, HEIGHT, 0.0);
    let nudged = scenes::tilted_scene(WIDTH, HEIGHT, 1.0);
    let dir = shoot(vec![
        moment("DSC_0001.NEF", &before, 0, "00"),
        moment("DSC_0002.NEF", &nudged, 0, "10"),
    ]);

    let strict = plan(
        dir.path(),
        Thresholds {
            similarity: 0.9999,
            ..Thresholds::default()
        },
    );
    let loose = plan(
        dir.path(),
        Thresholds {
            similarity: 0.10,
            ..Thresholds::default()
        },
    );

    assert_eq!(strict.groups, 2, "0.9999 asks for the same picture");
    assert_eq!(loose.groups, 1, "0.10 asks only for the same second");
}

#[test]
fn frames_that_share_a_timestamp_group_the_same_whatever_they_are_called() {
    // No subsecond stamps: the recorded times tie, so nothing about a name may
    // decide who is in which group.
    let image = scene();
    let tied = |name: &'static str| moment(name, &image, 0, "");
    let first = shoot(vec![
        tied("AAA_0001.NEF"),
        tied("AAA_0002.NEF"),
        moment("AAA_0003.NEF", &image, 9, ""),
    ]);
    let second = shoot(vec![
        tied("ZZZ_0009.NEF"),
        tied("MMM_0001.NEF"),
        moment("BBB_0042.NEF", &image, 9, ""),
    ]);

    let sizes = |plan: &group::Plan| {
        plan.grouped()
            .iter()
            .map(|row| row.frames.len())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        sizes(&plan(first.path(), Thresholds::default())),
        vec![2, 1]
    );
    assert_eq!(
        sizes(&plan(first.path(), Thresholds::default())),
        sizes(&plan(second.path(), Thresholds::default())),
        "a name cannot move a frame between groups"
    );
}

#[test]
fn reading_frames_at_a_time_changes_the_cost_and_nothing_else() {
    // A degree of parallelism is a scheduling decision: the same folder has to
    // give the same bytes at one job and at many, which is what makes the plan,
    // the CSV and the grouping hash comparable between machines.
    let image = scene();
    let other = scenes::tilted_scene(WIDTH, HEIGHT, 30.0);
    let dir = shoot(vec![
        moment("DSC_0001.NEF", &image, 0, "00"),
        moment("DSC_0002.NEF", &image, 0, "20"),
        moment("DSC_0003.NEF", &other, 0, "40"),
        moment("DSC_0004.NEF", &image, 3, "00"),
        moment("DSC_0005.NEF", &image, 3, "20"),
        moment("DSC_0006.NEF", &other, 9, "00"),
    ]);
    // A frame nothing can be read from keeps its row, and its place, at any
    // degree: a failure must not reorder the plan around it.
    std::fs::write(dir.path().join("DSC_0007.NEF"), b"not a raw file")
        .expect("write a broken file");

    let serial = plan_at(dir.path(), 1);
    for jobs in [2, 3, 8] {
        let parallel = plan_at(dir.path(), jobs);
        assert_eq!(parallel.jobs, jobs.min(serial.frames.len()));
        assert_eq!(
            group::render_csv(&parallel),
            group::render_csv(&serial),
            "{jobs} jobs wrote a different CSV"
        );
        assert_eq!(parallel.hash, serial.hash);
        assert_eq!(
            groups_by_name(&parallel),
            groups_by_name(&serial),
            "{jobs} jobs grouped the frames differently"
        );
    }
    assert_eq!(serial.jobs, 1);
    assert_eq!(
        serial.failures().count(),
        1,
        "the unreadable frame is the only failure"
    );
}

/// Everything a pass said, in the order it said it.
#[derive(Default)]
struct Told {
    events: std::cell::RefCell<Vec<group::Event>>,
}

impl group::Watcher for Told {
    fn event(&self, event: group::Event) {
        self.events.borrow_mut().push(event);
    }
}

#[test]
fn the_pass_says_each_frame_as_it_reads_it_in_the_passes_own_order() {
    // Five frames and one that cannot be read, read eight at a time: the lines
    // have to arrive in the scan's order, once each, with the status the CSV
    // will carry.
    let image = scene();
    let dir = shoot(vec![
        moment("DSC_0001.NEF", &image, 0, "00"),
        moment("DSC_0002.NEF", &image, 0, "20"),
        moment("DSC_0003.NEF", &image, 3, "00"),
        moment("DSC_0004.NEF", &image, 3, "20"),
        moment("DSC_0005.NEF", &image, 9, "00"),
    ]);
    std::fs::write(dir.path().join("DSC_0006.NEF"), b"not a raw file")
        .expect("write a broken file");

    let told = Told::default();
    group::run_with(
        dir.path(),
        &group::Options {
            jobs: Some(8),
            ..group::Options::default()
        },
        &told,
    )
    .expect("a folder that scans");

    let events = told.events.borrow();
    let group::Event::Started(started) = &events[0] else {
        panic!("a pass opens by saying how much work there is");
    };
    assert_eq!(started.total, 6);
    assert_eq!(started.jobs, 6, "never more workers than there are frames");

    let frames: Vec<&group::FrameEvent> = events[1..]
        .iter()
        .map(|event| match event {
            group::Event::Frame(frame) => frame,
            group::Event::Started(_) => panic!("a pass starts once"),
        })
        .collect();
    assert_eq!(frames.len(), 6, "one line per frame, and no more");
    assert_eq!(
        frames.iter().map(|frame| frame.index).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5, 6],
        "the lines arrive in the pass's order, not the workers'"
    );
    assert_eq!(
        frames
            .iter()
            .map(|frame| frame.filename.as_str())
            .collect::<Vec<_>>(),
        vec![
            "DSC_0001.NEF",
            "DSC_0002.NEF",
            "DSC_0003.NEF",
            "DSC_0004.NEF",
            "DSC_0005.NEF",
            "DSC_0006.NEF",
        ]
    );
    assert_eq!(frames[3].status, "ok");
    assert!(
        frames[5].failure.is_some(),
        "the frame nothing could be read from says why"
    );
    assert_ne!(frames[5].status, "ok");
    assert!(
        frames.iter().all(|frame| frame.total == 6),
        "every line knows the denominator"
    );
}

#[test]
fn the_lines_are_the_same_at_any_degree_of_parallelism() {
    let image = scene();
    let dir = shoot(vec![
        moment("DSC_0001.NEF", &image, 0, "00"),
        moment("DSC_0002.NEF", &image, 0, "20"),
        moment("DSC_0003.NEF", &image, 3, "00"),
        moment("DSC_0004.NEF", &image, 3, "20"),
        moment("DSC_0005.NEF", &image, 9, "00"),
    ]);
    std::fs::write(dir.path().join("DSC_0006.NEF"), b"not a raw file")
        .expect("write a broken file");

    let told = |jobs: usize| {
        let told = Told::default();
        group::run_with(
            dir.path(),
            &group::Options {
                jobs: Some(jobs),
                ..group::Options::default()
            },
            &told,
        )
        .expect("a folder that scans");
        let events = told.events.borrow().clone();
        // The degree is the one thing a pass may say differently; everything it
        // says about the frames has to be the same text.
        let (started, frames) = events.split_first().expect("a pass starts");
        assert_eq!(
            started,
            &group::Event::Started(group::Started { total: 6, jobs }),
            "{jobs} jobs announced the wrong work"
        );
        frames.to_vec()
    };

    let serial = told(1);
    for jobs in [2, 3, 6] {
        assert_eq!(
            told(jobs),
            serial,
            "{jobs} jobs said something about the frames a serial pass did not"
        );
    }
}

/// What a streaming pass handed a sink, in the order it handed them over.
#[derive(Default)]
struct Handed {
    groups: std::cell::RefCell<Vec<Vec<String>>>,
}

impl group::Sink for Handed {
    fn group(&mut self, found: group::Found) -> imagecull_core::Result<()> {
        self.groups.borrow_mut().push(
            found
                .frames
                .iter()
                .map(|frame| frame.source.relative.display().to_string())
                .collect(),
        );
        Ok(())
    }
}

#[test]
fn a_streaming_pass_hands_every_group_to_the_sink_in_the_plans_own_order() {
    let image = scene();
    let other = scenes::tilted_scene(WIDTH, HEIGHT, 30.0);
    let dir = shoot(vec![
        moment("DSC_0001.NEF", &image, 0, "00"),
        moment("DSC_0002.NEF", &image, 0, "20"),
        moment("DSC_0003.NEF", &other, 3, "00"),
        moment("DSC_0004.NEF", &other, 3, "20"),
        moment("DSC_0005.NEF", &image, 9, "00"),
    ]);

    let mut handed = Handed::default();
    let plan = group::stream_with(
        dir.path(),
        &group::Options {
            jobs: Some(4),
            ..group::Options::default()
        },
        &imagecull_core::progress::Silent,
        &mut handed,
    )
    .expect("a folder that scans");

    let planned: Vec<Vec<String>> = plan
        .grouped()
        .iter()
        .map(|row| {
            row.frames
                .iter()
                .map(|frame| frame.source.relative.display().to_string())
                .collect()
        })
        .collect();
    assert_eq!(
        handed.groups.borrow().clone(),
        planned,
        "the sink saw the plan's groups, in the plan's order, and nothing else"
    );
    assert_eq!(plan.groups, planned.len());
}

/// A sink that refuses its second group, however it is shaped.
struct RefusesSecond {
    seen: std::cell::Cell<usize>,
}

impl group::Sink for RefusesSecond {
    fn group(&mut self, _found: group::Found) -> imagecull_core::Result<()> {
        let seen = self.seen.get() + 1;
        self.seen.set(seen);
        if seen == 2 {
            return Err(imagecull_core::Error::fatal("the sink said no"));
        }
        Ok(())
    }
}

#[test]
fn a_sink_that_refuses_a_group_stops_the_streaming_pass() {
    let image = scene();
    let dir = shoot(vec![
        moment("DSC_0001.NEF", &image, 0, "00"),
        moment("DSC_0002.NEF", &image, 3, "00"),
        moment("DSC_0003.NEF", &image, 6, "00"),
    ]);
    let mut sink = RefusesSecond {
        seen: std::cell::Cell::new(0),
    };

    let error = group::stream_with(
        dir.path(),
        &group::Options::default(),
        &imagecull_core::progress::Silent,
        &mut sink,
    )
    .expect_err("a sink's refusal is the pass's failure");

    assert!(error.to_string().contains("the sink said no"), "{error}");
    assert_eq!(sink.seen.get(), 2, "the pass stopped at the refusal");
}
