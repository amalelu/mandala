// SPDX-License-Identifier: MPL-2.0

//! Edge-handle hit-test, ensure_glyph_connection, color-picker preview, edge style setters (first half).
//!
//! Part of the tests split for `document`. Helpers live in
//! `tests_common`; only the tests for this theme live here.
use super::tests_common::{first_testament_edge_ref, load_test_doc, pick_test_edge};
use super::*;

use baumhard::mindmap::model::ControlPoint;
use baumhard::mindmap::model::GlyphConnectionConfig;
use baumhard::mindmap::tree_builder::EdgeHandleKind;
use baumhard::util::geometry::aabb_center;
use glam::Vec2;

#[test]
fn test_hit_test_edge_handle_finds_anchor_from() {
    let doc = load_test_doc();
    let (edge_ref, _) = pick_test_edge(&doc);
    let edge = doc.mindmap.edges.iter().find(|e| edge_ref.matches(e)).unwrap();
    let from_node = doc.mindmap.nodes.get(&edge.from_id).unwrap();
    let to_node = doc.mindmap.nodes.get(&edge.to_id).unwrap();
    let from_pos = from_node.pos_vec2();
    let from_size = from_node.size_vec2();
    let to_pos = to_node.pos_vec2();
    let to_size = to_node.size_vec2();
    let to_center = aabb_center(to_pos, to_size);
    let anchor_from_pos = baumhard::mindmap::connection::resolve_anchor_point(
        from_pos,
        from_size,
        &edge.anchor_from,
        to_center,
    );

    let hit = doc.hit_test_edge_handle(anchor_from_pos, &edge_ref, 2.0);
    assert!(
        matches!(hit, Some((EdgeHandleKind::AnchorFrom, _))),
        "expected AnchorFrom hit, got {:?}",
        hit.as_ref().map(|(k, _)| k)
    );
}

#[test]
fn test_hit_test_edge_handle_finds_midpoint_on_straight_edge() {
    let mut doc = load_test_doc();
    // Make sure we have a straight edge with empty control_points
    let edge_idx = doc
        .mindmap
        .edges
        .iter()
        .position(|e| e.visible && e.control_points.is_empty())
        .expect("testament map should have at least one straight edge");
    let edge_ref = EdgeRef::new(
        &doc.mindmap.edges[edge_idx].from_id,
        &doc.mindmap.edges[edge_idx].to_id,
        &doc.mindmap.edges[edge_idx].edge_type,
    );
    let _ = &mut doc;

    let edge = &doc.mindmap.edges[edge_idx];
    let from_node = doc.mindmap.nodes.get(&edge.from_id).unwrap();
    let to_node = doc.mindmap.nodes.get(&edge.to_id).unwrap();
    let from_pos = from_node.pos_vec2();
    let from_size = from_node.size_vec2();
    let to_pos = to_node.pos_vec2();
    let to_size = to_node.size_vec2();
    let from_center = aabb_center(from_pos, from_size);
    let to_center = aabb_center(to_pos, to_size);
    let start = baumhard::mindmap::connection::resolve_anchor_point(
        from_pos,
        from_size,
        &edge.anchor_from,
        to_center,
    );
    let end =
        baumhard::mindmap::connection::resolve_anchor_point(to_pos, to_size, &edge.anchor_to, from_center);
    let midpoint = start.lerp(end, 0.5);

    let hit = doc.hit_test_edge_handle(midpoint, &edge_ref, 2.0);
    assert!(
        matches!(hit, Some((EdgeHandleKind::Midpoint, _))),
        "expected Midpoint hit for straight edge"
    );
}

#[test]
fn test_hit_test_edge_handle_no_midpoint_on_curved_edge() {
    let mut doc = load_test_doc();
    // Give an edge a control point so it's curved
    let edge_idx = doc.mindmap.edges.iter().position(|e| e.visible).unwrap();
    doc.mindmap.edges[edge_idx]
        .control_points
        .push(ControlPoint { x: 50.0, y: 50.0 });
    let edge_ref = EdgeRef::new(
        &doc.mindmap.edges[edge_idx].from_id,
        &doc.mindmap.edges[edge_idx].to_id,
        &doc.mindmap.edges[edge_idx].edge_type,
    );

    // Compute what the midpoint WOULD be on a straight line; the
    // hit test should NOT return Midpoint for this curved edge
    // regardless of whether some other handle happens to be near.
    let edge = &doc.mindmap.edges[edge_idx];
    let from_node = doc.mindmap.nodes.get(&edge.from_id).unwrap();
    let from_pos = from_node.pos_vec2();
    let from_size = from_node.size_vec2();
    let from_center = aabb_center(from_pos, from_size);

    // The control point is at from_center + (50, 50). Hit there:
    // should get ControlPoint(0), not Midpoint.
    let cp_pos = from_center + Vec2::new(50.0, 50.0);
    let hit = doc.hit_test_edge_handle(cp_pos, &edge_ref, 5.0);
    assert!(
        matches!(hit, Some((EdgeHandleKind::ControlPoint(0), _))),
        "expected ControlPoint(0) hit on curved edge, got {:?}",
        hit.as_ref().map(|(k, _)| k)
    );
}

#[test]
fn test_hit_test_edge_handle_miss_outside_tolerance() {
    let doc = load_test_doc();
    let (edge_ref, _) = pick_test_edge(&doc);
    let hit = doc.hit_test_edge_handle(Vec2::new(-99999.0, -99999.0), &edge_ref, 10.0);
    assert!(hit.is_none());
}

#[test]
fn test_reset_edge_to_straight_clears_control_points() {
    let mut doc = load_test_doc();
    let edge_idx = doc.mindmap.edges.iter().position(|e| e.visible).unwrap();
    doc.mindmap.edges[edge_idx]
        .control_points
        .push(ControlPoint { x: 10.0, y: 20.0 });
    let edge_ref = EdgeRef::new(
        &doc.mindmap.edges[edge_idx].from_id,
        &doc.mindmap.edges[edge_idx].to_id,
        &doc.mindmap.edges[edge_idx].edge_type,
    );
    let ok = doc.reset_edge_to_straight(&edge_ref);
    assert!(ok, "reset should report success");
    assert!(doc.mindmap.edges[edge_idx].control_points.is_empty());
    assert!(doc.dirty);
    assert_eq!(doc.undo_stack.len(), 1);
}

#[test]
fn test_reset_edge_to_straight_noop_on_already_straight() {
    let mut doc = load_test_doc();
    let edge_idx = doc
        .mindmap
        .edges
        .iter()
        .position(|e| e.visible && e.control_points.is_empty())
        .unwrap();
    let edge_ref = EdgeRef::new(
        &doc.mindmap.edges[edge_idx].from_id,
        &doc.mindmap.edges[edge_idx].to_id,
        &doc.mindmap.edges[edge_idx].edge_type,
    );
    let ok = doc.reset_edge_to_straight(&edge_ref);
    assert!(!ok, "reset on already-straight edge should be a no-op");
    assert!(doc.undo_stack.is_empty());
}

#[test]
fn test_reset_edge_position_line_mode_and_undo_round_trip() {
    // Line mode: reset clears non-auto anchors + label overrides.
    // Undo must reinstate *every* cleared field from a single
    // EditEdge snapshot — the whole point of snapshotting the
    // pre-edit edge instead of diffing per-field.
    let mut doc = load_test_doc();
    let edge_ref = first_testament_edge_ref(&doc);
    // Seed non-default state on the edge: mangle both anchors,
    // and give the label a position + perpendicular override.
    let edge_mut = doc
        .mindmap
        .edges
        .iter_mut()
        .find(|e| edge_ref.matches(e))
        .unwrap();
    edge_mut.anchor_from = "top".into();
    edge_mut.anchor_to = "bottom".into();
    doc.set_edge_label_position(&edge_ref, 0.25);
    doc.set_edge_label_perpendicular_offset(&edge_ref, Some(9.0));
    let before = doc
        .mindmap
        .edges
        .iter()
        .find(|e| edge_ref.matches(e))
        .unwrap()
        .clone();
    doc.undo_stack.clear();
    assert!(doc.reset_edge_position(&edge_ref, None));
    let after = doc.mindmap.edges.iter().find(|e| edge_ref.matches(e)).unwrap();
    assert_eq!(after.anchor_from, "auto");
    assert_eq!(after.anchor_to, "auto");
    assert!(after.label_config.is_none());
    // Undo must restore the full pre-reset edge.
    doc.undo();
    let restored = doc.mindmap.edges.iter().find(|e| edge_ref.matches(e)).unwrap();
    assert_eq!(restored.anchor_from, before.anchor_from);
    assert_eq!(restored.anchor_to, before.anchor_to);
    assert_eq!(restored.label_config, before.label_config);
}

