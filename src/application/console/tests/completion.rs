//! Completion-engine tests — verifies that the completion path emits
//! the expected suggestions for command-name prefixes, kv keys, kv
//! values, and unknown commands.

use super::fixtures::{load_test_doc, select_first_edge};
use crate::application::console::completion::complete;
use crate::application::console::ConsoleContext;

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
