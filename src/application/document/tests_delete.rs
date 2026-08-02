// SPDX-License-Identifier: MPL-2.0

//! Edge-ref equality, hit-test-edge, delete-node, delete-edge, cross-link creation, orphan selection.
//!
//! Part of the tests split for `document`. Helpers live in
//! `tests_common`; only the tests for this theme live here.
use super::tests_common::{first_testament_node_id, load_test_doc, pick_test_edge, two_testament_node_ids};
use super::*;

use baumhard::mindmap::model::MindEdge;
use glam::Vec2;

use super::defaults::{default_cross_link_edge, default_orphan_node, default_parent_child_edge};

/// on (or very near) the edge path.
#[test]
fn test_selection_state_edge_variant() {
    let edge_ref = EdgeRef::new("a", "b", "cross_link");
    let sel = SelectionState::Edge(edge_ref.clone());
    assert_eq!(sel.selected_edge(), Some(&edge_ref));
    // Node-selection queries on an edge selection return empty
    assert!(!sel.is_selected("a"));
    assert_eq!(sel.selected_ids().len(), 0);
}

#[test]
fn test_edge_ref_matches() {
    let edge_ref = EdgeRef::new("a", "b", "cross_link");
    let edge = MindEdge {
        from_id: "a".into(),
        to_id: "b".into(),
        edge_type: "cross_link".into(),
        color: "#fff".into(),
        width: 1,
        line_style: "solid".to_string(),
        visible: true,
        label: None,
        label_config: None,
        anchor_from: "auto".to_string(),
        anchor_to: "auto".to_string(),
        control_points: vec![],
        glyph_connection: None,
        display_mode: None,
        portal_from: None,
        portal_to: None,
        min_zoom_to_render: None,
        max_zoom_to_render: None,
    };
    assert!(edge_ref.matches(&edge));

    let wrong_type = EdgeRef::new("a", "b", "parent_child");
    assert!(!wrong_type.matches(&edge));
}

#[test]
fn test_hit_test_edge_hits_on_path() {
    let doc = load_test_doc();
    let (expected, point) = pick_test_edge(&doc);
    let hit = hit_test_edge(point, &doc.mindmap, 2.0);
    assert_eq!(hit, Some(expected));
}

#[test]
fn test_hit_test_edge_miss_far_away() {
    let doc = load_test_doc();
    // A point very far from any node/edge
    let hit = hit_test_edge(Vec2::new(-1_000_000.0, -1_000_000.0), &doc.mindmap, 8.0);
    assert_eq!(hit, None);
}

#[test]
fn test_hit_test_edge_respects_tolerance() {
    let doc = load_test_doc();
    let (_, point) = pick_test_edge(&doc);
    // Shift 50 units away from the path (orthogonal). Tolerance of 5
    // should NOT produce a hit; tolerance of 100 should.
    let offset = Vec2::new(0.0, 50.0);
    let shifted = point + offset;
    assert_eq!(hit_test_edge(shifted, &doc.mindmap, 5.0), None);
    assert!(hit_test_edge(shifted, &doc.mindmap, 100.0).is_some());
}

#[test]
fn test_remove_edge_returns_index_and_edge() {
    let mut doc = load_test_doc();
    let (edge_ref, _) = pick_test_edge(&doc);
    let orig_count = doc.mindmap.edges.len();

    let (idx, removed) = doc.remove_edge(&edge_ref).expect("edge should exist");
    assert!(edge_ref.matches(&removed));
    assert_eq!(doc.mindmap.edges.len(), orig_count - 1);
    // The index should be within the original range
    assert!(idx < orig_count);
}

#[test]
fn test_remove_edge_missing_returns_none() {
    let mut doc = load_test_doc();
    let missing = EdgeRef::new("nope_from", "nope_to", "cross_link");
    assert!(doc.remove_edge(&missing).is_none());
}

#[test]
fn test_undo_delete_edge_restores_at_original_index() {
    let mut doc = load_test_doc();
    let (edge_ref, _) = pick_test_edge(&doc);
    let orig_edges = doc.mindmap.edges.clone();
    let orig_idx = orig_edges.iter().position(|e| edge_ref.matches(e)).unwrap();

    let (idx, edge) = doc.remove_edge(&edge_ref).unwrap();
    doc.undo_stack.push(UndoAction::DeleteEdge { index: idx, edge });
    doc.dirty = true;

    assert_eq!(doc.mindmap.edges.len(), orig_edges.len() - 1);

    // Undo
    assert!(doc.undo());
    assert_eq!(doc.mindmap.edges.len(), orig_edges.len());
    // The edge should be back at its original position
    let restored = &doc.mindmap.edges[orig_idx];
    assert!(edge_ref.matches(restored));
}

