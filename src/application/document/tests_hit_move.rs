// SPDX-License-Identifier: MPL-2.0

//! Hit-testing, rect-select, selection, tree highlights, move, drag, animations.
//!
//! Part of the tests split for `document`. Helpers live in
//! `tests_common`; only the tests for this theme live here.
use super::tests_common::{load_test_doc, load_test_tree, TestNudgeMutation};
use super::*;

use baumhard::mindmap::animation::{AnimationTiming, Easing};
use baumhard::mindmap::custom_mutation::{CustomMutation as CM, TargetScope as TS};
use baumhard::mindmap::tree_builder::INACTIVE_NODE_ALPHA_MULTIPLIER;
use baumhard::util::geometry::{aabb_center, almost_equal_f64};
use glam::Vec2;

#[test]
fn test_hit_test_direct_hit() {
    let mut tree = load_test_tree();
    // "Lord God" node (id: 0) — get its position from the tree
    let node_id = tree.arena_id_for("0").unwrap();
    let area = tree.tree.arena.get(node_id).unwrap().get().glyph_area().unwrap();
    let center = aabb_center(
        Vec2::new(area.position.x.0, area.position.y.0),
        Vec2::new(area.render_bounds.x.0, area.render_bounds.y.0),
    );
    let result = hit_test(center, &mut tree);
    assert_eq!(result, Some("0".to_string()));
}

/// `hit_test_target` collapses single-section nodes (the
/// migration default) to `HitTarget::NodeContainer` — clicking
/// anywhere inside a node with one default section gives the
/// pre-section whole-node hit semantic.
#[test]
fn test_hit_test_target_single_section_collapses_to_node() {
    let mut tree = load_test_tree();
    let node_id = tree.arena_id_for("0").unwrap();
    let area = tree.tree.arena.get(node_id).unwrap().get().glyph_area().unwrap();
    let center = aabb_center(
        Vec2::new(area.position.x.0, area.position.y.0),
        Vec2::new(area.render_bounds.x.0, area.render_bounds.y.0),
    );
    match hit_test_target(center, &mut tree) {
        Some(HitTarget::NodeContainer { node_id }) => {
            assert_eq!(node_id, "0");
        }
        other => panic!("expected NodeContainer, got {other:?}"),
    }
}

#[test]
fn test_hit_test_miss() {
    let mut tree = load_test_tree();
    // A point far away from any node
    let result = hit_test(Vec2::new(-99999.0, -99999.0), &mut tree);
    assert_eq!(result, None);
}

#[test]
fn test_hit_test_returns_smallest_on_overlap() {
    let mut tree = load_test_tree();
    // Find a parent-child pair where child is inside parent's bounds
    // "Lord God" (0) has children — find one whose bounds overlap
    let parent_id_str = "0";
    let parent_size = {
        let nid = tree.arena_id_for(parent_id_str).unwrap();
        let area = tree.tree.arena.get(nid).unwrap().get().glyph_area().unwrap();
        area.render_bounds.x.0 * area.render_bounds.y.0
    };

    // Collect candidate (mind_id, center) pairs first to release
    // the immutable borrow on tree.node_map before calling
    // hit_test (which needs &mut tree).
    let candidate: Option<(String, Vec2)> =
        tree.node_ids()
            .filter(|(id, _)| *id != parent_id_str)
            .find_map(|(mind_id, nid)| {
                let a = tree.tree.arena.get(nid)?.get().glyph_area()?;
                let child_size = a.render_bounds.x.0 * a.render_bounds.y.0;
                let child_center = aabb_center(
                    Vec2::new(a.position.x.0, a.position.y.0),
                    Vec2::new(a.render_bounds.x.0, a.render_bounds.y.0),
                );
                if child_size < parent_size && point_in_node_aabb(child_center, parent_id_str, &tree) {
                    Some((mind_id.to_string(), child_center))
                } else {
                    None
                }
            });

    if let Some((expected_id, center)) = candidate {
        let result = hit_test(center, &mut tree);
        assert_eq!(
            result,
            Some(expected_id),
            "Should select smaller child node, not parent"
        );
    }
    // If no overlap found in test data, that's OK — test is structural
}

#[test]
fn test_apply_tree_highlights_via_walker() {
    let mut tree = load_test_tree();
    // Post-refactor: regions live on the section-area, not the
    // container. Read through the section_map.
    let section_id = tree.section_arena_id("0", 0).unwrap();

    // Before highlight: original color (white).
    let area = tree
        .tree
        .arena
        .get(section_id)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap();
    let original_color = area.regions.all_regions()[0].color.unwrap();
    assert!(
        (original_color[0] - 1.0).abs() < 0.01,
        "Expected white before highlight"
    );

    // Apply highlight via the new mutator-driven path.
    apply_tree_highlights(&mut tree, std::iter::once(("0", None, HIGHLIGHT_COLOR)));

    // After highlight: cyan on section-area's regions.
    let area = tree
        .tree
        .arena
        .get(section_id)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap();
    let highlighted_color = area.regions.all_regions()[0].color.unwrap();
    assert!((highlighted_color[0] - HIGHLIGHT_COLOR[0]).abs() < 0.01);
    assert!((highlighted_color[1] - HIGHLIGHT_COLOR[1]).abs() < 0.01);
    assert!((highlighted_color[2] - HIGHLIGHT_COLOR[2]).abs() < 0.01);
}

/// Alpha of the first region on a node's first section area.
fn first_section_alpha(tree: &MindMapTree, mind_id: &str) -> f32 {
    let section_id = tree
        .section_arena_id(mind_id, 0)
        .unwrap_or_else(|| panic!("{mind_id} must have a section area"));
    tree.tree
        .arena
        .get(section_id)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .regions
        .all_regions()[0]
        .color
        .unwrap()[3]
}

/// The NodeEdit affordance: with `active = Some(id)`, every *other*
/// node's text drops to half alpha and the active node keeps full
/// alpha. Pairs with the border half of the affordance, which the
/// border pass applies from the same constant.
#[test]
fn test_apply_inactive_node_dimming_halves_other_nodes_only() {
    let mut tree = load_test_tree();
    let others: Vec<String> = tree
        .node_ids()
        .filter(|(id, _)| *id != "0")
        .map(|(id, _)| id.to_string())
        .take(3)
        .collect();
    assert!(!others.is_empty(), "fixture must have more than one node");

    let active_before = first_section_alpha(&tree, "0");
    let others_before: Vec<f32> = others.iter().map(|id| first_section_alpha(&tree, id)).collect();

    apply_inactive_node_dimming(&mut tree, Some("0"));

    assert!(
        (first_section_alpha(&tree, "0") - active_before).abs() < 1e-3,
        "the active node must keep its alpha"
    );
    for (id, before) in others.iter().zip(others_before) {
        let after = first_section_alpha(&tree, id);
        assert!(
            (after - before * INACTIVE_NODE_ALPHA_MULTIPLIER).abs() < 1e-2,
            "{id}: alpha {after} should be {before} x {INACTIVE_NODE_ALPHA_MULTIPLIER}"
        );
    }
}

/// `None` (Default mode) is the no-op fast path — no node's alpha
/// moves. A regression here would dim the entire canvas outside
/// NodeEdit.
#[test]
fn test_apply_inactive_node_dimming_none_is_a_no_op() {
    let mut tree = load_test_tree();
    let before: Vec<(String, f32)> = tree
        .node_ids()
        .map(|(id, _)| id.to_string())
        .collect::<Vec<_>>()
        .into_iter()
        .map(|id| {
            let a = first_section_alpha(&tree, &id);
            (id, a)
        })
        .collect();

    apply_inactive_node_dimming(&mut tree, None);

    for (id, alpha) in before {
        assert!(
            (first_section_alpha(&tree, &id) - alpha).abs() < 1e-6,
            "{id} must be untouched in Default mode"
        );
    }
}

/// Dimming is applied *before* the selection highlight at every
/// rebuild site, so a node that is both inactive and selected reads
/// as selected rather than as faded. Locks that ordering.
#[test]
fn test_selection_highlight_wins_over_dimming_when_applied_after() {
    let mut tree = load_test_tree();
    let other = tree
        .node_ids()
        .map(|(id, _)| id.to_string())
        .find(|id| id != "0")
        .expect("fixture must have more than one node");

    apply_inactive_node_dimming(&mut tree, Some("0"));
    apply_tree_highlights(
        &mut tree,
        std::iter::once((other.as_str(), None, HIGHLIGHT_COLOR)),
    );

    let section_id = tree.section_arena_id(&other, 0).unwrap();
    let color = tree
        .tree
        .arena
        .get(section_id)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .regions
        .all_regions()[0]
        .color
        .unwrap();
    assert!((color[0] - HIGHLIGHT_COLOR[0]).abs() < 0.01);
    assert!(
        (color[3] - HIGHLIGHT_COLOR[3]).abs() < 0.01,
        "highlight alpha survives the dim"
    );
}

