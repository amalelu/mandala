//! Per-command execution tests: anchor / body / cap / spacing /
//! color / label / edge / font / help. Each test runs a single
//! command line through `parse → execute` and asserts the
//! observable model effect. Portal-specific creation / glyph
//! commands now live under `edge` (`edge portal`, `edge body=…`)
//! and are covered in this file alongside the other `edge` cases.

use super::fixtures::{load_test_doc, run, select_first_edge};
use crate::application::console::parser::{parse, Args, ParseResult};
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::document::{EdgeRef, SelectionState};

#[test]
fn test_anchor_kv_updates_edge_anchor() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let _ = run("anchor from=top", &mut doc);
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    assert_eq!(updated.anchor_from, "top");
}

#[test]
fn test_anchor_kv_idempotent_second_call() {
    let mut doc = load_test_doc();
    select_first_edge(&mut doc);
    let first = run("anchor from=left", &mut doc);
    // Second call reports "already left" as Lines (not Err).
    let second = run("anchor from=left", &mut doc);
    assert!(matches!(first, ExecResult::Ok(_)));
    assert!(matches!(second, ExecResult::Err(_) | ExecResult::Lines(_)));
}

#[test]
fn test_anchor_errors_without_edge_selection() {
    let mut doc = load_test_doc();
    let result = run("anchor from=top", &mut doc);
    assert!(matches!(result, ExecResult::Err(_)), "got {:?}", result);
}

#[test]
fn test_body_kv_sets_glyph() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let _ = run("body glyph=dash", &mut doc);
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    let body = updated.glyph_connection.as_ref().map(|c| c.body.as_str());
    assert_eq!(body, Some("\u{2500}"));
}

#[test]
fn test_cap_kv_from_arrow_sets_left_triangle() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let _ = run("cap from=arrow", &mut doc);
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    let cap = updated.glyph_connection.as_ref().and_then(|c| c.cap_start.as_deref());
    assert_eq!(cap, Some("\u{25C0}"));
}

#[test]
fn test_cap_kv_to_arrow_sets_right_triangle() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let _ = run("cap to=arrow", &mut doc);
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    let cap = updated.glyph_connection.as_ref().and_then(|c| c.cap_end.as_deref());
    assert_eq!(cap, Some("\u{25B6}"));
}

#[test]
fn test_spacing_kv_wide_sets_six() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let _ = run("spacing value=wide", &mut doc);
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    let spacing = updated.glyph_connection.as_ref().map(|c| c.spacing);
    assert_eq!(spacing, Some(6.0));
}

#[test]
fn test_color_kv_text_accent_sets_var_accent_on_edge() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let _ = run("color text=accent", &mut doc);
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    let color = updated.glyph_connection.as_ref().and_then(|c| c.color.clone());
    assert_eq!(color.as_deref(), Some("var(--accent)"));
}

#[test]
fn test_color_kv_bg_sets_node_background() {
    let mut doc = load_test_doc();
    let nid = doc.mindmap.nodes.keys().next().unwrap().clone();
    doc.selection = SelectionState::Single(nid.clone());
    let _ = run("color bg=#112233", &mut doc);
    assert_eq!(
        doc.mindmap.nodes.get(&nid).unwrap().style.background_color,
        "#112233"
    );
}

/// `color bg` with a single node selected opens the glyph-wheel
/// picker with a Node target carrying the `Bg` axis — matches the
/// "color used without a value brings up the wheel" UX.
#[test]
fn test_color_bg_no_value_on_node_opens_picker_with_bg_axis() {
    use crate::application::color_picker::{ColorTarget, NodeColorAxis};
    let mut doc = load_test_doc();
    let nid = doc.mindmap.nodes.keys().next().unwrap().clone();
    doc.selection = SelectionState::Single(nid.clone());
    let (cmd, toks) = match parse("color bg") {
        ParseResult::Ok { cmd, args } => (cmd, args),
        _ => panic!("parse failed"),
    };
    let mut eff = ConsoleEffects::new(&mut doc);
    let _ = (cmd.execute)(&Args::new(&toks), &mut eff);
    assert!(eff.close_console);
    match eff.open_color_picker {
        Some(ColorTarget::Node { id, axis }) => {
            assert_eq!(id, nid);
            assert_eq!(axis, NodeColorAxis::Bg);
        }
        other => panic!("expected picker with Node/Bg target, got {:?}", other),
    }
}

