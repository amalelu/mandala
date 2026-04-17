//! Clipboard trait dispatch tests for `TargetView`. Covers the
//! `HandlesCopy`, `HandlesPaste`, and `HandlesCut` impls per
//! component variant (Node / Edge / Portal).

use super::fixtures::{load_test_doc, select_first_edge};
use crate::application::console::traits::{
    view_for, ClipboardContent, HandlesCopy, HandlesCut, HandlesPaste, Outcome, TargetId,
};

// ── Node ─────────────────────────────────────────────────────────

#[test]
fn node_copy_returns_node_text() {
    let mut doc = load_test_doc();
    let nid = doc.mindmap.nodes.keys().next().unwrap().clone();
    let original = doc.mindmap.nodes.get(&nid).unwrap().text.clone();
    let tid = TargetId::Node(nid);
    let view = view_for(&mut doc, &tid);
    match view.clipboard_copy() {
        ClipboardContent::Text(t) => assert_eq!(t, original),
        other => panic!("expected Text, got {:?}", other),
    }
}

#[test]
fn node_copy_empty_text_returns_empty() {
    let mut doc = load_test_doc();
    let nid = doc.mindmap.nodes.keys().next().unwrap().clone();
    doc.set_node_text(&nid, String::new());
    let tid = TargetId::Node(nid);
    let view = view_for(&mut doc, &tid);
    assert_eq!(view.clipboard_copy(), ClipboardContent::Empty);
}

#[test]
fn node_paste_replaces_text_and_pushes_undo() {
    let mut doc = load_test_doc();
    let nid = doc.mindmap.nodes.keys().next().unwrap().clone();
    let undo_before = doc.undo_stack.len();
    let tid = TargetId::Node(nid.clone());
    let outcome = {
        let mut view = view_for(&mut doc, &tid);
        view.clipboard_paste("pasted text")
    };
    assert_eq!(outcome, Outcome::Applied);
    assert_eq!(doc.mindmap.nodes.get(&nid).unwrap().text, "pasted text");
    assert_eq!(doc.undo_stack.len(), undo_before + 1);
}

#[test]
fn node_paste_unchanged_text_reports_unchanged() {
    let mut doc = load_test_doc();
    let nid = doc.mindmap.nodes.keys().next().unwrap().clone();
    let original = doc.mindmap.nodes.get(&nid).unwrap().text.clone();
    let tid = TargetId::Node(nid);
    let mut view = view_for(&mut doc, &tid);
    assert_eq!(view.clipboard_paste(&original), Outcome::Unchanged);
}

#[test]
fn node_cut_returns_text_and_clears_node() {
    let mut doc = load_test_doc();
    let nid = doc.mindmap.nodes.keys().next().unwrap().clone();
    let original = doc.mindmap.nodes.get(&nid).unwrap().text.clone();
    assert!(!original.is_empty(), "fixture node should have text");
    let tid = TargetId::Node(nid.clone());
    let cut = {
        let mut view = view_for(&mut doc, &tid);
        view.clipboard_cut()
    };
    assert_eq!(cut, ClipboardContent::Text(original));
    assert_eq!(doc.mindmap.nodes.get(&nid).unwrap().text, "");
}

// ── Edge ─────────────────────────────────────────────────────────

#[test]
fn edge_copy_returns_label_when_present() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    doc.set_edge_label(&er, Some("hello".into()));
    let tid = TargetId::Edge(er);
    let view = view_for(&mut doc, &tid);
    assert_eq!(view.clipboard_copy(), ClipboardContent::Text("hello".into()));
}

#[test]
fn edge_copy_returns_empty_when_label_missing() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    doc.set_edge_label(&er, None);
    let tid = TargetId::Edge(er);
    let view = view_for(&mut doc, &tid);
    assert_eq!(view.clipboard_copy(), ClipboardContent::Empty);
}

