//! Render Markdown to 1-bit bitmaps sized for a thermal print head.
//!
//! ```no_run
//! use thermal_markdown::{FsImages, Renderer, Style};
//!
//! let mut renderer = Renderer::new(); // embedded fonts only: identical output everywhere
//! let bitmap = renderer.render("# Hello\n\nPrinted **bold**.", &Style::default(), &FsImages::new("."));
//! assert_eq!(bitmap.width(), 384);
//! ```
//!
//! Supported: headings, paragraphs, **bold**, *italic*, `code`, ~~strike~~, links
//! (underlined), block quotes, fenced code, ordered/unordered/task lists (nested),
//! GFM tables, horizontal rules, images (dithered with [`Style::image`]) and QR codes:
//!
//! ````markdown
//! ```qr
//! https://example.com/menu
//! ```
//! ````
//!
//! Text can be English and Tamil, freely mixed. All fonts are embedded, so output is
//! identical on every platform.
//!
//! [`Renderer::render_qr`] uses the same fonts to print a QR code with a caption and
//! its link underneath (see [`QrLabel`]).

mod layout;
mod parse;
mod qr;

use std::path::PathBuf;

use cosmic_text::{Fallback, FontSystem, SwashCache};
use thermal_raster::{Bitmap, DynamicImage, HAlign, ImageOptions};
use unicode_script::Script;

pub use parse::{Block, Item, Span, parse};
pub use qr::QrLabel;

/// Family name of the embedded body font.
pub const SANS_FAMILY: &str = "Atkinson Hyperlegible";
/// Family name of the embedded monospace font.
pub const MONO_FAMILY: &str = "Fira Mono";
/// Family name of the embedded Tamil font.
pub const TAMIL_FAMILY: &str = "Noto Sans Tamil";

const EMBEDDED_FONTS: [&[u8]; 8] = [
    include_bytes!("../fonts/AtkinsonHyperlegible-Regular.ttf"),
    include_bytes!("../fonts/AtkinsonHyperlegible-Bold.ttf"),
    include_bytes!("../fonts/AtkinsonHyperlegible-Italic.ttf"),
    include_bytes!("../fonts/AtkinsonHyperlegible-BoldItalic.ttf"),
    include_bytes!("../fonts/FiraMono-Regular.ttf"),
    include_bytes!("../fonts/FiraMono-Bold.ttf"),
    include_bytes!("../fonts/NotoSansTamil-Regular.ttf"),
    include_bytes!("../fonts/NotoSansTamil-Bold.ttf"),
];

/// Font fallback limited to the embedded fonts, so Tamil always uses Noto Sans Tamil
/// whatever the platform (instead of each OS's own fallback list).
struct EmbeddedFallback;

impl Fallback for EmbeddedFallback {
    fn common_fallback(&self) -> &[&'static str] {
        &[SANS_FAMILY, TAMIL_FAMILY, MONO_FAMILY]
    }

    fn forbidden_fallback(&self) -> &[&'static str] {
        &[]
    }

    fn script_fallback(&self, script: Script, _locale: &str) -> &[&'static str] {
        match script {
            Script::Tamil => &[TAMIL_FAMILY],
            _ => &[],
        }
    }
}

/// Layout and typography settings. All sizes are in printer dots (8 dots = 1 mm).
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    /// Canvas width; 384 for the MiniX print head.
    pub width: u32,
    /// Left and right margin.
    pub margin: u32,
    pub padding_top: u32,
    pub padding_bottom: u32,
    pub body_size: f32,
    /// Line height as a multiple of the font size.
    pub line_height: f32,
    /// Font sizes for `#` … `######`.
    pub heading_sizes: [f32; 6],
    pub code_size: f32,
    /// Vertical space between blocks.
    pub block_gap: f32,
    pub list_indent: u32,
    pub quote_indent: u32,
    /// Glyph coverage (0–255) at which a dot turns black. Lower = bolder text.
    pub text_threshold: u8,
    pub sans_family: String,
    pub mono_family: String,
    /// How embedded images are toned and dithered. `geometry.width` is replaced by
    /// the available content width.
    pub image: ImageOptions,
    /// Enlarge images narrower than the content width.
    pub image_upscale: bool,
    pub image_align: HAlign,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            width: thermal_raster::paper::WIDTH_DOTS,
            margin: 6,
            padding_top: 4,
            padding_bottom: 8,
            body_size: 22.0,
            line_height: 1.25,
            heading_sizes: [38.0, 31.0, 26.0, 23.0, 22.0, 20.0],
            code_size: 18.0,
            block_gap: 12.0,
            list_indent: 28,
            quote_indent: 16,
            text_threshold: 100,
            sans_family: SANS_FAMILY.into(),
            mono_family: MONO_FAMILY.into(),
            image: ImageOptions::photo(),
            image_upscale: false,
            image_align: HAlign::Center,
        }
    }
}

/// Extra fonts on top of the embedded ones.
#[derive(Clone, Debug, Default)]
pub struct FontOptions {
    /// Additional font files (TTF/OTF bytes), selectable via [`Style::sans_family`] /
    /// [`Style::mono_family`].
    pub extra_fonts: Vec<Vec<u8>>,
}

/// Supplies the pixels for `![alt](url)` images.
pub trait ImageResolver {
    fn resolve(&self, url: &str) -> Option<DynamicImage>;
}

/// Ignores all images (alt text is printed instead).
pub struct NoImages;

impl ImageResolver for NoImages {
    fn resolve(&self, _url: &str) -> Option<DynamicImage> {
        None
    }
}

/// Loads images from the local filesystem, relative to a base directory.
/// Remote (`http://`, `https://`, …) URLs are never fetched.
pub struct FsImages {
    pub base_dir: PathBuf,
}

impl FsImages {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }
}

