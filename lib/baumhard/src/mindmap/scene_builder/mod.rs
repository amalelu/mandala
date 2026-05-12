// SPDX-License-Identifier: MPL-2.0

//! Scene builder — projects a `MindMap` into a flat `RenderScene`
//! of per-element plain-data items (`TextElement`, `BorderElement`,
//! `ConnectionElement`, `PortalElement`, `ConnectionLabelElement`,
//! `EdgeHandleElement`) that the renderer walks into cosmic-text
//! buffers. Sharded by role so each pass stays focused; this file
//! owns the element structs and the `RenderScene` aggregate.

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

/// View-side overrides telling the scene builder which node /
/// section should receive mode-driven chrome this frame: resize
/// handles on the active resize target, and inactive-node dimming
/// when NodeEdit is open. Computed by the application layer
/// (translating from its interaction-mode state) and threaded into
/// [`build_scene_with_cache`] / [`build_scene_with_offsets_selection_and_overrides`].
///
/// `Default` is no handles + no dimming. Pre-Batch-2 of the
/// sections / borders / resize UX overhaul, the scene builder read
/// selection directly (`Single` → handles, `Section` → handles),
/// which produced the "accidental resize on selection" UX bug.
/// Decoupling the gate from selection — and putting it next to its
/// consumer `SceneSelectionContext` — keeps the model/view boundary
/// clean: the document doesn't know about modes, the app translates
/// mode to override, the scene builder consumes the override.
///
/// One bundle per `build_scene_*` call — adding a third mode-derived
/// override (e.g. `Resize`-mode body tinting) extends the struct
/// rather than threading another parameter through the signature.
#[derive(Debug, Default, Clone, Copy)]
pub struct InteractionModeOverrides<'a> {
    /// Which node should auto-emit 8 resize handles this frame, or
    /// `None` for no node handles.
    pub node: Option<&'a str>,
    /// Which section (`(node_id, section_idx)`) should auto-emit 8
    /// resize handles, or `None` for no section handles. Sections
    /// with `size == None` (fill-parent) emit zero handles inside
    /// the builder regardless — there's no own AABB to stretch.
    pub section: Option<(&'a str, usize)>,
    /// Active NodeEdit target. When `Some(active)`, every node other
    /// than `active` renders chrome + text at the inactive-alpha
    /// multiplier (see `node_pass::INACTIVE_NODE_ALPHA_MULTIPLIER`)
    /// — the "you are inside this node" affordance. `None` (the
    /// Default-mode case) is the no-op fast path.
    pub node_edit_for: Option<&'a str>,
    /// Section currently inside the inline text editor, if any.
    /// `Some((node_id, section_idx))` causes the matching
    /// `SectionFrameElement` to emit `focused = true` so the
    /// renderer draws its perimeter at a thicker stroke (Plan
    /// §4.4). `None` is the no-op — every emitted frame draws at
    /// the standard stroke. Read by the section-frame builder via
    /// [`SceneSelectionContext::focused_section`].
    pub focused_section: Option<(&'a str, usize)>,
}

impl<'a> InteractionModeOverrides<'a> {
    /// All-`None` overrides — equivalent to `Default::default()`
    /// but named for clarity at construction sites that want to
    /// be explicit about "this rebuild emits no handles".
    pub const fn none() -> Self {
        Self {
            node: None,
            section: None,
            node_edit_for: None,
            focused_section: None,
        }
    }
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

/// Transient, scene-build-only substitution of a border's resolved
/// configuration. Drives the `border preview …` /
/// `section frame preview …` / `canvas border preview …` /
/// `canvas section-frame [focused] preview …` console verbs.
///
/// While `Some(...)` is threaded through the build pipeline, the
/// scene builder folds the previewed `edits` into a clone of the
/// committed slot at the matching target before resolution — the
/// committed model in `MindMap` is never mutated; this preview is
/// purely a scene-level substitution. Borrow shape mirrors
/// [`EdgeColorPreview`] / [`PortalColorPreview`]: the application
/// layer owns the data, threads a borrow into the scene call.
///
/// `force_show_frame` lets `border preview preset=heavy` render
/// against a node whose committed `style.show_frame == false` —
/// otherwise the preview would be invisible and the user would
/// think the verb was broken. Commit writes the explicit
/// visibility flip through the normal setter, so the force flag
/// only lives here on the scene-side struct.
#[derive(Debug, Clone, Copy)]
pub struct BorderPreview<'a> {
    pub target: BorderPreviewTargetRef<'a>,
    /// View carried by value — it's already a borrow of the
    /// document's `BorderConfigEdits`, so cloning it just copies
    /// 17 fields of `Option<&str>` / `Option<f32>` / `bool`. No
    /// secondary borrow needed.
    pub edits: BorderConfigEditsView<'a>,
    pub force_show_frame: bool,
}

/// Borrowed view of the document-side `BorderPreviewTarget`. The
/// scene builder reads through these slices without taking
/// ownership of the doc's `Vec`s.
#[derive(Debug, Clone, Copy)]
pub enum BorderPreviewTargetRef<'a> {
    Nodes(&'a [String]),
    Sections(&'a [(String, usize)]),
    CanvasDefault,
    CanvasSectionFrame,
    CanvasSectionFrameFocused,
}

/// Per-field tri-state edit, mirroring the application crate's
/// `OptionEdit<T>` (`Keep` / `Clear` / `Set`). The scene-side
/// view carries this so `OptionEdit::Clear` round-trips into the
/// preview pipeline — pre-fix `BorderConfigEditsView` collapsed
/// `Clear` to "no edit" and the rendered preview diverged from
/// what commit produced (Risk #1 in the plan).
///
/// `Keep` = no edit (steady-state default); `Clear` = drop the
/// field on the slot; `Set(v)` = write the borrowed value.
/// `Default` is `Keep` so a `BorderConfigEditsView::default()`
/// is a no-op view.
#[derive(Debug, Clone, Copy, Default)]
pub enum EditView<T: Copy> {
    #[default]
    Keep,
    Clear,
    Set(T),
}

impl<T: Copy> EditView<T> {
    /// `true` iff the edit is `Set` or `Clear` — i.e. it touches
    /// the field, vs `Keep` which leaves it alone. Used by the
    /// "any field touched?" predicates that gate slot allocation
    /// + force-show-frame logic.
    pub fn is_edit(&self) -> bool {
        !matches!(self, EditView::Keep)
    }
}

/// Scene-side mirror of the application-crate `BorderConfigEdits`
/// struct. The application crate owns `BorderConfigEdits` (it
/// imports `OptionEdit` and shapes around the document layer);
/// this view exposes just the resolved option-fields the slot
/// helper needs at scene-build time. The application layer
/// constructs an instance from the owned `BorderConfigEdits` and
/// hands the borrow into [`BorderPreview`].
///
/// Mirrors the slot-helper's read shape — preset / font / size /
/// color / palette / palette_field / padding / four sides / four
/// corners — each as an [`EditView`] tri-state so that
/// `OptionEdit::Clear` survives the projection. Plus a top-level
/// `clear: bool` that empties the entire slot (mirrors
/// `BorderConfigEdits.clear`).
#[derive(Debug, Clone, Copy, Default)]
pub struct BorderConfigEditsView<'a> {
    pub preset: EditView<&'a str>,
    pub font: EditView<&'a str>,
    pub font_size_pt: EditView<f32>,
    pub color: EditView<&'a str>,
    pub padding: EditView<f32>,
    pub color_palette: EditView<&'a str>,
    pub color_palette_field: EditView<&'a str>,
    pub side_top: EditView<&'a str>,
    pub side_bottom: EditView<&'a str>,
    pub side_left: EditView<&'a str>,
    pub side_right: EditView<&'a str>,
    pub corner_top_left: EditView<&'a str>,
    pub corner_top_right: EditView<&'a str>,
    pub corner_bottom_left: EditView<&'a str>,
    pub corner_bottom_right: EditView<&'a str>,
    /// `true` clears the slot entirely (the cascade falls through
    /// to the canvas default or the hardcoded floor). Mirrors
    /// `BorderConfigEdits.clear`.
    pub clear: bool,
}

impl<'a> BorderConfigEditsView<'a> {
    /// `true` iff any per-field axis is `Set` or `Clear`. Used by
    /// the slot-allocation gate inside `apply_view_to_slot` and
    /// by the force-show-frame predicate (along with `clear`,
    /// which is its own axis).
    pub fn touches_any_field(&self) -> bool {
        self.preset.is_edit()
            || self.font.is_edit()
            || self.font_size_pt.is_edit()
            || self.color.is_edit()
            || self.padding.is_edit()
            || self.color_palette.is_edit()
            || self.color_palette_field.is_edit()
            || self.touches_glyphs()
    }

    /// `true` iff any side- or corner-glyph axis is `Set` or
    /// `Clear`. Mirrors the application-side `edits_touch_glyphs`
    /// predicate; lifts the eight-way OR into the type so the
    /// app side can drop its parallel copy.
    pub fn touches_glyphs(&self) -> bool {
        self.side_top.is_edit()
            || self.side_bottom.is_edit()
            || self.side_left.is_edit()
            || self.side_right.is_edit()
            || self.corner_top_left.is_edit()
            || self.corner_top_right.is_edit()
            || self.corner_bottom_left.is_edit()
            || self.corner_bottom_right.is_edit()
    }
}

/// Intermediate representation between MindMap data and GPU rendering.
/// Produced by `build_scene()`, consumed by Renderer to create cosmic-text buffers.
pub struct RenderScene {
    pub text_elements: Vec<TextElement>,
    pub border_elements: Vec<BorderElement>,
    pub connection_elements: Vec<ConnectionElement>,
    pub portal_elements: Vec<PortalElement>,
    /// Grab-handles rendered on top of the *selected* edge.
    /// Always empty unless `selected_edge` was `Some` on the scene-build
    /// call. Contains the two anchor endpoints, any existing control
    /// points, and (for straight edges only) a midpoint handle that
    /// triggers the "curve a straight line" gesture when dragged.
    pub edge_handles: Vec<EdgeHandleElement>,
    /// Resize handles rendered on top of the *selected* `Some`-
    /// sized section. Always empty unless the scene was built
    /// with a `selected_section` matching a `Some`-sized section.
    /// 8 handles when populated (corners + edge midpoints);
    /// `None`-sized sections (fill-parent) emit zero handles
    /// because there's no per-section AABB to stretch.
    pub section_resize_handles: Vec<SectionResizeHandleElement>,
    /// Resize handles rendered on top of the *selected* node.
    /// Always empty unless the scene was built with a
    /// `selected_node_for_resize` matching a node with finite +
    /// positive size. 8 handles when populated (corners + edge
    /// midpoints).
    pub node_resize_handles: Vec<NodeResizeHandleElement>,
    /// Section frames rendered on top of the active NodeEdit
    /// node's sections. Always empty unless the scene was built
    /// with `node_edit_for = Some(active)` AND the named node has
    /// `sections.len() >= 2` (single-section nodes skip frames —
    /// they would just duplicate the border, and the single-
    /// section short-circuit bypasses NodeEdit anyway). One element
    /// per section of the active node when populated; the renderer
    /// draws each as a thin glyph rectangle in the cyan
    /// SELECTED_EDGE_COLOR family. The element flagged `focused`
    /// (the section currently inside the text editor, if any) is
    /// rendered with a thicker / brighter stroke per Plan §4.4.
    pub section_frames: Vec<SectionFrameElement>,
    /// Labels attached to edges whose `label` field is non-empty.
    /// One element per labeled edge, positioned along the connection
    /// path at `label_config.position_t` (defaulting to 0.5), shifted
    /// by `label_config.perpendicular_offset` along the path normal
    /// when set. Not cached in `SceneConnectionCache` — labels are
    /// ≤ 1 per edge and rebuilt each frame at trivial cost.
    pub connection_label_elements: Vec<ConnectionLabelElement>,
    pub background_color: String,
}

/// A visible text element to be rendered. Each renderable
/// [`MindSection`](crate::mindmap::model::MindSection) of a
/// non-folded node emits one `TextElement`; sections without text
/// (and folded nodes' sections) skip emission entirely.
pub struct TextElement {
    /// Owning MindNode id — the same id every other per-node
    /// element (`BorderElement`, hit-test AABB, edge endpoint)
    /// keys on. Multiple `TextElement`s share this id when a
    /// node has multiple sections.
    pub node_id: String,
    /// Index into [`MindNode.sections`](crate::mindmap::model::MindNode::sections)
    /// — the position of the section that produced this element.
    /// Stable across scene rebuilds for unchanged nodes.
    pub section_idx: usize,
    pub text: String,
    pub text_runs: Vec<TextRun>,
    /// Top-left of the section AABB in canvas space —
    /// `node.position + section.offset`.
    pub position: (f32, f32),
    /// Section AABB size — `section.size.unwrap_or(node.size)`.
    pub size: (f32, f32),
}

/// A border to be rendered around a node.
pub struct BorderElement {
    pub node_id: String,
    pub border_style: BorderStyle,
    pub node_position: (f32, f32),
    pub node_size: (f32, f32),
    /// Inherited from the owning node — a border appearing when its
    /// node is culled would be a floating frame fragment, so the
    /// border renders only when the node does.
    pub zoom_visibility: crate::gfx_structs::zoom_visibility::ZoomVisibility,
    /// Resolved per-cycle-position colours when the user opts into
    /// `border_style.color_palette`; empty otherwise. Pre-resolved
    /// here so the renderer doesn't need `&MindMap` access on the
    /// hot rebuild path. Sibling of
    /// [`crate::mindmap::tree_builder::BorderNodeData::palette_cycle`].
    pub palette_cycle: Vec<[f32; 4]>,
}

/// A glyph-drawn rectangle outlining one section of the active
/// NodeEdit node — the visual cue telling the user "this is the
/// per-section subdivision you can pick from."
///
/// Section frames flow through the same [`BorderStyle`] machinery
/// node borders do: any preset, any per-side `SidePattern`, any
/// per-corner glyph, any font, any color, any palette. The
/// resolver cascade (`resolve_section_frame_border` in
/// `crate::mindmap::border`) is:
///   1. `MindSection.frame_border` if `Some` (per-section author
///      override).
///   2. else `Canvas.default_section_frame_border` (or
///      `default_focused_section_frame_border` when `focused`).
///   3. else a hardcoded thin (default) / heavy (focused) floor.
///
/// One element per section of the active node when emitted. Empty
/// for: Default mode, NodeEdit on a single-section node (frame
/// would duplicate the border), NodeEdit on a missing /
/// hidden-by-fold node.
#[derive(Debug, Clone)]
pub struct SectionFrameElement {
    /// Owning MindNode id — same id every per-node element keys
    /// on. Multiple `SectionFrameElement`s share this id when the
    /// active node has multiple sections.
    pub node_id: String,
    /// Index into [`MindNode.sections`](crate::mindmap::model::MindNode::sections).
    /// Stable across rebuilds for unchanged nodes.
    pub section_idx: usize,
    /// Top-left of the section's effective AABB in canvas space —
    /// `node.position + section.offset`. Same value that the
    /// matching `TextElement.position` carries; the renderer reads
    /// from here to draw the perimeter glyphs.
    pub position: (f32, f32),
    /// Size of the section's effective AABB —
    /// `section.size.unwrap_or(node.size)`. Mirrors
    /// `TextElement.size`.
    pub size: (f32, f32),
    /// Resolved per-frame [`BorderStyle`] — preset, side patterns,
    /// corners, font, size, color, palette field. Mirrors
    /// [`BorderElement::border_style`]; consumers feed it to
    /// `crate::mindmap::border::border_run_specs` for the four-side
    /// run geometry.
    pub border_style: BorderStyle,
    /// Resolved per-cycle-position colors when the frame uses a
    /// `color_palette`; empty otherwise. Mirrors
    /// [`BorderElement::palette_cycle`].
    pub palette_cycle: Vec<[f32; 4]>,
    /// `true` when this section is the focus of an active text
    /// editor. The renderer draws focused frames using the heavy-
    /// preset floor (or the canvas-level
    /// `default_focused_section_frame_border` when set) so the
    /// user sees which section is being edited among the active
    /// node's siblings. Plan §4.4.
    pub focused: bool,
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
    /// Zoom window for the whole connection (body glyphs + caps).
    /// Resolved directly from `MindEdge.min_zoom_to_render` /
    /// `MindEdge.max_zoom_to_render` — edges are the authoring
    /// unit, no per-glyph override.
    pub zoom_visibility: crate::gfx_structs::zoom_visibility::ZoomVisibility,
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
    /// Zoom window for this portal marker (icon + its adjacent
    /// text). Resolved with the replace-not-intersect cascade:
    /// `PortalEndpointState.min/max_zoom_to_render` override
    /// `MindEdge.min/max_zoom_to_render` when any of the pair is
    /// `Some`; otherwise inherit the edge window unchanged.
    pub zoom_visibility: crate::gfx_structs::zoom_visibility::ZoomVisibility,
}

/// A text label attached to a connection edge. Rendered as a
/// cosmic-text buffer positioned along the edge's path at a
/// parameter-space `t` derived from
/// `MindEdge.label_config.position_t`, optionally shifted
/// perpendicular to the path by
/// `MindEdge.label_config.perpendicular_offset`.
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
    /// Zoom window for the label. Resolved with the replace-not-
    /// intersect cascade: `EdgeLabelConfig.min/max_zoom_to_render`
    /// override `MindEdge.min/max_zoom_to_render` when any of the
    /// pair is `Some`; otherwise inherit the edge window.
    pub zoom_visibility: crate::gfx_structs::zoom_visibility::ZoomVisibility,
}