// ---------------------------------------------------------------
// Node deletion
// ---------------------------------------------------------------

/// Pick a node from the testament map that has at least one child
/// and at least one parent_child edge pointing at it. The "Lord
/// God" node has plenty of children and is a root, so we walk one
/// level down to find a good candidate that also has a parent.
fn find_node_with_children_and_parent(doc: &MindMapDocument) -> String {
    doc.mindmap
        .nodes
        .values()
        .find(|n| n.parent_id.is_some() && !doc.mindmap.children_of(&n.id).is_empty())
        .map(|n| n.id.clone())
        .expect("testament should have at least one non-root node with children")
}

#[test]
fn test_delete_node_orphans_children() {
    let mut doc = load_test_doc();
    let target = find_node_with_children_and_parent(&doc);
    let child_count = doc.mindmap.children_of(&target).len();
    assert!(child_count > 0, "target should have at least one child");

    let undo = doc.delete_node(&target).expect("delete should succeed");

    // The node itself is gone.
    assert!(!doc.mindmap.nodes.contains_key(&target));
    // Orphaned children got new root-level IDs and are now roots.
    if let UndoAction::DeleteNode {
        ref orphaned_children,
        ..
    } = undo
    {
        assert_eq!(orphaned_children.len(), child_count);
        for (_old_id, new_root_id) in orphaned_children {
            let child = doc
                .mindmap
                .nodes
                .get(new_root_id)
                .expect("orphaned child should exist under its new root ID");
            assert!(
                child.parent_id.is_none(),
                "child {} should be orphaned",
                new_root_id
            );
        }
    } else {
        panic!("expected DeleteNode undo action");
    }
    // No parent_child edges touch the deleted id anymore.
    assert!(
        doc.mindmap
            .edges
            .iter()
            .all(|e| e.from_id != target && e.to_id != target),
        "no edges should reference the deleted node"
    );
}

#[test]
fn test_delete_node_removes_all_touching_edges() {
    let mut doc = load_test_doc();
    let target = find_node_with_children_and_parent(&doc);
    // Count edges touching the target beforehand.
    let touching_before = doc
        .mindmap
        .edges
        .iter()
        .filter(|e| e.from_id == target || e.to_id == target)
        .count();
    assert!(
        touching_before > 0,
        "testament target should have at least one incident edge (parent_child)"
    );

    doc.delete_node(&target).unwrap();

    let touching_after = doc
        .mindmap
        .edges
        .iter()
        .filter(|e| e.from_id == target || e.to_id == target)
        .count();
    assert_eq!(touching_after, 0, "all incident edges should be removed");
}

#[test]
fn test_delete_node_undo_restores_node_edges_and_children() {
    let mut doc = load_test_doc();
    let target = find_node_with_children_and_parent(&doc);

    // Capture pre-delete state to compare after undo.
    let orig_node = doc.mindmap.nodes.get(&target).cloned().unwrap();
    let orig_edges = doc.mindmap.edges.clone();
    let orig_child_state: Vec<(String, Option<String>)> = doc
        .mindmap
        .children_of(&target)
        .iter()
        .map(|n| (n.id.clone(), n.parent_id.clone()))
        .collect();

    let undo = doc.delete_node(&target).unwrap();
    doc.undo_stack.push(undo);
    doc.dirty = true;

    assert!(doc.undo(), "undo should succeed");

    // Node is back.
    let restored = doc.mindmap.nodes.get(&target).expect("node should be restored");
    assert_eq!(restored.id, orig_node.id);
    assert_eq!(restored.display_text(), orig_node.display_text());
    // Edges are fully restored — same count AND same order.
    // Ordering matters: earlier versions of `delete_node` stored
    // post-removal indices, which silently reordered edges that
    // shared the deleted node's neighborhood. Compare each slot
    // by the (from, to, edge_type) triple since edges have no
    // stable id.
    assert_eq!(
        doc.mindmap.edges.len(),
        orig_edges.len(),
        "edge count should be restored"
    );
    for (i, (orig, restored)) in orig_edges.iter().zip(doc.mindmap.edges.iter()).enumerate() {
        assert_eq!(
            (
                orig.from_id.as_str(),
                orig.to_id.as_str(),
                orig.edge_type.as_str()
            ),
            (
                restored.from_id.as_str(),
                restored.to_id.as_str(),
                restored.edge_type.as_str()
            ),
            "edge at index {} should match after undo",
            i,
        );
    }
    // Children are re-attached with original parent_id.
    for (cid, old_parent) in orig_child_state {
        let child = doc.mindmap.nodes.get(&cid).unwrap();
        assert_eq!(
            child.parent_id, old_parent,
            "child {} parent_id should be restored",
            cid
        );
    }
}

