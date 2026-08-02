// SPDX-License-Identifier: MPL-2.0

//! Markdown export: node text → headings by tree depth, everything
//! else ignored. Empty-text nodes are transparent.

use baumhard::mindmap::model::{ChildIndex, MindMap, MindNode};

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
    let index = ChildIndex::build(map);
    let mut out = String::new();
    emit_level(&index, index.roots(), 1, &mut out);
    out
}

fn emit_level(index: &ChildIndex, nodes: &[&MindNode], depth: usize, out: &mut String) {
    for node in nodes {
        let children = index.children_of(&node.id);
        let text = node.display_text();
        if text.trim().is_empty() {
            emit_level(index, children, depth, out);
            continue;
        }
        let mut lines = text.lines();
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
        emit_level(index, children, depth + 1, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::mindmap::loader::load_from_file;
    use baumhard::mindmap::model::{
        Canvas, MindMap, MindNode, MindSection, NodeLayout, NodeStyle, Position, Size, TextRun,
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

    /// Minimal `MindNode`; non-text fields are throwaway defaults.
    fn make_node(id: &str, parent_id: Option<&str>, text: &str) -> MindNode {
        MindNode {
            id: id.to_string(),
            parent_id: parent_id.map(|s| s.to_string()),
            position: Position { x: 0.0, y: 0.0 },
            size: Size {
                width: 0.0,
                height: 0.0,
            },
            sections: vec![MindSection::new_default(text.to_string(), Vec::new())],
            style: NodeStyle {
                background_color: "#000000".to_string(),
                frame_color: "#ffffff".to_string(),
                text_color: "#ffffff".to_string(),
                shape: "rectangle".to_string(),
                corner_radius_percent: 0.0,
                frame_thickness: 0.0,
                show_frame: false,
                show_shadow: false,
                border: None,
            },
            layout: NodeLayout {
                layout_type: "map".to_string(),
                direction: "auto".to_string(),
                spacing: 0.0,
            },
            folded: false,
            notes: String::new(),
            color_schema: None,
            channel: 0,
            trigger_bindings: Vec::new(),
            inline_mutations: Vec::new(),
            inline_macros: Vec::new(),
            min_zoom_to_render: None,
            max_zoom_to_render: None,
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
                default_section_frame_border: None,
                default_focused_section_frame_border: None,
                theme_variables: HashMap::new(),
                theme_variants: HashMap::new(),
            },
            palettes: HashMap::new(),
            nodes: map_nodes,
            edges: Vec::new(),
            custom_mutations: Vec::new(),
            macros: Vec::new(),
        }
    }

    #[test]
    fn test_export_root_gets_single_hash() {
        let map = load_from_file(&testament_path()).expect("load testament");
        let out = mindmap_to_markdown(&map);
        assert!(
            out.starts_with("# "),
            "expected single-hash heading, got: {:?}",
            &out[..40.min(out.len())]
        );
        // Second char-run must not be `#` (so it's `# ` not `## `).
        let roots = map.root_nodes();
        let first_root_text = roots[0].display_text();
        let first_root_first_line = first_root_text.lines().next().unwrap_or("");
        let expected_first_line = format!("# {first_root_first_line}\n");
        assert!(out.starts_with(&expected_first_line), "unexpected first heading");
    }

    #[test]
    fn test_export_depth_increments_with_generation() {
        let map = make_map(vec![
            make_node("r", None, "Root"),
            make_node("c", Some("r"), "Child"),
            make_node("g", Some("c"), "Grand"),
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
            make_node("r", None, ""),
            make_node("a", Some("r"), "Alpha"),
            make_node("b", Some("r"), "Beta"),
        ]);
        let out = mindmap_to_markdown(&map);
        assert!(out.contains("# Alpha\n"), "Alpha should be top-level: {out}");
        assert!(out.contains("# Beta\n"), "Beta should be top-level: {out}");
        assert!(!out.contains("## Alpha"), "Alpha should not be nested: {out}");
        assert!(!out.contains("## Beta"), "Beta should not be nested: {out}");
    }

    #[test]
    fn test_export_ignores_notes_and_runs() {
        let mut node = make_node("r", None, "Visible");
        node.notes = "HIDDEN_NOTES_STRING".to_string();
        node.sections[0].text_runs = vec![TextRun {
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
        let map = make_map(vec![make_node("r", None, "Title\nbody line\nmore body")]);
        let out = mindmap_to_markdown(&map);
        assert!(out.starts_with("# Title\nbody line\nmore body\n"), "got: {out:?}");
    }

    #[test]
    fn test_export_sibling_order_matches_index() {
        // Insert out of order; emission must follow `id_sort_key`.
        let map = make_map(vec![
            make_node("0", None, "Root"),
            make_node("0.3", Some("0"), "Late"),
            make_node("0.2", Some("0"), "Mid"),
            make_node("0.1", Some("0"), "Early"),
        ]);
        let out = mindmap_to_markdown(&map);
        let early = out.find("## Early\n").expect("early");
        let mid = out.find("## Mid\n").expect("mid");
        let late = out.find("## Late\n").expect("late");
        assert!(early < mid && mid < late, "out: {out}");
    }
}
