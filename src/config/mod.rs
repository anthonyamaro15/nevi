//! Configuration system for nevi
//!
//! Loads settings from ~/.config/nevi/config.toml

pub mod keymap;

use serde::Deserialize;
use std::path::PathBuf;

pub use keymap::{KeymapLookup, LeaderAction};

/// Main settings structure
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub editor: EditorSettings,
    pub theme: ThemeSettings,
    pub keymap: KeymapSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            editor: EditorSettings::default(),
            theme: ThemeSettings::default(),
            keymap: KeymapSettings::default(),
        }
    }
}

/// Editor behavior settings
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct EditorSettings {
    /// Number of spaces per tab (default: 4)
    pub tab_width: usize,
    /// Show line numbers (default: true)
    pub line_numbers: bool,
    /// Show relative line numbers (default: false)
    pub relative_numbers: bool,
    /// Highlight current line (default: false)
    pub cursor_line: bool,
    /// Lines to keep visible above/below cursor (default: 0)
    pub scroll_off: usize,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            tab_width: 4,
            line_numbers: true,
            relative_numbers: false,
            cursor_line: false,
            scroll_off: 0,
        }
    }
}

/// Theme settings
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ThemeSettings {
    /// Color scheme name (default: "onedark")
    pub colorscheme: String,
}

impl Default for ThemeSettings {
    fn default() -> Self {
        Self {
            colorscheme: "onedark".to_string(),
        }
    }
}

/// Keymap customization settings
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct KeymapSettings {
    /// Leader key (default: "\")
    pub leader: String,
    /// Normal mode key remappings
    pub normal: Vec<KeymapEntry>,
    /// Insert mode key remappings
    pub insert: Vec<KeymapEntry>,
    /// Leader key mappings (e.g., <leader>w -> :w<CR>)
    pub leader_mappings: Vec<LeaderMapping>,
}

impl Default for KeymapSettings {
    fn default() -> Self {
        Self {
            leader: "\\".to_string(),
            normal: Vec::new(),
            insert: Vec::new(),
            leader_mappings: Vec::new(),
        }
    }
}

/// A single keymap entry
#[derive(Debug, Clone, Deserialize)]
pub struct KeymapEntry {
    /// Key notation to remap from (e.g., "H", "<C-s>", ";")
    pub from: String,
    /// Key notation to remap to (e.g., "^", ":w<CR>", ":")
    pub to: String,
}

/// A leader key mapping
#[derive(Debug, Clone, Deserialize)]
pub struct LeaderMapping {
    /// Key sequence after leader (e.g., "w", "wa", "q")
    pub key: String,
    /// Action to execute (e.g., ":w<CR>", ":wa<CR>", ":q<CR>")
    pub action: String,
    /// Optional description for which-key style display
    #[serde(default)]
    pub desc: Option<String>,
}

/// Get the path to the config file
/// Checks ~/.config/nevi/config.toml first (XDG-style), then falls back to platform default
pub fn config_path() -> Option<PathBuf> {
    // First, try XDG-style path (~/.config/nevi/config.toml)
    if let Some(home) = dirs::home_dir() {
        let xdg_path = home.join(".config/nevi/config.toml");
        if xdg_path.exists() {
            return Some(xdg_path);
        }
    }

    // Fall back to platform-specific config dir
    dirs::config_dir().map(|p| p.join("nevi/config.toml"))
}

/// Load settings from the config file
/// Returns default settings if the file doesn't exist or can't be parsed
pub fn load_config() -> Settings {
    let Some(path) = config_path() else {
        return Settings::default();
    };

    if !path.exists() {
        return Settings::default();
    }

    match std::fs::read_to_string(&path) {
        Ok(content) => match toml::from_str(&content) {
            Ok(settings) => settings,
            Err(e) => {
                eprintln!("Warning: Failed to parse config file: {}", e);
                Settings::default()
            }
        },
        Err(e) => {
            eprintln!("Warning: Failed to read config file: {}", e);
            Settings::default()
        }
    }
}
