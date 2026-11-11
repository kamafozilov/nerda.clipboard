//! What a clipboard history entry is, and how copied content is classified.

use std::path::PathBuf;

/// Longest preview kept in memory per entry; the full content stays in the database.
pub const PREVIEW_CHARS: usize = 600;
/// Longest lowercased search text kept in memory per entry.
pub const SEARCH_CHARS: usize = 4000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Text,
    Link,
    Image,
    Files,
    Color,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Link => "link",
            Kind::Image => "image",
            Kind::Files => "files",
            Kind::Color => "color",
        }
    }

    pub fn parse(s: &str) -> Kind {
        match s {
            "link" => Kind::Link,
            "image" => Kind::Image,
            "files" => Kind::Files,
            "color" => Kind::Color,
            _ => Kind::Text,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Text => "Text",
            Kind::Link => "Link",
            Kind::Image => "Image",
            Kind::Files => "Files",
            Kind::Color => "Color",
        }
    }
}

/// One row of the history, as the shelf shows it. Payloads are not held in memory.
#[derive(Clone, Debug)]
pub struct Entry {
    pub id: i64,
    pub kind: Kind,
    pub preview: String,
    pub search: String,
    pub detail: String,
    pub app_name: String,
    pub app_path: String,
    /// Unix seconds of the last copy or use.
    pub used: i64,
    pub pinned: bool,
    pub thumb: Option<PathBuf>,
    /// sRGB bytes of a copied colour.
    pub color: Option<[u8; 4]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Text,
    Links,
    Images,
    Files,
    Pinned,
}

impl Filter {
    pub const ALL: [Filter; 6] = [Filter::All, Filter::Text, Filter::Links, Filter::Images, Filter::Files, Filter::Pinned];

    pub fn label(self) -> &'static str {
        match self {
            Filter::All => "All",
            Filter::Text => "Text",
            Filter::Links => "Links",
            Filter::Images => "Images",
            Filter::Files => "Files",
            Filter::Pinned => "Pinned",
        }
    }

    pub fn accepts(self, e: &Entry) -> bool {
        match self {
            Filter::All => true,
            Filter::Text => matches!(e.kind, Kind::Text | Kind::Color),
            Filter::Links => e.kind == Kind::Link,
            Filter::Images => e.kind == Kind::Image,
            Filter::Files => e.kind == Kind::Files,
            Filter::Pinned => e.pinned,
        }
    }

    pub fn step(self, by: isize) -> Filter {
        let n = Self::ALL.len() as isize;
        let i = Self::ALL.iter().position(|f| *f == self).unwrap_or(0) as isize;
        Self::ALL[((i + by).rem_euclid(n)) as usize]
    }
}