#[test]
fn test_reset_edge_position_portal_single_endpoint() {
    // Single-endpoint reset: clearing one side must leave the
    // other side's `border_t` / `perpendicular_offset` intact,
    // and a subsequent undo must restore the cleared side
    // byte-for-byte from the one `EditEdge` snapshot — the
    // "single snapshot restores both sides" invariant the
    // whole reset shape rests on.
    let mut doc = load_test_doc();
    let edge_ref = first_testament_edge_ref(&doc);
    doc.set_edge_display_mode(&edge_ref, "portal");
    let edge = doc.mindmap.edges.iter().find(|e| edge_ref.matches(e)).unwrap();
    let from_id = edge.from_id.clone();
    let to_id = edge.to_id.clone();
    doc.set_portal_label_border_t(&edge_ref, &from_id, Some(1.5));
    doc.set_portal_label_perpendicular_offset(&edge_ref, &from_id, Some(10.0));
    doc.set_portal_label_border_t(&edge_ref, &to_id, Some(3.0));
    let before = doc
        .mindmap
        .edges
        .iter()
        .find(|e| edge_ref.matches(e))
        .unwrap()
        .clone();
    doc.undo_stack.clear();

    assert!(doc.reset_edge_position(&edge_ref, Some(&from_id)));
    let after = doc.mindmap.edges.iter().find(|e| edge_ref.matches(e)).unwrap();
    assert!(after.portal_from.is_none(), "from endpoint must prune");
    assert_eq!(
        after.portal_to.as_ref().and_then(|s| s.border_t),
        Some(3.0),
        "to-side border_t must survive a from-only reset"
    );

    // Undo: one snapshot, both sides restored.
    assert_eq!(doc.undo_stack.len(), 1);
    doc.undo();
    let restored = doc.mindmap.edges.iter().find(|e| edge_ref.matches(e)).unwrap();
    assert_eq!(
        restored.portal_from, before.portal_from,
        "undo must restore from-side border_t + perpendicular_offset"
    );
    assert_eq!(
        restored.portal_to, before.portal_to,
        "undo must leave to-side byte-equal to pre-reset snapshot"
    );
}

#[test]
fn test_reset_edge_position_portal_whole_edge_undo_restores_both_sides() {
    // Whole-edge reset: clearing both endpoints must produce
    // exactly one undo entry; undo must restore both sides
    // from that single snapshot, not just one.
    let mut doc = load_test_doc();
    let edge_ref = first_testament_edge_ref(&doc);
    doc.set_edge_display_mode(&edge_ref, "portal");
    let edge = doc.mindmap.edges.iter().find(|e| edge_ref.matches(e)).unwrap();
    let from_id = edge.from_id.clone();
    let to_id = edge.to_id.clone();
    doc.set_portal_label_border_t(&edge_ref, &from_id, Some(1.0));
    doc.set_portal_label_perpendicular_offset(&edge_ref, &from_id, Some(5.0));
    doc.set_portal_label_border_t(&edge_ref, &to_id, Some(2.0));
    doc.set_portal_label_perpendicular_offset(&edge_ref, &to_id, Some(-8.0));
    let before = doc
        .mindmap
        .edges
        .iter()
        .find(|e| edge_ref.matches(e))
        .unwrap()
        .clone();
    doc.undo_stack.clear();

    assert!(doc.reset_edge_position(&edge_ref, None));
    assert_eq!(doc.undo_stack.len(), 1, "one snapshot covers both sides");
    doc.undo();
    let restored = doc.mindmap.edges.iter().find(|e| edge_ref.matches(e)).unwrap();
    assert_eq!(restored.portal_from, before.portal_from);
    assert_eq!(restored.portal_to, before.portal_to);
}

#[test]
fn test_reset_edge_position_portal_preserves_non_position_endpoint_state() {
    // The reset clears only `border_t` and
    // `perpendicular_offset`. Sibling fields (color, text,
    // text_color, font clamps, zoom bounds) must survive
    // unchanged — verifies the "prune if all-default, else
    // preserve" contract on `PortalEndpointState`.
    use baumhard::mindmap::model::PortalEndpointState;
    let mut doc = load_test_doc();
    let edge_ref = first_testament_edge_ref(&doc);
    doc.set_edge_display_mode(&edge_ref, "portal");
    let edge = doc.mindmap.edges.iter().find(|e| edge_ref.matches(e)).unwrap();
    let from_id = edge.from_id.clone();
    let edge_idx = doc
        .mindmap
        .edges
        .iter()
        .position(|e| edge_ref.matches(e))
        .unwrap();
    doc.mindmap.edges[edge_idx].portal_from = Some(PortalEndpointState {
        color: Some("#ff0000".into()),
        border_t: Some(1.25),
        perpendicular_offset: Some(7.5),
        ..PortalEndpointState::default()
    });
    doc.undo_stack.clear();

    assert!(doc.reset_edge_position(&edge_ref, Some(&from_id)));
    let after = doc.mindmap.edges.iter().find(|e| edge_ref.matches(e)).unwrap();
    let pf = after.portal_from.as_ref().expect("color must keep slot alive");
    assert_eq!(pf.color.as_deref(), Some("#ff0000"));
    assert!(pf.border_t.is_none());
    assert!(pf.perpendicular_offset.is_none());
}

#[test]
fn test_reset_edge_position_already_default_is_noop() {
    // The no-op contract: if every targeted field already holds
    // its default, the reset reports false and does not push an
    // undo entry (no dead snapshots for idempotent console
    // invocations).
    let mut doc = load_test_doc();
    let edge_ref = first_testament_edge_ref(&doc);
    // First reset: may or may not mutate — the testament edge
    // could carry non-auto anchors. Run it and discard any
    // resulting undo entries so the second call starts clean.
    doc.reset_edge_position(&edge_ref, None);
    doc.undo_stack.clear();
    assert!(
        !doc.reset_edge_position(&edge_ref, None),
        "second reset must be a no-op"
    );
    assert!(doc.undo_stack.is_empty());
}

#[test]
fn test_curve_straight_edge_inserts_control_point() {
    // On a straight edge, `curve_straight_edge` inserts one CP
    // offset perpendicular to the anchor line. The resulting
    // control-point count is 1 (quadratic Bezier form).
    let mut doc = load_test_doc();
    let edge_idx = doc
        .mindmap
        .edges
        .iter()
        .position(|e| e.visible && e.control_points.is_empty())
        .unwrap();
    let edge_ref = EdgeRef::new(
        &doc.mindmap.edges[edge_idx].from_id,
        &doc.mindmap.edges[edge_idx].to_id,
        &doc.mindmap.edges[edge_idx].edge_type,
    );
    let ok = doc.curve_straight_edge(&edge_ref);
    assert!(ok, "curve on a straight edge should report success");
    assert_eq!(
        doc.mindmap.edges[edge_idx].control_points.len(),
        1,
        "a single CP bootstraps a quadratic Bezier"
    );
    assert!(doc.dirty);
    assert_eq!(doc.undo_stack.len(), 1);
}

#[test]
fn test_curve_straight_edge_returns_false_on_zero_length_edge() {
    // If from and to anchors collapse to the same point (e.g.
    // both nodes fully overlap and the anchors resolve to the
    // same side), `curve_straight_edge` has no meaningful
    // direction to push the CP and bails early.
    let mut doc = load_test_doc();
    let edge_idx = doc
        .mindmap
        .edges
        .iter()
        .position(|e| e.visible && e.control_points.is_empty())
        .unwrap();
    let edge_ref = EdgeRef::new(
        &doc.mindmap.edges[edge_idx].from_id,
        &doc.mindmap.edges[edge_idx].to_id,
        &doc.mindmap.edges[edge_idx].edge_type,
    );
    // Collapse both endpoint nodes into identical rectangles
    // AND force both anchors to the *same* side so the resolved
    // anchor points genuinely coincide (auto-anchor resolution
    // picks opposite sides by default and would defeat the
    // test).
    let from_id = doc.mindmap.edges[edge_idx].from_id.clone();
    let to_id = doc.mindmap.edges[edge_idx].to_id.clone();
    if let Some(from) = doc.mindmap.nodes.get_mut(&from_id) {
        from.position.x = 0.0;
        from.position.y = 0.0;
        from.size.width = 10.0;
        from.size.height = 10.0;
    }
    if let Some(to) = doc.mindmap.nodes.get_mut(&to_id) {
        to.position.x = 0.0;
        to.position.y = 0.0;
        to.size.width = 10.0;
        to.size.height = 10.0;
    }
    doc.mindmap.edges[edge_idx].anchor_from = "top".into();
    doc.mindmap.edges[edge_idx].anchor_to = "top".into();
    let ok = doc.curve_straight_edge(&edge_ref);
    assert!(!ok, "zero-length edge should return false");
    assert!(doc.mindmap.edges[edge_idx].control_points.is_empty());
    assert!(doc.undo_stack.is_empty());
}

