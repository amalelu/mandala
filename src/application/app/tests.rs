//! Double-click detection + already-editing guard tests. The
//! predicates under test (`is_double_click`, the guard
//! predicate embedded in the MouseInput handler) are pure
//! cursor / time math, so exercising them here keeps the
//! winit event loop out of the test scaffold.

use super::*;

// -----------------------------------------------------------------
// Double-click detection
// -----------------------------------------------------------------

#[test]
fn test_double_click_same_target_within_window_fires() {
    let prev = LastClick {
        time: 1000.0,
        screen_pos: (100.0, 100.0),
        hit: ClickHit::Node("node-a".to_string()),
    };
    assert!(is_double_click(
        &prev,
        1100.0,
        (101.0, 100.0),
        &ClickHit::Node("node-a".to_string()),
    ));
}

#[test]
fn test_double_click_different_targets_does_not_fire() {
    let prev = LastClick {
        time: 1000.0,
        screen_pos: (100.0, 100.0),
        hit: ClickHit::Node("node-a".to_string()),
    };
    assert!(!is_double_click(
        &prev,
        1100.0,
        (100.0, 100.0),
        &ClickHit::Node("node-b".to_string()),
    ));
}

#[test]
fn test_double_click_too_far_apart_does_not_fire() {
    let prev = LastClick {
        time: 1000.0,
        screen_pos: (100.0, 100.0),
        hit: ClickHit::Empty,
    };
    // Distance = sqrt(20² + 0²) = 20px → dist² = 400, threshold = 256.
    assert!(!is_double_click(&prev, 1100.0, (120.0, 100.0), &ClickHit::Empty));
}

#[test]
fn test_double_click_expired_does_not_fire() {
    let prev = LastClick {
        time: 1000.0,
        screen_pos: (100.0, 100.0),
        hit: ClickHit::Empty,
    };
    assert!(!is_double_click(&prev, 1500.0, (100.0, 100.0), &ClickHit::Empty));
}

#[test]
fn test_double_click_empty_space_both_misses_fires() {
    // Both clicks landed on no node — valid double-click for
    // the "create orphan" gesture.
    let prev = LastClick {
        time: 1000.0,
        screen_pos: (50.0, 50.0),
        hit: ClickHit::Empty,
    };
    assert!(is_double_click(&prev, 1150.0, (52.0, 51.0), &ClickHit::Empty));
}

#[test]
fn test_double_click_exact_boundary_does_not_fire() {
    // At exactly DOUBLE_CLICK_MS elapsed, should NOT fire (uses >= threshold).
    let prev = LastClick {
        time: 1000.0,
        screen_pos: (100.0, 100.0),
        hit: ClickHit::Empty,
    };
    assert!(!is_double_click(&prev, 1400.0, (100.0, 100.0), &ClickHit::Empty));
}

#[test]
fn test_double_click_just_under_boundary_fires() {
    let prev = LastClick {
        time: 1000.0,
        screen_pos: (100.0, 100.0),
        hit: ClickHit::Empty,
    };
    assert!(is_double_click(&prev, 1399.0, (100.0, 100.0), &ClickHit::Empty));
}

// -----------------------------------------------------------------
// "is_double_click + already_editing_same_target" guard semantics
// -----------------------------------------------------------------
//
// The bug report was: double-clicking inside an already-open
// editor on the same node silently discards the transient buffer
// because the Pressed path re-opens the editor, clobbering the
// in-progress buffer. The fix guards the dispatch with a check
// that re-opens are skipped if the editor is already on that
// target. We verify the guard predicate here; the actual event
// loop wiring is manually verified via `cargo run`.

#[test]
fn test_double_click_guard_skips_same_target_when_editor_open() {
    let editor = TextEditState::Open {
        node_id: "node-A".to_string(),
        buffer: "in progress".to_string(),
        cursor_grapheme_pos: 11,
        buffer_regions: baumhard::core::primitives::ColorFontRegions::new_empty(),
        original_text: String::new(),
        original_regions: baumhard::core::primitives::ColorFontRegions::new_empty(),
    };
    let hit = Some("node-A".to_string());
    let already_editing = editor
        .node_id()
        .map(|id| hit.as_deref() == Some(id))
        .unwrap_or(false);
    assert!(already_editing, "guard must fire for same target");
}

#[test]
fn test_double_click_guard_allows_different_target_when_editor_open() {
    let editor = TextEditState::Open {
        node_id: "node-A".to_string(),
        buffer: "in progress".to_string(),
        cursor_grapheme_pos: 11,
        buffer_regions: baumhard::core::primitives::ColorFontRegions::new_empty(),
        original_text: String::new(),
        original_regions: baumhard::core::primitives::ColorFontRegions::new_empty(),
    };
    let hit = Some("node-B".to_string());
    let already_editing = editor
        .node_id()
        .map(|id| hit.as_deref() == Some(id))
        .unwrap_or(false);
    assert!(!already_editing, "guard must NOT fire for different target");
}

