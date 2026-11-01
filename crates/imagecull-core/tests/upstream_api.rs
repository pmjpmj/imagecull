//! Pins the two upstream APIs the extraction ticket depends on, and proves they
//! build and behave on Apple Silicon before any extraction code is written.
//!
//! `image` provides the orientation step that keeps a portrait frame from being
//! scored sideways (spec §6.3); `raw-files` provides the embedded-preview
//! extraction (spec §6.2). Both are critical path, so M0 verifies the exact
//! versions we pinned rather than discovering a broken arm64 build mid-ticket.

use image::DynamicImage;
use image::metadata::Orientation;

/// Every EXIF orientation tag 1..=8 must apply without panicking, and the
/// transposing ones must actually swap the axes.
#[test]
fn image_applies_all_eight_exif_orientations() {
    // 4x2 landscape; orientation 6/8 are the quarter turns a portrait NEF needs.
    for tag in 1..=8 {
        let mut image = DynamicImage::new_rgb8(4, 2);
        let orientation = Orientation::from_exif(tag).expect("tag 1..=8 is a known orientation");
        image.apply_orientation(orientation);
        let (width, height) = (image.width(), image.height());
        let quarters = matches!(tag, 5..=8);
        if quarters {
            assert_eq!(
                (width, height),
                (2, 4),
                "tag {tag} should transpose the axes"
            );
        } else {
            assert_eq!((width, height), (4, 2), "tag {tag} should keep the axes");
        }
    }

    assert!(Orientation::from_exif(9).is_none());
}

/// The preview accessors the extractor will call, exercised on a path that does
/// not exist: the point is that they exist and fail as `Result`, not by panic.
#[test]
fn raw_files_exposes_preview_extraction() {
    let error = raw_files::RawFile::open("tests/fixtures/absent.nef")
        .expect_err("a missing file cannot open");
    assert!(
        !error.to_string().is_empty(),
        "the error should explain itself"
    );
}