#[test]
fn test_curve_straight_edge_missing_node_is_noop() {
    // Defensive branch: if an endpoint node vanished between
    // the last frame and this call, `curve_straight_edge`
    // bails cleanly rather than panicking.
    let mut doc = load_test_doc();
    let edge_idx = doc
        .mindmap
        .edges
        .iter()
        .position(|e| e.visible && e.control_points.is_empty())
        .unwrap();
    let er = EdgeRef::new(
        &doc.mindmap.edges[edge_idx].from_id,
        &doc.mindmap.edges[edge_idx].to_id,
        &doc.mindmap.edges[edge_idx].edge_type,
    );
    // Drop the from-node while keeping the edge record — the
    // defensive guard should return false.
    let from_id = doc.mindmap.edges[edge_idx].from_id.clone();
    doc.mindmap.nodes.remove(&from_id);
    let ok = doc.curve_straight_edge(&er);
    assert!(!ok);
    assert!(doc.undo_stack.is_empty());
}

#[test]
fn test_curve_straight_edge_noop_on_already_curved() {
    // Re-running `curve_straight_edge` on an already-curved edge
    // is a no-op — keeps the undo stack clean for console users
    // who repeat the command.
    let mut doc = load_test_doc();
    let edge_idx = doc.mindmap.edges.iter().position(|e| e.visible).unwrap();
    doc.mindmap.edges[edge_idx]
        .control_points
        .push(ControlPoint { x: 5.0, y: 5.0 });
    let edge_ref = EdgeRef::new(
        &doc.mindmap.edges[edge_idx].from_id,
        &doc.mindmap.edges[edge_idx].to_id,
        &doc.mindmap.edges[edge_idx].edge_type,
    );
    let ok = doc.curve_straight_edge(&edge_ref);
    assert!(!ok, "curve on already-curved edge should be a no-op");
    assert_eq!(doc.mindmap.edges[edge_idx].control_points.len(), 1);
}

#[test]
fn test_set_edge_anchor_pushes_undo() {
    let mut doc = load_test_doc();
    let edge_idx = doc.mindmap.edges.iter().position(|e| e.visible).unwrap();
    let edge_ref = EdgeRef::new(
        &doc.mindmap.edges[edge_idx].from_id,
        &doc.mindmap.edges[edge_idx].to_id,
        &doc.mindmap.edges[edge_idx].edge_type,
    );
    // Force a change by picking a value different from the current
    let original = doc.mindmap.edges[edge_idx].anchor_from.clone();
    let new_value = if original == "top" { "bottom" } else { "top" };
    let ok = doc.set_edge_anchor(&edge_ref, true, new_value);
    assert!(ok);
    assert_eq!(doc.mindmap.edges[edge_idx].anchor_from, new_value);
    assert_eq!(doc.undo_stack.len(), 1);
}

#[test]
fn test_set_edge_anchor_noop_when_already_set() {
    let mut doc = load_test_doc();
    let edge_idx = doc.mindmap.edges.iter().position(|e| e.visible).unwrap();
    let edge_ref = EdgeRef::new(
        &doc.mindmap.edges[edge_idx].from_id,
        &doc.mindmap.edges[edge_idx].to_id,
        &doc.mindmap.edges[edge_idx].edge_type,
    );
    let current = doc.mindmap.edges[edge_idx].anchor_from.clone();
    let ok = doc.set_edge_anchor(&edge_ref, true, &current);
    assert!(!ok);
    assert!(doc.undo_stack.is_empty());
}

#[test]
fn test_edit_edge_undo_restores_control_points() {
    let mut doc = load_test_doc();
    let edge_idx = doc.mindmap.edges.iter().position(|e| e.visible).unwrap();
    doc.mindmap.edges[edge_idx]
        .control_points
        .push(ControlPoint { x: 33.0, y: 44.0 });
    let edge_ref = EdgeRef::new(
        &doc.mindmap.edges[edge_idx].from_id,
        &doc.mindmap.edges[edge_idx].to_id,
        &doc.mindmap.edges[edge_idx].edge_type,
    );
    doc.reset_edge_to_straight(&edge_ref);
    assert!(doc.mindmap.edges[edge_idx].control_points.is_empty());
    assert!(doc.undo());
    assert_eq!(doc.mindmap.edges[edge_idx].control_points.len(), 1);
    assert_eq!(doc.mindmap.edges[edge_idx].control_points[0].x, 33.0);
}

#[test]
fn test_edit_edge_undo_restores_anchor() {
    let mut doc = load_test_doc();
    let edge_idx = doc.mindmap.edges.iter().position(|e| e.visible).unwrap();
    let original = doc.mindmap.edges[edge_idx].anchor_from.clone();
    let new_value = if original == "right" { "left" } else { "right" };
    let edge_ref = EdgeRef::new(
        &doc.mindmap.edges[edge_idx].from_id,
        &doc.mindmap.edges[edge_idx].to_id,
        &doc.mindmap.edges[edge_idx].edge_type,
    );
    doc.set_edge_anchor(&edge_ref, true, new_value);
    assert_eq!(doc.mindmap.edges[edge_idx].anchor_from, new_value);
    assert!(doc.undo());
    assert_eq!(doc.mindmap.edges[edge_idx].anchor_from, original);
}

#[test]
fn test_edge_index_finds_existing_edge() {
    let doc = load_test_doc();
    let (edge_ref, _) = pick_test_edge(&doc);
    let idx = doc.edge_index(&edge_ref);
    assert!(idx.is_some());
}

#[test]
fn test_edge_index_unknown_returns_none() {
    let doc = load_test_doc();
    let bogus = EdgeRef::new("nope", "nope2", "cross_link");
    assert!(doc.edge_index(&bogus).is_none());
}

// ========================================================================
// Connection style and label mutation tests
// ========================================================================

#[test]
fn test_ensure_glyph_connection_forks_from_hardcoded_default() {
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    // Make sure the test subject starts with no per-edge override
    // AND the canvas has no default — forces the hardcoded default
    // path.
    let idx = doc.edge_index(&er).unwrap();
    doc.mindmap.edges[idx].glyph_connection = None;
    doc.mindmap.canvas.default_connection = None;
    doc.undo_stack.clear();
    doc.dirty = false;

    // First style edit: changing the body glyph. The fork should
    // materialize a concrete GlyphConnectionConfig with the
    // hardcoded default body (·) — then the mutation overwrites
    // `body` with the requested value.
    let changed = doc.set_edge_body_glyph(&er, "\u{2500}");
    assert!(changed, "body change should succeed on fresh edge");
    let cfg = doc.mindmap.edges[idx]
        .glyph_connection
        .as_ref()
        .expect("fork should install a config");
    assert_eq!(cfg.body, "\u{2500}");
    // The other fields should match the hardcoded default.
    let hard = GlyphConnectionConfig::default();
    assert_eq!(cfg.font_size_pt, hard.font_size_pt);
    assert_eq!(cfg.min_font_size_pt, hard.min_font_size_pt);
}

#[test]
fn test_ensure_glyph_connection_forks_from_canvas_default() {
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    let idx = doc.edge_index(&er).unwrap();
    doc.mindmap.edges[idx].glyph_connection = None;
    // Set a canvas-level default with a distinctive body glyph.
    doc.mindmap.canvas.default_connection = Some(GlyphConnectionConfig {
        body: "\u{22EF}".to_string(), // ⋯
        ..GlyphConnectionConfig::default()
    });
    doc.undo_stack.clear();
    doc.dirty = false;

    // Change a different field (spacing) so the fork copies the
    // canvas body (⋯) into the edge before the field overwrite.
    let changed = doc.set_edge_spacing(&er, 6.0);
    assert!(changed);
    let cfg = doc.mindmap.edges[idx]
        .glyph_connection
        .as_ref()
        .expect("fork should install a config");
    // Body was copied from the canvas default, not from the
    // hardcoded default.
    assert_eq!(cfg.body, "\u{22EF}");
    assert_eq!(cfg.spacing, 6.0);
}

