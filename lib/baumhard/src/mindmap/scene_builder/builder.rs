// SPDX-License-Identifier: MPL-2.0

//! Scene builders — `build_scene`, `build_scene_with_cache`, and
//! the cache-less wrappers. The big `build_scene_with_cache`
//! orchestrator is a thin linear pipeline: [`super::node_pass`] →
//! [`super::connection`] → [`super::label`] → [`super::portal`],
//! assembled into a `RenderScene`. The selected-edge handle
//! emission rides inside the connection pass (delegating to
//! [`super::edge_handle::build_edge_handles`]).
//!
//! Portal-mode edges are routed to the portal pass and skipped by
//! the connection and label passes; line-mode edges take the usual
//! connection → label → edge-handle path. `selected_edge` is shared
//! between both pipelines so a selected edge highlights cyan
//! whichever form it renders as.

use std::collections::HashMap;

use crate::mindmap::model::MindMap;
use crate::mindmap::scene_cache::{EdgeKey, SceneConnectionCache};
use crate::util::color::resolve_var;

use super::connection::build_connection_elements;
use super::label::build_label_elements;
use super::node_pass::build_node_elements;
use super::node_resize_handle::{build_node_resize_handles, NodeResizeHandleElement};
use super::portal::build_portal_elements;
use super::portal::SelectedPortalLabel;
use super::section_resize_handle::{build_section_resize_handles, SectionResizeHandleElement};
use super::{BorderPreview, EdgeColorPreview, PortalColorPreview, RenderScene};