#[test]
fn test_apply_tree_highlights_does_not_affect_others() {
    let mut tree = load_test_tree();

    // Pick a different node and copy its regions before mutation.
    let other_id = tree
        .node_ids()
        .map(|(k, _)| k)
        .find(|k| *k != "0")
        .unwrap()
        .to_string();
    let other_node_id = tree.arena_id_for(&other_id).unwrap();
    let before = tree
        .tree
        .arena
        .get(other_node_id)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .regions
        .clone();

    apply_tree_highlights(&mut tree, std::iter::once(("0", None, HIGHLIGHT_COLOR)));

    let after = tree
        .tree
        .arena
        .get(other_node_id)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .regions
        .clone();
    assert_eq!(before, after, "Unselected node colors should not change");
}

#[test]
fn test_apply_tree_highlights_later_pair_overrides_earlier() {
    // The reparent-mode flow relies on source-orange overriding the
    // previously-applied selection-cyan on the same node. Verify the
    // last-write-wins semantics of apply_tree_highlights.
    let mut tree = load_test_tree();
    // Regions live on the section-area, not the container.
    let section_id = tree.section_arena_id("0", 0).unwrap();

    apply_tree_highlights(
        &mut tree,
        vec![("0", None, HIGHLIGHT_COLOR), ("0", None, REPARENT_SOURCE_COLOR)],
    );

    let area = tree
        .tree
        .arena
        .get(section_id)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap();
    let c = area.regions.all_regions()[0].color.unwrap();
    assert!((c[0] - REPARENT_SOURCE_COLOR[0]).abs() < 0.01);
    assert!((c[1] - REPARENT_SOURCE_COLOR[1]).abs() < 0.01);
    assert!((c[2] - REPARENT_SOURCE_COLOR[2]).abs() < 0.01);
}

#[test]
fn test_move_subtree_updates_all_positions() {
    let mut doc = load_test_doc();
    let node_id = "0"; // Lord God
    let descendants = doc.mindmap.all_descendants(node_id);
    assert!(!descendants.is_empty(), "Lord God should have descendants");

    // Record original positions
    let orig_pos: Vec<(String, f64, f64)> = std::iter::once(node_id.to_string())
        .chain(descendants.iter().cloned())
        .filter_map(|id| {
            let n = doc.mindmap.nodes.get(&id)?;
            Some((id, n.position.x, n.position.y))
        })
        .collect();

    let dx = 50.0;
    let dy = -30.0;
    doc.apply_move_subtree(node_id, dx, dy);

    for (id, ox, oy) in &orig_pos {
        let n = doc.mindmap.nodes.get(id).unwrap();
        assert!(
            (n.position.x - (ox + dx)).abs() < 0.001,
            "Node {} x not shifted",
            id
        );
        assert!(
            (n.position.y - (oy + dy)).abs() < 0.001,
            "Node {} y not shifted",
            id
        );
    }
}

#[test]
fn test_move_subtree_preserves_relative_positions() {
    let mut doc = load_test_doc();
    let node_id = "0";
    let descendants = doc.mindmap.all_descendants(node_id);

    // Record relative offsets from parent to each descendant
    let parent = doc.mindmap.nodes.get(node_id).unwrap();
    let offsets: Vec<(String, f64, f64)> = descendants
        .iter()
        .filter_map(|id| {
            let n = doc.mindmap.nodes.get(id)?;
            Some((
                id.clone(),
                n.position.x - parent.position.x,
                n.position.y - parent.position.y,
            ))
        })
        .collect();

    doc.apply_move_subtree(node_id, 100.0, 200.0);

    let parent = doc.mindmap.nodes.get(node_id).unwrap();
    for (id, dx, dy) in &offsets {
        let n = doc.mindmap.nodes.get(id).unwrap();
        let actual_dx = n.position.x - parent.position.x;
        let actual_dy = n.position.y - parent.position.y;
        assert!(
            (actual_dx - dx).abs() < 0.001,
            "Relative x offset changed for {}",
            id
        );
        assert!(
            (actual_dy - dy).abs() < 0.001,
            "Relative y offset changed for {}",
            id
        );
    }
}

#[test]
fn test_move_single_only_affects_target() {
    let mut doc = load_test_doc();
    let node_id = "0";
    let descendants = doc.mindmap.all_descendants(node_id);

    // Record descendant positions before
    let before: Vec<(String, f64, f64)> = descendants
        .iter()
        .filter_map(|id| {
            let n = doc.mindmap.nodes.get(id)?;
            Some((id.clone(), n.position.x, n.position.y))
        })
        .collect();

    doc.apply_move_single(node_id, 100.0, 200.0);

    // Descendants should be unchanged
    for (id, ox, oy) in &before {
        let n = doc.mindmap.nodes.get(id).unwrap();
        assert!(
            (n.position.x - ox).abs() < 0.001,
            "Descendant {} x changed unexpectedly",
            id
        );
        assert!(
            (n.position.y - oy).abs() < 0.001,
            "Descendant {} y changed unexpectedly",
            id
        );
    }

    // The target node still exists; we don't assert on its
    // exact post-move position here (the test pre-condition
    // captures only descendant positions).
    assert!(doc.mindmap.nodes.contains_key(node_id));
}

/// `start_animation` records an instance, snapshots from/to,
/// and `has_active_animations` flips true. The mutation never
/// touches the model — that's the boundary commit at completion.
#[test]
fn test_start_animation_records_instance_without_committing() {
    let mut doc = load_test_doc();
    let node_id = "0".to_string();
    let orig_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;

    let cm = make_test_mutation_with_timing(
        "nudge-anim",
        TS::SelfOnly,
        Some(AnimationTiming {
            duration_ms: 200,
            delay_ms: 0,
            easing: Easing::Linear,
            then: None,
        }),
    );
    doc.mutation_registry.insert(cm.id.clone(), cm.clone());
    assert!(!doc.has_active_animations());
    doc.start_animation(&cm, &node_id, 1_000);
    assert!(doc.has_active_animations());
    assert_eq!(doc.active_animations.len(), 1);

    // Model untouched at start.
    let pos_now = doc.mindmap.nodes.get(&node_id).unwrap().position.x;
    assert!(almost_equal_f64(pos_now, orig_x));

    // From / to snapshots reflect the nudge (test mutation is
    // NudgeRight(10.0)).
    let inst = &doc.active_animations[0];
    assert!(almost_equal_f64(inst.from_node.position.x, orig_x));
    assert!(almost_equal_f64(inst.to_node.position.x, orig_x + 10.0));
}

/// `tick_animations` at the linear midpoint writes the mean of
/// from / to into `mindmap.nodes`. Pins the per-tick blend
/// math against the canonical `lerp_f32` helper.
#[test]
fn test_tick_animations_linear_midpoint_blend() {
    let mut doc = load_test_doc();
    let node_id = "0".to_string();
    let orig_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;

    let cm = make_test_mutation_with_timing(
        "nudge-anim",
        TS::SelfOnly,
        Some(AnimationTiming {
            duration_ms: 200,
            delay_ms: 0,
            easing: Easing::Linear,
            then: None,
        }),
    );
    doc.mutation_registry.insert(cm.id.clone(), cm.clone());
    doc.start_animation(&cm, &node_id, 1_000);

    // Tick at the midpoint (start + 100ms of 200ms duration).
    let advanced = doc.tick_animations(1_100, None);
    assert!(advanced);
    let mid_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;
    // NudgeRight(10.0) at t=0.5 → +5.0 from origin.
    assert!(
        (mid_x - orig_x - 5.0).abs() < 1e-3,
        "midpoint x = {mid_x}, expected {}",
        orig_x + 5.0
    );

    // Animation still active mid-flight.
    assert!(doc.has_active_animations());
}

