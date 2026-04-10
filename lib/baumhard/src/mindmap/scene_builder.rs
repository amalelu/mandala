use std::collections::HashMap;
use crate::mindmap::border::BorderStyle;
use crate::mindmap::connection;
use crate::mindmap::model::{GlyphConnectionConfig, MindMap, TextRun};
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

    for node in map.nodes.values() {
        if map.is_hidden_by_fold(node) {
            continue;
        }

        let (ox, oy) = offsets.get(&node.id).copied().unwrap_or((0.0, 0.0));
        let pos_x = node.position.x as f32 + ox;
        let pos_y = node.position.y as f32 + oy;
        let size_x = node.size.width as f32;
        let size_y = node.size.height as f32;

        node_aabbs.push((Vec2::new(pos_x, pos_y), Vec2::new(size_x, size_y)));

        // Text element (skip empty text nodes)
        if !node.text.is_empty() {
            text_elements.push(TextElement {
                node_id: node.id.clone(),
                text: node.text.clone(),
                text_runs: node.text_runs.clone(),
                position: (pos_x, pos_y),
                size: (size_x, size_y),
            });
        }

        // Border element
        if node.style.show_frame {
            let border_style = BorderStyle::default_with_color(&node.style.frame_color);
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
            config.color.clone().unwrap_or_else(|| edge.color.clone())
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

        // Caps are anchored to the ORIGINAL first/last sample — they sit on
        // the source/target node's border and stay there regardless of any
        // interior-clipping the body glyphs go through below.
        let first_pos = (samples[0].position.x, samples[0].position.y);
        let last_pos = (
            samples.last().unwrap().position.x,
            samples.last().unwrap().position.y,
        );
        let cap_start = config.cap_start.as_ref().map(|glyph| (glyph.clone(), first_pos));
        let cap_end = config.cap_end.as_ref().map(|glyph| (glyph.clone(), last_pos));

        // Drop body-glyph positions that fall strictly inside any visible
        // node's AABB so a connection doesn't render over the interior of
        // a node it's passing through.
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
        background_color: map.canvas.background_color.clone(),
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

    #[test]
    fn test_scene_clips_connection_glyphs_inside_node() {
        // Build a minimal map with three nodes: A on the left, B on the
        // right, and a blocker node C directly on the path between them.
        // The A→B connection should skip body glyphs that fall inside C.
        use crate::mindmap::model::{
            Canvas, MindEdge, MindMap, MindNode, NodeLayout, NodeStyle, Position, Size,
        };
        use std::collections::HashMap;

        fn node_at(id: &str, x: f64, y: f64, w: f64, h: f64) -> MindNode {
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
                    show_frame: false,
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

        let mut nodes = HashMap::new();
        nodes.insert("a".into(), node_at("a", 0.0, 0.0, 40.0, 40.0));
        nodes.insert("b".into(), node_at("b", 400.0, 0.0, 40.0, 40.0));
        // Blocker C is directly on the path between A and B
        nodes.insert("c".into(), node_at("c", 180.0, 0.0, 60.0, 40.0));

        let map = MindMap {
            version: "1.0".into(),
            name: "test".into(),
            canvas: Canvas {
                background_color: "#000".into(),
                default_border: None,
                default_connection: None,
            },
            nodes,
            edges: vec![MindEdge {
                from_id: "a".into(),
                to_id: "b".into(),
                edge_type: "cross_link".into(),
                color: "#fff".into(),
                width: 1,
                line_style: 0,
                visible: true,
                label: None,
                anchor_from: 2, // right edge of A
                anchor_to: 4,   // left edge of B
                control_points: vec![],
                glyph_connection: None,
            }],
            custom_mutations: vec![],
        };

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
        // And at least some body glyphs should remain outside C, otherwise
        // the whole connection would be invisible.
        assert!(!conn.glyph_positions.is_empty(),
            "some glyphs should remain outside the blocker");
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
}
