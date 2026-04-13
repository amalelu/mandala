//! Integration tests that exercise the console against the canonical
//! `testament.mindmap.json` fixture. These live in their own file to
//! keep per-command modules small; they cover the cross-module paths
//! (parse + execute + applicability) that no single module owns.
//!
//! Per `TEST_CONVENTIONS.md §3.3` the `#[cfg(test)]` inline style is
//! the default for the mandala crate — this file is just `mod tests;`
//! from a handful of siblings, with a shared fixture helper.

use super::commands::command_by_name;
use super::completion::complete;
use super::parser::{parse, Args, ParseResult};
use super::{ConsoleContext, ConsoleEffects, ConsoleState, ExecResult};
use crate::application::document::{EdgeRef, MindMapDocument, PortalRef, SelectionState};
use baumhard::mindmap::loader;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

// ============================================================
// Fixtures
// ============================================================

fn test_map_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("maps/testament.mindmap.json");
    path
}

fn load_test_doc() -> MindMapDocument {
    let map = loader::load_from_file(&test_map_path()).unwrap();
    let mut doc = MindMapDocument {
        mindmap: map,
        file_path: None,
        dirty: false,
        selection: SelectionState::None,
        undo_stack: Vec::new(),
        mutation_registry: HashMap::new(),
        active_toggles: HashSet::new(),
        label_edit_preview: None,
        color_picker_preview: None,
    };
    doc.build_mutation_registry();
    doc
}

/// Pick the first edge in the map and point the selection at it.
/// Returns the edge ref so tests can assert against the mutated
/// fields afterwards.
fn select_first_edge(doc: &mut MindMapDocument) -> EdgeRef {
    let edge = doc.mindmap.edges[0].clone();
    let er = EdgeRef::new(&edge.from_id, &edge.to_id, &edge.edge_type);
    doc.selection = SelectionState::Edge(er.clone());
    er
}

/// Parse `line`, run the resolved command against `doc`, and return
/// the `ExecResult`. Panics on parse failure — these are unit tests
/// with known-good input.
fn run(line: &str, doc: &mut MindMapDocument) -> ExecResult {
    let (cmd, tokens) = match parse(line) {
        ParseResult::Ok { cmd, args } => (cmd, args),
        ParseResult::Empty => panic!("empty input: {:?}", line),
        ParseResult::Unknown(s) => panic!("unknown command '{}' in {:?}", s, line),
    };
    let mut eff = ConsoleEffects::new(doc);
    (cmd.execute)(&Args::new(&tokens), &mut eff)
}

// ============================================================
// Grapheme-safe line editing via baumhard::util::grapheme_chad
// ============================================================
//
// The `ConsoleState::cursor` is a grapheme-cluster index, not a
// byte offset. These tests lock in the invariant that cursor-
// manipulating operations stay correct across multi-byte and
// multi-codepoint characters — CODE_CONVENTIONS §2.

#[test]
fn test_console_cursor_is_grapheme_indexed_in_docs() {
    // A sentinel check — if someone reverts the cursor semantics,
    // this test will force them to re-read CODE_CONVENTIONS §2.
    let state = ConsoleState::open(Vec::new());
    match state {
        ConsoleState::Open { cursor, .. } => assert_eq!(cursor, 0),
        _ => panic!("expected Open"),
    }
}

#[test]
fn test_grapheme_space_insertion_via_helper() {
    // winit delivers the spacebar as `Key::Named(NamedKey::Space)`,
    // which `handle_console_key` treats as a named key rather than
    // a char payload. The named-key arm should insert a literal
    // space the same way the generic char path does — verified here
    // by driving the helper directly.
    use baumhard::util::grapheme_chad::insert_str_at_grapheme;
    let mut input = String::from("ab");
    let cursor = 1;
    insert_str_at_grapheme(&mut input, cursor, " ");
    assert_eq!(input, "a b");
}

