//! Edge handle builder tests — channel ordering/distinctness, mutator round-trip, identity shift on midpoint→control-point transitions.

use super::super::*;

#[test]
fn edge_handle_channels_preserve_ordering_and_distinctness() {
    use crate::mindmap::scene_builder::EdgeHandleKind;
    let from = edge_handle_channel_for(EdgeHandleKind::AnchorFrom);
    let to = edge_handle_channel_for(EdgeHandleKind::AnchorTo);
    let mid = edge_handle_channel_for(EdgeHandleKind::Midpoint);
    let cp0 = edge_handle_channel_for(EdgeHandleKind::ControlPoint(0));
    let cp1 = edge_handle_channel_for(EdgeHandleKind::ControlPoint(1));
    assert!(from < to, "AnchorFrom < AnchorTo");
    assert!(to < mid, "AnchorTo < Midpoint");
    assert!(to < cp0, "AnchorTo < ControlPoint(0)");
    assert!(cp0 < cp1, "ControlPoint(0) < ControlPoint(1)");
    assert_ne!(mid, cp0, "Midpoint and ControlPoint(0) must occupy different channels");
}

/// Round-trip: a tree built from handle set A, with the mutator
/// computed from handle set B applied, reads identical to a
/// fresh `build_edge_handle_tree(B)` — provided B has the same
/// identity sequence as A (same kind ordering). Pins the §B2
/// "mutation, not rebuild" promise for the drag hot path: only
/// positions move during a handle drag, so identity stays
/// stable and the mutator path is sound.
#[test]
fn edge_handle_mutator_round_trip_matches_full_rebuild() {
    use crate::core::primitives::Applicable;
    use crate::mindmap::scene_builder::{EdgeHandleElement, EdgeHandleKind};
    use crate::mindmap::scene_cache::EdgeKey;

    let mk = |kind: EdgeHandleKind, x: f32, y: f32| EdgeHandleElement {
        edge_key: EdgeKey::new("a", "b", "child"),
        kind,
        position: (x, y),
        glyph: "◆".into(),
        color: "#00E5FF".into(),
        font_size_pt: 14.0,
    };

    let set_a = vec![
        mk(EdgeHandleKind::AnchorFrom, 0.0, 0.0),
        mk(EdgeHandleKind::AnchorTo, 100.0, 0.0),
        mk(EdgeHandleKind::Midpoint, 50.0, 0.0),
    ];
    let set_b = vec![
        mk(EdgeHandleKind::AnchorFrom, 5.0, -2.0),
        mk(EdgeHandleKind::AnchorTo, 110.0, -2.0),
        mk(EdgeHandleKind::Midpoint, 57.0, -2.0),
    ];
    assert_eq!(
        edge_handle_identity_sequence(&set_a),
        edge_handle_identity_sequence(&set_b),
        "drag preserves identity sequence; only positions move"
    );

    let mut tree_a = build_edge_handle_tree(&set_a);
    let mutator = build_edge_handle_mutator_tree(&set_b);
    mutator.apply_to(&mut tree_a);

    let expected = build_edge_handle_tree(&set_b);
    let actual_leaves: Vec<NodeId> =
        tree_a.root.children(&tree_a.arena).collect();
    let expected_leaves: Vec<NodeId> =
        expected.root.children(&expected.arena).collect();
    assert_eq!(actual_leaves.len(), expected_leaves.len());
    for (a, e) in actual_leaves.iter().zip(expected_leaves.iter()) {
        let a_area = tree_a.arena.get(*a).unwrap().get().glyph_area().unwrap();
        let e_area = expected.arena.get(*e).unwrap().get().glyph_area().unwrap();
        assert_eq!(a_area.text, e_area.text);
        assert_eq!(a_area.position, e_area.position);
        assert_eq!(a_area.render_bounds, e_area.render_bounds);
        assert_eq!(a_area.regions, e_area.regions);
    }
}

/// Adding a control point (drag-midpoint-creates-cp) or
/// switching selection from a 0-CP edge to a 1-CP edge must
/// register as a structural change in the identity sequence,
/// so the dispatcher in `update_edge_handle_tree` falls back to
/// a full rebuild rather than apply a mutator against a tree
/// whose channel set has shifted.
#[test]
fn edge_handle_identity_sequence_changes_on_midpoint_to_cp() {
    use crate::mindmap::scene_builder::{EdgeHandleElement, EdgeHandleKind};
    use crate::mindmap::scene_cache::EdgeKey;

    let mk = |kind: EdgeHandleKind| EdgeHandleElement {
        edge_key: EdgeKey::new("a", "b", "child"),
        kind,
        position: (0.0, 0.0),
        glyph: "◆".into(),
        color: "#00E5FF".into(),
        font_size_pt: 14.0,
    };
    let straight = vec![
        mk(EdgeHandleKind::AnchorFrom),
        mk(EdgeHandleKind::AnchorTo),
        mk(EdgeHandleKind::Midpoint),
    ];
    let curved = vec![
        mk(EdgeHandleKind::AnchorFrom),
        mk(EdgeHandleKind::AnchorTo),
        mk(EdgeHandleKind::ControlPoint(0)),
    ];
    assert_ne!(
        edge_handle_identity_sequence(&straight),
        edge_handle_identity_sequence(&curved)
    );
}
