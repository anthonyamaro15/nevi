//! The LSP hover reply, parsed and highlighted once when it arrives and
//! wrapped to the popup width. Rendering only copies `rows`, so an open
//! hover costs no parsing per frame.

use crate::editor::Editor;
use crate::markdown_preview::{
    PreviewLine, PreviewLineKind, PreviewSpan, render_markdown, wrap_preview_lines,
};
use crate::syntax::{HighlightSpan, SyntaxManager, SyntaxStyle};
use crate::theme::Theme;
use std::path::Path;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Widest the text area gets; docs reflow to this, like a readable column.
const MAX_WIDTH: usize = 76;
const MIN_WIDTH: usize = 30;

#[derive(Debug, Clone, PartialEq)]
pub struct CodeSpan {
    pub text: String,
    /// `None` where the highlighter has nothing to say.
    pub style: Option<SyntaxStyle>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HoverRow {
    Text(PreviewLine),
    Code(Vec<CodeSpan>),
    Rule,
}

#[derive(Debug, Clone)]
enum Block {
    Text(Vec<PreviewLine>),
    Code(Vec<Vec<CodeSpan>>),
}

#[derive(Debug, Clone)]
pub struct Hover {
    /// The symbol's one-line signature, shown in the top border.
    pub title: Vec<CodeSpan>,
    blocks: Vec<Block>,
    /// Display rows wrapped to `width`.
    pub rows: Vec<HoverRow>,
    /// Text-area columns, borders and padding not included.
    pub width: usize,
    /// First row shown; moves a page at a time.
    pub scroll: usize,
}

impl Hover {
    pub fn for_editor(editor: &Editor, markdown: &str) -> Option<Self> {
        Self::new(
            markdown,
            editor.theme(),
            editor.buffer().path.as_deref(),
            editor.term_width,
        )
    }

    /// `path` picks the highlighter for code fences that name no language.
    pub fn new(
        markdown: &str,
        theme: &Theme,
        path: Option<&Path>,
        term_width: u16,
    ) -> Option<Self> {
        let mut blocks = parse(&expand_tabs(markdown), theme, path);
        // A one-line code block on top is the signature (`package fmt`,
        // `func fmt.Println(...)`): it becomes the title, unless it's all
        // there is.
        let title = match blocks.as_slice() {
            [Block::Code(lines), _, ..] if lines.len() == 1 => {
                let title = lines[0].clone();
                blocks.remove(0);
                title
            }
            _ => Vec::new(),
        };
        if blocks.is_empty() {
            return None;
        }
        let mut hover = Self {
            title,
            blocks,
            rows: Vec::new(),
            width: 0,
            scroll: 0,
        };
        hover.relayout(term_width);
        Some(hover)
    }

    pub fn relayout(&mut self, term_width: u16) {
        let max = MAX_WIDTH
            .min((term_width as usize).saturating_sub(6))
            .max(1);
        let natural = self
            .blocks
            .iter()
            .flat_map(|block| -> Vec<usize> {
                match block {
                    Block::Text(lines) => lines.iter().map(|l| l.plain_text().width()).collect(),
                    Block::Code(lines) => lines.iter().map(|l| spans_width(l)).collect(),
                }
            })
            .max()
            .unwrap_or(0)
            .max(spans_width(&self.title) + 1);
        self.width = natural.clamp(MIN_WIDTH.min(max), max);
        self.scroll = 0;

        let mut rows = Vec::new();
        for (i, block) in self.blocks.iter().enumerate() {
            if i > 0 {
                rows.push(HoverRow::Text(PreviewLine::blank()));
            }
            match block {
                Block::Code(lines) => {
                    for line in lines {
                        rows.extend(wrap_code(line, self.width).into_iter().map(HoverRow::Code));
                    }
                }
                Block::Text(lines) => {
                    rows.extend(
                        wrap_preview_lines(lines, self.width)
                            .into_iter()
                            .map(|line| {
                                if line.kind == PreviewLineKind::Rule {
                                    HoverRow::Rule
                                } else {
                                    HoverRow::Text(line)
                                }
                            }),
                    );
                }
            }
        }
        self.rows = tidy(rows);
    }
}

/// Tabs to spaces at 4-column stops, so gopls's tab-separated tables
/// (`\t%v\tthe value`) keep their columns.
fn expand_tabs(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut col = 0;
    for ch in text.chars() {
        match ch {
            '\t' => {
                let spaces = 4 - col % 4;
                out.extend(std::iter::repeat_n(' ', spaces));
                col += spaces;
            }
            '\n' => {
                out.push(ch);
                col = 0;
            }
            _ => {
                out.push(ch);
                col += ch.width().unwrap_or(0);
            }
        }
    }
    out
}

fn parse(markdown: &str, theme: &Theme, path: Option<&Path>) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut text = String::new();
    let mut lines = markdown.lines();
    while let Some(line) = lines.next() {
        let Some(lang) = line.trim_start().strip_prefix("```") else {
            text.push_str(line);
            text.push('\n');
            continue;
        };
        push_text(&mut blocks, &mut text);
        let code: Vec<&str> = lines
            .by_ref()
            .take_while(|l| !l.trim_start().starts_with("```"))
            .collect();
        if !code.is_empty() {
            blocks.push(Block::Code(highlight(lang.trim(), &code, theme, path)));
        }
    }
    push_text(&mut blocks, &mut text);
    blocks
}

