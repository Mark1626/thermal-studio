//! UniFFI bindings for the thermal-* libraries, shaped for mobile apps (Kotlin first).
//!
//! Everything here is a thin conversion layer; behaviour lives in `thermal-raster`,
//! `thermal-markdown`, `thermal-templates` and `thermal-proto`. Numbers crossing the boundary use signed
//! types so Kotlin sees `Int`/`Long` rather than `UInt`/`ULong`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use thermal_markdown::{FsImages, NoImages, Renderer, Style};
use thermal_raster::{
    Adjustments, Algorithm, Bitmap, DitherOptions, DynamicImage, Geometry, GrayF32, HAlign, Kind,
    paper,
};

uniffi::setup_scaffolding!();

// --- errors -------------------------------------------------------------------

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum ThermalError {
    #[error("{0}")]
    Image(String),
    #[error("{0}")]
    InvalidArgument(String),
    #[error("{0}")]
    Job(String),
}

// --- raster -------------------------------------------------------------------

/// A 1-bit image in printer layout: rows top to bottom, `stride` bytes per row,
/// most significant bit = leftmost dot, set bit = black.
#[derive(Clone, Debug, uniffi::Record)]
pub struct Raster {
    pub width: i32,
    pub height: i32,
    pub stride: i32,
    pub data: Vec<u8>,
    /// Paper used, in millimetres (8 dots = 1 mm).
    pub length_mm: f32,
    /// Fraction of black dots (0–1).
    pub ink_ratio: f32,
}

impl From<&Bitmap> for Raster {
    fn from(b: &Bitmap) -> Self {
        Raster {
            width: b.width() as i32,
            height: b.height() as i32,
            stride: b.stride() as i32,
            data: b.as_bytes().to_vec(),
            length_mm: paper::rows_to_mm(b.height()),
            ink_ratio: b.ink_ratio() as f32,
        }
    }
}

fn to_bitmap(r: &Raster) -> Result<Bitmap, ThermalError> {
    if r.width < 0 || r.height < 0 {
        return Err(ThermalError::InvalidArgument("negative raster size".into()));
    }
    Bitmap::from_packed(r.width as u32, r.height as u32, r.data.clone()).ok_or_else(|| {
        ThermalError::InvalidArgument("raster data length does not match its size".into())
    })
}

/// Dots across the print head (384).
#[uniffi::export]
pub fn paper_width_dots() -> i32 {
    paper::WIDTH_DOTS as i32
}

// --- dithering ----------------------------------------------------------------

#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum AlgorithmKind {
    Threshold,
    Ordered,
    ErrorDiffusion,
    Stylized,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct AlgorithmInfo {
    /// Stable identifier used in [`ImageSettings::algorithm`].
    pub id: String,
    pub name: String,
    pub description: String,
    pub kind: AlgorithmKind,
}

