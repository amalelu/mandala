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
            position: Position { x: 0.0, y: 0.0 },
            size: Size { width: w, height: h },
            text: text.to_string(),
            text_runs,
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
            palettes: HashMap::new(),
            nodes,
            edges: Vec::new(),
            custom_mutations: Vec::new(),
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
    // Portal-mode edge mutation tests
    //
    // Portals are edges with `display_mode = Some("portal")`. These
    // tests cover the create / delete / edit / undo chain through
    // the unified edge pipeline.
    // =====================================================================

    #[test]
    fn portal_edge_create_success_picks_first_glyph() {
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();

        let idx = doc.create_portal_edge(&a, &b).expect("should succeed");
        let edge = &doc.mindmap.edges[idx];
        assert_eq!(edge.from_id, a);
        assert_eq!(edge.to_id, b);
        assert_eq!(edge.edge_type, "cross_link");
        assert!(baumhard::mindmap::model::is_portal_edge(edge));
        assert_eq!(
            edge.glyph_connection.as_ref().unwrap().body,
            PORTAL_GLYPH_PRESETS[0]
        );
    }

    #[test]
    fn portal_edge_create_rotates_glyph_presets() {
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();
        let c = iter.next().unwrap().clone();

        let i1 = doc.create_portal_edge(&a, &b).unwrap();
        let i2 = doc.create_portal_edge(&b, &c).unwrap();
        assert_eq!(
            doc.mindmap.edges[i1].glyph_connection.as_ref().unwrap().body,
            PORTAL_GLYPH_PRESETS[0]
        );
        assert_eq!(
            doc.mindmap.edges[i2].glyph_connection.as_ref().unwrap().body,
            PORTAL_GLYPH_PRESETS[1]
        );
    }

    #[test]
    fn portal_edge_create_rejects_self() {
        let mut doc = load_test_doc();
        let id = doc.mindmap.nodes.keys().next().unwrap().clone();
        assert!(doc.create_portal_edge(&id, &id).is_none());
    }

    #[test]
    fn portal_edge_create_rejects_unknown_node() {
        let mut doc = load_test_doc();
        let known = doc.mindmap.nodes.keys().next().unwrap().clone();
        assert!(doc.create_portal_edge(&known, "does_not_exist").is_none());
        assert!(doc.create_portal_edge("does_not_exist", &known).is_none());
    }

    #[test]
    fn portal_edge_delete_and_undo_restore_index() {
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();
        let c = iter.next().unwrap().clone();

        let i1 = doc.create_portal_edge(&a, &b).unwrap();
        let i2 = doc.create_portal_edge(&b, &c).unwrap();
        let _i3 = doc.create_portal_edge(&a, &c).unwrap();
        doc.undo_stack.push(UndoAction::CreateEdge { index: i1 });
        doc.undo_stack.push(UndoAction::CreateEdge { index: i2 });
        let before_len = doc.mindmap.edges.len();

        let er_middle = EdgeRef::new(
            doc.mindmap.edges[i2].from_id.clone(),
            doc.mindmap.edges[i2].to_id.clone(),
            doc.mindmap.edges[i2].edge_type.clone(),
        );
        let (rem_idx, rem) = doc.remove_edge(&er_middle).expect("should delete");
        doc.undo_stack.push(UndoAction::DeleteEdge {
            index: rem_idx,
            edge: rem,
        });
        assert_eq!(doc.mindmap.edges.len(), before_len - 1);

        assert!(doc.undo());
        assert_eq!(doc.mindmap.edges.len(), before_len);
        assert!(baumhard::mindmap::model::is_portal_edge(&doc.mindmap.edges[i2]));
    }

    /// Undoing a freshly-created-and-selected edge clears the
    /// selection too — otherwise `SelectionState::Edge(er)` lingers
    /// pointing at an edge the map no longer contains, and scene
    /// builds + the color picker open against a dangling ref.
    /// Mirrors the long-standing `CreateNode` undo behaviour.
    #[test]
    fn undo_create_edge_clears_matching_selection() {
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();
        let idx = doc.create_cross_link_edge(&a, &b).unwrap();
        doc.undo_stack.push(UndoAction::CreateEdge { index: idx });
        let er = EdgeRef::new(&a, &b, "cross_link");
        doc.selection = SelectionState::Edge(er);

        assert!(doc.undo());
        assert!(matches!(doc.selection, SelectionState::None));
    }

    /// And: undoing a CreateEdge that did *not* match the current
    /// selection leaves the selection alone — only the matching
    /// ref triggers the clear.
    #[test]
    fn undo_create_edge_preserves_nonmatching_selection() {
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();
        let idx = doc.create_cross_link_edge(&a, &b).unwrap();
        doc.undo_stack.push(UndoAction::CreateEdge { index: idx });
        // Select some other node (not the edge we're about to undo).
        let other = doc.mindmap.nodes.keys().next().unwrap().clone();
        doc.selection = SelectionState::Single(other.clone());

        assert!(doc.undo());
        match doc.selection {
            SelectionState::Single(ref id) => assert_eq!(id, &other),
            ref s => panic!("expected single selection preserved, got {:?}", s),
        }
    }

    #[test]
    fn portal_edge_set_display_mode_toggles_visual() {
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();

        let idx = doc.create_portal_edge(&a, &b).unwrap();
        let er = EdgeRef::new(&a, &b, "cross_link");
        assert!(baumhard::mindmap::model::is_portal_edge(&doc.mindmap.edges[idx]));

        // Switch to line — marker glyph stays in glyph_connection.body
        // but the edge now renders as a connection path.
        assert!(doc.set_edge_display_mode(&er, "line"));
        assert!(!baumhard::mindmap::model::is_portal_edge(&doc.mindmap.edges[idx]));
        assert!(doc.undo());
        assert!(baumhard::mindmap::model::is_portal_edge(&doc.mindmap.edges[idx]));
    }

    #[test]
    fn portal_edge_set_color_via_generic_edge_api() {
        // `set_edge_color` is the same sink whether the edge is a
        // line or a portal. Portal markers read the resolved color
        // from `glyph_connection.color` (override) or `edge.color`
        // (fallback), so setting the override steers the marker.
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();

        let idx = doc.create_portal_edge(&a, &b).unwrap();
        let er = EdgeRef::new(&a, &b, "cross_link");
        assert!(doc.set_edge_color(&er, Some("var(--accent)")));
        assert_eq!(
            doc.mindmap.edges[idx]
                .glyph_connection
                .as_ref()
                .unwrap()
                .color
                .as_deref(),
            Some("var(--accent)")
        );
        assert!(doc.undo());
        assert_eq!(
            doc.mindmap.edges[idx]
                .glyph_connection
                .as_ref()
                .unwrap()
                .color
                .as_deref(),
            None
        );
    }

    #[test]
    fn portal_edge_selection_uses_edge_variant() {
        let mut doc = load_test_doc();
        let mut iter = doc.mindmap.nodes.keys();
        let a = iter.next().unwrap().clone();
        let b = iter.next().unwrap().clone();
        let idx = doc.create_portal_edge(&a, &b).unwrap();
        let er = EdgeRef::new(
            doc.mindmap.edges[idx].from_id.clone(),
            doc.mindmap.edges[idx].to_id.clone(),
            doc.mindmap.edges[idx].edge_type.clone(),
        );
        doc.selection = SelectionState::Edge(er.clone());

        assert!(!doc.selection.is_selected(&a));
        assert!(!doc.selection.is_selected(&b));
        assert!(doc.selection.selected_ids().is_empty());
        assert_eq!(doc.selection.selected_edge(), Some(&er));
    }

    // -----------------------------------------------------------------
    // Node text editing
    // -----------------------------------------------------------------

