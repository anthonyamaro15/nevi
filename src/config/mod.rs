//! Configuration system for nevi
//!
//! Loads settings from ~/.config/nevi/config.toml
//! Language-specific settings from ~/.config/nevi/languages.toml

pub mod keymap;
pub mod languages;

use serde::Deserialize;
use std::path::PathBuf;

pub use keymap::{KeymapLookup, LeaderAction};
pub use languages::{load_languages_config, FormatterConfig, LanguageConfig, LanguagesConfig};

/// Main settings structure
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub editor: EditorSettings,
    pub theme: ThemeSettings,
    pub keymap: KeymapSettings,
    pub finder: FinderSettings,
    pub lsp: LspSettings,
    pub copilot: CopilotSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            editor: EditorSettings::default(),
            theme: ThemeSettings::default(),
            keymap: KeymapSettings::default(),
            finder: FinderSettings::default(),
            lsp: LspSettings::default(),
            copilot: CopilotSettings::default(),
        }
    }
}

/// Autosave mode configuration
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AutosaveMode {
    /// Autosave disabled
    Off,
    /// Save after delay milliseconds of no edits
    AfterDelay,
    /// Save when editor loses focus (not yet implemented for terminal)
    OnFocusChange,
}

impl Default for AutosaveMode {
    fn default() -> Self {
        AutosaveMode::Off
    }
}

/// Editor behavior settings
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct EditorSettings {
    /// Number of spaces per tab (default: 4)
    pub tab_width: usize,
    /// Show line numbers (default: true)
    pub line_numbers: bool,
    /// Show relative line numbers (default: false)
    pub relative_numbers: bool,
    /// Highlight current line (default: false)
    pub cursor_line: bool,
    /// Lines to keep visible above/below cursor (default: 0)
    pub scroll_off: usize,
    /// Enable smart auto-indentation (default: true)
    pub auto_indent: bool,
    /// Enable soft word wrap (default: false)
    pub wrap: bool,
    /// Column to wrap at (default: 80)
    pub wrap_width: usize,
    /// Enable auto-pairs (auto-close brackets/quotes) (default: true)
    pub auto_pairs: bool,
    /// Format document on save using LSP (default: false)
    pub format_on_save: bool,
    /// Autosave mode (default: off)
    pub autosave: AutosaveMode,
    /// Autosave delay in milliseconds (default: 1000)
    pub autosave_delay_ms: u64,
    /// Use Nerd Font icons in explorer (default: true)
    /// Set to false to use Unicode fallback icons
    pub use_nerd_font_icons: bool,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            tab_width: 4,
            line_numbers: true,
            relative_numbers: false,
            cursor_line: false,
            scroll_off: 8, // Neovim-like default
            auto_indent: true,
            wrap: false,
            wrap_width: 80,
            auto_pairs: true,
            format_on_save: false,
            autosave: AutosaveMode::Off,
            autosave_delay_ms: 1000,
            use_nerd_font_icons: true,
        }
    }
}

/// Theme settings
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ThemeSettings {
    /// Color scheme name (default: "onedark")
    pub colorscheme: String,
}

impl Default for ThemeSettings {
    fn default() -> Self {
        Self {
            colorscheme: "onedark".to_string(),
        }
    }
}

/// Fuzzy finder settings
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct FinderSettings {
    /// Additional ignore patterns (beyond .gitignore)
    pub ignore_patterns: Vec<String>,
    /// Maximum files to scan (default: 10000)
    pub max_files: usize,
    /// Maximum grep results (default: 1000)
    pub max_grep_results: usize,
}

impl Default for FinderSettings {
    fn default() -> Self {
        Self {
            ignore_patterns: vec![
                ".git".to_string(),
                "node_modules".to_string(),
                "target".to_string(),
                "*.log".to_string(),
            ],
            max_files: 10000,
            max_grep_results: 1000,
        }
    }
}

/// Keymap customization settings
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct KeymapSettings {
    /// Leader key (default: "\")
    pub leader: String,
    /// Timeout in milliseconds for leader key sequences (default: 1000)
    /// When a sequence matches both an exact mapping AND is a prefix of longer mappings,
    /// wait this long before executing the shorter match.
    pub timeoutlen: u64,
    /// Normal mode key remappings
    pub normal: Vec<KeymapEntry>,
    /// Insert mode key remappings
    pub insert: Vec<KeymapEntry>,
    /// Leader key mappings (e.g., <leader>w -> :w<CR>)
    pub leader_mappings: Vec<LeaderMapping>,
}

