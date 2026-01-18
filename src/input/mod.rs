pub mod motion;

pub use motion::{Motion, apply_motion};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Type of find char command (f, F, t, T)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindCharType {
    Forward,      // f - find forward, land on char
    Backward,     // F - find backward, land on char
    TillForward,  // t - find forward, land before char
    TillBackward, // T - find backward, land after char
}

/// Input state machine for handling vim-style commands
#[derive(Debug, Clone, Default)]
pub struct InputState {
    /// Accumulated count (e.g., "23" in "23j")
    pub count: Option<usize>,
    /// Pending operator (e.g., 'd' waiting for motion in "dw")
    pub pending_operator: Option<Operator>,
    /// Partial key sequence (e.g., 'g' waiting for second key in "gg")
    pub partial_key: Option<char>,
    /// Pending text object modifier (i or a, waiting for object type)
    pub pending_text_object: Option<TextObjectModifier>,
    /// Pending find char type (f, F, t, T waiting for target char)
    pub pending_find_char: Option<FindCharType>,
    /// Last find char command for repeating with ; and ,
    pub last_find_char: Option<(FindCharType, char)>,
    /// Pending register selection (e.g., "a in "ayy")
    /// True means waiting for register name after pressing "
    pub pending_register_select: bool,
    /// Selected register for the next operation
    pub selected_register: Option<char>,
    /// Pending replace char (r waiting for replacement char)
    pub pending_replace: bool,
    /// Pending window command (Ctrl-w waiting for second key)
    pub pending_window_cmd: bool,
    /// Pending surround delete (ds waiting for char)
    pub pending_surround_delete: bool,
    /// Pending surround change (cs waiting for old char, then new char)
    /// None = waiting for old, Some(old) = waiting for new
    pub pending_surround_change: Option<Option<char>>,
    /// Pending surround add (ys waiting for motion/text object, then char)
    pub pending_surround_add: bool,
    /// Text object for surround add operation
    pub surround_add_object: Option<TextObject>,
    /// Pending comment toggle (gc waiting for motion or second c)
    pub pending_comment: bool,
}

/// Operators that can be combined with motions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    Delete,  // d
    Change,  // c
    Yank,    // y
    Indent,  // >
    Dedent,  // <
}

/// Text object modifier (inner vs around)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextObjectModifier {
    Inner,  // i - inside, excluding delimiters
    Around, // a - around, including delimiters/whitespace
}

/// Text object types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextObjectType {
    Word,        // w
    BigWord,     // W
    DoubleQuote, // "
    SingleQuote, // '
    BackTick,    // `
    Paren,       // ( ) b
    Brace,       // { } B
    Bracket,     // [ ]
    AngleBracket,// < >
}

/// A complete text object specification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextObject {
    pub modifier: TextObjectModifier,
    pub object_type: TextObjectType,
}

/// Result of processing a key in normal mode
#[derive(Debug, Clone)]
pub enum KeyAction {
    /// No action (key was consumed but needs more input)
    Pending,
    /// Execute a motion
    Motion(Motion, usize),
    /// Execute an operator on a motion range
    OperatorMotion(Operator, Motion, usize),
    /// Execute an operator on the current line (dd, yy, cc)
    OperatorLine(Operator, usize),
    /// Execute an operator on a text object (diw, ca", etc.)
    OperatorTextObject(Operator, TextObject),
    /// Select a text object in visual mode
    SelectTextObject(TextObject),
    /// Enter insert mode
    EnterInsert(InsertPosition),
    /// Delete character at cursor
    DeleteChar,
    /// Delete character before cursor (X)
    DeleteCharBefore,
    /// Replace character at cursor with given char (r)
    ReplaceChar(char),
    /// Join current line with next (J)
    JoinLines,
    /// Scroll cursor to center of screen (zz)
    ScrollCenter,
    /// Scroll cursor to top of screen (zt)
    ScrollTop,
    /// Scroll cursor to bottom of screen (zb)
    ScrollBottom,
    /// Repeat last change (.)
    RepeatLastChange,
    /// Paste after cursor
    PasteAfter,
    /// Paste before cursor
    PasteBefore,
    /// Undo
    Undo,
    /// Redo
    Redo,
    /// Enter command mode
    EnterCommand,
    /// Enter search mode (forward)
    EnterSearchForward,
    /// Enter search mode (backward)
    EnterSearchBackward,
    /// Search next (n)
    SearchNext,
    /// Search previous (N)
    SearchPrev,
    /// Search word under cursor forward (*)
    SearchWordForward,
    /// Search word under cursor backward (#)
    SearchWordBackward,
    /// Enter visual mode
    EnterVisual,
    /// Enter visual line mode
    EnterVisualLine,
    /// Enter visual block mode
    EnterVisualBlock,
    /// Enter replace mode
    EnterReplace,
    /// Quit
    Quit,
    /// Save
    Save,
    /// Window/pane operations
    WindowSplitVertical,
    WindowSplitHorizontal,
    WindowClose,
    WindowCloseOthers,
    WindowNext,
    WindowPrev,
    WindowLeft,
    WindowRight,
    WindowUp,
    WindowDown,
    /// Go to definition (gd)
    GotoDefinition,
    /// Show hover documentation (K)
    Hover,
    /// Jump back in jump list (Ctrl+o)
    JumpBack,
    /// Jump forward in jump list (Ctrl+i)
    JumpForward,
    /// Go to next diagnostic (]d)
    NextDiagnostic,
    /// Go to previous diagnostic ([d)
    PrevDiagnostic,
    /// Show diagnostic floating popup (<leader>d)
    ShowDiagnosticFloat,
    /// Find references (gr)
    FindReferences,
    /// Show code actions (ga)
    CodeActions,
    /// Rename symbol (leader+rn or F2)
    RenameSymbol,
    /// Delete surrounding (ds)
    DeleteSurround(char),
    /// Change surrounding (cs)
    ChangeSurround(char, char),
    /// Add surrounding to text object (ys)
    AddSurround(TextObject, char),
    /// Toggle comment on current line (gcc)
    ToggleCommentLine,
    /// Toggle comment with motion (gc{motion})
    ToggleCommentMotion(Motion, usize),
    /// Toggle comment on visual selection
    ToggleCommentVisual,
    /// Harpoon: add current file to marks (<leader>m)
    HarpoonAdd,
    /// Harpoon: toggle menu (<leader>h)
    HarpoonMenu,
    /// Harpoon: jump to slot 1-4 (<leader>1-4)
    HarpoonJump(usize),
    /// Harpoon: next file (]h)
    HarpoonNext,
    /// Harpoon: previous file ([h)
    HarpoonPrev,
    /// Copilot: accept ghost text completion (Tab in insert mode)
    CopilotAccept,
    /// Copilot: cycle to next completion (Alt+])
    CopilotNextCompletion,
    /// Copilot: cycle to previous completion (Alt+[)
    CopilotPrevCompletion,
    /// Copilot: dismiss ghost text (Esc, handled specially)
    CopilotDismiss,
    /// Unknown/unhandled key
    Unknown,
}

