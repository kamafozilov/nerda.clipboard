//! The shelf's state and behaviour: what is listed, what is selected, how keys act on it,
//! and when the panel shows and hides.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use rune::{AppHandle, DisplayTarget, Image, ImageCache, Key, KeyEvent, OverlayPlacement, SystemMaterial, TextEdit, Tx, Waker};

use crate::config::Config;
use crate::model::{Entry, Filter, unix_now};
use crate::pasteboard;
use crate::store::{Event, Op, Store};

/// The panel's height, and the window's: the panel plus room for its shadow and the margin
/// under it.
pub const PANEL_H: f32 = 312.0;
pub const MARGIN: f32 = 10.0;
pub const SHADOW_ROOM: f32 = 44.0;
pub const WINDOW_H: f32 = PANEL_H + SHADOW_ROOM + MARGIN;

/// How long the panel takes to leave before its window hides.
const CLOSE_SECONDS: f64 = 0.14;
/// Half of the caret's blink cycle, as AppKit's insertion point.
const BLINK_SECONDS: f64 = 0.53;
/// Thumbnails are kept for the cards on screen and this many either side of them.
const THUMB_MARGIN: usize = 4;
/// Cards whose thumbnails stay loaded while the shelf is hidden, so it opens complete.
const WARM_CARDS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Hidden,
    Shown,
    /// Leaving; the window hides at `due` on the app clock.
    Closing { due: f64 },
}

pub struct Ui {
    pub me: AppHandle<Ui>,
    pub store: Option<Store>,
    pub material: SystemMaterial,
    pub shortcut: String,
    /// A headless render: no native window, pasteboard or history file.
    pub demo: bool,
    pub entries: Vec<Entry>,
    pub query: TextEdit,
    pub filter: Filter,
    /// Index into [`Ui::listed`].
    pub selected: usize,
    pub phase: Phase,
    /// Unix seconds when the shelf last opened: what card ages count from.
    pub now: i64,
    pub images: Option<Rc<ImageCache>>,
    pub thumbs: HashMap<i64, Image>,
    pub icons: HashMap<String, Image>,
    pub on_screen: HashSet<i64>,
    pub notice: Option<String>,
    pub paused: bool,
    /// The search caret's blink: whether it shows, and when it next flips.
    pub caret_on: bool,
    pub blink_due: f64,
}

impl Ui {
    pub fn new(me: AppHandle<Ui>, store: Option<Store>, config: &Config, demo: bool) -> Ui {
        let mut query = TextEdit::new("");
        query.focused = true;
        Ui {
            me,
            store,
            material: if config.material.eq_ignore_ascii_case("popover") { SystemMaterial::Popover } else { SystemMaterial::Glass },
            shortcut: rune::Chord::parse(&config.shortcut).map_or_else(|_| config.shortcut.clone(), |chord| chord.label()),
            demo,
            entries: if demo { crate::demo::entries() } else { Vec::new() },
            query,
            filter: Filter::All,
            selected: 0,
            phase: if demo { Phase::Shown } else { Phase::Hidden },
            now: unix_now(),
            images: None,
            thumbs: HashMap::new(),
            icons: HashMap::new(),
            on_screen: HashSet::new(),
            notice: None,
            paused: false,
            caret_on: true,
            blink_due: 0.0,
        }
    }

