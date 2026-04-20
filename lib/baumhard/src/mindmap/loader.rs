//! `.mindmap.json` loader + saver. Accepts both the current edge-based
//! portal shape and pre-refactor files that still ship a top-level
//! `portals[]` array, rejecting the latter with a concrete migration
//! pointer instead of silently dropping data.

use std::fs;
use std::path::Path;
use crate::mindmap::model::MindMap;

/// Load a `MindMap` from a file path. Reads the entire file into
/// memory via `std::fs::read_to_string`, then delegates to
/// [`load_from_str`]. Native-only (synchronous I/O). Returns a
/// `String` error describing the path + underlying cause.
pub fn load_from_file(path: &Path) -> Result<MindMap, String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read file {}: {}", path.display(), e))?;
    load_from_str(&content)
}

/// Parse a `MindMap` from a JSON string. Rejects pre-refactor files
/// that still carry a top-level `portals[]` array with a concrete
/// migration pointer (`maptool convert --portals`) so a stale map
/// doesn't silently lose its portals — serde would otherwise ignore
/// the unknown field. Allocation-bounded by the input size.
pub fn load_from_str(json: &str) -> Result<MindMap, String> {
    // Pre-refactor maps stored portals in a separate `portals[]` array.
    // Post-refactor portals are edges with `display_mode = "portal"`,
    // and the `portals` field no longer exists on `MindMap`. Reject
    // legacy files with a clear pointer to `maptool convert --portals`
    // so a stale file doesn't silently drop its portals — serde would
    // otherwise ignore the unknown field.
    let raw: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| format!("Failed to parse mindmap JSON: {}", e))?;
    if let Some(arr) = raw.get("portals").and_then(|p| p.as_array()) {
        if !arr.is_empty() {
            return Err(
                "legacy `portals` field present; run `maptool convert --portals <file>` \
                 to migrate to portal-mode edges"
                    .to_string(),
            );
        }
    }
    serde_json::from_value(raw)
        .map_err(|e| format!("Failed to parse mindmap JSON: {}", e))
}