#[test]
fn test_grapheme_insert_advances_cursor_by_one_per_char() {
    // Simulate three-char insertion via the grapheme_chad helper
    // directly — mirrors what `handle_console_key` does on each
    // character key.
    use baumhard::util::grapheme_chad::{count_grapheme_clusters, insert_str_at_grapheme};
    let mut input = String::new();
    let mut cursor = 0usize;
    for ch in "abc".chars() {
        let mut buf = [0u8; 4];
        insert_str_at_grapheme(&mut input, cursor, ch.encode_utf8(&mut buf));
        cursor += 1;
    }
    assert_eq!(input, "abc");
    assert_eq!(cursor, 3);
    assert_eq!(count_grapheme_clusters(&input), 3);
}

#[test]
fn test_grapheme_delete_removes_whole_cluster() {
    // A ZWJ emoji family is 7+ codepoints but one grapheme cluster.
    // `delete_grapheme_at` must remove the whole cluster, not
    // just one codepoint.
    use baumhard::util::grapheme_chad::{count_grapheme_clusters, delete_grapheme_at};
    let mut input = String::from("a\u{1F469}\u{200D}\u{1F469}\u{200D}\u{1F466}b");
    assert_eq!(count_grapheme_clusters(&input), 3, "a + family + b");
    delete_grapheme_at(&mut input, 1); // delete the family
    assert_eq!(input, "ab");
}

// ============================================================
// State / shape smoke tests
// ============================================================

#[test]
fn test_console_state_open_is_not_closed() {
    let open = ConsoleState::open(Vec::new());
    assert!(open.is_open());
    let closed = ConsoleState::Closed;
    assert!(!closed.is_open());
}

#[test]
fn test_console_state_open_seeded_with_history() {
    let history = vec!["help".to_string(), "anchor set from auto".to_string()];
    match ConsoleState::open(history.clone()) {
        ConsoleState::Open { history: h, input, cursor, .. } => {
            assert_eq!(h, history);
            assert_eq!(input, "");
            assert_eq!(cursor, 0);
        }
        _ => panic!("expected Open"),
    }
}

// ============================================================
// Command execution
// ============================================================

#[test]
fn test_anchor_kv_updates_edge_anchor() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let _ = run("anchor from=top", &mut doc);
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    assert_eq!(updated.anchor_from, 1);
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

