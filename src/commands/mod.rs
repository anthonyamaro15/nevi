use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

/// Parsed command from command line
#[derive(Debug, Clone)]
pub enum Command {
    /// :w [filename] - Write buffer to file
    Write(Option<PathBuf>),
    /// :w! [filename] - Force write buffer to file
    ForceWrite(Option<PathBuf>),
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
    /// :bd or :bdelete - Close the current buffer
    BufferDelete(bool),
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
    /// :BufferSearch - Open fuzzy finder for lines in current buffer
    BufferSearch,
    /// :LiveGrep - Open fuzzy finder for live grep
    LiveGrep,
    /// :SearchWord - Live grep with word under cursor
    SearchWord,
    /// :FindDiagnostics - Open fuzzy finder for LSP diagnostics
    FindDiagnostics,
    /// :GitChanges - Open fuzzy finder for changed Git files
    GitChanges,
    /// :DiagnosticFloat - Show diagnostic floating popup at cursor line
    DiagnosticFloat,
    /// :MarkdownPreview - Open a rendered floating Markdown preview
    MarkdownPreview,
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
    /// :ProjectReplace/pattern/replacement/flags - Preview project-wide replace
    ProjectReplace {
        /// Search pattern
        pattern: String,
        /// Replacement string
        replacement: String,
        /// Global flag (replace all on line vs first only)
        global: bool,
    },
    /// :ProjectReplaceApply - Apply the last project-wide replace preview
    ProjectReplaceApply,
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
    /// :TerminalNew [name] - Create a new floating terminal session
    TerminalNew(Option<String>),
    /// :TerminalNext - Switch to next floating terminal session
    TerminalNext,
    /// :TerminalPrev - Switch to previous floating terminal session
    TerminalPrev,
    /// :TerminalList - List floating terminal sessions
    TerminalList,
    /// :Terminals - Open floating terminal session picker
    TerminalPicker,
    /// :TerminalSelect {index} - Select floating terminal session
    TerminalSelect(usize),
    /// :TerminalRename [index] {name} - Rename a floating terminal session
    TerminalRename(Option<usize>, String),
    /// :TerminalRename - Prompt to rename the active floating terminal session
    TerminalRenamePrompt,
    /// :TerminalKill - Kill the floating terminal process
    TerminalKill,
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
    /// :Keymaps - Open searchable keybinding cheatsheet
    Keymaps,
    /// :checkhealth - Open the editor health report
    CheckHealth,
    /// :ToolInstall - Open missing tool install plan
    ToolInstall,
    /// :FlightRecorder - Open recent performance timing report
    FlightRecorder,
    /// :Jump - Start labeled jump navigation for visible text
    Jump,
    /// :ConfigOpen - Open the user config file
    ConfigOpen,
    /// :ConfigDefaults - Open the latest default config template
    ConfigDefaults,
    /// :marks - Show all marks
    Marks,
    /// :delmarks {marks} - Delete specified marks
    DeleteMarks(String),
    /// :delmarks! - Delete all lowercase marks in current buffer
    DeleteMarksAll,
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

/// Where the command-line popup content is coming from
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CommandPopupMode {
    #[default]
    None,
    Completion,
    History,
}

/// One command candidate shown in completion/fuzzy search
#[derive(Debug, Clone, Copy)]
pub struct CommandSuggestion {
    /// Command inserted on completion accept
    pub command: &'static str,
    /// Human-friendly description shown in popup
    pub description: &'static str,
    /// Alias that matched the query (can differ from `command`)
    pub matched_alias: &'static str,
    /// Whether this command usually expects arguments
    pub takes_args: bool,
}

#[derive(Debug, Clone, Copy)]
struct CommandSpec {
    command: &'static str,
    aliases: &'static [&'static str],
    description: &'static str,
    takes_args: bool,
}

