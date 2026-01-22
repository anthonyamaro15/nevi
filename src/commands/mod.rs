use std::path::PathBuf;

/// Parsed command from command line
#[derive(Debug, Clone)]
pub enum Command {
    /// :w [filename] - Write buffer to file
    Write(Option<PathBuf>),
    /// :wa - Write all modified buffers
    WriteAll,
    /// :q - Quit (fails if unsaved changes)
    Quit,
    /// :q! - Force quit (discard changes)
    ForceQuit,
    /// :qa - Quit all (fails if any unsaved changes)
    QuitAll,
    /// :qa! - Force quit all (discard all changes)
    ForceQuitAll,
    /// :wq - Write and quit
    WriteQuit,
    /// :wqa - Write all and quit all
    WriteQuitAll,
    /// :x - Write if modified and quit
    WriteQuitIfModified,
    /// :xa - Write all if modified and quit all
    WriteQuitAllIfModified,
    /// :e [filename] - Edit a file
    Edit(Option<PathBuf>),
    /// :e! - Reload current file (discard changes)
    Reload,
    /// :n or :next - Go to next buffer
    Next,
    /// :N or :prev - Go to previous buffer
    Prev,
    /// :set option[=value] - Set an option
    Set(String, Option<String>),
    /// :[number] - Go to line number
    GotoLine(usize),
    /// :LazyGit - Open lazygit
    LazyGit,
    /// :! command - Run shell command
    Shell(String),
    /// :vs [file] - Vertical split
    VSplit(Option<PathBuf>),
    /// :sp [file] - Horizontal split
    HSplit(Option<PathBuf>),
    /// :only - Close all other panes
    Only,
    /// :FindFiles - Open fuzzy finder for files
    FindFiles,
    /// :FindBuffers - Open fuzzy finder for buffers
    FindBuffers,
    /// :LiveGrep - Open fuzzy finder for live grep
    LiveGrep,
    /// :FindDiagnostics - Open fuzzy finder for LSP diagnostics
    FindDiagnostics,
    /// :DiagnosticFloat - Show diagnostic floating popup at cursor line
    DiagnosticFloat,
    /// :noh or :nohlsearch - Clear search highlights
    NoHighlight,
    /// :s/pattern/replacement/flags or :%s/pattern/replacement/flags - Search and replace
    Substitute {
        /// Range: None for current line, Some(true) for entire file (%)
        entire_file: bool,
        /// Search pattern
        pattern: String,
        /// Replacement string
        replacement: String,
        /// Global flag (replace all on line vs first only)
        global: bool,
    },
    /// :new or :touch - Create a new file
    NewFile(PathBuf),
    /// :delete or :rm - Delete current file (requires confirmation)
    DeleteFile,
    /// :delete! or :rm! - Delete current file (force, no confirmation)
    DeleteFileForce,
    /// :rename or :mv - Rename current file
    RenameFile(PathBuf),
    /// :mkdir - Create a directory
    MakeDir(PathBuf),
    /// :Explorer - Toggle file explorer
    ToggleExplorer,
    /// :Explore - Open file explorer
    OpenExplorer,
    /// :Format - Format document using LSP
    Format,
    /// :codeaction - Show code actions (LSP)
    CodeAction,
    /// :rename <newname> - Rename symbol under cursor (LSP)
    Rename(String),
    /// :rename (no args) - Enter rename prompt mode (LSP)
    RenamePrompt,
    /// :HarpoonAdd - Add current file to harpoon
    HarpoonAdd,
    /// :HarpoonMenu - Toggle harpoon menu
    HarpoonMenu,
    /// :Harpoon1-4 - Jump to harpoon slot
    HarpoonJump(usize),
    /// :Terminal - Toggle floating terminal
    ToggleTerminal,
    /// :CopilotAuth - Initiate Copilot sign-in
    CopilotAuth,
    /// :CopilotSignOut - Sign out of Copilot
    CopilotSignOut,
    /// :CopilotStatus - Show Copilot status
    CopilotStatus,
    /// :CopilotToggle - Toggle Copilot on/off
    CopilotToggle,
    /// :Theme <name> - Switch to a theme
    Theme(String),
    /// :Themes - Open theme picker
    Themes,
    /// :marks - Show all marks
    Marks,
    /// Unknown command
    Unknown(String),
}