impl Default for KeymapSettings {
    fn default() -> Self {
        Self {
            leader: " ".to_string(), // Space as leader (common in Neovim)
            timeoutlen: 1000, // 1 second (matches Vim default)
            normal: Vec::new(),
            insert: Vec::new(),
            leader_mappings: vec![
                // LSP actions
                LeaderMapping {
                    key: "ca".to_string(),
                    action: ":codeaction".to_string(),
                    desc: Some("Code actions".to_string()),
                },
                LeaderMapping {
                    key: "rn".to_string(),
                    action: ":rn".to_string(),
                    desc: Some("Rename symbol".to_string()),
                },
                // File operations
                LeaderMapping {
                    key: "w".to_string(),
                    action: ":w".to_string(),
                    desc: Some("Save file".to_string()),
                },
                LeaderMapping {
                    key: "q".to_string(),
                    action: ":q".to_string(),
                    desc: Some("Quit".to_string()),
                },
                // Finder
                LeaderMapping {
                    key: "ff".to_string(),
                    action: ":FindFiles".to_string(),
                    desc: Some("Find files".to_string()),
                },
                LeaderMapping {
                    key: "fg".to_string(),
                    action: ":LiveGrep".to_string(),
                    desc: Some("Live grep".to_string()),
                },
                LeaderMapping {
                    key: "sw".to_string(),
                    action: ":SearchWord".to_string(),
                    desc: Some("Search word under cursor".to_string()),
                },
                LeaderMapping {
                    key: "fb".to_string(),
                    action: ":FindBuffers".to_string(),
                    desc: Some("Find buffers".to_string()),
                },
                LeaderMapping {
                    key: "d".to_string(),
                    action: ":FindDiagnostics".to_string(),
                    desc: Some("Find diagnostics".to_string()),
                },
                LeaderMapping {
                    key: "D".to_string(),
                    action: ":DiagnosticFloat".to_string(),
                    desc: Some("Show line diagnostic".to_string()),
                },
                // Explorer
                LeaderMapping {
                    key: "e".to_string(),
                    action: ":Explorer".to_string(),
                    desc: Some("Toggle explorer".to_string()),
                },
                // Git
                LeaderMapping {
                    key: "gg".to_string(),
                    action: ":LazyGit".to_string(),
                    desc: Some("Open lazygit".to_string()),
                },
                // Harpoon
                LeaderMapping {
                    key: "m".to_string(),
                    action: ":HarpoonAdd".to_string(),
                    desc: Some("Add to harpoon".to_string()),
                },
                LeaderMapping {
                    key: "h".to_string(),
                    action: ":HarpoonMenu".to_string(),
                    desc: Some("Harpoon menu".to_string()),
                },
                LeaderMapping {
                    key: "1".to_string(),
                    action: ":Harpoon1".to_string(),
                    desc: Some("Harpoon file 1".to_string()),
                },
                LeaderMapping {
                    key: "2".to_string(),
                    action: ":Harpoon2".to_string(),
                    desc: Some("Harpoon file 2".to_string()),
                },
                LeaderMapping {
                    key: "3".to_string(),
                    action: ":Harpoon3".to_string(),
                    desc: Some("Harpoon file 3".to_string()),
                },
                LeaderMapping {
                    key: "4".to_string(),
                    action: ":Harpoon4".to_string(),
                    desc: Some("Harpoon file 4".to_string()),
                },
                // Theme
                LeaderMapping {
                    key: "ft".to_string(),
                    action: ":Themes".to_string(),
                    desc: Some("Open theme picker".to_string()),
                },
                // Terminal - disabled due to rendering flicker issues
                // See NEOVIM_PARITY.md "On Hold" section for details
                // LeaderMapping {
                //     key: "t".to_string(),
                //     action: ":Terminal".to_string(),
                //     desc: Some("Toggle terminal".to_string()),
                // },
            ],
        }
    }
}

/// A single keymap entry
#[derive(Debug, Clone, Deserialize)]
pub struct KeymapEntry {
    /// Key notation to remap from (e.g., "H", "<C-s>", ";")
    pub from: String,
    /// Key notation to remap to (e.g., "^", ":w<CR>", ":")
    pub to: String,
}