/// Glyph-wheel color picker invariant: setting
/// `doc.color_picker_preview` never pushes an undo entry and
/// never flips dirty. Mirrors what the picker hover path does
/// after the Step C refactor, which moved preview from model-
/// mutation (`preview_edge_color`) to a transient scene-level
/// substitution via the document field.
#[test]
fn test_color_picker_preview_does_not_push_undo_or_dirty() {
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    let idx = doc.edge_index(&er).unwrap();
    let stack_depth = doc.undo_stack.len();
    let before = doc.mindmap.edges[idx].clone();
    doc.dirty = false;

    let key = baumhard::mindmap::scene_cache::EdgeKey::from_edge(&doc.mindmap.edges[idx]);
    doc.color_picker_preview = Some(ColorPickerPreview {
        key,
        color: "#abcdef".to_string(),
    });

    // Model is byte-identical to the pre-preview state.
    assert_eq!(doc.mindmap.edges[idx], before);
    assert_eq!(doc.undo_stack.len(), stack_depth);
    assert!(!doc.dirty);

    // And the scene builder substitutes the preview color into
    // the matching edge's label element.
    doc.selection = SelectionState::Edge(er.clone());
    let scene = crate::application::document::tests_common::project_roles(
        &doc,
        1.0,
        InteractionModeOverrides::none(),
    );
    // The edge has a glyph label → the label pass should emit a
    // ConnectionLabelElement for it. If the edge has no label
    // this test case simply verifies nothing crashes.
    let edge_key = baumhard::mindmap::scene_cache::EdgeKey::from_edge(&doc.mindmap.edges[idx]);
    // Preview beats selection on the previewed edge → the
    // connection color (body glyphs) should be the preview hex,
    // not the selection cyan.
    if let Some(conn) = scene.connection_elements.iter().find(|c| c.edge_key == edge_key) {
        assert_eq!(
            conn.color, "#abcdef",
            "preview should beat selection override on the previewed edge"
        );
    }
}

/// `SelectionState::EdgeLabel` paints the cyan selection
/// highlight onto the label's `ConnectionLabelElement` color
/// — the visual signal that the click landed on the label and
/// that the next keystroke will type into it. Other selections
/// on the same edge (`Edge`) inherit the same tint by design;
/// non-edge selections leave the label at its committed color.
#[test]
fn test_edge_label_selection_paints_label_cyan() {
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    // Give the first edge a label so the scene builder emits a
    // ConnectionLabelElement for it.
    doc.set_edge_label(&er, Some("hello".into()));
    let edge_key = baumhard::mindmap::scene_cache::EdgeKey::new(
        er.from_id.as_str(),
        er.to_id.as_str(),
        er.edge_type.as_str(),
    );

    // Baseline: no selection, label takes the committed color
    // (whatever the cascade resolves to from the model).
    doc.selection = SelectionState::None;
    let baseline = crate::application::document::tests_common::project_roles(
        &doc,
        1.0,
        InteractionModeOverrides::none(),
    )
    .connection_label_elements
    .iter()
    .find(|c| c.edge_key == edge_key)
    .expect("baseline label element")
    .color
    .clone();
    assert_ne!(
        baseline.to_lowercase(),
        "#00e5ff",
        "baseline must not already be the highlight color"
    );

    // EdgeLabel selection: label tints cyan.
    doc.selection = SelectionState::EdgeLabel(EdgeLabelSel::new(er.clone()));
    let highlighted = crate::application::document::tests_common::project_roles(
        &doc,
        1.0,
        InteractionModeOverrides::none(),
    )
    .connection_label_elements
    .iter()
    .find(|c| c.edge_key == edge_key)
    .expect("highlighted label element")
    .color
    .clone();
    assert_eq!(highlighted.to_uppercase(), "#00E5FF");

    // Whole-edge selection: same tint applied to the label so
    // the user reads "selected" the same way regardless of
    // which sub-part the click landed on.
    doc.selection = SelectionState::Edge(er.clone());
    let edge_selected = crate::application::document::tests_common::project_roles(
        &doc,
        1.0,
        InteractionModeOverrides::none(),
    )
    .connection_label_elements
    .iter()
    .find(|c| c.edge_key == edge_key)
    .expect("edge-selected label element")
    .color
    .clone();
    assert_eq!(edge_selected.to_uppercase(), "#00E5FF");
}

/// Clearing `doc.color_picker_preview` returns scene output to
/// the pre-preview state without any model mutation.
#[test]
fn test_color_picker_preview_cleared_returns_to_committed() {
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    let idx = doc.edge_index(&er).unwrap();
    let committed_before = doc.mindmap.edges[idx].clone();

    let key = baumhard::mindmap::scene_cache::EdgeKey::from_edge(&doc.mindmap.edges[idx]);
    doc.color_picker_preview = Some(ColorPickerPreview {
        key,
        color: "#112233".to_string(),
    });
    // ... hover frames would re-run the projection here ...
    doc.color_picker_preview = None;

    // Model is untouched across the full preview session.
    assert_eq!(doc.mindmap.edges[idx], committed_before);
}

#[test]
fn test_set_edge_body_glyph_pushes_edit_edge_undo() {
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    let stack_depth = doc.undo_stack.len();
    let changed = doc.set_edge_body_glyph(&er, "\u{2550}");
    assert!(changed);
    assert_eq!(doc.undo_stack.len(), stack_depth + 1);
    assert!(matches!(doc.undo_stack.last(), Some(UndoAction::EditEdge { .. })));
    assert!(doc.dirty);
}

#[test]
fn test_undo_after_first_style_edit_restores_pre_fork_none() {
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    let idx = doc.edge_index(&er).unwrap();
    // Force the pre-edit state to None so we can verify the fork
    // is rolled back on undo.
    doc.mindmap.edges[idx].glyph_connection = None;
    doc.undo_stack.clear();

    assert!(doc.set_edge_body_glyph(&er, "\u{2500}"));
    assert!(doc.mindmap.edges[idx].glyph_connection.is_some());
    doc.undo();
    assert!(
        doc.mindmap.edges[idx].glyph_connection.is_none(),
        "undo should restore the pre-fork None"
    );
}

#[test]
fn test_set_edge_font_family_set_and_clear() {
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    // Set a family.
    assert!(doc.set_edge_font_family(&er, Some("Norse")));
    let idx = doc.edge_index(&er).unwrap();
    assert_eq!(
        doc.mindmap.edges[idx]
            .glyph_connection
            .as_ref()
            .and_then(|c| c.font.as_deref()),
        Some("Norse")
    );
    // Idempotent re-set is a no-op.
    let stack_len = doc.undo_stack.len();
    assert!(!doc.set_edge_font_family(&er, Some("Norse")));
    assert_eq!(doc.undo_stack.len(), stack_len);
    // Clearing the override removes the family.
    assert!(doc.set_edge_font_family(&er, None));
    assert_eq!(
        doc.mindmap.edges[idx]
            .glyph_connection
            .as_ref()
            .and_then(|c| c.font.as_deref()),
        None
    );
    // Undo restores the prior 'Norse' value.
    assert!(doc.undo());
    assert_eq!(
        doc.mindmap.edges[idx]
            .glyph_connection
            .as_ref()
            .and_then(|c| c.font.as_deref()),
        Some("Norse")
    );
}

#[test]
fn test_set_edge_font_family_unknown_edge_returns_false() {
    let mut doc = load_test_doc();
    let bogus = crate::application::document::EdgeRef::new("nope-from", "nope-to", "cross_link");
    doc.undo_stack.clear();
    doc.dirty = false;
    assert!(!doc.set_edge_font_family(&bogus, Some("Norse")));
    assert!(doc.undo_stack.is_empty());
    assert!(!doc.dirty);
}

#[test]
fn test_set_edge_color_none_clears_override() {
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    // First install a color override.
    assert!(doc.set_edge_color(&er, Some("#112233")));
    let idx = doc.edge_index(&er).unwrap();
    assert_eq!(
        doc.mindmap.edges[idx]
            .glyph_connection
            .as_ref()
            .and_then(|c| c.color.as_deref()),
        Some("#112233")
    );
    // Then clear it.
    assert!(doc.set_edge_color(&er, None));
    assert_eq!(
        doc.mindmap.edges[idx]
            .glyph_connection
            .as_ref()
            .and_then(|c| c.color.as_deref()),
        None
    );
}

#[test]
fn test_set_edge_font_size_step_clamps_at_min_and_max() {
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    // Force a known starting config.
    let idx = doc.edge_index(&er).unwrap();
    doc.mindmap.edges[idx].glyph_connection = Some(GlyphConnectionConfig {
        font_size_pt: 12.0,
        min_font_size_pt: 8.0,
        max_font_size_pt: 24.0,
        ..GlyphConnectionConfig::default()
    });
    doc.undo_stack.clear();

    // Step down past the min: should clamp, returning a smaller
    // but not less-than-min value. Repeatedly stepping down should
    // eventually pin at the min and return false on subsequent
    // attempts (no-op).
    for _ in 0..20 {
        doc.set_edge_font_size_step(&er, -2.0);
    }
    let pinned_low = doc.mindmap.edges[idx]
        .glyph_connection
        .as_ref()
        .unwrap()
        .font_size_pt;
    assert_eq!(pinned_low, 8.0);
    // Further steps down return false.
    assert!(!doc.set_edge_font_size_step(&er, -2.0));

    // Step up past the max: clamps to 24.
    for _ in 0..20 {
        doc.set_edge_font_size_step(&er, 2.0);
    }
    let pinned_high = doc.mindmap.edges[idx]
        .glyph_connection
        .as_ref()
        .unwrap()
        .font_size_pt;
    assert_eq!(pinned_high, 24.0);
    assert!(!doc.set_edge_font_size_step(&er, 2.0));
}

