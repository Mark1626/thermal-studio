//! LuckPrinter "Lujiang" protocol for the Seznik MiniX / LuckJingle S1, as verified on
//! hardware (see `RESEARCH.md` §5). Pure byte building and parsing — no I/O — so any
//! transport (BLE, SPP, Android) can reuse it.
//!
//! A print job is a list of [`Segment`]s; the transport writes each one in order,
//! chunking the raster and waiting for the printer's `OK` where [`Segment::expect_ok`] is set.

mod session;

pub use session::{Action, PrintOptions, PrintSession, Progress, SessionError, SessionReport};

use thermal_raster::{Bitmap, HAlign, paper};

/// BLE GATT layout (16-bit UUIDs in the Bluetooth base UUID).
pub mod gatt {
    /// Printer service.
    pub const SERVICE: u16 = 0xFF00;
    /// Command / data input (write, write-without-response).
    pub const WRITE: u16 = 0xFF02;
    /// Replies: `OK`, query results (notify).
    pub const REPLY: u16 = 0xFF01;
    /// Flow-control credits (notify).
    pub const FLOW: u16 = 0xFF03;
    /// Name fragment the printer advertises (`Seznik MiniX_XXXX_LE`).
    pub const NAME_HINT: &str = "MiniX";
    /// Name fragments that mark a nearby device as a likely printer (the printer
    /// doesn't advertise its service UUID, so scans can only go by name).
    pub const NAME_HINTS: [&str; 3] = [NAME_HINT, "LuckP", "printer"];

    /// `true` if an advertised name looks like a supported printer.
    pub fn looks_like_printer(name: &str) -> bool {
        let name = name.to_lowercase();
        NAME_HINTS.iter().any(|h| name.contains(&h.to_lowercase()))
    }
}

/// Raw commands.
pub mod cmd {
    /// Wake the printer before a job.
    pub const WAKE: [u8; 12] = [0; 12];
    /// Enable "Lujiang mode"; without it the printer accepts data but never prints.
    pub const ENABLE: [u8; 4] = [0x10, 0xFF, 0xF1, 0x03];
    /// End of job.
    pub const END_JOB: [u8; 4] = [0x10, 0xFF, 0xF1, 0x45];

    /// Status bitfield → 1 byte on FF01 (see [`crate::Status`]).
    pub const QUERY_STATUS: [u8; 3] = [0x10, 0xFF, 0x40];
    /// Model string, e.g. `S1_EY628`.
    pub const QUERY_MODEL: [u8; 4] = [0x10, 0xFF, 0x20, 0xF0];
    /// Firmware string, e.g. `V1.6.23`.
    pub const QUERY_FIRMWARE: [u8; 4] = [0x10, 0xFF, 0x20, 0xF1];
    /// Serial number string.
    pub const QUERY_SERIAL: [u8; 4] = [0x10, 0xFF, 0x20, 0xF2];
    /// Battery → `00 <percent>`.
    pub const QUERY_BATTERY: [u8; 4] = [0x10, 0xFF, 0x50, 0xF1];
    /// Stored density → `<0|1|2> 00 00`.
    pub const QUERY_DENSITY: [u8; 3] = [0x10, 0xFF, 0x11];

    /// Set print density (replies `OK`).
    pub fn set_density(d: crate::Density) -> [u8; 5] {
        [0x10, 0xFF, 0x10, 0x00, d as u8]
    }

    /// Feed paper by `dots` rows (8 dots = 1 mm).
    pub fn feed(dots: u8) -> [u8; 3] {
        [0x1B, 0x4A, dots]
    }

    /// ESC/POS `GS v 0` raster header (normal scale).
    pub fn raster_header(bytes_per_row: u16, rows: u16) -> [u8; 8] {
        let [wl, wh] = bytes_per_row.to_le_bytes();
        let [rl, rh] = rows.to_le_bytes();
        [0x1D, 0x76, 0x30, 0x00, wl, wh, rl, rh]
    }
}

/// Print darkness (heating).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Density {
    Light = 0,
    Normal = 1,
    Dark = 2,
}

impl Density {
    pub const ALL: [Density; 3] = [Density::Light, Density::Normal, Density::Dark];

    pub fn name(self) -> &'static str {
        match self {
            Density::Light => "Light",
            Density::Normal => "Normal",
            Density::Dark => "Dark",
        }
    }
}

/// Reply to [`cmd::QUERY_STATUS`]. Bit meanings come from minixprint's notes; on our
/// unit only `0x00` (ready) has been observed so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status(pub u8);

