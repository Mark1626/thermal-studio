//! Packed 1-bit bitmaps in the layout thermal printers consume.

use image::{GrayImage, Luma};

/// Horizontal placement of narrower content on a wider canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum HAlign {
    Left,
    #[default]
    Center,
    Right,
}

/// A 1-bit image packed exactly as ESC/POS `GS v 0` expects it:
/// rows top to bottom, `ceil(width / 8)` bytes per row, most significant bit is the
/// leftmost pixel, and a **set bit is a black (heated) dot**.
///
/// Padding bits past `width` in the last byte of each row are always zero.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Bitmap {
    width: u32,
    height: u32,
    stride: usize,
    data: Vec<u8>,
}

impl std::fmt::Debug for Bitmap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bitmap")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("ink_dots", &self.ink_dots())
            .finish()
    }
}

impl Bitmap {
    /// A white (all-zero) bitmap.
    pub fn new(width: u32, height: u32) -> Self {
        let stride = width.div_ceil(8) as usize;
        Self {
            width,
            height,
            stride,
            data: vec![0; stride * height as usize],
        }
    }

    /// Wrap already-packed rows. Returns `None` if `data` has the wrong length.
    pub fn from_packed(width: u32, height: u32, data: Vec<u8>) -> Option<Self> {
        let stride = width.div_ceil(8) as usize;
        if data.len() != stride * height as usize {
            return None;
        }
        let mut bmp = Self {
            width,
            height,
            stride,
            data,
        };
        bmp.clear_padding();
        Some(bmp)
    }

    /// Build from a predicate that returns `true` for black dots.
    pub fn from_fn(width: u32, height: u32, mut black: impl FnMut(u32, u32) -> bool) -> Self {
        let mut bmp = Self::new(width, height);
        for y in 0..height {
            for x in 0..width {
                if black(x, y) {
                    bmp.set(x, y, true);
                }
            }
        }
        bmp
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Bytes per row (`ceil(width / 8)`).
    pub fn stride(&self) -> usize {
        self.stride
    }

    /// All rows, packed, top to bottom.
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.data
    }

    /// One packed row.
    pub fn row(&self, y: u32) -> &[u8] {
        let start = y as usize * self.stride;
        &self.data[start..start + self.stride]
    }

    /// Iterator over packed rows.
    pub fn rows(&self) -> impl ExactSizeIterator<Item = &[u8]> {
        self.data
            .chunks_exact(self.stride.max(1))
            .take(self.height as usize)
    }

