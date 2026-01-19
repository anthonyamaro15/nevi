//! Git integration module for displaying git signs (added/modified/deleted lines)

use similar::{ChangeTag, TextDiff};
use std::path::Path;

/// Status of a line compared to the HEAD version
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitLineStatus {
    /// Line was added (not in HEAD)
    Added,
    /// Line was modified (content differs from HEAD)
    Modified,
    /// Line(s) were deleted at this position
    Deleted,
}

/// A single hunk representing a change at a specific line
#[derive(Debug, Clone)]
pub struct GitHunk {
    /// Line number (0-indexed)
    pub line: usize,
    /// Type of change
    pub status: GitLineStatus,
}

/// Collection of git diff hunks for a file
#[derive(Debug, Clone, Default)]
pub struct GitDiff {
    /// All hunks in the file
    pub hunks: Vec<GitHunk>,
}

impl GitDiff {
    /// Get the status for a specific line
    pub fn status_for_line(&self, line: usize) -> Option<GitLineStatus> {
        self.hunks
            .iter()
            .find(|h| h.line == line)
            .map(|h| h.status)
    }
}

/// Wrapper around git2::Repository for git operations
pub struct GitRepo {
    repo: git2::Repository,
}

impl GitRepo {
    /// Try to open a git repository from the given path
    /// Searches upward to find .git directory
    pub fn open(path: &Path) -> Option<Self> {
        git2::Repository::discover(path)
            .ok()
            .map(|repo| Self { repo })
    }

    /// Get the working directory of the repository
    pub fn workdir(&self) -> Option<&Path> {
        self.repo.workdir()
    }

    /// Get the content of a file at HEAD
    pub fn head_content(&self, file_path: &Path) -> Option<String> {
        let head = self.repo.head().ok()?;
        let tree = head.peel_to_tree().ok()?;

        // Make the path relative to the repository root
        let relative = file_path.strip_prefix(self.repo.workdir()?).ok()?;

        let entry = tree.get_path(relative).ok()?;
        let blob = self.repo.find_blob(entry.id()).ok()?;

        // Convert blob content to string (skip binary files)
        String::from_utf8(blob.content().to_vec()).ok()
    }

    /// Check if a file is tracked by git
    pub fn is_tracked(&self, file_path: &Path) -> bool {
        let Some(workdir) = self.repo.workdir() else {
            return false;
        };

        let Ok(relative) = file_path.strip_prefix(workdir) else {
            return false;
        };

        // Check if file is in the index or HEAD tree
        if let Ok(index) = self.repo.index() {
            if index.get_path(relative, 0).is_some() {
                return true;
            }
        }

        // Also check HEAD tree
        if let Ok(head) = self.repo.head() {
            if let Ok(tree) = head.peel_to_tree() {
                if tree.get_path(relative).is_ok() {
                    return true;
                }
            }
        }

        false
    }
}

/// Compute the diff between HEAD content and current content
/// Returns a GitDiff with all changed hunks
pub fn compute_diff(head_content: &str, current_content: &str) -> GitDiff {
    let diff = TextDiff::from_lines(head_content, current_content);
    let mut hunks = Vec::new();

    // Track which lines have been marked as modified
    // (we use this to upgrade Add to Modified when appropriate)
    let mut modified_lines: std::collections::HashSet<usize> = std::collections::HashSet::new();

    // Track position in new file for delete markers
    let mut new_line_idx = 0;
    let mut pending_deletes = 0;

    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => {
                // Line was added
                if let Some(new_idx) = change.new_index() {
                    // If there were pending deletes at this position, this is a modification
                    if pending_deletes > 0 {
                        hunks.push(GitHunk {
                            line: new_idx,
                            status: GitLineStatus::Modified,
                        });
                        modified_lines.insert(new_idx);
                        pending_deletes -= 1;
                    } else {
                        hunks.push(GitHunk {
                            line: new_idx,
                            status: GitLineStatus::Added,
                        });
                    }
                    new_line_idx = new_idx + 1;
                }
            }
            ChangeTag::Delete => {
                // Line was deleted - track it for potential modification detection
                pending_deletes += 1;
            }
            ChangeTag::Equal => {
                // If we have pending deletes that weren't matched by inserts,
                // add a delete marker at the current position
                if pending_deletes > 0 {
                    // Show delete marker at the line where deletions occurred
                    // (just before the current line in the new file)
                    let delete_marker_line = new_line_idx;
                    hunks.push(GitHunk {
                        line: delete_marker_line,
                        status: GitLineStatus::Deleted,
                    });
                    pending_deletes = 0;
                }

                if let Some(new_idx) = change.new_index() {
                    new_line_idx = new_idx + 1;
                }
            }
        }
    }

    // Handle any remaining deletes at end of file
    if pending_deletes > 0 {
        // Mark delete at end of file (use last line index)
        let delete_marker_line = if new_line_idx > 0 { new_line_idx - 1 } else { 0 };
        hunks.push(GitHunk {
            line: delete_marker_line,
            status: GitLineStatus::Deleted,
        });
    }

    GitDiff { hunks }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_diff_added_lines() {
        let head = "line1\nline2\n";
        let current = "line1\nnew line\nline2\n";

        let diff = compute_diff(head, current);

        assert_eq!(diff.hunks.len(), 1);
        assert_eq!(diff.hunks[0].line, 1); // 0-indexed, "new line"
        assert_eq!(diff.hunks[0].status, GitLineStatus::Added);
    }

    #[test]
    fn test_compute_diff_modified_lines() {
        let head = "line1\nline2\nline3\n";
        let current = "line1\nmodified line\nline3\n";

        let diff = compute_diff(head, current);

        assert_eq!(diff.hunks.len(), 1);
        assert_eq!(diff.hunks[0].line, 1); // "modified line"
        assert_eq!(diff.hunks[0].status, GitLineStatus::Modified);
    }

    #[test]
    fn test_compute_diff_deleted_lines() {
        let head = "line1\nline2\nline3\n";
        let current = "line1\nline3\n";

        let diff = compute_diff(head, current);

        // Should have a delete marker
        assert!(!diff.hunks.is_empty());
        assert!(diff.hunks.iter().any(|h| h.status == GitLineStatus::Deleted));
    }

    #[test]
    fn test_compute_diff_empty_files() {
        let diff = compute_diff("", "");
        assert!(diff.hunks.is_empty());
    }

    #[test]
    fn test_compute_diff_new_file() {
        let head = "";
        let current = "line1\nline2\n";

        let diff = compute_diff(head, current);

        assert_eq!(diff.hunks.len(), 2);
        assert!(diff.hunks.iter().all(|h| h.status == GitLineStatus::Added));
    }
}