impl Status {
    pub fn busy(self) -> bool {
        self.0 & 0x01 != 0
    }
    pub fn cover_open(self) -> bool {
        self.0 & 0x02 != 0
    }
    pub fn paper_out(self) -> bool {
        self.0 & 0x04 != 0
    }
    pub fn low_battery(self) -> bool {
        self.0 & 0x08 != 0
    }
    pub fn overheated(self) -> bool {
        self.0 & 0x50 != 0
    }
    pub fn charging(self) -> bool {
        self.0 & 0x20 != 0
    }

    /// A condition that makes printing pointless, if any.
    pub fn problem(self) -> Option<&'static str> {
        if self.cover_open() {
            Some("cover is open")
        } else if self.paper_out() {
            Some("out of paper")
        } else if self.overheated() {
            Some("print head overheated")
        } else {
            None
        }
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = Vec::new();
        for (on, label) in [
            (self.busy(), "busy"),
            (self.cover_open(), "cover open"),
            (self.paper_out(), "paper out"),
            (self.low_battery(), "low battery"),
            (self.overheated(), "overheated"),
            (self.charging(), "charging"),
        ] {
            if on {
                parts.push(label);
            }
        }
        if parts.is_empty() {
            write!(f, "ready")
        } else {
            write!(f, "{}", parts.join(", "))
        }
    }
}

/// `true` for the `OK` (`4F 4B`) acknowledgement on FF01.
pub fn is_ok(reply: &[u8]) -> bool {
    reply == b"OK"
}

/// Battery percentage from a [`cmd::QUERY_BATTERY`] reply (`00 45` → 69).
pub fn parse_battery(reply: &[u8]) -> Option<u8> {
    reply.last().copied().filter(|p| *p <= 100)
}

/// Printable ASCII from a string reply (model, firmware, serial).
pub fn parse_text(reply: &[u8]) -> String {
    reply
        .iter()
        .filter(|b| b.is_ascii_graphic() || **b == b' ')
        .map(|&b| b as char)
        .collect::<String>()
        .trim()
        .to_string()
}

/// A frame on the FF03 flow-control characteristic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlowFrame {
    /// `01 nn`: permission to send `nn` more packets.
    Credits(u8),
    /// `02 lo hi`: printer-side MTU.
    Mtu(u16),
    Unknown(Vec<u8>),
}

pub fn parse_flow(frame: &[u8]) -> FlowFrame {
    match frame {
        [0x01, n] => FlowFrame::Credits(*n),
        [0x02, lo, hi] => FlowFrame::Mtu(u16::from_le_bytes([*lo, *hi])),
        other => FlowFrame::Unknown(other.to_vec()),
    }
}

/// Job-level settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrintJob {
    /// Set the density before printing; `None` keeps the printer's stored setting.
    pub density: Option<Density>,
    /// Paper to feed after the image so it clears the tear bar (8 dots = 1 mm).
    pub feed_dots: u8,
    /// Split tall images into several `GS v 0` commands of at most this many rows.
    /// The verified path is one command per image; the header's 16-bit row count
    /// forces a split above 65 535 rows.
    pub max_band_rows: u16,
}

impl Default for PrintJob {
    fn default() -> Self {
        Self {
            density: None,
            feed_dots: 80,
            max_band_rows: u16::MAX,
        }
    }
}

/// One write (or chunked stream of writes) in a job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub label: &'static str,
    pub data: Vec<u8>,
    /// Stream in transport-sized chunks (raster data).
    pub chunked: bool,
    /// Wait for `OK` on FF01 after this segment.
    pub expect_ok: bool,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum JobError {
    #[error("image is {0} dots wide; the print head is {max} dots", max = paper::WIDTH_DOTS)]
    TooWide(u32),
    #[error("image is empty")]
    Empty,
}

