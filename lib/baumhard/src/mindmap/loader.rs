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
