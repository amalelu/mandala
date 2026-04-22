//! Node data model: `MindNode` and the small structs that travel with
//! it — position, size, text runs, node style, layout, colour schema,
//! and the glyph-border config. Borders belong here because they are
//! always per-node (no edge-level borders exist).

use serde::{Deserialize, Serialize};

use crate::gfx_structs::zoom_visibility::ZoomVisibility;
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
    /// Dewey-decimal node id (e.g. `"0"`, `"0.1"`, `"0.1.3"`); the
    /// parent prefix establishes the tree structure redundantly with
    /// [`Self::parent_id`]. Must be unique across the map.
    pub id: String,
    /// Parent node id, or `None` for the root. Source of truth for
    /// tree structure; [`Self::id`]'s Dewey prefix must agree.
    pub parent_id: Option<String>,
    /// Canvas-space top-left corner of the node's AABB.
    pub position: Position,
    /// Canvas-space width and height of the node's AABB.
    pub size: Size,
    /// Primary text content. Styled-slice overrides live in
    /// [`Self::text_runs`]; gaps inherit node-level defaults.
    pub text: String,
    /// Styled slices of [`Self::text`] — see [`TextRun`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text_runs: Vec<TextRun>,
    /// Background / frame / text colours, border, shape, and the
    /// visible-frame toggle.
    pub style: NodeStyle,
    /// Layout descriptor carried through from miMind-format source
    /// maps. Mandala drives layout through custom mutations instead
    /// (see `format/mutations.md`); this is round-trip fidelity only.
    pub layout: NodeLayout,
    /// `true` when the node's subtree is collapsed — children stay in
    /// the model but the scene builder treats them as hidden.
    pub folded: bool,
    /// Long-form text attached to the node; rendered separately from
    /// [`Self::text`] by the notes overlay path.
    pub notes: String,
    /// Optional palette binding that colours this node and its
    /// descendants at a given depth level.
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

impl MindNode {
    /// This node's authored zoom window, as a
    /// [`ZoomVisibility`]. O(1).
    ///
    /// # Border inheritance
    ///
    /// Borders inherit this window verbatim via
    /// [`BorderNodeData::zoom_visibility`] in
    /// `tree_builder/border.rs` (stamped onto all four runs in
    /// `border_node_data`) and via the same field on
    /// `BorderElement` in `scene_builder/node_pass.rs` — both
    /// paths call this method directly. No separate
    /// per-border override exists today; the floating-frame-
    /// fragment case a non-inheriting border would produce is
    /// prevented by construction. A future
    /// `GlyphBorderConfig.min_zoom_to_render` field would need
    /// to revisit those two call sites together.
    ///
    /// [`BorderNodeData::zoom_visibility`]: crate::mindmap::tree_builder::BorderNodeData
    pub fn zoom_window(&self) -> ZoomVisibility {
        ZoomVisibility::from_pair(
            self.min_zoom_to_render,
            self.max_zoom_to_render,
        )
    }
}

/// Canvas-space top-left corner of a node's AABB. Units are
/// arbitrary canvas pixels (the camera transforms to screen space at
/// render time). Plain data; no runtime cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    /// Canvas-space x coordinate.
    pub x: f64,
    /// Canvas-space y coordinate (canvas y-axis grows downward).
    pub y: f64,
}

/// Canvas-space extent of a node's AABB. Width and height are
/// strictly positive in practice but not checked at type level —
/// scene-builder code guards against zero-size nodes on its own.
/// Plain data; no runtime cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Size {
    /// Width in canvas units.
    pub width: f64,
    /// Height in canvas units.
    pub height: f64,
}