/// All dithering algorithms, in display order.
#[uniffi::export]
pub fn algorithms() -> Vec<AlgorithmInfo> {
    Algorithm::ALL
        .iter()
        .map(|a| AlgorithmInfo {
            id: a.id().into(),
            name: a.name().into(),
            description: a.description().into(),
            kind: match a.kind() {
                Kind::Threshold => AlgorithmKind::Threshold,
                Kind::Ordered => AlgorithmKind::Ordered,
                Kind::ErrorDiffusion => AlgorithmKind::ErrorDiffusion,
                Kind::Stylized => AlgorithmKind::Stylized,
            },
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ImageRotation {
    None,
    Cw90,
    Rot180,
    Ccw90,
}

#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum ImagePreset {
    Photo,
    LineArt,
    Sketch,
    Neutral,
}

/// Every image option, flattened. See `thermal_raster` for the meaning of each field.
#[derive(Clone, Debug, uniffi::Record)]
pub struct ImageSettings {
    pub width: i32,
    pub upscale: bool,
    pub rotation: ImageRotation,
    pub auto_levels: bool,
    pub gamma: f32,
    pub contrast: f32,
    pub brightness: f32,
    pub sharpen: f32,
    pub invert: bool,
    pub algorithm: String,
    pub threshold: f32,
    pub serpentine: bool,
    pub diffusion: f32,
    pub halftone_cell: i32,
    pub sketch_line: f32,
    pub sketch_edges: f32,
    pub sketch_shading: f32,
    pub sketch_hatch_spacing: i32,
}

#[uniffi::export]
pub fn image_preset(preset: ImagePreset) -> ImageSettings {
    let o = match preset {
        ImagePreset::Photo => thermal_raster::ImageOptions::photo(),
        ImagePreset::LineArt => thermal_raster::ImageOptions::line_art(),
        ImagePreset::Sketch => thermal_raster::ImageOptions::sketch(),
        ImagePreset::Neutral => thermal_raster::ImageOptions::default(),
    };
    let (g, a, d) = (o.geometry, o.adjust, o.dither);
    ImageSettings {
        width: g.width as i32,
        upscale: g.upscale,
        rotation: match g.rotation {
            thermal_raster::Rotation::None => ImageRotation::None,
            thermal_raster::Rotation::Cw90 => ImageRotation::Cw90,
            thermal_raster::Rotation::Rot180 => ImageRotation::Rot180,
            thermal_raster::Rotation::Ccw90 => ImageRotation::Ccw90,
        },
        auto_levels: a.auto_levels,
        gamma: a.gamma,
        contrast: a.contrast,
        brightness: a.brightness,
        sharpen: a.sharpen,
        invert: a.invert,
        algorithm: d.algorithm.id().into(),
        threshold: d.threshold,
        serpentine: d.serpentine,
        diffusion: d.diffusion,
        halftone_cell: d.halftone_cell as i32,
        sketch_line: d.sketch_line,
        sketch_edges: d.sketch_edges,
        sketch_shading: d.sketch_shading,
        sketch_hatch_spacing: d.sketch_hatch_spacing as i32,
    }
}

fn parse_algorithm(id: &str) -> Result<Algorithm, ThermalError> {
    Algorithm::from_id(id)
        .ok_or_else(|| ThermalError::InvalidArgument(format!("unknown algorithm `{id}`")))
}

fn split_settings(
    s: &ImageSettings,
) -> Result<(Geometry, Adjustments, DitherOptions), ThermalError> {
    let geometry = Geometry {
        width: s.width.clamp(8, paper::WIDTH_DOTS as i32) as u32,
        upscale: s.upscale,
        filter: Default::default(),
        rotation: match s.rotation {
            ImageRotation::None => thermal_raster::Rotation::None,
            ImageRotation::Cw90 => thermal_raster::Rotation::Cw90,
            ImageRotation::Rot180 => thermal_raster::Rotation::Rot180,
            ImageRotation::Ccw90 => thermal_raster::Rotation::Ccw90,
        },
    };
    let adjust = Adjustments {
        auto_levels: s.auto_levels,
        gamma: s.gamma,
        contrast: s.contrast,
        brightness: s.brightness,
        sharpen: s.sharpen,
        invert: s.invert,
    };
    let dither = DitherOptions {
        algorithm: parse_algorithm(&s.algorithm)?,
        threshold: s.threshold,
        serpentine: s.serpentine,
        diffusion: s.diffusion,
        halftone_cell: s.halftone_cell.clamp(2, 16) as u8,
        sketch_line: s.sketch_line,
        sketch_edges: s.sketch_edges,
        sketch_shading: s.sketch_shading,
        sketch_hatch_spacing: s.sketch_hatch_spacing.clamp(3, 16) as u8,
    };
    Ok((geometry, adjust, dither))
}

/// A decoded picture plus a cache of its resized grayscale version, so changing tone
/// or dithering settings re-renders quickly.
#[derive(uniffi::Object)]
pub struct ImageSource {
    image: DynamicImage,
    gray: Mutex<Option<(Geometry, Arc<GrayF32>)>>,
}

impl ImageSource {
    fn gray(&self, geometry: Geometry) -> Arc<GrayF32> {
        let mut cache = self.gray.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((g, gray)) = cache.as_ref()
            && *g == geometry
        {
            return gray.clone();
        }
        let gray = Arc::new(thermal_raster::to_gray(&self.image, &geometry));
        *cache = Some((geometry, gray.clone()));
        gray
    }
}

#[uniffi::export]
impl ImageSource {
    /// Straight (non-premultiplied) RGBA, 4 bytes per pixel, rows top to bottom.
    #[uniffi::constructor]
    pub fn from_rgba(width: i32, height: i32, rgba: Vec<u8>) -> Result<Arc<Self>, ThermalError> {
        if width <= 0 || height <= 0 {
            return Err(ThermalError::InvalidArgument(
                "image size must be positive".into(),
            ));
        }
        let buf = thermal_raster_image_rgba(width as u32, height as u32, rgba)?;
        Ok(Arc::new(Self {
            image: buf,
            gray: Mutex::new(None),
        }))
    }

    /// Decode PNG, JPEG, GIF, WebP or BMP bytes. (On Android prefer decoding with
    /// `ImageDecoder`, which also handles HEIC and EXIF rotation, then `from_rgba`.)
    #[uniffi::constructor]
    pub fn decode(bytes: Vec<u8>) -> Result<Arc<Self>, ThermalError> {
        let image =
            thermal_raster::decode_image(&bytes).map_err(|e| ThermalError::Image(e.to_string()))?;
        Ok(Arc::new(Self {
            image,
            gray: Mutex::new(None),
        }))
    }

    pub fn width(&self) -> i32 {
        self.image.width() as i32
    }

    pub fn height(&self) -> i32 {
        self.image.height() as i32
    }

    /// Dither with `settings`. The result is always 384 dots wide (narrower images are centred).
    pub fn render(&self, settings: ImageSettings) -> Result<Raster, ThermalError> {
        let (geometry, adjust, dither) = split_settings(&settings)?;
        let gray = self.gray(geometry);
        let mut bmp = thermal_raster::dither(&thermal_raster::adjust(&gray, &adjust), &dither);
        if bmp.width() < paper::WIDTH_DOTS {
            bmp = bmp.padded_to_width(paper::WIDTH_DOTS, HAlign::Center);
        }
        Ok(Raster::from(&bmp))
    }

    /// Otsu threshold for the image after tone adjustments.
    pub fn auto_threshold(&self, settings: ImageSettings) -> Result<f32, ThermalError> {
        let (geometry, adjust, _) = split_settings(&settings)?;
        Ok(thermal_raster::otsu_threshold(&thermal_raster::adjust(
            &self.gray(geometry),
            &adjust,
        )))
    }
}

fn thermal_raster_image_rgba(
    width: u32,
    height: u32,
    rgba: Vec<u8>,
) -> Result<DynamicImage, ThermalError> {
    let expected = width as usize * height as usize * 4;
    if rgba.len() != expected {
        return Err(ThermalError::InvalidArgument(format!(
            "expected {expected} RGBA bytes, got {}",
            rgba.len()
        )));
    }
    thermal_raster::rgba_image(width, height, rgba)
        .ok_or_else(|| ThermalError::InvalidArgument("invalid RGBA buffer".into()))
}

// --- markdown -----------------------------------------------------------------

#[derive(Clone, Debug, uniffi::Record)]
pub struct MarkdownSettings {
    pub body_size: f32,
    pub line_height: f32,
    pub margin: i32,
    pub block_gap: f32,
    /// Glyph coverage (0–255) at which a dot turns black; lower = bolder.
    pub text_threshold: i32,
    /// Dithering algorithm id for embedded images.
    pub image_algorithm: String,
    pub image_upscale: bool,
}

#[uniffi::export]
pub fn default_markdown_settings() -> MarkdownSettings {
    let s = Style::default();
    MarkdownSettings {
        body_size: s.body_size,
        line_height: s.line_height,
        margin: s.margin as i32,
        block_gap: s.block_gap,
        text_threshold: s.text_threshold as i32,
        image_algorithm: s.image.dither.algorithm.id().into(),
        image_upscale: s.image_upscale,
    }
}

fn to_style(m: &MarkdownSettings) -> Result<Style, ThermalError> {
    let mut s = Style::default();
    let k = m.body_size.clamp(8.0, 96.0) / s.body_size;
    s.body_size *= k;
    s.code_size = (s.code_size * k).round();
    s.heading_sizes = s.heading_sizes.map(|h| (h * k).round());
    s.line_height = m.line_height.clamp(0.8, 3.0);
    s.margin = m.margin.clamp(0, 96) as u32;
    s.block_gap = m.block_gap.clamp(0.0, 96.0);
    s.text_threshold = m.text_threshold.clamp(1, 255) as u8;
    s.image.dither.algorithm = parse_algorithm(&m.image_algorithm)?;
    s.image_upscale = m.image_upscale;
    Ok(s)
}

/// Reusable Markdown renderer (holds fonts and caches; create once).
#[derive(uniffi::Object)]
pub struct MarkdownRenderer {
    inner: Mutex<Renderer>,
}

#[uniffi::export]
impl MarkdownRenderer {
    /// Renderer with the embedded English and Tamil fonts.
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Renderer::new()),
        })
    }

    /// Render to a 384-dot-wide raster. `image_dir` resolves relative image paths;
    /// `None` prints alt text instead of images.
    pub fn render(
        &self,
        markdown: String,
        settings: MarkdownSettings,
        image_dir: Option<String>,
    ) -> Result<Raster, ThermalError> {
        let style = to_style(&settings)?;
        let mut r = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let bmp = match image_dir {
            Some(dir) => r.render(&markdown, &style, &FsImages::new(dir)),
            None => r.render(&markdown, &style, &NoImages),
        };
        Ok(Raster::from(&bmp))
    }

    pub fn clear_image_cache(&self) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear_image_cache();
    }

    /// Render a QR code with its caption and link to a 384-dot-wide raster, using
    /// this renderer's fonts.
    pub fn render_qr(&self, settings: QrSettings) -> Result<Raster, ThermalError> {
        let label = to_qr_label(settings);
        let mut r = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let bmp = r
            .render_qr(&label)
            .map_err(|e| ThermalError::InvalidArgument(e.to_string()))?;
        Ok(Raster::from(&bmp))
    }
}

