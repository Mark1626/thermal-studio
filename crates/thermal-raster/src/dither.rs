//! Grayscale → 1-bit conversion.

use crate::sketch::{SketchParams, sketch};
use crate::{Bitmap, GrayF32};

/// Broad family of an [`Algorithm`]; tells a UI which options apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Threshold,
    Ordered,
    ErrorDiffusion,
    /// Artistic rendering rather than tone reproduction (e.g. [`Algorithm::Sketch`]).
    Stylized,
}

/// Dithering algorithm.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Algorithm {
    /// Hard cut at `threshold`. For text, logos and line art.
    Threshold,
    Bayer2,
    Bayer4,
    Bayer8,
    Bayer16,
    /// Clustered-dot screen: dots grow from the centre of each cell. Isolated single
    /// dots smear on thermal paper; clustered dots stay crisp.
    Halftone,
    FloydSteinberg,
    /// Propagates only 6/8 of the error: punchier highlights and shadows, fewer
    /// stray dots. A good default for thermal printers.
    #[default]
    Atkinson,
    JarvisJudiceNinke,
    Stucki,
    Burkes,
    Sierra,
    TwoRowSierra,
    SierraLite,
    /// Pen-and-ink drawing: outlines along edges plus diagonal hatching and
    /// cross-hatching for shading. Strokes stay crisp on thermal paper.
    Sketch,
}

impl Algorithm {
    pub const ALL: [Algorithm; 15] = [
        Algorithm::Threshold,
        Algorithm::Bayer2,
        Algorithm::Bayer4,
        Algorithm::Bayer8,
        Algorithm::Bayer16,
        Algorithm::Halftone,
        Algorithm::FloydSteinberg,
        Algorithm::Atkinson,
        Algorithm::JarvisJudiceNinke,
        Algorithm::Stucki,
        Algorithm::Burkes,
        Algorithm::Sierra,
        Algorithm::TwoRowSierra,
        Algorithm::SierraLite,
        Algorithm::Sketch,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Algorithm::Threshold => "Threshold",
            Algorithm::Bayer2 => "Bayer 2×2",
            Algorithm::Bayer4 => "Bayer 4×4",
            Algorithm::Bayer8 => "Bayer 8×8",
            Algorithm::Bayer16 => "Bayer 16×16",
            Algorithm::Halftone => "Halftone (clustered dot)",
            Algorithm::FloydSteinberg => "Floyd–Steinberg",
            Algorithm::Atkinson => "Atkinson",
            Algorithm::JarvisJudiceNinke => "Jarvis–Judice–Ninke",
            Algorithm::Stucki => "Stucki",
            Algorithm::Burkes => "Burkes",
            Algorithm::Sierra => "Sierra",
            Algorithm::TwoRowSierra => "Two-row Sierra",
            Algorithm::SierraLite => "Sierra Lite",
            Algorithm::Sketch => "Sketch (ink + hatching)",
        }
    }

    /// One-line guidance for choosing an algorithm on thermal paper.
    pub fn description(self) -> &'static str {
        match self {
            Algorithm::Threshold => "Hard black/white cut. Text, logos, QR codes, line art.",
            Algorithm::Bayer2 | Algorithm::Bayer4 | Algorithm::Bayer8 | Algorithm::Bayer16 => {
                "Ordered cross-hatch pattern. Fast, regular, retro look; fine patterns may smear."
            }
            Algorithm::Halftone => {
                "Newspaper-style dots that grow from cell centres. Robust against thermal smear."
            }
            Algorithm::FloydSteinberg => {
                "Classic error diffusion. Most detail, but many isolated dots."
            }
            Algorithm::Atkinson => {
                "Error diffusion with crisp highlights and shadows. Good thermal default."
            }
            Algorithm::JarvisJudiceNinke | Algorithm::Stucki => {
                "Wide error diffusion. Smooth gradients, softer look."
            }
            Algorithm::Burkes | Algorithm::Sierra | Algorithm::TwoRowSierra => {
                "Error diffusion between Floyd–Steinberg and JJN in smoothness and speed."
            }
            Algorithm::SierraLite => "Minimal error diffusion. Fast, slightly noisier.",
            Algorithm::Sketch => {
                "Pen drawing: outlines on edges, hatching for shadows. Very crisp on thermal paper."
            }
        }
    }

    /// Stable lowercase identifier for CLIs and config files.
    pub fn id(self) -> &'static str {
        match self {
            Algorithm::Threshold => "threshold",
            Algorithm::Bayer2 => "bayer2",
            Algorithm::Bayer4 => "bayer4",
            Algorithm::Bayer8 => "bayer8",
            Algorithm::Bayer16 => "bayer16",
            Algorithm::Halftone => "halftone",
            Algorithm::FloydSteinberg => "floyd-steinberg",
            Algorithm::Atkinson => "atkinson",
            Algorithm::JarvisJudiceNinke => "jjn",
            Algorithm::Stucki => "stucki",
            Algorithm::Burkes => "burkes",
            Algorithm::Sierra => "sierra",
            Algorithm::TwoRowSierra => "sierra2",
            Algorithm::SierraLite => "sierra-lite",
            Algorithm::Sketch => "sketch",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.id() == id)
    }

    pub fn kind(self) -> Kind {
        match self {
            Algorithm::Threshold => Kind::Threshold,
            Algorithm::Bayer2
            | Algorithm::Bayer4
            | Algorithm::Bayer8
            | Algorithm::Bayer16
            | Algorithm::Halftone => Kind::Ordered,
            Algorithm::Sketch => Kind::Stylized,
            _ => Kind::ErrorDiffusion,
        }
    }
}

