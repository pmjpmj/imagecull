//! Turning an embedded preview into the bytes a request carries.
//!
//! Three steps, in this order and no other: the EXIF orientation is applied so
//! a portrait frame reaches the model upright, the longest edge is scaled down
//! to the cap (never up), and the result is encoded as a JPEG with nothing but
//! pixels in it (spec §6.3–6.5). Getting the order wrong scores a sideways
//! frame; leaving the EXIF in makes the payload bigger for no reason; upscaling
//! invents detail the model would then judge.

use fast_image_resize::{FilterType, IntoImageView, ResizeAlg, ResizeOptions, Resizer};
use image::DynamicImage;
use image::codecs::jpeg::JpegEncoder;
use image::metadata::Orientation;

use crate::error::{Error, ErrorKind, Result};
use crate::preview::Embedded;

/// How much picture to send, and how hard to compress it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Params {
    /// The longest edge of the sent image, in pixels. Smaller previews are left
    /// alone: the cap is a ceiling, never a target.
    pub max_edge: u32,
    /// The JPEG quality of the sent image, 1–100.
    pub jpeg_quality: u8,
}

impl Params {
    /// The default longest edge: 1568 px (spec §6.5).
    pub const DEFAULT_MAX_EDGE: u32 = 1568;
    /// The default JPEG quality: 88 (spec §6.5).
    pub const DEFAULT_JPEG_QUALITY: u8 = 88;
}

impl Default for Params {
    /// The spec's defaults: 1568 px and quality 88 (spec §6.5).
    fn default() -> Self {
        Params {
            max_edge: Params::DEFAULT_MAX_EDGE,
            jpeg_quality: Params::DEFAULT_JPEG_QUALITY,
        }
    }
}

/// The bytes a request would carry, with the size they decode to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encoded {
    /// The encoded JPEG: oriented, downscaled, and carrying no EXIF.
    pub jpeg: Vec<u8>,
    /// The width of the sent image, after the orientation was applied.
    pub width: u32,
    /// The height of the sent image, after the orientation was applied.
    pub height: u32,
}

/// Decode, orient, downscale and encode an embedded preview.
pub fn render(embedded: &Embedded, params: &Params) -> Result<Encoded> {
    let mut image = image::load_from_memory_with_format(&embedded.jpeg, image::ImageFormat::Jpeg)
        .map_err(|error| {
            Error::new(
                ErrorKind::PreviewBroken,
                format!("the embedded preview does not decode as JPEG: {error}"),
            )
        })?;
    apply_orientation(&mut image, embedded.orientation);

    let mut rgb = image.to_rgb8();
    let (width, height) = scaled_size(rgb.width(), rgb.height(), params.max_edge);
    if (width, height) != rgb.dimensions() {
        rgb = downscale(&rgb, width, height)?;
    }

    let jpeg = encode(&rgb, params.jpeg_quality)?;
    Ok(Encoded {
        jpeg,
        width: rgb.width(),
        height: rgb.height(),
    })
}

/// Apply the EXIF orientation the camera recorded, so the model sees the frame
/// the photographer saw.
///
/// An unknown tag is left alone: `raw-files` reports 0 and 9-and-above as 1
/// already, and a preview with no orientation is stored upright.
///
/// The turn happens **exactly once**: `raw-files` reports the file's orientation
/// for every preview it finds, and `image`'s decoders return pixels as stored,
/// never applying an orientation tag of their own.
fn apply_orientation(image: &mut DynamicImage, exif_orientation: u16) {
    if let Some(orientation) = u8::try_from(exif_orientation)
        .ok()
        .and_then(Orientation::from_exif)
    {
        image.apply_orientation(orientation);
    }
}

/// The largest size inside a `max_edge` square with the same shape as the
/// original. Never larger than the original.
fn scaled_size(width: u32, height: u32, max_edge: u32) -> (u32, u32) {
    let max_edge = max_edge.max(1);
    let longest = width.max(height);
    if longest <= max_edge {
        return (width, height);
    }
    let scale = f64::from(max_edge) / f64::from(longest);
    let scale_to = |edge: u32| -> u32 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let scaled = (f64::from(edge) * scale).round() as u32;
        scaled.max(1)
    };
    (scale_to(width), scale_to(height))
}

/// One Lanczos3 pass, which is both the sharpest of the common filters and the
/// one `fast_image_resize` is built around (spec §6.4).
fn downscale(source: &image::RgbImage, width: u32, height: u32) -> Result<image::RgbImage> {
    let pixel_type = source.pixel_type().ok_or_else(|| {
        Error::new(
            ErrorKind::PreviewBroken,
            "the embedded preview has no resizable pixel format",
        )
    })?;
    let mut target = fast_image_resize::images::Image::new(width, height, pixel_type);
    let mut resizer = Resizer::new();
    resizer
        .resize(
            source,
            &mut target,
            &ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3)),
        )
        .map_err(|error| {
            Error::new(
                ErrorKind::PreviewBroken,
                format!("the embedded preview could not be downscaled: {error}"),
            )
        })?;
    image::RgbImage::from_raw(width, height, target.buffer().to_vec()).ok_or_else(|| {
        Error::new(
            ErrorKind::PreviewBroken,
            "the downscaled preview had the wrong buffer size",
        )
    })
}

/// Encode to JPEG. `image`'s encoder writes JFIF and no EXIF, so the output
/// carries pixels only.
fn encode(image: &image::RgbImage, quality: u8) -> Result<Vec<u8>> {
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, quality)
        .encode_image(image)
        .map_err(|error| {
            Error::new(
                ErrorKind::PreviewBroken,
                format!("the preview could not be encoded as JPEG: {error}"),
            )
        })?;
    Ok(jpeg)
}