/// At `t >= 1.0` the animation completes: the final state is
/// applied (matching the instant-mode result), the instance
/// is dropped, and `has_active_animations` flips back to false.
#[test]
fn test_tick_animations_completes_and_clears() {
    let mut doc = load_test_doc();
    let node_id = "0".to_string();
    let orig_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;

    let cm = make_test_mutation_with_timing(
        "nudge-anim",
        TS::SelfOnly,
        Some(AnimationTiming {
            duration_ms: 100,
            delay_ms: 0,
            easing: Easing::Linear,
            then: None,
        }),
    );
    doc.mutation_registry.insert(cm.id.clone(), cm.clone());
    doc.start_animation(&cm, &node_id, 0);

    // Tick past the duration. Without a tree, the model is set
    // to the `to` snapshot directly.
    let advanced = doc.tick_animations(150, None);
    assert!(advanced);
    assert!(!doc.has_active_animations());
    let final_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;
    // Default test-mutation `NudgeRight(10.0)` lands at +10.
    assert!((final_x - orig_x - 10.0).abs() < 1e-3);
}

/// Ctrl+Z mid-animation fast-forwards to the completion
/// state, pushes the animation's undo entry, and then the
/// undo pops it — net effect is that Ctrl+Z during an
/// animated transition reverses the animation in one
/// keystroke, same as Ctrl+Z after natural completion. Pins
/// the §4 "no half-features" contract the review called
/// out: without this, Ctrl+Z during an animation pops the
/// *previous* action, a silent user-visible regression.
#[test]
fn test_fast_forward_then_undo_reverses_animation() {
    let mut doc = load_test_doc();
    let node_id = "0".to_string();
    let orig_x = doc.mindmap.nodes.get(&node_id).unwrap().position.x;

    let cm = make_test_mutation_with_timing(
        "nudge-anim",
        TS::SelfOnly,
        Some(AnimationTiming {
            duration_ms: 1_000,
            delay_ms: 0,
            easing: Easing::Linear,
            then: None,
        }),
    );
    doc.mutation_registry.insert(cm.id.clone(), cm.clone());
    doc.start_animation(&cm, &node_id, 0);

    // Fast-forward (simulating Ctrl+Z entry in the event
    // loop). A tree is required because
    // `apply_custom_mutation` routes through it.
    let mut tree = doc.build_tree();
    doc.fast_forward_animations(Some(&mut tree));
    assert!(!doc.has_active_animations());
    let after_ff = doc.mindmap.nodes.get(&node_id).unwrap().position.x;
    assert!(
        (after_ff - orig_x - 10.0).abs() < 1e-3,
        "post fast-forward x = {after_ff}, expected {}",
        orig_x + 10.0
    );

    // Undo pops the entry fast-forward pushed. Position
    // returns to the original.
    let popped = doc.undo();
    assert!(popped, "undo must pop the fast-forward's entry");
    let after_undo = doc.mindmap.nodes.get(&node_id).unwrap().position.x;
    assert!(
        (after_undo - orig_x).abs() < 1e-3,
        "post undo x = {after_undo}, expected {orig_x}"
    );
}

/// Re-triggering the same `(mutation_id, node_id)` mid-flight
/// is a silent no-op — otherwise a held button could spawn dozens
/// of overlapping instances and the blend would overshoot.
#[test]
fn test_start_animation_re_trigger_mid_flight_is_noop() {
    let mut doc = load_test_doc();
    let node_id = "0".to_string();
    let cm = make_test_mutation_with_timing(
        "nudge-anim",
        TS::SelfOnly,
        Some(AnimationTiming {
            duration_ms: 200,
            delay_ms: 0,
            easing: Easing::Linear,
            then: None,
        }),
    );
    doc.mutation_registry.insert(cm.id.clone(), cm.clone());

    doc.start_animation(&cm, &node_id, 1_000);
    doc.start_animation(&cm, &node_id, 1_050);
    doc.start_animation(&cm, &node_id, 1_100);

    assert_eq!(doc.active_animations.len(), 1);
    assert_eq!(doc.active_animations[0].start_ms, 1_000);
}

fn make_test_mutation_with_timing(
    id: &str,
    scope: TS,
    timing: Option<baumhard::mindmap::animation::AnimationTiming>,
) -> CM {
    let mut b = TestNudgeMutation::new(id, scope).magnitude(10.0);
    if let Some(t) = timing {
        b = b.timing(t);
    }
    b.build()
}

#[test]
fn test_move_returns_original_positions() {
    let mut doc = load_test_doc();
    let node_id = "0";
    let orig_x = doc.mindmap.nodes.get(node_id).unwrap().position.x;
    let orig_y = doc.mindmap.nodes.get(node_id).unwrap().position.y;

    let undo_data = doc.apply_move_subtree(node_id, 50.0, 50.0);
    let target_entry = undo_data.iter().find(|(id, _)| id == node_id).unwrap();
    assert!((target_entry.1.x - orig_x).abs() < 0.001);
    assert!((target_entry.1.y - orig_y).abs() < 0.001);
}

#[test]
fn test_undo_restores_positions() {
    let mut doc = load_test_doc();
    let node_id = "0";

    // Record original positions
    let orig_x = doc.mindmap.nodes.get(node_id).unwrap().position.x;
    let orig_y = doc.mindmap.nodes.get(node_id).unwrap().position.y;

    // Move and push undo
    let undo_data = doc.apply_move_subtree(node_id, 100.0, 200.0);
    doc.undo_stack.push(UndoAction::MoveNodes {
        original_positions: undo_data,
    });

    // Verify moved
    assert!((doc.mindmap.nodes.get(node_id).unwrap().position.x - (orig_x + 100.0)).abs() < 0.001);

    // Undo
    assert!(doc.undo());

    // Verify restored
    assert!((doc.mindmap.nodes.get(node_id).unwrap().position.x - orig_x).abs() < 0.001);
    assert!((doc.mindmap.nodes.get(node_id).unwrap().position.y - orig_y).abs() < 0.001);
}

#[test]
fn test_apply_drag_delta() {
    let doc = load_test_doc();
    let mut tree = doc.build_tree();
    let node_id = "0";

    let tree_nid = tree.arena_id_for(node_id).unwrap();
    let orig_x = tree
        .tree
        .arena
        .get(tree_nid)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .position
        .x
        .0;
    let orig_y = tree
        .tree
        .arena
        .get(tree_nid)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .position
        .y
        .0;

    apply_drag_delta(&mut tree, node_id, 25.0, -15.0, false);

    let new_x = tree
        .tree
        .arena
        .get(tree_nid)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .position
        .x
        .0;
    let new_y = tree
        .tree
        .arena
        .get(tree_nid)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .position
        .y
        .0;
    assert!((new_x - (orig_x + 25.0)).abs() < 0.001);
    assert!((new_y - (orig_y - 15.0)).abs() < 0.001);
}

#[test]
fn test_apply_drag_delta_with_descendants() {
    let doc = load_test_doc();
    let mut tree = doc.build_tree();
    let node_id = "0";

    // Find a child of Lord God in the tree
    let child_ids: Vec<String> = doc.mindmap.all_descendants(node_id);
    assert!(!child_ids.is_empty());
    let child_id = &child_ids[0];
    let child_tree_nid = tree.arena_id_for(child_id).unwrap();
    let child_orig_x = tree
        .tree
        .arena
        .get(child_tree_nid)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .position
        .x
        .0;

    apply_drag_delta(&mut tree, node_id, 30.0, 20.0, true);

    let child_new_x = tree
        .tree
        .arena
        .get(child_tree_nid)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .position
        .x
        .0;
    assert!(
        (child_new_x - (child_orig_x + 30.0)).abs() < 0.001,
        "Descendant should be shifted when include_descendants=true"
    );
}

/// `apply_section_drag_delta_and_collect_patches` moves only the
/// targeted section's subtree — the owning node's container area
/// stays put, sibling sections stay put. Pins the per-frame
/// section-drag tree mutation that the `MovingSectionInteraction`
/// drain depends on.
#[test]
fn test_apply_section_drag_delta_moves_only_target_section() {
    use crate::application::document::apply_section_drag_delta_and_collect_patches;
    use crate::application::document::tests_common::pinned_two_section_node;
    let (doc, id) = pinned_two_section_node();
    let mut tree = doc.build_tree();

    let container_nid = tree.arena_id_for(&id).unwrap();
    let s0_nid = tree.section_arena_id(&id, 0).unwrap();
    let s1_nid = tree.section_arena_id(&id, 1).unwrap();
    let container_x = tree
        .tree
        .arena
        .get(container_nid)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .position
        .x
        .0;
    let s0_x = tree
        .tree
        .arena
        .get(s0_nid)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .position
        .x
        .0;
    let s1_x = tree
        .tree
        .arena
        .get(s1_nid)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .position
        .x
        .0;

    let mut patches = Vec::new();
    apply_section_drag_delta_and_collect_patches(&mut tree, &id, 1, 17.0, 0.0, &mut patches);

    // Target section moved by +17 on x.
    let s1_new_x = tree
        .tree
        .arena
        .get(s1_nid)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .position
        .x
        .0;
    assert!(
        (s1_new_x - (s1_x + 17.0)).abs() < 0.001,
        "section[1] must move by +17"
    );

    // Container untouched.
    let container_new_x = tree
        .tree
        .arena
        .get(container_nid)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .position
        .x
        .0;
    assert!(
        (container_new_x - container_x).abs() < 0.001,
        "container must NOT move during section drag"
    );

    // Sibling section[0] untouched.
    let s0_new_x = tree
        .tree
        .arena
        .get(s0_nid)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .position
        .x
        .0;
    assert!(
        (s0_new_x - s0_x).abs() < 0.001,
        "sibling section[0] must NOT move"
    );

    assert!(!patches.is_empty(), "drag must emit at least one buffer patch");
}

