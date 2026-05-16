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
    fn plain(text: impl Into<String>) -> Self {
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
    pub lines: Vec<PreviewLine>,
    pub scroll: usize,
}

impl MarkdownPreviewState {
    pub fn new(preview: MarkdownPreview) -> Self {
        Self {
            lines: preview.lines,
            scroll: 0,
        }
    }

    pub fn max_scroll(&self, visible_rows: usize) -> usize {
        self.lines.len().saturating_sub(visible_rows)
    }
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
            '[' => {
                flush_plain(&mut spans, &mut plain);
                let mut label = String::new();
                while let Some(next) = chars.next() {
                    if next == ']' {
                        break;
                    }
                    label.push(next);
                }
                if chars.next() == Some('(') {
                    while let Some(next) = chars.next() {
                        if next == ')' {
                            break;
                        }
                    }
                    spans.push(PreviewSpan {
                        text: label,
                        style: PreviewSpanStyle::Link,
                    });
                } else {
                    plain.push('[');
                    plain.push_str(&label);
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

        assert!(spans
            .iter()
            .any(|span| span.text == "bold" && span.style == PreviewSpanStyle::Strong));
        assert!(spans
            .iter()
            .any(|span| span.text == "soft" && span.style == PreviewSpanStyle::Emphasis));
        assert!(spans
            .iter()
            .any(|span| span.text == "code" && span.style == PreviewSpanStyle::InlineCode));
        assert!(spans
            .iter()
            .any(|span| span.text == "docs" && span.style == PreviewSpanStyle::Link));
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
    fn large_documents_render_without_special_cases_or_extra_dependencies() {
        let source = "# Heading\n\nparagraph\n".repeat(10_000);
        let preview = render_markdown(&source);

        assert_eq!(preview.lines.len(), 30_000);
        assert_eq!(preview.lines[0].plain_text(), "Heading");
    }
}