/// Regression test for an edge-ordering bug in node deletion:
/// when a deleted node has multiple incident edges scattered
/// through the edge vec, naive in-place removal stores
/// post-removal indices, so the undo reinserts them at the wrong
/// positions and silently reorders edges the caller never
/// touched. The fix stores pre-removal indices via `enumerate()`
/// + `retain()`.
///
/// Built as a self-contained test so we control the edge
/// neighborhood precisely.
#[test]
fn test_delete_node_undo_preserves_edge_order_with_gaps() {
    let mut doc = load_test_doc();
    // Pick any node with at least one incident edge.
    let target = find_node_with_children_and_parent(&doc);

    // Exclude both the target *and* its subtree: `delete_node`
    // cascade-renames descendants to fresh root ids, and rewrites
    // every edge that references those descendants. If any of the
    // picked ids is a descendant, the captured `a`/`b`/`c`/`d`
    // strings below go stale and the post-delete edge assertions
    // flake whenever HashMap iteration happens to put a descendant
    // of `target` in the first four keys.
    let target_prefix = format!("{}.", target);
    let other_ids: Vec<String> = doc
        .mindmap
        .nodes
        .keys()
        .filter(|id| id.as_str() != target.as_str() && !id.starts_with(&target_prefix))
        .take(4)
        .cloned()
        .collect();
    assert!(
        other_ids.len() >= 4,
        "need at least 4 non-target, non-descendant nodes"
    );
    let a = other_ids[0].clone();
    let b = other_ids[1].clone();
    let c = other_ids[2].clone();
    let d = other_ids[3].clone();

    let mk_edge = |from: &str, to: &str, etype: &str| MindEdge {
        from_id: from.to_string(),
        to_id: to.to_string(),
        edge_type: etype.to_string(),
        color: "#ffffff".to_string(),
        width: 1,
        line_style: "solid".to_string(),
        visible: true,
        label: None,
        label_config: None,
        anchor_from: "auto".to_string(),
        anchor_to: "auto".to_string(),
        control_points: Vec::new(),
        glyph_connection: None,
        display_mode: None,
        portal_from: None,
        portal_to: None,
        min_zoom_to_render: None,
        max_zoom_to_render: None,
    };

    // Edge layout: [a→b, a→target, c→d, target→d, b→c]
    //               idx 0      1       2      3       4
    // Positions 1 and 3 touch the target; 0, 2, 4 are bystanders.
    // Wrong behavior would end up reordering the bystanders.
    doc.mindmap.edges = vec![
        mk_edge(&a, &b, "cross_link"),
        mk_edge(&a, &target, "cross_link"),
        mk_edge(&c, &d, "cross_link"),
        mk_edge(&target, &d, "cross_link"),
        mk_edge(&b, &c, "cross_link"),
    ];
    let orig_edges = doc.mindmap.edges.clone();

    let undo = doc.delete_node(&target).unwrap();
    // Sanity: the two touching edges are gone, bystanders remain.
    assert_eq!(doc.mindmap.edges.len(), 3);
    assert_eq!(doc.mindmap.edges[0].to_id, b);
    assert_eq!(doc.mindmap.edges[1].from_id, c);
    assert_eq!(doc.mindmap.edges[2].from_id, b);

    // Undo and verify byte-for-byte positional recovery.
    doc.undo_stack.push(undo);
    doc.dirty = true;
    assert!(doc.undo());

    assert_eq!(doc.mindmap.edges.len(), orig_edges.len());
    for (i, (orig, restored)) in orig_edges.iter().zip(doc.mindmap.edges.iter()).enumerate() {
        assert_eq!(
            (orig.from_id.as_str(), orig.to_id.as_str()),
            (restored.from_id.as_str(), restored.to_id.as_str()),
            "edge at index {} out of order after undo",
            i,
        );
    }
}

