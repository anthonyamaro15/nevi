//! A buffer whose parse ran out of time is not parsed again on every change.
//! It waits for a pause in editing, so a held key or a range re-indent pays
//! for one slow parse instead of one per change.

use std::path::Path;
use std::time::Duration;

use crate::editor::Editor;
use crate::syntax::assert_tree_matches_fresh_parse;

/// Every parse runs out of time at once.
const NO_TIME: (Duration, Duration) = (Duration::ZERO, Duration::ZERO);
const NO_LIMIT: (Duration, Duration) = (Duration::from_secs(30), Duration::ZERO);

const UNINDENTED: &str = "fn main() {\nlet x = 1;\nif x > 0 {\ny();\n}\n}\n";

fn parse_attempts(editor: &Editor) -> usize {
    editor
        .flight_recorder
        .events()
        .filter(|event| event.name == "syntax_parse")
        .count()
}

fn rust_editor(budget: (Duration, Duration)) -> Editor {
    let mut editor = Editor::default();
    editor.replace_buffer_content(UNINDENTED);
    editor.syntax.set_language_from_path(Path::new("slow.rs"));
    editor.syntax.set_parse_budget(budget);
    editor.parse_current_buffer_now();
    editor
}

fn timed_out_editor() -> Editor {
    let editor = rust_editor(NO_TIME);
    assert!(editor.syntax.parse_timed_out(editor.buffer().id()));
    editor
}

fn edit(editor: &mut Editor) {
    editor.buffer_mut().insert_char(0, 0, ' ');
}

#[test]
fn eager_updates_skip_a_timed_out_buffer_and_the_pause_retries_once() {
    let mut editor = timed_out_editor();
    let attempts = parse_attempts(&editor);

    for _ in 0..20 {
        edit(&mut editor);
        editor.maybe_update_syntax();
    }
    assert_eq!(parse_attempts(&editor), attempts);

    assert!(editor.maybe_update_syntax_debounced(Duration::ZERO));
    assert_eq!(parse_attempts(&editor), attempts + 1);

    // Still too slow: the next try waits for the next edit.
    assert!(!editor.maybe_update_syntax_debounced(Duration::ZERO));
    assert_eq!(parse_attempts(&editor), attempts + 1);
}

#[test]
fn a_retry_that_fits_brings_the_tree_back_and_eager_parsing_resumes() {
    let mut editor = timed_out_editor();
    editor.syntax.set_parse_budget(NO_LIMIT);
    edit(&mut editor);
    editor.maybe_update_syntax();
    assert!(!editor.syntax.has_highlighting());

    assert!(editor.maybe_update_syntax_debounced(Duration::ZERO));
    assert!(editor.syntax.has_highlighting());
    assert!(!editor.syntax.get_line_highlights(0).is_empty());
    assert_tree_matches_fresh_parse(&editor.syntax, editor.buffer());

    let attempts = parse_attempts(&editor);
    edit(&mut editor);
    editor.maybe_update_syntax();
    assert_eq!(parse_attempts(&editor), attempts + 1);
    assert_tree_matches_fresh_parse(&editor.syntax, editor.buffer());
}

#[test]
fn range_reindent_of_a_timed_out_buffer_parses_nothing_and_uses_the_fallback() {
    // An editor with no language has no tree, so it shows the fallback.
    let mut plain = Editor::default();
    plain.replace_buffer_content(UNINDENTED);
    plain.settings.editor.tab_width = 4;
    plain.auto_indent_lines(0, 5);

    let mut editor = timed_out_editor();
    editor.settings.editor.tab_width = 4;
    let attempts = parse_attempts(&editor);
    editor.auto_indent_lines(0, 5);

    assert_ne!(editor.buffer().content(), UNINDENTED);
    assert_eq!(editor.buffer().content(), plain.buffer().content());
    assert_eq!(parse_attempts(&editor), attempts);
}

#[test]
fn a_language_change_on_a_timed_out_buffer_is_retried_after_the_pause() {
    let mut editor = timed_out_editor();
    editor.syntax.set_parse_budget(NO_LIMIT);
    let attempts = parse_attempts(&editor);

    // Renaming changes no text, so only the timed-out mark asks for a parse.
    editor.set_buffer_path(std::env::temp_dir().join("nevi_renamed_slow.rs"));
    assert_eq!(parse_attempts(&editor), attempts);

    assert!(editor.maybe_update_syntax_debounced(Duration::ZERO));
    assert_eq!(parse_attempts(&editor), attempts + 1);
    assert_tree_matches_fresh_parse(&editor.syntax, editor.buffer());
}

#[test]
fn a_normal_buffer_still_reparses_on_every_eager_update() {
    let mut editor = rust_editor(NO_LIMIT);
    assert!(editor.syntax.has_highlighting());

    for round in 1..=3 {
        let attempts = parse_attempts(&editor);
        edit(&mut editor);
        editor.maybe_update_syntax();
        assert_eq!(parse_attempts(&editor), attempts + 1, "round {round}");
        assert_tree_matches_fresh_parse(&editor.syntax, editor.buffer());
    }
}