/// `apply_section_resize_to_tree` writes the in-progress AABB
/// directly to the section-area's `GlyphArea`. Verify the
/// position and bounds round-trip through the helper.
#[test]
fn test_apply_section_resize_to_tree_writes_position_and_bounds() {
    use crate::application::document::apply_section_resize_to_tree;
    use crate::application::document::tests_common::pinned_two_section_node;
    use glam::Vec2;

    let (doc, id) = pinned_two_section_node();
    let mut tree = doc.build_tree();
    let new_pos = Vec2::new(100.0, 50.0);
    let new_size = Vec2::new(40.0, 20.0);
    let written = apply_section_resize_to_tree(&mut tree, &id, 1, new_pos, new_size);
    let arena_id = tree.section_arena_id(&id, 1).unwrap();
    // The returned id is what the resize drain re-shapes; if it
    // named anything but the section it just wrote, the drag would
    // refresh the wrong element and leave this one stale on screen.
    assert_eq!(written, Some(arena_id));
    let area = tree
        .tree
        .arena
        .get(arena_id)
        .and_then(|n| n.get().glyph_area())
        .unwrap();
    assert!((area.position.x.0 - 100.0).abs() < 0.001);
    assert!((area.position.y.0 - 50.0).abs() < 0.001);
    assert!((area.render_bounds.x.0 - 40.0).abs() < 0.001);
    assert!((area.render_bounds.y.0 - 20.0).abs() < 0.001);
}

/// Out-of-range section index is a no-op; helper returns without
/// panicking, tree unchanged.
#[test]
fn test_apply_section_resize_to_tree_unknown_section_no_op() {
    use crate::application::document::apply_section_resize_to_tree;
    use crate::application::document::tests_common::pinned_two_section_node;
    use glam::Vec2;

    let (doc, id) = pinned_two_section_node();
    let mut tree = doc.build_tree();
    let arena_id = tree.section_arena_id(&id, 1).unwrap();
    let area_before = tree
        .tree
        .arena
        .get(arena_id)
        .and_then(|n| n.get().glyph_area())
        .map(|a| (a.position.x.0, a.position.y.0))
        .unwrap();
    let written =
        apply_section_resize_to_tree(&mut tree, &id, 99, Vec2::new(0.0, 0.0), Vec2::new(10.0, 10.0));
    assert_eq!(
        written, None,
        "a write that did not happen must not name an element to re-shape"
    );
    let area_after = tree
        .tree
        .arena
        .get(arena_id)
        .and_then(|n| n.get().glyph_area())
        .map(|a| (a.position.x.0, a.position.y.0))
        .unwrap();
    assert_eq!(area_before, area_after, "tree must be untouched on unknown idx");
}

/// Out-of-range section index is a no-op; the helper returns
/// without panicking and emits no patches.
#[test]
fn test_apply_section_drag_delta_unknown_section_no_op() {
    use crate::application::document::apply_section_drag_delta_and_collect_patches;
    use crate::application::document::tests_common::pinned_two_section_node;
    let (doc, id) = pinned_two_section_node();
    let mut tree = doc.build_tree();
    let mut patches = Vec::new();
    apply_section_drag_delta_and_collect_patches(&mut tree, &id, 99, 5.0, 5.0, &mut patches);
    assert!(patches.is_empty(), "unknown section_idx → no patches");
}

/// Drag-release shape: simulate `MovingSectionInteraction`'s
/// release-commit by combining `start_offset` with `total_delta`
/// and calling `set_section_offset`. Verify the model accepts,
/// the offset lands at `start + delta`, and undo restores the
/// pre-drag offset. Pins the release-commit contract event-loop
/// integration depends on.
#[test]
fn test_section_drag_release_writes_through_set_section_offset() {
    use crate::application::document::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();
    // Fixture pins section[1] at offset (10, 10).
    let start_x = 10.0_f64;
    let start_y = 10.0_f64;
    // Simulate a drag that accumulated total_delta = (15, 7).
    let delta_x = 15.0_f64;
    let delta_y = 7.0_f64;
    let result = doc.set_section_offset(&id, 1, start_x + delta_x, start_y + delta_y);
    assert_eq!(result, Ok(true));
    let s = &doc.mindmap.nodes.get(&id).unwrap().sections[1];
    assert!((s.offset.x - 25.0).abs() < 0.001);
    assert!((s.offset.y - 17.0).abs() < 0.001);
    assert!(doc.undo());
    let restored = &doc.mindmap.nodes.get(&id).unwrap().sections[1];
    assert!(
        (restored.offset.x - 10.0).abs() < 0.001,
        "undo restores prior offset"
    );
    assert!((restored.offset.y - 10.0).abs() < 0.001);
}

/// AABB-overflow on drag release: the setter rejects with the
/// verify-mirror message, the model is unchanged, and a full
/// `rebuild_all` (simulated here by re-reading the model) snaps
/// the section back to its original offset. Pins the release-
/// commit error-recovery path.
#[test]
fn test_section_drag_release_aabb_overflow_rejects_and_preserves_model() {
    use crate::application::document::apply_section_drag_delta_and_collect_patches;
    use crate::application::document::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();

    // Simulate the drag: build the tree, mutate it per-frame as
    // the gesture would, then attempt the release-commit.
    let mut tree = doc.build_tree();
    let s1_root = tree.section_arena_id(&id, 1).unwrap();
    let original_section_pos = tree
        .tree
        .arena
        .get(s1_root)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .position
        .x
        .0;
    // Drag the tree past the parent's right edge (delta=+200,
    // would-be offset (210, 10), right edge 260 > 200).
    let mut patches = Vec::new();
    apply_section_drag_delta_and_collect_patches(&mut tree, &id, 1, 200.0, 0.0, &mut patches);
    let dragged_pos = tree
        .tree
        .arena
        .get(s1_root)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .position
        .x
        .0;
    assert!(
        (dragged_pos - (original_section_pos + 200.0)).abs() < 0.001,
        "tree mid-drag must reflect the per-frame mutation"
    );

    // Release-commit: setter rejects.
    let result = doc.set_section_offset(&id, 1, 210.0, 10.0);
    assert!(
        result
            .as_ref()
            .err()
            .map(|m| m.contains("extends past node right edge"))
            .unwrap_or(false),
        "AABB overflow must reject with the verify-mirror message; got {:?}",
        result
    );
    // Model unchanged — section's offset stays at (10, 10).
    let s = &doc.mindmap.nodes.get(&id).unwrap().sections[1];
    assert!(
        (s.offset.x - 10.0).abs() < 0.001,
        "model must not have moved on rejected release"
    );

    // Snap-back: simulate `rebuild_all`'s model→tree rebuild.
    // The release path calls `rebuild_all` (which walks
    // `doc.build_tree()` from the unchanged model). Build a fresh
    // tree and assert section[1] sits back at its pre-drag offset.
    let restored_tree = doc.build_tree();
    let s1_restored = restored_tree.section_arena_id(&id, 1).unwrap();
    let restored_pos = restored_tree
        .tree
        .arena
        .get(s1_restored)
        .unwrap()
        .get()
        .glyph_area()
        .unwrap()
        .position
        .x
        .0;
    assert!(
        (restored_pos - original_section_pos).abs() < 0.001,
        "rebuild_all from unchanged model must snap section back: original={}, restored={}",
        original_section_pos,
        restored_pos
    );
}

#[test]
fn test_dedup_subtree_roots() {
    let doc = load_test_doc();
    let parent_id = "0"; // Lord God
    let descendants = doc.mindmap.all_descendants(parent_id);
    assert!(!descendants.is_empty());
    let child_id = &descendants[0];

    // If both parent and child are selected, only parent should be a root
    let ids = vec![parent_id.to_string(), child_id.clone()];
    let roots = doc.dedup_subtree_roots(&ids);
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0], parent_id);
}

