//! Sample history for headless renders (`--snap`, `--script`), which touch neither the
//! pasteboard nor the history file.

use std::path::PathBuf;

use crate::model::{Entry, Kind, classify_text, unix_now};

pub fn entries() -> Vec<Entry> {
    let now = unix_now();
    let text = |id: i64, app: &str, age: i64, content: &str| {
        let (kind, color) = classify_text(content);
        let detail = match kind {
            Kind::Link => "Link".to_owned(),
            Kind::Color => color.map_or_else(String::new, |[r, g, b, _]| format!("rgb({r}, {g}, {b})")),
            _ => {
                let (chars, lines) = (content.chars().count(), content.lines().count());
                if lines > 1 { format!("{chars} characters · {lines} lines") } else { format!("{chars} characters") }
            }
        };
        Entry {
            id,
            kind,
            preview: content.to_owned(),
            search: content.to_lowercase(),
            detail,
            app_name: app.to_owned(),
            app_path: String::new(),
            used: now - age,
            pinned: false,
            thumb: None,
            color,
        }
    };
    let image = |id: i64, app: &str, age: i64, size: (u32, u32), hue: [f32; 3]| Entry {
        id,
        kind: Kind::Image,
        preview: "Image".to_owned(),
        search: "image".to_owned(),
        detail: format!("{} × {} · 1.4 MB", size.0 * 4, size.1 * 4),
        app_name: app.to_owned(),
        app_path: String::new(),
        used: now - age,
        pinned: false,
        thumb: picture(id, size, hue),
        color: None,
    };
    let files = |id: i64, age: i64, names: &[&str]| Entry {
        id,
        kind: Kind::Files,
        preview: names.join("\n"),
        search: names.join("\n").to_lowercase(),
        detail: if names.len() == 1 { "248.0 KB".to_owned() } else { format!("{} items", names.len()) },
        app_name: "Finder".to_owned(),
        app_path: String::new(),
        used: now - age,
        pinned: false,
        thumb: None,
        color: None,
    };

    let mut list = vec![
        text(1, "Zed", 20, "fn view(s: &State, _: &mut Cx<State>) -> El {\n    let t = tok::theme(true);\n    col().w_full().h_full().center()\n        .child(text(\"Hello, Rune\"))\n}"),
        text(2, "Safari", 95, "https://github.com/anomalyco/rune"),
        image(3, "Screenshot", 260, (480, 300), [0.95, 0.45, 0.35]),
        text(4, "Figma", 900, "#0A84FF"),
        text(5, "Notes", 2_400, "Buy oat milk, lemons, basil and sourdough. Call the landlord about the heating before Friday."),
        files(6, 4_100, &["Quarterly report.pdf", "Budget 2026.xlsx", "Roadmap.key"]),
        text(7, "Terminal", 7_300, "cargo build --release && scripts/bundle.sh"),
        image(8, "Preview", 9_000, (300, 420), [0.3, 0.7, 0.95]),
        text(9, "Mail", 26_000, "mailto:hello@example.com"),
        text(10, "Slack", 90_000, "Ship it on Thursday. The Rune rev is pinned, so the build is reproducible."),
        text(11, "Safari", 180_000, "https://developer.apple.com/design/human-interface-guidelines/materials"),
        text(12, "Xcode", 400_000, "rgb(255, 149, 0)"),
        files(13, 700_000, &["nerda-clipboard.app"]),
    ];
    list[4].pinned = true;
    list
}

/// A soft two-tone picture written to the temp folder, standing in for a copied image.
fn picture(id: i64, (width, height): (u32, u32), hue: [f32; 3]) -> Option<PathBuf> {
    let dir = std::env::temp_dir().join("nerda-clipboard-demo");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{id}.png"));
    let image = image::RgbaImage::from_fn(width, height, |x, y| {
        let (u, v) = (x as f32 / width as f32, y as f32 / height as f32);
        let wave = (0.5 + 0.5 * ((u * 5.0).sin() * (v * 4.0).cos())) * 0.35;
        let channel = |c: f32| ((c * (0.45 + 0.55 * (1.0 - v)) + wave * (1.0 - c)).clamp(0.0, 1.0) * 255.0) as u8;
        image::Rgba([channel(hue[0]), channel(hue[1]), channel(hue[2]), 255])
    });
    image.save(&path).ok()?;
    Some(path)
}
