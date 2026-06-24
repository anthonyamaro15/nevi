//! Keymap parsing and lookup for custom key remappings

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::{BTreeMap, HashMap};

use super::KeymapSettings;

/// Action to execute from a leader mapping
#[derive(Debug, Clone)]
pub enum LeaderAction {
    /// Execute a command (e.g., ":w", ":q", ":wq")
    Command(String),
    /// Execute a key sequence (for motions, etc.)
    Keys(Vec<KeyEvent>),
}

/// Metadata for one visible leader-key continuation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaderHint {
    /// Next key to press from the current prefix.
    pub key: String,
    /// Full sequence after the leader key.
    pub sequence: String,
    /// Description shown in leader popup UIs.
    pub description: String,
    /// Whether this sequence executes a mapping directly.
    pub is_exact: bool,
    /// Whether this sequence has longer continuations.
    pub has_children: bool,
}

#[derive(Debug, Clone)]
struct LeaderMetadata {
    action: String,
    desc: Option<String>,
}

/// Action for command mode (`:` prompt) keybindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandModeAction {
    /// Toggle history popup window
    HistoryToggle,
    /// Insert the next typed register into the command line
    InsertRegister,
    /// Accept current completion/history selection
    Complete,
    /// Move selection backward and accept completion
    CompletePrev,
    /// Move popup selection to next item
    PopupNext,
    /// Move popup selection to previous item
    PopupPrev,
}

/// Lookup table for custom key remappings
#[derive(Debug, Clone)]
pub struct KeymapLookup {
    /// Normal mode remappings: from key -> action (can be single key or command)
    normal: HashMap<KeyEvent, LeaderAction>,
    /// Visual mode remappings: from key -> action (can be single key or command)
    visual: HashMap<KeyEvent, LeaderAction>,
    /// Insert mode remappings: from -> to (single key only)
    insert: HashMap<KeyEvent, KeyEvent>,
    /// Command mode mappings: key -> command-line UX action
    command: HashMap<KeyEvent, CommandModeAction>,
    /// Leader key (None if not configured)
    leader_key: Option<KeyEvent>,
    /// Leader mappings: key sequence -> action
    leader_mappings: HashMap<String, LeaderAction>,
    /// Leader mapping metadata used for discoverability UI.
    leader_metadata: HashMap<String, LeaderMetadata>,
}

impl Default for KeymapLookup {
    fn default() -> Self {
        Self {
            normal: HashMap::new(),
            visual: HashMap::new(),
            insert: HashMap::new(),
            command: HashMap::new(),
            leader_key: None,
            leader_mappings: HashMap::new(),
            leader_metadata: HashMap::new(),
        }
    }
}

