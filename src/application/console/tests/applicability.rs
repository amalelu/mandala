//! Per-command `is_applicable` predicate tests — verifies that
//! commands hide / show themselves correctly based on the current
//! selection and document state.

use super::fixtures::{load_test_doc, select_first_edge};
use crate::application::console::commands::command_by_name;
use crate::application::console::ConsoleContext;

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
fn test_applicability_edge_requires_edge_or_two_nodes() {
    // Portals are now edges; the standalone `portal` command went
    // away and its verbs folded into `edge`. The `edge` command is
    // applicable when an edge (line-mode or portal-mode) or a
    // portal-label selection is active.
    let mut doc = load_test_doc();
    // With nothing selected, not applicable.
    let ctx = ConsoleContext::from_document(&doc);
    let cmd = command_by_name("edge").unwrap();
    assert!(!(cmd.applicable)(&ctx));
    // With an edge selected, applicable.
    let _ = select_first_edge(&mut doc);
    let ctx = ConsoleContext::from_document(&doc);
    assert!((cmd.applicable)(&ctx));
}

#[test]
fn test_applicability_help_always_true() {
    let doc = load_test_doc();
    let ctx = ConsoleContext::from_document(&doc);
    let cmd = command_by_name("help").unwrap();
    assert!((cmd.applicable)(&ctx));
}
