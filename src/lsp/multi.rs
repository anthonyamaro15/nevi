//! Multi-language LSP manager
//!
//! Manages multiple language servers simultaneously, routing requests
//! to the appropriate server based on file type.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::config::LspServerConfig;
use crate::lsp::{LspManager, LspNotification};

const PROGRESS_DISPLAY_DELAY: Duration = Duration::from_millis(250);

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
    last_error: Option<String>,
    progress: Option<LspProgressState>,
    current_file: Option<PathBuf>,
    document_version: i32,
}

struct LspProgressState {
    label: String,
    started_at: Instant,
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
    pub fn language_for_path(&self, path: &Path) -> Option<LanguageId> {
        let ext = path.extension().and_then(|ext| ext.to_str())?;
        if let Some(lang) = LanguageId::from_extension(ext) {
            return Some(lang);
        }

        [
            LanguageId::Rust,
            LanguageId::TypeScript,
            LanguageId::JavaScript,
            LanguageId::Css,
            LanguageId::Json,
            LanguageId::Toml,
            LanguageId::Markdown,
            LanguageId::Html,
            LanguageId::Python,
        ]
        .into_iter()
        .find(|lang| {
            self.configs
                .get(lang)
                .map(|config| {
                    config
                        .file_extensions
                        .iter()
                        .any(|configured| configured.eq_ignore_ascii_case(ext))
                })
                .unwrap_or(false)
        })
    }

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

    fn command_name(command: &str) -> &str {
        Path::new(command)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(command)
    }

