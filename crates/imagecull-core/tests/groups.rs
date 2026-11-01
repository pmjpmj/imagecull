//! Grouping, at the seam that matters: a shoot folder in, groups and measures
//! out.
//!
//! Every test here writes a small synthetic burst to disk — NEF-shaped files
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
use support::{ExifFacts, burst};

/// The size the grouping tests draw at: small, because a group is about times,
/// serials and settings, and every pixel here costs the suite a decode.
const WIDTH: u32 = 640;
const HEIGHT: u32 = 427;

fn larger() -> (u32, u32) {
    (WIDTH, HEIGHT)
}

/// What one frame of a synthetic burst is.
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

/// The scene every frame of a burst shows, freshly drawn.
fn scene() -> RgbImage {
    burst::scene(WIDTH, HEIGHT)
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
fn a_burst_inside_the_gap_with_one_body_and_one_scene_is_one_group() {
    let image = scene();
    let dir = shoot(vec![
        frame("DSC_0001.NEF", &image, 0, "10", "3001", 200),
        frame("DSC_0002.NEF", &image, 0, "20", "3001", 200),
        frame("DSC_0003.NEF", &image, 0, "30", "3001", 200),
        frame("DSC_0004.NEF", &image, 0, "40", "3001", 200),
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
        frame("DSC_0001.NEF", &image, 0, "00", "3001", 200),
        frame("DSC_0002.NEF", &image, 0, "10", "3001", 200),
        frame("DSC_0003.NEF", &image, 3, "50", "3001", 200),
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
fn a_different_body_or_a_different_exposure_is_a_different_moment() {
    let image = scene();
    let dir = shoot(vec![
        frame("DSC_0001.NEF", &image, 0, "00", "3001", 200),
        frame("DSC_0002.NEF", &image, 0, "10", "3002", 200),
        frame("DSC_0003.NEF", &image, 0, "20", "3002", 800),
    ]);

    let plan = plan(dir.path(), Thresholds::default());

    assert_eq!(plan.groups, 3, "{:?}", groups_by_name(&plan));
    for frame in &plan.frames {
        assert_eq!(frame.group_size, 1);
    }
}

#[test]
fn a_pan_splits_the_burst_into_the_compositions_it_produced() {
    // The camera held still, then swung: the same body, the same settings and
    // no gap, but a different picture.
    let before = scene();
    let after = burst::noise(WIDTH, HEIGHT, 11);
    let dir = shoot(vec![
        frame("DSC_0001.NEF", &before, 0, "00", "3001", 200),
        frame("DSC_0002.NEF", &before, 0, "10", "3001", 200),
        frame("DSC_0003.NEF", &after, 0, "20", "3001", 200),
        frame("DSC_0004.NEF", &after, 0, "30", "3001", 200),
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
    let mut undated = frame("DSC_0002.NEF", &image, 0, "10", "3001", 200);
    undated.facts.captured = None;
    undated.facts.subsecond = None;
    let dir = shoot(vec![
        frame("DSC_0001.NEF", &image, 0, "00", "3001", 200),
        undated,
        frame("DSC_0003.NEF", &image, 0, "20", "3001", 200),
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
        frame("DSC_10.NEF", &image, 0, "00", "3001", 200),
        frame("DSC_9.NEF", &image, 0, "10", "3001", 200),
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
        frame("AAA_0001.NEF", &image, 0, "00", "3001", 200),
        frame("AAA_0002.NEF", &image, 0, "10", "3001", 200),
        frame("AAA_0003.NEF", &image, 5, "00", "3001", 200),
    ]);
    let second = shoot(vec![
        frame("ZZZ_9999.NEF", &image, 0, "00", "3001", 200),
        frame("ZZZ_0001.NEF", &image, 0, "10", "3001", 200),
        frame("MMM_0042.NEF", &image, 5, "00", "3001", 200),
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
        frame("DSC_0001.NEF", &scene(), 0, "00", "3001", 200),
        frame(
            "DSC_0002.NEF",
            &burst::blurred(&scene(), 1.0),
            0,
            "10",
            "3001",
            200,
        ),
        frame("DSC_0003.NEF", &scene(), 0, "20", "3001", 200),
    ]);

    let first = group::render_csv(&plan(dir.path(), Thresholds::default()));
    let second = group::render_csv(&plan(dir.path(), Thresholds::default()));

    assert_eq!(first, second);
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

    assert_eq!(
        group::grouping_hash(&default),
        group::grouping_hash(&Thresholds::default())
    );
    assert_ne!(group::grouping_hash(&default), group::grouping_hash(&wider));
    assert_ne!(
        group::grouping_hash(&default),
        group::grouping_hash(&stricter)
    );
}

#[test]
fn the_synthetic_answer_key_orders_as_the_transforms_did() {
    // One group, five frames of the same moment, each a known transform of one
    // scene: the ranking issue 02 will do needs these numbers to come out in
    // the order the transforms say.
    let sharp = scene();
    let (width, height) = larger();
    let blown = burst::brighter(&sharp, 2.0);
    let tilted = burst::tilted_scene(width, height, 1.5);
    let dir = shoot(vec![
        frame("DSC_0001.NEF", &sharp, 0, "00", "3001", 200),
        frame(
            "DSC_0002.NEF",
            &burst::blurred(&sharp, 1.0),
            0,
            "10",
            "3001",
            200,
        ),
        frame(
            "DSC_0003.NEF",
            &burst::shaken(&sharp, 5),
            0,
            "20",
            "3001",
            200,
        ),
        frame("DSC_0004.NEF", &blown, 0, "30", "3001", 200),
        frame("DSC_0005.NEF", &tilted, 0, "40", "3001", 200),
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
    let dir = shoot(vec![frame("DSC_0001.NEF", &image, 0, "00", "3001", 200)]);
    // A file whose preview the reader cannot get at: the frame is still a frame,
    // and its numbers are missing rather than zero.
    let previewless = support::nef_with(1, &[], &frame("x", &image, 0, "10", "3001", 200).facts);
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
        frame("DSC_0001.NEF", &image, 0, "00", "3001", 200),
        frame("DSC_0002.NEF", &image, 0, "10", "3001", 200),
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
        frame("DSC_0001.NEF", &image, 0, "00", "3001", 200),
        frame("DSC_0002.NEF", &image, 0, "10", "3001", 200),
    ]);

    let plan = plan(dir.path(), Thresholds::default());

    assert_eq!(plan.frames.len(), 2);
    assert!(
        plan.elapsed > std::time::Duration::ZERO,
        "the plan records how long the pass took, so the evidence can quote it"
    );
    assert_eq!(plan.groups, 1);
}
