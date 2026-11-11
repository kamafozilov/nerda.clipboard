//! Settings, read once at launch from `config.json` in the app's data folder.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// The global shortcut that opens the shelf, as a Rune chord ("cmd+shift+v").
    pub shortcut: String,
    /// How many unpinned entries the history keeps.
    pub max_items: usize,
    /// Bundle ids whose copies are never recorded.
    pub ignored_apps: Vec<String>,
    /// "glass" (Liquid Glass on macOS 26 and later) or "popover" (frosted vibrancy).
    pub material: String,
    /// The SF Symbol shown in the menu bar; chosen from the menu's Menu Bar Icon list.
    pub menu_bar_icon: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            shortcut: "cmd+shift+v".to_owned(),
            max_items: 500,
            ignored_apps: vec![
                "com.apple.keychainaccess".to_owned(),
                "com.apple.Passwords".to_owned(),
                "com.1password.1password".to_owned(),
                "com.bitwarden.desktop".to_owned(),
            ],
            material: "glass".to_owned(),
            menu_bar_icon: "clipboard".to_owned(),
        }
    }
}

/// `~/Library/Application Support/nerda.clipboard`.
pub fn data_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    home.join("Library/Application Support/nerda.clipboard")
}

/// Reads `dir/config.json`, writing the defaults there when it's missing. A file that doesn't
/// parse is left alone and the defaults are used.
pub fn load(dir: &Path) -> Config {
    let path = dir.join("config.json");
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            eprintln!("nerda-clipboard: {} is not valid, using defaults: {e}", path.display());
            Config::default()
        }),
        Err(_) => {
            let config = Config::default();
            save(dir, &config);
            config
        }
    }
}

/// Writes `dir/config.json`, for a setting changed from the menu.
pub fn save(dir: &Path, config: &Config) {
    if std::fs::create_dir_all(dir).is_ok()
        && let Ok(text) = serde_json::to_string_pretty(config)
        && let Err(e) = std::fs::write(dir.join("config.json"), text + "\n")
    {
        eprintln!("nerda-clipboard: can't save settings: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_take_their_defaults() {
        let config: Config = serde_json::from_str(r#"{"shortcut": "alt+space"}"#).unwrap();
        assert_eq!(config.shortcut, "alt+space");
        assert_eq!(config.max_items, 500);
        assert_eq!(config.material, "glass");
    }
}
