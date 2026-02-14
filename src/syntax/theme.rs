use crossterm::style::Color;
use std::collections::HashMap;

/// Syntax highlighting style (color + attributes)
#[derive(Debug, Clone, Copy)]
pub struct SyntaxStyle {
    pub fg: Color,
    pub bold: bool,
    pub italic: bool,
}

impl SyntaxStyle {
    pub fn new(fg: Color) -> Self {
        Self { fg, bold: false, italic: false }
    }

    pub fn with_italic(mut self) -> Self {
        self.italic = true;
        self
    }

    pub fn with_bold(mut self) -> Self {
        self.bold = true;
        self
    }
}

/// Highlight group names used by tree-sitter queries
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HighlightGroup {
    Keyword,
    Function,
    Type,
    String,
    Number,
    Comment,
    Operator,
    Punctuation,
    Variable,
    Constant,
    Attribute,
    Namespace,
    Label,
    Property,
    Tag,
    Embedded, // For embedded expressions like ${} in template strings
    // New groups for improved Rust highlighting
    Macro,       // format!, println!
    Method,      // .clone(), .ok()
    Constructor, // Some, None, Ok, Err
    Boolean,     // true, false
}

impl HighlightGroup {
    /// Parse a tree-sitter capture name to a highlight group
    pub fn from_capture_name(name: &str) -> Option<Self> {
        // Check exact hierarchical matches first for specialized groups
        match name {
            "function.macro" => return Some(Self::Macro),
            "function.method" => return Some(Self::Method),
            "constructor" => return Some(Self::Constructor),
            "boolean" => return Some(Self::Boolean),
            _ => {}
        }

        // Handle hierarchical names like "keyword.control" -> Keyword
        let base = name.split('.').next()?;

        match base {
            "keyword" => Some(Self::Keyword),
            "function" => Some(Self::Function),
            "type" => Some(Self::Type),
            "string" => Some(Self::String),
            "number" => Some(Self::Number),
            "comment" => Some(Self::Comment),
            "operator" => Some(Self::Operator),
            "punctuation" => Some(Self::Punctuation),
            "variable" => Some(Self::Variable),
            "constant" => Some(Self::Constant),
            "attribute" => Some(Self::Attribute),
            "namespace" => Some(Self::Namespace),
            "label" => Some(Self::Label),
            "property" => Some(Self::Property),
            "tag" => Some(Self::Tag),
            "embedded" => Some(Self::Embedded),
            "constructor" => Some(Self::Constructor),
            "boolean" => Some(Self::Boolean),
            _ => None,
        }
    }
}

/// A syntax highlighting theme
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    styles: HashMap<HighlightGroup, SyntaxStyle>,
}

impl Theme {
    /// Create the default "One Dark" inspired theme
    pub fn default_theme() -> Self {
        let mut styles = HashMap::new();

        // One Dark inspired colors
        styles.insert(HighlightGroup::Keyword, SyntaxStyle::new(Color::Rgb { r: 198, g: 120, b: 221 }));    // Purple
        styles.insert(HighlightGroup::Function, SyntaxStyle::new(Color::Rgb { r: 97, g: 175, b: 239 }));    // Blue
        styles.insert(HighlightGroup::Type, SyntaxStyle::new(Color::Rgb { r: 229, g: 192, b: 123 }));       // Yellow
        styles.insert(HighlightGroup::String, SyntaxStyle::new(Color::Rgb { r: 152, g: 195, b: 121 }));     // Green
        styles.insert(HighlightGroup::Number, SyntaxStyle::new(Color::Rgb { r: 209, g: 154, b: 102 }));     // Orange
        styles.insert(HighlightGroup::Comment, SyntaxStyle::new(Color::Rgb { r: 92, g: 99, b: 112 }).with_italic());  // Gray, italic
        styles.insert(HighlightGroup::Operator, SyntaxStyle::new(Color::Rgb { r: 86, g: 182, b: 194 }));    // Cyan
        styles.insert(HighlightGroup::Punctuation, SyntaxStyle::new(Color::Rgb { r: 171, g: 178, b: 191 })); // Light gray
        styles.insert(HighlightGroup::Variable, SyntaxStyle::new(Color::Rgb { r: 224, g: 108, b: 117 }));   // Red
        styles.insert(HighlightGroup::Constant, SyntaxStyle::new(Color::Rgb { r: 209, g: 154, b: 102 }));   // Orange
        styles.insert(HighlightGroup::Attribute, SyntaxStyle::new(Color::Rgb { r: 229, g: 192, b: 123 }));  // Yellow
        styles.insert(HighlightGroup::Namespace, SyntaxStyle::new(Color::Rgb { r: 97, g: 175, b: 239 }));   // Blue
        styles.insert(HighlightGroup::Label, SyntaxStyle::new(Color::Rgb { r: 224, g: 108, b: 117 }));      // Red
        styles.insert(HighlightGroup::Property, SyntaxStyle::new(Color::Rgb { r: 224, g: 108, b: 117 }));   // Red
        styles.insert(HighlightGroup::Tag, SyntaxStyle::new(Color::Rgb { r: 224, g: 108, b: 117 }));        // Red (JSX/HTML tags)
        styles.insert(HighlightGroup::Embedded, SyntaxStyle::new(Color::Rgb { r: 86, g: 182, b: 194 }));    // Cyan (template string interpolations)
        // New groups for improved Rust highlighting
        styles.insert(HighlightGroup::Macro, SyntaxStyle::new(Color::Rgb { r: 86, g: 182, b: 194 }));       // Cyan
        styles.insert(HighlightGroup::Method, SyntaxStyle::new(Color::Rgb { r: 97, g: 175, b: 239 }));      // Blue
        styles.insert(HighlightGroup::Constructor, SyntaxStyle::new(Color::Rgb { r: 86, g: 182, b: 194 })); // Cyan
        styles.insert(HighlightGroup::Boolean, SyntaxStyle::new(Color::Rgb { r: 209, g: 154, b: 102 }));    // Orange

        Self {
            name: "default".to_string(),
            styles,
        }
    }