/// Result of executing a command
#[derive(Debug)]
pub enum CommandResult {
    /// Command executed successfully
    Ok,
    /// Command executed with a message to display
    Message(String),
    /// Command failed with an error
    Error(String),
    /// Quit the editor
    Quit,
    /// Run an external process (requires terminal to handle)
    RunExternal(String),
    /// Request confirmation for delete (shows prompt, user must type :delete! to confirm)
    ConfirmDelete(PathBuf),
}

/// Parse a command string into a Command
pub fn parse_command(input: &str) -> Command {
    let input = input.trim();

    // Handle empty input
    if input.is_empty() {
        return Command::Unknown(String::new());
    }

    // Handle shell command :!command
    if input.starts_with('!') {
        let shell_cmd = input[1..].trim().to_string();
        return Command::Shell(shell_cmd);
    }

    // Handle line number
    if let Ok(line_num) = input.parse::<usize>() {
        return Command::GotoLine(line_num);
    }

    // Handle substitute command: %s/pattern/replacement/flags or s/pattern/replacement/flags
    if let Some(sub_cmd) = parse_substitute_command(input) {
        return sub_cmd;
    }

    // Split into command and arguments
    let mut parts = input.splitn(2, char::is_whitespace);
    let cmd = parts.next().unwrap_or("");
    let args = parts.next().map(|s| s.trim());

    match cmd {
        // Write commands
        "w" | "write" => {
            Command::Write(args.filter(|s| !s.is_empty()).map(PathBuf::from))
        }
        "wa" | "wall" => Command::WriteAll,

        // Quit commands
        "q" | "quit" => Command::Quit,
        "q!" | "quit!" => Command::ForceQuit,
        "qa" | "qall" => Command::QuitAll,
        "qa!" | "qall!" => Command::ForceQuitAll,

        // Write and quit
        "wq" => Command::WriteQuit,
        "wqa" | "wqall" | "xall" => Command::WriteQuitAll,
        "x" | "exit" => Command::WriteQuitIfModified,
        "xa" => Command::WriteQuitAllIfModified,

        // Edit commands
        "e" | "edit" => {
            if args.map(|s| s.is_empty()).unwrap_or(true) {
                Command::Edit(None)
            } else {
                Command::Edit(args.map(PathBuf::from))
            }
        }
        "e!" | "edit!" => Command::Reload,

        // Buffer navigation
        "n" | "next" | "bn" | "bnext" => Command::Next,
        "N" | "prev" | "previous" | "bp" | "bprev" | "bprevious" => Command::Prev,

        // Set options
        "set" => {
            if let Some(arg) = args {
                let mut parts = arg.splitn(2, '=');
                let option = parts.next().unwrap_or("").to_string();
                let value = parts.next().map(|s| s.to_string());
                Command::Set(option, value)
            } else {
                Command::Unknown("set: missing option".to_string())
            }
        }

        // External tools
        "LazyGit" | "lazygit" | "lg" => Command::LazyGit,

        // Split commands
        "vs" | "vsplit" => {
            Command::VSplit(args.filter(|s| !s.is_empty()).map(PathBuf::from))
        }
        "sp" | "split" => {
            Command::HSplit(args.filter(|s| !s.is_empty()).map(PathBuf::from))
        }
        "only" | "on" => Command::Only,

        // Fuzzy finder commands
        "FindFiles" | "findfiles" | "ff" | "files" => Command::FindFiles,
        "FindBuffers" | "findbuffers" | "fb" | "buffers" => Command::FindBuffers,
        "LiveGrep" | "livegrep" | "grep" | "rg" => Command::LiveGrep,
        "FindDiagnostics" | "finddiagnostics" | "diagnostics" | "diag" | "fd" => Command::FindDiagnostics,
        "DiagnosticFloat" | "diagnosticfloat" | "df" | "linediag" => Command::DiagnosticFloat,

        // Clear search highlight
        "noh" | "nohlsearch" => Command::NoHighlight,

        // File management commands
        "new" | "touch" => {
            if let Some(path) = args.filter(|s| !s.is_empty()) {
                Command::NewFile(PathBuf::from(path))
            } else {
                Command::Unknown("new: missing file path".to_string())
            }
        }
        "delete" | "rm" => Command::DeleteFile,
        "delete!" | "rm!" => Command::DeleteFileForce,
        "rename" | "mv" => {
            if let Some(path) = args.filter(|s| !s.is_empty()) {
                Command::RenameFile(PathBuf::from(path))
            } else {
                Command::Unknown("rename: missing new name".to_string())
            }
        }
        "mkdir" => {
            if let Some(path) = args.filter(|s| !s.is_empty()) {
                Command::MakeDir(PathBuf::from(path))
            } else {
                Command::Unknown("mkdir: missing directory path".to_string())
            }
        }

        // File explorer commands
        "Explorer" | "explorer" | "ex" => Command::ToggleExplorer,
        "Explore" | "explore" | "Ex" => Command::OpenExplorer,

        // LSP commands
        "Format" | "format" => Command::Format,
        "codeaction" | "CodeAction" | "ca" => Command::CodeAction,
        "lsprename" | "LspRename" | "rn" => {
            if let Some(new_name) = args.filter(|s| !s.is_empty()) {
                Command::Rename(new_name.to_string())
            } else {
                // No args - enter rename prompt mode
                Command::RenamePrompt
            }
        }

        // Harpoon commands
        "HarpoonAdd" | "harpoonadd" => Command::HarpoonAdd,
        "HarpoonMenu" | "harpoonmenu" => Command::HarpoonMenu,
        "Harpoon1" | "harpoon1" => Command::HarpoonJump(1),
        "Harpoon2" | "harpoon2" => Command::HarpoonJump(2),
        "Harpoon3" | "harpoon3" => Command::HarpoonJump(3),
        "Harpoon4" | "harpoon4" => Command::HarpoonJump(4),

        // Terminal command
        "Terminal" | "terminal" | "term" => Command::ToggleTerminal,

        // Copilot commands
        "CopilotAuth" | "copilotauth" | "Copilot" | "copilot" => Command::CopilotAuth,
        "CopilotSignOut" | "copilotsignout" => Command::CopilotSignOut,
        "CopilotStatus" | "copilotstatus" => Command::CopilotStatus,
        "CopilotToggle" | "copilottoggle" => Command::CopilotToggle,

        // Theme commands
        "Theme" | "theme" | "colorscheme" => {
            if let Some(name) = args.filter(|s| !s.is_empty()) {
                Command::Theme(name.to_string())
            } else {
                Command::Themes // No args opens the picker
            }
        }
        "Themes" | "themes" => Command::Themes,

        // Marks command
        "marks" => Command::Marks,

        // Unknown command
        _ => Command::Unknown(cmd.to_string()),
    }
}

