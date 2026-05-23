//! Git integration module for displaying git signs (added/modified/deleted lines)

use similar::{ChangeTag, TextDiff};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

const TEXT_SAMPLE_LIMIT: usize = 8 * 1024;
const PREVIEW_BYTE_LIMIT: usize = 16 * 1024;

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

/// Status of a file in the working tree or index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GitFileStatus {
    /// File was added to the index
    Added,
    /// File was modified, renamed, or typechanged
    Modified,
    /// File was deleted
    Deleted,
    /// File is untracked
    Untracked,
    /// File has merge conflicts
    Conflicted,
}

impl GitFileStatus {
    fn priority(self) -> u8 {
        match self {
            GitFileStatus::Untracked => 1,
            GitFileStatus::Added => 2,
            GitFileStatus::Deleted => 3,
            GitFileStatus::Modified => 4,
            GitFileStatus::Conflicted => 5,
        }
    }

    pub fn merge(self, other: Self) -> Self {
        if other.priority() > self.priority() {
            other
        } else {
            self
        }
    }

    pub fn picker_prefix(self) -> &'static str {
        match self {
            GitFileStatus::Modified => "M",
            GitFileStatus::Added => "A",
            GitFileStatus::Deleted => "D",
            GitFileStatus::Untracked => "?",
            GitFileStatus::Conflicted => "!",
        }
    }

    pub fn picker_sort_rank(self) -> u8 {
        match self {
            GitFileStatus::Conflicted => 0,
            GitFileStatus::Modified => 1,
            GitFileStatus::Deleted => 2,
            GitFileStatus::Added => 3,
            GitFileStatus::Untracked => 4,
        }
    }

    pub fn is_deleted(self) -> bool {
        self == GitFileStatus::Deleted
    }
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
        self.hunks.iter().find(|h| h.line == line).map(|h| h.status)
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

    /// Get file statuses for the repository, keyed by absolute file path.
    pub fn file_statuses(&self) -> HashMap<PathBuf, GitFileStatus> {
        let mut statuses = HashMap::new();
        let Some(workdir) = self.repo.workdir() else {
            return statuses;
        };

        let mut options = git2::StatusOptions::new();
        options
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            .renames_head_to_index(true)
            .renames_index_to_workdir(true);

        let Ok(repo_statuses) = self.repo.statuses(Some(&mut options)) else {
            return statuses;
        };

        for entry in repo_statuses.iter() {
            let Some(status) = git_file_status_from_git2(entry.status()) else {
                continue;
            };
            let Some(relative) = git_status_entry_path(&entry) else {
                continue;
            };

            statuses.insert(workdir.join(relative), status);
        }

        statuses
    }

    pub fn diff_preview(
        &self,
        file_path: &Path,
        status: GitFileStatus,
        max_lines: usize,
    ) -> Vec<String> {
        if max_lines == 0 {
            return Vec::new();
        }

        let Some(workdir) = self.repo.workdir() else {
            return vec!["No git worktree available".to_string()];
        };

        let Some(relative) = relative_to_workdir(workdir, file_path) else {
            return vec!["File is outside the git worktree".to_string()];
        };

        if status == GitFileStatus::Untracked {
            if !is_text_previewable(file_path) {
                return vec!["File is binary or unreadable".to_string()];
            }
            return untracked_diff_preview(&relative, file_path, max_lines);
        }

        if !status.is_deleted() && !is_text_previewable(file_path) {
            return vec!["File is binary or unreadable".to_string()];
        }

        let mut lines = Vec::new();
        let mut options = git2::DiffOptions::new();
        options.pathspec(&relative);

        let index = self.repo.index().ok();
        let head_tree = self
            .repo
            .head()
            .ok()
            .and_then(|head| head.peel_to_tree().ok());

        if let Some(index) = index.as_ref() {
            if let Ok(diff) =
                self.repo
                    .diff_tree_to_index(head_tree.as_ref(), Some(index), Some(&mut options))
            {
                if !collect_diff_lines(&diff, &mut lines, max_lines) {
                    return vec!["File is binary or unreadable".to_string()];
                }
            }

            if !preview_is_truncated(&lines) {
                let mut options = git2::DiffOptions::new();
                options.pathspec(&relative);
                if let Ok(diff) = self
                    .repo
                    .diff_index_to_workdir(Some(index), Some(&mut options))
                {
                    if !collect_diff_lines(&diff, &mut lines, max_lines) {
                        return vec!["File is binary or unreadable".to_string()];
                    }
                }
            }
        }

        if lines.is_empty() {
            if let Some(rename_preview) = self.worktree_rename_preview(&relative, max_lines) {
                return rename_preview;
            }
            lines.push("No diff available".to_string());
        }

        lines
    }

    fn worktree_rename_preview(&self, relative: &Path, max_lines: usize) -> Option<Vec<String>> {
        let mut options = git2::StatusOptions::new();
        options
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            .renames_index_to_workdir(true);

        let repo_statuses = self.repo.statuses(Some(&mut options)).ok()?;
        for entry in repo_statuses.iter() {
            if !entry.status().contains(git2::Status::WT_RENAMED) {
                continue;
            }

            let delta = entry.index_to_workdir()?;
            let old_path = delta.old_file().path()?;
            let new_path = delta.new_file().path()?;
            if new_path != relative {
                continue;
            }

            return Some(rename_diff_preview(old_path, new_path, max_lines));
        }

        None
    }
}

