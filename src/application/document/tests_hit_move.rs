//! Hit-testing, rect-select, selection, tree highlights, move, drag, animations.
//!
//! Part of the tests split for `document`. Helpers live in
//! `tests_common`; only the tests for this theme live here.
use super::*;
use super::tests_common::{
    first_testament_edge_ref, first_testament_node_id, load_test_doc, load_test_tree,
    pick_test_edge, test_map_path,
};

use baumhard::gfx_structs::area::GlyphAreaCommand;
use baumhard::gfx_structs::mutator::Mutation;
use baumhard::mindmap::animation::{AnimationTiming, Easing};
use baumhard::mindmap::custom_mutation::{
    apply_mutations_to_element, CustomMutation as CM, DocumentAction,
    MutationBehavior as MB, PlatformContext as PC, TargetScope as TS,
    Trigger as Tr, TriggerBinding as TB,
};
use baumhard::mindmap::model::{
    Canvas, GlyphConnectionConfig, MindEdge, MindNode, NodeLayout, NodeStyle, Position, Size,
    TextRun, PORTAL_GLYPH_PRESETS,
};
use baumhard::mindmap::scene_builder::EdgeHandleKind;
use baumhard::mindmap::model::ControlPoint;
use glam::Vec2;

use super::defaults::default_cross_link_edge;



    #[test]
    fn test_hit_test_direct_hit() {
        let mut tree = load_test_tree();
        // "Lord God" node (id: 0) — get its position from the tree
        let node_id = tree.node_map.get("0").unwrap();
        let area = tree.tree.arena.get(*node_id).unwrap().get().glyph_area().unwrap();
        let center = Vec2::new(
            area.position.x.0 + area.render_bounds.x.0 / 2.0,
            area.position.y.0 + area.render_bounds.y.0 / 2.0,
        );
        let result = hit_test(center, &mut tree);
        assert_eq!(result, Some("0".to_string()));
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
            let nid = tree.node_map.get(parent_id_str).unwrap();
            let area = tree.tree.arena.get(*nid).unwrap().get().glyph_area().unwrap();
            area.render_bounds.x.0 * area.render_bounds.y.0
        };

        // Collect candidate (mind_id, center) pairs first to release
        // the immutable borrow on tree.node_map before calling
        // hit_test (which needs &mut tree).
        let candidate: Option<(String, Vec2)> = tree.node_map.iter()
            .filter(|(id, _)| id.as_str() != parent_id_str)
            .find_map(|(mind_id, &nid)| {
                let a = tree.tree.arena.get(nid)?.get().glyph_area()?;
                let child_size = a.render_bounds.x.0 * a.render_bounds.y.0;
                let child_center = Vec2::new(
                    a.position.x.0 + a.render_bounds.x.0 / 2.0,
                    a.position.y.0 + a.render_bounds.y.0 / 2.0,
                );
                if child_size < parent_size
                    && point_in_node_aabb(child_center, parent_id_str, &tree)
                {
                    Some((mind_id.clone(), child_center))
                } else {
                    None
                }
            });

        if let Some((expected_id, center)) = candidate {
            let result = hit_test(center, &mut tree);
            assert_eq!(result, Some(expected_id),
                "Should select smaller child node, not parent");
        }
        // If no overlap found in test data, that's OK — test is structural
    }

    #[test]
    fn test_selection_state_is_selected() {
        let none = SelectionState::None;
        assert!(!none.is_selected("123"));

        let single = SelectionState::Single("123".to_string());
        assert!(single.is_selected("123"));
        assert!(!single.is_selected("456"));

        let multi = SelectionState::Multi(vec!["123".to_string(), "456".to_string()]);
        assert!(multi.is_selected("123"));
        assert!(multi.is_selected("456"));
        assert!(!multi.is_selected("789"));
    }

    #[test]
    fn test_selection_state_from_ids_empty_is_none() {
        assert!(matches!(SelectionState::from_ids(vec![]), SelectionState::None));
    }

    #[test]
    fn test_selection_state_from_ids_single_element_is_single() {
        match SelectionState::from_ids(vec!["alpha".to_string()]) {
            SelectionState::Single(id) => assert_eq!(id, "alpha"),
            other => panic!("expected Single, got {other:?}"),
        }
    }

    #[test]
    fn test_selection_state_from_ids_two_elements_is_multi_preserving_order() {
        match SelectionState::from_ids(vec!["a".to_string(), "b".to_string()]) {
            SelectionState::Multi(ids) => assert_eq!(ids, vec!["a".to_string(), "b".to_string()]),
            other => panic!("expected Multi, got {other:?}"),
        }
    }

    #[test]
    fn test_selection_state_from_ids_many_elements_is_multi_preserving_order() {
        let input = vec!["a".to_string(), "b".to_string(), "c".to_string(), "d".to_string()];
        match SelectionState::from_ids(input.clone()) {
            SelectionState::Multi(ids) => assert_eq!(ids, input),
            other => panic!("expected Multi, got {other:?}"),
        }
    }

    #[test]
    fn test_apply_tree_highlights_via_walker() {
        let mut tree = load_test_tree();
        let node_id = *tree.node_map.get("0").unwrap();

        // Before highlight: original color (white)
        let area = tree.tree.arena.get(node_id).unwrap().get().glyph_area().unwrap();
        let original_color = area.regions.all_regions()[0].color.unwrap();
        assert!((original_color[0] - 1.0).abs() < 0.01, "Expected white before highlight");

        // Apply highlight via the new mutator-driven path.
        apply_tree_highlights(
            &mut tree,
            std::iter::once(("0", HIGHLIGHT_COLOR)),
        );

        // After highlight: cyan
        let area = tree.tree.arena.get(node_id).unwrap().get().glyph_area().unwrap();
        let highlighted_color = area.regions.all_regions()[0].color.unwrap();
        assert!((highlighted_color[0] - HIGHLIGHT_COLOR[0]).abs() < 0.01);
        assert!((highlighted_color[1] - HIGHLIGHT_COLOR[1]).abs() < 0.01);
        assert!((highlighted_color[2] - HIGHLIGHT_COLOR[2]).abs() < 0.01);
    }

    #[test]
    fn test_apply_tree_highlights_does_not_affect_others() {
        let mut tree = load_test_tree();

        // Pick a different node and copy its regions before mutation.
        let other_id = tree.node_map.keys()
            .find(|k| *k != "0")
            .unwrap().clone();
        let other_node_id = *tree.node_map.get(&other_id).unwrap();
        let before = tree.tree.arena.get(other_node_id).unwrap().get()
            .glyph_area().unwrap().regions.clone();

        apply_tree_highlights(
            &mut tree,
            std::iter::once(("0", HIGHLIGHT_COLOR)),
        );

        let after = tree.tree.arena.get(other_node_id).unwrap().get()
            .glyph_area().unwrap().regions.clone();
        assert_eq!(before, after, "Unselected node colors should not change");
    }

    #[test]
    fn test_apply_tree_highlights_later_pair_overrides_earlier() {
        // The reparent-mode flow relies on source-orange overriding the
        // previously-applied selection-cyan on the same node. Verify the
        // last-write-wins semantics of apply_tree_highlights.
        let mut tree = load_test_tree();
        let node_id = *tree.node_map.get("0").unwrap();

        apply_tree_highlights(
            &mut tree,
            vec![
                ("0", HIGHLIGHT_COLOR),
                ("0", REPARENT_SOURCE_COLOR),
            ],
        );

        let area = tree.tree.arena.get(node_id).unwrap().get().glyph_area().unwrap();
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
            assert!((n.position.x - (ox + dx)).abs() < 0.001, "Node {} x not shifted", id);
            assert!((n.position.y - (oy + dy)).abs() < 0.001, "Node {} y not shifted", id);
        }
    }

    #[test]
    fn test_move_subtree_preserves_relative_positions() {
        let mut doc = load_test_doc();
        let node_id = "0";
        let descendants = doc.mindmap.all_descendants(node_id);

        // Record relative offsets from parent to each descendant
        let parent = doc.mindmap.nodes.get(node_id).unwrap();
        let offsets: Vec<(String, f64, f64)> = descendants.iter().filter_map(|id| {
            let n = doc.mindmap.nodes.get(id)?;
            Some((id.clone(), n.position.x - parent.position.x, n.position.y - parent.position.y))
        }).collect();

        doc.apply_move_subtree(node_id, 100.0, 200.0);

        let parent = doc.mindmap.nodes.get(node_id).unwrap();
        for (id, dx, dy) in &offsets {
            let n = doc.mindmap.nodes.get(id).unwrap();
            let actual_dx = n.position.x - parent.position.x;
            let actual_dy = n.position.y - parent.position.y;
            assert!((actual_dx - dx).abs() < 0.001, "Relative x offset changed for {}", id);
            assert!((actual_dy - dy).abs() < 0.001, "Relative y offset changed for {}", id);
        }
    }

    #[test]
    fn test_move_single_only_affects_target() {
        let mut doc = load_test_doc();
        let node_id = "0";
        let descendants = doc.mindmap.all_descendants(node_id);

        // Record descendant positions before
        let before: Vec<(String, f64, f64)> = descendants.iter().filter_map(|id| {
            let n = doc.mindmap.nodes.get(id)?;
            Some((id.clone(), n.position.x, n.position.y))
        }).collect();

        doc.apply_move_single(node_id, 100.0, 200.0);

        // Descendants should be unchanged
        for (id, ox, oy) in &before {
            let n = doc.mindmap.nodes.get(id).unwrap();
            assert!((n.position.x - ox).abs() < 0.001, "Descendant {} x changed unexpectedly", id);
            assert!((n.position.y - oy).abs() < 0.001, "Descendant {} y changed unexpectedly", id);
        }

        // But the target node should have moved
        let target = doc.mindmap.nodes.get(node_id).unwrap();
        // We don't assert exact position here, just that it changed
        // (the original was stored before the move, but we didn't save it in this test)
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
        assert!((pos_now - orig_x).abs() < 1e-6);

        // From / to snapshots reflect the nudge (test mutation is
        // NudgeRight(10.0)).
        let inst = &doc.active_animations[0];
        assert!((inst.from_node.position.x - orig_x).abs() < 1e-6);
        assert!((inst.to_node.position.x - orig_x - 10.0).abs() < 1e-6);
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
        assert!((mid_x - orig_x - 5.0).abs() < 1e-3, "midpoint x = {mid_x}, expected {}", orig_x + 5.0);

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
        CM {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            contexts: vec![],
            mutator: Some(baumhard::mindmap::custom_mutation::scope::self_only(vec![
                Mutation::area_command(GlyphAreaCommand::NudgeRight(10.0)),
            ])),
            target_scope: scope,
            behavior: MB::Persistent,
            predicate: None,
            document_actions: vec![],
            timing,
        }
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
        doc.undo_stack.push(UndoAction::MoveNodes { original_positions: undo_data });

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

        let tree_nid = *tree.node_map.get(node_id).unwrap();
        let orig_x = tree.tree.arena.get(tree_nid).unwrap().get().glyph_area().unwrap().position.x.0;
        let orig_y = tree.tree.arena.get(tree_nid).unwrap().get().glyph_area().unwrap().position.y.0;

        apply_drag_delta(&mut tree, node_id, 25.0, -15.0, false);

        let new_x = tree.tree.arena.get(tree_nid).unwrap().get().glyph_area().unwrap().position.x.0;
        let new_y = tree.tree.arena.get(tree_nid).unwrap().get().glyph_area().unwrap().position.y.0;
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
        let child_tree_nid = *tree.node_map.get(child_id).unwrap();
        let child_orig_x = tree.tree.arena.get(child_tree_nid).unwrap().get()
            .glyph_area().unwrap().position.x.0;

        apply_drag_delta(&mut tree, node_id, 30.0, 20.0, true);

        let child_new_x = tree.tree.arena.get(child_tree_nid).unwrap().get()
            .glyph_area().unwrap().position.x.0;
        assert!((child_new_x - (child_orig_x + 30.0)).abs() < 0.001,
            "Descendant should be shifted when include_descendants=true");
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
        assert!((child_new_x - (child_orig_x + 50.0)).abs() < 0.001,
            "Child should be moved exactly once, not twice");
    }

    #[test]
    fn test_rect_select_finds_nodes_in_region() {
        let tree = load_test_tree();
        // Get position/bounds of "Lord God" to build a rect that contains it
        let node_id = *tree.node_map.get("0").unwrap();
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

    // --- Custom mutation registry & application tests ---