#[test]
fn test_set_edge_spacing_idempotent_noop() {
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    // First set succeeds.
    assert!(doc.set_edge_spacing(&er, 2.0));
    let stack_depth = doc.undo_stack.len();
    // Second set with the same value is a no-op; undo stack
    // doesn't grow.
    assert!(!doc.set_edge_spacing(&er, 2.0));
    assert_eq!(doc.undo_stack.len(), stack_depth);
}

#[test]
fn test_set_edge_label_round_trip() {
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    // Set a label.
    assert!(doc.set_edge_label(&er, Some("hello".to_string())));
    let idx = doc.edge_index(&er).unwrap();
    assert_eq!(doc.mindmap.edges[idx].label.as_deref(), Some("hello"));
    // Clear via Some("").
    assert!(doc.set_edge_label(&er, Some(String::new())));
    assert_eq!(doc.mindmap.edges[idx].label, None);
    // Setting the same None is a no-op.
    let depth = doc.undo_stack.len();
    assert!(!doc.set_edge_label(&er, None));
    assert_eq!(doc.undo_stack.len(), depth);
}

#[test]
fn test_set_edge_label_position_clamps_into_0_1() {
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    assert!(doc.set_edge_label_position(&er, -5.0));
    let idx = doc.edge_index(&er).unwrap();
    let pos = |d: &MindMapDocument, i: usize| {
        d.mindmap.edges[i]
            .label_config
            .as_ref()
            .and_then(|c| c.position_t)
    };
    assert_eq!(pos(&doc, idx), Some(0.0));

    assert!(doc.set_edge_label_position(&er, 42.0));
    assert_eq!(pos(&doc, idx), Some(1.0));

    assert!(doc.set_edge_label_position(&er, 0.75));
    assert_eq!(pos(&doc, idx), Some(0.75));
}

#[test]
fn test_set_edge_type_updates_selection_edge_ref() {
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    doc.selection = SelectionState::Edge(er.clone());
    let new_type = if er.edge_type == "parent_child" {
        "cross_link"
    } else {
        "parent_child"
    };
    assert!(doc.set_edge_type(&er, new_type));
    // Selection should now carry the new type.
    match &doc.selection {
        SelectionState::Edge(new_ref) => {
            assert_eq!(new_ref.edge_type, new_type);
            assert_eq!(new_ref.from_id, er.from_id);
            assert_eq!(new_ref.to_id, er.to_id);
        }
        _ => panic!("selection should still be an edge after type flip"),
    }
}

#[test]
fn test_set_edge_type_refuses_duplicate() {
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    // Seed a duplicate edge with the OPPOSITE type so conversion
    // would collide with it.
    let target_type = if er.edge_type == "parent_child" {
        "cross_link"
    } else {
        "parent_child"
    };
    let mut dup = doc.mindmap.edges[doc.edge_index(&er).unwrap()].clone();
    dup.edge_type = target_type.to_string();
    doc.mindmap.edges.push(dup);
    // Conversion should be refused.
    assert!(!doc.set_edge_type(&er, target_type));
    // Original edge is unchanged.
    assert_eq!(
        doc.mindmap.edges[doc.edge_index(&er).unwrap()].edge_type,
        er.edge_type
    );
}

#[test]
fn test_reset_edge_style_to_default_clears_glyph_connection() {
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    // Install an override.
    assert!(doc.set_edge_color(&er, Some("#00ff00")));
    let idx = doc.edge_index(&er).unwrap();
    assert!(doc.mindmap.edges[idx].glyph_connection.is_some());
    // Reset clears it.
    assert!(doc.reset_edge_style_to_default(&er));
    assert!(doc.mindmap.edges[idx].glyph_connection.is_none());
    // Repeat call is a no-op.
    assert!(!doc.reset_edge_style_to_default(&er));
}

#[test]
fn test_set_edge_cap_start_none_is_noop_when_already_none() {
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    // Force cap_start to None via a fresh config.
    let idx = doc.edge_index(&er).unwrap();
    doc.mindmap.edges[idx].glyph_connection = Some(GlyphConnectionConfig::default());
    doc.undo_stack.clear();
    // Setting cap_start to None when already None is a no-op.
    assert!(!doc.set_edge_cap_start(&er, None));
    // Undo stack didn't grow.
    assert_eq!(doc.undo_stack.len(), 0);
}

#[test]
fn test_set_portal_label_text_color_writes_and_clears() {
    // Write path for the new `text_color` channel: setter
    // writes into `PortalEndpointState.text_color` without
    // touching the icon `color`, and clearing rolls back an
    // all-default endpoint state so the undo snapshot stays
    // clean.
    use baumhard::mindmap::model::{is_portal_edge, portal_endpoint_state, DISPLAY_MODE_PORTAL};

    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    // Convert the first edge to portal mode so the setter has
    // a legal target (the setter itself doesn't require portal
    // mode, but it makes the scenario realistic).
    let idx = doc.edge_index(&er).unwrap();
    doc.mindmap.edges[idx].display_mode = Some(DISPLAY_MODE_PORTAL.to_string());
    assert!(is_portal_edge(&doc.mindmap.edges[idx]));
    let endpoint_id = doc.mindmap.edges[idx].from_id.clone();
    doc.undo_stack.clear();

    // Write: installs `text_color` on the from-endpoint.
    assert!(doc.set_portal_label_text_color(&er, &endpoint_id, Some("#11bb33")));
    let state = portal_endpoint_state(&doc.mindmap.edges[idx], &endpoint_id);
    assert_eq!(
        state.and_then(|s| s.text_color.as_deref()),
        Some("#11bb33"),
        "text_color should be set on the endpoint"
    );
    // Icon color on that same endpoint must NOT have been
    // touched — that's the whole point of the separate channel.
    assert_eq!(state.and_then(|s| s.color.as_deref()), None);
    assert_eq!(doc.undo_stack.len(), 1);

    // Re-setting the same value is a no-op.
    assert!(!doc.set_portal_label_text_color(&er, &endpoint_id, Some("#11bb33")));
    assert_eq!(doc.undo_stack.len(), 1);

    // Clear: `None` removes the text_color override. Since
    // the endpoint state was otherwise default (only
    // text_color was set), the whole state should roll back
    // to `None`.
    assert!(doc.set_portal_label_text_color(&er, &endpoint_id, None));
    assert!(
        portal_endpoint_state(&doc.mindmap.edges[idx], &endpoint_id).is_none(),
        "clearing the sole override should roll back to no endpoint state"
    );
}

#[test]
fn test_set_edge_font_atomic_ordering_applies_max_before_size() {
    // User asks size=14 max=10 atomically — the setter
    // applies max first, then clamps size against the new
    // bound. Confirms the documented ordering; a naive
    // size-then-max dispatch would leave the struct with
    // size=14, max=10 (bug).
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    let ok = doc.set_edge_font(&er, Some(14.0), None, Some(10.0));
    assert!(ok);
    let idx = doc.edge_index(&er).unwrap();
    let cfg = doc.mindmap.edges[idx].glyph_connection.as_ref().unwrap();
    assert!((cfg.max_font_size_pt - 10.0).abs() < 1.0e-4);
    assert!(
        (cfg.font_size_pt - 10.0).abs() < 1.0e-4,
        "size should clamp to new max; got {}",
        cfg.font_size_pt
    );
}

#[test]
fn test_set_edge_label_font_leaves_edge_body_untouched() {
    // Label font triple writes `label_config.*`; the edge
    // body's `glyph_connection.*` must not move.
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    // Seed a known baseline.
    assert!(doc.set_edge_font(&er, Some(12.0), Some(8.0), Some(64.0)));
    let before = {
        let idx = doc.edge_index(&er).unwrap();
        let cfg = doc.mindmap.edges[idx].glyph_connection.as_ref().unwrap();
        (cfg.font_size_pt, cfg.min_font_size_pt, cfg.max_font_size_pt)
    };
    assert!(doc.set_edge_label_font(&er, Some(18.0), Some(10.0), Some(30.0)));
    let idx = doc.edge_index(&er).unwrap();
    let body = doc.mindmap.edges[idx].glyph_connection.as_ref().unwrap();
    assert_eq!(
        (body.font_size_pt, body.min_font_size_pt, body.max_font_size_pt),
        before,
        "edge body font must be unchanged by a label-only write"
    );
    let label = doc.mindmap.edges[idx].label_config.as_ref().unwrap();
    assert_eq!(label.font_size_pt, Some(18.0));
    assert_eq!(label.min_font_size_pt, Some(10.0));
    assert_eq!(label.max_font_size_pt, Some(30.0));
}