/// A leader key mapping
#[derive(Debug, Clone, Deserialize)]
pub struct LeaderMapping {
    /// Key sequence after leader (e.g., "w", "wa", "q")
    pub key: String,
    /// Action to execute (e.g., ":w<CR>", ":wa<CR>", ":q<CR>")
    pub action: String,
    /// Optional description for which-key style display
    #[serde(default)]
    pub desc: Option<String>,
}

/// LSP (Language Server Protocol) settings
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LspSettings {
    /// Enable LSP support (default: true)
    pub enabled: bool,
    /// Delay before showing hover (milliseconds)
    pub hover_delay_ms: u64,
    /// Language server configurations
    pub servers: LspServers,
}

impl Default for LspSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            hover_delay_ms: 500,
            servers: LspServers::default(),
        }
    }
}

/// Per-language server configurations
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LspServers {
    pub rust: LspServerConfig,
    pub typescript: LspServerConfig,
    pub javascript: LspServerConfig,
    pub css: LspServerConfig,
    pub json: LspServerConfig,
    pub toml: LspServerConfig,
    pub markdown: LspServerConfig,
    pub html: LspServerConfig,
    pub python: LspServerConfig,
}

impl Default for LspServers {
    fn default() -> Self {
        Self {
            rust: LspServerConfig {
                enabled: true,
                preset: None,
                command: "rust-analyzer".to_string(),
                args: Vec::new(),
                root_patterns: vec!["Cargo.toml".to_string(), "rust-project.json".to_string()],
                file_extensions: vec!["rs".to_string()],
            },
            typescript: LspServerConfig {
                enabled: true,
                preset: None,
                command: "typescript-language-server".to_string(),
                args: vec!["--stdio".to_string()],
                root_patterns: vec!["tsconfig.json".to_string(), "package.json".to_string()],
                file_extensions: vec!["ts".to_string(), "tsx".to_string(), "mts".to_string(), "cts".to_string()],
            },
            javascript: LspServerConfig {
                enabled: true,
                preset: None,
                command: "typescript-language-server".to_string(),
                args: vec!["--stdio".to_string()],
                root_patterns: vec!["jsconfig.json".to_string(), "package.json".to_string()],
                file_extensions: vec!["js".to_string(), "jsx".to_string(), "mjs".to_string(), "cjs".to_string()],
            },
            css: LspServerConfig {
                enabled: true,
                preset: None,
                command: "vscode-css-language-server".to_string(),
                args: vec!["--stdio".to_string()],
                root_patterns: vec!["package.json".to_string()],
                file_extensions: vec!["css".to_string(), "scss".to_string(), "sass".to_string(), "less".to_string()],
            },
            json: LspServerConfig {
                enabled: true,
                preset: None,
                command: "vscode-json-language-server".to_string(),
                args: vec!["--stdio".to_string()],
                root_patterns: vec!["package.json".to_string()],
                file_extensions: vec!["json".to_string(), "jsonc".to_string()],
            },
            toml: LspServerConfig {
                enabled: true,
                preset: None,
                command: "taplo".to_string(),
                args: vec!["lsp".to_string(), "stdio".to_string()],
                root_patterns: vec!["Cargo.toml".to_string(), "pyproject.toml".to_string()],
                file_extensions: vec!["toml".to_string()],
            },
            markdown: LspServerConfig {
                enabled: false, // Disabled by default - marksman has limited LSP support
                preset: None,
                command: "marksman".to_string(),
                args: vec!["server".to_string()],
                root_patterns: vec![".marksman.toml".to_string()],
                file_extensions: vec!["md".to_string(), "markdown".to_string()],
            },
            html: LspServerConfig {
                enabled: true,
                preset: None,
                command: "vscode-html-language-server".to_string(),
                args: vec!["--stdio".to_string()],
                root_patterns: vec!["package.json".to_string()],
                file_extensions: vec!["html".to_string(), "htm".to_string()],
            },
            python: LspServerConfig {
                enabled: true,
                preset: None,
                command: "pyright-langserver".to_string(),
                args: vec!["--stdio".to_string()],
                root_patterns: vec![
                    "pyproject.toml".to_string(),
                    "setup.py".to_string(),
                    "setup.cfg".to_string(),
                    "requirements.txt".to_string(),
                    "pyrightconfig.json".to_string(),
                ],
                file_extensions: vec!["py".to_string(), "pyi".to_string()],
            },
        }
    }
}

