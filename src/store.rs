//! The history on disk (SQLite plus thumbnail files) and the one thread that owns it: it
//! watches the pasteboard, records copies, and writes entries back for a paste. Nothing here
//! runs on the UI thread.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};

use crate::config::Config;
use crate::model::{Entry, Kind, PREVIEW_CHARS, SEARCH_CHARS, bytes_label, clip};
use crate::pasteboard::{self, Capture};

/// How often the pasteboard's change count is checked; macOS has no change notification.
const POLL: Duration = Duration::from_millis(350);
/// From writing the pasteboard to pressing ⌘V: long enough for the shelf to hide and the app
/// behind it to take the keyboard back.
const PASTE_DELAY: Duration = Duration::from_millis(240);
/// The longest side of a stored thumbnail, in pixels: a card at 2×, with room to spare.
const THUMB_SIDE: u32 = 640;
const THUMB_FILE_LIMIT: u64 = 40 << 20;

pub enum Op {
    /// Put the entry back on the pasteboard, and press ⌘V when `keystroke`.
    Paste { id: i64, plain: bool, keystroke: bool },
    Delete(i64),
    Pin(i64, bool),
    /// Forget everything that isn't pinned.
    Clear,
    Pause(bool),
}

pub enum Event {
    /// The whole history, newest first: at launch and after a clear.
    Loaded(Vec<Entry>),
    /// A new copy, or an old entry copied or used again: it goes to the front.
    Captured(Entry),
    /// Entries the size limit dropped.
    Trimmed(Vec<i64>),
    /// A paste was asked for but macOS hasn't granted Accessibility; the entry is on the
    /// pasteboard for a manual ⌘V.
    NeedsAccessibility,
}

#[derive(Clone)]
pub struct Store(Sender<Op>);

impl Store {
    pub fn send(&self, op: Op) {
        let _ = self.0.send(op);
    }
}