#[test]
fn test_set_edge_label_font_size_clamps_into_inherited_body_bounds() {
    // Label sets only `size`, no own clamps — the clamp must
    // fall back to the edge's `glyph_connection` bounds. Here
    // the body max is 20, so a label size of 100 clamps to 20.
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    assert!(doc.set_edge_font(&er, Some(12.0), Some(8.0), Some(20.0)));
    assert!(doc.set_edge_label_font(&er, Some(100.0), None, None));
    let idx = doc.edge_index(&er).unwrap();
    let label = doc.mindmap.edges[idx].label_config.as_ref().unwrap();
    assert_eq!(
        label.font_size_pt,
        Some(20.0),
        "label size should clamp into body max when label carries no own clamps"
    );
}

#[test]
fn test_set_portal_text_font_writes_endpoint_state_only() {
    // Portal-text triple writes endpoint_state.text_*; the
    // icon's own font size (inherited from glyph_connection)
    // must not move.
    use baumhard::mindmap::model::{portal_endpoint_state, DISPLAY_MODE_PORTAL};
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    let idx = doc.edge_index(&er).unwrap();
    doc.mindmap.edges[idx].display_mode = Some(DISPLAY_MODE_PORTAL.to_string());
    assert!(doc.set_edge_font(&er, Some(30.0), Some(20.0), Some(60.0)));
    let endpoint_id = doc.mindmap.edges[idx].from_id.clone();
    assert!(doc.set_portal_text_font(&er, &endpoint_id, Some(14.0), Some(10.0), Some(48.0)));
    let edge = &doc.mindmap.edges[idx];
    let body = edge.glyph_connection.as_ref().unwrap();
    assert!((body.font_size_pt - 30.0).abs() < 1.0e-4);
    let state = portal_endpoint_state(edge, &endpoint_id).expect("endpoint state");
    assert_eq!(state.text_font_size_pt, Some(14.0));
    assert_eq!(state.text_min_font_size_pt, Some(10.0));
    assert_eq!(state.text_max_font_size_pt, Some(48.0));
}

#[test]
fn test_set_edge_font_rejects_non_positive_and_nan() {
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    // NaN and non-positive values are filtered silently (no
    // change, no undo entry); caller's responsibility to
    // validate at its own layer.
    doc.undo_stack.clear();
    assert!(!doc.set_edge_font(&er, Some(f32::NAN), None, None));
    assert!(!doc.set_edge_font(&er, Some(-5.0), None, None));
    assert!(!doc.set_edge_font(&er, Some(f32::INFINITY), None, None));
    assert!(doc.undo_stack.is_empty());
}

#[test]
fn test_set_edge_label_perpendicular_offset_writes_and_clears() {
    // Setter writes the signed offset into
    // `label_config.perpendicular_offset`; passing `None`
    // clears it and rolls back an all-default `EdgeLabelConfig`
    // so the edge goes back to no label config at all.
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    // Write a negative offset.
    assert!(doc.set_edge_label_perpendicular_offset(&er, Some(-12.5)));
    let idx = doc.edge_index(&er).unwrap();
    assert_eq!(
        doc.mindmap.edges[idx]
            .label_config
            .as_ref()
            .and_then(|c| c.perpendicular_offset),
        Some(-12.5)
    );
    // Re-setting the same value is a no-op.
    let depth = doc.undo_stack.len();
    assert!(!doc.set_edge_label_perpendicular_offset(&er, Some(-12.5)));
    assert_eq!(doc.undo_stack.len(), depth);
    // Clear it back — no other fields were set, so
    // `label_config` itself should become None.
    assert!(doc.set_edge_label_perpendicular_offset(&er, None));
    assert!(doc.mindmap.edges[idx].label_config.is_none());
}

#[test]
fn test_set_edge_label_perpendicular_offset_rejects_non_finite() {
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    doc.undo_stack.clear();
    assert!(!doc.set_edge_label_perpendicular_offset(&er, Some(f32::NAN)));
    assert!(!doc.set_edge_label_perpendicular_offset(&er, Some(f32::INFINITY)));
    assert!(doc.undo_stack.is_empty());
}

#[test]
fn test_set_edge_font_rejects_inverted_min_max_explicit() {
    // Explicit inverted bounds — `min=20 max=10` — would panic
    // `f32::clamp` on the next render frame. Setter refuses
    // to land the pair.
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    doc.undo_stack.clear();
    assert!(!doc.set_edge_font(&er, Some(14.0), Some(20.0), Some(10.0)));
    assert!(doc.undo_stack.is_empty(), "no undo entry for rejected triple");
}

#[test]
fn test_set_edge_font_rejects_inverted_against_existing_max() {
    // `min=20` alone with `max` already below 20. The
    // incoming min inverts against the struct's existing max.
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    // Seed a baseline with max=10.
    assert!(doc.set_edge_font(&er, Some(8.0), Some(5.0), Some(10.0)));
    doc.undo_stack.clear();
    // Now try to set min=20: resolved (20, 10) is inverted.
    assert!(!doc.set_edge_font(&er, None, Some(20.0), None));
    assert!(doc.undo_stack.is_empty());
}

#[test]
fn test_set_edge_label_font_rejects_inverted_min_max() {
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    doc.undo_stack.clear();
    assert!(!doc.set_edge_label_font(&er, Some(14.0), Some(30.0), Some(20.0)));
    assert!(doc.undo_stack.is_empty());
}

#[test]
fn test_set_portal_text_font_rejects_inverted_min_max() {
    use baumhard::mindmap::model::DISPLAY_MODE_PORTAL;
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    let idx = doc.edge_index(&er).unwrap();
    doc.mindmap.edges[idx].display_mode = Some(DISPLAY_MODE_PORTAL.to_string());
    let endpoint = doc.mindmap.edges[idx].from_id.clone();
    doc.undo_stack.clear();
    assert!(!doc.set_portal_text_font(&er, &endpoint, Some(14.0), Some(30.0), Some(20.0)));
    assert!(doc.undo_stack.is_empty());
}

#[test]
fn test_set_portal_text_font_does_not_clear_other_endpoint_default_state() {
    // Regression for R2.2: the pre-fix scrub block scanned
    // both `portal_from` and `portal_to` and nuked either one
    // that happened to be in `PortalEndpointState::default()`
    // shape. A setter call that forks a fresh state on the
    // `from` side must not silently discard a pre-existing
    // default state on the `to` side.
    use baumhard::mindmap::model::{PortalEndpointState, DISPLAY_MODE_PORTAL};
    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);
    let idx = doc.edge_index(&er).unwrap();
    doc.mindmap.edges[idx].display_mode = Some(DISPLAY_MODE_PORTAL.to_string());
    // Seed the to-endpoint with a persistent default state —
    // this shape is rare in normal flows but could arise
    // through hand-edited JSON or a future code path that
    // installs default without fields.
    doc.mindmap.edges[idx].portal_to = Some(PortalEndpointState::default());
    let from_id = doc.mindmap.edges[idx].from_id.clone();
    // Set text font only on the from-endpoint.
    assert!(doc.set_portal_text_font(&er, &from_id, Some(14.0), None, None));
    // Post-call: from-endpoint got the text_font_size; to-
    // endpoint's pre-existing state is untouched.
    assert!(doc.mindmap.edges[idx].portal_from.is_some());
    assert_eq!(
        doc.mindmap.edges[idx].portal_to,
        Some(PortalEndpointState::default()),
        "unrelated endpoint's default state must survive"
    );
}

// ── Edge font setters through `mutate_edge` ────────────────────
//
// `set_edge_font`, `set_edge_label_font`, `set_portal_text_font`
// and `set_edge_font_family` used to re-implement the lookup →
// clone → mutate → rollback → push(EditEdge) → dirty scaffolding
// each in their own way, with three copies of the `final_min` /
// `final_max` inversion guard. They now route through
// `mutate_edge` over the shared `font_triple` core; these pin the
// contract at each of the four public entry points.