impl KeymapLookup {
    /// Build lookup tables from settings, returning any parse errors
    pub fn from_settings(settings: &KeymapSettings) -> (Self, Vec<String>) {
        let mut normal = HashMap::new();
        let mut visual = HashMap::new();
        let mut insert = HashMap::new();
        let mut command = HashMap::new();
        let mut errors = Vec::new();

        for entry in &settings.normal {
            if let Some(from) = parse_key_notation(&entry.from) {
                // Parse the 'to' field as an action (command or keys)
                let action = parse_action(&entry.to);
                normal.insert(from, action);
            } else {
                errors.push(format!("Keymap: invalid normal mode key '{}'", entry.from));
            }
        }

        for entry in &settings.visual {
            if let Some(from) = parse_key_notation(&entry.from) {
                let action = parse_action(&entry.to);
                visual.insert(from, action);
            } else {
                errors.push(format!("Keymap: invalid visual mode key '{}'", entry.from));
            }
        }

        for entry in &settings.insert {
            match (
                parse_key_notation(&entry.from),
                parse_key_notation(&entry.to),
            ) {
                (Some(from), Some(to)) => {
                    insert.insert(from, to);
                }
                (None, _) => {
                    errors.push(format!(
                        "Keymap: invalid insert mode 'from' key '{}'",
                        entry.from
                    ));
                }
                (_, None) => {
                    errors.push(format!(
                        "Keymap: invalid insert mode 'to' key '{}'",
                        entry.to
                    ));
                }
            }
        }

        for mapping in &settings.command_mappings {
            match (
                parse_key_notation(&mapping.key),
                parse_command_mode_action(&mapping.action),
            ) {
                (Some(key), Some(action)) => {
                    command.insert(key, action);
                }
                (None, _) => {
                    errors.push(format!(
                        "Keymap: invalid command mode key '{}'",
                        mapping.key
                    ));
                }
                (_, None) => {
                    errors.push(format!(
                        "Keymap: invalid command mode action '{}'",
                        mapping.action
                    ));
                }
            }
        }

        // Ensure every command mode action has a fallback default binding.
        // This prevents accidental loss when a user provides an invalid override key.
        for mapping in KeymapSettings::default().command_mappings {
            if let (Some(key), Some(action)) = (
                parse_key_notation(&mapping.key),
                parse_command_mode_action(&mapping.action),
            ) {
                if !command.values().any(|existing| *existing == action) {
                    command.insert(key, action);
                }
            }
        }

        // Parse leader key
        let leader_key = parse_key_notation(&settings.leader);
        if leader_key.is_none() && !settings.leader.is_empty() {
            errors.push(format!("Keymap: invalid leader key '{}'", settings.leader));
        }

        // Parse leader mappings
        let mut leader_mappings = HashMap::new();
        let mut leader_metadata = HashMap::new();
        for mapping in &settings.leader_mappings {
            let action = parse_action(&mapping.action);
            leader_mappings.insert(mapping.key.clone(), action);
            leader_metadata.insert(
                mapping.key.clone(),
                LeaderMetadata {
                    action: mapping.action.clone(),
                    desc: mapping.desc.clone(),
                },
            );
        }

        (
            Self {
                normal,
                visual,
                insert,
                command,
                leader_key,
                leader_mappings,
                leader_metadata,
            },
            errors,
        )
    }

    /// Get the normal mode mapping for a key, if one exists
    pub fn get_normal_mapping(&self, key: KeyEvent) -> Option<&LeaderAction> {
        self.normal.get(&key)
    }

    /// Get the visual mode mapping for a key, if one exists
    pub fn get_visual_mapping(&self, key: KeyEvent) -> Option<&LeaderAction> {
        self.visual.get(&key)
    }

    /// Remap a key in insert mode, returning the original if no mapping exists
    pub fn remap_insert(&self, key: KeyEvent) -> KeyEvent {
        self.insert.get(&key).copied().unwrap_or(key)
    }

    /// Look up a command-mode mapping for command-line UX actions.
    pub fn get_command_action(&self, key: KeyEvent) -> Option<CommandModeAction> {
        self.command.get(&key).copied()
    }

    /// Check if there are any normal mode mappings
    pub fn has_normal_mappings(&self) -> bool {
        !self.normal.is_empty()
    }

    /// Check if there are any visual mode mappings
    pub fn has_visual_mappings(&self) -> bool {
        !self.visual.is_empty()
    }

    /// Check if there are any insert mode mappings
    pub fn has_insert_mappings(&self) -> bool {
        !self.insert.is_empty()
    }

    /// Check if the given key is the leader key
    pub fn is_leader_key(&self, key: KeyEvent) -> bool {
        self.leader_key.map_or(false, |leader| {
            // Normalize the comparison - ignore extra modifiers from crossterm
            key.code == leader.code && key.modifiers.contains(leader.modifiers)
        })
    }

    /// Check if there are any leader mappings
    pub fn has_leader_mappings(&self) -> bool {
        !self.leader_mappings.is_empty()
    }

    /// Look up a leader mapping by key sequence
    pub fn get_leader_action(&self, sequence: &str) -> Option<&LeaderAction> {
        self.leader_mappings.get(sequence)
    }

    /// Check if a sequence could be a prefix for a leader mapping
    /// Returns true if there's any mapping that starts with this sequence
    pub fn is_leader_prefix(&self, sequence: &str) -> bool {
        self.leader_mappings
            .keys()
            .any(|k| k.starts_with(sequence) && k != sequence)
    }

