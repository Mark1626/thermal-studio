use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use thermal_markdown::{FsImages, ImageResolver, NoImages, Renderer, Style};
use thermal_raster::{Algorithm, Bitmap, ImageOptions, Rotation, paper};
use thermal_templates::{Date, FieldKind, Index, Template, Value, Values};

/// Prepare images and Markdown for 384-dot thermal printers.
#[derive(Parser)]
#[command(name = "thermal", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Dither an image to a 1-bit bitmap.
    Image(ImageArgs),
    /// Render a Markdown file (or `-` for stdin) to a 1-bit bitmap.
    Markdown(MarkdownArgs),
    /// Fill in a template and render it.
    Template(TemplateArgs),
    /// List the built-in templates and their fields.
    Templates,
    /// Write `index.json` for a folder of templates (the list the apps offer to add).
    IndexTemplates {
        /// Folder holding the templates, such as `templates`.
        dir: PathBuf,
        /// Only check that `index.json` is up to date.
        #[arg(long)]
        check: bool,
    },
    /// List dithering algorithms.
    Algorithms,
    /// List nearby printers (Bluetooth LE).
    Printers {
        /// How long to search, in seconds.
        #[arg(long, default_value_t = 8)]
        seconds: u64,
    },
}

#[derive(Args)]
struct Output {
    /// Write a PNG preview here.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Pixels per printer dot in the PNG preview.
    #[arg(long, default_value_t = 1)]
    scale: u32,
    /// Write raw packed rows (48 bytes per row, MSB first, 1 = black) for `GS v 0`.
    #[arg(long)]
    raw: Option<PathBuf>,
    /// Print over Bluetooth LE.
    #[arg(long)]
    print: bool,
    /// Connect to the first printer whose BLE name contains this.
    #[arg(long, default_value = "MiniX")]
    printer: String,
    /// Print density (default: keep the printer's setting).
    #[arg(long, value_enum)]
    density: Option<DensityArg>,
    /// Paper to feed after printing, in mm.
    #[arg(long, default_value_t = 10.0)]
    feed_mm: f32,
}

#[derive(Clone, Copy, ValueEnum)]
enum DensityArg {
    Light,
    Normal,
    Dark,
}

#[derive(Clone, Copy, ValueEnum)]
enum Preset {
    /// Auto-levels, gamma 1.3, light sharpening, Atkinson.
    Photo,
    /// Hard threshold, no tone changes.
    LineArt,
    /// Pen-and-ink drawing: outlines plus hatching.
    Sketch,
    /// No adjustments, Atkinson.
    None,
}

#[derive(Clone, Copy, ValueEnum)]
enum Rotate {
    None,
    Cw90,
    #[value(name = "180")]
    R180,
    Ccw90,
}

#[derive(Args)]
struct ImageArgs {
    input: PathBuf,
    #[command(flatten)]
    out: Output,
    #[arg(long, value_enum, default_value_t = Preset::Photo)]
    preset: Preset,
    /// Dithering algorithm (see `thermal algorithms`).
    #[arg(long, value_parser = parse_algorithm)]
    algo: Option<Algorithm>,
    /// Threshold / bias, 0–1. `auto` picks one with Otsu's method.
    #[arg(long)]
    threshold: Option<String>,
    /// Fraction of error diffused (error-diffusion algorithms).
    #[arg(long)]
    diffusion: Option<f32>,
    /// Scan every row left-to-right instead of alternating.
    #[arg(long)]
    no_serpentine: bool,
    /// Halftone cell size in dots.
    #[arg(long)]
    cell: Option<u8>,
    /// Sketch: outline scale in dots (0.3–4); larger = bolder, simpler lines.
    #[arg(long)]
    sketch_line: Option<f32>,
    /// Sketch: edge sensitivity 0–1; higher = more lines and texture.
    #[arg(long)]
    sketch_edges: Option<f32>,
    /// Sketch: hatching amount 0–1; 0 = outlines only.
    #[arg(long)]
    sketch_shading: Option<f32>,
    /// Sketch: distance between hatch strokes in dots (3–16).
    #[arg(long)]
    hatch_spacing: Option<u8>,
    #[arg(long, default_value_t = paper::WIDTH_DOTS)]
    width: u32,
    /// Keep images narrower than --width at their original size.
    #[arg(long)]
    no_upscale: bool,
    #[arg(long, value_enum, default_value_t = Rotate::None)]
    rotate: Rotate,
    #[arg(long)]
    gamma: Option<f32>,
    #[arg(long, allow_hyphen_values = true)]
    contrast: Option<f32>,
    #[arg(long, allow_hyphen_values = true)]
    brightness: Option<f32>,
    #[arg(long)]
    sharpen: Option<f32>,
    #[arg(long)]
    auto_levels: Option<bool>,
    #[arg(long)]
    invert: bool,
}

