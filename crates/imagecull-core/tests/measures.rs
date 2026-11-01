//! What a frame's own pixels can say: the measure seam.
//!
//! Every test here builds a frame from the synthetic scene in `support::burst`,
//! applies one known transform, and asserts the number that comes back. That is
//! the only honest way to test a sharpness or horizon measurement: the answer
//! key is the transform, not another implementation of the same formula. No
//! test in this file starts a server, and none of them can: `measure` has no
//! `Decider` to call.

mod support;

use image::RgbImage;
use imagecull_core::measure::{self, AfPoint};
use imagecull_core::preview::Embedded;
use support::burst;

/// A preview of `image`, as a camera would have embedded it.
fn embedded(image: &RgbImage, orientation: u16) -> Embedded {
    Embedded {
        jpeg: support::jpeg_of(image, 95),
        width: image.width(),
        height: image.height(),
        orientation,
    }
}

/// The measures of one frame the camera recorded no focus for.
fn measures(image: &RgbImage) -> measure::Measures {
    measure::of_preview(&embedded(image, 1), None).expect("a preview that decodes")
}

/// The measures of `image` with the camera's focus point at `centre`, in the
/// frame's own pixels.
fn measures_with_focus(image: &RgbImage, centre: (u32, u32)) -> measure::Measures {
    let af = AfPoint {
        frame: (image.width(), image.height()),
        center: centre,
        size: None,
        in_focus: Some(true),
    };
    measure::of_preview(&embedded(image, 1), Some(&af)).expect("a preview that decodes")
}

#[test]
fn roll_recovers_a_known_rotation() {
    let (width, height) = (1_600, 1_067);
    let tilt = 1.5_f64;

    let tilted = measures(&burst::tilted_scene(width, height, tilt as f32));
    let level = measures(&burst::tilted_scene(width, height, 0.0));

    let angled = tilted.roll_deg.expect("a scene with a horizon has a roll");
    assert!(
        (angled - tilt).abs() <= 0.2,
        "a {tilt}° tilt should measure as {tilt}° ± 0.2, measured {angled}"
    );
    let level_deg = level.roll_deg.expect("a level horizon still has a roll");
    assert!(
        level_deg.abs() <= 0.1,
        "the level scene is level, measured {level_deg}"
    );
    assert!(
        tilted.roll_strength > Some(measure::ROLL_MIN_STRENGTH),
        "a horizon has to clear the strength floor: {tilted:?}"
    );
}

#[test]
fn a_frame_with_no_dominant_line_reports_no_roll_rather_than_zero() {
    let speckle = burst::noise(1_600, 1_067, 7);
    let measured = measures(&speckle);

    assert_eq!(
        measured.roll_deg, None,
        "noise has no horizon, so there is no angle to report"
    );
    let strength = measured.roll_strength.expect("strength is always recorded");
    assert!(
        strength < measure::ROLL_MIN_STRENGTH,
        "noise must fall below the floor, measured {strength}"
    );
}

#[test]
fn shake_tells_a_moving_camera_from_a_lens_that_missed_focus() {
    let (width, height) = (1_600, 1_067);
    let scene = burst::scene(width, height);

    let shaken = measures(&burst::shaken(&scene, 5));
    let softly = measures(&burst::blurred(&scene, 1.0));
    let sharp = measures(&scene);

    let shaken_by = shaken.shake.expect("a frame with pixels has a shake");
    let blurred_by = softly.shake.expect("a frame with pixels has a shake");
    assert!(
        shaken_by > 0.1,
        "5 px of horizontal blur is a camera that moved: {shaken_by}"
    );
    assert!(
        blurred_by < 0.05,
        "an isotropic blur leaves the axes much as they were: {blurred_by}"
    );
    assert!(
        shaken_by > 10.0 * blurred_by,
        "directional blur must read as far more shake than an isotropic blur of the same \
         scene ({shaken_by} vs {blurred_by})"
    );
    assert!(
        shaken.acutance < sharp.acutance,
        "the shaken frame is also the softer one: {:?} vs {:?}",
        shaken.acutance,
        sharp.acutance
    );
}

