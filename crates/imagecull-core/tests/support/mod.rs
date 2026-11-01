//! Fixtures for the preview tests: tiny NEF-shaped files built in memory.
//!
//! Real NEFs are tens of megabytes and are not committed (spec §"Fixtures"),
//! so these build the smallest file `raw-files` will read as a Nikon NEF: a
//! TIFF whose IFD0 names Nikon, carries an EXIF orientation, and points at the
//! JPEG previews the camera would have embedded. That is enough to drive the
//! whole extraction, orientation, downscale and encode path without a camera.
//!
//! [`stub`] and [`decider`] are the other shared fixtures: a loopback HTTP
//! server that answers canned bytes, for the tests that drive the real client,
//! and a scripted [`Decider`](imagecull_core::scoring::Decider) that records
//! what the pipeline would have sent.

#![allow(dead_code)]

pub mod decider;
pub mod scenes;
pub mod stub;

use image::{ImageBuffer, Rgb, RgbImage, codecs::jpeg::JpegEncoder};
use imagecull_core::report::Outputs;
use imagecull_core::rubric::Rubric;

/// A rubric in TOML, written the way the embedded one is, so a test can change
/// one word and see only the hash move.
pub fn rubric_toml(model: &str, instructions: &str) -> String {
    format!(
        r#"
model = "{model}"

[[question]]
id = "stars"
type = "choice"
instructions = "{instructions}"
criteria = [
    {{ key = "1", description = "delete" }},
    {{ key = "2", description = "not great" }},
    {{ key = "3", description = "okay to keep" }},
    {{ key = "4", description = "good" }},
    {{ key = "5", description = "great" }},
]
"#
    )
}

pub fn rubric(model: &str, instructions: &str) -> Rubric {
    Rubric::from_toml(&rubric_toml(model, instructions)).expect("a valid rubric")
}

/// A temp shoot folder holding `files`, creating subfolders as needed.
pub fn shoot(files: &[(&str, Vec<u8>)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temp dir");
    for (name, bytes) in files {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create a subfolder");
        }
        std::fs::write(path, bytes).expect("write fixture");
    }
    dir
}

/// Where the report of a test that reads the answers from the run itself goes: a
/// throwaway directory nobody looks at.
///
/// A run always writes its CSV and the ledger beside it, and it must never write
/// into the folder it is scoring. The directory is kept rather than deleted, so
/// the path stays valid for the length of the run.
pub fn unwatched() -> Outputs {
    let dir = tempfile::tempdir().expect("a temp dir").keep();
    Outputs {
        csv: dir.join("scores.csv"),
        json_scores: None,
        absolute: false,
    }
}

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

