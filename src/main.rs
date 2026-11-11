//! Nerda Clipboard: clipboard history for macOS on a glass shelf that rises from the bottom of
//! the display under the pointer. Built with Rune (github.com/anomalyco/rune).
//!
//! ```sh
//! cargo run --release                 # menu bar app; ⇧⌘V opens the shelf
//! cargo run --release -- --show       # also open the shelf at launch
//! cargo run --release -- --snap shelf.png [--theme light]   # headless render with sample data
//! ```

#[cfg(not(target_os = "macos"))]
compile_error!("Nerda Clipboard is a macOS app.");

mod alloc;
#[cfg(test)]
mod bench;
mod config;
mod demo;
mod model;
mod pasteboard;
mod status_item;
mod store;
mod ui;
mod view;

use rune::{App, AppSpec, RunOptions, WindowOptions, WindowSpec};

use crate::model::Filter;
use crate::ui::Ui;

/// Rune's headless tooling flags: with one of them the app renders sample data and touches
/// neither the pasteboard nor the history.
const HEADLESS: [&str; 12] = [
    "--snap", "--tree", "--trace", "--script", "--video", "--frames", "--strip", "--out", "--first-pixels",
    "--list-scenes", "--list-windows", "--desktop",
];

#[global_allocator]
static ALLOCATOR: alloc::Returning = alloc::Returning;

/// Whether `NERDA_CLIPBOARD_DEBUG` is set.
fn debugging() -> bool {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("NERDA_CLIPBOARD_DEBUG").is_some());
    *ON
}

/// Prints a diagnostic line when `NERDA_CLIPBOARD_DEBUG` is set.
fn debug(message: impl FnOnce() -> String) {
    if debugging() {
        eprintln!("nerda-clipboard: {}", message());
    }
}

fn main() {
    rune::prewarm_gpu();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let show_at_launch = args.iter().any(|a| a == "--show");
    args.retain(|a| a != "--show");
    let headless = args.iter().any(|a| HEADLESS.contains(&a.as_str()));

    let data_dir = config::data_dir();
    let config = if headless { config::Config::default() } else { config::load(&data_dir) };

    let spec = AppSpec::<Ui> {
        // The system font, as AppKit draws it; Inter where it can't be found.
        families: (".AppleSystemUIFont, SF Pro Text, SF Pro, Helvetica Neue, Inter", "ui-monospace"),
        setup: Some(ui::setup),
        poll: Some(ui::poll),
        scenes: &[
            ("search", |s| {
                s.query.replace_text("rune");
            }),
            ("images", |s| s.filter = Filter::Images),
            ("selected", |s| s.selected = 3),
            ("empty", |s| s.entries.clear()),
        ],
        ..AppSpec::new("Nerda Clipboard", (1280.0, ui::WINDOW_H), || unreachable!(), view::view)
    };

    let app = App::new(spec).windowless();
    let me = app.handle();
    let store = (!headless).then(|| {
        let ui = me.clone();
        store::spawn(data_dir.clone(), config.clone(), move |event| ui.update(move |s, tx| ui::apply(s, event, tx)))
    });

    let (state_handle, ready_handle) = (me.clone(), me);
    let (state_config, shortcut) = (config.clone(), config.shortcut.clone());
    let menu_config = config.clone();
    let mut installed = false;
    let app = app
        .init(move || Ui::new(state_handle, store, &state_config, headless))
        .global_shortcut(&shortcut, ui::toggle)
        .on_focus(ui::on_focus)
        // After the first (hidden) frame: the menu bar icon, and the shelf itself with --show.
        .on_ready(move |_| {
            if installed || headless {
                return;
            }
            installed = true;
            status_item::install(ready_handle.clone(), data_dir.clone(), menu_config.clone());
            if show_at_launch {
                ready_handle.update(ui::open);
            }
        });

    let panel = WindowOptions::launcher_panel().macos(|native| native.shadow(false));
    rune::run_windows_with_args(RunOptions { accessory: true }, args, move |_| vec![WindowSpec::from_app("shelf", app).configured_options(panel)]);
}
