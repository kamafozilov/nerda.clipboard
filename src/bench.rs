//! Measurements, not checks: how the history and the shelf behave as content and history
//! grow. Each prints a table.
//!
//! ```sh
//! cargo test --release -- --ignored --nocapture --test-threads 1 bench::
//! NERDA_BENCH_DIR=/some/dir ...   # also keep the generated histories there, one per size
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use rune::{Env, Key, KeyEvent, Mods, Runtime};

use crate::config::Config;
use crate::model::{Kind, classify_text};
use crate::pasteboard::{self, Capture};
use crate::store::Db;
use crate::ui::{self, Ui, WINDOW_H};

/// A small deterministic generator, so every run measures the same content.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const WORDS: [&str; 24] = [
    "clipboard", "history", "render", "window", "shelf", "paste", "thumbnail", "database", "frame", "layout", "search",
    "filter", "pinned", "screenshot", "monitor", "pointer", "native", "glass", "rust", "rune", "keyboard", "shortcut",
    "memory", "budget",
];

fn prose(rng: &mut Rng, bytes: usize) -> String {
    let mut text = String::with_capacity(bytes + 16);
    while text.len() < bytes {
        text.push_str(WORDS[rng.below(WORDS.len() as u64) as usize]);
        text.push(if rng.below(14) == 0 { '\n' } else { ' ' });
    }
    text
}

/// A screenshot-like PNG: flat panels, coloured blocks and rows of fine detail standing in
/// for text, which compresses the way real UI does (far better than noise, worse than a fill).
fn screenshot(rng: &mut Rng, width: u32, height: u32) -> Vec<u8> {
    let mut image = image::RgbaImage::from_pixel(width, height, image::Rgba([28, 28, 32, 255]));
    for _ in 0..60 {
        let (w, h) = (rng.below(u64::from(width) / 3) as u32 + 40, rng.below(u64::from(height) / 4) as u32 + 20);
        let (x, y) = (rng.below(u64::from(width)) as u32, rng.below(u64::from(height)) as u32);
        let color = image::Rgba([rng.below(256) as u8, rng.below(256) as u8, rng.below(256) as u8, 255]);
        for py in y..(y + h).min(height) {
            for px in x..(x + w).min(width) {
                image.put_pixel(px, py, color);
            }
        }
    }
    for row in (0..height).step_by(22) {
        for px in 0..width {
            if rng.below(3) == 0 {
                for py in row..(row + 9).min(height) {
                    let v = 150 + rng.below(100) as u8;
                    image.put_pixel(px, py, image::Rgba([v, v, v, 255]));
                }
            }
        }
    }
    let mut png = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new_with_quality(&mut png, image::codecs::png::CompressionType::Fast, image::codecs::png::FilterType::Adaptive);
    image.write_with_encoder(encoder).unwrap();
    png
}

fn capture(kind: Kind, text: String, reps: Vec<(&str, Vec<u8>)>) -> Capture {
    let color = if kind == Kind::Color { classify_text(&text).1 } else { None };
    Capture {
        kind,
        text,
        color,
        reps: reps.into_iter().map(|(uti, data)| (uti.to_owned(), data)).collect(),
        app_name: "Bench".to_owned(),
        app_path: "/System/Applications/Notes.app".to_owned(),
        bundle_id: String::new(),
    }
}

fn text_capture(text: String) -> Capture {
    let kind = classify_text(&text).0;
    let bytes = text.clone().into_bytes();
    capture(kind, text, vec![(pasteboard::TEXT, bytes)])
}

fn image_capture(png: Vec<u8>) -> Capture {
    capture(Kind::Image, String::new(), vec![(pasteboard::PNG, png)])
}