/// Built-in LSP server presets
/// These provide convenient shortcuts for common language servers
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LspPreset {
    /// typescript-language-server --stdio (default for TS/JS)
    Typescript,
    /// biome lsp-proxy (fast Rust-native alternative)
    Biome,
    /// deno lsp (for Deno projects)
    Deno,
    /// vscode-eslint-language-server --stdio
    Eslint,
    /// rust-analyzer (default for Rust)
    RustAnalyzer,
    /// pyright-langserver --stdio (default for Python)
    Pyright,
    /// pylsp (alternative Python LSP)
    Pylsp,
    /// Custom - use explicit command/args
    Custom,
}

impl LspPreset {
    /// Get the command and args for this preset
    pub fn resolve(&self) -> Option<(String, Vec<String>)> {
        match self {
            LspPreset::Typescript => Some((
                "typescript-language-server".to_string(),
                vec!["--stdio".to_string()],
            )),
            LspPreset::Biome => Some((
                "biome".to_string(),
                vec!["lsp-proxy".to_string()],
            )),
            LspPreset::Deno => Some((
                "deno".to_string(),
                vec!["lsp".to_string()],
            )),
            LspPreset::Eslint => Some((
                "vscode-eslint-language-server".to_string(),
                vec!["--stdio".to_string()],
            )),
            LspPreset::RustAnalyzer => Some((
                "rust-analyzer".to_string(),
                Vec::new(),
            )),
            LspPreset::Pyright => Some((
                "pyright-langserver".to_string(),
                vec!["--stdio".to_string()],
            )),
            LspPreset::Pylsp => Some((
                "pylsp".to_string(),
                Vec::new(),
            )),
            LspPreset::Custom => None, // Use explicit command/args
        }
    }
}

/// Configuration for a single language server
#[derive(Debug, Clone, Deserialize)]
pub struct LspServerConfig {
    /// Enable this language server (default: true)
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Preset to use (optional - provides command and args automatically)
    /// If set to a known preset, command/args are auto-configured
    /// Use "custom" with explicit command/args for custom servers
    #[serde(default)]
    pub preset: Option<LspPreset>,
    /// Command to run the server (overrides preset if set)
    #[serde(default)]
    pub command: String,
    /// Arguments to pass to the server (overrides preset if set)
    #[serde(default)]
    pub args: Vec<String>,
    /// Files that indicate the project root
    #[serde(default)]
    pub root_patterns: Vec<String>,
    /// File extensions this server handles
    #[serde(default)]
    pub file_extensions: Vec<String>,
}

impl LspServerConfig {
    /// Get the effective command for this server
    /// Resolves presets if set, otherwise uses explicit command
    pub fn effective_command(&self) -> &str {
        // Explicit command takes priority
        if !self.command.is_empty() {
            return &self.command;
        }
        // Fall back to preset default (return static strings for known presets)
        if let Some(preset) = &self.preset {
            return match preset {
                LspPreset::Typescript => "typescript-language-server",
                LspPreset::Biome => "biome",
                LspPreset::Deno => "deno",
                LspPreset::Eslint => "vscode-eslint-language-server",
                LspPreset::RustAnalyzer => "rust-analyzer",
                LspPreset::Pyright => "pyright-langserver",
                LspPreset::Pylsp => "pylsp",
                LspPreset::Custom => &self.command,
            };
        }
        &self.command
    }

    /// Get the effective args for this server
    /// Resolves presets if set, otherwise uses explicit args
    pub fn effective_args(&self) -> Vec<String> {
        // Explicit args take priority (if command is also set)
        if !self.command.is_empty() {
            return self.args.clone();
        }
        // Fall back to preset defaults
        if let Some(preset) = &self.preset {
            if let Some((_, args)) = preset.resolve() {
                return args;
            }
        }
        self.args.clone()
    }
}

fn default_true() -> bool {
    true
}

/// GitHub Copilot settings
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CopilotSettings {
    /// Enable Copilot (default: true)
    pub enabled: bool,
    /// Path to the Copilot language server (auto-detected if empty)
    pub server_path: String,
    /// Path to Node.js executable (auto-detected if empty)
    pub node_path: String,
    /// Debounce delay in milliseconds before requesting completions (default: 150)
    pub debounce_ms: u64,
    /// Automatically trigger completions in insert mode (default: true)
    pub auto_trigger: bool,
    /// Hide ghost text when LSP completion popup is visible (default: true)
    pub hide_during_completion: bool,
    /// Languages where Copilot is disabled (default: empty)
    pub disabled_languages: Vec<String>,
}

