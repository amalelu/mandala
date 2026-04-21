//! Clipboard trait dispatch tests for `TargetView`. Covers the
//! `HandlesCopy`, `HandlesPaste`, and `HandlesCut` impls per
//! component variant (Node / Edge / EdgeLabel / PortalLabel /
//! PortalText).

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

// ── Edge (body) ──────────────────────────────────────────────────
//
// Clipboard semantics: copy / cut return the resolved edge color
// hex; paste sets the edge color from a hex. Label text copy /
// paste is gone (edge labels are edited through the inline modal,
// which owns its own OS-clipboard surface).

#[test]
fn edge_copy_returns_resolved_color_hex() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    doc.set_edge_color(&er, Some("#abcdef"));
    let tid = TargetId::Edge(er);
    let view = view_for(&mut doc, &tid);
    match view.clipboard_copy() {
        ClipboardContent::Text(hex) => assert_eq!(hex, "#abcdef"),
        other => panic!("expected Text with hex, got {:?}", other),
    }
}

#[test]
fn edge_paste_valid_hex_sets_color() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let tid = TargetId::Edge(er.clone());
    let outcome = {
        let mut view = view_for(&mut doc, &tid);
        view.clipboard_paste("#112233")
    };
    assert_eq!(outcome, Outcome::Applied);
    let edge = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    assert_eq!(
        edge.glyph_connection
            .as_ref()
            .and_then(|c| c.color.as_deref()),
        Some("#112233")
    );
}

#[test]
fn edge_paste_invalid_content_reports_invalid() {
    // The paste path rejects arbitrary text — it expects a hex
    // code or `var(--name)` — so garbage produces `Invalid`
    // rather than silently losing a colour edit.
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let tid = TargetId::Edge(er);
    let mut view = view_for(&mut doc, &tid);
    match view.clipboard_paste("not a color") {
        Outcome::Invalid(msg) => assert!(msg.contains("not a color")),
        other => panic!("expected Invalid, got {:?}", other),
    }
}

#[test]
fn edge_paste_rejects_malformed_var_forms() {
    // Tightened `is_valid_color_literal`: reject trailing
    // garbage after the closing `)`, empty var name, and nested
    // parens. The previous `starts_with / ends_with` pair let
    // `var(--accent)extra)` slip through and be stored verbatim
    // on the color field — the renderer then fell back to its
    // "malformed hex" path silently.
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    // Note: the paste path trims leading/trailing whitespace
    // before validation, so trailing-space cases aren't tested
    // here — they normalise to a valid form.
    for malformed in [
        "var(--accent)extra)",  // trailing garbage before the final `)`
        "var(--)",              // empty name
        "var(--foo(bar))",      // nested paren
        "var",                  // no name at all
    ] {
        let tid = TargetId::Edge(er.clone());
        let mut view = view_for(&mut doc, &tid);
        match view.clipboard_paste(malformed) {
            Outcome::Invalid(_) => {}
            other => panic!(
                "expected Invalid for {:?}, got {:?}",
                malformed, other
            ),
        }
    }
}

#[test]
fn edge_paste_accepts_mixed_case_hex() {
    // CSS-style mixed-case hex (`#AbCdEf`) parses as an ordinary
    // 6-digit hex code. Important that the validator doesn't
    // reject uppercase letters; `is_ascii_hexdigit` covers both
    // cases but the matcher needs to stay case-insensitive.
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let tid = TargetId::Edge(er);
    let mut view = view_for(&mut doc, &tid);
    assert_eq!(view.clipboard_paste("#AbCdEf"), Outcome::Applied);
}

#[test]
fn edge_paste_empty_clears_color_override() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    doc.set_edge_color(&er, Some("#abcdef"));
    let tid = TargetId::Edge(er.clone());
    {
        let mut view = view_for(&mut doc, &tid);
        view.clipboard_paste("");
    }
    let edge = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    assert!(edge
        .glyph_connection
        .as_ref()
        .and_then(|c| c.color.as_deref())
        .is_none());
}

#[test]
fn edge_cut_returns_hex_and_clears_override() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    doc.set_edge_color(&er, Some("#abcdef"));
    let tid = TargetId::Edge(er.clone());
    let cut = {
        let mut view = view_for(&mut doc, &tid);
        view.clipboard_cut()
    };
    assert_eq!(cut, ClipboardContent::Text("#abcdef".into()));
    let edge = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    assert!(edge
        .glyph_connection
        .as_ref()
        .and_then(|c| c.color.as_deref())
        .is_none());
}

// ── EdgeLabel ───────────────────────────────────────────────────

#[test]
fn edge_label_copy_returns_resolved_label_color_hex() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    doc.set_edge_label_color(&er, Some("#ff8800"));
    let tid = TargetId::EdgeLabel(er);
    let view = view_for(&mut doc, &tid);
    match view.clipboard_copy() {
        ClipboardContent::Text(hex) => assert_eq!(hex, "#ff8800"),
        other => panic!("expected Text with hex, got {:?}", other),
    }
}