const COMMAND_SPECS: &[CommandSpec] = &[
    CommandSpec {
        command: "w",
        aliases: &["write"],
        description: "Write current buffer unless it changed on disk",
        takes_args: false,
    },
    CommandSpec {
        command: "w!",
        aliases: &["write!"],
        description: "Force write current buffer and overwrite disk changes",
        takes_args: false,
    },
    CommandSpec {
        command: "wa",
        aliases: &["wall"],
        description: "Write all modified buffers",
        takes_args: false,
    },
    CommandSpec {
        command: "q",
        aliases: &["quit"],
        description: "Quit current pane/editor",
        takes_args: false,
    },
    CommandSpec {
        command: "q!",
        aliases: &["quit!"],
        description: "Force quit (discard changes)",
        takes_args: false,
    },
    CommandSpec {
        command: "qa",
        aliases: &["qall"],
        description: "Quit all (checks unsaved changes)",
        takes_args: false,
    },
    CommandSpec {
        command: "qa!",
        aliases: &["qall!"],
        description: "Force quit all (discard changes)",
        takes_args: false,
    },
    CommandSpec {
        command: "wq",
        aliases: &[],
        description: "Write and quit",
        takes_args: false,
    },
    CommandSpec {
        command: "wqa",
        aliases: &["wqall", "xall"],
        description: "Write all and quit all",
        takes_args: false,
    },
    CommandSpec {
        command: "x",
        aliases: &["exit"],
        description: "Write if modified and quit",
        takes_args: false,
    },
    CommandSpec {
        command: "xa",
        aliases: &[],
        description: "Write all if modified and quit all",
        takes_args: false,
    },
    CommandSpec {
        command: "e",
        aliases: &["edit"],
        description: "Edit a file or reload current",
        takes_args: false,
    },
    CommandSpec {
        command: "e!",
        aliases: &["edit!"],
        description: "Reload current file (discard changes)",
        takes_args: false,
    },
    CommandSpec {
        command: "n",
        aliases: &["next", "bn", "bnext"],
        description: "Go to next buffer",
        takes_args: false,
    },
    CommandSpec {
        command: "N",
        aliases: &["prev", "previous", "bp", "bprev", "bprevious"],
        description: "Go to previous buffer",
        takes_args: false,
    },
    CommandSpec {
        command: "bd",
        aliases: &["bdelete"],
        description: "Delete current buffer",
        takes_args: false,
    },
    CommandSpec {
        command: "bd!",
        aliases: &["bdelete!"],
        description: "Force delete current buffer",
        takes_args: false,
    },
    CommandSpec {
        command: "set",
        aliases: &[],
        description: "Set an editor option",
        takes_args: true,
    },
    CommandSpec {
        command: "!",
        aliases: &[],
        description: "Run shell command",
        takes_args: true,
    },
    CommandSpec {
        command: "vs",
        aliases: &["vsplit"],
        description: "Open vertical split",
        takes_args: false,
    },
    CommandSpec {
        command: "sp",
        aliases: &["split"],
        description: "Open horizontal split",
        takes_args: false,
    },
    CommandSpec {
        command: "only",
        aliases: &["on"],
        description: "Close all other panes",
        takes_args: false,
    },
    CommandSpec {
        command: "FindFiles",
        aliases: &["findfiles", "ff", "files"],
        description: "Open file finder",
        takes_args: false,
    },
    CommandSpec {
        command: "FindBuffers",
        aliases: &["findbuffers", "fb", "buffers"],
        description: "Open buffer finder",
        takes_args: false,
    },
    CommandSpec {
        command: "BufferSearch",
        aliases: &["buffersearch", "findlines", "lines", "bl"],
        description: "Find lines in current buffer",
        takes_args: false,
    },
    CommandSpec {
        command: "LiveGrep",
        aliases: &["livegrep", "grep", "rg"],
        description: "Search text in project",
        takes_args: false,
    },
    CommandSpec {
        command: "SearchWord",
        aliases: &["searchword", "sw"],
        description: "Search word under cursor",
        takes_args: false,
    },
    CommandSpec {
        command: "FindDiagnostics",
        aliases: &["finddiagnostics", "diagnostics", "diag", "fd"],
        description: "Open diagnostics finder",
        takes_args: false,
    },
    CommandSpec {
        command: "GitChanges",
        aliases: &["gitchanges", "changes", "gc"],
        description: "Open Git changes finder",
        takes_args: false,
    },
    CommandSpec {
        command: "DiagnosticFloat",
        aliases: &["diagnosticfloat", "df", "linediag"],
        description: "Show diagnostics at cursor line",
        takes_args: false,
    },
    CommandSpec {
        command: "MarkdownPreview",
        aliases: &["markdownpreview", "mdpreview", "mdp"],
        description: "Open rendered Markdown preview",
        takes_args: false,
    },
    CommandSpec {
        command: "noh",
        aliases: &["nohlsearch"],
        description: "Clear search highlights",
        takes_args: false,
    },
    CommandSpec {
        command: "s",
        aliases: &[],
        description: "Substitute on current line",
        takes_args: true,
    },
    CommandSpec {
        command: "%s",
        aliases: &[],
        description: "Substitute in entire file",
        takes_args: true,
    },
    CommandSpec {
        command: "ProjectReplace",
        aliases: &["projectreplace", "PReplace", "preplace"],
        description: "Preview project-wide replace",
        takes_args: true,
    },
    CommandSpec {
        command: "ProjectReplaceApply",
        aliases: &["projectreplaceapply", "PReplaceApply", "preplaceapply"],
        description: "Apply last project replace preview",
        takes_args: false,
    },
    CommandSpec {
        command: "new",
        aliases: &["touch"],
        description: "Create new file",
        takes_args: true,
    },
    CommandSpec {
        command: "delete",
        aliases: &["rm"],
        description: "Delete current file (confirm)",
        takes_args: false,
    },
    CommandSpec {
        command: "delete!",
        aliases: &["rm!"],
        description: "Delete current file (force)",
        takes_args: false,
    },
    CommandSpec {
        command: "rename",
        aliases: &["mv"],
        description: "Rename current file",
        takes_args: true,
    },
    CommandSpec {
        command: "mkdir",
        aliases: &[],
        description: "Create directory",
        takes_args: true,
    },
    CommandSpec {
        command: "Explorer",
        aliases: &["explorer", "ex"],
        description: "Toggle file explorer",
        takes_args: false,
    },
    CommandSpec {
        command: "Explore",
        aliases: &["explore", "Ex"],
        description: "Open file explorer",
        takes_args: false,
    },
    CommandSpec {
        command: "Format",
        aliases: &["format"],
        description: "Format current document",
        takes_args: false,
    },
    CommandSpec {
        command: "codeaction",
        aliases: &["CodeAction", "ca"],
        description: "Show LSP code actions",
        takes_args: false,
    },
    CommandSpec {
        command: "rn",
        aliases: &["lsprename", "LspRename"],
        description: "LSP rename symbol",
        takes_args: false,
    },
    CommandSpec {
        command: "HarpoonAdd",
        aliases: &["harpoonadd"],
        description: "Add file to harpoon",
        takes_args: false,
    },
    CommandSpec {
        command: "HarpoonMenu",
        aliases: &["harpoonmenu"],
        description: "Open harpoon menu",
        takes_args: false,
    },
    CommandSpec {
        command: "Harpoon1",
        aliases: &["harpoon1"],
        description: "Jump to harpoon slot 1",
        takes_args: false,
    },
    CommandSpec {
        command: "Harpoon2",
        aliases: &["harpoon2"],
        description: "Jump to harpoon slot 2",
        takes_args: false,
    },
    CommandSpec {
        command: "Harpoon3",
        aliases: &["harpoon3"],
        description: "Jump to harpoon slot 3",
        takes_args: false,
    },
    CommandSpec {
        command: "Harpoon4",
        aliases: &["harpoon4"],
        description: "Jump to harpoon slot 4",
        takes_args: false,
    },
    CommandSpec {
        command: "Terminal",
        aliases: &["terminal", "term"],
        description: "Toggle floating terminal",
        takes_args: false,
    },
    CommandSpec {
        command: "TerminalNew",
        aliases: &["terminalnew", "termnew"],
        description: "Create floating terminal session",
        takes_args: true,
    },
    CommandSpec {
        command: "TerminalNext",
        aliases: &["terminalnext", "termnext"],
        description: "Switch to next floating terminal session",
        takes_args: false,
    },
    CommandSpec {
        command: "TerminalPrev",
        aliases: &["terminalprev", "termprev"],
        description: "Switch to previous floating terminal session",
        takes_args: false,
    },
    CommandSpec {
        command: "TerminalList",
        aliases: &["terminallist", "termlist", "termls"],
        description: "List floating terminal sessions",
        takes_args: false,
    },
    CommandSpec {
        command: "Terminals",
        aliases: &[
            "terminals",
            "terminalmenu",
            "termmenu",
            "terminalpicker",
            "termpicker",
        ],
        description: "Open floating terminal session picker",
        takes_args: false,
    },
    CommandSpec {
        command: "TerminalSelect",
        aliases: &["terminalselect", "termsel", "termselect"],
        description: "Select floating terminal session",
        takes_args: true,
    },
    CommandSpec {
        command: "TerminalRename",
        aliases: &["terminalrename", "termrename"],
        description: "Rename floating terminal session",
        takes_args: true,
    },
    CommandSpec {
        command: "TerminalKill",
        aliases: &["terminalkill", "termkill"],
        description: "Kill floating terminal",
        takes_args: false,
    },
    CommandSpec {
        command: "CopilotAuth",
        aliases: &["copilotauth", "Copilot", "copilot"],
        description: "Sign in to Copilot",
        takes_args: false,
    },
    CommandSpec {
        command: "CopilotSignOut",
        aliases: &["copilotsignout"],
        description: "Sign out of Copilot",
        takes_args: false,
    },
    CommandSpec {
        command: "CopilotStatus",
        aliases: &["copilotstatus"],
        description: "Show Copilot status",
        takes_args: false,
    },
    CommandSpec {
        command: "CopilotToggle",
        aliases: &["copilottoggle"],
        description: "Toggle Copilot on/off",
        takes_args: false,
    },
    CommandSpec {
        command: "Theme",
        aliases: &["theme", "colorscheme"],
        description: "Switch theme",
        takes_args: false,
    },
    CommandSpec {
        command: "Themes",
        aliases: &["themes"],
        description: "Open theme picker",
        takes_args: false,
    },
    CommandSpec {
        command: "Keymaps",
        aliases: &["keymaps", "keys"],
        description: "Open searchable keybinding cheatsheet",
        takes_args: false,
    },
    CommandSpec {
        command: "checkhealth",
        aliases: &["CheckHealth", "Health", "health"],
        description: "Open editor health report",
        takes_args: false,
    },
    CommandSpec {
        command: "ToolInstall",
        aliases: &[
            "toolinstall",
            "toolsinstall",
            "ToolsInstall",
            "LspInstall",
            "lspinstall",
        ],
        description: "Open missing LSP/tool install plan",
        takes_args: false,
    },
    CommandSpec {
        command: "FlightRecorder",
        aliases: &["flightrecorder", "flight", "WhySlow", "whyslow"],
        description: "Open recent performance timing report",
        takes_args: false,
    },
    CommandSpec {
        command: "Jump",
        aliases: &["jump"],
        description: "Jump to visible text with labels",
        takes_args: false,
    },
    CommandSpec {
        command: "ConfigOpen",
        aliases: &["configopen", "config", "ConfigEdit", "configedit"],
        description: "Open user config file",
        takes_args: false,
    },
    CommandSpec {
        command: "ConfigDefaults",
        aliases: &["configdefaults", "defaults"],
        description: "Open latest default config in read-only buffer",
        takes_args: false,
    },
    CommandSpec {
        command: "marks",
        aliases: &[],
        description: "Show all marks",
        takes_args: false,
    },
    CommandSpec {
        command: "delmarks",
        aliases: &["delm"],
        description: "Delete specified marks",
        takes_args: true,
    },
    CommandSpec {
        command: "delmarks!",
        aliases: &["delm!"],
        description: "Delete all local lowercase marks",
        takes_args: false,
    },
];

