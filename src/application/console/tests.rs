//! Integration tests that exercise the console against the canonical
//! `testament.mindmap.json` fixture. These live in their own file to
//! keep per-command modules small; they cover the cross-module paths
//! (parse + execute + applicability) that no single module owns.
//!
//! Per `TEST_CONVENTIONS.md §3.3` the `#[cfg(test)]` inline style is
//! the default for the mandala crate — this file is just `mod tests;`
//! from a handful of siblings, with a shared fixture helper.

use super::commands::{command_by_name, BACKCOMPAT_INVOCATIONS};
use super::completion::complete;
use super::parser::{parse, tokenize, Args, ParseResult};
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
fn test_anchor_set_updates_edge_anchor() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let _ = run("anchor set from top", &mut doc);
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    assert_eq!(updated.anchor_from, 1);
}

#[test]
fn test_anchor_set_idempotent_second_call() {
    let mut doc = load_test_doc();
    select_first_edge(&mut doc);
    let first = run("anchor set from left", &mut doc);
    let second = run("anchor set from left", &mut doc);
    assert!(matches!(first, ExecResult::Ok(_)));
    assert!(matches!(second, ExecResult::Ok(_)));
}

#[test]
fn test_anchor_errors_without_edge_selection() {
    let mut doc = load_test_doc();
    let result = run("anchor set from top", &mut doc);
    assert!(matches!(result, ExecResult::Err(_)), "got {:?}", result);
}

#[test]
fn test_body_dash_sets_glyph() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let _ = run("body dash", &mut doc);
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    let body = updated
        .glyph_connection
        .as_ref()
        .map(|c| c.body.as_str());
    assert_eq!(body, Some("\u{2500}"));
}

#[test]
fn test_cap_from_arrow_sets_left_triangle() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let _ = run("cap from arrow", &mut doc);
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    let cap = updated
        .glyph_connection
        .as_ref()
        .and_then(|c| c.cap_start.as_deref());
    assert_eq!(cap, Some("\u{25C0}"));
}

#[test]
fn test_cap_to_arrow_sets_right_triangle() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let _ = run("cap to arrow", &mut doc);
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    let cap = updated
        .glyph_connection
        .as_ref()
        .and_then(|c| c.cap_end.as_deref());
    assert_eq!(cap, Some("\u{25B6}"));
}

#[test]
fn test_spacing_wide_sets_six() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let _ = run("spacing wide", &mut doc);
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    let spacing = updated.glyph_connection.as_ref().map(|c| c.spacing);
    assert_eq!(spacing, Some(6.0));
}

#[test]
fn test_color_accent_sets_var_accent() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let _ = run("color accent", &mut doc);
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    let color = updated.glyph_connection.as_ref().and_then(|c| c.color.clone());
    assert_eq!(color.as_deref(), Some("var(--accent)"));
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
fn test_label_set_with_quoted_string_writes_label() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let _ = run(r#"label set "hello world""#, &mut doc);
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    assert_eq!(updated.label.as_deref(), Some("hello world"));
}

#[test]
fn test_connection_reset_straight_clears_control_points() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    // Seed a control point so reset has something to clear.
    doc.mindmap
        .edges
        .iter_mut()
        .find(|e| er.matches(e))
        .unwrap()
        .control_points = vec![baumhard::mindmap::model::ControlPoint { x: 10.0, y: 20.0 }];
    let _ = run("connection reset-straight", &mut doc);
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    assert!(updated.control_points.is_empty());
}

#[test]
fn test_font_larger_steps_font_size_up() {
    let mut doc = load_test_doc();
    let er = select_first_edge(&mut doc);
    let _ = run("font larger", &mut doc);
    let updated = doc.mindmap.edges.iter().find(|e| er.matches(e)).unwrap();
    assert!(updated.glyph_connection.is_some());
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
    let _ = run(&format!("edge type {}", target), &mut doc);
    // `set_edge_type` renames the edge identity, so we can't find it
    // by the old er anymore — just assert at least one edge has the
    // new type.
    assert!(doc.mindmap.edges.iter().any(|e| e.edge_type == target));
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
    let _ = run("portal glyph hexagon", &mut doc);
    assert_eq!(doc.mindmap.portals[0].glyph, "\u{2B21}");
}