/// The first `max` characters of `s`, on a character boundary.
pub fn clip(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// A single-token URL with a scheme people paste as a link.
pub fn is_link(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() || t.len() > 2048 || t.chars().any(char::is_whitespace) {
        return false;
    }
    let lower = t.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("mailto:") {
        return rest.contains('@');
    }
    ["http://", "https://", "ftp://", "file://"].iter().any(|scheme| lower.strip_prefix(scheme).is_some_and(|rest| !rest.is_empty()))
}

/// The host of a link, without `www.`, for a card's headline.
pub fn link_host(url: &str) -> String {
    let t = url.trim();
    let after = t.split_once("://").map_or_else(|| t.split_once(':').map_or(t, |(_, r)| r), |(_, r)| r);
    let host = after.split(['/', '?', '#']).next().unwrap_or(after);
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    host.strip_prefix("www.").unwrap_or(host).to_owned()
}

/// `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb(r, g, b)` or `rgba(r, g, b, a)` as sRGB bytes.
pub fn parse_color(text: &str) -> Option<[u8; 4]> {
    let t = text.trim();
    if let Some(hex) = t.strip_prefix('#') {
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let v = |s: &str| u8::from_str_radix(s, 16).ok();
        return match hex.len() {
            3 => {
                let d: Vec<u8> = hex.chars().filter_map(|c| c.to_digit(16).map(|d| (d * 17) as u8)).collect();
                Some([d[0], d[1], d[2], 255])
            }
            6 => Some([v(&hex[0..2])?, v(&hex[2..4])?, v(&hex[4..6])?, 255]),
            8 => Some([v(&hex[0..2])?, v(&hex[2..4])?, v(&hex[4..6])?, v(&hex[6..8])?]),
            _ => None,
        };
    }
    let lower = t.to_ascii_lowercase();
    let inner = lower.strip_prefix("rgba(").or_else(|| lower.strip_prefix("rgb("))?.strip_suffix(')')?;
    let parts: Vec<&str> = inner.split([',', '/']).map(str::trim).collect();
    if parts.len() != 3 && parts.len() != 4 {
        return None;
    }
    let channel = |s: &str| s.parse::<f32>().ok().filter(|v| (0.0..=255.0).contains(v)).map(|v| v.round() as u8);
    let alpha = match parts.get(3) {
        None => 255,
        Some(a) => match a.strip_suffix('%') {
            Some(p) => (p.parse::<f32>().ok()?.clamp(0.0, 100.0) * 2.55).round() as u8,
            None => (a.parse::<f32>().ok()?.clamp(0.0, 1.0) * 255.0).round() as u8,
        },
    };
    Some([channel(parts[0])?, channel(parts[1])?, channel(parts[2])?, alpha])
}

pub fn classify_text(text: &str) -> (Kind, Option<[u8; 4]>) {
    if let Some(color) = parse_color(text) {
        (Kind::Color, Some(color))
    } else if is_link(text) {
        (Kind::Link, None)
    } else {
        (Kind::Text, None)
    }
}

/// "now", "5m", "3h", "2d" or "4w": how long ago `then` was.
pub fn ago(now: i64, then: i64) -> String {
    let s = (now - then).max(0);
    match s {
        0..60 => "now".to_owned(),
        60..3600 => format!("{}m", s / 60),
        3600..86_400 => format!("{}h", s / 3600),
        86_400..604_800 => format!("{}d", s / 86_400),
        _ => format!("{}w", s / 604_800),
    }
}

/// 1536 as "1.5 KB".
pub fn bytes_label(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = n as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{n} B") } else { format!("{v:.1} {}", UNITS[unit]) }
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_single_tokens_with_a_scheme() {
        assert!(is_link("https://github.com/anomalyco/rune"));
        assert!(is_link("  http://example.com/a?b=c  "));
        assert!(is_link("mailto:someone@example.com"));
        assert!(is_link("file:///tmp/a.txt"));
        assert!(!is_link("see https://example.com"));
        assert!(!is_link("https://"));
        assert!(!is_link("example.com"));
    }

    #[test]
    fn hosts_drop_www_and_credentials() {
        assert_eq!(link_host("https://www.example.com/path?q=1"), "example.com");
        assert_eq!(link_host("https://user:pw@host.dev:8080/x"), "host.dev:8080");
        assert_eq!(link_host("mailto:someone@example.com"), "example.com");
    }

    #[test]
    fn colours_parse_from_hex_and_rgb() {
        assert_eq!(parse_color("#fff"), Some([255, 255, 255, 255]));
        assert_eq!(parse_color("#0A84FF"), Some([10, 132, 255, 255]));
        assert_eq!(parse_color("#0a84ff80"), Some([10, 132, 255, 128]));
        assert_eq!(parse_color("rgb(10, 132, 255)"), Some([10, 132, 255, 255]));
        assert_eq!(parse_color("rgba(10, 132, 255, 0.5)"), Some([10, 132, 255, 128]));
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("#issue"), None);
        assert_eq!(parse_color("rgb(300, 0, 0)"), None);
    }

    #[test]
    fn text_is_classified_by_its_shape() {
        assert_eq!(classify_text("hello world").0, Kind::Text);
        assert_eq!(classify_text("https://example.com").0, Kind::Link);
        assert_eq!(classify_text("#ff8800").0, Kind::Color);
    }

    #[test]
    fn clip_respects_character_boundaries() {
        assert_eq!(clip("salom dunyo", 5), "salom");
        assert_eq!(clip("oʻzbek", 2), "oʻ");
        assert_eq!(clip("ab", 5), "ab");
    }

    #[test]
    fn ages_read_short() {
        assert_eq!(ago(100, 90), "now");
        assert_eq!(ago(1000, 100), "15m");
        assert_eq!(ago(10_000, 100), "2h");
        assert_eq!(ago(200_000, 100), "2d");
    }

    #[test]
    fn filters_cycle_both_ways() {
        assert_eq!(Filter::All.step(1), Filter::Text);
        assert_eq!(Filter::All.step(-1), Filter::Pinned);
        assert_eq!(Filter::Pinned.step(1), Filter::All);
    }
}
