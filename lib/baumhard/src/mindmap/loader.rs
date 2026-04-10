use std::fs;
use std::path::Path;
use crate::mindmap::model::MindMap;

pub fn load_from_file(path: &Path) -> Result<MindMap, String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read file {}: {}", path.display(), e))?;
    load_from_str(&content)
}

pub fn load_from_str(json: &str) -> Result<MindMap, String> {
    serde_json::from_str(json)
        .map_err(|e| format!("Failed to parse mindmap JSON: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_map_path() -> PathBuf {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.pop(); // lib/baumhard -> lib
        path.pop(); // lib -> root
        path.push("maps/testament.mindmap.json");
        path
    }

    #[test]
    fn test_load_testament_map() {
        let path = test_map_path();
        let map = load_from_file(&path).expect("Failed to load testament map");

        assert_eq!(map.version, "1.0");
        assert_eq!(map.name, "testament");
        assert_eq!(map.canvas.background_color, "#141414");
        assert_eq!(map.nodes.len(), 243);
        assert_eq!(map.edges.len(), 250);
    }

    #[test]
    fn test_root_nodes() {
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        let roots = map.root_nodes();
        assert!(!roots.is_empty());
        for root in &roots {
            assert!(root.parent_id.is_none());
        }
        // Verify sorted by index
        for w in roots.windows(2) {
            assert!(w[0].index <= w[1].index);
        }
    }

    #[test]
    fn test_children_of() {
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        // Lord God node
        let children = map.children_of("348068464");
        assert!(!children.is_empty());
        for child in &children {
            assert_eq!(child.parent_id.as_deref(), Some("348068464"));
        }
        // Verify sorted by index
        for w in children.windows(2) {
            assert!(w[0].index <= w[1].index);
        }
    }

    #[test]
    fn test_text_runs() {
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        let node = map.nodes.get("348068464").unwrap();
        assert_eq!(node.text, "Lord God");
        assert_eq!(node.text_runs.len(), 1);
        let run = &node.text_runs[0];
        assert_eq!(run.start, 0);
        assert_eq!(run.end, 8);
        assert!(run.bold);
        assert!(run.underline);
        assert_eq!(run.font, "LiberationSans");
        assert_eq!(run.size_pt, 74);
        assert_eq!(run.color, "#ffffff");
    }

    #[test]
    fn test_color_schema() {
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        let root_node = map.nodes.get("348068464").unwrap();
        let schema = root_node.color_schema.as_ref().unwrap();
        assert_eq!(schema.level, 0);
        assert_eq!(schema.palette, "coral");
        assert!(!schema.groups.is_empty());
        assert_eq!(schema.groups[0].frame, "#30b082");
    }

    #[test]
    fn test_edges() {
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        let edge = &map.edges[0];
        assert_eq!(edge.from_id, "348068464");
        assert_eq!(edge.to_id, "351582192");
        assert_eq!(edge.edge_type, "parent_child");
        assert!(edge.visible);

        // Find an edge with control points
        let curved = map.edges.iter().find(|e| !e.control_points.is_empty());
        assert!(curved.is_some());
    }

    #[test]
    fn test_resolve_theme_colors() {
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        // Root schema node should resolve to level 0 group
        let root_node = map.nodes.get("348068464").unwrap();
        let colors = map.resolve_theme_colors(root_node).unwrap();
        assert_eq!(colors.frame, "#30b082");
    }

    #[test]
    fn test_testament_edges_produce_paths() {
        use crate::mindmap::connection;
        use glam::Vec2;

        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        let mut straight_count = 0;
        let mut bezier_count = 0;
        for edge in &map.edges {
            let from_node = map.nodes.get(&edge.from_id).expect("Missing from_node");
            let to_node = map.nodes.get(&edge.to_id).expect("Missing to_node");

            let from_pos = Vec2::new(from_node.position.x as f32, from_node.position.y as f32);
            let from_size = Vec2::new(from_node.size.width as f32, from_node.size.height as f32);
            let to_pos = Vec2::new(to_node.position.x as f32, to_node.position.y as f32);
            let to_size = Vec2::new(to_node.size.width as f32, to_node.size.height as f32);

            let conn_path = connection::build_connection_path(
                from_pos, from_size, edge.anchor_from,
                to_pos, to_size, edge.anchor_to,
                &edge.control_points,
            );
            match conn_path {
                connection::ConnectionPath::Straight { .. } => straight_count += 1,
                connection::ConnectionPath::CubicBezier { .. } => bezier_count += 1,
            }

            // Verify sampling produces non-empty result
            let samples = connection::sample_path(&conn_path, 7.2);
            assert!(!samples.is_empty(), "Edge {}→{} produced no samples", edge.from_id, edge.to_id);
        }
        assert_eq!(straight_count + bezier_count, 250);
        assert!(straight_count > 200, "Expected most edges to be straight");
        assert!(bezier_count > 0, "Expected some Bezier edges");
    }

    #[test]
    fn test_testament_scene_has_connections() {
        use crate::mindmap::scene_builder;

        let path = test_map_path();
        let map = load_from_file(&path).unwrap();
        let scene = scene_builder::build_scene(&map);

        // All visible edges should produce connection elements
        let visible_edges = map.edges.iter().filter(|e| e.visible).count();
        assert_eq!(scene.connection_elements.len(), visible_edges,
            "Expected {} connection elements, got {}", visible_edges, scene.connection_elements.len());

        // Each connection element should have glyph positions
        for elem in &scene.connection_elements {
            assert!(!elem.glyph_positions.is_empty(), "Connection has no glyph positions");
            assert!(!elem.body_glyph.is_empty(), "Connection has no body glyph");
            assert!(!elem.color.is_empty(), "Connection has no color");
        }
    }

    #[test]
    fn test_backward_compat_no_custom_mutations() {
        // Existing maps without custom_mutations/trigger_bindings/inline_mutations
        // should load with empty defaults
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        assert!(map.custom_mutations.is_empty(), "Existing map should have no custom_mutations");

        let node = map.nodes.get("348068464").unwrap();
        assert!(node.trigger_bindings.is_empty(), "Existing node should have no trigger_bindings");
        assert!(node.inline_mutations.is_empty(), "Existing node should have no inline_mutations");
    }

    #[test]
    fn test_is_hidden_by_fold() {
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        // Root node has no parent, so it should never be hidden
        let root = map.nodes.get("348068464").unwrap();
        assert!(!map.is_hidden_by_fold(root));

        // A child of a non-folded parent should not be hidden
        let children = map.children_of("348068464");
        assert!(!children.is_empty());
        // The root is not folded by default, so its children are visible
        assert!(!map.is_hidden_by_fold(children[0]));
    }
}
