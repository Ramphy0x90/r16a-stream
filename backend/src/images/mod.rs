//! Posters, backdrops and stills (SPEC 4.3, 8.2, 8.3).
//!
//! The API downloads a chosen TMDB image into the uploads bucket and queues an image job. The
//! worker decodes it with ffmpeg in its sandbox, writes AVIF and WebP per width to the media
//! bucket, and marks the image ready. Clients get presigned URLs that stay the same for a day.

pub mod ffmpeg;
pub mod ingest;
pub mod repository;
pub mod urls;
pub mod worker;

use uuid::Uuid;

/// Largest side and pixel count an image may have (SPEC 8.2). A decompression bomb is refused
/// after the probe, before any decode.
pub const MAX_SIDE: u32 = 16_384;
pub const MAX_PIXELS: u64 = 40_000_000;

/// Output widths per image kind, never wider than the source (SPEC 4.3).
pub fn widths(kind: &str, source_width: u32) -> Vec<u32> {
    let ladder: &[u32] = match kind {
        "backdrop" => &[780, 1280, 1920],
        "profile" => &[185],
        _ => &[185, 342, 500, 780],
    };
    let mut out: Vec<u32> = ladder.iter().copied().filter(|w| *w <= source_width).collect();
    // A source narrower than the smallest rung still gets one output at its own width.
    if out.is_empty() {
        out.push(source_width.clamp(16, 4096));
    }
    out
}

pub fn source_key(image: Uuid) -> String {
    format!("images/{image}/source")
}

pub fn variant_key(image: Uuid, width: u32, ext: &str) -> String {
    format!("i/{image}/{width}.{ext}")
}

/// The HSL notation of the design tokens, `H S% L%`, for an sRGB colour.
// r, g, b, h, s and l are the names of the colour model itself.
#[allow(clippy::many_single_char_names)]
pub fn hsl(r: u8, g: u8, b: u8) -> String {
    let (r, g, b) = (f64::from(r) / 255.0, f64::from(g) / 255.0, f64::from(b) / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = f64::midpoint(max, min);
    let d = max - min;
    let (h, s) = if d == 0.0 {
        (0.0, 0.0)
    } else {
        let s = d / (1.0 - (2.0 * l - 1.0).abs());
        let h = if (max - r).abs() < f64::EPSILON {
            60.0 * (((g - b) / d) % 6.0)
        } else if (max - g).abs() < f64::EPSILON {
            60.0 * ((b - r) / d + 2.0)
        } else {
            60.0 * ((r - g) / d + 4.0)
        };
        (if h < 0.0 { h + 360.0 } else { h }, s)
    };
    format!(
        "{:.0} {:.0}% {:.0}%",
        h.round() % 360.0,
        (s * 100.0).round(),
        (l * 100.0).round()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_follow_the_kind_and_the_source() {
        assert_eq!(widths("poster", 2000), [185, 342, 500, 780]);
        assert_eq!(widths("poster", 400), [185, 342]);
        assert_eq!(widths("backdrop", 1280), [780, 1280]);
        assert_eq!(widths("poster", 100), [100]);
        assert_eq!(widths("still", 8), [16]);
    }

    #[test]
    fn hsl_matches_the_token_notation() {
        assert_eq!(hsl(0x10, 0xb9, 0x81), "160 84% 39%");
        assert_eq!(hsl(0, 0, 0), "0 0% 0%");
        assert_eq!(hsl(255, 255, 255), "0 0% 100%");
        assert_eq!(hsl(255, 0, 0), "0 100% 50%");
    }

    #[test]
    fn keys_pass_the_storage_key_check() {
        let id = Uuid::new_v4();
        assert!(crate::storage::valid_key(&source_key(id)));
        assert!(crate::storage::valid_key(&variant_key(id, 342, "avif")));
    }
}