#[test]
fn test_delete_node_missing_returns_none() {
    let mut doc = load_test_doc();
    assert!(doc.delete_node("no_such_node_id_exists").is_none());
}

#[test]
fn test_delete_root_node_works() {
    // Delete a top-level root and confirm its children become
    // their own roots. Tests that "orphan children" handles the
    // case where the deleted node has no parent itself.
    let mut doc = load_test_doc();
    // "Lord God" is a known root with children in testament.
    let target = "0".to_string();
    assert!(doc.mindmap.nodes.get(&target).unwrap().parent_id.is_none());
    let child_count = doc.mindmap.children_of(&target).len();
    assert!(child_count > 0);

    let undo = doc.delete_node(&target).unwrap();
    assert!(!doc.mindmap.nodes.contains_key(&target));
    // Orphaned children should have new root-level IDs and be roots.
    if let UndoAction::DeleteNode {
        ref orphaned_children,
        ..
    } = undo
    {
        assert_eq!(orphaned_children.len(), child_count);
        for (_old_id, new_root_id) in orphaned_children {
            assert!(doc.mindmap.nodes.get(new_root_id).unwrap().parent_id.is_none());
        }
    }
}

// ---------------------------------------------------------------
// Regression: delete+undo must never corrupt the map (issue #1 /
// P0-01). The deleted node's children are re-rooted with fresh Dewey
// ids; the original bug scanned the already-shrunk map and handed a
// child the just-deleted node's own id, which undo then overwrote —
// destroying the child and leaving a dangling parent_id. The helpers
// below drive delete+undo and assert byte-equal restoration; the
// tests only differ in the fixture shape.
// ---------------------------------------------------------------

/// Build a doc with the given `roots` (all childless except
/// `subtree_root`), where `subtree_root` owns a two-level subtree
/// `{subtree_root}.0 → {subtree_root}.0.0`. Edges: parent_child down
/// the subtree plus a cross_link from the first root into the deepest
/// descendant, so edge restoration is exercised across a cascaded
/// rename, not just the direct parent_child edges.
fn build_roots_with_deep_subtree(roots: &[&str], subtree_root: &str) -> MindMapDocument {
    assert!(
        roots.contains(&subtree_root),
        "subtree_root {subtree_root} must be one of the roots"
    );
    let mut doc = MindMapDocument::with_orphan(roots[0], Vec2::ZERO);
    for &rid in &roots[1..] {
        doc.mindmap
            .nodes
            .insert(rid.to_string(), default_orphan_node(rid, Vec2::ZERO));
    }
    let child = format!("{subtree_root}.0");
    let grandchild = format!("{subtree_root}.0.0");
    for (cid, parent) in [
        (child.as_str(), subtree_root),
        (grandchild.as_str(), child.as_str()),
    ] {
        let mut node = default_orphan_node(cid, Vec2::ZERO);
        node.parent_id = Some(parent.to_string());
        doc.mindmap.nodes.insert(cid.to_string(), node);
    }
    doc.mindmap.edges = vec![
        default_parent_child_edge(subtree_root, &child),
        default_parent_child_edge(&child, &grandchild),
        default_cross_link_edge(roots[0], &grandchild),
    ];
    doc.undo_stack.clear();
    doc.dirty = false;
    doc
}

/// id → `Debug`-formatted node payload, order-independent. `MindNode`
/// has no interior `HashMap`, so its `Debug` string is a
/// deterministic, field-complete fingerprint — the practical stand-in
/// for the `PartialEq` the model deliberately doesn't derive. Used to
/// assert the post-undo node set is byte-equal to the pre-delete one.
fn node_debug_snapshot(doc: &MindMapDocument) -> std::collections::BTreeMap<String, String> {
    doc.mindmap
        .nodes
        .iter()
        .map(|(id, node)| (id.clone(), format!("{:?}", node)))
        .collect()
}

/// Every `parent_id` in the map must reference a node that exists.
/// The corruption left a child keyed `"3.0"` pointing at a `"3"` that
/// no longer existed; this catches that class of dangling reference.
fn assert_no_dangling_parents(doc: &MindMapDocument) {
    for node in doc.mindmap.nodes.values() {
        if let Some(pid) = node.parent_id.as_deref() {
            assert!(
                doc.mindmap.nodes.contains_key(pid),
                "node {} has dangling parent_id {}",
                node.id,
                pid
            );
        }
    }
}