/// Any frame as the JPEG a camera would have embedded: a preview to measure,
/// a signature to compare, or the bytes a request would carry.
pub fn jpeg_of(image: &RgbImage, quality: u8) -> Vec<u8> {
    encode(image, quality)
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

/// What the camera recorded about a frame, in the Exif IFD of a synthetic file.
///
/// Every value is optional, as it is on a real file: a test that leaves one out
/// gets a frame whose metadata does not record it, which is the case the
/// grouping gates have to treat as unknown rather than as a difference.
#[derive(Debug, Clone, Default)]
pub struct ExifFacts {
    /// DateTimeOriginal, as Exif writes it: `YYYY:MM:DD HH:MM:SS`.
    pub captured: Option<String>,
    /// SubSecTimeOriginal: the fraction of a second, as its digits.
    pub subsecond: Option<String>,
    /// BodySerialNumber.
    pub serial: Option<String>,
    /// PhotographicSensitivity.
    pub iso: Option<u16>,
    /// ExposureTime, as `(numerator, denominator)`.
    pub exposure_time: Option<(u32, u32)>,
    /// FNumber, as `(numerator, denominator)`.
    pub f_number: Option<(u32, u32)>,
    /// FocalLength, as `(numerator, denominator)`.
    pub focal_length: Option<(u32, u32)>,
}

/// A NEF-shaped file embedding `previews` in the given order, with EXIF
/// `orientation` for every one of them.
///
/// IFD0 declares the first preview the way a camera does
/// (`JPEGInterchangeFormat`); the rest hang off SubIFDs.
pub fn nef(orientation: u16, previews: &[Vec<u8>]) -> Vec<u8> {
    tiff(orientation, previews, false, None)
}

/// The same file, with what a camera would have written in its Exif IFD.
pub fn nef_with(orientation: u16, previews: &[Vec<u8>], facts: &ExifFacts) -> Vec<u8> {
    tiff(orientation, previews, false, Some(facts))
}

/// A DNG-shaped file: the same TIFF container, marked with `DNGVersion` so that
/// `raw-files` stops calling it a NEF, exactly as an Adobe-converted or
/// camera-native DNG does.
pub fn dng(orientation: u16, previews: &[Vec<u8>]) -> Vec<u8> {
    tiff(orientation, previews, true, None)
}

const TYPE_BYTE: u16 = 1;
const TYPE_ASCII: u16 = 2;
const TYPE_SHORT: u16 = 3;
const TYPE_LONG: u16 = 4;
const TYPE_RATIONAL: u16 = 5;
const MAKE: u16 = 0x010F;
const ORIENTATION: u16 = 0x0112;
const SUB_IFDS: u16 = 0x014A;
const JPEG_OFFSET: u16 = 0x0201;
const JPEG_LENGTH: u16 = 0x0202;
const DNG_VERSION: u16 = 0xC612;
const EXIF_IFD: u16 = 0x8769;
const EXPOSURE_TIME: u16 = 0x829A;
const F_NUMBER: u16 = 0x829D;
const PHOTOGRAPHIC_SENSITIVITY: u16 = 0x8827;
const DATE_TIME_ORIGINAL: u16 = 0x9003;
const FOCAL_LENGTH: u16 = 0x920A;
const SUB_SEC_TIME_ORIGINAL: u16 = 0x9291;
const BODY_SERIAL_NUMBER: u16 = 0xA431;

/// One tag of a directory, its value inline or out of line.
struct Spec {
    tag: u16,
    kind: u16,
    count: u32,
    value: Value,
}

enum Value {
    /// Four bytes or fewer, held in the entry itself.
    Inline([u8; 4]),
    /// Longer than four bytes, held after the directory at some offset.
    Blob(Vec<u8>),
}

impl Spec {
    fn ascii(tag: u16, text: &str) -> Spec {
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(0);
        let count = u32::try_from(bytes.len()).expect("a short string");
        // TIFF stores a value of four bytes or fewer in the entry itself, and
        // longer ones after the directory. A reader follows the entry as an
        // offset, so a short text stored out of line is read as garbage — which
        // is exactly what a two-digit SubSecTimeOriginal became.
        let value = if bytes.len() <= 4 {
            let mut inline = [0u8; 4];
            inline[..bytes.len()].copy_from_slice(&bytes);
            Value::Inline(inline)
        } else {
            Value::Blob(bytes)
        };
        Spec {
            tag,
            kind: TYPE_ASCII,
            count,
            value,
        }
    }

    fn short(tag: u16, value: u16) -> Spec {
        Spec {
            tag,
            kind: TYPE_SHORT,
            count: 1,
            value: Value::Inline(u32::from(value).to_le_bytes()),
        }
    }

    fn long(tag: u16, value: u32) -> Spec {
        Spec {
            tag,
            kind: TYPE_LONG,
            count: 1,
            value: Value::Inline(value.to_le_bytes()),
        }
    }

    fn rational(tag: u16, value: (u32, u32)) -> Spec {
        let mut bytes = Vec::with_capacity(8);
        bytes.extend(value.0.to_le_bytes());
        bytes.extend(value.1.to_le_bytes());
        Spec {
            tag,
            kind: TYPE_RATIONAL,
            count: 1,
            value: Value::Blob(bytes),
        }
    }
}

/// A directory and the values that hang off it.
#[derive(Default)]
struct Ifd {
    specs: Vec<Spec>,
}

impl Ifd {
    /// The entry table alone: count, entries, and the empty next-directory link.
    fn table_size(&self) -> usize {
        if self.specs.is_empty() {
            0
        } else {
            2 + 12 * self.specs.len() + 4
        }
    }

    /// How many bytes this directory takes, starting at `start`, its own
    /// out-of-line values included.
    fn region_size(&self, start: usize) -> usize {
        if self.specs.is_empty() {
            return 0;
        }
        let mut cursor = start + self.table_size();
        for spec in &self.specs {
            if let Value::Blob(bytes) = &spec.value {
                // TIFF values are word aligned.
                if cursor % 2 == 1 {
                    cursor += 1;
                }
                cursor += bytes.len();
            }
        }
        cursor - start
    }

    /// The directory at `start`, followed by its out-of-line values, zero-filled
    /// to `region_size(start)`.
    fn region(&self, start: usize) -> Vec<u8> {
        let size = self.region_size(start);
        if size == 0 {
            return Vec::new();
        }
        let mut cursor = start + self.table_size();
        let mut entries: Vec<(u16, u16, u32, [u8; 4])> = Vec::new();
        let mut blobs: Vec<(usize, &[u8])> = Vec::new();
        for spec in &self.specs {
            match &spec.value {
                Value::Inline(value) => entries.push((spec.tag, spec.kind, spec.count, *value)),
                Value::Blob(bytes) => {
                    if cursor % 2 == 1 {
                        cursor += 1;
                    }
                    entries.push((
                        spec.tag,
                        spec.kind,
                        spec.count,
                        u32::try_from(cursor).expect("a small file").to_le_bytes(),
                    ));
                    blobs.push((cursor, bytes));
                    cursor += bytes.len();
                }
            }
        }
        entries.sort_by_key(|entry| entry.0);

        let mut out = Vec::with_capacity(size);
        out.extend(
            u16::try_from(entries.len())
                .expect("few entries")
                .to_le_bytes(),
        );
        for (tag, kind, count, value) in &entries {
            out.extend(tag.to_le_bytes());
            out.extend(kind.to_le_bytes());
            out.extend(count.to_le_bytes());
            out.extend(value);
        }
        out.extend(0u32.to_le_bytes()); // no next IFD
        for (offset, bytes) in blobs {
            pad_to(&mut out, offset - start);
            out.extend(bytes);
        }
        pad_to(&mut out, size);
        out
    }

    fn push(&mut self, spec: Spec) {
        self.specs.push(spec);
    }
}

fn tiff(orientation: u16, previews: &[Vec<u8>], dng: bool, facts: Option<&ExifFacts>) -> Vec<u8> {
    let sub_count = previews.len().saturating_sub(1);

    let mut exif = Ifd::default();
    if let Some(facts) = facts {
        if let Some(captured) = &facts.captured {
            exif.push(Spec::ascii(DATE_TIME_ORIGINAL, captured));
        }
        if let Some(subsecond) = &facts.subsecond {
            exif.push(Spec::ascii(SUB_SEC_TIME_ORIGINAL, subsecond));
        }
        if let Some(serial) = &facts.serial {
            exif.push(Spec::ascii(BODY_SERIAL_NUMBER, serial));
        }
        if let Some(iso) = facts.iso {
            exif.push(Spec::short(PHOTOGRAPHIC_SENSITIVITY, iso));
        }
        for (tag, value) in [
            (EXPOSURE_TIME, facts.exposure_time),
            (F_NUMBER, facts.f_number),
            (FOCAL_LENGTH, facts.focal_length),
        ] {
            if let Some(value) = value {
                exif.push(Spec::rational(tag, value));
            }
        }
    }

    let mut ifd0 = Ifd::default();
    ifd0.push(Spec::ascii(MAKE, "NIKON CORPORATION"));
    ifd0.push(Spec::short(ORIENTATION, orientation));
    if !exif.specs.is_empty() {
        // Filled in once the layout is known.
        ifd0.push(Spec::long(EXIF_IFD, 0));
    }
    if sub_count > 0 {
        ifd0.push(Spec::long(SUB_IFDS, 0));
    }
    if !previews.is_empty() {
        ifd0.push(Spec::long(JPEG_OFFSET, 0));
        ifd0.push(Spec::long(JPEG_LENGTH, 0));
    }
    if dng {
        // DNGVersion 1.4.0.0, stored inline: four bytes need no offset.
        ifd0.push(Spec {
            tag: DNG_VERSION,
            kind: TYPE_BYTE,
            count: 4,
            value: Value::Inline([1, 4, 0, 0]),
        });
    }

    // Lay the file out: header, IFD0, the Exif IFD, the SubIFDs, then the
    // previews themselves. Every offset below counts from the file start.
    let exif_start = 8 + ifd0.region_size(8);
    let mut cursor = exif_start + exif.region_size(exif_start);
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

    // The pointers IFD0 could not hold until the layout was settled.
    for spec in &mut ifd0.specs {
        let pointer = match spec.tag {
            EXIF_IFD => Some(exif_start),
            SUB_IFDS => Some(sub_array_offset),
            JPEG_OFFSET => previews.first().map(|_| jpeg_offsets[0]),
            JPEG_LENGTH => previews.first().map(|preview| preview.len()),
            _ => None,
        };
        if let Some(pointer) = pointer {
            spec.value = Value::Inline(u32::try_from(pointer).expect("a small file").to_le_bytes());
        }
    }

    let mut out = Vec::with_capacity(cursor);
    out.extend(b"II");
    out.extend(42u16.to_le_bytes());
    out.extend(8u32.to_le_bytes());
    out.extend(ifd0.region(8));
    out.extend(exif.region(exif_start));
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
    assert_eq!(
        out.len(),
        cursor,
        "the layout must land where it was planned"
    );
    out
}

/// Zero-fill `out` up to `offset`.
fn pad_to(out: &mut Vec<u8>, offset: usize) {
    assert!(out.len() <= offset, "the layout must not overlap");
    out.resize(offset, 0);
}