impl ImageResolver for FsImages {
    fn resolve(&self, url: &str) -> Option<DynamicImage> {
        let path = match url.strip_prefix("file://") {
            Some(p) => PathBuf::from(p),
            None if url.contains("://") => return None,
            None => self.base_dir.join(url),
        };
        thermal_raster::load_image(path).ok()
    }
}

/// Reusable Markdown renderer. Holds the font database, glyph cache and a cache of
/// resized images, so keep one around for interactive use.
pub struct Renderer {
    fonts: FontSystem,
    swash: SwashCache,
    gray_cache: layout::GrayCache,
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer {
    /// Embedded fonts (English and Tamil); output is identical on every platform.
    pub fn new() -> Self {
        Self::with_fonts(FontOptions::default())
    }

    pub fn with_fonts(opts: FontOptions) -> Self {
        let mut db = fontdb::Database::new();
        for font in EMBEDDED_FONTS {
            db.load_font_data(font.to_vec());
        }
        for font in opts.extra_fonts {
            db.load_font_data(font);
        }
        db.set_sans_serif_family(SANS_FAMILY);
        db.set_serif_family(SANS_FAMILY);
        db.set_monospace_family(MONO_FAMILY);
        Self {
            fonts: FontSystem::new_with_locale_and_db_and_fallback(
                "en-US".into(),
                db,
                EmbeddedFallback,
            ),
            swash: SwashCache::new(),
            gray_cache: Default::default(),
        }
    }

    /// Forget cached images (call after an image file changes on disk).
    pub fn clear_image_cache(&mut self) {
        self.gray_cache.clear();
    }

    /// Render a Markdown document to a bitmap `style.width` dots wide.
    pub fn render(&mut self, markdown: &str, style: &Style, images: &dyn ImageResolver) -> Bitmap {
        self.render_blocks(&parse(markdown), style, images)
    }

    /// Render an already-parsed document.
    pub fn render_blocks(
        &mut self,
        blocks: &[Block],
        style: &Style,
        images: &dyn ImageResolver,
    ) -> Bitmap {
        let mut layout = layout::Layout {
            fs: &mut self.fonts,
            swash: &mut self.swash,
            gray_cache: &mut self.gray_cache,
            style,
            images,
            canvas: Bitmap::new(style.width, 0),
        };
        layout.document(blocks);
        layout.canvas
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_fonts_resolve_by_family_name() {
        let r = Renderer::new();
        let families: Vec<String> = r
            .fonts
            .db()
            .faces()
            .flat_map(|f| f.families.iter().map(|(n, _)| n.clone()))
            .collect();
        assert!(families.iter().any(|f| f == SANS_FAMILY), "{families:?}");
        assert!(families.iter().any(|f| f == MONO_FAMILY), "{families:?}");
        assert!(families.iter().any(|f| f == TAMIL_FAMILY), "{families:?}");
    }

    /// Every glyph of mixed English/Tamil text (regular and bold) must come from a real
    /// font glyph, never the "missing glyph" box (glyph id 0).
    #[test]
    fn tamil_and_english_have_no_missing_glyphs() {
        use cosmic_text::{Attrs, Buffer, Family, Metrics, Shaping, Weight};
        let mut r = Renderer::new();
        for weight in [Weight::NORMAL, Weight::BOLD] {
            let mut b = Buffer::new(&mut r.fonts, Metrics::new(22.0, 28.0));
            b.set_size(Some(384.0), None);
            let attrs = Attrs::new()
                .family(Family::Name(SANS_FAMILY))
                .weight(weight);
            b.set_text(
                "Hello வணக்கம்! தமிழிலும் அச்சிடலாம் 123",
                &attrs,
                Shaping::Advanced,
                None,
            );
            b.shape_until_scroll(&mut r.fonts, false);
            let glyphs: Vec<_> = b
                .layout_runs()
                .flat_map(|run| run.glyphs.to_vec())
                .collect();
            assert!(!glyphs.is_empty());
            let missing: Vec<_> = glyphs
                .iter()
                .filter(|g| g.glyph_id == 0)
                .map(|g| (g.start, g.end))
                .collect();
            assert!(
                missing.is_empty(),
                "missing glyphs at {missing:?} (weight {weight:?})"
            );
        }
    }

    #[test]
    fn renders_full_width_with_ink() {
        let mut r = Renderer::new();
        let md = "# Title\n\nHello **world**.\n\n- a\n- [x] b\n\n| x | y |\n|---|---|\n| 1 | 2 |\n\n---\n\n```\ncode\n```";
        let bmp = r.render(md, &Style::default(), &NoImages);
        assert_eq!(bmp.width(), 384);
        assert!(bmp.height() > 200, "height {}", bmp.height());
        let ink = bmp.ink_ratio();
        assert!(ink > 0.02 && ink < 0.5, "ink {ink}");
    }

    #[test]
    fn longer_text_wraps_to_more_rows() {
        let mut r = Renderer::new();
        let short = r.render("word", &Style::default(), &NoImages).height();
        let long = r
            .render(&"word ".repeat(80), &Style::default(), &NoImages)
            .height();
        assert!(long > short * 4, "short {short}, long {long}");
    }

    #[test]
    fn missing_image_prints_alt_text() {
        let mut r = Renderer::new();
        let bmp = r.render(
            "![a cat](nope.png)",
            &Style::default(),
            &FsImages::new("/nonexistent"),
        );
        assert!(bmp.ink_dots() > 0);
    }

    #[test]
    fn rendering_is_deterministic() {
        let mut r = Renderer::new();
        let a = r.render("Same *input*.", &Style::default(), &NoImages);
        let b = Renderer::new().render("Same *input*.", &Style::default(), &NoImages);
        assert_eq!(a, b);
    }
}