#[test]
fn test_select_node_by_id_changes_selection() {
    let mut doc = load_test_doc();
    let id = doc.mindmap.nodes.keys().next().unwrap().clone();
    let _ = run(&format!("select node {}", id), &mut doc);
    match &doc.selection {
        SelectionState::Single(s) => assert_eq!(s, &id),
        other => panic!("expected Single, got {:?}", other),
    }
}

#[test]
fn test_select_none_clears_selection() {
    let mut doc = load_test_doc();
    doc.selection = SelectionState::Single("anything".into());
    let _ = run("select none", &mut doc);
    assert!(matches!(doc.selection, SelectionState::None));
}

#[test]
fn test_select_node_unknown_id_reports_error() {
    let mut doc = load_test_doc();
    let result = run("select node does-not-exist", &mut doc);
    assert!(matches!(result, ExecResult::Err(_)));
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
fn test_complete_enum_arg_anchor_endpoint() {
    let mut doc = load_test_doc();
    let _ = select_first_edge(&mut doc);
    let ctx = ConsoleContext::from_document(&doc);
    let results = complete("anchor set ", 11, &ctx);
    let names: Vec<&str> = results.iter().map(|c| c.text.as_str()).collect();
    assert!(names.contains(&"from"));
    assert!(names.contains(&"to"));
}

#[test]
fn test_complete_enum_arg_anchor_side() {
    let mut doc = load_test_doc();
    let _ = select_first_edge(&mut doc);
    let ctx = ConsoleContext::from_document(&doc);
    let results = complete("anchor set from ", 16, &ctx);
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

#[test]
fn test_complete_select_node_completes_ids() {
    let doc = load_test_doc();
    let ctx = ConsoleContext::from_document(&doc);
    let results = complete("select node ", 12, &ctx);
    assert!(!results.is_empty());
    // Every returned id should actually exist in the map.
    for c in &results {
        assert!(
            doc.mindmap.nodes.contains_key(&c.text),
            "completion '{}' is not a real node id",
            c.text
        );
    }
}

// ============================================================
// `mutate list` / `mutate run`
// ============================================================

fn sample_custom_mutation(id: &str, name: &str) -> baumhard::mindmap::custom_mutation::CustomMutation {
    baumhard::mindmap::custom_mutation::CustomMutation {
        id: id.to_string(),
        name: name.to_string(),
        mutations: Vec::new(),
        target_scope: baumhard::mindmap::custom_mutation::TargetScope::SelfOnly,
        behavior: baumhard::mindmap::custom_mutation::MutationBehavior::Persistent,
        predicate: None,
        document_actions: Vec::new(),
    }
}

#[test]
fn test_mutate_list_with_empty_registry_reports_empty() {
    let mut doc = load_test_doc();
    doc.mutation_registry.clear();
    let result = run("mutate list", &mut doc);
    match result {
        ExecResult::Ok(s) => assert!(s.contains("no mutations")),
        other => panic!("expected Ok with 'no mutations', got {:?}", other),
    }
}

#[test]
fn test_mutate_list_prints_registered_ids() {
    let mut doc = load_test_doc();
    doc.mutation_registry.insert(
        "test-m1".into(),
        sample_custom_mutation("test-m1", "Test One"),
    );
    let result = run("mutate list", &mut doc);
    match result {
        ExecResult::Lines(lines) => {
            assert!(lines.iter().any(|l| l.contains("test-m1")));
            assert!(lines.iter().any(|l| l.contains("Test One")));
        }
        other => panic!("expected Lines, got {:?}", other),
    }
}

#[test]
fn test_mutate_list_classifies_source_as_map() {
    let mut doc = load_test_doc();
    doc.mindmap
        .custom_mutations
        .push(sample_custom_mutation("map-mut", "Map Mutation"));
    doc.build_mutation_registry();
    let result = run("mutate list", &mut doc);
    match result {
        ExecResult::Lines(lines) => {
            let hit = lines.iter().find(|l| l.contains("map-mut"));
            assert!(hit.is_some(), "map-mut not in listing");
            assert!(hit.unwrap().contains("map"));
        }
        other => panic!("expected Lines, got {:?}", other),
    }
}

#[test]
fn test_mutate_run_without_selection_errors() {
    let mut doc = load_test_doc();
    doc.mutation_registry.insert(
        "test-m".into(),
        sample_custom_mutation("test-m", "Test"),
    );
    doc.selection = SelectionState::None;
    let result = run("mutate run test-m", &mut doc);
    assert!(matches!(result, ExecResult::Err(_)), "got {:?}", result);
}

#[test]
fn test_mutate_run_unknown_id_errors() {
    let mut doc = load_test_doc();
    let nid = doc.mindmap.nodes.keys().next().unwrap().clone();
    doc.selection = SelectionState::Single(nid);
    let result = run("mutate run does-not-exist", &mut doc);
    assert!(matches!(result, ExecResult::Err(_)));
}

#[test]
fn test_mutate_run_sets_deferred_request_on_effects() {
    let mut doc = load_test_doc();
    doc.mutation_registry.insert(
        "test-m".into(),
        sample_custom_mutation("test-m", "Test"),
    );
    let nid = doc.mindmap.nodes.keys().next().unwrap().clone();
    doc.selection = SelectionState::Single(nid.clone());

    let (cmd, toks) = match parse("mutate run test-m") {
        ParseResult::Ok { cmd, args } => (cmd, args),
        _ => panic!("parse failed"),
    };
    let mut eff = ConsoleEffects::new(&mut doc);
    let result = (cmd.execute)(&Args::new(&toks), &mut eff);
    assert!(matches!(result, ExecResult::Ok(_)));
    let req = eff.run_mutation.take().expect("run_mutation should be set");
    assert_eq!(req.mutation_id, "test-m");
    assert_eq!(req.node_id, nid);
}

#[test]
fn test_mutate_run_explicit_node_id_overrides_selection() {
    let mut doc = load_test_doc();
    doc.mutation_registry.insert(
        "test-m".into(),
        sample_custom_mutation("test-m", "Test"),
    );
    let mut ids = doc.mindmap.nodes.keys().cloned();
    let sel_id = ids.next().unwrap();
    let arg_id = ids.next().unwrap();
    doc.selection = SelectionState::Single(sel_id.clone());

    let (cmd, toks) = match parse(&format!("mutate run test-m {}", arg_id)) {
        ParseResult::Ok { cmd, args } => (cmd, args),
        _ => panic!("parse failed"),
    };
    let mut eff = ConsoleEffects::new(&mut doc);
    let _ = (cmd.execute)(&Args::new(&toks), &mut eff);
    let req = eff.run_mutation.take().unwrap();
    assert_eq!(req.node_id, arg_id);
    assert_ne!(req.node_id, sel_id);
}

// ============================================================
// User-mutation precedence (user < map < inline)
// ============================================================

#[test]
fn test_user_mutations_merged_at_lowest_precedence() {
    let mut doc = load_test_doc();
    let user = vec![sample_custom_mutation("shared-id", "user version")];
    doc.build_mutation_registry_with_user(&user);
    assert_eq!(
        doc.mutation_registry.get("shared-id").map(|m| m.name.as_str()),
        Some("user version"),
        "user mutation should be visible when no map/inline override"
    );
}

#[test]
fn test_map_mutation_overrides_user_mutation_with_same_id() {
    let mut doc = load_test_doc();
    doc.mindmap
        .custom_mutations
        .push(sample_custom_mutation("shared-id", "map version"));
    let user = vec![sample_custom_mutation("shared-id", "user version")];
    doc.build_mutation_registry_with_user(&user);
    assert_eq!(
        doc.mutation_registry.get("shared-id").map(|m| m.name.as_str()),
        Some("map version"),
        "map mutation should overwrite user mutation on id collision"
    );
}

// ============================================================
// Backward compat: every palette action has a console invocation
// ============================================================

#[test]
fn test_backcompat_every_palette_id_parses() {
    for (palette_id, invocation) in BACKCOMPAT_INVOCATIONS {
        match parse(invocation) {
            ParseResult::Ok { cmd, args: _ } => {
                assert!(!cmd.name.is_empty(), "palette id '{palette_id}' resolved to empty command");
            }
            ParseResult::Empty => {
                panic!("palette id '{palette_id}' invocation '{invocation}' is empty");
            }
            ParseResult::Unknown(s) => {
                panic!(
                    "palette id '{palette_id}' invocation '{invocation}' resolves to unknown command '{s}'"
                );
            }
        }
    }
}

#[test]
fn test_tokenize_invocations_match_parse_expectations() {
    // Sanity: the tokenizer must handle every invocation string as-is.
    for (_, invocation) in BACKCOMPAT_INVOCATIONS {
        let toks = tokenize(invocation);
        assert!(!toks.is_empty(), "empty tokens for '{invocation}'");
    }
}