/// `color text` on a portal is nonsensical (portals have no text
/// axis) — reject with a helpful error.
#[test]
fn test_color_text_no_value_on_portal_errors() {
    let mut doc = load_test_doc();
    let mut ids = doc.mindmap.nodes.keys().cloned();
    let a = ids.next().unwrap();
    let b = ids.next().unwrap();
    let pref = doc.apply_create_portal(&a, &b).unwrap();
    doc.selection = SelectionState::Portal(pref);
    let result = run("color text", &mut doc);
    assert!(matches!(result, ExecResult::Err(_)), "got {:?}", result);
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

// ============================================================
// AcceptsWheelColor dispatch
// ============================================================
//
// The standalone color wheel applies a single color to whatever is
// selected, and each component type decides which channel that
// color lands on. These tests lock in the per-variant choice so a
// future refactor can't silently migrate a node's default to
// `text_color` or an edge's default to a non-existent `bg_color`.

/// A node under the wheel takes its color on the **background fill**.
/// Asserted via `style.background_color` after dispatch.
#[test]
fn wheel_color_on_node_paints_background() {
    use crate::application::console::traits::{
        view_for, AcceptsWheelColor, ColorValue, Outcome, TargetId,
    };
    let mut doc = load_test_doc();
    let nid = doc.mindmap.nodes.keys().next().unwrap().clone();
    let tid = TargetId::Node(nid.clone());
    let outcome = {
        let mut view = view_for(&mut doc, &tid);
        view.apply_wheel_color(ColorValue::Hex("#112233".into()))
    };
    assert_eq!(outcome, Outcome::Applied);
    assert_eq!(
        doc.mindmap.nodes.get(&nid).unwrap().style.background_color,
        "#112233"
    );
}

/// An edge under the wheel takes its color on the **single edge
/// color field** — the line and label share it. Asserted via the
/// glyph-connection override written by `set_edge_color`.
#[test]
fn wheel_color_on_edge_paints_line() {
    use crate::application::console::traits::{
        view_for, AcceptsWheelColor, ColorValue, Outcome, TargetId,
    };
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let tid = TargetId::Edge(er.clone());
    let outcome = {
        let mut view = view_for(&mut doc, &tid);
        view.apply_wheel_color(ColorValue::Hex("#445566".into()))
    };
    assert_eq!(outcome, Outcome::Applied);
    let edge = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    // `set_edge_color(Some(..))` writes the override onto the
    // glyph_connection config, which takes precedence over
    // `edge.color`. Checking the effective string covers both the
    // forked-connection path and the raw-color fallback.
    let effective = edge
        .glyph_connection
        .as_ref()
        .and_then(|gc| gc.color.clone())
        .unwrap_or_else(|| edge.color.clone());
    assert_eq!(effective, "#445566");
}

/// A portal under the wheel returns `NotApplicable` today —
/// portals aren't Baumhard-native yet, so the standalone wheel
/// deliberately does nothing on portal selections until the port
/// lands. Regression guard so the deferred state is visible to
/// anyone changing the trait impl.
#[test]
fn wheel_color_on_portal_is_not_applicable() {
    use crate::application::console::traits::{
        view_for, AcceptsWheelColor, ColorValue, Outcome, TargetId,
    };
    use crate::application::document::PortalRef;
    let mut doc = load_test_doc();
    // Build a synthetic portal ref — even if the testament map has
    // no portals, the dispatch returns NotApplicable before it
    // tries to look the portal up.
    let pr = PortalRef {
        label: "A".into(),
        endpoint_a: "x".into(),
        endpoint_b: "y".into(),
    };
    let tid = TargetId::Portal(pr);
    let mut view = view_for(&mut doc, &tid);
    let outcome = view.apply_wheel_color(ColorValue::Hex("#778899".into()));
    assert_eq!(outcome, Outcome::NotApplicable);
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

// ============================================================
// apply_kvs dispatcher aggregation
// ============================================================

/// Empty selection → "no target" message, report marked as all_failed.
#[test]
fn test_apply_kvs_with_no_selection_reports_no_target_and_fails() {
    use crate::application::console::traits::{apply_kvs, Outcome};
    let mut doc = load_test_doc();
    doc.selection = SelectionState::None;
    let kvs = vec![("bg".to_string(), "#123".to_string())];
    let report = apply_kvs(&mut doc, &kvs, |_v, _k, _val| Some(Outcome::Applied));
    assert!(report.all_failed);
    assert_eq!(report.messages.len(), 1);
    assert!(report.messages[0].contains("no target"));
}

/// Applier returning None (unknown key) reports exactly once per
/// pair, not once per target.
#[test]
fn test_apply_kvs_unknown_key_reported_once_per_pair() {
    use crate::application::console::traits::{apply_kvs, Outcome};
    let mut doc = load_test_doc();
    let mut ids = doc.mindmap.nodes.keys().cloned();
    let a = ids.next().unwrap();
    let b = ids.next().unwrap();
    doc.selection = SelectionState::Multi(vec![a, b]);
    let kvs = vec![("bogus".to_string(), "x".to_string())];
    let seen_calls = std::cell::Cell::new(0usize);
    let report = apply_kvs(&mut doc, &kvs, |_v, _k, _val| {
        seen_calls.set(seen_calls.get() + 1);
        None::<Outcome>
    });
    assert!(report.all_failed);
    assert_eq!(report.messages.len(), 1);
    assert!(report.messages[0].contains("bogus"));
    // Short-circuit after first None on the first target.
    assert_eq!(seen_calls.get(), 1);
}

/// NotApplicable from every target collapses into a single
/// per-pair message, and the label is plural for `Multi`.
#[test]
fn test_apply_kvs_not_applicable_reported_when_no_target_applies() {
    use crate::application::console::traits::{apply_kvs, Outcome};
    let mut doc = load_test_doc();
    let mut ids = doc.mindmap.nodes.keys().cloned();
    let a = ids.next().unwrap();
    let b = ids.next().unwrap();
    doc.selection = SelectionState::Multi(vec![a, b]);
    let kvs = vec![("text".to_string(), "accent".to_string())];
    let report = apply_kvs(&mut doc, &kvs, |_v, _k, _val| Some(Outcome::NotApplicable));
    assert!(!report.any_applied);
    assert_eq!(report.messages.len(), 1);
    assert!(report.messages[0].contains("not applicable"));
    assert!(report.messages[0].contains("nodes"));
}

/// An Applied result on every target produces zero messages and
/// flags any_applied.
#[test]
fn test_apply_kvs_all_applied_produces_no_messages() {
    use crate::application::console::traits::{apply_kvs, Outcome};
    let mut doc = load_test_doc();
    let nid = doc.mindmap.nodes.keys().next().unwrap().clone();
    doc.selection = SelectionState::Single(nid);
    let kvs = vec![("bg".to_string(), "#123".to_string())];
    let report = apply_kvs(&mut doc, &kvs, |_v, _k, _val| Some(Outcome::Applied));
    assert!(report.any_applied);
    assert!(report.messages.is_empty());
    assert!(!report.all_failed);
}

/// Invalid outcome surfaces as an error message for that pair.
#[test]
fn test_apply_kvs_invalid_is_reported_as_error_per_pair() {
    use crate::application::console::traits::{apply_kvs, Outcome};
    let mut doc = load_test_doc();
    let nid = doc.mindmap.nodes.keys().next().unwrap().clone();
    doc.selection = SelectionState::Single(nid);
    let kvs = vec![("size".to_string(), "nope".to_string())];
    let report = apply_kvs(&mut doc, &kvs, |_v, _k, val| {
        Some(Outcome::Invalid(format!("'{}' is not a number", val)))
    });
    assert!(report.all_failed);
    assert_eq!(report.messages.len(), 1);
    assert!(report.messages[0].contains("size"));
    assert!(report.messages[0].contains("not a number"));
}

// ============================================================
// Multi-selection fanout + trait dispatcher aggregation
// ============================================================

/// A `color bg=#abc` run against a `Multi(ids)` selection must
/// mutate *every* selected node and push one undo entry per node —
/// the fanout contract.
#[test]
fn test_color_bg_fans_out_across_multi_selection() {
    let mut doc = load_test_doc();
    let mut ids = doc.mindmap.nodes.keys().cloned();
    let a = ids.next().unwrap();
    let b = ids.next().unwrap();
    doc.selection = SelectionState::Multi(vec![a.clone(), b.clone()]);
    doc.undo_stack.clear();
    let _ = run("color bg=#402030", &mut doc);
    assert_eq!(doc.mindmap.nodes.get(&a).unwrap().style.background_color, "#402030");
    assert_eq!(doc.mindmap.nodes.get(&b).unwrap().style.background_color, "#402030");
    // One undo per node — the dispatcher doesn't batch.
    assert_eq!(doc.undo_stack.len(), 2);
}

/// `label text=...` with a node selection — HasLabel only matches
/// Edge, so the dispatcher should report "not applicable" and leave
/// the node untouched.
#[test]
fn test_label_text_kv_not_applicable_on_node_selection() {
    let mut doc = load_test_doc();
    let nid = doc.mindmap.nodes.keys().next().unwrap().clone();
    doc.selection = SelectionState::Single(nid);
    let result = run(r#"label text="hello""#, &mut doc);
    // Dispatcher reports the single pair as NotApplicable; with
    // nothing applied it turns the report into an Err.
    assert!(matches!(result, ExecResult::Err(_)), "got {:?}", result);
}

#[test]
fn test_portal_create_with_two_nodes_selected() {
    let mut doc = load_test_doc();
    let mut ids = doc.mindmap.nodes.keys().cloned();
    let a = ids.next().unwrap();
    let b = ids.next().unwrap();
    doc.selection = SelectionState::Multi(vec![a, b]);
    let _ = run("portal create", &mut doc);
    assert!(matches!(doc.selection, SelectionState::Portal(_)));
    assert_eq!(doc.mindmap.portals.len(), 1);
}

#[test]
fn test_portal_create_errors_without_two_node_selection() {
    let mut doc = load_test_doc();
    doc.selection = SelectionState::None;
    let result = run("portal create", &mut doc);
    assert!(matches!(result, ExecResult::Err(_)));
}

#[test]
fn test_portal_glyph_hexagon_updates_marker() {
    let mut doc = load_test_doc();
    let mut ids = doc.mindmap.nodes.keys().cloned();
    let a = ids.next().unwrap();
    let b = ids.next().unwrap();
    let pref: PortalRef = doc.apply_create_portal(&a, &b).unwrap();
    doc.selection = SelectionState::Portal(pref);
    let _ = run("portal glyph=hexagon", &mut doc);
    assert_eq!(doc.mindmap.portals[0].glyph, "\u{2B21}");
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

// ============================================================
// Applicability
// ============================================================

#[test]
fn test_applicability_anchor_hidden_without_edge() {
    let doc = load_test_doc();
    let ctx = ConsoleContext::from_document(&doc);
    let cmd = command_by_name("anchor").unwrap();
    assert!(!(cmd.applicable)(&ctx));
}

#[test]
fn test_applicability_anchor_visible_with_edge() {
    let mut doc = load_test_doc();
    let _ = select_first_edge(&mut doc);
    let ctx = ConsoleContext::from_document(&doc);
    let cmd = command_by_name("anchor").unwrap();
    assert!((cmd.applicable)(&ctx));
}

#[test]
fn test_applicability_portal_always_true() {
    let doc = load_test_doc();
    let ctx = ConsoleContext::from_document(&doc);
    let cmd = command_by_name("portal").unwrap();
    assert!((cmd.applicable)(&ctx));
}

#[test]
fn test_applicability_help_always_true() {
    let doc = load_test_doc();
    let ctx = ConsoleContext::from_document(&doc);
    let cmd = command_by_name("help").unwrap();
    assert!((cmd.applicable)(&ctx));
}

// ============================================================
// Completion
// ============================================================

#[test]
fn test_complete_command_name_prefix() {
    let doc = load_test_doc();
    let ctx = ConsoleContext::from_document(&doc);
    let results = complete("he", 2, &ctx);
    assert!(results.iter().any(|c| c.text == "help"));
}

#[test]
fn test_complete_command_name_empty_input() {
    let doc = load_test_doc();
    let ctx = ConsoleContext::from_document(&doc);
    let results = complete("", 0, &ctx);
    assert!(results.iter().any(|c| c.text == "help"));
    // Non-applicable commands (e.g. `anchor` without edge selected)
    // should be hidden.
    assert!(!results.iter().any(|c| c.text == "anchor"));
}

#[test]
fn test_complete_anchor_kv_keys() {
    let mut doc = load_test_doc();
    let _ = select_first_edge(&mut doc);
    let ctx = ConsoleContext::from_document(&doc);
    let results = complete("anchor ", 7, &ctx);
    let names: Vec<&str> = results.iter().map(|c| c.text.as_str()).collect();
    assert!(names.contains(&"from="));
    assert!(names.contains(&"to="));
}

#[test]
fn test_complete_anchor_kv_values() {
    let mut doc = load_test_doc();
    let _ = select_first_edge(&mut doc);
    let ctx = ConsoleContext::from_document(&doc);
    let results = complete("anchor from=", 12, &ctx);
    let names: Vec<&str> = results.iter().map(|c| c.text.as_str()).collect();
    assert!(names.contains(&"auto"));
    assert!(names.contains(&"top"));
}

#[test]
fn test_complete_unknown_command_returns_empty_for_args() {
    let doc = load_test_doc();
    let ctx = ConsoleContext::from_document(&doc);
    // "wibble " has a trailing space so cursor is at token 1.
    let results = complete("wibble ", 7, &ctx);
    assert!(results.is_empty());
}

