pub(crate) mod command_resolver;
pub mod commands;
pub mod config;
pub mod copilot;
pub mod editor;
pub mod explorer;
pub mod file_diff;
pub mod finder;
pub mod floating_terminal;
pub mod formatter;
pub mod frecency;
#[cfg(test)]
mod fuzz;
pub mod git;
pub mod harpoon;
pub mod health;
pub mod indent;
pub mod input;
#[cfg(test)]
mod keybind_coverage;
pub mod labeled_jump;
pub mod lsp;
pub mod macro_lens;
pub mod markdown_preview;
pub mod mouse;
#[cfg(test)]
mod parity_report;
pub mod perf;
pub mod project_replace;
pub mod render_damage;
pub mod statusline;
pub mod syntax;
pub mod terminal;
pub mod theme;
pub mod tool_installer;
pub mod ui_glyphs;
#[cfg(test)]
mod vim_oracle;

pub use config::{
    AutosaveMode, CopilotSettings, KeymapLookup, LeaderAction, Settings, load_config,
};
pub use config::{FormatterConfig, LanguagesConfig, load_languages_config};
pub use copilot::types::CopilotNotification;
pub use copilot::{CopilotManager, CopilotStatus, GhostTextState};
pub use editor::{
    Buffer, CopilotAction, CopilotGhostText, Cursor, Editor, LspAction, Mode, ThemePicker,
};
pub use explorer::FileExplorer;
pub use finder::{FinderMode, FloatingWindow, FuzzyFinder};
pub use floating_terminal::FloatingTerminal;
pub use frecency::FrecencyDb;
pub use harpoon::Harpoon;
pub use lsp::{LanguageId, LspManager, LspNotification, LspStatus, MultiLspManager};
pub use markdown_preview::{
    MarkdownPreview, MarkdownPreviewState, PreviewLine, PreviewLineKind, PreviewSpan,
    PreviewSpanStyle, render_markdown,
};
pub use render_damage::RenderDamage;
pub use terminal::Terminal;
pub use theme::{Theme, ThemeManager};
