//! The menu bar icon and its menu. Rune has no macOS status item yet, so this is AppKit.

use std::cell::Cell;

use block2::RcBlock;
use std::path::PathBuf;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBezierPath, NSColor, NSControlStateValueOff, NSControlStateValueOn, NSEventModifierFlags, NSImage, NSLineCapStyle,
    NSMenu, NSMenuDelegate, NSMenuItem, NSStatusBar, NSWorkspace,
};
use objc2_foundation::{MainThreadMarker, NSError, NSPoint, NSRect, NSSize, NSString, NSURL};
use rune::AppHandle;

use crate::config::Config;
use crate::ui::{self, Ui};

const TAG_PAUSE: isize = 1;
const TAG_LOGIN: isize = 2;

struct Ivars {
    app: AppHandle<Ui>,
    data_dir: PathBuf,
    paused: Cell<bool>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and the target is only used on the
    // main thread, where AppKit sends menu actions.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "NerdaClipboardMenuTarget"]
    #[ivars = Ivars]
    struct Target;

    impl Target {
        #[unsafe(method(open:))]
        fn open(&self, _sender: Option<&AnyObject>) {
            self.ivars().app.update(ui::open);
        }

        #[unsafe(method(togglePause:))]
        fn toggle_pause(&self, _sender: Option<&AnyObject>) {
            let paused = !self.ivars().paused.get();
            self.ivars().paused.set(paused);
            self.ivars().app.update(move |s, _| ui::set_paused(s, paused));
        }

        #[unsafe(method(clear:))]
        fn clear(&self, _sender: Option<&AnyObject>) {
            self.ivars().app.update(|s, _| ui::clear_history(s));
        }

        #[unsafe(method(toggleLogin:))]
        fn toggle_login(&self, _sender: Option<&AnyObject>) {
            if let Err(reason) = login_item::set(!login_item::enabled()) {
                eprintln!("nerda-clipboard: launch at login: {reason}");
            }
        }

        #[unsafe(method(openSettings:))]
        fn open_settings(&self, _sender: Option<&AnyObject>) {
            open_path(&self.ivars().data_dir.join("config.json"));
        }

        #[unsafe(method(showData:))]
        fn show_data(&self, _sender: Option<&AnyObject>) {
            open_path(&self.ivars().data_dir);
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: Option<&AnyObject>) {
            self.ivars().app.quit();
        }
    }

    unsafe impl NSObjectProtocol for Target {}

    unsafe impl NSMenuDelegate for Target {
        /// Checkmarks follow the real state each time the menu opens: Launch at Login can
        /// also be switched off in System Settings.
        #[unsafe(method(menuNeedsUpdate:))]
        fn menu_needs_update(&self, menu: &NSMenu) {
            let ivars = self.ivars();
            let check = |menu: &NSMenu, tag: isize, on: bool| {
                if let Some(item) = menu.itemWithTag(tag) {
                    item.setState(if on { NSControlStateValueOn } else { NSControlStateValueOff });
                }
            };
            check(menu, TAG_PAUSE, ivars.paused.get());
            check(menu, TAG_LOGIN, login_item::enabled());
        }
    }
);

fn open_path(path: &std::path::Path) {
    let path = NSString::from_str(&path.to_string_lossy());
    NSWorkspace::sharedWorkspace().openURL(&NSURL::fileURLWithPath(&path));
}

/// The menu bar icon: a clipboard with two more edges stacked behind it, for a history
/// rather than a single copy. Drawn as a template on an 18 pt square, the size of the
/// system's own menu bar icons, with their 1.5 pt stroke; AppKit tints it for the bar.
fn menu_bar_image() -> Retained<NSImage> {
    let draw = RcBlock::new(|_rect: NSRect| -> Bool {
        let point = |x: f64, y: f64| NSPoint::new(x, y);
        let rounded = |x: f64, y: f64, w: f64, h: f64, r: f64| {
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(NSRect::new(point(x, y), NSSize::new(w, h)), r, r)
        };
        NSColor::blackColor().set();

        let board = rounded(1.75, 3.75, 10.5, 12.5, 2.0);
        board.setLineWidth(1.5);
        board.stroke();
        rounded(4.5, 2.0, 5.0, 3.0, 1.0).fill();

        let edges = NSBezierPath::bezierPath();
        edges.moveToPoint(point(14.5, 5.5));
        edges.lineToPoint(point(14.5, 14.5));
        edges.moveToPoint(point(16.75, 7.25));
        edges.lineToPoint(point(16.75, 12.75));
        edges.setLineWidth(1.5);
        edges.setLineCapStyle(NSLineCapStyle::Round);
        edges.stroke();
        Bool::YES
    });
    // Flipped, so the coordinates read top-down like the SVG the shape was drawn in.
    let image = NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(18.0, 18.0), true, &draw);
    image.setTemplate(true);
    image.setAccessibilityDescription(Some(&NSString::from_str("Nerda Clipboard")));
    image
}

/// A Rune chord ("cmd+shift+v") as a menu key equivalent, when its key is one character.
fn key_equivalent(chord: &str) -> Option<(String, NSEventModifierFlags)> {
    let mut flags = NSEventModifierFlags::empty();
    let mut key = None;
    for part in chord.split('+').map(|p| p.trim().to_ascii_lowercase()) {
        match part.as_str() {
            "cmd" | "command" | "super" | "mod" => flags |= NSEventModifierFlags::Command,
            "shift" => flags |= NSEventModifierFlags::Shift,
            "alt" | "option" | "opt" => flags |= NSEventModifierFlags::Option,
            "ctrl" | "control" => flags |= NSEventModifierFlags::Control,
            "space" => key = Some(" ".to_owned()),
            other if other.chars().count() == 1 => key = Some(other.to_owned()),
            _ => return None,
        }
    }
    key.map(|key| (key, flags))
}

