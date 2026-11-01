mod support;

use std::fs;
use std::path::{Path, PathBuf};

use imagecull_core::{preprocess, preview};
use tempfile::TempDir;

fn shoot() -> TempDir {
    tempfile::tempdir().expect("a temp dir")
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, bytes).expect("write fixture");
    path
}

#[test]
fn the_preview_taken_is_the_one_with_the_largest_pixel_area() {
    let dir = shoot();
    // A big, cheap-to-encode frame and a tiny frame buried in padding: the
    // small one has many more bytes, the big one many more pixels.
    let large_area = support::solid(64, 32, 128);
    let small_area = support::padded(&support::jpeg(8, 4, 95), 4000);
    assert!(
        small_area.len() > large_area.len(),
        "the fixture must make byte count and pixel area disagree: {} vs {}",
        small_area.len(),
        large_area.len()
    );

    let path = write(
        dir.path(),
        "DSC_0001.NEF",
        &support::nef(1, &[large_area.clone(), small_area]),
    );

    let embedded = preview::extract(&path).expect("the fixture carries previews");

    assert_eq!(
        (embedded.width, embedded.height),
        (64, 32),
        "the largest pixel area wins, not the largest byte count"
    );
    assert_eq!(embedded.jpeg, large_area, "the bytes are the preview's own");
    assert_eq!(embedded.orientation, 1);
}

/// The four quadrant greys of `jpeg`, sampled away from the edges so that JPEG
/// ringing cannot be mistaken for a different quadrant.
fn quadrant_greys(jpeg: &[u8]) -> [u8; 4] {
    let image = image::load_from_memory(jpeg).expect("decode").to_rgb8();
    let (width, height) = image.dimensions();
    let sample = |x: u32, y: u32| image.get_pixel(x, y).0[0];
    [
        sample(width / 4, height / 4),
        sample(width * 3 / 4, height / 4),
        sample(width / 4, height * 3 / 4),
        sample(width * 3 / 4, height * 3 / 4),
    ]
}

/// The greys nearest to `sampled`: 0, 85, 170 and 255 are far enough apart that
/// a lossy encode cannot move a sample to its neighbour.
fn classify(sampled: [u8; 4]) -> [u8; 4] {
    const GREYS: [u8; 4] = [0, 85, 170, 255];
    sampled.map(|value| {
        *GREYS
            .iter()
            .min_by_key(|grey| value.abs_diff(**grey))
            .expect("four greys")
    })
}

#[test]
fn every_exif_orientation_reaches_the_model_upright() {
    // Stored 32x16 with top-left black, top-right 85, bottom-left 170 and
    // bottom-right white. Each row is what the displayed frame's quadrants must
    // hold once that orientation has been applied (EXIF 2.32 §4.6.4, "0th row
    // and 0th column"): tag 6, the portrait case, puts the stored left edge on
    // top.
    let expected: [(u16, (u32, u32), [u8; 4]); 8] = [
        (1, (32, 16), [0, 85, 170, 255]),
        (2, (32, 16), [85, 0, 255, 170]),
        (3, (32, 16), [255, 170, 85, 0]),
        (4, (32, 16), [170, 255, 0, 85]),
        (5, (16, 32), [0, 170, 85, 255]),
        (6, (16, 32), [170, 0, 255, 85]),
        (7, (16, 32), [255, 85, 170, 0]),
        (8, (16, 32), [85, 255, 0, 170]),
    ];

    for (tag, dimensions, greys) in expected {
        let embedded = preview::Embedded {
            jpeg: support::quadrants(32, 16),
            width: 32,
            height: 16,
            orientation: tag,
        };
        let encoded = preprocess::render(&embedded, &preprocess::Params::default())
            .expect("render a fixture");

        assert_eq!(
            (encoded.width, encoded.height),
            dimensions,
            "orientation {tag} has the wrong shape"
        );
        assert_eq!(
            classify(quadrant_greys(&encoded.jpeg)),
            greys,
            "orientation {tag} put the frame's corners in the wrong place"
        );
    }
}

