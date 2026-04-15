//! Scene builders — `build_scene`, `build_scene_with_cache`, and
//! the three cache-less wrappers. The big `build_scene_with_cache`
//! orchestrator walks nodes (via `super::node_pass`), edges (inline
//! connection pass), labels (via `super::label`), and portals (via
//! `super::portal`) to assemble a `RenderScene`. The edge-handle
//! emission for the selected edge delegates to
//! `super::edge_handles::build_edge_handles`.

use std::collections::{HashMap, HashSet};

use glam::Vec2;

use crate::mindmap::connection;
use crate::mindmap::model::{GlyphConnectionConfig, MindMap};
use crate::mindmap::scene_cache::{CachedConnection, EdgeKey, SceneConnectionCache};
use crate::util::color::resolve_var;

use super::edge_handles::build_edge_handles;
use super::label::build_label_elements;
use super::node_pass::build_node_elements;
use super::portal::build_portal_elements;
use super::{
    ConnectionElement, EdgeColorPreview, EdgeHandleElement, PortalColorPreview,
    RenderScene, SELECTED_EDGE_COLOR,
};

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
        None,
        None,
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
        map, offsets, None, None, None, None, None, &mut scratch, camera_zoom,
    )
}

/// Cache-less wrapper that threads transient interaction overrides:
///
/// - `label_edit_override`: inline label-edit buffer + caret
///   substitution for a single edge.
/// - `edge_color_preview`: color-picker hover preview for a single
///   edge, beats selection on the previewed edge.
/// - `portal_color_preview`: same, for portals.
///
/// Prefer [`build_scene_with_cache`] on the hot drag path — this
/// variant allocates a throwaway cache per call.
pub fn build_scene_with_offsets_selection_and_overrides(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    selected_edge: Option<(&str, &str, &str)>,
    selected_portal: Option<(&str, &str, &str)>,
    label_edit_override: Option<(&EdgeKey, &str)>,
    edge_color_preview: Option<EdgeColorPreview<'_>>,
    portal_color_preview: Option<PortalColorPreview<'_>>,
    camera_zoom: f32,
) -> RenderScene {
    let mut scratch = SceneConnectionCache::new();
    build_scene_with_cache(
        map,
        offsets,
        selected_edge,
        selected_portal,
        label_edit_override,
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
    selected_edge: Option<(&str, &str, &str)>,
    selected_portal: Option<(&str, &str, &str)>,
    label_edit_override: Option<(&EdgeKey, &str)>,
    edge_color_preview: Option<EdgeColorPreview<'_>>,
    portal_color_preview: Option<PortalColorPreview<'_>>,
    cache: &mut SceneConnectionCache,
    camera_zoom: f32,
) -> RenderScene {
    // The per-edge sample spacing depends on the effective font size,
    // which depends on `camera_zoom`. Flush cached samples if the
    // incoming zoom differs from the one the cache was built at, so
    // stale spacing doesn't leak into this frame.
    cache.ensure_zoom(camera_zoom);

    // Theme variable map — every color string we hand off to render
    // elements is run through `resolve_var` first so authors can use
    // `var(--name)` anywhere a literal hex was accepted.
    let vars = &map.canvas.theme_variables;

    // Per-node pass: emits `TextElement`s + `BorderElement`s and
    // computes the clip AABBs the connection pass below consumes.
    let (text_elements, border_elements, node_aabbs) = build_node_elements(map, offsets);

    // Build connection elements from edges
    let default_config = GlyphConnectionConfig::default();
    let mut connection_elements = Vec::new();
    // Grab-handles for the currently selected edge. Populated at most
    // once per scene build (selection is single-edge); empty otherwise.
    let mut edge_handles: Vec<EdgeHandleElement> = Vec::new();
    // Keys seen this frame — used after the loop to evict stale cache
    // entries for edges that were removed from the model between builds.
    let mut seen_keys: HashSet<EdgeKey> = HashSet::with_capacity(map.edges.len());
    for edge in &map.edges {
        if !edge.visible {
            continue;
        }
        let from_node = match map.nodes.get(&edge.from_id) {
            Some(n) => n,
            None => continue,
        };
        let to_node = match map.nodes.get(&edge.to_id) {
            Some(n) => n,
            None => continue,
        };
        if map.is_hidden_by_fold(from_node) || map.is_hidden_by_fold(to_node) {
            continue;
        }

        let edge_key = EdgeKey::from_edge(edge);
        seen_keys.insert(edge_key.clone());

        // Resolve glyph config: edge override > canvas default > hardcoded default
        let config = edge.glyph_connection.as_ref()
            .or(map.canvas.default_connection.as_ref())
            .unwrap_or(&default_config);

        let is_selected = selected_edge.map_or(false, |(f, t, ty)| {
            f == edge.from_id && t == edge.to_id && ty == edge.edge_type
        });

        // Emit grab-handles for the selected edge. Done once, from
        // the LIVE edge + current (offset-applied) endpoint positions —
        // the caller may be in the middle of a drag and the handle
        // positions have to track that live state. Cost is bounded
        // (one edge per build) so no cache.
        if is_selected {
            let (fox, foy) = offsets.get(&from_node.id).copied().unwrap_or((0.0, 0.0));
            let (tox, toy) = offsets.get(&to_node.id).copied().unwrap_or((0.0, 0.0));
            let from_pos = Vec2::new(
                from_node.position.x as f32 + fox,
                from_node.position.y as f32 + foy,
            );
            let from_size = Vec2::new(
                from_node.size.width as f32,
                from_node.size.height as f32,
            );
            let to_pos = Vec2::new(
                to_node.position.x as f32 + tox,
                to_node.position.y as f32 + toy,
            );
            let to_size = Vec2::new(
                to_node.size.width as f32,
                to_node.size.height as f32,
            );
            edge_handles.extend(build_edge_handles(
                edge, &edge_key, from_pos, from_size, to_pos, to_size,
            ));
        }

        // Did either endpoint of THIS edge move this frame?
        let endpoint_moved = offsets.contains_key(&from_node.id)
            || offsets.contains_key(&to_node.id);

        // --- Fast path: cached geometry is still valid ---
        //
        // If the endpoints haven't moved and we have a cached entry for
        // this edge, reuse the cached pre-clip samples and skip
        // `build_connection_path` / `sample_path` entirely. The cheap
        // clip filter still runs against THIS frame's `node_aabbs` so a
        // stable edge correctly clips around a third node that moved
        // through its path.
        // Color picker preview: resolve once here so both the cached
        // and slow paths pick it up. Preview beats selection on the
        // previewed edge so the user's live feedback is visible on the
        // connection body, not masked by the cyan selection highlight.
        let preview_for_this_edge: Option<&str> = edge_color_preview
            .and_then(|p| if *p.edge_key == edge_key { Some(p.color) } else { None });

        if !endpoint_moved {
            if let Some(cached) = cache.get(&edge_key) {
                let color = if let Some(p) = preview_for_this_edge {
                    resolve_var(p, vars).to_string()
                } else if is_selected {
                    SELECTED_EDGE_COLOR.to_string()
                } else {
                    cached.color.clone()
                };
                let cap_start = match &cached.cap_start {
                    Some((g, p)) if !point_inside_any_node(*p, &node_aabbs) => {
                        Some((g.clone(), (p.x, p.y)))
                    }
                    _ => None,
                };
                let cap_end = match &cached.cap_end {
                    Some((g, p)) if !point_inside_any_node(*p, &node_aabbs) => {
                        Some((g.clone(), (p.x, p.y)))
                    }
                    _ => None,
                };
                let glyph_positions: Vec<(f32, f32)> = cached
                    .pre_clip_positions
                    .iter()
                    .filter(|p| !point_inside_any_node(**p, &node_aabbs))
                    .map(|p| (p.x, p.y))
                    .collect();
                if glyph_positions.is_empty() && cap_start.is_none() && cap_end.is_none() {
                    continue;
                }
                connection_elements.push(ConnectionElement {
                    edge_key,
                    glyph_positions,
                    body_glyph: cached.body_glyph.clone(),
                    cap_start,
                    cap_end,
                    font: cached.font.clone(),
                    font_size_pt: cached.font_size_pt,
                    color,
                });
                continue;
            }
        }

        // --- Slow path: sample fresh and update the cache ---
        let stored_color = {
            // The color we STORE in the cache is the resolved-but-unselected
            // color. Selection overrides are applied at read time above so
            // selection changes don't invalidate the cache.
            let raw = config.color.as_deref().unwrap_or(edge.color.as_str());
            resolve_var(raw, vars).to_string()
        };
        let color = if let Some(p) = preview_for_this_edge {
            resolve_var(p, vars).to_string()
        } else if is_selected {
            SELECTED_EDGE_COLOR.to_string()
        } else {
            stored_color.clone()
        };
        // Canvas-space font size clamped to keep the on-screen glyph
        // size inside [min_font_size_pt, max_font_size_pt]. At extreme
        // zoom-out this inflates the canvas-space size so sample
        // spacing grows and the per-edge glyph count falls — the LOD
        // mechanism that keeps zoomed-out connections from becoming a
        // dust cloud.
        let font_size = config.effective_font_size_pt(camera_zoom);
        let approx_glyph_width = font_size * 0.6;
        let effective_spacing = approx_glyph_width + config.spacing;

        let (fox, foy) = offsets.get(&from_node.id).copied().unwrap_or((0.0, 0.0));
        let (tox, toy) = offsets.get(&to_node.id).copied().unwrap_or((0.0, 0.0));

        let from_pos = Vec2::new(from_node.position.x as f32 + fox, from_node.position.y as f32 + foy);
        let from_size = Vec2::new(from_node.size.width as f32, from_node.size.height as f32);
        let to_pos = Vec2::new(to_node.position.x as f32 + tox, to_node.position.y as f32 + toy);
        let to_size = Vec2::new(to_node.size.width as f32, to_node.size.height as f32);

        let path = connection::build_connection_path(
            from_pos, from_size, edge.anchor_from,
            to_pos, to_size, edge.anchor_to,
            &edge.control_points,
        );
        let samples = connection::sample_path(&path, effective_spacing);
        if samples.is_empty() {
            // Edge produces no samples; make sure any stale cache entry is
            // dropped so we re-try next frame.
            cache.invalidate_edge(&edge_key);
            continue;
        }

        // Caps live at the ORIGINAL first and last sample positions (the
        // anchor points resolved from the source/target node bounds).
        // Those points sit on the raw node edge — which is ON the clip
        // AABB boundary for an unframed node (so they survive clipping)
        // but INSIDE the expanded clip AABB for a framed node (so they
        // get dropped along with the body glyphs that would also render
        // inside the frame area).
        let first_pos = samples[0].position;
        let last_pos = samples.last().unwrap().position;
        let cached_cap_start = config.cap_start.as_ref().map(|g| (g.clone(), first_pos));
        let cached_cap_end = config.cap_end.as_ref().map(|g| (g.clone(), last_pos));

        let pre_clip_positions: Vec<Vec2> = samples.iter().map(|s| s.position).collect();

        // Write fresh geometry back into the cache BEFORE applying the
        // frame-specific clip filter so next frame can reuse it.
        cache.insert(
            edge_key.clone(),
            CachedConnection {
                pre_clip_positions: pre_clip_positions.clone(),
                cap_start: cached_cap_start.clone(),
                cap_end: cached_cap_end.clone(),
                body_glyph: config.body.clone(),
                font: config.font.clone(),
                font_size_pt: font_size,
                color: stored_color,
            },
        );

        // Now produce the post-clip element for THIS frame.
        let cap_start = match cached_cap_start {
            Some((g, p)) if !point_inside_any_node(p, &node_aabbs) => {
                Some((g, (p.x, p.y)))
            }
            _ => None,
        };
        let cap_end = match cached_cap_end {
            Some((g, p)) if !point_inside_any_node(p, &node_aabbs) => {
                Some((g, (p.x, p.y)))
            }
            _ => None,
        };
        let glyph_positions: Vec<(f32, f32)> = pre_clip_positions
            .iter()
            .filter(|p| !point_inside_any_node(**p, &node_aabbs))
            .map(|p| (p.x, p.y))
            .collect();

        // If every sample was clipped (e.g. an entirely-internal edge),
        // there's nothing to draw for the body — skip the element unless a
        // cap survives to represent the connection.
        if glyph_positions.is_empty() && cap_start.is_none() && cap_end.is_none() {
            continue;
        }

        connection_elements.push(ConnectionElement {
            edge_key,
            glyph_positions,
            body_glyph: config.body.clone(),
            cap_start,
            cap_end,
            font: config.font.clone(),
            font_size_pt: font_size,
            color,
        });
    }

    // Evict any cache entries for edges that were in the cache but NOT in
    // the map this frame — handles edges that were deleted between builds.
    cache.retain_keys(&seen_keys);

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

    // Portal pass — two markers per visible pair, colored by
    // preview > selection > portal default.
    let portal_elements = build_portal_elements(
        map,
        offsets,
        selected_portal,
        portal_color_preview,
    );


    RenderScene {
        text_elements,
        border_elements,
        connection_elements,
        portal_elements,
        edge_handles,
        connection_label_elements,
        background_color: resolve_var(&map.canvas.background_color, vars).to_string(),
    }
}

/// Returns true if `point` is strictly inside any of the given AABBs. Uses a
/// small epsilon so points that sit exactly on a border (e.g. connection
/// anchor points, which are placed at node-edge midpoints) are NOT
/// considered inside — that would accidentally clip the endpoints.
pub(super) fn point_inside_any_node(point: Vec2, aabbs: &[(Vec2, Vec2)]) -> bool {
    const EDGE_EPSILON: f32 = 0.5;
    for (pos, size) in aabbs {
        if point.x > pos.x + EDGE_EPSILON
            && point.x < pos.x + size.x - EDGE_EPSILON
            && point.y > pos.y + EDGE_EPSILON
            && point.y < pos.y + size.y - EDGE_EPSILON
        {
            return true;
        }
    }
    false
}

