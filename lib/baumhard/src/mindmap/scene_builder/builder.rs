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
use super::portal::build_portal_elements;
use super::portal::SelectedPortalLabel;
use super::{EdgeColorPreview, PortalColorPreview, RenderScene};

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
#[derive(Debug, Clone, Copy, Default)]
pub struct SceneSelectionContext<'a> {
    /// Whole edge selection — applies the cyan highlight to both
    /// markers of a portal-mode edge (or the body glyphs of a
    /// line-mode edge). Tuple is `(from_id, to_id, edge_type)`.
    pub edge: Option<(&'a str, &'a str, &'a str)>,
    /// Per-label selection — applies the cyan highlight to just
    /// one endpoint's marker on a portal-mode edge. Mutually
    /// exclusive with `edge` by construction on the caller side
    /// (`SelectionState` is an enum).
    pub portal_label: Option<SelectedPortalLabel<'a>>,
    /// Inline edge-label editor override — substitutes the
    /// in-progress buffer + caret for the committed label text
    /// on the named edge, so label edits render live.
    pub label_edit: Option<(&'a EdgeKey, &'a str)>,
    /// Inline portal-text editor override — substitutes the
    /// in-progress buffer for the committed
    /// `PortalEndpointState.text` on the named (edge, endpoint)
    /// pair, same pattern as `label_edit` but keyed to a portal
    /// endpoint instead of an edge path.
    pub portal_text_edit: Option<PortalTextEditOverride<'a>>,
}

/// Substitution pair for the portal-text inline edit preview.
/// Carries the `(edge_key, endpoint_node_id)` identity of the
/// target portal label plus the current buffer contents to be
/// rendered in place of the committed `PortalEndpointState.text`.
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
    camera_zoom: f32,
) -> RenderScene {
    let mut scratch = SceneConnectionCache::new();
    build_scene_with_cache(
        map,
        offsets,
        selection,
        edge_color_preview,
        portal_color_preview,
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
pub fn build_scene_with_cache(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    selection: SceneSelectionContext<'_>,
    edge_color_preview: Option<EdgeColorPreview<'_>>,
    portal_color_preview: Option<PortalColorPreview<'_>>,
    cache: &mut SceneConnectionCache,
    camera_zoom: f32,
) -> RenderScene {
    let SceneSelectionContext {
        edge: selected_edge,
        portal_label: selected_portal_label,
        label_edit: label_edit_override,
        portal_text_edit,
    } = selection;
    // The per-edge sample spacing depends on the effective font size,
    // which depends on `camera_zoom`. Flush cached samples if the
    // incoming zoom differs from the one the cache was built at, so
    // stale spacing doesn't leak into this frame.
    cache.ensure_zoom(camera_zoom);

    // Per-node pass: emits `TextElement`s + `BorderElement`s and
    // computes the clip AABBs the connection pass below consumes.
    let (text_elements, border_elements, node_aabbs) = build_node_elements(map, offsets);

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
    // substitution + synthesis for empty committed labels.
    let connection_label_elements = build_label_elements(
        map,
        offsets,
        label_edit_override,
        edge_color_preview,
        camera_zoom,
    );

    // Portal pass — two markers per visible portal-mode edge,
    // colored by preview > selection > edge color. Text labels
    // for each endpoint reflect the committed `text` plus the
    // inline-edit buffer preview when the editor is open.
    let portal_elements = build_portal_elements(
        map,
        offsets,
        selected_edge,
        selected_portal_label,
        portal_color_preview,
        portal_text_edit,
    );

    RenderScene {
        text_elements,
        border_elements,
        connection_elements,
        portal_elements,
        edge_handles,
        connection_label_elements,
        background_color: resolve_var(&map.canvas.background_color, &map.canvas.theme_variables)
            .to_string(),
    }
}