fn git_status_entry_path(entry: &git2::StatusEntry<'_>) -> Option<PathBuf> {
    if entry.status().contains(git2::Status::INDEX_RENAMED) {
        return entry
            .head_to_index()
            .and_then(|delta| delta.new_file().path().map(Path::to_path_buf));
    }

    if entry.status().contains(git2::Status::WT_RENAMED) {
        return entry
            .index_to_workdir()
            .and_then(|delta| delta.new_file().path().map(Path::to_path_buf));
    }

    entry.path().map(PathBuf::from)
}

fn is_text_previewable(file_path: &Path) -> bool {
    let Ok(file) = std::fs::File::open(file_path) else {
        return false;
    };
    let mut sample = Vec::with_capacity(TEXT_SAMPLE_LIMIT);
    let Ok(_) = file.take(TEXT_SAMPLE_LIMIT as u64).read_to_end(&mut sample) else {
        return false;
    };

    bytes_are_previewable_text(&sample)
}

fn bytes_are_previewable_text(bytes: &[u8]) -> bool {
    if bytes.contains(&0) {
        return false;
    }

    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };

    for ch in text.chars() {
        if ch.is_control() && !matches!(ch, '\n' | '\r' | '\t') {
            return false;
        }
    }

    true
}

fn relative_to_workdir(workdir: &Path, file_path: &Path) -> Option<PathBuf> {
    if let Ok(relative) = file_path.strip_prefix(workdir) {
        return Some(relative.to_path_buf());
    }

    let canonical_workdir = workdir.canonicalize().ok()?;
    let canonical_file = file_path.canonicalize().ok()?;
    canonical_file
        .strip_prefix(canonical_workdir)
        .ok()
        .map(Path::to_path_buf)
}

fn collect_diff_lines(diff: &git2::Diff<'_>, lines: &mut Vec<String>, max_lines: usize) -> bool {
    let mut safe = true;
    let _ = diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        if preview_is_truncated(lines) {
            return false;
        }

        let mut content = line.content();
        if content.ends_with(b"\n") {
            content = &content[..content.len() - 1];
        }

        if line.origin() == 'B' || content.starts_with(b"Binary files ") {
            safe = false;
            return false;
        }

        let truncated_by_bytes = content.len() > PREVIEW_BYTE_LIMIT;
        let mut content = content[..content.len().min(PREVIEW_BYTE_LIMIT)].to_vec();
        while !std::str::from_utf8(&content).is_ok() {
            content.pop();
        }
        if !bytes_are_previewable_text(&content) {
            safe = false;
            return false;
        }

        let content = String::from_utf8(content).expect("validated utf-8");
        let content = sanitize_preview_text(&content);
        let origin = line.origin();
        let rendered = match origin {
            '+' | '-' | ' ' => format!("{origin}{content}"),
            _ => content,
        };
        if !push_preview_line(lines, max_lines, rendered) {
            return false;
        }
        if truncated_by_bytes {
            let _ = push_preview_line(lines, max_lines, "... (truncated)".to_string());
            return false;
        }
        true
    });
    safe
}

