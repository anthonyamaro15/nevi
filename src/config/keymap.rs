//! Keymap parsing and lookup for custom key remappings

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::HashMap;

use super::KeymapSettings;

/// Action to execute from a leader mapping
#[derive(Debug, Clone)]
pub enum LeaderAction {
    /// Execute a command (e.g., ":w", ":q", ":wq")
    Command(String),
    /// Execute a key sequence (for motions, etc.)
    Keys(Vec<KeyEvent>),
}

/// Lookup table for custom key remappings
#[derive(Debug, Clone)]
pub struct KeymapLookup {
    /// Normal mode remappings: from key -> action (can be single key or command)
    normal: HashMap<KeyEvent, LeaderAction>,
    /// Insert mode remappings: from -> to (single key only)
    insert: HashMap<KeyEvent, KeyEvent>,
    /// Leader key (None if not configured)
    leader_key: Option<KeyEvent>,
    /// Leader mappings: key sequence -> action
    leader_mappings: HashMap<String, LeaderAction>,
}

impl Default for KeymapLookup {
    fn default() -> Self {
        Self {
            normal: HashMap::new(),
            insert: HashMap::new(),
            leader_key: None,
            leader_mappings: HashMap::new(),
        }
    }
}

impl KeymapLookup {
    /// Build lookup tables from settings, returning any parse errors
    pub fn from_settings(settings: &KeymapSettings) -> (Self, Vec<String>) {
        let mut normal = HashMap::new();
        let mut insert = HashMap::new();
        let mut errors = Vec::new();

        for entry in &settings.normal {
            if let Some(from) = parse_key_notation(&entry.from) {
                // Parse the 'to' field as an action (command or keys)
                let action = parse_action(&entry.to);
                normal.insert(from, action);
            } else {
                errors.push(format!(
                    "Keymap: invalid normal mode key '{}'",
                    entry.from
                ));
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

        // Parse leader key
        let leader_key = parse_key_notation(&settings.leader);
        if leader_key.is_none() && !settings.leader.is_empty() {
            errors.push(format!(
                "Keymap: invalid leader key '{}'",
                settings.leader
            ));
        }

        // Parse leader mappings
        let mut leader_mappings = HashMap::new();
        for mapping in &settings.leader_mappings {
            let action = parse_action(&mapping.action);
            leader_mappings.insert(mapping.key.clone(), action);
        }

        (
            Self {
                normal,
                insert,
                leader_key,
                leader_mappings,
            },
            errors,
        )
    }

    /// Get the normal mode mapping for a key, if one exists
    pub fn get_normal_mapping(&self, key: KeyEvent) -> Option<&LeaderAction> {
        self.normal.get(&key)
    }

    /// Remap a key in insert mode, returning the original if no mapping exists
    pub fn remap_insert(&self, key: KeyEvent) -> KeyEvent {
        self.insert.get(&key).copied().unwrap_or(key)
    }

    /// Check if there are any normal mode mappings
    pub fn has_normal_mappings(&self) -> bool {
        !self.normal.is_empty()
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
        self.leader_mappings.keys().any(|k| k.starts_with(sequence) && k != sequence)
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

/// Parse a key notation string into a KeyEvent
///
/// Supported formats:
/// - Single characters: "a", "H", ";", "0"
/// - Control keys: "<C-r>", "<C-s>"
/// - Special keys: "<CR>", "<Esc>", "<Tab>", "<BS>", "<Space>"
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
    let inner_lower = inner.to_lowercase();

    // Control key: <C-x>
    if inner_lower.starts_with("c-") && inner.len() == 3 {
        let c = inner.chars().nth(2)?;
        return Some(KeyEvent::new(
            KeyCode::Char(c.to_ascii_lowercase()),
            KeyModifiers::CONTROL,
        ));
    }

    // Alt/Meta key: <A-x> or <M-x>
    if (inner_lower.starts_with("a-") || inner_lower.starts_with("m-")) && inner.len() == 3 {
        let c = inner.chars().nth(2)?;
        return Some(KeyEvent::new(
            KeyCode::Char(c.to_ascii_lowercase()),
            KeyModifiers::ALT,
        ));
    }

    // Shift key: <S-x>
    if inner_lower.starts_with("s-") && inner.len() == 3 {
        let c = inner.chars().nth(2)?;
        return Some(KeyEvent::new(
            KeyCode::Char(c.to_ascii_uppercase()),
            KeyModifiers::SHIFT,
        ));
    }

    // Function keys: <F1> through <F12>
    if inner_lower.starts_with('f') {
        if let Ok(n) = inner[1..].parse::<u8>() {
            if (1..=12).contains(&n) {
                return Some(KeyEvent::new(KeyCode::F(n), KeyModifiers::NONE));
            }
        }
    }

    // Special named keys
    match inner_lower.as_str() {
        "cr" | "enter" | "return" => Some(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        "esc" | "escape" => Some(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        "tab" => Some(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        "bs" | "backspace" => Some(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
        "del" | "delete" => Some(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)),
        "space" => Some(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)),
        "up" => Some(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
        "down" => Some(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        "left" => Some(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)),
        "right" => Some(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
        "home" => Some(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)),
        "end" => Some(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
        "pageup" => Some(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE)),
        "pagedown" => Some(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE)),
        "insert" => Some(KeyEvent::new(KeyCode::Insert, KeyModifiers::NONE)),
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
    fn test_parse_special() {
        let key = parse_key_notation("<CR>").unwrap();
        assert_eq!(key.code, KeyCode::Enter);

        let key = parse_key_notation("<Esc>").unwrap();
        assert_eq!(key.code, KeyCode::Esc);

        let key = parse_key_notation("<Tab>").unwrap();
        assert_eq!(key.code, KeyCode::Tab);

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
            leader: " ".to_string(),  // Literal space, as used in default config
            normal: vec![],
            insert: vec![],
            leader_mappings: vec![
                super::super::LeaderMapping {
                    key: "m".to_string(),
                    action: ":HarpoonAdd".to_string(),
                    desc: Some("Add to harpoon".to_string()),
                },
            ],
        };

        let (lookup, errors) = KeymapLookup::from_settings(&settings);
        assert!(errors.is_empty(), "Should have no errors");
        let space_key = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);

        assert!(lookup.has_leader_mappings(), "Should have leader mappings");
        assert!(lookup.is_leader_key(space_key), "Literal space should be recognized as leader key");

        // Check we can look up the mapping
        let action = lookup.get_leader_action("m");
        assert!(action.is_some(), "Should find 'm' mapping");
    }

    #[test]
    fn test_leader_key_matching() {
        use super::super::KeymapSettings;

        let settings = KeymapSettings {
            leader: "<Space>".to_string(),
            normal: vec![],
            insert: vec![],
            leader_mappings: vec![
                super::super::LeaderMapping {
                    key: "w".to_string(),
                    action: ":w<CR>".to_string(),
                    desc: Some("Save".to_string()),
                },
            ],
        };

        let (lookup, errors) = KeymapLookup::from_settings(&settings);
        assert!(errors.is_empty(), "Should have no errors");

        // Simulate a space key press from crossterm
        let space_key = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);

        assert!(lookup.has_leader_mappings(), "Should have leader mappings");
        assert!(lookup.is_leader_key(space_key), "Space should be recognized as leader key");

        // Check we can look up the mapping
        let action = lookup.get_leader_action("w");
        assert!(action.is_some(), "Should find 'w' mapping");
    }

    #[test]
    fn test_parse_function() {
        let key = parse_key_notation("<F1>").unwrap();
        assert_eq!(key.code, KeyCode::F(1));

        let key = parse_key_notation("<F12>").unwrap();
        assert_eq!(key.code, KeyCode::F(12));
    }
}