    /// Indices into `entries` of what the filter and the search leave, in order.
    pub fn listed(&self) -> Vec<usize> {
        let query = self.query.text.trim().to_lowercase();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| self.filter.accepts(e) && (query.is_empty() || e.search.contains(&query) || e.app_name.to_lowercase().contains(&query)))
            .map(|(i, _)| i)
            .collect()
    }

    /// Shows the caret solid and restarts its blink: after opening and after every key.
    fn hold_caret(&mut self, tx: &mut Tx) {
        self.caret_on = true;
        self.blink_due = tx.clock() + BLINK_SECONDS;
        tx.wake_at(self.blink_due);
    }

    fn selected_entry(&self) -> Option<&Entry> {
        self.listed().get(self.selected).map(|&i| &self.entries[i])
    }

    fn send(&self, op: Op) {
        if let Some(store) = &self.store {
            store.send(op);
        }
    }

    /// Loads the thumbnails and app icons the visible cards need and drops the rest.
    pub fn sync_images(&mut self) {
        let Some(cache) = self.images.clone() else { return };
        let listed = self.listed();
        let mut wanted: HashSet<i64> = HashSet::new();
        let mut want_around = |position: usize| {
            let from = position.saturating_sub(THUMB_MARGIN);
            let to = (position + THUMB_MARGIN + 1).min(listed.len());
            wanted.extend(listed[from.min(to)..to].iter().map(|&i| self.entries[i].id));
        };
        want_around(self.selected.min(WARM_CARDS / 2));
        if self.phase != Phase::Hidden {
            want_around(self.selected);
            for (position, &i) in listed.iter().enumerate() {
                if self.on_screen.contains(&self.entries[i].id) {
                    want_around(position);
                }
            }
        }
        self.thumbs.retain(|id, _| wanted.contains(id));
        for entry in &self.entries {
            if wanted.contains(&entry.id)
                && !self.thumbs.contains_key(&entry.id)
                && let Some(path) = &entry.thumb
            {
                let image = cache.file(path, 240.0, 2.0);
                if self.demo {
                    // A headless render draws one frame: have the picture ready for it.
                    let _ = image.wait();
                }
                self.thumbs.insert(entry.id, image);
            }
            if !entry.app_path.is_empty() && !self.icons.contains_key(&entry.app_path) {
                self.icons.insert(entry.app_path.clone(), cache.app_icon(&entry.app_path, 16.0));
            }
        }
        let used: HashSet<&str> = self.entries.iter().map(|e| e.app_path.as_str()).collect();
        self.icons.retain(|path, _| used.contains(path.as_str()));
    }

    fn select(&mut self, position: usize, tx: &mut Tx) {
        let listed = self.listed();
        if listed.is_empty() {
            self.selected = 0;
            return;
        }
        self.selected = position.min(listed.len() - 1);
        tx.scroll_into_view(card_key(self.entries[listed[self.selected]].id));
        self.sync_images();
    }

    /// Back to the first card of whatever is listed, after the filter or the search changed.
    pub fn rewind(&mut self, tx: &mut Tx) {
        self.selected = 0;
        tx.scroll_to_x("strip", 0.0);
        self.sync_images();
    }
}

pub fn card_key(id: i64) -> String {
    format!("card-{id}")
}

pub fn setup(s: &mut Ui, waker: Waker) {
    s.images = Some(Rc::new(ImageCache::new(waker)));
    s.sync_images();
}

/// The global shortcut: opens the shelf on the display under the pointer, or closes it.
pub fn toggle(s: &mut Ui, tx: &mut Tx) {
    if s.phase == Phase::Shown { close(s, tx) } else { open(s, tx) }
}

pub fn open(s: &mut Ui, tx: &mut Tx) {
    s.now = unix_now();
    s.filter = Filter::All;
    s.query.clear();
    s.query.focused = true;
    s.selected = 0;
    s.on_screen.clear();
    if s.notice.is_some() && pasteboard::can_paste(false) {
        s.notice = None;
    }
    // The window spans the display's width and sits on its bottom edge, above the Dock.
    if let Some(screen) = pasteboard::pointer_screen() {
        let placement = OverlayPlacement::Rect {
            x: 0.0,
            y: screen.height - screen.bottom_inset - WINDOW_H,
            width: screen.width,
            height: WINDOW_H,
        };
        s.me.place(DisplayTarget::UnderPointer, placement);
    }
    s.me.open();
    s.phase = Phase::Shown;
    s.hold_caret(tx);
    tx.scroll_to_x("strip", 0.0);
    s.sync_images();
}

pub fn close(s: &mut Ui, tx: &mut Tx) {
    if s.phase != Phase::Shown {
        return;
    }
    let due = tx.clock() + CLOSE_SECONDS;
    s.phase = Phase::Closing { due };
    tx.wake_at(due);
}