/// Bundle of "what is the user currently pointing at?" inputs
/// threaded into the scene build. Groups the three selection-
/// like overrides (whole-edge select, per-label select, inline
/// label-edit substitution) so [`build_scene_with_cache`] and
/// siblings stay readable; the in-flight color previews stay
/// separate because they're hover-state, not selection-state.
///
/// Empty context (all three fields `None`) is the common case —
/// use [`SceneSelectionContext::default`] instead of spelling
/// out `SceneSelectionContext { edge: None, .. }` at call sites.
#[derive(Debug, Clone, Default)]
pub struct SceneSelectionContext<'a> {
    /// Whole edge selection — applies the cyan highlight to both
    /// markers of a portal-mode edge (or the body glyphs of a
    /// line-mode edge). Tuple is `(from_id, to_id, edge_type)`.
    pub edge: Option<(&'a str, &'a str, &'a str)>,
    /// Edge-label sub-selection — applies the cyan highlight to
    /// just the line-mode label text for the named edge, without
    /// tinting the body glyphs. Set by `SelectionState::EdgeLabel`;
    /// mutually exclusive with `edge` by construction on the caller
    /// side (`SelectionState` is an enum). Distinct from `edge` so
    /// clicking just the label tints only the label, matching what
    /// the user pointed at.
    ///
    /// Stored by value (not as a borrow) because `EdgeLabelSel`
    /// holds an `EdgeRef` — three strings, which the context
    /// assembly at the document layer converts into an `EdgeKey`
    /// per call. The cost is three `String` clones; negligible
    /// next to the per-frame scene build.
    pub edge_label: Option<EdgeKey>,
    /// Per-label selection — applies the cyan highlight to just
    /// one endpoint's marker on a portal-mode edge. Mutually
    /// exclusive with `edge` by construction on the caller side
    /// (`SelectionState` is an enum).
    pub portal_label: Option<SelectedPortalLabel<'a>>,
    /// Inline edge-label editor override — substitutes the
    /// in-progress buffer + caret for the committed label text
    /// on the named edge, so label edits render live.
    pub label_edit: Option<(&'a EdgeKey, &'a str)>,
    /// Selected section identity — `(node_id, section_idx)` —
    /// driving section-resize-handle emission. When `Some` and
    /// the named section has `Some` size, the scene includes 8
    /// handles for the section. `None` (the default) emits no
    /// section handles.
    pub selected_section: Option<(&'a str, usize)>,
    /// Selected node identity for node-resize-handle emission.
    /// Set by callers when the selection is `Single(node_id)`;
    /// empty otherwise. The scene includes 8 handles for the
    /// node when its size is finite + positive.
    pub selected_node_for_resize: Option<&'a str>,
    /// Active NodeEdit target. When `Some(active)`, every node
    /// other than `active` renders chrome + text at the inactive-
    /// alpha multiplier (see `node_pass::INACTIVE_NODE_ALPHA_MULTIPLIER`)
    /// — the "you are inside this node" affordance for
    /// `InteractionMode::NodeEdit`. `None` (the Default-mode
    /// case) is the no-op fast path: every node draws at full
    /// opacity. Set from the application layer at the same call
    /// site that fills `selected_node_for_resize` /
    /// `selected_section`; routes through `node_pass` inside
    /// [`build_scene_with_cache`].
    pub node_edit_for: Option<&'a str>,
    /// Section currently inside the inline text editor, if any.
    /// Read by the section-frame pass to mark the matching
    /// `SectionFrameElement.focused = true`. `None` = no editor
    /// open or the editor's section isn't part of an emitted frame
    /// set (Default mode, single-section node).
    pub focused_section: Option<(&'a str, usize)>,
}

/// Substitution pair for the portal-text inline edit preview.
/// Carries the `(edge_key, endpoint_node_id)` identity of the
/// target portal label plus the current buffer contents to be
/// rendered in place of the committed `PortalEndpointState.text`.
/// Consumed by the tree-builder path (the live portal render
/// pipeline), not by the scene builder — portal text never
/// materialized through `RenderScene.portal_elements`.
#[derive(Debug, Clone, Copy)]
pub struct PortalTextEditOverride<'a> {
    pub edge_key: &'a EdgeKey,
    pub endpoint_node_id: &'a str,
    pub buffer: &'a str,
}

/// Builds a RenderScene from a MindMap, determining which nodes and borders
/// are visible (accounting for fold state) and extracting their layout data.
///
/// `camera_zoom` is used to compute the effective (clamped) canvas-space
/// font size for each connection — see
/// [`crate::mindmap::model::GlyphConnectionConfig::effective_font_size_pt`].
/// Pass `1.0` if no camera context applies (e.g. loader tests).
pub fn build_scene(map: &MindMap, camera_zoom: f32) -> RenderScene {
    let mut scratch = SceneConnectionCache::new();
    build_scene_with_cache(
        map,
        &HashMap::new(),
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut scratch,
        camera_zoom,
    )
}

/// Builds a RenderScene with position offsets applied to specific nodes.
/// Used during drag to update connections and borders in real-time without
/// modifying the MindMap model. Each entry in `offsets` maps a node ID to
/// a (dx, dy) delta that is added to the node's model position.
pub fn build_scene_with_offsets(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    camera_zoom: f32,
) -> RenderScene {
    let mut scratch = SceneConnectionCache::new();
    build_scene_with_cache(
        map,
        offsets,
        SceneSelectionContext::default(),
        None,
        None,
        None,
        &mut scratch,
        camera_zoom,
    )
}

/// Cache-less wrapper that threads selection + transient
/// interaction overrides:
///
/// - `selection`: whole-edge, per-label, or inline-label-edit
///   overrides — see [`SceneSelectionContext`].
/// - `edge_color_preview`: color-picker hover preview for a
///   single edge, beats selection on the previewed edge.
/// - `portal_color_preview`: same, but routes to the portal
///   pass for edges with `display_mode = "portal"`.
///
/// Prefer [`build_scene_with_cache`] on the hot drag path —
/// this variant allocates a throwaway cache per call.
pub fn build_scene_with_offsets_selection_and_overrides(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    selection: SceneSelectionContext<'_>,
    edge_color_preview: Option<EdgeColorPreview<'_>>,
    portal_color_preview: Option<PortalColorPreview<'_>>,
    border_preview: Option<BorderPreview<'_>>,
    camera_zoom: f32,
) -> RenderScene {
    let mut scratch = SceneConnectionCache::new();
    build_scene_with_cache(
        map,
        offsets,
        selection,
        edge_color_preview,
        portal_color_preview,
        border_preview,
        &mut scratch,
        camera_zoom,
    )
}

/// Cache-aware scene builder. For each edge:
/// - if neither endpoint is in `offsets` AND the edge's geometry is already
///   in `cache`, reuse the cached pre-clip samples (skip `sample_path`) and
///   only re-run the cheap clip filter against this frame's `node_aabbs`
///   so stable edges still clip correctly around moved-but-unrelated nodes;
/// - otherwise, run the full `build_connection_path` + `sample_path` +
///   clip path and **write the fresh entry back** into the cache.
///
/// Selection changes do NOT invalidate the cache: the `SELECTED_EDGE_COLOR`
/// override is applied at read time below.
///
/// At the end of the build, any cached entry whose key was not seen this
/// frame (i.e. the edge was deleted from the model) is evicted.
// `clippy::too_many_arguments` is silenced here for an
// architectural reason rather than a stylistic one: this is the
// scene-build entry point and each argument has a different
// caller-vs-builder lifetime relationship. `map` and `offsets`
// borrow the persisted document; `selection` is a per-frame
// snapshot; the three `*_preview` Options are short-lived
// borrowed views constructed by `assemble_scene_overrides`;
// `cache` is mut-borrowed across frames for memoisation;
// `camera_zoom` is by-value. Bundling them into a struct would
// either duplicate every field's lifetime/mutability annotation
// or hide the borrow shape from the caller. Keep the function
// signature as the documentation surface for "what this function
// reads vs. writes".
#[allow(clippy::too_many_arguments)]
pub fn build_scene_with_cache(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    selection: SceneSelectionContext<'_>,
    edge_color_preview: Option<EdgeColorPreview<'_>>,
    portal_color_preview: Option<PortalColorPreview<'_>>,
    border_preview: Option<BorderPreview<'_>>,
    cache: &mut SceneConnectionCache,
    camera_zoom: f32,
) -> RenderScene {
    let SceneSelectionContext {
        edge: selected_edge,
        edge_label: selected_edge_label,
        portal_label: selected_portal_label,
        label_edit: label_edit_override,
        selected_section,
        selected_node_for_resize,
        node_edit_for,
        focused_section,
    } = selection;
    // The per-edge sample spacing depends on the effective font size,
    // which depends on `camera_zoom`. Flush cached samples if the
    // incoming zoom differs from the one the cache was built at, so
    // stale spacing doesn't leak into this frame.
    cache.ensure_zoom(camera_zoom);

    // Per-node pass: emits `TextElement`s + `BorderElement`s and
    // computes the clip AABBs the connection pass below consumes.
    // `node_edit_for` dims chrome on every other node — see
    // `node_pass::INACTIVE_NODE_ALPHA_MULTIPLIER`.
    let (text_elements, border_elements, node_aabbs) =
        build_node_elements(map, offsets, node_edit_for, border_preview);

    // Connection pass — fast/slow cache path, clip filter against
    // `node_aabbs`, edge-handle emission for the selected edge.
    // Cache lifecycle (retain_keys) lives inside the sub-builder so
    // eviction stays colocated with the keys-seen bookkeeping.
    let (connection_elements, edge_handles) = build_connection_elements(
        map,
        offsets,
        &node_aabbs,
        selected_edge,
        edge_color_preview,
        cache,
        camera_zoom,
    );

    // Label pass — sub-builder rebuilds paths per labeled edge
    // (trivial cost, no cache). Handles the label-edit override
    // substitution + synthesis for empty committed labels, and
    // paints the cyan highlight onto the label when the selection
    // is either the whole edge or just the label sub-part (both
    // map onto the label text — "selected" reads the same way on
    // every sub-part of the edge).
    let selected_edge_label_highlight_key: Option<EdgeKey> = selected_edge_label
        .clone()
        .or_else(|| selected_edge.map(|(f, t, ty)| EdgeKey::new(f, t, ty)));
    let connection_label_elements = build_label_elements(
        map,
        offsets,
        label_edit_override,
        edge_color_preview,
        selected_edge_label_highlight_key.as_ref(),
        camera_zoom,
    );

    // Portal pass — one marker per endpoint of every visible
    // portal-mode edge, colored by preview > selection > edge
    // color. Text labels do not go through this path; the tree
    // builder (`tree_builder::portal`) is the live portal
    // renderer and emits text as sibling GlyphAreas under each
    // endpoint's subtree.
    let portal_elements = build_portal_elements(
        map,
        offsets,
        selected_edge,
        selected_portal_label,
        portal_color_preview,
        camera_zoom,
    );

    // Section resize handles — only emitted for the currently-
    // selected section, and only when it has `Some` size (a
    // fill-parent section has no per-section AABB to stretch).
    // Selection-gated here, mirroring edge-handle "only on
    // selected edge" precedent.
    let section_resize_handles = build_selected_section_handles(map, offsets, selected_section);

    // Node resize handles — emitted only for the currently-
    // selected node. Selection-gated like edge handles and
    // section resize handles. Hidden-by-fold and missing-node
    // cases produce zero handles.
    let node_resize_handles = build_selected_node_handles(map, offsets, selected_node_for_resize);

    // Section frames — one per section of the active NodeEdit
    // node so the user sees the per-section subdivisions. Empty
    // in Default mode and on single-section nodes (the
    // single-section short-circuit bypasses NodeEdit entirely).
    // Plan §3.5 / §4.3.
    let section_frames = super::section_frame::build_section_frames(
        map,
        offsets,
        node_edit_for,
        focused_section,
        border_preview,
    );

    RenderScene {
        text_elements,
        border_elements,
        connection_elements,
        portal_elements,
        edge_handles,
        section_resize_handles,
        node_resize_handles,
        section_frames,
        connection_label_elements,
        background_color: resolve_var(&map.canvas.background_color, &map.canvas.theme_variables).to_string(),
    }
}

/// Resolve `selected_node_for_resize` into the node's
/// `(canvas_pos, canvas_size)` and dispatch to
/// [`build_node_resize_handles`]. Returns `Vec::new()` when no
/// node is selected, the named node is missing or hidden, or
/// the node's size has any non-finite / non-positive component.
fn build_selected_node_handles(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    selected_node_for_resize: Option<&str>,
) -> Vec<NodeResizeHandleElement> {
    let node_id = match selected_node_for_resize {
        Some(s) => s,
        None => return Vec::new(),
    };
    let node = match map.nodes.get(node_id) {
        Some(n) => n,
        None => return Vec::new(),
    };
    if map.is_hidden_by_fold(node) {
        return Vec::new();
    }
    let (ox, oy) = offsets.get(&node.id).copied().unwrap_or((0.0, 0.0));
    let node_pos = node.pos_vec2();
    let node_size = node.size_vec2();
    let canvas_pos = glam::Vec2::new(node_pos.x + ox, node_pos.y + oy);
    build_node_resize_handles(node_id, canvas_pos, node_size)
}

/// Resolve `selected_section` into the `(canvas-pos, canvas-size)`
/// of the section's AABB and dispatch to
/// [`build_section_resize_handles`]. Returns `Vec::new()` when no
/// section is selected, the named node / section is missing or
/// hidden, or the section is `None`-sized (fill-parent).
fn build_selected_section_handles(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    selected_section: Option<(&str, usize)>,
) -> Vec<SectionResizeHandleElement> {
    let (node_id, section_idx) = match selected_section {
        Some(s) => s,
        None => return Vec::new(),
    };
    let node = match map.nodes.get(node_id) {
        Some(n) => n,
        None => return Vec::new(),
    };
    if map.is_hidden_by_fold(node) {
        return Vec::new();
    }
    let section = match node.sections.get(section_idx) {
        Some(s) => s,
        None => return Vec::new(),
    };
    let (ox, oy) = offsets.get(&node.id).copied().unwrap_or((0.0, 0.0));
    let node_pos = node.pos_vec2();
    let section_pos = glam::Vec2::new(
        node_pos.x + ox + section.offset.x as f32,
        node_pos.y + oy + section.offset.y as f32,
    );
    let section_size = section
        .size
        .as_ref()
        .map(|s| glam::Vec2::new(s.width as f32, s.height as f32));
    // `None` size returns Vec::new() inside the builder — skip the
    // emission and the eventual scene-tree register/mutator
    // dispatch deals with the empty payload identically.
    build_section_resize_handles(node_id, section_idx, section_pos, section_size)
}
