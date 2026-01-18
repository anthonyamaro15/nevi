//! Configuration system for nevi
//!
//! Loads settings from ~/.config/nevi/config.toml

pub mod keymap;

use serde::Deserialize;
use std::path::PathBuf;

pub use keymap::{KeymapLookup, LeaderAction};

/// Main settings structure
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub editor: EditorSettings,
    pub theme: ThemeSettings,
    pub keymap: KeymapSettings,
    pub finder: FinderSettings,
    pub lsp: LspSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            editor: EditorSettings::default(),
            theme: ThemeSettings::default(),
            keymap: KeymapSettings::default(),
            finder: FinderSettings::default(),
            lsp: LspSettings::default(),
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
                    key: "fb".to_string(),
                    action: ":FindBuffers".to_string(),
                    desc: Some("Find buffers".to_string()),
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
}

impl Default for LspServers {
    fn default() -> Self {
        Self {
            rust: LspServerConfig {
                enabled: true,
                command: "rust-analyzer".to_string(),
                args: Vec::new(),
                root_patterns: vec!["Cargo.toml".to_string(), "rust-project.json".to_string()],
                file_extensions: vec!["rs".to_string()],
            },
            typescript: LspServerConfig {
                enabled: true,
                command: "typescript-language-server".to_string(),
                args: vec!["--stdio".to_string()],
                root_patterns: vec!["tsconfig.json".to_string(), "package.json".to_string()],
                file_extensions: vec!["ts".to_string(), "tsx".to_string(), "mts".to_string(), "cts".to_string()],
            },
            javascript: LspServerConfig {
                enabled: true,
                command: "typescript-language-server".to_string(),
                args: vec!["--stdio".to_string()],
                root_patterns: vec!["jsconfig.json".to_string(), "package.json".to_string()],
                file_extensions: vec!["js".to_string(), "jsx".to_string(), "mjs".to_string(), "cjs".to_string()],
            },
            css: LspServerConfig {
                enabled: true,
                command: "vscode-css-language-server".to_string(),
                args: vec!["--stdio".to_string()],
                root_patterns: vec!["package.json".to_string()],
                file_extensions: vec!["css".to_string(), "scss".to_string(), "sass".to_string(), "less".to_string()],
            },
            json: LspServerConfig {
                enabled: true,
                command: "vscode-json-language-server".to_string(),
                args: vec!["--stdio".to_string()],
                root_patterns: vec!["package.json".to_string()],
                file_extensions: vec!["json".to_string(), "jsonc".to_string()],
            },
        }
    }
}

/// Configuration for a single language server
#[derive(Debug, Clone, Deserialize)]
pub struct LspServerConfig {
    /// Enable this language server (default: true)
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Command to run the server
    pub command: String,
    /// Arguments to pass to the server
    #[serde(default)]
    pub args: Vec<String>,
    /// Files that indicate the project root
    #[serde(default)]
    pub root_patterns: Vec<String>,
    /// File extensions this server handles
    #[serde(default)]
    pub file_extensions: Vec<String>,
}

fn default_true() -> bool {
    true
}

/// Get the path to the config file
/// Checks ~/.config/nevi/config.toml first (XDG-style), then falls back to platform default
pub fn config_path() -> Option<PathBuf> {
    // First, try XDG-style path (~/.config/nevi/config.toml)
    if let Some(home) = dirs::home_dir() {
        let xdg_path = home.join(".config/nevi/config.toml");
        if xdg_path.exists() {
            return Some(xdg_path);
        }
    }

    // Fall back to platform-specific config dir
    dirs::config_dir().map(|p| p.join("nevi/config.toml"))
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

# ============================================================================
# THEME
# ============================================================================
# [theme]
# colorscheme = "onedark"    # Color scheme name

# ============================================================================
# FINDER (Fuzzy file picker, grep)
# ============================================================================
# [finder]
# ignore_patterns = [".git", "node_modules", "target", "*.log"]
# max_files = 10000          # Max files to scan
# max_grep_results = 1000    # Max grep results

# ============================================================================
# KEYMAP
# ============================================================================
# All standard vim keybindings work by default (hjkl, w, b, e, d, c, y, etc.)
# Leader key is Space by default.
#
# Default leader mappings (built-in):
#   <leader>w   - Save file
#   <leader>q   - Quit
#   <leader>ff  - Find files
#   <leader>fg  - Live grep
#   <leader>fb  - Find buffers
#   <leader>e   - Toggle file explorer
#   <leader>ca  - Code actions
#   <leader>rn  - Rename symbol
#   <leader>gg  - Open lazygit
#
# To add or override leader mappings:
# [keymap]
# leader = " "  # Space (default)
#
# [[keymap.leader_mappings]]
# key = "w"
# action = ":w"
# desc = "Save file"
#
# To remap keys in normal mode:
# [[keymap.normal]]
# from = "H"
# to = "^"
#
# [[keymap.normal]]
# from = "L"
# to = "$"

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
# To configure a specific server:
# [lsp.servers.rust]
# enabled = true
# command = "rust-analyzer"
# args = []
#
# [lsp.servers.typescript]
# enabled = true
# command = "typescript-language-server"
# args = ["--stdio"]
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