impl Default for CopilotSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            server_path: String::new(),
            node_path: String::new(),
            debounce_ms: 150,
            auto_trigger: true,
            hide_during_completion: true,
            disabled_languages: Vec::new(),
        }
    }
}

/// Get the path to the config file
/// Always uses ~/.config/nevi/config.toml (XDG-style) for consistency across platforms
pub fn config_path() -> Option<PathBuf> {
    // Always prefer XDG-style path (~/.config/nevi/config.toml)
    // This is more discoverable and consistent with other CLI tools
    dirs::home_dir().map(|home| home.join(".config/nevi/config.toml"))
}

/// Template config file with comments explaining all options
/// This is generated when no config file exists
fn default_config_template() -> &'static str {
    r#"# Nevi Configuration
# This file is for overriding default settings.
# All vim/neovim keybindings work out of the box - you don't need to configure them here.
# Only add settings you want to change from the defaults.

# ============================================================================
# EDITOR SETTINGS
# ============================================================================
# [editor]
# tab_width = 4              # Spaces per tab
# line_numbers = true        # Show line numbers
# relative_numbers = false   # Show relative line numbers
# cursor_line = false        # Highlight current line
# scroll_off = 8             # Lines to keep visible above/below cursor
# auto_indent = true         # Smart indentation on new lines
# wrap = false               # Soft word wrap
# wrap_width = 80            # Column to wrap at
# auto_pairs = true          # Auto-close brackets and quotes
# format_on_save = false     # Format with LSP on save
# autosave = "off"           # Options: "off", "after_delay", "on_focus_change"
# autosave_delay_ms = 1000   # Delay for after_delay mode
# use_nerd_font_icons = true # Use Nerd Font icons in explorer (set false for Unicode fallback)

# ============================================================================
# THEME
# ============================================================================
# [theme]
# colorscheme = "onedark"    # Color scheme name

# ============================================================================
# FINDER (Fuzzy file picker, grep)
# ============================================================================
# [finder]
# max_files = 10000          # Max files to scan
# max_grep_results = 1000    # Max grep results
#
# Finder keybinds (when finder is open):
#   Ctrl+t or p    - Toggle preview panel (50/50 split)
#   Ctrl+j/k       - Navigate results (insert mode)
#   j/k            - Navigate results (normal mode)
#   Esc            - Switch to normal mode / close finder
#   Enter          - Open selected file
#   Ctrl+d/u       - Scroll preview down/up
#
# Add extra ignore patterns (these are ADDED to defaults, not replacing):
# ignore_patterns = ["my-folder", "*.generated.ts"]
#
# Default patterns already ignored:
#   Version control:  .git, .svn, .hg
#   Dependencies:     node_modules, vendor
#   Build outputs:    target, build, dist, out, .next, .nuxt, .output, *-build
#   Cache:            .cache, __pycache__, .pytest_cache, .mypy_cache
#   IDE/Editor:       .idea, .vscode
#   Coverage:         coverage, .nyc_output
#   File patterns:    *.log, *.tmp, *.bak
#
# Pattern syntax:
#   "build"     - exact match
#   "*.log"     - ends with .log
#   "*-build"   - ends with -build (matches aws-build, my-build, etc.)
#   "tmp*"      - starts with tmp
#   "*test*"    - contains test

