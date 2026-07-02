use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ToolCategory {
    Lsp,
    Formatter,
    Optional,
}

impl ToolCategory {
    fn heading(self) -> &'static str {
        match self {
            Self::Lsp => "LSP servers",
            Self::Formatter => "Formatters",
            Self::Optional => "Optional tools",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolInstallItem {
    pub category: ToolCategory,
    pub labels: Vec<String>,
    pub command: String,
    pub install_command: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolInstallReport {
    pub items: Vec<ToolInstallItem>,
}

impl ToolInstallReport {
    pub fn render_markdown(&self) -> String {
        let mut output = String::from("# Nevi Tool Installer\n\n");
        output.push_str(
            "This report lists missing configured tools and the command Nevi knows how to install them with.\n",
        );
        output.push_str("Run install commands in your terminal, then restart Nevi or run `:checkhealth` again.\n\n");

        if self.items.is_empty() {
            output.push_str("No missing tools with known checks.\n");
            return output;
        }

        for category in [
            ToolCategory::Lsp,
            ToolCategory::Formatter,
            ToolCategory::Optional,
        ] {
            let items = self
                .items
                .iter()
                .filter(|item| item.category == category)
                .collect::<Vec<_>>();
            if items.is_empty() {
                continue;
            }

            output.push_str(&format!("## {}\n", category.heading()));
            for item in items {
                output.push_str(&format!(
                    "- {} (`{}`)\n",
                    item.labels.join(", "),
                    item.command
                ));
                if let Some(command) = &item.install_command {
                    output.push_str(&format!("  - Install: `{command}`\n"));
                } else {
                    output.push_str(
                        "  - No known install command. Install it manually, add it to PATH, or update `config.toml`.\n",
                    );
                }
            }
            output.push('\n');
        }

        output
    }
}

pub fn collect_tool_install_report<F>(
    settings: &crate::config::Settings,
    languages_config: &crate::config::LanguagesConfig,
    is_command_available: F,
) -> ToolInstallReport
where
    F: Fn(&str) -> bool,
{
    let mut grouped = BTreeMap::<(ToolCategory, String), ToolInstallItem>::new();

    if settings.lsp.enabled {
        let servers = &settings.lsp.servers;
        add_lsp_tool(&mut grouped, "rust", &servers.rust, &is_command_available);
        add_lsp_tool(
            &mut grouped,
            "typescript",
            &servers.typescript,
            &is_command_available,
        );
        add_lsp_tool(
            &mut grouped,
            "javascript",
            &servers.javascript,
            &is_command_available,
        );
        add_lsp_tool(&mut grouped, "css", &servers.css, &is_command_available);
        add_lsp_tool(&mut grouped, "json", &servers.json, &is_command_available);
        add_lsp_tool(&mut grouped, "toml", &servers.toml, &is_command_available);
        add_lsp_tool(
            &mut grouped,
            "markdown",
            &servers.markdown,
            &is_command_available,
        );
        add_lsp_tool(&mut grouped, "html", &servers.html, &is_command_available);
        add_lsp_tool(
            &mut grouped,
            "python",
            &servers.python,
            &is_command_available,
        );
    }

    for (language, config) in &languages_config.languages {
        let Some(formatter) = config.formatter.as_ref() else {
            continue;
        };
        add_command_tool(
            &mut grouped,
            ToolCategory::Formatter,
            language,
            &formatter.command,
            &is_command_available,
        );
    }

    let mut items = grouped.into_values().collect::<Vec<_>>();
    for item in &mut items {
        item.labels.sort();
        item.labels.dedup();
    }
    items.sort_by(|a, b| {
        a.category
            .cmp(&b.category)
            .then(a.labels.join(", ").cmp(&b.labels.join(", ")))
            .then(a.command.cmp(&b.command))
    });

    ToolInstallReport { items }
}

fn add_lsp_tool<F>(
    grouped: &mut BTreeMap<(ToolCategory, String), ToolInstallItem>,
    label: &str,
    config: &crate::config::LspServerConfig,
    is_command_available: &F,
) where
    F: Fn(&str) -> bool,
{
    if !config.enabled {
        return;
    }

    add_command_tool(
        grouped,
        ToolCategory::Lsp,
        label,
        config.effective_command(),
        is_command_available,
    );
}

fn add_command_tool<F>(
    grouped: &mut BTreeMap<(ToolCategory, String), ToolInstallItem>,
    category: ToolCategory,
    label: &str,
    command: &str,
    is_command_available: &F,
) where
    F: Fn(&str) -> bool,
{
    let command = command.trim();
    if command.is_empty() || is_command_available(command) {
        return;
    }

    let key = (category, command.to_string());
    let install_command = install_command_for(command).map(str::to_string);
    grouped
        .entry(key)
        .and_modify(|item| item.labels.push(label.to_string()))
        .or_insert_with(|| ToolInstallItem {
            category,
            labels: vec![label.to_string()],
            command: command.to_string(),
            install_command,
        });
}

pub fn install_command_for(command: &str) -> Option<&'static str> {
    match command_name(command) {
        "typescript-language-server" | "typescript-language-server.cmd" => {
            Some("npm install -g typescript typescript-language-server")
        }
        "rust-analyzer" | "rust-analyzer.exe" => Some("rustup component add rust-analyzer"),
        "ra-multiplex" | "ra-multiplex.exe" => Some("cargo install ra-multiplex"),
        "vscode-css-language-server"
        | "vscode-json-language-server"
        | "vscode-html-language-server"
        | "vscode-eslint-language-server" => Some("npm install -g vscode-langservers-extracted"),
        "taplo" | "taplo.exe" => Some("cargo install taplo-cli --locked"),
        "pyright-langserver" | "pyright-langserver.cmd" => Some("npm install -g pyright"),
        "pylsp" => Some("pipx install python-lsp-server"),
        "biome" | "biome.cmd" => Some("npm install -g @biomejs/biome"),
        "oxfmt" | "oxfmt.cmd" => Some("npm install -g oxfmt"),
        "prettier" | "prettier.cmd" => Some("npm install -g prettier"),
        _ => None,
    }
}

fn command_name(command: &str) -> &str {
    Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(command)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::config::{
        FormatterConfig, LanguageConfig, LanguagesConfig, LspServerConfig, Settings,
    };

    fn disable_all_lsp(settings: &mut Settings) {
        settings.lsp.servers.rust.enabled = false;
        settings.lsp.servers.typescript.enabled = false;
        settings.lsp.servers.javascript.enabled = false;
        settings.lsp.servers.css.enabled = false;
        settings.lsp.servers.json.enabled = false;
        settings.lsp.servers.toml.enabled = false;
        settings.lsp.servers.markdown.enabled = false;
        settings.lsp.servers.html.enabled = false;
        settings.lsp.servers.python.enabled = false;
    }

    #[test]
    fn install_command_for_known_lsp_command_uses_basename() {
        assert_eq!(
            super::install_command_for("/usr/local/bin/typescript-language-server"),
            Some("npm install -g typescript typescript-language-server")
        );
        assert_eq!(
            super::install_command_for("rust-analyzer.exe"),
            Some("rustup component add rust-analyzer")
        );
    }

    #[test]
    fn tool_install_report_deduplicates_missing_lsp_commands() {
        let mut settings = Settings::default();
        disable_all_lsp(&mut settings);
        settings.lsp.servers.typescript.enabled = true;
        settings.lsp.servers.javascript.enabled = true;

        let report =
            super::collect_tool_install_report(&settings, &LanguagesConfig::default(), |_| false);

        assert_eq!(report.items.len(), 1);
        assert_eq!(report.items[0].category, super::ToolCategory::Lsp);
        assert_eq!(report.items[0].labels, vec!["javascript", "typescript"]);
        assert_eq!(report.items[0].command, "typescript-language-server");
        assert_eq!(
            report.items[0].install_command.as_deref(),
            Some("npm install -g typescript typescript-language-server")
        );

        let rendered = report.render_markdown();
        assert!(rendered.contains("# Nevi Tool Installer"));
        assert!(rendered.contains("javascript, typescript"));
        assert!(rendered.contains("npm install -g typescript typescript-language-server"));
    }

    #[test]
    fn tool_install_report_includes_missing_formatters() {
        let mut settings = Settings::default();
        settings.lsp.enabled = false;
        let languages = LanguagesConfig {
            languages: HashMap::from([(
                "python".to_string(),
                LanguageConfig {
                    formatter: Some(FormatterConfig {
                        command: "black".to_string(),
                        args: vec!["-".to_string()],
                        timeout: 5,
                    }),
                    tab_width: None,
                },
            )]),
        };

        let report = super::collect_tool_install_report(&settings, &languages, |_| false);

        assert_eq!(report.items.len(), 1);
        assert_eq!(report.items[0].category, super::ToolCategory::Formatter);
        assert_eq!(report.items[0].labels, vec!["python"]);
        assert_eq!(report.items[0].command, "black");
        assert!(report.items[0].install_command.is_none());
        assert!(report
            .render_markdown()
            .contains("No known install command"));
    }

    #[test]
    fn tool_install_report_says_when_known_tools_are_installed() {
        let mut settings = Settings::default();
        disable_all_lsp(&mut settings);
        settings.lsp.servers.rust = LspServerConfig {
            enabled: true,
            ..settings.lsp.servers.rust
        };

        let report =
            super::collect_tool_install_report(&settings, &LanguagesConfig::default(), |_| true);

        assert!(report.items.is_empty());
        assert!(report
            .render_markdown()
            .contains("No missing tools with known checks."));
    }
}