fn bench_dir(name: &str) -> PathBuf {
    let base = std::env::var_os("NERDA_BENCH_DIR").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("nerda-clipboard-bench"));
    let dir = base.join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn dir_bytes(dir: &Path) -> u64 {
    std::fs::read_dir(dir).map_or(0, |entries| {
        entries.flatten().map(|e| e.metadata().map_or(0, |m| if m.is_dir() { dir_bytes(&e.path()) } else { m.len() })).sum()
    })
}

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn mb(bytes: u64) -> f64 {
    bytes as f64 / (1 << 20) as f64
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

/// What recording one copy costs on the history thread, by content.
#[test]
#[ignore]
fn ingest() {
    let db = Db::open(&bench_dir("ingest")).unwrap();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    println!("\n| Copied content | Payload | Record time | Thumbnail |");
    println!("| --- | --- | --- | --- |");
    let mut row = |label: &str, make: &mut dyn FnMut(&mut Rng) -> Capture| {
        let mut times = Vec::new();
        let (mut payload, mut thumb) = (0u64, 0u64);
        for _ in 0..5 {
            let c = make(&mut rng);
            payload = c.reps.iter().map(|(_, d)| d.len() as u64).sum();
            let start = Instant::now();
            let entry = db.record(&c).unwrap();
            times.push(ms(start));
            thumb = entry.thumb.and_then(|p| std::fs::metadata(p).ok()).map_or(0, |m| m.len());
        }
        let thumb = if thumb == 0 { "–".to_owned() } else { format!("{:.0} KB", thumb as f64 / 1024.0) };
        println!("| {label} | {:.2} MB | {:.2} ms | {thumb} |", mb(payload), median(times));
    };
    for (label, bytes) in [("Text, 60 bytes", 60), ("Text, 2 KB", 2 << 10), ("Text, 100 KB", 100 << 10), ("Text, 1 MB", 1 << 20), ("Text, 8 MB", (8 << 20) - 64)] {
        row(label, &mut |rng| text_capture(prose(rng, bytes)));
    }
    row("Rich text, 50 KB + HTML + RTF", &mut |rng| {
        let text = prose(rng, 50 << 10);
        let (html, rtf) = (format!("<p>{text}</p>").into_bytes(), format!("{{\\rtf1 {text}}}").into_bytes());
        capture(Kind::Text, text.clone(), vec![(pasteboard::TEXT, text.into_bytes()), (pasteboard::HTML, html), (pasteboard::RTF, rtf)])
    });
    row("File list, 50 paths", &mut |rng| {
        let paths: Vec<String> = (0..50).map(|_| format!("/Users/someone/Documents/{}-{}.pdf", WORDS[rng.below(24) as usize], rng.below(1_000_000))).collect();
        let urls: Vec<String> = paths.iter().map(|p| format!("file://{p}")).collect();
        capture(Kind::Files, paths.join("\n"), vec![(pasteboard::FILE_URL, urls.join("\n").into_bytes())])
    });
    for (label, w, h) in [("Image, 1280 × 800", 1280, 800), ("Image, 1920 × 1080", 1920, 1080), ("Image, 3024 × 1964 (Retina screenshot)", 3024, 1964), ("Image, 5120 × 2880 (5K screenshot)", 5120, 2880)] {
        row(label, &mut |rng| image_capture(screenshot(rng, w, h)));
    }
}

/// One entry of an everyday mix: mostly short text, some long, some links, images and files.
fn mixed(rng: &mut Rng, pictures: &[Vec<u8>], i: usize) -> Capture {
    match rng.below(100) {
        0..62 => {
            let bytes = 20 + rng.below(280) as usize;
            text_capture(format!("{} #{i}", prose(rng, bytes)))
        }
        62..72 => {
            let bytes = 2_000 + rng.below(18_000) as usize;
            text_capture(format!("{} #{i}", prose(rng, bytes)))
        }
        72..80 => text_capture(format!("https://example.com/{}/{}?item={i}", WORDS[rng.below(24) as usize], WORDS[rng.below(24) as usize])),
        80..82 => text_capture(format!("#{:06x}", (i as u64 * 2_654_435_761) & 0xFF_FFFF)),
        82..84 => {
            let path = format!("/Users/someone/Documents/{}-{i}.pdf", WORDS[rng.below(24) as usize]);
            capture(Kind::Files, path.clone(), vec![(pasteboard::FILE_URL, format!("file://{path}").into_bytes())])
        }
        // A picture: one of a few screenshots with a unique trailing chunk, so each is a new entry.
        _ => {
            let mut png = pictures[rng.below(pictures.len() as u64) as usize].clone();
            png.extend_from_slice(format!("\0bench-{i}").as_bytes());
            image_capture(png)
        }
    }
}

fn runtime(entries: Vec<crate::model::Entry>) -> Runtime<Ui> {
    let app = rune::App::new(rune::AppSpec::new("bench", (1512.0, WINDOW_H), || unreachable!(), crate::view::view));
    let mut state = Ui::new(app.handle(), None, &Config::default(), true);
    state.entries = entries;
    Runtime::new(state, crate::view::view, (1512.0, WINDOW_H), Env { system_dark: true, scale: 2.0 })
}

fn key(rt: &mut Runtime<Ui>, key: Key) -> f64 {
    let start = Instant::now();
    rt.key(KeyEvent { key, mods: Mods::default() });
    rt.frame(1.0 / 120.0);
    ms(start)
}

/// How the history file, launch and the shelf's frames scale with the number of entries.
#[test]
#[ignore]
fn scale() {
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let pictures: Vec<Vec<u8>> = [(1920, 1200), (3024, 1964), (1440, 900), (2560, 1440)].iter().map(|&(w, h)| screenshot(&mut rng, w, h)).collect();
    let dir = bench_dir("scale");
    let db = Db::open(&dir).unwrap();
    let sizes = [100usize, 200, 500, 1000, 2000, 5000];

    let mut rows = Vec::new();
    let mut made = 0;
    for &size in &sizes {
        while made < size {
            db.record(&mixed(&mut rng, &pictures, made)).unwrap();
            made += 1;
        }
        let images = db.entries().unwrap().iter().filter(|e| e.kind == Kind::Image).count();
        let disk = dir_bytes(&dir);
        let thumbs = dir_bytes(&dir.join("thumbs"));

        let load = median((0..5).map(|_| { let start = Instant::now(); let n = db.entries().unwrap().len(); assert_eq!(n, size); ms(start) }).collect());
        let entries = db.entries().unwrap();
        let held: usize = entries.iter().map(|e| e.preview.len() + e.search.len() + e.detail.len() + e.app_name.len() + e.app_path.len() + std::mem::size_of_val(e)).sum();
        let record = median((0..5).map(|i| {
            let c = text_capture(format!("one more copy {size} {i} {}", prose(&mut rng, 80)));
            let start = Instant::now();
            db.record(&c).unwrap();
            db.trim(usize::MAX, u64::MAX).unwrap();
            ms(start)
        }).collect());
        db.trim(size, u64::MAX).unwrap();

        let mut rt = runtime(entries);
        let start = Instant::now();
        rt.frame(0.0);
        let first = ms(start);
        while rt.frame(1.0 / 120.0) {}
        let start = Instant::now();
        ui::setup(&mut rt.state, Arc::new(|| {}));
        let warm = ms(start);
        let warm_count = rt.state.thumbs.len();
        while rt.frame(1.0 / 120.0) {}
        let arrow = median((0..40).map(|i| key(&mut rt, if i % 2 == 0 { Key::Right } else { Key::Left })).collect());
        let rest = median((0..40).map(|_| { let start = Instant::now(); rt.frame(1.0 / 120.0); ms(start) }).collect());
        let typing = median("rune clipboard history".chars().map(|c| key(&mut rt, Key::Text(c.to_string()))).collect());
        rows.push(format!(
            "| {size} | {images} | {:.1} MB | {:.1} MB | {load:.1} ms | {:.2} MB | {record:.2} ms | {first:.1} ms | {arrow:.2} ms | {rest:.2} ms | {typing:.2} ms | {warm:.0} ms ({warm_count}) |",
            mb(disk), mb(thumbs), mb(held as u64)
        ));

        if std::env::var_os("NERDA_BENCH_DIR").is_some() {
            let keep = dir.with_file_name(format!("history-{size}"));
            let _ = std::fs::remove_dir_all(&keep);
            db.checkpoint();
            copy_dir(&dir, &keep);
        }
    }
    println!("\n| Entries | Images | On disk | Thumbnails | Load at launch | Held in memory | Record one more | First frame | Arrow key frame | Animation frame | Typing frame | Warm thumbnails |");
    println!("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |");
    for row in rows {
        println!("{row}");
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() { copy_dir(&entry.path(), &target) } else { std::fs::copy(entry.path(), target).map(|_| ()).unwrap() }
    }
}

/// What showing pictures costs: decoding thumbnails when the shelf opens or scrolls.
#[test]
#[ignore]
fn thumbnails() {
    let mut rng = Rng(0xA076_1D64_78BD_642F);
    let db = Db::open(&bench_dir("thumbnails")).unwrap();
    println!("\n| Source image | Thumbnail | File | Decoded (with mips) | Decode, image crate | Decode, Rune cache |");
    println!("| --- | --- | --- | --- | --- | --- |");
    let cache = rune::ImageCache::new(Arc::new(|| {}));
    let mut paths = Vec::new();
    for (w, h) in [(1280u32, 800u32), (3024, 1964), (5120, 2880), (800, 2400)] {
        let entry = db.record(&image_capture(screenshot(&mut rng, w, h))).unwrap();
        let path = entry.thumb.unwrap();
        let file = std::fs::metadata(&path).unwrap().len();
        let direct = median((0..7).map(|_| { let start = Instant::now(); image::open(&path).unwrap(); ms(start) }).collect());
        let thumb = image::open(&path).unwrap();
        let decoded = u64::from(thumb.width()) * u64::from(thumb.height()) * 4 * 4 / 3;
        // A distinct file each time: the cache shares a handle for content it already holds.
        let start = Instant::now();
        let image = cache.file(&path, 240.0, 2.0);
        let _ = image.wait();
        let rune = ms(start);
        println!("| {w} × {h} | {} × {} | {:.0} KB | {:.2} MB | {direct:.1} ms | {rune:.1} ms |", thumb.width(), thumb.height(), file as f64 / 1024.0, mb(decoded));
        paths.push(path);
    }
    drop(cache);
    let cache = rune::ImageCache::new(Arc::new(|| {}));
    let start = Instant::now();
    let images: Vec<_> = (0..9).map(|i| cache.file(&paths[i % paths.len()], 240.0, 2.0)).collect();
    for image in &images {
        let _ = image.wait();
    }
    println!("\nNine thumbnails asked for at once (a shelf opening cold): {:.1} ms until the last is ready.", ms(start));
}
