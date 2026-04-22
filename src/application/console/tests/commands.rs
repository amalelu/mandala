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
fn test_edge_reset_curve_inserts_control_point_on_straight_edge() {
    // Console wiring for the `edge reset=curve` verb: picks up
    // the selected straight edge and drives
    // `curve_straight_edge`, which seeds a quadratic Bezier.
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    doc.mindmap
        .edges
        .iter_mut()
        .find(|e| er.matches(e))
        .unwrap()
        .control_points
        .clear();
    let result = run("edge reset=curve", &mut doc);
    assert!(
        matches!(result, ExecResult::Ok(_)),
        "edge reset=curve should succeed, got {:?}",
        result
    );
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    assert_eq!(
        updated.control_points.len(),
        1,
        "curve should seed exactly one control point"
    );
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
fn test_font_kv_size_min_max_atomic_on_edge() {
    // User asks for `size=14 max=10` — the atomic setter should
    // apply max first, then clamp size into the new bound,
    // landing on `size=10, max=10`. Naive order would produce
    // `size=14, max=10` (bug).
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let result = run("font size=14 max=10", &mut doc);
    assert!(matches!(result, ExecResult::Ok(_)));
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    let cfg = updated.glyph_connection.as_ref().unwrap();
    assert!((cfg.max_font_size_pt - 10.0).abs() < 0.01, "max={}", cfg.max_font_size_pt);
    assert!(
        (cfg.font_size_pt - 10.0).abs() < 0.01,
        "size should clamp to new max; got {}",
        cfg.font_size_pt
    );
}

#[test]
fn test_font_kv_min_alone_narrows_edge_clamp() {
    // `min=20` with no size touches only the min clamp. The
    // base size remains whatever it was before (which for the
    // fixture happens to be 12).
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let size_before = doc
        .mindmap
        .edges
        .iter()
        .find(|e| er.matches(e))
        .unwrap()
        .glyph_connection
        .as_ref()
        .map(|c| c.font_size_pt)
        .unwrap_or(12.0);
    let _ = run("font min=20", &mut doc);
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    let cfg = updated.glyph_connection.as_ref().unwrap();
    assert!((cfg.min_font_size_pt - 20.0).abs() < 0.01, "min={}", cfg.min_font_size_pt);
    // Size wasn't in the kvs, so it shouldn't have moved.
    assert!(
        (cfg.font_size_pt - size_before).abs() < 0.01,
        "size should be unchanged; was {}, now {}",
        size_before,
        cfg.font_size_pt
    );
}

#[test]
fn test_font_kv_invalid_value_reports_error() {
    // Non-numeric input is rejected at parse time, not silently
    // ignored. Returns `Err` so the console displays it as an
    // error line.
    let mut doc = load_test_doc();
    let _ = select_first_edge(&mut doc);
    let result = run("font size=abc", &mut doc);
    assert!(matches!(result, ExecResult::Err(_)), "got {:?}", result);
}

#[test]
fn test_font_kv_inverted_min_max_reports_error() {
    // `min=20 max=10` is inverted — would panic `f32::clamp` on
    // the next render frame. Console rejects up front so the
    // user sees a clear error (setter also rejects defence-in-
    // depth, but via silent no-op).
    let mut doc = load_test_doc();
    let _ = select_first_edge(&mut doc);
    let result = run("font min=20 max=10", &mut doc);
    assert!(
        matches!(result, ExecResult::Err(_)),
        "inverted bounds must surface as Err, got {:?}",
        result
    );
}

#[test]
fn test_label_position_t_writes_label_config() {
    // `label position_t=0.25` lands the value directly into
    // `label_config.position_t`. Values outside [0, 1] clamp
    // silently (the setter is the authority).
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let result = run("label position_t=0.25", &mut doc);
    assert!(matches!(result, ExecResult::Ok(_)));
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    assert_eq!(
        updated.label_config.as_ref().and_then(|c| c.position_t),
        Some(0.25)
    );
}

#[test]
fn test_label_perpendicular_writes_label_config() {
    // `label perpendicular=12.5` writes the signed offset; an
    // empty string clears it.
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let _ = run("label perpendicular=12.5", &mut doc);
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    assert_eq!(
        updated
            .label_config
            .as_ref()
            .and_then(|c| c.perpendicular_offset),
        Some(12.5)
    );
    // Clear it back.
    let _ = run("label perpendicular=", &mut doc);
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    assert!(updated
        .label_config
        .as_ref()
        .and_then(|c| c.perpendicular_offset)
        .is_none());
}

#[test]
fn test_label_position_t_writes_portal_border_t_on_portal_label_selection() {
    // Portal labels: `label position_t=2.5` writes to the
    // endpoint's `border_t` (canonical range [0, 4)) instead of the
    // line-mode `label_config.position_t`. Confirms the dispatch
    // splits on selection and that wrapping into [0, 4) is
    // applied.
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    // Flip to portal mode so the edge has a portal endpoint to
    // address; clone the first edge's id triple to drive selection.
    doc.set_edge_display_mode(&er, "portal");
    let edge = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    let endpoint = edge.from_id.clone();
    let edge_key = baumhard::mindmap::scene_cache::EdgeKey::new(
        &edge.from_id,
        &edge.to_id,
        &edge.edge_type,
    );
    doc.selection = SelectionState::PortalLabel(
        crate::application::document::PortalLabelSel {
            edge_key,
            endpoint_node_id: endpoint.clone(),
        },
    );
    let _ = run("label position_t=2.5", &mut doc);
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    let pf = updated.portal_from.as_ref().expect("portal_from installed");
    assert_eq!(pf.border_t, Some(2.5));
}

#[test]
fn test_label_perpendicular_writes_portal_offset_on_portal_text_selection() {
    // Same dispatch shape as above but for `perpendicular=`.
    // PortalText selections should land in the SAME endpoint slot
    // as PortalLabel selections — both target the same portal
    // endpoint, just different sub-parts.
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    doc.set_edge_display_mode(&er, "portal");
    let edge = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    let endpoint = edge.to_id.clone();
    let edge_key = baumhard::mindmap::scene_cache::EdgeKey::new(
        &edge.from_id,
        &edge.to_id,
        &edge.edge_type,
    );
    doc.selection = SelectionState::PortalText(
        crate::application::document::PortalLabelSel {
            edge_key,
            endpoint_node_id: endpoint,
        },
    );
    let _ = run("label perpendicular=18.5", &mut doc);
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    let pt = updated.portal_to.as_ref().expect("portal_to installed");
    assert_eq!(pt.perpendicular_offset, Some(18.5));

    // Clearing returns the slot to default and prunes the now-empty
    // endpoint state — match the line-mode label_config behaviour.
    let _ = run("label perpendicular=", &mut doc);
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    assert!(updated
        .portal_to
        .as_ref()
        .map(|s| s.perpendicular_offset)
        .unwrap_or(None)
        .is_none());
}

#[test]
fn test_label_position_t_invalid_reports_error() {
    let mut doc = load_test_doc();
    let _ = select_first_edge(&mut doc);
    let result = run("label position_t=nan", &mut doc);
    assert!(matches!(result, ExecResult::Err(_)), "got {:?}", result);
}

#[test]
fn test_label_position_t_out_of_range_echoes_clamp() {
    // `position_t=2.5` clamps silently to 1.0 on the setter;
    // the console echoes the clamp so the user doesn't think
    // their 2.5 was accepted literally.
    let mut doc = load_test_doc();
    let _ = select_first_edge(&mut doc);
    let result = run("label position_t=2.5", &mut doc);
    match result {
        ExecResult::Lines(lines) => {
            assert!(
                lines.iter().any(|l| l.contains("clamped to 1")),
                "expected clamp echo in output lines, got {:?}",
                lines
            );
        }
        other => panic!("expected Lines with clamp echo, got {:?}", other),
    }
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

// `edge portal` subcommand was removed — creation now goes
// through connect mode + `edge display_mode=portal`. Tests
// previously pinned here are obsolete.

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
