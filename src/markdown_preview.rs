#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewSpanStyle {
    Plain,
    Emphasis,
    Strong,
    InlineCode,
    Link,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewLineKind {
    Blank,
    Paragraph,
    Heading(u8),
    Quote,
    ListItem,
    Rule,
    CodeBlock,
    Placeholder,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewSpan {
    pub text: String,
    pub style: PreviewSpanStyle,
}

impl PreviewSpan {
    pub(crate) fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: PreviewSpanStyle::Plain,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewLine {
    pub kind: PreviewLineKind,
    pub spans: Vec<PreviewSpan>,
}

impl PreviewLine {
    pub(crate) fn blank() -> Self {
        Self::from_text(PreviewLineKind::Blank, "")
    }

    fn from_text(kind: PreviewLineKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            spans: vec![PreviewSpan::plain(text)],
        }
    }

    fn from_spans(kind: PreviewLineKind, spans: Vec<PreviewSpan>) -> Self {
        Self { kind, spans }
    }

    pub fn plain_text(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownPreview {
    pub lines: Vec<PreviewLine>,
}

#[derive(Debug, Clone)]
pub struct MarkdownPreviewState {
    pub title: String,
    pub lines: Vec<PreviewLine>,
    display_lines: Vec<PreviewLine>,
    pub scroll: usize,
    wrap_width: usize,
}

impl MarkdownPreviewState {
    pub fn new(preview: MarkdownPreview, width: usize) -> Self {
        Self::with_title(preview, width, "Markdown Preview")
    }

    pub fn with_title(preview: MarkdownPreview, width: usize, title: impl Into<String>) -> Self {
        let display_lines = wrap_preview_lines(&preview.lines, width);
        Self {
            title: title.into(),
            lines: preview.lines,
            display_lines,
            scroll: 0,
            wrap_width: width,
        }
    }

    pub fn display_lines(&self) -> &[PreviewLine] {
        &self.display_lines
    }

    pub fn max_scroll(&self, visible_rows: usize) -> usize {
        self.display_lines.len().saturating_sub(visible_rows)
    }

    pub fn reflow(&mut self, width: usize) {
        if self.wrap_width == width {
            return;
        }

        self.display_lines = wrap_preview_lines(&self.lines, width);
        self.wrap_width = width;
    }
}

pub fn preview_popup_width(term_width: u16) -> u16 {
    let preferred_width = term_width.saturating_mul(9) / 10;
    let max_width = term_width.saturating_sub(4);
    preferred_width.min(max_width).max(term_width.min(20))
}

pub fn preview_content_width(term_width: u16) -> usize {
    preview_popup_width(term_width).saturating_sub(2) as usize
}

pub fn render_markdown(source: &str) -> MarkdownPreview {
    if source.trim().is_empty() {
        return MarkdownPreview {
            lines: vec![PreviewLine::from_text(
                PreviewLineKind::Placeholder,
                "Nothing to preview yet.",
            )],
        };
    }

    let mut lines = Vec::new();
    let mut in_code_block = false;

    for raw_line in source.lines() {
        let line = raw_line.trim_end_matches('\r');
        let trimmed = line.trim();

        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }

        if in_code_block {
            lines.push(PreviewLine::from_text(PreviewLineKind::CodeBlock, line));
            continue;
        }

        if trimmed.is_empty() {
            lines.push(PreviewLine::from_text(PreviewLineKind::Blank, ""));
            continue;
        }

        if trimmed == "---" || trimmed == "***" || trimmed == "___" {
            lines.push(PreviewLine::from_text(PreviewLineKind::Rule, ""));
            continue;
        }

        if let Some((level, text)) = parse_heading(trimmed) {
            lines.push(PreviewLine::from_spans(
                PreviewLineKind::Heading(level),
                parse_inline(text),
            ));
            continue;
        }

        if let Some(text) = trimmed.strip_prefix('>') {
            lines.push(PreviewLine::from_spans(
                PreviewLineKind::Quote,
                parse_inline(text.trim_start()),
            ));
            continue;
        }

        if let Some(text) = parse_task_item(trimmed) {
            lines.push(PreviewLine::from_spans(
                PreviewLineKind::ListItem,
                parse_inline(&text),
            ));
            continue;
        }

        if let Some(text) = parse_unordered_item(trimmed) {
            lines.push(PreviewLine::from_spans(
                PreviewLineKind::ListItem,
                parse_inline(&text),
            ));
            continue;
        }

        if let Some(text) = parse_ordered_item(trimmed) {
            lines.push(PreviewLine::from_spans(
                PreviewLineKind::ListItem,
                parse_inline(&text),
            ));
            continue;
        }

        lines.push(PreviewLine::from_spans(
            PreviewLineKind::Paragraph,
            parse_inline(line),
        ));
    }

    MarkdownPreview { lines }
}

fn parse_heading(line: &str) -> Option<(u8, &str)> {
    let level = line.chars().take_while(|ch| *ch == '#').count();
    if level == 0 || level > 6 {
        return None;
    }

    let rest = line.get(level..)?.trim_start();
    Some((level as u8, rest))
}

fn parse_task_item(line: &str) -> Option<String> {
    for (prefix, marker) in [("- [x] ", "☑ "), ("- [X] ", "☑ "), ("- [ ] ", "☐ ")] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return Some(format!("{marker}{rest}"));
        }
    }
    None
}