    fn install_hint_for_command(command: &str) -> Option<&'static str> {
        match Self::command_name(command) {
            "typescript-language-server" | "typescript-language-server.cmd" => {
                Some("npm install -g typescript typescript-language-server")
            }
            "rust-analyzer" | "rust-analyzer.exe" => Some("rustup component add rust-analyzer"),
            "vscode-css-language-server"
            | "vscode-json-language-server"
            | "vscode-html-language-server"
            | "vscode-eslint-language-server" => {
                Some("npm install -g vscode-langservers-extracted")
            }
            "taplo" | "taplo.exe" => Some("cargo install taplo-cli --locked"),
            "pyright-langserver" | "pyright-langserver.cmd" => Some("npm install -g pyright"),
            "pylsp" => Some("pipx install python-lsp-server"),
            "biome" | "biome.cmd" => Some("npm install -g @biomejs/biome"),
            _ => None,
        }
    }

    fn is_missing_command_error(message: &str) -> bool {
        let msg = message.to_ascii_lowercase();
        msg.contains("no such file or directory")
            || msg.contains("os error 2")
            || msg.contains("not found")
    }

    fn format_error_for_display(command: &str, message: &str) -> String {
        let command_name = Self::command_name(command);
        if Self::is_missing_command_error(message) {
            if let Some(hint) = Self::install_hint_for_command(command_name) {
                return format!("{command_name} not found. Install: {hint}");
            }
            return format!("{command_name} not found. Add it to PATH or update config.toml");
        }

        format!("{command_name}: {message}")
    }

    fn format_progress_for_display(
        title: &str,
        message: &Option<String>,
        percentage: Option<u64>,
    ) -> String {
        let mut label = message
            .as_deref()
            .filter(|message| !message.trim().is_empty())
            .map(Self::compact_progress_message)
            .unwrap_or_else(|| Self::compact_progress_message(title));
        if let Some(percentage) = percentage {
            label = format!("{label} {percentage}%");
        }
        label
    }

    fn compact_progress_message(message: &str) -> String {
        let message = message.trim();
        if let Some((count, path)) = message.split_once(':') {
            let count = count.trim();
            if Self::looks_like_progress_count(count) && Path::new(path.trim()).is_absolute() {
                return format!("{count} files");
            }
        }

        Self::truncate_status_text(message, 48)
    }

    fn looks_like_progress_count(text: &str) -> bool {
        let Some((current, total)) = text.split_once('/') else {
            return false;
        };

        !current.is_empty()
            && !total.is_empty()
            && current.chars().all(|ch| ch.is_ascii_digit())
            && total.chars().all(|ch| ch.is_ascii_digit())
    }

    fn truncate_status_text(text: &str, max_chars: usize) -> String {
        let mut chars = text.chars();
        let mut truncated = String::new();
        for _ in 0..max_chars {
            let Some(ch) = chars.next() else {
                return text.to_string();
            };
            truncated.push(ch);
        }

        if chars.next().is_some() {
            truncated.push_str("...");
        }
        truncated
    }

    fn should_clear_progress(done: bool, percentage: Option<u64>) -> bool {
        done || percentage.map_or(false, |percentage| percentage >= 100)
    }

    fn should_show_progress(started_at: Instant, now: Instant) -> bool {
        now.duration_since(started_at) >= PROGRESS_DISPLAY_DELAY
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
            let config = self
                .configs
                .get(&lang)
                .ok_or_else(|| anyhow::anyhow!("No config for language {:?}", lang))?;
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
                self.instances.insert(
                    lang,
                    LspInstance {
                        manager,
                        ready: false,
                        last_error: None,
                        progress: None,
                        current_file: None,
                        document_version: 1,
                    },
                );
                Ok(true)
            }
            Err(e) => Err(e),
        }
    }

    /// Start a server for a file if needed
    pub fn ensure_server_for_file(&mut self, path: &Path) -> anyhow::Result<Option<LanguageId>> {
        if let Some(lang) = self.language_for_path(path) {
            let Some(config) = self.configs.get(&lang) else {
                return Ok(None);
            };
            if !config.enabled {
                return Ok(None);
            }
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
        self.language_for_path(path)
            .map_or(false, |lang| self.is_ready(lang))
    }

    /// Get a mutable reference to the instance for a language
    fn get_instance_mut(&mut self, lang: LanguageId) -> Option<&mut LspInstance> {
        self.instances.get_mut(&lang)
    }

    /// Poll all servers for notifications
    pub fn poll_notifications(&mut self) -> Vec<(LanguageId, LspNotification)> {
        self.poll_notifications_limited(None)
    }

    /// Poll all servers for notifications, optionally limiting total work.
    ///
    /// The editor uses a small limit while keyboard input is active so LSP progress
    /// and hover responses keep moving without draining a large notification burst.
    pub fn poll_notifications_limited(
        &mut self,
        limit: Option<usize>,
    ) -> Vec<(LanguageId, LspNotification)> {
        let mut notifications = Vec::new();

        for (&lang, instance) in &mut self.instances {
            if limit.map_or(false, |limit| notifications.len() >= limit) {
                break;
            }

            while let Some(notification) = instance.manager.try_recv() {
                // Update ready state
                if let LspNotification::Initialized = &notification {
                    instance.ready = true;
                    instance.last_error = None;
                }
                if let LspNotification::Progress {
                    title,
                    message,
                    percentage,
                    done,
                } = &notification
                {
                    if Self::should_clear_progress(*done, *percentage) {
                        instance.progress = None;
                    } else {
                        let label = Self::format_progress_for_display(title, message, *percentage);
                        if let Some(progress) = &mut instance.progress {
                            progress.label = label;
                        } else {
                            instance.progress = Some(LspProgressState {
                                label,
                                started_at: Instant::now(),
                            });
                        }
                    }
                }
                if let LspNotification::Error { message } = &notification {
                    // Not all LSP "error" notifications are fatal (for example stderr logs).
                    // Keep the server ready unless we detect a transport/startup failure.
                    if Self::is_fatal_error(message) {
                        instance.ready = false;
                        let command = self
                            .configs
                            .get(&lang)
                            .map(|config| config.effective_command())
                            .unwrap_or(lang.as_lsp_id());
                        instance.last_error =
                            Some(Self::format_error_for_display(command, message));
                    }
                }
                notifications.push((lang, notification));

                if limit.map_or(false, |limit| notifications.len() >= limit) {
                    break;
                }
            }
        }

        notifications
    }

    /// Send did_open notification to appropriate server
    pub fn did_open(&mut self, path: &PathBuf, text: &str) -> anyhow::Result<()> {
        let lang = self
            .language_for_path(path)
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
        let lang = self
            .language_for_path(path)
            .ok_or_else(|| anyhow::anyhow!("Unknown language for {:?}", path))?;

        if let Some(instance) = self.get_instance_mut(lang) {
            if instance.ready {
                instance.document_version += 1;
                instance
                    .manager
                    .did_change(path, instance.document_version, text)?;
            }
        }
        Ok(())
    }

    /// Send did_close notification to appropriate server
    pub fn did_close(&mut self, path: &PathBuf) -> anyhow::Result<()> {
        let lang = self
            .language_for_path(path)
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
    pub fn completion(
        &mut self,
        path: &PathBuf,
        line: u32,
        character: u32,
        buffer_version: u64,
    ) -> anyhow::Result<()> {
        let lang = self
            .language_for_path(path)
            .ok_or_else(|| anyhow::anyhow!("Unknown language for {:?}", path))?;

        if let Some(instance) = self.get_instance_mut(lang) {
            if instance.ready {
                instance
                    .manager
                    .completion(path, line, character, buffer_version)?;
            }
        }
        Ok(())
    }

    /// Resolve a completion item to get full documentation
    pub fn completion_resolve(
        &mut self,
        path: &PathBuf,
        item: serde_json::Value,
        item_id: u64,
        label: String,
    ) -> anyhow::Result<()> {
        let lang = self
            .language_for_path(path)
            .ok_or_else(|| anyhow::anyhow!("Unknown language for {:?}", path))?;

        if let Some(instance) = self.get_instance_mut(lang) {
            if instance.ready {
                instance.manager.completion_resolve(item, item_id, label)?;
            }
        }
        Ok(())
    }

    /// Request hover information for a file
    pub fn hover(&mut self, path: &PathBuf, line: u32, character: u32) -> anyhow::Result<()> {
        let lang = self
            .language_for_path(path)
            .ok_or_else(|| anyhow::anyhow!("Unknown language for {:?}", path))?;

        if let Some(instance) = self.get_instance_mut(lang) {
            if instance.ready {
                instance.manager.hover(path, line, character)?;
            }
        }
        Ok(())
    }

    /// Request go-to-definition for a file
    pub fn goto_definition(
        &mut self,
        path: &PathBuf,
        line: u32,
        character: u32,
    ) -> anyhow::Result<()> {
        let lang = self
            .language_for_path(path)
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
        let lang = self
            .language_for_path(path)
            .ok_or_else(|| anyhow::anyhow!("Unknown language for {:?}", path))?;

        if let Some(instance) = self.get_instance_mut(lang) {
            if instance.ready {
                instance.manager.references(path, line, character)?;
            }
        }
        Ok(())
    }

    /// Request signature help
    pub fn signature_help(
        &mut self,
        path: &PathBuf,
        line: u32,
        character: u32,
    ) -> anyhow::Result<()> {
        let lang = self
            .language_for_path(path)
            .ok_or_else(|| anyhow::anyhow!("Unknown language for {:?}", path))?;

        if let Some(instance) = self.get_instance_mut(lang) {
            if instance.ready {
                instance.manager.signature_help(path, line, character)?;
            }
        }
        Ok(())
    }

    /// Request document formatting
    pub fn formatting(
        &mut self,
        path: &PathBuf,
        tab_size: u32,
        buffer_version: u64,
    ) -> anyhow::Result<()> {
        let lang = self
            .language_for_path(path)
            .ok_or_else(|| anyhow::anyhow!("Unknown language for {:?}", path))?;

        if let Some(instance) = self.get_instance_mut(lang) {
            if instance.ready {
                instance
                    .manager
                    .formatting(path, tab_size, buffer_version)?;
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
        buffer_version: u64,
        diagnostics: Vec<crate::lsp::types::Diagnostic>,
    ) -> anyhow::Result<()> {
        let lang = self
            .language_for_path(path)
            .ok_or_else(|| anyhow::anyhow!("Unknown language for {:?}", path))?;

        if let Some(instance) = self.get_instance_mut(lang) {
            if instance.ready {
                instance.manager.code_action(
                    path,
                    start_line,
                    start_character,
                    end_line,
                    end_character,
                    buffer_version,
                    diagnostics,
                )?;
            }
        }
        Ok(())
    }

    /// Request rename
    pub fn rename(
        &mut self,
        path: &PathBuf,
        line: u32,
        character: u32,
        new_name: String,
        buffer_version: u64,
    ) -> anyhow::Result<()> {
        let lang = self
            .language_for_path(path)
            .ok_or_else(|| anyhow::anyhow!("Unknown language for {:?}", path))?;

        if let Some(instance) = self.get_instance_mut(lang) {
            if instance.ready {
                instance
                    .manager
                    .rename(path, line, character, new_name, buffer_version)?;
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
            if let Some(lang) = self.language_for_path(p) {
                if let Some(instance) = self.instances.get(&lang) {
                    // Get the server name from config
                    let server_name = self
                        .configs
                        .get(&lang)
                        .map(|c| c.effective_command())
                        .unwrap_or("unknown");

                    if let Some(error) = &instance.last_error {
                        return format!("LSP: {error}");
                    } else if let Some(progress) = &instance.progress {
                        if Self::should_show_progress(progress.started_at, Instant::now()) {
                            return format!(
                                "LSP: {} loading: {} ({})",
                                server_name,
                                progress.label,
                                lang.as_lsp_id()
                            );
                        }
                    } else if instance.ready {
                        return format!("LSP: {} ready ({})", server_name, lang.as_lsp_id());
                    }

                    if instance.ready {
                        return format!("LSP: {} ready ({})", server_name, lang.as_lsp_id());
                    }

                    return format!("LSP: starting {} ({})...", server_name, lang.as_lsp_id());
                } else {
                    // Check if config exists and is enabled
                    if let Some(config) = self.configs.get(&lang) {
                        if config.enabled {
                            return format!(
                                "LSP: {} not started ({})",
                                config.effective_command(),
                                lang.as_lsp_id()
                            );
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

    pub fn user_facing_error(&self, lang: LanguageId, message: &str) -> String {
        let command = self
            .configs
            .get(&lang)
            .map(|config| config.effective_command())
            .unwrap_or(lang.as_lsp_id());
        Self::format_error_for_display(command, message)
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

    #[test]
    fn status_names_configured_server_before_startup() {
        let tmp = unique_temp_dir("nevi_lsp_status");
        let workspace_root = tmp.join("workspace");
        fs::create_dir_all(&workspace_root).expect("create workspace");

        let manager = make_manager(workspace_root);
        assert_eq!(
            manager.status(Some(Path::new("src/main.rs"))),
            "LSP: rust-analyzer not started (rust)"
        );

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn missing_typescript_server_error_includes_install_hint() {
        let message = MultiLspManager::format_error_for_display(
            "typescript-language-server",
            "Failed to start LSP server: Failed to spawn LSP server 'typescript-language-server': No such file or directory (os error 2)",
        );

        assert_eq!(
            message,
            "typescript-language-server not found. Install: npm install -g typescript typescript-language-server"
        );
    }

    #[test]
    fn missing_unknown_server_error_points_to_path_or_config() {
        let message = MultiLspManager::format_error_for_display(
            "custom-ls",
            "Failed to spawn LSP server 'custom-ls': No such file or directory",
        );

        assert_eq!(
            message,
            "custom-ls not found. Add it to PATH or update config.toml"
        );
    }

    #[test]
    fn progress_status_prefers_message_and_percentage() {
        assert_eq!(
            MultiLspManager::format_progress_for_display(
                "rust-analyzer",
                &Some("indexing".to_string()),
                Some(42)
            ),
            "indexing 42%"
        );
        assert_eq!(
            MultiLspManager::format_progress_for_display("rust-analyzer", &None, None),
            "rust-analyzer"
        );
    }

    #[test]
    fn progress_status_compacts_rust_analyzer_absolute_paths() {
        assert_eq!(
            MultiLspManager::format_progress_for_display(
                "Indexing",
                &Some(
                    "191/193: /Users/aamaro/.rustup/toolchains/stable-aarch64-apple-darwin/lib/rustlib/src/rust/library/std/src/lib.rs"
                        .to_string()
                ),
                None,
            ),
            "191/193 files"
        );
    }

    #[test]
    fn progress_status_truncates_long_messages() {
        assert_eq!(
            MultiLspManager::format_progress_for_display(
                "rust-analyzer",
                &Some("this is a very long progress message that should not take over the status line".to_string()),
                None,
            ),
            "this is a very long progress message that should..."
        );
    }

    #[test]
    fn progress_status_clears_completed_reports() {
        assert!(MultiLspManager::should_clear_progress(false, Some(100)));
        assert!(MultiLspManager::should_clear_progress(true, Some(42)));
        assert!(!MultiLspManager::should_clear_progress(false, Some(99)));
        assert!(!MultiLspManager::should_clear_progress(false, None));
    }

    #[test]
    fn progress_status_waits_before_displaying() {
        let now = Instant::now();

        assert!(!MultiLspManager::should_show_progress(now, now));
        assert!(MultiLspManager::should_show_progress(
            now - PROGRESS_DISPLAY_DELAY,
            now
        ));
    }
}
