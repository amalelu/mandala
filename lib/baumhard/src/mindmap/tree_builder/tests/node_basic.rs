// SPDX-License-Identifier: MPL-2.0

//! Tree-builder node tests — structure, root nodes, glyph_area properties, color regions, parent/child hierarchy, unique IDs.

use super::super::*;
use super::fixtures::*;
use crate::mindmap::loader;

#[test]
fn test_build_tree_structure() {
    let path = test_map_path();
    let map = loader::load_from_file(&path).unwrap();
    let result = build_mindmap_tree(&map);

    // Testament map has 252 nodes (none folded by default)
    assert_eq!(result.node_count(), 252);

    // Root of tree is Void, its children are the mindmap root nodes
    let root_children: Vec<_> = result.tree.root.children(&result.tree.arena).collect();
    let mindmap_roots = map.root_nodes();
    assert_eq!(root_children.len(), mindmap_roots.len());
}

#[test]
fn test_tree_root_nodes_match_mindmap() {
    let path = test_map_path();
    let map = loader::load_from_file(&path).unwrap();
    let result = build_mindmap_tree(&map);

    let mindmap_roots = map.root_nodes();
    let tree_root_children: Vec<NodeId> = result.tree.root.children(&result.tree.arena).collect();

    // Each mindmap root should be in the node_map and a child of tree root
    for root in &mindmap_roots {
        let node_id = result.arena_id_for(&root.id).expect("Root not in node_map");
        assert!(
            tree_root_children.contains(&node_id),
            "Root {} not a child of tree root",
            root.id
        );
    }
}

#[test]
fn test_glyph_area_properties() {
    let path = test_map_path();
    let map = loader::load_from_file(&path).unwrap();
    let result = build_mindmap_tree(&map);

    // Container area: chrome only — empty text post-section-refactor.
    let lord_god = map.nodes.get("0").unwrap();
    let node_id = result.arena_id_for("0").unwrap();
    let element = result.tree.arena.get(node_id).unwrap().get();
    let container = element.glyph_area().expect("container is a GlyphArea");
    assert!(container.text.is_empty(), "container area carries no glyphs");
    assert_eq!(container.position.x.0, lord_god.position.x as f32);
    assert_eq!(container.position.y.0, lord_god.position.y as f32);
    assert_eq!(container.render_bounds.x.0, lord_god.size.width as f32);
    assert_eq!(container.render_bounds.y.0, lord_god.size.height as f32);

    // Section[0] area: text-bearing surface — carries the section's
    // `text` and the scale derived from its first run.
    let section_id = result.section_arena_id("0", 0).expect("section[0] arena id");
    let section_area = result
        .tree
        .arena
        .get(section_id)
        .unwrap()
        .get()
        .glyph_area()
        .expect("section-area is a GlyphArea");
    assert_eq!(section_area.text, "Lord God");
    assert_eq!(section_area.scale.0, lord_god.sections[0].text_runs[0].size_pt);
}

#[test]
fn test_color_regions_from_text_runs() {
    let path = test_map_path();
    let map = loader::load_from_file(&path).unwrap();
    let result = build_mindmap_tree(&map);

    // Regions live on the section-area, not the container.
    let section_id = result.section_arena_id("0", 0).unwrap();
    let element = result.tree.arena.get(section_id).unwrap().get();
    let area = element.glyph_area().unwrap();

    assert_eq!(area.regions.num_regions(), 1);
    let region = area.regions.all_regions()[0];
    assert_eq!(region.range.start, 0);
    assert_eq!(region.range.end, 8);
    // White color: [1.0, 1.0, 1.0, 1.0]
    let c = region.color.unwrap();
    assert!((c[0] - 1.0).abs() < 0.01);
    assert!((c[1] - 1.0).abs() < 0.01);
    assert!((c[2] - 1.0).abs() < 0.01);
}

