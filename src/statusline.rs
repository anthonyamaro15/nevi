//! Pure statusline segment builder.
//!
//! Turns cached editor state (`StatusContext`) into colored segments for the
//! rich (powerline) or minimal (flat) layout. No I/O, no `Editor`, no
//! terminal — `render_status_line` in `terminal/mod.rs` is the only consumer
//! and does nothing but gather state, call this, and emit ANSI. That keeps
//! both visual modes testable without a terminal.

use crossterm::style::Color;

use crate::editor::Mode;
use crate::lsp::LspStatus;
use crate::theme::Theme;
use crate::ui_glyphs::UiGlyphs;

pub struct StatusSegment {
    pub text: String,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
}

/// Segments in draw order. The renderer paints `left`, pads with the
/// statusline background to the terminal width, then paints `right`.
pub struct StatusLineContent {
    pub left: Vec<StatusSegment>,
    pub right: Vec<StatusSegment>,
}

/// Everything the statusline shows, pre-gathered from cached editor state.
/// Render-reads-cache-only rule: callers must not compute git/LSP data here.
pub struct StatusContext<'a> {
    pub mode: Mode,
    /// The finder is open in live-grep mode; the badge reads GREP, not FINDER.
    pub live_grep: bool,
    /// Preformatted pending count/operator, e.g. "3d" ("" when none).
    pub pending: &'a str,
    /// Macro register currently recording.
    pub recording: Option<char>,
    pub project: Option<&'a str>,
    pub filename: &'a str,
    pub modified: bool,
    pub readonly: bool,
    pub large_file: bool,
    pub branch: Option<&'a str>,
    /// (added, modified) line counts from the cached git diff.
    pub diff: Option<(usize, usize)>,
    /// (errors, warnings) for the current buffer.
    pub diag: (usize, usize),
    pub lang: &'a str,
    /// Lifecycle of the buffer's server: nothing when `Stopped`, spinner
    /// while `Starting`, check when `Ready`, cross when `Error`.
    pub lsp: LspStatus,
    /// Current spinner frame (advanced per LSP notification, never a timer).
    pub lsp_spinner: &'a str,
    /// `[current/total]` search counter, cached on search state — event-driven,
    /// never counted at render time.
    pub search_count: Option<(usize, usize)>,
    /// 1-based for display.
    pub line: usize,
    pub col: usize,
    /// Vim-style ruler position (All/Top/Bot/percent through the file).
    pub position: ScrollPosition,
}

/// Vim ruler semantics: All when the whole file fits, Top/Bot at the
/// edges, percent-scrolled otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollPosition {
    All,
    Top,
    Bot,
    Percent(usize),
}

impl ScrollPosition {
    /// Compute from viewport state, mirroring Vim's ruler.
    pub fn from_viewport(offset: usize, visible_rows: usize, total_lines: usize) -> Self {
        if total_lines <= visible_rows {
            ScrollPosition::All
        } else if offset == 0 {
            ScrollPosition::Top
        } else if offset + visible_rows >= total_lines {
            ScrollPosition::Bot
        } else {
            ScrollPosition::Percent((offset * 100) / (total_lines - visible_rows))
        }
    }

    fn label(self) -> String {
        match self {
            ScrollPosition::All => "All".to_string(),
            ScrollPosition::Top => "Top".to_string(),
            ScrollPosition::Bot => "Bot".to_string(),
            ScrollPosition::Percent(p) => format!("{}%", p),
        }
    }
}

fn mode_color(mode: Mode, theme: &Theme) -> Color {
    match mode {
        Mode::Normal | Mode::Command => theme.ui.statusline_mode_normal,
        Mode::Insert => theme.ui.statusline_mode_insert,
        Mode::Visual | Mode::VisualLine | Mode::VisualBlock => theme.ui.statusline_mode_visual,
        Mode::Replace => theme.ui.statusline_mode_replace,
        // Search reads as its own mode, like INSERT-green does
        Mode::Search => theme.ui.statusline_mode_command,
        _ => theme.ui.statusline_mode_normal,
    }
}

/// Vim parity: command mode keeps showing NORMAL in the statusline.
fn mode_label(ctx: &StatusContext) -> &'static str {
    match ctx.mode {
        Mode::Command => "NORMAL",
        Mode::Finder if ctx.live_grep => "GREP",
        mode => mode.as_str(),
    }
}