/// Serialize a `MindMap` to pretty-printed JSON and write it to disk.
/// Mirrors `load_from_file` — the same `Result<_, String>` error
/// convention, native-only synchronous I/O via `std::fs`. Pretty
/// printing keeps the on-disk format diff-friendly so authors can
/// inspect saved maps with normal text tools. Streams through a
/// `BufWriter` so large maps don't have to materialize the entire
/// JSON in memory before hitting disk.
pub fn save_to_file(path: &Path, map: &MindMap) -> Result<(), String> {
    let file = fs::File::create(path)
        .map_err(|e| format!("Failed to create {}: {}", path.display(), e))?;
    let writer = std::io::BufWriter::new(file);
    serde_json::to_writer_pretty(writer, map)
        .map_err(|e| format!("Failed to write {}: {}", path.display(), e))
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
        assert_eq!(map.canvas.background_color, "#000000");
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
            assert!(crate::mindmap::model::id_sort_key(&w[0].id) <= crate::mindmap::model::id_sort_key(&w[1].id));
        }
    }

    #[test]
    fn test_children_of() {
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        // Lord God node
        let children = map.children_of("0");
        assert!(!children.is_empty());
        for child in &children {
            assert_eq!(child.parent_id.as_deref(), Some("0"));
        }
        // Verify sorted by index
        for w in children.windows(2) {
            assert!(crate::mindmap::model::id_sort_key(&w[0].id) <= crate::mindmap::model::id_sort_key(&w[1].id));
        }
    }

    #[test]
    fn test_text_runs() {
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        let node = map.nodes.get("0").unwrap();
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

        let root_node = map.nodes.get("0").unwrap();
        let schema = root_node.color_schema.as_ref().unwrap();
        assert_eq!(schema.level, 0);
        assert!(schema.palette.starts_with("coral"));
        let palette = map.palettes.get(&schema.palette).unwrap();
        assert!(!palette.groups.is_empty());
        assert_eq!(palette.groups[0].frame, "#30b082");
    }

    #[test]
    fn test_edges() {
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        let edge = &map.edges[0];
        assert_eq!(edge.from_id, "0");
        assert_eq!(edge.to_id, "0.0");
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
        let root_node = map.nodes.get("0").unwrap();
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
                from_pos, from_size, &edge.anchor_from,
                to_pos, to_size, &edge.anchor_to,
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
        let scene = scene_builder::build_scene(&map, 1.0);

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

        let node = map.nodes.get("0").unwrap();
        assert!(node.trigger_bindings.is_empty(), "Existing node should have no trigger_bindings");
        assert!(node.inline_mutations.is_empty(), "Existing node should have no inline_mutations");
    }

    #[test]
    fn test_backward_compat_no_theme_variables() {
        // Existing maps without theme_variables/theme_variants should load
        // with empty defaults (the new fields must be opt-in via serde default).
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();
        assert!(map.canvas.theme_variables.is_empty());
        assert!(map.canvas.theme_variants.is_empty());
    }

    fn theme_demo_path() -> PathBuf {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.pop();
        path.pop();
        path.push("maps/theme_demo.mindmap.json");
        path
    }

    #[test]
    fn test_load_theme_demo_map() {
        let path = theme_demo_path();
        let map = load_from_file(&path).expect("Failed to load theme demo map");
        assert_eq!(map.version, "1.0");
        assert_eq!(map.name, "theme_demo");
        assert_eq!(map.canvas.background_color, "var(--bg)");
        assert!(map.canvas.theme_variables.contains_key("--bg"));
        assert_eq!(map.canvas.theme_variants.len(), 3);
        assert!(map.canvas.theme_variants.contains_key("dark"));
        assert!(map.canvas.theme_variants.contains_key("light"));
        assert!(map.canvas.theme_variants.contains_key("forest"));
        assert_eq!(map.custom_mutations.len(), 3);
    }

    #[test]
    fn test_theme_demo_scene_resolves_background() {
        use crate::mindmap::scene_builder;
        let path = theme_demo_path();
        let map = load_from_file(&path).unwrap();
        let scene = scene_builder::build_scene(&map, 1.0);
        // Background should resolve through the dark theme var set.
        assert_eq!(scene.background_color, "#141414");
    }

    #[test]
    fn test_theme_demo_roundtrip() {
        let path = theme_demo_path();
        let map = load_from_file(&path).unwrap();
        let json = serde_json::to_string(&map).unwrap();
        let back: MindMap = serde_json::from_str(&json).unwrap();
        assert_eq!(back.canvas.theme_variants.len(), 3);
        assert_eq!(back.custom_mutations.len(), 3);
    }

    #[test]
    fn test_save_to_file_round_trip() {
        // A `save_to_file` followed by `load_from_file` must reproduce
        // the same MindMap, locking the on-disk format as the canonical
        // serialization.
        let path = test_map_path();
        let original = load_from_file(&path).unwrap();

        let tmp = std::env::temp_dir().join("mandala_save_round_trip.mindmap.json");
        save_to_file(&tmp, &original).expect("save failed");
        let reloaded = load_from_file(&tmp).expect("reload failed");

        assert_eq!(reloaded.version, original.version);
        assert_eq!(reloaded.name, original.name);
        assert_eq!(reloaded.nodes.len(), original.nodes.len());
        assert_eq!(reloaded.edges.len(), original.edges.len());
        assert_eq!(reloaded.canvas.background_color, original.canvas.background_color);

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_save_blank_map_round_trip() {
        // A freshly-created blank map must serialize to JSON that
        // re-parses cleanly — the `new` console command relies on
        // this.
        let blank = MindMap::new_blank("untitled");
        let tmp = std::env::temp_dir().join("mandala_blank_round_trip.mindmap.json");
        save_to_file(&tmp, &blank).expect("save failed");
        let reloaded = load_from_file(&tmp).expect("reload failed");

        assert_eq!(reloaded.name, "untitled");
        assert_eq!(reloaded.version, "1.0");
        assert!(reloaded.nodes.is_empty());
        assert!(reloaded.edges.is_empty());
        assert_eq!(reloaded.canvas.background_color, "#000000");

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_is_hidden_by_fold() {
        let path = test_map_path();
        let map = load_from_file(&path).unwrap();

        // Root node has no parent, so it should never be hidden
        let root = map.nodes.get("0").unwrap();
        assert!(!map.is_hidden_by_fold(root));

        // A child of a non-folded parent should not be hidden
        let children = map.children_of("0");
        assert!(!children.is_empty());
        // The root is not folded by default, so its children are visible
        assert!(!map.is_hidden_by_fold(children[0]));
    }
}