#[test]
fn a_portrait_frame_smaller_than_the_cap_comes_out_portrait() {
    let embedded = preview::Embedded {
        jpeg: support::left_black_right_white(32, 16),
        width: 32,
        height: 16,
        orientation: 6,
    };

    let encoded = preprocess::render(&embedded, &preprocess::Params::default()).expect("render");

    assert_eq!((encoded.width, encoded.height), (16, 32));
    let image = image::load_from_memory(&encoded.jpeg)
        .expect("decode")
        .to_rgb8();
    assert!(
        image.get_pixel(8, 8).0[0] < 40,
        "the stored left edge is the displayed top: the frame is upright, not sideways"
    );
    assert!(image.get_pixel(8, 24).0[0] > 215);
}

#[test]
fn the_longest_edge_is_capped_and_nothing_is_ever_upscaled() {
    let landscape = preview::Embedded {
        jpeg: support::jpeg(400, 200, 80),
        width: 400,
        height: 200,
        orientation: 1,
    };

    let capped = preprocess::render(
        &landscape,
        &preprocess::Params {
            max_edge: 100,
            ..preprocess::Params::default()
        },
    )
    .expect("render");
    assert_eq!((capped.width, capped.height), (100, 50));

    let small = preview::Embedded {
        jpeg: support::jpeg(40, 20, 80),
        width: 40,
        height: 20,
        orientation: 1,
    };
    let untouched = preprocess::render(&small, &preprocess::Params::default()).expect("render");
    assert_eq!(
        (untouched.width, untouched.height),
        (40, 20),
        "a 40 px preview is never blown up to the 1568 px cap"
    );
}

#[test]
fn the_encoded_jpeg_carries_no_exif() {
    let embedded = preview::Embedded {
        jpeg: support::jpeg(64, 48, 80),
        width: 64,
        height: 48,
        orientation: 1,
    };

    let encoded = preprocess::render(&embedded, &preprocess::Params::default()).expect("render");

    // Walk the JPEG's segment chain: APP1 (0xFFE1) is where EXIF lives, and a
    // camera's orientation tag must not ride along into the request.
    let bytes = &encoded.jpeg;
    assert_eq!(&bytes[..2], &[0xFF, 0xD8], "a JPEG starts at SOI");
    let mut cursor = 2;
    while cursor + 4 <= bytes.len() {
        assert_eq!(bytes[cursor], 0xFF, "segment markers start with 0xFF");
        let marker = bytes[cursor + 1];
        if marker == 0xDA {
            break; // start of scan: entropy-coded data follows
        }
        let length = usize::from(u16::from_be_bytes([bytes[cursor + 2], bytes[cursor + 3]]));
        assert_ne!(
            marker, 0xE1,
            "APP1/EXIF must not be written into the preview"
        );
        cursor += 2 + length;
    }
}

#[test]
fn the_quality_setting_reaches_the_encoder() {
    let embedded = preview::Embedded {
        jpeg: support::jpeg(64, 64, 80),
        width: 64,
        height: 64,
        orientation: 1,
    };

    let cheap = preprocess::render(
        &embedded,
        &preprocess::Params {
            jpeg_quality: 20,
            ..preprocess::Params::default()
        },
    )
    .expect("render");
    let fine = preprocess::render(&embedded, &preprocess::Params::default()).expect("render");

    assert!(
        cheap.jpeg.len() < fine.jpeg.len(),
        "quality 20 ({} bytes) should spend less than the default 88 ({} bytes)",
        cheap.jpeg.len(),
        fine.jpeg.len()
    );
}

#[test]
fn the_defaults_are_the_specs_defaults() {
    let params = preprocess::Params::default();
    assert_eq!(params.max_edge, 1568);
    assert_eq!(params.jpeg_quality, 88);
}
