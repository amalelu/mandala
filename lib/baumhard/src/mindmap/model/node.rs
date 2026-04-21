//! Node data model: `MindNode` and the small structs that travel with
//! it — position, size, text runs, node style, layout, colour schema,
//! and the glyph-border config. Borders belong here because they are
//! always per-node (no edge-level borders exist).

use serde::{Deserialize, Serialize};

use crate::mindmap::custom_mutation::{CustomMutation, TriggerBinding};

/// A single node in the mindmap: one rectangle of text + style,
/// attached to a tree position via [`Self::parent_id`] and a
/// Dewey-decimal [`Self::id`]. The loader materializes one of these
/// per `.mindmap.json` entry; the scene builder and tree builder
/// both project from this shape.
///
/// Plain data; no runtime cost beyond the `String` allocations serde
/// performs on deserialize.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MindNode {
    pub id: String,
    pub parent_id: Option<String>,
    pub position: Position,
    pub size: Size,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text_runs: Vec<TextRun>,
    pub style: NodeStyle,
    pub layout: NodeLayout,
    pub folded: bool,
    pub notes: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_schema: Option<ColorSchema>,
    /// Channel index for mutation targeting in the baumhard tree.
    /// Multiple siblings can share a channel to form broadcast groups.
    #[serde(default)]
    pub channel: usize,
    /// Trigger bindings attached to this specific node.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trigger_bindings: Vec<TriggerBinding>,
    /// Inline custom mutations defined on this node (not shared with other nodes).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inline_mutations: Vec<CustomMutation>,
    /// Lower bound on `camera.zoom` at which this node (and its
    /// glyph border, which inherits from the node) renders.
    /// `None` = unbounded below. Mirrors the
    /// `min_font_size_pt` / `max_font_size_pt` pair on
    /// [`crate::mindmap::model::edge::GlyphConnectionConfig`] —
    /// same flat-optional posture, orthogonal concept (presence
    /// vs. size). Inclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_zoom_to_render: Option<f32>,
    /// Upper bound on `camera.zoom` at which this node renders.
    /// `None` = unbounded above. Inclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_zoom_to_render: Option<f32>,
}

/// Canvas-space top-left corner of a node's AABB. Units are
/// arbitrary canvas pixels (the camera transforms to screen space at
/// render time). Plain data; no runtime cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

/// Canvas-space extent of a node's AABB. Width and height are
/// strictly positive in practice but not checked at type level —
/// scene-builder code guards against zero-size nodes on its own.
/// Plain data; no runtime cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

/// A styled slice of a node's `text`, matching miMind's text-run
/// concept: `[start, end)` byte indices carry one font / size /
/// color / style combination, with optional hyperlink target.
/// Multiple runs describe a single multi-style string; gaps in
/// coverage render with node-level defaults.
///
/// Plain data; no runtime cost beyond the string allocations.
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

/// Visual style for one node's frame / background / text. Colors are
/// raw `#RRGGBB` or `var(--name)` strings — callers pass them through
/// `util::color::resolve_var` against the canvas theme map before
/// rasterizing. Plain data; no runtime cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeStyle {
    pub background_color: String,
    pub frame_color: String,
    pub text_color: String,
    #[serde(default = "default_shape")]
    pub shape: String,
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

fn default_shape() -> String { "rectangle".to_string() }
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

/// Descriptor for how this node arranges its children — a
/// miMind-compat record carried through for round-trip fidelity.
/// Mandala does not currently drive layout from these fields;
/// custom mutations (see `format/mutations.md`) are the active
/// layout mechanism. Plain data; no runtime cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeLayout {
    #[serde(rename = "type")]
    pub layout_type: String,
    pub direction: String,
    pub spacing: f64,
}

/// Links a node to one entry in a named [`super::Palette`] keyed by
/// depth. `level` is the index into the palette's `groups`; clamped
/// at theme-resolve time (`resolve_theme_colors`) so a schema
/// referencing a level beyond the palette's length falls back to
/// the last group rather than erroring.
/// `starts_at_root` and `connections_colored` are round-tripped
/// miMind-compat flags that the renderer interprets when resolving
/// effective colors. Plain data; no runtime cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorSchema {
    pub palette: String,
    pub level: i32,
    pub starts_at_root: bool,
    pub connections_colored: bool,
}

/// One palette entry — the four colors a themed node inherits at a
/// given depth level. Referenced from [`ColorSchema::level`] via
/// [`super::Palette::groups`]. Plain data; no runtime cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorGroup {
    pub background: String,
    pub frame: String,
    pub text: String,
    pub title: String,
}