/// Which part of a selected edge a grab-handle targets. The
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

/// Glyph used for anchor and control-point edge grab-handles. A
/// solid black diamond reads as a clickable control point across
/// most fonts.
const EDGE_HANDLE_GLYPH: &str = "\u{25C6}"; // ◆

/// Distinct glyph for the `Midpoint` handle that appears only on
/// straight edges and bootstraps the "curve this line" gesture on
/// drag. A curved arrow reads as "bend me" — specifically an
/// anticlockwise hook (`↺`) so nothing about the handle looks like
/// a plain re-selection target. Without this second glyph the
/// midpoint handle is visually identical to the anchor handles and
/// the gesture is undiscoverable (see `commands/edge.rs` for the
/// console-side counterpart, `edge reset=curve`).
const EDGE_MIDPOINT_HANDLE_GLYPH: &str = "\u{21BA}"; // ↺

/// Font size (in points) for the edge handle glyphs. Slightly larger
/// than the default connection glyph size so handles stand out on top
/// of the selected edge.
const EDGE_HANDLE_FONT_SIZE_PT: f32 = 14.0;

mod builder;
mod connection;
mod edge_handle;
mod label;
mod node_pass;
mod node_resize_handle;
/// Portal-marker emission — one `PortalElement` per endpoint of
/// each `display_mode = "portal"` edge, attached to its owning
/// node's border at the point facing the opposite endpoint.
pub mod portal;
mod section_frame;
mod section_resize_handle;

#[cfg(test)]
mod tests;

pub use builder::{
    build_scene, build_scene_with_cache, build_scene_with_offsets,
    build_scene_with_offsets_selection_and_overrides, PortalTextEditOverride, SceneSelectionContext,
};
pub use edge_handle::{build_edge_handles, edge_handle_channel_for};
pub use node_resize_handle::{build_node_resize_handles, NodeResizeHandleElement};
pub use portal::SelectedPortalLabel;
pub use section_frame::build_section_frames;
pub use section_resize_handle::{
    build_section_resize_handles, infer_resize_anchor, ResizeHandleSide, SectionResizeHandleElement,
    SECTION_RESIZE_HANDLE_FONT_SIZE_PT, SECTION_RESIZE_HANDLE_GLYPH,
};
