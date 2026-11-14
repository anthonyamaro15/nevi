//! `:set`: change an `[editor]` option for the session, with Vim's option
//! names, short names, and argument forms (`nu`, `nonu`, `nu!`, `invnu`,
//! `nu?`, `ts=4`). Messages follow Vim's wording without the error numbers.

use super::Editor;
use crate::config::SignColumn;

/// The largest `tabstop` Vim accepts.
const MAX_TABSTOP: usize = 9999;

/// An option `:set` understands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SetOption {
    Number,
    RelativeNumber,
    Wrap,
    CursorLine,
    TabStop,
    ScrollOff,
    SignColumn,
    Mouse,
}

impl SetOption {
    pub(crate) const ALL: [SetOption; 8] = [
        SetOption::Number,
        SetOption::RelativeNumber,
        SetOption::Wrap,
        SetOption::CursorLine,
        SetOption::TabStop,
        SetOption::ScrollOff,
        SetOption::SignColumn,
        SetOption::Mouse,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            SetOption::Number => "number",
            SetOption::RelativeNumber => "relativenumber",
            SetOption::Wrap => "wrap",
            SetOption::CursorLine => "cursorline",
            SetOption::TabStop => "tabstop",
            SetOption::ScrollOff => "scrolloff",
            SetOption::SignColumn => "signcolumn",
            SetOption::Mouse => "mouse",
        }
    }

    /// Vim's short name, for the options that have one.
    fn short_name(self) -> Option<&'static str> {
        match self {
            SetOption::Number => Some("nu"),
            SetOption::RelativeNumber => Some("rnu"),
            SetOption::CursorLine => Some("cul"),
            SetOption::TabStop => Some("ts"),
            SetOption::ScrollOff => Some("so"),
            SetOption::SignColumn => Some("scl"),
            SetOption::Wrap | SetOption::Mouse => None,
        }
    }

    fn lookup(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|option| option.name() == name || option.short_name() == Some(name))
    }

    /// The on/off setting behind the option. `mouse` is a string option in
    /// Vim, but Nevi only tracks whether capture is on, so it also takes the
    /// boolean forms.
    fn flag(self, editor: &mut Editor) -> Option<&mut bool> {
        let settings = &mut editor.settings.editor;
        match self {
            SetOption::Number => Some(&mut settings.line_numbers),
            SetOption::RelativeNumber => Some(&mut settings.relative_numbers),
            SetOption::Wrap => Some(&mut settings.wrap),
            SetOption::CursorLine => Some(&mut settings.cursor_line),
            SetOption::Mouse => Some(&mut settings.mouse),
            SetOption::TabStop | SetOption::ScrollOff | SetOption::SignColumn => None,
        }
    }

    /// The option as Vim's `:set name?` prints it: `  number`, `nonumber`,
    /// `  tabstop=4`.
    pub(crate) fn display(self, editor: &Editor) -> String {
        let settings = &editor.settings.editor;
        let name = self.name();
        let flag = |on: bool| {
            if on {
                format!("  {name}")
            } else {
                format!("no{name}")
            }
        };
        match self {
            SetOption::Number => flag(settings.line_numbers),
            SetOption::RelativeNumber => flag(settings.relative_numbers),
            SetOption::Wrap => flag(settings.wrap),
            SetOption::CursorLine => flag(settings.cursor_line),
            SetOption::TabStop => format!("  {name}={}", editor.get_effective_tab_width()),
            SetOption::ScrollOff => format!("  {name}={}", settings.scroll_off),
            SetOption::SignColumn => format!("  {name}={}", settings.sign_column.as_str()),
            // Capture works like nvim's `mouse=nvi` when it is on.
            SetOption::Mouse => format!("  {name}={}", if settings.mouse { "nvi" } else { "" }),
        }
    }
}