// --- QR codes -----------------------------------------------------------------

/// How much of a code can be smudged or torn and still scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum QrEcc {
    /// About 7%.
    Low,
    /// About 15%.
    Medium,
    /// About 25%.
    Quartile,
    /// About 30%.
    High,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct QrSettings {
    /// The link (or any text) as typed; the code holds [`normalize_url`] of it.
    pub url: String,
    /// Printed in bold under the code; empty for none.
    pub caption: String,
    /// Also print the link under the code.
    pub show_url: bool,
    /// Minimum error correction (raised automatically when that costs no space).
    pub ecc: QrEcc,
    /// Widest the code may be, quiet zone included, in dots (up to 384).
    pub max_width: i32,
}

#[uniffi::export]
pub fn default_qr_settings() -> QrSettings {
    let l = thermal_markdown::QrLabel::default();
    QrSettings {
        url: l.url,
        caption: l.caption,
        show_url: l.show_url,
        ecc: match l.code.ecc {
            thermal_raster::QrEcc::Low => QrEcc::Low,
            thermal_raster::QrEcc::Medium => QrEcc::Medium,
            thermal_raster::QrEcc::Quartile => QrEcc::Quartile,
            thermal_raster::QrEcc::High => QrEcc::High,
        },
        max_width: l.code.max_width as i32,
    }
}