/// The no-op contract for all four font setters: an unchanged
/// value pushes nothing, dirties nothing, and — critically — does
/// not leave a forked `glyph_connection` / `label_config` /
/// `PortalEndpointState` behind. `mutate_edge`'s rollback is what
/// guarantees the last part; before the fold, each setter
/// hand-rolled it.
#[test]
fn test_edge_font_setters_no_op_leaves_no_forked_config() {
    let (mut doc, er) = super::tests_common::doc_with_one_edge();
    doc.undo_stack.clear();
    doc.dirty = false;
    assert!(doc.mindmap.edges[0].glyph_connection.is_none());
    assert!(doc.mindmap.edges[0].label_config.is_none());

    // All-`None` request: nothing to apply on any channel.
    assert!(!doc.set_edge_font(&er, None, None, None));
    assert!(!doc.set_edge_label_font(&er, None, None, None));
    assert!(!doc.set_portal_text_font(&er, "0", None, None, None));
    // Clearing a family that was never set.
    assert!(!doc.set_edge_font_family(&er, None));

    assert!(
        doc.mindmap.edges[0].glyph_connection.is_none(),
        "a no-op font edit must not fork glyph_connection"
    );
    assert!(
        doc.mindmap.edges[0].label_config.is_none(),
        "a no-op label-font edit must not fork label_config"
    );
    assert!(
        doc.mindmap.edges[0].portal_from.is_none() && doc.mindmap.edges[0].portal_to.is_none(),
        "a no-op portal-text edit must not fork endpoint state"
    );
    assert!(doc.undo_stack.is_empty(), "no-op font edits pushed an undo entry");
    assert!(!doc.dirty, "no-op font edits dirtied the document");
}

/// Re-applying the value a channel already carries is a no-op on
/// every channel — the second half of the idempotency contract,
/// with the config already forked.
#[test]
fn test_edge_font_setters_no_op_when_value_already_set() {
    let (mut doc, er) = super::tests_common::doc_with_one_edge();
    assert!(doc.set_edge_font(&er, Some(20.0), Some(6.0), Some(60.0)));
    assert!(doc.set_edge_label_font(&er, Some(18.0), Some(5.0), Some(50.0)));
    assert!(doc.set_edge_font_family(&er, Some("Norse")));
    doc.undo_stack.clear();
    doc.dirty = false;

    assert!(!doc.set_edge_font(&er, Some(20.0), Some(6.0), Some(60.0)));
    assert!(!doc.set_edge_label_font(&er, Some(18.0), Some(5.0), Some(50.0)));
    assert!(!doc.set_edge_font_family(&er, Some("Norse")));
    assert!(doc.undo_stack.is_empty());
    assert!(!doc.dirty);
}

/// The atomic-ordering contract at the public API: `min` / `max`
/// land before the clamped `size`, so `size=14 max=10` resolves
/// to `size=10, max=10` on every channel rather than the wrong
/// `size=14`. Previously three separate implementations of this.
#[test]
fn test_edge_font_setters_clamp_size_against_the_new_bounds() {
    let (mut doc, er) = super::tests_common::doc_with_one_edge();

    assert!(doc.set_edge_font(&er, Some(14.0), None, Some(10.0)));
    let cfg = doc.mindmap.edges[0]
        .glyph_connection
        .as_ref()
        .expect("body config forked");
    assert_eq!(cfg.font_size_pt, 10.0);
    assert_eq!(cfg.max_font_size_pt, 10.0);

    assert!(doc.set_edge_label_font(&er, Some(14.0), None, Some(9.0)));
    let label = doc.mindmap.edges[0]
        .label_config
        .as_ref()
        .expect("label config forked");
    assert_eq!(label.font_size_pt, Some(9.0));
    assert_eq!(label.max_font_size_pt, Some(9.0));

    assert!(doc.set_portal_text_font(&er, "0", Some(14.0), None, Some(8.0)));
    let state = doc.mindmap.edges[0]
        .portal_from
        .as_ref()
        .expect("endpoint state forked");
    assert_eq!(state.text_font_size_pt, Some(8.0));
    assert_eq!(state.text_max_font_size_pt, Some(8.0));
}

/// An inverted `(min, max)` request is rejected on every channel
/// without mutating — the guard that keeps `f32::clamp` from
/// panicking here and in the next renderer frame. One
/// implementation now, three call sites.
#[test]
fn test_edge_font_setters_reject_inverted_bounds() {
    let (mut doc, er) = super::tests_common::doc_with_one_edge();
    doc.undo_stack.clear();
    doc.dirty = false;

    assert!(!doc.set_edge_font(&er, None, Some(60.0), Some(10.0)));
    assert!(!doc.set_edge_label_font(&er, None, Some(60.0), Some(10.0)));
    assert!(!doc.set_portal_text_font(&er, "0", None, Some(60.0), Some(10.0)));

    assert!(doc.mindmap.edges[0].glyph_connection.is_none());
    assert!(doc.mindmap.edges[0].label_config.is_none());
    assert!(doc.mindmap.edges[0].portal_from.is_none());
    assert!(doc.undo_stack.is_empty());
    assert!(!doc.dirty);
}

/// Inversion is checked against the clamps a channel *inherits*,
/// not just the ones it authors: a label that sets only `min`
/// must still be ordered against the body's `max`, or the
/// renderer clamps an inverted pair.
#[test]
fn test_edge_label_font_rejects_inversion_against_inherited_body_max() {
    let (mut doc, er) = super::tests_common::doc_with_one_edge();
    // Pin a tight body clamp for the label to inherit.
    assert!(doc.set_edge_font(&er, Some(12.0), Some(8.0), Some(20.0)));
    doc.undo_stack.clear();
    doc.dirty = false;
    assert!(
        !doc.set_edge_label_font(&er, None, Some(50.0), None),
        "label min=50 against an inherited body max=20 must be rejected"
    );
    assert!(doc.mindmap.edges[0].label_config.is_none());
    assert!(doc.undo_stack.is_empty());
    assert!(!doc.dirty);
}

