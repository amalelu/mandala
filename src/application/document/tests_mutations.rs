//! Custom-mutation application, registry, trigger evaluation.
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



    fn make_test_mutation(id: &str, scope: TS) -> CM {
        CM {
            id: id.to_string(),
            name: id.to_string(),
            mutations: vec![
                Mutation::area_command(GlyphAreaCommand::NudgeRight(10.0)),
            ],
            target_scope: scope,
            behavior: MB::Persistent,
            predicate: None,
            document_actions: vec![],
            timing: None,
        }
    }

    /// Build a `CustomMutation` whose only payload is a single
    /// `SetThemeVariables` document-level action that sets `--bg`
    /// to the given value. Used by the `apply_document_actions`
    /// regression tests.
    fn make_set_bg_doc_mutation(value: &str) -> CM {
            let mut vars = HashMap::new();
        vars.insert("--bg".to_string(), value.to_string());
        CM {
            id: "set-bg".to_string(),
            name: "Set --bg".to_string(),
            mutations: vec![],
            target_scope: TS::SelfOnly,
            behavior: MB::Persistent,
            predicate: None,
            document_actions: vec![DocumentAction::SetThemeVariables(vars)],
            timing: None,
        }
    }

    /// Round-trip regression for `UndoAction::CanvasSnapshot`. The
    /// `apply_document_actions` path is the only producer of this
    /// variant, and prior to chunk 5 it had zero test coverage —
    /// CODE_CONVENTIONS.md §6 says every undo variant ships with at
    /// least a forward-and-back test.
    #[test]
    fn test_apply_document_actions_undo_round_trip() {
        let mut doc = load_test_doc();
        // Capture the canvas state before any document-level mutation.
        let before = doc.mindmap.canvas.clone();
        let undo_len_before = doc.undo_stack.len();

        // Apply a single SetThemeVariables action that sets --bg to a
        // sentinel value not present in the testament map.
        let custom = make_set_bg_doc_mutation("#bada55");
        let changed = doc.apply_document_actions(&custom);
        assert!(changed, "applying a new theme var must report a change");
        assert_eq!(
            doc.mindmap.canvas.theme_variables.get("--bg"),
            Some(&"#bada55".to_string())
        );
        assert_eq!(
            doc.undo_stack.len(),
            undo_len_before + 1,
            "exactly one CanvasSnapshot entry should have been pushed"
        );
        assert!(doc.dirty);

        // Undo restores the entire pre-mutation canvas wholesale.
        assert!(doc.undo());
        assert_eq!(doc.mindmap.canvas.theme_variables, before.theme_variables);
        assert_eq!(doc.mindmap.canvas.background_color, before.background_color);
        assert_eq!(
            doc.undo_stack.len(),
            undo_len_before,
            "undo should have popped the CanvasSnapshot entry"
        );
    }

    /// `apply_document_actions` returns false and pushes nothing
    /// when the action would not actually change anything (writing
    /// the same value that's already there). Guards the dirty/undo
    /// no-op path that the docstring on `apply_document_actions`
    /// promises.
    #[test]
    fn test_apply_document_actions_noop_does_not_push_undo() {
        let mut doc = load_test_doc();
        // First write — should change the canvas and push undo.
        let custom = make_set_bg_doc_mutation("#bada55");
        doc.apply_document_actions(&custom);
        let undo_len_after_first = doc.undo_stack.len();
        doc.dirty = false;

        // Second write of the same value — no-op, no undo push,
        // dirty flag should stay false.
        let changed = doc.apply_document_actions(&custom);
        assert!(!changed, "writing the same value must not report a change");
        assert_eq!(doc.undo_stack.len(), undo_len_after_first);
        assert!(!doc.dirty);
    }

    #[test]
    fn test_mutation_registry_empty_for_existing_map() {
        let doc = load_test_doc();
        assert!(doc.mutation_registry.is_empty(),
            "Existing map without custom_mutations should have empty registry");
    }

    #[test]
    fn test_mutation_registry_from_map_level() {
        let mut doc = load_test_doc();
        doc.mindmap.custom_mutations.push(make_test_mutation("nudge-right", TS::SelfOnly));
        doc.build_mutation_registry();
        assert_eq!(doc.mutation_registry.len(), 1);
        assert!(doc.mutation_registry.contains_key("nudge-right"));
    }

    #[test]
    fn test_mutation_registry_inline_overrides_map() {
        let mut doc = load_test_doc();
        // Map-level mutation
        let mut map_cm = make_test_mutation("shared-id", TS::SelfOnly);
        map_cm.name = "Map Version".to_string();
        doc.mindmap.custom_mutations.push(map_cm);

        // Inline mutation on a node with the same id
        let mut inline_cm = make_test_mutation("shared-id", TS::Children);
        inline_cm.name = "Inline Version".to_string();
        let node_id = "348068464";
        doc.mindmap.nodes.get_mut(node_id).unwrap().inline_mutations.push(inline_cm);

        doc.build_mutation_registry();
        assert_eq!(doc.mutation_registry.len(), 1);
        let cm = doc.mutation_registry.get("shared-id").unwrap();
        assert_eq!(cm.name, "Inline Version", "Inline should override map-level");
        assert_eq!(cm.target_scope, TS::Children);
    }

    #[test]
    fn test_find_triggered_mutations_match() {
        let mut doc = load_test_doc();
        doc.mindmap.custom_mutations.push(make_test_mutation("nudge", TS::SelfOnly));
        doc.build_mutation_registry();

        let node_id = "348068464";
        doc.mindmap.nodes.get_mut(node_id).unwrap().trigger_bindings.push(TB {
            trigger: Tr::OnClick,
            mutation_id: "nudge".to_string(),
            contexts: vec![],
        });

        let results = doc.find_triggered_mutations(node_id, &Tr::OnClick, &PC::Desktop);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "nudge");
    }

    #[test]
    fn test_find_triggered_mutations_no_match() {
        let mut doc = load_test_doc();
        doc.mindmap.custom_mutations.push(make_test_mutation("nudge", TS::SelfOnly));
        doc.build_mutation_registry();

        let node_id = "348068464";
        doc.mindmap.nodes.get_mut(node_id).unwrap().trigger_bindings.push(TB {
            trigger: Tr::OnClick,
            mutation_id: "nudge".to_string(),
            contexts: vec![],
        });

        // OnHover should not match
        let results = doc.find_triggered_mutations(node_id, &Tr::OnHover, &PC::Desktop);
        assert!(results.is_empty());
    }

    #[test]
    fn test_find_triggered_mutations_platform_filter() {
        let mut doc = load_test_doc();
        doc.mindmap.custom_mutations.push(make_test_mutation("desktop-only", TS::SelfOnly));
        doc.build_mutation_registry();

        let node_id = "348068464";
        doc.mindmap.nodes.get_mut(node_id).unwrap().trigger_bindings.push(TB {
            trigger: Tr::OnClick,
            mutation_id: "desktop-only".to_string(),
            contexts: vec![PC::Desktop],
        });

        // Desktop should match
        let results = doc.find_triggered_mutations(node_id, &Tr::OnClick, &PC::Desktop);
        assert_eq!(results.len(), 1);

        // Touch should be filtered out
        let results = doc.find_triggered_mutations(node_id, &Tr::OnClick, &PC::Touch);
        assert!(results.is_empty());
    }

    #[test]
    fn test_collect_affected_node_ids_self_only() {
        let doc = load_test_doc();
        let ids = doc.collect_affected_node_ids("348068464", &TS::SelfOnly);
        assert_eq!(ids, vec!["348068464"]);
    }

    #[test]
    fn test_collect_affected_node_ids_children() {
        let doc = load_test_doc();
        let children = doc.mindmap.children_of("348068464");
        let ids = doc.collect_affected_node_ids("348068464", &TS::Children);
        assert_eq!(ids.len(), children.len());
        for child in &children {
            assert!(ids.contains(&child.id));
        }
    }

    #[test]
    fn test_collect_affected_node_ids_descendants() {
        let doc = load_test_doc();
        let all_desc = doc.mindmap.all_descendants("348068464");
        let ids = doc.collect_affected_node_ids("348068464", &TS::Descendants);
        assert_eq!(ids.len(), all_desc.len());
    }

    #[test]
    fn test_collect_affected_node_ids_self_and_descendants() {
        let doc = load_test_doc();
        let all_desc = doc.mindmap.all_descendants("348068464");
        let ids = doc.collect_affected_node_ids("348068464", &TS::SelfAndDescendants);
        assert_eq!(ids.len(), all_desc.len() + 1);
        assert!(ids.contains(&"348068464".to_string()));
    }

    #[test]
    fn test_apply_custom_mutation_persistent_sets_dirty() {
        let mut doc = load_test_doc();
        let cm = make_test_mutation("nudge", TS::SelfOnly);
        doc.mindmap.custom_mutations.push(cm.clone());
        doc.build_mutation_registry();
        let mut tree = doc.build_tree();

        assert!(!doc.dirty);
        doc.apply_custom_mutation(&cm, "348068464", &mut tree);
        assert!(doc.dirty, "Persistent mutation should set dirty flag");
        assert_eq!(doc.undo_stack.len(), 1, "Should push undo action");
    }

    #[test]
    fn test_apply_custom_mutation_toggle_does_not_set_dirty() {
        let mut doc = load_test_doc();
        let mut cm = make_test_mutation("toggle-test", TS::SelfOnly);
        cm.behavior = MB::Toggle;
        doc.mindmap.custom_mutations.push(cm.clone());
        doc.build_mutation_registry();
        let mut tree = doc.build_tree();

        doc.apply_custom_mutation(&cm, "348068464", &mut tree);
        assert!(!doc.dirty, "Toggle mutation should not set dirty flag");
        assert!(doc.undo_stack.is_empty(), "Toggle mutation should not push undo");
        assert!(doc.active_toggles.contains(&("348068464".to_string(), "toggle-test".to_string())));
    }

    #[test]
    fn test_apply_custom_mutation_toggle_reverses() {
        let mut doc = load_test_doc();
        let mut cm = make_test_mutation("toggle-test", TS::SelfOnly);
        cm.behavior = MB::Toggle;
        doc.mindmap.custom_mutations.push(cm.clone());
        doc.build_mutation_registry();
        let mut tree = doc.build_tree();

        // First apply: activates toggle
        doc.apply_custom_mutation(&cm, "348068464", &mut tree);
        assert!(doc.active_toggles.contains(&("348068464".to_string(), "toggle-test".to_string())));

        // Second apply: deactivates toggle
        doc.apply_custom_mutation(&cm, "348068464", &mut tree);
        assert!(!doc.active_toggles.contains(&("348068464".to_string(), "toggle-test".to_string())));
    }

    #[test]
    fn test_undo_custom_mutation_restores_node() {
        let mut doc = load_test_doc();
        let cm = make_test_mutation("nudge", TS::SelfOnly);
        let node_id = "348068464";

        let orig_x = doc.mindmap.nodes.get(node_id).unwrap().position.x;
        let mut tree = doc.build_tree();

        doc.apply_custom_mutation(&cm, node_id, &mut tree);
        // Position may have been synced from tree; verify undo restores original
        assert!(doc.undo());
        let restored_x = doc.mindmap.nodes.get(node_id).unwrap().position.x;
        assert!((restored_x - orig_x).abs() < 0.001, "Undo should restore original position");
    }
