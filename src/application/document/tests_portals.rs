//! Portal CRUD undo round-trips. Completes the UndoAction variant
//! coverage: CreatePortal, DeletePortal, and EditPortal were the
//! only three variants without explicit undo tests.

use super::*;
use super::tests_common::{load_test_doc, first_testament_node_id};

/// Find a second node id distinct from the root (348068464).
fn second_testament_node_id(doc: &MindMapDocument) -> String {
    doc.mindmap.nodes.keys()
        .find(|k| k.as_str() != "348068464")
        .expect("testament map has more than one node")
        .clone()
}

#[test]
fn test_undo_create_portal_removes_it() {
    let mut doc = load_test_doc();
    let node_a = first_testament_node_id(&doc);
    let node_b = second_testament_node_id(&doc);
    let initial_count = doc.mindmap.portals.len();

    let pref = doc.apply_create_portal(&node_a, &node_b).unwrap();
    assert_eq!(doc.mindmap.portals.len(), initial_count + 1);
    assert!(doc.dirty);
    assert_eq!(doc.undo_stack.len(), 1);

    // Verify the portal is findable
    assert!(doc.mindmap.portals.iter().any(|p| pref.matches(p)));

    // Undo
    assert!(doc.undo());
    assert_eq!(doc.mindmap.portals.len(), initial_count);
    assert!(!doc.mindmap.portals.iter().any(|p| pref.matches(p)));
}

#[test]
fn test_undo_delete_portal_restores_at_index() {
    let mut doc = load_test_doc();
    let node_a = first_testament_node_id(&doc);
    let node_b = second_testament_node_id(&doc);

    // Create a portal so we have something to delete
    let pref = doc.apply_create_portal(&node_a, &node_b).unwrap();
    let portal_snapshot = doc.mindmap.portals.last().unwrap().clone();
    doc.undo_stack.clear();
    doc.dirty = false;

    // Delete it
    let removed = doc.apply_delete_portal(&pref);
    assert!(removed.is_some());
    assert!(doc.dirty);
    assert!(!doc.mindmap.portals.iter().any(|p| pref.matches(p)));

    // Undo the delete — portal should reappear at the same index
    assert!(doc.undo());
    assert!(doc.mindmap.portals.iter().any(|p| pref.matches(p)));
    let restored = doc.mindmap.portals.iter().find(|p| pref.matches(p)).unwrap();
    assert_eq!(restored.label, portal_snapshot.label);
    assert_eq!(restored.glyph, portal_snapshot.glyph);
    assert_eq!(restored.color, portal_snapshot.color);
}

#[test]
fn test_undo_edit_portal_restores_before_snapshot() {
    let mut doc = load_test_doc();
    let node_a = first_testament_node_id(&doc);
    let node_b = second_testament_node_id(&doc);

    // Create a portal
    let pref = doc.apply_create_portal(&node_a, &node_b).unwrap();
    let original_glyph = doc.mindmap.portals.last().unwrap().glyph.clone();
    doc.undo_stack.clear();
    doc.dirty = false;

    // Edit the glyph
    let changed = doc.set_portal_glyph(&pref, "🔥");
    assert!(changed);
    assert!(doc.dirty);
    let edited = doc.mindmap.portals.iter().find(|p| pref.matches(p)).unwrap();
    assert_eq!(edited.glyph, "🔥");

    // Undo the edit — should restore original glyph
    assert!(doc.undo());
    let restored = doc.mindmap.portals.iter().find(|p| pref.matches(p)).unwrap();
    assert_eq!(restored.glyph, original_glyph);
}

#[test]
fn test_create_portal_rejects_same_node() {
    let mut doc = load_test_doc();
    let node_a = first_testament_node_id(&doc);
    let result = doc.apply_create_portal(&node_a, &node_a);
    assert!(result.is_err());
    assert!(doc.undo_stack.is_empty());
}

#[test]
fn test_create_portal_rejects_unknown_node() {
    let mut doc = load_test_doc();
    let result = doc.apply_create_portal("nonexistent", "also_nonexistent");
    assert!(result.is_err());
    assert!(doc.undo_stack.is_empty());
}

#[test]
fn test_delete_nonexistent_portal_returns_none() {
    let mut doc = load_test_doc();
    let fake_ref = super::types::PortalRef {
        label: "ZZZZZ".to_string(),
        endpoint_a: "x".to_string(),
        endpoint_b: "y".to_string(),
    };
    let result = doc.apply_delete_portal(&fake_ref);
    assert!(result.is_none());
    assert!(doc.undo_stack.is_empty());
}
