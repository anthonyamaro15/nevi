//! Shared glyph tables for rich (Nerd Font) vs minimal (ASCII/basic-Unicode)
//! UI chrome. One layout code path per surface; only the table differs.
//! Minimal doubles as the no-Nerd-Font fallback, so it must avoid
//! private-use-area codepoints entirely (enforced by test below).

pub struct UiGlyphs {
    /// Powerline transition, left side (rendered with fg = previous segment's
    /// bg and bg = next segment's bg).
    pub sep_left: &'static str,
    /// Powerline transition, right side.
    pub sep_right: &'static str,
    /// Item separator inside a flat segment (minimal statusline).
    pub item_sep: &'static str,
    /// Git branch prefix.
    pub branch: &'static str,
    /// Modified-buffer marker (replaces "[+]").
    pub modified: &'static str,
    /// Read-only marker.
    pub readonly: &'static str,
    /// Macro-recording marker prefix (register char is appended by caller).
    pub recording: &'static str,
    /// Diagnostic count prefixes.
    pub diag_error: &'static str,
    pub diag_warn: &'static str,
    /// LSP idle indicator.
    pub lsp_ok: &'static str,
    /// LSP busy indicator frames; advanced per LSP notification
    /// (event-driven — never on a timer).
    pub lsp_busy_frames: &'static [&'static str],
    /// Floating-window corners (rounded in rich, square in minimal).
    pub corner_tl: &'static str,
    pub corner_tr: &'static str,
    pub corner_bl: &'static str,
    pub corner_br: &'static str,
    /// Finder input prompt, rendered after the "[I]"/"[N]" mode indicator.
    /// Both variants occupy 3 columns so the prompt width math is shared.
    pub finder_prompt: &'static str,
    /// Selected-row accent bar; empty = no reserved bar column (legacy look).
    pub finder_selection_bar: &'static str,
    /// Prefix inside floating-window border titles.
    pub finder_title_icon: &'static str,
    /// Buffer gutter diagnostic signs (error/warning/info/hint priority).
    pub gutter_error: &'static str,
    pub gutter_warn: &'static str,
    pub gutter_info: &'static str,
    pub gutter_hint: &'static str,
    /// Icon before the project name in the explorer header.
    pub explorer_header_icon: &'static str,
}

pub static RICH: UiGlyphs = UiGlyphs {
    sep_left: "\u{e0b0}",
    sep_right: "\u{e0b2}",
    item_sep: " ",
    branch: "\u{e0a0} ",
    modified: "●",
    readonly: "\u{f023} ",
    recording: "\u{f111} @",
    diag_error: "\u{f057} ",
    diag_warn: "\u{f071} ",
    lsp_ok: "✓",
    lsp_busy_frames: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"],
    corner_tl: "╭",
    corner_tr: "╮",
    corner_bl: "╰",
    corner_br: "╯",
    finder_prompt: " \u{f002} ",
    finder_selection_bar: "▌",
    finder_title_icon: "\u{f002} ",
    gutter_error: "\u{f057}",
    gutter_warn: "\u{f071}",
    gutter_info: "\u{f05a}",
    gutter_hint: "\u{f0eb}",
    explorer_header_icon: "\u{f07b} ",
};

pub static MINIMAL: UiGlyphs = UiGlyphs {
    sep_left: "",
    sep_right: "",
    item_sep: " · ",
    branch: "",
    modified: "•",
    readonly: "[RO] ",
    recording: "[recording @",
    diag_error: "E:",
    diag_warn: "W:",
    lsp_ok: "✓",
    lsp_busy_frames: &["~"],
    corner_tl: "┌",
    corner_tr: "┐",
    corner_bl: "└",
    corner_br: "┘",
    finder_prompt: " > ",
    finder_selection_bar: "",
    finder_title_icon: "",
    gutter_error: "●",
    gutter_warn: "▲",
    gutter_info: "■",
    gutter_hint: "○",
    explorer_header_icon: "",
};

/// Devicon glyph for the finder's two-char file-type chips. The chip
/// classification (FuzzyFinder::get_file_icon) stays the single source of
/// file-type truth; rich mode only swaps its visual representation.
pub fn devicon_for_chip(chip: &str) -> &'static str {
    match chip {
        "TR" => "\u{f120}",               // terminal
        "RS" => "\u{e7a8}",               // rust
        "TS" => "\u{e628}",               // typescript
        "TX" => "\u{e7ba}",               // tsx/react
        "JS" => "\u{e74e}",               // javascript
        "JX" => "\u{e7ba}",               // jsx/react
        "PY" => "\u{e73c}",               // python
        "GO" => "\u{e626}",               // go
        "RB" => "\u{e739}",               // ruby
        "HT" => "\u{e736}",               // html
        "CS" | "SC" => "\u{e749}",        // css/scss
        "MD" => "\u{e73e}",               // markdown
        "YM" | "TM" | "CF" => "\u{e615}", // yaml/toml/config
        "GT" => "\u{e702}",               // git
        "EN" => "\u{f462}",               // env
        "SH" | "ZS" | "FS" => "\u{e795}", // shell
        _ => "\u{f15b}",                  // generic file
    }
}