/// The LSP glyph after the language name, with its color. `None` when the
/// buffer has no server, so nothing claims a state it doesn't have.
fn lsp_indicator<'a>(
    ctx: &StatusContext<'a>,
    glyphs: &UiGlyphs,
    theme: &Theme,
    fg: Color,
) -> Option<(&'a str, Color)> {
    match ctx.lsp {
        LspStatus::Stopped => None,
        LspStatus::Starting => Some((ctx.lsp_spinner, fg)),
        LspStatus::Ready => Some((glyphs.lsp_ok, fg)),
        LspStatus::Error => Some((glyphs.lsp_err, theme.diagnostic.error)),
    }
}

fn seg(text: String, fg: Color, bg: Color) -> StatusSegment {
    StatusSegment {
        text,
        fg,
        bg,
        bold: false,
    }
}

fn seg_bold(text: String, fg: Color, bg: Color) -> StatusSegment {
    StatusSegment {
        text,
        fg,
        bg,
        bold: true,
    }
}

/// Trailing file-state markers shared by both layouts: modified dot,
/// read-only, large-file, pending operator, macro recording.
fn push_file_markers(
    out: &mut Vec<StatusSegment>,
    ctx: &StatusContext,
    glyphs: &UiGlyphs,
    theme: &Theme,
    bg: Color,
    minimal: bool,
) {
    let fg = theme.ui.statusline_fg;
    if ctx.modified {
        out.push(seg(format!(" {}", glyphs.modified), theme.git.modified, bg));
    }
    if ctx.readonly {
        out.push(seg(format!(" {}", glyphs.readonly.trim_end()), fg, bg));
    }
    if ctx.large_file {
        out.push(seg(" [large]".to_string(), fg, bg));
    }
    if !ctx.pending.is_empty() {
        out.push(seg(format!(" [{}]", ctx.pending), fg, bg));
    }
    if let Some(register) = ctx.recording {
        // Minimal's prefix opens a bracket that needs closing; rich is a glyph.
        let close = if minimal { "]" } else { "" };
        out.push(seg(
            format!(" {}{}{}", glyphs.recording, register, close),
            theme.diagnostic.error,
            bg,
        ));
    }
}

pub fn build_status_segments(
    ctx: &StatusContext,
    glyphs: &UiGlyphs,
    theme: &Theme,
    minimal: bool,
) -> StatusLineContent {
    if minimal {
        build_minimal(ctx, glyphs, theme)
    } else {
        build_rich(ctx, glyphs, theme)
    }
}

fn build_rich(ctx: &StatusContext, glyphs: &UiGlyphs, theme: &Theme) -> StatusLineContent {
    let mode_bg = mode_color(ctx.mode, theme);
    let base_bg = theme.ui.statusline_bg;
    let base_fg = theme.ui.statusline_fg;
    let section_bg = theme.ui.statusline_section_bg;

    let mut left = Vec::new();
    left.push(seg_bold(format!(" {} ", mode_label(ctx)), base_bg, mode_bg));

    // Mode → git (or file) powerline transition.
    let after_mode_bg = if ctx.branch.is_some() {
        section_bg
    } else {
        base_bg
    };
    left.push(seg(glyphs.sep_left.to_string(), mode_bg, after_mode_bg));

    if let Some(branch) = ctx.branch {
        left.push(seg(
            format!(" {}{}", glyphs.branch, branch),
            base_fg,
            section_bg,
        ));
        if let Some((added, modified)) = ctx.diff {
            if added > 0 {
                left.push(seg(format!(" +{}", added), theme.git.added, section_bg));
            }
            if modified > 0 {
                left.push(seg(
                    format!(" ~{}", modified),
                    theme.git.modified,
                    section_bg,
                ));
            }
        }
        left.push(seg(" ".to_string(), base_fg, section_bg));
        left.push(seg(glyphs.sep_left.to_string(), section_bg, base_bg));
    }

    let project = ctx.project.map(|p| format!("[{}] ", p)).unwrap_or_default();
    left.push(seg(
        format!(" {}{}", project, ctx.filename),
        base_fg,
        base_bg,
    ));
    push_file_markers(&mut left, ctx, glyphs, theme, base_bg, false);

    let mut right = Vec::new();
    push_search_count(&mut right, ctx, theme, base_fg, base_bg);
    let (errors, warnings) = ctx.diag;
    if errors > 0 {
        right.push(seg(
            format!("{}{} ", glyphs.diag_error, errors),
            theme.diagnostic.error,
            base_bg,
        ));
    }
    if warnings > 0 {
        right.push(seg(
            format!("{}{} ", glyphs.diag_warn, warnings),
            theme.diagnostic.warning,
            base_bg,
        ));
    }
    right.push(seg(glyphs.sep_right.to_string(), section_bg, base_bg));
    right.push(seg(format!(" {}", ctx.lang), base_fg, section_bg));
    if let Some((glyph, color)) = lsp_indicator(ctx, glyphs, theme, base_fg) {
        right.push(seg(format!(" {}", glyph), color, section_bg));
    }
    right.push(seg(" ".to_string(), base_fg, section_bg));
    right.push(seg(glyphs.sep_right.to_string(), mode_bg, section_bg));
    right.push(seg_bold(
        format!(" {}:{}  {} ", ctx.line, ctx.col, ctx.position.label()),
        base_bg,
        mode_bg,
    ));

    StatusLineContent { left, right }
}