#[test]
fn test_apply_move_multiple_no_double_movement() {
    let mut doc = load_test_doc();
    let parent_id = "0";
    let descendants = doc.mindmap.all_descendants(parent_id);
    let child_id = &descendants[0];

    let child_orig_x = doc.mindmap.nodes.get(child_id).unwrap().position.x;

    // Move both parent and child as subtrees — child should only move once (via parent)
    let ids = vec![parent_id.to_string(), child_id.clone()];
    doc.apply_move_multiple(&ids, 50.0, 0.0, false);

    let child_new_x = doc.mindmap.nodes.get(child_id).unwrap().position.x;
    assert!(
        (child_new_x - (child_orig_x + 50.0)).abs() < 0.001,
        "Child should be moved exactly once, not twice"
    );
}

#[test]
fn test_rect_select_finds_nodes_in_region() {
    let tree = load_test_tree();
    // Get position/bounds of "Lord God" to build a rect that contains it
    let node_id = tree.arena_id_for("0").unwrap();
    let area = tree.tree.arena.get(node_id).unwrap().get().glyph_area().unwrap();
    let x = area.position.x.0;
    let y = area.position.y.0;
    let w = area.render_bounds.x.0;
    let h = area.render_bounds.y.0;

    // A rect that exactly contains this node should select it
    let hits = rect_select(
        Vec2::new(x - 1.0, y - 1.0),
        Vec2::new(x + w + 1.0, y + h + 1.0),
        &tree,
    );
    assert!(hits.contains(&"0".to_string()), "Should find Lord God in rect");
}

#[test]
fn test_rect_select_misses_distant_nodes() {
    let tree = load_test_tree();
    // A rect far from any node should select nothing
    let hits = rect_select(
        Vec2::new(-99999.0, -99999.0),
        Vec2::new(-99998.0, -99998.0),
        &tree,
    );
    assert!(hits.is_empty(), "Should find no nodes in distant rect");
}

// --- NodeShape integration tests ---
//
// These exercise the end-to-end wiring: flip a node's `shape` on
// the tree side to `Ellipse`, then assert that hit_test /
// point_in_node_aabb / rect_select all honor the new geometry.
// The baumhard-level math is covered exhaustively in
// `lib/baumhard/src/gfx_structs/shape.rs#tests`; these tests
// guard the plumbing.

fn set_node_shape_ellipse(tree: &mut baumhard::mindmap::tree_builder::MindMapTree, node_id: &str) {
    use baumhard::gfx_structs::shape::NodeShape;
    let nid = tree.arena_id_for(node_id).expect("node exists");
    let node = tree.tree.arena.get_mut(nid).expect("arena has node");
    let area = node.get_mut().glyph_area_mut().expect("node is a GlyphArea");
    area.shape = NodeShape::Ellipse;
}

fn node_bounds(tree: &baumhard::mindmap::tree_builder::MindMapTree, node_id: &str) -> (Vec2, Vec2) {
    let nid = tree.arena_id_for(node_id).unwrap();
    let area = tree.tree.arena.get(nid).unwrap().get().glyph_area().unwrap();
    (
        Vec2::new(area.position.x.0, area.position.y.0),
        Vec2::new(area.render_bounds.x.0, area.render_bounds.y.0),
    )
}

#[test]
fn test_hit_test_ellipse_center_hits() {
    let mut tree = load_test_tree();
    set_node_shape_ellipse(&mut tree, "0");
    let (pos, bounds) = node_bounds(&tree, "0");
    let center = aabb_center(pos, bounds);
    assert_eq!(hit_test(center, &mut tree), Some("0".to_string()));
}

#[test]
fn test_hit_test_ellipse_aabb_corner_misses() {
    let mut tree = load_test_tree();
    set_node_shape_ellipse(&mut tree, "0");
    let (pos, _bounds) = node_bounds(&tree, "0");
    // Epsilon just inside the AABB corner — rectangle would
    // pick it, ellipse must not.
    let near_corner = pos + Vec2::new(0.5, 0.5);
    let hit = hit_test(near_corner, &mut tree);
    assert_ne!(
        hit,
        Some("0".to_string()),
        "Ellipse hit-test must reject AABB corner clicks"
    );
}

#[test]
fn test_point_in_node_aabb_is_shape_aware() {
    let mut tree = load_test_tree();
    set_node_shape_ellipse(&mut tree, "0");
    let (pos, bounds) = node_bounds(&tree, "0");
    let center = aabb_center(pos, bounds);
    let near_corner = pos + Vec2::new(0.5, 0.5);
    assert!(
        point_in_node_aabb(center, "0", &tree),
        "Center must count as inside the ellipse"
    );
    assert!(
        !point_in_node_aabb(near_corner, "0", &tree),
        "AABB corner must NOT count as inside the ellipse"
    );
}

#[test]
fn test_rect_select_ignores_ellipse_aabb_corner_only() {
    let mut tree = load_test_tree();
    set_node_shape_ellipse(&mut tree, "0");
    let (pos, _bounds) = node_bounds(&tree, "0");
    // Tiny selection rect tucked into the top-left corner —
    // inside the AABB, outside the ellipse. The old pure-AABB
    // `rect_select` would have returned "0"; the shape-aware
    // version must not.
    let hits = rect_select(pos, pos + Vec2::new(2.0, 2.0), &tree);
    assert!(
        !hits.contains(&"0".to_string()),
        "Rect-select inside ellipse's AABB corner must miss"
    );
}

#[test]
fn test_rect_select_still_catches_ellipse_through_center() {
    let mut tree = load_test_tree();
    set_node_shape_ellipse(&mut tree, "0");
    let (pos, bounds) = node_bounds(&tree, "0");
    // Selection rect crossing the center of the ellipse must
    // still register a hit.
    let center = aabb_center(pos, bounds);
    let hits = rect_select(center - Vec2::new(5.0, 5.0), center + Vec2::new(5.0, 5.0), &tree);
    assert!(
        hits.contains(&"0".to_string()),
        "Rect-select crossing the ellipse center must hit"
    );
}

/// `point_in_node_aabb` consults the cached subtree AABB so a
/// click on a section that overflows the container's AABB still
/// counts as "inside the node". Pre-fix, the function read only
/// the container's render_bounds, leaving overflowing sections
/// unhittable. Anchors the documented "click-outside-commit"
/// gesture for multi-section nodes.
#[test]
fn test_point_in_node_aabb_includes_overflowing_section() {
    use super::tests_common::doc_with_one_orphan_node;
    use baumhard::mindmap::model::{MindSection, Position, Size};
    let mut doc = doc_with_one_orphan_node();
    // Append a second section positioned past the container's
    // right edge. The container's AABB is [0,0]→[240,60]; the
    // overflow section sits at offset (300, 0) with its own
    // 100×40 size, putting its left edge well past the container.
    {
        let node = doc.mindmap.nodes.get_mut("0").unwrap();
        let mut overflow = MindSection::new_default("over".into(), vec![]);
        overflow.offset = Position { x: 300.0, y: 0.0 };
        overflow.size = Some(Size {
            width: 100.0,
            height: 40.0,
        });
        node.sections.push(overflow);
    }
    let mut tree = doc.build_tree();
    // Force the subtree-AABB cache to populate. The runtime hot
    // path invalidates / recomputes on every render and hit-test;
    // tests have to ask for it explicitly.
    tree.tree.ensure_subtree_aabbs();

    // A point well inside the overflow section but outside the
    // container's own AABB.
    let in_overflow = Vec2::new(350.0, 20.0);
    assert!(
        point_in_node_aabb(in_overflow, "0", &tree),
        "point inside overflow section must register as inside node 0"
    );
    // A point that's outside both the container AND the overflow
    // section's bounding box must still miss.
    let outside_all = Vec2::new(500.0, 200.0);
    assert!(
        !point_in_node_aabb(outside_all, "0", &tree),
        "point outside both container and overflow must miss"
    );
}