    /// `true` if the dot at (x, y) is black. Out-of-range coordinates read as white.
    #[inline]
    pub fn get(&self, x: u32, y: u32) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        self.data[y as usize * self.stride + (x / 8) as usize] & (0x80 >> (x % 8)) != 0
    }

    /// Set the dot at (x, y). Out-of-range coordinates are ignored.
    #[inline]
    pub fn set(&mut self, x: u32, y: u32, black: bool) {
        if x >= self.width || y >= self.height {
            return;
        }
        let byte = &mut self.data[y as usize * self.stride + (x / 8) as usize];
        let mask = 0x80u8 >> (x % 8);
        if black {
            *byte |= mask;
        } else {
            *byte &= !mask;
        }
    }

    /// Fill a rectangle, clipped to the bitmap. Coordinates may be negative.
    pub fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, black: bool) {
        let x0 = x.max(0) as u32;
        let y0 = y.max(0) as u32;
        let x1 = (x as i64 + w as i64).clamp(0, self.width as i64) as u32;
        let y1 = (y as i64 + h as i64).clamp(0, self.height as i64) as u32;
        for yy in y0..y1 {
            for xx in x0..x1 {
                self.set(xx, yy, black);
            }
        }
    }

    /// Outline of a rectangle with the given stroke thickness (drawn inside the rectangle).
    pub fn stroke_rect(&mut self, x: i32, y: i32, w: u32, h: u32, thickness: u32) {
        let t = thickness.min(w).min(h);
        self.fill_rect(x, y, w, t, true);
        self.fill_rect(x, y + h as i32 - t as i32, w, t, true);
        self.fill_rect(x, y, t, h, true);
        self.fill_rect(x + w as i32 - t as i32, y, t, h, true);
    }

    /// OR the black dots of `src` onto `self` with `src`'s top-left at (x, y).
    pub fn blit(&mut self, src: &Bitmap, x: i32, y: i32) {
        for sy in 0..src.height {
            let ty = y + sy as i32;
            if ty < 0 || ty >= self.height as i32 {
                continue;
            }
            for sx in 0..src.width {
                let tx = x + sx as i32;
                if tx >= 0 && tx < self.width as i32 && src.get(sx, sy) {
                    self.set(tx as u32, ty as u32, true);
                }
            }
        }
    }

    /// Grow to at least `height` rows, adding white rows at the bottom.
    pub fn ensure_height(&mut self, height: u32) {
        if height > self.height {
            self.data.resize(self.stride * height as usize, 0);
            self.height = height;
        }
    }

    /// Shrink or grow to exactly `height` rows.
    pub fn set_height(&mut self, height: u32) {
        self.data.resize(self.stride * height as usize, 0);
        self.height = height;
    }

    /// Swap black and white.
    pub fn invert(&mut self) {
        for b in &mut self.data {
            *b = !*b;
        }
        self.clear_padding();
    }

    /// Number of black dots.
    pub fn ink_dots(&self) -> u64 {
        self.data.iter().map(|b| b.count_ones() as u64).sum()
    }

    /// Fraction of dots that are black (0.0–1.0).
    pub fn ink_ratio(&self) -> f64 {
        let total = self.width as u64 * self.height as u64;
        if total == 0 {
            0.0
        } else {
            self.ink_dots() as f64 / total as f64
        }
    }

    /// Place this bitmap on a canvas of `width` dots, aligned horizontally.
    /// If the bitmap is already wider it is cropped on the same alignment.
    pub fn padded_to_width(&self, width: u32, align: HAlign) -> Bitmap {
        let mut out = Bitmap::new(width, self.height);
        let free = width as i64 - self.width as i64;
        let x = match align {
            HAlign::Left => 0,
            HAlign::Center => free / 2,
            HAlign::Right => free,
        };
        out.blit(self, x as i32, 0);
        out
    }

    /// Stack bitmaps vertically (left-aligned) with `gap` white rows between them.
    pub fn vstack(parts: &[Bitmap], gap: u32) -> Bitmap {
        let width = parts.iter().map(|b| b.width).max().unwrap_or(0);
        let height = parts.iter().map(|b| b.height).sum::<u32>()
            + gap * parts.len().saturating_sub(1) as u32;
        let mut out = Bitmap::new(width, height);
        let mut y = 0i32;
        for p in parts {
            out.blit(p, 0, y);
            y += (p.height + gap) as i32;
        }
        out
    }

    /// Render as an 8-bit grayscale image (black = 0, white = 255), each dot
    /// becoming a `scale` × `scale` block. Useful for previews and PNG export.
    pub fn to_gray_image(&self, scale: u32) -> GrayImage {
        let s = scale.max(1);
        GrayImage::from_fn(self.width * s, self.height * s, |x, y| {
            Luma([if self.get(x / s, y / s) { 0 } else { 255 }])
        })
    }

    fn clear_padding(&mut self) {
        let rem = self.width % 8;
        if rem == 0 || self.stride == 0 {
            return;
        }
        let mask = 0xFFu8 << (8 - rem);
        for row in self.data.chunks_exact_mut(self.stride) {
            if let Some(last) = row.last_mut() {
                *last &= mask;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_msb_first_with_black_as_one() {
        let mut b = Bitmap::new(16, 1);
        b.set(0, 0, true);
        b.set(9, 0, true);
        assert_eq!(b.as_bytes(), &[0b1000_0000, 0b0100_0000]);
        assert!(b.get(0, 0) && b.get(9, 0) && !b.get(1, 0));
    }

    #[test]
    fn printer_width_is_48_bytes_per_row() {
        assert_eq!(Bitmap::new(384, 2).stride(), 48);
        assert_eq!(Bitmap::new(384, 2).as_bytes().len(), 96);
    }

    #[test]
    fn invert_keeps_padding_bits_clear() {
        let mut b = Bitmap::new(10, 1);
        b.invert();
        assert_eq!(b.as_bytes(), &[0xFF, 0b1100_0000]);
        assert_eq!(b.ink_dots(), 10);
    }

    #[test]
    fn padded_to_width_centres() {
        let b = Bitmap::from_fn(4, 1, |_, _| true);
        let p = b.padded_to_width(8, HAlign::Center);
        assert_eq!(p.as_bytes(), &[0b0011_1100]);
    }

    #[test]
    fn fill_rect_clips_negative_and_oversized() {
        let mut b = Bitmap::new(8, 8);
        b.fill_rect(-4, -4, 100, 6, true);
        assert_eq!(b.ink_dots(), 8 * 2);
    }
}
