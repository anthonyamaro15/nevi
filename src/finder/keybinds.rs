//! Keybinding cheatsheet data, parsed from the embedded `keybinds.toml`.
//!
//! `src/finder/keybinds.toml` documents Nevi's built-in default keybindings,
//! embedded at compile time so the `:Keymaps` picker ships in the binary. At
//! runtime, commands and leader/remaps are overlaid from live sources.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;

use super::FinderItem;
use crate::config::{KeymapEntry, KeymapSettings, LeaderMapping};

/// Raw entry as stored in KEYBINDS.toml. Mode + category come from the table path.
#[derive(Debug, Clone, Deserialize)]
struct RawKeybind {
    key: String,
    action: String,
    #[serde(default)]
    desc: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    vim_default: bool,
}

/// A single documented keybinding, flattened with its mode and category.
/// Some fields (`action`, `vim_default`) are carried for future introspection
/// features (which-key, keymap doctor) and not read yet.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct KeybindEntry {
    pub mode: String,
    pub category: String,
    pub key: String,
    pub action: String,
    pub desc: String,
    pub status: String,
    pub vim_default: bool,
}

/// A mode's entries are stored one of two ways in KEYBINDS.toml:
/// - `[[mode.category]]` — grouped by category (normal, visual, insert, leader, finder, commands)
/// - `[[mode]]` — a flat array with no category (terminal, text_objects, registers)
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ModeSection {
    /// `[[mode]]` — entries directly under the mode.
    Flat(Vec<RawKeybind>),
    /// `[[mode.category]]` — entries grouped by category.
    Categorized(BTreeMap<String, Vec<RawKeybind>>),
}

/// Parse KEYBINDS.toml text into a flat, deterministically-ordered list.
///
/// Handles both the categorized (`[[mode.category]]`) and flat (`[[mode]]`)
/// table shapes. A parse failure yields an empty list.
pub fn parse_keybinds(raw: &str) -> Vec<KeybindEntry> {
    let parsed: BTreeMap<String, ModeSection> = match toml::from_str(raw) {
        Ok(parsed) => parsed,
        Err(_) => return Vec::new(),
    };

    let mut entries = Vec::new();
    for (mode, section) in parsed {
        match section {
            ModeSection::Flat(binds) => {
                for b in binds {
                    entries.push(make_entry(&mode, "", b));
                }
            }
            ModeSection::Categorized(categories) => {
                for (category, binds) in categories {
                    for b in binds {
                        entries.push(make_entry(&mode, &category, b));
                    }
                }
            }
        }
    }
    entries
}

fn make_entry(mode: &str, category: &str, b: RawKeybind) -> KeybindEntry {
    KeybindEntry {
        mode: mode.to_string(),
        category: category.to_string(),
        key: b.key,
        action: b.action,
        desc: b.desc,
        status: b.status,
        vim_default: b.vim_default,
    }
}

/// Load and parse the embedded KEYBINDS.toml.
pub fn load_keybinds() -> Vec<KeybindEntry> {
    const RAW: &str = include_str!("keybinds.toml");
    parse_keybinds(RAW)
}

/// Short mode tag for the display row (e.g. "normal" -> "n").
/// Provided for you to use inside `display_row`.
fn mode_tag(mode: &str) -> &str {
    match mode {
        "normal" => "n",
        "visual" => "v",
        "insert" => "i",
        "leader" => "leader",
        "commands" => "cmd",
        "cmdline" => "cmdline",
        "explorer" => "expl",
        "finder" => "finder",
        "terminal" => "term",
        "text_objects" => "text-obj",
        "registers" => "reg",
        other => other,
    }
}

/// Format one keybinding as a single row shown in the `:Keymaps` picker.
pub fn display_row(entry: &KeybindEntry) -> String {
    let key = if entry.mode == "leader" {
        format!("<leader>{}", entry.key)
    } else {
        entry.key.clone()
    };
    format!("{:<7} {:<18} {}", mode_tag(&entry.mode), key, entry.desc)
}

