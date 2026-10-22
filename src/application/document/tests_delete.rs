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

/// `hit_test_edge` as it stood at `bf9aa96d`: build every visible
/// edge's path, measure the unbounded `distance_to_path`, keep the
/// nearest one within `tolerance`.
///
/// Spelled out here rather than called so the tests below compare the
/// early-out against the filter it replaced, not against itself. The
/// visibility rules are copied deliberately: an eligibility change
/// both forms shared would be invisible to this, and eligibility is
/// not what these tests are about.
fn reference_hit(
    canvas_pos: Vec2,
    map: &baumhard::mindmap::model::MindMap,
    tolerance: f32,
) -> Option<EdgeRef> {
    use baumhard::mindmap::connection;

    let mut best: Option<(EdgeRef, f32)> = None;
    for edge in &map.edges {
        if !edge.visible {
            continue;
        }
        let (Some(from_node), Some(to_node)) = (map.nodes.get(&edge.from_id), map.nodes.get(&edge.to_id))
        else {
            continue;
        };
        if map.is_hidden_by_fold(from_node) || map.is_hidden_by_fold(to_node) {
            continue;
        }
        let path = connection::build_connection_path(
            from_node.pos_vec2(),
            from_node.size_vec2(),
            &edge.anchor_from,
            to_node.pos_vec2(),
            to_node.size_vec2(),
            &edge.anchor_to,
            &edge.control_points,
        );
        let dist = connection::distance_to_path(canvas_pos, &path);
        if dist > tolerance {
            continue;
        }
        if best.as_ref().is_none_or(|(_, best_dist)| dist < *best_dist) {
            best = Some((EdgeRef::new(&edge.from_id, &edge.to_id, &edge.edge_type), dist));
        }
    }
    best.map(|(e, _)| e)
}