/// What the QR code for a typed link will hold, e.g. `example.com` → `https://example.com`.
#[uniffi::export]
pub fn normalize_url(input: String) -> String {
    thermal_raster::normalize_url(&input)
}

fn to_qr_label(s: QrSettings) -> thermal_markdown::QrLabel {
    thermal_markdown::QrLabel {
        url: s.url,
        caption: s.caption,
        show_url: s.show_url,
        code: thermal_raster::QrOptions {
            ecc: match s.ecc {
                QrEcc::Low => thermal_raster::QrEcc::Low,
                QrEcc::Medium => thermal_raster::QrEcc::Medium,
                QrEcc::Quartile => thermal_raster::QrEcc::Quartile,
                QrEcc::High => thermal_raster::QrEcc::High,
            },
            max_width: s.max_width.clamp(0, paper::WIDTH_DOTS as i32) as u32,
        },
    }
}

// --- templates ----------------------------------------------------------------

#[derive(Clone, Debug, uniffi::Record)]
pub struct TemplateChoice {
    /// What the template sees.
    pub value: String,
    /// What the form shows.
    pub label: String,
}

/// The kind of input a field needs. (Names avoid Kotlin's `List`.)
#[derive(Clone, Debug, uniffi::Enum)]
pub enum TemplateFieldKind {
    /// One line of text.
    Text,
    /// Several lines of text.
    Multiline,
    /// One of `options`.
    Choice { options: Vec<TemplateChoice> },
    /// On or off.
    Switch,
    /// Any number of lines, one per entry.
    Entries,
    /// A day, written `2026-11-05`.
    Date,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct TemplateField {
    pub key: String,
    pub label: String,
    /// Longer explanation; may be empty.
    pub help: String,
    /// Example text for an empty input; may be empty.
    pub placeholder: String,
    pub required: bool,
    pub kind: TemplateFieldKind,
}

/// A field's value: `Text` for text, multiline, choice and date fields (dates as
/// `2026-11-05`, or empty), `Switch` and `Entries` for the others.
#[derive(Clone, Debug, PartialEq, uniffi::Enum)]
pub enum TemplateValue {
    Text { value: String },
    Switch { on: bool },
    Entries { items: Vec<String> },
}

impl From<thermal_templates::Value> for TemplateValue {
    fn from(v: thermal_templates::Value) -> Self {
        match v {
            thermal_templates::Value::Text(value) => TemplateValue::Text { value },
            thermal_templates::Value::Bool(on) => TemplateValue::Switch { on },
            thermal_templates::Value::List(items) => TemplateValue::Entries { items },
        }
    }
}

fn to_values(values: HashMap<String, TemplateValue>) -> thermal_templates::Values {
    values
        .into_iter()
        .map(|(k, v)| {
            let v = match v {
                TemplateValue::Text { value } => thermal_templates::Value::Text(value),
                TemplateValue::Switch { on } => thermal_templates::Value::Bool(on),
                TemplateValue::Entries { items } => thermal_templates::Value::List(items),
            };
            (k, v)
        })
        .collect()
}

fn parse_today(today: &str) -> Result<thermal_templates::Date, ThermalError> {
    thermal_templates::Date::parse(today).ok_or_else(|| {
        ThermalError::InvalidArgument(format!("`{today}` is not a date like 2026-11-05"))
    })
}

fn template_error(e: thermal_templates::Error) -> ThermalError {
    ThermalError::InvalidArgument(e.to_string())
}

/// A fill-in-the-blanks note (see `thermal-templates` for the file format).
#[derive(uniffi::Object)]
pub struct NoteTemplate {
    inner: thermal_templates::Template,
}

#[uniffi::export]
impl NoteTemplate {
    /// Read a template file. The error message explains what's wrong, for showing to
    /// the person who added it.
    #[uniffi::constructor]
    pub fn parse(source: String) -> Result<Arc<Self>, ThermalError> {
        let inner = thermal_templates::Template::parse(&source).map_err(template_error)?;
        Ok(Arc::new(Self { inner }))
    }