#[derive(Args)]
struct MarkdownArgs {
    input: PathBuf,
    #[command(flatten)]
    out: Output,
    #[arg(long)]
    body_size: Option<f32>,
    #[arg(long)]
    margin: Option<u32>,
    /// Glyph coverage (0–255) at which a dot turns black; lower = bolder.
    #[arg(long)]
    text_threshold: Option<u8>,
    /// Dithering algorithm for embedded images.
    #[arg(long, value_parser = parse_algorithm)]
    image_algo: Option<Algorithm>,
}

#[derive(Args)]
struct TemplateArgs {
    /// A built-in template (see `thermal templates`) or a template file (`.md`).
    template: String,
    /// Fill in a field. Repeat a list field to add entries; switches take true or false.
    #[arg(short, long = "set", value_name = "KEY=VALUE")]
    set: Vec<String>,
    /// Write the filled-in Markdown to stdout instead of rendering it.
    #[arg(long)]
    markdown: bool,
    #[command(flatten)]
    out: Output,
}

fn parse_algorithm(s: &str) -> Result<Algorithm, String> {
    Algorithm::from_id(s).ok_or_else(|| {
        let ids: Vec<_> = Algorithm::ALL.iter().map(|a| a.id()).collect();
        format!(
            "unknown algorithm `{s}`; expected one of: {}",
            ids.join(", ")
        )
    })
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Algorithms => {
            for a in Algorithm::ALL {
                println!("{:<16} {}", a.id(), a.name());
            }
            Ok(())
        }
        Cmd::Printers { seconds } => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            let found = rt.block_on(thermal_ble::scan_printers(
                std::time::Duration::from_secs(seconds),
                |p| println!("{}", p.name),
            ))?;
            if found.is_empty() {
                eprintln!("no printers found (is it switched on and free?)");
            }
            Ok(())
        }
        Cmd::Image(args) => image_cmd(args),
        Cmd::Markdown(args) => markdown_cmd(args),
        Cmd::Template(args) => template_cmd(args),
        Cmd::Templates => {
            for (i, t) in thermal_templates::builtin().iter().enumerate() {
                if i > 0 {
                    println!();
                }
                describe_template(t);
            }
            println!("\nFill one in: thermal template <name> --set key=value -o note.png");
            Ok(())
        }
        Cmd::IndexTemplates { dir, check } => {
            let index = Index::from_dir(&dir).map_err(anyhow::Error::msg)?;
            let path = dir.join("index.json");
            let json = index.to_json();
            if check {
                if std::fs::read_to_string(&path).ok().as_deref() != Some(json.as_str()) {
                    bail!(
                        "{} is out of date; run `thermal index-templates {}`",
                        path.display(),
                        dir.display()
                    );
                }
                eprintln!("{} is up to date", path.display());
            } else {
                std::fs::write(&path, json)
                    .with_context(|| format!("writing {}", path.display()))?;
                eprintln!(
                    "wrote {} ({} templates)",
                    path.display(),
                    index.templates.len()
                );
            }
            Ok(())
        }
    }
}

fn image_cmd(a: ImageArgs) -> Result<()> {
    let img = thermal_raster::load_image(&a.input)
        .with_context(|| format!("reading {}", a.input.display()))?;
    let mut o = match a.preset {
        Preset::Photo => ImageOptions::photo(),
        Preset::LineArt => ImageOptions::line_art(),
        Preset::Sketch => ImageOptions::sketch(),
        Preset::None => ImageOptions::default(),
    };
    o.geometry.width = a.width;
    o.geometry.upscale = !a.no_upscale;
    o.geometry.rotation = match a.rotate {
        Rotate::None => Rotation::None,
        Rotate::Cw90 => Rotation::Cw90,
        Rotate::R180 => Rotation::Rot180,
        Rotate::Ccw90 => Rotation::Ccw90,
    };
    if let Some(v) = a.gamma {
        o.adjust.gamma = v;
    }
    if let Some(v) = a.contrast {
        o.adjust.contrast = v;
    }
    if let Some(v) = a.brightness {
        o.adjust.brightness = v;
    }
    if let Some(v) = a.sharpen {
        o.adjust.sharpen = v;
    }
    if let Some(v) = a.auto_levels {
        o.adjust.auto_levels = v;
    }
    o.adjust.invert |= a.invert;
    if let Some(alg) = a.algo {
        o.dither.algorithm = alg;
    }
    if let Some(v) = a.diffusion {
        o.dither.diffusion = v;
    }
    if let Some(v) = a.cell {
        o.dither.halftone_cell = v;
    }
    if let Some(v) = a.sketch_line {
        o.dither.sketch_line = v;
    }
    if let Some(v) = a.sketch_edges {
        o.dither.sketch_edges = v;
    }
    if let Some(v) = a.sketch_shading {
        o.dither.sketch_shading = v;
    }
    if let Some(v) = a.hatch_spacing {
        o.dither.sketch_hatch_spacing = v;
    }
    o.dither.serpentine = !a.no_serpentine;

    let gray = thermal_raster::adjust(&thermal_raster::to_gray(&img, &o.geometry), &o.adjust);
    match a.threshold.as_deref() {
        None => {}
        Some("auto") => {
            o.dither.threshold = thermal_raster::otsu_threshold(&gray);
            eprintln!("auto threshold: {:.3}", o.dither.threshold);
        }
        Some(v) => {
            o.dither.threshold = v
                .parse()
                .context("--threshold must be a number or `auto`")?
        }
    }
    let bmp = thermal_raster::dither(&gray, &o.dither);
    let bmp = if bmp.width() < paper::WIDTH_DOTS && a.width == paper::WIDTH_DOTS {
        bmp.padded_to_width(paper::WIDTH_DOTS, thermal_raster::HAlign::Center)
    } else {
        bmp
    };
    finish(&bmp, &a.out)
}