const MAX_COMMAND_SUGGESTIONS: usize = 12;
const MAX_HISTORY_ITEMS: usize = 20;
const MAX_HISTORY_ENTRIES: usize = 500;

/// Build fuzzy command suggestions for the current command-line input.
pub fn command_suggestions(input: &str, limit: usize) -> Vec<CommandSuggestion> {
    let (_, token, _) = split_input_segments(input);
    command_suggestions_for_token(token, limit)
}

fn command_suggestions_for_token(token: &str, limit: usize) -> Vec<CommandSuggestion> {
    let limit = limit.max(1);
    let token = token.trim();

    if token.is_empty() {
        return COMMAND_SPECS
            .iter()
            .take(limit)
            .map(|spec| CommandSuggestion {
                command: spec.command,
                description: spec.description,
                matched_alias: spec.command,
                takes_args: spec.takes_args,
            })
            .collect();
    }

    let token_lower = token.to_lowercase();

    let mut scored: Vec<(i32, CommandSuggestion)> = Vec::new();
    for spec in COMMAND_SPECS {
        let mut best_score = None::<i32>;
        let mut best_alias = spec.command;

        for alias in std::iter::once(&spec.command).chain(spec.aliases.iter()) {
            if let Some(score) = alias_match_score(&token_lower, alias) {
                if best_score.map(|v| score > v).unwrap_or(true) {
                    best_score = Some(score);
                    best_alias = alias;
                }
            }
        }

        if let Some(score) = best_score {
            let canonical_bonus = if best_alias.eq_ignore_ascii_case(spec.command) {
                8
            } else {
                0
            };
            scored.push((
                score + canonical_bonus,
                CommandSuggestion {
                    command: spec.command,
                    description: spec.description,
                    matched_alias: best_alias,
                    takes_args: spec.takes_args,
                },
            ));
        }
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.command.cmp(b.1.command)));

    scored
        .into_iter()
        .take(limit)
        .map(|(_, suggestion)| suggestion)
        .collect()
}

fn alias_match_score(query_lower: &str, alias: &str) -> Option<i32> {
    if query_lower.is_empty() {
        return Some(0);
    }

    let alias_lower = alias.to_lowercase();

    if alias_lower == query_lower {
        return Some(1200);
    }

    if alias_lower.starts_with(query_lower) {
        let tail_penalty = alias_lower.len().saturating_sub(query_lower.len()) as i32;
        return Some(1000 - tail_penalty);
    }

    if let Some(pos) = alias_lower.find(query_lower) {
        return Some(780 - (pos as i32 * 12));
    }

    fuzzy_subsequence_score(query_lower, &alias_lower).map(|score| 520 + score)
}

fn fuzzy_subsequence_score(query_lower: &str, candidate_lower: &str) -> Option<i32> {
    if query_lower.is_empty() {
        return Some(0);
    }

    let query: Vec<char> = query_lower.chars().collect();
    let candidate: Vec<char> = candidate_lower.chars().collect();

    let mut query_idx = 0;
    let mut score = 0i32;
    let mut last_match_idx: Option<usize> = None;

    for (idx, ch) in candidate.iter().enumerate() {
        if query_idx >= query.len() {
            break;
        }

        if *ch == query[query_idx] {
            score += 12;

            if let Some(prev) = last_match_idx {
                if idx == prev + 1 {
                    score += 7;
                } else {
                    score -= (idx.saturating_sub(prev + 1) as i32).min(5);
                }
            } else {
                score += (candidate.len().saturating_sub(idx) as i32).min(12);
            }

            last_match_idx = Some(idx);
            query_idx += 1;
        }
    }

    if query_idx == query.len() {
        Some(score - candidate.len().saturating_sub(query.len()) as i32)
    } else {
        None
    }
}

fn split_input_segments(input: &str) -> (&str, &str, &str) {
    let mut start = input.len();
    for (idx, ch) in input.char_indices() {
        if !ch.is_whitespace() {
            start = idx;
            break;
        }
    }

    if start == input.len() {
        return (input, "", "");
    }

    let mut end = input.len();
    for (offset, ch) in input[start..].char_indices() {
        if ch.is_whitespace() {
            end = start + offset;
            break;
        }
    }

    (&input[..start], &input[start..end], &input[end..])
}

fn common_prefix_preserving_left_case(left: &str, right: &str) -> String {
    let mut end_byte = 0;

    for ((left_idx, left_ch), right_ch) in left.char_indices().zip(right.chars()) {
        if left_ch.eq_ignore_ascii_case(&right_ch) {
            end_byte = left_idx + left_ch.len_utf8();
        } else {
            break;
        }
    }

    left[..end_byte].to_string()
}

