//! Synthetic frames: one scene, transformed the ways a sequence of one moment
//! goes wrong.
//!
//! A real sequence of one moment has no ground truth about which frame is the
//! sharpest, so the measure and grouping tests build their own. [`scene`] is the same picture
//! every time — a bright sky over a textured ground across a level horizon at
//! [`HORIZON`], with one deliberately soft patch — and the transforms here turn
//! it into the answer key the spec's Testing Decisions describe: `soft` (a
//! Gaussian blur), `shaken` (a horizontal motion blur), `tilted` (a rotation of
//! a known angle), `blown` (two stops over) and a pan (a different scene).
//!
//! Everything is a pure function of its arguments: no randomness beyond the
//! seeded noise in [`noise`], so two runs of the suite see the same pixels.

use image::{ImageBuffer, Rgb, RgbImage};

/// Where the horizon sits, as a fraction of the frame's height.
pub const HORIZON: f32 = 0.55;

/// The soft patch, as fractions of the frame: left, top, side.
pub const SOFT_PATCH: (f32, f32, f32) = (0.12, 0.66, 0.24);

/// A frame with a level horizon, a bright sky, a finely textured ground and one
/// deliberately smooth patch in the ground.
///
/// The sky is a gentle vertical gradient (little gradient energy, no lines),
/// the ground a checkerboard (lots of gradient energy in both axes) and the
/// horizon between them a single long, strong, horizontal edge: the one line a
/// roll measurement should find. The soft patch is a faint ramp inside the
/// checkerboard, so the camera's focus point can be placed on something sharp
/// or on something smooth.
pub fn scene(width: u32, height: u32) -> RgbImage {
    let (left, top, side) = patch(width, height);
    ImageBuffer::from_fn(width, height, |x, y| {
        let grey = if inside(x, y, left, top, side) {
            // A ramp across the patch: a little variance, almost no Laplacian,
            // so it is smooth without being a flat zero-variance hole.
            let across = f32::from(u16::try_from(x - left).unwrap_or(0)) / side.max(1) as f32;
            90.0 + 70.0 * across
        } else if (y as f32) < HORIZON * height as f32 {
            // Sky: bright, and gently brighter towards the top.
            let down = y as f32 / (HORIZON * height as f32).max(1.0);
            205.0 + 45.0 * (1.0 - down)
        } else {
            // Ground: a checkerboard fine enough to be sharp, coarse enough to
            // survive a downscale to the working size.
            let square = ((x / 6) + (y / 6)) % 2;
            if square == 0 { 60.0 } else { 185.0 }
        };
        let value = to_u8(grey);
        Rgb([value, value, value])
    })
}

/// A different scene of the same size: dense seeded noise, which has gradient
/// energy in every direction and so no dominant line.
pub fn noise(width: u32, height: u32, seed: u32) -> RgbImage {
    let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    ImageBuffer::from_fn(width, height, |_, _| {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let value = u8::try_from((state >> 24) & 0xFF).unwrap_or(0);
        Rgb([value, value, value])
    })
}

/// A scene whose horizon is drawn at `degrees`, positive falling to the right:
/// the answer key for a roll measurement.
///
/// Drawn at the angle rather than rotated afterwards, because rotating a
/// hard-edged synthetic picture with bilinear sampling quantises its gradients
/// to the axes, and the ground truth would then be the resampler's rather than
/// the measure's. Both halves are flat, so nothing but the horizon has a
/// gradient: this frame answers "can the detector find a known angle", and the
/// noise in [`noise`] answers "does it invent one".
pub fn tilted_scene(width: u32, height: u32, degrees: f32) -> RgbImage {
    let centre_x = width as f32 / 2.0;
    let slope = degrees.to_radians().tan();
    let horizon_at = HORIZON * height as f32;
    ImageBuffer::from_fn(width, height, |x, y| {
        let horizon = horizon_at + (x as f32 - centre_x) * slope;
        // Downwards in the frame is below the horizon.
        let down = smooth_edge(y as f32 + 0.5 - horizon);
        let value = 235.0 + (95.0 - 235.0) * down;
        Rgb([to_u8(value); 3])
    })
}

/// The frame blurred by a Gaussian of `sigma` pixels in both directions: an
/// isotropic blur, as a lens that missed focus gives.
pub fn blurred(image: &RgbImage, sigma: f32) -> RgbImage {
    let kernel = gaussian(sigma);
    convolve(&convolve(image, &kernel, true), &kernel, false)
}