fn markdown_cmd(a: MarkdownArgs) -> Result<()> {
    let (text, base) = if a.input.as_os_str() == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        (s, PathBuf::from("."))
    } else {
        let s = std::fs::read_to_string(&a.input)
            .with_context(|| format!("reading {}", a.input.display()))?;
        (
            s,
            a.input.parent().map(Path::to_path_buf).unwrap_or_default(),
        )
    };
    let mut style = Style::default();
    if let Some(v) = a.body_size {
        let k = v / style.body_size;
        style.body_size = v;
        style.code_size = (style.code_size * k).round();
        style.heading_sizes = style.heading_sizes.map(|h| (h * k).round());
    }
    if let Some(v) = a.margin {
        style.margin = v;
    }
    if let Some(v) = a.text_threshold {
        style.text_threshold = v;
    }
    if let Some(v) = a.image_algo {
        style.image.dither.algorithm = v;
    }
    let mut r = Renderer::new();
    let bmp = r.render(&text, &style, &FsImages::new(base));
    finish(&bmp, &a.out)
}

fn template_cmd(a: TemplateArgs) -> Result<()> {
    // A path, or the id of a built-in template.
    let looks_like_path = a.template.ends_with(".md") || a.template.contains(['/', '\\']);
    let (t, images): (Template, Box<dyn ImageResolver>) = if looks_like_path {
        let path = Path::new(&a.template);
        let src =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let t = Template::parse(&src).with_context(|| format!("in {}", path.display()))?;
        let base = path.parent().map(Path::to_path_buf).unwrap_or_default();
        (t, Box::new(FsImages::new(base)))
    } else {
        let all = thermal_templates::builtin();
        let ids: Vec<_> = all.iter().map(|t| t.id.clone()).collect();
        let t = all
            .into_iter()
            .find(|t| t.id == a.template)
            .with_context(|| {
                format!(
                    "no built-in template called `{}` (there are: {}); to use a file, give its path",
                    a.template,
                    ids.join(", ")
                )
            })?;
        (t, Box::new(NoImages))
    };

    let mut values = Values::new();
    for kv in &a.set {
        let (key, v) = kv
            .split_once('=')
            .with_context(|| format!("`--set {kv}`: expected KEY=VALUE"))?;
        let Some(field) = t.field(key) else {
            let keys: Vec<_> = t.fields.iter().map(|f| f.key.as_str()).collect();
            bail!(
                "`{}` has no field called `{key}`; its fields are: {}",
                t.id,
                keys.join(", ")
            );
        };
        let value = match &field.kind {
            FieldKind::Bool { .. } => Value::Bool(match v.to_ascii_lowercase().as_str() {
                "true" | "yes" | "on" | "1" => true,
                "false" | "no" | "off" | "0" => false,
                _ => bail!("`--set {kv}`: `{key}` is a switch; use true or false"),
            }),
            // The first --set replaces the default entries; later ones add to them.
            FieldKind::List { .. } => match values.remove(key) {
                Some(Value::List(mut items)) => {
                    items.push(v.into());
                    Value::List(items)
                }
                _ => Value::List(vec![v.into()]),
            },
            FieldKind::Date { .. } if v.eq_ignore_ascii_case("today") => {
                Value::Text(today().to_string())
            }
            _ => Value::Text(v.into()),
        };
        values.insert(key.into(), value);
    }

    let markdown = t.fill(&values, today())?;
    if a.markdown {
        print!("{markdown}");
        return Ok(());
    }
    let missing: Vec<_> = t
        .missing(&values, today())
        .iter()
        .map(|f| format!("--set {}=… ({})", f.key, f.label))
        .collect();
    if !missing.is_empty() {
        bail!("fill in the required fields first: {}", missing.join(", "));
    }
    let bmp = Renderer::new().render(&markdown, &Style::default(), images.as_ref());
    finish(&bmp, &a.out)
}

