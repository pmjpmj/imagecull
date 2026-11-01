//! What a frame's own pixels can say, without asking a model.
//!
//! The rubric asks the model whether a frame is in focus, whether it is blurred
//! and whether the horizon is level. A vision model reading a 1568 px preview is
//! making a coarse aesthetic call; it is not measuring acutance, and it cannot
//! see the difference between "sharp" and "sharpest" across two frames of the
//! same moment. These measures are what the pixels themselves can answer:
//! [`acutance`](Measures::acutance), [`shake`](Measures::shake),
//! [`roll_deg`](Measures::roll_deg) with the
//! [`roll_strength`](Measures::roll_strength) that says whether there was a line
//! to measure at all, and [`clip_hi`](Measures::clip_hi) /
//! [`clip_lo`](Measures::clip_lo). The camera's own autofocus report joins them
//! as [`af_in_focus`](Measures::af_in_focus), and
//! [`af_off_subject`](Measures::af_off_subject) says whether that point landed
//! on something sharp — which is not the same fact as the camera confirming.
//!
//! Every number here is deterministic: a pure function of the preview's pixels
//! and the two fixed parameters below. Nothing in this module opens a socket,
//! reads a clock, or consults a [`Decider`](crate::scoring::Decider). That is
//! the point of it: a measure that silently decided which frames get a model
//! request would turn a plausible wrong number into missing scores, so measures
//! ship first as a command that needs no server at all.
//!
//! # The working image
//!
//! Measures are taken on the largest embedded preview, oriented, with its
//! longest edge at most [`WORK_EDGE`] px. The fixed size is not arbitrary: it
//! makes a crop-mode frame comparable with a full-frame one, and 2048 is above
//! the 1568 px sent to the model, so a ranking is not reading the encoder's
//! artefacts. Never up: a preview smaller than the working size is measured as
//! it is.
//!
//! # What a measure is not
//!
//! Acutance is not a score out of ten. It ranks, within a group, frames of the
//! same scene; comparing it across a grey wall and a crowded street means
//! nothing, which is exactly why it is contrast-normalised per block and read at
//! a percentile. Nothing here changes any of the six answers.

use image::RgbImage;

use crate::error::Result;
use crate::preprocess;
use crate::preview::Embedded;

/// The longest edge a measure is taken at, in pixels. Above the 1568 px sent to
/// the model and fixed, so frames of different sensors compare.
pub const WORK_EDGE: u32 = 2048;

/// The side of one acutance block, in pixels of the working image.
pub const BLOCK: u32 = 64;

/// The width of one bin of the orientation histogram, in degrees.
const ROLL_BIN_DEG: f64 = 0.1;

/// How far either side of an axis a horizon may lie and still be found.
const ROLL_PEAK_WINDOW_DEG: f64 = 10.0;

/// How far either side of its peak counts as the peak's own weight.
///
/// One degree: narrower than the wobble a real horizon's edge carries, and
/// narrow enough that a uniform spread of directions holds barely a hundredth of
/// its weight inside it, which is what makes the strength below mean something.
const ROLL_STRENGTH_WINDOW_DEG: f64 = 1.0;

/// The least share of the frame's orientation weight a peak must hold to be
/// called a line at all (see [`Measures::roll_strength`]).
///
/// Below it, `roll_deg` is empty rather than `0.0`: a portrait against a plain
/// wall has no horizon, and reporting 0° would invent a level one. A uniform
/// spread of directions holds about 0.01 of its weight in the peak's window, so
/// the floor is set at several times that rather than at a tidy number. It is a
/// default to be validated against a photographer's eye, not a tuned one.
pub const ROLL_MIN_STRENGTH: f64 = 0.05;

/// How much of the frame's acutance the window around the focus point must keep
/// to count as being on the subject.
pub const AF_RATIO: f64 = 0.6;