/// A click on a curved edge whose control polygon is axis-aligned,
/// far from the origin, still selects that edge.
///
/// This is the shape that broke the bounding-box early-out, and it is
/// entirely ordinary content: two nodes stacked vertically at the same
/// x, and an edge curved with control offsets whose x is 0 — "curve
/// this edge straight up". Every control point of the resulting cubic
/// then shares one x, so the control-point box is **zero-width** on
/// that axis and any rounding in the curve evaluation leaves it. The
/// escape scales with the coordinate, which is why the same map near
/// the origin is fine and the same map at x = 10⁴ is not.
///
/// The probe points are the edge's own samples, so they are hits by
/// construction — `distance_to_path` answers them at ~0. A reject that
/// does not allow for the sampler's escape answers `None` instead, and
/// a user watches clicks land on nothing.
#[test]
fn test_hit_test_edge_finds_an_axis_aligned_curved_edge_far_from_the_origin() {
    use baumhard::mindmap::connection;
    use baumhard::mindmap::model::ControlPoint;

    let mut doc = load_test_doc();
    doc.mindmap.nodes.clear();
    doc.mindmap.edges.clear();

    let mut expected_refs = Vec::new();
    for (index, offset) in [1.0e2f32, 1.1e3, 1.0e4, 1.0e5, 1.0e6].iter().enumerate() {
        let top = format!("top{index}");
        let bottom = format!("bottom{index}");
        doc.mindmap
            .nodes
            .insert(top.clone(), default_orphan_node(&top, Vec2::new(*offset, 0.0)));
        doc.mindmap.nodes.insert(
            bottom.clone(),
            default_orphan_node(&bottom, Vec2::new(*offset, 900.0)),
        );
        let mut edge = default_cross_link_edge(&top, &bottom);
        // Offsets from the respective node centers, x = 0: the curve
        // bulges along y only, so every control point keeps the nodes'
        // shared x.
        edge.control_points = vec![
            ControlPoint { x: 0.0, y: 260.0 },
            ControlPoint { x: 0.0, y: -260.0 },
        ];
        expected_refs.push(EdgeRef::new(&edge.from_id, &edge.to_id, &edge.edge_type));
        doc.mindmap.edges.push(edge);
    }

    let mut probed = 0usize;
    let mut hits = 0usize;
    for (index, _expected) in expected_refs.iter().enumerate() {
        let edge = &doc.mindmap.edges[index];
        let from_node = &doc.mindmap.nodes[&edge.from_id];
        let to_node = &doc.mindmap.nodes[&edge.to_id];
        let path = connection::build_connection_path(
            from_node.pos_vec2(),
            from_node.size_vec2(),
            &edge.anchor_from,
            to_node.pos_vec2(),
            to_node.size_vec2(),
            &edge.anchor_to,
            &edge.control_points,
        );
        // Precondition: the control polygon really is axis-aligned, or
        // this is probing an ordinary curve and proves nothing.
        let (min, max) = connection::path_bounds(&path);
        assert_eq!(
            min.x, max.x,
            "edge {index} must have a zero-width control-point box; got {min:?}..{max:?}"
        );

        let samples = connection::sample_path(&path, 4.0, connection::MAX_PATH_SAMPLES);
        assert!(
            samples.len() > 50,
            "edge {index} must sample densely; got {}",
            samples.len()
        );
        let step = (samples.len() / 12).max(1);
        for sample in samples.iter().step_by(step) {
            // The tolerance a click carries is `EDGE_HIT_TOLERANCE_PX`
            // scaled by canvas-per-pixel, so it shrinks as the user
            // zooms *in*. The small values are that view: a tenth of a
            // canvas unit is an ordinary tolerance at high zoom, and it
            // is where an escape of a tenth of a unit stops being a
            // knife edge and starts dropping the click outright.
            for tolerance in [0.02f32, 0.1, 0.5, 4.0, 8.0, 12.0] {
                for probe in [
                    *sample,
                    // Just inside the tolerance on either side, which
                    // is the other half of the failure: a click the
                    // reference accepts by a hair.
                    *sample + Vec2::new(tolerance * 0.999, 0.0),
                    *sample - Vec2::new(tolerance * 0.999, 0.0),
                ] {
                    let expected_hit = reference_hit(probe, &doc.mindmap, tolerance);
                    assert_eq!(
                        hit_test_edge(probe, &doc.mindmap, tolerance),
                        expected_hit,
                        "edge {index} at {probe:?} (tolerance {tolerance}): \
                         the early-out disagreed with the unbounded filter"
                    );
                    if expected_hit.is_some() {
                        hits += 1;
                    }
                    probed += 1;
                }
            }
        }
    }
    assert!(probed > 500, "the sweep must actually run; probed {probed}");
    // Agreement on "nothing" is what every implementation manages.
    assert!(
        hits > 200,
        "the sweep must land on the edges it built; hit {hits} of {probed}"
    );
}

/// The bounding-box early-out `hit_test_edge` now opens with must
/// not change which edge it picks, on any point.
///
/// The reference is the filter it replaced — `distance_to_path`
/// against the tolerance — run over the same visibility-filtered
/// edge set and the same "nearest wins" rule, spelled out here
/// rather than called so the two derivations do not share the line
/// that could be wrong.
///
/// The sweep walks a grid over the fixture's own node extent at
/// several tolerances, so it covers points on an edge, points beside
/// one, points between two, and points nowhere near any. The input
/// that makes it fail is a box that does not contain its path — the
/// early-out then rejects an edge the reference accepts.
#[test]
fn test_hit_test_edge_agrees_with_the_unbounded_distance_filter() {
    use baumhard::mindmap::connection;

    let doc = load_test_doc();
    // Anchor the grid on the fixture's own geometry rather than on
    // round numbers, so the points land where the edges are.
    let (mut min, mut max) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
    for node in doc.mindmap.nodes.values() {
        min = min.min(node.pos_vec2());
        max = max.max(node.pos_vec2() + node.size_vec2());
    }
    assert!(min.x < max.x && min.y < max.y, "the fixture must have an extent");

    let mut hits = 0usize;
    let mut probes = 0usize;
    let mut compare = |point: Vec2, tolerance: f32| {
        let expected = reference_hit(point, &doc.mindmap, tolerance);
        let actual = hit_test_edge(point, &doc.mindmap, tolerance);
        assert_eq!(actual, expected, "at {point:?} with tolerance {tolerance}");
        if expected.is_some() {
            hits += 1;
        }
        probes += 1;
    };

    const STEPS: usize = 24;
    for tolerance in [1.0f32, 8.0, 40.0] {
        for ix in 0..=STEPS {
            for iy in 0..=STEPS {
                let t = |i: usize, lo: f32, hi: f32| lo + (hi - lo) * (i as f32 / STEPS as f32);
                compare(Vec2::new(t(ix, min.x, max.x), t(iy, min.y, max.y)), tolerance);
            }
        }
    }

    // A grid is blind to the case that matters most: a point sitting
    // *on* a curved edge, where a bounding box drawn from the two
    // anchors instead of the four control points would wrongly reject
    // the edge it belongs to. Probe every visible edge's own samples.
    let mut curved_probes = 0usize;
    for edge in &doc.mindmap.edges {
        if !edge.visible || edge.control_points.is_empty() {
            continue;
        }
        let (Some(from_node), Some(to_node)) = (
            doc.mindmap.nodes.get(&edge.from_id),
            doc.mindmap.nodes.get(&edge.to_id),
        ) else {
            continue;
        };
        let path = connection::build_connection_path(
            from_node.pos_vec2(),
            from_node.size_vec2(),
            &edge.anchor_from,
            to_node.pos_vec2(),
            to_node.size_vec2(),
            &edge.anchor_to,
            &edge.control_points,
        );
        for sample in connection::sample_path(&path, 4.0, connection::MAX_PATH_SAMPLES) {
            compare(sample, 1.0);
            curved_probes += 1;
        }
    }

    assert!(probes > 1_500, "the sweep must actually run; probed {probes}");
    assert!(
        curved_probes > 100,
        "the fixture must still carry curved edges for the on-path probes to mean anything;          probed {curved_probes}"
    );
    // Without a hit somewhere the sweep only proves both forms agree
    // on saying "nothing", which every implementation does.
    assert!(hits > 20, "the sweep must land on edges; hit {hits} times");
}