/// Delete `victim`, push the undo, undo it, and assert the whole model
/// round-trips byte-for-byte: node payloads + parent_ids (order-
/// independent snapshot) and edges (order *and* payload). Also asserts
/// the node is gone mid-way and that neither state dangles. Returns the
/// `orphaned_children` mapping so callers can additionally assert
/// properties of the fresh-root assignment. Single source of truth for
/// the delete+undo contract so the scenario tests can't silently
/// diverge from it.
fn assert_delete_undo_roundtrip(doc: &mut MindMapDocument, victim: &str) -> Vec<(String, String)> {
    let before_nodes = node_debug_snapshot(doc);
    let before_edges = doc.mindmap.edges.clone();

    let undo = doc.delete_node(victim).expect("delete should succeed");
    let orphaned = match &undo {
        UndoAction::DeleteNode {
            orphaned_children, ..
        } => orphaned_children.clone(),
        _ => panic!("expected DeleteNode undo action"),
    };
    assert!(
        !doc.mindmap.nodes.contains_key(victim),
        "{victim} should be gone after delete"
    );
    assert_no_dangling_parents(doc);

    doc.undo_stack.push(undo);
    doc.dirty = true;
    assert!(doc.undo(), "undo of delete({victim}) should succeed");

    assert_eq!(
        node_debug_snapshot(doc),
        before_nodes,
        "delete+undo of {victim}: node set + payloads must be byte-equal"
    );
    assert_eq!(
        doc.mindmap.edges, before_edges,
        "delete+undo of {victim}: edges must be restored exactly (order + payload)"
    );
    assert_no_dangling_parents(doc);
    orphaned
}

#[test]
fn test_delete_max_root_does_not_reuse_deleted_id() {
    // Highest root "3" (roots "0".."3") owns a subtree; re-rooting its
    // child must not be handed the just-deleted "3" (forward-delete
    // invariant, no undo).
    let mut doc = build_roots_with_deep_subtree(&["0", "1", "2", "3"], "3");
    let undo = doc.delete_node("3").expect("delete should succeed");

    if let UndoAction::DeleteNode {
        ref orphaned_children,
        ..
    } = undo
    {
        assert_eq!(orphaned_children.len(), 1, "root \"3\" has exactly one child");
        let (old_id, new_root_id) = &orphaned_children[0];
        assert_eq!(old_id, "3.0");
        assert_ne!(
            new_root_id, "3",
            "orphaned child must not reuse the just-deleted root id"
        );
    } else {
        panic!("expected DeleteNode undo action");
    }
    assert!(!doc.mindmap.nodes.contains_key("3"), "\"3\" was deleted");
    assert_no_dangling_parents(&doc);
}

#[test]
fn test_delete_max_root_with_subtree_undo_restores_exact_model() {
    let mut doc = build_roots_with_deep_subtree(&["0", "1", "2", "3"], "3");
    assert_delete_undo_roundtrip(&mut doc, "3");

    // Structural spot-checks so a failure localizes fast.
    assert!(doc.mindmap.nodes.get("3").unwrap().parent_id.is_none());
    assert_eq!(
        doc.mindmap.nodes.get("3.0").unwrap().parent_id.as_deref(),
        Some("3")
    );
    assert_eq!(
        doc.mindmap.nodes.get("3.0.0").unwrap().parent_id.as_deref(),
        Some("3.0")
    );
}

#[test]
fn test_delete_sole_root_with_subtree_undo_restores_exact_model() {
    // A single root "0" with a two-level subtree is the other face of
    // the same bug: removing the only root before minting leaves zero
    // root-level ids, so a naive mint hands the first orphan "0" — the
    // deleted id — and undo overwrites it.
    let mut doc = build_roots_with_deep_subtree(&["0"], "0");
    assert_delete_undo_roundtrip(&mut doc, "0");
}

// ---------------------------------------------------------------
// Regression: id and tree structure can diverge — `apply_reparent`
// / `apply_orphan_selection` re-point `parent_id` without re-keying
// the node — so a node's Dewey id need not match its position in the
// tree. Deleting such a node must not panic or corrupt. Both fixtures
// below build states reachable via orphan+reparent and both crash /
// corrupt on the naive "keep the deleted node in the map while
// cascading" approach.
// ---------------------------------------------------------------

