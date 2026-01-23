//! Theme system for nevi
//!
//! Provides comprehensive theming for syntax highlighting, UI elements,
//! diagnostics, and git indicators.

pub mod bundled;
pub mod loader;

use crossterm::style::Color;
use std::collections::HashMap;

/// A complete theme definition
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub syntax: SyntaxColors,
    pub ui: UiColors,
    pub diagnostic: DiagnosticColors,
    pub git: GitColors,
}

/// Style definition for syntax elements
#[derive(Debug, Clone, Copy)]
pub struct StyleDef {
    pub fg: Color,
    pub bg: Option<Color>,
    pub bold: bool,
    pub italic: bool,
}

impl StyleDef {
    pub fn new(fg: Color) -> Self {
        Self {
            fg,
            bg: None,
            bold: false,
            italic: false,
        }
    }

    pub fn with_italic(mut self) -> Self {
        self.italic = true;
        self
    }

    pub fn with_bold(mut self) -> Self {
        self.bold = true;
        self
    }
}

/// Syntax highlighting colors (16 groups)
#[derive(Debug, Clone)]
pub struct SyntaxColors {
    pub keyword: StyleDef,
    pub function: StyleDef,
    pub type_: StyleDef,
    pub string: StyleDef,
    pub number: StyleDef,
    pub comment: StyleDef,
    pub operator: StyleDef,
    pub punctuation: StyleDef,
    pub variable: StyleDef,
    pub constant: StyleDef,
    pub attribute: StyleDef,
    pub namespace: StyleDef,
    pub label: StyleDef,
    pub property: StyleDef,
    pub tag: StyleDef,
    pub embedded: StyleDef, // For embedded expressions like ${} in template strings
}

/// UI element colors
#[derive(Debug, Clone)]
pub struct UiColors {
    // Editor core
    pub background: Color,
    pub foreground: Color,
    pub cursor_line: Color,
    pub selection: Color,
    pub line_number: Color,
    pub line_number_active: Color,

    // Status line
    pub statusline_bg: Color,
    pub statusline_fg: Color,
    pub statusline_mode_normal: Color,
    pub statusline_mode_insert: Color,
    pub statusline_mode_visual: Color,
    pub statusline_mode_command: Color,
    pub statusline_mode_replace: Color,

    // Popups/Floating windows
    pub popup_bg: Color,
    pub popup_border: Color,
    pub popup_selection: Color,

    // Completion
    pub completion_bg: Color,
    pub completion_border: Color,
    pub completion_selected: Color,
    pub completion_match: Color,
    pub completion_detail: Color,

    // Finder
    pub finder_bg: Color,
    pub finder_border: Color,
    pub finder_selected: Color,
    pub finder_match: Color,
    pub finder_prompt: Color,

    // Search
    pub search_match_bg: Color,
    pub search_match_fg: Color,

    // Visual mode
    pub visual_bg: Color,

    // Explorer
    pub explorer_bg: Color,
    pub explorer_border: Color,
    pub explorer_selected: Color,
    pub explorer_directory: Color,

    // Harpoon menu
    pub harpoon_bg: Color,
    pub harpoon_border: Color,
    pub harpoon_selected: Color,
}

/// Diagnostic colors (LSP errors, warnings, etc.)
#[derive(Debug, Clone, Copy)]
pub struct DiagnosticColors {
    pub error: Color,
    pub warning: Color,
    pub info: Color,
    pub hint: Color,
}

/// Git indicator colors
#[derive(Debug, Clone, Copy)]
pub struct GitColors {
    pub added: Color,
    pub modified: Color,
    pub deleted: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self::onedark()
    }
}

