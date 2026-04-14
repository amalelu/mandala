//! Edge undo chain + remaining edge-setter coverage (second half).
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
    fn test_undo_chain_round_trips_multiple_edits() {
        let mut doc = load_test_doc();
        let er = first_testament_edge_ref(&doc);
        // Snapshot the starting state of the edge.
        let idx = doc.edge_index(&er).unwrap();
        let original = doc.mindmap.edges[idx].clone();
        doc.undo_stack.clear();

        // Apply three edits.
        assert!(doc.set_edge_body_glyph(&er, "\u{2500}"));
        assert!(doc.set_edge_color(&er, Some("#abcdef")));
        assert!(doc.set_edge_label(&er, Some("x".to_string())));
        assert_eq!(doc.undo_stack.len(), 3);

        // Undo all three in LIFO order.
        doc.undo();
        doc.undo();
        doc.undo();
        let restored = &doc.mindmap.edges[idx];
        assert_eq!(restored.label, original.label);
        assert_eq!(
            restored.glyph_connection.as_ref().map(|c| c.body.clone()),
            original.glyph_connection.as_ref().map(|c| c.body.clone())
        );
        assert_eq!(
            restored.glyph_connection.as_ref().and_then(|c| c.color.clone()),
            original.glyph_connection.as_ref().and_then(|c| c.color.clone())
        );
    }

    // -----------------------------------------------------------------
    // grow_node_sizes_to_fit_text — node box auto-sizing
    //
    // Grow-only pass that runs once at load time to ensure every node's
    // stored size is at least big enough to contain its text. These
    // tests lock in the three invariants described in the helper's
    // doc-comment: grow-only, idempotent, and consistent with rendering.
    // -----------------------------------------------------------------

    /// Build a synthetic single-node map with a given text and stored
    /// size. The text_runs carry a 14 pt scale to match the default.
    fn synthetic_single_node_map(text: &str, w: f64, h: f64) -> MindMap {
            let text_runs = vec![TextRun {
            start: 0,
            end: text.chars().count(),
            bold: false,
            italic: false,
            underline: false,
            font: "LiberationSans".to_string(),
            size_pt: 14,
            color: "#ffffff".to_string(),
            hyperlink: None,
        }];
        let node = MindNode {
            id: "n1".to_string(),
            parent_id: None,
            index: 0,
            position: Position { x: 0.0, y: 0.0 },
            size: Size { width: w, height: h },
            text: text.to_string(),
            text_runs,
            style: NodeStyle {
                background_color: "#141414".to_string(),
                frame_color: "#30b082".to_string(),
                text_color: "#ffffff".to_string(),
                shape_type: 0,
                corner_radius_percent: 10.0,
                frame_thickness: 4.0,
                show_frame: true,
                show_shadow: false,
                border: None,
            },
            layout: NodeLayout {
                layout_type: 0,
                direction: 0,
                spacing: 50.0,
            },
            folded: false,
            notes: String::new(),
            color_schema: None,
            trigger_bindings: Vec::new(),
            inline_mutations: Vec::new(),
        };
        let mut nodes = HashMap::new();
        nodes.insert("n1".to_string(), node);
        MindMap {
            version: "1.0".to_string(),
            name: "test".to_string(),
            canvas: Canvas {
                background_color: "#000000".to_string(),
                default_border: None,
                default_connection: None,
                theme_variables: HashMap::new(),
                theme_variants: HashMap::new(),
            },
            nodes,
            edges: Vec::new(),
            custom_mutations: Vec::new(),
            portals: Vec::new(),
        }
    }

    /// A stored box that's already comfortably larger than the text
    /// must be left alone — the helper is strictly grow-only.
    #[test]
    fn grow_node_sizes_to_fit_text_does_not_shrink() {
        let mut map = synthetic_single_node_map("Hi", 500.0, 500.0);
        grow_node_sizes_to_fit_text(&mut map);
        let n = map.nodes.get("n1").unwrap();
        assert_eq!(n.size.width, 500.0);
        assert_eq!(n.size.height, 500.0);
    }

    /// A tiny stored box with a long text must grow so the measured
    /// bounds (plus padding) fit inside. Exact measurements depend on
    /// available fonts, so assertions are lower-bound only.
    #[test]
    fn grow_node_sizes_to_fit_text_grows_undersized_boxes() {
        let long = "The quick brown fox jumps over the lazy dog a few times";
        let mut map = synthetic_single_node_map(long, 20.0, 20.0);
        grow_node_sizes_to_fit_text(&mut map);
        let n = map.nodes.get("n1").unwrap();
        // Definitely bigger than 20×20 — the long string shapes to at
        // least a few hundred pixels wide at 14 pt.
        assert!(
            n.size.width > 100.0,
            "expected grown width > 100, got {}",
            n.size.width
        );
        // Height grows by at least one line of text + pad_y (0.5 *
        // 14 = 7) → ≥ one line height (14 * 1.2 ≈ 16.8) + 7 ≈ 23.8.
        assert!(
            n.size.height >= 20.0,
            "expected height ≥ 20 (grow-only floor), got {}",
            n.size.height
        );
    }

    /// Running the pass twice must be a no-op the second time —
    /// after the first run, every node's stored size is already
    /// `>= measured + pad`, so the `max(stored, ...)` reduces to
    /// `stored` on the second call.
    #[test]
    fn grow_node_sizes_to_fit_text_is_idempotent() {
        let mut map = synthetic_single_node_map("Some text here", 10.0, 10.0);
        grow_node_sizes_to_fit_text(&mut map);
        let first_w = map.nodes.get("n1").unwrap().size.width;
        let first_h = map.nodes.get("n1").unwrap().size.height;
        grow_node_sizes_to_fit_text(&mut map);
        let second_w = map.nodes.get("n1").unwrap().size.width;
        let second_h = map.nodes.get("n1").unwrap().size.height;
        assert_eq!(first_w, second_w);
        assert_eq!(first_h, second_h);
    }

    // =====================================================================
    // Session 6E — portal mutation tests
    // =====================================================================

    #[test]
    fn portal_create_success_assigns_first_label() {
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();

        let pref = doc.apply_create_portal(&a, &b).expect("should succeed");
        assert_eq!(pref.label, "A");
        assert_eq!(pref.endpoint_a, a);
        assert_eq!(pref.endpoint_b, b);
        assert_eq!(doc.mindmap.portals.len(), 1);
        assert_eq!(doc.mindmap.portals[0].label, "A");
        assert!(doc.dirty);
    }

    #[test]
    fn portal_create_assigns_sequential_labels_a_b_c() {
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();
        let c = iter.next().unwrap().clone();

        let p1 = doc.apply_create_portal(&a, &b).unwrap();
        let p2 = doc.apply_create_portal(&b, &c).unwrap();
        let p3 = doc.apply_create_portal(&a, &c).unwrap();
        assert_eq!(p1.label, "A");
        assert_eq!(p2.label, "B");
        assert_eq!(p3.label, "C");
    }

    #[test]
    fn portal_create_assigns_rotating_glyphs() {
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();
        let c = iter.next().unwrap().clone();

        doc.apply_create_portal(&a, &b).unwrap();
        doc.apply_create_portal(&b, &c).unwrap();
        assert_eq!(doc.mindmap.portals[0].glyph, PORTAL_GLYPH_PRESETS[0]);
        assert_eq!(doc.mindmap.portals[1].glyph, PORTAL_GLYPH_PRESETS[1]);
    }

    #[test]
    fn portal_create_rejects_self_portal() {
        let mut doc = load_test_doc();
        let id = doc.mindmap.nodes.keys().next().unwrap().clone();
        let result = doc.apply_create_portal(&id, &id);
        assert!(result.is_err());
        assert!(doc.mindmap.portals.is_empty());
    }

    #[test]
    fn portal_create_rejects_unknown_node() {
        let mut doc = load_test_doc();
        let known = doc.mindmap.nodes.keys().next().unwrap().clone();
        assert!(doc.apply_create_portal(&known, "does_not_exist").is_err());
        assert!(doc.apply_create_portal("does_not_exist", &known).is_err());
        assert!(doc.mindmap.portals.is_empty());
    }

    #[test]
    fn portal_undo_create_removes_portal() {
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();

        let pref = doc.apply_create_portal(&a, &b).unwrap();
        doc.selection = SelectionState::Portal(pref);
        assert_eq!(doc.mindmap.portals.len(), 1);
        assert!(doc.undo());
        assert!(doc.mindmap.portals.is_empty());
        // Undoing a CreatePortal that was selected should clear the selection.
        assert!(matches!(doc.selection, SelectionState::None));
    }

    #[test]
    fn portal_delete_and_undo_restore_original_index() {
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();
        let c = iter.next().unwrap().clone();

        let p1 = doc.apply_create_portal(&a, &b).unwrap();
        let p2 = doc.apply_create_portal(&b, &c).unwrap();
        let _p3 = doc.apply_create_portal(&a, &c).unwrap();
        assert_eq!(doc.mindmap.portals.len(), 3);

        // Delete the middle portal.
        let removed = doc.apply_delete_portal(&p2).expect("should delete");
        assert_eq!(removed.label, "B");
        assert_eq!(doc.mindmap.portals.len(), 2);

        // Undo should slot it back at its original middle index.
        assert!(doc.undo());
        assert_eq!(doc.mindmap.portals.len(), 3);
        assert_eq!(doc.mindmap.portals[1].label, "B");
        // And the other portals should still be intact.
        assert_eq!(doc.mindmap.portals[0].label, p1.label);
    }

    #[test]
    fn portal_edit_glyph_and_undo_restores_before() {
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();

        let pref = doc.apply_create_portal(&a, &b).unwrap();
        let original_glyph = doc.mindmap.portals[0].glyph.clone();
        assert!(doc.set_portal_glyph(&pref, "\u{2B22}"));
        assert_eq!(doc.mindmap.portals[0].glyph, "\u{2B22}");
        assert!(doc.undo());
        assert_eq!(doc.mindmap.portals[0].glyph, original_glyph);
    }

    #[test]
    fn portal_edit_color_and_undo_restores_before() {
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();

        let pref = doc.apply_create_portal(&a, &b).unwrap();
        let original_color = doc.mindmap.portals[0].color.clone();
        assert!(doc.set_portal_color(&pref, "var(--accent)"));
        assert_eq!(doc.mindmap.portals[0].color, "var(--accent)");
        assert!(doc.undo());
        assert_eq!(doc.mindmap.portals[0].color, original_color);
    }

    #[test]
    fn portal_delete_returns_none_for_unknown_ref() {
        let mut doc = load_test_doc();
        let ghost = PortalRef::new("Z", "ghost_a", "ghost_b");
        assert!(doc.apply_delete_portal(&ghost).is_none());
    }

    #[test]
    fn portal_next_label_reuses_gap_after_delete() {
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();
        let c = iter.next().unwrap().clone();

        let _ = doc.apply_create_portal(&a, &b).unwrap();
        let p2 = doc.apply_create_portal(&b, &c).unwrap();
        let _ = doc.apply_create_portal(&a, &c).unwrap();
        // Delete the middle ("B").
        doc.apply_delete_portal(&p2).unwrap();
        // Next creation should reuse "B" since it is now the lowest unused.
        let d = doc.apply_create_portal(&b, &c).unwrap();
        assert_eq!(d.label, "B");
    }

    #[test]
    fn selection_state_portal_is_not_node_selection() {
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();
        let pref = doc.apply_create_portal(&a, &b).unwrap();
        doc.selection = SelectionState::Portal(pref.clone());

        assert!(!doc.selection.is_selected(&a));
        assert!(!doc.selection.is_selected(&b));
        assert!(doc.selection.selected_ids().is_empty());
        assert_eq!(doc.selection.selected_portal(), Some(&pref));
        assert_eq!(doc.selection.selected_edge(), None);
    }

    // -----------------------------------------------------------------
    // Session 7A: node text editing
    // -----------------------------------------------------------------

