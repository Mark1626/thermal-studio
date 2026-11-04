//! Turning arbitrary images into a grayscale buffer sized for the print head.
//!
//! Pipeline: rotate → linear-light luminance (alpha composited on white) →
//! box pre-shrink + filtered resize in linear light → sRGB-encode → adjustments.

use image::{DynamicImage, ImageBuffer, Luma, imageops::FilterType};

/// Single-channel float image on a perceptual (sRGB-encoded) scale:
/// `0.0` is black, `1.0` is white.
#[derive(Clone, Debug, PartialEq)]
pub struct GrayF32 {
    pub width: u32,
    pub height: u32,
    pub data: Vec<f32>,
}

impl GrayF32 {
    pub fn new(width: u32, height: u32, fill: f32) -> Self {
        Self {
            width,
            height,
            data: vec![fill; width as usize * height as usize],
        }
    }

    #[inline]
    pub fn get(&self, x: u32, y: u32) -> f32 {
        self.data[y as usize * self.width as usize + x as usize]
    }

    #[inline]
    pub fn set(&mut self, x: u32, y: u32, v: f32) {
        self.data[y as usize * self.width as usize + x as usize] = v;
    }

    /// 8-bit preview of the continuous-tone image (before dithering).
    pub fn to_gray_image(&self) -> image::GrayImage {
        image::GrayImage::from_fn(self.width, self.height, |x, y| {
            Luma([(self.get(x, y).clamp(0.0, 1.0) * 255.0).round() as u8])
        })
    }
}

/// Rotation applied before sizing. Use `Cw90`/`Ccw90` to print landscape images
/// along the length of the paper.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Rotation {
    #[default]
    None,
    Cw90,
    Rot180,
    Ccw90,
}

impl Rotation {
    pub const ALL: [Rotation; 4] = [
        Rotation::None,
        Rotation::Cw90,
        Rotation::Rot180,
        Rotation::Ccw90,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Rotation::None => "None",
            Rotation::Cw90 => "90° clockwise",
            Rotation::Rot180 => "180°",
            Rotation::Ccw90 => "90° counter-clockwise",
        }
    }
}

/// Resampling filter for the final resize step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ResizeFilter {
    Nearest,
    Triangle,
    CatmullRom,
    Gaussian,
    #[default]
    Lanczos3,
}

impl ResizeFilter {
    pub const ALL: [ResizeFilter; 5] = [
        ResizeFilter::Nearest,
        ResizeFilter::Triangle,
        ResizeFilter::CatmullRom,
        ResizeFilter::Gaussian,
        ResizeFilter::Lanczos3,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ResizeFilter::Nearest => "Nearest (pixel art)",
            ResizeFilter::Triangle => "Bilinear",
            ResizeFilter::CatmullRom => "Catmull-Rom",
            ResizeFilter::Gaussian => "Gaussian",
            ResizeFilter::Lanczos3 => "Lanczos3",
        }
    }

    fn filter_type(self) -> FilterType {
        match self {
            ResizeFilter::Nearest => FilterType::Nearest,
            ResizeFilter::Triangle => FilterType::Triangle,
            ResizeFilter::CatmullRom => FilterType::CatmullRom,
            ResizeFilter::Gaussian => FilterType::Gaussian,
            ResizeFilter::Lanczos3 => FilterType::Lanczos3,
        }
    }
}

/// How the source image is oriented and sized.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Geometry {
    /// Target width in dots (the print head is 384 dots on the MiniX).
    pub width: u32,
    /// Enlarge images narrower than `width`. If `false` they keep their size.
    pub upscale: bool,
    pub filter: ResizeFilter,
    pub rotation: Rotation,
}

impl Default for Geometry {
    fn default() -> Self {
        Self {
            width: crate::paper::WIDTH_DOTS,
            upscale: true,
            filter: ResizeFilter::default(),
            rotation: Rotation::None,
        }
    }
}

/// Tone adjustments applied after resizing, in this order:
/// auto-levels → gamma → contrast → brightness → sharpen → invert.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Adjustments {
    /// Stretch the 0.5th–99.5th percentile to the full black–white range.
    pub auto_levels: bool,
    /// Midtone gamma. `1.0` = unchanged, `> 1.0` lightens midtones, which offsets
    /// thermal dot gain (neighbouring heated dots bleeding together).
    pub gamma: f32,
    /// `-1.0..=1.0`, `0.0` = unchanged.
    pub contrast: f32,
    /// `-1.0..=1.0`, `0.0` = unchanged.
    pub brightness: f32,
    /// Unsharp-mask amount (radius ≈ 1 dot). `0.0` = off, `0.5–1.5` is typical.
    pub sharpen: f32,
    pub invert: bool,
}

impl Default for Adjustments {
    fn default() -> Self {
        Self {
            auto_levels: false,
            gamma: 1.0,
            contrast: 0.0,
            brightness: 0.0,
            sharpen: 0.0,
            invert: false,
        }
    }
}

