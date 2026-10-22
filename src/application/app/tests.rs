// SPDX-License-Identifier: MPL-2.0

//! Double-click detection + already-editing guard tests. The
//! predicates under test ([`super::is_double_click`],
//! [`super::already_editing_same_target`], [`super::wheel_lines`]
//! and [`super::wheel_gesture`]) are pure cursor / time / scroll
//! math, so exercising them here keeps the winit event loop out of
//! the test scaffold.

use super::*;

// -----------------------------------------------------------------
// Double-click detection
// -----------------------------------------------------------------

#[test]
fn test_double_click_same_target_within_window_fires() {
    let prev = LastClick {
        time: 1000.0,
        screen_pos: (100.0, 100.0),
        hit: ClickHit::Node("node-a".to_string(), None),
    };
    assert!(is_double_click(
        &prev,
        1100.0,
        (101.0, 100.0),
        &ClickHit::Node("node-a".to_string(), None),
    ));
}

#[test]
fn test_double_click_different_targets_does_not_fire() {
    let prev = LastClick {
        time: 1000.0,
        screen_pos: (100.0, 100.0),
        hit: ClickHit::Node("node-a".to_string(), None),
    };
    assert!(!is_double_click(
        &prev,
        1100.0,
        (100.0, 100.0),
        &ClickHit::Node("node-b".to_string(), None),
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
//
// These tests were promised in this file from the day the guard
// landed and could not be written: the predicate was inline in
// `event_mouse_click.rs`'s handler body, reachable only with a
// live winit event loop. It is now
// `already_editing_same_target`, which both targets call, so the
// promise is finally fulfillable — and the cross-target part
// matters, because the two copies of this guard had *different*
// predicates (the browser's had no single-line term).

#[test]
fn test_guard_is_open_when_the_press_lands_on_the_edited_node() {
    assert!(already_editing_same_target(Some("n1"), Some("n1"), false));
}

/// The common productive case: an editor is open and the user
/// double-clicks a *different* node to move the edit there. The
/// guard must not fire, or that gesture stops working.
#[test]
fn test_guard_is_closed_for_a_press_on_a_different_node() {
    assert!(!already_editing_same_target(Some("n1"), Some("n2"), false));
}

/// Empty-canvas press while an editor is open. `hit_node` is
/// `None`, which must not be read as "matches".
#[test]
fn test_guard_is_closed_for_an_empty_canvas_press_while_editing() {
    assert!(!already_editing_same_target(Some("n1"), None, false));
}

/// No editor open at all. Pinned separately from the case above
/// because the two `None`s sit on opposite sides of the
/// comparison, and a predicate that confused them would report
/// "already editing" with nothing open — suppressing every
/// double-click on the map.
#[test]
fn test_guard_is_closed_when_no_editor_is_open() {
    assert!(!already_editing_same_target(None, Some("n1"), false));
    assert!(!already_editing_same_target(None, None, false));
}

/// The single-line term stands alone: the edge-label / portal-caption
/// editor is open on this press's target even though no *node*
/// editor is. Native computes this from `single_line_edit_state`;
/// the browser has no single-line editor and passes `false`.
#[test]
fn test_guard_is_open_on_the_single_line_term_alone() {
    assert!(already_editing_same_target(None, None, true));
    assert!(already_editing_same_target(None, Some("n1"), true));
}

/// Both terms true. Pinned so the disjunction cannot be narrowed to
/// a conjunction without a test going red — a `&&` here would leave
/// the single-line editor unguarded on every press that isn't also
/// on the edited node.
#[test]
fn test_guard_is_open_when_both_terms_hold() {
    assert!(already_editing_same_target(Some("n1"), Some("n1"), true));
}

/// The node term stands alone, with the single-line term false —
/// exactly the input shape the browser produces, so a predicate
/// that leaned on the single-line term would be dead on WASM.
/// Paired with the converse (single-line true, node term false) so
/// both disjuncts are shown to carry the result on their own.
#[test]
fn test_guard_each_term_carries_the_result_alone() {
    assert!(already_editing_same_target(Some("n1"), Some("n1"), false));
    assert!(already_editing_same_target(Some("n1"), Some("n2"), true));
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
    use crate::application::document::defaults::{default_cross_link_edge, default_orphan_node};
    use crate::application::document::{EdgeRef, MindMapDocument};
    use baumhard::mindmap::model::{MindEdge, MindNode, Size, DISPLAY_MODE_PORTAL};
    use glam::Vec2;

    const FROM_ID: &str = "node-a";
    const TO_ID: &str = "node-b";
    const EDGE_TYPE: &str = "cross_link";

    /// Tighter than the production `default_orphan_node`: 100×60 box,
    /// single-grapheme `"n"` text. The drag-projection math under test
    /// is a function of node geometry, so the size is load-bearing for
    /// the cursor coordinates the tests below pick.
    fn fixture_node(id: &str, x: f64, y: f64) -> MindNode {
        let mut n = default_orphan_node(id, Vec2::new(x as f32, y as f32));
        n.size = Size {
            width: 100.0,
            height: 60.0,
        };
        n.sections[0].text = "n".to_string();
        n.sections[0].text_runs[0].end = 1;
        n
    }

    /// Cross-link edge in line-mode (`portal=false`) or portal-mode
    /// (`portal=true`). The portal variant deliberately omits
    /// `glyph_connection` — `default_portal_edge` would supply one,
    /// but these tests want the bare display-mode flip without the
    /// glyph-marker baggage.
    fn fixture_edge(portal: bool) -> MindEdge {
        let mut e = default_cross_link_edge(FROM_ID, TO_ID);
        if portal {
            e.display_mode = Some(DISPLAY_MODE_PORTAL.to_string());
        }
        e
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
        MindMapDocument::from_json_str(&json, None).expect("fixture JSON must parse")
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
        MindMapDocument::from_json_str(&json, None).expect("fixture JSON must parse")
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
        assert!(
            apply_portal_label_drag(&mut doc, &edge_ref(), FROM_ID, cursor),
            "first call must change the model"
        );
        assert!(
            !apply_portal_label_drag(&mut doc, &edge_ref(), FROM_ID, cursor),
            "repeat call with same cursor must be a no-op"
        );
    }

    #[test]
    fn test_apply_edge_label_drag_idempotent_same_cursor() {
        let mut doc = line_doc();
        let cursor = Vec2::new(200.0, 10.0);
        assert!(
            apply_edge_label_drag(&mut doc, &edge_ref(), cursor),
            "first call must change the model"
        );
        assert!(
            !apply_edge_label_drag(&mut doc, &edge_ref(), cursor),
            "repeat call with same cursor must be a no-op"
        );
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
            .portal_from
            .as_ref()
            .and_then(|s| s.border_t);
        let t_direct = doc_direct.mindmap.edges[0]
            .portal_from
            .as_ref()
            .and_then(|s| s.border_t);
        assert_eq!(
            t_seq, t_direct,
            "sequential A→B must equal direct B — intermediate cursors \
             dropped by the throttle must not affect final state"
        );
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

        let seq = doc_seq.mindmap.edges[0]
            .label_config
            .as_ref()
            .map(|c| (c.position_t, c.perpendicular_offset));
        let direct = doc_direct.mindmap.edges[0]
            .label_config
            .as_ref()
            .map(|c| (c.position_t, c.perpendicular_offset));
        assert_eq!(
            seq, direct,
            "sequential A→B must equal direct B for edge-label drag"
        );
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
        let t_after_a = doc.mindmap.edges[0].portal_from.as_ref().and_then(|s| s.border_t);
        // Frame 2: throttle skips (drain not called); cursor
        // moves to B — in prod this writes `pending_cursor`
        // only, no model touch. Simulated by not calling apply.
        // Release: flush Some(B).
        apply_portal_label_drag(&mut doc, &edge_ref, FROM_ID, Vec2::new(-10.0, 30.0));
        let t_after_b = doc.mindmap.edges[0].portal_from.as_ref().and_then(|s| s.border_t);
        assert!(
            t_after_a != t_after_b,
            "release flush must change state — otherwise the drop \
             position would silently snap back to the throttle's \
             last drained cursor"
        );
    }

    #[test]
    fn test_release_flush_applies_final_cursor_edge_label() {
        let mut doc = line_doc();
        let edge_ref = edge_ref();
        apply_edge_label_drag(&mut doc, &edge_ref, Vec2::new(200.0, 10.0));
        let after_a = doc.mindmap.edges[0]
            .label_config
            .as_ref()
            .map(|c| (c.position_t, c.perpendicular_offset));
        apply_edge_label_drag(&mut doc, &edge_ref, Vec2::new(300.0, -20.0));
        let after_b = doc.mindmap.edges[0]
            .label_config
            .as_ref()
            .map(|c| (c.position_t, c.perpendicular_offset));
        assert!(
            after_a != after_b,
            "release flush must change state for edge-label drag"
        );
    }
}

// -----------------------------------------------------------------
// `click_hit_from_priority` — the pure ladder behind
// `compute_click_hit`. The cascade gating in `compute_click_hit`
// already guarantees lower-priority hits are `None` when a
// higher-priority one matches; these tests exercise the ladder
// directly to lock the priority contract regardless.
// -----------------------------------------------------------------

#[cfg(test)]
mod click_hit_priority_tests {
    use super::*;
    use baumhard::mindmap::scene_cache::EdgeKey;

    fn ek() -> EdgeKey {
        EdgeKey::new("a", "b", "cross_link")
    }

    #[test]
    fn click_hit_priority_node_wins_over_all_others() {
        let hit = click_hit_from_priority(
            &Some("node-x".to_string()),
            None,
            &Some((ek(), "n1".to_string())),
            &Some((ek(), "n2".to_string())),
            &Some(ek()),
        );
        assert_eq!(hit, ClickHit::Node("node-x".to_string(), None));
    }

    /// Section-aware double-click: a click on `Section(N, K)`
    /// produces `ClickHit::Node(N, Some(K))`. Two clicks on
    /// different sections of the same node compare unequal under
    /// `PartialEq` and therefore *don't* fire `is_double_click`,
    /// pinning the regression Tier-D introduced (the section idx
    /// was resolved by `compute_click_hit` but dropped before the
    /// double-click compare and the editor open).
    #[test]
    fn click_hit_priority_node_carries_section_idx() {
        let hit = click_hit_from_priority(&Some("node-x".to_string()), Some(2), &None, &None, &None);
        assert_eq!(hit, ClickHit::Node("node-x".to_string(), Some(2)));
    }

    #[test]
    fn click_hit_priority_portal_text_wins_over_icon_and_label() {
        let hit = click_hit_from_priority(
            &None,
            None,
            &Some((ek(), "n1".to_string())),
            &Some((ek(), "n2".to_string())),
            &Some(ek()),
        );
        if let ClickHit::PortalText { endpoint, .. } = hit {
            assert_eq!(endpoint, "n1");
        } else {
            panic!("expected PortalText, got {:?}", hit);
        }
    }

    #[test]
    fn click_hit_priority_portal_icon_wins_over_edge_label() {
        let hit = click_hit_from_priority(&None, None, &None, &Some((ek(), "n2".to_string())), &Some(ek()));
        assert!(matches!(hit, ClickHit::PortalMarker { .. }));
    }

    #[test]
    fn click_hit_priority_edge_label_wins_when_alone() {
        let hit = click_hit_from_priority(&None, None, &None, &None, &Some(ek()));
        assert!(matches!(hit, ClickHit::EdgeLabel(_)));
    }

    /// **The click path and the drag path resolve a node under a
    /// portal marker the same way**, because the press stores the
    /// portal hit for the drag promotion only when the shared click
    /// chain left `hit_node` empty.
    ///
    /// Fails when: the `hit_node.is_none()` gate in
    /// `portal_label_drag_capture` goes. The drag promotion in
    /// `event_cursor_moved.rs` lists portal-label above node, so
    /// without the gate a press on a marker over a node would select
    /// the node on a click and slide the marker on a drag — one
    /// press, two targets, decided by whether the hand moved.
    ///
    /// Control, on this function: with no node the same marker *is*
    /// captured, so "returns None" is not a capture that never
    /// fires. The other half of the rule — that a press on the
    /// portal's *text* reaches this function as `None` and so arms
    /// nothing — is not expressible in these arguments and is
    /// pinned against the real scene by
    /// `test_a_portal_text_press_arms_no_drag_while_the_icon_does`.
    #[test]
    fn test_portal_label_drag_capture_is_gated_by_the_same_node_hit_the_click_ladder_uses() {
        let icon = Some((ek(), "endpoint-1".to_string()));

        assert!(
            super::portal_label_drag_capture(&icon, Some("node-x")).is_none(),
            "a node under the marker wins the press, so no portal drag is armed"
        );
        // ...and the click ladder agrees on the same inputs.
        assert_eq!(
            click_hit_from_priority(&Some("node-x".to_string()), None, &None, &icon, &None),
            ClickHit::Node("node-x".to_string(), None),
            "the two paths must name the same target for one press"
        );

        let armed = super::portal_label_drag_capture(&icon, None);
        assert_eq!(
            armed, icon,
            "control: with no node hit the marker must arm the drag"
        );
    }

    /// **Only the icon is a drag affordance.** A press on a portal
    /// endpoint's *text* selects it and arms nothing; a press on the
    /// icon beside it arms the slide-along-the-border drag.
    ///
    /// The split is made in `compute_click_hit`, which turns one
    /// `portal_at` answer into `portal_text_hit` / `portal_icon_hit`
    /// by the sub-part the hit names, and only the icon half is
    /// handed to `portal_label_drag_capture`. So the claim is about
    /// that fan-out, not about the capture function — whose
    /// arguments cannot express it, since a text press reaches it as
    /// a plain `None` that any `None` would satisfy.
    ///
    /// Fails when: the two arms of that `match hit.part` are
    /// swapped, or the text arm starts filling the icon slot too —
    /// either way a press on the text arms a drag that slides a
    /// glyph the user pressed for its selection.
    ///
    /// Runs against the real portal tree (the same fixture
    /// `scene_host`'s own hit-test tests use) rather than a
    /// hand-built pair of `Option`s, because the sub-part is decided
    /// by which leaf the BVH descent lands on — a fabricated hit
    /// would be asserting on the fixture, not on the resolution.
    /// The control is the icon row: both parts of one endpoint are
    /// probed, so "no drag" cannot be a scene in which nothing is
    /// hit at all.
    #[test]
    fn test_a_portal_text_press_arms_no_drag_while_the_icon_does() {
        use crate::application::scene_host::tests::{
            portal_fixture_map, portal_leaf_probes, register_portals,
        };
        use crate::application::scene_host::AppScene;
        use baumhard::mindmap::tree_builder::PortalPart;

        let map = portal_fixture_map();
        let mut app = AppScene::new();
        register_portals(&mut app, &map);
        let probes = portal_leaf_probes(&app);

        let point_on = |part: PortalPart| {
            probes
                .iter()
                .find(|(_, hit)| hit.part == part)
                .unwrap_or_else(|| panic!("the portal fixture must carry a clickable {part:?} leaf"))
                .0
        };

        // No mindmap tree, so `hit_node` is empty on both presses —
        // the node gate is the sibling test's subject and is held
        // out of the way here.
        let on_text = super::compute_click_hit(point_on(PortalPart::Text), None, &mut app);
        assert!(
            matches!(on_text.click_hit, ClickHit::PortalText { .. }),
            "the text leaf must resolve as a text click: {:?}",
            on_text.click_hit
        );
        assert!(
            super::portal_label_drag_capture(&on_text.portal_icon_hit, on_text.hit_node.as_deref()).is_none(),
            "a press on a portal's text must arm no drag — sliding text is not a gesture"
        );

        let on_icon = super::compute_click_hit(point_on(PortalPart::Icon), None, &mut app);
        assert!(
            matches!(on_icon.click_hit, ClickHit::PortalMarker { .. }),
            "control: the icon leaf must resolve as a marker click: {:?}",
            on_icon.click_hit
        );
        assert_eq!(
            super::portal_label_drag_capture(&on_icon.portal_icon_hit, on_icon.hit_node.as_deref()),
            on_icon.portal_icon_hit,
            "control: the icon beside it must arm the drag, or this test proves only \
             that the fixture is unreachable"
        );
    }

    #[test]
    fn click_hit_priority_all_none_yields_empty() {
        let hit = click_hit_from_priority(&None, None, &None, &None, &None);
        assert_eq!(hit, ClickHit::Empty);
    }

    /// Same-node, different-section "double-click" must NOT fire.
    /// Pre-fix the section index was discarded inside `ClickHit::Node`,
    /// so two slow clicks on adjacent sections of the same node
    /// were collapsed into one double-click event by the
    /// `PartialEq` compare in `is_double_click`.
    #[test]
    fn double_click_different_section_of_same_node_does_not_fire() {
        let prev = LastClick {
            time: 1000.0,
            screen_pos: (100.0, 100.0),
            hit: ClickHit::Node("node-a".to_string(), Some(0)),
        };
        assert!(!is_double_click(
            &prev,
            1100.0,
            (101.0, 100.0),
            &ClickHit::Node("node-a".to_string(), Some(1)),
        ));
    }

    /// Same-section double-click must fire — section index
    /// equality is the genuine same-target signal.
    #[test]
    fn double_click_same_section_fires() {
        let prev = LastClick {
            time: 1000.0,
            screen_pos: (100.0, 100.0),
            hit: ClickHit::Node("node-a".to_string(), Some(1)),
        };
        assert!(is_double_click(
            &prev,
            1100.0,
            (101.0, 100.0),
            &ClickHit::Node("node-a".to_string(), Some(1)),
        ));
    }
}

// -----------------------------------------------------------------
// Wheel decomposition
//
// `wheel_lines` and `wheel_gesture` are the two halves of "what did
// the user's scroll mean". Before unification the arithmetic lived
// at both wheel handlers and the browser's half never reached the
// second step at all — it went straight to a hardcoded 1.1x zoom.
// These pin the shared bodies both targets now run.
// -----------------------------------------------------------------

mod wheel_tests {
    use super::super::{wheel_gesture, wheel_lines};
    use crate::application::keybinds::MouseGesture;
    use crate::application::platform::input::MouseScrollDelta;

    /// A notched wheel reports whole lines and they pass through
    /// untouched — no divisor is applied to this shape.
    #[test]
    fn test_wheel_lines_passes_line_deltas_through_unscaled() {
        assert_eq!(wheel_lines(MouseScrollDelta::LineDelta(0.0, 1.0)), 1.0);
        assert_eq!(wheel_lines(MouseScrollDelta::LineDelta(0.0, -3.0)), -3.0);
    }

    /// The horizontal component is discarded: this app zooms on
    /// vertical scroll only, and a trackpad's sideways drift must
    /// not be read as a zoom.
    #[test]
    fn test_wheel_lines_ignores_the_horizontal_component() {
        assert_eq!(wheel_lines(MouseScrollDelta::LineDelta(99.0, 1.0)), 1.0);
    }

    /// Pixel deltas divide by the app's 50-px-per-line convention.
    /// Pinned as an exact value so a change to the divisor is a
    /// visible test edit rather than a silent shift in trackpad
    /// sensitivity on one target.
    #[test]
    fn test_wheel_lines_converts_pixel_deltas_at_fifty_px_per_line() {
        let d = MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, 100.0));
        assert_eq!(wheel_lines(d), 2.0);
        let up = MouseScrollDelta::PixelDelta(winit::dpi::PhysicalPosition::new(0.0, -25.0));
        assert_eq!(wheel_lines(up), -0.5);
    }

    #[test]
    fn test_wheel_gesture_maps_positive_to_up_and_negative_to_down() {
        assert_eq!(wheel_gesture(1.0), MouseGesture::WheelUp);
        assert_eq!(wheel_gesture(0.01), MouseGesture::WheelUp);
        assert_eq!(wheel_gesture(-1.0), MouseGesture::WheelDown);
    }

    /// Zero is a decision, not arithmetic: a scroll event with no
    /// vertical component resolves to `WheelDown`, matching the
    /// `> 0.0` test both handlers used before unification. Pinned
    /// because it is the one input where a `>=` would diverge, and
    /// because both targets must make the same call.
    #[test]
    fn test_wheel_gesture_treats_zero_as_down() {
        assert_eq!(wheel_gesture(0.0), MouseGesture::WheelDown);
        assert_eq!(wheel_gesture(-0.0), MouseGesture::WheelDown);
    }
}

// -----------------------------------------------------------------
// One-shot NativeOnly-binding warn latch
//
// `warn_unhandled_native_only_once` is what a user gets when they
// bind a NativeOnly Action to a browser gesture: one line, once. It
// lived inside `run_wasm`, which is `#![cfg(target_arch = "wasm32")]`
// and therefore out of `cargo test`'s reach, though nothing in it is
// browser-specific (§T9). It is cross-platform now, and its return
// value is the emission decision — the only way to assert "exactly
// one emission" in a workspace with no log-capture facility and a
// §T10 rule against process-global mocks.
// -----------------------------------------------------------------

mod native_only_warn_latch_tests {
    use super::super::warn_unhandled_native_only_once;
    use crate::application::keybinds::Action;
    use std::sync::atomic::AtomicBool;

    fn warn(latch: &AtomicBool) -> bool {
        warn_unhandled_native_only_once(latch, "DoubleClick", &Action::OpenConsole, "remedy")
    }

    /// Once means once. The second call and every call after it are
    /// silent, so a gesture the user repeats does not fill the
    /// console — `warn!` survives into release (§9).
    #[test]
    fn test_warn_unhandled_native_only_once_emits_exactly_once() {
        let latch = AtomicBool::new(false);
        assert!(warn(&latch), "the first call must emit");
        assert!(!warn(&latch), "the second call must not emit");
        assert!(!warn(&latch), "and neither must any later one");
    }

    /// The latch is per-input-class, and that is the whole reason it
    /// is a parameter rather than a `static` inside the function: a
    /// dead double-click binding must not silence a dead wheel
    /// binding, which is what one shared latch across three call
    /// sites would do.
    #[test]
    fn test_warn_unhandled_native_only_once_latches_are_independent() {
        let double_click = AtomicBool::new(false);
        let wheel = AtomicBool::new(false);
        assert!(warn(&double_click));
        assert!(!warn(&double_click));
        assert!(warn(&wheel), "a separate input class gets its own line");
    }

    /// A latch that arrives already set stays silent — the caller's
    /// `static` is `false` at startup and never reset, so "already
    /// warned in this session" is the steady state.
    #[test]
    fn test_warn_unhandled_native_only_once_respects_a_pre_set_latch() {
        let latch = AtomicBool::new(true);
        assert!(!warn(&latch));
    }
}

// -----------------------------------------------------------------
// Drag-state abandonment guard
// -----------------------------------------------------------------

/// [`super::DragState::would_abandon_gesture`] — the class of states
/// a freshly-armed gesture may not overwrite.
///
/// Lives here rather than beside either caller because both read it:
/// the left-button press in `event_mouse_click` and the
/// `Action::PanCanvas` arm in `dispatch::native`, which the keyboard
/// and every macro tier reach as well as the middle button.
mod abandonment_guard {
    use super::*;
    use crate::application::app::throttled_interaction::moving_node::MovingNodeInteraction;
    use crate::application::app::throttled_interaction::node_resize::NodeResizeInteraction;
    use crate::application::app::throttled_interaction::ThrottledDrag;
    use baumhard::mindmap::model::{Position, Size};
    use baumhard::mindmap::tree_builder::ResizeHandleSide;
    use std::collections::HashSet;

    /// A left-started move-node drag, mid-gesture: pending state
    /// buffered, `commit_on_release_core` not yet run.
    fn moving_node_drag() -> DragState {
        DragState::throttled(ThrottledDrag::MovingNode(MovingNodeInteraction::new(
            vec!["n0".to_string()],
            false,
            HashSet::new(),
        )))
    }

    /// A right-started fast-resize, mid-gesture. `started_with_right`
    /// is what makes this one reachable with the left button free.
    fn fast_resize_drag() -> DragState {
        DragState::throttled(ThrottledDrag::NodeResize(NodeResizeInteraction::new(
            "n0".to_string(),
            ResizeHandleSide::SE,
            Position { x: 0.0, y: 0.0 },
            Size {
                width: 100.0,
                height: 50.0,
            },
            true,
        )))
    }

    /// The left button carries the same hole one button over: a
    /// right-started fast-resize is in flight while the left button
    /// is still free, so a left press reached
    /// `DragState::Throttled(..)` and replaced it with `Pending` —
    /// same silent loss, same missing undo entry.
    ///
    /// Fails when `would_abandon_gesture` drops its `Throttled` arm,
    /// which is the pre-fix shape (the guard read
    /// `matches!(.., DragState::PendingRight { .. })`).
    #[test]
    fn test_left_press_is_refused_while_a_right_started_fast_resize_is_in_flight() {
        assert!(
            fast_resize_drag().would_abandon_gesture(),
            "a left press must not replace a right-started fast-resize"
        );
        assert!(
            moving_node_drag().would_abandon_gesture(),
            "nor any other throttled drag reachable with the left button free"
        );
        assert!(
            DragState::PendingRight {
                start_pos: (0.0, 0.0),
                start_canvas: glam::Vec2::ZERO,
                hit_node: None,
                hit_section_idx: None,
            }
            .would_abandon_gesture(),
            "the guard's original member stays in the class"
        );
    }

    /// The complement, and the reason the guard is narrower than the
    /// right button's `!matches!(.., None)`: these four states owe
    /// the model nothing, and a left press has to keep re-arming from
    /// them or a click after a release the window never delivered
    /// would do nothing at all.
    ///
    /// `Pending` is the load-bearing row for `Action::PanCanvas`
    /// rather than for the press: the `LeftDrag` threshold cross
    /// dispatches `PanCanvas` *from* `Pending`, so a guard that
    /// refused there would leave the left button unable to pan at
    /// all.
    ///
    /// Fails if the guard is widened to the right button's spelling.
    #[test]
    fn test_left_press_still_rearms_from_every_state_that_owes_the_model_nothing() {
        for drag in [
            DragState::None,
            DragState::Panning,
            DragState::SelectingRect {
                start_canvas: glam::Vec2::ZERO,
                current_canvas: glam::Vec2::new(10.0, 10.0),
            },
            DragState::Pending(Box::new(PendingPress {
                start_pos: (0.0, 0.0),
                hit_node: None,
                hit_section_idx: None,
                hit_edge_handle: None,
                hit_portal_label: None,
                hit_edge_label: None,
                hit_section_resize_handle: None,
                hit_node_resize_handle: None,
            })),
        ] {
            assert!(
                !drag.would_abandon_gesture(),
                "a left press must still re-arm from {:?}",
                std::mem::discriminant(&drag)
            );
        }
    }
}