/// `collect_patches_recursive` (drag delta) only emits patches
/// for `GlyphArea`-bearing elements; `GlyphModel` siblings of
/// section-areas (the structural seam children) are skipped so
/// `patch_drag_positions` doesn't pay a cold hash miss per drag
/// tick on every section's model child. Pins the filter — a
/// future refactor that re-flattens the conditional would
/// silently regress drag performance and the test would fail.
#[test]
fn test_collect_patches_recursive_skips_glyph_model_children() {
    use crate::application::document::apply_drag_delta_and_collect_patches;
    use crate::application::document::tests_common::doc_with_one_orphan_node;
    use baumhard::core::primitives::{Flag, Flaggable};
    use baumhard::mindmap::model::MindSection;

    let mut doc = doc_with_one_orphan_node();
    {
        // Multi-section node — 3 sections × (section-area +
        // section-model) = 6 arena entries. Patches should
        // count only the 3 section-areas + 1 container = 4.
        let node = doc.mindmap.nodes.get_mut("0").unwrap();
        node.sections
            .push(MindSection::new_default("two".into(), Vec::new()));
        node.sections
            .push(MindSection::new_default("three".into(), Vec::new()));
    }
    let mut tree = doc.build_tree();
    let mut patches: Vec<(usize, (f32, f32))> = Vec::new();
    apply_drag_delta_and_collect_patches(&mut tree, "0", 5.0, 7.0, false, &mut patches);

    // Every patch's unique_id must correspond to a GlyphArea-
    // bearing element in the arena. GlyphModel elements (the
    // structural section-model siblings) carry the
    // `Flag::SectionRoot` marker but no glyph_area; verify
    // none of those leaked into the patch list.
    for (uid, _pos) in &patches {
        let element = tree
            .tree
            .arena
            .iter()
            .find(|n| n.get().unique_id() == *uid)
            .map(|n| n.get())
            .expect("patch references a real arena entry");
        assert!(
            element.glyph_area().is_some(),
            "patch with unique_id {uid} references a non-GlyphArea element ({:?})",
            element.flag_is_set(Flag::SectionRoot)
        );
    }
    // 1 container + 3 section-areas = 4 GlyphArea entries; the
    // 3 section-models are skipped.
    assert_eq!(
        patches.len(),
        4,
        "expected 4 patches (1 container + 3 section-areas), got {}",
        patches.len()
    );
}

// --- Section resize-handle hit tests ---

#[test]
fn test_hit_test_section_resize_handle_returns_none_for_none_sized_section() {
    use crate::application::document::hit_test_section_resize_handle;
    use crate::application::document::tests_common::doc_with_one_orphan_node;
    use glam::Vec2;

    let doc = doc_with_one_orphan_node();
    // Default orphan section has `size = None` — fill-parent
    // sections emit no resize handles, so the hit-test must
    // return `None` regardless of cursor position.
    let result = hit_test_section_resize_handle(&doc.mindmap, Vec2::new(0.0, 0.0), "0", 0, 100.0);
    assert!(result.is_none(), "fill-parent section must not surface handles");
}

#[test]
fn test_hit_test_section_resize_handle_returns_none_for_missing_node_or_section() {
    use crate::application::document::hit_test_section_resize_handle;
    use crate::application::document::tests_common::pinned_two_section_node;
    use glam::Vec2;

    let (doc, _id) = pinned_two_section_node();
    assert!(
        hit_test_section_resize_handle(&doc.mindmap, Vec2::ZERO, "nope", 0, 100.0).is_none(),
        "missing node id must return None"
    );
    let (doc, id) = pinned_two_section_node();
    assert!(
        hit_test_section_resize_handle(&doc.mindmap, Vec2::ZERO, &id, 99, 100.0).is_none(),
        "out-of-range section_idx must return None"
    );
}

#[test]
fn test_hit_test_section_resize_handle_lands_on_se_corner() {
    use crate::application::document::hit_test_section_resize_handle;
    use crate::application::document::tests_common::pinned_two_section_node;
    use baumhard::mindmap::tree_builder::ResizeHandleSide;
    use glam::Vec2;

    let (doc, id) = pinned_two_section_node();
    let node = &doc.mindmap.nodes[&id];
    // pinned_two_section_node fixes section[1] at offset (10,10)
    // size 50×30. SE corner is at canvas (node.pos + (60, 40)).
    let np = &node.position;
    let se = Vec2::new(np.x as f32 + 60.0, np.y as f32 + 40.0);
    let result = hit_test_section_resize_handle(&doc.mindmap, se, &id, 1, 4.0);
    assert_eq!(result, Some(ResizeHandleSide::SE));
}

#[test]
fn test_hit_test_section_resize_handle_lands_on_n_edge_midpoint() {
    use crate::application::document::hit_test_section_resize_handle;
    use crate::application::document::tests_common::pinned_two_section_node;
    use baumhard::mindmap::tree_builder::ResizeHandleSide;
    use glam::Vec2;

    let (doc, id) = pinned_two_section_node();
    let node = &doc.mindmap.nodes[&id];
    // N midpoint at offset (10+25, 10) = (35, 10) relative to node.
    let np = &node.position;
    let n = Vec2::new(np.x as f32 + 35.0, np.y as f32 + 10.0);
    assert_eq!(
        hit_test_section_resize_handle(&doc.mindmap, n, &id, 1, 4.0),
        Some(ResizeHandleSide::N)
    );
}

/// A section under a folded ancestor surfaces no resize handles
/// (scene-builder gates on `is_hidden_by_fold`); the hit-test
/// must mirror that gate so a stale `Section` selection that
/// survived a fold can't capture phantom handle presses.
#[test]
fn test_hit_test_section_resize_handle_returns_none_for_hidden_by_fold() {
    use crate::application::document::hit_test_section_resize_handle;
    use crate::application::document::tests_common::pinned_two_section_node;
    use glam::Vec2;

    let (mut doc, id) = pinned_two_section_node();
    // Find an ancestor and fold it so the target node is hidden.
    let parent_id = doc.mindmap.nodes[&id].parent_id.clone();
    if let Some(pid) = parent_id {
        if let Some(p) = doc.mindmap.nodes.get_mut(&pid) {
            p.folded = true;
        }
    } else {
        // No parent — fold the node itself; `is_hidden_by_fold`
        // returns false for the folded node itself but true for
        // its descendants. For the no-parent case fold isn't
        // visible; skip the assertion.
        return;
    }
    let node = &doc.mindmap.nodes[&id];
    let np = &node.position;
    let se = Vec2::new(np.x as f32 + 60.0, np.y as f32 + 40.0);
    assert!(
        hit_test_section_resize_handle(&doc.mindmap, se, &id, 1, 4.0).is_none(),
        "fold-hidden section must not surface handles"
    );
}

#[test]
fn test_hit_test_section_resize_handle_misses_outside_tolerance() {
    use crate::application::document::hit_test_section_resize_handle;
    use crate::application::document::tests_common::pinned_two_section_node;
    use glam::Vec2;

    let (doc, id) = pinned_two_section_node();
    let node = &doc.mindmap.nodes[&id];
    // Cursor is centered on the section AABB — far from every
    // corner / edge handle.
    let center = Vec2::new(node.position.x as f32 + 35.0, node.position.y as f32 + 25.0);
    assert!(
        hit_test_section_resize_handle(&doc.mindmap, center, &id, 1, 4.0).is_none(),
        "center of section must not hit any handle"
    );
}

// --- Node resize-handle hit + tree-mutation tests ---

#[test]
fn test_hit_test_node_resize_handle_lands_on_se_corner() {
    use crate::application::document::hit_test_node_resize_handle;
    use crate::application::document::tests_common::pinned_two_section_node;
    use baumhard::mindmap::tree_builder::ResizeHandleSide;
    use glam::Vec2;

    let (doc, id) = pinned_two_section_node();
    let node = &doc.mindmap.nodes[&id];
    let np = &node.position;
    let nw = &node.size;
    let se = Vec2::new(np.x as f32 + nw.width as f32, np.y as f32 + nw.height as f32);
    assert_eq!(
        hit_test_node_resize_handle(&doc.mindmap, se, &id, 4.0),
        Some(ResizeHandleSide::SE)
    );
}

#[test]
fn test_hit_test_node_resize_handle_misses_outside_tolerance() {
    use crate::application::document::hit_test_node_resize_handle;
    use crate::application::document::tests_common::pinned_two_section_node;

    let (doc, id) = pinned_two_section_node();
    let center = doc.mindmap.nodes[&id].center_vec2();
    assert!(
        hit_test_node_resize_handle(&doc.mindmap, center, &id, 4.0).is_none(),
        "center of node must not hit any handle"
    );
}

#[test]
fn test_hit_test_node_resize_handle_returns_none_for_missing_node() {
    use crate::application::document::hit_test_node_resize_handle;
    use crate::application::document::tests_common::pinned_two_section_node;
    use glam::Vec2;

    let (doc, _id) = pinned_two_section_node();
    assert!(
        hit_test_node_resize_handle(&doc.mindmap, Vec2::ZERO, "nope", 100.0).is_none(),
        "missing node id must return None"
    );
}