/// Parse a substitute command: %s/pattern/replacement/flags or s/pattern/replacement/flags
fn parse_substitute_command(input: &str) -> Option<Command> {
    // Check for %s or s prefix
    let (entire_file, rest) = if input.starts_with("%s") {
        (true, &input[2..])
    } else if input.starts_with('s') && input.len() > 1 && !input.chars().nth(1).unwrap().is_alphanumeric() {
        (false, &input[1..])
    } else {
        return None;
    };

    // Must have a delimiter after s or %s
    if rest.is_empty() {
        return None;
    }

    // The delimiter is the first character (usually /)
    let delimiter = rest.chars().next()?;
    let rest = &rest[delimiter.len_utf8()..];

    // Split by delimiter, handling escaped delimiters
    let parts = split_by_delimiter(rest, delimiter);
    if parts.len() < 2 {
        return None;
    }

    let pattern = unescape_delimiter(&parts[0], delimiter);
    let replacement = unescape_delimiter(&parts[1], delimiter);
    let flags = if parts.len() > 2 { &parts[2] } else { "" };

    let global = flags.contains('g');

    Some(Command::Substitute {
        entire_file,
        pattern,
        replacement,
        global,
    })
}

/// Split a string by delimiter, respecting escaped delimiters
fn split_by_delimiter(s: &str, delimiter: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            // Check if next char is the delimiter (escaped)
            if chars.peek() == Some(&delimiter) {
                current.push('\\');
                current.push(chars.next().unwrap());
            } else {
                current.push(c);
            }
        } else if c == delimiter {
            parts.push(current);
            current = String::new();
        } else {
            current.push(c);
        }
    }
    parts.push(current);
    parts
}