#[test]
fn test_double_click_guard_allows_when_editor_closed() {
    let editor = TextEditState::Closed;
    let hit = Some("node-A".to_string());
    let already_editing = editor
        .node_id()
        .map(|id| hit.as_deref() == Some(id))
        .unwrap_or(false);
    assert!(!already_editing, "guard must NOT fire when editor is closed");
}

// -----------------------------------------------------------------
// Drag-helper + release-flush invariants
//
// The `DraggingPortalLabel` / `DraggingEdgeLabel` drain path stores
// the latest cursor on the drag variant and drains once per frame.
// Release must unconditionally flush any `pending_cursor` so the
// drop position lands on the model even if the throttle skipped the
// final `CursorMoved`. These tests lock in the invariants the apply
// helpers depend on for that pattern to be correct.
// -----------------------------------------------------------------

#[cfg(test)]
mod drag_helper_tests {
    use super::super::edge_label_drag::apply_edge_label_drag;
    use super::super::portal_label_drag::apply_portal_label_drag;
    use crate::application::document::{EdgeRef, MindMapDocument};
    use baumhard::mindmap::model::{
        MindEdge, MindNode, NodeLayout, NodeStyle, Position, Size, TextRun,
        DISPLAY_MODE_PORTAL,
    };
    use glam::Vec2;

    const FROM_ID: &str = "node-a";
    const TO_ID: &str = "node-b";
    const EDGE_TYPE: &str = "cross_link";

    fn fixture_node(id: &str, x: f64, y: f64) -> MindNode {
        MindNode {
            id: id.to_string(),
            parent_id: None,
            position: Position { x, y },
            size: Size { width: 100.0, height: 60.0 },
            text: "n".to_string(),
            text_runs: vec![TextRun {
                start: 0,
                end: 1,
                bold: false,
                italic: false,
                underline: false,
                font: "LiberationSans".to_string(),
                size_pt: 24,
                color: "#ffffff".to_string(),
                hyperlink: None,
            }],
            style: NodeStyle {
                background_color: "#141414".to_string(),
                frame_color: "#30b082".to_string(),
                text_color: "#ffffff".to_string(),
                shape: "rectangle".to_string(),
                corner_radius_percent: 10.0,
                frame_thickness: 4.0,
                show_frame: true,
                show_shadow: false,
                border: None,
            },
            layout: NodeLayout {
                layout_type: "map".to_string(),
                direction: "auto".to_string(),
                spacing: 50.0,
            },
            folded: false,
            notes: String::new(),
            color_schema: None,
            channel: 0,
            trigger_bindings: Vec::new(),
            inline_mutations: Vec::new(),
            min_zoom_to_render: None,
            max_zoom_to_render: None,
        }
    }

    fn fixture_edge(portal: bool) -> MindEdge {
        MindEdge {
            from_id: FROM_ID.to_string(),
            to_id: TO_ID.to_string(),
            edge_type: EDGE_TYPE.to_string(),
            color: "#aa88cc".to_string(),
            width: 3,
            line_style: "solid".to_string(),
            visible: true,
            label: None,
            label_config: None,
            anchor_from: "auto".to_string(),
            anchor_to: "auto".to_string(),
            control_points: Vec::new(),
            glyph_connection: None,
            display_mode: if portal {
                Some(DISPLAY_MODE_PORTAL.to_string())
            } else {
                None
            },
            portal_from: None,
            portal_to: None,
            min_zoom_to_render: None,
            max_zoom_to_render: None,
        }
    }

    fn portal_doc() -> MindMapDocument {
        let json = serde_json::json!({
            "version": "1.0",
            "name": "fixture",
            "canvas": {"background_color": "#000000"},
            "nodes": {
                FROM_ID: fixture_node(FROM_ID, 0.0, 0.0),
                TO_ID: fixture_node(TO_ID, 400.0, 0.0),
            },
            "edges": [fixture_edge(true)],
        })
        .to_string();
        MindMapDocument::from_json_str(&json, None)
            .expect("fixture JSON must parse")
    }

    fn line_doc() -> MindMapDocument {
        let json = serde_json::json!({
            "version": "1.0",
            "name": "fixture",
            "canvas": {"background_color": "#000000"},
            "nodes": {
                FROM_ID: fixture_node(FROM_ID, 0.0, 0.0),
                TO_ID: fixture_node(TO_ID, 400.0, 0.0),
            },
            "edges": [fixture_edge(false)],
        })
        .to_string();
        MindMapDocument::from_json_str(&json, None)
            .expect("fixture JSON must parse")
    }

    fn edge_ref() -> EdgeRef {
        EdgeRef::new(FROM_ID, TO_ID, EDGE_TYPE)
    }

    // Idempotency: the drain may safely call `apply_*_drag` once
    // per frame with the same cursor — a no-op write returns
    // `false` and leaves the model alone. Critical because the
    // release arm unconditionally calls `apply_*` one more time
    // even if the last drain already consumed that cursor.
    #[test]
    fn test_apply_portal_label_drag_idempotent_same_cursor() {
        let mut doc = portal_doc();
        let cursor = Vec2::new(50.0, -10.0);
        assert!(apply_portal_label_drag(&mut doc, &edge_ref(), FROM_ID, cursor),
            "first call must change the model");
        assert!(!apply_portal_label_drag(&mut doc, &edge_ref(), FROM_ID, cursor),
            "repeat call with same cursor must be a no-op");
    }