    pub fn id(&self) -> String {
        self.inner.id.clone()
    }

    pub fn name(&self) -> String {
        self.inner.name.clone()
    }

    pub fn description(&self) -> String {
        self.inner.description.clone()
    }

    pub fn version(&self) -> String {
        self.inner.version.clone()
    }

    pub fn author(&self) -> String {
        self.inner.author.clone()
    }

    pub fn fields(&self) -> Vec<TemplateField> {
        use thermal_templates::FieldKind as K;
        self.inner
            .fields
            .iter()
            .map(|f| TemplateField {
                key: f.key.clone(),
                label: f.label.clone(),
                help: f.help.clone(),
                placeholder: f.placeholder.clone(),
                required: f.required,
                kind: match &f.kind {
                    K::Text { .. } => TemplateFieldKind::Text,
                    K::Multiline { .. } => TemplateFieldKind::Multiline,
                    K::Choice { options, .. } => TemplateFieldKind::Choice {
                        options: options
                            .iter()
                            .map(|o| TemplateChoice {
                                value: o.value.clone(),
                                label: o.label.clone(),
                            })
                            .collect(),
                    },
                    K::Bool { .. } => TemplateFieldKind::Switch,
                    K::List { .. } => TemplateFieldKind::Entries,
                    K::Date { .. } => TemplateFieldKind::Date,
                },
            })
            .collect()
    }

    /// Every field's starting value. `today` is the local date, as `2026-11-05`.
    pub fn defaults(&self, today: String) -> Result<HashMap<String, TemplateValue>, ThermalError> {
        let today = parse_today(&today)?;
        Ok(self
            .inner
            .defaults(today)
            .into_iter()
            .map(|(k, v)| (k, v.into()))
            .collect())
    }

    /// Labels of required fields that are still empty.
    pub fn missing(
        &self,
        values: HashMap<String, TemplateValue>,
        today: String,
    ) -> Result<Vec<String>, ThermalError> {
        let today = parse_today(&today)?;
        Ok(self
            .inner
            .missing(&to_values(values), today)
            .iter()
            .map(|f| f.label.clone())
            .collect())
    }

    /// The filled-in note as Markdown, for [`MarkdownRenderer::render`].
    pub fn fill(
        &self,
        values: HashMap<String, TemplateValue>,
        today: String,
    ) -> Result<String, ThermalError> {
        let today = parse_today(&today)?;
        self.inner
            .fill(&to_values(values), today)
            .map_err(template_error)
    }
}

/// The templates that ship with the app.
#[uniffi::export]
pub fn builtin_templates() -> Vec<Arc<NoteTemplate>> {
    thermal_templates::builtin()
        .into_iter()
        .map(|inner| Arc::new(NoteTemplate { inner }))
        .collect()
}

/// Check a link typed to add a template (https only; GitHub file pages become the raw
/// file). The error message is written for the person who typed it.
#[uniffi::export]
pub fn template_link(input: String) -> Result<String, ThermalError> {
    thermal_templates::template_link(&input).map_err(ThermalError::InvalidArgument)
}

/// Largest template file accepted, in bytes.
#[uniffi::export]
pub fn max_template_bytes() -> i32 {
    thermal_templates::MAX_TEMPLATE_BYTES as i32
}

/// A template the official list offers.
#[derive(Clone, Debug, uniffi::Record)]
pub struct CatalogEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub author: String,
    /// Link to the template file.
    pub url: String,
}

