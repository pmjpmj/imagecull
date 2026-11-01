//! Fixtures for the preview tests: tiny NEF-shaped files built in memory.
//!
//! Real NEFs are tens of megabytes and are not committed (spec §"Fixtures"),
//! so these build the smallest file `raw-files` will read as a Nikon NEF: a
//! TIFF whose IFD0 names Nikon, carries an EXIF orientation, and points at the
//! JPEG previews the camera would have embedded. That is enough to drive the
//! whole extraction, orientation, downscale and encode path without a camera.

#![allow(dead_code)]

use image::{ImageBuffer, Rgb, RgbImage, codecs::jpeg::JpegEncoder};

/// A JPEG of `width` by `height` with a pattern, so a resize has something to
/// lose and the encoder has something to spend bytes on.
pub fn jpeg(width: u32, height: u32, quality: u8) -> Vec<u8> {
    let image: RgbImage = ImageBuffer::from_fn(width, height, |x, y| {
        Rgb([
            u8::try_from((x * 8) % 256).unwrap_or(0),
            u8::try_from((y * 8) % 256).unwrap_or(0),
            u8::try_from(((x + y) * 4) % 256).unwrap_or(0),
        ])
    });
    encode(&image, quality)
}

/// A JPEG whose stored left half is black and right half white: after the EXIF
/// orientation is applied, which half is on top says whether the frame was
/// turned upright or left sideways.
pub fn left_black_right_white(width: u32, height: u32) -> Vec<u8> {
    let image: RgbImage = ImageBuffer::from_fn(width, height, |x, _| {
        if x < width / 2 {
            Rgb([0, 0, 0])
        } else {
            Rgb([255, 255, 255])
        }
    });
    encode(&image, 90)
}

/// A solid frame of one colour: compresses to very few bytes however large.
pub fn solid(width: u32, height: u32, grey: u8) -> Vec<u8> {
    let image: RgbImage = ImageBuffer::from_pixel(width, height, Rgb([grey, grey, grey]));
    encode(&image, 40)
}

/// Four quadrants of four greys: top-left black, top-right dark, bottom-left
/// light, bottom-right white.
///
/// Each of the eight EXIF orientations moves them to a different arrangement,
/// so one sample per quadrant says which transform was applied.
pub fn quadrants(width: u32, height: u32) -> Vec<u8> {
    let image: RgbImage = ImageBuffer::from_fn(width, height, |x, y| {
        let grey = match (x < width / 2, y < height / 2) {
            (true, true) => 0,
            (false, true) => 85,
            (true, false) => 170,
            (false, false) => 255,
        };
        Rgb([grey, grey, grey])
    });
    encode(&image, 95)
}

fn encode(image: &RgbImage, quality: u8) -> Vec<u8> {
    let mut bytes = Vec::new();
    JpegEncoder::new_with_quality(&mut bytes, quality)
        .encode_image(image)
        .expect("encode a JPEG fixture");
    bytes
}

/// `jpeg` with a JPEG comment segment of `filler` `x` bytes spliced in after the
/// start-of-image marker.
///
/// Decoders ignore it, so the frame's pixel size stays what it was while its
/// byte length grows: a fixture for "the preview picked is the one with the
/// largest pixel area, not the one with the most bytes".
pub fn padded(jpeg: &[u8], filler: usize) -> Vec<u8> {
    let mut out = jpeg[..2].to_vec(); // SOI
    let length = u16::try_from(filler + 2).expect("a short comment");
    out.extend([0xFF, 0xFE]); // COM
    out.extend(length.to_be_bytes());
    out.extend(std::iter::repeat_n(b'x', filler));
    out.extend(&jpeg[2..]);
    out
}

/// A NEF-shaped file embedding `previews` in the given order, with EXIF
/// `orientation` for every one of them.
///
/// IFD0 declares the first preview the way a camera does
/// (`JPEGInterchangeFormat`); the rest hang off SubIFDs.
pub fn nef(orientation: u16, previews: &[Vec<u8>]) -> Vec<u8> {
    tiff(orientation, previews, false)
}

/// A DNG-shaped file: the same TIFF container, marked with `DNGVersion` so that
/// `raw-files` stops calling it a NEF, exactly as an Adobe-converted or
/// camera-native DNG does.
pub fn dng(orientation: u16, previews: &[Vec<u8>]) -> Vec<u8> {
    tiff(orientation, previews, true)
}