    #[test]
    fn test_apply_edge_label_drag_idempotent_same_cursor() {
        let mut doc = line_doc();
        let cursor = Vec2::new(200.0, 10.0);
        assert!(apply_edge_label_drag(&mut doc, &edge_ref(), cursor),
            "first call must change the model");
        assert!(!apply_edge_label_drag(&mut doc, &edge_ref(), cursor),
            "repeat call with same cursor must be a no-op");
    }

    // Absolute-cursor / last-wins semantics: the drain overwrites
    // `pending_cursor` on every `CursorMoved`, so intermediate
    // positions get discarded when the throttle skips frames.
    // This is only sound if the final state depends solely on the
    // latest cursor. Verify: apply(A) then apply(B) must produce
    // the same state as apply(B) from a fresh doc.
    #[test]
    fn test_apply_portal_label_drag_last_cursor_wins() {
        let edge_ref = edge_ref();
        let cursor_a = Vec2::new(50.0, -10.0);
        let cursor_b = Vec2::new(-10.0, 30.0);

        let mut doc_seq = portal_doc();
        apply_portal_label_drag(&mut doc_seq, &edge_ref, FROM_ID, cursor_a);
        apply_portal_label_drag(&mut doc_seq, &edge_ref, FROM_ID, cursor_b);

        let mut doc_direct = portal_doc();
        apply_portal_label_drag(&mut doc_direct, &edge_ref, FROM_ID, cursor_b);

        let t_seq = doc_seq.mindmap.edges[0]
            .portal_from.as_ref().and_then(|s| s.border_t);
        let t_direct = doc_direct.mindmap.edges[0]
            .portal_from.as_ref().and_then(|s| s.border_t);
        assert_eq!(t_seq, t_direct,
            "sequential A→B must equal direct B — intermediate cursors \
             dropped by the throttle must not affect final state");
    }

    #[test]
    fn test_apply_edge_label_drag_last_cursor_wins() {
        let edge_ref = edge_ref();
        let cursor_a = Vec2::new(200.0, 10.0);
        let cursor_b = Vec2::new(300.0, -20.0);

        let mut doc_seq = line_doc();
        apply_edge_label_drag(&mut doc_seq, &edge_ref, cursor_a);
        apply_edge_label_drag(&mut doc_seq, &edge_ref, cursor_b);

        let mut doc_direct = line_doc();
        apply_edge_label_drag(&mut doc_direct, &edge_ref, cursor_b);

        let seq = doc_seq.mindmap.edges[0].label_config.as_ref()
            .map(|c| (c.position_t, c.perpendicular_offset));
        let direct = doc_direct.mindmap.edges[0].label_config.as_ref()
            .map(|c| (c.position_t, c.perpendicular_offset));
        assert_eq!(seq, direct,
            "sequential A→B must equal direct B for edge-label drag");
    }

    // Release-flush invariant: simulates the release arm. The
    // last drain consumed cursor A (drain clears `pending_cursor`
    // to None), then cursor B arrived but the throttle skipped
    // that frame (pending_cursor = Some(B)), then the user
    // released. The release must apply B so the drop position
    // lands on B, not A. Verified by applying A, then B, and
    // asserting the final state reflects B.
    #[test]
    fn test_release_flush_applies_final_cursor_portal() {
        let mut doc = portal_doc();
        let edge_ref = edge_ref();
        // Frame 1: drain runs, applies A.
        apply_portal_label_drag(&mut doc, &edge_ref, FROM_ID, Vec2::new(50.0, -10.0));
        let t_after_a = doc.mindmap.edges[0]
            .portal_from.as_ref().and_then(|s| s.border_t);
        // Frame 2: throttle skips (drain not called); cursor
        // moves to B — in prod this writes `pending_cursor`
        // only, no model touch. Simulated by not calling apply.
        // Release: flush Some(B).
        apply_portal_label_drag(&mut doc, &edge_ref, FROM_ID, Vec2::new(-10.0, 30.0));
        let t_after_b = doc.mindmap.edges[0]
            .portal_from.as_ref().and_then(|s| s.border_t);
        assert!(t_after_a != t_after_b,
            "release flush must change state — otherwise the drop \
             position would silently snap back to the throttle's \
             last drained cursor");
    }

    #[test]
    fn test_release_flush_applies_final_cursor_edge_label() {
        let mut doc = line_doc();
        let edge_ref = edge_ref();
        apply_edge_label_drag(&mut doc, &edge_ref, Vec2::new(200.0, 10.0));
        let after_a = doc.mindmap.edges[0].label_config.as_ref()
            .map(|c| (c.position_t, c.perpendicular_offset));
        apply_edge_label_drag(&mut doc, &edge_ref, Vec2::new(300.0, -20.0));
        let after_b = doc.mindmap.edges[0].label_config.as_ref()
            .map(|c| (c.position_t, c.perpendicular_offset));
        assert!(after_a != after_b,
            "release flush must change state for edge-label drag");
    }
}
