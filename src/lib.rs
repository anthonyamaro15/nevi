pub mod commands;
pub mod config;
pub mod copilot;
pub mod editor;
pub mod explorer;
pub mod finder;
pub mod floating_terminal;
pub mod formatter;
pub mod frecency;
pub mod git;
pub mod harpoon;
pub mod indent;
pub mod input;
pub mod lsp;
pub mod markdown_preview;
pub mod perf;
pub mod syntax;
pub mod terminal;
pub mod theme;

pub use config::{
    load_config, AutosaveMode, CopilotSettings, KeymapLookup, LeaderAction, Settings,
};
pub use config::{load_languages_config, FormatterConfig, LanguagesConfig};
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
    render_markdown, MarkdownPreview, MarkdownPreviewState, PreviewLine, PreviewLineKind,
    PreviewSpan, PreviewSpanStyle,
};
pub use terminal::Terminal;
pub use theme::{Theme, ThemeManager};
