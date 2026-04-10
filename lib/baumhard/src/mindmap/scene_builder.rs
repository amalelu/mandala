use std::collections::HashMap;
use crate::mindmap::border::BorderStyle;
use crate::mindmap::connection;
use crate::mindmap::model::{GlyphConnectionConfig, MindMap, TextRun};
use crate::util::color::resolve_var;
use glam::Vec2;

/// Intermediate representation between MindMap data and GPU rendering.
/// Produced by `build_scene()`, consumed by Renderer to create cosmic-text buffers.
pub struct RenderScene {
    pub text_elements: Vec<TextElement>,
    pub border_elements: Vec<BorderElement>,
    pub connection_elements: Vec<ConnectionElement>,
    pub portal_elements: Vec<PortalElement>,
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
    /// Stable identity of the underlying edge: `(from_id, to_id, edge_type)`.
    /// Edges have no intrinsic ID in the file format, so this tuple plays
    /// the role. Used by the renderer to key the incremental rebuild
    /// path (Phase 4(B)) — a drag frame only re-builds entries whose
    /// endpoint is in the `offsets` map, and the key is how each
    /// `ConnectionElement` matches back to its stored buffers.
    pub from_id: String,
    pub to_id: String,
    pub edge_type: String,
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

impl ConnectionElement {
    /// Stable composite key used by the renderer to identify which stored
    /// buffers correspond to this element. Matches the `(from, to, type)`
    /// triple used throughout the codebase to reference edges.
    pub fn key(&self) -> (String, String, String) {
        (self.from_id.clone(), self.to_id.clone(), self.edge_type.clone())
    }
}

/// Placeholder for future portal rendering (M3).
pub struct PortalElement {}

/// Color override applied to the `ConnectionElement` of a selected edge.
/// Kept in sync visually with the cyan node selection highlight in
/// `src/application/document.rs::HIGHLIGHT_COLOR`.
const SELECTED_EDGE_COLOR: &str = "#00E5FF";

/// Builds a RenderScene from a MindMap, determining which nodes and borders
/// are visible (accounting for fold state) and extracting their layout data.
pub fn build_scene(map: &MindMap) -> RenderScene {
    build_scene_with_offsets_and_selection(map, &HashMap::new(), None)
}

/// Builds a RenderScene with position offsets applied to specific nodes.
/// Used during drag to update connections and borders in real-time without
/// modifying the MindMap model. Each entry in `offsets` maps a node ID to
/// a (dx, dy) delta that is added to the node's model position.
pub fn build_scene_with_offsets(map: &MindMap, offsets: &HashMap<String, (f32, f32)>) -> RenderScene {
    build_scene_with_offsets_and_selection(map, offsets, None)
}

/// Builds a RenderScene with position offsets and an optional selected-edge
/// highlight. If `selected_edge` matches an edge (by `from_id`, `to_id`,
/// `edge_type`), that edge's `ConnectionElement.color` is overwritten with
/// `SELECTED_EDGE_COLOR` so the renderer paints it in the selection color.
pub fn build_scene_with_offsets_and_selection(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    selected_edge: Option<(&str, &str, &str)>,
) -> RenderScene {
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

        // Resolve glyph config: edge override > canvas default > hardcoded default
        let config = edge.glyph_connection.as_ref()
            .or(map.canvas.default_connection.as_ref())
            .unwrap_or(&default_config);

        let is_selected = selected_edge.map_or(false, |(f, t, ty)| {
            f == edge.from_id && t == edge.to_id && ty == edge.edge_type
        });
        let color = if is_selected {
            SELECTED_EDGE_COLOR.to_string()
        } else {
            // Resolve through theme variables: connection override >
            // edge color > (hardcoded default would be here). Resolution
            // happens after inheritance so `var(--edge)` on either layer
            // is honored.
            let raw = config.color.as_deref().unwrap_or(edge.color.as_str());
            resolve_var(raw, vars).to_string()
        };
        let font_size = config.font_size_pt;
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
        let first_visible = !point_inside_any_node(first_pos, &node_aabbs);
        let last_visible = !point_inside_any_node(last_pos, &node_aabbs);
        let cap_start = if first_visible {
            config.cap_start.as_ref()
                .map(|g| (g.clone(), (first_pos.x, first_pos.y)))
        } else {
            None
        };
        let cap_end = if last_visible {
            config.cap_end.as_ref()
                .map(|g| (g.clone(), (last_pos.x, last_pos.y)))
        } else {
            None
        };

        // Drop body-glyph positions that fall strictly inside any visible
        // node's (frame-expanded) AABB so a connection doesn't render
        // over the interior of a node or over its visible border area.
        let glyph_positions: Vec<(f32, f32)> = samples.iter()
            .map(|s| s.position)
            .filter(|p| !point_inside_any_node(*p, &node_aabbs))
            .map(|p| (p.x, p.y))
            .collect();

        // If every sample was clipped (e.g. an entirely-internal edge),
        // there's nothing to draw for the body — skip the element unless a
        // cap survives to represent the connection.
        if glyph_positions.is_empty() && cap_start.is_none() && cap_end.is_none() {
            continue;
        }

        connection_elements.push(ConnectionElement {
            from_id: edge.from_id.clone(),
            to_id: edge.to_id.clone(),
            edge_type: edge.edge_type.clone(),
            glyph_positions,
            body_glyph: config.body.clone(),
            cap_start,
            cap_end,
            font: config.font.clone(),
            font_size_pt: font_size,
            color,
        });
    }