/// The side of the default focus window, as a fraction of the working long edge,
/// used when the camera records where it focused but not how large the point is.
pub const AF_WINDOW_FRACTION: f64 = 1.0 / 16.0;

/// The pixels a measure is taken from: oriented, greyscale, at the working size,
/// with the clipping of the whole preview already counted.
#[derive(Debug, Clone)]
pub struct Working {
    /// Width after orientation and the working-size fit.
    pub width: u32,
    /// Height after orientation and the working-size fit.
    pub height: u32,
    /// Luminance, row by row, `0.0..=255.0`.
    grey: Vec<f32>,
    /// Percentage of channel samples at the top of the scale, over the whole
    /// preview as it was decoded.
    clip_hi: f64,
    /// Percentage of channel samples at the bottom of the scale.
    clip_lo: f64,
}

impl Working {
    /// Fit an upright RGB image to the working size and take its grey.
    ///
    /// Clipping is counted here, on the image as given, before any downscale can
    /// average a blown pixel back into range.
    pub fn from_rgb(image: &RgbImage) -> Result<Working> {
        let (clip_hi, clip_lo) = clipping(image);
        let fitted = preprocess::fit(image, WORK_EDGE)?;
        let grey = fitted
            .pixels()
            .map(|pixel| {
                0.299 * f32::from(pixel.0[0])
                    + 0.587 * f32::from(pixel.0[1])
                    + 0.114 * f32::from(pixel.0[2])
            })
            .collect();
        Ok(Working {
            width: fitted.width(),
            height: fitted.height(),
            grey,
            clip_hi,
            clip_lo,
        })
    }

    /// The luminance at `(x, y)`.
    fn at(&self, x: u32, y: u32) -> f32 {
        let index = (y * self.width + x) as usize;
        self.grey.get(index).copied().unwrap_or(0.0)
    }

    /// The longest edge of the working image.
    fn long_edge(&self) -> u32 {
        self.width.max(self.height)
    }

    /// The 95th percentile of the contrast-normalised Laplacian variance over
    /// blocks, in the rectangle `(x, y, width, height)`, or `None` when no block
    /// in it has any contrast to normalise by.
    fn acutance_in(&self, x: u32, y: u32, width: u32, height: u32) -> Option<f64> {
        let mut ratios = Vec::new();
        for block_y in steps(y, height, BLOCK) {
            for block_x in steps(x, width, BLOCK) {
                if let Some(ratio) = self.block_ratio(block_x, block_y) {
                    ratios.push(ratio);
                }
            }
        }
        percentile(&mut ratios, 0.95)
    }

    /// One block's `var(∇²I) / var(I)`: how much sharper a block is than its own
    /// contrast allows, so a flat wall and a busy street share a scale.
    ///
    /// `None` for a block with too little extent to hold a Laplacian, or with no
    /// variance at all: a uniform block has no shape to be sharp or soft, and
    /// dividing by it would manufacture one.
    fn block_ratio(&self, x: u32, y: u32) -> Option<f64> {
        let end_x = (x + BLOCK).min(self.width);
        let end_y = (y + BLOCK).min(self.height);
        if end_x.saturating_sub(x) < 3 || end_y.saturating_sub(y) < 3 {
            return None;
        }
        let mut count = 0.0;
        let (mut sum_i, mut sum_i2, mut sum_l, mut sum_l2) = (0.0, 0.0, 0.0, 0.0);
        for current_y in (y + 1)..(end_y - 1) {
            for current_x in (x + 1)..(end_x - 1) {
                let value = f64::from(self.at(current_x, current_y));
                let laplacian = f64::from(
                    4.0 * self.at(current_x, current_y)
                        - self.at(current_x - 1, current_y)
                        - self.at(current_x + 1, current_y)
                        - self.at(current_x, current_y - 1)
                        - self.at(current_x, current_y + 1),
                );
                count += 1.0;
                sum_i += value;
                sum_i2 += value * value;
                sum_l += laplacian;
                sum_l2 += laplacian * laplacian;
            }
        }
        if count < 1.0 {
            return None;
        }
        let variance_i = sum_i2 / count - (sum_i / count).powi(2);
        // A block with no variance is either a flat wall or a clipped white
        // patch: no shape either way, and a ratio would be `0/0`.
        if variance_i <= 1e-6 {
            return None;
        }
        let variance_l = sum_l2 / count - (sum_l / count).powi(2);
        Some(variance_l / variance_i)
    }