fn parse_unordered_item(line: &str) -> Option<String> {
    for prefix in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return Some(format!("• {rest}"));
        }
    }
    None
}

fn parse_ordered_item(line: &str) -> Option<String> {
    let dot = line.find(". ")?;
    let (number, rest) = line.split_at(dot);
    if number.chars().all(|ch| ch.is_ascii_digit()) {
        Some(format!("{}. {}", number, &rest[2..]))
    } else {
        None
    }
}

fn parse_inline(text: &str) -> Vec<PreviewSpan> {
    let mut spans = Vec::new();
    let mut chars = text.chars().peekable();
    let mut plain = String::new();

    while let Some(ch) = chars.next() {
        match ch {
            '`' => {
                flush_plain(&mut spans, &mut plain);
                let mut code = String::new();
                while let Some(next) = chars.next() {
                    if next == '`' {
                        break;
                    }
                    code.push(next);
                }
                spans.push(PreviewSpan {
                    text: code,
                    style: PreviewSpanStyle::InlineCode,
                });
            }
            '*' if chars.peek() == Some(&'*') => {
                chars.next();
                flush_plain(&mut spans, &mut plain);
                let mut strong = String::new();
                while let Some(next) = chars.next() {
                    if next == '*' && chars.peek() == Some(&'*') {
                        chars.next();
                        break;
                    }
                    strong.push(next);
                }
                spans.push(PreviewSpan {
                    text: strong,
                    style: PreviewSpanStyle::Strong,
                });
            }
            '*' => {
                flush_plain(&mut spans, &mut plain);
                let mut emphasis = String::new();
                while let Some(next) = chars.next() {
                    if next == '*' {
                        break;
                    }
                    emphasis.push(next);
                }
                spans.push(PreviewSpan {
                    text: emphasis,
                    style: PreviewSpanStyle::Emphasis,
                });
            }
            // gopls escapes its doc links (`\[Println]`); let the `[` below
            // decide, so they read like the unescaped ones.
            '\\' if chars.peek() == Some(&'[') => {}
            '\\' if chars.peek().is_some_and(char::is_ascii_punctuation) => {
                plain.extend(chars.next());
            }
            '[' => {
                let after_word = plain
                    .chars()
                    .last()
                    .or_else(|| spans.last().and_then(|span| span.text.chars().last()))
                    .is_some_and(|prev| prev.is_alphanumeric() || prev == '_');
                let mut label = String::new();
                let mut closed = false;
                for next in chars.by_ref() {
                    if next == ']' {
                        closed = true;
                        break;
                    }
                    label.push(next);
                }
                // `[label](url)` and `[label][ref]` show the label. A bare
                // `[Name]` is a doc link the way gopls writes them (`[os.Stdout]`),
                // but not after a word (`a[i]`) and not with spaces inside.
                let is_link = match chars.peek().copied() {
                    Some(open @ ('(' | '[')) if closed => {
                        chars.next();
                        let close = if open == '(' { ')' } else { ']' };
                        chars.by_ref().find(|next| *next == close);
                        true
                    }
                    _ => {
                        closed
                            && !after_word
                            && !label.is_empty()
                            && !label.contains(char::is_whitespace)
                    }
                };
                if is_link {
                    flush_plain(&mut spans, &mut plain);
                    spans.push(PreviewSpan {
                        // gopls labels its pkg.go.dev link "`fmt.Println` on ...".
                        text: label.replace('`', ""),
                        style: PreviewSpanStyle::Link,
                    });
                } else {
                    plain.push('[');
                    plain.push_str(&label);
                    if closed {
                        plain.push(']');
                    }
                }
            }
            _ => plain.push(ch),
        }
    }

    flush_plain(&mut spans, &mut plain);
    spans
}

fn flush_plain(spans: &mut Vec<PreviewSpan>, plain: &mut String) {
    if !plain.is_empty() {
        spans.push(PreviewSpan::plain(std::mem::take(plain)));
    }
}

#[derive(Debug, Clone, Copy)]
struct StyledChar {
    ch: char,
    style: PreviewSpanStyle,
}

