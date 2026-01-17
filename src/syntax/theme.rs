use crossterm::style::Color;
use std::collections::HashMap;

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
}

impl HighlightGroup {
    /// Parse a tree-sitter capture name to a highlight group
    pub fn from_capture_name(name: &str) -> Option<Self> {
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
            _ => None,
        }
    }
}

/// A syntax highlighting theme
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    colors: HashMap<HighlightGroup, Color>,
}

impl Theme {
    /// Create the default "One Dark" inspired theme
    pub fn default_theme() -> Self {
        let mut colors = HashMap::new();

        // One Dark inspired colors
        colors.insert(HighlightGroup::Keyword, Color::Rgb { r: 198, g: 120, b: 221 });    // Purple
        colors.insert(HighlightGroup::Function, Color::Rgb { r: 97, g: 175, b: 239 });    // Blue
        colors.insert(HighlightGroup::Type, Color::Rgb { r: 229, g: 192, b: 123 });       // Yellow
        colors.insert(HighlightGroup::String, Color::Rgb { r: 152, g: 195, b: 121 });     // Green
        colors.insert(HighlightGroup::Number, Color::Rgb { r: 209, g: 154, b: 102 });     // Orange
        colors.insert(HighlightGroup::Comment, Color::Rgb { r: 92, g: 99, b: 112 });      // Gray
        colors.insert(HighlightGroup::Operator, Color::Rgb { r: 86, g: 182, b: 194 });    // Cyan
        colors.insert(HighlightGroup::Punctuation, Color::Rgb { r: 171, g: 178, b: 191 }); // Light gray
        colors.insert(HighlightGroup::Variable, Color::Rgb { r: 224, g: 108, b: 117 });   // Red
        colors.insert(HighlightGroup::Constant, Color::Rgb { r: 209, g: 154, b: 102 });   // Orange
        colors.insert(HighlightGroup::Attribute, Color::Rgb { r: 229, g: 192, b: 123 });  // Yellow
        colors.insert(HighlightGroup::Namespace, Color::Rgb { r: 97, g: 175, b: 239 });   // Blue
        colors.insert(HighlightGroup::Label, Color::Rgb { r: 224, g: 108, b: 117 });      // Red
        colors.insert(HighlightGroup::Property, Color::Rgb { r: 224, g: 108, b: 117 });   // Red
        colors.insert(HighlightGroup::Tag, Color::Rgb { r: 224, g: 108, b: 117 });        // Red (JSX/HTML tags)

        Self {
            name: "default".to_string(),
            colors,
        }
    }

    /// Get the color for a highlight group
    pub fn get_color(&self, group: HighlightGroup) -> Option<Color> {
        self.colors.get(&group).copied()
    }

    /// Get the color for a capture name (convenience method)
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