/// The local date.
fn today() -> Date {
    let d = jiff::Zoned::now().date();
    Date::new(d.year().into(), d.month() as u32, d.day() as u32).unwrap_or(Date::MIN)
}

fn describe_template(t: &Template) {
    println!("{}  {}", t.id, t.name);
    if !t.description.is_empty() {
        println!("  {}", t.description);
    }
    let width = t.fields.iter().map(|f| f.key.len()).max().unwrap_or(0);
    for f in &t.fields {
        let kind = match &f.kind {
            FieldKind::Choice { options, .. } => {
                let values: Vec<_> = options.iter().map(|o| o.value.as_str()).collect();
                values.join(" | ")
            }
            FieldKind::Bool { .. } => "true | false".into(),
            FieldKind::List { .. } => "list (repeat --set)".into(),
            FieldKind::Date { .. } => "date: 2026-11-05 | today".into(),
            _ => f.type_name().into(),
        };
        let required = if f.required { ", required" } else { "" };
        println!(
            "  {:<width$}  {:<26}  {}",
            f.key,
            format!("{kind}{required}"),
            f.label
        );
    }
}

fn finish(bmp: &Bitmap, out: &Output) -> Result<()> {
    eprintln!(
        "{}x{} dots, {:.1} mm of paper, {:.1}% ink",
        bmp.width(),
        bmp.height(),
        paper::rows_to_mm(bmp.height()),
        bmp.ink_ratio() * 100.0
    );
    if out.output.is_none() && out.raw.is_none() && !out.print {
        bail!("nothing to do: pass --output <file.png>, --raw <file.bin> and/or --print");
    }
    if let Some(p) = &out.output {
        bmp.to_gray_image(out.scale)
            .save(p)
            .with_context(|| format!("writing {}", p.display()))?;
        eprintln!("wrote {}", p.display());
    }
    if let Some(p) = &out.raw {
        std::fs::write(p, bmp.as_bytes()).with_context(|| format!("writing {}", p.display()))?;
        eprintln!(
            "wrote {} ({} bytes, {} per row)",
            p.display(),
            bmp.as_bytes().len(),
            bmp.stride()
        );
    }
    if out.print {
        print(bmp, out)?;
    }
    Ok(())
}

fn print(bmp: &Bitmap, out: &Output) -> Result<()> {
    use std::io::Write;
    use thermal_proto::{Density, PrintJob};

    let job = PrintJob {
        density: out.density.map(|d| match d {
            DensityArg::Light => Density::Light,
            DensityArg::Normal => Density::Normal,
            DensityArg::Dark => Density::Dark,
        }),
        feed_dots: (out.feed_mm * paper::DOTS_PER_MM).round().clamp(0.0, 255.0) as u8,
        ..Default::default()
    };
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        eprintln!("searching for a printer named \"*{}*\"…", out.printer);
        let mut printer =
            thermal_ble::Printer::find(&out.printer, std::time::Duration::from_secs(10)).await?;
        let info = printer.info().await?;
        eprintln!(
            "connected: {} ({}, firmware {}, battery {}, {})",
            info.name,
            info.model.as_deref().unwrap_or("?"),
            info.firmware.as_deref().unwrap_or("?"),
            info.battery.map_or("?".into(), |b| format!("{b}%")),
            info.status.map_or("?".into(), |s| s.to_string()),
        );
        let report = printer
            .print(bmp, &job, &Default::default(), |p| {
                eprint!(
                    "\r{:<10} {:>3.0}%",
                    p.stage,
                    p.sent as f32 * 100.0 / p.total.max(1) as f32
                );
                let _ = std::io::stderr().flush();
            })
            .await;
        eprintln!();
        let report = report?;
        eprintln!(
            "printed {} bytes in {} writes, {:.1} s, acknowledged: {}, status after: {}",
            report.bytes,
            report.writes,
            report.elapsed.as_secs_f32(),
            report.acknowledged,
            report.status_after.map_or("?".into(), |s| s.to_string()),
        );
        printer.disconnect().await?;
        Ok::<_, anyhow::Error>(())
    })
}