/// Options for [`dither`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DitherOptions {
    pub algorithm: Algorithm,
    /// `0.0..=1.0`. The cut-off for `Threshold`; for the other algorithms it biases
    /// the result (above 0.5 = darker, below = lighter).
    pub threshold: f32,
    /// Error diffusion: alternate scan direction every row (reduces "worm" artefacts).
    pub serpentine: bool,
    /// Error diffusion: fraction of the quantisation error passed on (`1.0` = all).
    /// Lower values give cleaner flats and more contrast.
    pub diffusion: f32,
    /// Halftone: screen cell size in dots (2–16). 4–6 suits 203 dpi.
    pub halftone_cell: u8,
    /// Sketch: outline scale in dots (0.3–4). Larger = bolder, simpler outlines.
    pub sketch_line: f32,
    /// Sketch: edge sensitivity (0–1). Higher picks up fainter edges and texture.
    pub sketch_edges: f32,
    /// Sketch: amount of hatching (0–1). `0` = outlines only.
    pub sketch_shading: f32,
    /// Sketch: distance between hatch strokes in dots (3–16).
    pub sketch_hatch_spacing: u8,
}

impl Default for DitherOptions {
    fn default() -> Self {
        Self {
            algorithm: Algorithm::default(),
            threshold: 0.5,
            serpentine: true,
            diffusion: 1.0,
            halftone_cell: 5,
            sketch_line: 1.0,
            sketch_edges: 0.6,
            sketch_shading: 0.6,
            sketch_hatch_spacing: 6,
        }
    }
}

/// Convert a grayscale image (0 = black, 1 = white) to a 1-bit bitmap.
pub fn dither(img: &GrayF32, opts: &DitherOptions) -> Bitmap {
    let bias = opts.threshold - 0.5;
    match opts.algorithm {
        Algorithm::Threshold => {
            Bitmap::from_fn(img.width, img.height, |x, y| img.get(x, y) < opts.threshold)
        }
        Algorithm::Bayer2 => ordered(img, &bayer_matrix(2), 2, bias),
        Algorithm::Bayer4 => ordered(img, &bayer_matrix(4), 4, bias),
        Algorithm::Bayer8 => ordered(img, &bayer_matrix(8), 8, bias),
        Algorithm::Bayer16 => ordered(img, &bayer_matrix(16), 16, bias),
        Algorithm::Halftone => {
            let n = opts.halftone_cell.clamp(2, 16) as usize;
            ordered(img, &clustered_dot_matrix(n), n, bias)
        }
        Algorithm::FloydSteinberg => diffuse(img, FLOYD_STEINBERG, 16.0, opts),
        Algorithm::Atkinson => diffuse(img, ATKINSON, 8.0, opts),
        Algorithm::JarvisJudiceNinke => diffuse(img, JJN, 48.0, opts),
        Algorithm::Stucki => diffuse(img, STUCKI, 42.0, opts),
        Algorithm::Burkes => diffuse(img, BURKES, 32.0, opts),
        Algorithm::Sierra => diffuse(img, SIERRA, 32.0, opts),
        Algorithm::TwoRowSierra => diffuse(img, SIERRA2, 16.0, opts),
        Algorithm::SierraLite => diffuse(img, SIERRA_LITE, 4.0, opts),
        Algorithm::Sketch => sketch(
            img,
            &SketchParams {
                line: opts.sketch_line,
                edges: opts.sketch_edges,
                shading: opts.sketch_shading,
                spacing: opts.sketch_hatch_spacing,
                bias,
            },
        ),
    }
}