# ============================================================================
# KEYMAP CUSTOMIZATION
# ============================================================================
# All standard vim keybindings work by default.
# Below is a comprehensive reference of implemented keybinds.
# To override any keybind, uncomment and modify the examples at the end.
#
# Leader key is Space by default.
# [keymap]
# leader = " "
# timeoutlen = 1000          # Timeout in ms for leader key sequences (default: 1000)
#                            # When a sequence matches both an exact mapping AND is a prefix
#                            # of longer mappings (e.g., <leader>e and <leader>ee), wait this
#                            # long before executing the shorter match. Set to 0 for immediate
#                            # execution (disables overlapping mapping support).
#
# ----------------------------------------------------------------------------
# NORMAL MODE - Movement
# ----------------------------------------------------------------------------
# h/j/k/l          - Move cursor left/down/up/right
# w/W              - Move to start of next word/WORD
# b/B              - Move to start of previous word/WORD
# e/E              - Move to end of word/WORD
# ge/gE            - Move to end of previous word/WORD
# 0                - Move to start of line
# ^                - Move to first non-blank character
# $                - Move to end of line
# gg               - Move to start of file
# G                - Move to end of file (or line N with count)
# {/}              - Move to previous/next paragraph
# %                - Jump to matching bracket
# H/M/L            - Move to top/middle/bottom of screen
# f{char}/F{char}  - Find character forward/backward
# t{char}/T{char}  - Move till character forward/backward
# ;/,              - Repeat last f/F/t/T / in reverse
#
# ----------------------------------------------------------------------------
# NORMAL MODE - Scrolling
# ----------------------------------------------------------------------------
# Ctrl+f/Ctrl+b    - Page down/up
# Ctrl+d/Ctrl+u    - Half page down/up
# zz/zt/zb         - Center cursor / cursor to top / cursor to bottom
#
# ----------------------------------------------------------------------------
# NORMAL MODE - Jump List
# ----------------------------------------------------------------------------
# Ctrl+o           - Jump to older position in jump list
# Ctrl+i           - Jump to newer position in jump list
#
# ----------------------------------------------------------------------------
# NORMAL MODE - Change List
# ----------------------------------------------------------------------------
# g;               - Jump to older change position (where you edited)
# g,               - Jump to newer change position
#
# ----------------------------------------------------------------------------
# NORMAL MODE - Editing
# ----------------------------------------------------------------------------
# d{motion}/dd/D   - Delete with motion / line / to end
# c{motion}/cc/C   - Change with motion / line / to end
# y{motion}/yy/Y   - Yank with motion / line / line
# p/P              - Paste after/before cursor
# x/X              - Delete char under/before cursor
# r{char}          - Replace character
# J                - Join lines with space
# gJ               - Join lines without space
# .                - Repeat last change
# u/Ctrl+r         - Undo/redo
#
# ----------------------------------------------------------------------------
# NORMAL MODE - Indent
# ----------------------------------------------------------------------------
# >>               - Indent current line
# <<               - Dedent current line
# >{motion}        - Indent with motion
# <{motion}        - Dedent with motion
#
# ----------------------------------------------------------------------------
# NORMAL MODE - Case
# ----------------------------------------------------------------------------
# ~                - Swap case of character
# gu{motion}/guu   - Lowercase with motion / line
# gU{motion}/gUU   - Uppercase with motion / line
# g~{motion}/g~~   - Toggle case with motion / line
#
# ----------------------------------------------------------------------------
# NORMAL MODE - Marks
# ----------------------------------------------------------------------------
# m{a-zA-Z}        - Set mark (a-z local, A-Z global)
# '{a-zA-Z}        - Jump to line of mark
# `{a-zA-Z}        - Jump to exact position of mark
# :marks           - Show all marks in interactive picker
# :delmarks {m}    - Delete marks (e.g., :delmarks a, :delmarks a-d)
# :delmarks!       - Delete all lowercase marks in buffer
#
# ----------------------------------------------------------------------------
# NORMAL MODE - Macros
# ----------------------------------------------------------------------------
# q{a-z}           - Start recording macro into register
# q                - Stop recording (when recording)
# @{a-z}           - Play macro from register
# @@               - Replay last executed macro
#
# ----------------------------------------------------------------------------
# NORMAL MODE - Search
# ----------------------------------------------------------------------------
# /                - Search forward
# ?                - Search backward
# n/N              - Next/previous search match
# *                - Search word under cursor forward
# #                - Search word under cursor backward
#
# ----------------------------------------------------------------------------
# NORMAL MODE - LSP
# ----------------------------------------------------------------------------
# gd               - Go to definition
# gr               - Find references
# K                - Show hover documentation
# gl               - Show diagnostic in float
# ]d/[d            - Next/previous diagnostic
#
# ----------------------------------------------------------------------------
# NORMAL MODE - Surround (vim-surround style)
# ----------------------------------------------------------------------------
# ds{char}         - Delete surrounding pair
# cs{old}{new}     - Change surrounding pair
# ys{motion}{char} - Add surrounding pair
#
# ----------------------------------------------------------------------------
# NORMAL MODE - Comment
# ----------------------------------------------------------------------------
# gcc              - Toggle comment on line
# gc{motion}       - Toggle comment with motion
#
# ----------------------------------------------------------------------------
# NORMAL MODE - gv (Visual Reselect)
# ----------------------------------------------------------------------------
# gv               - Reselect last visual selection
#
# ----------------------------------------------------------------------------
# NORMAL MODE - Mode Switching
# ----------------------------------------------------------------------------
# i/a              - Insert before/after cursor
# I/A              - Insert at line start/end
# o/O              - Open line below/above
# gi               - Go to last insert position and enter insert mode
# v/V/Ctrl+v       - Visual / Visual line / Visual block
# R                - Replace mode
# :                - Command mode
#
# ----------------------------------------------------------------------------
# NORMAL MODE - Window Management
# ----------------------------------------------------------------------------
# Ctrl+w v         - Split vertical
# Ctrl+w s         - Split horizontal
# Ctrl+w c         - Close window
# Ctrl+w o         - Close other windows
# Ctrl+w w/p       - Next/previous window
# Ctrl+w h/j/k/l   - Move to window left/down/up/right
#
# ----------------------------------------------------------------------------
# NORMAL MODE - Harpoon
# ----------------------------------------------------------------------------
# ]h/[h            - Next/previous harpoon file
#
# ----------------------------------------------------------------------------
# INSERT MODE
# ----------------------------------------------------------------------------
# Esc/Ctrl+[       - Exit insert mode
# Backspace        - Delete character before
# Enter            - Insert new line
# Tab              - Insert tab/spaces
# Ctrl+w           - Delete word before cursor
# Ctrl+u           - Delete to start of line
# Ctrl+l           - Accept Copilot suggestion
# Alt+]/Alt+[      - Next/previous Copilot suggestion
#
# ----------------------------------------------------------------------------
# VISUAL MODE
# ----------------------------------------------------------------------------
# Esc              - Exit visual mode
# d/c/y            - Delete/change/yank selection
# p                - Paste over selection
# o                - Swap selection end
# >/<              - Indent/dedent selection
# gc               - Toggle comment
# S{char}          - Surround selection
#
# ----------------------------------------------------------------------------
# TEXT OBJECTS (use with d, c, y, etc.)
# ----------------------------------------------------------------------------
# iw/aw            - Inner/around word
# iW/aW            - Inner/around WORD
# i"/a"            - Inner/around double quotes
# i'/a'            - Inner/around single quotes
# i`/a`            - Inner/around backticks
# i(/a( or ib/ab   - Inner/around parentheses
# i{/a{ or iB/aB   - Inner/around braces
# i[/a[            - Inner/around brackets
# i</a<            - Inner/around angle brackets
#
# ----------------------------------------------------------------------------
# REGISTERS (prefix with ")
# ----------------------------------------------------------------------------
# "{a-z}           - Use named register
# "{A-Z}           - Append to named register
# "+               - System clipboard
# "*               - Selection clipboard
# "_               - Black hole (discard)
# "0               - Last yank
#
# ----------------------------------------------------------------------------
# LEADER MAPPINGS (default: Space)
# ----------------------------------------------------------------------------
# <leader>w        - Save file
# <leader>q        - Quit
# <leader>e        - Toggle file explorer
# <leader>ff       - Find files
# <leader>fg       - Live grep
# <leader>sw       - Search word under cursor (grep)
# <leader>fb       - Find buffers
# <leader>ft       - Theme picker
# <leader>ca       - Code actions
# <leader>rn       - Rename symbol
# <leader>d        - Search diagnostics
# <leader>D        - Show line diagnostic
# <leader>gg       - Open lazygit
# <leader>m        - Add to harpoon
# <leader>h        - Harpoon menu
# <leader>1-4      - Jump to harpoon slot 1-4
#
# ============================================================================
# CUSTOM KEYBIND EXAMPLES
# ============================================================================
# To remap keys in normal mode:
# [[keymap.normal]]
# from = "H"
# to = "^"
#
# [[keymap.normal]]
# from = "L"
# to = "$"
#
# To add/override leader mappings:
# [[keymap.leader_mappings]]
# key = "w"
# action = ":w"
# desc = "Save file"