fn rename_diff_preview(old_path: &Path, new_path: &Path, max_lines: usize) -> Vec<String> {
    let old_display = sanitize_path_display(old_path);
    let new_display = sanitize_path_display(new_path);
    let mut lines = Vec::new();
    for line in [
        format!("diff --git a/{old_display} b/{new_display}"),
        format!("rename from {old_display}"),
        format!("rename to {new_display}"),
    ] {
        if !push_preview_line(&mut lines, max_lines, line) {
            break;
        }
    }
    lines
}

fn untracked_diff_preview(relative_path: &Path, file_path: &Path, max_lines: usize) -> Vec<String> {
    let Ok(mut file) = std::fs::File::open(file_path) else {
        return vec!["File is binary or unreadable".to_string()];
    };

    let display_path = sanitize_path_display(relative_path);
    let mut lines = Vec::new();
    for line in [
        format!("diff --git a/{display_path} b/{display_path}"),
        "new file mode 100644".to_string(),
        "--- /dev/null".to_string(),
        format!("+++ b/{display_path}"),
    ] {
        if !push_preview_line(&mut lines, max_lines, line) {
            return lines;
        }
    }

    let mut content = Vec::new();
    let Ok(_) = file
        .by_ref()
        .take((PREVIEW_BYTE_LIMIT + 1) as u64)
        .read_to_end(&mut content)
    else {
        return vec!["File is binary or unreadable".to_string()];
    };
    let truncated_by_bytes = content.len() > PREVIEW_BYTE_LIMIT;
    if truncated_by_bytes {
        content.truncate(PREVIEW_BYTE_LIMIT);
        while !std::str::from_utf8(&content).is_ok() {
            content.pop();
        }
    }
    if !bytes_are_previewable_text(&content) {
        return vec!["File is binary or unreadable".to_string()];
    }
    let content = String::from_utf8(content).expect("validated utf-8");

    for line in content.split('\n') {
        let line = sanitize_preview_text(line);
        if !push_preview_line(&mut lines, max_lines, format!("+{line}")) {
            return lines;
        }
    }

    if truncated_by_bytes {
        let _ = push_preview_line(&mut lines, max_lines, "... (truncated)".to_string());
    }

    lines
}

fn sanitize_path_display(path: &Path) -> String {
    sanitize_preview_text(&path.to_string_lossy().replace('\\', "/"))
}

fn sanitize_preview_text(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch.is_control() && !matches!(ch, '\t') {
                '?'
            } else {
                ch
            }
        })
        .collect()
}

fn preview_is_truncated(lines: &[String]) -> bool {
    lines.last().is_some_and(|line| line == "... (truncated)")
}

fn push_preview_line(lines: &mut Vec<String>, max_lines: usize, line: String) -> bool {
    if max_lines == 0 {
        return false;
    }

    if lines.len() < max_lines {
        lines.push(line);
        return true;
    }

    if let Some(last) = lines.last_mut() {
        *last = "... (truncated)".to_string();
    }
    false
}