pub fn poll(s: &mut Ui, clock: f64, tx: &mut Tx) -> bool {
    match s.phase {
        Phase::Closing { due } if clock >= due => {
            s.phase = Phase::Hidden;
            s.me.hide();
            s.on_screen.clear();
            s.selected = 0;
            s.caret_on = true;
            s.sync_images();
            false
        }
        // The caret blinks only while the shelf shows, so a hidden shelf never wakes.
        Phase::Shown if !s.demo && clock >= s.blink_due => {
            blink(s, tx);
            s.blink_due = clock + BLINK_SECONDS;
            tx.wake_at(s.blink_due);
            true
        }
        _ => false,
    }
}

/// Flips the caret. Rune lands a caret colour change in one frame today; `instant` keeps a
/// blink at one frame should that colour ever get a spring.
fn blink(s: &mut Ui, tx: &mut Tx) {
    tx.instant();
    s.caret_on = !s.caret_on;
}

/// A click outside the panel, or another app coming forward, puts the shelf away.
pub fn on_focus(s: &mut Ui, focused: bool, tx: &mut Tx) {
    if !focused && !s.demo {
        close(s, tx);
    }
}

/// An event from the history thread.
pub fn apply(s: &mut Ui, event: Event, tx: &mut Tx) {
    // The shelf is usually hidden when history changes; when it shows, nothing should slide.
    tx.instant();
    let selected_id = s.selected_entry().map(|e| e.id);
    match event {
        Event::Loaded(entries) => s.entries = entries,
        Event::Captured(entry) => {
            s.entries.retain(|e| e.id != entry.id);
            s.thumbs.remove(&entry.id);
            s.entries.insert(0, entry);
        }
        Event::Trimmed(ids) => s.entries.retain(|e| !ids.contains(&e.id)),
        Event::NeedsAccessibility => {
            s.notice = Some("Copied. Allow Nerda Clipboard in System Settings → Privacy & Security → Accessibility to paste directly.".to_owned());
        }
    }
    // Keep the selection on the same entry while the shelf is open.
    let listed = s.listed();
    s.selected = match selected_id.filter(|_| s.phase == Phase::Shown) {
        Some(id) => listed.iter().position(|&i| s.entries[i].id == id).unwrap_or(0),
        None => 0,
    };
    s.sync_images();
}

/// Pastes (or with `keystroke` off, only copies) the entry at `position` and closes.
fn activate(s: &mut Ui, position: usize, plain: bool, keystroke: bool, tx: &mut Tx) {
    let Some(&i) = s.listed().get(position) else { return };
    let id = s.entries[i].id;
    s.send(Op::Paste { id, plain, keystroke });
    close(s, tx);
}

pub fn paste_card(s: &mut Ui, id: i64, tx: &mut Tx) {
    if let Some(position) = s.listed().iter().position(|&i| s.entries[i].id == id) {
        activate(s, position, false, true, tx);
    }
}

pub fn select_card(s: &mut Ui, id: i64, tx: &mut Tx) {
    if let Some(position) = s.listed().iter().position(|&i| s.entries[i].id == id) {
        s.select(position, tx);
    }
}

fn delete_selected(s: &mut Ui, tx: &mut Tx) {
    let Some(id) = s.selected_entry().map(|e| e.id) else { return };
    s.send(Op::Delete(id));
    s.entries.retain(|e| e.id != id);
    s.thumbs.remove(&id);
    s.select(s.selected, tx);
}

fn pin_selected(s: &mut Ui) {
    let Some(&i) = s.listed().get(s.selected) else { return };
    let entry = &mut s.entries[i];
    entry.pinned = !entry.pinned;
    let (id, pinned) = (entry.id, entry.pinned);
    s.send(Op::Pin(id, pinned));
    // Unpinning inside the Pinned filter removes the card under the selection.
    s.selected = s.selected.min(s.listed().len().saturating_sub(1));
}