/// The `n/m` search counter as a badge in the match colors, so it reads as
/// part of the search. Plain match-colored text would vanish on light themes,
/// whose match color is a pale background tint.
fn push_search_count(
    right: &mut Vec<StatusSegment>,
    ctx: &StatusContext,
    theme: &Theme,
    fg: Color,
    bg: Color,
) {
    if let Some((current, total)) = ctx.search_count {
        right.push(seg(
            format!(" {}/{} ", current, total),
            theme.ui.search_match_fg,
            theme.ui.search_match_bg,
        ));
        right.push(seg(" ".to_string(), fg, bg));
    }
}

fn build_minimal(ctx: &StatusContext, glyphs: &UiGlyphs, theme: &Theme) -> StatusLineContent {
    let bg = theme.ui.statusline_bg;
    let fg = theme.ui.statusline_fg;

    let mut left = Vec::new();
    left.push(seg_bold(
        format!(" {}", mode_label(ctx)),
        mode_color(ctx.mode, theme),
        bg,
    ));
    if let Some(branch) = ctx.branch {
        left.push(seg(format!("{}{}", glyphs.item_sep, branch), fg, bg));
        if let Some((added, modified)) = ctx.diff {
            if added > 0 {
                left.push(seg(format!(" +{}", added), theme.git.added, bg));
            }
            if modified > 0 {
                left.push(seg(format!(" ~{}", modified), theme.git.modified, bg));
            }
        }
    }
    let project = ctx.project.map(|p| format!("[{}] ", p)).unwrap_or_default();
    left.push(seg(
        format!("{}{}{}", glyphs.item_sep, project, ctx.filename),
        fg,
        bg,
    ));
    push_file_markers(&mut left, ctx, glyphs, theme, bg, true);

    let mut right = Vec::new();
    push_search_count(&mut right, ctx, theme, fg, bg);
    let (errors, warnings) = ctx.diag;
    if errors > 0 {
        right.push(seg(
            format!("{}{} ", glyphs.diag_error, errors),
            theme.diagnostic.error,
            bg,
        ));
    }
    if warnings > 0 {
        right.push(seg(
            format!("{}{} ", glyphs.diag_warn, warnings),
            theme.diagnostic.warning,
            bg,
        ));
    }
    right.push(seg(format!(" {}", ctx.lang), fg, bg));
    if let Some((glyph, color)) = lsp_indicator(ctx, glyphs, theme, fg) {
        right.push(seg(format!(" {}", glyph), color, bg));
    }
    right.push(seg(format!(" {}:{} ", ctx.line, ctx.col), fg, bg));

    StatusLineContent { left, right }
}

