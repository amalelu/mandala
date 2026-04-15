//! Scene builders — `build_scene`, `build_scene_with_cache`, and
//! the edge-handle layout helper. Split out from `mod.rs` to keep
//! the type module small and greppable.

use std::collections::{HashMap, HashSet};

use glam::Vec2;

use crate::mindmap::border::BorderStyle;
use crate::mindmap::connection;
use crate::mindmap::model::{GlyphConnectionConfig, MindMap, TextRun};
use crate::mindmap::scene_cache::{CachedConnection, EdgeKey, SceneConnectionCache};
use crate::util::color::resolve_var;

use super::{
    BorderElement, ConnectionElement, ConnectionLabelElement, EdgeColorPreview,
    EdgeHandleElement, EdgeHandleKind, PortalColorPreview, PortalElement, PortalRefKey,
    RenderScene, TextElement, EDGE_HANDLE_FONT_SIZE_PT, EDGE_HANDLE_GLYPH, SELECTED_EDGE_COLOR,
};

/// Build the grab-handle set for a single selected edge, given the
/// current (offset-applied) positions and sizes of its endpoint
/// nodes. Called once per scene build (for the selected edge only),
/// so the cost is trivial and needs no cache.
///
/// Always emits AnchorFrom + AnchorTo. On top of that:
/// - an edge with 0 control points gets a `Midpoint` handle
///   (dragging it curves the straight line);
/// - an edge with ≥ 1 control points gets `ControlPoint(i)` handles
///   at each stored offset-from-center.
pub fn build_edge_handles(
    edge: &crate::mindmap::model::MindEdge,
    edge_key: &EdgeKey,
    from_pos: Vec2,
    from_size: Vec2,
    to_pos: Vec2,
    to_size: Vec2,
) -> Vec<EdgeHandleElement> {
    let path = connection::build_connection_path(
        from_pos, from_size, edge.anchor_from,
        to_pos, to_size, edge.anchor_to,
        &edge.control_points,
    );
    let (start, end) = match &path {
        connection::ConnectionPath::Straight { start, end } => (*start, *end),
        connection::ConnectionPath::CubicBezier { start, end, .. } => (*start, *end),
    };

    let from_center = Vec2::new(from_pos.x + from_size.x * 0.5, from_pos.y + from_size.y * 0.5);
    let to_center = Vec2::new(to_pos.x + to_size.x * 0.5, to_pos.y + to_size.y * 0.5);

    let make = |kind: EdgeHandleKind, position: Vec2| EdgeHandleElement {
        edge_key: edge_key.clone(),
        kind,
        position: (position.x, position.y),
        glyph: EDGE_HANDLE_GLYPH.to_string(),
        color: SELECTED_EDGE_COLOR.to_string(),
        font_size_pt: EDGE_HANDLE_FONT_SIZE_PT,
    };

    let mut handles = Vec::with_capacity(5);
    handles.push(make(EdgeHandleKind::AnchorFrom, start));
    handles.push(make(EdgeHandleKind::AnchorTo, end));

    match edge.control_points.len() {
        0 => {
            // Straight edge: offer a midpoint handle that starts a
            // "curve this line" gesture on drag.
            let mid = start.lerp(end, 0.5);
            handles.push(make(EdgeHandleKind::Midpoint, mid));
        }
        1 => {
            // Quadratic Bezier (stored as 1 CP offset from from_center).
            let cp0 = from_center + Vec2::new(
                edge.control_points[0].x as f32,
                edge.control_points[0].y as f32,
            );
            handles.push(make(EdgeHandleKind::ControlPoint(0), cp0));
        }
        _ => {
            // Cubic Bezier (stored as 2 CPs: cp[0] from from_center,
            // cp[1] from to_center).
            let cp0 = from_center + Vec2::new(
                edge.control_points[0].x as f32,
                edge.control_points[0].y as f32,
            );
            let cp1 = to_center + Vec2::new(
                edge.control_points[1].x as f32,
                edge.control_points[1].y as f32,
            );
            handles.push(make(EdgeHandleKind::ControlPoint(0), cp0));
            handles.push(make(EdgeHandleKind::ControlPoint(1), cp1));
        }
    }

    handles
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

    let mut text_elements = Vec::new();
    let mut border_elements = Vec::new();
    // Axis-aligned bounding boxes of every visible node (with drag offsets
    // applied). Used below to clip connection glyphs that would otherwise
    // render over the interior of a node.
    let mut node_aabbs: Vec<(Vec2, Vec2)> = Vec::new();

    // Theme variable map — every color string we hand off to render
    // elements is run through `resolve_var` first so authors can use
    // `var(--name)` anywhere a literal hex was accepted.
    let vars = &map.canvas.theme_variables;

    for node in map.nodes.values() {
        if map.is_hidden_by_fold(node) {
            continue;
        }

        let (ox, oy) = offsets.get(&node.id).copied().unwrap_or((0.0, 0.0));
        let pos_x = node.position.x as f32 + ox;
        let pos_y = node.position.y as f32 + oy;
        let size_x = node.size.width as f32;
        let size_y = node.size.height as f32;

        // Resolve the frame color through theme variables once — used for
        // both the clip AABB sizing and the border element below.
        let frame_color = resolve_var(&node.style.frame_color, vars);

        // Clip AABB: when a node has a visible frame, the rendered border
        // extends beyond the raw node rect by roughly one border
        // `font_size` vertically and one `approx_char_width` horizontally.
        // Expand the clip box to match so connection glyphs don't land
        // inside the visible frame area (see renderer::rebuild_border_buffers
        // for the matching layout math).
        let (clip_pos, clip_size) = if node.style.show_frame {
            let border_style = BorderStyle::default_with_color(frame_color);
            let bf = border_style.font_size_pt;
            let bcw = bf * crate::mindmap::border::BORDER_APPROX_CHAR_WIDTH_FRAC;
            (
                Vec2::new(pos_x - bcw, pos_y - bf),
                Vec2::new(size_x + 2.0 * bcw, size_y + 2.0 * bf),
            )
        } else {
            (Vec2::new(pos_x, pos_y), Vec2::new(size_x, size_y))
        };
        node_aabbs.push((clip_pos, clip_size));

        // Text element (skip empty text nodes). Resolve each text run's
        // color through theme variables so the renderer downstream never
        // sees a `var(--name)` literal.
        if !node.text.is_empty() {
            let resolved_runs: Vec<TextRun> = node.text_runs.iter().map(|run| {
                let mut r = run.clone();
                r.color = resolve_var(&run.color, vars).to_string();
                r
            }).collect();
            text_elements.push(TextElement {
                node_id: node.id.clone(),
                text: node.text.clone(),
                text_runs: resolved_runs,
                position: (pos_x, pos_y),
                size: (size_x, size_y),
            });
        }

        // Border element
        if node.style.show_frame {
            let border_style = BorderStyle::default_with_color(frame_color);
            border_elements.push(BorderElement {
                node_id: node.id.clone(),
                border_style,
                node_position: (pos_x, pos_y),
                node_size: (size_x, size_y),
            });
        }
    }

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

    // Session 6D: emit connection labels as a separate pass. Labels
    // are ≤ 1 per edge and rebuilt each frame at trivial cost, so we
    // don't integrate them into the hot cache path above. Rebuild the
    // path for each labeled edge, look up the point at
    // `edge.label_position_t` (defaulting to 0.5), and emit a
    // `ConnectionLabelElement` centered on that point.
    //
    // If `label_edit_override` is `Some((edge_key, buffer))`, the
    // matching edge's committed label is replaced in the emitted
    // element with `buffer + caret` so the inline label editor's
    // live buffer renders on the next frame. The substitution
    // happens here — in the scene builder — rather than in the
    // renderer, so the scene stays the single source of truth for
    // what will be drawn.
    let mut connection_label_elements: Vec<ConnectionLabelElement> = Vec::new();
    let mut label_override_emitted = false;
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
        let is_edited = label_edit_override
            .map_or(false, |(k, _)| *k == edge_key);

        // The label text to render: either the inline edit buffer
        // (plus caret) for the currently-edited edge, or the
        // committed label. Committed-empty non-edited edges skip
        // emission entirely.
        let rendered_label: String = if is_edited {
            let (_, buf) = label_edit_override.unwrap();
            format!("{buf}\u{258C}")
        } else {
            match edge.label.as_deref() {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => continue,
            }
        };

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

        let path = connection::build_connection_path(
            from_pos,
            from_size,
            edge.anchor_from,
            to_pos,
            to_size,
            edge.anchor_to,
            &edge.control_points,
        );
        let t = edge.label_position_t.unwrap_or(0.5);
        let anchor = connection::point_at_t(&path, t);

        let config = GlyphConnectionConfig::resolved_for(edge, &map.canvas);
        let base_font_size = config.effective_font_size_pt(camera_zoom);
        // Labels render slightly larger than the body glyphs so they
        // read as a distinct element on top of the connection path.
        let font_size_pt = base_font_size * 1.1;
        // Color picker preview: substitute the preview hex for this
        // edge's label color if the preview targets it. Applied
        // before `resolve_var` so `var(--accent)`-style preview values
        // still theme-resolve correctly.
        let raw_color: &str = edge_color_preview
            .and_then(|p| if *p.edge_key == edge_key { Some(p.color) } else { None })
            .unwrap_or_else(|| config.color.as_deref().unwrap_or(edge.color.as_str()));
        let color = resolve_var(raw_color, vars).to_string();

        // Loose AABB sized from the glyph-count approximation
        // (`font_size * 0.6` per glyph — same constant the connection
        // body sampler uses). Height is one font-size plus a small
        // vertical margin.
        let char_count = rendered_label.chars().count() as f32;
        let bounds_w = (char_count * font_size_pt * 0.6).max(font_size_pt);
        let bounds_h = font_size_pt * 1.3;
        // Center the AABB on the path anchor.
        let top_left = (anchor.x - bounds_w * 0.5, anchor.y - bounds_h * 0.5);

        if is_edited {
            label_override_emitted = true;
        }

        connection_label_elements.push(ConnectionLabelElement {
            edge_key,
            text: rendered_label,
            position: top_left,
            bounds: (bounds_w, bounds_h),
            color,
            font: config.font.clone(),
            font_size_pt,
        });
    }

    // If the label edit override targets an edge whose committed
    // label was empty / None (so the normal loop above skipped it),
    // synthesize a label element anyway so the caret is visible
    // while typing the very first character. This fixes the gap in
    // the previous renderer-side override path, whose "belt and
    // suspenders" branch was a dead no-op for exactly this case.
    if let Some((target_key, buffer)) = label_edit_override {
        if !label_override_emitted {
            if let Some(edge) = map.edges.iter().find(|e| {
                e.visible && EdgeKey::from_edge(e) == *target_key
            }) {
                if let (Some(from_node), Some(to_node)) = (
                    map.nodes.get(&edge.from_id),
                    map.nodes.get(&edge.to_id),
                ) {
                    if !map.is_hidden_by_fold(from_node)
                        && !map.is_hidden_by_fold(to_node)
                    {
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
                        let path = connection::build_connection_path(
                            from_pos,
                            from_size,
                            edge.anchor_from,
                            to_pos,
                            to_size,
                            edge.anchor_to,
                            &edge.control_points,
                        );
                        let t = edge.label_position_t.unwrap_or(0.5);
                        let anchor = connection::point_at_t(&path, t);
                        let config = GlyphConnectionConfig::resolved_for(edge, &map.canvas);
                        let base_font_size = config.effective_font_size_pt(camera_zoom);
                        let font_size_pt = base_font_size * 1.1;
                        // The synthesized-label path is for an edge
                        // being edited with an empty committed label —
                        // if the color picker is also previewing
                        // this edge, substitute the preview value.
                        let raw_color: &str = edge_color_preview
                            .and_then(|p| {
                                if p.edge_key == target_key {
                                    Some(p.color)
                                } else {
                                    None
                                }
                            })
                            .unwrap_or_else(|| config.color.as_deref().unwrap_or(edge.color.as_str()));
                        let color = resolve_var(raw_color, vars).to_string();
                        let rendered = format!("{buffer}\u{258C}");
                        let char_count = rendered.chars().count() as f32;
                        let bounds_w = (char_count * font_size_pt * 0.6).max(font_size_pt);
                        let bounds_h = font_size_pt * 1.3;
                        let top_left = (anchor.x - bounds_w * 0.5, anchor.y - bounds_h * 0.5);
                        connection_label_elements.push(ConnectionLabelElement {
                            edge_key: target_key.clone(),
                            text: rendered,
                            position: top_left,
                            bounds: (bounds_w, bounds_h),
                            color,
                            font: config.font.clone(),
                            font_size_pt,
                        });
                    }
                }
            }
        }
    }

    // Session 6E: emit portal markers as a post-pass over `map.portals`.
    // Each pair produces two `PortalElement`s — one floating above the
    // top-right corner of each endpoint node. Portals whose endpoints
    // are missing or hidden by fold are skipped silently. Portal colors
    // resolve through the theme variable map so `var(--accent)` auto-
    // restyles on theme swap, matching the connection-label pattern.
    let mut portal_elements: Vec<PortalElement> = Vec::new();
    for portal in &map.portals {
        let node_a = match map.nodes.get(&portal.endpoint_a) {
            Some(n) => n,
            None => continue,
        };
        let node_b = match map.nodes.get(&portal.endpoint_b) {
            Some(n) => n,
            None => continue,
        };
        if map.is_hidden_by_fold(node_a) || map.is_hidden_by_fold(node_b) {
            continue;
        }
        let is_selected = selected_portal.map_or(false, |(l, a, b)| {
            l == portal.label && a == portal.endpoint_a && b == portal.endpoint_b
        });
        let key = PortalRefKey::from_portal(portal);
        // Color picker preview beats selection on the previewed
        // portal so the user's live feedback is visible on both
        // markers — the same rule as the edge body path.
        let preview_for_this_portal: Option<&str> = portal_color_preview
            .and_then(|p| if *p.portal_key == key { Some(p.color) } else { None });
        let raw_color: &str = if let Some(p) = preview_for_this_portal {
            p
        } else if is_selected {
            SELECTED_PORTAL_COLOR_HEX
        } else {
            portal.color.as_str()
        };
        let color = resolve_var(raw_color, vars).to_string();

        for endpoint in [node_a, node_b] {
            let (ox, oy) = offsets.get(&endpoint.id).copied().unwrap_or((0.0, 0.0));
            let node_x = endpoint.position.x as f32 + ox;
            let node_y = endpoint.position.y as f32 + oy;
            let node_w = endpoint.size.width as f32;

            // Loose square AABB sized from the glyph font; matches the
            // connection-label sizing heuristic (≈0.6 × font_size per
            // char, one char wide for the single marker glyph).
            let bounds_w = portal.font_size_pt * 1.4;
            let bounds_h = portal.font_size_pt * 1.4;
            // Float the marker just above the node's top-right corner.
            let top_left = (
                node_x + node_w - bounds_w * 0.9,
                node_y - bounds_h - 8.0,
            );

            portal_elements.push(PortalElement {
                portal_ref: key.clone(),
                endpoint_node_id: endpoint.id.clone(),
                glyph: portal.glyph.clone(),
                position: top_left,
                bounds: (bounds_w, bounds_h),
                color: color.clone(),
                font: portal.font.clone(),
                font_size_pt: portal.font_size_pt,
            });
        }
    }

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

/// Session 6E: cyan highlight hex for selected portals. Matches the
/// `HIGHLIGHT_COLOR` constant in `document.rs` that drives the
/// node-selection color mutation.
const SELECTED_PORTAL_COLOR_HEX: &str = "#00E5FF";

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

