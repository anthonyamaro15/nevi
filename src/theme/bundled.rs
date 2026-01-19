//! Bundled themes compiled into the binary
//!
//! Uses include_str! to embed theme TOML files at compile time.

use super::loader::load_theme_from_toml;
use super::Theme;

// Bundled theme TOML files
const ONEDARK_TOML: &str = include_str!("../../themes/onedark.toml");
const DRACULA_TOML: &str = include_str!("../../themes/dracula.toml");
const GRUVBOX_TOML: &str = include_str!("../../themes/gruvbox.toml");
const NORD_TOML: &str = include_str!("../../themes/nord.toml");
const TOKYONIGHT_TOML: &str = include_str!("../../themes/tokyonight.toml");

/// Get all bundled themes
pub fn get_bundled_themes() -> Vec<Theme> {
    let mut themes = Vec::new();

    // Load each bundled theme, falling back to hardcoded if TOML fails
    if let Some(theme) = load_theme_from_toml("onedark", ONEDARK_TOML) {
        themes.push(theme);
    } else {
        themes.push(Theme::onedark());
    }

    if let Some(theme) = load_theme_from_toml("dracula", DRACULA_TOML) {
        themes.push(theme);
    }

    if let Some(theme) = load_theme_from_toml("gruvbox", GRUVBOX_TOML) {
        themes.push(theme);
    }

    if let Some(theme) = load_theme_from_toml("nord", NORD_TOML) {
        themes.push(theme);
    }

    if let Some(theme) = load_theme_from_toml("tokyonight", TOKYONIGHT_TOML) {
        themes.push(theme);
    }

    themes
}

/// Get the names of bundled themes in display order
pub fn bundled_theme_names() -> Vec<&'static str> {
    vec!["onedark", "dracula", "gruvbox", "nord", "tokyonight"]
}
