use std::collections::HashMap;

/// Type of register content
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterContent {
    /// Character-wise (inline) content
    Chars(String),
    /// Line-wise content (includes trailing newline semantically)
    Lines(String),
}

/// Check if a register is the black hole register
pub fn is_black_hole_register(name: Option<char>) -> bool {
    matches!(name, Some('_'))
}

/// Check if a register is a clipboard register
pub fn is_clipboard_register(name: Option<char>) -> bool {
    matches!(name, Some('+') | Some('*'))
}

impl RegisterContent {
    pub fn as_str(&self) -> &str {
        match self {
            RegisterContent::Chars(s) => s,
            RegisterContent::Lines(s) => s,
        }
    }

    pub fn is_linewise(&self) -> bool {
        matches!(self, RegisterContent::Lines(_))
    }

    pub fn to_shada_entry(&self) -> crate::shada::RegisterEntry {
        crate::shada::RegisterEntry {
            text: self.as_str().to_string(),
            linewise: self.is_linewise(),
        }
    }

    pub fn from_shada_entry(entry: crate::shada::RegisterEntry) -> Self {
        if entry.linewise {
            RegisterContent::Lines(entry.text)
        } else {
            RegisterContent::Chars(entry.text)
        }
    }
}

fn clipboard_text_for_content(content: &RegisterContent) -> String {
    match content {
        RegisterContent::Chars(text) => text.clone(),
        RegisterContent::Lines(text) if text.ends_with('\n') => text.clone(),
        RegisterContent::Lines(text) => format!("{text}\n"),
    }
}

fn register_content_from_clipboard_text(text: String) -> Option<RegisterContent> {
    if text.is_empty() {
        None
    } else if text.ends_with('\n') {
        Some(RegisterContent::Lines(text))
    } else {
        Some(RegisterContent::Chars(text))
    }
}

/// Vim-style register system
#[derive(Debug, Clone, Default)]
pub struct Registers {
    /// Named registers (a-z)
    named: HashMap<char, RegisterContent>,
    /// Unnamed register (default for yank/delete)
    unnamed: Option<RegisterContent>,
    /// Small delete register (for deletes less than one line)
    small_delete: Option<RegisterContent>,
    /// Numbered registers 1-9 (for delete history)
    numbered: [Option<RegisterContent>; 9],
    /// Last clipboard error (for display to user)
    clipboard_error: Option<String>,
    /// Test builds never touch the real clipboard; see `read_clipboard`.
    #[cfg(test)]
    test_clipboard: Option<String>,
}

impl Registers {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the content of a register
    /// Note: For clipboard registers (+ and *), use get_clipboard() instead
    pub fn get(&self, name: Option<char>) -> Option<&RegisterContent> {
        match name {
            None | Some('"') => self.unnamed.as_ref(),
            Some('-') => self.small_delete.as_ref(),
            Some(c @ 'a'..='z') | Some(c @ 'A'..='Z') => self.named.get(&c.to_ascii_lowercase()),
            Some(c @ '1'..='9') => {
                let idx = c.to_digit(10).unwrap() as usize - 1;
                self.numbered[idx].as_ref()
            }
            Some('0') => {
                // Register 0 contains the last yank
                // For simplicity, we'll just return unnamed for now
                self.unnamed.as_ref()
            }
            Some('_') => {
                // Black hole register - always empty
                None
            }
            Some('+') | Some('*') => {
                // Clipboard registers - handled separately by get_clipboard()
                // Return None here as the caller should use get_clipboard()
                None
            }
            _ => None,
        }
    }

    /// Get content from the system clipboard
    pub fn get_clipboard(&mut self) -> Option<RegisterContent> {
        match self.read_clipboard() {
            Ok(text) => {
                self.clipboard_error = None;
                text.and_then(register_content_from_clipboard_text)
            }
            Err(e) => {
                self.clipboard_error = Some(e);
                None
            }
        }
    }

    /// Set content to the system clipboard
    pub fn set_clipboard(&mut self, content: &RegisterContent) {
        let text = clipboard_text_for_content(content);
        self.clipboard_error = self.write_clipboard(text).err();
    }

    #[cfg(not(test))]
    fn read_clipboard(&mut self) -> Result<Option<String>, String> {
        let mut clipboard =
            arboard::Clipboard::new().map_err(|e| format!("Clipboard unavailable: {e}"))?;
        let text = clipboard
            .get_text()
            .map_err(|e| format!("Clipboard read failed: {e}"))?;
        Ok(Some(text))
    }

    #[cfg(not(test))]
    fn write_clipboard(&mut self, text: String) -> Result<(), String> {
        let mut clipboard =
            arboard::Clipboard::new().map_err(|e| format!("Clipboard unavailable: {e}"))?;
        clipboard
            .set_text(text)
            .map_err(|e| format!("Clipboard write failed: {e}"))
    }

    /// Tests run in parallel, and parallel access to the real macOS
    /// pasteboard crashes the test binary (SIGSEGV in arboard). It would
    /// also overwrite whatever the developer had copied.
    #[cfg(test)]
    fn read_clipboard(&mut self) -> Result<Option<String>, String> {
        Ok(self.test_clipboard.clone())
    }

    #[cfg(test)]
    fn write_clipboard(&mut self, text: String) -> Result<(), String> {
        self.test_clipboard = Some(text);
        Ok(())
    }

    /// Take any clipboard error (clears it after returning)
    pub fn take_clipboard_error(&mut self) -> Option<String> {
        self.clipboard_error.take()
    }