    RenderScene {
        text_elements,
        border_elements,
        connection_elements,
        portal_elements: Vec::new(),
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

        let scene = build_scene(&map);
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

        let scene = build_scene(&map);
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

        let scene = build_scene(&map);
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
        let scene = build_scene(&map);
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

        let scene = build_scene(&map);
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

        let scene = build_scene(&map);
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
        });
        let map = synthetic_map(
            vec![
                synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
                synthetic_node("b", 400.0, 0.0, 40.0, 40.0, false),
            ],
            vec![edge],
        );
        let scene = build_scene(&map);
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
        });
        let map = synthetic_map(
            vec![
                synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false),
                synthetic_node("b", 400.0, 0.0, 40.0, 40.0, true), // framed!
            ],
            vec![edge],
        );
        let scene = build_scene(&map);
        let conn = &scene.connection_elements[0];
        // Source is unframed — cap_start still shows at A's right edge.
        assert!(conn.cap_start.is_some(),
            "cap_start should survive for unframed source");
        // Target is framed — cap_end falls inside the expanded clip AABB.
        assert!(conn.cap_end.is_none(),
            "cap_end should be clipped when target has a visible frame");
    }

    #[test]
    fn test_scene_build_still_works_on_real_map() {
        // Smoke test: loading the testament map and building a scene
        // should not crash, and connections should still render (the
        // clipping filter should not wipe out every glyph).
        let map = loader::load_from_file(&test_map_path()).unwrap();
        let scene = build_scene(&map);
        assert!(!scene.text_elements.is_empty());
        assert!(!scene.connection_elements.is_empty());
        // At least one connection should have a non-empty glyph list.
        let any_with_glyphs = scene.connection_elements.iter()
            .any(|c| !c.glyph_positions.is_empty());
        assert!(any_with_glyphs,
            "at least one connection should have un-clipped glyphs");
    }

    #[test]
    fn test_connection_element_exposes_edge_identifiers() {
        // Phase 4(B): ConnectionElement must carry the stable
        // `(from_id, to_id, edge_type)` triple so the renderer can key
        // its incremental rebuild path off it. Verify the scene builder
        // populates these fields and `.key()` returns the expected
        // tuple.
        let map = synthetic_map(
            vec![
                synthetic_node("source", 0.0, 0.0, 40.0, 40.0, false),
                synthetic_node("target", 400.0, 0.0, 40.0, 40.0, false),
            ],
            vec![synthetic_edge("source", "target", 2, 4)],
        );
        let scene = build_scene(&map);
        assert_eq!(scene.connection_elements.len(), 1);
        let conn = &scene.connection_elements[0];
        assert_eq!(conn.from_id, "source");
        assert_eq!(conn.to_id, "target");
        assert_eq!(conn.edge_type, "cross_link");
        assert_eq!(
            conn.key(),
            ("source".to_string(), "target".to_string(), "cross_link".to_string())
        );
    }

    #[test]
    fn test_connection_element_key_disambiguates_parallel_edges() {
        // Two edges between the same pair but with different types
        // should get distinct keys so the renderer tracks their buffers
        // separately.
        let a = synthetic_node("a", 0.0, 0.0, 40.0, 40.0, false);
        let b = synthetic_node("b", 400.0, 0.0, 40.0, 40.0, false);
        let mut parent_child = synthetic_edge("a", "b", 2, 4);
        parent_child.edge_type = "parent_child".into();
        let cross_link = synthetic_edge("a", "b", 2, 4);
        let map = synthetic_map(vec![a, b], vec![parent_child, cross_link]);
        let scene = build_scene(&map);
        assert_eq!(scene.connection_elements.len(), 2);
        let keys: std::collections::HashSet<_> = scene
            .connection_elements
            .iter()
            .map(|c| c.key())
            .collect();
        assert_eq!(keys.len(), 2, "parallel edges should have distinct keys");
    }

    #[test]
    fn test_border_element_exposes_node_id() {
        // BorderElement has always had node_id but Phase 4(B) now uses
        // it as the renderer's HashMap key, so lock the field in with
        // an explicit assertion.
        let map = synthetic_map(
            vec![synthetic_node("the_node", 0.0, 0.0, 40.0, 40.0, true)],
            vec![],
        );
        let scene = build_scene(&map);
        assert_eq!(scene.border_elements.len(), 1);
        assert_eq!(scene.border_elements[0].node_id, "the_node");
    }
}