impl Editor {
    /// Run the arguments of a `:set` command. Several options may be given,
    /// separated by spaces; they apply in order and stop at the first error,
    /// like Vim. Returns the text to show for `name?` queries.
    pub fn set_options(&mut self, args: &str) -> Result<Option<String>, String> {
        let mut shown = Vec::new();
        let mut changed = false;
        let mut result = Ok(());

        for word in args.split_whitespace() {
            match self.set_option_word(word) {
                Ok(Some(text)) => shown.push(text),
                Ok(None) => changed = true,
                Err(error) => {
                    result = Err(error);
                    break;
                }
            }
        }

        if changed {
            // Most of these move or resize the text area: the gutter, the
            // wrap, the tab width, the scroll margin. The offsets were
            // computed for the old layout, so repaint and rescroll.
            self.render_damage.mark_full();
            self.scroll_to_cursor();
        }

        result?;
        Ok((!shown.is_empty()).then(|| shown.join("  ")))
    }

    /// Apply one `:set` argument. `Ok(Some(text))` is the answer to a query,
    /// `Ok(None)` means the option was changed.
    fn set_option_word(&mut self, word: &str) -> Result<Option<String>, String> {
        let unknown = |name: &str| format!("Unknown option: {name}");

        if let Some((name, value)) = word.split_once('=') {
            let option = SetOption::lookup(name).ok_or_else(|| unknown(name))?;
            self.assign_option(option, value, word)?;
            return Ok(None);
        }
        if let Some(name) = word.strip_suffix('?') {
            let option = SetOption::lookup(name).ok_or_else(|| unknown(name))?;
            return Ok(Some(option.display(self)));
        }
        if let Some(name) = word.strip_suffix('!') {
            let option = SetOption::lookup(name).ok_or_else(|| unknown(name))?;
            return self.switch_option(option, None, word);
        }
        if let Some(option) = SetOption::lookup(word) {
            // A bare name turns a flag on and shows any other option.
            return match option.flag(self) {
                Some(flag) => {
                    *flag = true;
                    Ok(None)
                }
                None => Ok(Some(option.display(self))),
            };
        }
        if let Some(option) = word.strip_prefix("no").and_then(SetOption::lookup) {
            return self.switch_option(option, Some(false), word);
        }
        if let Some(option) = word.strip_prefix("inv").and_then(SetOption::lookup) {
            return self.switch_option(option, None, word);
        }
        Err(unknown(word))
    }

    /// Turn a flag off (`noname`) or flip it (`name!`, `invname`).
    fn switch_option(
        &mut self,
        option: SetOption,
        value: Option<bool>,
        word: &str,
    ) -> Result<Option<String>, String> {
        let flag = option
            .flag(self)
            .ok_or_else(|| format!("Invalid argument: {word}"))?;
        *flag = value.unwrap_or(!*flag);
        Ok(None)
    }

    /// `name=value`.
    fn assign_option(&mut self, option: SetOption, value: &str, word: &str) -> Result<(), String> {
        match option {
            SetOption::TabStop => {
                let width = parse_number(value, word)?;
                if width == 0 {
                    return Err(format!("Argument must be positive: {word}"));
                }
                if width > MAX_TABSTOP {
                    return Err(format!("Invalid argument: {word}"));
                }
                self.set_tab_width(width);
            }
            SetOption::ScrollOff => {
                self.settings.editor.scroll_off = parse_number(value, word)?;
            }
            SetOption::SignColumn => {
                self.settings.editor.sign_column = SignColumn::parse(value)
                    .ok_or_else(|| "signcolumn: expected auto, yes, or no".to_string())?;
            }
            // Vim's 'mouse' holds mode flags: empty turns capture off, any
            // flags (`a`, `nvi`) turn it on.
            SetOption::Mouse => self.settings.editor.mouse = !value.is_empty(),
            SetOption::Number
            | SetOption::RelativeNumber
            | SetOption::Wrap
            | SetOption::CursorLine => return Err(format!("Invalid argument: {word}")),
        }
        Ok(())
    }

    /// Set the tab width for the session. Like Vim's `:set tabstop`, which
    /// sets the buffer's value and the default for new ones, this sets the
    /// default and, when `languages.toml` gives the current file's language
    /// its own width, that one too. Other languages keep theirs.
    fn set_tab_width(&mut self, width: usize) {
        self.settings.editor.tab_width = width;
        let language = self
            .buffer()
            .path
            .as_deref()
            .and_then(Self::config_language_for_path);
        if let Some(config) =
            language.and_then(|name| self.languages_config.languages.get_mut(&name))
        {
            if config.tab_width.is_some() {
                config.tab_width = Some(width);
            }
        }
    }
}

