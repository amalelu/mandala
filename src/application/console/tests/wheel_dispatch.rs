//! AcceptsWheelColor dispatch tests.
//!
//! The standalone color wheel applies a single color to whatever is
//! selected, and each component type decides which channel that
//! color lands on. These tests lock in the per-variant choice so a
//! future refactor can't silently migrate a node's default to
//! `text_color` or an edge's default to a non-existent `bg_color`.

use super::fixtures::{load_test_doc, select_first_edge};
use crate::application::console::traits::{
    view_for, AcceptsWheelColor, ColorValue, Outcome, TargetId,
};
use crate::application::document::EdgeRef;

/// A node under the wheel takes its color on the **background fill**.
/// Asserted via `style.background_color` after dispatch.
#[test]
fn wheel_color_on_node_paints_background() {
    let mut doc = load_test_doc();
    let nid = doc.mindmap.nodes.keys().next().unwrap().clone();
    let tid = TargetId::Node(nid.clone());
    let outcome = {
        let mut view = view_for(&mut doc, &tid);
        view.apply_wheel_color(ColorValue::Hex("#112233".into()))
    };
    assert_eq!(outcome, Outcome::Applied);
    assert_eq!(
        doc.mindmap.nodes.get(&nid).unwrap().style.background_color,
        "#112233"
    );
}

/// An edge under the wheel takes its color on the **single edge
/// color field** — the line and label share it. Asserted via the
/// glyph-connection override written by `set_edge_color`.
#[test]
fn wheel_color_on_edge_paints_line() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let tid = TargetId::Edge(er.clone());
    let outcome = {
        let mut view = view_for(&mut doc, &tid);
        view.apply_wheel_color(ColorValue::Hex("#445566".into()))
    };
    assert_eq!(outcome, Outcome::Applied);
    let edge = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    // `set_edge_color(Some(..))` writes the override onto the
    // glyph_connection config, which takes precedence over
    // `edge.color`. Checking the effective string covers both the
    // forked-connection path and the raw-color fallback.
    let effective = edge
        .glyph_connection
        .as_ref()
        .and_then(|gc| gc.color.clone())
        .unwrap_or_else(|| edge.color.clone());
    assert_eq!(effective, "#445566");
}

/// A portal-mode edge under the wheel behaves exactly like a
/// line-mode edge — same `TargetId::Edge` dispatch, same
/// `apply_wheel_color` → `set_border_color` route, same sink on
/// `MindEdge.color` (via the glyph_connection override). The only
/// visual difference is where that color lands (markers vs. line),
/// which is a rendering concern outside the trait's scope.
#[test]
fn wheel_color_on_portal_mode_edge_paints_through_edge_path() {
    let mut doc = load_test_doc();
    let mut ids = doc.mindmap.nodes.keys().cloned();
    let a = ids.next().unwrap();
    let b = ids.next().unwrap();
    doc.create_portal_edge(&a, &b).unwrap();
    let er = EdgeRef::new(&a, &b, "cross_link");
    let tid = TargetId::Edge(er.clone());
    let outcome = {
        let mut view = view_for(&mut doc, &tid);
        view.apply_wheel_color(ColorValue::Hex("#778899".into()))
    };
    assert_eq!(outcome, Outcome::Applied);
    let edge = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    let effective = edge
        .glyph_connection
        .as_ref()
        .and_then(|gc| gc.color.clone())
        .unwrap_or_else(|| edge.color.clone());
    assert_eq!(effective, "#778899");
}