    /// Get the style for a highlight group
    pub fn get_style(&self, group: HighlightGroup) -> Option<SyntaxStyle> {
        self.styles.get(&group).copied()
    }

    /// Get the color for a highlight group (for backwards compatibility)
    pub fn get_color(&self, group: HighlightGroup) -> Option<Color> {
        self.styles.get(&group).map(|s| s.fg)
    }

    /// Get the style for a capture name
    pub fn get_style_for_capture(&self, capture_name: &str) -> Option<SyntaxStyle> {
        HighlightGroup::from_capture_name(capture_name)
            .and_then(|group| self.get_style(group))
    }

    /// Get the color for a capture name (for backwards compatibility)
    pub fn get_color_for_capture(&self, capture_name: &str) -> Option<Color> {
        HighlightGroup::from_capture_name(capture_name)
            .and_then(|group| self.get_color(group))
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::default_theme()
    }
}

impl Theme {
    /// Create a syntax theme from the UI theme system
    pub fn from_ui_theme(ui_theme: &crate::theme::Theme) -> Self {
        let mut styles = HashMap::new();

        // Helper to convert StyleDef to SyntaxStyle
        let convert = |def: &crate::theme::StyleDef| SyntaxStyle {
            fg: def.fg,
            bold: def.bold,
            italic: def.italic,
        };

        styles.insert(HighlightGroup::Keyword, convert(&ui_theme.syntax.keyword));
        styles.insert(HighlightGroup::Function, convert(&ui_theme.syntax.function));
        styles.insert(HighlightGroup::Type, convert(&ui_theme.syntax.type_));
        styles.insert(HighlightGroup::String, convert(&ui_theme.syntax.string));
        styles.insert(HighlightGroup::Number, convert(&ui_theme.syntax.number));
        styles.insert(HighlightGroup::Comment, convert(&ui_theme.syntax.comment));
        styles.insert(HighlightGroup::Operator, convert(&ui_theme.syntax.operator));
        styles.insert(HighlightGroup::Punctuation, convert(&ui_theme.syntax.punctuation));
        styles.insert(HighlightGroup::Variable, convert(&ui_theme.syntax.variable));
        styles.insert(HighlightGroup::Constant, convert(&ui_theme.syntax.constant));
        styles.insert(HighlightGroup::Attribute, convert(&ui_theme.syntax.attribute));
        styles.insert(HighlightGroup::Namespace, convert(&ui_theme.syntax.namespace));
        styles.insert(HighlightGroup::Label, convert(&ui_theme.syntax.label));
        styles.insert(HighlightGroup::Property, convert(&ui_theme.syntax.property));
        styles.insert(HighlightGroup::Tag, convert(&ui_theme.syntax.tag));
        styles.insert(HighlightGroup::Embedded, convert(&ui_theme.syntax.embedded));
        // New groups
        styles.insert(HighlightGroup::Macro, convert(&ui_theme.syntax.macro_));
        styles.insert(HighlightGroup::Method, convert(&ui_theme.syntax.method));
        styles.insert(HighlightGroup::Constructor, convert(&ui_theme.syntax.constructor));
        styles.insert(HighlightGroup::Boolean, convert(&ui_theme.syntax.boolean));

        Self {
            name: ui_theme.name.clone(),
            styles,
        }
    }
}
