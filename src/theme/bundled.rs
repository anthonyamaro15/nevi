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
const CATPPUCCIN_MOCHA_TOML: &str = include_str!("../../themes/catppuccin-mocha.toml");
const ROSE_PINE_TOML: &str = include_str!("../../themes/rose-pine.toml");
const SOLARIZED_DARK_TOML: &str = include_str!("../../themes/solarized-dark.toml");
const KANAGAWA_TOML: &str = include_str!("../../themes/kanagawa.toml");
const MONOKAI_TOML: &str = include_str!("../../themes/monokai.toml");
const EVERFOREST_TOML: &str = include_str!("../../themes/everforest.toml");
const GITHUB_DARK_TOML: &str = include_str!("../../themes/github-dark.toml");
const AYU_DARK_TOML: &str = include_str!("../../themes/ayu-dark.toml");
const PALENIGHT_TOML: &str = include_str!("../../themes/palenight.toml");
const NIGHTFOX_TOML: &str = include_str!("../../themes/nightfox.toml");

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

    if let Some(theme) = load_theme_from_toml("catppuccin-mocha", CATPPUCCIN_MOCHA_TOML) {
        themes.push(theme);
    }

    if let Some(theme) = load_theme_from_toml("rose-pine", ROSE_PINE_TOML) {
        themes.push(theme);
    }

    if let Some(theme) = load_theme_from_toml("solarized-dark", SOLARIZED_DARK_TOML) {
        themes.push(theme);
    }

    if let Some(theme) = load_theme_from_toml("kanagawa", KANAGAWA_TOML) {
        themes.push(theme);
    }

    if let Some(theme) = load_theme_from_toml("monokai", MONOKAI_TOML) {
        themes.push(theme);
    }

    if let Some(theme) = load_theme_from_toml("everforest", EVERFOREST_TOML) {
        themes.push(theme);
    }

    if let Some(theme) = load_theme_from_toml("github-dark", GITHUB_DARK_TOML) {
        themes.push(theme);
    }

    if let Some(theme) = load_theme_from_toml("ayu-dark", AYU_DARK_TOML) {
        themes.push(theme);
    }

    if let Some(theme) = load_theme_from_toml("palenight", PALENIGHT_TOML) {
        themes.push(theme);
    }

    if let Some(theme) = load_theme_from_toml("nightfox", NIGHTFOX_TOML) {
        themes.push(theme);
    }

    themes
}

/// Get the names of bundled themes in display order
pub fn bundled_theme_names() -> Vec<&'static str> {
    vec![
        "onedark",
        "dracula",
        "gruvbox",
        "nord",
        "tokyonight",
        "catppuccin-mocha",
        "rose-pine",
        "solarized-dark",
        "kanagawa",
        "monokai",
        "everforest",
        "github-dark",
        "ayu-dark",
        "palenight",
        "nightfox",
    ]
}