/// Build the finder rows for the `:Keymaps` cheatsheet, merging live sources so
/// it reflects the editor's actual behavior:
/// - raw built-in keys from the embedded KEYBINDS.toml (the only place they
///   live), minus any the user has remapped;
/// - the user's own normal/visual/insert remaps, from `keymap`;
/// - command-line UX mappings from `keymap.command_mappings`;
/// - commands from the live `COMMAND_SPECS` registry;
/// - leader bindings from `keymap.leader_mappings` (the active, merged set).
///
/// Everything except the raw built-ins comes from runtime sources, so it never
/// drifts from what the editor does. `keymap` is the editor's active keymap
/// (`settings.keymap`).
pub fn keymap_finder_items(keymap: &KeymapSettings) -> Vec<FinderItem> {
    // Drop built-in rows the user has remapped; show their remap instead.
    let mut entries: Vec<KeybindEntry> = raw_key_entries()
        .into_iter()
        .filter(|entry| !is_remapped(keymap, &entry.mode, &entry.key))
        .collect();
    entries.extend(remap_entries(keymap));
    entries.extend(command_mode_entries(keymap));
    entries.extend(command_entries());
    entries.extend(leader_entries(&keymap.leader_mappings));
    entries.iter().map(entry_to_item).collect()
}

/// True if the user has remapped `key` in the given mode, so the built-in
/// default row should be replaced by the user's remap.
fn is_remapped(keymap: &KeymapSettings, mode: &str, key: &str) -> bool {
    match mode {
        "normal" => keymap.normal.iter().any(|entry| entry.from == key),
        "visual" => keymap.visual.iter().any(|entry| entry.from == key),
        "insert" => keymap.insert.iter().any(|entry| entry.from == key),
        "cmdline" => keymap.command_mappings.iter().any(|entry| entry.key == key),
        _ => false,
    }
}

/// Rows for the user's own normal/visual/insert remaps, from the live config.
fn remap_entries(keymap: &KeymapSettings) -> Vec<KeybindEntry> {
    let sources: [(&str, &Vec<KeymapEntry>); 3] = [
        ("normal", &keymap.normal),
        ("visual", &keymap.visual),
        ("insert", &keymap.insert),
    ];
    let mut entries = Vec::new();
    for (mode, remaps) in sources {
        for remap in remaps {
            entries.push(KeybindEntry {
                mode: mode.to_string(),
                category: String::new(),
                key: remap.from.clone(),
                action: remap.to.clone(),
                desc: format!("remapped to {}", remap.to),
                status: "implemented".to_string(),
                vim_default: false,
            });
        }
    }
    entries
}

/// Implemented raw-key entries from the embedded file, excluding the `commands`
/// and `leader` sections — those are now sourced live (registry + config) to
/// avoid drift and duplication.
fn raw_key_entries() -> Vec<KeybindEntry> {
    load_keybinds()
        .into_iter()
        .filter(|entry| entry.status == "implemented")
        .filter(|entry| entry.mode != "commands" && entry.mode != "leader")
        .collect()
}

/// Command rows from the live `COMMAND_SPECS` registry.
fn command_entries() -> Vec<KeybindEntry> {
    crate::commands::command_cheatsheet_rows()
        .into_iter()
        .map(|(name, desc)| KeybindEntry {
            mode: "commands".to_string(),
            category: String::new(),
            key: name,
            action: String::new(),
            desc,
            status: "implemented".to_string(),
            vim_default: false,
        })
        .collect()
}

/// Leader rows from the live keymap config. Falls back to the action string when
/// a mapping carries no description (e.g. a user-defined one).
fn leader_entries(leader_mappings: &[LeaderMapping]) -> Vec<KeybindEntry> {
    leader_mappings
        .iter()
        .map(|mapping| {
            let desc = mapping
                .desc
                .clone()
                .unwrap_or_else(|| mapping.action.clone());
            KeybindEntry {
                mode: "leader".to_string(),
                category: String::new(),
                key: mapping.key.clone(),
                action: mapping.action.clone(),
                desc,
                status: "implemented".to_string(),
                vim_default: false,
            }
        })
        .collect()
}