impl Theme {
    /// Create the default One Dark theme
    pub fn onedark() -> Self {
        // One Dark palette
        let red = Color::Rgb { r: 224, g: 108, b: 117 };
        let green = Color::Rgb { r: 152, g: 195, b: 121 };
        let yellow = Color::Rgb { r: 229, g: 192, b: 123 };
        let blue = Color::Rgb { r: 97, g: 175, b: 239 };
        let purple = Color::Rgb { r: 198, g: 120, b: 221 };
        let cyan = Color::Rgb { r: 86, g: 182, b: 194 };
        let orange = Color::Rgb { r: 209, g: 154, b: 102 };
        let gray = Color::Rgb { r: 92, g: 99, b: 112 };
        let fg = Color::Rgb { r: 171, g: 178, b: 191 };
        let bg = Color::Rgb { r: 40, g: 44, b: 52 };
        let bg_dark = Color::Rgb { r: 33, g: 37, b: 43 };

        Self {
            name: "onedark".to_string(),
            syntax: SyntaxColors {
                keyword: StyleDef::new(purple),
                function: StyleDef::new(blue),
                type_: StyleDef::new(yellow),
                string: StyleDef::new(green),
                number: StyleDef::new(orange),
                comment: StyleDef::new(gray).with_italic(),
                operator: StyleDef::new(cyan),
                punctuation: StyleDef::new(fg),
                variable: StyleDef::new(red),
                constant: StyleDef::new(orange),
                attribute: StyleDef::new(yellow),
                namespace: StyleDef::new(blue),
                label: StyleDef::new(red),
                property: StyleDef::new(red),
                tag: StyleDef::new(red),
                embedded: StyleDef::new(cyan),
            },
            ui: UiColors {
                background: bg,
                foreground: fg,
                cursor_line: Color::Rgb { r: 44, g: 49, b: 60 },
                selection: Color::Rgb { r: 62, g: 68, b: 81 },
                line_number: gray,
                line_number_active: fg,

                statusline_bg: bg_dark,
                statusline_fg: fg,
                statusline_mode_normal: blue,
                statusline_mode_insert: green,
                statusline_mode_visual: purple,
                statusline_mode_command: yellow,
                statusline_mode_replace: red,

                popup_bg: bg_dark,
                popup_border: Color::Rgb { r: 55, g: 55, b: 65 },
                popup_selection: Color::Rgb { r: 55, g: 77, b: 95 },

                completion_bg: Color::Rgb { r: 30, g: 30, b: 36 },
                completion_border: Color::Rgb { r: 55, g: 55, b: 65 },
                completion_selected: Color::Rgb { r: 55, g: 77, b: 95 },
                completion_match: yellow,
                completion_detail: Color::Rgb { r: 100, g: 100, b: 115 },

                finder_bg: Color::Rgb { r: 25, g: 25, b: 30 },
                finder_border: Color::Rgb { r: 100, g: 100, b: 100 },
                finder_selected: Color::Rgb { r: 60, g: 60, b: 100 },
                finder_match: yellow,
                finder_prompt: blue,

                search_match_bg: Color::Rgb { r: 180, g: 160, b: 60 },
                search_match_fg: Color::Rgb { r: 0, g: 0, b: 0 },

                visual_bg: Color::Rgb { r: 62, g: 68, b: 81 },

                explorer_bg: bg_dark,
                explorer_border: Color::Rgb { r: 55, g: 55, b: 65 },
                explorer_selected: Color::Rgb { r: 55, g: 77, b: 95 },
                explorer_directory: blue,

                harpoon_bg: bg_dark,
                harpoon_border: Color::Rgb { r: 55, g: 55, b: 65 },
                harpoon_selected: Color::Rgb { r: 55, g: 77, b: 95 },
            },
            diagnostic: DiagnosticColors {
                error: Color::Rgb { r: 255, g: 100, b: 100 },
                warning: Color::Rgb { r: 255, g: 200, b: 100 },
                info: blue,
                hint: cyan,
            },
            git: GitColors {
                added: green,
                modified: yellow,
                deleted: red,
            },
        }
    }

    /// Get the syntax color for a highlight group by name
    pub fn get_syntax_color(&self, capture_name: &str) -> Option<Color> {
        let base = capture_name.split('.').next()?;
        let style = match base {
            "keyword" => &self.syntax.keyword,
            "function" => &self.syntax.function,
            "type" => &self.syntax.type_,
            "string" => &self.syntax.string,
            "number" => &self.syntax.number,
            "comment" => &self.syntax.comment,
            "operator" => &self.syntax.operator,
            "punctuation" => &self.syntax.punctuation,
            "variable" => &self.syntax.variable,
            "constant" => &self.syntax.constant,
            "attribute" => &self.syntax.attribute,
            "namespace" => &self.syntax.namespace,
            "label" => &self.syntax.label,
            "property" => &self.syntax.property,
            "tag" => &self.syntax.tag,
            _ => return None,
        };
        Some(style.fg)
    }

