//! Reparent operations + undo round-trips.
//!
//! Part of the tests split for `document`. Helpers live in
//! `tests_common`; only the tests for this theme live here.
use super::*;
use super::tests_common::{
    first_testament_edge_ref, first_testament_node_id, load_test_doc, load_test_tree,
    pick_test_edge, test_map_path,
};

use baumhard::gfx_structs::area::GlyphAreaCommand;
use baumhard::gfx_structs::mutator::Mutation;
use baumhard::mindmap::animation::{AnimationTiming, Easing};
use baumhard::mindmap::custom_mutation::{
    apply_mutations_to_element, CustomMutation as CM, DocumentAction,
    MutationBehavior as MB, PlatformContext as PC, TargetScope as TS,
    Trigger as Tr, TriggerBinding as TB,
};
use baumhard::mindmap::model::{
    Canvas, GlyphConnectionConfig, MindEdge, MindNode, NodeLayout, NodeStyle, Position, Size,
    TextRun, PORTAL_GLYPH_PRESETS,
};
use baumhard::mindmap::scene_builder::EdgeHandleKind;
use baumhard::mindmap::model::ControlPoint;
use glam::Vec2;

use super::defaults::default_cross_link_edge;


    fn find_reparent_pair(doc: &MindMapDocument) -> (String, String) {
        // Find two distinct nodes where the source is not an ancestor of the target.
        // Simplest approach: pick two unrelated leaf-ish nodes.
        let ids: Vec<String> = doc.mindmap.nodes.keys().cloned().collect();
        for a in &ids {
            for b in &ids {
                if a == b { continue; }
                // source = a, target parent = b. Valid iff a is not an ancestor of b.
                if !doc.mindmap.is_ancestor_or_self(a, b) {
                    return (b.clone(), a.clone());
                }
            }
        }
        panic!("testament map should contain a valid reparent pair");
    }

    #[test]
    fn test_apply_reparent_single_node_updates_parent_and_index() {
        let mut doc = load_test_doc();
        let (new_parent, source) = find_reparent_pair(&doc);
        let expected_index = doc.mindmap.children_of(&new_parent)
            .iter().map(|n| n.index).max().map(|m| m + 1).unwrap_or(0);

        let undo = doc.apply_reparent(&[source.clone()], Some(&new_parent));
        assert_eq!(undo.entries.len(), 1, "should have one undo entry");

        let node = doc.mindmap.nodes.get(&source).unwrap();
        assert_eq!(node.parent_id.as_deref(), Some(new_parent.as_str()),
            "parent_id should now point to new parent");
        assert_eq!(node.index, expected_index, "index should be max+1 of new siblings");
    }

    #[test]
    fn test_apply_reparent_updates_parent_child_edges() {
        let mut doc = load_test_doc();
        let (new_parent, source) = find_reparent_pair(&doc);

        // Precondition: there should be a parent_child edge leading to source
        // (the testament map wires every hierarchy link as an explicit edge).
        let had_old_edge = doc.mindmap.edges.iter().any(|e|
            e.edge_type == "parent_child" && e.to_id == source
        );

        doc.apply_reparent(&[source.clone()], Some(&new_parent));

        // After reparent: any parent_child edge pointing at source must have
        // from_id == new_parent. There should be at least one such edge if
        // there was one before (or if we're attaching a formerly-root node).
        let parent_edges: Vec<&MindEdge> = doc.mindmap.edges.iter()
            .filter(|e| e.edge_type == "parent_child" && e.to_id == source)
            .collect();
        if had_old_edge {
            assert_eq!(parent_edges.len(), 1,
                "should still have exactly one parent_child edge to source");
            assert_eq!(parent_edges[0].from_id, new_parent,
                "parent_child edge from_id should be updated to new parent");
        }
    }

    #[test]
    fn test_apply_reparent_to_root_removes_edge() {
        let mut doc = load_test_doc();
        let source = doc.mindmap.nodes.values()
            .find(|n| n.parent_id.is_some())
            .map(|n| n.id.clone())
            .expect("testament should have at least one non-root node");

        // Precondition: there should be an existing parent_child edge to source.
        let had_old_edge = doc.mindmap.edges.iter().any(|e|
            e.edge_type == "parent_child" && e.to_id == source
        );
        assert!(had_old_edge, "testament non-root node should have an incoming parent_child edge");

        doc.apply_reparent(&[source.clone()], None);

        // The parent_child edge should have been removed (promoted to root).
        let still_has_edge = doc.mindmap.edges.iter().any(|e|
            e.edge_type == "parent_child" && e.to_id == source
        );
        assert!(!still_has_edge,
            "parent_child edge to source should be removed when promoted to root");
    }

    #[test]
    fn test_apply_reparent_multiple_nodes_become_siblings() {
        let mut doc = load_test_doc();
        // Find a node with two unrelated siblings we can reparent.
        // Use two unrelated nodes from find_reparent_pair repeatedly.
        let (new_parent, first_source) = find_reparent_pair(&doc);
        // Find a second source that is also not an ancestor of new_parent and is
        // not the same as first_source.
        let second_source = doc.mindmap.nodes.keys()
            .find(|k| **k != new_parent && **k != first_source
                && !doc.mindmap.is_ancestor_or_self(k, &new_parent))
            .expect("testament should have another candidate source")
            .clone();

        let start_index = doc.mindmap.children_of(&new_parent)
            .iter().map(|n| n.index).max().map(|m| m + 1).unwrap_or(0);

        let sources = vec![first_source.clone(), second_source.clone()];
        let undo = doc.apply_reparent(&sources, Some(&new_parent));
        assert_eq!(undo.entries.len(), 2, "both sources should be reparented");

        let n1 = doc.mindmap.nodes.get(&first_source).unwrap();
        let n2 = doc.mindmap.nodes.get(&second_source).unwrap();
        assert_eq!(n1.parent_id.as_deref(), Some(new_parent.as_str()));
        assert_eq!(n2.parent_id.as_deref(), Some(new_parent.as_str()));
        // Indices should be start_index and start_index+1, preserving argument order
        assert_eq!(n1.index, start_index);
        assert_eq!(n2.index, start_index + 1);
    }

    #[test]
    fn test_apply_reparent_to_root() {
        let mut doc = load_test_doc();
        // Pick any non-root node
        let source = doc.mindmap.nodes.values()
            .find(|n| n.parent_id.is_some())
            .map(|n| n.id.clone())
            .expect("testament should have at least one non-root node");

        let expected_index = doc.mindmap.root_nodes()
            .iter().map(|n| n.index).max().map(|m| m + 1).unwrap_or(0);

        let undo = doc.apply_reparent(&[source.clone()], None);
        assert_eq!(undo.entries.len(), 1);

        let node = doc.mindmap.nodes.get(&source).unwrap();
        assert_eq!(node.parent_id, None, "should be promoted to root");
        assert_eq!(node.index, expected_index);
    }

    #[test]
    fn test_apply_reparent_rejects_cycle() {
        let mut doc = load_test_doc();
        // Find a parent with a grandchild so we can try to reparent the grandparent
        // under its own grandchild.
        let (grandparent, _child, grandchild) = {
            let mut found = None;
            'outer: for root in doc.mindmap.root_nodes() {
                for child in doc.mindmap.children_of(&root.id) {
                    let grands = doc.mindmap.children_of(&child.id);
                    if let Some(g) = grands.first() {
                        found = Some((root.id.clone(), child.id.clone(), g.id.clone()));
                        break 'outer;
                    }
                }
            }
            found.expect("testament should have a three-level chain")
        };

        let orig_parent = doc.mindmap.nodes.get(&grandparent).unwrap().parent_id.clone();
        let orig_index = doc.mindmap.nodes.get(&grandparent).unwrap().index;

        // Try to reparent grandparent under grandchild — should be silently rejected
        let undo = doc.apply_reparent(&[grandparent.clone()], Some(&grandchild));
        assert!(undo.entries.is_empty(), "cycle should be rejected, no entries in undo data");

        // State should be unchanged
        let gp = doc.mindmap.nodes.get(&grandparent).unwrap();
        assert_eq!(gp.parent_id, orig_parent);
        assert_eq!(gp.index, orig_index);
    }

    #[test]
    fn test_apply_reparent_rejects_self() {
        let mut doc = load_test_doc();
        let source = doc.mindmap.nodes.keys().next().unwrap().clone();
        let orig_parent = doc.mindmap.nodes.get(&source).unwrap().parent_id.clone();

        // Try to reparent a node under itself — should be silently rejected
        let undo = doc.apply_reparent(&[source.clone()], Some(&source));
        assert!(undo.entries.is_empty(), "self-reparent should be rejected");
        assert_eq!(doc.mindmap.nodes.get(&source).unwrap().parent_id, orig_parent);
    }

    #[test]
    fn test_reparent_undo_restores_parent_index_and_edges() {
        let mut doc = load_test_doc();
        let (new_parent, source) = find_reparent_pair(&doc);
        let orig_parent = doc.mindmap.nodes.get(&source).unwrap().parent_id.clone();
        let orig_index = doc.mindmap.nodes.get(&source).unwrap().index;
        let orig_edges_snapshot = doc.mindmap.edges.clone();

        let undo_data = doc.apply_reparent(&[source.clone()], Some(&new_parent));
        doc.undo_stack.push(UndoAction::ReparentNodes {
            entries: undo_data.entries,
            old_edges: undo_data.old_edges,
        });

        // Precondition: actually moved
        assert_eq!(
            doc.mindmap.nodes.get(&source).unwrap().parent_id.as_deref(),
            Some(new_parent.as_str())
        );

        // Undo and verify restoration
        assert!(doc.undo());
        let restored = doc.mindmap.nodes.get(&source).unwrap();
        assert_eq!(restored.parent_id, orig_parent);
        assert_eq!(restored.index, orig_index);

        // Edges should also be restored bit-for-bit
        assert_eq!(doc.mindmap.edges.len(), orig_edges_snapshot.len(),
            "edges Vec length should be restored");
        for (orig, restored) in orig_edges_snapshot.iter().zip(doc.mindmap.edges.iter()) {
            assert_eq!(orig.from_id, restored.from_id);
            assert_eq!(orig.to_id, restored.to_id);
            assert_eq!(orig.edge_type, restored.edge_type);
        }
    }
