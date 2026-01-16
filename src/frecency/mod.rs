//! Frecency tracking for completion items
//!
//! Frecency = Frequency + Recency. Items that are used more often and more recently
//! get higher scores. Based on the algorithm from blink.cmp.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Entry tracking usage of a single completion item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrecencyEntry {
    /// Number of times this item was selected
    pub count: u32,
    /// Unix timestamp of last selection
    pub last_used: u64,
}

/// Frecency database for completion items
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct FrecencyDb {
    /// Map from completion label to frecency entry
    entries: HashMap<String, FrecencyEntry>,
}

impl FrecencyDb {
    /// Load frecency database from file, or create empty if not found
    pub fn load() -> Self {
        let path = Self::db_path();
        if path.exists() {
            if let Ok(contents) = fs::read_to_string(&path) {
                if let Ok(db) = serde_json::from_str(&contents) {
                    return db;
                }
            }
        }
        Self::default()
    }

    /// Save frecency database to file
    pub fn save(&self) {
        let path = Self::db_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, json);
        }
    }

    /// Get the path to the frecency database file
    fn db_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("nevi")
            .join("frecency.json")
    }

    /// Record that a completion item was selected
    pub fn record_use(&mut self, label: &str) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let entry = self.entries.entry(label.to_string()).or_insert(FrecencyEntry {
            count: 0,
            last_used: now,
        });
        entry.count += 1;
        entry.last_used = now;
    }

    /// Calculate frecency score for a completion item
    /// Higher score = more frequently/recently used
    ///
    /// Formula from blink.cmp: score * (1 / (1 + elapsed_hours))^0.2
    /// We return a boost factor to multiply with other scores
    pub fn score(&self, label: &str) -> f64 {
        let entry = match self.entries.get(label) {
            Some(e) => e,
            None => return 1.0, // No history, neutral score
        };

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Calculate elapsed time in hours
        let elapsed_secs = now.saturating_sub(entry.last_used);
        let elapsed_hours = elapsed_secs as f64 / 3600.0;

        // Recency decay: (1 / (1 + elapsed_hours))^0.2
        // This decays slowly - even after 24 hours, the factor is still ~0.55
        let recency_factor = (1.0 / (1.0 + elapsed_hours)).powf(0.2);

        // Frequency boost: log2(count + 1) gives diminishing returns
        // count=1 -> 1.0, count=3 -> 2.0, count=7 -> 3.0, count=15 -> 4.0
        let frequency_factor = (entry.count as f64 + 1.0).log2();

        // Combined score: frequency * recency
        frequency_factor * recency_factor
    }

    /// Get all entries (for debugging)
    #[allow(dead_code)]
    pub fn entries(&self) -> &HashMap<String, FrecencyEntry> {
        &self.entries
    }

    /// Prune old entries that haven't been used in a long time
    /// Keeps the database from growing indefinitely
    #[allow(dead_code)]
    pub fn prune(&mut self, max_age_days: u64) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let max_age_secs = max_age_days * 24 * 60 * 60;
        self.entries.retain(|_, entry| {
            now.saturating_sub(entry.last_used) < max_age_secs
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frecency_scoring() {
        let mut db = FrecencyDb::default();

        // New item has neutral score
        assert_eq!(db.score("new_item"), 1.0);

        // Record usage
        db.record_use("test_item");
        let score1 = db.score("test_item");
        assert!(score1 > 1.0, "Used item should have higher score");

        // More usage = higher score
        db.record_use("test_item");
        db.record_use("test_item");
        let score2 = db.score("test_item");
        assert!(score2 > score1, "More frequent use should increase score");
    }
}