    /// Get the full style for a highlight group by name
    pub fn get_syntax_style(&self, capture_name: &str) -> Option<&StyleDef> {
        let base = capture_name.split('.').next()?;
        match base {
            "keyword" => Some(&self.syntax.keyword),
            "function" => Some(&self.syntax.function),
            "type" => Some(&self.syntax.type_),
            "string" => Some(&self.syntax.string),
            "number" => Some(&self.syntax.number),
            "comment" => Some(&self.syntax.comment),
            "operator" => Some(&self.syntax.operator),
            "punctuation" => Some(&self.syntax.punctuation),
            "variable" => Some(&self.syntax.variable),
            "constant" => Some(&self.syntax.constant),
            "attribute" => Some(&self.syntax.attribute),
            "namespace" => Some(&self.syntax.namespace),
            "label" => Some(&self.syntax.label),
            "property" => Some(&self.syntax.property),
            "tag" => Some(&self.syntax.tag),
            _ => None,
        }
    }
}

/// Theme manager for loading and switching themes
pub struct ThemeManager {
    /// Currently active theme
    pub current: Theme,
    /// All available themes (bundled + user)
    pub available: HashMap<String, Theme>,
    /// Original theme (for cancel restore in picker)
    original_theme_name: Option<String>,
}

impl Default for ThemeManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ThemeManager {
    /// Create a new theme manager with bundled themes loaded
    pub fn new() -> Self {
        let mut available = HashMap::new();

        // Load bundled themes
        for theme in bundled::get_bundled_themes() {
            available.insert(theme.name.clone(), theme);
        }

        Self {
            current: Theme::default(),
            available,
            original_theme_name: None,
        }
    }

    /// Load user themes from ~/.config/nevi/themes/
    /// Returns any errors that occurred during theme loading
    pub fn load_user_themes(&mut self) -> Vec<String> {
        // Ensure themes directory and template exist
        loader::ensure_themes_dir_exists();

        let (themes, errors) = loader::load_user_themes();
        for theme in themes {
            self.available.insert(theme.name.clone(), theme);
        }
        errors
    }

    /// Get list of available theme names
    pub fn list_themes(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.available.keys().map(|s| s.as_str()).collect();
        names.sort();
        names
    }

    /// Get list of theme names with bundled first, then user themes
    pub fn list_themes_sorted(&self) -> Vec<(&str, bool)> {
        let bundled_names = bundled::bundled_theme_names();
        let mut result: Vec<(&str, bool)> = Vec::new();

        // Add bundled themes first (in order)
        for name in &bundled_names {
            if self.available.contains_key(*name) {
                result.push((name, true));
            }
        }

        // Add user themes (sorted)
        let mut user_themes: Vec<&str> = self.available.keys()
            .filter(|name| !bundled_names.contains(&name.as_str()))
            .map(|s| s.as_str())
            .collect();
        user_themes.sort();

        for name in user_themes {
            result.push((name, false));
        }

        result
    }

    /// Switch to a theme by name
    pub fn set_theme(&mut self, name: &str) -> bool {
        if let Some(theme) = self.available.get(name) {
            self.current = theme.clone();
            true
        } else {
            false
        }
    }

    /// Start theme preview (for picker)
    pub fn start_preview(&mut self) {
        self.original_theme_name = Some(self.current.name.clone());
    }

    /// Preview a theme (apply temporarily)
    pub fn preview_theme(&mut self, name: &str) -> bool {
        self.set_theme(name)
    }

    /// Cancel preview and restore original theme
    pub fn cancel_preview(&mut self) {
        if let Some(name) = self.original_theme_name.take() {
            self.set_theme(&name);
        }
    }

    /// Confirm the current preview
    pub fn confirm_preview(&mut self) {
        self.original_theme_name = None;
    }

    /// Get the current theme
    pub fn theme(&self) -> &Theme {
        &self.current
    }
}