    /// Set the content of a register
    /// Restore a delete-history register from persisted state. Only shada
    /// apply uses this: `set` deliberately excludes "1-"9 because the shift
    /// logic in the delete path owns those slots during normal editing.
    pub fn restore_numbered(&mut self, digit: char, content: RegisterContent) {
        if let Some(idx) = digit.to_digit(10).filter(|d| (1..=9).contains(d)) {
            self.numbered[idx as usize - 1] = Some(content);
        }
    }

    pub fn set(&mut self, name: Option<char>, content: RegisterContent) {
        match name {
            None | Some('"') => {
                self.unnamed = Some(content);
            }
            Some('-') => {
                self.small_delete = Some(content);
            }
            Some('_') => {
                // Black hole register - discard content
            }
            Some('+') | Some('*') => {
                // Clipboard registers
                self.set_clipboard(&content);
            }
            Some(c @ 'a'..='z') => {
                self.named.insert(c, content);
            }
            Some(c @ 'A'..='Z') => {
                // Uppercase appends to the register
                let lower = c.to_ascii_lowercase();
                if let Some(existing) = self.named.get_mut(&lower) {
                    match (existing, &content) {
                        (RegisterContent::Chars(s), RegisterContent::Chars(new)) => {
                            s.push_str(new);
                        }
                        (RegisterContent::Lines(s), RegisterContent::Lines(new)) => {
                            s.push_str(new);
                        }
                        (RegisterContent::Chars(s), RegisterContent::Lines(new)) => {
                            s.push('\n');
                            s.push_str(new);
                        }
                        (RegisterContent::Lines(s), RegisterContent::Chars(new)) => {
                            s.push_str(new);
                        }
                    }
                } else {
                    self.named.insert(lower, content);
                }
            }
            _ => {
                // For other registers, just set unnamed
                self.unnamed = Some(content);
            }
        }
    }

    /// Set content from a yank operation (also updates unnamed register)
    /// Note: Always syncs with system clipboard for modern UX (like clipboard=unnamedplus)
    pub fn yank(&mut self, name: Option<char>, content: RegisterContent) {
        // Black hole register discards content
        if is_black_hole_register(name) {
            return;
        }

        // Clipboard registers
        if is_clipboard_register(name) {
            self.set_clipboard(&content);
            self.unnamed = Some(content);
            return;
        }

        // Always update unnamed register
        self.unnamed = Some(content.clone());

        // Always sync with system clipboard (modern behavior like clipboard=unnamedplus)
        self.set_clipboard(&content);

        // Also update named register if specified
        if let Some(c) = name {
            if c != '"' {
                self.set(Some(c), content);
            }
        }
    }

    /// Set content from a delete operation (updates numbered registers)
    /// Note: Always syncs with system clipboard for modern UX (like clipboard=unnamedplus)
    pub fn delete(&mut self, name: Option<char>, content: RegisterContent, is_small: bool) {
        // Black hole register discards content and doesn't update any registers
        if is_black_hole_register(name) {
            return;
        }

        // Clipboard registers
        if is_clipboard_register(name) {
            self.set_clipboard(&content);
            self.unnamed = Some(content);
            return;
        }

        // Always update unnamed register
        self.unnamed = Some(content.clone());

        // Always sync with system clipboard (modern behavior like clipboard=unnamedplus)
        self.set_clipboard(&content);

        if let Some(c) = name {
            // If a register was specified, use it
            self.set(Some(c), content);
        } else if is_small {
            // Small deletes go to the small delete register
            self.small_delete = Some(content);
        } else {
            // Shift numbered registers
            for i in (1..9).rev() {
                self.numbered[i] = self.numbered[i - 1].take();
            }
            self.numbered[0] = Some(content);
        }
    }

    /// Get content for a register, including clipboard support
    /// This is the main method to use for getting register content
    /// Note: For unnamed register, checks system clipboard first for modern UX
    pub fn get_content(&mut self, name: Option<char>) -> Option<RegisterContent> {
        if is_clipboard_register(name) {
            self.get_clipboard()
        } else if name.is_none() || name == Some('"') {
            // For unnamed register, try system clipboard first (modern behavior)
            // This allows pasting content copied from external apps
            self.get_clipboard().or_else(|| self.get(name).cloned())
        } else {
            self.get(name).cloned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RegisterContent, clipboard_text_for_content, register_content_from_clipboard_text,
    };

    #[test]
    fn linewise_clipboard_round_trip_preserves_shape_without_final_newline() {
        let serialized = clipboard_text_for_content(&RegisterContent::Lines("abc".to_string()));

        assert_eq!(serialized, "abc\n");
        assert_eq!(
            register_content_from_clipboard_text(serialized),
            Some(RegisterContent::Lines("abc\n".to_string()))
        );
    }

    #[test]
    fn characterwise_clipboard_round_trip_does_not_add_a_newline() {
        let serialized = clipboard_text_for_content(&RegisterContent::Chars("abc".to_string()));

        assert_eq!(serialized, "abc");
        assert_eq!(
            register_content_from_clipboard_text(serialized),
            Some(RegisterContent::Chars("abc".to_string()))
        );
    }

    #[test]
    fn linewise_clipboard_text_does_not_duplicate_an_existing_newline() {
        let serialized = clipboard_text_for_content(&RegisterContent::Lines("abc\n".to_string()));

        assert_eq!(serialized, "abc\n");
    }
}