    /// The gradient energy in each axis, as `(E_x, E_y)`.
    ///
    /// Central differences over the interior: a horizontal motion blur smears
    /// along x and leaves the x energy short, while an isotropic blur of the
    /// same acutance takes the same bite out of both.
    fn gradient_energy(&self) -> (f64, f64) {
        let (mut energy_x, mut energy_y) = (0.0, 0.0);
        if self.width < 3 || self.height < 3 {
            return (energy_x, energy_y);
        }
        for y in 1..(self.height - 1) {
            for x in 1..(self.width - 1) {
                let dx = f64::from(self.at(x + 1, y) - self.at(x - 1, y));
                let dy = f64::from(self.at(x, y + 1) - self.at(x, y - 1));
                energy_x += dx * dx;
                energy_y += dy * dy;
            }
        }
        (energy_x, energy_y)
    }

    /// The horizon: the offset of the strongest line from its axis, in degrees,
    /// and the share of the frame's orientation weight that line holds.
    ///
    /// Positive roll is a line that falls towards the right in the image's own
    /// coordinates, where y grows downwards. The angle is empty when the peak
    /// does not clear [`ROLL_MIN_STRENGTH`]; the strength is always given, so
    /// "no lines" and "a level line" are different answers rather than the same
    /// one.
    fn roll(&self) -> (Option<f64>, Option<f64>) {
        if self.width < 3 || self.height < 3 {
            return (None, None);
        }
        let bins = (180.0 / ROLL_BIN_DEG).round() as usize;
        let mut histogram = vec![0.0f64; bins];
        let mut total = 0.0;
        for y in 1..(self.height - 1) {
            for x in 1..(self.width - 1) {
                let dx = f64::from(self.at(x + 1, y) - self.at(x - 1, y));
                let dy = f64::from(self.at(x, y + 1) - self.at(x, y - 1));
                let magnitude = dx.hypot(dy);
                if magnitude <= 0.0 {
                    continue;
                }
                let angle = dy.atan2(dx).to_degrees().rem_euclid(180.0);
                let bin = ((angle / ROLL_BIN_DEG).round() as usize).min(bins - 1);
                histogram[bin] += magnitude;
                total += magnitude;
            }
        }
        if total <= 0.0 {
            return (None, None);
        }

        let half_peak = (ROLL_PEAK_WINDOW_DEG / ROLL_BIN_DEG).round() as isize;
        let half_strength = (ROLL_STRENGTH_WINDOW_DEG / ROLL_BIN_DEG).round() as isize;
        let bins_i = isize::try_from(bins).unwrap_or(1);
        let wrap = |index: isize| index.rem_euclid(bins_i) as usize;
        let bin_of = |degrees: f64| -> isize { (degrees / ROLL_BIN_DEG).round() as isize };

        // A horizontal line's gradient points along y (90°); a vertical line's
        // along x (0°). Whichever peak holds more of the weight wins, and the
        // horizontal one keeps a tie: a horizon is the line a photographer
        // means by roll.
        let mut best: Option<(f64, usize, f64)> = None;
        for axis in [90.0f64, 0.0] {
            let centre = bin_of(axis);
            let Some(peak) = (-half_peak..=half_peak)
                .map(|offset| wrap(centre + offset))
                .max_by(|left, right| {
                    histogram[*left]
                        .partial_cmp(&histogram[*right])
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
            else {
                continue;
            };
            let share: f64 = (-half_strength..=half_strength)
                .map(|offset| histogram[wrap(peak as isize + offset)])
                .sum::<f64>()
                / total;
            if best.is_none_or(|(_, _, winner)| share > winner) {
                best = Some((axis, peak, share));
            }
        }

        let Some((axis, peak, strength)) = best else {
            return (None, None);
        };
        // The peak bin says *where* the line is; the magnitude-weighted mean of
        // the whole search window says how far off the axis, to better than a
        // bin. The mean is taken over the window and not over the peak alone
        // because 8-bit gradients quantise: an edge a degree and a half off
        // level lands as a scatter of bins from 90° to 93°, and the mode or the
        // mean of a window cut out of one side of that scatter is off by a few
        // tenths of a degree — while the mean of the whole scatter is the line.
        let mut weight = 0.0;
        let mut weighted = 0.0;
        for step in -half_peak..=half_peak {
            let bin = wrap(peak as isize + step);
            let angle = bin as f64 * ROLL_BIN_DEG;
            // Each bin's offset from the axis on its own, wrapped to
            // `(-90, 90]`, so the average is taken around the axis and never
            // across the histogram's own wrap at 0°/180°.
            let delta = ((angle - axis + 90.0).rem_euclid(180.0)) - 90.0;
            weight += histogram[bin];
            weighted += histogram[bin] * delta;
        }
        let offset = if weight > 0.0 { weighted / weight } else { 0.0 };
        let degrees = if strength < ROLL_MIN_STRENGTH {
            None
        } else {
            Some(offset)
        };
        (degrees, Some(strength))
    }

    /// Whether the camera's focus point landed on something sharp, given the
    /// frame's own acutance.
    ///
    /// `None` when there is no point to compare (no focus report, or one outside
    /// the frame) or no acutance to compare against: an unknown is never a
    /// `false`, which would read as "the subject is fine".
    fn af_off_subject(
        &self,
        af: &AfPoint,
        orientation: u16,
        acutance: Option<f64>,
    ) -> Option<bool> {
        let frame_acutance = acutance?;
        let (frame_width, frame_height) = (af.frame.0 as f32, af.frame.1 as f32);
        if frame_width <= 0.0 || frame_height <= 0.0 {
            return None;
        }
        let (normalised_x, normalised_y) = orient_point(
            orientation,
            (
                af.center.0 as f32 / frame_width,
                af.center.1 as f32 / frame_height,
            ),
        );
        if !(0.0..=1.0).contains(&normalised_x) || !(0.0..=1.0).contains(&normalised_y) {
            return None;
        }
        let work_width = self.width as f32;
        let work_height = self.height as f32;
        let centre_x = normalised_x * work_width;
        let centre_y = normalised_y * work_height;

        // The camera's own area when it records one, scaled with the frame;
        // otherwise a window a fraction of the working long edge. Never smaller
        // than a block, or there would be nothing to average.
        let scale = work_width / frame_width;
        let side = af
            .size
            .map(|(width, height)| width.max(height) as f32 * scale)
            .unwrap_or(self.long_edge() as f32 * AF_WINDOW_FRACTION as f32)
            .max(BLOCK as f32);
        let half = side / 2.0;
        let left = (centre_x - half).max(0.0);
        let top = (centre_y - half).max(0.0);
        let right = (centre_x + half).min(work_width);
        let bottom = (centre_y + half).min(work_height);
        if right - left < 3.0 || bottom - top < 3.0 {
            return None;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let window = self.acutance_in(
            left.round() as u32,
            top.round() as u32,
            (right - left).round() as u32,
            (bottom - top).round() as u32,
        )?;
        Some(window < AF_RATIO * frame_acutance)
    }
}

/// Where the camera said it focused, in the frame the camera recorded it in.
///
/// `frame` is the sensor frame the point is expressed in (the raw file's own
/// size), never the preview's: the point is normalised against it, so it does
/// not matter that the preview is smaller. `in_focus` is the camera's own
/// confirmation, `None` when it records none — which is every manual-focus shot
/// and every body that writes no such tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AfPoint {
    /// The sensor frame the point's coordinates count in, `(width, height)`.
    pub frame: (u32, u32),
    /// The point's centre, in that frame.
    pub center: (u32, u32),
    /// The area the camera used, when it records one.
    pub size: Option<(u32, u32)>,
    /// Whether the camera confirmed focus.
    pub in_focus: Option<bool>,
}

/// One frame's measures. A measure that could not be taken is `None`, never a
/// zero: "no answer" and "a bad answer" are different facts.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Measures {
    /// The 95th percentile of `var(∇²I)/var(I)` over 64×64 blocks: whether
    /// anything in the frame is genuinely sharp. Higher is sharper.
    pub acutance: Option<f64>,
    /// `|E_x − E_y| / (E_x + E_y)`: how directional the frame's gradient energy
    /// is. High means the camera moved along one axis; isotropic focus blur
    /// leaves it near what the scene had.
    pub shake: Option<f64>,
    /// The horizon's offset from level, in degrees, positive falling to the
    /// right. Empty when the frame has no dominant line.
    pub roll_deg: Option<f64>,
    /// The winning line's share of the frame's orientation weight, so "no
    /// lines" is distinguishable from "0.0°".
    pub roll_strength: Option<f64>,
    /// Percentage of channel samples at the top of the scale.
    pub clip_hi: Option<f64>,
    /// Percentage of channel samples at the bottom of the scale.
    pub clip_lo: Option<f64>,
    /// What the camera reported about focus, `None` when it reported nothing.
    pub af_in_focus: Option<bool>,
    /// Whether the camera's focus point sits on something far softer than the
    /// frame's own sharpest region.
    pub af_off_subject: Option<bool>,
}

/// Measure one embedded preview. No server, no decider, no writes.
///
/// The `af` point, when the file records one, is mapped onto the preview through
/// the frame's EXIF orientation, so a portrait frame's point lands on the
/// subject rather than a quarter turn away.
pub fn of_preview(embedded: &Embedded, af: Option<&AfPoint>) -> Result<Measures> {
    let image = preprocess::decode(embedded)?;
    let working = Working::from_rgb(&image.to_rgb8())?;
    Ok(of_working(&working, af, embedded.orientation))
}

/// Measure a working image that has already been decoded, oriented and fitted.
pub fn of_working(working: &Working, af: Option<&AfPoint>, orientation: u16) -> Measures {
    let acutance = working.acutance_in(0, 0, working.width, working.height);
    let (energy_x, energy_y) = working.gradient_energy();
    let shake = if energy_x + energy_y > 0.0 {
        Some((energy_x - energy_y).abs() / (energy_x + energy_y))
    } else {
        None
    };
    let (roll_deg, roll_strength) = working.roll();
    Measures {
        acutance,
        shake,
        roll_deg,
        roll_strength,
        clip_hi: Some(working.clip_hi),
        clip_lo: Some(working.clip_lo),
        af_in_focus: af.and_then(|af| af.in_focus),
        af_off_subject: af.and_then(|af| working.af_off_subject(af, orientation, acutance)),
    }
}

/// The point as it sits on the upright frame, after the EXIF orientation is
/// applied. Coordinates are fractions of the frame, so a preview and a sensor
/// frame map onto each other without knowing either size.
///
/// An unknown tag is treated as upright, as the reader already normalises it.
pub fn orient_point(orientation: u16, at: (f32, f32)) -> (f32, f32) {
    let (x, y) = at;
    match orientation {
        2 => (1.0 - x, y),
        3 => (1.0 - x, 1.0 - y),
        4 => (x, 1.0 - y),
        5 => (y, x),
        6 => (1.0 - y, x),
        7 => (1.0 - y, 1.0 - x),
        8 => (y, 1.0 - x),
        _ => (x, y),
    }
}

/// The percentage of channel samples at each end of the scale.
fn clipping(image: &RgbImage) -> (f64, f64) {
    let mut high = 0u64;
    let mut low = 0u64;
    for pixel in image.pixels() {
        for &channel in &pixel.0 {
            if channel == u8::MAX {
                high += 1;
            } else if channel == 0 {
                low += 1;
            }
        }
    }
    let samples = u64::from(image.width()) * u64::from(image.height()) * 3;
    if samples == 0 {
        return (0.0, 0.0);
    }
    let samples = samples as f64;
    (100.0 * high as f64 / samples, 100.0 * low as f64 / samples)
}

/// Where the blocks of `BLOCK` px start inside `start..start + length`.
///
/// The last block may be short; a block too short to hold a Laplacian is dropped
/// by [`Working::block_ratio`] rather than padded into existence.
fn steps(start: u32, length: u32, block: u32) -> Vec<u32> {
    let block = block.max(1);
    let mut starts = Vec::new();
    let mut offset = 0;
    while offset < length {
        starts.push(start + offset);
        offset += block;
    }
    if starts.is_empty() {
        starts.push(start);
    }
    starts
}

/// The `fraction` percentile of `values` by the nearest-rank rule, or `None` for
/// an empty sample.
fn percentile(values: &mut [f64], fraction: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let count = values.len();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let rank = ((fraction * count as f64).ceil() as usize).clamp(1, count);
    values.get(rank - 1).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_percentile_is_the_nearest_rank_and_a_sample_of_none_is_empty() {
        let mut values: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(percentile(&mut values, 0.95), Some(95.0));
        assert_eq!(percentile(&mut values, 0.5), Some(50.0));
        let mut single = vec![7.0];
        assert_eq!(percentile(&mut single, 0.95), Some(7.0));
        assert_eq!(percentile(&mut [], 0.95), None);
    }

    #[test]
    fn orientation_maps_a_point_the_way_the_image_is_turned() {
        let at = (0.25, 0.75);
        assert_eq!(orient_point(1, at), at);
        assert_eq!(orient_point(2, at), (0.75, 0.75));
        assert_eq!(orient_point(3, at), (0.75, 0.25));
        assert_eq!(orient_point(4, at), (0.25, 0.25));
        assert_eq!(orient_point(5, at), (0.75, 0.25));
        assert_eq!(orient_point(6, at), (0.25, 0.25));
        assert_eq!(orient_point(7, at), (0.25, 0.75));
        assert_eq!(orient_point(8, at), (0.75, 0.75));
        // Every turn is undone by its own inverse, which is the property the
        // focus mapping relies on.
        for (turn, inverse) in [(2, 2), (3, 3), (4, 4), (5, 5), (6, 8), (7, 7), (8, 6)] {
            let once = orient_point(turn, at);
            let back = orient_point(inverse, once);
            assert!(
                (back.0 - at.0).abs() < 1e-6 && (back.1 - at.1).abs() < 1e-6,
                "{turn} then {inverse} is {back:?}, not {at:?}"
            );
        }
    }

    #[test]
    fn a_uniform_block_has_no_acutance_and_a_textured_one_does() {
        let flat = image::ImageBuffer::from_pixel(80, 80, image::Rgb([120, 120, 120]));
        let flat = Working::from_rgb(&flat).expect("a working image");
        assert_eq!(flat.acutance_in(0, 0, 80, 80), None);

        let textured: RgbImage = image::ImageBuffer::from_fn(80, 80, |x, _| {
            if (x / 2) % 2 == 0 {
                image::Rgb([40, 40, 40])
            } else {
                image::Rgb([200, 200, 200])
            }
        });
        let sharp = Working::from_rgb(&textured).expect("a working image");
        assert!(sharp.acutance_in(0, 0, 80, 80).is_some());
    }
}