/// `color text` on an edge still opens the picker — edges only have
/// one color so the axis collapses onto `ColorTarget::Edge`.
#[test]
fn test_color_text_no_value_on_edge_opens_picker_on_edge() {
    use crate::application::color_picker::ColorTarget;
    let mut doc = load_test_doc();
    let _ = select_first_edge(&mut doc);
    let (cmd, toks) = match parse("color text") {
        ParseResult::Ok { cmd, args } => (cmd, args),
        _ => panic!("parse failed"),
    };
    let mut eff = ConsoleEffects::new(&mut doc);
    let _ = (cmd.execute)(&Args::new(&toks), &mut eff);
    assert!(matches!(eff.open_color_picker, Some(ColorTarget::Edge(_))));
}

/// `color text` on a portal-mode edge routes through the same
/// `set_edge_color` sink line-mode edges use (there's only one
/// color field). Applied, not rejected — the user's mental model
/// doesn't distinguish "text color of an edge" from "color of an
/// edge" because edges have one color.
#[test]
fn test_color_text_on_portal_edge_applies() {
    let mut doc = load_test_doc();
    let mut ids = doc.mindmap.nodes.keys().cloned();
    let a = ids.next().unwrap();
    let b = ids.next().unwrap();
    let _idx = doc.create_portal_edge(&a, &b).unwrap();
    doc.selection = SelectionState::Edge(EdgeRef::new(&a, &b, "cross_link"));
    let result = run("color text=#112233", &mut doc);
    assert!(matches!(result, ExecResult::Ok(_)), "got {:?}", result);
}

#[test]
fn test_color_kv_rejects_invalid_hex() {
    let mut doc = load_test_doc();
    let nid = doc.mindmap.nodes.keys().next().unwrap().clone();
    doc.selection = SelectionState::Single(nid);
    let result = run("color bg=notacolor", &mut doc);
    assert!(matches!(result, ExecResult::Err(_)));
}

#[test]
fn test_color_pick_sets_open_color_picker_handoff() {
    let mut doc = load_test_doc();
    let _ = select_first_edge(&mut doc);
    let (cmd, toks) = match parse("color pick") {
        ParseResult::Ok { cmd, args } => (cmd, args),
        _ => panic!("parse failed"),
    };
    let mut eff = ConsoleEffects::new(&mut doc);
    let _ = (cmd.execute)(&Args::new(&toks), &mut eff);
    assert!(eff.open_color_picker.is_some());
    assert!(eff.close_console);
}

#[test]
fn test_label_edit_hands_off_to_inline_editor() {
    let mut doc = load_test_doc();
    let _ = select_first_edge(&mut doc);
    let (cmd, toks) = match parse("label edit") {
        ParseResult::Ok { cmd, args } => (cmd, args),
        _ => panic!("parse failed"),
    };
    let mut eff = ConsoleEffects::new(&mut doc);
    let _ = (cmd.execute)(&Args::new(&toks), &mut eff);
    assert!(eff.open_label_edit.is_some());
}

#[test]
fn test_label_kv_with_quoted_string_writes_label() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let _ = run(r#"label text="hello world""#, &mut doc);
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    assert_eq!(updated.label.as_deref(), Some("hello world"));
}