/// A structural child whose Dewey id is a *prefix* of its parent's id
/// (child `"1"`, parent `"1.0"`). Reachable by orphaning `"1.0"` to a
/// root, then reparenting `"1"` under it. Deleting `"1.0"` must not
/// let the child's cascade sweep the still-keyed parent up by prefix
/// and strand the removal (a panic on the prior commit). Round-trips
/// byte-equal on undo.
#[test]
fn test_delete_node_child_id_is_dewey_prefix_of_parent() {
    let mut doc = MindMapDocument::with_orphan("1.0", Vec2::ZERO);
    // "1" is a structural child of "1.0" (id diverges from structure).
    let mut child = default_orphan_node("1", Vec2::ZERO);
    child.parent_id = Some("1.0".to_string());
    doc.mindmap.nodes.insert("1".to_string(), child);
    // ...and "1" has its own child "1.2" (here id and structure agree).
    let mut grandchild = default_orphan_node("1.2", Vec2::ZERO);
    grandchild.parent_id = Some("1".to_string());
    doc.mindmap.nodes.insert("1.2".to_string(), grandchild);
    doc.mindmap.edges = vec![
        default_parent_child_edge("1.0", "1"),
        default_parent_child_edge("1", "1.2"),
    ];
    doc.undo_stack.clear();
    doc.dirty = false;

    // Must not panic (the pre-fix code hit `remove(node_id).expect(..)`).
    assert_delete_undo_roundtrip(&mut doc, "1.0");
}

/// A subtree whose deepest descendant would reclaim the deleted node's
/// own id if re-rooted naively: delete `"2.0"` (child `"2.0.0"`,
/// grandchild `"2.0.0.0"`) while roots `"0"`/`"1"` exist. Minting the
/// re-rooted id from the shrunk root set yields `"2"`, so the grandchild
/// `"2.0.0.0"` cascades to `"2.0"` — colliding with the node still being
/// deleted. The fix mints above every leading segment, avoiding it.
#[test]
fn test_delete_node_grandchild_would_reclaim_deleted_id() {
    let mut doc = MindMapDocument::with_orphan("0", Vec2::ZERO);
    doc.mindmap
        .nodes
        .insert("1".to_string(), default_orphan_node("1", Vec2::ZERO));
    // "2.0" is a root (its former root "2" was deleted); it owns a
    // two-level subtree.
    doc.mindmap
        .nodes
        .insert("2.0".to_string(), default_orphan_node("2.0", Vec2::ZERO));
    for (cid, parent) in [("2.0.0", "2.0"), ("2.0.0.0", "2.0.0")] {
        let mut node = default_orphan_node(cid, Vec2::ZERO);
        node.parent_id = Some(parent.to_string());
        doc.mindmap.nodes.insert(cid.to_string(), node);
    }
    doc.mindmap.edges = vec![
        default_parent_child_edge("2.0", "2.0.0"),
        default_parent_child_edge("2.0.0", "2.0.0.0"),
        default_cross_link_edge("0", "2.0.0.0"),
    ];
    doc.undo_stack.clear();
    doc.dirty = false;

    let orphaned = assert_delete_undo_roundtrip(&mut doc, "2.0");

    // The re-rooted child must not have reclaimed the deleted id "2.0"
    // (nor a Dewey prefix of it — else a descendant would).
    for (_old, new_root) in &orphaned {
        assert_ne!(new_root, "2.0", "orphan must not reclaim the deleted id");
        assert!(
            !"2.0".starts_with(&format!("{}.", new_root)),
            "orphan root {} must not be a Dewey prefix of the deleted id",
            new_root
        );
    }
}

#[test]
fn test_connection_pass_highlights_selected_edge() {
    let mut doc = load_test_doc();
    let (edge_ref, _) = pick_test_edge(&doc);

    // Without selection: the edge renders with its model color
    let scene_normal = crate::application::document::tests_common::project_roles(&doc, 1.0, InteractionModeOverrides::none());
    let normal_colors: Vec<String> = scene_normal
        .connection_elements
        .iter()
        .map(|c| c.color.clone())
        .collect();

    // With edge selected: its element color should be the cyan highlight
    doc.selection = SelectionState::Edge(edge_ref);
    let scene_selected = crate::application::document::tests_common::project_roles(&doc, 1.0, InteractionModeOverrides::none());
    let highlighted_count = scene_selected
        .connection_elements
        .iter()
        .filter(|c| c.color.eq_ignore_ascii_case("#00E5FF"))
        .count();
    assert_eq!(
        highlighted_count, 1,
        "exactly one connection element should carry the selection color"
    );
    // And exactly one color should have changed vs. the unselected scene
    let changed: usize = scene_selected
        .connection_elements
        .iter()
        .zip(normal_colors.iter())
        .filter(|(c, orig)| &c.color != *orig)
        .count();
    assert_eq!(changed, 1);
}