// --- ordered -----------------------------------------------------------------

/// Thresholds in `0..1`; a pixel is black if `value < threshold + bias`.
fn ordered(img: &GrayF32, matrix: &[f32], n: usize, bias: f32) -> Bitmap {
    Bitmap::from_fn(img.width, img.height, |x, y| {
        img.get(x, y) < matrix[(y as usize % n) * n + (x as usize % n)] + bias
    })
}

/// Recursive Bayer index matrix, normalised to thresholds in (0, 1).
fn bayer_matrix(n: usize) -> Vec<f32> {
    let mut m = vec![0u32];
    let mut size = 1;
    while size < n {
        let mut next = vec![0u32; (size * 2) * (size * 2)];
        for y in 0..size {
            for x in 0..size {
                let v = 4 * m[y * size + x];
                next[y * size * 2 + x] = v;
                next[y * size * 2 + x + size] = v + 2;
                next[(y + size) * size * 2 + x] = v + 3;
                next[(y + size) * size * 2 + x + size] = v + 1;
            }
        }
        m = next;
        size *= 2;
    }
    let cells = (n * n) as f32;
    m.into_iter().map(|v| (v as f32 + 0.5) / cells).collect()
}

/// Round-dot clustered screen: cells darken from the centre outwards.
fn clustered_dot_matrix(n: usize) -> Vec<f32> {
    let mut order: Vec<(usize, f32)> = (0..n * n)
        .map(|i| {
            let (x, y) = ((i % n) as f32, (i / n) as f32);
            let u = (x + 0.5) / n as f32 * 2.0 - 1.0;
            let v = (y + 0.5) / n as f32 * 2.0 - 1.0;
            let spot = (std::f32::consts::PI * u).cos() + (std::f32::consts::PI * v).cos();
            (i, spot)
        })
        .collect();
    // Highest spot value (cell centre) gets the highest threshold → turns black first.
    order.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut m = vec![0.0; n * n];
    let cells = (n * n) as f32;
    for (rank, (i, _)) in order.into_iter().enumerate() {
        m[i] = 1.0 - (rank as f32 + 0.5) / cells;
    }
    m
}

// --- error diffusion ---------------------------------------------------------

type Kernel = &'static [(i32, i32, f32)];

#[rustfmt::skip]
const FLOYD_STEINBERG: Kernel = &[(1, 0, 7.0), (-1, 1, 3.0), (0, 1, 5.0), (1, 1, 1.0)];
#[rustfmt::skip]
const ATKINSON: Kernel = &[(1, 0, 1.0), (2, 0, 1.0), (-1, 1, 1.0), (0, 1, 1.0), (1, 1, 1.0), (0, 2, 1.0)];
#[rustfmt::skip]
const JJN: Kernel = &[
    (1, 0, 7.0), (2, 0, 5.0),
    (-2, 1, 3.0), (-1, 1, 5.0), (0, 1, 7.0), (1, 1, 5.0), (2, 1, 3.0),
    (-2, 2, 1.0), (-1, 2, 3.0), (0, 2, 5.0), (1, 2, 3.0), (2, 2, 1.0),
];
#[rustfmt::skip]
const STUCKI: Kernel = &[
    (1, 0, 8.0), (2, 0, 4.0),
    (-2, 1, 2.0), (-1, 1, 4.0), (0, 1, 8.0), (1, 1, 4.0), (2, 1, 2.0),
    (-2, 2, 1.0), (-1, 2, 2.0), (0, 2, 4.0), (1, 2, 2.0), (2, 2, 1.0),
];
#[rustfmt::skip]
const BURKES: Kernel = &[
    (1, 0, 8.0), (2, 0, 4.0),
    (-2, 1, 2.0), (-1, 1, 4.0), (0, 1, 8.0), (1, 1, 4.0), (2, 1, 2.0),
];
#[rustfmt::skip]
const SIERRA: Kernel = &[
    (1, 0, 5.0), (2, 0, 3.0),
    (-2, 1, 2.0), (-1, 1, 4.0), (0, 1, 5.0), (1, 1, 4.0), (2, 1, 2.0),
    (-1, 2, 2.0), (0, 2, 3.0), (1, 2, 2.0),
];
#[rustfmt::skip]
const SIERRA2: Kernel = &[
    (1, 0, 4.0), (2, 0, 3.0),
    (-2, 1, 1.0), (-1, 1, 2.0), (0, 1, 3.0), (1, 1, 2.0), (2, 1, 1.0),
];
#[rustfmt::skip]
const SIERRA_LITE: Kernel = &[(1, 0, 2.0), (-1, 1, 1.0), (0, 1, 1.0)];

