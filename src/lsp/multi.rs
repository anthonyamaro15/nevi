//! Multi-language LSP manager
//!
//! Manages multiple language servers simultaneously, routing requests
//! to the appropriate server based on file type.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::config::LspServerConfig;
use crate::lsp::{LspManager, LspNotification};

/// Language identifier for routing LSP requests
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LanguageId {
    Rust,
    TypeScript,
    JavaScript,
    Css,
    Json,
    Toml,
    Markdown,
    Html,
    Python,
}

impl LanguageId {
    /// Detect language from file extension
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "rs" => Some(Self::Rust),
            "ts" | "tsx" | "mts" | "cts" => Some(Self::TypeScript),
            "js" | "jsx" | "mjs" | "cjs" => Some(Self::JavaScript),
            "css" | "scss" | "sass" | "less" => Some(Self::Css),
            "json" | "jsonc" => Some(Self::Json),
            "toml" => Some(Self::Toml),
            "md" | "markdown" => Some(Self::Markdown),
            "html" | "htm" => Some(Self::Html),
            "py" | "pyi" | "pyw" => Some(Self::Python),
            _ => None,
        }
    }

    /// Detect language from file path
    pub fn from_path(path: &Path) -> Option<Self> {
        path.extension()
            .and_then(|ext| ext.to_str())
            .and_then(Self::from_extension)
    }

    /// Get the LSP language identifier string
    pub fn as_lsp_id(&self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::TypeScript => "typescript",
            Self::JavaScript => "javascript",
            Self::Css => "css",
            Self::Json => "json",
            Self::Toml => "toml",
            Self::Markdown => "markdown",
            Self::Html => "html",
            Self::Python => "python",
        }
    }
}

/// State for a single language server
struct LspInstance {
    manager: LspManager,
    ready: bool,
    current_file: Option<PathBuf>,
    document_version: i32,
}

/// Manages multiple language servers
pub struct MultiLspManager {
    /// Active language server instances
    instances: HashMap<LanguageId, LspInstance>,
    /// Server configurations
    configs: HashMap<LanguageId, LspServerConfig>,
    /// Workspace root for all servers
    workspace_root: PathBuf,
}

impl MultiLspManager {
    /// Create a new multi-LSP manager with the given configurations
    pub fn new(
        workspace_root: PathBuf,
        rust_config: LspServerConfig,
        typescript_config: LspServerConfig,
        javascript_config: LspServerConfig,
        css_config: LspServerConfig,
        json_config: LspServerConfig,
        toml_config: LspServerConfig,
        markdown_config: LspServerConfig,
        html_config: LspServerConfig,
        python_config: LspServerConfig,
    ) -> Self {
        let mut configs = HashMap::new();
        configs.insert(LanguageId::Rust, rust_config);
        configs.insert(LanguageId::TypeScript, typescript_config);
        configs.insert(LanguageId::JavaScript, javascript_config);
        configs.insert(LanguageId::Css, css_config);
        configs.insert(LanguageId::Json, json_config);
        configs.insert(LanguageId::Toml, toml_config);
        configs.insert(LanguageId::Markdown, markdown_config);
        configs.insert(LanguageId::Html, html_config);
        configs.insert(LanguageId::Python, python_config);

        Self {
            instances: HashMap::new(),
            configs,
            workspace_root,
        }
    }

    /// Start a language server for the given language (if not already running)
    pub fn ensure_server_for_language(&mut self, lang: LanguageId) -> anyhow::Result<bool> {
        // Already running?
        if self.instances.contains_key(&lang) {
            return Ok(false);
        }

        // Get config
        let config = self.configs.get(&lang).ok_or_else(|| {
            anyhow::anyhow!("No config for language {:?}", lang)
        })?;

        // Check if enabled
        if !config.enabled {
            return Ok(false);
        }

        // Try to start the server
        match LspManager::start(&config.command, &config.args, self.workspace_root.clone()) {
            Ok(manager) => {
                self.instances.insert(lang, LspInstance {
                    manager,
                    ready: false,
                    current_file: None,
                    document_version: 1,
                });
                Ok(true)
            }
            Err(e) => Err(e),
        }
    }

    /// Start a server for a file if needed
    pub fn ensure_server_for_file(&mut self, path: &Path) -> anyhow::Result<Option<LanguageId>> {
        if let Some(lang) = LanguageId::from_path(path) {
            self.ensure_server_for_language(lang)?;
            Ok(Some(lang))
        } else {
            Ok(None)
        }
    }

