//! Thermal Studio: immediate-mode desktop front end for the thermal-* libraries.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

mod printer;

use eframe::egui::{self, Color32, ColorImage, RichText, TextureHandle, TextureOptions};
use printer::{Command, Event, PrinterWorker};
use thermal_ble::PrinterInfo;
use thermal_markdown::{FsImages, Renderer, Style};
use thermal_proto::{Density, PrintJob};
use thermal_raster::{
    Algorithm, Bitmap, DynamicImage, Geometry, GrayF32, ImageOptions, Kind, ResizeFilter, Rotation,
    paper,
};

const PAPER: Color32 = Color32::from_rgb(250, 250, 246);
const INK: Color32 = Color32::from_rgb(24, 24, 28);
const IMAGE_EXTS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];
const MARKDOWN_EXTS: [&str; 3] = ["md", "markdown", "txt"];
const SAMPLE_MD: &str = include_str!("../../../examples/sample.md");
const SAMPLE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");
/// Ask before printing anything longer than this (paper is precious).
const LONG_PRINT_MM: f32 = 200.0;

/// Usage: `thermal-studio [FILE]` opens an image or Markdown file.
/// `--screenshot OUT.png` (development aid) saves one rendered frame and quits.
fn main() -> eframe::Result {
    let mut args = std::env::args_os().skip(1);
    let (mut open, mut screenshot) = (None, None);
    while let Some(a) = args.next() {
        if a == "--screenshot" {
            screenshot = args.next().map(PathBuf::from);
        } else {
            open = Some(PathBuf::from(a));
        }
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Thermal Studio")
            .with_inner_size([1280.0, 860.0])
            .with_min_inner_size([820.0, 520.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "Thermal Studio",
        options,
        Box::new(move |cc| {
            let mut studio = Studio {
                screenshot,
                ..Default::default()
            };
            studio.printer.chosen = cc
                .storage
                .and_then(|s| s.get_string(PRINTER_KEY))
                .filter(|n| !n.is_empty());
            if let Some(path) = open {
                studio.open_any(path);
            }
            Ok(Box::new(studio))
        }),
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Image,
    Markdown,
}

/// A rendered bitmap plus its on-screen texture.
#[derive(Default)]
struct Preview {
    bitmap: Option<Bitmap>,
    texture: Option<TextureHandle>,
    millis: f32,
}

impl Preview {
    fn set_bitmap(&mut self, ctx: &egui::Context, name: &str, bmp: Bitmap, millis: f32) {
        let (w, h) = (bmp.width() as usize, bmp.height() as usize);
        let mut pixels = Vec::with_capacity(w * h);
        for y in 0..h as u32 {
            for x in 0..w as u32 {
                pixels.push(if bmp.get(x, y) { INK } else { PAPER });
            }
        }
        self.upload(ctx, name, ColorImage::new([w, h], pixels));
        self.bitmap = Some(bmp);
        self.millis = millis;
    }

    fn set_gray(&mut self, ctx: &egui::Context, name: &str, gray: &GrayF32) {
        let pixels = gray
            .data
            .iter()
            .map(|v| {
                let t = v.clamp(0.0, 1.0);
                let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
                Color32::from_rgb(
                    mix(INK.r(), PAPER.r()),
                    mix(INK.g(), PAPER.g()),
                    mix(INK.b(), PAPER.b()),
                )
            })
            .collect();
        self.upload(
            ctx,
            name,
            ColorImage::new([gray.width as usize, gray.height as usize], pixels),
        );
    }

    fn upload(&mut self, ctx: &egui::Context, name: &str, img: ColorImage) {
        match &mut self.texture {
            Some(t) => t.set(img, TextureOptions::NEAREST),
            None => self.texture = Some(ctx.load_texture(name, img, TextureOptions::NEAREST)),
        }
    }

    fn status(&self) -> String {
        match &self.bitmap {
            Some(b) => format!(
                "{} × {} dots  ·  {:.1} mm of paper  ·  {:.1}% ink  ·  {:.0} ms",
                b.width(),
                b.height(),
                paper::rows_to_mm(b.height()),
                b.ink_ratio() * 100.0,
                self.millis
            ),
            None => "Nothing rendered yet".into(),
        }
    }

    fn show(&self, ui: &mut egui::Ui, zoom: f32, empty_hint: &str) {
        egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(12.0);
                match &self.texture {
                    Some(tex) => {
                        let size = tex.size_vec2() * zoom;
                        let resp = ui.add(egui::Image::new(egui::load::SizedTexture::new(
                            tex.id(),
                            size,
                        )));
                        ui.painter().rect_stroke(
                            resp.rect,
                            0.0,
                            egui::Stroke::new(1.0, Color32::from_gray(150)),
                            egui::StrokeKind::Outside,
                        );
                    }
                    None => {
                        ui.add_space(80.0);
                        ui.label(RichText::new(empty_hint).weak().size(16.0));
                    }
                }
                ui.add_space(24.0);
            });
        });
    }
}

