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

    /// Generates a vertical side column of `rows` rows, using
    /// `self.left` as the glyph. Rows are separated by `'\n'`, and
    /// the returned string ends without a trailing newline — one
    /// glyph per line cell, `rows` lines total.
    ///
    /// Callers that want the right side can either use this same
    /// string (since the rounded/light presets have `left == right`)
    /// or call `right_side_border` below for an explicit right column.
    ///
    /// Cost: O(rows) push operations, one allocation sized to
    /// `rows * (left.len_utf8() + 1)`.
    pub fn side_border(&self, rows: usize) -> String {
        build_side_column(self.left, rows)
    }

    /// Like [`side_border`] but uses `self.right`. Presets where
    /// `left == right` can call either — this exists so callers
    /// never need to know which preset they have.
    pub fn right_side_border(&self, rows: usize) -> String {
        build_side_column(self.right, rows)
    }
}

fn build_side_column(glyph: char, rows: usize) -> String {
    if rows == 0 {
        return String::new();
    }
    let glyph_len = glyph.len_utf8();
    let mut s = String::with_capacity(rows * (glyph_len + 1) - 1);
    for i in 0..rows {
        s.push(glyph);
        if i + 1 < rows {
            s.push('\n');
        }
    }
    s
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

// -----------------------------------------------------------------
// Tests
//
// Border string generation is on every scene-rebuild hot path: one
// call to `top_border` / `bottom_border` per framed node, per frame.
// The loops look trivial today but are easy to break in ways that
// either quietly misalign corners or accidentally go quadratic. These
// tests double as perf regression guards.
// -----------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    /// The light preset's top border at width 5 is corners + 3 fill
    /// characters. Structural invariant: first char is `top_left`, last
    /// is `top_right`, all middle chars equal `top`.
    #[test]
    fn test_top_border_light_basic_shape() {
        let glyphs = BorderGlyphSet::box_drawing_light();
        let border = glyphs.top_border(5);
        assert_eq!(border, "\u{250C}\u{2500}\u{2500}\u{2500}\u{2510}");
        let chars: Vec<char> = border.chars().collect();
        assert_eq!(chars.len(), 5);
        assert_eq!(chars[0], glyphs.top_left);
        assert_eq!(chars[4], glyphs.top_right);
        for c in &chars[1..4] {
            assert_eq!(*c, glyphs.top);
        }
    }

    /// Widths below 2 have no room for both corners, so the function
    /// returns an empty string. Guards the early-return branch.
    #[test]
    fn test_top_border_width_under_two_is_empty() {
        let glyphs = BorderGlyphSet::box_drawing_light();
        assert_eq!(glyphs.top_border(0), "");
        assert_eq!(glyphs.top_border(1), "");
        assert_eq!(glyphs.bottom_border(0), "");
        assert_eq!(glyphs.bottom_border(1), "");
    }

    /// The bottom border must use the `bottom_*` corners, not the
    /// `top_*` corners. Copy-paste slip guard.
    #[test]
    fn test_bottom_border_uses_bottom_corners() {
        let glyphs = BorderGlyphSet::box_drawing_light();
        let border = glyphs.bottom_border(4);
        let chars: Vec<char> = border.chars().collect();
        assert_eq!(chars.len(), 4);
        assert_eq!(chars[0], glyphs.bottom_left);
        assert_eq!(chars[3], glyphs.bottom_right);
        assert_ne!(chars[0], glyphs.top_left);
        assert_ne!(chars[3], glyphs.top_right);
    }

    /// Every preset must produce a length-N string for width N ≥ 2 on
    /// both top and bottom. Catches a preset accidentally missing a
    /// glyph field (serde would default it to `'\0'`, which would still
    /// produce a length-N string — so also spot-check the first char is
    /// non-null).
    #[test]
    fn test_all_four_presets_produce_non_empty_borders() {
        let presets = [
            BorderGlyphSet::box_drawing_light(),
            BorderGlyphSet::box_drawing_heavy(),
            BorderGlyphSet::box_drawing_double(),
            BorderGlyphSet::box_drawing_rounded(),
        ];
        for glyphs in &presets {
            let top = glyphs.top_border(6);
            let bottom = glyphs.bottom_border(6);
            assert_eq!(top.chars().count(), 6);
            assert_eq!(bottom.chars().count(), 6);
            assert_ne!(top.chars().next().unwrap(), '\0');
            assert_ne!(bottom.chars().next().unwrap(), '\0');
            assert_ne!(glyphs.left_char(), '\0');
            assert_ne!(glyphs.right_char(), '\0');
        }
    }

    /// `top_border(10_000)` must succeed without panic and produce
    /// exactly 10,000 characters. Guards against accidental integer
    /// overflow on `char_width.saturating_sub(2)` or a quadratic
    /// string-growth refactor.
    #[test]
    fn test_top_border_large_width_no_panic() {
        let glyphs = BorderGlyphSet::box_drawing_light();
        let border = glyphs.top_border(10_000);
        assert_eq!(border.chars().count(), 10_000);
        // First and last are still corners, not middle fill.
        let chars: Vec<char> = border.chars().collect();
        assert_eq!(chars[0], glyphs.top_left);
        assert_eq!(chars[9_999], glyphs.top_right);
    }

    /// `side_border(rows)` emits exactly `rows` glyphs separated by
    /// newlines — one glyph per logical row. Guards against an
    /// off-by-one on the trailing newline.
    #[test]
    fn test_side_border_exact_row_count() {
        let glyphs = BorderGlyphSet::box_drawing_rounded();
        assert_eq!(glyphs.side_border(0), "");
        assert_eq!(glyphs.side_border(1), "│");
        assert_eq!(glyphs.side_border(3), "│\n│\n│");
        // Each of the 3 rows is exactly the `left` char, no more.
        let border = glyphs.side_border(5);
        assert_eq!(border.lines().count(), 5);
        for line in border.lines() {
            assert_eq!(line.chars().count(), 1);
            assert_eq!(line.chars().next().unwrap(), glyphs.left);
        }
    }

    /// Right-side helper uses `self.right`; for the rounded preset
    /// that's the same as `left`, but the API keeps them distinct so
    /// callers don't have to know.
    #[test]
    fn test_right_side_border_uses_right_glyph() {
        let glyphs = BorderGlyphSet::box_drawing_rounded();
        let border = glyphs.right_side_border(4);
        for line in border.lines() {
            assert_eq!(line.chars().next().unwrap(), glyphs.right);
        }
    }

    /// `BorderStyle::default_with_color` is what the scene builder
    /// constructs for every framed node. Spot-check its fields.
    #[test]
    fn test_border_style_default_with_color() {
        let style = BorderStyle::default_with_color("#ff0000");
        assert_eq!(style.color, "#ff0000");
        assert!(style.visible);
        // Default preset is rounded.
        assert_eq!(
            style.glyph_set.top_left,
            BorderGlyphSet::box_drawing_rounded().top_left
        );
        assert_eq!(style.font_name, None);
    }
}