/// The frame smeared `pixels` pixels horizontally: a camera that shook along
/// one axis, which no amount of isotropic blur reproduces.
pub fn shaken(image: &RgbImage, pixels: u32) -> RgbImage {
    let radius = i32::try_from(pixels.max(1) / 2).unwrap_or(1);
    let share = 1.0 / (2.0 * radius as f32 + 1.0);
    let kernel: Vec<f32> = (0..=2 * radius).map(|_| share).collect();
    convolve(image, &kernel, true)
}

/// The frame `stops` stops brighter, clipped at both ends: two stops over blows
/// the sky out.
pub fn brighter(image: &RgbImage, stops: f32) -> RgbImage {
    let scale = 2f32.powf(stops);
    ImageBuffer::from_fn(image.width(), image.height(), |x, y| {
        let pixel = image.get_pixel(x, y);
        let clamped = |index: usize| to_u8(f32::from(pixel.0[index]) * scale);
        Rgb([clamped(0), clamped(1), clamped(2)])
    })
}

/// The centre of the deliberately soft patch, in the frame's pixels.
pub fn soft_centre(width: u32, height: u32) -> (u32, u32) {
    let (left, top, side) = patch(width, height);
    (left + side / 2, top + side / 2)
}

/// A sharp point of the ground, in the frame's pixels: inside the checkerboard
/// and outside the soft patch.
pub fn sharp_centre(width: u32, height: u32) -> (u32, u32) {
    (width / 2, height - height / 8)
}

/// The soft patch as `(left, top, side)`, in the frame's pixels.
fn patch(width: u32, height: u32) -> (u32, u32, u32) {
    let (left, top, side) = SOFT_PATCH;
    let x = (left * width as f32) as u32;
    let y = (top * height as f32) as u32;
    let side = (side * width as f32) as u32;
    (x, y, side)
}

fn inside(x: u32, y: u32, left: u32, top: u32, side: u32) -> bool {
    x >= left && x < left + side && y >= top && y < top + side
}

fn gaussian(sigma: f32) -> Vec<f32> {
    let radius = (3.0 * sigma).round().max(1.0) as i32;
    let weight = |offset: i32| {
        let distance = offset as f32;
        (-(distance * distance) / (2.0 * sigma * sigma)).exp()
    };
    let mut kernel: Vec<f32> = (-radius..=radius).map(weight).collect();
    let total: f32 = kernel.iter().sum();
    for value in &mut kernel {
        *value /= total;
    }
    kernel
}

/// One separable pass of `kernel` over `image`, across when `horizontal`.
fn convolve(image: &RgbImage, kernel: &[f32], horizontal: bool) -> RgbImage {
    let radius = i32::try_from(kernel.len() / 2).unwrap_or(0);
    let (width, height) = image.dimensions();
    let sample = |x: i32, y: i32, channel: usize| -> f32 {
        let x = x.clamp(0, i32::try_from(width).unwrap_or(1) - 1);
        let y = y.clamp(0, i32::try_from(height).unwrap_or(1) - 1);
        f32::from(
            image
                .get_pixel(u32::try_from(x).unwrap_or(0), u32::try_from(y).unwrap_or(0))
                .0[channel],
        )
    };
    ImageBuffer::from_fn(width, height, |x, y| {
        let at = (i32::try_from(x).unwrap_or(0), i32::try_from(y).unwrap_or(0));
        let channel = |index: usize| {
            let mut sum = 0.0;
            for (offset, weight) in kernel.iter().enumerate() {
                let step = i32::try_from(offset).unwrap_or(0) - radius;
                let (sx, sy) = if horizontal {
                    (at.0 + step, at.1)
                } else {
                    (at.0, at.1 + step)
                };
                sum += weight * sample(sx, sy, index);
            }
            to_u8(sum)
        };
        Rgb([channel(0), channel(1), channel(2)])
    })
}

/// A soft edge: 0 well above the line, 1 well below it, blended across a pixel
/// and a half so the horizon is anti-aliased as a lens would leave it.
fn smooth_edge(distance: f32) -> f32 {
    let t = (distance / 1.5).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A grey level, clamped to the byte a JPEG can hold.
fn to_u8(value: f32) -> u8 {
    let rounded = value.round().clamp(0.0, 255.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let byte = rounded as u8;
    byte
}