/// A styled slice of a node's `text`, matching miMind's text-run
/// concept: `[start, end)` grapheme indices carry one font / size /
/// color / style combination, with optional hyperlink target.
/// Multiple runs describe a single multi-style string; gaps in
/// coverage render with node-level defaults.
///
/// Plain data; no runtime cost beyond the string allocations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextRun {
    /// Grapheme-cluster index where this run begins (inclusive).
    pub start: usize,
    /// Grapheme-cluster index where this run ends (exclusive).
    pub end: usize,
    /// Bold weight flag.
    pub bold: bool,
    /// Italic style flag.
    pub italic: bool,
    /// Underline decoration flag.
    pub underline: bool,
    /// Font-family name; matched against `AppFont` at layout time
    /// with a fallback for unrecognised families.
    pub font: String,
    /// Font size in points.
    pub size_pt: u32,
    /// `#RRGGBB` or `var(--name)` text colour.
    pub color: String,
    /// Optional hyperlink target URL; the renderer decorates the
    /// run's underline when set.
    pub hyperlink: Option<String>,
}

/// Visual style for one node's frame / background / text. Colors are
/// raw `#RRGGBB` or `var(--name)` strings — callers pass them through
/// `util::color::resolve_var` against the canvas theme map before
/// rasterizing. Plain data; no runtime cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeStyle {
    /// Fill colour (`#RRGGBB` or `var(--name)`).
    pub background_color: String,
    /// Border / frame colour (`#RRGGBB` or `var(--name)`).
    pub frame_color: String,
    /// Default text colour for the node's primary text (`#RRGGBB`
    /// or `var(--name)`).
    pub text_color: String,
    /// Background shape spelling — matched against
    /// [`crate::gfx_structs::shape::NodeShape::from_style_string`].
    /// Falls back to rectangle on unknown values.
    #[serde(default = "default_shape")]
    pub shape: String,
    /// Corner radius as a percentage of the smaller AABB dimension
    /// (0 = square corners).
    pub corner_radius_percent: f64,
    /// Frame stroke thickness in canvas units.
    pub frame_thickness: f64,
    /// When `true`, render the frame stroke at all.
    pub show_frame: bool,
    /// When `true`, render a drop shadow behind the node.
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
    /// Glyph used for the horizontal top edge.
    #[serde(default = "default_h_glyph")]
    pub top: String,
    /// Glyph used for the horizontal bottom edge.
    #[serde(default = "default_h_glyph")]
    pub bottom: String,
    /// Glyph used for the vertical left edge.
    #[serde(default = "default_v_glyph")]
    pub left: String,
    /// Glyph used for the vertical right edge.
    #[serde(default = "default_v_glyph")]
    pub right: String,
    /// Glyph used for the top-left corner.
    #[serde(default = "default_tl_glyph")]
    pub top_left: String,
    /// Glyph used for the top-right corner.
    #[serde(default = "default_tr_glyph")]
    pub top_right: String,
    /// Glyph used for the bottom-left corner.
    #[serde(default = "default_bl_glyph")]
    pub bottom_left: String,
    /// Glyph used for the bottom-right corner.
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
    /// Layout-algorithm name from the miMind format — round-tripped
    /// but not currently honoured by the renderer.
    #[serde(rename = "type")]
    pub layout_type: String,
    /// Growth direction hint carried through from miMind.
    pub direction: String,
    /// Inter-child spacing hint carried through from miMind.
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
    /// Named palette to bind this node's colours to — keys into
    /// [`super::MindMap::palettes`].
    pub palette: String,
    /// Index into the palette's `groups` for this node's depth.
    /// Clamped against the palette's length at resolve time so a
    /// level past the end falls back to the last group.
    pub level: i32,
    /// `true` when depth indexing begins at the root (level 0 is the
    /// root itself); `false` shifts the indexing so the root is
    /// transparent and children start at level 0.
    pub starts_at_root: bool,
    /// When `true`, outgoing connections inherit the palette's
    /// colour at this node's level instead of the edge's own colour.
    pub connections_colored: bool,
}

/// One palette entry — the four colors a themed node inherits at a
/// given depth level. Referenced from [`ColorSchema::level`] via
/// [`super::Palette::groups`]. Plain data; no runtime cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorGroup {
    /// Background-fill colour for a node at this level.
    pub background: String,
    /// Frame-stroke colour for a node at this level.
    pub frame: String,
    /// Text colour for a node at this level.
    pub text: String,
    /// First-line / title colour — overrides `text` for the first
    /// line of a node's text when present.
    pub title: String,
}
