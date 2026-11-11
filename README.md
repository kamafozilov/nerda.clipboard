# Nerda Clipboard

Clipboard history for macOS on a glass shelf. Press ⇧⌘V and the shelf rises from the bottom of
the display the pointer is on; pick an entry and it is pasted into the app you were in.

Built in Rust with [Rune](https://github.com/anomalyco/rune). Bundle id: `nerda.clipboard`.

## What it records

- Text, with its rich forms (RTF, HTML) so formatting survives a paste
- Links, shown by host
- Images (PNG, or anything AppKit can read, stored as PNG) with a thumbnail
- Files and folders copied in Finder; a single image file gets a thumbnail
- Colours (`#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb()`, `rgba()`), shown as a swatch

Copies marked private by password managers (the nspasteboard.org markers) and copies from the
apps listed in `ignored_apps` are never recorded. The same content copied again moves to the
front instead of duplicating.

## Keys

| Key | Action |
| --- | --- |
| ⇧⌘V | Open or close the shelf (global) |
| ← → | Move between cards (⌘← ⌘→ jump to the ends) |
| ↩ | Paste the selected entry |
| ⇧↩ | Paste it as plain text |
| ⌘1 … ⌘9 | Paste the first nine entries directly |
| ⌘C | Copy the entry without pasting |
| ⌘P | Pin or unpin (pinned entries are never trimmed) |
| ⌘⌫ | Delete the entry |
| ⇥ / ⇧⇥ | Switch filter: All, Text, Links, Images, Files, Pinned |
| esc | Clear the search, then close |

Typing searches. A click selects a card and a double click pastes it. The menu bar icon has
Pause Recording, Clear History, Show Data Folder and Quit.

## Build and run

```sh
scripts/bundle.sh --install     # builds dist/Nerda Clipboard.app, copies it to ~/Applications, starts it
cargo run --release             # or run the bare binary
cargo test
```

Pasting presses ⌘V for you, which macOS only allows for apps enabled under System Settings →
Privacy & Security → Accessibility. The first paste asks for it; until then the entry is put
on the pasteboard and you press ⌘V yourself. An ad hoc signed build counts as a new app after
every rebuild, so set `SIGN_IDENTITY` for `scripts/bundle.sh` to keep the permission.

## Settings

`~/Library/Application Support/nerda.clipboard/config.json`, read at launch:

```json
{
  "shortcut": "cmd+shift+v",
  "max_items": 500,
  "ignored_apps": ["com.apple.keychainaccess", "com.apple.Passwords", "com.1password.1password", "com.bitwarden.desktop"],
  "material": "glass"
}
```

`material` is `glass` (Liquid Glass on macOS 26 and later) or `popover` (frosted vibrancy).
The history lives beside it in `history.db`, with thumbnails in `thumbs/`.

## Checking the UI without a window

Rune renders headlessly with sample data; nothing touches the pasteboard or the history:

```sh
target/release/nerda-clipboard --snap shelf.png --theme dark
target/release/nerda-clipboard --snap search.png --scene search
target/release/nerda-clipboard --list-scenes
```

## Layout

- `src/main.rs`: wiring, window options, the global shortcut
- `src/ui.rs`: state, keys, show and hide
- `src/view.rs`: the panel and its cards
- `src/store.rs`: the history thread (SQLite, thumbnails, pasteboard polling)
- `src/pasteboard.rs`: `NSPasteboard`, the frontmost app, the ⌘V keystroke
- `src/status_item.rs`: the menu bar icon
- `src/model.rs`, `src/config.rs`, `src/demo.rs`: entries, settings, sample data