pub fn wrap_preview_lines(lines: &[PreviewLine], width: usize) -> Vec<PreviewLine> {
    lines
        .iter()
        .flat_map(|line| wrap_preview_line(line, width))
        .collect()
}

fn wrap_preview_line(line: &PreviewLine, width: usize) -> Vec<PreviewLine> {
    use unicode_width::UnicodeWidthChar;

    if width == 0
        || matches!(
            line.kind,
            PreviewLineKind::Blank | PreviewLineKind::Rule | PreviewLineKind::CodeBlock
        )
    {
        return vec![line.clone()];
    }

    let mut remaining = styled_chars(line);
    if remaining.is_empty() {
        return vec![line.clone()];
    }

    let continuation_indent = continuation_indent(line);
    let continuation_width = continuation_indent.chars().count().min(width);
    let continuation_prefix = vec![
        StyledChar {
            ch: ' ',
            style: PreviewSpanStyle::Plain,
        };
        continuation_width
    ];

    let mut wrapped = Vec::new();
    let mut first_row = true;

    while !remaining.is_empty() {
        let prefix = if first_row {
            Vec::new()
        } else {
            continuation_prefix.clone()
        };
        let prefix_width = prefix
            .iter()
            .map(|styled| UnicodeWidthChar::width(styled.ch).unwrap_or(0))
            .sum::<usize>();
        let available_width = width.saturating_sub(prefix_width).max(1);

        let mut consumed_width = 0usize;
        let mut fit_count = 0usize;
        let mut last_whitespace = None;

        for (idx, styled) in remaining.iter().enumerate() {
            let char_width = UnicodeWidthChar::width(styled.ch).unwrap_or(0);
            if consumed_width.saturating_add(char_width) > available_width {
                break;
            }

            consumed_width += char_width;
            fit_count = idx + 1;
            if styled.ch.is_whitespace() {
                last_whitespace = Some(idx);
            }
        }

        let (line_count, mut consume_count) = if fit_count == remaining.len() {
            (fit_count, fit_count)
        } else if let Some(last_whitespace) = last_whitespace.filter(|idx| *idx > 0) {
            (last_whitespace, last_whitespace + 1)
        } else {
            let forced = fit_count.max(1);
            (forced, forced)
        };

        while consume_count < remaining.len() && remaining[consume_count].ch.is_whitespace() {
            consume_count += 1;
        }

        let mut row = prefix;
        row.extend_from_slice(&remaining[..line_count]);
        wrapped.push(PreviewLine::from_spans(
            line.kind,
            spans_from_styled_chars(row),
        ));

        remaining.drain(..consume_count);
        first_row = false;
    }

    wrapped
}

fn continuation_indent(line: &PreviewLine) -> String {
    match line.kind {
        PreviewLineKind::ListItem => {
            let text = line.plain_text();
            let indent = text
                .chars()
                .position(|ch| ch == ' ')
                .map(|idx| idx + 1)
                .unwrap_or(2);
            " ".repeat(indent)
        }
        PreviewLineKind::Quote => "  ".to_string(),
        _ => String::new(),
    }
}

fn styled_chars(line: &PreviewLine) -> Vec<StyledChar> {
    line.spans
        .iter()
        .flat_map(|span| {
            span.text.chars().map(move |ch| StyledChar {
                ch,
                style: span.style,
            })
        })
        .collect()
}