/// Opens the history in `dir` and starts the thread. `sink` receives events on that thread.
pub fn spawn(dir: PathBuf, config: Config, sink: impl Fn(Event) + Send + 'static) -> Store {
    let (tx, rx) = mpsc::channel::<Op>();
    std::thread::Builder::new()
        .name("history".to_owned())
        .spawn(move || {
            let db = match Db::open(&dir) {
                Ok(db) => db,
                Err(e) => {
                    eprintln!("nerda-clipboard: can't open the history in {}: {e}", dir.display());
                    return;
                }
            };
            sink(Event::Loaded(db.entries().unwrap_or_default()));
            // What was copied before launch isn't recorded.
            let mut seen = pasteboard::change_count();
            let mut paused = false;
            loop {
                match rx.recv_timeout(POLL) {
                    Ok(op) => {
                        if let Err(e) = apply(&db, op, &mut seen, &mut paused, &sink) {
                            eprintln!("nerda-clipboard: history: {e}");
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
                let count = pasteboard::change_count();
                if count == seen {
                    continue;
                }
                seen = count;
                if paused {
                    continue;
                }
                let Some(capture) = pasteboard::read() else { continue };
                if config.ignored_apps.iter().any(|id| id.eq_ignore_ascii_case(&capture.bundle_id)) {
                    continue;
                }
                match db.record(&capture) {
                    Ok(entry) => {
                        sink(Event::Captured(entry));
                        match db.trim(config.max_items) {
                            Ok(gone) if !gone.is_empty() => sink(Event::Trimmed(gone)),
                            Ok(_) => {}
                            Err(e) => eprintln!("nerda-clipboard: history: {e}"),
                        }
                    }
                    Err(e) => eprintln!("nerda-clipboard: history: {e}"),
                }
            }
        })
        .expect("the history thread starts");
    Store(tx)
}

fn apply(db: &Db, op: Op, seen: &mut isize, paused: &mut bool, sink: &impl Fn(Event)) -> rusqlite::Result<()> {
    match op {
        Op::Paste { id, plain, keystroke } => {
            let mut reps = db.reps(id)?;
            if plain && reps.iter().any(|(uti, _)| uti == pasteboard::TEXT) {
                reps.retain(|(uti, _)| uti == pasteboard::TEXT);
            }
            if reps.is_empty() {
                return Ok(());
            }
            *seen = pasteboard::write(&reps);
            if let Some(entry) = db.touch(id)? {
                sink(Event::Captured(entry));
            }
            if keystroke {
                if pasteboard::can_paste(false) {
                    std::thread::sleep(PASTE_DELAY);
                    pasteboard::send_paste();
                } else {
                    pasteboard::can_paste(true);
                    sink(Event::NeedsAccessibility);
                }
            }
        }
        Op::Delete(id) => db.delete(id)?,
        Op::Pin(id, pinned) => db.pin(id, pinned)?,
        Op::Clear => {
            db.clear()?;
            sink(Event::Loaded(db.entries()?));
        }
        Op::Pause(on) => {
            *paused = on;
            // Copies made while paused stay unrecorded after it ends.
            *seen = pasteboard::change_count();
        }
    }
    Ok(())
}

/// `used` is stored in milliseconds, so copies within one second keep their order.
fn unix_millis() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

struct Db {
    conn: Connection,
    thumbs: PathBuf,
}

const COLUMNS: &str = "id, kind, preview, search, detail, app_name, app_path, used, pinned, color, has_thumb";

impl Db {
    fn open(dir: &Path) -> Result<Db, Box<dyn std::error::Error>> {
        let thumbs = dir.join("thumbs");
        std::fs::create_dir_all(&thumbs)?;
        let conn = Connection::open(dir.join("history.db"))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS items(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL,
                hash BLOB NOT NULL UNIQUE,
                preview TEXT NOT NULL,
                search TEXT NOT NULL,
                detail TEXT NOT NULL,
                app_name TEXT NOT NULL,
                app_path TEXT NOT NULL,
                used INTEGER NOT NULL,
                pinned INTEGER NOT NULL DEFAULT 0,
                color INTEGER,
                has_thumb INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS items_used ON items(used DESC);
            CREATE TABLE IF NOT EXISTS reps(
                item_id INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
                uti TEXT NOT NULL,
                data BLOB NOT NULL,
                PRIMARY KEY(item_id, uti)
            );",
        )?;
        Ok(Db { conn, thumbs })
    }

    fn thumb_path(&self, id: i64) -> PathBuf {
        self.thumbs.join(format!("{id}.png"))
    }

    fn entry(&self, row: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
        let id: i64 = row.get(0)?;
        let kind: String = row.get(1)?;
        let color: Option<i64> = row.get(9)?;
        let has_thumb: bool = row.get(10)?;
        Ok(Entry {
            id,
            kind: Kind::parse(&kind),
            preview: row.get(2)?,
            search: row.get(3)?,
            detail: row.get(4)?,
            app_name: row.get(5)?,
            app_path: row.get(6)?,
            used: row.get::<_, i64>(7)? / 1000,
            pinned: row.get(8)?,
            thumb: has_thumb.then(|| self.thumb_path(id)),
            color: color.map(|c| (c as u32).to_be_bytes()),
        })
    }

    fn entries(&self) -> rusqlite::Result<Vec<Entry>> {
        let mut query = self.conn.prepare(&format!("SELECT {COLUMNS} FROM items ORDER BY used DESC, id DESC"))?;
        query.query_map([], |row| self.entry(row))?.collect()
    }

    fn by_id(&self, id: i64) -> rusqlite::Result<Option<Entry>> {
        self.conn.query_row(&format!("SELECT {COLUMNS} FROM items WHERE id = ?1"), [id], |row| self.entry(row)).optional()
    }

    /// Marks an entry used now, which moves it to the front.
    fn touch(&self, id: i64) -> rusqlite::Result<Option<Entry>> {
        self.conn.execute("UPDATE items SET used = ?2 WHERE id = ?1", params![id, unix_millis()])?;
        self.by_id(id)
    }

    /// Stores a copy. The same content copied again moves its entry to the front instead.
    fn record(&self, capture: &Capture) -> rusqlite::Result<Entry> {
        let main = capture.reps.first().map_or(&[][..], |(_, data)| data.as_slice());
        let hash = blake3::hash(main);
        let known: Option<i64> = self.conn.query_row("SELECT id FROM items WHERE hash = ?1", [hash.as_bytes().as_slice()], |row| row.get(0)).optional()?;
        if let Some(id) = known {
            self.conn.execute(
                "UPDATE items SET used = ?2, app_name = ?3, app_path = ?4 WHERE id = ?1",
                params![id, unix_millis(), capture.app_name, capture.app_path],
            )?;
            return self.by_id(id)?.ok_or(rusqlite::Error::QueryReturnedNoRows);
        }

        let summary = summarize(capture);
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO items(kind, hash, preview, search, detail, app_name, app_path, used, color) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                capture.kind.as_str(),
                hash.as_bytes().as_slice(),
                summary.preview,
                summary.search,
                summary.detail,
                capture.app_name,
                capture.app_path,
                unix_millis(),
                capture.color.map(|c| i64::from(u32::from_be_bytes(c))),
            ],
        )?;
        let id = tx.last_insert_rowid();
        for (uti, data) in &capture.reps {
            tx.execute("INSERT INTO reps(item_id, uti, data) VALUES (?1, ?2, ?3)", params![id, uti, data])?;
        }
        tx.commit()?;

        if let Some(thumb) = summary.thumb
            && thumb.save(self.thumb_path(id)).is_ok()
        {
            self.conn.execute("UPDATE items SET has_thumb = 1 WHERE id = ?1", [id])?;
        }
        self.by_id(id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
    }

    fn reps(&self, id: i64) -> rusqlite::Result<Vec<(String, Vec<u8>)>> {
        // Plain text first: it's written with `setString`, the rest as data.
        let mut query = self.conn.prepare("SELECT uti, data FROM reps WHERE item_id = ?1 ORDER BY uti = ?2 DESC")?;
        query.query_map(params![id, pasteboard::TEXT], |row| Ok((row.get(0)?, row.get(1)?)))?.collect()
    }

    fn delete(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM items WHERE id = ?1", [id])?;
        let _ = std::fs::remove_file(self.thumb_path(id));
        Ok(())
    }

    fn pin(&self, id: i64, pinned: bool) -> rusqlite::Result<()> {
        self.conn.execute("UPDATE items SET pinned = ?2 WHERE id = ?1", params![id, pinned])?;
        Ok(())
    }

    fn clear(&self) -> rusqlite::Result<()> {
        self.drop_where("pinned = 0", [])?;
        self.conn.execute_batch("VACUUM")
    }

    /// Drops the oldest unpinned entries beyond `max` and returns their ids.
    fn trim(&self, max: usize) -> rusqlite::Result<Vec<i64>> {
        self.drop_where(
            "pinned = 0 AND id NOT IN (SELECT id FROM items WHERE pinned = 0 ORDER BY used DESC, id DESC LIMIT ?1)",
            [max as i64],
        )
    }

    fn drop_where<P: rusqlite::Params + Clone>(&self, condition: &str, params: P) -> rusqlite::Result<Vec<i64>> {
        let ids: Vec<i64> = {
            let mut query = self.conn.prepare(&format!("SELECT id FROM items WHERE {condition}"))?;
            query.query_map(params.clone(), |row| row.get(0))?.collect::<rusqlite::Result<_>>()?
        };
        if !ids.is_empty() {
            self.conn.execute(&format!("DELETE FROM items WHERE {condition}"), params)?;
            for id in &ids {
                let _ = std::fs::remove_file(self.thumb_path(*id));
            }
        }
        Ok(ids)
    }
}

struct Summary {
    preview: String,
    search: String,
    detail: String,
    thumb: Option<image::DynamicImage>,
}

/// What a card shows for a capture, and what a search matches.
fn summarize(capture: &Capture) -> Summary {
    let text = capture.text.as_str();
    let plain = |detail: String| Summary {
        preview: clip(text.trim(), PREVIEW_CHARS).to_owned(),
        search: clip(text, SEARCH_CHARS).to_lowercase(),
        detail,
        thumb: None,
    };
    match capture.kind {
        Kind::Text => {
            let (chars, lines) = (text.chars().count(), text.lines().count());
            plain(if lines > 1 { format!("{chars} characters · {lines} lines") } else { format!("{chars} characters") })
        }
        Kind::Link => plain("Link".to_owned()),
        Kind::Color => {
            let [r, g, b, a] = capture.color.unwrap_or_default();
            plain(if a == 255 { format!("rgb({r}, {g}, {b})") } else { format!("rgba({r}, {g}, {b}, {:.2})", f32::from(a) / 255.0) })
        }
        Kind::Files => {
            let paths: Vec<&str> = text.lines().collect();
            let names: Vec<&str> = paths.iter().map(|p| p.trim_end_matches('/').rsplit('/').next().unwrap_or(p)).collect();
            let size = |p: &str| std::fs::metadata(p).ok().filter(|m| m.is_file()).map(|m| m.len());
            let detail = match paths.as_slice() {
                [one] => size(one).map_or_else(|| "Folder".to_owned(), bytes_label),
                many => format!("{} items", many.len()),
            };
            let thumb = match paths.as_slice() {
                [one] if is_image_path(one) && size(one).is_some_and(|n| n <= THUMB_FILE_LIMIT) => image::open(one).ok().map(thumbnail),
                _ => None,
            };
            Summary { preview: clip(&names.join("\n"), PREVIEW_CHARS).to_owned(), search: clip(text, SEARCH_CHARS).to_lowercase(), detail, thumb }
        }
        Kind::Image => {
            let png = capture.reps.first().map_or(&[][..], |(_, data)| data.as_slice());
            let decoded = image::load_from_memory(png).ok();
            let detail = match &decoded {
                Some(image) => format!("{} × {} · {}", image.width(), image.height(), bytes_label(png.len() as u64)),
                None => bytes_label(png.len() as u64),
            };
            Summary { preview: "Image".to_owned(), search: "image".to_owned(), detail, thumb: decoded.map(thumbnail) }
        }
    }
}

fn thumbnail(image: image::DynamicImage) -> image::DynamicImage {
    if image.width().max(image.height()) <= THUMB_SIDE { image } else { image.thumbnail(THUMB_SIDE, THUMB_SIDE) }
}

fn is_image_path(path: &str) -> bool {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture(kind: Kind, text: &str, reps: Vec<(&str, &[u8])>) -> Capture {
        Capture {
            kind,
            text: text.to_owned(),
            color: None,
            reps: reps.into_iter().map(|(uti, data)| (uti.to_owned(), data.to_vec())).collect(),
            app_name: "Tests".to_owned(),
            app_path: String::new(),
            bundle_id: String::new(),
        }
    }

    fn text(s: &str) -> Capture {
        capture(Kind::Text, s, vec![(pasteboard::TEXT, s.as_bytes())])
    }

    fn temp_db(name: &str) -> (Db, PathBuf) {
        let dir = std::env::temp_dir().join(format!("nerda-clipboard-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        (Db::open(&dir).unwrap(), dir)
    }

    #[test]
    fn a_repeated_copy_moves_to_the_front_instead_of_duplicating() {
        let (db, dir) = temp_db("repeat");
        let first = db.record(&text("one")).unwrap();
        db.record(&text("two")).unwrap();
        db.conn.execute("UPDATE items SET used = used - 10 WHERE id = ?1", [first.id]).unwrap();
        std::thread::sleep(Duration::from_millis(2));
        let again = db.record(&text("one")).unwrap();
        assert_eq!(again.id, first.id);
        let entries = db.entries().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].preview, "one");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn trimming_keeps_pinned_entries_and_the_newest() {
        let (db, dir) = temp_db("trim");
        let ids: Vec<i64> = (0..5)
            .map(|i| {
                let id = db.record(&text(&format!("entry {i}"))).unwrap().id;
                db.conn.execute("UPDATE items SET used = ?2 WHERE id = ?1", params![id, i]).unwrap();
                id
            })
            .collect();
        db.pin(ids[0], true).unwrap();
        let gone = db.trim(2).unwrap();
        assert_eq!(gone.len(), 2);
        let left: Vec<i64> = db.entries().unwrap().iter().map(|e| e.id).collect();
        assert_eq!(left, vec![ids[4], ids[3], ids[0]]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn representations_come_back_with_plain_text_first() {
        let (db, dir) = temp_db("reps");
        let rich = capture(Kind::Text, "bold", vec![(pasteboard::TEXT, b"bold"), (pasteboard::HTML, b"<b>bold</b>"), (pasteboard::RTF, b"{\\rtf1 bold}")]);
        let id = db.record(&rich).unwrap().id;
        let reps = db.reps(id).unwrap();
        assert_eq!(reps.len(), 3);
        assert_eq!(reps[0].0, pasteboard::TEXT);
        db.delete(id).unwrap();
        assert!(db.reps(id).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn summaries_describe_each_kind() {
        let s = summarize(&text("line one\nline two"));
        assert_eq!(s.detail, "17 characters · 2 lines");
        assert_eq!(s.search, "line one\nline two");
        let files = summarize(&capture(Kind::Files, "/tmp/a/report.pdf\n/tmp/b/notes", vec![]));
        assert_eq!(files.preview, "report.pdf\nnotes");
        assert_eq!(files.detail, "2 items");
    }

    #[test]
    fn an_image_gets_its_size_and_a_thumbnail() {
        let (db, dir) = temp_db("image");
        let mut png = Vec::new();
        image::DynamicImage::new_rgba8(1600, 400).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let entry = db.record(&capture(Kind::Image, "", vec![(pasteboard::PNG, &png)])).unwrap();
        assert!(entry.detail.starts_with("1600 × 400"));
        let thumb = image::open(entry.thumb.unwrap()).unwrap();
        assert_eq!((thumb.width(), thumb.height()), (640, 160));
        let _ = std::fs::remove_dir_all(dir);
    }
}
