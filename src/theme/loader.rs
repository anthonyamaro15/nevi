//! Theme loading from TOML files
//!
//! Handles parsing theme TOML files and converting them to Theme structs.

use super::{DiagnosticColors, GitColors, StyleDef, SyntaxColors, Theme, UiColors};
use crossterm::style::Color;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

/// Raw theme data from TOML
#[derive(Debug, Deserialize)]
pub struct ThemeToml {
    #[serde(default)]
    pub palette: HashMap<String, String>,
    #[serde(default)]
    pub syntax: SyntaxToml,
    #[serde(default)]
    pub ui: UiToml,
    #[serde(default)]
    pub diagnostic: DiagnosticToml,
    #[serde(default)]
    pub git: GitToml,
}

#[derive(Debug, Default, Deserialize)]
pub struct StyleToml {
    pub fg: Option<String>,
    pub bg: Option<String>,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
}

#[derive(Debug, Default, Deserialize)]
pub struct SyntaxToml {
    pub keyword: Option<StyleToml>,
    pub function: Option<StyleToml>,
    #[serde(rename = "type")]
    pub type_: Option<StyleToml>,
    pub string: Option<StyleToml>,
    pub number: Option<StyleToml>,
    pub comment: Option<StyleToml>,
    pub operator: Option<StyleToml>,
    pub punctuation: Option<StyleToml>,
    pub variable: Option<StyleToml>,
    pub constant: Option<StyleToml>,
    pub attribute: Option<StyleToml>,
    pub namespace: Option<StyleToml>,
    pub label: Option<StyleToml>,
    pub property: Option<StyleToml>,
    pub tag: Option<StyleToml>,
}

#[derive(Debug, Default, Deserialize)]
pub struct UiToml {
    // Editor core
    pub background: Option<String>,
    pub foreground: Option<String>,
    pub cursor_line: Option<String>,
    pub selection: Option<String>,
    pub line_number: Option<String>,
    pub line_number_active: Option<String>,
    pub visual_bg: Option<String>,

    // Status line (nested table)
    #[serde(default)]
    pub statusline: StatuslineToml,

    // Popup (nested table)
    #[serde(default)]
    pub popup: PopupToml,

    // Completion (nested table)
    #[serde(default)]
    pub completion: CompletionToml,

    // Finder (nested table)
    #[serde(default)]
    pub finder: FinderToml,

    // Search (nested table)
    #[serde(default)]
    pub search: SearchToml,

    // Explorer (nested table)
    #[serde(default)]
    pub explorer: ExplorerToml,

    // Harpoon (nested table)
    #[serde(default)]
    pub harpoon: HarpoonToml,
}