    /// Return available leader continuations for the current prefix.
    pub fn leader_hints(&self, prefix: &str) -> Vec<LeaderHint> {
        let mut hints = BTreeMap::new();

        for sequence in self.leader_mappings.keys() {
            let Some(remainder) = sequence.strip_prefix(prefix) else {
                continue;
            };
            if remainder.is_empty() {
                continue;
            }

            let Some(next_key) = remainder.chars().next() else {
                continue;
            };
            let key = next_key.to_string();
            let candidate = format!("{}{}", prefix, key);
            let is_exact = self.leader_mappings.contains_key(&candidate);
            let has_children = self.is_leader_prefix(&candidate);
            let description = self.leader_hint_description(&candidate, is_exact, has_children);

            hints.entry(key.clone()).or_insert(LeaderHint {
                key,
                sequence: candidate,
                description,
                is_exact,
                has_children,
            });
        }

        hints.into_values().collect()
    }

    fn leader_hint_description(
        &self,
        sequence: &str,
        is_exact: bool,
        has_children: bool,
    ) -> String {
        if is_exact {
            if let Some(metadata) = self.leader_metadata.get(sequence) {
                if let Some(desc) = metadata.desc.as_ref().filter(|desc| !desc.is_empty()) {
                    return desc.clone();
                }
                return metadata.action.clone();
            }
        }

        if has_children {
            "prefix".to_string()
        } else {
            String::new()
        }
    }
}

/// Parse an action string into a LeaderAction
fn parse_action(action: &str) -> LeaderAction {
    // If it starts with ':', it's a command
    if action.starts_with(':') {
        // Strip the leading ':' and trailing <CR> if present
        let cmd = action.trim_start_matches(':');
        let cmd = if cmd.to_lowercase().ends_with("<cr>") {
            &cmd[..cmd.len() - 4]
        } else {
            cmd
        };
        LeaderAction::Command(cmd.to_string())
    } else {
        // Otherwise, parse as key sequence
        let mut keys = Vec::new();
        let mut remaining = action;

        while !remaining.is_empty() {
            if remaining.starts_with('<') {
                // Find the closing >
                if let Some(end) = remaining.find('>') {
                    let notation = &remaining[..=end];
                    if let Some(key) = parse_key_notation(notation) {
                        keys.push(key);
                    }
                    remaining = &remaining[end + 1..];
                } else {
                    break;
                }
            } else {
                // Single character
                let c = remaining.chars().next().unwrap();
                if let Some(key) = parse_key_notation(&c.to_string()) {
                    keys.push(key);
                }
                remaining = &remaining[c.len_utf8()..];
            }
        }

        LeaderAction::Keys(keys)
    }
}

fn parse_command_mode_action(action: &str) -> Option<CommandModeAction> {
    match action.trim().to_lowercase().as_str() {
        "history_toggle" => Some(CommandModeAction::HistoryToggle),
        "insert_register" => Some(CommandModeAction::InsertRegister),
        "complete" => Some(CommandModeAction::Complete),
        "complete_prev" => Some(CommandModeAction::CompletePrev),
        "popup_next" => Some(CommandModeAction::PopupNext),
        "popup_prev" => Some(CommandModeAction::PopupPrev),
        _ => None,
    }
}