impl PrintJob {
    /// Build the write sequence for `bitmap`. Narrower bitmaps are centred on the head.
    pub fn segments(&self, bitmap: &Bitmap) -> Result<Vec<Segment>, JobError> {
        if bitmap.width() > paper::WIDTH_DOTS {
            return Err(JobError::TooWide(bitmap.width()));
        }
        if bitmap.height() == 0 || bitmap.width() == 0 {
            return Err(JobError::Empty);
        }
        let padded;
        let bitmap = if bitmap.width() < paper::WIDTH_DOTS {
            padded = bitmap.padded_to_width(paper::WIDTH_DOTS, HAlign::Center);
            &padded
        } else {
            bitmap
        };

        let seg = |label, data: &[u8]| Segment {
            label,
            data: data.to_vec(),
            chunked: false,
            expect_ok: false,
        };
        let mut out = vec![seg("wake", &cmd::WAKE), seg("enable", &cmd::ENABLE)];
        if let Some(d) = self.density {
            out.push(seg("density", &cmd::set_density(d)));
        }

        let stride = bitmap.stride();
        let band = self.max_band_rows.max(1) as usize;
        let rows: Vec<&[u8]> = bitmap.rows().collect();
        let bands: Vec<&[&[u8]]> = rows.chunks(band).collect();
        let last = bands.len() - 1;
        for (i, band_rows) in bands.into_iter().enumerate() {
            let mut data = Vec::with_capacity(8 + stride * band_rows.len());
            data.extend_from_slice(&cmd::raster_header(stride as u16, band_rows.len() as u16));
            for r in band_rows {
                data.extend_from_slice(r);
            }
            out.push(Segment {
                label: "raster",
                data,
                chunked: true,
                expect_ok: i == last,
            });
        }

        if self.feed_dots > 0 {
            out.push(seg("feed", &cmd::feed(self.feed_dots)));
        }
        out.push(seg("end", &cmd::END_JOB));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reproduces_the_verified_test_print() {
        // RESEARCH.md §5.3: 384 × 200 image, 80-dot feed, stored density.
        let bmp = Bitmap::from_fn(384, 200, |x, y| (x + y) % 7 == 0);
        let segs = PrintJob::default().segments(&bmp).unwrap();
        let labels: Vec<_> = segs.iter().map(|s| s.label).collect();
        assert_eq!(labels, ["wake", "enable", "raster", "feed", "end"]);
        assert_eq!(segs[0].data, [0u8; 12]);
        assert_eq!(segs[1].data, [0x10, 0xFF, 0xF1, 0x03]);
        assert_eq!(
            segs[2].data[..8],
            [0x1D, 0x76, 0x30, 0x00, 0x30, 0x00, 0xC8, 0x00]
        );
        assert_eq!(&segs[2].data[8..], bmp.as_bytes());
        assert!(segs[2].chunked && segs[2].expect_ok);
        assert_eq!(segs[3].data, [0x1B, 0x4A, 0x50]);
        assert_eq!(segs[4].data, [0x10, 0xFF, 0xF1, 0x45]);
    }

    #[test]
    fn density_is_set_after_enable() {
        let job = PrintJob {
            density: Some(Density::Dark),
            ..Default::default()
        };
        let segs = job.segments(&Bitmap::new(384, 1)).unwrap();
        assert_eq!(segs[2].data, [0x10, 0xFF, 0x10, 0x00, 0x02]);
    }

    #[test]
    fn splits_into_bands_and_waits_only_after_the_last() {
        let job = PrintJob {
            max_band_rows: 100,
            ..Default::default()
        };
        let segs = job.segments(&Bitmap::new(384, 250)).unwrap();
        let raster: Vec<_> = segs.iter().filter(|s| s.label == "raster").collect();
        let rows: Vec<u16> = raster
            .iter()
            .map(|s| u16::from_le_bytes([s.data[6], s.data[7]]))
            .collect();
        assert_eq!(rows, [100, 100, 50]);
        assert_eq!(raster.iter().filter(|s| s.expect_ok).count(), 1);
        assert!(raster[2].expect_ok);
    }

    #[test]
    fn narrow_images_are_centred_and_wide_ones_rejected() {
        let segs = PrintJob::default()
            .segments(&Bitmap::from_fn(8, 1, |_, _| true))
            .unwrap();
        let row = &segs[2].data[8..];
        assert_eq!(row.len(), 48);
        assert_eq!(row[23], 0x0F);
        assert_eq!(row[24], 0xF0);
        assert_eq!(
            PrintJob::default().segments(&Bitmap::new(400, 1)),
            Err(JobError::TooWide(400))
        );
        assert_eq!(
            PrintJob::default().segments(&Bitmap::new(384, 0)),
            Err(JobError::Empty)
        );
    }

    #[test]
    fn parses_observed_replies() {
        assert!(is_ok(&[0x4F, 0x4B]));
        assert_eq!(parse_battery(&[0x00, 0x45]), Some(69));
        assert_eq!(parse_text(b"S1_EY628"), "S1_EY628");
        assert_eq!(parse_flow(&[0x01, 0x07]), FlowFrame::Credits(7));
        assert_eq!(parse_flow(&[0x02, 0xC8, 0x00]), FlowFrame::Mtu(200));
        assert_eq!(Status(0).problem(), None);
        assert_eq!(Status(0x04).problem(), Some("out of paper"));
        assert_eq!(Status(0).to_string(), "ready");
    }
}