/// Remove escape sequences for the delimiter
fn unescape_delimiter(s: &str, delimiter: char) -> String {
    let escaped = format!("\\{}", delimiter);
    s.replace(&escaped, &delimiter.to_string())
}

/// Command line state
#[derive(Debug, Clone, Default)]
pub struct CommandLine {
    /// The current input buffer
    pub input: String,
    /// Cursor position in the input
    pub cursor: usize,
    /// Command history
    pub history: Vec<String>,
    /// Current position in history (for up/down navigation)
    pub history_index: Option<usize>,
    /// Saved input when browsing history
    pub saved_input: Option<String>,
}

impl CommandLine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear the command line
    pub fn clear(&mut self) {
        self.input.clear();
        self.cursor = 0;
        self.history_index = None;
        self.saved_input = None;
    }

    /// Convert char index to byte index
    fn char_to_byte_index(&self, char_idx: usize) -> usize {
        self.input
            .char_indices()
            .nth(char_idx)
            .map(|(byte_idx, _)| byte_idx)
            .unwrap_or(self.input.len())
    }

    /// Get the number of characters in the input
    fn char_count(&self) -> usize {
        self.input.chars().count()
    }

    /// Insert a character at the cursor position (cursor is char index)
    pub fn insert_char(&mut self, ch: char) {
        let byte_idx = self.char_to_byte_index(self.cursor);
        self.input.insert(byte_idx, ch);
        self.cursor += 1;
    }

    /// Delete character before cursor (backspace)
    pub fn delete_char_before(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let byte_idx = self.char_to_byte_index(self.cursor);
            self.input.remove(byte_idx);
        }
    }

    /// Delete character at cursor (delete key)
    pub fn delete_char_at(&mut self) {
        if self.cursor < self.char_count() {
            let byte_idx = self.char_to_byte_index(self.cursor);
            self.input.remove(byte_idx);
        }
    }

    /// Move cursor left
    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    /// Move cursor right
    pub fn move_right(&mut self) {
        if self.cursor < self.char_count() {
            self.cursor += 1;
        }
    }

    /// Move cursor to start
    pub fn move_to_start(&mut self) {
        self.cursor = 0;
    }

    /// Move cursor to end
    pub fn move_to_end(&mut self) {
        self.cursor = self.char_count();
    }

    /// Navigate to previous history entry
    pub fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }

        match self.history_index {
            None => {
                // Save current input and go to most recent history
                self.saved_input = Some(self.input.clone());
                self.history_index = Some(self.history.len() - 1);
                self.input = self.history[self.history.len() - 1].clone();
            }
            Some(idx) if idx > 0 => {
                self.history_index = Some(idx - 1);
                self.input = self.history[idx - 1].clone();
            }
            _ => {}
        }
        self.cursor = self.char_count();
    }

    /// Navigate to next history entry
    pub fn history_next(&mut self) {
        match self.history_index {
            Some(idx) => {
                if idx + 1 < self.history.len() {
                    self.history_index = Some(idx + 1);
                    self.input = self.history[idx + 1].clone();
                } else {
                    // Restore saved input
                    self.history_index = None;
                    if let Some(saved) = self.saved_input.take() {
                        self.input = saved;
                    }
                }
                self.cursor = self.char_count();
            }
            None => {}
        }
    }

    /// Add current input to history and execute
    pub fn execute(&mut self) -> Command {
        let input = self.input.trim().to_string();

        // Add to history if non-empty and different from last entry
        if !input.is_empty() {
            if self.history.last().map(|s| s != &input).unwrap_or(true) {
                self.history.push(input.clone());
            }
        }

        let cmd = parse_command(&input);
        self.clear();
        cmd
    }

    /// Get display string (with ':' prefix)
    pub fn display(&self) -> String {
        format!(":{}", self.input)
    }
}
