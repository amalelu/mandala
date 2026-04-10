use std::collections::{HashMap, HashSet};
use crate::mindmap::border::BorderStyle;
use crate::mindmap::connection;
use crate::mindmap::model::{GlyphConnectionConfig, MindMap, TextRun};
use crate::mindmap::scene_cache::{CachedConnection, EdgeKey, SceneConnectionCache};
use crate::util::color::resolve_var;
use glam::Vec2;

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

/// Placeholder for future portal rendering (M3).
pub struct PortalElement {}

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

/// Color override applied to the `ConnectionElement` of a selected edge.
/// Kept in sync visually with the cyan node selection highlight in
/// `src/application/document.rs::HIGHLIGHT_COLOR`.
const SELECTED_EDGE_COLOR: &str = "#00E5FF";

/// Glyph used for edge grab-handles in Session 6C's connection
/// reshape surface. A solid black diamond reads as a clickable
/// control point across most fonts.
const EDGE_HANDLE_GLYPH: &str = "\u{25C6}"; // ◆

/// Font size (in points) for the edge handle glyphs. Slightly larger
/// than the default connection glyph size so handles stand out on top
/// of the selected edge.
const EDGE_HANDLE_FONT_SIZE_PT: f32 = 14.0;

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
    build_scene_with_offsets_and_selection(map, &HashMap::new(), None, camera_zoom)
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
    build_scene_with_offsets_and_selection(map, offsets, None, camera_zoom)
}

