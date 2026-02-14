//! Language-specific configuration
//!
//! Loads settings from ~/.config/nevi/languages.toml

use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

/// Configuration for a single language
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct LanguageConfig {
    /// External formatter (optional - falls back to LSP if not set)
    pub formatter: Option<FormatterConfig>,
    /// Tab width override for this language
    pub tab_width: Option<usize>,
}

/// External formatter configuration
#[derive(Debug, Clone, Deserialize)]
pub struct FormatterConfig {
    /// Command to run (e.g., "biome", "prettier", "black")
    pub command: String,
    /// Arguments to pass (use {file} as placeholder for file path)
    pub args: Vec<String>,
    /// Timeout in seconds (default: 5)
    #[serde(default = "default_timeout")]
    pub timeout: u64,
}

fn default_timeout() -> u64 {
    5
}

/// All language configurations loaded from languages.toml
#[derive(Debug, Clone, Deserialize, Default)]
pub struct LanguagesConfig {
    #[serde(flatten)]
    pub languages: HashMap<String, LanguageConfig>,
}

impl LanguagesConfig {
    /// Get formatter config for a language by name
    pub fn get_formatter(&self, language: &str) -> Option<&FormatterConfig> {
        self.languages
            .get(language)
            .and_then(|config| config.formatter.as_ref())
    }

    /// Get tab width override for a language
    pub fn get_tab_width(&self, language: &str) -> Option<usize> {
        self.languages
            .get(language)
            .and_then(|config| config.tab_width)
    }
}

/// Get the path to languages.toml
pub fn languages_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".config/nevi/languages.toml"))
}

/// Default template for languages.toml with examples
fn default_languages_template() -> &'static str {
    r#"# Nevi Language Configuration
# Configure per-language settings like formatters and tab width.
# If no formatter is specified, nevi falls back to LSP formatting.
#
# Placeholder: {file} - replaced with the full file path
#
# Example configurations are commented out below.
# Uncomment and modify to enable.

# ============================================================================
# TYPESCRIPT / JAVASCRIPT (Biome - fast Rust-native formatter)
# ============================================================================
# [typescript]
# formatter = { command = "biome", args = ["format", "--stdin-file-path", "{file}"] }
# tab_width = 2
#
# [javascript]
# formatter = { command = "biome", args = ["format", "--stdin-file-path", "{file}"] }
# tab_width = 2
#
# [tsx]
# formatter = { command = "biome", args = ["format", "--stdin-file-path", "{file}"] }
# tab_width = 2
#
# [jsx]
# formatter = { command = "biome", args = ["format", "--stdin-file-path", "{file}"] }
# tab_width = 2

# ============================================================================
# TYPESCRIPT / JAVASCRIPT (Oxfmt - fastest, ~2x faster than Biome)
# Install: npm install -g oxfmt
# ============================================================================
# [typescript]
# formatter = { command = "oxfmt", args = ["--stdin-filepath={file}"] }
# tab_width = 2
#
# [javascript]
# formatter = { command = "oxfmt", args = ["--stdin-filepath={file}"] }
# tab_width = 2
#
# [tsx]
# formatter = { command = "oxfmt", args = ["--stdin-filepath={file}"] }
# tab_width = 2
#
# [jsx]
# formatter = { command = "oxfmt", args = ["--stdin-filepath={file}"] }
# tab_width = 2

# ============================================================================
# TYPESCRIPT / JAVASCRIPT (Prettier - if you prefer)
# ============================================================================
# [typescript]
# formatter = { command = "prettier", args = ["--stdin-filepath", "{file}"] }
# tab_width = 2
#
# [javascript]
# formatter = { command = "prettier", args = ["--stdin-filepath", "{file}"] }
# tab_width = 2

# ============================================================================
# PYTHON (Black formatter)
# ============================================================================
# [python]
# formatter = { command = "black", args = ["-", "--stdin-filename", "{file}"] }
# tab_width = 4

# ============================================================================
# CSS / SCSS (Oxfmt)
# ============================================================================
# [css]
# formatter = { command = "oxfmt", args = ["--stdin-filepath={file}"] }
# tab_width = 2
#
# [scss]
# formatter = { command = "oxfmt", args = ["--stdin-filepath={file}"] }
# tab_width = 2

# ============================================================================
# CSS / SCSS (Prettier)
# ============================================================================
# [css]
# formatter = { command = "prettier", args = ["--stdin-filepath", "{file}"] }
# tab_width = 2
#
# [scss]
# formatter = { command = "prettier", args = ["--stdin-filepath", "{file}"] }
# tab_width = 2

# ============================================================================
# JSON (Oxfmt)
# ============================================================================
# [json]
# formatter = { command = "oxfmt", args = ["--stdin-filepath={file}"] }
# tab_width = 2

# ============================================================================
# JSON (Prettier)
# ============================================================================
# [json]
# formatter = { command = "prettier", args = ["--stdin-filepath", "{file}"] }
# tab_width = 2

# ============================================================================
# RUST (uses rust-analyzer LSP by default - no external formatter needed)
# ============================================================================
# [rust]
# tab_width = 4

# ============================================================================
# GO
# ============================================================================
# [go]
# formatter = { command = "gofmt", args = [] }
# tab_width = 4
"#
}

/// Ensure languages.toml exists with template
fn ensure_languages_config_exists() {
    let Some(path) = languages_config_path() else {
        return;
    };

    // Create config directory if it doesn't exist
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            let _ = std::fs::create_dir_all(parent);
        }
    }

    // Create template if file doesn't exist
    if !path.exists() {
        let _ = std::fs::write(&path, default_languages_template());
    }
}

/// Load languages configuration from ~/.config/nevi/languages.toml
/// Returns default (empty) config if file doesn't exist or can't be parsed
pub fn load_languages_config() -> LanguagesConfig {
    // Ensure config file exists (creates template if not)
    ensure_languages_config_exists();

    let Some(path) = languages_config_path() else {
        return LanguagesConfig::default();
    };

    if !path.exists() {
        return LanguagesConfig::default();
    }

    match std::fs::read_to_string(&path) {
        Ok(content) => {
            match toml::from_str::<LanguagesConfig>(&content) {
                Ok(config) => config,
                Err(e) => {
                    eprintln!("Warning: Failed to parse languages.toml: {}", e);
                    LanguagesConfig::default()
                }
            }
        }
        Err(e) => {
            eprintln!("Warning: Failed to read languages.toml: {}", e);
            LanguagesConfig::default()
        }
    }
}
