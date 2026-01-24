//! Harpoon - Quick file marks for fast navigation
//!
//! Provides unlimited slots for frequently accessed files with instant jump.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Harpoon file data for persistence
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct HarpoonData {
    files: Vec<String>,
}

/// Harpoon manager for quick file navigation
#[derive(Debug)]
pub struct Harpoon {
    /// List of marked file paths
    files: Vec<PathBuf>,
    /// Current index for ]h/[h navigation
    current_index: Option<usize>,
    /// Project root for persistence
    project_root: Option<PathBuf>,
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
            files: Vec::new(),
            current_index: None,
            project_root: None,
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

        self.files.push(path);
        self.save();
        format!("Added to harpoon slot {}", self.files.len())
    }

    /// Remove a file from harpoon by index (0-indexed)
    pub fn remove(&mut self, index: usize) -> bool {
        if index < self.files.len() {
            self.files.remove(index);
            self.save();
            true
        } else {
            false
        }
    }

    /// Get file at slot (1-indexed)
    pub fn get_slot(&self, slot: usize) -> Option<&PathBuf> {
        if slot >= 1 {
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

    /// Swap two items in the list
    pub fn swap(&mut self, a: usize, b: usize) {
        if a < self.files.len() && b < self.files.len() && a != b {
            self.files.swap(a, b);
            self.save();
        }
    }
}