/// Where the official list of templates (`index.json`) is.
#[uniffi::export]
pub fn official_index_url() -> String {
    thermal_templates::OFFICIAL_INDEX.into()
}

/// Largest list of templates accepted, in bytes.
#[uniffi::export]
pub fn max_index_bytes() -> i32 {
    thermal_templates::MAX_INDEX_BYTES as i32
}

/// Read an `index.json` downloaded from `index_url`: the templates it offers that this
/// version can use, with links to their files.
#[uniffi::export]
pub fn parse_template_index(
    json: String,
    index_url: String,
) -> Result<Vec<CatalogEntry>, ThermalError> {
    use thermal_templates::Index;
    let index = Index::parse(&json).map_err(template_error)?;
    Ok(index
        .templates
        .into_iter()
        .map(|e| CatalogEntry {
            url: Index::file_url(&index_url, &e),
            id: e.id,
            name: e.name,
            description: e.description,
            version: e.version,
            author: e.author,
        })
        .collect())
}

// --- printer protocol ---------------------------------------------------------

/// BLE layout as full 128-bit UUID strings, plus the advertised-name fragment.
#[derive(Clone, Debug, uniffi::Record)]
pub struct GattInfo {
    pub service: String,
    pub write: String,
    pub reply: String,
    pub flow: String,
    /// Client Characteristic Configuration descriptor (enable notifications).
    pub cccd: String,
    pub name_hint: String,
}

/// `true` if an advertised BLE name looks like a supported printer.
#[uniffi::export]
pub fn looks_like_printer(name: String) -> bool {
    thermal_proto::gatt::looks_like_printer(&name)
}

fn uuid16(short: u16) -> String {
    format!("0000{short:04x}-0000-1000-8000-00805f9b34fb")
}

#[uniffi::export]
pub fn gatt_info() -> GattInfo {
    use thermal_proto::gatt;
    GattInfo {
        service: uuid16(gatt::SERVICE),
        write: uuid16(gatt::WRITE),
        reply: uuid16(gatt::REPLY),
        flow: uuid16(gatt::FLOW),
        cccd: uuid16(0x2902),
        name_hint: gatt::NAME_HINT.into(),
    }
}

#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum Query {
    Status,
    Model,
    Firmware,
    Serial,
    Battery,
}