impl Adjustments {
    /// Starting point for photos on thermal paper.
    pub fn photo() -> Self {
        Self {
            auto_levels: true,
            gamma: 1.3,
            contrast: 0.1,
            brightness: 0.0,
            sharpen: 0.8,
            invert: false,
        }
    }
}

/// Rotate, convert to luminance and resize. Cache this result: it is the slow
/// step, and [`adjust`] + dithering can be re-run cheaply on top of it.
pub fn to_gray(img: &DynamicImage, geo: &Geometry) -> GrayF32 {
    let rotated;
    let img = match geo.rotation {
        Rotation::None => img,
        Rotation::Cw90 => {
            rotated = img.rotate90();
            &rotated
        }
        Rotation::Rot180 => {
            rotated = img.rotate180();
            &rotated
        }
        Rotation::Ccw90 => {
            rotated = img.rotate270();
            &rotated
        }
    };

    let mut lin = linear_luminance(img);

    let target_w = if lin.width > geo.width || geo.upscale {
        geo.width.max(1)
    } else {
        lin.width
    };
    if target_w != lin.width {
        let target_h =
            ((lin.height as f64 * target_w as f64 / lin.width as f64).round() as u32).max(1);
        // Cheap exact box pre-shrink so the windowed filter only handles ≤ ~2× reduction.
        let k = (lin.width / (target_w * 2)).min(lin.height / (target_h * 2).max(1));
        if k >= 2 {
            lin = box_shrink(&lin, k);
        }
        lin = resize(&lin, target_w, target_h, geo.filter);
    }

    for v in &mut lin.data {
        *v = linear_to_srgb(v.clamp(0.0, 1.0));
    }
    lin
}

/// Apply tone adjustments, returning a new image.
pub fn adjust(src: &GrayF32, adj: &Adjustments) -> GrayF32 {
    let mut img = src.clone();

    if adj.auto_levels {
        let (lo, hi) = percentiles(&img, 0.005, 0.995);
        if hi - lo > 1e-3 {
            for v in &mut img.data {
                *v = (*v - lo) / (hi - lo);
            }
        }
    }

    let inv_gamma = 1.0 / adj.gamma.max(0.05);
    let contrast = (1.0 + adj.contrast).max(0.0);
    for v in &mut img.data {
        let mut x = v.clamp(0.0, 1.0);
        if inv_gamma != 1.0 {
            x = x.powf(inv_gamma);
        }
        x = (x - 0.5) * contrast + 0.5 + adj.brightness;
        *v = x.clamp(0.0, 1.0);
    }

    if adj.sharpen > 0.0 {
        let blurred = gaussian_blur(&img, 1.0);
        for (v, b) in img.data.iter_mut().zip(&blurred.data) {
            *v = (*v + adj.sharpen * (*v - b)).clamp(0.0, 1.0);
        }
    }

    if adj.invert {
        for v in &mut img.data {
            *v = 1.0 - *v;
        }
    }
    img
}

/// Otsu's method: the threshold (0–1) that best separates dark and light pixels.
/// Good for logos, scanned text and line art.
pub fn otsu_threshold(img: &GrayF32) -> f32 {
    let mut hist = [0u64; 256];
    for v in &img.data {
        hist[(v.clamp(0.0, 1.0) * 255.0).round() as usize] += 1;
    }
    let total = img.data.len() as f64;
    let sum_all: f64 = hist
        .iter()
        .enumerate()
        .map(|(i, &c)| i as f64 * c as f64)
        .sum();
    let (mut w_b, mut sum_b, mut best, mut best_t) = (0.0, 0.0, -1.0, 127usize);
    for (t, &c) in hist.iter().enumerate() {
        w_b += c as f64;
        if w_b == 0.0 {
            continue;
        }
        let w_f = total - w_b;
        if w_f == 0.0 {
            break;
        }
        sum_b += t as f64 * c as f64;
        let m_b = sum_b / w_b;
        let m_f = (sum_all - sum_b) / w_f;
        let between = w_b * w_f * (m_b - m_f).powi(2);
        if between > best {
            best = between;
            best_t = t;
        }
    }
    (best_t as f32 + 0.5) / 255.0
}

// ---------------------------------------------------------------------------

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Rec.709 luminance in linear light, with transparency composited over white paper.
fn linear_luminance(img: &DynamicImage) -> GrayF32 {
    let lut: Vec<f32> = (0..256).map(|i| srgb_to_linear(i as f32 / 255.0)).collect();
    let rgba = img.to_rgba8();
    let data = rgba
        .pixels()
        .map(|p| {
            let [r, g, b, a] = p.0;
            let y = 0.2126 * lut[r as usize] + 0.7152 * lut[g as usize] + 0.0722 * lut[b as usize];
            let a = a as f32 / 255.0;
            y * a + (1.0 - a)
        })
        .collect();
    GrayF32 {
        width: rgba.width(),
        height: rgba.height(),
        data,
    }
}

