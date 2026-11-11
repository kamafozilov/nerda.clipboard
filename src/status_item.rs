//! The menu bar icon and its menu. Rune has no macOS status item yet, so this is AppKit.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSControlStateValueOff, NSControlStateValueOn, NSEventModifierFlags, NSImage, NSMenu, NSMenuDelegate, NSMenuItem,
    NSStatusBar, NSStatusItem, NSWorkspace,
};
use objc2_foundation::{MainThreadMarker, NSError, NSString, NSURL};
use rune::AppHandle;

use crate::config::{self, Config};
use crate::ui::{self, Ui};

/// Menu bar icons to choose from: the name shown in the menu and its SF Symbol.
pub const ICONS: [(&str, &str); 8] = [
    ("Clipboard", "clipboard"),
    ("Filled Clipboard", "clipboard.fill"),
    ("Checklist", "list.clipboard"),
    ("Document on Clipboard", "doc.on.clipboard"),
    ("Documents", "doc.on.doc"),
    ("Paperclip", "paperclip"),
    ("Scissors", "scissors"),
    ("Tray", "tray.full"),
];

const TAG_PAUSE: isize = 1;
const TAG_LOGIN: isize = 2;
/// An icon item's tag is this plus its index in [`ICONS`].
const TAG_ICON: isize = 100;

struct Ivars {
    app: AppHandle<Ui>,
    data_dir: PathBuf,
    config: RefCell<Config>,
    paused: Cell<bool>,
    status: Retained<NSStatusItem>,
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

        #[unsafe(method(pickIcon:))]
        fn pick_icon(&self, sender: Option<&NSMenuItem>) {
            let Some((_, symbol)) = sender.and_then(|item| ICONS.get((item.tag() - TAG_ICON) as usize)) else { return };
            let ivars = self.ivars();
            ivars.config.borrow_mut().menu_bar_icon = (*symbol).to_owned();
            config::save(&ivars.data_dir, &ivars.config.borrow());
            if let Some(mtm) = MainThreadMarker::new() {
                set_icon(&ivars.status, symbol, mtm);
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
            let current = ivars.config.borrow().menu_bar_icon.clone();
            for (i, (_, symbol)) in ICONS.iter().enumerate() {
                check(menu, TAG_ICON + i as isize, *symbol == current);
            }
        }
    }
);

fn open_path(path: &std::path::Path) {
    let path = NSString::from_str(&path.to_string_lossy());
    NSWorkspace::sharedWorkspace().openURL(&NSURL::fileURLWithPath(&path));
}

fn symbol_image(symbol: &str) -> Option<Retained<NSImage>> {
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(symbol), Some(&NSString::from_str("Nerda Clipboard")))?;
    image.setTemplate(true);
    Some(image)
}

fn set_icon(status: &NSStatusItem, symbol: &str, mtm: MainThreadMarker) {
    let Some(button) = status.button(mtm) else { return };
    match symbol_image(symbol).or_else(|| symbol_image(ICONS[0].1)) {
        Some(image) => button.setImage(Some(&image)),
        None => button.setTitle(&NSString::from_str("⌘V")),
    }
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
    set_icon(&status, &config.menu_bar_icon, mtm);
    let shortcut = config.shortcut.clone();

    let target = Target::alloc(mtm).set_ivars(Ivars { app, data_dir, config: RefCell::new(config), paused: Cell::new(false), status: status.clone() });
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

    let icons = NSMenu::new(mtm);
    for (i, (name, symbol)) in ICONS.iter().enumerate() {
        let choice = tagged(item(name, sel!(pickIcon:)), TAG_ICON + i as isize);
        choice.setImage(symbol_image(symbol).as_deref());
        icons.addItem(&choice);
    }
    icons.setDelegate(Some(ProtocolObject::from_ref(&*target)));
    let icon_item = item("Menu Bar Icon", sel!(pickIcon:));
    // SAFETY: a submenu's parent item needs no action.
    unsafe { icon_item.setAction(None) };
    icon_item.setSubmenu(Some(&icons));
    menu.addItem(&icon_item);

    menu.addItem(&item("Open Settings File", sel!(openSettings:)));
    menu.addItem(&item("Show Data Folder", sel!(showData:)));

    menu.addItem(&NSMenuItem::separatorItem(mtm));
    let quit = item("Quit Nerda Clipboard", sel!(quit:));
    with_key(&quit, "q", NSEventModifierFlags::Command);
    menu.addItem(&quit);

    menu.setDelegate(Some(ProtocolObject::from_ref(&*target)));
    status.setMenu(Some(&menu));

    // The icon lives as long as the app: AppKit holds none of these strongly.
    std::mem::forget((status, menu, icons, target));
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