fn git_file_status_from_git2(status: git2::Status) -> Option<GitFileStatus> {
    if status.contains(git2::Status::CONFLICTED) {
        return Some(GitFileStatus::Conflicted);
    }

    if status.intersects(git2::Status::WT_DELETED | git2::Status::INDEX_DELETED) {
        return Some(GitFileStatus::Deleted);
    }

    if status.intersects(
        git2::Status::WT_MODIFIED
            | git2::Status::INDEX_MODIFIED
            | git2::Status::WT_RENAMED
            | git2::Status::INDEX_RENAMED
            | git2::Status::WT_TYPECHANGE
            | git2::Status::INDEX_TYPECHANGE,
    ) {
        return Some(GitFileStatus::Modified);
    }

    if status.contains(git2::Status::INDEX_NEW) {
        return Some(GitFileStatus::Added);
    }

    if status.contains(git2::Status::WT_NEW) {
        return Some(GitFileStatus::Untracked);
    }

    None
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
        let delete_marker_line = if new_line_idx > 0 {
            new_line_idx - 1
        } else {
            0
        };
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
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), nanos))
    }

    fn commit_file(repo: &git2::Repository, relative_path: &Path, message: &str) {
        let signature =
            git2::Signature::now("Nevi Test", "nevi-test@example.com").expect("signature");
        let mut index = repo.index().expect("index");
        index.add_path(relative_path).expect("add file");
        index.write().expect("write index");
        let tree_id = index.write_tree().expect("write tree");
        let tree = repo.find_tree(tree_id).expect("find tree");

        if let Some(parent_id) = repo.head().ok().and_then(|head| head.target()) {
            let parent = repo.find_commit(parent_id).expect("find parent");
            repo.commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &[&parent],
            )
            .expect("commit");
        } else {
            repo.commit(Some("HEAD"), &signature, &signature, message, &tree, &[])
                .expect("initial commit");
        }
    }

    #[test]
    fn git_changes_status_metadata_matches_picker_order() {
        assert_eq!(GitFileStatus::Modified.picker_prefix(), "M");
        assert_eq!(GitFileStatus::Added.picker_prefix(), "A");
        assert_eq!(GitFileStatus::Deleted.picker_prefix(), "D");
        assert_eq!(GitFileStatus::Untracked.picker_prefix(), "?");
        assert_eq!(GitFileStatus::Conflicted.picker_prefix(), "!");
        assert!(GitFileStatus::Deleted.is_deleted());
        assert!(!GitFileStatus::Modified.is_deleted());

        let mut statuses = vec![
            GitFileStatus::Untracked,
            GitFileStatus::Added,
            GitFileStatus::Deleted,
            GitFileStatus::Modified,
            GitFileStatus::Conflicted,
        ];
        statuses.sort_by_key(|status| status.picker_sort_rank());

        assert_eq!(
            statuses,
            vec![
                GitFileStatus::Conflicted,
                GitFileStatus::Modified,
                GitFileStatus::Deleted,
                GitFileStatus::Added,
                GitFileStatus::Untracked,
            ]
        );
    }

    #[test]
    fn git_changes_diff_preview_includes_modified_file_patch() {
        let root = unique_temp_dir("nevi_git_changes_diff_modified");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let path = root.join("tracked.rs");
        std::fs::write(&path, "old\n").expect("write original");
        let raw_repo = git2::Repository::init(&root).expect("init repo");
        commit_file(&raw_repo, Path::new("tracked.rs"), "initial");
        std::fs::write(&path, "new\n").expect("write modified");

        let repo = GitRepo::open(&root).expect("open repo");
        let preview = repo.diff_preview(&path, GitFileStatus::Modified, 50);
        let joined = preview.join("\n");

        assert!(joined.contains("diff --git"));
        assert!(joined.contains("-old"));
        assert!(joined.contains("+new"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn git_changes_diff_preview_formats_untracked_text_file_as_added() {
        let root = unique_temp_dir("nevi_git_changes_diff_untracked");
        std::fs::create_dir_all(&root).expect("create temp dir");
        git2::Repository::init(&root).expect("init repo");
        let path = root.join("new.rs");
        std::fs::write(&path, "fn main() {}\n").expect("write untracked");

        let repo = GitRepo::open(&root).expect("open repo");
        let preview = repo.diff_preview(&path, GitFileStatus::Untracked, 50);
        let joined = preview.join("\n");

        assert!(joined.contains("new file mode"));
        assert!(joined.contains("--- /dev/null"));
        assert!(joined.contains("+++ b/new.rs"));
        assert!(joined.contains("+fn main() {}"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn git_changes_diff_preview_uses_placeholder_for_untracked_nul_file() {
        let root = unique_temp_dir("nevi_git_changes_diff_untracked_nul");
        std::fs::create_dir_all(&root).expect("create temp dir");
        git2::Repository::init(&root).expect("init repo");
        let path = root.join("nul.txt");
        std::fs::write(&path, b"hello\0world\n").expect("write untracked");

        let repo = GitRepo::open(&root).expect("open repo");
        let preview = repo.diff_preview(&path, GitFileStatus::Untracked, 50);

        assert_eq!(preview, vec!["File is binary or unreadable"]);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn git_changes_diff_preview_uses_placeholder_for_control_heavy_untracked_file() {
        let root = unique_temp_dir("nevi_git_changes_diff_untracked_control");
        std::fs::create_dir_all(&root).expect("create temp dir");
        git2::Repository::init(&root).expect("init repo");
        let path = root.join("ansi.txt");
        let content = "\x1b[31m\x1b[0m".repeat(256);
        std::fs::write(&path, content).expect("write untracked");

        let repo = GitRepo::open(&root).expect("open repo");
        let preview = repo.diff_preview(&path, GitFileStatus::Untracked, 50);

        assert_eq!(preview, vec!["File is binary or unreadable"]);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn git_changes_diff_preview_uses_placeholder_for_single_control_untracked_file() {
        let root = unique_temp_dir("nevi_git_changes_diff_untracked_single_control");
        std::fs::create_dir_all(&root).expect("create temp dir");
        git2::Repository::init(&root).expect("init repo");
        let path = root.join("bell.txt");
        std::fs::write(&path, "mostly normal text with bell \x07\n").expect("write untracked");

        let repo = GitRepo::open(&root).expect("open repo");
        let preview = repo.diff_preview(&path, GitFileStatus::Untracked, 50);

        assert_eq!(preview, vec!["File is binary or unreadable"]);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn git_changes_untracked_diff_preview_bounds_large_line_and_truncates() {
        let root = unique_temp_dir("nevi_git_changes_diff_untracked_large_line");
        std::fs::create_dir_all(&root).expect("create temp dir");
        git2::Repository::init(&root).expect("init repo");
        let path = root.join("large.txt");
        std::fs::write(&path, "a".repeat(TEXT_SAMPLE_LIMIT * 4)).expect("write untracked");

        let repo = GitRepo::open(&root).expect("open repo");
        let preview = repo.diff_preview(&path, GitFileStatus::Untracked, 5);

        assert_eq!(preview.len(), 5);
        assert_eq!(preview.last().map(String::as_str), Some("... (truncated)"));
        assert!(preview
            .iter()
            .all(|line| line.len() <= PREVIEW_BYTE_LIMIT + 1));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn git_changes_diff_preview_uses_placeholder_for_tracked_binary_file() {
        let root = unique_temp_dir("nevi_git_changes_diff_binary");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let path = root.join("image.bin");
        std::fs::write(&path, [0, 159, 146, 150]).expect("write original");
        let raw_repo = git2::Repository::init(&root).expect("init repo");
        commit_file(&raw_repo, Path::new("image.bin"), "initial");
        std::fs::write(&path, [0, 255, 146, 150]).expect("write modified");

        let repo = GitRepo::open(&root).expect("open repo");
        let preview = repo.diff_preview(&path, GitFileStatus::Modified, 50);

        assert_eq!(preview, vec!["File is binary or unreadable"]);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn git_changes_diff_preview_uses_placeholder_for_deleted_tracked_binary_file() {
        let root = unique_temp_dir("nevi_git_changes_diff_deleted_binary");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let path = root.join("image.bin");
        std::fs::write(&path, [0, 159, 146, 150]).expect("write original");
        let raw_repo = git2::Repository::init(&root).expect("init repo");
        commit_file(&raw_repo, Path::new("image.bin"), "initial");
        std::fs::remove_file(&path).expect("delete tracked file");

        let repo = GitRepo::open(&root).expect("open repo");
        let statuses = repo.file_statuses();
        let (path, status) = statuses
            .iter()
            .find(|(path, _status)| path.ends_with("image.bin"))
            .expect("deleted file status");
        let preview = repo.diff_preview(path, *status, 50);

        assert_eq!(preview, vec!["File is binary or unreadable"]);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn git_changes_tracked_diff_preview_strips_raw_carriage_returns() {
        let root = unique_temp_dir("nevi_git_changes_diff_tracked_cr");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let path = root.join("crlf.txt");
        std::fs::write(&path, "old\r\n").expect("write original");
        let raw_repo = git2::Repository::init(&root).expect("init repo");
        commit_file(&raw_repo, Path::new("crlf.txt"), "initial");
        std::fs::write(&path, "new\r\n").expect("write modified");

        let repo = GitRepo::open(&root).expect("open repo");
        let preview = repo.diff_preview(&path, GitFileStatus::Modified, 50);

        assert!(preview.iter().all(|line| !line.contains('\r')));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn git_changes_untracked_diff_preview_strips_raw_carriage_returns() {
        let root = unique_temp_dir("nevi_git_changes_diff_untracked_cr");
        std::fs::create_dir_all(&root).expect("create temp dir");
        git2::Repository::init(&root).expect("init repo");
        let path = root.join("crlf.txt");
        std::fs::write(&path, "new\r\n").expect("write untracked");

        let repo = GitRepo::open(&root).expect("open repo");
        let preview = repo.diff_preview(&path, GitFileStatus::Untracked, 50);

        assert!(preview.iter().all(|line| !line.contains('\r')));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn git_changes_preview_path_display_replaces_control_chars() {
        let old_path = Path::new("old\x1b\r\nname.rs");
        let new_path = Path::new("new\x1b\r\nname.rs");
        let preview = rename_diff_preview(old_path, new_path, 10);

        assert!(preview.iter().all(|line| !line.contains('\x1b')));
        assert!(preview.iter().all(|line| !line.contains('\r')));
        assert!(preview.iter().all(|line| !line.contains('\n')));
        assert!(preview.iter().any(|line| line.contains('?')));
    }

    #[test]
    fn git_changes_diff_preview_returns_empty_for_zero_line_budget() {
        let root = unique_temp_dir("nevi_git_changes_diff_zero_budget");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let tracked_path = root.join("tracked.rs");
        std::fs::write(&tracked_path, "old\n").expect("write original");
        let raw_repo = git2::Repository::init(&root).expect("init repo");
        commit_file(&raw_repo, Path::new("tracked.rs"), "initial");
        std::fs::write(&tracked_path, "new\n").expect("write modified");
        let untracked_path = root.join("new.rs");
        std::fs::write(&untracked_path, "fn main() {}\n").expect("write untracked");

        let repo = GitRepo::open(&root).expect("open repo");

        assert!(repo
            .diff_preview(&tracked_path, GitFileStatus::Modified, 0)
            .is_empty());
        assert!(repo
            .diff_preview(&untracked_path, GitFileStatus::Untracked, 0)
            .is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn git_changes_diff_preview_uses_placeholder_for_tracked_deleted_control_file() {
        let root = unique_temp_dir("nevi_git_changes_diff_deleted_control");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let path = root.join("control.txt");
        std::fs::write(&path, "safe\nbad \x1b\n").expect("write original");
        let raw_repo = git2::Repository::init(&root).expect("init repo");
        commit_file(&raw_repo, Path::new("control.txt"), "initial");
        std::fs::remove_file(&path).expect("delete tracked file");

        let repo = GitRepo::open(&root).expect("open repo");
        let statuses = repo.file_statuses();
        let (path, status) = statuses
            .iter()
            .find(|(path, _status)| path.ends_with("control.txt"))
            .expect("deleted file status");
        let preview = repo.diff_preview(path, *status, 50);

        assert_eq!(preview, vec!["File is binary or unreadable"]);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn git_changes_tracked_diff_preview_bounds_large_line_and_truncates() {
        let root = unique_temp_dir("nevi_git_changes_diff_tracked_large_line");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let path = root.join("large.txt");
        std::fs::write(&path, "old\n").expect("write original");
        let raw_repo = git2::Repository::init(&root).expect("init repo");
        commit_file(&raw_repo, Path::new("large.txt"), "initial");
        std::fs::write(&path, "a".repeat(PREVIEW_BYTE_LIMIT * 4)).expect("write modified");

        let repo = GitRepo::open(&root).expect("open repo");
        let preview = repo.diff_preview(&path, GitFileStatus::Modified, 50);

        assert!(preview
            .iter()
            .all(|line| line.len() <= PREVIEW_BYTE_LIMIT + 1));
        assert_eq!(preview.last().map(String::as_str), Some("... (truncated)"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn git_changes_diff_preview_handles_worktree_rename_destination_path() {
        let root = unique_temp_dir("nevi_git_changes_diff_rename_preview");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let old_path = root.join("old.rs");
        std::fs::write(&old_path, "fn old() {}\n").expect("write original");
        let raw_repo = git2::Repository::init(&root).expect("init repo");
        commit_file(&raw_repo, Path::new("old.rs"), "initial");
        let new_path = root.join("new.rs");
        std::fs::rename(&old_path, &new_path).expect("rename file");

        let repo = GitRepo::open(&root).expect("open repo");
        let statuses = repo.file_statuses();
        let (path, status) = statuses
            .iter()
            .find(|(path, _status)| path.ends_with("new.rs"))
            .expect("renamed destination status");
        let preview = repo.diff_preview(path, *status, 50);
        let joined = preview.join("\n");

        assert_ne!(preview, vec!["No diff available"]);
        assert!(joined.contains("diff --git"));
        assert!(joined.contains("old.rs"));
        assert!(joined.contains("new.rs"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn git_changes_diff_preview_handles_nested_worktree_rename_destination_path() {
        let root = unique_temp_dir("nevi_git_changes_diff_nested_rename_preview");
        let old_dir = root.join("old");
        let new_dir = root.join("new");
        std::fs::create_dir_all(&old_dir).expect("create old dir");
        let old_path = old_dir.join("file.rs");
        std::fs::write(&old_path, "fn old() {}\n").expect("write original");
        let raw_repo = git2::Repository::init(&root).expect("init repo");
        commit_file(&raw_repo, Path::new("old/file.rs"), "initial");
        std::fs::create_dir_all(&new_dir).expect("create new dir");
        let new_path = new_dir.join("file.rs");
        std::fs::rename(&old_path, &new_path).expect("rename file");

        let repo = GitRepo::open(&root).expect("open repo");
        let statuses = repo.file_statuses();
        let (path, status) = statuses
            .iter()
            .find(|(path, _status)| path.ends_with("new/file.rs"))
            .expect("renamed destination status");
        let preview = repo.diff_preview(path, *status, 50);
        let joined = preview.join("\n");

        assert_ne!(preview, vec!["No diff available"]);
        assert!(joined.contains("old/file.rs"));
        assert!(joined.contains("new/file.rs"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn git_changes_file_statuses_use_rename_destination_path() {
        let root = unique_temp_dir("nevi_git_changes_status_rename");
        std::fs::create_dir_all(&root).expect("create temp dir");
        let old_path = root.join("old.rs");
        let new_path = root.join("new.rs");
        std::fs::write(&old_path, "fn old() {}\n").expect("write original");
        let raw_repo = git2::Repository::init(&root).expect("init repo");
        commit_file(&raw_repo, Path::new("old.rs"), "initial");
        std::fs::rename(&old_path, &new_path).expect("rename file");

        let repo = GitRepo::open(&root).expect("open repo");
        let statuses = repo.file_statuses();

        assert_eq!(
            statuses
                .iter()
                .find(|(path, _status)| path.ends_with("new.rs"))
                .map(|(_path, status)| status),
            Some(&GitFileStatus::Modified)
        );
        assert!(!statuses.keys().any(|path| path.ends_with("old.rs")));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn git_changes_untracked_diff_preview_truncates_at_line_limit() {
        let root = unique_temp_dir("nevi_git_changes_diff_untracked_truncated");
        std::fs::create_dir_all(&root).expect("create temp dir");
        git2::Repository::init(&root).expect("init repo");
        let path = root.join("large.rs");
        std::fs::write(&path, "one\ntwo\nthree\nfour\nfive\n").expect("write untracked");

        let repo = GitRepo::open(&root).expect("open repo");
        let preview = repo.diff_preview(&path, GitFileStatus::Untracked, 5);

        assert_eq!(preview.len(), 5);
        assert_eq!(preview.last().map(String::as_str), Some("... (truncated)"));

        let _ = std::fs::remove_dir_all(&root);
    }

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
        assert!(diff
            .hunks
            .iter()
            .any(|h| h.status == GitLineStatus::Deleted));
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

    #[test]
    fn test_git_file_status_priority() {
        assert_eq!(
            GitFileStatus::Added.merge(GitFileStatus::Modified),
            GitFileStatus::Modified
        );
        assert_eq!(
            GitFileStatus::Modified.merge(GitFileStatus::Conflicted),
            GitFileStatus::Conflicted
        );
        assert_eq!(
            GitFileStatus::Deleted.merge(GitFileStatus::Untracked),
            GitFileStatus::Deleted
        );
    }

    #[test]
    fn test_git_file_status_from_git2() {
        assert_eq!(
            git_file_status_from_git2(git2::Status::WT_MODIFIED),
            Some(GitFileStatus::Modified)
        );
        assert_eq!(
            git_file_status_from_git2(git2::Status::INDEX_NEW),
            Some(GitFileStatus::Added)
        );
        assert_eq!(
            git_file_status_from_git2(git2::Status::WT_NEW),
            Some(GitFileStatus::Untracked)
        );
        assert_eq!(
            git_file_status_from_git2(git2::Status::CONFLICTED | git2::Status::WT_MODIFIED),
            Some(GitFileStatus::Conflicted)
        );
    }
}