/// Parse a key notation string into a KeyEvent
///
/// Supported formats:
/// - Single characters: "a", "H", ";", "0"
/// - Control keys: "<C-r>", "<C-s>"
/// - Special keys: "<CR>", "<Esc>", "<Tab>", "<BackTab>", "<BS>", "<Space>"
/// - Function keys: "<F1>" through "<F12>"
pub fn parse_key_notation(s: &str) -> Option<KeyEvent> {
    // Handle single space character before trimming (since space is a valid leader key)
    if s == " " {
        return Some(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    }

    let s = s.trim();

    if s.is_empty() {
        return None;
    }

    // Handle special notation <...>
    if s.starts_with('<') && s.ends_with('>') {
        let inner = &s[1..s.len() - 1];
        return parse_special_notation(inner);
    }

    // Handle single character
    if s.chars().count() == 1 {
        let c = s.chars().next()?;
        return Some(char_to_key_event(c));
    }

    None
}

/// Parse special notation (content inside < >)
fn parse_special_notation(inner: &str) -> Option<KeyEvent> {
    let parts = inner.split('-').collect::<Vec<_>>();
    let (modifier_parts, key_part) = parts.split_at(parts.len().saturating_sub(1));
    let key_part = key_part.first().copied()?;
    let mut modifiers = KeyModifiers::NONE;

    for modifier in modifier_parts {
        match modifier.to_lowercase().as_str() {
            "c" | "ctrl" | "control" => modifiers |= KeyModifiers::CONTROL,
            "a" | "alt" | "m" | "meta" => modifiers |= KeyModifiers::ALT,
            "s" | "shift" => modifiers |= KeyModifiers::SHIFT,
            _ => return None,
        }
    }

    let code = parse_special_key_code(key_part, modifiers.contains(KeyModifiers::SHIFT))?;
    if matches!(code, KeyCode::BackTab) {
        modifiers |= KeyModifiers::SHIFT;
    }

    Some(KeyEvent::new(code, modifiers))
}

fn parse_special_key_code(key: &str, shifted: bool) -> Option<KeyCode> {
    let key_lower = key.to_lowercase();

    if key_lower.starts_with('f') {
        if let Ok(n) = key[1..].parse::<u8>() {
            if (1..=12).contains(&n) {
                return Some(KeyCode::F(n));
            }
        }
    }

    match key_lower.as_str() {
        "cr" | "enter" | "return" => Some(KeyCode::Enter),
        "esc" | "escape" => Some(KeyCode::Esc),
        "tab" if shifted => Some(KeyCode::BackTab),
        "tab" => Some(KeyCode::Tab),
        "backtab" => Some(KeyCode::BackTab),
        "bs" | "backspace" => Some(KeyCode::Backspace),
        "del" | "delete" => Some(KeyCode::Delete),
        "space" => Some(KeyCode::Char(' ')),
        "up" => Some(KeyCode::Up),
        "down" => Some(KeyCode::Down),
        "left" => Some(KeyCode::Left),
        "right" => Some(KeyCode::Right),
        "home" => Some(KeyCode::Home),
        "end" => Some(KeyCode::End),
        "pageup" => Some(KeyCode::PageUp),
        "pagedown" => Some(KeyCode::PageDown),
        "insert" => Some(KeyCode::Insert),
        _ if key.chars().count() == 1 => {
            let ch = key.chars().next()?;
            let ch = if shifted {
                ch.to_ascii_uppercase()
            } else {
                ch.to_ascii_lowercase()
            };
            Some(KeyCode::Char(ch))
        }
        _ => None,
    }
}

/// Convert a single character to a KeyEvent
fn char_to_key_event(c: char) -> KeyEvent {
    // Uppercase letters need SHIFT modifier for proper matching
    if c.is_ascii_uppercase() {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT)
    } else {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_single_char() {
        let key = parse_key_notation("a").unwrap();
        assert_eq!(key.code, KeyCode::Char('a'));
        assert_eq!(key.modifiers, KeyModifiers::NONE);

        let key = parse_key_notation("H").unwrap();
        assert_eq!(key.code, KeyCode::Char('H'));
        assert_eq!(key.modifiers, KeyModifiers::SHIFT);

        let key = parse_key_notation(";").unwrap();
        assert_eq!(key.code, KeyCode::Char(';'));
        assert_eq!(key.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn test_parse_control() {
        let key = parse_key_notation("<C-r>").unwrap();
        assert_eq!(key.code, KeyCode::Char('r'));
        assert_eq!(key.modifiers, KeyModifiers::CONTROL);
    }

    #[test]
    fn test_parse_combined_modifiers_for_terminal_shortcuts() {
        let key = parse_key_notation("<C-S-t>").unwrap();
        assert_eq!(key.code, KeyCode::Char('T'));
        assert_eq!(key.modifiers, KeyModifiers::CONTROL | KeyModifiers::SHIFT);

        let key = parse_key_notation("<C-Tab>").unwrap();
        assert_eq!(key.code, KeyCode::Tab);
        assert_eq!(key.modifiers, KeyModifiers::CONTROL);

        let key = parse_key_notation("<C-S-Tab>").unwrap();
        assert_eq!(key.code, KeyCode::BackTab);
        assert_eq!(key.modifiers, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    }

    #[test]
    fn test_parse_special() {
        let key = parse_key_notation("<CR>").unwrap();
        assert_eq!(key.code, KeyCode::Enter);

        let key = parse_key_notation("<Esc>").unwrap();
        assert_eq!(key.code, KeyCode::Esc);

        let key = parse_key_notation("<Tab>").unwrap();
        assert_eq!(key.code, KeyCode::Tab);

        let key = parse_key_notation("<BackTab>").unwrap();
        assert_eq!(key.code, KeyCode::BackTab);

        let key = parse_key_notation("<Space>").unwrap();
        assert_eq!(key.code, KeyCode::Char(' '));
        assert_eq!(key.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn test_parse_literal_space() {
        // Test that a literal space " " parses correctly as a leader key
        let key = parse_key_notation(" ").unwrap();
        assert_eq!(key.code, KeyCode::Char(' '));
        assert_eq!(key.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn test_leader_key_with_literal_space() {
        // This tests the default config where leader = " " (literal space)
        use super::super::KeymapSettings;

        let settings = KeymapSettings {
            leader: " ".to_string(), // Literal space, as used in default config
            timeoutlen: 1000,
            show_leader_popup: true,
            normal: vec![],
            visual: vec![],
            insert: vec![],
            command_mappings: vec![],
            leader_mappings: vec![super::super::LeaderMapping {
                key: "m".to_string(),
                action: ":HarpoonAdd".to_string(),
                desc: Some("Add to harpoon".to_string()),
            }],
        };

        let (lookup, errors) = KeymapLookup::from_settings(&settings);
        assert!(errors.is_empty(), "Should have no errors");
        let space_key = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);

        assert!(lookup.has_leader_mappings(), "Should have leader mappings");
        assert!(
            lookup.is_leader_key(space_key),
            "Literal space should be recognized as leader key"
        );

        // Check we can look up the mapping
        let action = lookup.get_leader_action("m");
        assert!(action.is_some(), "Should find 'm' mapping");
    }

    #[test]
    fn test_leader_key_matching() {
        use super::super::KeymapSettings;

        let settings = KeymapSettings {
            leader: "<Space>".to_string(),
            timeoutlen: 1000,
            show_leader_popup: true,
            normal: vec![],
            visual: vec![],
            insert: vec![],
            command_mappings: vec![],
            leader_mappings: vec![super::super::LeaderMapping {
                key: "w".to_string(),
                action: ":w<CR>".to_string(),
                desc: Some("Save".to_string()),
            }],
        };

        let (lookup, errors) = KeymapLookup::from_settings(&settings);
        assert!(errors.is_empty(), "Should have no errors");

        // Simulate a space key press from crossterm
        let space_key = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);

        assert!(lookup.has_leader_mappings(), "Should have leader mappings");
        assert!(
            lookup.is_leader_key(space_key),
            "Space should be recognized as leader key"
        );

        // Check we can look up the mapping
        let action = lookup.get_leader_action("w");
        assert!(action.is_some(), "Should find 'w' mapping");
    }

    #[test]
    fn default_keymap_includes_documented_leader_mappings() {
        use super::super::KeymapSettings;

        let (lookup, errors) = KeymapLookup::from_settings(&KeymapSettings::default());

        assert!(errors.is_empty(), "Should have no errors");
        let expected = [
            ("ca", "codeaction"),
            ("rn", "rn"),
            ("w", "w"),
            ("q", "q"),
            ("e", "Explorer"),
            ("ff", "FindFiles"),
            ("fg", "LiveGrep"),
            ("sw", "SearchWord"),
            ("fb", "FindBuffers"),
            ("ft", "Themes"),
            ("tt", "Terminals"),
            ("tn", "TerminalNew"),
            ("tj", "TerminalNext"),
            ("tk", "TerminalPrev"),
            ("tr", "TerminalRename"),
            ("tx", "TerminalKill"),
            ("t1", "TerminalSelect 1"),
            ("t2", "TerminalSelect 2"),
            ("t3", "TerminalSelect 3"),
            ("t4", "TerminalSelect 4"),
            ("d", "FindDiagnostics"),
            ("D", "DiagnosticFloat"),
            ("gg", "LazyGit"),
            ("gc", "GitChanges"),
            ("m", "HarpoonAdd"),
            ("h", "HarpoonMenu"),
            ("1", "Harpoon1"),
            ("2", "Harpoon2"),
            ("3", "Harpoon3"),
            ("4", "Harpoon4"),
        ];

        for (sequence, expected_command) in expected {
            match lookup.get_leader_action(sequence) {
                Some(LeaderAction::Command(command)) => assert_eq!(command, expected_command),
                other => panic!(
                    "Expected <leader>{} to run :{}, got {:?}",
                    sequence, expected_command, other
                ),
            }
        }

        assert!(lookup.is_leader_prefix("f"));
        assert!(lookup.is_leader_prefix("t"));
        assert!(lookup.is_leader_prefix("g"));
    }

    #[test]
    fn leader_hints_show_available_continuations_for_prefix() {
        use super::super::KeymapSettings;

        let (lookup, errors) = KeymapLookup::from_settings(&KeymapSettings::default());
        assert!(errors.is_empty(), "Should have no errors");

        let root_hints = lookup.leader_hints("");
        let files_group = root_hints
            .iter()
            .find(|hint| hint.key == "f")
            .expect("expected <leader>f group");
        assert_eq!(files_group.sequence, "f");
        assert!(files_group.has_children);
        assert!(!files_group.is_exact);

        let save = root_hints
            .iter()
            .find(|hint| hint.key == "w")
            .expect("expected <leader>w");
        assert_eq!(save.sequence, "w");
        assert_eq!(save.description, "Save file");
        assert!(save.is_exact);
        assert!(!save.has_children);

        let file_hints = lookup.leader_hints("f");
        assert!(file_hints.iter().any(|hint| {
            hint.key == "f" && hint.sequence == "ff" && hint.description == "Find files"
        }));
        assert!(file_hints.iter().any(|hint| {
            hint.key == "g" && hint.sequence == "fg" && hint.description == "Live grep"
        }));
        assert!(file_hints.iter().any(|hint| {
            hint.key == "k" && hint.sequence == "fk" && hint.description == "Search keymaps"
        }));
    }

    #[test]
    fn test_visual_mapping_parses() {
        use super::super::{KeymapEntry, KeymapSettings};

        let mut settings = KeymapSettings::default();
        settings.visual = vec![KeymapEntry {
            from: "s".to_string(),
            to: "S".to_string(),
        }];

        let (lookup, errors) = KeymapLookup::from_settings(&settings);
        assert!(errors.is_empty(), "Should have no errors");
        assert!(lookup.has_visual_mappings(), "Should have visual mappings");
        assert!(lookup.get_visual_mapping(char_to_key_event('s')).is_some());
    }

    #[test]
    fn test_parse_function() {
        let key = parse_key_notation("<F1>").unwrap();
        assert_eq!(key.code, KeyCode::F(1));

        let key = parse_key_notation("<F12>").unwrap();
        assert_eq!(key.code, KeyCode::F(12));
    }

    #[test]
    fn default_leader_includes_keymaps_picker() {
        use super::super::KeymapSettings;
        let settings = KeymapSettings::default();
        assert!(
            settings
                .leader_mappings
                .iter()
                .any(|m| m.key == "fk" && m.action.contains("Keymaps")),
            "default leader mappings should bind fk -> :Keymaps"
        );
    }
}