/// Adds the menu bar icon. Call once, on the main thread, after the app has started.
pub fn install(app: AppHandle<Ui>, data_dir: PathBuf, config: Config) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let status = NSStatusBar::systemStatusBar().statusItemWithLength(-1.0);
    if let Some(button) = status.button(mtm) {
        button.setImage(Some(&menu_bar_image()));
    }
    let shortcut = config.shortcut;

    let target = Target::alloc(mtm).set_ivars(Ivars { app, data_dir, paused: Cell::new(false) });
    // SAFETY: NSObject's `init` on a freshly allocated subclass instance.
    let target: Retained<Target> = unsafe { msg_send![super(target), init] };

    let item = |title: &str, action: Sel| {
        // SAFETY: the selectors are the target's own methods, and the target outlives the menu.
        let item = unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &NSString::from_str(title), Some(action), &NSString::new()) };
        unsafe { item.setTarget(Some(&target)) };
        item
    };
    let with_key = |item: &NSMenuItem, key: &str, flags: NSEventModifierFlags| {
        item.setKeyEquivalent(&NSString::from_str(key));
        item.setKeyEquivalentModifierMask(flags);
    };
    let tagged = |item: Retained<NSMenuItem>, tag: isize| {
        item.setTag(tag);
        item
    };
    let header = |title: &str| NSMenuItem::sectionHeaderWithTitle(&NSString::from_str(title), mtm);

    let menu = NSMenu::new(mtm);
    let open = item("Open Clipboard History", sel!(open:));
    if let Some((key, flags)) = key_equivalent(&shortcut) {
        with_key(&open, &key, flags);
    }
    menu.addItem(&open);

    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&header("History"));
    menu.addItem(&tagged(item("Pause Recording", sel!(togglePause:)), TAG_PAUSE));
    menu.addItem(&item("Clear Unpinned History", sel!(clear:)));

    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&header("Settings"));
    let login = tagged(item("Launch at Login", sel!(toggleLogin:)), TAG_LOGIN);
    if !login_item::available() {
        // A bare binary has no bundle for macOS to register.
        // SAFETY: clearing an action is always valid; AppKit then shows the item disabled.
        unsafe { login.setAction(None) };
        login.setToolTip(Some(&NSString::from_str("Available when run as Nerda Clipboard.app")));
    }
    menu.addItem(&login);

    menu.addItem(&item("Open Settings File", sel!(openSettings:)));
    menu.addItem(&item("Show Data Folder", sel!(showData:)));

    menu.addItem(&NSMenuItem::separatorItem(mtm));
    let quit = item("Quit Nerda Clipboard", sel!(quit:));
    with_key(&quit, "q", NSEventModifierFlags::Command);
    menu.addItem(&quit);

    menu.setDelegate(Some(ProtocolObject::from_ref(&*target)));
    status.setMenu(Some(&menu));

    // The icon lives as long as the app: AppKit holds none of these strongly.
    std::mem::forget((status, menu, target));
}

/// Launch at login through `SMAppService.mainApp` (macOS 13 and later).
mod login_item {
    use super::*;

    #[link(name = "ServiceManagement", kind = "framework")]
    unsafe extern "C" {}

    const STATUS_ENABLED: isize = 1;

    fn service() -> Option<Retained<AnyObject>> {
        let class = AnyClass::get(c"SMAppService")?;
        // SAFETY: a class property returning the shared service object.
        unsafe { msg_send![class, mainAppService] }
    }

    /// Whether this process is an app bundle macOS can register.
    pub fn available() -> bool {
        let in_bundle = std::env::current_exe().is_ok_and(|exe| exe.to_string_lossy().contains(".app/Contents/MacOS/"));
        in_bundle && service().is_some()
    }

    pub fn enabled() -> bool {
        service().is_some_and(|service| {
            // SAFETY: `status` returns the SMAppServiceStatus enum, an NSInteger.
            let status: isize = unsafe { msg_send![&*service, status] };
            status == STATUS_ENABLED
        })
    }

    pub fn set(on: bool) -> Result<(), String> {
        let service = service().ok_or("ServiceManagement is unavailable")?;
        let mut error: *mut NSError = std::ptr::null_mut();
        // SAFETY: both methods take an NSError out-pointer and return BOOL.
        let ok: bool = unsafe {
            if on { msg_send![&*service, registerAndReturnError: &mut error] } else { msg_send![&*service, unregisterAndReturnError: &mut error] }
        };
        if ok {
            return Ok(());
        }
        // SAFETY: on failure the out-pointer holds an autoreleased NSError or stays null.
        Err(unsafe { error.as_ref() }.map_or_else(|| "macOS refused the request".to_owned(), |e| e.localizedDescription().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_become_menu_key_equivalents() {
        let (key, flags) = key_equivalent("cmd+shift+v").unwrap();
        assert_eq!(key, "v");
        assert_eq!(flags, NSEventModifierFlags::Command | NSEventModifierFlags::Shift);
        assert_eq!(key_equivalent("alt+space").unwrap().0, " ");
        assert!(key_equivalent("cmd+f5").is_none());
        assert!(key_equivalent("cmd+shift").is_none());
    }
}
