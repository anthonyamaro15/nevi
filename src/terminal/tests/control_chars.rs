//! Text from outside the editor (file names, language server replies, status
//! messages) must never reach the terminal with its control characters: an ESC
//! in it would start an escape sequence that sets the title, writes the
//! clipboard or redraws the screen. Each control character draws as one
//! blank instead, the way buffer text does, and popups still line up.

use super::{render_editor_to_string, screen_row_text};
use crate::editor::{Editor, Mode};
use crate::hover::Hover;
use crate::lsp::path_to_uri;
use crate::lsp::types::{
    CompletionItem, CompletionKind, Diagnostic, DiagnosticSeverity, DiagnosticTags, ParameterInfo,
    SignatureHelpResult, SignatureInfo,
};
use std::path::PathBuf;

/// Sets the terminal title when written raw.
const PAYLOAD: &str = "\x1b]0;PWN\x07";
const RAW: &str = "\x1b]0;PWN";
/// The payload on screen: ESC and BEL each become one blank.
const SHOWN: &str = " ]0;PWN ";

const WIDTH: u16 = 100;
const HEIGHT: u16 = 30;

fn editor() -> Editor {
    let mut editor = Editor::default();
    editor.set_size(WIDTH, HEIGHT);
    editor
}

/// Renders a frame, checks the payload never went out raw, and returns the
/// screen rows as text.
fn screen(editor: &Editor) -> Vec<String> {
    let rendered = render_editor_to_string(editor);
    assert!(
        !rendered.contains(RAW),
        "the escape sequence reached the terminal"
    );
    (0..HEIGHT as usize)
        .map(|row| screen_row_text(&rendered, row))
        .collect()
}

fn row_with<'a>(screen: &'a [String], text: &str) -> &'a str {
    screen
        .iter()
        .find(|row| row.contains(text))
        .unwrap_or_else(|| panic!("{text:?} not on screen:\n{}", screen.join("\n")))
}

/// Column of the last cell drawn on `row`.
fn right_edge(row: &str) -> usize {
    row.chars().count() - 1
}

/// Every row of the popup whose top border holds `top_left` must end in the
/// same column as that border.
fn assert_popup_aligned(screen: &[String], top_left: char, bottom_left: char) {
    let top = screen
        .iter()
        .position(|row| row.contains(top_left))
        .expect("popup top border");
    let bottom = top
        + screen[top..]
            .iter()
            .position(|row| row.contains(bottom_left))
            .expect("popup bottom border");
    let edge = right_edge(&screen[top]);
    for row in &screen[top..=bottom] {
        assert_eq!(
            right_edge(row),
            edge,
            "popup edge moved on {row:?}:\n{}",
            screen.join("\n")
        );
    }
}

/// A fresh directory named after the test, so parallel tests never share one.
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nevi-control-chars-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn status_and_error_messages_draw_control_characters_as_blanks() {
    let mut editor = editor();
    editor.set_status(format!("hello {PAYLOAD} world"));
    let last = screen(&editor).pop().unwrap();
    assert_eq!(last, format!("hello {SHOWN} world"));

    editor.set_error(format!("E: bad {PAYLOAD} thing"));
    let last = screen(&editor).pop().unwrap();
    assert_eq!(last, format!("E: bad {SHOWN} thing"));
}

/// A newline draws as a blank, so it takes a cell: a message one cell too
/// wide is cut instead of spilling past the last column.
#[test]
fn status_message_counts_a_newline_as_a_cell() {
    let mut editor = editor();
    let msg = format!("{}\nb", "a".repeat(WIDTH as usize - 1));
    editor.set_status(msg);
    let last = screen(&editor).pop().unwrap();
    assert!(last.ends_with('…'), "{last:?}");
    assert!(last.chars().count() <= WIDTH as usize, "{last:?}");
}