#[test]
fn edge_paste_sets_label() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let tid = TargetId::Edge(er.clone());
    let outcome = {
        let mut view = view_for(&mut doc, &tid);
        view.clipboard_paste("pasted edge label")
    };
    assert_eq!(outcome, Outcome::Applied);
    let edge = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    assert_eq!(edge.label.as_deref(), Some("pasted edge label"));
}

#[test]
fn edge_paste_empty_clears_label() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    doc.set_edge_label(&er, Some("seed".into()));
    let tid = TargetId::Edge(er.clone());
    {
        let mut view = view_for(&mut doc, &tid);
        view.clipboard_paste("");
    }
    let edge = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    assert!(edge.label.is_none());
}

#[test]
fn edge_cut_returns_label_and_clears_it() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    doc.set_edge_label(&er, Some("to be cut".into()));
    let tid = TargetId::Edge(er.clone());
    let cut = {
        let mut view = view_for(&mut doc, &tid);
        view.clipboard_cut()
    };
    assert_eq!(cut, ClipboardContent::Text("to be cut".into()));
    let edge = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    assert!(edge.label.is_none());
}

// ── Portal ───────────────────────────────────────────────────────

fn create_test_portal(doc: &mut crate::application::document::MindMapDocument)
    -> crate::application::document::PortalRef
{
    let nodes: Vec<String> = doc.mindmap.nodes.keys().take(2).cloned().collect();
    assert!(nodes.len() >= 2, "fixture needs at least two nodes");
    doc.apply_create_portal(&nodes[0], &nodes[1]).unwrap()
}

#[test]
fn portal_copy_returns_color_string() {
    let mut doc = load_test_doc();
    let pr = create_test_portal(&mut doc);
    let tid = TargetId::Portal(pr);
    let view = view_for(&mut doc, &tid);
    match view.clipboard_copy() {
        ClipboardContent::Text(c) => assert!(c.starts_with('#'), "expected hex, got {}", c),
        other => panic!("expected Text, got {:?}", other),
    }
}

#[test]
fn portal_paste_accepts_hex_and_sets_color() {
    let mut doc = load_test_doc();
    let pr = create_test_portal(&mut doc);
    let tid = TargetId::Portal(pr.clone());
    let outcome = {
        let mut view = view_for(&mut doc, &tid);
        view.clipboard_paste("#abcdef")
    };
    assert_eq!(outcome, Outcome::Applied);
    let portal = doc.mindmap.portals.iter().find(|p| pr.matches(p)).unwrap();
    assert_eq!(portal.color, "#abcdef");
}

#[test]
fn portal_paste_accepts_var_reference() {
    let mut doc = load_test_doc();
    let pr = create_test_portal(&mut doc);
    let tid = TargetId::Portal(pr.clone());
    let outcome = {
        let mut view = view_for(&mut doc, &tid);
        view.clipboard_paste("var(--accent)")
    };
    assert_eq!(outcome, Outcome::Applied);
    let portal = doc.mindmap.portals.iter().find(|p| pr.matches(p)).unwrap();
    assert_eq!(portal.color, "var(--accent)");
}

#[test]
fn portal_paste_rejects_non_color_text() {
    let mut doc = load_test_doc();
    let pr = create_test_portal(&mut doc);
    let tid = TargetId::Portal(pr);
    let mut view = view_for(&mut doc, &tid);
    match view.clipboard_paste("not a color") {
        Outcome::Invalid(msg) => assert!(msg.contains("not a color")),
        other => panic!("expected Invalid, got {:?}", other),
    }
}

#[test]
fn portal_cut_returns_color_and_resets_to_default() {
    use crate::application::console::constants::PORTAL_DEFAULT_COLOR;
    let mut doc = load_test_doc();
    let pr = create_test_portal(&mut doc);
    let tid = TargetId::Portal(pr.clone());
    let cut = {
        let mut view = view_for(&mut doc, &tid);
        view.clipboard_cut()
    };
    assert!(matches!(cut, ClipboardContent::Text(_)));
    let portal = doc.mindmap.portals.iter().find(|p| pr.matches(p)).unwrap();
    assert_eq!(portal.color, PORTAL_DEFAULT_COLOR);
}
