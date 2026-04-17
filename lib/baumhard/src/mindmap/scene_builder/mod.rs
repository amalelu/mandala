//! Scene builder — projects a `MindMap` into a flat `RenderScene`
//! of per-element plain-data items (`TextElement`, `BorderElement`,
//! `ConnectionElement`, `PortalElement`, `ConnectionLabelElement`,
//! `EdgeHandleElement`) that the renderer walks into cosmic-text
//! buffers.
//!
//! Sharded by role so each file stays focused:
//! - [`mod@self`] — element structs, preview wrappers, `RenderScene`
//!   aggregate, edge-handle glyph constants, the public re-export
//!   surface.
//! - [`builder`] — `build_scene`, `build_scene_with_cache`, and the
//!   cache-less wrappers. Thin orchestrator; delegates to the
//!   role modules below.
//! - [`node_pass`] — emits `TextElement`s + `BorderElement`s + clip
//!   AABBs in a single walk over visible nodes.
//! - [`connection`] — connection body glyphs (with
//!   `SceneConnectionCache` fast/slow paths), edge-handle glyphs,
//!   and the `point_inside_any_node` clip predicate.
//! - [`label`] — connection labels with the inline-edit override
//!   + synthesize-if-empty pass.
//! - [`portal`] — two markers per edge with `display_mode = "portal"`.
//! - [`edge_handle`] — `build_edge_handles` helper (re-exported
//!   for external callers that hit-test handles without building
//!   a full scene).

use crate::mindmap::border::BorderStyle;
use crate::mindmap::model::TextRun;
use crate::mindmap::scene_cache::EdgeKey;
use crate::mindmap::SELECTION_HIGHLIGHT_HEX as SELECTED_EDGE_COLOR;

/// A transient, scene-build-only substitution of an edge's effective
/// color. Used by the inline color picker's hover preview so the edge
/// under the wheel reflects the in-flight HSV value **without** any
/// mutation to the committed model. One edge at a time (the picker is
/// modal) so a single Option is enough.
///
/// Applied after the normal "glyph_connection.color → edge.color →
/// canvas default" resolution path but **before** the selection
/// override, so a selected edge being previewed still renders cyan on
/// the body glyphs. The preview is visible on the connection label,
/// matching the pre-refactor behavior.
#[derive(Debug, Clone, Copy)]
pub struct EdgeColorPreview<'a> {
    pub edge_key: &'a EdgeKey,
    pub color: &'a str,
}

/// Portal equivalent of `EdgeColorPreview`. Matched against the
/// portal-mode edge's `EdgeKey`. A portal-mode edge and a line-mode
/// edge with identical endpoints and `edge_type` would share the
/// same key; since `display_mode` is not part of `EdgeKey`, that
/// collision never occurs in practice — portal and line edges with
/// matching endpoints are distinct by `edge_type`.
#[derive(Debug, Clone, Copy)]
pub struct PortalColorPreview<'a> {
    pub edge_key: &'a EdgeKey,
    pub color: &'a str,
}

/// Intermediate representation between MindMap data and GPU rendering.
/// Produced by `build_scene()`, consumed by Renderer to create cosmic-text buffers.
pub struct RenderScene {
    pub text_elements: Vec<TextElement>,
    pub border_elements: Vec<BorderElement>,
    pub connection_elements: Vec<ConnectionElement>,
    pub portal_elements: Vec<PortalElement>,
    /// Session 6C: grab-handles rendered on top of the *selected* edge.
    /// Always empty unless `selected_edge` was `Some` on the scene-build
    /// call. Contains the two anchor endpoints, any existing control
    /// points, and (for straight edges only) a midpoint handle that
    /// triggers the "curve a straight line" gesture when dragged.
    pub edge_handles: Vec<EdgeHandleElement>,
    /// Session 6D: labels attached to edges whose `label` field is
    /// non-empty. One element per labeled edge, positioned along the
    /// connection path at `edge.label_position_t` (defaulting to 0.5).
    /// Not cached in `SceneConnectionCache` — labels are ≤ 1 per edge
    /// and rebuilt each frame at trivial cost.
    pub connection_label_elements: Vec<ConnectionLabelElement>,
    pub background_color: String,
}

/// A visible text node to be rendered.
pub struct TextElement {
    pub node_id: String,
    pub text: String,
    pub text_runs: Vec<TextRun>,
    pub position: (f32, f32),
    pub size: (f32, f32),
}

/// A border to be rendered around a node.
pub struct BorderElement {
    pub node_id: String,
    pub border_style: BorderStyle,
    pub node_position: (f32, f32),
    pub node_size: (f32, f32),
}

/// A connection (edge) between two nodes, with pre-computed glyph positions.
pub struct ConnectionElement {
    /// Stable identity of the edge — `(from_id, to_id, edge_type)`. Used by
    /// the renderer's keyed connection buffer map so unchanged edges can
    /// reuse their shaped `cosmic_text::Buffer`s across drag frames.
    pub edge_key: EdgeKey,
    /// Sampled glyph positions along the path (canvas coordinates).
    pub glyph_positions: Vec<(f32, f32)>,
    /// The body glyph string repeated at each position.
    pub body_glyph: String,
    /// Optional start cap glyph and its position.
    pub cap_start: Option<(String, (f32, f32))>,
    /// Optional end cap glyph and its position.
    pub cap_end: Option<(String, (f32, f32))>,
    /// Font family name, if specified.
    pub font: Option<String>,
    /// Font size in points.
    pub font_size_pt: f32,
    /// Color as #RRGGBB hex string.
    pub color: String,
}

