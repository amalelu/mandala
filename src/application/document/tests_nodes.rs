//! Node text / background / border / text-colour / font-size setters + set_node_style_field helper.
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
use baumhard::util::grapheme_chad::count_grapheme_clusters;
use glam::Vec2;

use super::defaults::default_cross_link_edge;


    #[test]
    fn test_set_node_text_updates_text_and_collapses_runs() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        let changed = doc.set_node_text(&nid, "Hello world".to_string());
        assert!(changed);
        let node = doc.mindmap.nodes.get(&nid).unwrap();
        assert_eq!(node.text, "Hello world");
        assert_eq!(node.text_runs.len(), 1);
        assert_eq!(node.text_runs[0].start, 0);
        assert_eq!(node.text_runs[0].end, count_grapheme_clusters("Hello world"));
        assert!(doc.dirty);
        assert!(matches!(
            doc.undo_stack.last(),
            Some(UndoAction::EditNodeText { .. })
        ));
    }

    #[test]
    fn test_set_node_text_noop_on_unchanged() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        let current = doc.mindmap.nodes.get(&nid).unwrap().text.clone();
        doc.undo_stack.clear();
        doc.dirty = false;
        let changed = doc.set_node_text(&nid, current);
        assert!(!changed);
        assert!(doc.undo_stack.is_empty());
        assert!(!doc.dirty);
    }

    #[test]
    fn test_set_node_text_undo_round_trip() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        let before_text = doc.mindmap.nodes.get(&nid).unwrap().text.clone();
        let before_runs_len = doc.mindmap.nodes.get(&nid).unwrap().text_runs.len();
        let before_first_run_color = doc
            .mindmap
            .nodes
            .get(&nid)
            .unwrap()
            .text_runs
            .first()
            .map(|r| r.color.clone());
        assert!(doc.set_node_text(&nid, "mutated".to_string()));
        assert_eq!(doc.mindmap.nodes.get(&nid).unwrap().text, "mutated");
        assert!(doc.undo());
        let restored = doc.mindmap.nodes.get(&nid).unwrap();
        assert_eq!(restored.text, before_text);
        // TextRun doesn't implement PartialEq, so compare the parts
        // we care about: count + first run's color.
        assert_eq!(restored.text_runs.len(), before_runs_len);
        assert_eq!(
            restored.text_runs.first().map(|r| r.color.clone()),
            before_first_run_color
        );
    }

    #[test]
    fn test_set_node_text_multiline_with_newlines() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        assert!(doc.set_node_text(&nid, "line 1\nline 2\nline 3".to_string()));
        let node = doc.mindmap.nodes.get(&nid).unwrap();
        assert_eq!(node.text, "line 1\nline 2\nline 3");
        // Collapsed single run spans the full char count, including newlines.
        assert_eq!(node.text_runs.len(), 1);
        assert_eq!(node.text_runs[0].end, count_grapheme_clusters("line 1\nline 2\nline 3"));
    }

    #[test]
    fn test_set_node_text_unknown_id_returns_false() {
        let mut doc = load_test_doc();
        doc.undo_stack.clear();
        doc.dirty = false;
        assert!(!doc.set_node_text("nonexistent-id", "x".to_string()));
        assert!(doc.undo_stack.is_empty());
        assert!(!doc.dirty);
    }

    #[test]
    fn test_set_node_text_inherits_first_run_formatting() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        // Force a specific first-run formatting we can check for.
        {
            let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
            if node.text_runs.is_empty() {
                node.text_runs.push(TextRun {
                    start: 0,
                    end: count_grapheme_clusters(&node.text),
                    bold: false,
                    italic: false,
                    underline: false,
                    font: "LiberationSans".to_string(),
                    size_pt: 24,
                    color: "#ffffff".to_string(),
                    hyperlink: None,
                });
            }
            node.text_runs[0].bold = true;
            node.text_runs[0].color = "#abcdef".to_string();
            node.text_runs[0].size_pt = 33;
        }
        assert!(doc.set_node_text(&nid, "rewritten".to_string()));
        let run = &doc.mindmap.nodes.get(&nid).unwrap().text_runs[0];
        assert!(run.bold);
        assert_eq!(run.color, "#abcdef");
        assert_eq!(run.size_pt, 33);
    }

    // -----------------------------------------------------------------
    // Node style setters (bg / border / text color, font size)
    // -----------------------------------------------------------------

    #[test]
    fn test_set_node_bg_color_round_trips_through_undo() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        let before = doc.mindmap.nodes.get(&nid).unwrap().style.background_color.clone();
        assert!(doc.set_node_bg_color(&nid, "#123456".to_string()));
        assert_eq!(doc.mindmap.nodes.get(&nid).unwrap().style.background_color, "#123456");
        assert!(matches!(
            doc.undo_stack.last(),
            Some(UndoAction::EditNodeStyle { .. })
        ));
        assert!(doc.undo());
        assert_eq!(doc.mindmap.nodes.get(&nid).unwrap().style.background_color, before);
    }

    #[test]
    fn test_set_node_bg_color_unchanged_is_noop() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        let current = doc.mindmap.nodes.get(&nid).unwrap().style.background_color.clone();
        doc.undo_stack.clear();
        doc.dirty = false;
        assert!(!doc.set_node_bg_color(&nid, current));
        assert!(doc.undo_stack.is_empty());
        assert!(!doc.dirty);
    }

    #[test]
    fn test_set_node_border_color_writes_frame_color() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        assert!(doc.set_node_border_color(&nid, "#ff00ff".to_string()));
        assert_eq!(doc.mindmap.nodes.get(&nid).unwrap().style.frame_color, "#ff00ff");
    }

    /// Setting text color rewrites `style.text_color` and every run
    /// whose color matched the pre-edit default. A run the user
    /// colored by hand (mismatched) keeps its override.
    #[test]
    fn test_set_node_text_color_preserves_per_run_overrides() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        // Seed the node with a known default and two runs: one
        // matching the default, one hand-colored.
        {
            let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
            node.style.text_color = "#dddddd".into();
            node.text_runs = vec![
                TextRun {
                    start: 0, end: 3,
                    bold: false, italic: false, underline: false,
                    font: "LiberationSans".into(), size_pt: 24,
                    color: "#dddddd".into(), // matches default
                    hyperlink: None,
                },
                TextRun {
                    start: 3, end: 6,
                    bold: false, italic: false, underline: false,
                    font: "LiberationSans".into(), size_pt: 24,
                    color: "#abcdef".into(), // user override
                    hyperlink: None,
                },
            ];
        }
        assert!(doc.set_node_text_color(&nid, "#111111".into()));
        let node = doc.mindmap.nodes.get(&nid).unwrap();
        assert_eq!(node.style.text_color, "#111111");
        assert_eq!(node.text_runs[0].color, "#111111", "default-following run should update");
        assert_eq!(node.text_runs[1].color, "#abcdef", "per-run override should be preserved");
    }

    #[test]
    fn test_set_node_text_color_round_trips_through_undo() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        {
            let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
            node.style.text_color = "#dddddd".into();
            for run in node.text_runs.iter_mut() {
                run.color = "#dddddd".into();
            }
        }
        let before_default = doc.mindmap.nodes.get(&nid).unwrap().style.text_color.clone();
        let before_run_colors: Vec<String> = doc
            .mindmap
            .nodes
            .get(&nid)
            .unwrap()
            .text_runs
            .iter()
            .map(|r| r.color.clone())
            .collect();
        assert!(doc.set_node_text_color(&nid, "#222222".into()));
        assert!(doc.undo());
        let restored = doc.mindmap.nodes.get(&nid).unwrap();
        assert_eq!(restored.style.text_color, before_default);
        let restored_colors: Vec<String> =
            restored.text_runs.iter().map(|r| r.color.clone()).collect();
        assert_eq!(restored_colors, before_run_colors);
    }

    #[test]
    fn test_set_node_font_size_writes_all_runs_and_round_trips() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        let before_sizes: Vec<u32> = doc
            .mindmap
            .nodes
            .get(&nid)
            .unwrap()
            .text_runs
            .iter()
            .map(|r| r.size_pt)
            .collect();
        assert!(doc.set_node_font_size(&nid, 48.0));
        let node = doc.mindmap.nodes.get(&nid).unwrap();
        assert!(node.text_runs.iter().all(|r| r.size_pt == 48));
        assert!(doc.undo());
        let after_sizes: Vec<u32> = doc
            .mindmap
            .nodes
            .get(&nid)
            .unwrap()
            .text_runs
            .iter()
            .map(|r| r.size_pt)
            .collect();
        assert_eq!(after_sizes, before_sizes);
    }

    #[test]
    fn test_set_node_font_size_clamps_below_one() {
        let mut doc = load_test_doc();
        let nid = first_testament_node_id(&doc);
        assert!(doc.set_node_font_size(&nid, 0.5));
        let node = doc.mindmap.nodes.get(&nid).unwrap();
        assert!(node.text_runs.iter().all(|r| r.size_pt == 1));
    }

    #[test]
    fn test_set_node_style_unknown_id_returns_false() {
        let mut doc = load_test_doc();
        doc.undo_stack.clear();
        doc.dirty = false;
        assert!(!doc.set_node_bg_color("nope", "#000".into()));
        assert!(!doc.set_node_border_color("nope", "#000".into()));
        assert!(!doc.set_node_text_color("nope", "#000".into()));
        assert!(!doc.set_node_font_size("nope", 10.0));
        assert!(doc.undo_stack.is_empty());
        assert!(!doc.dirty);
    }