fn box_shrink(src: &GrayF32, k: u32) -> GrayF32 {
    let (w, h) = (src.width / k, src.height / k);
    let mut out = GrayF32::new(w.max(1), h.max(1), 0.0);
    let norm = 1.0 / (k * k) as f32;
    for y in 0..out.height {
        for x in 0..out.width {
            let mut s = 0.0;
            for dy in 0..k {
                let row = ((y * k + dy) * src.width) as usize;
                for dx in 0..k {
                    s += src.data[row + (x * k + dx) as usize];
                }
            }
            out.set(x, y, s * norm);
        }
    }
    out
}

fn resize(src: &GrayF32, w: u32, h: u32, filter: ResizeFilter) -> GrayF32 {
    let buf: ImageBuffer<Luma<f32>, Vec<f32>> =
        ImageBuffer::from_raw(src.width, src.height, src.data.clone())
            .expect("buffer size matches");
    let out = image::imageops::resize(&buf, w, h, filter.filter_type());
    GrayF32 {
        width: w,
        height: h,
        data: out.into_raw(),
    }
}

fn percentiles(img: &GrayF32, lo: f32, hi: f32) -> (f32, f32) {
    const BINS: usize = 1024;
    let mut hist = vec![0u32; BINS];
    for v in &img.data {
        hist[((v.clamp(0.0, 1.0)) * (BINS - 1) as f32).round() as usize] += 1;
    }
    let n = img.data.len() as f32;
    let find = |q: f32| {
        let target = q * n;
        let mut acc = 0.0;
        for (i, &c) in hist.iter().enumerate() {
            acc += c as f32;
            if acc >= target {
                return i as f32 / (BINS - 1) as f32;
            }
        }
        1.0
    };
    (find(lo), find(hi))
}

pub(crate) fn gaussian_blur(src: &GrayF32, sigma: f32) -> GrayF32 {
    let radius = (sigma * 3.0).ceil() as i32;
    let kernel: Vec<f32> = (-radius..=radius)
        .map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp())
        .collect();
    let sum: f32 = kernel.iter().sum();
    let kernel: Vec<f32> = kernel.iter().map(|k| k / sum).collect();
    let (w, h) = (src.width as i32, src.height as i32);

    let mut tmp = GrayF32::new(src.width, src.height, 0.0);
    for y in 0..h {
        for x in 0..w {
            let mut s = 0.0;
            for (i, k) in kernel.iter().enumerate() {
                let sx = (x + i as i32 - radius).clamp(0, w - 1);
                s += k * src.get(sx as u32, y as u32);
            }
            tmp.set(x as u32, y as u32, s);
        }
    }
    let mut out = GrayF32::new(src.width, src.height, 0.0);
    for y in 0..h {
        for x in 0..w {
            let mut s = 0.0;
            for (i, k) in kernel.iter().enumerate() {
                let sy = (y + i as i32 - radius).clamp(0, h - 1);
                s += k * tmp.get(x as u32, sy as u32);
            }
            out.set(x as u32, y as u32, s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    #[test]
    fn transparent_pixels_become_white_paper() {
        let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(4, 4, Rgba([0, 0, 0, 0])));
        let g = to_gray(
            &img,
            &Geometry {
                width: 4,
                ..Default::default()
            },
        );
        assert!(g.data.iter().all(|v| (*v - 1.0).abs() < 1e-4));
    }

    #[test]
    fn resizes_to_target_width_keeping_aspect() {
        let img =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(1000, 500, Rgba([128, 128, 128, 255])));
        let g = to_gray(&img, &Geometry::default());
        assert_eq!((g.width, g.height), (384, 192));
        // 50 % sRGB gray stays ≈ 50 % after a linear-light resize.
        assert!((g.data[g.data.len() / 2] - 128.0 / 255.0).abs() < 0.01);
    }

    #[test]
    fn rotation_swaps_axes() {
        let img =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(100, 400, Rgba([255, 255, 255, 255])));
        let g = to_gray(
            &img,
            &Geometry {
                rotation: Rotation::Cw90,
                ..Default::default()
            },
        );
        assert_eq!((g.width, g.height), (384, 96));
    }

    #[test]
    fn no_upscale_keeps_small_images() {
        let img =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(100, 50, Rgba([255, 255, 255, 255])));
        let g = to_gray(
            &img,
            &Geometry {
                upscale: false,
                ..Default::default()
            },
        );
        assert_eq!((g.width, g.height), (100, 50));
    }

    #[test]
    fn otsu_splits_bimodal_image() {
        let mut g = GrayF32::new(10, 1, 0.1);
        for x in 5..10 {
            g.set(x, 0, 0.9);
        }
        let t = otsu_threshold(&g);
        assert!(t > 0.1 && t < 0.9, "t = {t}");
    }
}
