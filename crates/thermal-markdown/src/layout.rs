//! Block layout and rasterisation onto a growing 1-bit canvas.

use std::collections::HashMap;
use std::sync::Arc;

use cosmic_text::{
    Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, Style as FontStyle,
    SwashCache, UnderlineStyle, Weight, Wrap,
};
use pulldown_cmark::Alignment;
use thermal_raster::{Bitmap, Geometry, GrayF32, HAlign};

use crate::parse::{Block, Item, Span};
use crate::{ImageResolver, Style};

pub(crate) type GrayCache = HashMap<(String, Geometry), Option<Arc<GrayF32>>>;

pub(crate) struct Layout<'a> {
    pub fs: &'a mut FontSystem,
    pub swash: &'a mut SwashCache,
    pub gray_cache: &'a mut GrayCache,
    pub style: &'a Style,
    pub images: &'a dyn ImageResolver,
    pub canvas: Bitmap,
}

#[derive(Clone, Copy)]
struct TextStyle {
    size: f32,
    bold: bool,
    mono: bool,
    align: Align,
}

fn buffer_height(b: &Buffer) -> f32 {
    b.layout_runs()
        .map(|r| r.line_top + r.line_height)
        .fold(0.0, f32::max)
}

fn buffer_width(b: &Buffer) -> f32 {
    b.layout_runs().map(|r| r.line_w).fold(0.0, f32::max)
}

