//! Markdown export for mindmap files.
//!
//! Walks the node tree starting at the roots and emits a plain
//! Markdown document where each node's first line of `text` becomes a
//! heading (`#`, `##`, ...) and every other field (notes, runs, style,
//! edges) is ignored. Empty-text nodes are treated as transparent:
//! their children surface at the same heading depth, so if the roots
//! have no text the first-text generation becomes the `#` level.

use baumhard::mindmap::model::{MindMap, MindNode};

/// Convert `map` into a Markdown document containing only node text,
/// indented by tree depth via `#` heading characters. The first line
/// of each node's `text` becomes the heading; any further lines are
/// emitted as plain paragraph text underneath. Nodes with empty
/// `text` (after trimming whitespace) pass through — their children
/// are emitted at the same depth.
///
/// Note: Markdown only defines heading levels `#`..`######`. For
/// trees deeper than six, we keep emitting extra `#` characters.
/// Most renderers treat 7+ as plain text, which is fine here since
/// the goal is a lossless text-and-shape dump, not a styled document.
pub fn mindmap_to_markdown(map: &MindMap) -> String {
    let mut out = String::new();
    emit_level(map, &map.root_nodes(), 1, &mut out);
    out
}

fn emit_level(map: &MindMap, nodes: &[&MindNode], depth: usize, out: &mut String) {
    for node in nodes {
        let children = map.children_of(&node.id);
        if node.text.trim().is_empty() {
            emit_level(map, &children, depth, out);
            continue;
        }
        let mut lines = node.text.lines();
        let first = lines.next().unwrap_or("");
        for _ in 0..depth {
            out.push('#');
        }
        out.push(' ');
        out.push_str(first);
        out.push('\n');
        for rest in lines {
            out.push_str(rest);
            out.push('\n');
        }
        out.push('\n');
        emit_level(map, &children, depth + 1, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::mindmap::loader::load_from_file;
    use baumhard::mindmap::model::{
        Canvas, MindMap, MindNode, NodeLayout, NodeStyle, Position, Size, TextRun,
    };
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn testament_path() -> PathBuf {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.pop(); // crates/maptool -> crates
        path.pop(); // crates -> root
        path.push("maps/testament.mindmap.json");
        path
    }

    /// Build a minimal `MindNode` with the given id/parent/index/text.
    /// All the style/layout/position fields are filled with throwaway
    /// defaults — the export code never reads them.
    fn make_node(id: &str, parent_id: Option<&str>, index: i32, text: &str) -> MindNode {
        MindNode {
            id: id.to_string(),
            parent_id: parent_id.map(|s| s.to_string()),
            index,
            position: Position { x: 0.0, y: 0.0 },
            size: Size { width: 0.0, height: 0.0 },
            text: text.to_string(),
            text_runs: Vec::new(),
            style: NodeStyle {
                background_color: "#000000".to_string(),
                frame_color: "#ffffff".to_string(),
                text_color: "#ffffff".to_string(),
                shape_type: 0,
                corner_radius_percent: 0.0,
                frame_thickness: 0.0,
                show_frame: false,
                show_shadow: false,
                border: None,
            },
            layout: NodeLayout {
                layout_type: 0,
                direction: 0,
                spacing: 0.0,
            },
            folded: false,
            notes: String::new(),
            color_schema: None,
            trigger_bindings: Vec::new(),
            inline_mutations: Vec::new(),
        }
    }

    /// Build an empty `MindMap` with the given nodes inserted.
    fn make_map(nodes: Vec<MindNode>) -> MindMap {
        let mut map_nodes = HashMap::new();
        for node in nodes {
            map_nodes.insert(node.id.clone(), node);
        }
        MindMap {
            version: "1.0".to_string(),
            name: "test".to_string(),
            canvas: Canvas {
                background_color: "#000000".to_string(),
                default_border: None,
                default_connection: None,
                theme_variables: HashMap::new(),
                theme_variants: HashMap::new(),
            },
            nodes: map_nodes,
            edges: Vec::new(),
            custom_mutations: Vec::new(),
            portals: Vec::new(),
        }
    }

    #[test]
    fn test_export_root_gets_single_hash() {
        let map = load_from_file(&testament_path()).expect("load testament");
        let out = mindmap_to_markdown(&map);
        assert!(out.starts_with("# "), "expected single-hash heading, got: {:?}", &out[..40.min(out.len())]);
        // Second char-run must not be `#` (so it's `# ` not `## `).
        let roots = map.root_nodes();
        let first_root_first_line = roots[0].text.lines().next().unwrap_or("");
        let expected_first_line = format!("# {first_root_first_line}\n");
        assert!(out.starts_with(&expected_first_line), "unexpected first heading");
    }

    #[test]
    fn test_export_depth_increments_with_generation() {
        let map = make_map(vec![
            make_node("r", None, 0, "Root"),
            make_node("c", Some("r"), 0, "Child"),
            make_node("g", Some("c"), 0, "Grand"),
        ]);
        let out = mindmap_to_markdown(&map);
        let root_pos = out.find("# Root\n").expect("root heading");
        let child_pos = out.find("## Child\n").expect("child heading");
        let grand_pos = out.find("### Grand\n").expect("grand heading");
        assert!(root_pos < child_pos && child_pos < grand_pos);
    }

    #[test]
    fn test_export_passthrough_empty_text() {
        // Empty-text root with two text children — both should appear as `#`.
        let map = make_map(vec![
            make_node("r", None, 0, ""),
            make_node("a", Some("r"), 0, "Alpha"),
            make_node("b", Some("r"), 1, "Beta"),
        ]);
        let out = mindmap_to_markdown(&map);
        assert!(out.contains("# Alpha\n"), "Alpha should be top-level: {out}");
        assert!(out.contains("# Beta\n"), "Beta should be top-level: {out}");
        assert!(!out.contains("## Alpha"), "Alpha should not be nested: {out}");
        assert!(!out.contains("## Beta"), "Beta should not be nested: {out}");
    }

    #[test]
    fn test_export_ignores_notes_and_runs() {
        let mut node = make_node("r", None, 0, "Visible");
        node.notes = "HIDDEN_NOTES_STRING".to_string();
        node.text_runs = vec![TextRun {
            start: 0,
            end: 7,
            bold: true,
            italic: true,
            underline: true,
            font: "HIDDEN_FONT_NAME".to_string(),
            size_pt: 42,
            color: "#ff0000".to_string(),
            hyperlink: Some("HIDDEN_URL".to_string()),
        }];
        let map = make_map(vec![node]);
        let out = mindmap_to_markdown(&map);
        assert!(out.contains("# Visible\n"));
        assert!(!out.contains("HIDDEN_NOTES_STRING"));
        assert!(!out.contains("HIDDEN_FONT_NAME"));
        assert!(!out.contains("HIDDEN_URL"));
    }

    #[test]
    fn test_export_multiline_text_first_line_is_heading() {
        let map = make_map(vec![make_node("r", None, 0, "Title\nbody line\nmore body")]);
        let out = mindmap_to_markdown(&map);
        assert!(out.starts_with("# Title\nbody line\nmore body\n"), "got: {out:?}");
    }

    #[test]
    fn test_export_sibling_order_matches_index() {
        // Indexes deliberately out of insertion order.
        let map = make_map(vec![
            make_node("r", None, 0, "Root"),
            make_node("late", Some("r"), 100, "Late"),
            make_node("mid", Some("r"), 50, "Mid"),
            make_node("early", Some("r"), 10, "Early"),
        ]);
        let out = mindmap_to_markdown(&map);
        let early = out.find("## Early\n").expect("early");
        let mid = out.find("## Mid\n").expect("mid");
        let late = out.find("## Late\n").expect("late");
        assert!(early < mid && mid < late, "out: {out}");
    }
}
