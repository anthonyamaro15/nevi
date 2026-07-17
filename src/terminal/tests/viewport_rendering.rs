use super::render_editor_to_string;
use crate::editor::Editor;
use crate::input::Motion;

fn rendered_tilde_count(content: &str, wrap: bool) -> usize {
    let mut editor = Editor::default();
    editor.set_size(80, 12);
    editor.settings.editor.wrap = wrap;
    editor.settings.editor.wrap_width = 80;
    editor.replace_buffer_content(content);

    render_editor_to_string(&editor).matches('~').count()
}

#[test]
fn full_render_treats_trailing_empty_rope_line_as_end_of_buffer() {
    for wrap in [false, true] {
        assert_eq!(
            rendered_tilde_count("alpha\nbeta\n", wrap),
            8,
            "wrap={wrap}"
        );
    }
}

#[test]
fn full_render_keeps_real_final_blank_line_visible() {
    for wrap in [false, true] {
        assert_eq!(
            rendered_tilde_count("alpha\nbeta\n\n", wrap),
            7,
            "wrap={wrap}"
        );
    }
}

#[test]
fn full_render_after_wrapped_file_end_shows_context_above_last_line() {
    let mut editor = Editor::default();
    editor.set_size(80, 24);
    editor.settings.editor.scroll_off = 8;
    editor.settings.editor.wrap = true;
    editor.settings.editor.wrap_width = 80;
    let content = (1..=100)
        .map(|line| format!("line {line:03}\n"))
        .collect::<String>();
    editor.replace_buffer_content(&content);
    editor.apply_motion(Motion::FileEnd, 1);

    let rendered = render_editor_to_string(&editor);

    assert!(
        rendered.contains("line 079"),
        "render should keep visible context above the final line; output={rendered:?}"
    );
    assert!(
        rendered.contains("line 100"),
        "render should still include the final line; output={rendered:?}"
    );
}

#[test]
fn full_render_after_wrapped_file_end_shows_tall_final_line_cursor_segment() {
    let mut editor = Editor::default();
    editor.set_size(80, 24);
    editor.settings.editor.scroll_off = 8;
    editor.settings.editor.wrap = true;
    editor.settings.editor.wrap_width = 20;
    editor.replace_buffer_content(&format!("context\nCURSOR_PREFIX{}\n", "x".repeat(5_000)));
    editor.apply_motion(Motion::FileEnd, 1);

    let rendered = render_editor_to_string(&editor);

    assert!(
        rendered.contains("CURSOR_PREFIX"),
        "render should include the final line cursor segment; output={rendered:?}"
    );
}

#[test]
fn full_render_after_wrapped_file_end_in_horizontal_split_shows_final_context() {
    let mut editor = Editor::default();
    editor.set_size(80, 24);
    editor.settings.editor.scroll_off = 8;
    editor.settings.editor.wrap = true;
    editor.settings.editor.wrap_width = 80;
    let content = (1..=100)
        .map(|line| format!("line {line:03}\n"))
        .collect::<String>();
    editor.replace_buffer_content(&content);
    editor.hsplit(None).expect("horizontal split");
    editor.apply_motion(Motion::FileEnd, 1);

    let rendered = render_editor_to_string(&editor);

    assert!(
        rendered.contains("line 090"),
        "active split should show context above EOF; output={rendered:?}"
    );
    assert!(
        rendered.contains("line 100"),
        "active split should show the final cursor line; output={rendered:?}"
    );
}