#[test]
fn acutance_ranks_a_sharp_frame_above_a_soft_one_at_two_sensor_sizes() {
    for (width, height) in [(1_600, 1_067), (800, 533)] {
        let scene = burst::scene(width, height);
        let sharp = measures(&scene)
            .acutance
            .expect("a textured frame has acutance");
        let soft = measures(&burst::blurred(&scene, 1.0))
            .acutance
            .expect("a blurred frame still has acutance");

        assert!(
            sharp > soft * 3.0,
            "at {width}×{height} the sharp frame must beat the blurred one by a clear margin: \
             {sharp} vs {soft}"
        );
    }
}

#[test]
fn clipping_reads_the_blown_highlights_and_leaves_a_normal_frame_alone() {
    let scene = burst::scene(1_600, 1_067);

    let normal = measures(&scene);
    let blown = measures(&burst::brighter(&scene, 2.0));

    assert!(
        normal.clip_hi.expect("clip_hi") < 1.0,
        "a normally exposed frame barely clips: {:?}",
        normal.clip_hi
    );
    assert!(
        blown.clip_hi.expect("clip_hi") > 50.0,
        "+2 EV blows the sky out: {:?}",
        blown.clip_hi
    );
    assert_eq!(
        normal.clip_lo,
        Some(0.0),
        "nothing in this scene is crushed to black"
    );
}

#[test]
fn af_off_subject_fires_when_the_camera_focused_on_a_soft_patch() {
    let (width, height) = (1_600, 1_067);
    let scene = burst::scene(width, height);

    let on_the_patch = measures_with_focus(&scene, burst::soft_centre(width, height));
    let on_the_subject = measures_with_focus(&scene, burst::sharp_centre(width, height));

    assert_eq!(
        on_the_patch.af_off_subject,
        Some(true),
        "focus over the smooth patch is behind the subject"
    );
    assert_eq!(
        on_the_subject.af_off_subject,
        Some(false),
        "focus over the textured ground is on the subject"
    );
    assert_eq!(
        (on_the_patch.af_in_focus, on_the_subject.af_in_focus),
        (Some(true), Some(true)),
        "the camera confirmed both, which is why the flag is needed"
    );
}

#[test]
fn an_unrecorded_focus_report_is_unknown_not_excluded() {
    let scene = burst::scene(800, 533);
    let measured = measures(&scene);

    assert_eq!(measured.af_in_focus, None);
    assert_eq!(
        measured.af_off_subject, None,
        "no AF point means nothing to compare, never a flag"
    );
}

#[test]
fn the_focus_point_is_mapped_through_the_frames_orientation() {
    // A portrait frame stored sideways: the scene turned a quarter turn, with
    // EXIF orientation 6 to turn it back. The AF point is where the soft patch
    // sits in the *stored* frame, which is the frame the camera records it in.
    let (width, height) = (1_600, 1_067);
    let scene = burst::scene(width, height);
    let stored = image::imageops::rotate270(&scene);
    let (stored_width, stored_height) = stored.dimensions();

    let (soft_x, soft_y) = burst::soft_centre(width, height);
    let upright = (soft_x as f32 / width as f32, soft_y as f32 / height as f32);
    // Orientation 8 is the turn from the upright frame back to the stored one.
    let (stored_x, stored_y) = measure::orient_point(8, upright);
    let af = AfPoint {
        frame: (stored_width, stored_height),
        center: (
            (stored_x * stored_width as f32) as u32,
            (stored_y * stored_height as f32) as u32,
        ),
        size: None,
        in_focus: Some(true),
    };

    let measured =
        measure::of_preview(&embedded(&stored, 6), Some(&af)).expect("a preview that decodes");
    assert_eq!(
        measured.af_off_subject,
        Some(true),
        "the point must land on the soft patch after the orientation is applied"
    );
}

#[test]
fn an_undecodable_preview_is_an_error_rather_than_a_zero() {
    let broken = Embedded {
        jpeg: b"not a JPEG at all".to_vec(),
        width: 100,
        height: 100,
        orientation: 1,
    };

    let outcome = measure::of_preview(&broken, None);
    assert!(
        outcome.is_err(),
        "a measure that cannot be taken is a failure, never a 0 that reads as bad"
    );
}
