//! QR codes drawn dot-for-dot: every module is a whole number of print-head dots, so
//! edges stay sharp instead of being resampled.

use qrcodegen::{QrCode, QrCodeEcc, QrSegment, Version};

use crate::{Bitmap, paper};

/// Smallest module, in dots (0.375 mm). Thermal dots spread a little, and 3 dots
/// is the smallest that phones still scan reliably.
pub const QR_MIN_MODULE_DOTS: u32 = 3;

/// White border around the code, in modules (the QR specification's minimum).
const QUIET_ZONE: u32 = 4;

/// Largest version that fits across the print head at [`QR_MIN_MODULE_DOTS`]
/// (version 25: 117 modules, about 1,000 characters of a URL).
const MAX_VERSION: u8 = ((paper::WIDTH_DOTS / QR_MIN_MODULE_DOTS - 2 * QUIET_ZONE - 17) / 4) as u8;

/// How much of a code can be smudged or torn and still scan.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum QrEcc {
    Low,
    #[default]
    Medium,
    Quartile,
    High,
}

impl QrEcc {
    pub const ALL: [QrEcc; 4] = [QrEcc::Low, QrEcc::Medium, QrEcc::Quartile, QrEcc::High];

    pub fn name(self) -> &'static str {
        match self {
            QrEcc::Low => "Low (7%)",
            QrEcc::Medium => "Medium (15%)",
            QrEcc::Quartile => "Quartile (25%)",
            QrEcc::High => "High (30%)",
        }
    }
}