#[test]
fn edge_label_copy_falls_back_to_edge_color_when_override_absent() {
    // With no `label_config.color` override the cascade resolves
    // through `glyph_connection.color` → `edge.color`; copy
    // reports the final concrete hex (no "Empty" — there's always
    // a resolved colour).
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    doc.set_edge_color(&er, Some("#445566"));
    // Ensure no label-specific override is set.
    doc.set_edge_label_color(&er, None);
    let tid = TargetId::EdgeLabel(er);
    let view = view_for(&mut doc, &tid);
    match view.clipboard_copy() {
        ClipboardContent::Text(hex) => assert_eq!(hex, "#445566"),
        other => panic!("expected Text with fallback hex, got {:?}", other),
    }
}

#[test]
fn edge_label_paste_valid_hex_sets_label_color_only() {
    // Pasting a colour onto an `EdgeLabel` selection must NOT
    // touch the edge body's own colour cascade — that's the
    // whole point of a separate label channel.
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    doc.set_edge_color(&er, Some("#000000"));
    let tid = TargetId::EdgeLabel(er.clone());
    let outcome = {
        let mut view = view_for(&mut doc, &tid);
        view.clipboard_paste("#ff00ff")
    };
    assert_eq!(outcome, Outcome::Applied);
    let edge = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    assert_eq!(
        edge.label_config.as_ref().and_then(|c| c.color.as_deref()),
        Some("#ff00ff"),
        "label color should land in label_config"
    );
    assert_eq!(
        edge.glyph_connection
            .as_ref()
            .and_then(|c| c.color.as_deref()),
        Some("#000000"),
        "edge body color must remain unchanged"
    );
}

#[test]
fn edge_label_paste_invalid_content_reports_invalid() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let tid = TargetId::EdgeLabel(er);
    let mut view = view_for(&mut doc, &tid);
    match view.clipboard_paste("not a color") {
        Outcome::Invalid(msg) => assert!(msg.contains("not a color")),
        other => panic!("expected Invalid, got {:?}", other),
    }
}

#[test]
fn edge_label_cut_returns_hex_and_clears_override() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    doc.set_edge_label_color(&er, Some("#ff8800"));
    let tid = TargetId::EdgeLabel(er.clone());
    let cut = {
        let mut view = view_for(&mut doc, &tid);
        view.clipboard_cut()
    };
    assert_eq!(cut, ClipboardContent::Text("#ff8800".into()));
    let edge = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    assert!(edge
        .label_config
        .as_ref()
        .and_then(|c| c.color.as_deref())
        .is_none());
}

// ── PortalText ──────────────────────────────────────────────────

#[test]
fn portal_text_paste_valid_hex_sets_text_color_only() {
    use baumhard::mindmap::model::{is_portal_edge, DISPLAY_MODE_PORTAL};
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    // Convert to portal mode so the endpoint state is meaningful.
    let idx = doc.edge_index(&er).unwrap();
    doc.mindmap.edges[idx].display_mode = Some(DISPLAY_MODE_PORTAL.to_string());
    assert!(is_portal_edge(&doc.mindmap.edges[idx]));
    let endpoint = doc.mindmap.edges[idx].from_id.clone();
    doc.set_portal_label_color(&er, &endpoint, Some("#000000"));

    let tid = TargetId::PortalText {
        edge: er.clone(),
        endpoint_node_id: endpoint.clone(),
    };
    let outcome = {
        let mut view = view_for(&mut doc, &tid);
        view.clipboard_paste("#99ccff")
    };
    assert_eq!(outcome, Outcome::Applied);
    // Confirm `text_color` got the paste and the icon `color`
    // was not touched — the two channels are independent by
    // design.
    let state = baumhard::mindmap::model::portal_endpoint_state(
        &doc.mindmap.edges[idx],
        &endpoint,
    )
    .expect("endpoint state should exist");
    assert_eq!(state.text_color.as_deref(), Some("#99ccff"));
    assert_eq!(state.color.as_deref(), Some("#000000"));
}

#[test]
fn portal_text_cut_returns_hex_and_clears_text_override() {
    use baumhard::mindmap::model::DISPLAY_MODE_PORTAL;
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let idx = doc.edge_index(&er).unwrap();
    doc.mindmap.edges[idx].display_mode = Some(DISPLAY_MODE_PORTAL.to_string());
    let endpoint = doc.mindmap.edges[idx].from_id.clone();
    doc.set_portal_label_text_color(&er, &endpoint, Some("#99ccff"));

    let tid = TargetId::PortalText {
        edge: er.clone(),
        endpoint_node_id: endpoint.clone(),
    };
    let cut = {
        let mut view = view_for(&mut doc, &tid);
        view.clipboard_cut()
    };
    assert_eq!(cut, ClipboardContent::Text("#99ccff".into()));
    let state = baumhard::mindmap::model::portal_endpoint_state(
        &doc.mindmap.edges[idx],
        &endpoint,
    );
    assert!(state.and_then(|s| s.text_color.as_deref()).is_none());
}

// Portal-mode icon (PortalLabel) clipboard continues to work as
// before — covered indirectly through the PortalLabel variant
// sharing the `set_portal_label_color` setter. No dedicated
// PortalLabel copy/paste tests are added here because the
// behaviour was unchanged in this commit.