/// File-type tint for chips and devicons — lifted unchanged from the
/// former inline match in render_finder so both representations share it.
pub fn file_chip_color(chip: &str) -> crossterm::style::Color {
    use crossterm::style::Color;
    match chip {
        "TR" => Color::Rgb {
            r: 90,
            g: 210,
            b: 120,
        }, // Terminal - green
        "RS" => Color::Rgb {
            r: 255,
            g: 100,
            b: 50,
        }, // Rust - orange
        "TS" | "TX" => Color::Rgb {
            r: 50,
            g: 150,
            b: 255,
        }, // TypeScript - blue
        "JS" | "JX" => Color::Rgb {
            r: 255,
            g: 220,
            b: 50,
        }, // JavaScript - yellow
        "PY" => Color::Rgb {
            r: 80,
            g: 180,
            b: 80,
        }, // Python - green
        "GO" => Color::Rgb {
            r: 100,
            g: 200,
            b: 220,
        }, // Go - cyan
        "RB" => Color::Rgb {
            r: 220,
            g: 50,
            b: 50,
        }, // Ruby - red
        "HT" => Color::Rgb {
            r: 230,
            g: 100,
            b: 50,
        }, // HTML - orange
        "CS" | "SC" => Color::Rgb {
            r: 100,
            g: 150,
            b: 255,
        }, // CSS - blue
        "MD" => Color::Rgb {
            r: 150,
            g: 150,
            b: 150,
        }, // Markdown - gray
        "YM" | "TM" | "CF" => Color::Rgb {
            r: 180,
            g: 140,
            b: 100,
        }, // Config - tan
        "GT" => Color::Rgb {
            r: 240,
            g: 80,
            b: 50,
        }, // Git - red-orange
        "EN" => Color::Rgb {
            r: 255,
            g: 200,
            b: 50,
        }, // Env - yellow
        "SH" | "ZS" | "FS" => Color::Rgb {
            r: 100,
            g: 200,
            b: 100,
        }, // Shell - green
        _ => Color::Rgb {
            r: 120,
            g: 120,
            b: 120,
        }, // Default - gray
    }
}

impl UiGlyphs {
    pub fn for_minimal(minimal: bool) -> &'static UiGlyphs {
        if minimal { &MINIMAL } else { &RICH }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(g: &UiGlyphs) -> Vec<&'static str> {
        let mut v = vec![
            g.sep_left,
            g.sep_right,
            g.item_sep,
            g.branch,
            g.modified,
            g.readonly,
            g.recording,
            g.diag_error,
            g.diag_warn,
            g.lsp_ok,
            g.corner_tl,
            g.corner_tr,
            g.corner_bl,
            g.corner_br,
            g.finder_prompt,
            g.finder_selection_bar,
            g.finder_title_icon,
            g.gutter_error,
            g.gutter_warn,
            g.gutter_info,
            g.gutter_hint,
            g.explorer_header_icon,
        ];
        v.extend(g.lsp_busy_frames);
        v
    }

    #[test]
    fn devicon_for_chip_maps_known_types() {
        let rust = devicon_for_chip("RS");
        let ts = devicon_for_chip("TS");
        let md = devicon_for_chip("MD");
        let unknown = devicon_for_chip("??");
        assert!(!rust.is_empty() && !ts.is_empty() && !md.is_empty());
        assert_ne!(rust, ts);
        assert_ne!(ts, md);
        assert_eq!(
            unknown, "\u{f15b}",
            "unknown chips fall back to a generic file glyph"
        );
    }

    #[test]
    fn minimal_gutter_signs_match_legacy_chars() {
        assert_eq!(MINIMAL.gutter_error, "●");
        assert_eq!(MINIMAL.gutter_warn, "▲");
        assert_eq!(MINIMAL.gutter_info, "■");
        assert_eq!(MINIMAL.gutter_hint, "○");
        assert!(MINIMAL.explorer_header_icon.is_empty());
    }

    #[test]
    fn minimal_finder_prompt_matches_legacy() {
        assert_eq!(MINIMAL.finder_prompt, " > ");
        assert_eq!(MINIMAL.corner_tl, "┌");
        assert!(MINIMAL.finder_selection_bar.is_empty());
    }

    #[test]
    fn minimal_avoids_nerd_font_private_use_codepoints() {
        for s in fields(&MINIMAL) {
            assert!(
                !s.chars()
                    .any(|c| matches!(c as u32, 0xE000..=0xF8FF | 0xF0000..=0xFFFFD)),
                "minimal glyph {s:?} uses a private-use codepoint"
            );
        }
    }

    #[test]
    fn for_minimal_selects_tables() {
        assert!(std::ptr::eq(UiGlyphs::for_minimal(true), &MINIMAL));
        assert!(std::ptr::eq(UiGlyphs::for_minimal(false), &RICH));
    }
}