fn diffuse(img: &GrayF32, kernel: Kernel, divisor: f32, opts: &DitherOptions) -> Bitmap {
    let (w, h) = (img.width as i32, img.height as i32);
    let mut buf = img.data.clone();
    let mut out = Bitmap::new(img.width, img.height);
    let scale = opts.diffusion.clamp(0.0, 1.0) / divisor;

    for y in 0..h {
        let reverse = opts.serpentine && y % 2 == 1;
        for i in 0..w {
            let x = if reverse { w - 1 - i } else { i };
            let idx = (y * w + x) as usize;
            let old = buf[idx];
            let black = old < opts.threshold;
            if black {
                out.set(x as u32, y as u32, true);
            }
            let err = (old - if black { 0.0 } else { 1.0 }) * scale;
            if err == 0.0 {
                continue;
            }
            for &(dx, dy, wgt) in kernel {
                let nx = if reverse { x - dx } else { x + dx };
                let ny = y + dy;
                if nx >= 0 && nx < w && ny < h {
                    buf[(ny * w + nx) as usize] += err * wgt;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(v: f32) -> GrayF32 {
        GrayF32::new(64, 64, v)
    }

    #[test]
    fn white_stays_white_and_black_stays_black() {
        for alg in Algorithm::ALL {
            let opts = DitherOptions {
                algorithm: alg,
                ..Default::default()
            };
            assert_eq!(dither(&flat(1.0), &opts).ink_dots(), 0, "{alg:?} on white");
            assert_eq!(
                dither(&flat(0.0), &opts).ink_dots(),
                64 * 64,
                "{alg:?} on black"
            );
        }
    }

    #[test]
    fn mid_gray_is_about_half_ink() {
        for alg in Algorithm::ALL {
            if matches!(alg.kind(), Kind::Threshold | Kind::Stylized) {
                continue;
            }
            let opts = DitherOptions {
                algorithm: alg,
                ..Default::default()
            };
            let r = dither(&flat(0.5), &opts).ink_ratio();
            // Atkinson discards 2/8 of the error, so it drifts further from 50 %.
            let tol = if alg == Algorithm::Atkinson {
                0.2
            } else {
                0.06
            };
            assert!((r - 0.5).abs() < tol, "{alg:?}: ink ratio {r}");
        }
    }

    #[test]
    fn bayer_matrices_are_permutations() {
        for n in [2, 4, 8, 16] {
            let mut m = bayer_matrix(n);
            m.sort_by(f32::total_cmp);
            for (i, v) in m.iter().enumerate() {
                assert!((v - (i as f32 + 0.5) / (n * n) as f32).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn halftone_darkens_from_cell_centre() {
        let m = clustered_dot_matrix(6);
        let centre = m[2 * 6 + 2].max(m[3 * 6 + 3]);
        let corner = m[0];
        assert!(centre > corner);
    }

    #[test]
    fn ids_round_trip() {
        for alg in Algorithm::ALL {
            assert_eq!(Algorithm::from_id(alg.id()), Some(alg));
        }
    }
}