/// Bytes to write for a query; the answer arrives on the reply characteristic.
#[uniffi::export]
pub fn query_command(query: Query) -> Vec<u8> {
    use thermal_proto::cmd;
    match query {
        Query::Status => cmd::QUERY_STATUS.to_vec(),
        Query::Model => cmd::QUERY_MODEL.to_vec(),
        Query::Firmware => cmd::QUERY_FIRMWARE.to_vec(),
        Query::Serial => cmd::QUERY_SERIAL.to_vec(),
        Query::Battery => cmd::QUERY_BATTERY.to_vec(),
    }
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct PrinterStatus {
    pub raw: i32,
    pub ready: bool,
    /// Blocking problem, e.g. "out of paper".
    pub problem: Option<String>,
    pub description: String,
    pub low_battery: bool,
    pub charging: bool,
}

impl From<thermal_proto::Status> for PrinterStatus {
    fn from(s: thermal_proto::Status) -> Self {
        PrinterStatus {
            raw: s.0 as i32,
            ready: s.problem().is_none(),
            problem: s.problem().map(Into::into),
            description: s.to_string(),
            low_battery: s.low_battery(),
            charging: s.charging(),
        }
    }
}

#[uniffi::export]
pub fn parse_status(reply: Vec<u8>) -> PrinterStatus {
    thermal_proto::Status(reply.last().copied().unwrap_or(0)).into()
}

#[uniffi::export]
pub fn parse_battery(reply: Vec<u8>) -> Option<i32> {
    thermal_proto::parse_battery(&reply).map(i32::from)
}

#[uniffi::export]
pub fn parse_text(reply: Vec<u8>) -> String {
    thermal_proto::parse_text(&reply)
}

#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum Density {
    Light,
    Normal,
    Dark,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct JobSettings {
    /// `None` keeps the printer's stored density.
    pub density: Option<Density>,
    /// Paper fed after the image.
    pub feed_mm: f32,
    /// Largest value one BLE write can carry on this link (negotiated MTU − 3).
    pub max_write_len: i32,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct PrintReport {
    pub bytes: i64,
    pub writes: i64,
    pub acknowledged: bool,
    pub credits: i64,
    pub status_before: Option<PrinterStatus>,
    pub status_after: Option<PrinterStatus>,
}

/// What the transport should do next; see `thermal_proto::session`.
#[derive(Clone, Debug, uniffi::Enum)]
pub enum SessionAction {
    DrainReplies,
    Write { data: Vec<u8> },
    Sleep { until_ms: i64 },
    AwaitReply { deadline_ms: i64 },
    AwaitWrite,
    Finished { report: PrintReport },
    Failed { message: String },
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct SessionProgress {
    pub stage: String,
    pub sent: i64,
    pub total: i64,
}

/// The hardware-verified print flow, driven by the platform's BLE code.
#[derive(uniffi::Object)]
pub struct PrintSession {
    inner: Mutex<thermal_proto::PrintSession>,
}

impl PrintSession {
    fn with<T>(&self, f: impl FnOnce(&mut thermal_proto::PrintSession) -> T) -> T {
        f(&mut self.inner.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

#[uniffi::export]
impl PrintSession {
    #[uniffi::constructor]
    pub fn new(raster: Raster, job: JobSettings) -> Result<Arc<Self>, ThermalError> {
        let bitmap = to_bitmap(&raster)?;
        let print_job = thermal_proto::PrintJob {
            density: job.density.map(|d| match d {
                Density::Light => thermal_proto::Density::Light,
                Density::Normal => thermal_proto::Density::Normal,
                Density::Dark => thermal_proto::Density::Dark,
            }),
            feed_dots: (job.feed_mm * paper::DOTS_PER_MM).round().clamp(0.0, 255.0) as u8,
            ..Default::default()
        };
        let mut opts = thermal_proto::PrintOptions::default();
        if job.max_write_len > 0 {
            opts.chunk_size = opts.chunk_size.min(job.max_write_len as usize);
        }
        let session = thermal_proto::PrintSession::new(&bitmap, &print_job, &opts)
            .map_err(|e| ThermalError::Job(e.to_string()))?;
        Ok(Arc::new(Self {
            inner: Mutex::new(session),
        }))
    }

    pub fn poll(&self, now_ms: i64) -> SessionAction {
        use thermal_proto::Action;
        match self.with(|s| s.poll(now_ms.max(0) as u64)) {
            Action::DrainReplies => SessionAction::DrainReplies,
            Action::Write(data) => SessionAction::Write { data },
            Action::Sleep { until_ms } => SessionAction::Sleep {
                until_ms: until_ms as i64,
            },
            Action::AwaitReply { deadline_ms } => SessionAction::AwaitReply {
                deadline_ms: deadline_ms as i64,
            },
            Action::AwaitWrite => SessionAction::AwaitWrite,
            Action::Finished(r) => SessionAction::Finished {
                report: PrintReport {
                    bytes: r.bytes as i64,
                    writes: r.writes as i64,
                    acknowledged: r.acknowledged,
                    credits: r.credits as i64,
                    status_before: r.status_before.map(Into::into),
                    status_after: r.status_after.map(Into::into),
                },
            },
            Action::Failed(e) => SessionAction::Failed {
                message: e.to_string(),
            },
        }
    }

    pub fn on_write_done(&self) {
        self.with(|s| s.on_write_done());
    }

    pub fn on_reply(&self, reply: Vec<u8>) {
        self.with(|s| s.on_reply(&reply));
    }

    pub fn on_timeout(&self) {
        self.with(|s| s.on_timeout());
    }

    pub fn on_flow(&self, frame: Vec<u8>) {
        self.with(|s| s.on_flow(&frame));
    }

    pub fn progress(&self) -> SessionProgress {
        let p = self.with(|s| s.progress());
        SessionProgress {
            stage: p.stage.into(),
            sent: p.sent as i64,
            total: p.total as i64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_round_trip_through_settings() {
        for p in [
            ImagePreset::Photo,
            ImagePreset::LineArt,
            ImagePreset::Sketch,
            ImagePreset::Neutral,
        ] {
            assert!(split_settings(&image_preset(p)).is_ok());
        }
        assert_eq!(algorithms().len(), Algorithm::ALL.len());
    }

    #[test]
    fn image_source_renders_full_width() {
        let src = ImageSource::from_rgba(
            2,
            2,
            vec![
                0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 255,
            ],
        )
        .unwrap();
        let r = src.render(image_preset(ImagePreset::Neutral)).unwrap();
        assert_eq!((r.width, r.stride, r.height), (384, 48, 384));
        assert!(ImageSource::from_rgba(2, 2, vec![0; 3]).is_err());
    }

    #[test]
    fn markdown_and_session_work_end_to_end() {
        let r = MarkdownRenderer::new()
            .render("# Hi".into(), default_markdown_settings(), None)
            .unwrap();
        assert_eq!(r.width, 384);
        let s = PrintSession::new(
            r,
            JobSettings {
                density: None,
                feed_mm: 10.0,
                max_write_len: 20,
            },
        )
        .unwrap();
        assert!(matches!(s.poll(0), SessionAction::DrainReplies));
        assert!(matches!(s.poll(0), SessionAction::Write { .. }));
        assert_eq!(gatt_info().write, "0000ff02-0000-1000-8000-00805f9b34fb");
    }

    #[test]
    fn templates_fill_and_render() {
        let all = builtin_templates();
        let wifi = all.iter().find(|t| t.id() == "wifi-card").unwrap();
        let today = "2026-11-05".to_string();
        let mut values = wifi.defaults(today.clone()).unwrap();
        assert_eq!(
            wifi.missing(values.clone(), today.clone()).unwrap(),
            ["Network name"]
        );
        values.insert(
            "ssid".into(),
            TemplateValue::Text {
                value: "Home".into(),
            },
        );
        values.insert("hidden".into(), TemplateValue::Switch { on: true });
        assert!(
            wifi.missing(values.clone(), today.clone())
                .unwrap()
                .is_empty()
        );
        let markdown = wifi.fill(values, today.clone()).unwrap();
        assert!(markdown.contains("S:Home;"), "{markdown}");
        let raster = MarkdownRenderer::new()
            .render(markdown, default_markdown_settings(), None)
            .unwrap();
        assert_eq!(raster.width, 384);

        let jar = all.iter().find(|t| t.id() == "jar-label").unwrap();
        assert!(matches!(jar.fields()[1].kind, TemplateFieldKind::Date));
        assert_eq!(
            jar.defaults(today.clone()).unwrap()["made"],
            TemplateValue::Text {
                value: today.clone()
            }
        );
        let checklist = all.iter().find(|t| t.id() == "checklist").unwrap();
        assert!(matches!(
            checklist.defaults(today.clone()).unwrap()["items"],
            TemplateValue::Entries { .. }
        ));
        assert!(wifi.fill(HashMap::new(), "05/11/2026".into()).is_err());
    }

    #[test]
    fn template_errors_explain_themselves() {
        let e = NoteTemplate::parse("# not a template".into())
            .err()
            .unwrap();
        assert!(e.to_string().contains("`---` block"), "{e}");
        let e = template_link("http://example.com/t.md".into()).unwrap_err();
        assert!(e.to_string().contains("https://"), "{e}");
        assert_eq!(max_template_bytes(), 256 * 1024);
    }

    #[test]
    fn template_index_resolves_links() {
        let json = include_str!("../../../templates/index.json");
        let entries = parse_template_index(json.into(), official_index_url()).unwrap();
        let tracker = entries.iter().find(|e| e.id == "calendar-tracker").unwrap();
        assert_eq!(
            tracker.url,
            "https://raw.githubusercontent.com/Mark1626/thermal-studio/master/templates/calendar-tracker.md"
        );
        assert!(!tracker.version.is_empty());
        assert!(parse_template_index("nope".into(), official_index_url()).is_err());
        assert_eq!(max_index_bytes(), 1024 * 1024);
    }

    #[test]
    fn qr_settings_render_and_report_errors() {
        let r = MarkdownRenderer::new();
        let settings = QrSettings {
            url: "example.com".into(),
            caption: "Menu".into(),
            ..default_qr_settings()
        };
        let raster = r.render_qr(settings.clone()).unwrap();
        assert_eq!((raster.width, raster.stride), (384, 48));
        assert!(raster.height > 0 && raster.ink_ratio > 0.0);
        // Out-of-range sizes are clamped rather than rejected.
        for max_width in [-5, 10_000] {
            assert!(
                r.render_qr(QrSettings {
                    max_width,
                    ..settings.clone()
                })
                .is_ok()
            );
        }
        let too_long = QrSettings {
            url: "x".repeat(1500),
            ..settings
        };
        assert!(matches!(
            r.render_qr(too_long),
            Err(ThermalError::InvalidArgument(_))
        ));
        assert_eq!(normalize_url(" example.com ".into()), "https://example.com");
    }
}