    /// Check if a server is ready for the given language
    pub fn is_ready(&self, lang: LanguageId) -> bool {
        self.instances.get(&lang).map_or(false, |i| i.ready)
    }

    /// Check if any server is ready for the given file
    pub fn is_ready_for_file(&self, path: &Path) -> bool {
        LanguageId::from_path(path)
            .map_or(false, |lang| self.is_ready(lang))
    }

    /// Get a mutable reference to the instance for a language
    fn get_instance_mut(&mut self, lang: LanguageId) -> Option<&mut LspInstance> {
        self.instances.get_mut(&lang)
    }

    /// Poll all servers for notifications
    pub fn poll_notifications(&mut self) -> Vec<(LanguageId, LspNotification)> {
        let mut notifications = Vec::new();

        for (&lang, instance) in &mut self.instances {
            while let Some(notification) = instance.manager.try_recv() {
                // Update ready state
                if let LspNotification::Initialized = &notification {
                    instance.ready = true;
                }
                if let LspNotification::Error { .. } = &notification {
                    instance.ready = false;
                }
                notifications.push((lang, notification));
            }
        }

        notifications
    }

    /// Send did_open notification to appropriate server
    pub fn did_open(&mut self, path: &PathBuf, text: &str) -> anyhow::Result<()> {
        let lang = LanguageId::from_path(path)
            .ok_or_else(|| anyhow::anyhow!("Unknown language for {:?}", path))?;

        if let Some(instance) = self.get_instance_mut(lang) {
            if instance.ready {
                instance.document_version = 1;
                instance.manager.did_open(path, text)?;
                instance.current_file = Some(path.clone());
            }
        }
        Ok(())
    }

    /// Send did_change notification to appropriate server
    pub fn did_change(&mut self, path: &PathBuf, text: &str) -> anyhow::Result<()> {
        let lang = LanguageId::from_path(path)
            .ok_or_else(|| anyhow::anyhow!("Unknown language for {:?}", path))?;

        if let Some(instance) = self.get_instance_mut(lang) {
            if instance.ready {
                instance.document_version += 1;
                instance.manager.did_change(path, instance.document_version, text)?;
            }
        }
        Ok(())
    }

    /// Send did_close notification to appropriate server
    pub fn did_close(&mut self, path: &PathBuf) -> anyhow::Result<()> {
        let lang = LanguageId::from_path(path)
            .ok_or_else(|| anyhow::anyhow!("Unknown language for {:?}", path))?;

        if let Some(instance) = self.get_instance_mut(lang) {
            if instance.ready {
                instance.manager.did_close(path)?;
                if instance.current_file.as_ref() == Some(path) {
                    instance.current_file = None;
                }
            }
        }
        Ok(())
    }

    /// Request completions for a file
    pub fn completion(&mut self, path: &PathBuf, line: u32, character: u32) -> anyhow::Result<()> {
        let lang = LanguageId::from_path(path)
            .ok_or_else(|| anyhow::anyhow!("Unknown language for {:?}", path))?;

        if let Some(instance) = self.get_instance_mut(lang) {
            if instance.ready {
                instance.manager.completion(path, line, character)?;
            }
        }
        Ok(())
    }

    /// Resolve a completion item to get full documentation
    pub fn completion_resolve(&mut self, path: &PathBuf, item: serde_json::Value, label: String) -> anyhow::Result<()> {
        let lang = LanguageId::from_path(path)
            .ok_or_else(|| anyhow::anyhow!("Unknown language for {:?}", path))?;

        if let Some(instance) = self.get_instance_mut(lang) {
            if instance.ready {
                instance.manager.completion_resolve(item, label)?;
            }
        }
        Ok(())
    }

    /// Request hover information for a file
    pub fn hover(&mut self, path: &PathBuf, line: u32, character: u32) -> anyhow::Result<()> {
        let lang = LanguageId::from_path(path)
            .ok_or_else(|| anyhow::anyhow!("Unknown language for {:?}", path))?;

        if let Some(instance) = self.get_instance_mut(lang) {
            if instance.ready {
                instance.manager.hover(path, line, character)?;
            }
        }
        Ok(())
    }

