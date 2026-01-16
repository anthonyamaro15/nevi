use std::collections::HashMap;
use arboard::Clipboard;

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
            Some(c @ 'a'..='z') | Some(c @ 'A'..='Z') => {
                self.named.get(&c.to_ascii_lowercase())
            }
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
    pub fn get_clipboard(&self) -> Option<RegisterContent> {
        if let Ok(mut clipboard) = Clipboard::new() {
            if let Ok(text) = clipboard.get_text() {
                if !text.is_empty() {
                    // Determine if it's line-wise (ends with newline)
                    if text.ends_with('\n') {
                        return Some(RegisterContent::Lines(text));
                    } else {
                        return Some(RegisterContent::Chars(text));
                    }
                }
            }
        }
        None
    }

    /// Set content to the system clipboard
    pub fn set_clipboard(&mut self, content: &RegisterContent) {
        if let Ok(mut clipboard) = Clipboard::new() {
            let _ = clipboard.set_text(content.as_str().to_string());
        }
    }

    /// Set the content of a register
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
                        (RegisterContent::Chars(ref mut s), RegisterContent::Chars(new)) => {
                            s.push_str(new);
                        }
                        (RegisterContent::Lines(ref mut s), RegisterContent::Lines(new)) => {
                            s.push_str(new);
                        }
                        (RegisterContent::Chars(ref mut s), RegisterContent::Lines(new)) => {
                            s.push('\n');
                            s.push_str(new);
                        }
                        (RegisterContent::Lines(ref mut s), RegisterContent::Chars(new)) => {
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

        // Also update named register if specified
        if let Some(c) = name {
            if c != '"' {
                self.set(Some(c), content);
            }
        }
    }

    /// Set content from a delete operation (updates numbered registers)
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
    pub fn get_content(&self, name: Option<char>) -> Option<RegisterContent> {
        if is_clipboard_register(name) {
            self.get_clipboard()
        } else {
            self.get(name).cloned()
        }
    }
}