fn longest_common_command_prefix(suggestions: &[CommandSuggestion]) -> String {
    let Some(first) = suggestions.first() else {
        return String::new();
    };

    let mut prefix = first.command.to_string();
    for suggestion in suggestions.iter().skip(1) {
        prefix = common_prefix_preserving_left_case(&prefix, suggestion.command);
        if prefix.is_empty() {
            break;
        }
    }

    prefix
}

fn command_history_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("nevi")
        .join("command_history.txt")
}

/// Parse a command string into a Command
/// Rows for the keymap cheatsheet: `(":CommandName", description)` for every
/// registered command. Sourced from the live `COMMAND_SPECS` registry, so this
/// can never drift from the commands the editor actually accepts.
pub fn command_cheatsheet_rows() -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = COMMAND_SPECS
        .iter()
        .map(|spec| (format!(":{}", spec.command), spec.description.to_string()))
        .collect();
    // `:{number}` (go to line) is parser-only — not a named spec. (`:q!`/`:qa!`
    // are already in COMMAND_SPECS, so they must not be re-added here.)
    rows.push((":{number}".to_string(), "Go to line number".to_string()));
    rows
}

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

    // Handle project replace command: ProjectReplace/pattern/replacement/flags
    if let Some(replace_cmd) = parse_project_replace_command(input) {
        return replace_cmd;
    }

    // Split into command and arguments
    let mut parts = input.splitn(2, char::is_whitespace);
    let cmd = parts.next().unwrap_or("");
    let args = parts.next().map(|s| s.trim());

    match cmd {
        // Write commands
        "w" | "write" => Command::Write(args.filter(|s| !s.is_empty()).map(PathBuf::from)),
        "w!" | "write!" => Command::ForceWrite(args.filter(|s| !s.is_empty()).map(PathBuf::from)),
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
        "bd" | "bdelete" => Command::BufferDelete(false),
        "bd!" | "bdelete!" => Command::BufferDelete(true),

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
        "vs" | "vsplit" => Command::VSplit(args.filter(|s| !s.is_empty()).map(PathBuf::from)),
        "sp" | "split" => Command::HSplit(args.filter(|s| !s.is_empty()).map(PathBuf::from)),
        "only" | "on" => Command::Only,

        // Fuzzy finder commands
        "FindFiles" | "findfiles" | "ff" | "files" => Command::FindFiles,
        "FindBuffers" | "findbuffers" | "fb" | "buffers" => Command::FindBuffers,
        "BufferSearch" | "buffersearch" | "FindLines" | "findlines" | "Lines" | "lines" | "bl" => {
            Command::BufferSearch
        }
        "LiveGrep" | "livegrep" | "grep" | "rg" => Command::LiveGrep,
        "SearchWord" | "searchword" | "sw" => Command::SearchWord,
        "FindDiagnostics" | "finddiagnostics" | "diagnostics" | "diag" | "fd" => {
            Command::FindDiagnostics
        }
        "GitChanges" | "gitchanges" | "changes" | "gc" => Command::GitChanges,
        "DiagnosticFloat" | "diagnosticfloat" | "df" | "linediag" => Command::DiagnosticFloat,
        "MarkdownPreview" | "markdownpreview" | "mdpreview" | "mdp" => Command::MarkdownPreview,

        // Clear search highlight
        "noh" | "nohlsearch" => Command::NoHighlight,

        // Project-wide replace
        "ProjectReplaceApply" | "projectreplaceapply" | "PReplaceApply" | "preplaceapply" => {
            Command::ProjectReplaceApply
        }

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
        "TerminalNew" | "terminalnew" | "TermNew" | "termnew" => {
            Command::TerminalNew(args.map(|value| value.to_string()))
        }
        "TerminalNext" | "terminalnext" | "TermNext" | "termnext" => Command::TerminalNext,
        "TerminalPrev" | "terminalprev" | "TermPrev" | "termprev" => Command::TerminalPrev,
        "TerminalList" | "terminallist" | "TermList" | "termlist" | "termls" => {
            Command::TerminalList
        }
        "Terminals" | "terminals" | "TerminalMenu" | "terminalmenu" | "TermMenu" | "termmenu"
        | "TerminalPicker" | "terminalpicker" | "TermPicker" | "termpicker" => {
            Command::TerminalPicker
        }
        "TerminalSelect" | "terminalselect" | "TermSelect" | "termselect" | "termsel" => {
            if let Some(index) = args.and_then(|value| value.parse::<usize>().ok()) {
                Command::TerminalSelect(index)
            } else {
                Command::Unknown("termselect: missing session number".to_string())
            }
        }
        "TerminalRename" | "terminalrename" | "TermRename" | "termrename" => {
            parse_terminal_rename_args(args)
        }
        "TerminalKill" | "terminalkill" | "TermKill" | "termkill" => Command::TerminalKill,

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
        "Keymaps" | "keymaps" | "keys" => Command::Keymaps,
        "checkhealth" | "CheckHealth" | "Health" | "health" => Command::CheckHealth,
        "ToolInstall" | "toolinstall" | "ToolsInstall" | "toolsinstall" | "LspInstall"
        | "lspinstall" => Command::ToolInstall,
        "FlightRecorder" | "flightrecorder" | "flight" | "WhySlow" | "whyslow" => {
            Command::FlightRecorder
        }
        "Jump" | "jump" => Command::Jump,
        "ConfigOpen" | "configopen" | "config" | "ConfigEdit" | "configedit" => Command::ConfigOpen,
        "ConfigDefaults" | "configdefaults" | "defaults" => Command::ConfigDefaults,

        // Marks commands
        "marks" => Command::Marks,
        "delmarks" | "delm" => {
            if let Some(arg) = args.filter(|s| !s.is_empty()) {
                Command::DeleteMarks(arg.to_string())
            } else {
                Command::Unknown("delmarks: missing mark argument".to_string())
            }
        }
        "delmarks!" | "delm!" => Command::DeleteMarksAll,

        // Unknown command
        _ => Command::Unknown(cmd.to_string()),
    }
}

fn parse_terminal_rename_args(args: Option<&str>) -> Command {
    let Some(args) = args.map(str::trim).filter(|value| !value.is_empty()) else {
        return Command::TerminalRenamePrompt;
    };

    let mut parts = args.splitn(2, char::is_whitespace);
    let first = parts.next().unwrap_or("");
    let rest = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if let (Ok(position), Some(name)) = (first.parse::<usize>(), rest) {
        Command::TerminalRename(Some(position), name.to_string())
    } else {
        Command::TerminalRename(None, args.to_string())
    }
}