#[test]
fn test_hit_test_node_resize_handle_returns_none_for_hidden_by_fold() {
    use crate::application::document::hit_test_node_resize_handle;
    use crate::application::document::tests_common::pinned_two_section_node;
    use glam::Vec2;

    let (mut doc, id) = pinned_two_section_node();
    let parent_id = doc.mindmap.nodes[&id].parent_id.clone();
    if let Some(pid) = parent_id {
        if let Some(p) = doc.mindmap.nodes.get_mut(&pid) {
            p.folded = true;
        }
    } else {
        return;
    }
    let node = &doc.mindmap.nodes[&id];
    let se = Vec2::new(
        node.position.x as f32 + node.size.width as f32,
        node.position.y as f32 + node.size.height as f32,
    );
    assert!(
        hit_test_node_resize_handle(&doc.mindmap, se, &id, 4.0).is_none(),
        "fold-hidden node must not surface handles"
    );
}

#[test]
fn test_apply_node_resize_to_tree_writes_position_and_bounds() {
    use crate::application::document::apply_node_resize_to_tree;
    use crate::application::document::tests_common::pinned_two_section_node;
    use glam::Vec2;

    let (doc, id) = pinned_two_section_node();
    let mut tree = doc.build_tree();
    let new_pos = Vec2::new(500.0, 200.0);
    let new_size = Vec2::new(150.0, 70.0);
    let mut patches = Vec::new();
    let written = apply_node_resize_to_tree(&mut tree, &id, new_pos, new_size, Vec2::ZERO, &mut patches);
    let arena_id = tree.arena_id_for(&id).unwrap();
    assert_eq!(written, Some(arena_id));
    assert!(
        patches.is_empty(),
        "a zero position delta moves no section, so nothing needs a position patch"
    );
    let area = tree
        .tree
        .arena
        .get(arena_id)
        .and_then(|n| n.get().glyph_area())
        .unwrap();
    assert!((area.position.x.0 - 500.0).abs() < 0.001);
    assert!((area.position.y.0 - 200.0).abs() < 0.001);
    assert!((area.render_bounds.x.0 - 150.0).abs() < 0.001);
    assert!((area.render_bounds.y.0 - 70.0).abs() < 0.001);
}

/// Non-zero `position_delta` shifts the node's section children
/// but leaves child mind-node containers in place. Pre-fix the
/// helper walked every descendant including child mind-nodes,
/// so a NW-handle drag visually translated the entire descendant
/// subtree mid-drag.
#[test]
fn test_apply_node_resize_to_tree_shifts_sections_but_not_child_nodes() {
    use crate::application::document::apply_node_resize_to_tree;
    use crate::application::document::tests_common::pinned_two_section_node;
    use glam::Vec2;

    let (doc, id) = pinned_two_section_node();
    // Pick a child mind-node id (any non-`id` node in the testament map)
    // and snapshot its tree-side position pre-resize.
    let child_id = doc
        .mindmap
        .nodes
        .keys()
        .find(|k| k.as_str() != id)
        .expect("testament map has more than one node")
        .clone();

    let mut tree = doc.build_tree();
    let child_arena_id = tree.arena_id_for(&child_id).unwrap();
    let child_pos_before = tree
        .tree
        .arena
        .get(child_arena_id)
        .and_then(|n| n.get().glyph_area())
        .map(|a| (a.position.x.0, a.position.y.0))
        .unwrap();

    let new_pos = Vec2::new(500.0, 200.0);
    let new_size = Vec2::new(150.0, 70.0);
    let position_delta = Vec2::new(50.0, 25.0);
    let mut patches = Vec::new();
    apply_node_resize_to_tree(&mut tree, &id, new_pos, new_size, position_delta, &mut patches);

    // Child mind-node should be untouched — even when it's not
    // a descendant of `id`, the helper should never have walked
    // its arena entry. Pin the invariant.
    let child_pos_after = tree
        .tree
        .arena
        .get(child_arena_id)
        .and_then(|n| n.get().glyph_area())
        .map(|a| (a.position.x.0, a.position.y.0))
        .unwrap();
    assert_eq!(
        child_pos_before, child_pos_after,
        "node resize must not visually translate other mind-nodes' tree state"
    );

    // The resized node's container *did* move.
    let resized_arena_id = tree.arena_id_for(&id).unwrap();
    let resized_pos = tree
        .tree
        .arena
        .get(resized_arena_id)
        .and_then(|n| n.get().glyph_area())
        .map(|a| (a.position.x.0, a.position.y.0))
        .unwrap();
    assert!((resized_pos.0 - 500.0).abs() < 0.001);
    assert!((resized_pos.1 - 200.0).abs() < 0.001);
}

/// The node-resize drag no longer re-shapes the whole arena per
/// frame: it re-shapes the container this call returns and patches
/// the positions this call reports. That split is only safe while
/// those two outputs *between them name every element the call
/// touched* — and the two halves fail differently. An element moved
/// but unreported keeps a stale buffer position for the rest of the
/// drag; an element whose *shaping* inputs changed (bounds, text,
/// styled runs) but which is only reported as a patch keeps a stale
/// buffer, full stop, because `patch_drag_positions` rewrites a
/// position and shapes nothing. Neither is visible to any pixel
/// assertion in this repo (§T8).
///
/// So this walks the whole arena and snapshots every element's whole
/// `GlyphArea`, then asserts both halves across the call:
///
/// - the set of elements whose area changed at all is exactly
///   `{container} ∪ patched`, so nothing is touched unreported;
/// - every *patched* element changed its `position` and **nothing
///   else**, so a position patch is a sufficient refresh for it.
///
/// A future edit that shifts one more element without emitting its
/// patch fails the first; one that reshapes an element it only
/// patches fails the second.
#[test]
fn test_node_resize_reports_every_element_it_moved() {
    use crate::application::document::apply_node_resize_to_tree;
    use crate::application::document::tests_common::pinned_two_section_node;
    use baumhard::gfx_structs::area::GlyphArea;
    use glam::Vec2;
    use std::collections::{HashMap, HashSet};

    let (doc, id) = pinned_two_section_node();
    let mut tree = doc.build_tree();

    // Every input the renderer's shaper reads off an element is its
    // `GlyphArea` (see `shape_one_element_into_buffers`), so the
    // whole area is the snapshot. Compared through derived `Debug`
    // rather than `PartialEq`: the latter is deliberately blind to a
    // region recolor, which is exactly the class of silent reshape
    // this is here to catch.
    let described = |area: &GlyphArea| format!("{area:?}");
    let areas = |tree: &baumhard::mindmap::tree_builder::MindMapTree| {
        let mut out: HashMap<usize, GlyphArea> = HashMap::new();
        for arena_id in tree.tree.root().descendants(&tree.tree.arena) {
            let Some(element) = tree.tree.arena.get(arena_id).map(|n| n.get()) else {
                continue;
            };
            if let Some(area) = element.glyph_area() {
                out.insert(element.unique_id(), area.clone());
            }
        }
        out
    };
    let positions = |tree: &baumhard::mindmap::tree_builder::MindMapTree| {
        areas(tree)
            .into_iter()
            .map(|(uid, area)| (uid, (area.position.x.0, area.position.y.0)))
            .collect::<HashMap<usize, (f32, f32)>>()
    };

    let before_areas = areas(&tree);
    let container_uid = tree
        .tree
        .arena
        .get(tree.arena_id_for(&id).unwrap())
        .map(|n| n.get().unique_id())
        .unwrap();

    let mut patches = Vec::new();
    let written = apply_node_resize_to_tree(
        &mut tree,
        &id,
        Vec2::new(500.0, 200.0),
        Vec2::new(150.0, 70.0),
        Vec2::new(50.0, 25.0),
        &mut patches,
    );
    assert_eq!(written, Some(tree.arena_id_for(&id).unwrap()));

    let after_areas = areas(&tree);
    let after = positions(&tree);
    // Union both key sets rather than walking `after_areas` alone, so an
    // element the call *removed* counts as touched. Resize removes nothing
    // today; the union is here so the next edit that does cannot slip past.
    let touched: HashSet<usize> = before_areas
        .keys()
        .chain(after_areas.keys())
        .copied()
        .filter(|uid| before_areas.get(uid).map(described) != after_areas.get(uid).map(described))
        .collect();
    let patched: HashSet<usize> = patches.iter().map(|(uid, _)| *uid).collect();

    assert!(
        !patched.is_empty(),
        "the fixture must have sections to move, or this test proves nothing"
    );
    assert!(
        !touched.contains(&container_uid) || touched.len() > 1,
        "the fixture must touch more than the container, or this test proves nothing"
    );

    let mut reported = patched.clone();
    reported.insert(container_uid);
    assert_eq!(
        touched, reported,
        "every element the resize wrote to must be either the returned container \
         or a reported patch — an unreported write leaves a stale buffer"
    );

    // The reported position has to be the one actually written, not
    // merely the right element: the caller writes it straight into
    // the buffer without re-reading the arena.
    for (uid, pos) in &patches {
        assert_eq!(
            after.get(uid),
            Some(pos),
            "patch for {uid} names the wrong position"
        );
    }

    // The reshape half. `patch_drag_positions` moves an existing
    // buffer and shapes nothing, so a patched element must have
    // changed its position and nothing else: rewinding the position
    // to its pre-call value has to make the whole area identical
    // again. An element that also grew, re-wrapped or restyled needs
    // a `reshape_buffer_for`, not a patch, and lands here.
    for uid in &patched {
        let before_area = before_areas
            .get(uid)
            .expect("patched element existed before the call");
        let mut rewound = after_areas
            .get(uid)
            .expect("patched element still exists")
            .clone();
        rewound.set_position((before_area.position.x.0, before_area.position.y.0));
        assert_eq!(
            described(&rewound),
            described(before_area),
            "element {uid} is reported as a position patch, but the resize changed more than its \
             position — a patch cannot refresh that, only a re-shape can"
        );
    }
}

