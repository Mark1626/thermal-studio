//! QR-code labels: a code across the print head with optional text under it.

use thermal_raster::{Bitmap, HAlign, QrError, QrOptions, normalize_url, qr_code};

use crate::layout::Layout;
use crate::{NoImages, Renderer, Style};

/// Caption size in dots: a little larger than body text.
const CAPTION_SIZE: f32 = 26.0;
/// The printed link: small, but still easy to read and type in.
const URL_SIZE: f32 = 19.0;
/// Space between the caption and the link.
const LINE_GAP: i32 = 4;

/// A QR code with an optional caption and the link printed under it.
#[derive(Clone, Debug, PartialEq)]
pub struct QrLabel {
    /// The link (or any text) as typed. The code holds [`normalize_url`] of it, so
    /// `example.com` opens `https://example.com`.
    pub url: String,
    /// Printed in bold under the code; empty for none.
    pub caption: String,
    /// Also print the link, as typed, under the caption.
    pub show_url: bool,
    pub code: QrOptions,
}

impl Default for QrLabel {
    fn default() -> Self {
        Self {
            url: String::new(),
            caption: String::new(),
            show_url: true,
            code: QrOptions::default(),
        }
    }
}

impl Renderer {
    /// Render `label` [`Style::default`]-width: the code centred with its quiet zone,
    /// then the caption and the link, each centred and wrapped.
    pub fn render_qr(&mut self, label: &QrLabel) -> Result<Bitmap, QrError> {
        let code = qr_code(&normalize_url(&label.url), &label.code)?;
        let style = Style::default();
        let mut layout = Layout {
            fs: &mut self.fonts,
            swash: &mut self.swash,
            gray_cache: &mut self.gray_cache,
            style: &style,
            images: &NoImages,
            canvas: code.padded_to_width(style.width, HAlign::Center),
        };
        let url = if label.show_url { label.url.trim() } else { "" };
        let lines = [
            (label.caption.trim(), CAPTION_SIZE, true),
            (url, URL_SIZE, false),
        ];

        // The code's quiet zone already separates it from the text.
        let mut y = layout.canvas.height() as i32;
        let mut any = false;
        for (text, size, bold) in lines.into_iter().filter(|(t, ..)| !t.is_empty()) {
            if any {
                y += LINE_GAP;
            }
            y += layout.centered_text(text, y, size, bold);
            any = true;
        }
        if any {
            layout
                .canvas
                .set_height((y + style.padding_bottom as i32) as u32);
        }
        Ok(layout.canvas)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thermal_raster::paper;

    fn decode(bmp: &Bitmap) -> String {
        let mut img = rqrr::PreparedImage::prepare_from_greyscale(
            bmp.width() as usize,
            bmp.height() as usize,
            |x, y| if bmp.get(x as u32, y as u32) { 0 } else { 255 },
        );
        let grids = img.detect_grids();
        assert_eq!(grids.len(), 1, "expected exactly one QR code");
        grids[0].decode().expect("QR code should decode").1
    }

    fn label(url: &str, caption: &str, show_url: bool) -> QrLabel {
        QrLabel {
            url: url.into(),
            caption: caption.into(),
            show_url,
            ..Default::default()
        }
    }

    #[test]
    fn code_alone_is_a_centred_square() {
        let mut r = Renderer::new();
        let url = "https://example.com/menu";
        let bmp = r.render_qr(&label(url, "", false)).unwrap();
        let code = qr_code(url, &QrOptions::default()).unwrap();
        assert_eq!(bmp.width(), paper::WIDTH_DOTS);
        assert_eq!(bmp.height(), code.height());
        assert_eq!(decode(&bmp), url);
    }

    #[test]
    fn caption_and_link_print_under_the_code_and_it_still_scans() {
        let mut r = Renderer::new();
        let url = "https://example.com/a/rather/long/path/that/has/to/wrap?with=query&and=more";
        let plain = r.render_qr(&label(url, "", false)).unwrap();
        let captioned = r
            .render_qr(&label(url, "Scan for the menu", false))
            .unwrap();
        let both = r.render_qr(&label(url, "Scan for the menu", true)).unwrap();
        assert!(captioned.height() > plain.height());
        assert!(both.height() > captioned.height() + URL_SIZE as u32);
        for bmp in [&captioned, &both] {
            assert_eq!(decode(bmp), url);
            // Text never intrudes on the code or its quiet zone.
            for y in 0..plain.height() {
                assert_eq!(bmp.row(y), plain.row(y), "row {y}");
            }
        }
    }

    #[test]
    fn bare_domains_open_over_https_but_print_as_typed() {
        let mut r = Renderer::new();
        let typed = r
            .render_qr(&label("  example.com/menu ", "", true))
            .unwrap();
        let full = r
            .render_qr(&label("https://example.com/menu", "", true))
            .unwrap();
        assert_eq!(decode(&typed), "https://example.com/menu");
        // The printed text is shorter than the full URL would be.
        assert!(typed.ink_dots() < full.ink_dots());
    }

    #[test]
    fn errors_pass_through() {
        let mut r = Renderer::new();
        assert_eq!(r.render_qr(&label("   ", "x", true)), Err(QrError::Empty));
        assert_eq!(
            r.render_qr(&label(&"x".repeat(1500), "", false)),
            Err(QrError::TooLong)
        );
    }
}