/// Terminal columns occupied by the segments — unicode-width, never `.len()`.
/// Icons are multi-byte and CJK filenames are double-width; byte math
/// misaligns the right side.
pub fn display_width(segments: &[StatusSegment]) -> usize {
    use unicode_width::UnicodeWidthStr;
    segments
        .iter()
        .map(|s| UnicodeWidthStr::width(s.text.as_str()))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_glyphs::{MINIMAL, RICH};

    fn theme() -> Theme {
        Theme::default()
    }

    fn ctx_base<'a>() -> StatusContext<'a> {
        StatusContext {
            mode: Mode::Normal,
            live_grep: false,
            pending: "",
            recording: None,
            project: Some("rvim"),
            filename: "mod.rs",
            modified: true,
            readonly: false,
            large_file: false,
            branch: Some("master"),
            diff: Some((12, 3)),
            diag: (2, 1),
            lang: "rust",
            lsp: LspStatus::Ready,
            lsp_spinner: "",
            search_count: None,
            line: 128,
            col: 14,
            position: ScrollPosition::Percent(93),
        }
    }

    fn all_text(content: &StatusLineContent) -> String {
        content
            .left
            .iter()
            .chain(content.right.iter())
            .map(|s| s.text.as_str())
            .collect()
    }

    #[test]
    fn rich_layout_has_mode_block_and_powerline_transitions() {
        let content = build_status_segments(&ctx_base(), &RICH, &theme(), false);
        assert!(content.left[0].text.contains("NORMAL"));
        assert!(content.left[0].bold);
        let all = all_text(&content);
        assert!(all.contains("\u{e0b0}"), "left powerline separator present");
        assert!(all.contains("master"), "branch renders");
        assert!(all.contains("●"), "modified dot renders");
        assert!(
            all.contains("+12") && all.contains("~3"),
            "diff stats render"
        );
    }

    #[test]
    fn minimal_layout_is_flat_and_ascii_prefixed() {
        let theme = theme();
        let content = build_status_segments(&ctx_base(), &MINIMAL, &theme, true);
        for s in content.left.iter().chain(content.right.iter()) {
            assert_eq!(s.bg, theme.ui.statusline_bg, "minimal = one background");
        }
        let all = all_text(&content);
        assert!(all.contains("E:2"));
        assert!(all.contains("W:1"));
        assert!(!all.contains("\u{e0b0}"));
    }

    #[test]
    fn zero_diagnostics_are_hidden() {
        let mut ctx = ctx_base();
        ctx.diag = (0, 0);
        for minimal in [false, true] {
            let content =
                build_status_segments(&ctx, UiGlyphs::for_minimal(minimal), &theme(), minimal);
            let all = all_text(&content);
            assert!(!all.contains("E:0"));
            assert!(!all.contains('\u{f057}'));
            assert!(!all.contains('\u{f071}'));
        }
    }

    #[test]
    fn recording_and_pending_are_preserved() {
        let mut ctx = ctx_base();
        ctx.pending = "3d";
        ctx.recording = Some('q');
        let content = build_status_segments(&ctx, &MINIMAL, &theme(), true);
        let all = all_text(&content);
        assert!(all.contains("[3d]"));
        assert!(all.contains("[recording @q]"));

        let content = build_status_segments(&ctx, &RICH, &theme(), false);
        let all = all_text(&content);
        assert!(all.contains("[3d]"));
        assert!(all.contains("@q"));
    }

    #[test]
    fn search_mode_gets_command_color_badge() {
        let theme = theme();
        let mut ctx = ctx_base();
        ctx.mode = Mode::Search;
        let content = build_status_segments(&ctx, &RICH, &theme, false);
        assert!(content.left[0].text.contains("SEARCH"));
        assert_eq!(
            content.left[0].bg, theme.ui.statusline_mode_command,
            "SEARCH badge uses the command color, not normal-blue"
        );
    }

    #[test]
    fn scroll_position_renders_vim_style_ruler_tokens() {
        for (pos, expect) in [
            (ScrollPosition::All, "All"),
            (ScrollPosition::Top, "Top"),
            (ScrollPosition::Bot, "Bot"),
            (ScrollPosition::Percent(52), "52%"),
        ] {
            let mut ctx = ctx_base();
            ctx.position = pos;
            let content = build_status_segments(&ctx, &RICH, &theme(), false);
            let last = content.right.last().expect("position segment");
            assert!(
                last.text.contains(expect),
                "expected {expect:?} in {:?}",
                last.text
            );
        }
    }

    #[test]
    fn search_count_renders_in_both_layouts_and_hides_when_absent() {
        for minimal in [false, true] {
            let glyphs = UiGlyphs::for_minimal(minimal);
            let content = build_status_segments(&ctx_base(), glyphs, &theme(), minimal);
            assert!(!all_text(&content).contains("3/12"));

            let mut ctx = ctx_base();
            ctx.search_count = Some((3, 12));
            let content = build_status_segments(&ctx, glyphs, &theme(), minimal);
            let badge = content
                .right
                .iter()
                .find(|s| s.text == " 3/12 ")
                .expect("counter badge");
            assert_eq!(badge.fg, theme().ui.search_match_fg);
            assert_eq!(badge.bg, theme().ui.search_match_bg);
        }
    }

    #[test]
    fn command_mode_displays_normal_badge() {
        let mut ctx = ctx_base();
        ctx.mode = Mode::Command;
        let content = build_status_segments(&ctx, &RICH, &theme(), false);
        assert!(content.left[0].text.contains("NORMAL"));
    }

    #[test]
    fn display_width_counts_columns_not_bytes() {
        let segs = vec![StatusSegment {
            text: "日本".into(),
            fg: Color::Reset,
            bg: Color::Reset,
            bold: false,
        }];
        // 2 double-width chars = 4 columns (6 bytes).
        assert_eq!(display_width(&segs), 4);
    }

    /// The LSP glyph segment(s) and their colors, for both layouts.
    fn lsp_text(ctx: &StatusContext, minimal: bool) -> (String, Vec<Color>) {
        let content = build_status_segments(ctx, UiGlyphs::for_minimal(minimal), &theme(), minimal);
        let glyphs = UiGlyphs::for_minimal(minimal);
        let marks = [glyphs.lsp_ok, glyphs.lsp_err, "⠹"];
        let hits: Vec<&StatusSegment> = content
            .right
            .iter()
            .filter(|s| marks.iter().any(|m| s.text.trim() == *m))
            .collect();
        (
            hits.iter().map(|s| s.text.trim()).collect(),
            hits.iter().map(|s| s.fg).collect(),
        )
    }

    #[test]
    fn lsp_glyph_follows_the_lifecycle_state_in_both_layouts() {
        let theme = theme();
        for minimal in [false, true] {
            let glyphs = UiGlyphs::for_minimal(minimal);
            let mut ctx = ctx_base();
            ctx.lsp_spinner = "⠹";

            // No server for this buffer: nothing after the language name,
            // even though main.rs stores "LSP: (no server)" text at startup.
            ctx.lsp = LspStatus::Stopped;
            assert_eq!(lsp_text(&ctx, minimal).0, "", "minimal={minimal}");

            ctx.lsp = LspStatus::Starting;
            assert_eq!(lsp_text(&ctx, minimal).0, "⠹", "minimal={minimal}");

            ctx.lsp = LspStatus::Ready;
            assert_eq!(
                lsp_text(&ctx, minimal).0,
                glyphs.lsp_ok,
                "minimal={minimal}"
            );

            ctx.lsp = LspStatus::Error;
            let (text, colors) = lsp_text(&ctx, minimal);
            assert_eq!(text, glyphs.lsp_err, "minimal={minimal}");
            assert_eq!(
                colors,
                vec![theme.diagnostic.error],
                "a failed server reads as an error, minimal={minimal}"
            );
        }
    }

    #[test]
    fn lsp_state_does_not_change_the_statusline_width_between_ready_and_error() {
        // Ready and Error swap one glyph for another; the right side must not
        // jump when a server dies.
        for minimal in [false, true] {
            let mut ctx = ctx_base();
            ctx.lsp = LspStatus::Ready;
            let ready =
                build_status_segments(&ctx, UiGlyphs::for_minimal(minimal), &theme(), minimal);
            ctx.lsp = LspStatus::Error;
            let error =
                build_status_segments(&ctx, UiGlyphs::for_minimal(minimal), &theme(), minimal);
            assert_eq!(display_width(&ready.right), display_width(&error.right));
        }
    }

    #[test]
    fn live_grep_badge_reads_grep_and_other_finders_keep_finder() {
        for minimal in [false, true] {
            let mut ctx = ctx_base();
            ctx.mode = Mode::Finder;
            ctx.live_grep = true;
            let grep =
                build_status_segments(&ctx, UiGlyphs::for_minimal(minimal), &theme(), minimal);
            assert!(grep.left[0].text.contains("GREP"), "minimal={minimal}");

            ctx.live_grep = false;
            let files =
                build_status_segments(&ctx, UiGlyphs::for_minimal(minimal), &theme(), minimal);
            assert!(files.left[0].text.contains("FINDER"), "minimal={minimal}");

            // A stale grep flag never relabels a non-finder mode.
            ctx.mode = Mode::Normal;
            ctx.live_grep = true;
            let normal =
                build_status_segments(&ctx, UiGlyphs::for_minimal(minimal), &theme(), minimal);
            assert!(normal.left[0].text.contains("NORMAL"), "minimal={minimal}");
        }
    }
}
