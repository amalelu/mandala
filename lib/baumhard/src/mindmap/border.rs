use serde::{Deserialize, Serialize};

/// Defines which glyphs to use for rendering a node's border.
/// Each field is a single character (glyph) from the selected font.
/// The border is rendered as positioned text elements around the node content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BorderGlyphSet {
    pub top: char,
    pub bottom: char,
    pub left: char,
    pub right: char,
    pub top_left: char,
    pub top_right: char,
    pub bottom_left: char,
    pub bottom_right: char,
}

impl BorderGlyphSet {
    /// Standard Unicode box-drawing characters (light lines)
    pub fn box_drawing_light() -> Self {
        BorderGlyphSet {
            top: '\u{2500}',        // ─
            bottom: '\u{2500}',     // ─
            left: '\u{2502}',       // │
            right: '\u{2502}',      // │
            top_left: '\u{250C}',   // ┌
            top_right: '\u{2510}',  // ┐
            bottom_left: '\u{2514}',// └
            bottom_right: '\u{2518}',// ┘
        }
    }

    /// Heavy box-drawing characters
    pub fn box_drawing_heavy() -> Self {
        BorderGlyphSet {
            top: '\u{2501}',        // ━
            bottom: '\u{2501}',     // ━
            left: '\u{2503}',       // ┃
            right: '\u{2503}',      // ┃
            top_left: '\u{250F}',   // ┏
            top_right: '\u{2513}',  // ┓
            bottom_left: '\u{2517}',// ┗
            bottom_right: '\u{251B}',// ┛
        }
    }

    /// Double-line box-drawing characters
    pub fn box_drawing_double() -> Self {
        BorderGlyphSet {
            top: '\u{2550}',        // ═
            bottom: '\u{2550}',     // ═
            left: '\u{2551}',       // ║
            right: '\u{2551}',      // ║
            top_left: '\u{2554}',   // ╔
            top_right: '\u{2557}',  // ╗
            bottom_left: '\u{255A}',// ╚
            bottom_right: '\u{255D}',// ╝
        }
    }

    /// Rounded box-drawing characters
    pub fn box_drawing_rounded() -> Self {
        BorderGlyphSet {
            top: '\u{2500}',        // ─
            bottom: '\u{2500}',     // ─
            left: '\u{2502}',       // │
            right: '\u{2502}',      // │
            top_left: '\u{256D}',   // ╭
            top_right: '\u{256E}',  // ╮
            bottom_left: '\u{2570}',// ╰
            bottom_right: '\u{256F}',// ╯
        }
    }

    /// Generates the top border string for a given width in characters.
    pub fn top_border(&self, char_width: usize) -> String {
        if char_width < 2 {
            return String::new();
        }
        let mut s = String::with_capacity(char_width);
        s.push(self.top_left);
        for _ in 0..char_width.saturating_sub(2) {
            s.push(self.top);
        }
        s.push(self.top_right);
        s
    }

    /// Generates the bottom border string for a given width in characters.
    pub fn bottom_border(&self, char_width: usize) -> String {
        if char_width < 2 {
            return String::new();
        }
        let mut s = String::with_capacity(char_width);
        s.push(self.bottom_left);
        for _ in 0..char_width.saturating_sub(2) {
            s.push(self.bottom);
        }
        s.push(self.bottom_right);
        s
    }

    /// Generates a left side character (repeated for each row).
    pub fn left_char(&self) -> char {
        self.left
    }

    /// Generates a right side character (repeated for each row).
    pub fn right_char(&self) -> char {
        self.right
    }
}

/// Configuration for how a node's border should be rendered.
/// This struct is intended to be attached per-node or as a global default,
/// and is the key extensibility point for the editing experience.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BorderStyle {
    pub glyph_set: BorderGlyphSet,
    /// The font to use for border glyphs. None means use default system font.
    pub font_name: Option<String>,
    /// Font size for border glyphs in points.
    pub font_size_pt: f32,
    /// Border color as #RRGGBB hex string.
    pub color: String,
    /// Whether to render this border at all.
    pub visible: bool,
}

impl BorderStyle {
    pub fn default_with_color(color: &str) -> Self {
        BorderStyle {
            glyph_set: BorderGlyphSet::box_drawing_rounded(),
            font_name: None,
            font_size_pt: 14.0,
            color: color.to_string(),
            visible: true,
        }
    }
}

impl Default for BorderStyle {
    fn default() -> Self {
        Self::default_with_color("#ffffff")
    }
}