/// Thin wrapper over the cache-aware builder that uses a scratch
/// (throwaway) cache so call sites that don't track a persistent cache
/// still work. Prefer `build_scene_with_cache` on the hot drag path.
pub fn build_scene_with_offsets_and_selection(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    selected_edge: Option<(&str, &str, &str)>,
    camera_zoom: f32,
) -> RenderScene {
    let mut scratch = SceneConnectionCache::new();
    build_scene_with_cache(map, offsets, selected_edge, &mut scratch, camera_zoom)
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
            let bcw = bf * 0.6;
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
        if !endpoint_moved {
            if let Some(cached) = cache.get(&edge_key) {
                let color = if is_selected {
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
        let color = if is_selected {
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

    RenderScene {
        text_elements,
        border_elements,
        connection_elements,
        portal_elements: Vec::new(),
        edge_handles,
        background_color: resolve_var(&map.canvas.background_color, vars).to_string(),
    }
}

/// Returns true if `point` is strictly inside any of the given AABBs. Uses a
/// small epsilon so points that sit exactly on a border (e.g. connection
/// anchor points, which are placed at node-edge midpoints) are NOT
/// considered inside — that would accidentally clip the endpoints.
fn point_inside_any_node(point: Vec2, aabbs: &[(Vec2, Vec2)]) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mindmap::loader;
    use std::path::PathBuf;

    fn test_map_path() -> PathBuf {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.pop(); // lib/baumhard -> lib
        path.pop(); // lib -> root
        path.push("maps/testament.mindmap.json");
        path
    }

    #[test]
    fn test_point_inside_any_node_strictly_inside() {
        let aabbs = vec![
            (Vec2::new(0.0, 0.0), Vec2::new(100.0, 50.0)),
        ];
        assert!(point_inside_any_node(Vec2::new(50.0, 25.0), &aabbs));
    }

    #[test]
    fn test_point_inside_any_node_on_boundary_is_not_inside() {
        // A point exactly on the right edge should NOT be considered
        // inside — this is where connection anchor points live.
        let aabbs = vec![
            (Vec2::new(0.0, 0.0), Vec2::new(100.0, 50.0)),
        ];
        assert!(!point_inside_any_node(Vec2::new(100.0, 25.0), &aabbs));
        assert!(!point_inside_any_node(Vec2::new(0.0, 25.0), &aabbs));
        assert!(!point_inside_any_node(Vec2::new(50.0, 0.0), &aabbs));
        assert!(!point_inside_any_node(Vec2::new(50.0, 50.0), &aabbs));
    }

    #[test]
    fn test_point_inside_any_node_outside_returns_false() {
        let aabbs = vec![
            (Vec2::new(0.0, 0.0), Vec2::new(100.0, 50.0)),
        ];
        assert!(!point_inside_any_node(Vec2::new(200.0, 25.0), &aabbs));
        assert!(!point_inside_any_node(Vec2::new(-10.0, 25.0), &aabbs));
    }

    #[test]
    fn test_point_inside_any_node_checks_all_aabbs() {
        let aabbs = vec![
            (Vec2::new(0.0, 0.0), Vec2::new(10.0, 10.0)),
            (Vec2::new(100.0, 100.0), Vec2::new(50.0, 50.0)),
        ];
        // Inside the second box
        assert!(point_inside_any_node(Vec2::new(125.0, 125.0), &aabbs));
    }

    // Shared helpers for the synthetic-map scene tests below.
    use crate::mindmap::model::{
        Canvas, MindEdge, MindMap, MindNode, NodeLayout, NodeStyle, Position, Size,
    };

    fn synthetic_node(id: &str, x: f64, y: f64, w: f64, h: f64, show_frame: bool) -> MindNode {
        MindNode {
            id: id.to_string(),
            parent_id: None,
            index: 0,
            position: Position { x, y },
            size: Size { width: w, height: h },
            text: id.to_string(),
            text_runs: vec![],
            style: NodeStyle {
                background_color: "#000".into(),
                frame_color: "#fff".into(),
                text_color: "#fff".into(),
                shape_type: 0,
                corner_radius_percent: 0.0,
                frame_thickness: 1.0,
                show_frame,
                show_shadow: false,
                border: None,
            },
            layout: NodeLayout { layout_type: 0, direction: 0, spacing: 0.0 },
            folded: false,
            notes: String::new(),
            color_schema: None,
            trigger_bindings: vec![],
            inline_mutations: vec![],
        }
    }

    fn synthetic_edge(from: &str, to: &str, anchor_from: i32, anchor_to: i32) -> MindEdge {
        MindEdge {
            from_id: from.to_string(),
            to_id: to.to_string(),
            edge_type: "cross_link".to_string(),
            color: "#fff".to_string(),
            width: 1,
            line_style: 0,
            visible: true,
            label: None,
            anchor_from,
            anchor_to,
            control_points: vec![],
            glyph_connection: None,
        }
    }

    fn synthetic_map(nodes_vec: Vec<MindNode>, edges: Vec<MindEdge>) -> MindMap {
        use std::collections::HashMap;
        let mut nodes = HashMap::new();
        for n in nodes_vec {
            nodes.insert(n.id.clone(), n);
        }
        MindMap {
            version: "1.0".into(),
            name: "test".into(),
            canvas: Canvas {
                background_color: "#000".into(),
                default_border: None,
                default_connection: None,
                theme_variables: HashMap::new(),
                theme_variants: HashMap::new(),
            },
            nodes,
            edges,
            custom_mutations: vec![],
        }
    }

    fn themed_node(id: &str, bg: &str, frame: &str, text: &str) -> MindNode {
        let mut n = synthetic_node(id, 0.0, 0.0, 40.0, 40.0, true);
        n.style.background_color = bg.to_string();
        n.style.frame_color = frame.to_string();
        n.style.text_color = text.to_string();
        n
    }

    #[test]
    fn test_scene_background_resolves_theme_variable() {
        use std::collections::HashMap;
        let mut map = synthetic_map(
            vec![synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false)],
            vec![],
        );
        map.canvas.background_color = "var(--bg)".into();
        let mut vars = HashMap::new();
        vars.insert("--bg".into(), "#123456".into());
        map.canvas.theme_variables = vars;

        let scene = build_scene(&map, 1.0);
        assert_eq!(scene.background_color, "#123456");
    }

    #[test]
    fn test_scene_frame_color_resolves_theme_variable() {
        use std::collections::HashMap;
        let mut map = synthetic_map(
            vec![themed_node("a", "#000", "var(--frame)", "#fff")],
            vec![],
        );
        let mut vars = HashMap::new();
        vars.insert("--frame".into(), "#abcdef".into());
        map.canvas.theme_variables = vars;

        let scene = build_scene(&map, 1.0);
        assert_eq!(scene.border_elements.len(), 1);
        // `BorderStyle::default_with_color` stores the color string as-is
        // on the style; check the resolved hex ends up there.
        let border = &scene.border_elements[0];
        assert_eq!(border.border_style.color, "#abcdef");
    }

    #[test]
    fn test_scene_connection_color_resolves_theme_variable() {
        use std::collections::HashMap;
        let mut a = synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false);
        let mut b = synthetic_node("b", 200.0, 0.0, 40.0, 40.0, false);
        a.text = "".into(); // skip text element
        b.text = "".into();
        let mut edge = synthetic_edge("a", "b", 2, 4);
        edge.color = "var(--edge)".into();
        let mut map = synthetic_map(vec![a, b], vec![edge]);
        let mut vars = HashMap::new();
        vars.insert("--edge".into(), "#fedcba".into());
        map.canvas.theme_variables = vars;

        let scene = build_scene(&map, 1.0);
        assert_eq!(scene.connection_elements.len(), 1);
        assert_eq!(scene.connection_elements[0].color, "#fedcba");
    }

    #[test]
    fn test_scene_missing_variable_passes_through_raw() {
        let mut map = synthetic_map(
            vec![synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false)],
            vec![],
        );
        map.canvas.background_color = "var(--missing)".into();
        let scene = build_scene(&map, 1.0);
        // Unknown var is passed through verbatim — downstream consumers
        // decide how to handle it (hex_to_rgba_safe falls back to the
        // fallback color).
        assert_eq!(scene.background_color, "var(--missing)");
    }

    #[test]
    fn test_scene_clips_connection_glyphs_inside_node() {
        // A on the left, B on the right, blocker C directly on the path
        // between them. The A→B connection should skip body glyphs that
        // fall inside C. All three nodes are unframed so only the raw
        // AABB clipping is exercised here.
        let map = synthetic_map(
            vec![
                synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
                synthetic_node("b", 400.0, 0.0, 40.0, 40.0, false),
                synthetic_node("c", 180.0, 0.0, 60.0, 40.0, false),
            ],
            vec![synthetic_edge("a", "b", 2, 4)], // right edge of A → left edge of B
        );

        let scene = build_scene(&map, 1.0);
        assert_eq!(scene.connection_elements.len(), 1);
        let conn = &scene.connection_elements[0];

        // No body glyph position should fall strictly inside C's AABB.
        for &(x, y) in &conn.glyph_positions {
            let inside_c = x > 180.5 && x < 239.5 && y > 0.5 && y < 39.5;
            assert!(!inside_c,
                "glyph at ({}, {}) should have been clipped by blocker C",
                x, y);
        }
        assert!(!conn.glyph_positions.is_empty(),
            "some glyphs should remain outside the blocker");
    }

    #[test]
    fn test_scene_clips_connection_glyphs_in_frame_area() {
        // Same A→B→blocker layout but this time C has a visible frame.
        // The border at default 14pt font extends ~8.4 px horizontally and
        // ~14 px vertically past C's AABB, so body glyphs in the expanded
        // region should also be clipped.
        let border_font = 14.0_f32;
        let border_char_w = border_font * 0.6;

        let map = synthetic_map(
            vec![
                synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
                synthetic_node("b", 400.0, 0.0, 40.0, 40.0, false),
                synthetic_node("c", 180.0, 0.0, 60.0, 40.0, true),
            ],
            vec![synthetic_edge("a", "b", 2, 4)],
        );

        let scene = build_scene(&map, 1.0);
        assert_eq!(scene.connection_elements.len(), 1);
        let conn = &scene.connection_elements[0];

        // The clip AABB for framed C is expanded by (border_char_w,
        // border_font) on every side. No body glyph should fall inside
        // the expanded region.
        let min_x = 180.0 - border_char_w + 0.5;
        let max_x = 240.0 + border_char_w - 0.5;
        let min_y = 0.0 - border_font + 0.5;
        let max_y = 40.0 + border_font - 0.5;
        for &(x, y) in &conn.glyph_positions {
            let inside_expanded_c =
                x > min_x && x < max_x && y > min_y && y < max_y;
            assert!(!inside_expanded_c,
                "glyph at ({}, {}) should have been clipped by framed C's expanded AABB",
                x, y);
        }
        // Body glyphs should still render in the space between A, C's
        // expanded clip box, and B.
        assert!(!conn.glyph_positions.is_empty(),
            "connection between A and B should still have visible body glyphs outside C's frame");
    }

    #[test]
    fn test_scene_caps_survive_for_unframed_endpoints() {
        // A→B connection with a cap_start glyph configured. Because A and
        // B are unframed, the anchor point sits exactly on A's edge and
        // the cap should render there.
        use crate::mindmap::model::GlyphConnectionConfig;
        let mut edge = synthetic_edge("a", "b", 2, 4);
        edge.glyph_connection = Some(GlyphConnectionConfig {
            body: "·".into(),
            cap_start: Some("►".into()),
            cap_end: Some("◄".into()),
            font: None,
            font_size_pt: 12.0,
            color: None,
            spacing: 0.0,
            ..GlyphConnectionConfig::default()
        });
        let map = synthetic_map(
            vec![
                synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
                synthetic_node("b", 400.0, 0.0, 40.0, 40.0, false),
            ],
            vec![edge],
        );
        let scene = build_scene(&map, 1.0);
        let conn = &scene.connection_elements[0];
        assert!(conn.cap_start.is_some(),
            "cap_start should survive for unframed source");
        assert!(conn.cap_end.is_some(),
            "cap_end should survive for unframed target");
    }

    #[test]
    fn test_scene_caps_clipped_for_framed_endpoints() {
        // A→B connection where the target B has a visible frame. The
        // cap_end sits on B's node edge, which is STRICTLY inside B's
        // frame-expanded clip AABB, so it should be dropped — otherwise
        // the cap would render in the visible border area.
        use crate::mindmap::model::GlyphConnectionConfig;
        let mut edge = synthetic_edge("a", "b", 2, 4);
        edge.glyph_connection = Some(GlyphConnectionConfig {
            body: "·".into(),
            cap_start: Some("►".into()),
            cap_end: Some("◄".into()),
            font: None,
            font_size_pt: 12.0,
            color: None,
            spacing: 0.0,
            ..GlyphConnectionConfig::default()
        });
        let map = synthetic_map(
            vec![
                synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
                synthetic_node("b", 400.0, 0.0, 40.0, 40.0, true), // framed!
            ],
            vec![edge],
        );
        let scene = build_scene(&map, 1.0);
        let conn = &scene.connection_elements[0];
        // Source is unframed — cap_start still shows at A's right edge.
        assert!(conn.cap_start.is_some(),
            "cap_start should survive for unframed source");
        // Target is framed — cap_end falls inside the expanded clip AABB.
        assert!(conn.cap_end.is_none(),
            "cap_end should be clipped when target has a visible frame");
    }

    // --- Phase B cache tests --------------------------------------------

    fn two_node_edge_map() -> MindMap {
        synthetic_map(
            vec![
                synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
                synthetic_node("b", 400.0, 0.0, 40.0, 40.0, false),
            ],
            vec![synthetic_edge("a", "b", 2, 4)],
        )
    }

    #[test]
    fn test_cache_populated_on_first_build() {
        let map = two_node_edge_map();
        let mut cache = SceneConnectionCache::new();
        let scene = build_scene_with_cache(&map, &HashMap::new(), None, &mut cache, 1.0);

        assert_eq!(scene.connection_elements.len(), 1);
        assert_eq!(cache.len(), 1);
        let key = EdgeKey::new("a", "b", "cross_link");
        assert!(cache.get(&key).is_some());
        assert_eq!(cache.edges_touching("a"), std::slice::from_ref(&key));
        assert_eq!(cache.edges_touching("b"), std::slice::from_ref(&key));
    }

    #[test]
    fn test_cache_hit_preserves_sample_identity() {
        // Two builds with empty offsets — the second one should serve
        // from cache. We verify the cache by mutating the cached entry in
        // place between builds and observing that the mutation flows into
        // the second build's output. If the second build had re-sampled,
        // it would have overwritten our mutation with fresh geometry.
        let map = two_node_edge_map();
        let mut cache = SceneConnectionCache::new();
        let _first = build_scene_with_cache(&map, &HashMap::new(), None, &mut cache, 1.0);

        // Mutate the cached entry so we can see whether build #2 read it.
        let key = EdgeKey::new("a", "b", "cross_link");
        // Replace with a sentinel entry with no positions and a unique
        // body glyph. If the cache is used, the second build's
        // ConnectionElement body_glyph will match.
        cache.insert(
            key.clone(),
            CachedConnection {
                pre_clip_positions: vec![Vec2::new(200.0, 20.0)],
                cap_start: None,
                cap_end: None,
                body_glyph: "SENTINEL".into(),
                font: None,
                font_size_pt: 12.0,
                color: "#ff00ff".into(),
            },
        );

        let second = build_scene_with_cache(&map, &HashMap::new(), None, &mut cache, 1.0);
        assert_eq!(second.connection_elements.len(), 1);
        let conn = &second.connection_elements[0];
        assert_eq!(conn.body_glyph, "SENTINEL",
            "cache-hit path should have used the stored entry");
        assert_eq!(conn.color, "#ff00ff");
        // Single cached pre-clip point should have survived the clip
        // filter (it's outside both nodes).
        assert_eq!(conn.glyph_positions.len(), 1);
    }

    #[test]
    fn test_cache_invalidated_on_endpoint_offset() {
        // If endpoint `a` moves, the a↔b edge must be re-sampled — we
        // should observe fresh `body_glyph` on the element, not the
        // sentinel we stashed in the cache.
        let map = two_node_edge_map();
        let mut cache = SceneConnectionCache::new();
        let _first = build_scene_with_cache(&map, &HashMap::new(), None, &mut cache, 1.0);

        let key = EdgeKey::new("a", "b", "cross_link");
        cache.insert(
            key.clone(),
            CachedConnection {
                pre_clip_positions: vec![],
                cap_start: None,
                cap_end: None,
                body_glyph: "SENTINEL".into(),
                font: None,
                font_size_pt: 12.0,
                color: "#ff00ff".into(),
            },
        );

        let mut offsets = HashMap::new();
        offsets.insert("a".to_string(), (10.0, 0.0));
        let second = build_scene_with_cache(&map, &offsets, None, &mut cache, 1.0);
        let conn = &second.connection_elements[0];
        assert_ne!(conn.body_glyph, "SENTINEL",
            "endpoint-moved edge should have been re-sampled");
        // The cache should contain the freshly-resampled entry now.
        let refreshed = cache.get(&key).unwrap();
        assert_ne!(refreshed.body_glyph, "SENTINEL");
        assert!(!refreshed.pre_clip_positions.is_empty());
    }

    #[test]
    fn test_cache_preserves_unrelated_edge_under_drag() {
        // Two edges: a↔b (long) and c↔d (short). Drag node `a`. The c↔d
        // edge should NOT be re-sampled; its cache entry should remain as
        // our sentinel.
        let map = synthetic_map(
            vec![
                synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
                synthetic_node("b", 400.0, 0.0, 40.0, 40.0, false),
                synthetic_node("c", 0.0, 300.0, 40.0, 40.0, false),
                synthetic_node("d", 400.0, 300.0, 40.0, 40.0, false),
            ],
            vec![
                synthetic_edge("a", "b", 2, 4),
                synthetic_edge("c", "d", 2, 4),
            ],
        );
        let mut cache = SceneConnectionCache::new();
        let _first = build_scene_with_cache(&map, &HashMap::new(), None, &mut cache, 1.0);

        let cd_key = EdgeKey::new("c", "d", "cross_link");
        cache.insert(
            cd_key.clone(),
            CachedConnection {
                pre_clip_positions: vec![Vec2::new(200.0, 320.0)],
                cap_start: None,
                cap_end: None,
                body_glyph: "STABLE_SENTINEL".into(),
                font: None,
                font_size_pt: 12.0,
                color: "#00ff00".into(),
            },
        );

        let mut offsets = HashMap::new();
        offsets.insert("a".to_string(), (5.0, 0.0));
        let second = build_scene_with_cache(&map, &offsets, None, &mut cache, 1.0);

        // Find the c↔d connection element and verify it came from the
        // cache unchanged.
        let cd_elem = second
            .connection_elements
            .iter()
            .find(|e| e.edge_key == cd_key)
            .expect("c↔d element should exist");
        assert_eq!(cd_elem.body_glyph, "STABLE_SENTINEL",
            "unrelated edge should have been served from cache, not re-sampled");

        // The a↔b edge should have been re-sampled.
        let ab_key = EdgeKey::new("a", "b", "cross_link");
        let ab_elem = second
            .connection_elements
            .iter()
            .find(|e| e.edge_key == ab_key)
            .expect("a↔b element should exist");
        assert_ne!(ab_elem.body_glyph, "SENTINEL");
    }

    #[test]
    fn test_cache_clip_reruns_against_fresh_aabbs() {
        // Governing-invariant correctness: even when an edge is served
        // from cache, the clip filter must run against the current
        // frame's `node_aabbs`. Here, a stable a↔b edge has a blocker
        // node `c` in the middle. Moving `c` through the edge should
        // change which glyphs survive clipping, even though a↔b itself
        // is served from cache.
        let mut map = synthetic_map(
            vec![
                synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
                synthetic_node("b", 400.0, 0.0, 40.0, 40.0, false),
                // Blocker far above the connection — no clip effect yet.
                synthetic_node("c", 180.0, -500.0, 60.0, 40.0, false),
            ],
            vec![synthetic_edge("a", "b", 2, 4)],
        );

        let mut cache = SceneConnectionCache::new();
        let first = build_scene_with_cache(&map, &HashMap::new(), None, &mut cache, 1.0);
        let first_count = first.connection_elements[0].glyph_positions.len();

        // Now move `c` into the middle of the connection — use a drag
        // offset. `a↔b` is NOT in the dirty set (endpoints didn't move),
        // so it hits the cache path, but the clip filter must still
        // notice `c`'s new position.
        let mut offsets = HashMap::new();
        offsets.insert("c".to_string(), (0.0, 500.0));
        let second = build_scene_with_cache(&map, &offsets, None, &mut cache, 1.0);
        let second_count = second.connection_elements[0].glyph_positions.len();
        assert!(second_count < first_count,
            "moving c through the edge should reduce post-clip glyph count: {} → {}",
            first_count, second_count);

        // Now move `c` back out of the way via a model edit + full rebuild.
        map.nodes.get_mut("c").unwrap().position.y = -500.0;
        let third = build_scene_with_cache(&map, &HashMap::new(), None, &mut cache, 1.0);
        assert_eq!(third.connection_elements[0].glyph_positions.len(), first_count);
    }

    #[test]
    fn test_cache_evicts_deleted_edges() {
        let mut map = two_node_edge_map();
        let mut cache = SceneConnectionCache::new();
        let _first = build_scene_with_cache(&map, &HashMap::new(), None, &mut cache, 1.0);
        let key = EdgeKey::new("a", "b", "cross_link");
        assert!(cache.get(&key).is_some());

        // Remove the edge from the model and rebuild.
        map.edges.clear();
        let second = build_scene_with_cache(&map, &HashMap::new(), None, &mut cache, 1.0);
        assert!(second.connection_elements.is_empty());
        assert!(cache.get(&key).is_none(),
            "deleted edge should be evicted from cache");
    }

    #[test]
    fn test_connection_element_edge_key_always_populated() {
        // Sanity: every ConnectionElement emitted by the cache-aware
        // builder carries a valid EdgeKey matching the source MindEdge.
        // The renderer's keyed buffer map is keyed off this; a missing
        // or wrong edge_key would silently break the incremental path.
        let map = synthetic_map(
            vec![
                synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
                synthetic_node("b", 400.0, 0.0, 40.0, 40.0, false),
                synthetic_node("c", 0.0, 200.0, 40.0, 40.0, false),
            ],
            vec![
                synthetic_edge("a", "b", 2, 4),
                synthetic_edge("b", "c", 2, 4),
            ],
        );
        let mut cache = SceneConnectionCache::new();
        let scene = build_scene_with_cache(&map, &HashMap::new(), None, &mut cache, 1.0);
        assert_eq!(scene.connection_elements.len(), 2);
        let ab = EdgeKey::new("a", "b", "cross_link");
        let bc = EdgeKey::new("b", "c", "cross_link");
        let keys: Vec<&EdgeKey> =
            scene.connection_elements.iter().map(|e| &e.edge_key).collect();
        assert!(keys.contains(&&ab));
        assert!(keys.contains(&&bc));
    }

    #[test]
    fn test_second_cache_hit_produces_identical_output() {
        // Regression guard: build twice with no changes; the two scenes
        // must have byte-equivalent connection_element glyph_positions
        // (same count, same coordinates, same body glyph). This
        // verifies the cache-hit read path returns the same element as
        // a fresh build would.
        let map = two_node_edge_map();
        let mut cache = SceneConnectionCache::new();
        let first = build_scene_with_cache(&map, &HashMap::new(), None, &mut cache, 1.0);
        let second = build_scene_with_cache(&map, &HashMap::new(), None, &mut cache, 1.0);

        assert_eq!(
            first.connection_elements.len(),
            second.connection_elements.len(),
        );
        let a = &first.connection_elements[0];
        let b = &second.connection_elements[0];
        assert_eq!(a.edge_key, b.edge_key);
        assert_eq!(a.glyph_positions, b.glyph_positions);
        assert_eq!(a.body_glyph, b.body_glyph);
        assert_eq!(a.color, b.color);
        assert_eq!(a.font_size_pt, b.font_size_pt);
    }

    #[test]
    fn test_cache_is_empty_after_new() {
        let cache = SceneConnectionCache::new();
        assert_eq!(cache.len(), 0);
        assert!(cache.is_empty());
    }

    #[test]
    fn test_fold_hidden_edge_does_not_populate_cache() {
        // When an endpoint is hidden by fold state, the edge is skipped
        // entirely — it should not appear in the output OR the cache.
        let mut a = synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false);
        let mut b_child = synthetic_node("b", 400.0, 0.0, 40.0, 40.0, false);
        b_child.parent_id = Some("a".to_string());
        a.folded = true; // hides b
        let edge = synthetic_edge("a", "b", 2, 4);
        let map = synthetic_map(vec![a, b_child], vec![edge]);

        let mut cache = SceneConnectionCache::new();
        let scene = build_scene_with_cache(&map, &HashMap::new(), None, &mut cache, 1.0);
        assert!(scene.connection_elements.is_empty(),
            "folded edge should be skipped");
        assert!(cache.is_empty(),
            "folded edge should not appear in cache");
    }

    #[test]
    fn test_cache_selection_change_does_not_invalidate() {
        // Build with no selection → cache populated with the resolved
        // color. Build again with the edge selected → cache entry should
        // not be rewritten; the element's color should still reflect the
        // selection override.
        let map = two_node_edge_map();
        let mut cache = SceneConnectionCache::new();
        let _first = build_scene_with_cache(&map, &HashMap::new(), None, &mut cache, 1.0);
        let key = EdgeKey::new("a", "b", "cross_link");
        let stored_color = cache.get(&key).unwrap().color.clone();

        // Inject a sentinel body_glyph into the cache so we can detect
        // whether the cache path was taken on the second build.
        cache.insert(
            key.clone(),
            CachedConnection {
                pre_clip_positions: vec![Vec2::new(200.0, 20.0)],
                cap_start: None,
                cap_end: None,
                body_glyph: "SENTINEL".into(),
                font: None,
                font_size_pt: 12.0,
                color: stored_color.clone(),
            },
        );

        let second = build_scene_with_cache(
            &map,
            &HashMap::new(),
            Some(("a", "b", "cross_link")),
            &mut cache,
            1.0,
        );
        let conn = &second.connection_elements[0];
        assert_eq!(conn.body_glyph, "SENTINEL",
            "selection change should not have dropped the cache");
        assert_eq!(conn.color, SELECTED_EDGE_COLOR,
            "selected element should pick up the highlight color");
        // And the cache's stored color should be unchanged (still the
        // pre-selection value).
        assert_eq!(cache.get(&key).unwrap().color, stored_color);
    }

    #[test]
    fn test_scene_build_still_works_on_real_map() {
        // Smoke test: loading the testament map and building a scene
        // should not crash, and connections should still render (the
        // clipping filter should not wipe out every glyph).
        let map = loader::load_from_file(&test_map_path()).unwrap();
        let scene = build_scene(&map, 1.0);
        assert!(!scene.text_elements.is_empty());
        assert!(!scene.connection_elements.is_empty());
        // At least one connection should have a non-empty glyph list.
        let any_with_glyphs = scene.connection_elements.iter()
            .any(|c| !c.glyph_positions.is_empty());
        assert!(any_with_glyphs,
            "at least one connection should have un-clipped glyphs");
    }

    // ---------------------------------------------------------------------
    // Session 6C: edge handle emission
    // ---------------------------------------------------------------------

    #[test]
    fn test_no_edge_handles_when_nothing_selected() {
        let map = loader::load_from_file(&test_map_path()).unwrap();
        let scene = build_scene(&map, 1.0);
        assert!(scene.edge_handles.is_empty(),
            "no selection → no handles emitted");
    }

    #[test]
    fn test_edge_handles_straight_edge_emits_midpoint() {
        let map = loader::load_from_file(&test_map_path()).unwrap();
        // Find a straight edge
        let edge = map.edges.iter()
            .find(|e| e.visible && e.control_points.is_empty())
            .expect("testament map should have a straight edge");
        let mut cache = SceneConnectionCache::new();
        let scene = build_scene_with_cache(
            &map,
            &HashMap::new(),
            Some((&edge.from_id, &edge.to_id, &edge.edge_type)),
            &mut cache,
            1.0,
        );
        assert_eq!(
            scene.edge_handles.len(),
            3,
            "straight edge: AnchorFrom + AnchorTo + Midpoint = 3 handles"
        );
        let kinds: Vec<&EdgeHandleKind> = scene.edge_handles
            .iter()
            .map(|h| &h.kind)
            .collect();
        assert!(kinds.iter().any(|k| matches!(k, EdgeHandleKind::AnchorFrom)));
        assert!(kinds.iter().any(|k| matches!(k, EdgeHandleKind::AnchorTo)));
        assert!(kinds.iter().any(|k| matches!(k, EdgeHandleKind::Midpoint)));
    }

    #[test]
    fn test_edge_handles_curved_edge_emits_control_points_not_midpoint() {
        let mut map = loader::load_from_file(&test_map_path()).unwrap();
        // Find a visible edge and curve it (quadratic)
        let edge_idx = map.edges.iter()
            .position(|e| e.visible)
            .unwrap();
        map.edges[edge_idx].control_points.push(
            crate::mindmap::model::ControlPoint { x: 20.0, y: 30.0 },
        );
        let edge = map.edges[edge_idx].clone();
        let mut cache = SceneConnectionCache::new();
        let scene = build_scene_with_cache(
            &map,
            &HashMap::new(),
            Some((&edge.from_id, &edge.to_id, &edge.edge_type)),
            &mut cache,
            1.0,
        );
        // 2 anchors + 1 control point = 3 handles, no midpoint
        assert_eq!(scene.edge_handles.len(), 3);
        assert!(scene.edge_handles.iter().any(|h| matches!(h.kind, EdgeHandleKind::ControlPoint(0))));
        assert!(scene.edge_handles.iter().all(|h| !matches!(h.kind, EdgeHandleKind::Midpoint)));
    }

    #[test]
    fn test_edge_handles_cubic_edge_emits_both_control_points() {
        let mut map = loader::load_from_file(&test_map_path()).unwrap();
        let edge_idx = map.edges.iter()
            .position(|e| e.visible)
            .unwrap();
        map.edges[edge_idx].control_points.push(
            crate::mindmap::model::ControlPoint { x: 10.0, y: 10.0 },
        );
        map.edges[edge_idx].control_points.push(
            crate::mindmap::model::ControlPoint { x: 40.0, y: 40.0 },
        );
        let edge = map.edges[edge_idx].clone();
        let mut cache = SceneConnectionCache::new();
        let scene = build_scene_with_cache(
            &map,
            &HashMap::new(),
            Some((&edge.from_id, &edge.to_id, &edge.edge_type)),
            &mut cache,
            1.0,
        );
        // 2 anchors + 2 control points = 4 handles
        assert_eq!(scene.edge_handles.len(), 4);
        assert!(scene.edge_handles.iter().any(|h| matches!(h.kind, EdgeHandleKind::ControlPoint(0))));
        assert!(scene.edge_handles.iter().any(|h| matches!(h.kind, EdgeHandleKind::ControlPoint(1))));
    }

    #[test]
    fn test_edge_handle_control_point_position_is_absolute_canvas() {
        let mut map = loader::load_from_file(&test_map_path()).unwrap();
        let edge_idx = map.edges.iter()
            .position(|e| e.visible)
            .unwrap();
        let cp_x = 55.0;
        let cp_y = 77.0;
        map.edges[edge_idx].control_points.push(
            crate::mindmap::model::ControlPoint { x: cp_x, y: cp_y },
        );
        let edge = map.edges[edge_idx].clone();
        let from_node = map.nodes.get(&edge.from_id).unwrap();
        let from_center_x = from_node.position.x as f32 + from_node.size.width as f32 * 0.5;
        let from_center_y = from_node.position.y as f32 + from_node.size.height as f32 * 0.5;

        let mut cache = SceneConnectionCache::new();
        let scene = build_scene_with_cache(
            &map,
            &HashMap::new(),
            Some((&edge.from_id, &edge.to_id, &edge.edge_type)),
            &mut cache,
            1.0,
        );
        let cp_handle = scene.edge_handles.iter()
            .find(|h| matches!(h.kind, EdgeHandleKind::ControlPoint(0)))
            .unwrap();
        assert!((cp_handle.position.0 - (from_center_x + cp_x as f32)).abs() < 0.01);
        assert!((cp_handle.position.1 - (from_center_y + cp_y as f32)).abs() < 0.01);
    }
}