#[test]
fn test_apply_node_resize_to_tree_unknown_node_no_op() {
    use crate::application::document::apply_node_resize_to_tree;
    use crate::application::document::tests_common::pinned_two_section_node;
    use glam::Vec2;

    let (doc, _id) = pinned_two_section_node();
    let mut tree = doc.build_tree();
    // No panic; tree untouched. A stale entry left in `patches`
    // would make the caller patch a buffer this call never moved,
    // so the clear-on-entry contract is asserted with a non-empty
    // input rather than a fresh `Vec`.
    let mut patches = vec![(usize::MAX, (1.0, 2.0))];
    let written = apply_node_resize_to_tree(
        &mut tree,
        "nope",
        Vec2::ZERO,
        Vec2::new(10.0, 10.0),
        Vec2::ZERO,
        &mut patches,
    );
    assert_eq!(written, None);
    assert!(patches.is_empty());
}

/// A long `parent_id` chain is legal and acyclic, so the loader
/// accepts it and the scene tree inherits its depth.
/// `walk_drag_subtree` used to recurse over that depth, so opening
/// such a map and dragging a node killed the process with `SIGABRT`
/// — not a panic, so nothing could catch, log, or degrade it.
///
/// This is the same class the loader-side walkers were converted
/// for, on the one walker that conversion missed. It sits on the far
/// side of both: the load and the scene build already succeed on
/// this map because they are iterative, and the *drag* was what
/// died — which is exactly what made it easy to miss.
///
/// The walk runs on a 256 KiB stack rather than a test thread's
/// default couple of megabytes. That is what keeps this both fast
/// and honest: 20,000 nodes is cheap to build, and any reintroduced
/// recursion blows a stack that small long before the chain ends,
/// while the iterative form is indifferent to it.
///
/// A failure here aborts the test binary rather than failing an
/// assertion — that is what a stack overflow does, and it is
/// precisely the outcome under test.
#[test]
fn test_deep_chain_drag_does_not_exhaust_the_stack() {
    const DEPTH: usize = 20_000;
    const SMALL_STACK: usize = 256 * 1024;

    let nodes = (0..DEPTH)
        .map(|i| {
            let parent = if i == 0 {
                "null".to_string()
            } else {
                format!("\"n{}\"", i - 1)
            };
            format!(
                r##""n{i}": {{
                    "id": "n{i}", "parent_id": {parent},
                    "position": {{"x": 0, "y": 0}},
                    "size": {{"width": 100, "height": 50}},
                    "sections": [{{"text": "n"}}],
                    "style": {{"background_color":"#000","frame_color":"#000",
                              "text_color":"#fff","shape":"rectangle",
                              "corner_radius_percent":0,"frame_thickness":0,
                              "show_frame":false,"show_shadow":false}},
                    "layout": {{"type":"map","direction":"auto","spacing":0}},
                    "folded": false, "notes": ""
                }}"##
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let json = format!(
        r##"{{
            "version": "1.0",
            "name": "deep",
            "canvas": {{"background_color": "#000"}},
            "nodes": {{{nodes}}},
            "edges": []
        }}"##
    );
    let map = baumhard::mindmap::loader::load_from_str(&json).expect("a deep chain is acyclic and loads");

    let moved = std::thread::Builder::new()
        .stack_size(SMALL_STACK)
        .spawn(move || {
            let mut tree = baumhard::mindmap::tree_builder::build_mindmap_tree(&map);
            let arena_nodes = tree.tree.arena.len();
            // `include_descendants = true` is the whole point: it is
            // what routes into `walk_drag_subtree` and inherits the
            // chain's depth. The patch-collecting variant shares that
            // body, so covering one covers both.
            let mut patches = Vec::new();
            apply_drag_delta_and_collect_patches(&mut tree, "n0", 1.0, 2.0, true, &mut patches);
            (arena_nodes, patches.len())
        })
        .expect("spawn the small-stack drag")
        .join()
        .expect("the drag walk must not exhaust a 256 KiB stack");

    let (arena_nodes, patched) = moved;
    assert!(
        arena_nodes >= DEPTH,
        "the scene tree must carry every node: {arena_nodes} < {DEPTH}"
    );
    assert!(
        patched >= DEPTH,
        "dragging the root must move every descendant: {patched} < {DEPTH}"
    );
}

/// The recursive form `walk_drag_subtree` replaced, kept as the
/// oracle for the test below.
///
/// Deliberately recursive in a file whose subject is removing
/// recursion: it is the specification the iterative rewrite has to
/// match, and it is only ever called on the small fixture tree, so
/// its depth is a handful of levels. Do not call it on a
/// caller-supplied map.
fn reference_recursive_walk(
    arena: &mut indextree::Arena<baumhard::gfx_structs::element::GfxElement>,
    node_id: indextree::NodeId,
    dx: f32,
    dy: f32,
    mut patches: Option<&mut Vec<(usize, (f32, f32))>>,
) {
    if let Some(node) = arena.get_mut(node_id) {
        let elem = node.get_mut();
        if let Some(area) = elem.glyph_area_mut() {
            area.move_position(dx, dy);
            if let Some(p) = patches.as_deref_mut() {
                let pos = elem.position();
                p.push((elem.unique_id(), (pos.x, pos.y)));
            }
        }
    }
    let mut child = arena.get(node_id).and_then(|n| n.first_child());
    while let Some(cid) = child {
        child = arena.get(cid).and_then(|n| n.next_sibling());
        reference_recursive_walk(arena, cid, dx, dy, patches.as_deref_mut());
    }
}

/// The iterative drag walk must emit exactly the sequence the
/// recursion emitted — not merely the same set.
///
/// Not because the renderer needs it: `patch_drag_positions` looks
/// each patch up by `unique_id` in a map, so it is keyed and
/// order-independent, and a reordered buffer would render
/// identically. An earlier version of this comment claimed the
/// consumer was positional, which was simply wrong.
///
/// The order is pinned because it is what makes the
/// recursion→iteration rewrite *checkable*. "Same set" is a
/// property a green suite already implies; "same sequence" is a
/// statement about this walk against the one it replaced, and it is
/// the only cheap evidence that the rewrite changed nothing. If a
/// future change has a reason to reorder, this test is the place to
/// argue it — not an obstacle, but a prompt.
///
/// Zero delta is included on purpose: it makes the test sensitive to
/// visit order alone, with no positional arithmetic to mask a
/// divergence.
#[test]
fn test_iterative_drag_matches_the_recursive_order() {
    for (dx, dy) in [(1.0f32, 2.0f32), (-7.5, 0.25), (0.0, 0.0)] {
        for id in ["0", "1", "2"] {
            let mut iterative_tree = load_test_tree();
            let mut reference_tree = load_test_tree();
            let Some(root) = reference_tree.arena_id_for(id) else {
                continue;
            };

            let mut got = Vec::new();
            apply_drag_delta_and_collect_patches(&mut iterative_tree, id, dx, dy, true, &mut got);

            let mut want = Vec::new();
            reference_recursive_walk(&mut reference_tree.tree.arena, root, dx, dy, Some(&mut want));

            assert_eq!(
                got, want,
                "patch sequence diverged for node {id} at delta ({dx}, {dy})"
            );
            assert!(
                !got.is_empty(),
                "node {id} must produce patches, or this proves nothing"
            );
        }
    }
}

// --- Custom mutation registry & application tests ---
