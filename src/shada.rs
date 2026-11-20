//! Shada-lite: persist small editor state across sessions — vim's
//! shada/viminfo in miniature. Covers macros, named + unnamed registers,
//! global marks, and search history. Command history keeps its own file
//! (`command_history.txt`) but shares this module's state-dir resolution.
//!
//! Macros are stored as vim notation (the macro-lens codec), so the state
//! file remains human-readable and a hand-edited entry that fails to parse is
//! skipped rather than poisoning the rest. Loading never fails: a missing or
//! corrupt file yields empty state.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{DirBuilder, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub const FORMAT_VERSION: u32 = 1;

/// Matches `MAX_SEARCH_HISTORY_ENTRIES` in the editor.
const MAX_HISTORY_ENTRIES: usize = 100;
/// Registers above this size are session-only (vim's shada caps these too).
const MAX_REGISTER_TEXT_CHARS: usize = 100_000;
/// Matches `MAX_JUMPS` in the editor's jump list.
const MAX_JUMPLIST_ENTRIES: usize = 100;

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShadaState {
    #[serde(default)]
    pub version: u32,
    /// Register -> vim notation, e.g. "a" -> "0f,ci\"hi<Esc>".
    #[serde(default)]
    pub macros: BTreeMap<char, String>,
    #[serde(default)]
    pub registers: BTreeMap<char, RegisterEntry>,
    #[serde(default)]
    pub unnamed_register: Option<RegisterEntry>,
    /// The last-yank register "0 and the delete-history registers "1-"9.
    #[serde(default)]
    pub numbered_registers: BTreeMap<char, RegisterEntry>,
    #[serde(default)]
    pub global_marks: BTreeMap<char, MarkEntry>,
    #[serde(default)]
    pub search_history: Vec<String>,
    /// Ctrl-O/Ctrl-I navigation history, oldest first. A jump is a
    /// (path, line, col) triple exactly like a global mark, so the entry
    /// type is shared; jumps in scratch buffers are session-only.
    #[serde(default)]
    pub jumplist: Vec<MarkEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegisterEntry {
    pub text: String,
    pub linewise: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarkEntry {
    pub path: PathBuf,
    pub line: usize,
    pub col: usize,
}

/// Machine-written state lives in the XDG state dir on every platform,
/// matching nvim's `stdpath('state')`: `$XDG_STATE_HOME/nevi`, else
/// `~/.local/state/nevi`. Deliberately not `dirs::config_dir()` — on macOS
/// that is `~/Library/Application Support`, and terminal editors follow XDG
/// everywhere (same reasoning as the hardcoded `~/.config` in the config
/// loader).
pub fn state_dir() -> PathBuf {
    if let Some(dir) = isolated_state_dir() {
        return dir;
    }
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        // Per the XDG spec, a relative $XDG_STATE_HOME must be ignored.
        .filter(|p| p.is_absolute())
        .or_else(|| dirs::home_dir().map(|h| h.join(".local").join("state")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("nevi")
}

/// Read location for a state file: the XDG path, falling back to the legacy
/// `dirs::config_dir()/nevi` location where nevi <= 0.3.0 wrote
/// `frecency.json` and `command_history.txt`. Writes must use `state_dir()`
/// so data migrates to the XDG home on its next save; the legacy copy is
/// left behind, inert.
pub fn state_file(name: &str) -> PathBuf {
    if let Some(dir) = isolated_state_dir() {
        return dir.join(name);
    }
    let legacy = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("nevi")
        .join(name);
    resolve_state_file(state_dir().join(name), legacy)
}

/// Tests must never see or change the developer's real state. The library's
/// own tests are isolated from the start; the binary's tests link the library
/// built without `cfg(test)`, so they flip this switch themselves.
static STATE_ISOLATED: AtomicBool = AtomicBool::new(cfg!(test));

/// Turn off all state persistence for the rest of this process. Only for
/// test code; nothing in the editor calls it.
#[doc(hidden)]
pub fn isolate_state_for_tests() {
    STATE_ISOLATED.store(true, Ordering::Relaxed);
}

/// Under isolation, state paths point inside the running executable. That is
/// a file, so the path can never exist or be created: reads find no state and
/// writes fail (every caller already ignores state write errors). Tests get no
/// persistence at all, so they cannot leak history into each other either.
fn isolated_state_dir() -> Option<PathBuf> {
    STATE_ISOLATED.load(Ordering::Relaxed).then(|| {
        std::env::current_exe()
            .expect("isolated state needs the test executable path")
            .join("nevi-state")
    })
}

fn resolve_state_file(new: PathBuf, legacy: PathBuf) -> PathBuf {
    if !new.exists() && legacy.exists() {
        legacy
    } else {
        new
    }
}

/// Global, like vim's shada — registers and macros aren't project-scoped.
/// No legacy fallback: `state.json` never shipped at the old location.
pub fn state_file_path() -> PathBuf {
    state_dir().join("state.json")
}

pub fn load() -> ShadaState {
    load_from(&state_file_path())
}

/// Missing or corrupt state must never block startup.
pub fn load_from(path: &Path) -> ShadaState {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return ShadaState::default();
    };
    let mut state: ShadaState = serde_json::from_str(&contents).unwrap_or_default();
    enforce_caps(&mut state);
    state
}

pub fn save(state: &ShadaState) -> io::Result<()> {
    save_to(&state_file_path(), state)
}

pub fn save_to(path: &Path, state: &ShadaState) -> io::Result<()> {
    let mut capped = state.clone();
    capped.version = FORMAT_VERSION;
    enforce_caps(&mut capped);

    let json = serde_json::to_string_pretty(&capped)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    // Atomic replace so a crash mid-write can't corrupt existing state.
    let tmp = path.with_extension("json.tmp");
    // A stale temp file may be world-readable and already open elsewhere, so
    // start from a fresh inode rather than reuse it.
    let _ = std::fs::remove_file(&tmp);
    open_private(&tmp)?.write_all(json.as_bytes())?;
    std::fs::rename(&tmp, path)
}

/// Open a state file for a full rewrite, readable by its owner alone.
///
/// State files hold yanked text and command lines, which can be secrets, so
/// on Unix they are 0600 like Vim's viminfo and Neovim's shada. A new file is
/// 0600 from creation; an existing one (an older Nevi wrote 0644) is emptied
/// and tightened before the caller writes. A missing parent directory is
/// created 0700; an existing one keeps its mode.
pub fn open_private(path: &Path) -> io::Result<File> {
    let mut dir = DirBuilder::new();
    dir.recursive(true);
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        dir.mode(0o700);
        options.mode(0o600);
    }

    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        dir.create(parent)?;
    }
    let file = options.open(path)?;
    // The open mode only applies to a new file, and is masked by the umask.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

/// Applied on both save and load: caps hold even for hand-edited files.
fn enforce_caps(state: &mut ShadaState) {
    state
        .registers
        .retain(|name, entry| name.is_ascii_lowercase() && fits_register_cap(entry));
    if let Some(entry) = &state.unnamed_register {
        if !fits_register_cap(entry) {
            state.unnamed_register = None;
        }
    }
    state
        .numbered_registers
        .retain(|name, entry| name.is_ascii_digit() && fits_register_cap(entry));
    state.macros.retain(|name, _| name.is_ascii_lowercase());
    state
        .global_marks
        .retain(|name, _| name.is_ascii_uppercase());
    if state.search_history.len() > MAX_HISTORY_ENTRIES {
        let extra = state.search_history.len() - MAX_HISTORY_ENTRIES;
        state.search_history.drain(0..extra);
    }
    if state.jumplist.len() > MAX_JUMPLIST_ENTRIES {
        let extra = state.jumplist.len() - MAX_JUMPLIST_ENTRIES;
        state.jumplist.drain(0..extra);
    }
}

fn fits_register_cap(entry: &RegisterEntry) -> bool {
    entry.text.chars().count() <= MAX_REGISTER_TEXT_CHARS
}

#[cfg(test)]
mod tests {
    use super::{
        FORMAT_VERSION, MAX_HISTORY_ENTRIES, MAX_REGISTER_TEXT_CHARS, MarkEntry, RegisterEntry,
        ShadaState, load_from, open_private, resolve_state_file, save_to, state_dir, state_file,
        state_file_path,
    };
    use std::path::PathBuf;

    /// The developer's real state homes: XDG, the `~/.local/state` default,
    /// and the pre-0.4.0 legacy location.
    fn real_state_roots() -> Vec<PathBuf> {
        let mut roots = Vec::new();
        if let Some(xdg) = std::env::var_os("XDG_STATE_HOME") {
            roots.push(PathBuf::from(xdg).join("nevi"));
        }
        if let Some(home) = dirs::home_dir() {
            roots.push(home.join(".local").join("state").join("nevi"));
        }
        if let Some(config) = dirs::config_dir() {
            roots.push(config.join("nevi"));
        }
        roots
    }

    #[test]
    fn test_builds_never_resolve_a_real_state_path() {
        let resolved = [
            state_dir(),
            state_file_path(),
            state_file("command_history.txt"),
            state_file("frecency.json"),
            state_file("recent_files.json"),
        ];
        for path in &resolved {
            assert!(!path.exists(), "test state path exists: {}", path.display());
            for root in real_state_roots() {
                assert!(
                    !path.starts_with(&root),
                    "test state path {} is inside real state {}",
                    path.display(),
                    root.display()
                );
            }
        }
        assert!(
            open_private(&state_dir().join("command_history.txt")).is_err(),
            "test state must not be writable"
        );
    }

    fn temp_state_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "nevi_shada_{tag}_{}_{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn state_file_resolution_prefers_new_then_falls_back_to_legacy() {
        let new = temp_state_path("resolve_new");
        let legacy = temp_state_path("resolve_legacy");

        // Neither exists: resolve to the new path (fresh install).
        assert_eq!(resolve_state_file(new.clone(), legacy.clone()), new);

        // Only the legacy file exists: read from it (pre-0.4.0 install).
        std::fs::write(&legacy, "{}").unwrap();
        assert_eq!(resolve_state_file(new.clone(), legacy.clone()), legacy);

        // Both exist: the new path wins (migration already happened).
        std::fs::write(&new, "{}").unwrap();
        assert_eq!(resolve_state_file(new.clone(), legacy.clone()), new);

        let _ = std::fs::remove_file(&new);
        let _ = std::fs::remove_file(&legacy);
    }

    fn sample_state() -> ShadaState {
        let mut state = ShadaState::default();
        state.macros.insert('a', "0f,ci\"hi<Esc>".to_string());
        state.registers.insert(
            'r',
            RegisterEntry {
                text: "yanked line\n".to_string(),
                linewise: true,
            },
        );
        state.unnamed_register = Some(RegisterEntry {
            text: "word".to_string(),
            linewise: false,
        });
        state.global_marks.insert(
            'A',
            MarkEntry {
                path: PathBuf::from("/tmp/somewhere.rs"),
                line: 41,
                col: 3,
            },
        );
        state.search_history = vec!["foo".to_string(), "bar".to_string()];
        state.numbered_registers.insert(
            '1',
            RegisterEntry {
                text: "deleted line\n".to_string(),
                linewise: true,
            },
        );
        state.jumplist = vec![
            MarkEntry {
                path: PathBuf::from("/tmp/first.rs"),
                line: 5,
                col: 0,
            },
            MarkEntry {
                path: PathBuf::from("/tmp/second.rs"),
                line: 20,
                col: 7,
            },
        ];
        state
    }

    #[test]
    fn state_roundtrips_through_disk() {
        let path = temp_state_path("roundtrip");
        let state = sample_state();

        save_to(&path, &state).expect("save");
        let loaded = load_from(&path);

        assert_eq!(loaded.version, FORMAT_VERSION);
        assert_eq!(loaded.macros, state.macros);
        assert_eq!(loaded.registers, state.registers);
        assert_eq!(loaded.unnamed_register, state.unnamed_register);
        assert_eq!(loaded.global_marks, state.global_marks);
        assert_eq!(loaded.search_history, state.search_history);
        assert_eq!(loaded.numbered_registers, state.numbered_registers);
        assert_eq!(loaded.jumplist, state.jumplist);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn caps_drop_invalid_numbered_keys_and_trim_jumplist() {
        let path = temp_state_path("v2caps");
        let mut state = ShadaState::default();
        // Letters are not numbered registers; only "0-"9 may persist.
        for key in ['0', '5', 'x'] {
            state.numbered_registers.insert(
                key,
                RegisterEntry {
                    text: "n".to_string(),
                    linewise: false,
                },
            );
        }
        state.jumplist = (0..150)
            .map(|i| MarkEntry {
                path: PathBuf::from(format!("/tmp/f{i}.rs")),
                line: i,
                col: 0,
            })
            .collect();

        save_to(&path, &state).expect("save");
        let loaded = load_from(&path);

        assert_eq!(
            loaded.numbered_registers.keys().collect::<Vec<_>>(),
            vec![&'0', &'5']
        );
        assert_eq!(loaded.jumplist.len(), 100);
        assert_eq!(
            loaded.jumplist.last().map(|j| j.line),
            Some(149),
            "caps keep the newest jumps"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_and_corrupt_files_load_as_empty_state() {
        let missing = temp_state_path("missing");
        assert_eq!(load_from(&missing), ShadaState::default());

        let corrupt = temp_state_path("corrupt");
        std::fs::write(&corrupt, "{ not json !!").unwrap();
        assert_eq!(load_from(&corrupt), ShadaState::default());
        let _ = std::fs::remove_file(&corrupt);
    }

    #[test]
    fn caps_drop_oversized_registers_and_old_history() {
        let path = temp_state_path("caps");
        let mut state = sample_state();
        state.registers.insert(
            'h',
            RegisterEntry {
                text: "x".repeat(MAX_REGISTER_TEXT_CHARS + 1),
                linewise: false,
            },
        );
        state.search_history = (0..(MAX_HISTORY_ENTRIES + 25))
            .map(|i| format!("query {i}"))
            .collect();

        save_to(&path, &state).expect("save");
        let loaded = load_from(&path);

        assert!(!loaded.registers.contains_key(&'h'), "oversized dropped");
        assert!(loaded.registers.contains_key(&'r'), "normal kept");
        assert_eq!(loaded.search_history.len(), MAX_HISTORY_ENTRIES);
        assert_eq!(
            loaded.search_history.last().map(String::as_str),
            Some("query 124"),
            "caps keep the newest entries"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn invalid_hand_edited_keys_are_dropped_on_load() {
        let path = temp_state_path("badkeys");
        std::fs::write(
            &path,
            r#"{
                "version": 1,
                "macros": { "A": "x", "b": "dw" },
                "registers": { "Z": { "text": "no", "linewise": false } },
                "global_marks": { "q": { "path": "/x", "line": 0, "col": 0 } }
            }"#,
        )
        .unwrap();

        let loaded = load_from(&path);

        assert!(!loaded.macros.contains_key(&'A'), "macros are a-z only");
        assert!(loaded.macros.contains_key(&'b'));
        assert!(loaded.registers.is_empty(), "registers are a-z only");
        assert!(loaded.global_marks.is_empty(), "global marks are A-Z only");
        let _ = std::fs::remove_file(&path);
    }

    /// A state.json written by a NEWER nevi (higher version, fields this build
    /// doesn't know) must still load the fields we do understand, so a
    /// downgrade never wipes user state. Pins that ShadaState keeps serde's
    /// ignore-unknown-fields default and per-field defaults.
    ///
    /// Caution when growing the schema: a KNOWN field whose shape changes is
    /// not tolerated — the whole file fails to parse and loads as empty (the
    /// v2 jumplist field broke this test's fake future data exactly that
    /// way). New capabilities must be new fields, never reshaped old ones.
    #[test]
    fn future_version_file_loads_known_fields() {
        let path = temp_state_path("future");
        std::fs::write(
            &path,
            r#"{
                "version": 99,
                "macros": { "a": "dw" },
                "window_layouts": [{ "panes": 2 }],
                "registers": { "b": { "text": "kept", "linewise": false, "shape": "block" } }
            }"#,
        )
        .unwrap();

        let loaded = load_from(&path);

        assert_eq!(loaded.macros.get(&'a').map(String::as_str), Some("dw"));
        assert_eq!(
            loaded.registers.get(&'b').map(|r| r.text.as_str()),
            Some("kept"),
            "unknown nested fields must not reject the entry"
        );
        assert!(loaded.search_history.is_empty(), "absent fields default");
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(unix)]
    mod private_mode {
        use super::super::{open_private, save_to};
        use super::{sample_state, temp_state_path};
        use crate::shada::load_from;
        use std::fs;
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        use std::path::Path;

        fn mode(path: &Path) -> u32 {
            fs::metadata(path).unwrap().permissions().mode() & 0o777
        }

        fn set_mode(path: &Path, mode: u32) {
            fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
        }

        #[test]
        fn fresh_state_file_is_private_in_a_private_new_dir() {
            let root = temp_state_path("fresh_dir");
            let dir = root.join("nevi");
            let path = dir.join("state.json");

            save_to(&path, &sample_state()).unwrap();

            assert_eq!(mode(&path), 0o600);
            assert_eq!(mode(&dir), 0o700, "a state dir Nevi creates is private");
            assert_eq!(load_from(&path).registers, sample_state().registers);
            let _ = fs::remove_dir_all(&root);
        }

        #[test]
        fn wide_state_file_is_tightened_and_existing_dir_keeps_its_mode() {
            let dir = temp_state_path("wide_file");
            fs::create_dir_all(&dir).unwrap();
            set_mode(&dir, 0o755);
            let path = dir.join("state.json");
            fs::write(&path, "{}").unwrap();
            set_mode(&path, 0o644);

            save_to(&path, &sample_state()).unwrap();

            assert_eq!(mode(&path), 0o600);
            assert_eq!(load_from(&path).registers, sample_state().registers);
            assert_eq!(mode(&dir), 0o755, "an existing dir is left alone");
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn stale_wide_temp_file_does_not_widen_state_file() {
            let dir = temp_state_path("stale_tmp");
            fs::create_dir_all(&dir).unwrap();
            let path = dir.join("state.json");
            let tmp = dir.join("state.json.tmp");
            fs::write(&tmp, "stale").unwrap();
            set_mode(&tmp, 0o644);

            save_to(&path, &sample_state()).unwrap();

            assert_eq!(mode(&path), 0o600);
            assert!(!tmp.exists(), "the temp file is renamed into place");
            assert_eq!(load_from(&path).registers, sample_state().registers);
            let _ = fs::remove_dir_all(&dir);
        }

        /// The helper hands back the file before any content is written, so
        /// the mode it reports here is the mode the content goes into.
        #[test]
        fn open_private_is_private_before_content_goes_in() {
            let dir = temp_state_path("open_private");
            let fresh = dir.join("sub").join("command_history.txt");

            let mut file = open_private(&fresh).unwrap();
            assert_eq!(mode(&fresh), 0o600, "new file is private from creation");
            assert_eq!(mode(fresh.parent().unwrap()), 0o700);
            file.write_all(b"one\n").unwrap();
            drop(file);

            let wide = dir.join("frecency.json");
            fs::write(&wide, "old secret, much longer than the new text").unwrap();
            set_mode(&wide, 0o644);

            let mut file = open_private(&wide).unwrap();
            assert_eq!(mode(&wide), 0o600, "tightened before new content");
            assert_eq!(fs::metadata(&wide).unwrap().len(), 0, "old content is gone");
            file.write_all(b"new").unwrap();
            drop(file);

            assert_eq!(fs::read_to_string(&wide).unwrap(), "new");
            assert_eq!(fs::read_to_string(&fresh).unwrap(), "one\n");
            let _ = fs::remove_dir_all(&dir);
        }
    }
}