/// Rows for the command-line UX mappings (keys active while typing a `:`
/// command, e.g. `<A-r>` for history), from the live config.
fn command_mode_entries(keymap: &KeymapSettings) -> Vec<KeybindEntry> {
    keymap
        .command_mappings
        .iter()
        .map(|mapping| {
            let desc = mapping
                .desc
                .clone()
                .unwrap_or_else(|| mapping.action.clone());
            KeybindEntry {
                mode: "cmdline".to_string(),
                category: String::new(),
                key: mapping.key.clone(),
                action: mapping.action.clone(),
                desc,
                status: "implemented".to_string(),
                vim_default: false,
            }
        })
        .collect()
}

/// Render one entry as an inert finder item (empty path, no metadata) so Enter
/// has nothing to act on.
fn entry_to_item(entry: &KeybindEntry) -> FinderItem {
    FinderItem::new(display_row(entry), PathBuf::new()).with_icon("  ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[[normal.movement]]
key = "h"
action = "move_left"
desc = "Move cursor left"
status = "implemented"
vim_default = true

[[leader.files]]
key = "ff"
action = ":FindFiles<CR>"
desc = "Find files"
status = "implemented"

[[normal.lsp]]
key = "gD"
action = "goto_declaration"
desc = "Go to declaration"
status = "planned"
vim_default = true
"#;

    #[test]
    fn parses_mode_and_category_from_table_path() {
        let entries = parse_keybinds(SAMPLE);
        assert_eq!(entries.len(), 3);
        let h = entries.iter().find(|e| e.key == "h").expect("h entry");
        assert_eq!(h.mode, "normal");
        assert_eq!(h.category, "movement");
        assert_eq!(h.desc, "Move cursor left");
        assert_eq!(h.status, "implemented");
        assert!(h.vim_default);
    }

    #[test]
    fn picker_sources_commands_from_registry() {
        let items = keymap_finder_items(&KeymapSettings::default());
        let gitchanges = items
            .iter()
            .filter(|item| item.display.contains(":GitChanges"))
            .count();
        assert_eq!(
            gitchanges, 1,
            "exactly one :GitChanges row, from the registry"
        );
        // FindFiles is in both the registry and the (now-excluded) TOML commands
        // section — it must appear exactly once, proving no duplication.
        let findfiles = items
            .iter()
            .filter(|item| item.display.contains(":FindFiles"))
            .count();
        assert_eq!(
            findfiles, 1,
            ":FindFiles should appear once (registry only)"
        );
    }

    #[test]
    fn picker_sources_leader_from_config() {
        let mut keymap = KeymapSettings::default();
        keymap.leader_mappings = vec![LeaderMapping {
            key: "gc".to_string(),
            action: ":GitChanges".to_string(),
            desc: Some("Git changes picker".to_string()),
        }];
        let items = keymap_finder_items(&keymap);
        assert!(
            items.iter().any(|item| item.display.contains("<leader>gc")),
            "leader rows should come from the live config"
        );
    }

    #[test]
    fn picker_overlays_user_normal_remaps() {
        let mut keymap = KeymapSettings::default();
        keymap.normal = vec![KeymapEntry {
            from: "H".to_string(),
            to: "^".to_string(),
        }];
        let items = keymap_finder_items(&keymap);
        assert!(
            items
                .iter()
                .any(|item| item.display.contains("remapped to ^")),
            "the user's H -> ^ remap should appear in the cheatsheet"
        );
    }

    #[test]
    fn picker_sources_command_mode_mappings() {
        let items = keymap_finder_items(&KeymapSettings::default());
        assert!(
            items
                .iter()
                .any(|item| item.display.contains("<C-r>") && item.display.contains("register")),
            "command-line UX mappings (e.g. <C-r> register insertion) should appear"
        );
        assert!(
            items
                .iter()
                .any(|item| item.display.contains("<A-r>") && item.display.contains("history")),
            "command-line UX mappings (e.g. <A-r> history) should appear"
        );
        assert!(
            items.iter().any(|item| {
                item.display.contains("<C-d>") && item.display.contains("completions")
            }),
            "command-line UX mappings (e.g. <C-d> completions) should appear"
        );
        assert!(
            items.iter().any(|item| {
                item.display.contains("<C-l>") && item.display.contains("common command prefix")
            }),
            "command-line UX mappings (e.g. <C-l> common prefix completion) should appear"
        );
        assert!(
            items.iter().any(|item| {
                item.display.contains("<C-a>") && item.display.contains("all matching")
            }),
            "command-line UX mappings (e.g. <C-a> all completions) should appear"
        );
        assert!(
            items.iter().any(|item| {
                item.display.contains("<C-f>") && item.display.contains("command-line window")
            }),
            "command-line UX mappings (e.g. <C-f> command-line window) should appear"
        );
    }

    #[test]
    fn picker_sources_explorer_mode_mappings() {
        let items = keymap_finder_items(&KeymapSettings::default());
        assert!(
            items.iter().any(|item| {
                item.display.contains("expl")
                    && item.display.contains("gg")
                    && item.display.contains("top")
            }),
            "explorer mappings (e.g. gg top) should appear"
        );
        assert!(
            items.iter().any(|item| {
                item.display.contains("expl")
                    && item.display.contains("<C-d>")
                    && item.display.contains("half page")
            }),
            "explorer mappings (e.g. Ctrl+d half page) should appear"
        );
        assert!(
            items.iter().any(|item| {
                item.display.contains("expl")
                    && item.display.contains("p")
                    && item.display.contains("Paste")
            }),
            "existing explorer mappings (e.g. paste) should appear"
        );
        assert!(
            items.iter().any(|item| {
                item.display.contains("expl")
                    && item.display.contains("?")
                    && item.display.contains("keymaps")
            }),
            "explorer help mapping should appear"
        );
    }

    #[test]
    fn picker_includes_builtin_command_line_editing_keys() {
        let items = keymap_finder_items(&KeymapSettings::default());
        assert!(
            items.iter().any(|item| {
                item.display.contains("cmdline")
                    && item.display.contains("<C-b>")
                    && item.display.contains("beginning")
            }),
            "command-line Ctrl+b should appear in :Keymaps"
        );
        assert!(
            items.iter().any(|item| {
                item.display.contains("cmdline")
                    && item.display.contains("<C-e>")
                    && item.display.contains("end")
            }),
            "command-line Ctrl+e should appear in :Keymaps"
        );
        assert!(
            items.iter().any(|item| {
                item.display.contains("cmdline")
                    && item.display.contains("<C-w>")
                    && item.display.contains("Delete word")
            }),
            "command-line Ctrl+w should appear in :Keymaps"
        );
        assert!(
            items.iter().any(|item| {
                item.display.contains("cmdline")
                    && item.display.contains("<C-u>")
                    && item.display.contains("beginning")
            }),
            "command-line Ctrl+u should appear in :Keymaps"
        );
    }

    #[test]
    fn picker_includes_leader_popup_trigger() {
        let items = keymap_finder_items(&KeymapSettings::default());
        assert!(
            items.iter().any(|item| {
                item.display.contains("<leader>") && item.display.contains("Show leader key popup")
            }),
            "leader popup trigger should appear in :Keymaps"
        );
    }

    #[test]
    fn leader_rows_get_leader_prefix() {
        let entries = parse_keybinds(SAMPLE);
        let ff = entries.iter().find(|e| e.key == "ff").unwrap();
        let row = display_row(ff);
        assert!(row.contains("<leader>ff"), "row was: {}", row);
    }

    #[test]
    fn embedded_file_parses_and_is_fully_described() {
        let entries = load_keybinds();
        assert!(
            entries.len() > 250,
            "expected a substantial keymap, got {}",
            entries.len()
        );
        assert!(
            entries.iter().all(|e| !e.desc.is_empty()),
            "every entry needs a description"
        );
        assert!(entries.iter().any(|e| e.status == "implemented"));
    }
}