struct ImageTab {
    path: Option<PathBuf>,
    source: Option<DynamicImage>,
    opts: ImageOptions,
    gray: Option<(Geometry, GrayF32)>,
    show_gray: bool,
    dirty: bool,
    preview: Preview,
}

impl Default for ImageTab {
    fn default() -> Self {
        Self {
            path: None,
            source: None,
            opts: ImageOptions::photo(),
            gray: None,
            show_gray: false,
            dirty: false,
            preview: Preview::default(),
        }
    }
}

struct MarkdownTab {
    text: String,
    path: Option<PathBuf>,
    base_dir: PathBuf,
    style: Style,
    renderer: Renderer,
    changed_at: Option<f64>,
    preview: Preview,
}

impl Default for MarkdownTab {
    fn default() -> Self {
        Self {
            text: SAMPLE_MD.to_string(),
            path: None,
            base_dir: PathBuf::from(SAMPLE_DIR),
            style: Style::default(),
            renderer: Renderer::new(),
            changed_at: Some(0.0),
            preview: Preview::default(),
        }
    }
}

/// The printer is only connected while a job or status check runs.
#[derive(Default)]
enum PrinterState {
    #[default]
    Idle,
    Busy(&'static str),
    Printing {
        stage: &'static str,
        fraction: f32,
    },
}

/// The "Choose printer" window.
struct Picker {
    found: Vec<String>,
    scanning: bool,
    /// Print as soon as a printer is chosen (opened from the Print button).
    print_after: bool,
}

struct PrinterPanel {
    /// Spawned on first use, so Bluetooth isn't touched until the user asks.
    worker: Option<PrinterWorker>,
    state: PrinterState,
    /// Advertised name of the chosen printer (unique per printer); remembered across launches.
    chosen: Option<String>,
    /// Model, battery and status from the last connection.
    last_info: Option<PrinterInfo>,
    density: Option<Density>,
    feed_mm: f32,
    /// Pending confirmation for a long print (length in mm).
    confirm: Option<f32>,
    picker: Option<Picker>,
}

impl Default for PrinterPanel {
    fn default() -> Self {
        Self {
            worker: None,
            state: PrinterState::Idle,
            chosen: None,
            last_info: None,
            density: None,
            feed_mm: 10.0,
            confirm: None,
            picker: None,
        }
    }
}

impl PrinterPanel {
    fn worker(&mut self, ctx: &egui::Context) -> &PrinterWorker {
        self.worker
            .get_or_insert_with(|| PrinterWorker::spawn(ctx.clone()))
    }

    fn busy(&self) -> bool {
        !matches!(self.state, PrinterState::Idle)
    }

    fn job(&self) -> PrintJob {
        PrintJob {
            density: self.density,
            feed_dots: (self.feed_mm * thermal_raster::paper::DOTS_PER_MM)
                .round()
                .clamp(0.0, 255.0) as u8,
            ..Default::default()
        }
    }

