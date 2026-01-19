use crossterm::event::KeyEvent;
use std::collections::HashMap;

/// Macro recording and playback state
#[derive(Debug, Clone, Default)]
pub struct MacroState {
    /// Stored macros by register (a-z)
    macros: HashMap<char, Vec<KeyEvent>>,
    /// Currently recording to this register (None if not recording)
    recording: Option<char>,
    /// Keys being recorded for the current macro
    current_recording: Vec<KeyEvent>,
    /// Last executed macro register (for @@)
    last_executed: Option<char>,
}

impl MacroState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if currently recording a macro
    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    /// Get the register currently being recorded to
    pub fn recording_register(&self) -> Option<char> {
        self.recording
    }

    /// Start recording a macro to the given register
    pub fn start_recording(&mut self, register: char) {
        self.recording = Some(register);
        self.current_recording.clear();
    }

    /// Stop recording and save the macro
    pub fn stop_recording(&mut self) {
        if let Some(register) = self.recording.take() {
            // Only save if we recorded something
            if !self.current_recording.is_empty() {
                self.macros.insert(register, self.current_recording.clone());
            }
            self.current_recording.clear();
        }
    }

    /// Record a key event (called during recording)
    pub fn record_key(&mut self, key: KeyEvent) {
        if self.recording.is_some() {
            self.current_recording.push(key);
        }
    }

    /// Get a stored macro by register
    pub fn get_macro(&self, register: char) -> Option<&Vec<KeyEvent>> {
        self.macros.get(&register)
    }

    /// Get the last executed macro register
    pub fn last_executed(&self) -> Option<char> {
        self.last_executed
    }

    /// Set the last executed macro register
    pub fn set_last_executed(&mut self, register: char) {
        self.last_executed = Some(register);
    }

    /// Check if a register name is valid for macros (a-z)
    pub fn is_valid_register(c: char) -> bool {
        c.is_ascii_lowercase()
    }
}
