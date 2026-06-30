use std::path::PathBuf;

pub const PROFILE_LOG_PATH: &str = "/tmp/nevi_profile.log";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileMetricSummary {
    pub name: String,
    pub count: u64,
    pub samples: u64,
    pub total_us: u128,
    pub avg_us: u128,
    pub p50_us: u128,
    pub p95_us: u128,
    pub max_us: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileCheckStatus {
    Missing,
    Ok,
    Error(String),
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileLogStatus {
    Missing,
    NoSummary,
    Summary(Vec<ProfileMetricSummary>),
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthReportInput {
    pub config_path: Option<PathBuf>,
    pub config_status: FileCheckStatus,
    pub languages_path: Option<PathBuf>,
    pub languages_status: FileCheckStatus,
    pub keymap: KeymapHealth,
    pub profile_enabled: bool,
    pub profile_log_path: PathBuf,
    pub profile_log_status: ProfileLogStatus,
    pub lsp_enabled: bool,
    pub lsp_servers: Vec<LspServerHealth>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeymapHealth {
    pub leader: String,
    pub timeoutlen: u64,
    pub show_leader_popup: bool,
    pub normal_mappings: Vec<KeymapMappingHealth>,
    pub visual_mappings: Vec<KeymapMappingHealth>,
    pub insert_mappings: Vec<KeymapMappingHealth>,
    pub leader_mapping_count: usize,
    pub command_mapping_count: usize,
    pub explorer_mapping_count: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeymapMappingHealth {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LspServerHealth {
    pub language: &'static str,
    pub enabled: bool,
    pub command: String,
}

pub fn parse_profile_summary(contents: &str) -> Vec<ProfileMetricSummary> {
    let mut in_summary = false;
    let mut metrics = Vec::new();

    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line == "# profile summary" {
            in_summary = true;
            continue;
        }

        if !in_summary || line.is_empty() || line.starts_with('#') {
            continue;
        }

        if let Some(metric) = parse_profile_metric_line(line) {
            metrics.push(metric);
        }
    }

    metrics
}

fn parse_profile_metric_line(line: &str) -> Option<ProfileMetricSummary> {
    let mut parts = line.split_whitespace();
    let name = parts.next()?.to_string();

    let mut count = None;
    let mut samples = None;
    let mut total_us = None;
    let mut avg_us = None;
    let mut p50_us = None;
    let mut p95_us = None;
    let mut max_us = None;

    for part in parts {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };

        match key {
            "count" => count = value.parse::<u64>().ok(),
            "samples" => samples = value.parse::<u64>().ok(),
            "total_us" => total_us = value.parse::<u128>().ok(),
            "avg_us" => avg_us = value.parse::<u128>().ok(),
            "p50_us" => p50_us = value.parse::<u128>().ok(),
            "p95_us" => p95_us = value.parse::<u128>().ok(),
            "max_us" => max_us = value.parse::<u128>().ok(),
            _ => {}
        }
    }

    Some(ProfileMetricSummary {
        name,
        count: count?,
        samples: samples?,
        total_us: total_us?,
        avg_us: avg_us?,
        p50_us: p50_us?,
        p95_us: p95_us?,
        max_us: max_us?,
    })
}

pub fn build_health_report(input: &HealthReportInput) -> String {
    let mut report = String::new();
    report.push_str("# Nevi Health\n\n");

    report.push_str("## Configuration\n");
    report.push_str(&format!(
        "- Configuration file: {} ({})\n",
        file_status_label(&input.config_status),
        path_label(input.config_path.as_ref())
    ));
    report.push_str(&format!(
        "- Languages file: {} ({})\n",
        file_status_label(&input.languages_status),
        path_label(input.languages_path.as_ref())
    ));
    report.push_str("- Open user config: `:ConfigOpen`\n");
    report.push_str("- View latest default config: `:ConfigDefaults`\n\n");

    report.push_str("## Keymaps\n");
    report.push_str(&format!(
        "- Leader key: {}\n",
        keymap_key_label(&input.keymap.leader)
    ));
    report.push_str(&format!(
        "- Leader popup: {}\n",
        if input.keymap.show_leader_popup {
            "enabled"
        } else {
            "disabled"
        }
    ));
    report.push_str(&format!("- Timeout: {}ms\n", input.keymap.timeoutlen));
    write_keymap_remaps(&mut report, "Normal remaps", &input.keymap.normal_mappings);
    write_keymap_remaps(&mut report, "Visual remaps", &input.keymap.visual_mappings);
    write_keymap_remaps(&mut report, "Insert remaps", &input.keymap.insert_mappings);
    report.push_str(&format!(
        "- Leader mappings: {} configured\n",
        input.keymap.leader_mapping_count
    ));
    report.push_str(&format!(
        "- Command mappings: {} configured\n",
        input.keymap.command_mapping_count
    ));
    report.push_str(&format!(
        "- Explorer mappings: {} configured\n",
        input.keymap.explorer_mapping_count
    ));
    if input.keymap.warnings.is_empty() {
        report.push_str("- Warnings: none\n");
    } else {
        report.push_str("- Warnings:\n");
        for warning in &input.keymap.warnings {
            report.push_str(&format!("  - {warning}\n"));
        }
    }
    report.push('\n');

    report.push_str("## Performance\n");
    report.push_str(&format!(
        "- Profiling: {} for this session (`NEVI_PROFILE=1`)\n",
        if input.profile_enabled {
            "enabled"
        } else {
            "disabled"
        }
    ));
    report.push_str(&format!(
        "- Profile log: {}\n",
        input.profile_log_path.display()
    ));
    match &input.profile_log_status {
        ProfileLogStatus::Missing => {
            report.push_str(
                "- Profile summary: missing. Restart with `NEVI_PROFILE=1`, reproduce the issue, quit Nevi so the summary is written, then run `:checkhealth` again.\n",
            );
        }
        ProfileLogStatus::NoSummary => {
            report.push_str(
                "- Profile summary: log exists, but no summary is present yet. The summary is written when Nevi exits.\n",
            );
        }
        ProfileLogStatus::Summary(metrics) => {
            if input.profile_enabled {
                report.push_str("- Profile summary: found\n");
            } else {
                report.push_str("- Profile summary: found from saved log\n");
            }
            for metric in metrics {
                report.push_str(&format!(
                    "- {}: count={} avg={}us p95={}us max={}us\n",
                    metric.name, metric.count, metric.avg_us, metric.p95_us, metric.max_us
                ));
            }
        }
        ProfileLogStatus::Error(error) => {
            report.push_str(&format!("- Profile summary: error ({error})\n"));
        }
    }
    report.push('\n');

    report.push_str("## LSP\n");
    report.push_str(&format!(
        "- LSP: {}\n",
        if input.lsp_enabled {
            "enabled"
        } else {
            "disabled"
        }
    ));
    if input.lsp_servers.is_empty() {
        report.push_str("- Configured servers: none\n");
    } else {
        report.push_str("- Configured servers:\n");
        for server in &input.lsp_servers {
            report.push_str(&format!(
                "  - {}: {} ({})\n",
                server.language,
                if server.enabled {
                    "enabled"
                } else {
                    "disabled"
                },
                if server.command.is_empty() {
                    "<unset>"
                } else {
                    server.command.as_str()
                }
            ));
        }
    }
    report.push_str("- External tool checks: skipped in v1 to keep `:checkhealth` cheap.\n");

    report
}

pub fn collect_health_report(settings: &crate::config::Settings) -> String {
    let config_path = crate::config::config_path();
    let languages_path = crate::config::languages::languages_config_path();
    let profile_log_path = PathBuf::from(PROFILE_LOG_PATH);

    build_health_report(&HealthReportInput {
        config_status: inspect_toml_file::<crate::config::Settings>(config_path.as_ref()),
        languages_status: inspect_toml_file::<crate::config::LanguagesConfig>(
            languages_path.as_ref(),
        ),
        config_path,
        languages_path,
        keymap: keymap_health_from_settings(&settings.keymap),
        profile_enabled: profile_enabled_from_env(),
        profile_log_status: inspect_profile_log(&profile_log_path),
        profile_log_path,
        lsp_enabled: settings.lsp.enabled,
        lsp_servers: lsp_server_health(settings),
    })
}

pub fn keymap_health_from_settings(settings: &crate::config::KeymapSettings) -> KeymapHealth {
    let normal_mappings = keymap_entry_health(&settings.normal);
    let mut warnings = Vec::new();

    for mapping in &normal_mappings {
        if let Some(default) = known_normal_mode_default(&mapping.from) {
            warnings.push(format!(
                "{} overrides Vim default: {}",
                mapping.from, default
            ));
        }
    }

    KeymapHealth {
        leader: settings.leader.clone(),
        timeoutlen: settings.timeoutlen,
        show_leader_popup: settings.show_leader_popup,
        normal_mappings,
        visual_mappings: keymap_entry_health(&settings.visual),
        insert_mappings: keymap_entry_health(&settings.insert),
        leader_mapping_count: settings.leader_mappings.len(),
        command_mapping_count: settings.command_mappings.len(),
        explorer_mapping_count: settings.explorer.len(),
        warnings,
    }
}

fn keymap_entry_health(entries: &[crate::config::KeymapEntry]) -> Vec<KeymapMappingHealth> {
    entries
        .iter()
        .map(|entry| KeymapMappingHealth {
            from: entry.from.clone(),
            to: entry.to.clone(),
        })
        .collect()
}

fn known_normal_mode_default(key: &str) -> Option<&'static str> {
    match key {
        "H" => Some("Move to top of visible screen"),
        "L" => Some("Move to bottom of visible screen"),
        "M" => Some("Move to middle of visible screen"),
        "U" => Some("Undo latest changes on the current line"),
        ";" => Some("Repeat latest f/F/t/T search"),
        "," => Some("Repeat latest f/F/t/T search in reverse"),
        "|" => Some("Go to a screen column"),
        "_" => Some("Move to first non-blank character of a line"),
        "=" => Some("Format with motion"),
        _ => None,
    }
}

fn write_keymap_remaps(report: &mut String, label: &str, mappings: &[KeymapMappingHealth]) {
    if mappings.is_empty() {
        report.push_str(&format!("- {label}: none\n"));
        return;
    }

    report.push_str(&format!("- {label}: {}\n", mappings.len()));
    for mapping in mappings {
        report.push_str(&format!("  - {} -> {}\n", mapping.from, mapping.to));
    }
}

fn keymap_key_label(key: &str) -> String {
    match key {
        " " | "<Space>" | "<space>" => "<Space>".to_string(),
        "" => "<unset>".to_string(),
        other => other.to_string(),
    }
}

pub fn profile_enabled_from_env() -> bool {
    profile_enabled_from_value(std::env::var("NEVI_PROFILE").ok().as_deref())
}

pub fn profile_enabled_from_value(value: Option<&str>) -> bool {
    let Some(value) = value.map(str::trim) else {
        return false;
    };

    value == "1"
        || value.eq_ignore_ascii_case("true")
        || value.eq_ignore_ascii_case("yes")
        || value.eq_ignore_ascii_case("on")
}

fn inspect_toml_file<T>(path: Option<&PathBuf>) -> FileCheckStatus
where
    T: serde::de::DeserializeOwned,
{
    let Some(path) = path else {
        return FileCheckStatus::Unavailable;
    };

    if !path.exists() {
        return FileCheckStatus::Missing;
    }

    match std::fs::read_to_string(path) {
        Ok(contents) => match toml::from_str::<T>(&contents) {
            Ok(_) => FileCheckStatus::Ok,
            Err(error) => FileCheckStatus::Error(error.to_string()),
        },
        Err(error) => FileCheckStatus::Error(error.to_string()),
    }
}

fn inspect_profile_log(path: &PathBuf) -> ProfileLogStatus {
    if !path.exists() {
        return ProfileLogStatus::Missing;
    }

    match std::fs::read_to_string(path) {
        Ok(contents) => {
            let metrics = parse_profile_summary(&contents);
            if metrics.is_empty() {
                ProfileLogStatus::NoSummary
            } else {
                ProfileLogStatus::Summary(metrics)
            }
        }
        Err(error) => ProfileLogStatus::Error(error.to_string()),
    }
}

fn lsp_server_health(settings: &crate::config::Settings) -> Vec<LspServerHealth> {
    let servers = &settings.lsp.servers;
    vec![
        server_health("rust", &servers.rust),
        server_health("typescript", &servers.typescript),
        server_health("javascript", &servers.javascript),
        server_health("css", &servers.css),
        server_health("json", &servers.json),
        server_health("toml", &servers.toml),
        server_health("markdown", &servers.markdown),
        server_health("html", &servers.html),
        server_health("python", &servers.python),
    ]
}

fn server_health(
    language: &'static str,
    config: &crate::config::LspServerConfig,
) -> LspServerHealth {
    LspServerHealth {
        language,
        enabled: config.enabled,
        command: config.effective_command().to_string(),
    }
}

fn file_status_label(status: &FileCheckStatus) -> String {
    match status {
        FileCheckStatus::Missing => "missing".to_string(),
        FileCheckStatus::Ok => "ok".to_string(),
        FileCheckStatus::Error(error) => format!("error: {error}"),
        FileCheckStatus::Unavailable => "unavailable".to_string(),
    }
}

fn path_label(path: Option<&PathBuf>) -> String {
    path.map(|path| path.display().to_string())
        .unwrap_or_else(|| "path unavailable".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{KeymapEntry, Settings};

    fn default_keymap_health() -> KeymapHealth {
        keymap_health_from_settings(&Settings::default().keymap)
    }

    #[test]
    fn parses_profile_summary_after_header() {
        let metrics = parse_profile_summary(
            "render: 1ms\n\
             # profile summary\n\
             handle_key count=97 samples=97 total_us=93125 avg_us=960 p50_us=11 p95_us=4759 max_us=33710\n\
             render count=300 samples=300 total_us=479017 avg_us=1596 p50_us=877 p95_us=3396 max_us=54689\n",
        );

        assert_eq!(
            metrics,
            vec![
                ProfileMetricSummary {
                    name: "handle_key".to_string(),
                    count: 97,
                    samples: 97,
                    total_us: 93_125,
                    avg_us: 960,
                    p50_us: 11,
                    p95_us: 4_759,
                    max_us: 33_710,
                },
                ProfileMetricSummary {
                    name: "render".to_string(),
                    count: 300,
                    samples: 300,
                    total_us: 479_017,
                    avg_us: 1_596,
                    p50_us: 877,
                    p95_us: 3_396,
                    max_us: 54_689,
                },
            ]
        );
    }

    #[test]
    fn health_report_guides_when_profile_summary_is_missing() {
        let report = build_health_report(&HealthReportInput {
            config_path: Some(PathBuf::from("/home/me/.config/nevi/config.toml")),
            config_status: FileCheckStatus::Ok,
            languages_path: Some(PathBuf::from("/home/me/.config/nevi/languages.toml")),
            languages_status: FileCheckStatus::Missing,
            keymap: default_keymap_health(),
            profile_enabled: false,
            profile_log_path: PathBuf::from(PROFILE_LOG_PATH),
            profile_log_status: ProfileLogStatus::Missing,
            lsp_enabled: true,
            lsp_servers: vec![LspServerHealth {
                language: "rust",
                enabled: true,
                command: "rust-analyzer".to_string(),
            }],
        });

        assert!(report.contains("# Nevi Health"));
        assert!(report.contains("/home/me/.config/nevi/config.toml"));
        assert!(report.contains("Configuration file: ok"));
        assert!(report.contains(":ConfigOpen"));
        assert!(report.contains(":ConfigDefaults"));
        assert!(report.contains("Languages file: missing"));
        assert!(report.contains("Profiling: disabled"));
        assert!(report.contains("/tmp/nevi_profile.log"));
        assert!(report.contains("NEVI_PROFILE=1"));
        assert!(report.contains("rust: enabled (rust-analyzer)"));
    }

    #[test]
    fn health_report_lists_profile_summary_metrics() {
        let report = build_health_report(&HealthReportInput {
            config_path: None,
            config_status: FileCheckStatus::Unavailable,
            languages_path: None,
            languages_status: FileCheckStatus::Unavailable,
            keymap: default_keymap_health(),
            profile_enabled: true,
            profile_log_path: PathBuf::from(PROFILE_LOG_PATH),
            profile_log_status: ProfileLogStatus::Summary(vec![ProfileMetricSummary {
                name: "render".to_string(),
                count: 300,
                samples: 300,
                total_us: 479_017,
                avg_us: 1_596,
                p50_us: 877,
                p95_us: 3_396,
                max_us: 54_689,
            }]),
            lsp_enabled: false,
            lsp_servers: Vec::new(),
        });

        assert!(report.contains("Profiling: enabled"));
        assert!(report.contains("render: count=300 avg=1596us p95=3396us max=54689us"));
        assert!(report.contains("LSP: disabled"));
    }

    #[test]
    fn health_report_marks_saved_profile_summary_when_current_session_is_not_profiled() {
        let report = build_health_report(&HealthReportInput {
            config_path: None,
            config_status: FileCheckStatus::Unavailable,
            languages_path: None,
            languages_status: FileCheckStatus::Unavailable,
            keymap: default_keymap_health(),
            profile_enabled: false,
            profile_log_path: PathBuf::from(PROFILE_LOG_PATH),
            profile_log_status: ProfileLogStatus::Summary(vec![ProfileMetricSummary {
                name: "render".to_string(),
                count: 1,
                samples: 1,
                total_us: 42,
                avg_us: 42,
                p50_us: 42,
                p95_us: 42,
                max_us: 42,
            }]),
            lsp_enabled: true,
            lsp_servers: Vec::new(),
        });

        assert!(report.contains("Profiling: disabled for this session"));
        assert!(report.contains("Profile summary: found from saved log"));
    }

    #[test]
    fn health_report_lists_keymap_overrides_and_warnings() {
        let mut settings = Settings::default();
        settings.keymap.normal.push(KeymapEntry {
            from: "H".to_string(),
            to: "^".to_string(),
        });
        settings.keymap.normal.push(KeymapEntry {
            from: "L".to_string(),
            to: "$".to_string(),
        });
        settings.keymap.normal.push(KeymapEntry {
            from: ";".to_string(),
            to: ":".to_string(),
        });

        let report = build_health_report(&HealthReportInput {
            config_path: None,
            config_status: FileCheckStatus::Unavailable,
            languages_path: None,
            languages_status: FileCheckStatus::Unavailable,
            profile_enabled: false,
            profile_log_path: PathBuf::from(PROFILE_LOG_PATH),
            profile_log_status: ProfileLogStatus::Missing,
            lsp_enabled: true,
            lsp_servers: Vec::new(),
            keymap: keymap_health_from_settings(&settings.keymap),
        });

        assert!(report.contains("## Keymaps"));
        assert!(report.contains("Leader key: <Space>"));
        assert!(report.contains("Leader popup: enabled"));
        assert!(report.contains("Normal remaps: 3"));
        assert!(report.contains("H -> ^"));
        assert!(report.contains("L -> $"));
        assert!(report.contains("; -> :"));
        assert!(report.contains("Warnings:"));
        assert!(report.contains("H overrides Vim default: Move to top of visible screen"));
        assert!(report.contains("L overrides Vim default: Move to bottom of visible screen"));
        assert!(report.contains("; overrides Vim default: Repeat latest f/F/t/T search"));
    }
}