/// A portal marker — one half of a portal-mode edge rendered as a
/// single glyph above the top-right corner of one of its two endpoint
/// nodes. Each edge with `display_mode = "portal"` emits two
/// `PortalElement`s per scene build (one per endpoint).
///
/// Like `ConnectionLabelElement`, portal markers are cheap to rebuild
/// from scratch every frame (≤ two glyphs per portal, portal counts
/// stay in the dozens) so there is no per-portal cache.
pub struct PortalElement {
    /// Stable identity of the owning edge — the same `EdgeKey` the
    /// connection pipeline would use for this edge's line form, so
    /// selection, color picker, and hit-testing share one key space.
    pub edge_key: EdgeKey,
    /// Which of the two endpoints this marker is drawn next to.
    /// The renderer keys its buffer map by `(edge_key, endpoint_node_id)`
    /// so the two markers of one edge are stored separately.
    pub endpoint_node_id: String,
    /// The visible glyph string, e.g. `"◈"`.
    pub glyph: String,
    /// Top-left corner of the marker AABB in canvas coordinates.
    pub position: (f32, f32),
    /// Width and height of the marker AABB.
    pub bounds: (f32, f32),
    /// Resolved color (hex) — `var(--name)` references already expanded
    /// through the theme variable map. Overridden to the cyan highlight
    /// color at emission time when the edge is selected.
    pub color: String,
    /// Optional font family override. `None` falls back to the
    /// renderer's default font.
    pub font: Option<String>,
    /// Font size in points.
    pub font_size_pt: f32,
}

/// Session 6D: a text label attached to a connection edge. Rendered
/// as a cosmic-text buffer positioned along the edge's path at a
/// parameter-space `t` derived from `MindEdge.label_position_t`.
///
/// The AABB (`position`, `bounds`) is used by the Renderer both to
/// build the text buffer and to populate the label-hit-test index so
/// the app can detect clicks on the label for inline editing.
pub struct ConnectionLabelElement {
    /// Stable identity of the edge carrying this label.
    pub edge_key: EdgeKey,
    /// The label text (guaranteed non-empty — labels with empty or
    /// missing text are not emitted).
    pub text: String,
    /// Top-left corner of the label's AABB, in canvas coordinates.
    /// Centered horizontally and vertically on the path point.
    pub position: (f32, f32),
    /// Width and height of the label's AABB. Sized loosely from the
    /// character count × an approximate glyph width.
    pub bounds: (f32, f32),
    /// Resolved color (hex) — `var(--name)` references already
    /// expanded through the theme variable map.
    pub color: String,
    /// Optional font family override. `None` falls back to the
    /// renderer's default font.
    pub font: Option<String>,
    /// Font size in points, already multiplied by the label's size
    /// factor (1.1× the body glyph size by default) and clamped by
    /// `GlyphConnectionConfig::effective_font_size_pt`.
    pub font_size_pt: f32,
}

/// Which part of a selected edge a grab-handle targets. Session 6C's
/// connection reshape surface: anchor endpoints can be dragged to
/// change which side of a node an edge attaches to, control points
/// can be dragged to reshape a curve, and the `Midpoint` handle on a
/// straight edge inserts a control point on first drag to convert
/// the straight line into a quadratic Bezier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeHandleKind {
    /// Endpoint anchor on the `from_id` side.
    AnchorFrom,
    /// Endpoint anchor on the `to_id` side.
    AnchorTo,
    /// Existing control point at `edge.control_points[index]`.
    ControlPoint(usize),
    /// Only emitted for straight edges (empty `control_points`).
    /// Dragging this handle inserts a new control point to curve
    /// the edge. After insertion, subsequent frames treat the drag
    /// as a `ControlPoint(0)` drag.
    Midpoint,
}

/// One grab-handle glyph emitted on top of a selected edge. Rendered
/// as a small cosmic-text buffer in canvas space — the Renderer
/// treats `edge_handles` as its own buffer family since the handle
/// set is small, bounded, and only exists for the currently-selected
/// edge.
pub struct EdgeHandleElement {
    pub edge_key: EdgeKey,
    pub kind: EdgeHandleKind,
    /// Canvas-space position of the handle, already resolved from
    /// the edge's current `control_points` and anchors.
    pub position: (f32, f32),
    /// Glyph string (usually a single char like ◆).
    pub glyph: String,
    /// Color as `#RRGGBB` hex.
    pub color: String,
    /// Font size in points.
    pub font_size_pt: f32,
}

/// Glyph used for edge grab-handles. A solid black diamond reads as
/// a clickable control point across most fonts.
const EDGE_HANDLE_GLYPH: &str = "\u{25C6}"; // ◆

/// Font size (in points) for the edge handle glyphs. Slightly larger
/// than the default connection glyph size so handles stand out on top
/// of the selected edge.
const EDGE_HANDLE_FONT_SIZE_PT: f32 = 14.0;

mod builder;
mod connection;
mod edge_handle;
mod label;
mod node_pass;
pub mod portal;

#[cfg(test)]
mod tests;

pub use builder::{
    build_scene, build_scene_with_cache, build_scene_with_offsets,
    build_scene_with_offsets_selection_and_overrides,
};
pub use edge_handle::build_edge_handles;
pub use portal::SelectedPortalLabel;