/// Where to position cursor when entering insert mode
#[derive(Debug, Clone, Copy)]
pub enum InsertPosition {
    AtCursor,      // i
    AfterCursor,   // a
    LineStart,     // I
    LineEnd,       // A
    NewLineBelow,  // o
    NewLineAbove,  // O
}

impl InputState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset input state (preserves last_find_char for ; and , repeats)
    pub fn reset(&mut self) {
        self.count = None;
        self.pending_operator = None;
        self.partial_key = None;
        self.pending_text_object = None;
        self.pending_find_char = None;
        self.pending_register_select = false;
        self.selected_register = None;
        self.pending_replace = false;
        self.pending_window_cmd = false;
        self.pending_surround_delete = false;
        self.pending_surround_change = None;
        self.pending_surround_add = false;
        self.surround_add_object = None;
        self.pending_comment = false;
        // Note: last_find_char is NOT reset - it persists for ; and , repeats
    }

    /// Take the selected register and clear it
    pub fn take_register(&mut self) -> Option<char> {
        self.selected_register.take()
    }

    /// Get the effective count (1 if not specified)
    pub fn effective_count(&self) -> usize {
        self.count.unwrap_or(1)
    }

    /// Process a digit for count accumulation
    fn accumulate_count(&mut self, digit: char) {
        let d = digit.to_digit(10).unwrap() as usize;
        self.count = Some(self.count.unwrap_or(0) * 10 + d);
    }

    /// Process a key in normal mode
    pub fn process_normal_key(&mut self, key: KeyEvent) -> KeyAction {
        let count = self.effective_count();

        // Handle partial sequences first (like "gg")
        if let Some(partial) = self.partial_key.take() {
            return self.handle_partial_sequence(partial, key, count);
        }

        // Handle register selection after "
        if self.pending_register_select {
            self.pending_register_select = false;
            if let KeyCode::Char(c) = key.code {
                // Valid register names: a-z, A-Z, 0-9, ", -, +, *, _, /
                if c.is_ascii_alphabetic() || c.is_ascii_digit()
                    || c == '"' || c == '-' || c == '+' || c == '*' || c == '_' || c == '/' {
                    self.selected_register = Some(c);
                    return KeyAction::Pending;
                }
            }
            // Invalid register name - reset and ignore
            self.reset();
            return KeyAction::Unknown;
        }

        // Handle find char target after f/F/t/T
        if let Some(find_type) = self.pending_find_char.take() {
            return self.handle_find_char_target(find_type, key, count);
        }

        // Handle replace char target after 'r'
        if self.pending_replace {
            self.pending_replace = false;
            if let KeyCode::Char(c) = key.code {
                self.reset();
                return KeyAction::ReplaceChar(c);
            } else if key.code == KeyCode::Esc {
                self.reset();
                return KeyAction::Pending;
            }
            self.reset();
            return KeyAction::Unknown;
        }

        // Handle window command after Ctrl-w
        if self.pending_window_cmd {
            self.pending_window_cmd = false;
            return self.handle_window_command(key);
        }

        // Handle surround delete (ds waiting for char)
        if self.pending_surround_delete {
            self.pending_surround_delete = false;
            if let KeyCode::Char(c) = key.code {
                let surround_char = Self::normalize_surround_char(c);
                self.reset();
                return KeyAction::DeleteSurround(surround_char);
            }
            self.reset();
            return KeyAction::Unknown;
        }

        // Handle surround change (cs waiting for old char, then new char)
        if let Some(old_char_opt) = self.pending_surround_change.take() {
            if let KeyCode::Char(c) = key.code {
                match old_char_opt {
                    None => {
                        // Waiting for old char
                        let old = Self::normalize_surround_char(c);
                        self.pending_surround_change = Some(Some(old));
                        return KeyAction::Pending;
                    }
                    Some(old) => {
                        // Waiting for new char
                        let new = Self::normalize_surround_char(c);
                        self.reset();
                        return KeyAction::ChangeSurround(old, new);
                    }
                }
            }
            self.reset();
            return KeyAction::Unknown;
        }

        // Handle surround add (ys waiting for text object, then char)
        if self.pending_surround_add {
            // First, check if we have a text object already
            if let Some(text_object) = self.surround_add_object.take() {
                // We have the text object, now get the surround char
                if let KeyCode::Char(c) = key.code {
                    let surround_char = Self::normalize_surround_char(c);
                    self.pending_surround_add = false;
                    self.reset();
                    return KeyAction::AddSurround(text_object, surround_char);
                }
                self.reset();
                return KeyAction::Unknown;
            }

            // Need to get text object (i/a modifier or direct object)
            match key.code {
                KeyCode::Char('i') => {
                    self.pending_text_object = Some(TextObjectModifier::Inner);
                    // Keep pending_surround_add true
                    return KeyAction::Pending;
                }
                KeyCode::Char('a') => {
                    self.pending_text_object = Some(TextObjectModifier::Around);
                    // Keep pending_surround_add true
                    return KeyAction::Pending;
                }
                // Direct text object shortcuts
                KeyCode::Char('w') => {
                    self.surround_add_object = Some(TextObject {
                        modifier: TextObjectModifier::Inner,
                        object_type: TextObjectType::Word,
                    });
                    return KeyAction::Pending;
                }
                KeyCode::Char('W') => {
                    self.surround_add_object = Some(TextObject {
                        modifier: TextObjectModifier::Inner,
                        object_type: TextObjectType::BigWord,
                    });
                    return KeyAction::Pending;
                }
                _ => {
                    self.reset();
                    return KeyAction::Unknown;
                }
            }
        }

        // Handle text object type after i/a modifier
        if let Some(modifier) = self.pending_text_object.take() {
            // Check if this is part of a surround add operation
            if self.pending_surround_add {
                if let Some(obj_type) = Self::char_to_text_object_type(key.code) {
                    self.surround_add_object = Some(TextObject {
                        modifier,
                        object_type: obj_type,
                    });
                    return KeyAction::Pending;
                }
                self.reset();
                return KeyAction::Unknown;
            }
            return self.handle_text_object_type(modifier, key);
        }

        // Handle pending comment toggle (gc waiting for motion or 'c')
        if self.pending_comment {
            return self.handle_comment_motion(key, count);
        }

        match (key.modifiers, key.code) {
            // Digits for count (but '0' is line start if no count started)
            (KeyModifiers::NONE, KeyCode::Char(c @ '1'..='9')) => {
                self.accumulate_count(c);
                KeyAction::Pending
            }
            (KeyModifiers::NONE, KeyCode::Char('0')) if self.count.is_some() => {
                self.accumulate_count('0');
                KeyAction::Pending
            }

            // Register selection with "
            (KeyModifiers::SHIFT, KeyCode::Char('"')) | (KeyModifiers::NONE, KeyCode::Char('"')) => {
                self.pending_register_select = true;
                KeyAction::Pending
            }

            // Operators
            (KeyModifiers::NONE, KeyCode::Char('d')) => {
                if self.pending_operator == Some(Operator::Delete) {
                    // dd - delete line
                    self.reset();
                    KeyAction::OperatorLine(Operator::Delete, count)
                } else {
                    self.pending_operator = Some(Operator::Delete);
                    KeyAction::Pending
                }
            }
            (KeyModifiers::NONE, KeyCode::Char('c')) => {
                if self.pending_operator == Some(Operator::Change) {
                    // cc - change line
                    self.reset();
                    KeyAction::OperatorLine(Operator::Change, count)
                } else {
                    self.pending_operator = Some(Operator::Change);
                    KeyAction::Pending
                }
            }
            (KeyModifiers::NONE, KeyCode::Char('y')) => {
                if self.pending_operator == Some(Operator::Yank) {
                    // yy - yank line
                    self.reset();
                    KeyAction::OperatorLine(Operator::Yank, count)
                } else {
                    self.pending_operator = Some(Operator::Yank);
                    KeyAction::Pending
                }
            }

            // Surround operations (ds, cs, ys)
            (KeyModifiers::NONE, KeyCode::Char('s')) if self.pending_operator.is_some() => {
                match self.pending_operator {
                    Some(Operator::Delete) => {
                        // ds - delete surrounding
                        self.pending_operator = None;
                        self.pending_surround_delete = true;
                        KeyAction::Pending
                    }
                    Some(Operator::Change) => {
                        // cs - change surrounding
                        self.pending_operator = None;
                        self.pending_surround_change = Some(None); // waiting for old char
                        KeyAction::Pending
                    }
                    Some(Operator::Yank) => {
                        // ys - add surrounding
                        self.pending_operator = None;
                        self.pending_surround_add = true;
                        KeyAction::Pending
                    }
                    _ => {
                        self.reset();
                        KeyAction::Unknown
                    }
                }
            }

            // Motions
            (KeyModifiers::NONE, KeyCode::Char('h')) | (_, KeyCode::Left) => {
                self.motion_or_operator(Motion::Left, count)
            }
            (KeyModifiers::NONE, KeyCode::Char('j')) | (_, KeyCode::Down) => {
                self.motion_or_operator(Motion::Down, count)
            }
            (KeyModifiers::NONE, KeyCode::Char('k')) | (_, KeyCode::Up) => {
                self.motion_or_operator(Motion::Up, count)
            }
            (KeyModifiers::NONE, KeyCode::Char('l')) | (_, KeyCode::Right) => {
                self.motion_or_operator(Motion::Right, count)
            }

            // Word motions
            (KeyModifiers::NONE, KeyCode::Char('w')) => {
                self.motion_or_operator(Motion::WordForward, count)
            }
            (KeyModifiers::SHIFT, KeyCode::Char('W')) => {
                self.motion_or_operator(Motion::BigWordForward, count)
            }
            (KeyModifiers::NONE, KeyCode::Char('b')) => {
                self.motion_or_operator(Motion::WordBackward, count)
            }
            (KeyModifiers::SHIFT, KeyCode::Char('B')) => {
                self.motion_or_operator(Motion::BigWordBackward, count)
            }
            (KeyModifiers::NONE, KeyCode::Char('e')) => {
                self.motion_or_operator(Motion::WordEnd, count)
            }
            (KeyModifiers::SHIFT, KeyCode::Char('E')) => {
                self.motion_or_operator(Motion::BigWordEnd, count)
            }

            // Line motions
            (KeyModifiers::NONE, KeyCode::Char('0')) => {
                self.motion_or_operator(Motion::LineStart, count)
            }
            (_, KeyCode::Char('^')) => {
                self.motion_or_operator(Motion::FirstNonBlank, count)
            }
            (_, KeyCode::Char('$')) => {
                self.motion_or_operator(Motion::LineEnd, count)
            }

            // Paragraph motions
            (_, KeyCode::Char('}')) => {
                self.motion_or_operator(Motion::ParagraphForward, count)
            }
            (_, KeyCode::Char('{')) => {
                self.motion_or_operator(Motion::ParagraphBackward, count)
            }

            // Bracket matching
            (_, KeyCode::Char('%')) => {
                self.motion_or_operator(Motion::MatchingBracket, count)
            }

            // Find char motions (f, F, t, T)
            (KeyModifiers::NONE, KeyCode::Char('f')) => {
                self.pending_find_char = Some(FindCharType::Forward);
                KeyAction::Pending
            }
            (KeyModifiers::SHIFT, KeyCode::Char('F')) => {
                self.pending_find_char = Some(FindCharType::Backward);
                KeyAction::Pending
            }
            (KeyModifiers::NONE, KeyCode::Char('t')) => {
                self.pending_find_char = Some(FindCharType::TillForward);
                KeyAction::Pending
            }
            (KeyModifiers::SHIFT, KeyCode::Char('T')) => {
                self.pending_find_char = Some(FindCharType::TillBackward);
                KeyAction::Pending
            }

            // Repeat find char (; and ,)
            (_, KeyCode::Char(';')) => {
                if let Some((find_type, target)) = self.last_find_char {
                    let motion = self.find_type_to_motion(find_type, target);
                    self.motion_or_operator(motion, count)
                } else {
                    self.reset();
                    KeyAction::Unknown
                }
            }
            (_, KeyCode::Char(',')) => {
                if let Some((find_type, target)) = self.last_find_char {
                    // Reverse the direction
                    let reversed = match find_type {
                        FindCharType::Forward => FindCharType::Backward,
                        FindCharType::Backward => FindCharType::Forward,
                        FindCharType::TillForward => FindCharType::TillBackward,
                        FindCharType::TillBackward => FindCharType::TillForward,
                    };
                    let motion = self.find_type_to_motion(reversed, target);
                    self.motion_or_operator(motion, count)
                } else {
                    self.reset();
                    KeyAction::Unknown
                }
            }

            // File motions
            (KeyModifiers::NONE, KeyCode::Char('g')) => {
                self.partial_key = Some('g');
                KeyAction::Pending
            }
            (KeyModifiers::SHIFT, KeyCode::Char('G')) => {
                if self.count.is_some() {
                    self.motion_or_operator(Motion::GotoLine(count), 1)
                } else {
                    self.motion_or_operator(Motion::FileEnd, 1)
                }
            }

            // Screen motions (H, M, L)
            (KeyModifiers::SHIFT, KeyCode::Char('H')) => {
                self.motion_or_operator(Motion::ScreenTop, count)
            }
            (KeyModifiers::SHIFT, KeyCode::Char('M')) => {
                self.motion_or_operator(Motion::ScreenMiddle, count)
            }
            (KeyModifiers::SHIFT, KeyCode::Char('L')) => {
                self.motion_or_operator(Motion::ScreenBottom, count)
            }

            // Page motions
            (KeyModifiers::CONTROL, KeyCode::Char('d')) => {
                self.motion_or_operator(Motion::HalfPageDown, count)
            }
            (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
                self.motion_or_operator(Motion::HalfPageUp, count)
            }
            (KeyModifiers::CONTROL, KeyCode::Char('f')) => {
                self.motion_or_operator(Motion::PageDown, count)
            }
            (KeyModifiers::CONTROL, KeyCode::Char('b')) => {
                self.motion_or_operator(Motion::PageUp, count)
            }

            // Insert mode entry (or text object modifier if operator pending)
            (KeyModifiers::NONE, KeyCode::Char('i')) => {
                if self.pending_operator.is_some() {
                    // 'i' after operator means "inner" text object
                    self.pending_text_object = Some(TextObjectModifier::Inner);
                    KeyAction::Pending
                } else {
                    self.reset();
                    KeyAction::EnterInsert(InsertPosition::AtCursor)
                }
            }
            (KeyModifiers::SHIFT, KeyCode::Char('I')) => {
                self.reset();
                KeyAction::EnterInsert(InsertPosition::LineStart)
            }
            (KeyModifiers::NONE, KeyCode::Char('a')) => {
                if self.pending_operator.is_some() {
                    // 'a' after operator means "around" text object
                    self.pending_text_object = Some(TextObjectModifier::Around);
                    KeyAction::Pending
                } else {
                    self.reset();
                    KeyAction::EnterInsert(InsertPosition::AfterCursor)
                }
            }
            (KeyModifiers::SHIFT, KeyCode::Char('A')) => {
                self.reset();
                KeyAction::EnterInsert(InsertPosition::LineEnd)
            }
            (KeyModifiers::NONE, KeyCode::Char('o')) => {
                self.reset();
                KeyAction::EnterInsert(InsertPosition::NewLineBelow)
            }
            (KeyModifiers::SHIFT, KeyCode::Char('O')) => {
                self.reset();
                KeyAction::EnterInsert(InsertPosition::NewLineAbove)
            }

            // Simple operations
            (KeyModifiers::NONE, KeyCode::Char('x')) => {
                self.reset();
                KeyAction::DeleteChar
            }
            (KeyModifiers::SHIFT, KeyCode::Char('X')) => {
                self.reset();
                KeyAction::DeleteCharBefore
            }
            (KeyModifiers::NONE, KeyCode::Char('r')) => {
                // r - replace character (wait for replacement char)
                self.pending_replace = true;
                KeyAction::Pending
            }
            (KeyModifiers::SHIFT, KeyCode::Char('R')) | (KeyModifiers::NONE, KeyCode::Char('R')) => {
                // R - enter replace mode
                self.reset();
                KeyAction::EnterReplace
            }
            (KeyModifiers::SHIFT, KeyCode::Char('J')) => {
                // J - join lines
                self.reset();
                KeyAction::JoinLines
            }
            (KeyModifiers::SHIFT, KeyCode::Char('K')) => {
                // K - show hover documentation (LSP)
                self.reset();
                KeyAction::Hover
            }
            (KeyModifiers::NONE, KeyCode::Char('.')) => {
                // . - repeat last change
                self.reset();
                KeyAction::RepeatLastChange
            }
            (KeyModifiers::NONE, KeyCode::Char('z')) => {
                // z prefix for scroll commands (zz, zt, zb)
                self.partial_key = Some('z');
                KeyAction::Pending
            }
            (KeyModifiers::NONE, KeyCode::Char(']')) => {
                // ] prefix for forward navigation (]d = next diagnostic)
                self.partial_key = Some(']');
                KeyAction::Pending
            }
            (KeyModifiers::NONE, KeyCode::Char('[')) => {
                // [ prefix for backward navigation ([d = prev diagnostic)
                self.partial_key = Some('[');
                KeyAction::Pending
            }
            (KeyModifiers::NONE, KeyCode::Char('p')) => {
                self.reset();
                KeyAction::PasteAfter
            }
            (KeyModifiers::SHIFT, KeyCode::Char('P')) => {
                self.reset();
                KeyAction::PasteBefore
            }

            // D = d$ (delete to end of line)
            (KeyModifiers::SHIFT, KeyCode::Char('D')) => {
                self.reset();
                KeyAction::OperatorMotion(Operator::Delete, Motion::LineEnd, 1)
            }
            // C = c$ (change to end of line)
            (KeyModifiers::SHIFT, KeyCode::Char('C')) => {
                self.reset();
                KeyAction::OperatorMotion(Operator::Change, Motion::LineEnd, 1)
            }
            // Y = yy (yank line) - vim behavior
            (KeyModifiers::SHIFT, KeyCode::Char('Y')) => {
                self.reset();
                KeyAction::OperatorLine(Operator::Yank, count)
            }

            // Undo/Redo
            (KeyModifiers::NONE, KeyCode::Char('u')) => {
                self.reset();
                KeyAction::Undo
            }
            (KeyModifiers::CONTROL, KeyCode::Char('r')) => {
                self.reset();
                KeyAction::Redo
            }

            // Command mode
            (_, KeyCode::Char(':')) => {
                self.reset();
                KeyAction::EnterCommand
            }

            // Search
            (_, KeyCode::Char('/')) => {
                self.reset();
                KeyAction::EnterSearchForward
            }
            (_, KeyCode::Char('?')) => {
                self.reset();
                KeyAction::EnterSearchBackward
            }
            (KeyModifiers::NONE, KeyCode::Char('n')) => {
                self.reset();
                KeyAction::SearchNext
            }
            (KeyModifiers::SHIFT, KeyCode::Char('N')) => {
                self.reset();
                KeyAction::SearchPrev
            }
            // Star search (* and #)
            (_, KeyCode::Char('*')) => {
                self.reset();
                KeyAction::SearchWordForward
            }
            (_, KeyCode::Char('#')) => {
                self.reset();
                KeyAction::SearchWordBackward
            }

            // Visual mode
            (KeyModifiers::NONE, KeyCode::Char('v')) => {
                self.reset();
                KeyAction::EnterVisual
            }
            (KeyModifiers::SHIFT, KeyCode::Char('V')) => {
                self.reset();
                KeyAction::EnterVisualLine
            }
            (KeyModifiers::CONTROL, KeyCode::Char('v')) => {
                self.reset();
                KeyAction::EnterVisualBlock
            }

            // Quit
            (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                self.reset();
                KeyAction::Quit
            }

            // Save (temporary)
            (KeyModifiers::CONTROL, KeyCode::Char('s')) => {
                self.reset();
                KeyAction::Save
            }

            // Window/pane commands (Ctrl-w prefix)
            (KeyModifiers::CONTROL, KeyCode::Char('w')) => {
                self.pending_window_cmd = true;
                KeyAction::Pending
            }

            // Direct pane navigation (Ctrl+h/j/k/l) - alternative to Ctrl-w h/j/k/l
            (KeyModifiers::CONTROL, KeyCode::Char('h')) => {
                self.reset();
                KeyAction::WindowLeft
            }
            (KeyModifiers::CONTROL, KeyCode::Char('j')) => {
                self.reset();
                KeyAction::WindowDown
            }
            (KeyModifiers::CONTROL, KeyCode::Char('k')) => {
                self.reset();
                KeyAction::WindowUp
            }
            (KeyModifiers::CONTROL, KeyCode::Char('l')) => {
                self.reset();
                KeyAction::WindowRight
            }

            // Jump list navigation
            (KeyModifiers::CONTROL, KeyCode::Char('o')) => {
                self.reset();
                KeyAction::JumpBack
            }
            (KeyModifiers::CONTROL, KeyCode::Char('i')) => {
                self.reset();
                KeyAction::JumpForward
            }

            // Escape cancels pending operations
            (KeyModifiers::NONE, KeyCode::Esc) => {
                self.reset();
                KeyAction::Pending
            }

            _ => {
                self.reset();
                KeyAction::Unknown
            }
        }
    }

    /// Handle partial key sequences (like "gg", "zz")
    fn handle_partial_sequence(&mut self, partial: char, key: KeyEvent, count: usize) -> KeyAction {
        match (partial, key.modifiers, key.code) {
            // gg - go to start
            ('g', KeyModifiers::NONE, KeyCode::Char('g')) => {
                if self.count.is_some() {
                    let action = self.motion_or_operator(Motion::GotoLine(count), 1);
                    self.reset();
                    action
                } else {
                    let action = self.motion_or_operator(Motion::FileStart, 1);
                    self.reset();
                    action
                }
            }
            // gd - go to definition (LSP)
            ('g', KeyModifiers::NONE, KeyCode::Char('d')) => {
                self.reset();
                KeyAction::GotoDefinition
            }
            // gr - find references (LSP)
            ('g', KeyModifiers::NONE, KeyCode::Char('r')) => {
                self.reset();
                KeyAction::FindReferences
            }
            // gl - show line diagnostic floating popup
            ('g', KeyModifiers::NONE, KeyCode::Char('l')) => {
                self.reset();
                KeyAction::ShowDiagnosticFloat
            }
            // gc - comment toggle (waits for motion or 'c' for line)
            ('g', KeyModifiers::NONE, KeyCode::Char('c')) => {
                self.pending_comment = true;
                KeyAction::Pending
            }
            // zz - scroll cursor to center of screen
            ('z', KeyModifiers::NONE, KeyCode::Char('z')) => {
                self.reset();
                KeyAction::ScrollCenter
            }
            // zt - scroll cursor to top of screen
            ('z', KeyModifiers::NONE, KeyCode::Char('t')) => {
                self.reset();
                KeyAction::ScrollTop
            }
            // zb - scroll cursor to bottom of screen
            ('z', KeyModifiers::NONE, KeyCode::Char('b')) => {
                self.reset();
                KeyAction::ScrollBottom
            }
            // ]d - go to next diagnostic
            (']', KeyModifiers::NONE, KeyCode::Char('d')) => {
                self.reset();
                KeyAction::NextDiagnostic
            }
            // [d - go to previous diagnostic
            ('[', KeyModifiers::NONE, KeyCode::Char('d')) => {
                self.reset();
                KeyAction::PrevDiagnostic
            }
            // ]h - go to next harpoon file
            (']', KeyModifiers::NONE, KeyCode::Char('h')) => {
                self.reset();
                KeyAction::HarpoonNext
            }
            // [h - go to previous harpoon file
            ('[', KeyModifiers::NONE, KeyCode::Char('h')) => {
                self.reset();
                KeyAction::HarpoonPrev
            }
            // Other prefixed commands can be added here
            _ => {
                self.reset();
                KeyAction::Unknown
            }
        }
    }

    /// Return a motion action, or operator+motion if operator is pending
    fn motion_or_operator(&mut self, motion: Motion, count: usize) -> KeyAction {
        if let Some(op) = self.pending_operator.take() {
            self.reset();
            KeyAction::OperatorMotion(op, motion, count)
        } else {
            self.reset();
            KeyAction::Motion(motion, count)
        }
    }

    /// Handle text object type key after i/a modifier
    fn handle_text_object_type(&mut self, modifier: TextObjectModifier, key: KeyEvent) -> KeyAction {
        let object_type = match (key.modifiers, key.code) {
            // Word objects
            (KeyModifiers::NONE, KeyCode::Char('w')) => Some(TextObjectType::Word),
            (KeyModifiers::SHIFT, KeyCode::Char('W')) => Some(TextObjectType::BigWord),
            // Quote objects
            (_, KeyCode::Char('"')) => Some(TextObjectType::DoubleQuote),
            (_, KeyCode::Char('\'')) => Some(TextObjectType::SingleQuote),
            (_, KeyCode::Char('`')) => Some(TextObjectType::BackTick),
            // Bracket objects
            (_, KeyCode::Char('(')) | (_, KeyCode::Char(')')) => Some(TextObjectType::Paren),
            (KeyModifiers::NONE, KeyCode::Char('b')) => Some(TextObjectType::Paren),
            (_, KeyCode::Char('{')) | (_, KeyCode::Char('}')) => Some(TextObjectType::Brace),
            (KeyModifiers::SHIFT, KeyCode::Char('B')) => Some(TextObjectType::Brace),
            (_, KeyCode::Char('[')) | (_, KeyCode::Char(']')) => Some(TextObjectType::Bracket),
            (_, KeyCode::Char('<')) | (_, KeyCode::Char('>')) => Some(TextObjectType::AngleBracket),
            _ => None,
        };

        if let Some(obj_type) = object_type {
            let text_object = TextObject {
                modifier,
                object_type: obj_type,
            };

            if let Some(op) = self.pending_operator.take() {
                self.reset();
                KeyAction::OperatorTextObject(op, text_object)
            } else {
                // In visual mode, just select the text object
                self.reset();
                KeyAction::SelectTextObject(text_object)
            }
        } else {
            self.reset();
            KeyAction::Unknown
        }
    }

    /// Normalize surround character (handle aliases like b for ( and B for {)
    fn normalize_surround_char(c: char) -> char {
        match c {
            'b' => '(',
            'B' => '{',
            'r' => '[',
            'a' => '<',
            _ => c,
        }
    }

    /// Convert key code to text object type for surround add
    fn char_to_text_object_type(code: KeyCode) -> Option<TextObjectType> {
        match code {
            KeyCode::Char('w') => Some(TextObjectType::Word),
            KeyCode::Char('W') => Some(TextObjectType::BigWord),
            KeyCode::Char('"') => Some(TextObjectType::DoubleQuote),
            KeyCode::Char('\'') => Some(TextObjectType::SingleQuote),
            KeyCode::Char('`') => Some(TextObjectType::BackTick),
            KeyCode::Char('(') | KeyCode::Char(')') | KeyCode::Char('b') => Some(TextObjectType::Paren),
            KeyCode::Char('{') | KeyCode::Char('}') | KeyCode::Char('B') => Some(TextObjectType::Brace),
            KeyCode::Char('[') | KeyCode::Char(']') | KeyCode::Char('r') => Some(TextObjectType::Bracket),
            KeyCode::Char('<') | KeyCode::Char('>') | KeyCode::Char('a') => Some(TextObjectType::AngleBracket),
            _ => None,
        }
    }

    /// Handle the target character after f/F/t/T
    fn handle_find_char_target(&mut self, find_type: FindCharType, key: KeyEvent, count: usize) -> KeyAction {
        // Only accept regular character input
        if let KeyCode::Char(target) = key.code {
            // Store for repeat with ; and ,
            self.last_find_char = Some((find_type, target));

            let motion = self.find_type_to_motion(find_type, target);
            self.motion_or_operator(motion, count)
        } else {
            // Escape or other key cancels
            self.reset();
            KeyAction::Unknown
        }
    }

    /// Convert FindCharType + target char to a Motion
    fn find_type_to_motion(&self, find_type: FindCharType, target: char) -> Motion {
        match find_type {
            FindCharType::Forward => Motion::FindChar(target),
            FindCharType::Backward => Motion::FindCharBack(target),
            FindCharType::TillForward => Motion::TillChar(target),
            FindCharType::TillBackward => Motion::TillCharBack(target),
        }
    }

    /// Handle window command after Ctrl-w
    fn handle_window_command(&mut self, key: KeyEvent) -> KeyAction {
        self.reset();
        match (key.modifiers, key.code) {
            // Navigation
            (KeyModifiers::NONE, KeyCode::Char('h')) | (_, KeyCode::Left) => {
                KeyAction::WindowLeft
            }
            (KeyModifiers::NONE, KeyCode::Char('j')) | (_, KeyCode::Down) => {
                KeyAction::WindowDown
            }
            (KeyModifiers::NONE, KeyCode::Char('k')) | (_, KeyCode::Up) => {
                KeyAction::WindowUp
            }
            (KeyModifiers::NONE, KeyCode::Char('l')) | (_, KeyCode::Right) => {
                KeyAction::WindowRight
            }
            // Cycle through windows
            (KeyModifiers::NONE, KeyCode::Char('w')) => {
                KeyAction::WindowNext
            }
            (KeyModifiers::SHIFT, KeyCode::Char('W')) => {
                KeyAction::WindowPrev
            }
            // Splits
            (KeyModifiers::NONE, KeyCode::Char('v')) => {
                KeyAction::WindowSplitVertical
            }
            (KeyModifiers::NONE, KeyCode::Char('s')) => {
                KeyAction::WindowSplitHorizontal
            }
            // Close
            (KeyModifiers::NONE, KeyCode::Char('q')) => {
                KeyAction::WindowClose
            }
            (KeyModifiers::NONE, KeyCode::Char('o')) => {
                KeyAction::WindowCloseOthers
            }
            // Escape cancels
            (_, KeyCode::Esc) => {
                KeyAction::Pending
            }
            _ => KeyAction::Unknown,
        }
    }

    /// Handle motion after gc (comment toggle)
    fn handle_comment_motion(&mut self, key: KeyEvent, count: usize) -> KeyAction {
        match (key.modifiers, key.code) {
            // gcc - toggle comment on current line
            (KeyModifiers::NONE, KeyCode::Char('c')) => {
                self.reset();
                KeyAction::ToggleCommentLine
            }
            // Escape cancels
            (_, KeyCode::Esc) => {
                self.reset();
                KeyAction::Pending
            }
            // Line motions
            (KeyModifiers::NONE, KeyCode::Char('j')) | (_, KeyCode::Down) => {
                self.reset();
                KeyAction::ToggleCommentMotion(Motion::Down, count)
            }
            (KeyModifiers::NONE, KeyCode::Char('k')) | (_, KeyCode::Up) => {
                self.reset();
                KeyAction::ToggleCommentMotion(Motion::Up, count)
            }
            // Word motions
            (KeyModifiers::NONE, KeyCode::Char('w')) => {
                self.reset();
                KeyAction::ToggleCommentMotion(Motion::WordForward, count)
            }
            (KeyModifiers::SHIFT, KeyCode::Char('W')) => {
                self.reset();
                KeyAction::ToggleCommentMotion(Motion::BigWordForward, count)
            }
            (KeyModifiers::NONE, KeyCode::Char('b')) => {
                self.reset();
                KeyAction::ToggleCommentMotion(Motion::WordBackward, count)
            }
            (KeyModifiers::SHIFT, KeyCode::Char('B')) => {
                self.reset();
                KeyAction::ToggleCommentMotion(Motion::BigWordBackward, count)
            }
            (KeyModifiers::NONE, KeyCode::Char('e')) => {
                self.reset();
                KeyAction::ToggleCommentMotion(Motion::WordEnd, count)
            }
            // Line position motions
            (KeyModifiers::NONE, KeyCode::Char('0')) => {
                self.reset();
                KeyAction::ToggleCommentMotion(Motion::LineStart, count)
            }
            (_, KeyCode::Char('$')) => {
                self.reset();
                KeyAction::ToggleCommentMotion(Motion::LineEnd, count)
            }
            (_, KeyCode::Char('^')) => {
                self.reset();
                KeyAction::ToggleCommentMotion(Motion::FirstNonBlank, count)
            }
            // Paragraph motions
            (_, KeyCode::Char('}')) => {
                self.reset();
                KeyAction::ToggleCommentMotion(Motion::ParagraphForward, count)
            }
            (_, KeyCode::Char('{')) => {
                self.reset();
                KeyAction::ToggleCommentMotion(Motion::ParagraphBackward, count)
            }
            // File motions
            (KeyModifiers::SHIFT, KeyCode::Char('G')) => {
                self.reset();
                if self.count.is_some() {
                    KeyAction::ToggleCommentMotion(Motion::GotoLine(count), 1)
                } else {
                    KeyAction::ToggleCommentMotion(Motion::FileEnd, 1)
                }
            }
            // gg - file start (need to handle 'g' prefix)
            (KeyModifiers::NONE, KeyCode::Char('g')) => {
                // Set partial_key to handle gg
                self.partial_key = Some('g');
                // Keep pending_comment true for gcgg
                KeyAction::Pending
            }
            // Text object support (gcip, gciw, etc.)
            (KeyModifiers::NONE, KeyCode::Char('i')) => {
                self.pending_text_object = Some(TextObjectModifier::Inner);
                KeyAction::Pending
            }
            (KeyModifiers::NONE, KeyCode::Char('a')) => {
                self.pending_text_object = Some(TextObjectModifier::Around);
                KeyAction::Pending
            }
            _ => {
                self.reset();
                KeyAction::Unknown
            }
        }
    }
}