#[test]
fn test_parent_child_hierarchy_preserved() {
    let path = test_map_path();
    let map = loader::load_from_file(&path).unwrap();
    let result = build_mindmap_tree(&map);

    // Lord God's child *MindNodes* still appear as children of
    // its container in the arena. The container also gains
    // section-area / section-model children post-refactor; this
    // test filters them out so the hierarchy invariant is checked
    // against mind-node ids only.
    let lord_god_tree_id = result.arena_id_for("0").unwrap();
    let mindmap_children = map.children_of("0");

    let containers: std::collections::HashSet<NodeId> = result.node_ids().map(|(_, id)| id).collect();
    let mind_child_arena_ids: Vec<NodeId> = lord_god_tree_id
        .children(&result.tree.arena)
        .filter(|cid| containers.contains(cid))
        .collect();
    assert_eq!(mind_child_arena_ids.len(), mindmap_children.len());

    for child in &mindmap_children {
        let child_tree_id = result.arena_id_for(&child.id).expect("Child not in node_map");
        assert!(
            mind_child_arena_ids.contains(&child_tree_id),
            "Child {} not a tree child of Lord God",
            child.id
        );
    }
}

#[test]
fn test_unique_ids_are_unique() {
    let path = test_map_path();
    let map = loader::load_from_file(&path).unwrap();
    let result = build_mindmap_tree(&map);

    let mut seen_ids = std::collections::HashSet::new();
    for (_, node_id) in result.node_ids() {
        let element = result.tree.arena.get(node_id).unwrap().get();
        let uid = element.unique_id();
        assert!(seen_ids.insert(uid), "Duplicate unique_id: {}", uid);
    }
}

#[test]
fn test_all_elements_are_glyph_areas() {
    let path = test_map_path();
    let map = loader::load_from_file(&path).unwrap();
    let result = build_mindmap_tree(&map);

    for (_, node_id) in result.node_ids() {
        let element = result.tree.arena.get(node_id).unwrap().get();
        assert!(element.glyph_area().is_some(), "Expected GlyphArea for node");
    }
}

#[test]
fn test_text_run_font_propagates_to_color_font_region() {
    use crate::font::fonts;
    use crate::mindmap::model::TextRun;

    fonts::init();
    // Pick any loaded family — the first one keeps the test
    // resilient against future font additions / renames.
    let family = fonts::list_loaded_families()
        .into_iter()
        .next()
        .expect("at least one font family must be loaded");
    let expected = fonts::app_font_by_family(&family).expect("the family we just picked must round-trip");

    let mut node = synthetic_node("font-run", None, 0.0, 0.0);
    node.sections[0].text = "Hi".to_string();
    node.sections[0].text_runs = vec![TextRun {
        start: 0,
        end: 2,
        bold: false,
        italic: false,
        underline: false,
        font: family.clone(),
        size_pt: 14.0,
        color: "#ffffff".into(),
        hyperlink: None,
    }];
    let map = synthetic_map(vec![node], vec![]);
    let result = build_mindmap_tree(&map);
    // Regions live on the section-area, not the container.
    let section_id = result.section_arena_id("font-run", 0).unwrap();
    let area = glyph_area_of(&result.tree, section_id);
    let regions = area.regions.all_regions();
    assert_eq!(regions.len(), 1);
    assert_eq!(
        regions[0].font,
        Some(expected),
        "TextRun.font='{}' must resolve to AppFont::{:?}",
        family,
        expected
    );
}

#[test]
fn test_text_run_unknown_font_falls_back_to_none() {
    use crate::font::fonts;
    use crate::mindmap::model::TextRun;

    fonts::init();
    let mut node = synthetic_node("font-unknown", None, 0.0, 0.0);
    node.sections[0].text = "Hi".to_string();
    node.sections[0].text_runs = vec![TextRun {
        start: 0,
        end: 2,
        bold: false,
        italic: false,
        underline: false,
        font: "DefinitelyNotAFontFamilyXYZ".into(),
        size_pt: 14.0,
        color: "#ffffff".into(),
        hyperlink: None,
    }];
    let map = synthetic_map(vec![node], vec![]);
    let result = build_mindmap_tree(&map);
    let section_id = result.section_arena_id("font-unknown", 0).unwrap();
    let area = glyph_area_of(&result.tree, section_id);
    let regions = area.regions.all_regions();
    assert_eq!(regions.len(), 1);
    assert_eq!(
        regions[0].font, None,
        "unknown family must resolve to None so the attrs builder \
         falls back to monospace"
    );
}
