//! Node data model: `MindNode` and the small structs that travel with
//! it — position, size, text runs, node style, layout, colour schema,
//! and the glyph-border config. Borders belong here because they are
//! always per-node (no edge-level borders exist).

use serde::{Deserialize, Serialize};

use crate::mindmap::custom_mutation::{CustomMutation, TriggerBinding};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MindNode {
    pub id: String,
    pub parent_id: Option<String>,
    pub index: i32,
    pub position: Position,
    pub size: Size,
    pub text: String,
    pub text_runs: Vec<TextRun>,
    pub style: NodeStyle,
    pub layout: NodeLayout,
    pub folded: bool,
    pub notes: String,
    pub color_schema: Option<ColorSchema>,
    /// Trigger bindings attached to this specific node.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trigger_bindings: Vec<TriggerBinding>,
    /// Inline custom mutations defined on this node (not shared with other nodes).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inline_mutations: Vec<CustomMutation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextRun {
    pub start: usize,
    pub end: usize,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub font: String,
    pub size_pt: u32,
    pub color: String,
    pub hyperlink: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeStyle {
    pub background_color: String,
    pub frame_color: String,
    pub text_color: String,
    pub shape_type: i32,
    pub corner_radius_percent: f64,
    pub frame_thickness: f64,
    pub show_frame: bool,
    pub show_shadow: bool,
    /// Glyph-based border configuration. Optional — if absent, the renderer
    /// applies a default border style based on the node's frame_color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<GlyphBorderConfig>,
}

/// Configures how a node's border is rendered using font glyphs.
/// All fields are optional with sensible defaults so the format stays forgiving.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlyphBorderConfig {
    /// Which glyph preset to use: "light", "heavy", "double", "rounded", or "custom"
    #[serde(default = "default_border_preset")]
    pub preset: String,
    /// Font family name for border glyphs. None = system default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// Font size in points for border glyphs.
    #[serde(default = "default_border_font_size")]
    pub font_size_pt: f32,
    /// Border color override as #RRGGBB. None = inherit from frame_color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Custom glyph definitions. Only used when preset = "custom".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glyphs: Option<CustomBorderGlyphs>,
    /// Padding between border and content (in pixels).
    #[serde(default = "default_border_padding")]
    pub padding: f32,
}

fn default_border_preset() -> String { "rounded".to_string() }
fn default_border_font_size() -> f32 { 14.0 }
fn default_border_padding() -> f32 { 4.0 }

/// Custom glyphs for each part of the border.
/// Each field is a string (single char or multi-char glyph).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomBorderGlyphs {
    #[serde(default = "default_h_glyph")]
    pub top: String,
    #[serde(default = "default_h_glyph")]
    pub bottom: String,
    #[serde(default = "default_v_glyph")]
    pub left: String,
    #[serde(default = "default_v_glyph")]
    pub right: String,
    #[serde(default = "default_tl_glyph")]
    pub top_left: String,
    #[serde(default = "default_tr_glyph")]
    pub top_right: String,
    #[serde(default = "default_bl_glyph")]
    pub bottom_left: String,
    #[serde(default = "default_br_glyph")]
    pub bottom_right: String,
}

fn default_h_glyph() -> String { "\u{2500}".to_string() }
fn default_v_glyph() -> String { "\u{2502}".to_string() }
fn default_tl_glyph() -> String { "\u{256D}".to_string() }
fn default_tr_glyph() -> String { "\u{256E}".to_string() }
fn default_bl_glyph() -> String { "\u{2570}".to_string() }
fn default_br_glyph() -> String { "\u{256F}".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeLayout {
    #[serde(rename = "type")]
    pub layout_type: i32,
    pub direction: i32,
    pub spacing: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorSchema {
    pub level: i32,
    pub palette: String,
    pub variant: i32,
    pub starts_at_root: bool,
    pub connections_colored: bool,
    pub theme_id: String,
    pub groups: Vec<ColorGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorGroup {
    pub background: String,
    pub frame: String,
    pub text: String,
    pub title: String,
}
