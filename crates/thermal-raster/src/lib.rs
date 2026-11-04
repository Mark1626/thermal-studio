//! Raster pipeline for 1-bit thermal printers.
//!
//! ```no_run
//! use thermal_raster::{ImageOptions, load_image, render_image};
//!
//! let img = load_image("photo.jpg")?;
//! let bitmap = render_image(&img, &ImageOptions::photo());
//! assert_eq!(bitmap.width(), 384);
//! // bitmap.as_bytes() is ready for a `GS v 0` raster command.
//! # Ok::<(), thermal_raster::Error>(())
//! ```

mod bitmap;
mod dither;
mod prepare;
mod sketch;

pub use bitmap::{Bitmap, HAlign};
pub use dither::{Algorithm, DitherOptions, Kind, dither};
pub use prepare::{
    Adjustments, Geometry, GrayF32, ResizeFilter, Rotation, adjust, otsu_threshold, to_gray,
};

pub use image::DynamicImage;

/// Paper and print-head constants for the Seznik MiniX / LuckJingle S1.
pub mod paper {
    /// Dots across the print head (48 bytes per raster row).
    pub const WIDTH_DOTS: u32 = 384;
    /// 203 dpi.
    pub const DOTS_PER_MM: f32 = 8.0;

    /// Length of paper used by `rows` raster rows, in millimetres.
    pub fn rows_to_mm(rows: u32) -> f32 {
        rows as f32 / DOTS_PER_MM
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not decode image: {0}")]
    Image(#[from] image::ImageError),
}

/// Decode an image file (PNG, JPEG, GIF, WebP or BMP).
pub fn load_image(path: impl AsRef<std::path::Path>) -> Result<DynamicImage, Error> {
    Ok(image::ImageReader::open(path)
        .map_err(image::ImageError::IoError)?
        .with_guessed_format()
        .map_err(image::ImageError::IoError)?
        .decode()?)
}

/// Decode an image from memory.
pub fn decode_image(bytes: &[u8]) -> Result<DynamicImage, Error> {
    Ok(image::load_from_memory(bytes)?)
}

/// Wrap straight-alpha RGBA pixels (4 bytes per pixel, rows top to bottom).
/// Returns `None` if `rgba.len() != width * height * 4`.
pub fn rgba_image(width: u32, height: u32, rgba: Vec<u8>) -> Option<DynamicImage> {
    image::RgbaImage::from_raw(width, height, rgba).map(DynamicImage::ImageRgba8)
}

/// Everything needed to turn a picture into a printable bitmap.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct ImageOptions {
    pub geometry: Geometry,
    pub adjust: Adjustments,
    pub dither: DitherOptions,
}

impl ImageOptions {
    /// Photo defaults: auto-levels, lightened midtones, light sharpening, Atkinson.
    pub fn photo() -> Self {
        Self {
            adjust: Adjustments::photo(),
            ..Default::default()
        }
    }

    /// Pen-and-ink drawing: outlines plus hatching, with auto-levels so the
    /// hatching spans the full tonal range.
    pub fn sketch() -> Self {
        Self {
            adjust: Adjustments {
                auto_levels: true,
                ..Default::default()
            },
            dither: DitherOptions {
                algorithm: Algorithm::Sketch,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Logos and line art: hard threshold, no tone changes.
    pub fn line_art() -> Self {
        Self {
            dither: DitherOptions {
                algorithm: Algorithm::Threshold,
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

/// One-shot pipeline: [`to_gray`] → [`adjust`] → [`dither`].
pub fn render_image(img: &DynamicImage, opts: &ImageOptions) -> Bitmap {
    let gray = to_gray(img, &opts.geometry);
    dither(&adjust(&gray, &opts.adjust), &opts.dither)
}
