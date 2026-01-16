use std::path::PathBuf;

/// Parsed command from command line
#[derive(Debug, Clone)]
pub enum Command {
    /// :w [filename] - Write buffer to file
    Write(Option<PathBuf>),
    /// :q - Quit (fails if unsaved changes)
    Quit,
    /// :q! - Force quit (discard changes)
    ForceQuit,
    /// :wq - Write and quit
    WriteQuit,
    /// :x - Write if modified and quit
    WriteQuitIfModified,
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

    // Split into command and arguments
    let mut parts = input.splitn(2, char::is_whitespace);
    let cmd = parts.next().unwrap_or("");
    let args = parts.next().map(|s| s.trim());

    match cmd {
        // Write commands
        "w" | "write" => {
            Command::Write(args.filter(|s| !s.is_empty()).map(PathBuf::from))
        }

        // Quit commands
        "q" | "quit" => Command::Quit,
        "q!" | "quit!" => Command::ForceQuit,

        // Write and quit
        "wq" => Command::WriteQuit,
        "x" | "exit" => Command::WriteQuitIfModified,

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

        // Unknown command
        _ => Command::Unknown(cmd.to_string()),
    }
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

    /// Insert a character at the cursor position
    pub fn insert_char(&mut self, ch: char) {
        self.input.insert(self.cursor, ch);
        self.cursor += 1;
    }

    /// Delete character before cursor (backspace)
    pub fn delete_char_before(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.input.remove(self.cursor);
        }
    }

    /// Delete character at cursor (delete key)
    pub fn delete_char_at(&mut self) {
        if self.cursor < self.input.len() {
            self.input.remove(self.cursor);
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
        if self.cursor < self.input.len() {
            self.cursor += 1;
        }
    }

    /// Move cursor to start
    pub fn move_to_start(&mut self) {
        self.cursor = 0;
    }

    /// Move cursor to end
    pub fn move_to_end(&mut self) {
        self.cursor = self.input.len();
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
        self.cursor = self.input.len();
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
                self.cursor = self.input.len();
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