# ============================================================================
# LSP (Language Server Protocol)
# ============================================================================
# LSP servers are auto-detected and enabled by default.
# Supported: rust-analyzer, typescript-language-server, vscode-css-language-server, vscode-json-language-server
#
# To disable LSP entirely:
# [lsp]
# enabled = false
#
# ----------------------------------------------------------------------------
# LSP Presets
# ----------------------------------------------------------------------------
# Use presets for quick configuration of alternative LSP servers.
# Available presets: typescript, biome, deno, eslint, rust_analyzer, pyright, pylsp
#
# Example: Use Biome LSP instead of typescript-language-server for faster linting
# [lsp.servers.typescript]
# preset = "biome"
#
# Example: Use Deno LSP for Deno projects
# [lsp.servers.typescript]
# preset = "deno"
#
# ----------------------------------------------------------------------------
# Custom LSP Configuration
# ----------------------------------------------------------------------------
# For full control, specify command and args directly (overrides preset):
# [lsp.servers.rust]
# enabled = true
# command = "rust-analyzer"
# args = []
#
# [lsp.servers.typescript]
# enabled = true
# command = "typescript-language-server"
# args = ["--stdio"]
#
# You can also combine preset with custom root_patterns:
# [lsp.servers.typescript]
# preset = "biome"
# root_patterns = ["biome.json", "package.json"]