fn push_text(blocks: &mut Vec<Block>, text: &mut String) {
    if !text.trim().is_empty() {
        blocks.push(Block::Text(join_paragraphs(render_markdown(text).lines)));
    }
    text.clear();
}

/// Markdown joins a paragraph's source lines; servers hard-wrap their docs
/// at ~80 columns, so without this every source line would be its own row.
/// Indented lines are preformatted and stay as they are.
fn join_paragraphs(lines: Vec<PreviewLine>) -> Vec<PreviewLine> {
    let flows = |line: &PreviewLine| {
        line.kind == PreviewLineKind::Paragraph
            && !line.plain_text().starts_with(char::is_whitespace)
    };
    let mut out: Vec<PreviewLine> = Vec::new();
    for line in lines {
        if let Some(prev) = out.last_mut().filter(|prev| flows(prev) && flows(&line)) {
            if let Some(last) = prev.spans.last_mut() {
                last.text.truncate(last.text.trim_end().len());
            }
            prev.spans.push(PreviewSpan::plain(" "));
            prev.spans.extend(line.spans);
            continue;
        }
        out.push(line);
    }
    out
}

// ponytail: a fresh SyntaxManager compiles its highlight query on every
// hover, about 15ms in release for Rust or TypeScript, once per `K` reply.
// Keep one per language if that ever shows up next to the LSP round trip.
fn highlight(lang: &str, code: &[&str], theme: &Theme, path: Option<&Path>) -> Vec<Vec<CodeSpan>> {
    let mut syntax = SyntaxManager::new();
    let ext = match lang {
        "rust" => "rs",
        "typescript" => "ts",
        "javascript" => "js",
        "python" => "py",
        "ruby" => "rb",
        "bash" | "shell" | "zsh" => "sh",
        "typst" => "typ",
        other => other,
    };
    if !ext.is_empty() {
        syntax.set_language_from_path(Path::new(&format!("hover.{ext}")));
    } else if let Some(path) = path {
        syntax.set_language_from_path(path);
    }
    syntax.sync_theme(theme);
    syntax.parse_string(&code.join("\n"));
    code.iter()
        .enumerate()
        .map(|(i, line)| styled_line(line, &syntax.get_line_highlights(i)))
        .collect()
}

fn styled_line(line: &str, highlights: &[HighlightSpan]) -> Vec<CodeSpan> {
    let chars: Vec<char> = line.chars().collect();
    let mut styles = vec![None; chars.len()];
    for span in highlights {
        for style in styles
            .iter_mut()
            .take(span.end_col.min(chars.len()))
            .skip(span.start_col)
        {
            *style = Some(span.style);
        }
    }
    let mut spans = Vec::new();
    for (ch, style) in chars.into_iter().zip(styles) {
        push_char(&mut spans, ch, style);
    }
    spans
}

fn push_char(spans: &mut Vec<CodeSpan>, ch: char, style: Option<SyntaxStyle>) {
    match spans.last_mut() {
        Some(last) if last.style == style => last.text.push(ch),
        _ => spans.push(CodeSpan {
            text: ch.to_string(),
            style,
        }),
    }
}

pub fn spans_width(spans: &[CodeSpan]) -> usize {
    spans.iter().map(|span| span.text.width()).sum()
}

/// Code keeps its spacing, so it breaks at the edge instead of at words.
fn wrap_code(line: &[CodeSpan], width: usize) -> Vec<Vec<CodeSpan>> {
    let mut rows = vec![Vec::new()];
    let mut used = 0;
    for span in line {
        for ch in span.text.chars() {
            let w = ch.width().unwrap_or(0);
            if used + w > width && used > 0 {
                rows.push(Vec::new());
                used = 0;
            }
            if let Some(row) = rows.last_mut() {
                push_char(row, ch, span.style);
            }
            used += w;
        }
    }
    rows
}