fn spans_from_styled_chars(chars: Vec<StyledChar>) -> Vec<PreviewSpan> {
    let mut spans: Vec<PreviewSpan> = Vec::new();

    for styled in chars {
        if let Some(last) = spans.last_mut() {
            if last.style == styled.style {
                last.text.push(styled.ch);
                continue;
            }
        }

        spans.push(PreviewSpan {
            text: styled.ch.to_string(),
            style: styled.style,
        });
    }

    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_common_block_markdown_into_readable_lines() {
        let preview = render_markdown(
            "# Title\n\n> note\n\n- first\n1. second\n- [x] done\n\n---\n\n```rust\nfn main() {}\n```",
        );

        assert_eq!(preview.lines[0].kind, PreviewLineKind::Heading(1));
        assert_eq!(preview.lines[0].plain_text(), "Title");
        assert_eq!(preview.lines[2].kind, PreviewLineKind::Quote);
        assert_eq!(preview.lines[4].plain_text(), "• first");
        assert_eq!(preview.lines[5].plain_text(), "1. second");
        assert_eq!(preview.lines[6].plain_text(), "☑ done");
        assert_eq!(preview.lines[8].kind, PreviewLineKind::Rule);
        assert_eq!(preview.lines[10].kind, PreviewLineKind::CodeBlock);
        assert_eq!(preview.lines[10].plain_text(), "fn main() {}");
    }

    #[test]
    fn renders_inline_markup_as_styled_spans() {
        let preview =
            render_markdown("Read **bold**, *soft*, `code`, and [docs](https://example.com).");
        let spans = &preview.lines[0].spans;

        assert!(
            spans
                .iter()
                .any(|span| span.text == "bold" && span.style == PreviewSpanStyle::Strong)
        );
        assert!(
            spans
                .iter()
                .any(|span| span.text == "soft" && span.style == PreviewSpanStyle::Emphasis)
        );
        assert!(
            spans
                .iter()
                .any(|span| span.text == "code" && span.style == PreviewSpanStyle::InlineCode)
        );
        assert!(
            spans
                .iter()
                .any(|span| span.text == "docs" && span.style == PreviewSpanStyle::Link)
        );
    }

    #[test]
    fn doc_links_show_their_label_and_brackets_after_a_word_stay() {
        let line = |source: &str| render_markdown(source).lines[0].plain_text();

        assert_eq!(
            line("[Print], [Println] and [Printf] write to [os.Stdout];"),
            "Print, Println and Printf write to os.Stdout;"
        );
        assert_eq!(line("see [the docs][ref] now"), "see the docs now");
        assert_eq!(
            line("index a[i] or [two words]"),
            "index a[i] or [two words]"
        );
        assert_eq!(line("unclosed [bracket"), "unclosed [bracket");
        assert_eq!(line(r"not \*emphasis\* and a\_b"), "not *emphasis* and a_b");
        assert_eq!(
            line(
                r"gopls: \[Print] and \[Println] write to [os.Stdout](https://pkg.go.dev/os#Stdout); a\[i]"
            ),
            "gopls: Print and Println write to os.Stdout; a[i]"
        );
        assert_eq!(
            line("[`fmt.Println` on pkg.go.dev](https://pkg.go.dev/fmt#Println)"),
            "fmt.Println on pkg.go.dev"
        );
    }

    #[test]
    fn empty_documents_render_a_placeholder_line() {
        let preview = render_markdown("");

        assert_eq!(preview.lines.len(), 1);
        assert_eq!(preview.lines[0].plain_text(), "Nothing to preview yet.");
        assert_eq!(preview.lines[0].kind, PreviewLineKind::Placeholder);
    }

    #[test]
    fn unsupported_markdown_stays_readable_instead_of_failing() {
        let preview = render_markdown("| a | b |\n|---|---|");

        assert_eq!(preview.lines[0].plain_text(), "| a | b |");
        assert_eq!(preview.lines[1].plain_text(), "|---|---|");
    }

    #[test]
    fn wraps_prose_at_word_boundaries_for_the_preview_width() {
        let preview = render_markdown("alpha beta gamma delta");
        let wrapped = wrap_preview_lines(&preview.lines, 12);
        let text: Vec<_> = wrapped.iter().map(PreviewLine::plain_text).collect();

        assert_eq!(text, vec!["alpha beta", "gamma delta"]);
    }

    #[test]
    fn wraps_list_continuations_with_indent_but_keeps_code_blocks_unwrapped() {
        let preview = render_markdown("- alpha beta gamma\n\n```txt\nalpha beta gamma\n```");
        let wrapped = wrap_preview_lines(&preview.lines, 10);
        let text: Vec<_> = wrapped.iter().map(PreviewLine::plain_text).collect();

        assert_eq!(
            text,
            vec!["• alpha", "  beta", "  gamma", "", "alpha beta gamma"]
        );
    }

    #[test]
    fn wrapped_rows_drive_scroll_limits() {
        let preview = MarkdownPreviewState::new(render_markdown("alpha beta gamma"), 10);

        assert_eq!(preview.max_scroll(1), 1);
        assert_eq!(preview.max_scroll(2), 0);
    }

    #[test]
    fn preview_state_supports_custom_titles() {
        let default_preview = MarkdownPreviewState::new(render_markdown("alpha"), 20);
        assert_eq!(default_preview.title, "Markdown Preview");

        let health_preview =
            MarkdownPreviewState::with_title(render_markdown("# Nevi Health"), 20, "Health");
        assert_eq!(health_preview.title, "Health");
    }

    #[test]
    fn reflows_cached_rows_when_the_preview_width_changes() {
        let mut preview = MarkdownPreviewState::new(render_markdown("alpha beta gamma"), 20);
        assert_eq!(preview.display_lines().len(), 1);

        preview.reflow(10);
        assert_eq!(preview.display_lines().len(), 2);
    }

    #[test]
    fn large_documents_render_without_special_cases_or_extra_dependencies() {
        let source = "# Heading\n\nparagraph\n".repeat(10_000);
        let preview = render_markdown(&source);

        assert_eq!(preview.lines.len(), 30_000);
        assert_eq!(preview.lines[0].plain_text(), "Heading");
    }
}
