//! Harpoon - Quick file marks for fast navigation
//!
//! Provides up to 4 slots for frequently accessed files with instant jump.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Maximum number of harpoon slots
pub const MAX_SLOTS: usize = 4;

/// Harpoon file data for persistence
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct HarpoonData {
    files: Vec<String>,
}

/// Harpoon manager for quick file navigation
#[derive(Debug)]
pub struct Harpoon {
    /// List of marked file paths (max 4)
    files: Vec<PathBuf>,
    /// Current index for ]h/[h navigation
    current_index: Option<usize>,
    /// Project root for persistence
    project_root: Option<PathBuf>,
    /// Whether the menu is currently open
    pub menu_open: bool,
    /// Currently selected item in menu (0-indexed)
    pub menu_selection: usize,
}

impl Default for Harpoon {
    fn default() -> Self {
        Self::new()
    }
}

impl Harpoon {
    /// Create a new Harpoon instance
    pub fn new() -> Self {
        Self {
            files: Vec::with_capacity(MAX_SLOTS),
            current_index: None,
            project_root: None,
            menu_open: false,
            menu_selection: 0,
        }
    }

    /// Set the project root and load existing harpoon data
    pub fn set_project_root(&mut self, root: PathBuf) {
        self.project_root = Some(root);
        self.load();
    }

    /// Get the harpoon data file path
    fn data_file_path(&self) -> Option<PathBuf> {
        self.project_root.as_ref().map(|root| root.join(".nevi").join("harpoon.json"))
    }

    /// Load harpoon data from disk
    fn load(&mut self) {
        let Some(path) = self.data_file_path() else {
            return;
        };

        if !path.exists() {
            return;
        }

        match std::fs::read_to_string(&path) {
            Ok(content) => {
                if let Ok(data) = serde_json::from_str::<HarpoonData>(&content) {
                    self.files = data.files.into_iter()
                        .take(MAX_SLOTS)
                        .map(PathBuf::from)
                        .collect();
                }
            }
            Err(_) => {}
        }
    }

    /// Save harpoon data to disk
    fn save(&self) {
        let Some(path) = self.data_file_path() else {
            return;
        };

        // Create .nevi directory if it doesn't exist
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let data = HarpoonData {
            files: self.files.iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect(),
        };

        if let Ok(json) = serde_json::to_string_pretty(&data) {
            let _ = std::fs::write(&path, json);
        }
    }

    /// Add a file to harpoon. If already exists, moves it to the end.
    /// If at max capacity, removes the first file.
    pub fn add_file(&mut self, path: &Path) -> String {
        let path = path.to_path_buf();

        // Check if file already exists
        if let Some(pos) = self.files.iter().position(|p| p == &path) {
            // Move to end
            self.files.remove(pos);
            self.files.push(path.clone());
            self.save();
            return format!("Moved to harpoon slot {}", self.files.len());
        }

        // If at max capacity, remove first
        if self.files.len() >= MAX_SLOTS {
            self.files.remove(0);
        }

        self.files.push(path);
        self.save();
        format!("Added to harpoon slot {}", self.files.len())
    }

    /// Remove a file from harpoon by index (0-indexed)
    pub fn remove(&mut self, index: usize) -> bool {
        if index < self.files.len() {
            self.files.remove(index);
            self.save();
            // Adjust menu selection if needed
            if self.menu_selection >= self.files.len() && self.menu_selection > 0 {
                self.menu_selection = self.files.len().saturating_sub(1);
            }
            true
        } else {
            false
        }
    }

    /// Get file at slot (1-indexed, like keybindings)
    pub fn get_slot(&self, slot: usize) -> Option<&PathBuf> {
        if slot >= 1 && slot <= MAX_SLOTS {
            self.files.get(slot - 1)
        } else {
            None
        }
    }

    /// Jump to next harpoon file, returns the path
    pub fn next(&mut self) -> Option<&PathBuf> {
        if self.files.is_empty() {
            return None;
        }

        let next_index = match self.current_index {
            Some(idx) => (idx + 1) % self.files.len(),
            None => 0,
        };

        self.current_index = Some(next_index);
        self.files.get(next_index)
    }

    /// Jump to previous harpoon file, returns the path
    pub fn prev(&mut self) -> Option<&PathBuf> {
        if self.files.is_empty() {
            return None;
        }

        let prev_index = match self.current_index {
            Some(idx) => {
                if idx == 0 {
                    self.files.len() - 1
                } else {
                    idx - 1
                }
            }
            None => self.files.len() - 1,
        };

        self.current_index = Some(prev_index);
        self.files.get(prev_index)
    }

    /// Set current index when a file is opened (to sync ]h/[h navigation)
    pub fn set_current_file(&mut self, path: &Path) {
        self.current_index = self.files.iter().position(|p| p == path);
    }

    /// Get all files for menu display
    pub fn files(&self) -> &[PathBuf] {
        &self.files
    }

    /// Get number of marked files
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Check if harpoon is empty
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Toggle menu open/closed
    pub fn toggle_menu(&mut self) {
        self.menu_open = !self.menu_open;
        if self.menu_open {
            self.menu_selection = 0;
        }
    }

    /// Close menu
    pub fn close_menu(&mut self) {
        self.menu_open = false;
    }

    /// Move menu selection up
    pub fn menu_up(&mut self) {
        if !self.files.is_empty() && self.menu_selection > 0 {
            self.menu_selection -= 1;
        }
    }

    /// Move menu selection down
    pub fn menu_down(&mut self) {
        if !self.files.is_empty() && self.menu_selection < self.files.len() - 1 {
            self.menu_selection += 1;
        }
    }

    /// Get currently selected file in menu
    pub fn menu_selected_file(&self) -> Option<&PathBuf> {
        self.files.get(self.menu_selection)
    }

    /// Move selected item up in the list
    pub fn move_up(&mut self) {
        if self.menu_selection > 0 && self.files.len() > 1 {
            self.files.swap(self.menu_selection, self.menu_selection - 1);
            self.menu_selection -= 1;
            self.save();
        }
    }

    /// Move selected item down in the list
    pub fn move_down(&mut self) {
        if self.menu_selection < self.files.len() - 1 && self.files.len() > 1 {
            self.files.swap(self.menu_selection, self.menu_selection + 1);
            self.menu_selection += 1;
            self.save();
        }
    }

    /// Remove currently selected item in menu
    pub fn remove_selected(&mut self) -> bool {
        self.remove(self.menu_selection)
    }
}