// ---------------------------------------------------------------------
// Connection creation
// ---------------------------------------------------------------------

#[test]
fn test_default_cross_link_edge_fields() {
    let e = default_cross_link_edge("a", "b");
    assert_eq!(e.from_id, "a");
    assert_eq!(e.to_id, "b");
    assert_eq!(e.edge_type, "cross_link");
    assert!(e.visible);
    assert_eq!(e.anchor_from, "auto");
    assert_eq!(e.anchor_to, "auto");
    assert!(e.control_points.is_empty());
    assert!(e.label.is_none());
}

#[test]
fn test_create_cross_link_edge_success() {
    let mut doc = load_test_doc();
    // Pick two nodes that are definitely distinct
    let (a, b) = two_testament_node_ids(&doc);
    let orig_count = doc.mindmap.edges.len();

    let idx = doc.create_cross_link_edge(&a, &b).expect("should succeed");
    assert_eq!(idx, orig_count);
    assert_eq!(doc.mindmap.edges.len(), orig_count + 1);
    let created = &doc.mindmap.edges[idx];
    assert_eq!(created.edge_type, "cross_link");
    assert_eq!(created.from_id, a);
    assert_eq!(created.to_id, b);
}

#[test]
fn test_create_cross_link_rejects_self_link() {
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    let orig_count = doc.mindmap.edges.len();
    assert!(doc.create_cross_link_edge(&id, &id).is_none());
    assert_eq!(doc.mindmap.edges.len(), orig_count);
}

#[test]
fn test_create_cross_link_rejects_duplicate() {
    let mut doc = load_test_doc();
    let (a, b) = two_testament_node_ids(&doc);

    assert!(doc.create_cross_link_edge(&a, &b).is_some());
    // Second attempt should be a no-op
    let orig_count = doc.mindmap.edges.len();
    assert!(doc.create_cross_link_edge(&a, &b).is_none());
    assert_eq!(doc.mindmap.edges.len(), orig_count);
}

#[test]
fn test_create_cross_link_rejects_unknown_node() {
    let mut doc = load_test_doc();
    let known = first_testament_node_id(&doc);
    assert!(doc.create_cross_link_edge(&known, "does_not_exist").is_none());
    assert!(doc.create_cross_link_edge("does_not_exist", &known).is_none());
}

#[test]
fn test_undo_create_edge_removes_it() {
    let mut doc = load_test_doc();
    let (a, b) = two_testament_node_ids(&doc);
    let orig_count = doc.mindmap.edges.len();

    let idx = doc.create_cross_link_edge(&a, &b).unwrap();
    doc.undo_stack.push(UndoAction::CreateEdge { index: idx });

    assert!(doc.undo());
    assert_eq!(doc.mindmap.edges.len(), orig_count);
    // No cross_link between a and b should remain
    let still_there = doc
        .mindmap
        .edges
        .iter()
        .any(|e| e.edge_type == "cross_link" && e.from_id == a && e.to_id == b);
    assert!(!still_there);
}

// ---------------------------------------------------------------------
// Orphan node creation + orphan-selection action
// ---------------------------------------------------------------------

#[test]
fn test_create_orphan_node_adds_to_map() {
    let mut doc = load_test_doc();
    let orig_count = doc.mindmap.nodes.len();
    let pos = Vec2::new(123.0, 456.0);

    let new_id = doc.apply_create_orphan_node(pos);

    assert_eq!(doc.mindmap.nodes.len(), orig_count + 1);
    let node = doc.mindmap.nodes.get(&new_id).expect("new node must exist");
    assert_eq!(node.id, new_id);
    assert!(node.parent_id.is_none(), "orphan should have no parent");
    assert_eq!(node.position.x, 123.0);
    assert_eq!(node.position.y, 456.0);
    assert!(
        !node.sections[0].text.is_empty(),
        "orphan should have placeholder text"
    );
}

