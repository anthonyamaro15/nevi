use super::Editor;
use crate::config::{FormatterConfig, LanguageConfig, LanguagesConfig};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) struct Fixture(pub PathBuf);
impl Fixture {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "nevi-save-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    pub fn file(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }
    pub fn editor(&self, name: &str) -> Editor {
        let mut editor = Editor::default();
        editor.languages_config = LanguagesConfig::default();
        editor.settings.editor.format_on_save = true;
        editor.open_file(self.file(name, "foo\n")).unwrap();
        editor
    }
    pub fn formatter(&self, editor: &mut Editor, command: &str, args: &[&str]) {
        editor.languages_config.languages.insert(
            "markdown".into(),
            LanguageConfig {
                formatter: Some(FormatterConfig {
                    command: command.into(),
                    args: args.iter().map(|s| s.to_string()).collect(),
                    timeout: 1,
                }),
                tab_width: None,
            },
        );
    }
}

/// A named pipe with no writer, so `open()` for reading would block on it.
#[cfg(unix)]
pub(crate) fn make_fifo(path: &Path) {
    let status = std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("run mkfifo");
    assert!(status.success(), "mkfifo {}", path.display());
}

/// Runs `read` and fails the test instead of hanging the suite when it
/// blocks on `fifo`. After the timeout, writers are opened and closed in
/// turn: each one lets a reader parked in `open()` through to end of file,
/// and a reader that opens the pipe again needs another.
#[cfg(unix)]
pub(crate) fn finish_without_blocking_on<T: Send + 'static>(
    fifo: &Path,
    read: impl FnOnce() -> T + Send + 'static,
) -> T {
    use std::os::unix::fs::OpenOptionsExt;
    use std::time::Duration;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(read());
    });
    if let Ok(value) = rx.recv_timeout(Duration::from_secs(5)) {
        return value;
    }
    for _ in 0..50 {
        let _ = std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(fifo);
        if rx.recv_timeout(Duration::from_millis(100)).is_ok() {
            break;
        }
    }
    panic!("reading {} blocked on a FIFO", fifo.display());
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
