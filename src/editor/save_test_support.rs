use super::Editor;
use crate::config::{FormatterConfig, LanguageConfig, LanguagesConfig};
use std::path::PathBuf;
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
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