#[test]
fn test_create_orphan_node_ids_are_unique() {
    let mut doc = load_test_doc();
    let a = doc.apply_create_orphan_node(Vec2::ZERO);
    let b = doc.apply_create_orphan_node(Vec2::ZERO);
    let c = doc.apply_create_orphan_node(Vec2::ZERO);
    assert_ne!(a, b);
    assert_ne!(b, c);
    assert_ne!(a, c);
    // All three should exist in the map
    assert!(doc.mindmap.nodes.contains_key(&a));
    assert!(doc.mindmap.nodes.contains_key(&b));
    assert!(doc.mindmap.nodes.contains_key(&c));
}

#[test]
fn test_undo_create_node_removes_it() {
    let mut doc = load_test_doc();
    let orig_count = doc.mindmap.nodes.len();

    let new_id = doc.apply_create_orphan_node(Vec2::new(0.0, 0.0));
    doc.undo_stack.push(UndoAction::CreateNode {
        node_id: new_id.clone(),
    });
    doc.selection = SelectionState::Single(new_id.clone());

    assert!(doc.undo());
    assert_eq!(doc.mindmap.nodes.len(), orig_count);
    assert!(!doc.mindmap.nodes.contains_key(&new_id));
    // Selection should have been cleared since it referenced the deleted node
    assert!(matches!(doc.selection, SelectionState::None));
}

#[test]
fn test_orphan_selection_promotes_to_root_and_keeps_subtree() {
    // Pick a non-root node that has at least one child, so we can
    // verify the subtree stays attached after orphaning.
    let mut doc = load_test_doc();
    let (parent_having_child, child) = doc
        .mindmap
        .nodes
        .values()
        .find_map(|n| {
            let kids = doc.mindmap.children_of(&n.id);
            if !kids.is_empty() && n.parent_id.is_some() {
                Some((n.id.clone(), kids[0].id.clone()))
            } else {
                None
            }
        })
        .expect("testament map should have at least one non-root parent node");

    // Precondition: the selected node has a parent, and has a child
    assert!(doc
        .mindmap
        .nodes
        .get(&parent_having_child)
        .unwrap()
        .parent_id
        .is_some());
    let child_of_node = doc.mindmap.nodes.get(&child).unwrap().parent_id.clone();
    assert_eq!(child_of_node.as_deref(), Some(parent_having_child.as_str()));

    let undo = doc.apply_orphan_selection(&[parent_having_child.clone()]);
    assert_eq!(undo.entries.len(), 1);

    // The orphaned node is now a root...
    assert!(doc
        .mindmap
        .nodes
        .get(&parent_having_child)
        .unwrap()
        .parent_id
        .is_none());
    // ...but its child is still attached to it.
    assert_eq!(
        doc.mindmap.nodes.get(&child).unwrap().parent_id.as_deref(),
        Some(parent_having_child.as_str()),
        "child subtree should stay attached to the orphaned node"
    );
}

#[test]
fn test_orphan_selection_undo_reattaches() {
    let mut doc = load_test_doc();
    let non_root = doc
        .mindmap
        .nodes
        .values()
        .find(|n| n.parent_id.is_some())
        .map(|n| n.id.clone())
        .expect("at least one non-root node");
    let original_parent = doc.mindmap.nodes.get(&non_root).unwrap().parent_id.clone();

    let undo = doc.apply_orphan_selection(&[non_root.clone()]);
    doc.undo_stack.push(UndoAction::ReparentNodes {
        entries: undo.entries,
        old_edges: undo.old_edges,
    });

    // Precondition: it's now a root
    assert!(doc.mindmap.nodes.get(&non_root).unwrap().parent_id.is_none());

    // Undo restores the parent link
    assert!(doc.undo());
    let restored = doc.mindmap.nodes.get(&non_root).unwrap();
    assert_eq!(restored.parent_id, original_parent);
}

#[test]
fn test_orphan_selection_on_root_is_noop() {
    let mut doc = load_test_doc();
    let root = doc.mindmap.root_nodes().first().map(|n| n.id.clone()).unwrap();
    let orig_edges_len = doc.mindmap.edges.len();

    let undo = doc.apply_orphan_selection(&[root.clone()]);
    // The node is already a root, so there are entries (it's a valid
    // "move-to-last-root-index" op), but nothing meaningful changed:
    // parent_id is still None.
    assert!(doc.mindmap.nodes.get(&root).unwrap().parent_id.is_none());
    // And since it was already a root, no parent_child edge was removed.
    assert_eq!(doc.mindmap.edges.len(), orig_edges_len);
    // undo.entries may be non-empty but the restoration is a no-op.
    let _ = undo;
}