    fn open_picker(&mut self, ctx: &egui::Context, print_after: bool) {
        self.picker = Some(Picker {
            found: Vec::new(),
            scanning: true,
            print_after,
        });
        self.worker(ctx).send(Command::Scan);
    }
}

const PRINTER_KEY: &str = "printer";

struct Studio {
    tab: Tab,
    zoom: f32,
    image: ImageTab,
    markdown: MarkdownTab,
    printer: PrinterPanel,
    message: Option<String>,
    notice: Option<String>,
    screenshot: Option<PathBuf>,
    screenshot_state: Option<Arc<AtomicBool>>,
}

impl Default for Studio {
    fn default() -> Self {
        Self {
            tab: Tab::Markdown,
            zoom: 1.5,
            image: ImageTab::default(),
            markdown: MarkdownTab::default(),
            printer: PrinterPanel::default(),
            message: None,
            notice: None,
            screenshot: None,
            screenshot_state: None,
        }
    }
}

impl eframe::App for Studio {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(PRINTER_KEY, self.printer.chosen.clone().unwrap_or_default());
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.capture_if_requested(&ctx);
        self.poll_printer();
        self.handle_dropped_files(&ctx);
        self.update_image(&ctx);
        self.update_markdown(&ctx);

        self.picker_window(&ctx);
        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                let preview = match self.tab {
                    Tab::Image => &self.image.preview,
                    Tab::Markdown => &self.markdown.preview,
                };
                ui.label(preview.status());
                if let Some(n) = &self.notice {
                    ui.separator();
                    ui.label(RichText::new(n).color(ui.visuals().hyperlink_color));
                }
                if let Some(m) = &self.message {
                    ui.separator();
                    ui.label(RichText::new(m).color(ui.visuals().warn_fg_color));
                }
            });
        });
        match self.tab {
            Tab::Image => {
                egui::Panel::left("image-controls")
                    .resizable(true)
                    .default_size(310.0)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| self.image_controls(ui));
                    });
                egui::CentralPanel::default().show(ui, |ui| {
                    self.image.preview.show(
                        ui,
                        self.zoom,
                        "Open or drop an image (PNG, JPEG, GIF, WebP, BMP)",
                    )
                });
            }
            Tab::Markdown => {
                egui::Panel::left("markdown-editor")
                    .resizable(true)
                    .default_size(460.0)
                    .show(ui, |ui| {
                        self.markdown_controls(ui);
                    });
                egui::CentralPanel::default().show(ui, |ui| {
                    self.markdown
                        .preview
                        .show(ui, self.zoom, "Type some Markdown")
                });
            }
        }
    }
}