#[derive(Debug, Default, Deserialize)]
pub struct StatuslineToml {
    pub background: Option<String>,
    pub foreground: Option<String>,
    pub mode_normal: Option<String>,
    pub mode_insert: Option<String>,
    pub mode_visual: Option<String>,
    pub mode_command: Option<String>,
    pub mode_replace: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct PopupToml {
    pub background: Option<String>,
    pub border: Option<String>,
    pub selection: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct CompletionToml {
    pub background: Option<String>,
    pub border: Option<String>,
    pub selected: Option<String>,
    #[serde(rename = "match")]
    pub match_: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct FinderToml {
    pub background: Option<String>,
    pub border: Option<String>,
    pub selected: Option<String>,
    #[serde(rename = "match")]
    pub match_: Option<String>,
    pub prompt: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct SearchToml {
    pub match_bg: Option<String>,
    pub match_fg: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ExplorerToml {
    pub background: Option<String>,
    pub border: Option<String>,
    pub selected: Option<String>,
    pub directory: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct HarpoonToml {
    pub background: Option<String>,
    pub border: Option<String>,
    pub selected: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct DiagnosticToml {
    pub error: Option<String>,
    pub warning: Option<String>,
    pub info: Option<String>,
    pub hint: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct GitToml {
    pub added: Option<String>,
    pub modified: Option<String>,
    pub deleted: Option<String>,
}

/// Parse a color string (either a palette reference or hex color)
fn resolve_color(value: &str, palette: &HashMap<String, String>) -> Option<Color> {
    // First check if it's a palette reference
    if let Some(hex) = palette.get(value) {
        parse_hex_color(hex)
    } else {
        // Try to parse as hex color directly
        parse_hex_color(value)
    }
}

/// Parse a hex color string (e.g., "#ff0000" or "ff0000")
fn parse_hex_color(s: &str) -> Option<Color> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }

    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;

    Some(Color::Rgb { r, g, b })
}

/// Parse a style from TOML
fn parse_style(style_toml: &Option<StyleToml>, palette: &HashMap<String, String>, default: StyleDef) -> StyleDef {
    match style_toml {
        Some(s) => {
            StyleDef {
                fg: s.fg.as_ref()
                    .and_then(|v| resolve_color(v, palette))
                    .unwrap_or(default.fg),
                bg: s.bg.as_ref().and_then(|v| resolve_color(v, palette)),
                bold: s.bold,
                italic: s.italic,
            }
        }
        None => default,
    }
}

/// Load a theme from TOML content (returns Result for error reporting)
pub fn try_load_theme_from_toml(name: &str, content: &str) -> Result<Theme, String> {
    let toml: ThemeToml = toml::from_str(content)
        .map_err(|e| format!("Theme '{}': {}", name, e))?;
    Ok(load_theme_from_toml_inner(name, &toml))
}

/// Load a theme from TOML content (returns Option, for bundled themes)
pub fn load_theme_from_toml(name: &str, content: &str) -> Option<Theme> {
    let toml: ThemeToml = toml::from_str(content).ok()?;
    Some(load_theme_from_toml_inner(name, &toml))
}

/// Internal helper to build theme from parsed TOML
fn load_theme_from_toml_inner(name: &str, toml: &ThemeToml) -> Theme {
    let palette = &toml.palette;

    // Get base theme for defaults
    let base = Theme::onedark();

    // Parse syntax colors
    let syntax = SyntaxColors {
        keyword: parse_style(&toml.syntax.keyword, palette, base.syntax.keyword),
        function: parse_style(&toml.syntax.function, palette, base.syntax.function),
        type_: parse_style(&toml.syntax.type_, palette, base.syntax.type_),
        string: parse_style(&toml.syntax.string, palette, base.syntax.string),
        number: parse_style(&toml.syntax.number, palette, base.syntax.number),
        comment: parse_style(&toml.syntax.comment, palette, base.syntax.comment),
        operator: parse_style(&toml.syntax.operator, palette, base.syntax.operator),
        punctuation: parse_style(&toml.syntax.punctuation, palette, base.syntax.punctuation),
        variable: parse_style(&toml.syntax.variable, palette, base.syntax.variable),
        constant: parse_style(&toml.syntax.constant, palette, base.syntax.constant),
        attribute: parse_style(&toml.syntax.attribute, palette, base.syntax.attribute),
        namespace: parse_style(&toml.syntax.namespace, palette, base.syntax.namespace),
        label: parse_style(&toml.syntax.label, palette, base.syntax.label),
        property: parse_style(&toml.syntax.property, palette, base.syntax.property),
        tag: parse_style(&toml.syntax.tag, palette, base.syntax.tag),
    };

    // Parse UI colors
    let ui = UiColors {
        background: toml.ui.background.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.background),
        foreground: toml.ui.foreground.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.foreground),
        cursor_line: toml.ui.cursor_line.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.cursor_line),
        selection: toml.ui.selection.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.selection),
        line_number: toml.ui.line_number.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.line_number),
        line_number_active: toml.ui.line_number_active.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.line_number_active),

        statusline_bg: toml.ui.statusline.background.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.statusline_bg),
        statusline_fg: toml.ui.statusline.foreground.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.statusline_fg),
        statusline_mode_normal: toml.ui.statusline.mode_normal.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.statusline_mode_normal),
        statusline_mode_insert: toml.ui.statusline.mode_insert.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.statusline_mode_insert),
        statusline_mode_visual: toml.ui.statusline.mode_visual.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.statusline_mode_visual),
        statusline_mode_command: toml.ui.statusline.mode_command.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.statusline_mode_command),
        statusline_mode_replace: toml.ui.statusline.mode_replace.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.statusline_mode_replace),

        popup_bg: toml.ui.popup.background.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.popup_bg),
        popup_border: toml.ui.popup.border.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.popup_border),
        popup_selection: toml.ui.popup.selection.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.popup_selection),

        completion_bg: toml.ui.completion.background.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.completion_bg),
        completion_border: toml.ui.completion.border.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.completion_border),
        completion_selected: toml.ui.completion.selected.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.completion_selected),
        completion_match: toml.ui.completion.match_.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.completion_match),
        completion_detail: toml.ui.completion.detail.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.completion_detail),

        finder_bg: toml.ui.finder.background.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.finder_bg),
        finder_border: toml.ui.finder.border.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.finder_border),
        finder_selected: toml.ui.finder.selected.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.finder_selected),
        finder_match: toml.ui.finder.match_.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.finder_match),
        finder_prompt: toml.ui.finder.prompt.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.finder_prompt),

        search_match_bg: toml.ui.search.match_bg.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.search_match_bg),
        search_match_fg: toml.ui.search.match_fg.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.search_match_fg),

        visual_bg: toml.ui.visual_bg.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.visual_bg),

        explorer_bg: toml.ui.explorer.background.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.explorer_bg),
        explorer_border: toml.ui.explorer.border.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.explorer_border),
        explorer_selected: toml.ui.explorer.selected.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.explorer_selected),
        explorer_directory: toml.ui.explorer.directory.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.explorer_directory),

        harpoon_bg: toml.ui.harpoon.background.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.harpoon_bg),
        harpoon_border: toml.ui.harpoon.border.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.harpoon_border),
        harpoon_selected: toml.ui.harpoon.selected.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.ui.harpoon_selected),
    };

    // Parse diagnostic colors
    let diagnostic = DiagnosticColors {
        error: toml.diagnostic.error.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.diagnostic.error),
        warning: toml.diagnostic.warning.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.diagnostic.warning),
        info: toml.diagnostic.info.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.diagnostic.info),
        hint: toml.diagnostic.hint.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.diagnostic.hint),
    };

    // Parse git colors
    let git = GitColors {
        added: toml.git.added.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.git.added),
        modified: toml.git.modified.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.git.modified),
        deleted: toml.git.deleted.as_ref()
            .and_then(|v| resolve_color(v, palette))
            .unwrap_or(base.git.deleted),
    };

    Theme {
        name: name.to_string(),
        syntax,
        ui,
        diagnostic,
        git,
    }
}

/// Get the user themes directory path
pub fn user_themes_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".config/nevi/themes"))
}

/// Load all user themes from ~/.config/nevi/themes/
/// Returns (themes, errors) tuple
pub fn load_user_themes() -> (Vec<Theme>, Vec<String>) {
    let Some(themes_dir) = user_themes_dir() else {
        return (Vec::new(), Vec::new());
    };

    if !themes_dir.exists() {
        return (Vec::new(), Vec::new());
    }

    let mut themes = Vec::new();
    let mut errors = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&themes_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "toml").unwrap_or(false) {
                if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                    match std::fs::read_to_string(&path) {
                        Ok(content) => {
                            match try_load_theme_from_toml(name, &content) {
                                Ok(theme) => themes.push(theme),
                                Err(e) => errors.push(e),
                            }
                        }
                        Err(e) => {
                            errors.push(format!("Theme '{}': failed to read file: {}", name, e));
                        }
                    }
                }
            }
        }
    }

    (themes, errors)
}