#[test]
fn test_edge_reset_straight_clears_control_points() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    // Seed a control point so reset has something to clear.
    doc.mindmap
        .edges
        .iter_mut()
        .find(|e| er.matches(e))
        .unwrap()
        .control_points = vec![baumhard::mindmap::model::ControlPoint { x: 10.0, y: 20.0 }];
    let _ = run("edge reset=straight", &mut doc);
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    assert!(updated.control_points.is_empty());
}

#[test]
fn test_font_kv_size_sets_absolute_value_on_edge() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let _ = run("font size=18", &mut doc);
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    let cfg = updated.glyph_connection.as_ref().unwrap();
    assert!((cfg.font_size_pt - 18.0).abs() < 0.01, "got {}", cfg.font_size_pt);
}

#[test]
fn test_edge_type_parent_child_updates_edge() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    // Pick whichever type the edge *isn't* currently so the command
    // actually changes something.
    let current = doc
        .mindmap
        .edges
        .iter()
        .find(|e| er.matches(e))
        .unwrap()
        .edge_type
        .clone();
    let target = if current == "parent_child" { "cross_link" } else { "parent_child" };
    let _ = run(&format!("edge type={}", target), &mut doc);
    // `set_edge_type` renames the edge identity, so we can't find it
    // by the old er anymore — just assert at least one edge has the
    // new type.
    assert!(doc.mindmap.edges.iter().any(|e| e.edge_type == target));
}

#[test]
fn test_edge_portal_with_two_nodes_selected() {
    let mut doc = load_test_doc();
    let mut ids = doc.mindmap.nodes.keys().cloned();
    let a = ids.next().unwrap();
    let b = ids.next().unwrap();
    doc.selection = SelectionState::Multi(vec![a.clone(), b.clone()]);
    let _ = run("edge portal", &mut doc);
    // Post-refactor: portal creation yields a SelectionState::Edge
    // whose matching edge has `display_mode = "portal"`.
    assert!(matches!(doc.selection, SelectionState::Edge(_)));
    let edge = doc
        .mindmap
        .edges
        .iter()
        .find(|e| e.from_id == a && e.to_id == b)
        .expect("portal edge must exist");
    assert!(baumhard::mindmap::model::is_portal_edge(edge));
}

#[test]
fn test_edge_portal_errors_without_two_node_selection() {
    let mut doc = load_test_doc();
    doc.selection = SelectionState::None;
    let result = run("edge portal", &mut doc);
    assert!(matches!(result, ExecResult::Err(_)));
}

#[test]
fn test_edge_display_mode_portal_then_line_toggles() {
    let mut doc = load_test_doc();
    let mut ids = doc.mindmap.nodes.keys().cloned();
    let a = ids.next().unwrap();
    let b = ids.next().unwrap();
    let _ = doc.create_portal_edge(&a, &b).unwrap();
    doc.selection = SelectionState::Edge(EdgeRef::new(&a, &b, "cross_link"));
    let _ = run("edge display_mode=line", &mut doc);
    let edge = doc
        .mindmap
        .edges
        .iter()
        .find(|e| e.from_id == a && e.to_id == b)
        .unwrap();
    assert!(!baumhard::mindmap::model::is_portal_edge(edge));
}

#[test]
fn test_help_no_args_returns_lines() {
    let mut doc = load_test_doc();
    let result = run("help", &mut doc);
    match result {
        ExecResult::Lines(lines) => assert!(lines.len() > 1),
        other => panic!("expected Lines, got {:?}", other),
    }
}

#[test]
fn test_help_with_known_command_prints_usage() {
    let mut doc = load_test_doc();
    let result = run("help anchor", &mut doc);
    match result {
        ExecResult::Lines(lines) => {
            assert!(lines.iter().any(|l| l.contains("anchor")));
            assert!(lines.iter().any(|l| l.contains("usage:")));
        }
        other => panic!("expected Lines, got {:?}", other),
    }
}

#[test]
fn test_help_unknown_command_reports_error() {
    let mut doc = load_test_doc();
    let result = run("help nope", &mut doc);
    assert!(matches!(result, ExecResult::Err(_)));
}