#[test]
fn file_names_in_the_statusline_and_bufferline_draw_control_characters_as_blanks() {
    let dir = temp_dir("names");
    // Same length as the hostile name, so the statusline must match it cell
    // for cell. The newline would otherwise count as no width at all.
    let clean = dir.join("aXXXXXXXXXb.txt");
    let hostile = dir.join(format!("a{PAYLOAD}\nb.txt"));
    std::fs::write(&clean, "x\n").unwrap();
    std::fs::write(&hostile, "x\n").unwrap();

    let with_bufferline = |path: PathBuf| {
        let mut editor = Editor::default();
        editor.settings.editor.bufferline = true;
        editor.set_size(WIDTH, HEIGHT);
        editor.open_file(path).unwrap();
        screen(&editor)
    };
    let clean_status = with_bufferline(clean)[HEIGHT as usize - 2].clone();
    let screen = with_bufferline(hostile);
    let shown = format!("a{SHOWN} b.txt");
    assert!(screen[0].contains(&shown), "bufferline: {:?}", screen[0]);
    let status = &screen[HEIGHT as usize - 2];
    assert!(status.contains(&shown), "statusline: {status:?}");
    assert_eq!(status.chars().count(), clean_status.chars().count());

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn file_names_in_the_explorer_draw_control_characters_as_blanks() {
    let dir = temp_dir("explorer");
    std::fs::write(dir.join(format!("a{PAYLOAD}b.txt")), "x\n").unwrap();

    let mut editor = editor();
    editor.set_project_root(dir.clone());
    editor.open_explorer();
    row_with(&screen(&editor), &format!("a{SHOWN}b.txt"));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn diagnostics_inline_and_in_the_float_draw_control_characters_as_blanks() {
    let dir = temp_dir("diagnostics");
    let path = dir.join("main.rs");
    std::fs::write(&path, "fn main() {}\nlet x = 1;\n").unwrap();

    let mut editor = editor();
    editor.open_file(path.clone()).unwrap();
    editor.set_diagnostics(
        path_to_uri(&path),
        vec![Diagnostic {
            line: 0,
            end_line: 0,
            col_start: 0,
            col_end: 2,
            severity: DiagnosticSeverity::Error,
            message: format!("bad {PAYLOAD} thing\nsecond\tline"),
            source: Some(format!("src{PAYLOAD}")),
            code: None,
            tags: DiagnosticTags::default(),
        }],
    );
    let screen_inline = screen(&editor);
    assert!(
        screen_inline[0].contains(&format!("bad {SHOWN} thing")),
        "{:?}",
        screen_inline[0]
    );

    editor.show_diagnostic_float = true;
    let screen = screen(&editor);
    row_with(&screen, "Diagnostics:");
    row_with(&screen, "second line");
    assert_popup_aligned(&screen, '╭', '╰');

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn hover_draws_control_characters_as_blanks_and_keeps_its_border() {
    let mut editor = editor();
    editor.replace_buffer_content(&"line\n".repeat(40));
    editor.cursor.line = 25;
    editor.hover = Hover::for_editor(
        &editor,
        &format!("```go\nfunc {PAYLOAD}x\n```\n\ndocs {PAYLOAD} here"),
    );
    let screen = screen(&editor);
    row_with(&screen, &format!("func {SHOWN}x"));
    row_with(&screen, &format!("docs {SHOWN} here"));
    assert_popup_aligned(&screen, '╭', '╰');
}

#[test]
fn completion_label_detail_and_docs_draw_control_characters_as_blanks() {
    let mut editor = editor();
    editor.replace_buffer_content("fo\n");
    editor.mode = Mode::Insert;
    editor.cursor.col = 2;
    editor.show_completions(
        vec![CompletionItem {
            item_id: 1,
            label: format!("fo{PAYLOAD}o"),
            kind: CompletionKind::Function,
            detail: Some(format!("det{PAYLOAD}ail")),
            documentation: Some(format!("doc{PAYLOAD}s")),
            insert_text: None,
            filter_text: Some("foo".into()),
            sort_text: None,
            text_edit: None,
            additional_text_edits: vec![],
            raw_data: None,
        }],
        0,
        0,
        false,
    );
    let screen = screen(&editor);
    row_with(&screen, &format!("fo{SHOWN}o"));
    row_with(&screen, &format!("det{SHOWN}ail"));
    row_with(&screen, &format!("doc{SHOWN}s"));
}

#[test]
fn signature_help_draws_control_characters_as_blanks() {
    let mut editor = editor();
    editor.replace_buffer_content("foo(\n");
    editor.mode = Mode::Insert;
    editor.cursor.col = 4;
    editor.signature_help = Some(SignatureHelpResult {
        signatures: vec![SignatureInfo {
            label: format!("fn foo(a{PAYLOAD}: i32)"),
            documentation: Some(format!("doc {PAYLOAD}")),
            parameters: vec![ParameterInfo {
                label_offsets: None,
                label: format!("a{PAYLOAD}"),
                documentation: Some(format!("p {PAYLOAD}")),
            }],
        }],
        active_signature: 0,
        active_parameter: Some(0),
    });
    let screen = screen(&editor);
    row_with(&screen, &format!("fn foo(a{SHOWN}: i32)"));
    assert_popup_aligned(&screen, '╭', '╰');
}

#[test]
fn markdown_preview_draws_control_characters_as_blanks_and_keeps_its_border() {
    let dir = temp_dir("markdown");
    let path = dir.join("README.md");
    std::fs::write(
        &path,
        format!(
            "# Title {PAYLOAD}\n\ntext {PAYLOAD} here\n\n`code{PAYLOAD}`\n\n```\nblock {PAYLOAD}\tx\n```\n"
        ),
    )
    .unwrap();

    let mut editor = editor();
    editor.open_file(path).unwrap();
    editor.open_markdown_preview().unwrap();
    let screen = screen(&editor);
    row_with(&screen, &format!("Title {SHOWN}"));
    row_with(&screen, &format!("text {SHOWN} here"));
    row_with(&screen, &format!("code{SHOWN}"));
    row_with(&screen, &format!("block {SHOWN} x"));
    assert_popup_aligned(&screen, '╭', '╰');

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn dashboard_recent_files_draw_control_characters_as_blanks() {
    let dir = temp_dir("dashboard");
    let mut editor = editor();
    editor.recent_files =
        crate::recent_files::RecentFiles::load_from(dir.join("recent_files.json"));
    editor.project_root = Some(dir.clone());
    let hostile = dir.join(format!("r{PAYLOAD}.rs"));
    std::fs::write(&hostile, "x\n").unwrap();
    editor.recent_files.record(&hostile);

    row_with(&screen(&editor), &format!("r{SHOWN}.rs"));

    let _ = std::fs::remove_dir_all(dir);
}
