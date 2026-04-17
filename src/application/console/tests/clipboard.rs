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
    // Pre-trim the node's text. The paste handler trims trailing
    // whitespace (paragraph-copy ergonomics); pasting raw `original`
    // on a node whose text happens to end in whitespace would
    // report `Applied`, and HashMap iteration order picks the
    // "first" node non-deterministically. Normalising first pins
    // the assertion to the round-trip we actually care about.
    let original = doc
        .mindmap
        .nodes
        .get(&nid)
        .unwrap()
        .text
        .trim_end()
        .to_string();
    doc.set_node_text(&nid, original.clone());
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

// Portal-mode edges now flow through the same `TargetId::Edge`
// path as line-mode edges — copy / paste / cut semantics on a
// portal edge use its label (none by default) and fall back to
// the common edge-color path for color edits, so the dedicated
// portal clipboard tests folded into the edge block above and
// were deleted during the portals-as-display-mode refactor.