fn tiff(orientation: u16, previews: &[Vec<u8>], dng: bool) -> Vec<u8> {
    const TYPE_ASCII: u16 = 2;
    const TYPE_SHORT: u16 = 3;
    const TYPE_LONG: u16 = 4;
    const MAKE: u16 = 0x010F;
    const ORIENTATION: u16 = 0x0112;
    const SUB_IFDS: u16 = 0x014A;
    const JPEG_OFFSET: u16 = 0x0201;
    const JPEG_LENGTH: u16 = 0x0202;
    const DNG_VERSION: u16 = 0xC612;
    const TYPE_BYTE: u16 = 1;

    let make: &[u8] = b"NIKON CORPORATION\0";
    let sub_count = previews.len().saturating_sub(1);

    let mut entries: Vec<(u16, u16, u32, [u8; 4])> = Vec::new();
    let entry_count = 2
        + usize::from(dng)
        + usize::from(sub_count > 0)
        + if previews.is_empty() { 0 } else { 2 };
    let ifd0_size = 2 + 12 * entry_count + 4;

    // Lay the file out: header, IFD0, out-of-line values, SubIFDs, then the
    // previews themselves. Every offset below counts from the file start.
    let mut cursor = 8 + ifd0_size;
    let make_offset = cursor;
    cursor += make.len();
    // TIFF values are word aligned.
    if cursor % 2 == 1 {
        cursor += 1;
    }
    let sub_array_offset = cursor;
    cursor += 4 * sub_count;

    let mut sub_offsets = Vec::new();
    for _ in 0..sub_count {
        sub_offsets.push(cursor);
        cursor += 2 + 12 * 2 + 4;
    }
    let mut jpeg_offsets = Vec::new();
    for preview in previews {
        jpeg_offsets.push(cursor);
        cursor += preview.len();
    }

    entries.push((
        MAKE,
        TYPE_ASCII,
        u32::try_from(make.len()).expect("a short make"),
        u32::try_from(make_offset)
            .expect("a small file")
            .to_le_bytes(),
    ));
    entries.push((
        ORIENTATION,
        TYPE_SHORT,
        1,
        u32::from(orientation).to_le_bytes(),
    ));
    if sub_count > 0 {
        entries.push((
            SUB_IFDS,
            TYPE_LONG,
            u32::try_from(sub_count).expect("few previews"),
            u32::try_from(sub_array_offset)
                .expect("a small file")
                .to_le_bytes(),
        ));
    }
    if let (Some(first), Some(_)) = (jpeg_offsets.first(), previews.first()) {
        entries.push((
            JPEG_OFFSET,
            TYPE_LONG,
            1,
            u32::try_from(*first).expect("a small file").to_le_bytes(),
        ));
        entries.push((
            JPEG_LENGTH,
            TYPE_LONG,
            1,
            u32::try_from(previews[0].len())
                .expect("a small preview")
                .to_le_bytes(),
        ));
    }
    if dng {
        // DNGVersion 1.4.0.0, stored inline: four bytes need no offset.
        entries.push((DNG_VERSION, TYPE_BYTE, 4, [1, 4, 0, 0]));
    }
    entries.sort_by_key(|entry| entry.0);

    let mut out = Vec::with_capacity(cursor);
    out.extend(b"II");
    out.extend(42u16.to_le_bytes());
    out.extend(8u32.to_le_bytes());

    out.extend(u16::try_from(entries.len()).expect("few entries").to_le_bytes());
    for (tag, kind, count, value) in &entries {
        out.extend(tag.to_le_bytes());
        out.extend(kind.to_le_bytes());
        out.extend(count.to_le_bytes());
        out.extend(value);
    }
    out.extend(0u32.to_le_bytes()); // no next IFD

    pad_to(&mut out, make_offset);
    out.extend(make);
    pad_to(&mut out, sub_array_offset);
    for offset in &sub_offsets {
        out.extend(u32::try_from(*offset).expect("a small file").to_le_bytes());
    }

    for (index, _) in sub_offsets.iter().enumerate() {
        let preview = &previews[index + 1];
        out.extend(2u16.to_le_bytes());
        out.extend(JPEG_OFFSET.to_le_bytes());
        out.extend(TYPE_LONG.to_le_bytes());
        out.extend(1u32.to_le_bytes());
        out.extend(
            u32::try_from(jpeg_offsets[index + 1])
                .expect("a small file")
                .to_le_bytes(),
        );
        out.extend(JPEG_LENGTH.to_le_bytes());
        out.extend(TYPE_LONG.to_le_bytes());
        out.extend(1u32.to_le_bytes());
        out.extend(
            u32::try_from(preview.len())
                .expect("a small preview")
                .to_le_bytes(),
        );
        out.extend(0u32.to_le_bytes()); // no next IFD
    }

    for preview in previews {
        out.extend(preview);
    }
    assert_eq!(out.len(), cursor, "the layout must land where it was planned");
    out
}

/// Zero-fill `out` up to `offset`.
fn pad_to(out: &mut Vec<u8>, offset: usize) {
    assert!(out.len() <= offset, "the layout must not overlap");
    out.resize(offset, 0);
}