/// One labeled edge setter in a table-driven battery — the
/// `EdgeRef` sibling of `tests_nodes::NodeSetterCase`, aliased
/// for the same `clippy::type_complexity` reason.
type EdgeSetterCase = (&'static str, Box<dyn Fn(&mut MindMapDocument, &EdgeRef) -> bool>);

/// A real font edit on any channel pushes exactly one `EditEdge`
/// entry, and one `undo()` restores the whole edge — including
/// un-forking a config the edit created.
#[test]
fn test_edge_font_setters_round_trip_through_undo() {
    let cases: Vec<EdgeSetterCase> = vec![
        (
            "set_edge_font",
            Box::new(|doc: &mut MindMapDocument, er: &EdgeRef| doc.set_edge_font(er, Some(21.0), None, None)),
        ),
        (
            "set_edge_label_font",
            Box::new(|doc: &mut MindMapDocument, er: &EdgeRef| {
                doc.set_edge_label_font(er, Some(19.0), None, None)
            }),
        ),
        (
            "set_portal_text_font",
            Box::new(|doc: &mut MindMapDocument, er: &EdgeRef| {
                doc.set_portal_text_font(er, "0", Some(17.0), None, None)
            }),
        ),
        (
            "set_edge_font_family",
            Box::new(|doc: &mut MindMapDocument, er: &EdgeRef| doc.set_edge_font_family(er, Some("Norse"))),
        ),
    ];

    for (label, call) in cases {
        let (mut doc, er) = super::tests_common::doc_with_one_edge();
        doc.undo_stack.clear();
        let before = doc.mindmap.edges[0].clone();

        assert!(call(&mut doc, &er), "{label}: expected a real change");
        assert_eq!(doc.undo_stack.len(), 1, "{label}: expected one undo entry");
        assert!(matches!(doc.undo_stack[0], UndoAction::EditEdge { .. }));
        assert!(doc.dirty, "{label}: a real change must dirty the document");

        assert!(doc.undo(), "{label}: undo must succeed");
        assert_eq!(
            doc.mindmap.edges[0], before,
            "{label}: undo did not restore the pre-edit edge"
        );
    }
}

/// A bogus endpoint id is a no-op, not a panic, and forks
/// nothing. `mutate_edge`'s rollback covers the clone the old
/// hand-rolled version took pains to avoid.
#[test]
fn test_set_portal_text_font_rejects_stranger_endpoint_id() {
    let (mut doc, er) = super::tests_common::doc_with_one_edge();
    doc.undo_stack.clear();
    doc.dirty = false;
    assert!(!doc.set_portal_text_font(&er, "not_an_endpoint", Some(20.0), None, None));
    assert!(doc.mindmap.edges[0].portal_from.is_none());
    assert!(doc.mindmap.edges[0].portal_to.is_none());
    assert!(doc.undo_stack.is_empty());
    assert!(!doc.dirty);
}

/// Missing edge: every font setter degrades to `false`.
#[test]
fn test_edge_font_setters_no_op_on_missing_edge() {
    let (mut doc, _er) = super::tests_common::doc_with_one_edge();
    let ghost = EdgeRef::new("nope-from", "nope-to", "cross_link");
    doc.undo_stack.clear();
    doc.dirty = false;
    assert!(!doc.set_edge_font(&ghost, Some(20.0), None, None));
    assert!(!doc.set_edge_label_font(&ghost, Some(20.0), None, None));
    assert!(!doc.set_portal_text_font(&ghost, "0", Some(20.0), None, None));
    assert!(!doc.set_edge_font_family(&ghost, Some("Norse")));
    assert!(doc.undo_stack.is_empty());
    assert!(!doc.dirty);
}

/// Forking is undoable: an edit that materializes a
/// `label_config` / `PortalEndpointState` on an edge that had
/// none must restore the pre-fork `None` on undo. `mutate_edge`
/// snapshots before the fork, which is what makes this hold
/// without a per-setter scrub.
#[test]
fn test_edge_font_setters_undo_restores_the_pre_fork_none() {
    let (mut doc, er) = super::tests_common::doc_with_one_edge();
    assert!(doc.mindmap.edges[0].label_config.is_none());
    assert!(doc.set_edge_label_font(&er, Some(18.0), None, None));
    assert!(doc.mindmap.edges[0].label_config.is_some());
    assert!(doc.undo());
    assert!(
        doc.mindmap.edges[0].label_config.is_none(),
        "undo must restore the pre-fork None on label_config"
    );

    let (mut doc, er) = super::tests_common::doc_with_one_edge();
    assert!(doc.mindmap.edges[0].portal_from.is_none());
    assert!(doc.set_portal_text_font(&er, "0", Some(18.0), None, None));
    assert!(doc.mindmap.edges[0].portal_from.is_some());
    assert!(
        doc.mindmap.edges[0].portal_to.is_none(),
        "the other endpoint must stay untouched"
    );
    assert!(doc.undo());
    assert!(
        doc.mindmap.edges[0].portal_from.is_none(),
        "undo must restore the pre-fork None on portal_from"
    );
}

/// The clipboard color resolvers read the same
/// `MindEdge::*_color` cascade the scene builder does. A label
/// override detaches the label; the portal text channel falls
/// through to the icon.
#[test]
fn test_resolve_color_readers_follow_the_model_cascade() {
    let (mut doc, er) = super::tests_common::doc_with_one_edge();
    let edge_color = doc.mindmap.edges[0].color.clone();
    assert_eq!(doc.resolve_edge_color(&er).as_deref(), Some(edge_color.as_str()));
    assert_eq!(
        doc.resolve_edge_label_color(&er).as_deref(),
        Some(edge_color.as_str()),
        "an un-overridden label follows the edge body"
    );

    assert!(doc.set_edge_color(&er, Some("#112233")));
    assert_eq!(doc.resolve_edge_color(&er).as_deref(), Some("#112233"));
    assert_eq!(
        doc.resolve_edge_label_color(&er).as_deref(),
        Some("#112233"),
        "the label still follows the body cascade"
    );

    assert!(doc.set_edge_label_color(&er, Some("#445566")));
    assert_eq!(doc.resolve_edge_label_color(&er).as_deref(), Some("#445566"));
    assert_eq!(
        doc.resolve_edge_color(&er).as_deref(),
        Some("#112233"),
        "the body is unaffected by a label override"
    );

    // Portal channels: icon override wins over the body, text
    // falls through to the icon until it has its own.
    assert!(doc.set_portal_label_color(&er, "0", Some("#778899")));
    assert_eq!(
        doc.resolve_portal_label_color(&er, "0").as_deref(),
        Some("#778899")
    );
    assert_eq!(
        doc.resolve_portal_text_color(&er, "0").as_deref(),
        Some("#778899"),
        "portal text with no override follows the icon"
    );
    assert!(doc.set_portal_label_text_color(&er, "0", Some("#aabbcc")));
    assert_eq!(
        doc.resolve_portal_text_color(&er, "0").as_deref(),
        Some("#aabbcc")
    );
    assert_eq!(
        doc.resolve_portal_label_color(&er, "0").as_deref(),
        Some("#778899"),
        "the icon is unaffected by a text override"
    );
}

/// **The edge font triple clamps, and it had no test.**
///
/// `resolve_font_triple` clamps `size`, `min` and `max` into the
/// shaper's domain rather than merely screening them for positivity,
/// because the loader rejects a connection font size outside it —
/// so an unclamped write would author an edge the editor then
/// refused to reopen. The console's `parse_finite_pt` accepts
/// `0.001` and `5000` alike, which is exactly how such a value
/// arrives.
///
/// This drives the three public setters that route through it and
/// round-trips the result through the real save and the real strict
/// load, because "in the domain" is only meaningful as "the loader
/// takes it".
#[test]
fn test_edge_font_triple_clamps_into_the_loaders_domain() {
    use baumhard::font::fonts::{MAX_FONT_SIZE_PT, MIN_FONT_SIZE_PT};

    let in_domain = |v: f32, what: &str| {
        assert!(
            (MIN_FONT_SIZE_PT..=MAX_FONT_SIZE_PT).contains(&v),
            "{what} must be clamped into {MIN_FONT_SIZE_PT}..={MAX_FONT_SIZE_PT}, got {v}"
        );
    };

    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);

    // Both ends of what the console will parse, on the body channel.
    doc.set_edge_font(&er, Some(1.0e6), Some(1.0e-6), Some(1.0e9));
    let idx = doc.edge_index(&er).unwrap();
    let cfg = doc.mindmap.edges[idx]
        .glyph_connection
        .as_ref()
        .expect("the setter authored an inline connection");
    in_domain(cfg.font_size_pt, "body font_size_pt");
    in_domain(cfg.min_font_size_pt, "body min_font_size_pt");
    in_domain(cfg.max_font_size_pt, "body max_font_size_pt");
    assert!(
        cfg.min_font_size_pt <= cfg.max_font_size_pt,
        "the clamped pair must stay ordered — an inverted window panics f32::clamp"
    );

    // The label and portal channels share the same resolver. The
    // portal channel is per-endpoint, so it needs the endpoint id.
    doc.set_edge_label_font(&er, Some(1.0e6), Some(1.0e-6), Some(1.0e9));
    let endpoint = er.from_id.clone();
    doc.set_portal_text_font(&er, &endpoint, Some(1.0e6), Some(1.0e-6), Some(1.0e9));

    // The only assertion that matters in the end: the map reopens.
    let dir = baumhard::util::test_temp::TempDir::new("edge-font-triple");
    let path = dir.join("fonts.mindmap.json");
    baumhard::mindmap::loader::save_to_file(&path, &doc.mindmap).expect("save must succeed");
    baumhard::mindmap::loader::load_from_file(&path).unwrap_or_else(|e| {
        panic!("the editor wrote an edge its own loader refuses — the lockout case: {e}")
    });
}

/// **A curve reset must not write a control point the loader
/// refuses.** `add_edge_curve` computes the control point from two
/// node centers plus a quarter of the distance between them, so two
/// positions that are each inside `MAX_CANVAS_COORD` can produce an
/// offset outside it — and the loader bounds control points at that
/// same ceiling. Unclamped, the verb reported success and the saved
/// map would not reopen.
#[test]
fn test_curve_reset_on_distant_nodes_still_reloads() {
    use baumhard::mindmap::model::{Position, Size};

    let mut doc = load_test_doc();
    let er = first_testament_edge_ref(&doc);

    // Both endpoints inside the loader's bound, as far apart as it
    // allows — the shape that made the midpoint-plus-quarter-length
    // arithmetic leave the domain.
    let coord = baumhard::mindmap::model::validate::MAX_CANVAS_COORD;
    for (id, sign) in [(er.from_id.clone(), -1.0), (er.to_id.clone(), 1.0)] {
        if let Some(n) = doc.mindmap.nodes.get_mut(&id) {
            n.position = Position {
                x: coord * sign,
                y: coord * sign,
            };
            n.size = Size {
                width: 100.0,
                height: 50.0,
            };
        }
    }

    doc.curve_straight_edge(&er);

    let dir = baumhard::util::test_temp::TempDir::new("curve-reset-reload");
    let path = dir.join("curved.mindmap.json");
    baumhard::mindmap::loader::save_to_file(&path, &doc.mindmap).expect("save must succeed");
    let reloaded = baumhard::mindmap::loader::load_from_file(&path).unwrap_or_else(|e| {
        panic!("the editor wrote a curve its own loader refuses — the lockout case: {e}")
    });

    for edge in &reloaded.edges {
        for cp in &edge.control_points {
            assert!(
                cp.x.abs() <= coord && cp.y.abs() <= coord,
                "control point ({}, {}) is outside the loader's bound",
                cp.x,
                cp.y
            );
        }
    }
}
