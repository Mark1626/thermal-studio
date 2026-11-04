//! Pen-and-ink "sketch" stylisation: XDoG outlines plus tonal cross-hatching.
//!
//! Instead of reproducing tone with scattered dots, this draws **outlines** where the
//! image has edges and **hatching strokes** whose density follows the tone. Continuous
//! strokes survive thermal dot gain far better than isolated dots.
//!
//! Outlines use the thresholded difference of Gaussians from Winnemöller et al.,
//! "XDoG: An eXtended difference-of-Gaussians compendium" (2012):
//! a dot is inked where `G_σ(I) − τ·G_{1.6σ}(I) < 0`, i.e. on the dark side of edges.

use crate::prepare::gaussian_blur;
use crate::{Bitmap, GrayF32};

/// Parameters for [`sketch`]; see the matching fields on `DitherOptions`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SketchParams {
    /// Outline scale σ in dots.
    pub line: f32,
    /// Edge sensitivity 0–1 (maps to τ 0.90–0.995).
    pub edges: f32,
    /// Hatching amount 0–1 (0 = outlines only).
    pub shading: f32,
    /// Distance between hatch strokes in dots.
    pub spacing: u8,
    /// Tone bias: positive = darker (from `DitherOptions::threshold − 0.5`).
    pub bias: f32,
}

pub(crate) fn sketch(img: &GrayF32, p: &SketchParams) -> Bitmap {
    let sigma = p.line.clamp(0.3, 4.0);
    let near = gaussian_blur(img, sigma);
    let wide = gaussian_blur(img, sigma * 1.6);
    let tau = 0.90 + 0.095 * p.edges.clamp(0.0, 1.0);

    // Tone levels below which each hatch layer appears. All collapse to 0 at shading = 0.
    let s = p.shading.clamp(0.0, 1.0);
    let (light, medium, dark, fill) = (0.80 * s, 0.55 * s, 0.32 * s, 0.10 * s);
    let gap = p.spacing.clamp(3, 16) as i64;

    Bitmap::from_fn(img.width, img.height, |x, y| {
        let i = y as usize * img.width as usize + x as usize;
        // Outline on the dark side of edges.
        if near.data[i] - tau * wide.data[i] < 0.0 {
            return true;
        }
        // Hatching follows the smoothed tone so strokes outline shapes rather than noise.
        let t = wide.data[i] - p.bias;
        let (x, y) = (x as i64, y as i64);
        t < fill
            || (t < light && (x + y).rem_euclid(gap) == 0)
            || (t < medium && (x - y).rem_euclid(gap) == 0)
            || (t < dark && (x + y + gap / 2).rem_euclid(gap) == 0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> SketchParams {
        SketchParams {
            line: 1.0,
            edges: 0.6,
            shading: 0.6,
            spacing: 6,
            bias: 0.0,
        }
    }

    #[test]
    fn outlines_a_step_edge_and_leaves_flats_clean() {
        // Left half mid-gray, right half white: a vertical edge at x = 32.
        let mut img = GrayF32::new(64, 64, 1.0);
        for y in 0..64 {
            for x in 0..32 {
                img.set(x, y, 0.7);
            }
        }
        let bmp = sketch(
            &img,
            &SketchParams {
                shading: 0.0,
                ..params()
            },
        );
        let column_ink = |x: u32| (0..64).filter(|&y| bmp.get(x, y)).count();
        assert!(
            (28..34).any(|x| column_ink(x) > 56),
            "no outline near the edge"
        );
        assert_eq!(column_ink(10), 0, "flat gray should not be outlined");
        assert_eq!(column_ink(55), 0, "white should stay white");
    }

    #[test]
    fn darker_tone_gets_denser_hatching() {
        let ink = |v: f32| sketch(&GrayF32::new(64, 64, v), &params()).ink_ratio();
        let (light, mid, dark) = (ink(0.45), ink(0.30), ink(0.15));
        assert!(
            light > 0.05 && light < mid && mid < dark,
            "{light} {mid} {dark}"
        );
        assert_eq!(ink(0.9), 0.0);
    }

    #[test]
    fn zero_shading_is_outlines_only() {
        let bmp = sketch(
            &GrayF32::new(64, 64, 0.2),
            &SketchParams {
                shading: 0.0,
                ..params()
            },
        );
        assert_eq!(bmp.ink_dots(), 0);
    }
}