/// One blank row between blocks at most, none next to a rule, and nothing
/// blank at the top or bottom.
fn tidy(rows: Vec<HoverRow>) -> Vec<HoverRow> {
    let blank =
        |row: &HoverRow| matches!(row, HoverRow::Text(line) if line.plain_text().trim().is_empty());
    let mut out: Vec<HoverRow> = Vec::new();
    for row in rows {
        if blank(&row)
            && out
                .last()
                .is_none_or(|last| blank(last) || *last == HoverRow::Rule)
        {
            continue;
        }
        if row == HoverRow::Rule {
            while out.last().is_some_and(blank) {
                out.pop();
            }
            if out.is_empty() || out.last() == Some(&HoverRow::Rule) {
                continue;
            }
        }
        out.push(row);
    }
    while out
        .last()
        .is_some_and(|last| blank(last) || *last == HoverRow::Rule)
    {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hover(markdown: &str, term_width: u16) -> Hover {
        Hover::new(markdown, &Theme::default(), None, term_width).expect("hover")
    }

    fn texts(hover: &Hover) -> Vec<String> {
        hover
            .rows
            .iter()
            .map(|row| match row {
                HoverRow::Text(line) => line.plain_text(),
                HoverRow::Code(spans) => spans.iter().map(|s| s.text.as_str()).collect(),
                HoverRow::Rule => "---".to_string(),
            })
            .collect()
    }

    #[test]
    fn one_line_signature_becomes_the_title_and_docs_reflow() {
        let hover = hover(
            "```go\npackage fmt\n```\n\nPackage fmt implements formatted I/O with functions analogous\nto C's printf and scanf.\n\n# Printing\n\n[Print], [Println] and [Printf] write to [os.Stdout];",
            100,
        );

        assert_eq!(
            hover
                .title
                .iter()
                .map(|s| s.text.as_str())
                .collect::<String>(),
            "package fmt"
        );
        assert_eq!(
            texts(&hover),
            [
                "Package fmt implements formatted I/O with functions analogous to C's printf",
                "and scanf.",
                "",
                "Printing",
                "",
                "Print, Println and Printf write to os.Stdout;",
            ]
        );
        assert!(texts(&hover).iter().all(|row| row.width() <= hover.width));
    }

    #[test]
    fn a_lone_code_block_stays_in_the_body() {
        let hover = hover("```go\nvar x int\n```", 100);

        assert!(hover.title.is_empty());
        assert_eq!(texts(&hover), ["var x int"]);
    }

    #[test]
    fn code_is_highlighted_by_its_fence_language_and_breaks_at_the_edge() {
        let long = format!("pub fn demo(value: ExampleType) -> {}", "u8".repeat(30));
        let hover = hover(&format!("```rust\nuse demo;\n{long}\n```\n---\nDocs."), 40);

        assert!(hover.title.is_empty());
        assert_eq!(hover.width, 34);
        let HoverRow::Code(signature) = &hover.rows[1] else {
            panic!("expected code, got {:?}", hover.rows[1]);
        };
        assert!(
            signature
                .iter()
                .any(|s| s.text == "pub" && s.style.is_some())
        );
        let rows = texts(&hover);
        assert_eq!(rows[1].width(), 34);
        assert!(rows.iter().all(|row| row.width() <= 34));
        assert_eq!(rows[1..rows.len() - 2].concat(), long);
        assert_eq!(rows[rows.len() - 2..], ["---", "Docs."]);
    }

    #[test]
    fn rules_and_blanks_never_stack_or_trail() {
        let hover = hover(
            "```rust\nmod a\n```\n\n```rust\nfn b()\n```\n\n---\n\n\nDocs\n\n---\n",
            100,
        );

        assert_eq!(texts(&hover), ["fn b()", "---", "Docs"]);
    }

    #[test]
    fn indented_lines_stay_preformatted() {
        let hover = hover(
            "Verbs:\n\n\t%v\tthe value\n\t%#v\ta Go value\n\t\tmore",
            100,
        );

        assert_eq!(
            texts(&hover),
            [
                "Verbs:",
                "",
                "    %v  the value",
                "    %#v a Go value",
                "        more"
            ]
        );
    }

    #[test]
    fn relayout_rewraps_to_the_new_width_and_resets_the_page() {
        let mut hover = hover(&"word ".repeat(60), 200);
        hover.scroll = 3;
        let wide = hover.rows.len();

        hover.relayout(40);

        assert_eq!(hover.width, 34);
        assert!(hover.rows.len() > wide);
        assert_eq!(hover.scroll, 0);
    }

    #[test]
    fn empty_replies_make_no_hover() {
        assert!(Hover::new("  \n\n", &Theme::default(), None, 100).is_none());
    }
}
