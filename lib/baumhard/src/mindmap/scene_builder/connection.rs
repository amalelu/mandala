//! Connection element emission — the cache-coupled pass. For each
//! visible edge: fast path if the endpoints haven't moved AND the
//! cache already has a pre-clip sample set; slow path otherwise
//! (build path, sample, write cache). The cheap clip filter runs
//! against the current frame's `node_aabbs` on both paths so a
//! stable edge correctly clips around third nodes that moved
//! through its path this frame.
//!
//! Cache lifecycle: `ensure_zoom` is caller-managed (the
//! orchestrator flushes on zoom change before any pass starts);
//! `retain_keys` runs here at the end of the loop so eviction of
//! deleted edges stays colocated with the keys-seen bookkeeping.
//!
//! Selected-edge handle emission rides along in the same loop:
//! single-edge selection means at most one handle batch per scene
//! build, so there's no cost to bundling it with the connection
//! pass rather than adding a separate iteration.

use std::collections::{HashMap, HashSet};

use glam::Vec2;

use crate::mindmap::connection;
use crate::mindmap::model::{GlyphConnectionConfig, MindMap};
use crate::mindmap::scene_cache::{CachedConnection, EdgeKey, SceneConnectionCache};
use crate::util::color::resolve_var;

use super::edge_handle::build_edge_handles;
use super::{
    ConnectionElement, EdgeColorPreview, EdgeHandleElement, SELECTED_EDGE_COLOR,
};

/// Emit connection elements + edge-handle elements. Consumes
/// `node_aabbs` from the node pass for the clip filter; mutates
/// `cache` on slow-path edges and after the loop (retain_keys
/// evicts deleted edges).
pub(super) fn build_connection_elements(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    node_aabbs: &[(Vec2, Vec2)],
    selected_edge: Option<(&str, &str, &str)>,
    edge_color_preview: Option<EdgeColorPreview<'_>>,
    cache: &mut SceneConnectionCache,
    camera_zoom: f32,
) -> (Vec<ConnectionElement>, Vec<EdgeHandleElement>) {
    let vars = &map.canvas.theme_variables;
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
        // Portal-mode edges render as markers in the portal pass,
        // not as a path. Skip them here so the connection pipeline
        // (sampling, clipping, edge handles, labels) never touches
        // an edge that has no line form.
        if crate::mindmap::model::is_portal_edge(edge) {
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
        let config = edge
            .glyph_connection
            .as_ref()
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
            let from_size = Vec2::new(from_node.size.width as f32, from_node.size.height as f32);
            let to_pos = Vec2::new(
                to_node.position.x as f32 + tox,
                to_node.position.y as f32 + toy,
            );
            let to_size = Vec2::new(to_node.size.width as f32, to_node.size.height as f32);
            edge_handles.extend(build_edge_handles(
                edge, &edge_key, from_pos, from_size, to_pos, to_size,
            ));
        }

        // Did either endpoint of THIS edge move this frame?
        let endpoint_moved =
            offsets.contains_key(&from_node.id) || offsets.contains_key(&to_node.id);

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
        let preview_for_this_edge: Option<&str> = edge_color_preview.and_then(|p| {
            if *p.edge_key == edge_key {
                Some(p.color)
            } else {
                None
            }
        });

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
                    Some((g, p)) if !point_inside_any_node(*p, node_aabbs) => {
                        Some((g.clone(), (p.x, p.y)))
                    }
                    _ => None,
                };
                let cap_end = match &cached.cap_end {
                    Some((g, p)) if !point_inside_any_node(*p, node_aabbs) => {
                        Some((g.clone(), (p.x, p.y)))
                    }
                    _ => None,
                };
                let glyph_positions: Vec<(f32, f32)> = cached
                    .pre_clip_positions
                    .iter()
                    .filter(|p| !point_inside_any_node(**p, node_aabbs))
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
                    zoom_visibility: edge.zoom_window(),
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

        let from_pos = Vec2::new(
            from_node.position.x as f32 + fox,
            from_node.position.y as f32 + foy,
        );
        let from_size = Vec2::new(from_node.size.width as f32, from_node.size.height as f32);
        let to_pos = Vec2::new(
            to_node.position.x as f32 + tox,
            to_node.position.y as f32 + toy,
        );
        let to_size = Vec2::new(to_node.size.width as f32, to_node.size.height as f32);

        let path = connection::build_connection_path(
            from_pos,
            from_size,
            &edge.anchor_from,
            to_pos,
            to_size,
            &edge.anchor_to,
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
            Some((g, p)) if !point_inside_any_node(p, node_aabbs) => Some((g, (p.x, p.y))),
            _ => None,
        };
        let cap_end = match cached_cap_end {
            Some((g, p)) if !point_inside_any_node(p, node_aabbs) => Some((g, (p.x, p.y))),
            _ => None,
        };
        let glyph_positions: Vec<(f32, f32)> = pre_clip_positions
            .iter()
            .filter(|p| !point_inside_any_node(**p, node_aabbs))
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
            zoom_visibility: edge.zoom_window(),
        });
    }

    // Evict any cache entries for edges that were in the cache but NOT in
    // the map this frame — handles edges that were deleted between builds.
    cache.retain_keys(&seen_keys);

    (connection_elements, edge_handles)
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