/// Parse a substitute command: %s/pattern/replacement/flags or s/pattern/replacement/flags
fn parse_substitute_command(input: &str) -> Option<Command> {
    // Check for %s or s prefix
    let (entire_file, rest) = if input.starts_with("%s") {
        (true, &input[2..])
    } else if input.starts_with('s')
        && input.len() > 1
        && !input.chars().nth(1).unwrap().is_alphanumeric()
    {
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

/// Parse a project replace command: ProjectReplace/pattern/replacement/flags
fn parse_project_replace_command(input: &str) -> Option<Command> {
    let rest = ["ProjectReplace", "projectreplace", "PReplace", "preplace"]
        .iter()
        .find_map(|prefix| input.strip_prefix(prefix))?
        .trim_start();

    if rest.is_empty() {
        return None;
    }

    let delimiter = rest.chars().next()?;
    let rest = &rest[delimiter.len_utf8()..];
    let parts = split_by_delimiter(rest, delimiter);
    if parts.len() < 2 {
        return None;
    }

    let pattern = unescape_delimiter(&parts[0], delimiter);
    let replacement = unescape_delimiter(&parts[1], delimiter);
    let flags = if parts.len() > 2 { &parts[2] } else { "" };

    Some(Command::ProjectReplace {
        pattern,
        replacement,
        global: flags.contains('g'),
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
    /// Current popup mode (completion/history)
    pub popup_mode: CommandPopupMode,
    /// Fuzzy command suggestions for current input
    pub suggestions: Vec<CommandSuggestion>,
    /// Selected item in the suggestions popup
    pub suggestion_index: usize,
    /// Filtered history entries shown when history window is open
    pub history_popup_items: Vec<String>,
    /// Selected item in history popup
    pub history_popup_index: usize,
    /// Waiting for a register name after command-line Ctrl+r
    pub pending_register: bool,
    /// Waiting to insert the next key literally after command-line Ctrl+v/Ctrl+q
    pub pending_literal: bool,
    /// Waiting for one or two digraph characters after command-line Ctrl+k
    pub pending_digraph: PendingDigraph,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PendingDigraph {
    #[default]
    None,
    First,
    Second(char),
}

impl CommandLine {
    pub fn new() -> Self {
        let mut state = Self::default();
        state.load_history();
        state
    }

    /// Prepare a fresh command prompt state when entering `:` mode.
    pub fn begin_prompt(&mut self) {
        self.clear();
        self.refresh_command_suggestions();
    }

    /// Prepare command mode with prefilled input.
    pub fn begin_prompt_with_input(&mut self, input: impl Into<String>) {
        self.clear();
        self.input = input.into();
        self.cursor = self.char_count();
        self.refresh_command_suggestions();
    }

    /// Clear the command line
    pub fn clear(&mut self) {
        self.input.clear();
        self.cursor = 0;
        self.history_index = None;
        self.saved_input = None;
        self.popup_mode = CommandPopupMode::None;
        self.suggestions.clear();
        self.suggestion_index = 0;
        self.history_popup_items.clear();
        self.history_popup_index = 0;
        self.pending_register = false;
        self.pending_literal = false;
        self.pending_digraph = PendingDigraph::None;
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
        self.on_input_edited();
    }

    /// Insert text at the command-line cursor.
    pub fn insert_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        let byte_idx = self.char_to_byte_index(self.cursor);
        self.input.insert_str(byte_idx, text);
        self.cursor += text.chars().count();
        self.on_input_edited();
    }

    /// Delete character before cursor (backspace)
    pub fn delete_char_before(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let byte_idx = self.char_to_byte_index(self.cursor);
            self.input.remove(byte_idx);
            self.on_input_edited();
        }
    }

    /// Delete character at cursor (delete key)
    pub fn delete_char_at(&mut self) {
        if self.cursor < self.char_count() {
            let byte_idx = self.char_to_byte_index(self.cursor);
            self.input.remove(byte_idx);
            self.on_input_edited();
        }
    }

    /// Delete the word before the cursor, matching Vim command-line Ctrl+w.
    pub fn delete_word_before(&mut self) {
        if self.cursor == 0 {
            return;
        }

        let chars: Vec<char> = self.input.chars().collect();
        let start_cursor = self.cursor.min(chars.len());
        let mut cursor = start_cursor;

        while cursor > 0 && chars[cursor - 1].is_whitespace() {
            cursor -= 1;
        }

        if cursor > 0 {
            let delete_word_chars = chars[cursor - 1].is_alphanumeric() || chars[cursor - 1] == '_';
            if delete_word_chars {
                while cursor > 0
                    && (chars[cursor - 1].is_alphanumeric() || chars[cursor - 1] == '_')
                {
                    cursor -= 1;
                }
            } else {
                while cursor > 0 {
                    let ch = chars[cursor - 1];
                    if ch.is_whitespace() || ch.is_alphanumeric() || ch == '_' {
                        break;
                    }
                    cursor -= 1;
                }
            }
        }

        if cursor < start_cursor {
            let start_byte = self.char_to_byte_index(cursor);
            let end_byte = self.char_to_byte_index(start_cursor);
            self.input.replace_range(start_byte..end_byte, "");
            self.cursor = cursor;
            self.on_input_edited();
        }
    }

    /// Delete from the cursor back to the start of the command line.
    pub fn delete_to_start(&mut self) {
        if self.cursor == 0 {
            return;
        }

        let end_byte = self.char_to_byte_index(self.cursor);
        self.input.replace_range(0..end_byte, "");
        self.cursor = 0;
        self.on_input_edited();
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
        self.popup_mode = CommandPopupMode::None;
        self.refresh_command_suggestions();
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
        self.popup_mode = CommandPopupMode::None;
        self.refresh_command_suggestions();
    }

    /// Toggle the command history popup window.
    pub fn toggle_history_popup(&mut self) {
        if self.popup_mode == CommandPopupMode::History {
            self.popup_mode = if self.suggestions.is_empty() {
                CommandPopupMode::None
            } else {
                CommandPopupMode::Completion
            };
            self.history_popup_items.clear();
            self.history_popup_index = 0;
            return;
        }

        self.popup_mode = CommandPopupMode::History;
        self.refresh_history_popup();
    }

    /// Open the command-line history window.
    pub fn open_command_line_window(&mut self) {
        self.popup_mode = CommandPopupMode::History;
        self.refresh_history_popup();
    }

    /// Show available command-line completions without accepting one.
    pub fn list_completions(&mut self) -> bool {
        self.refresh_command_suggestions();
        self.history_popup_items.clear();
        self.history_popup_index = 0;

        if self.suggestions.is_empty() {
            self.popup_mode = CommandPopupMode::None;
            false
        } else {
            self.popup_mode = CommandPopupMode::Completion;
            true
        }
    }

    /// Complete to the longest common command prefix across current matches.
    pub fn complete_longest_common_prefix(&mut self) -> bool {
        self.refresh_command_suggestions();
        self.history_popup_items.clear();
        self.history_popup_index = 0;

        if self.suggestions.is_empty() {
            self.popup_mode = CommandPopupMode::None;
            return false;
        }

        self.popup_mode = CommandPopupMode::Completion;
        let (prefix, token, suffix) = split_input_segments(&self.input);
        if token.is_empty() {
            return false;
        }

        let matches = command_suggestions_for_token(token, COMMAND_SPECS.len());
        let common_prefix = longest_common_command_prefix(&matches);
        if common_prefix.is_empty() || common_prefix == token {
            return false;
        }

        let prefix = prefix.to_string();
        let suffix = suffix.to_string();
        self.input = format!("{prefix}{common_prefix}{suffix}");
        self.cursor = prefix.chars().count() + common_prefix.chars().count();
        self.refresh_command_suggestions();
        if self.suggestions.is_empty() {
            self.popup_mode = CommandPopupMode::None;
        } else {
            self.popup_mode = CommandPopupMode::Completion;
        }
        true
    }

    /// Insert every matching command completion into the command line.
    pub fn insert_all_matching_completions(&mut self) -> bool {
        self.history_popup_items.clear();
        self.history_popup_index = 0;

        let (prefix, token, suffix) = split_input_segments(&self.input);
        let matches = command_suggestions_for_token(token, COMMAND_SPECS.len());
        if matches.is_empty() {
            self.suggestions.clear();
            self.popup_mode = CommandPopupMode::None;
            return false;
        }

        let inserted = matches
            .iter()
            .map(|suggestion| suggestion.command)
            .collect::<Vec<_>>()
            .join(" ");
        let prefix = prefix.to_string();
        let suffix = suffix.to_string();
        self.input = format!("{prefix}{inserted}{suffix}");
        self.cursor = prefix.chars().count() + inserted.chars().count();
        self.suggestions.clear();
        self.popup_mode = CommandPopupMode::None;
        true
    }

    /// Move selection down in current popup.
    pub fn popup_next(&mut self) {
        match self.popup_mode {
            CommandPopupMode::History => {
                if self.history_popup_items.is_empty() {
                    return;
                }
                self.history_popup_index =
                    (self.history_popup_index + 1) % self.history_popup_items.len();
            }
            CommandPopupMode::Completion => {
                if self.suggestions.is_empty() {
                    return;
                }
                self.suggestion_index = (self.suggestion_index + 1) % self.suggestions.len();
            }
            CommandPopupMode::None => {}
        }
    }

    /// Move selection up in current popup.
    pub fn popup_prev(&mut self) {
        match self.popup_mode {
            CommandPopupMode::History => {
                if self.history_popup_items.is_empty() {
                    return;
                }
                self.history_popup_index = if self.history_popup_index == 0 {
                    self.history_popup_items.len() - 1
                } else {
                    self.history_popup_index - 1
                };
            }
            CommandPopupMode::Completion => {
                if self.suggestions.is_empty() {
                    return;
                }
                self.suggestion_index = if self.suggestion_index == 0 {
                    self.suggestions.len() - 1
                } else {
                    self.suggestion_index - 1
                };
            }
            CommandPopupMode::None => {}
        }
    }

    /// Accept the selected popup item.
    /// Returns true when the command line input changed.
    pub fn accept_popup_selection(&mut self) -> bool {
        match self.popup_mode {
            CommandPopupMode::History => self.accept_history_popup_selection(),
            CommandPopupMode::Completion => self.accept_completion_selection(),
            CommandPopupMode::None => false,
        }
    }

    /// Accept the selected completion suggestion.
    pub fn accept_completion_selection(&mut self) -> bool {
        if self.suggestions.is_empty() {
            self.refresh_command_suggestions();
        }

        let Some(suggestion) = self.suggestions.get(self.suggestion_index).copied() else {
            return false;
        };

        let (prefix, _token, suffix) = split_input_segments(&self.input);
        let prefix = prefix.to_string();
        let suffix = suffix.to_string();

        let mut new_input = String::with_capacity(self.input.len() + suggestion.command.len() + 4);
        new_input.push_str(&prefix);
        new_input.push_str(suggestion.command);
        if suffix.trim().is_empty() && suggestion.takes_args {
            new_input.push(' ');
        } else {
            new_input.push_str(&suffix);
        }

        self.input = new_input;
        self.cursor = self.char_count();
        self.popup_mode = CommandPopupMode::Completion;
        self.refresh_command_suggestions();
        true
    }

    /// Accept the selected history item.
    pub fn accept_history_popup_selection(&mut self) -> bool {
        let Some(selected) = self
            .history_popup_items
            .get(self.history_popup_index)
            .cloned()
        else {
            return false;
        };

        self.input = selected;
        self.cursor = self.char_count();
        self.popup_mode = if self.suggestions.is_empty() {
            CommandPopupMode::None
        } else {
            CommandPopupMode::Completion
        };
        self.history_popup_items.clear();
        self.history_popup_index = 0;
        self.refresh_command_suggestions();
        true
    }

    /// Add current input to history and execute
    pub fn execute(&mut self) -> Command {
        let input = self.input.trim().to_string();

        // Add to history if non-empty, keeping it deduplicated and bounded.
        if !input.is_empty() {
            if let Some(existing_idx) = self.history.iter().position(|s| s == &input) {
                self.history.remove(existing_idx);
            }
            self.history.push(input.clone());
            if self.history.len() > MAX_HISTORY_ENTRIES {
                let extra = self.history.len().saturating_sub(MAX_HISTORY_ENTRIES);
                self.history.drain(0..extra);
            }
            self.save_history();
        }

        let cmd = parse_command(&input);
        self.clear();
        cmd
    }

    /// Get display string (with ':' prefix)
    pub fn display(&self) -> String {
        format!(":{}", display_command_text(&self.input))
    }

    /// Get the display cursor column for the command-line row.
    pub fn display_cursor_col(&self) -> usize {
        let prefix = self
            .input
            .chars()
            .take(self.cursor.min(self.char_count()))
            .collect::<String>();
        1 + display_command_text(&prefix).chars().count()
    }

    fn on_input_edited(&mut self) {
        self.history_index = None;
        self.saved_input = None;
        self.refresh_command_suggestions();
        if self.popup_mode == CommandPopupMode::History {
            self.refresh_history_popup();
        }
    }

    fn refresh_command_suggestions(&mut self) {
        self.suggestions = command_suggestions(&self.input, MAX_COMMAND_SUGGESTIONS);
        if self.suggestions.is_empty() {
            self.suggestion_index = 0;
            if self.popup_mode == CommandPopupMode::Completion {
                self.popup_mode = CommandPopupMode::None;
            }
        } else {
            if self.suggestion_index >= self.suggestions.len() {
                self.suggestion_index = self.suggestions.len() - 1;
            }
            if self.popup_mode != CommandPopupMode::History {
                self.popup_mode = CommandPopupMode::Completion;
            }
        }
    }

    fn refresh_history_popup(&mut self) {
        let query = self.input.trim();
        let query_lower = query.to_lowercase();
        let mut seen = HashSet::new();

        if query_lower.is_empty() {
            self.history_popup_items = self
                .history
                .iter()
                .rev()
                .filter(|entry| !entry.trim().is_empty())
                .filter(|entry| seen.insert((*entry).clone()))
                .take(MAX_HISTORY_ITEMS)
                .cloned()
                .collect();
        } else {
            let mut scored: Vec<(i32, usize, String)> = Vec::new();
            for (recency_idx, entry) in self.history.iter().rev().enumerate() {
                if entry.trim().is_empty() {
                    continue;
                }
                if !seen.insert(entry.clone()) {
                    continue;
                }
                if let Some(score) = alias_match_score(&query_lower, entry) {
                    let recency_bonus = 140i32.saturating_sub((recency_idx as i32) * 4);
                    scored.push((score + recency_bonus, recency_idx, entry.clone()));
                }
            }
            scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
            self.history_popup_items = scored
                .into_iter()
                .take(MAX_HISTORY_ITEMS)
                .map(|(_, _, entry)| entry)
                .collect();
        }

        if self.history_popup_items.is_empty() {
            self.history_popup_index = 0;
        } else if self.history_popup_index >= self.history_popup_items.len() {
            self.history_popup_index = self.history_popup_items.len() - 1;
        }
    }

    fn load_history(&mut self) {
        let path = command_history_path();
        let Ok(contents) = fs::read_to_string(path) else {
            return;
        };

        self.history = contents
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect();

        if self.history.len() > MAX_HISTORY_ENTRIES {
            let extra = self.history.len() - MAX_HISTORY_ENTRIES;
            self.history.drain(0..extra);
        }
    }

    fn save_history(&self) {
        let path = command_history_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        if self.history.is_empty() {
            return;
        }

        let mut contents = self.history.join("\n");
        contents.push('\n');
        let _ = fs::write(path, contents);
    }
}

fn display_command_text(input: &str) -> String {
    input.chars().map(display_command_char).collect()
}

fn display_command_char(ch: char) -> String {
    let code = ch as u32;
    if code == 0x7f {
        "^?".to_string()
    } else if code <= 0x1f {
        let visible = char::from_u32(code + 0x40).unwrap_or('?');
        format!("^{visible}")
    } else {
        ch.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_suggestions_match_aliases() {
        let suggestions = command_suggestions("ff", 5);
        assert!(
            suggestions
                .iter()
                .any(|item| item.command == "FindFiles" || item.matched_alias == "ff"),
            "expected FindFiles to match alias 'ff'"
        );
    }

    #[test]
    fn git_changes_command_suggestions_match_aliases() {
        let suggestions = command_suggestions_for_token("gc", 10);

        assert!(
            suggestions
                .iter()
                .any(|item| item.command == "GitChanges" && item.matched_alias == "gc"),
            "expected GitChanges to match alias 'gc'"
        );

        assert!(matches!(parse_command("GitChanges"), Command::GitChanges));
        assert!(matches!(parse_command("changes"), Command::GitChanges));
        assert!(matches!(parse_command("gc"), Command::GitChanges));
    }

    #[test]
    fn parses_keymaps_command() {
        assert!(matches!(parse_command("Keymaps"), Command::Keymaps));
        assert!(matches!(parse_command("keymaps"), Command::Keymaps));
        assert!(matches!(parse_command("keys"), Command::Keymaps));
    }

    #[test]
    fn buffer_search_command_is_parseable() {
        assert!(matches!(
            parse_command("BufferSearch"),
            Command::BufferSearch
        ));
        assert!(matches!(parse_command("FindLines"), Command::BufferSearch));
        assert!(matches!(parse_command("Lines"), Command::BufferSearch));
        assert!(matches!(parse_command("bl"), Command::BufferSearch));
    }

    #[test]
    fn project_replace_commands_are_parseable_and_listed() {
        match parse_command("ProjectReplace/foo/bar/g") {
            Command::ProjectReplace {
                pattern,
                replacement,
                global,
            } => {
                assert_eq!(pattern, "foo");
                assert_eq!(replacement, "bar");
                assert!(global);
            }
            other => panic!("expected ProjectReplace command, got {other:?}"),
        }

        match parse_command("PReplace#/api/v1#/api/v2#g") {
            Command::ProjectReplace {
                pattern,
                replacement,
                global,
            } => {
                assert_eq!(pattern, "/api/v1");
                assert_eq!(replacement, "/api/v2");
                assert!(global);
            }
            other => panic!("expected ProjectReplace command, got {other:?}"),
        }

        assert!(matches!(
            parse_command("ProjectReplaceApply"),
            Command::ProjectReplaceApply
        ));

        let rows = command_cheatsheet_rows();
        assert!(
            rows.iter().any(|(name, _)| name == ":ProjectReplace"),
            "expected :ProjectReplace in command cheatsheet rows"
        );
        assert!(
            rows.iter().any(|(name, _)| name == ":ProjectReplaceApply"),
            "expected :ProjectReplaceApply in command cheatsheet rows"
        );
    }

    #[test]
    fn checkhealth_command_is_parseable_and_suggested() {
        assert!(matches!(parse_command("checkhealth"), Command::CheckHealth));
        assert!(matches!(parse_command("CheckHealth"), Command::CheckHealth));
        assert!(matches!(parse_command("Health"), Command::CheckHealth));

        let suggestions = command_suggestions("health", 8);
        assert!(
            suggestions.iter().any(|item| item.command == "checkhealth"),
            "expected checkhealth to match health query"
        );

        let rows = command_cheatsheet_rows();
        assert!(
            rows.iter().any(|(name, _)| name == ":checkhealth"),
            "expected :checkhealth in command cheatsheet rows"
        );
    }

    #[test]
    fn tool_install_commands_are_parseable_and_listed() {
        assert!(matches!(parse_command("ToolInstall"), Command::ToolInstall));
        assert!(matches!(parse_command("toolinstall"), Command::ToolInstall));
        assert!(matches!(parse_command("LspInstall"), Command::ToolInstall));
        assert!(matches!(parse_command("lspinstall"), Command::ToolInstall));

        let suggestions = command_suggestions("install", 12);
        assert!(
            suggestions.iter().any(|item| item.command == "ToolInstall"),
            "expected ToolInstall to match install query"
        );

        let rows = command_cheatsheet_rows();
        assert!(
            rows.iter().any(|(name, _)| name == ":ToolInstall"),
            "expected :ToolInstall in command cheatsheet rows"
        );
    }

    #[test]
    fn flight_recorder_commands_are_parseable_and_listed() {
        assert!(matches!(
            parse_command("FlightRecorder"),
            Command::FlightRecorder
        ));
        assert!(matches!(parse_command("flight"), Command::FlightRecorder));
        assert!(matches!(parse_command("WhySlow"), Command::FlightRecorder));
        assert!(matches!(parse_command("whyslow"), Command::FlightRecorder));

        let rows = command_cheatsheet_rows();
        assert!(
            rows.iter().any(|(name, _)| name == ":FlightRecorder"),
            "expected :FlightRecorder in command cheatsheet rows"
        );
    }

    #[test]
    fn jump_command_is_parseable_and_listed() {
        assert_eq!(format!("{:?}", parse_command("Jump")), "Jump");
        assert_eq!(format!("{:?}", parse_command("jump")), "Jump");

        let suggestions = command_suggestions("jump", 8);
        assert!(
            suggestions.iter().any(|item| item.command == "Jump"),
            "expected Jump to match jump query"
        );

        let rows = command_cheatsheet_rows();
        assert!(
            rows.iter().any(|(name, _)| name == ":Jump"),
            "expected :Jump in command cheatsheet rows"
        );
    }

    #[test]
    fn config_commands_are_parseable_and_suggested() {
        assert!(matches!(parse_command("ConfigOpen"), Command::ConfigOpen));
        assert!(matches!(parse_command("configopen"), Command::ConfigOpen));
        assert!(matches!(parse_command("config"), Command::ConfigOpen));
        assert!(matches!(
            parse_command("ConfigDefaults"),
            Command::ConfigDefaults
        ));
        assert!(matches!(
            parse_command("configdefaults"),
            Command::ConfigDefaults
        ));

        let suggestions = command_suggestions("config", 8);
        assert!(
            suggestions.iter().any(|item| item.command == "ConfigOpen"),
            "expected ConfigOpen to match config query"
        );
        assert!(
            suggestions
                .iter()
                .any(|item| item.command == "ConfigDefaults"),
            "expected ConfigDefaults to match config query"
        );

        let rows = command_cheatsheet_rows();
        assert!(
            rows.iter().any(|(name, _)| name == ":ConfigOpen"),
            "expected :ConfigOpen in command cheatsheet rows"
        );
        assert!(
            rows.iter().any(|(name, _)| name == ":ConfigDefaults"),
            "expected :ConfigDefaults in command cheatsheet rows"
        );
    }

    #[test]
    fn parses_buffer_delete_commands() {
        assert!(matches!(parse_command("bd"), Command::BufferDelete(false)));
        assert!(matches!(
            parse_command("bdelete"),
            Command::BufferDelete(false)
        ));
        assert!(matches!(parse_command("bd!"), Command::BufferDelete(true)));
        assert!(matches!(
            parse_command("bdelete!"),
            Command::BufferDelete(true)
        ));
    }

    #[test]
    fn command_cheatsheet_includes_all_registered_commands() {
        let rows = command_cheatsheet_rows();
        assert!(
            rows.iter().any(|(name, _)| name == ":GitChanges"),
            "registry-sourced rows should include :GitChanges"
        );
        assert!(
            rows.iter().any(|(name, _)| name == ":{number}"),
            "parser-only commands like :{{number}} should be documented"
        );
        assert_eq!(
            rows.iter().filter(|(name, _)| name == ":q!").count(),
            1,
            ":q! should appear once (from COMMAND_SPECS, not duplicated)"
        );
        assert!(
            rows.iter().all(|(_, desc)| !desc.is_empty()),
            "every command row should have a description"
        );
    }

    #[test]
    fn command_suggestions_match_fuzzy_queries() {
        let suggestions = command_suggestions("dgflt", 8);
        assert!(
            suggestions
                .iter()
                .any(|item| item.command == "DiagnosticFloat"),
            "expected DiagnosticFloat to match fuzzy query"
        );
    }

    #[test]
    fn markdown_preview_command_is_parseable_and_suggested() {
        assert!(matches!(
            parse_command("MarkdownPreview"),
            Command::MarkdownPreview
        ));
        assert!(matches!(
            parse_command("mdpreview"),
            Command::MarkdownPreview
        ));

        let suggestions = command_suggestions("mdp", 8);
        assert!(
            suggestions
                .iter()
                .any(|item| item.command == "MarkdownPreview"),
            "expected MarkdownPreview to match fuzzy query"
        );
    }

    #[test]
    fn completion_accept_replaces_token_and_keeps_args() {
        let mut line = CommandLine::default();
        line.input = "mk src/components".to_string();
        line.cursor = line.input.chars().count();
        line.refresh_command_suggestions();
        line.suggestion_index = line
            .suggestions
            .iter()
            .position(|item| item.command == "mkdir")
            .unwrap();

        let changed = line.accept_completion_selection();
        assert!(changed);
        assert_eq!(line.input, "mkdir src/components");
    }

    #[test]
    fn terminal_session_commands_parse_arguments() {
        assert!(matches!(
            parse_command("termnew server"),
            Command::TerminalNew(Some(name)) if name == "server"
        ));
        assert!(matches!(
            parse_command("termsel 2"),
            Command::TerminalSelect(2)
        ));
        assert!(matches!(parse_command("termnext"), Command::TerminalNext));
        assert!(matches!(parse_command("termprev"), Command::TerminalPrev));
        assert!(matches!(parse_command("termls"), Command::TerminalList));
        assert!(matches!(parse_command("termmenu"), Command::TerminalPicker));
        assert!(matches!(
            parse_command("termrename server"),
            Command::TerminalRename(None, name) if name == "server"
        ));
        assert!(matches!(
            parse_command("termrename"),
            Command::TerminalRenamePrompt
        ));
        assert!(matches!(
            parse_command("termrename 2 test runner"),
            Command::TerminalRename(Some(2), name) if name == "test runner"
        ));
    }

    #[test]
    fn force_write_commands_parse_arguments() {
        assert!(matches!(parse_command("w!"), Command::ForceWrite(None)));
        assert!(matches!(parse_command("write!"), Command::ForceWrite(None)));
        assert!(matches!(
            parse_command("w! /tmp/nevi-force-save.txt"),
            Command::ForceWrite(Some(path)) if path == PathBuf::from("/tmp/nevi-force-save.txt")
        ));
    }

    #[test]
    fn history_popup_filters_by_query() {
        let mut line = CommandLine::default();
        line.history = vec![
            "FindBuffers".to_string(),
            "Format".to_string(),
            "FindFiles".to_string(),
        ];
        line.input = "ff".to_string();
        line.popup_mode = CommandPopupMode::History;
        line.refresh_history_popup();

        assert!(!line.history_popup_items.is_empty());
        assert!(line
            .history_popup_items
            .iter()
            .any(|item| item == "FindFiles"));
        assert!(line
            .history_popup_items
            .iter()
            .any(|item| item == "FindBuffers"));
        assert!(!line.history_popup_items.iter().any(|item| item == "Format"));
    }
}