fn parse_number(value: &str, word: &str) -> Result<usize, String> {
    if value.starts_with('-') && value[1..].parse::<usize>().is_ok() {
        return Err(format!("Argument must be positive: {word}"));
    }
    value
        .parse()
        .map_err(|_| format!("Number required after =: {word}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LanguageConfig;

    fn editor() -> Editor {
        let mut editor = Editor::default();
        editor.set_size(80, 24);
        editor.update_pane_rects();
        editor
    }

    fn set(editor: &mut Editor, args: &str) -> Option<String> {
        editor
            .set_options(args)
            .unwrap_or_else(|error| panic!(":set {args} failed: {error}"))
    }

    fn error(editor: &mut Editor, args: &str) -> String {
        editor
            .set_options(args)
            .expect_err(&format!(":set {args} should fail"))
    }

    #[test]
    fn flags_take_every_vim_form_under_both_names() {
        type Field = fn(&Editor) -> bool;
        let flags: [(&str, Field); 7] = [
            ("number", |e| e.settings.editor.line_numbers),
            ("nu", |e| e.settings.editor.line_numbers),
            ("relativenumber", |e| e.settings.editor.relative_numbers),
            ("rnu", |e| e.settings.editor.relative_numbers),
            ("wrap", |e| e.settings.editor.wrap),
            ("cursorline", |e| e.settings.editor.cursor_line),
            ("cul", |e| e.settings.editor.cursor_line),
        ];
        for (name, field) in flags {
            let mut editor = editor();
            assert_eq!(set(&mut editor, name), None);
            assert!(field(&editor), ":set {name}");
            set(&mut editor, &format!("no{name}"));
            assert!(!field(&editor), ":set no{name}");
            set(&mut editor, &format!("{name}!"));
            assert!(field(&editor), ":set {name}!");
            set(&mut editor, &format!("inv{name}"));
            assert!(!field(&editor), ":set inv{name}");
        }
    }

    #[test]
    fn queries_print_the_value_like_vim() {
        let mut editor = editor();
        assert_eq!(set(&mut editor, "nu?").as_deref(), Some("  number"));
        set(&mut editor, "nonu");
        assert_eq!(set(&mut editor, "number?").as_deref(), Some("nonumber"));
        assert_eq!(set(&mut editor, "ts?").as_deref(), Some("  tabstop=4"));
        // A bare name shows an option that is not a flag.
        assert_eq!(set(&mut editor, "tabstop").as_deref(), Some("  tabstop=4"));
        set(&mut editor, "so=3");
        assert_eq!(set(&mut editor, "so?").as_deref(), Some("  scrolloff=3"));
        assert_eq!(set(&mut editor, "scl").as_deref(), Some("  signcolumn=yes"));
        assert_eq!(set(&mut editor, "mouse?").as_deref(), Some("  mouse=nvi"));
        set(&mut editor, "nomouse");
        assert_eq!(set(&mut editor, "mouse?").as_deref(), Some("  mouse="));
        assert_eq!(
            set(&mut editor, "wrap? cul?").as_deref(),
            Some("nowrap  nocursorline")
        );
    }

    #[test]
    fn number_options_take_a_value() {
        let mut editor = editor();
        set(&mut editor, "ts=2");
        assert_eq!(editor.settings.editor.tab_width, 2);
        set(&mut editor, "tabstop=8");
        assert_eq!(editor.settings.editor.tab_width, 8);
        set(&mut editor, "so=5");
        assert_eq!(editor.settings.editor.scroll_off, 5);
        set(&mut editor, "scrolloff=0");
        assert_eq!(editor.settings.editor.scroll_off, 0);
    }

    #[test]
    fn bad_arguments_are_rejected_with_vims_wording() {
        let mut editor = editor();
        let scroll_off = editor.settings.editor.scroll_off;
        for (args, expected) in [
            ("foo", "Unknown option: foo"),
            ("foo=1", "Unknown option: foo"),
            ("nofoo", "Unknown option: nofoo"),
            ("foo?", "Unknown option: foo"),
            ("nu=1", "Invalid argument: nu=1"),
            ("nots", "Invalid argument: nots"),
            ("ts!", "Invalid argument: ts!"),
            ("invso", "Invalid argument: invso"),
            ("ts=abc", "Number required after =: ts=abc"),
            ("ts=", "Number required after =: ts="),
            ("ts=0", "Argument must be positive: ts=0"),
            ("ts=10000", "Invalid argument: ts=10000"),
            ("so=-1", "Argument must be positive: so=-1"),
            ("scl=maybe", "signcolumn: expected auto, yes, or no"),
        ] {
            assert_eq!(error(&mut editor, args), expected, ":set {args}");
        }
        assert_eq!(editor.settings.editor.tab_width, 4, "nothing changed");
        assert_eq!(
            editor.settings.editor.scroll_off, scroll_off,
            "nothing changed"
        );
        assert!(editor.settings.editor.line_numbers, "nothing changed");
    }

    #[test]
    fn several_options_apply_in_order_and_stop_at_the_first_error() {
        let mut editor = editor();
        set(&mut editor, "nonu rnu so=3");
        assert!(!editor.settings.editor.line_numbers);
        assert!(editor.settings.editor.relative_numbers);
        assert_eq!(editor.settings.editor.scroll_off, 3);

        assert_eq!(
            error(&mut editor, "wrap bogus cul"),
            "Unknown option: bogus"
        );
        assert!(editor.settings.editor.wrap, "applied before the error");
        assert!(!editor.settings.editor.cursor_line, "not reached");
    }

    #[test]
    fn tabstop_also_changes_the_current_languages_own_width() {
        let width = |tab_width| LanguageConfig {
            tab_width: Some(tab_width),
            ..LanguageConfig::default()
        };
        let mut editor = editor();
        let languages = &mut editor.languages_config.languages;
        languages.insert("rust".to_string(), width(4));
        languages.insert("typescript".to_string(), width(2));
        editor.buffer_mut().path = Some("main.rs".into());
        assert_eq!(editor.get_effective_tab_width(), 4);

        set(&mut editor, "ts=8");

        assert_eq!(editor.get_effective_tab_width(), 8);
        assert_eq!(set(&mut editor, "ts?").as_deref(), Some("  tabstop=8"));
        assert_eq!(editor.settings.editor.tab_width, 8, "the default too");
        assert_eq!(
            editor.languages_config.get_tab_width("typescript"),
            Some(2),
            "other languages keep their width"
        );
    }

    #[test]
    fn turning_wrap_on_recomputes_the_scroll_for_the_wrapped_layout() {
        let mut editor = editor();
        editor.set_size(30, 6);
        editor.update_pane_rects();
        editor.replace_buffer_content(&format!("{}\n", "x".repeat(60)));
        editor.cursor.col = 50;
        editor.scroll_to_cursor();
        assert!(editor.h_offset > 20, "columns scrolled off without wrap");

        set(&mut editor, "wrap");

        // With wrap on, h_offset counts wrapped rows of the top line. The
        // line takes three rows here, so a stale column count would scroll
        // the whole line out of view.
        assert!(editor.h_offset < 3, "h_offset = {}", editor.h_offset);
    }

    #[test]
    fn showing_line_numbers_rescrolls_so_the_cursor_stays_in_view() {
        let mut editor = editor();
        editor.set_size(30, 6);
        editor.update_pane_rects();
        set(&mut editor, "nonu");
        editor.replace_buffer_content(&format!("{}\n", "x".repeat(60)));
        editor.cursor.col = 40;
        editor.scroll_to_cursor();
        let before = editor.h_offset;

        // The gutter grows, so the text area shrinks under the cursor.
        set(&mut editor, "nu");

        assert!(editor.h_offset > before);
        assert!(editor.cursor.col < editor.h_offset + editor.text_area_width());
    }
}