    /// Request go-to-definition for a file
    pub fn goto_definition(&mut self, path: &PathBuf, line: u32, character: u32) -> anyhow::Result<()> {
        let lang = LanguageId::from_path(path)
            .ok_or_else(|| anyhow::anyhow!("Unknown language for {:?}", path))?;

        if let Some(instance) = self.get_instance_mut(lang) {
            if instance.ready {
                instance.manager.goto_definition(path, line, character)?;
            }
        }
        Ok(())
    }

    /// Request references for a symbol
    pub fn references(&mut self, path: &PathBuf, line: u32, character: u32) -> anyhow::Result<()> {
        let lang = LanguageId::from_path(path)
            .ok_or_else(|| anyhow::anyhow!("Unknown language for {:?}", path))?;

        if let Some(instance) = self.get_instance_mut(lang) {
            if instance.ready {
                instance.manager.references(path, line, character)?;
            }
        }
        Ok(())
    }

    /// Request signature help
    pub fn signature_help(&mut self, path: &PathBuf, line: u32, character: u32) -> anyhow::Result<()> {
        let lang = LanguageId::from_path(path)
            .ok_or_else(|| anyhow::anyhow!("Unknown language for {:?}", path))?;

        if let Some(instance) = self.get_instance_mut(lang) {
            if instance.ready {
                instance.manager.signature_help(path, line, character)?;
            }
        }
        Ok(())
    }

    /// Request document formatting
    pub fn formatting(&mut self, path: &PathBuf, tab_size: u32) -> anyhow::Result<()> {
        let lang = LanguageId::from_path(path)
            .ok_or_else(|| anyhow::anyhow!("Unknown language for {:?}", path))?;

        if let Some(instance) = self.get_instance_mut(lang) {
            if instance.ready {
                instance.manager.formatting(path, tab_size)?;
            }
        }
        Ok(())
    }

    /// Request code actions
    pub fn code_action(
        &mut self,
        path: &PathBuf,
        start_line: u32,
        start_character: u32,
        end_line: u32,
        end_character: u32,
        diagnostics: Vec<crate::lsp::types::Diagnostic>,
    ) -> anyhow::Result<()> {
        let lang = LanguageId::from_path(path)
            .ok_or_else(|| anyhow::anyhow!("Unknown language for {:?}", path))?;

        if let Some(instance) = self.get_instance_mut(lang) {
            if instance.ready {
                instance.manager.code_action(
                    path,
                    start_line,
                    start_character,
                    end_line,
                    end_character,
                    diagnostics,
                )?;
            }
        }
        Ok(())
    }

    /// Request rename
    pub fn rename(&mut self, path: &PathBuf, line: u32, character: u32, new_name: String) -> anyhow::Result<()> {
        let lang = LanguageId::from_path(path)
            .ok_or_else(|| anyhow::anyhow!("Unknown language for {:?}", path))?;

        if let Some(instance) = self.get_instance_mut(lang) {
            if instance.ready {
                instance.manager.rename(path, line, character, new_name)?;
            }
        }
        Ok(())
    }

    /// Shutdown all servers
    pub fn shutdown(&mut self) {
        for (_, instance) in &mut self.instances {
            instance.manager.shutdown();
        }
        self.instances.clear();
    }

    /// Get status string for display
    pub fn status(&self, path: Option<&Path>) -> String {
        if let Some(p) = path {
            if let Some(lang) = LanguageId::from_path(p) {
                if let Some(instance) = self.instances.get(&lang) {
                    if instance.ready {
                        return format!("LSP: {} ready", lang.as_lsp_id());
                    } else {
                        return format!("LSP: {} starting...", lang.as_lsp_id());
                    }
                } else {
                    // Check if config exists and is enabled
                    if let Some(config) = self.configs.get(&lang) {
                        if config.enabled {
                            return format!("LSP: {} (not started)", lang.as_lsp_id());
                        } else {
                            return format!("LSP: {} (disabled)", lang.as_lsp_id());
                        }
                    }
                }
            }
        }

        // Count active servers
        let active = self.instances.len();
        let ready = self.instances.values().filter(|i| i.ready).count();
        if active > 0 {
            format!("LSP: {}/{} servers", ready, active)
        } else {
            "LSP: (no server)".to_string()
        }
    }
}

impl Drop for MultiLspManager {
    fn drop(&mut self) {
        self.shutdown();
    }
}