impl From<QrEcc> for QrCodeEcc {
    fn from(e: QrEcc) -> Self {
        match e {
            QrEcc::Low => QrCodeEcc::Low,
            QrEcc::Medium => QrCodeEcc::Medium,
            QrEcc::Quartile => QrCodeEcc::Quartile,
            QrEcc::High => QrCodeEcc::High,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct QrOptions {
    /// Minimum error correction. A higher level is used automatically when it
    /// doesn't make the code any larger.
    pub ecc: QrEcc,
    /// Widest the code may be, quiet zone included, in dots (at most the print head).
    /// A code that would need modules smaller than [`QR_MIN_MODULE_DOTS`] is drawn
    /// larger than this rather than unreadably small.
    pub max_width: u32,
}

impl Default for QrOptions {
    fn default() -> Self {
        Self {
            ecc: QrEcc::default(),
            max_width: paper::WIDTH_DOTS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QrError {
    #[error("nothing to encode")]
    Empty,
    #[error(
        "too much text to fit a scannable QR code on the paper; shorten it or lower the error correction"
    )]
    TooLong,
}

/// Encode `data` as a square QR code, quiet zone included, at most
/// [`paper::WIDTH_DOTS`] wide. Modules are as large as `opts.max_width` allows.
pub fn qr_code(data: &str, opts: &QrOptions) -> Result<Bitmap, QrError> {
    if data.is_empty() {
        return Err(QrError::Empty);
    }
    let code = QrCode::encode_segments_advanced(
        &QrSegment::make_segments(data),
        opts.ecc.into(),
        Version::MIN,
        Version::new(MAX_VERSION),
        None,
        true,
    )
    .map_err(|_| QrError::TooLong)?;

    let total = code.size() as u32 + 2 * QUIET_ZONE;
    // MAX_VERSION guarantees `total * QR_MIN_MODULE_DOTS` fits the print head.
    let scale = (opts.max_width.min(paper::WIDTH_DOTS) / total).max(QR_MIN_MODULE_DOTS);
    let quiet = (QUIET_ZONE * scale) as i32;
    let module = |dot: u32| (dot as i32 - quiet).div_euclid(scale as i32);
    let side = total * scale;
    Ok(Bitmap::from_fn(side, side, |x, y| {
        code.get_module(module(x), module(y))
    }))
}

/// What a QR code for a typed link should hold: surrounding whitespace is removed and
/// `https://` is added to bare addresses like `example.com/menu`, which many phone
/// cameras would otherwise treat as plain text. Anything with a scheme (`http:`,
/// `mailto:`, `WIFI:` …), spaces or no domain is kept as typed.
pub fn normalize_url(input: &str) -> String {
    let s = input.trim();
    let host = s.split(['/', '?', '#', ':']).next().unwrap_or_default();
    if !s.contains(char::is_whitespace) && !has_scheme(s) && is_host(host) {
        format!("https://{s}")
    } else {
        s.to_string()
    }
}

/// `localhost`, an IPv4 address, or a dotted name whose last label starts with a letter.
fn is_host(host: &str) -> bool {
    let labels: Vec<&str> = host.split('.').collect();
    let dotted = labels.len() >= 2 && labels.iter().all(|l| !l.is_empty() && !l.contains('@'));
    let ipv4 = labels.len() == 4 && labels.iter().all(|l| l.parse::<u8>().is_ok());
    let tld = labels
        .last()
        .is_some_and(|l| l.starts_with(char::is_alphabetic));
    host.eq_ignore_ascii_case("localhost") || dotted && (ipv4 || tld)
}

/// `true` for `scheme:…`, but not for a `host:port` like `example.com:8080`.
fn has_scheme(s: &str) -> bool {
    let Some((scheme, rest)) = s.split_once(':') else {
        return false;
    };
    scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        && !rest.starts_with(|c: char| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Position of the first black dot (the finder pattern's corner) and the module
    /// size implied by the quiet zone in front of it.
    fn corner_and_scale(bmp: &Bitmap) -> (u32, u32) {
        let first = (0..bmp.height())
            .flat_map(|y| (0..bmp.width()).map(move |x| (x, y)))
            .find(|&(x, y)| bmp.get(x, y))
            .expect("some ink");
        assert_eq!(first.0, first.1, "code should start on the diagonal");
        (first.0, first.0 / QUIET_ZONE)
    }

    #[test]
    fn round_trips_through_a_decoder() {
        let samples = [
            "https://example.com",
            "https://github.com/nayuki/QR-Code-generator/blob/master/rust/src/lib.rs?plain=1#L42",
            "WIFI:T:WPA;S:Home;P:correct horse battery staple;;",
            "வணக்கம் https://example.com/தமிழ்",
            "12345678901234567890",
        ];
        for data in samples {
            for ecc in QrEcc::ALL {
                let opts = QrOptions {
                    ecc,
                    ..Default::default()
                };
                assert_eq!(decode(&qr_code(data, &opts).unwrap()), data, "{ecc:?}");
            }
        }
    }

    #[test]
    fn modules_are_whole_dots_with_a_quiet_zone() {
        for max_width in [96, 200, 300, paper::WIDTH_DOTS] {
            let opts = QrOptions {
                max_width,
                ..Default::default()
            };
            let bmp = qr_code("https://example.com/menu", &opts).unwrap();
            assert_eq!(bmp.width(), bmp.height());
            assert!(bmp.width() <= paper::WIDTH_DOTS);
            let (corner, scale) = corner_and_scale(&bmp);
            assert!(scale >= QR_MIN_MODULE_DOTS, "scale {scale}");
            assert_eq!(corner, QUIET_ZONE * scale);
            assert_eq!(bmp.width() % scale, 0);
            // The finder pattern's top edge is exactly 7 modules of solid black.
            let run = (corner..bmp.width())
                .take_while(|&x| bmp.get(x, corner))
                .count();
            assert_eq!(run as u32, 7 * scale);
        }
    }

    #[test]
    fn max_width_sets_the_size_but_never_below_the_minimum_module() {
        let data = "https://example.com/menu";
        let size = |max_width| {
            qr_code(
                data,
                &QrOptions {
                    max_width,
                    ..Default::default()
                },
            )
            .unwrap()
            .width()
        };
        assert!(size(200) <= 200);
        assert!(size(paper::WIDTH_DOTS) > size(200));
        let (_, scale) = corner_and_scale(
            &qr_code(
                data,
                &QrOptions {
                    max_width: 1,
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        assert_eq!(scale, QR_MIN_MODULE_DOTS);
    }

    #[test]
    fn the_largest_code_still_scans_at_the_minimum_module() {
        let long = format!("https://example.com/{}", "a".repeat(900));
        let opts = QrOptions {
            ecc: QrEcc::Low,
            ..Default::default()
        };
        let bmp = qr_code(&long, &opts).unwrap();
        assert_eq!(corner_and_scale(&bmp).1, QR_MIN_MODULE_DOTS);
        assert_eq!(decode(&bmp), long);
    }

    #[test]
    fn rejects_empty_and_oversized_input() {
        let opts = QrOptions::default();
        assert_eq!(qr_code("", &opts), Err(QrError::Empty));
        assert_eq!(qr_code(&"x".repeat(1500), &opts), Err(QrError::TooLong));
    }

    #[test]
    fn normalizes_typed_links() {
        let cases = [
            ("example.com", "https://example.com"),
            (
                "  www.example.com/menu?a=1  ",
                "https://www.example.com/menu?a=1",
            ),
            ("example.com:8080/x", "https://example.com:8080/x"),
            ("localhost:3000", "https://localhost:3000"),
            ("192.168.1.10/admin", "https://192.168.1.10/admin"),
            ("medium.com/@someone", "https://medium.com/@someone"),
            ("http://example.com", "http://example.com"),
            ("HTTPS://EXAMPLE.COM", "HTTPS://EXAMPLE.COM"),
            ("mailto:a@example.com", "mailto:a@example.com"),
            ("tel:+15551234", "tel:+15551234"),
            ("WIFI:T:WPA;S:Home;P:pw;;", "WIFI:T:WPA;S:Home;P:pw;;"),
            ("someone@example.com", "someone@example.com"),
            ("hello", "hello"),
            ("hello world.com", "hello world.com"),
            ("1.5", "1.5"),
            ("a..com", "a..com"),
            ("", ""),
        ];
        for (input, want) in cases {
            assert_eq!(normalize_url(input), want, "{input:?}");
        }
    }
}
