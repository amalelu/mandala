//! `apply_kvs` dispatcher aggregation tests.

use super::fixtures::load_test_doc;
use crate::application::console::traits::{apply_kvs, Outcome};
use crate::application::document::SelectionState;

/// Empty selection → "no target" message, report marked as all_failed.
#[test]
fn test_apply_kvs_with_no_selection_reports_no_target_and_fails() {
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