impl Layout<'_> {
    pub fn document(&mut self, blocks: &[Block]) {
        let s = self.style;
        let x = s.margin as i32;
        let width = s.width.saturating_sub(2 * s.margin).max(16);
        let end = self.blocks(blocks, x, s.padding_top as i32, width, 0);
        self.canvas
            .set_height((end + s.padding_bottom as i32).max(1) as u32);
    }

    fn body(&self) -> TextStyle {
        TextStyle {
            size: self.style.body_size,
            bold: false,
            mono: false,
            align: Align::Left,
        }
    }

    fn line_px(&self, size: f32) -> f32 {
        (size * self.style.line_height).round()
    }

    fn blocks(&mut self, blocks: &[Block], x: i32, mut y: i32, width: u32, depth: usize) -> i32 {
        for (i, b) in blocks.iter().enumerate() {
            if i > 0 {
                let gap = self.style.block_gap;
                y += match b {
                    Block::Heading(..) => (gap * 1.6) as i32,
                    // A list nested in a list item hugs its parent line.
                    Block::List { .. } if depth > 0 => (self.style.body_size * 0.25) as i32,
                    _ => gap as i32,
                };
            }
            y = self.block(b, x, y, width, depth);
        }
        y
    }

    fn block(&mut self, b: &Block, x: i32, y: i32, width: u32, depth: usize) -> i32 {
        match b {
            Block::Paragraph(spans) => y + self.text(spans, x, y, width, self.body()),
            Block::Heading(level, spans) => {
                let size = self.style.heading_sizes[(*level as usize).clamp(1, 6) - 1];
                let h = self.text(
                    spans,
                    x,
                    y,
                    width,
                    TextStyle {
                        size,
                        bold: true,
                        ..self.body()
                    },
                );
                match level {
                    1 => {
                        self.rect(x, y + h + 2, width, 3);
                        y + h + 5
                    }
                    2 => {
                        self.rect(x, y + h + 2, width, 1);
                        y + h + 3
                    }
                    _ => y + h,
                }
            }
            Block::Code(code) => {
                let pad = 6;
                let span = Span {
                    text: code.replace('\t', "    "),
                    ..Default::default()
                };
                let ts = TextStyle {
                    size: self.style.code_size,
                    mono: true,
                    ..self.body()
                };
                let h = self.text(
                    &[span],
                    x + pad,
                    y + pad,
                    width.saturating_sub(2 * pad as u32),
                    ts,
                );
                let box_h = h + 2 * pad;
                self.canvas.ensure_height((y + box_h) as u32);
                self.canvas.stroke_rect(x, y, width, box_h as u32, 1);
                y + box_h
            }
            Block::Quote(inner) => {
                let indent = self.style.quote_indent;
                let end = self.blocks(
                    inner,
                    x + indent as i32,
                    y,
                    width.saturating_sub(indent),
                    depth,
                );
                self.rect(x, y, 3, (end - y).max(0) as u32);
                end
            }
            Block::List { start, items } => self.list(*start, items, x, y, width, depth),
            Block::Rule => {
                let mid = y + self.style.block_gap as i32 / 2;
                self.rect(x, mid, width, 2);
                mid + 2 + self.style.block_gap as i32 / 2
            }
            Block::Table { aligns, head, rows } => self.table(aligns, head, rows, x, y, width),
            Block::Image { url, alt } => self.image(url, alt, x, y, width),
        }
    }

    // --- primitives ---------------------------------------------------------

    fn rect(&mut self, x: i32, y: i32, w: u32, h: u32) {
        self.canvas.ensure_height((y + h as i32).max(0) as u32);
        self.canvas.fill_rect(x, y, w, h, true);
    }

    /// Thick line made of square stamps.
    fn line(&mut self, (x0, y0): (f32, f32), (x1, y1): (f32, f32), thick: u32) {
        let steps = ((x1 - x0).abs().max((y1 - y0).abs()) * 2.0).ceil().max(1.0) as i32;
        let half = thick as f32 / 2.0;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let (px, py) = (x0 + (x1 - x0) * t - half, y0 + (y1 - y0) * t - half);
            self.rect(px.round() as i32, py.round() as i32, thick, thick);
        }
    }

    fn disc(&mut self, cx: f32, cy: f32, r: f32, filled: bool) {
        let ri = r.ceil() as i32 + 1;
        self.canvas.ensure_height((cy + r + 2.0) as u32);
        for dy in -ri..=ri {
            for dx in -ri..=ri {
                let d = ((dx as f32 + 0.5).powi(2) + (dy as f32 + 0.5).powi(2)).sqrt();
                if d <= r && (filled || d >= r - 2.0) {
                    self.canvas
                        .set((cx as i32 + dx) as u32, (cy as i32 + dy) as u32, true);
                }
            }
        }
    }

    // --- text ---------------------------------------------------------------

    fn shape(&mut self, spans: &[Span], width: Option<u32>, ts: TextStyle) -> Buffer {
        let s = self.style;
        let line_h = self.line_px(ts.size);
        let mut buffer = Buffer::new(self.fs, Metrics::new(ts.size, line_h));
        buffer.set_size(width.map(|w| w as f32), None);
        buffer.set_wrap(if width.is_some() {
            Wrap::WordOrGlyph
        } else {
            Wrap::None
        });

        let family = |mono: bool| {
            Family::Name(if mono {
                s.mono_family.as_str()
            } else {
                s.sans_family.as_str()
            })
        };
        let base = Attrs::new().family(family(ts.mono)).weight(if ts.bold {
            Weight::BOLD
        } else {
            Weight::NORMAL
        });
        let attrs: Vec<Attrs> = spans
            .iter()
            .map(|sp| {
                let mut a = Attrs::new()
                    .family(family(ts.mono || sp.code))
                    .weight(if ts.bold || sp.bold {
                        Weight::BOLD
                    } else {
                        Weight::NORMAL
                    })
                    .style(if sp.italic {
                        FontStyle::Italic
                    } else {
                        FontStyle::Normal
                    });
                if sp.code && !ts.mono {
                    a = a.metrics(Metrics::new((ts.size * 0.9).round(), line_h));
                }
                if sp.link {
                    a = a.underline(UnderlineStyle::Single);
                }
                if sp.strike {
                    a = a.strikethrough();
                }
                a
            })
            .collect();
        buffer.set_rich_text(
            spans.iter().zip(attrs).map(|(sp, a)| (sp.text.as_str(), a)),
            &base,
            Shaping::Advanced,
            Some(ts.align),
        );
        buffer.shape_until_scroll(self.fs, false);
        buffer
    }

    fn draw_buffer(&mut self, buffer: &mut Buffer, x: i32, y: i32) {
        let h = buffer_height(buffer);
        self.canvas
            .ensure_height((y as f32 + h * 1.5 + 4.0).max(0.0) as u32);
        let threshold = self.style.text_threshold;
        let canvas = &mut self.canvas;
        buffer.draw(
            self.fs,
            self.swash,
            Color::rgb(0, 0, 0),
            |gx, gy, w, h, c| {
                if c.a() >= threshold {
                    canvas.fill_rect(x + gx, y + gy, w, h, true);
                }
            },
        );
    }

    /// Lay out and draw text; returns the height used.
    fn text(&mut self, spans: &[Span], x: i32, y: i32, width: u32, ts: TextStyle) -> i32 {
        let mut buffer = self.shape(spans, Some(width), ts);
        self.draw_buffer(&mut buffer, x, y);
        buffer_height(&buffer).ceil() as i32
    }

    // --- lists --------------------------------------------------------------

    fn list(
        &mut self,
        start: Option<u64>,
        items: &[Item],
        x: i32,
        mut y: i32,
        width: u32,
        depth: usize,
    ) -> i32 {
        let indent = self.style.list_indent;
        let body = self.style.body_size;
        let first_line_mid = self.line_px(body) / 2.0;
        let item_gap = (body * 0.25) as i32;

        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                y += item_gap;
            }
            let content_x = x + indent as i32;
            let end = self.blocks(
                &item.blocks,
                content_x,
                y,
                width.saturating_sub(indent),
                depth + 1,
            );
            let cy = y as f32 + first_line_mid;
            let cx = x as f32 + indent as f32 * 0.45;

            match (item.checked, start) {
                (Some(done), _) => {
                    let s = (body * 0.62).round();
                    let (bx, by) = (cx - s / 2.0, cy - s / 2.0);
                    self.canvas.ensure_height((by + s + 1.0) as u32);
                    self.canvas
                        .stroke_rect(bx as i32, by as i32, s as u32, s as u32, 2);
                    if done {
                        self.line(
                            (bx + s * 0.22, by + s * 0.52),
                            (bx + s * 0.42, by + s * 0.74),
                            2,
                        );
                        self.line(
                            (bx + s * 0.42, by + s * 0.74),
                            (bx + s * 0.80, by + s * 0.24),
                            2,
                        );
                    }
                }
                (None, Some(n)) => {
                    let label = Span {
                        text: format!("{}.", n + i as u64),
                        ..Default::default()
                    };
                    let ts = TextStyle {
                        align: Align::Right,
                        ..self.body()
                    };
                    self.text(&[label], x, y, indent.saturating_sub(6), ts);
                }
                (None, None) => {
                    let r = body * 0.15;
                    match depth % 3 {
                        0 => self.disc(cx, cy, r, true),
                        1 => self.disc(cx, cy, r + 0.5, false),
                        _ => {
                            let s = (r * 1.8).round();
                            self.rect(
                                (cx - s / 2.0) as i32,
                                (cy - s / 2.0) as i32,
                                s as u32,
                                s as u32,
                            );
                        }
                    }
                }
            }
            y = end.max(y + self.line_px(body) as i32);
        }
        y
    }

    // --- tables -------------------------------------------------------------

    fn table(
        &mut self,
        aligns: &[Alignment],
        head: &[Vec<Span>],
        rows: &[Vec<Vec<Span>>],
        x: i32,
        y: i32,
        width: u32,
    ) -> i32 {
        let ncols = rows
            .iter()
            .map(Vec::len)
            .chain([head.len()])
            .max()
            .unwrap_or(0);
        if ncols == 0 {
            return y;
        }
        let pad = 4i32;
        let size = (self.style.body_size * 0.9).round();
        let cell_style = |bold: bool, col: usize| TextStyle {
            size,
            bold,
            mono: false,
            align: match aligns.get(col) {
                Some(Alignment::Center) => Align::Center,
                Some(Alignment::Right) => Align::Right,
                _ => Align::Left,
            },
        };

        // Natural (unwrapped) column widths.
        let all_rows: Vec<(bool, &[Vec<Span>])> = std::iter::once((true, head))
            .chain(rows.iter().map(|r| (false, r.as_slice())))
            .collect();
        let mut natural = vec![1.0f32; ncols];
        for (bold, row) in &all_rows {
            for (c, cell) in row.iter().enumerate() {
                let b = self.shape(cell, None, cell_style(*bold, c));
                natural[c] = natural[c].max(buffer_width(&b).ceil());
            }
        }
        let chrome = (ncols as i32 + 1) + ncols as i32 * 2 * pad;
        let avail = (width as i32 - chrome).max(ncols as i32 * 8) as f32;
        let total: f32 = natural.iter().sum();
        let cols: Vec<u32> = if total <= avail {
            natural.iter().map(|w| *w as u32).collect()
        } else {
            let min = (avail / ncols as f32 * 0.5).floor();
            let raw: Vec<f32> = natural
                .iter()
                .map(|w| (w / total * avail).max(min))
                .collect();
            let k = avail / raw.iter().sum::<f32>();
            raw.iter()
                .map(|w| (w * k).floor().max(4.0) as u32)
                .collect()
        };
        let table_w = cols.iter().sum::<u32>() as i32 + chrome;

        let mut cy = y;
        self.rect(x, cy, table_w as u32, 1);
        cy += 1;
        for (ri, (bold, row)) in all_rows.iter().enumerate() {
            if ri == 0 && head.is_empty() {
                continue;
            }
            let mut buffers: Vec<Buffer> = (0..ncols)
                .map(|c| {
                    let empty = Vec::new();
                    let cell = row.get(c).unwrap_or(&empty);
                    self.shape(cell, Some(cols[c]), cell_style(*bold, c))
                })
                .collect();
            let row_h = buffers
                .iter()
                .map(|b| buffer_height(b).ceil() as i32)
                .max()
                .unwrap_or(0)
                + 2 * pad;
            let mut cx = x + 1;
            for (c, buf) in buffers.iter_mut().enumerate() {
                self.draw_buffer(buf, cx + pad, cy + pad);
                cx += cols[c] as i32 + 2 * pad + 1;
            }
            cy += row_h;
            let thick = if ri == 0 && !head.is_empty() { 2 } else { 1 };
            self.rect(x, cy, table_w as u32, thick);
            cy += thick as i32;
        }
        // Vertical rules.
        let mut vx = x;
        self.rect(vx, y, 1, (cy - y) as u32);
        for w in &cols {
            vx += *w as i32 + 2 * pad + 1;
            self.rect(vx, y, 1, (cy - y) as u32);
        }
        cy
    }

    // --- images -------------------------------------------------------------

    fn image(&mut self, url: &str, alt: &str, x: i32, y: i32, width: u32) -> i32 {
        let opts = &self.style.image;
        let geo = Geometry {
            width,
            upscale: self.style.image_upscale,
            ..opts.geometry
        };
        let key = (url.to_string(), geo);
        let gray = match self.gray_cache.get(&key) {
            Some(g) => g.clone(),
            None => {
                let g = self
                    .images
                    .resolve(url)
                    .map(|img| Arc::new(thermal_raster::to_gray(&img, &geo)));
                self.gray_cache.insert(key, g.clone());
                g
            }
        };
        let Some(gray) = gray else {
            let label = if alt.is_empty() {
                format!("[image: {url}]")
            } else {
                format!("[image: {alt}]")
            };
            let span = Span {
                text: label,
                italic: true,
                ..Default::default()
            };
            return y + self.text(&[span], x, y, width, self.body());
        };
        let bmp =
            thermal_raster::dither(&thermal_raster::adjust(&gray, &opts.adjust), &opts.dither);
        let free = width as i32 - bmp.width() as i32;
        let ix = x + match self.style.image_align {
            HAlign::Left => 0,
            HAlign::Center => free / 2,
            HAlign::Right => free,
        };
        self.canvas.ensure_height((y + bmp.height() as i32) as u32);
        self.canvas.blit(&bmp, ix, y);
        y + bmp.height() as i32
    }
}