impl Studio {
    // --- shared -------------------------------------------------------------

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(
                &mut self.tab,
                Tab::Markdown,
                RichText::new("Markdown").size(15.0),
            );
            ui.selectable_value(&mut self.tab, Tab::Image, RichText::new("Image").size(15.0));
            ui.separator();
            ui.label("Zoom");
            for z in [1.0, 1.5, 2.0, 3.0] {
                ui.selectable_value(&mut self.zoom, z, format!("{z}×"));
            }
            ui.separator();
            let current = match self.tab {
                Tab::Image => self.image.preview.bitmap.as_ref(),
                Tab::Markdown => self.markdown.preview.bitmap.as_ref(),
            };
            let enabled = current.is_some();
            if ui
                .add_enabled(enabled, egui::Button::new("Export PNG…"))
                .clicked()
                && let Some(bmp) = current
            {
                self.message = export_png(bmp).err();
            }
            if ui
                .add_enabled(enabled, egui::Button::new("Export raw…"))
                .on_hover_text("Packed rows for GS v 0: 48 bytes per row, MSB first, 1 = black")
                .clicked()
                && let Some(bmp) = current
            {
                self.message = export_raw(bmp).err();
            }
            ui.separator();
            self.print_controls(ui);
        });
    }

    fn current_bitmap(&self) -> Option<&Bitmap> {
        match self.tab {
            Tab::Image => self.image.preview.bitmap.as_ref(),
            Tab::Markdown => self.markdown.preview.bitmap.as_ref(),
        }
    }

    // --- printing -----------------------------------------------------------

    fn print_controls(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let length_mm = self
            .current_bitmap()
            .map(|b| paper::rows_to_mm(b.height()) + self.printer.feed_mm);

        if let Some(mm) = self.printer.confirm {
            ui.label(RichText::new(format!("Use {mm:.0} mm of paper?")).strong());
            if ui.button("Print").clicked() {
                self.printer.confirm = None;
                self.start_print(&ctx);
            }
            if ui.button("Cancel").clicked() {
                self.printer.confirm = None;
            }
        } else {
            let enabled = length_mm.is_some() && !self.printer.busy();
            let hover = match length_mm {
                Some(mm) => format!("Send this preview to the printer (≈{mm:.0} mm of paper)"),
                None => "Nothing to print yet".into(),
            };
            if ui
                .add_enabled(enabled, egui::Button::new(RichText::new("Print").strong()))
                .on_hover_text(hover)
                .clicked()
                && let Some(mm) = length_mm
            {
                if self.printer.chosen.is_none() {
                    self.printer.open_picker(&ctx, true);
                } else if mm > LONG_PRINT_MM {
                    self.printer.confirm = Some(mm);
                } else {
                    self.start_print(&ctx);
                }
            }
        }

        match &self.printer.state {
            PrinterState::Idle => {
                let label = match (&self.printer.chosen, &self.printer.last_info) {
                    (None, _) => RichText::new("No printer chosen").weak(),
                    (Some(_), Some(info)) => {
                        let battery = info.battery.map(|b| format!(" · {b}%")).unwrap_or_default();
                        RichText::new(format!(
                            "{}{battery}",
                            info.model.as_deref().unwrap_or(&info.name)
                        ))
                    }
                    (Some(name), None) => RichText::new(name.as_str()),
                };
                ui.label(label);
            }
            PrinterState::Busy(label) => {
                ui.spinner();
                ui.label(*label);
            }
            PrinterState::Printing { stage, fraction } => {
                ui.add(
                    egui::ProgressBar::new(*fraction)
                        .desired_width(180.0)
                        .text(format!("{stage} {:.0}%", fraction * 100.0)),
                );
            }
        }

        ui.menu_button("Printer ▾", |ui| self.printer_menu(ui));
    }

    fn printer_menu(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let p = &mut self.printer;
        let busy = p.busy();
        match p.chosen.clone() {
            None => {
                ui.label("No printer chosen yet.");
                if ui.button("Choose printer…").clicked() {
                    p.open_picker(&ctx, false);
                    ui.close();
                }
            }
            Some(name) => {
                ui.label(RichText::new(&name).strong());
                if let Some(i) = &p.last_info {
                    ui.label(format!(
                        "Model {} · firmware {}",
                        i.model.as_deref().unwrap_or("?"),
                        i.firmware.as_deref().unwrap_or("?")
                    ));
                    ui.label(format!(
                        "Battery {} · {}",
                        i.battery.map_or("?".into(), |b| format!("{b}%")),
                        i.status.map_or("status unknown".into(), |s| s.to_string())
                    ));
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!busy, egui::Button::new("Check status"))
                        .clicked()
                    {
                        p.worker(&ctx).send(Command::Check { name: name.clone() });
                    }
                    if ui
                        .add_enabled(!busy, egui::Button::new("Change…"))
                        .clicked()
                    {
                        p.open_picker(&ctx, false);
                        ui.close();
                    }
                    if ui.add_enabled(!busy, egui::Button::new("Forget")).clicked() {
                        p.chosen = None;
                        p.last_info = None;
                    }
                });
            }
        }
        ui.separator();
        egui::ComboBox::from_label("Density")
            .selected_text(p.density.map_or("Printer default", Density::name))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut p.density, None, "Printer default");
                for d in Density::ALL {
                    ui.selectable_value(&mut p.density, Some(d), d.name());
                }
            });
        ui.add(egui::Slider::new(&mut p.feed_mm, 0.0..=30.0).text("feed after print (mm)"));
    }

    /// The "Choose printer" window: printers found by a live search.
    fn picker_window(&mut self, ctx: &egui::Context) {
        let Some(picker) = &self.printer.picker else {
            return;
        };
        let (mut chosen, mut rescan, mut open) = (None, false, true);
        egui::Window::new("Choose printer")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_min_width(300.0);
                ui.horizontal(|ui| {
                    if picker.scanning {
                        ui.spinner();
                        ui.label("Searching…");
                    } else {
                        ui.label("Search finished");
                        rescan = ui.button("Search again").clicked();
                    }
                });
                ui.separator();
                if picker.found.is_empty() && !picker.scanning {
                    ui.label(
                        RichText::new("No printers found. Switch the printer on, disconnect it from other apps, and search again.")
                            .weak(),
                    );
                }
                for name in &picker.found {
                    if ui.add(egui::Button::new(name).min_size(egui::vec2(280.0, 28.0))).clicked() {
                        chosen = Some(name.clone());
                    }
                }
            });
        if rescan {
            let print_after = picker.print_after;
            self.printer.open_picker(ctx, print_after);
        } else if let Some(name) = chosen {
            let print_after = picker.print_after;
            self.printer.picker = None;
            self.printer.chosen = Some(name.clone());
            self.printer.last_info = None;
            if print_after {
                self.start_print(ctx);
            } else {
                self.printer.worker(ctx).send(Command::Check { name });
            }
        } else if !open {
            self.printer.picker = None;
        }
    }

    fn start_print(&mut self, ctx: &egui::Context) {
        let Some(bitmap) = self.current_bitmap().cloned() else {
            return;
        };
        let Some(name) = self.printer.chosen.clone() else {
            return;
        };
        let job = self.printer.job();
        self.message = None;
        self.notice = None;
        self.printer.state = PrinterState::Printing {
            stage: "connecting",
            fraction: 0.0,
        };
        self.printer
            .worker(ctx)
            .send(Command::Print { bitmap, job, name });
    }

    fn poll_printer(&mut self) {
        let Some(worker) = &self.printer.worker else {
            return;
        };
        for event in worker.poll() {
            match event {
                Event::Found(p) => {
                    if let Some(picker) = &mut self.printer.picker
                        && !picker.found.contains(&p.name)
                    {
                        picker.found.push(p.name);
                    }
                }
                Event::ScanDone => {
                    if let Some(picker) = &mut self.printer.picker {
                        picker.scanning = false;
                    }
                }
                Event::Busy(label) => {
                    if !matches!(self.printer.state, PrinterState::Printing { .. }) {
                        self.printer.state = PrinterState::Busy(label);
                    }
                }
                Event::Progress { stage, fraction } => {
                    self.printer.state = PrinterState::Printing { stage, fraction };
                }
                Event::Info(info) => self.printer.last_info = Some(info),
                Event::Printed(r) => {
                    self.notice = Some(format!(
                        "Printed {:.1} KB in {:.1} s",
                        r.bytes as f32 / 1024.0,
                        r.elapsed.as_secs_f32()
                    ));
                }
                Event::Failed(e) => self.message = Some(format!("Printer: {e}")),
                Event::Idle => self.printer.state = PrinterState::Idle,
            }
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        for f in dropped {
            self.open_any(f.path().to_path_buf());
        }
    }

    fn open_any(&mut self, path: PathBuf) {
        if has_ext(&path, &MARKDOWN_EXTS) {
            self.open_markdown(path);
            self.tab = Tab::Markdown;
        } else {
            self.open_image(path);
            self.tab = Tab::Image;
        }
    }

    /// `--screenshot`: once the UI has settled, save one frame and close the window.
    fn capture_if_requested(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot.clone() else {
            return;
        };
        ctx.request_repaint();
        match &self.screenshot_state {
            None if ctx.input(|i| i.time) > 1.5 => {
                let done = Arc::new(AtomicBool::new(false));
                self.screenshot_state = Some(done.clone());
                ctx.request_screenshot(move |img| {
                    let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                    let saved =
                        image::RgbaImage::from_raw(img.size[0] as u32, img.size[1] as u32, bytes)
                            .map(|buf| buf.save(&path));
                    eprintln!("screenshot {}: {saved:?}", path.display());
                    done.store(true, Ordering::SeqCst);
                });
            }
            Some(done) if done.load(Ordering::SeqCst) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close)
            }
            _ => {}
        }
    }

    // --- image --------------------------------------------------------------

    fn open_image(&mut self, path: PathBuf) {
        match thermal_raster::load_image(&path) {
            Ok(img) => {
                self.image.source = Some(img);
                self.image.path = Some(path);
                self.image.gray = None;
                self.image.dirty = true;
                self.message = None;
            }
            Err(e) => self.message = Some(format!("{}: {e}", path.display())),
        }
    }

    fn update_image(&mut self, ctx: &egui::Context) {
        let tab = &mut self.image;
        if !tab.dirty {
            return;
        }
        tab.dirty = false;
        let Some(src) = &tab.source else { return };
        let start = Instant::now();
        if tab
            .gray
            .as_ref()
            .is_none_or(|(geo, _)| *geo != tab.opts.geometry)
        {
            tab.gray = Some((
                tab.opts.geometry,
                thermal_raster::to_gray(src, &tab.opts.geometry),
            ));
        }
        let (_, gray) = tab.gray.as_ref().unwrap();
        let adjusted = thermal_raster::adjust(gray, &tab.opts.adjust);
        let mut bmp = thermal_raster::dither(&adjusted, &tab.opts.dither);
        if bmp.width() < paper::WIDTH_DOTS {
            bmp = bmp.padded_to_width(paper::WIDTH_DOTS, thermal_raster::HAlign::Center);
        }
        let ms = start.elapsed().as_secs_f32() * 1000.0;
        tab.preview.set_bitmap(ctx, "image-preview", bmp, ms);
        if tab.show_gray {
            tab.preview.set_gray(ctx, "image-preview", &adjusted);
        }
    }

    fn image_controls(&mut self, ui: &mut egui::Ui) {
        let before = (self.image.opts, self.image.show_gray);
        let tab = &mut self.image;

        ui.add_space(4.0);
        if ui.button("Open image…").clicked()
            && let Some(p) = rfd::FileDialog::new()
                .add_filter("Images", &IMAGE_EXTS)
                .pick_file()
        {
            self.open_image(p);
            return;
        }
        match &tab.path {
            Some(p) => {
                ui.label(RichText::new(p.file_name().unwrap_or_default().to_string_lossy()).weak())
            }
            None => ui.label(RichText::new("…or drop a file on the window").weak()),
        };

        ui.horizontal(|ui| {
            ui.label("Preset");
            if ui.button("Photo").clicked() {
                tab.opts = ImageOptions {
                    geometry: tab.opts.geometry,
                    ..ImageOptions::photo()
                };
            }
            if ui.button("Line art").clicked() {
                tab.opts = ImageOptions {
                    geometry: tab.opts.geometry,
                    ..ImageOptions::line_art()
                };
            }
            if ui.button("Sketch").clicked() {
                tab.opts = ImageOptions {
                    geometry: tab.opts.geometry,
                    ..ImageOptions::sketch()
                };
            }
            if ui.button("Neutral").clicked() {
                tab.opts = ImageOptions {
                    geometry: tab.opts.geometry,
                    ..ImageOptions::default()
                };
            }
        });
        ui.checkbox(&mut tab.show_gray, "Show grayscale (before dithering)");

        egui::CollapsingHeader::new("Size & orientation")
            .default_open(true)
            .show(ui, |ui| {
                let g = &mut tab.opts.geometry;
                ui.add(
                    egui::Slider::new(&mut g.width, 8..=paper::WIDTH_DOTS)
                        .step_by(8.0)
                        .text("width (dots)"),
                );
                ui.checkbox(&mut g.upscale, "Enlarge small images");
                egui::ComboBox::from_label("Resize filter")
                    .selected_text(g.filter.name())
                    .show_ui(ui, |ui| {
                        for f in ResizeFilter::ALL {
                            ui.selectable_value(&mut g.filter, f, f.name());
                        }
                    });
                egui::ComboBox::from_label("Rotation")
                    .selected_text(g.rotation.name())
                    .show_ui(ui, |ui| {
                        for r in Rotation::ALL {
                            ui.selectable_value(&mut g.rotation, r, r.name());
                        }
                    });
            });

        egui::CollapsingHeader::new("Tone")
            .default_open(true)
            .show(ui, |ui| {
                let a = &mut tab.opts.adjust;
                ui.checkbox(&mut a.auto_levels, "Auto levels");
                ui.add(
                    egui::Slider::new(&mut a.gamma, 0.3..=3.0)
                        .logarithmic(true)
                        .text("gamma"),
                )
                .on_hover_text("> 1 lightens midtones, offsetting thermal dot gain");
                ui.add(egui::Slider::new(&mut a.contrast, -1.0..=1.0).text("contrast"));
                ui.add(egui::Slider::new(&mut a.brightness, -1.0..=1.0).text("brightness"));
                ui.add(egui::Slider::new(&mut a.sharpen, 0.0..=3.0).text("sharpen"));
                ui.checkbox(&mut a.invert, "Invert");
            });

        egui::CollapsingHeader::new("Dithering")
            .default_open(true)
            .show(ui, |ui| {
                let d = &mut tab.opts.dither;
                egui::ComboBox::from_label("Algorithm")
                    .selected_text(d.algorithm.name())
                    .show_ui(ui, |ui| {
                        for alg in Algorithm::ALL {
                            ui.selectable_value(&mut d.algorithm, alg, alg.name())
                                .on_hover_text(alg.description());
                        }
                    });
                ui.label(RichText::new(d.algorithm.description()).small().weak());
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(&mut d.threshold, 0.0..=1.0).text("threshold"));
                    if ui.button("Auto").on_hover_text("Otsu's method").clicked()
                        && let Some((_, gray)) = &tab.gray
                    {
                        d.threshold = thermal_raster::otsu_threshold(&thermal_raster::adjust(
                            gray,
                            &tab.opts.adjust,
                        ));
                    }
                });
                match d.algorithm.kind() {
                    Kind::ErrorDiffusion => {
                        ui.add(
                            egui::Slider::new(&mut d.diffusion, 0.0..=1.0).text("error diffusion"),
                        );
                        ui.checkbox(&mut d.serpentine, "Serpentine scan");
                    }
                    Kind::Ordered if d.algorithm == Algorithm::Halftone => {
                        ui.add(egui::Slider::new(&mut d.halftone_cell, 2..=16).text("cell (dots)"));
                    }
                    Kind::Stylized => {
                        ui.add(egui::Slider::new(&mut d.sketch_line, 0.3..=4.0).text("line width"))
                            .on_hover_text(
                                "Outline scale in dots. Larger = bolder, simpler outlines.",
                            );
                        ui.add(
                            egui::Slider::new(&mut d.sketch_edges, 0.0..=1.0)
                                .text("edge sensitivity"),
                        )
                        .on_hover_text("Higher picks up fainter edges and more texture.");
                        ui.add(egui::Slider::new(&mut d.sketch_shading, 0.0..=1.0).text("shading"))
                            .on_hover_text("Amount of hatching. 0 = outlines only.");
                        ui.add(
                            egui::Slider::new(&mut d.sketch_hatch_spacing, 3..=16)
                                .text("hatch spacing"),
                        );
                    }
                    _ => {}
                }
            });

        if (tab.opts, tab.show_gray) != before {
            tab.dirty = true;
        }
    }

    // --- markdown -----------------------------------------------------------

    fn open_markdown(&mut self, path: PathBuf) {
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let tab = &mut self.markdown;
                tab.text = text;
                tab.base_dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
                tab.path = Some(path);
                tab.renderer.clear_image_cache();
                tab.changed_at = Some(0.0);
                self.message = None;
            }
            Err(e) => self.message = Some(format!("{}: {e}", path.display())),
        }
    }

    fn update_markdown(&mut self, ctx: &egui::Context) {
        let tab = &mut self.markdown;
        let Some(changed) = tab.changed_at else {
            return;
        };
        let now = ctx.input(|i| i.time);
        const DEBOUNCE: f64 = 0.15;
        if now - changed < DEBOUNCE {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                DEBOUNCE - (now - changed),
            ));
            return;
        }
        tab.changed_at = None;
        let start = Instant::now();
        let bmp = tab
            .renderer
            .render(&tab.text, &tab.style, &FsImages::new(&tab.base_dir));
        tab.preview.set_bitmap(
            ctx,
            "markdown-preview",
            bmp,
            start.elapsed().as_secs_f32() * 1000.0,
        );
    }

    fn markdown_controls(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("Open…").clicked()
                && let Some(p) = rfd::FileDialog::new()
                    .add_filter("Markdown", &MARKDOWN_EXTS)
                    .pick_file()
            {
                self.open_markdown(p);
            }
            let tab = &mut self.markdown;
            if ui.button("Save").clicked() {
                let target = tab.path.clone().or_else(|| {
                    rfd::FileDialog::new()
                        .add_filter("Markdown", &["md"])
                        .set_file_name("note.md")
                        .save_file()
                });
                if let Some(p) = target {
                    self.message = std::fs::write(&p, &tab.text)
                        .err()
                        .map(|e| format!("{}: {e}", p.display()));
                    tab.base_dir = p.parent().map(Path::to_path_buf).unwrap_or_default();
                    tab.path = Some(p);
                }
            }
            let tab = &self.markdown;
            let name = tab
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned());
            ui.label(RichText::new(name.unwrap_or_else(|| "sample.md (unsaved)".into())).weak());
        });

        let tab = &mut self.markdown;
        let before = tab.style.clone();
        egui::CollapsingHeader::new("Style")
            .default_open(false)
            .show(ui, |ui| {
                let s = &mut tab.style;
                let mut body = s.body_size;
                if ui
                    .add(
                        egui::Slider::new(&mut body, 14.0..=40.0)
                            .step_by(1.0)
                            .text("body size"),
                    )
                    .changed()
                {
                    let k = body / s.body_size;
                    s.body_size = body;
                    s.code_size = (s.code_size * k).round();
                    s.heading_sizes = s.heading_sizes.map(|h| (h * k).round());
                }
                ui.add(egui::Slider::new(&mut s.line_height, 1.0..=2.0).text("line height"));
                ui.add(egui::Slider::new(&mut s.margin, 0..=48).text("margin (dots)"));
                ui.add(egui::Slider::new(&mut s.block_gap, 0.0..=40.0).text("block gap"));
                ui.add(egui::Slider::new(&mut s.text_threshold, 30..=220).text("text threshold"))
                    .on_hover_text(
                        "Glyph coverage at which a dot turns black. Lower = bolder text.",
                    );
                egui::ComboBox::from_label("Image dithering")
                    .selected_text(s.image.dither.algorithm.name())
                    .show_ui(ui, |ui| {
                        for alg in Algorithm::ALL {
                            ui.selectable_value(&mut s.image.dither.algorithm, alg, alg.name());
                        }
                    });
                ui.checkbox(&mut s.image_upscale, "Enlarge small images");
                if ui.button("Reset style").clicked() {
                    *s = Style::default();
                }
            });
        if tab.style != before {
            tab.changed_at = Some(0.0);
        }

        ui.separator();
        egui::ScrollArea::vertical().show(ui, |ui| {
            let edit = egui::TextEdit::multiline(&mut tab.text)
                .code_editor()
                .desired_width(f32::INFINITY)
                .desired_rows(40);
            if ui.add(edit).changed() {
                tab.changed_at = Some(ui.ctx().input(|i| i.time));
            }
        });
    }
}

fn has_ext(path: &Path, exts: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

fn export_png(bmp: &Bitmap) -> Result<(), String> {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("PNG", &["png"])
        .set_file_name("print.png")
        .save_file()
    else {
        return Ok(());
    };
    bmp.to_gray_image(1)
        .save(&path)
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn export_raw(bmp: &Bitmap) -> Result<(), String> {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("Raw raster", &["bin"])
        .set_file_name("print.bin")
        .save_file()
    else {
        return Ok(());
    };
    std::fs::write(&path, bmp.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))
}