/// A non-finite cursor selects no edge.
///
/// This is a deliberate change of behavior, not a preserved one. The
/// filter this replaced was `distance > tolerance { continue }`, and
/// `NaN > tolerance` is false — so a `NaN` cursor fell through the
/// filter and became a candidate at an unordered distance, taking
/// whichever eligible edge came first. The tolerance-bounded form
/// asks `distance <= tolerance`, which `NaN` fails, so there is no
/// candidate at all.
#[test]
fn test_hit_test_edge_rejects_a_non_finite_cursor() {
    let doc = load_test_doc();
    // The fixture has visible edges, so "no hit" here is a decision
    // about the cursor rather than about an empty map.
    let (_, on_path) = pick_test_edge(&doc);
    assert!(hit_test_edge(on_path, &doc.mindmap, 2.0).is_some());

    for cursor in [
        Vec2::new(f32::NAN, 0.0),
        Vec2::new(0.0, f32::NAN),
        Vec2::splat(f32::NAN),
    ] {
        assert_eq!(
            hit_test_edge(cursor, &doc.mindmap, 1.0e9),
            None,
            "a non-finite cursor at {cursor:?} must select no edge"
        );
    }
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
    let scene_normal = crate::application::document::tests_common::project_roles(
        &doc,
        1.0,
        InteractionModeOverrides::none(),
    );
    let normal_colors: Vec<String> = scene_normal
        .connection_elements
        .iter()
        .map(|c| c.color.clone())
        .collect();

    // With edge selected: its element color should be the cyan highlight
    doc.selection = SelectionState::Edge(edge_ref);
    let scene_selected = crate::application::document::tests_common::project_roles(
        &doc,
        1.0,
        InteractionModeOverrides::none(),
    );
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

    let undo = doc.apply_orphan_selection(std::slice::from_ref(&parent_having_child));
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

    let undo = doc.apply_orphan_selection(std::slice::from_ref(&non_root));
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

    let undo = doc.apply_orphan_selection(std::slice::from_ref(&root));
    // The node is already a root, so there are entries (it's a valid
    // "move-to-last-root-index" op), but nothing meaningful changed:
    // parent_id is still None.
    assert!(doc.mindmap.nodes.get(&root).unwrap().parent_id.is_none());
    // And since it was already a root, no parent_child edge was removed.
    assert_eq!(doc.mindmap.edges.len(), orig_edges_len);
    // undo.entries may be non-empty but the restoration is a no-op.
    let _ = undo;
}
