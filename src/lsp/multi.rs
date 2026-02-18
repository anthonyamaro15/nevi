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
    fn resolve_server_root(&self, lang: LanguageId, file_path: Option<&Path>) -> PathBuf {
        let Some(path) = file_path else {
            return self.workspace_root.clone();
        };

        let Some(config) = self.configs.get(&lang) else {
            return self.workspace_root.clone();
        };

        if config.root_patterns.is_empty() {
            return self.workspace_root.clone();
        }

        let mut current = if path.is_dir() {
            Some(path.to_path_buf())
        } else {
            path.parent().map(Path::to_path_buf)
        };

        while let Some(dir) = current {
            let is_root = config
                .root_patterns
                .iter()
                .filter(|marker| !marker.trim().is_empty())
                .any(|marker| dir.join(marker).exists());
            if is_root {
                return dir;
            }
            current = dir.parent().map(Path::to_path_buf);
        }

        self.workspace_root.clone()
    }

    fn is_fatal_error(message: &str) -> bool {
        let msg = message.to_ascii_lowercase();
        msg.contains("failed to start lsp server")
            || msg.contains("failed to get lsp server stdout")
            || msg.contains("failed to initialize lsp")
            || msg.contains("broken pipe")
            || msg.contains("connection reset")
            || msg.contains("transport is closing")
            || msg.contains("channel closed")
    }

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
        self.ensure_server_for_language_with_file(lang, None)
    }

    fn ensure_server_for_language_with_file(
        &mut self,
        lang: LanguageId,
        file_path: Option<&Path>,
    ) -> anyhow::Result<bool> {
        // Already running?
        if self.instances.contains_key(&lang) {
            return Ok(false);
        }

        // Get config data without holding the borrow across server startup.
        let (enabled, command, args) = {
            let config = self.configs.get(&lang).ok_or_else(|| {
                anyhow::anyhow!("No config for language {:?}", lang)
            })?;
            (
                config.enabled,
                config.effective_command().to_string(),
                config.effective_args(),
            )
        };

        // Check if enabled
        if !enabled {
            return Ok(false);
        }

        let root_path = self.resolve_server_root(lang, file_path);

        // Try to start the server (using effective command/args which resolve presets)
        match LspManager::start(&command, &args, root_path) {
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
            self.ensure_server_for_language_with_file(lang, Some(path))?;
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
                if let LspNotification::Error { message } = &notification {
                    // Not all LSP "error" notifications are fatal (for example stderr logs).
                    // Keep the server ready unless we detect a transport/startup failure.
                    if Self::is_fatal_error(message) {
                        instance.ready = false;
                    }
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
                    // Get the server name from config
                    let server_name = self.configs.get(&lang)
                        .map(|c| c.effective_command())
                        .unwrap_or("unknown");

                    if instance.ready {
                        return format!("LSP: {} ({})", server_name, lang.as_lsp_id());
                    } else {
                        return format!("LSP: {} starting...", server_name);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_manager(workspace_root: PathBuf) -> MultiLspManager {
        let servers = crate::config::LspServers::default();
        MultiLspManager::new(
            workspace_root,
            servers.rust,
            servers.typescript,
            servers.javascript,
            servers.css,
            servers.json,
            servers.toml,
            servers.markdown,
            servers.html,
            servers.python,
        )
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), nanos))
    }

    #[test]
    fn resolve_server_root_uses_language_root_markers() {
        let tmp = unique_temp_dir("nevi_lsp_root");
        let workspace_root = tmp.join("workspace");
        let project_root = workspace_root.join("project");
        let nested = project_root.join("src/bin");
        fs::create_dir_all(&nested).expect("create nested tree");
        fs::write(
            project_root.join("Cargo.toml"),
            "[package]\nname=\"x\"\nversion=\"0.1.0\"\n",
        )
        .expect("write cargo marker");

        let manager = make_manager(workspace_root.clone());
        let file_path = nested.join("main.rs");
        let resolved = manager.resolve_server_root(LanguageId::Rust, Some(file_path.as_path()));
        assert_eq!(resolved, project_root);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn resolve_server_root_falls_back_to_workspace_root() {
        let tmp = unique_temp_dir("nevi_lsp_root_fallback");
        let workspace_root = tmp.join("workspace");
        let nested = workspace_root.join("scratch/src");
        fs::create_dir_all(&nested).expect("create nested tree");

        let manager = make_manager(workspace_root.clone());
        let file_path = nested.join("main.rs");
        let resolved = manager.resolve_server_root(LanguageId::Rust, Some(file_path.as_path()));
        assert_eq!(resolved, workspace_root);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn fatal_error_detection_is_not_triggered_by_stderr_logs() {
        assert!(MultiLspManager::is_fatal_error(
            "Failed to start LSP server: No such file or directory"
        ));
        assert!(MultiLspManager::is_fatal_error(
            "Failed to send didChange: Broken pipe (os error 32)"
        ));
        assert!(!MultiLspManager::is_fatal_error(
            "LSP stderr: rust-analyzer: using proc-macro server"
        ));
    }
}