# ============================================================================
# COPILOT
# ============================================================================
# [copilot]
# enabled = true             # Enable GitHub Copilot
# debounce_ms = 150          # Delay before requesting completions
# auto_trigger = true        # Auto-trigger in insert mode
# hide_during_completion = true  # Hide when LSP popup visible
# disabled_languages = []    # Languages where Copilot is disabled
"#
}

/// Ensure config directory and template file exist
fn ensure_config_exists() {
    let Some(path) = config_path() else {
        return;
    };

    // Create config directory if it doesn't exist
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            let _ = std::fs::create_dir_all(parent);
        }
    }

    // Create template config if it doesn't exist
    if !path.exists() {
        let _ = std::fs::write(&path, default_config_template());
    }
}

/// Load settings from the config file
/// Returns default settings if the file doesn't exist or can't be parsed
/// User settings are merged with defaults - user values take precedence,
/// but default leader mappings are preserved unless explicitly overridden.
pub fn load_config() -> Settings {
    // Ensure config file exists (creates template if not)
    ensure_config_exists();

    let Some(path) = config_path() else {
        return Settings::default();
    };

    if !path.exists() {
        return Settings::default();
    }

    match std::fs::read_to_string(&path) {
        Ok(content) => match toml::from_str::<Settings>(&content) {
            Ok(mut user_settings) => {
                // Merge leader mappings: defaults + user overrides
                user_settings.keymap.leader_mappings =
                    merge_leader_mappings(&user_settings.keymap.leader_mappings);
                user_settings
            }
            Err(e) => {
                eprintln!("Warning: Failed to parse config file: {}", e);
                Settings::default()
            }
        },
        Err(e) => {
            eprintln!("Warning: Failed to read config file: {}", e);
            Settings::default()
        }
    }
}

/// Merge user leader mappings with defaults.
/// User mappings take precedence for the same key.
fn merge_leader_mappings(user_mappings: &[LeaderMapping]) -> Vec<LeaderMapping> {
    let defaults = KeymapSettings::default().leader_mappings;

    // Collect user-defined keys for quick lookup
    let user_keys: std::collections::HashSet<&str> =
        user_mappings.iter().map(|m| m.key.as_str()).collect();

    // Start with defaults that aren't overridden by user
    let mut merged: Vec<LeaderMapping> = defaults
        .into_iter()
        .filter(|m| !user_keys.contains(m.key.as_str()))
        .collect();

    // Add all user mappings
    merged.extend(user_mappings.iter().cloned());

    merged
}

/// Save the theme setting to config.toml
/// Uses toml_edit to preserve formatting and comments
pub fn save_theme(theme_name: &str) -> Result<(), String> {
    let Some(path) = config_path() else {
        return Err("Could not determine config path".to_string());
    };

    // Ensure config exists
    ensure_config_exists();

    // Read existing config
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read config: {}", e))?;

    // Parse as editable TOML document
    let mut doc = content.parse::<toml_edit::DocumentMut>()
        .map_err(|e| format!("Failed to parse config: {}", e))?;

    // Ensure [theme] table exists
    if doc.get("theme").is_none() {
        doc["theme"] = toml_edit::Item::Table(toml_edit::Table::new());
    }

    // Set colorscheme
    doc["theme"]["colorscheme"] = toml_edit::value(theme_name);

    // Write back
    std::fs::write(&path, doc.to_string())
        .map_err(|e| format!("Failed to write config: {}", e))?;

    Ok(())
}
