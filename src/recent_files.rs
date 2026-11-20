//! Persistent recently-opened-files list backing the start screen.
//!
//! Reuses [`FrecencyDb`] scoring with its own database file. Opens are
//! recorded in memory only; the binary saves once on exit, so file opens
//! never pay disk I/O and the test suite never touches the real db.

use crate::frecency::FrecencyDb;
use std::path::{Path, PathBuf};

/// Entries untouched for this long are dropped at save time.
const MAX_AGE_DAYS: u64 = 90;

pub struct RecentFiles {
    db: FrecencyDb,
    db_path: PathBuf,
}

impl RecentFiles {
    /// Reads from the XDG state dir (with the legacy pre-0.4.0 fallback that
    /// `shada::state_file` provides); saves always write the state dir, so
    /// data in an old location migrates on its first save.
    pub fn load() -> Self {
        Self {
            db: FrecencyDb::load_at(&crate::shada::state_file("recent_files.json")),
            db_path: crate::shada::state_dir().join("recent_files.json"),
        }
    }

    /// Load and save at one explicit db file (tests point this at a temp path).
    pub fn load_from(db_path: PathBuf) -> Self {
        Self {
            db: FrecencyDb::load_at(&db_path),
            db_path,
        }
    }

    /// Record a file open. In-memory only — call `save` on editor exit.
    pub fn record(&mut self, file: &Path) {
        let absolute = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
        self.db.record_use(&absolute.to_string_lossy());
    }

    pub fn save(&mut self) {
        self.db.prune(MAX_AGE_DAYS);
        self.db.save_at(&self.db_path);
    }

    /// Highest-scored files that still exist. The existence check (one stat
    /// per candidate) runs lazily down the score-sorted list, and only while
    /// the start screen is on screen.
    pub fn top(&self, limit: usize) -> Vec<PathBuf> {
        let mut scored: Vec<(&str, f64)> = self
            .db
            .entries()
            .keys()
            .map(|label| (label.as_str(), self.db.score(label)))
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        scored
            .iter()
            .map(|(label, _)| PathBuf::from(label))
            .filter(|p| p.is_file())
            .take(limit)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(name: &str) -> RecentFiles {
        let dir = std::env::temp_dir().join(format!("nevi-recents-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        RecentFiles::load_from(dir.join("recent_files.json"))
    }

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, "x").unwrap();
        p
    }

    #[test]
    fn top_skips_missing_files() {
        let base = std::env::temp_dir().join("nevi-recents-test-top");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();

        let mut store = RecentFiles::load_from(base.join("recent_files.json"));
        let kept = touch(&base, "a.rs");
        let missing = base.join("deleted.rs");
        store.record(&kept);
        store.record(&missing);
        store.record(&missing);

        let top = store.top(5);
        assert_eq!(
            top,
            vec![std::fs::canonicalize(&kept).unwrap()],
            "deleted files never reach the start screen"
        );
    }

    #[test]
    fn save_and_reload_round_trips() {
        let mut store = temp_store("roundtrip");
        let base = std::env::temp_dir().join("nevi-recents-test-roundtrip");
        let file = touch(&base, "kept.rs");
        store.record(&file);
        store.save();

        let reloaded = RecentFiles::load_from(store.db_path.clone());
        assert_eq!(reloaded.top(5), vec![std::fs::canonicalize(&file).unwrap()]);
    }

    #[test]
    fn top_respects_limit_ordering_by_score() {
        let base = std::env::temp_dir().join("nevi-recents-test-limit");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let mut store = RecentFiles::load_from(base.join("recent_files.json"));

        let a = touch(&base, "a.rs");
        let b = touch(&base, "b.rs");
        let c = touch(&base, "c.rs");
        // b used three times, a twice, c once → score order b, a, c.
        store.record(&b);
        store.record(&b);
        store.record(&b);
        store.record(&a);
        store.record(&a);
        store.record(&c);

        let top = store.top(2);
        assert_eq!(
            top,
            vec![
                std::fs::canonicalize(&b).unwrap(),
                std::fs::canonicalize(&a).unwrap()
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn saved_db_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let mut store = temp_store("private");
        store.save();
        let mode = std::fs::metadata(&store.db_path)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