pub fn set_filter(s: &mut Ui, filter: Filter, tx: &mut Tx) {
    if s.filter != filter {
        s.filter = filter;
        s.rewind(tx);
    }
}

pub fn set_paused(s: &mut Ui, paused: bool) {
    s.paused = paused;
    s.send(Op::Pause(paused));
}

pub fn clear_history(s: &mut Ui) {
    s.send(Op::Clear);
}

/// Keys, seen before the search field: arrows move between cards, the rest act on the
/// selected one. Anything not taken types into the search.
pub fn on_key(s: &mut Ui, ev: &KeyEvent, tx: &mut Tx) -> bool {
    s.hold_caret(tx);
    let count = s.listed().len();
    let last = count.saturating_sub(1);
    match &ev.key {
        Key::Left if ev.mods.cmd => s.select(0, tx),
        Key::Right if ev.mods.cmd => s.select(last, tx),
        Key::Left => s.select(s.selected.saturating_sub(1), tx),
        Key::Right => s.select(s.selected + 1, tx),
        Key::Home => s.select(0, tx),
        Key::End => s.select(last, tx),
        Key::Enter => activate(s, s.selected, ev.mods.shift || ev.mods.alt, true, tx),
        Key::Escape if !s.query.text.is_empty() => {
            s.query.clear();
            s.rewind(tx);
        }
        Key::Escape => close(s, tx),
        Key::Tab => {
            let filter = s.filter.step(if ev.mods.shift { -1 } else { 1 });
            set_filter(s, filter, tx);
        }
        Key::Backspace if ev.mods.cmd => delete_selected(s, tx),
        Key::Delete => delete_selected(s, tx),
        Key::Command('c') => activate(s, s.selected, false, false, tx),
        Key::Command('p') => pin_selected(s),
        Key::Command('q') => tx.quit(),
        Key::Command(c @ '1'..='9') => {
            let position = (*c as u8 - b'1') as usize;
            if position < count {
                activate(s, position, ev.mods.shift || ev.mods.alt, true, tx);
            }
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Kind;
    use rune::{Env, Mods, Runtime};

    fn runtime() -> Runtime<Ui> {
        let app = rune::App::new(rune::AppSpec::new("test", (1280.0, WINDOW_H), || unreachable!(), crate::view::view));
        let ui = Ui::new(app.handle(), None, &Config::default(), true);
        let mut rt = Runtime::new(ui, crate::view::view, (1280.0, WINDOW_H), Env { system_dark: true, scale: 2.0 });
        rt.frame(0.0);
        rt
    }

    fn key(rt: &mut Runtime<Ui>, key: Key, mods: Mods) {
        rt.key(KeyEvent { key, mods });
        rt.frame(1.0 / 60.0);
    }

    #[test]
    fn arrows_move_the_selection_and_stop_at_the_ends() {
        let mut rt = runtime();
        key(&mut rt, Key::Left, Mods::default());
        assert_eq!(rt.state.selected, 0);
        key(&mut rt, Key::Right, Mods::default());
        key(&mut rt, Key::Right, Mods::default());
        assert_eq!(rt.state.selected, 2);
        key(&mut rt, Key::End, Mods::default());
        assert_eq!(rt.state.selected, rt.state.entries.len() - 1);
        key(&mut rt, Key::Right, Mods::default());
        assert_eq!(rt.state.selected, rt.state.entries.len() - 1);
    }

    #[test]
    fn typing_searches_and_escape_clears_before_it_closes() {
        let mut rt = runtime();
        for c in "rune".chars() {
            key(&mut rt, Key::Text(c.to_string()), Mods::default());
        }
        assert_eq!(rt.state.query.text, "rune");
        let listed = rt.state.listed();
        assert!(!listed.is_empty() && listed.len() < rt.state.entries.len());
        assert!(listed.iter().all(|&i| rt.state.entries[i].search.contains("rune")));
        key(&mut rt, Key::Escape, Mods::default());
        assert_eq!(rt.state.query.text, "");
        assert_eq!(rt.state.phase, Phase::Shown);
        key(&mut rt, Key::Escape, Mods::default());
        assert!(matches!(rt.state.phase, Phase::Closing { .. }));
    }

    #[test]
    fn tab_cycles_filters_and_each_lists_only_its_kind() {
        let mut rt = runtime();
        key(&mut rt, Key::Tab, Mods::default());
        key(&mut rt, Key::Tab, Mods::default());
        assert_eq!(rt.state.filter, Filter::Links);
        assert!(rt.state.listed().iter().all(|&i| rt.state.entries[i].kind == Kind::Link));
        key(&mut rt, Key::Tab, Mods { shift: true, ..Mods::default() });
        assert_eq!(rt.state.filter, Filter::Text);
    }

    #[test]
    fn pinning_and_deleting_act_on_the_selected_card() {
        let mut rt = runtime();
        key(&mut rt, Key::Right, Mods::default());
        let id = rt.state.selected_entry().unwrap().id;
        let was_pinned = rt.state.selected_entry().unwrap().pinned;
        key(&mut rt, Key::Command('p'), Mods { cmd: true, ..Mods::default() });
        assert_eq!(rt.state.entries.iter().find(|e| e.id == id).unwrap().pinned, !was_pinned);
        let before = rt.state.entries.len();
        key(&mut rt, Key::Delete, Mods::default());
        assert_eq!(rt.state.entries.len(), before - 1);
        assert!(rt.state.entries.iter().all(|e| e.id != id));
    }

    #[test]
    fn a_new_copy_goes_first_and_a_repeat_does_not_duplicate() {
        let mut rt = runtime();
        let mut entry = rt.state.entries[3].clone();
        let before = rt.state.entries.len();
        entry.used += 100;
        let id = entry.id;
        rt.update(move |s, tx| apply(s, Event::Captured(entry), tx));
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.entries.len(), before);
        assert_eq!(rt.state.entries[0].id, id);
    }

    /// Not a check: prints what one caret blink costs. `cargo test --release -- --ignored --nocapture blink_cost`
    #[test]
    #[ignore]
    fn blink_cost() {
        for count in [13usize, 100, 500] {
            let mut rt = runtime();
            let base = rt.state.entries.clone();
            rt.state.entries = (0..count).map(|i| { let mut e = base[i % base.len()].clone(); e.id = i as i64 + 1; e.thumb = None; e }).collect();
            for _ in 0..30 { rt.frame(1.0 / 60.0); }
            let rounds = 200;
            let start = std::time::Instant::now();
            for _ in 0..rounds {
                rt.update(blink);
                rt.frame(1.0 / 60.0);
            }
            let blink = start.elapsed().as_secs_f64() * 1000.0 / rounds as f64;
            let start = std::time::Instant::now();
            for _ in 0..rounds { rt.frame(1.0 / 60.0); }
            let idle = start.elapsed().as_secs_f64() * 1000.0 / rounds as f64;
            println!("{count} entries: blink frame {blink:.3} ms, plain frame {idle:.3} ms");
        }
    }

    /// Frames drawn until the window rests again after `change`.
    fn frames_to_rest(rt: &mut Runtime<Ui>, change: fn(&mut Ui, &mut Tx)) -> usize {
        rt.update(change);
        (1..200).find(|_| !rt.frame(1.0 / 120.0)).unwrap_or(200)
    }

    #[test]
    fn a_blink_costs_one_frame() {
        let mut rt = runtime();
        while rt.frame(1.0 / 120.0) {}
        let plain = frames_to_rest(&mut rt, |s, _| s.caret_on = !s.caret_on);
        let instant = frames_to_rest(&mut rt, blink);
        // The measure is real: a change that does animate (the selection's colours) takes many.
        let animated = frames_to_rest(&mut rt, |s, _| s.selected = 2);
        println!("plain blink: {plain} frames, instant blink: {instant} frames, selection change: {animated} frames");
        assert!(animated > 2, "frames_to_rest sees animation: {animated}");
        assert!(instant <= 2, "an instant blink draws {instant} frames");
        assert!(instant <= plain);
    }
}
