// SPDX-License-Identifier: MPL-2.0

//! `apply_kvs` dispatcher aggregation tests.

use super::fixtures::{first_node_id, load_test_doc, two_testament_node_ids};
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
    let (a, b) = two_testament_node_ids(&doc);
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
    let (a, b) = two_testament_node_ids(&doc);
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
    let nid = first_node_id(&doc);
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
    let nid = first_node_id(&doc);
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

/// `invalid` names the one failure class no substring of
/// `messages` can identify: an [`Outcome::Invalid`] message is
/// whatever the target wrote.
///
/// Fails when: `invalid` is filled from `messages` wholesale (the
/// not-applicable, unknown-key, no-target and already-set rows all
/// go red), or when the `Invalid` arm stops recording it (the
/// first row goes red). Each row pins a *different* silent-no-op
/// shape against the same field, which is what makes "only Invalid
/// lands here" a claim rather than a coincidence.
///
/// The `not applicable` row is the control the caller actually
/// depends on: `log_not_applicable_if_silent` reports it at
/// `info!` and an invalid value at `warn!`, and it can only tell
/// them apart if this field separates them.
#[test]
fn test_dispatch_report_records_only_invalid_values_in_invalid() {
    let one_pair = || vec![("bg".to_string(), "#123".to_string())];

    let mut doc = load_test_doc();
    doc.selection = SelectionState::Single(first_node_id(&doc));
    let report = apply_kvs(&mut doc, &one_pair(), |_v, _k, _val| {
        Some(Outcome::Invalid("not a color I can parse".into()))
    });
    assert_eq!(report.invalid.len(), 1, "an Invalid outcome must be recorded");
    assert!(
        report.invalid[0].contains("not a color I can parse"),
        "the target's own words are the diagnosis: {:?}",
        report.invalid
    );

    let mut doc = load_test_doc();
    doc.selection = SelectionState::Single(first_node_id(&doc));
    let report = apply_kvs(&mut doc, &one_pair(), |_v, _k, _val| Some(Outcome::NotApplicable));
    assert!(
        report.invalid.is_empty(),
        "a wrong-selection outcome is not an unusable value: {:?}",
        report.invalid
    );
    assert!(
        report.messages.iter().any(|m| m.contains("not applicable")),
        "control: this row must actually produce the not-applicable message it stands for"
    );

    let mut doc = load_test_doc();
    doc.selection = SelectionState::Single(first_node_id(&doc));
    let report = apply_kvs(&mut doc, &one_pair(), |_v, _k, _val| Some(Outcome::Unchanged));
    assert!(
        report.invalid.is_empty(),
        "already at the requested value is not a rejection: {:?}",
        report.invalid
    );

    let mut doc = load_test_doc();
    doc.selection = SelectionState::Single(first_node_id(&doc));
    let report = apply_kvs(&mut doc, &one_pair(), |_v, _k, _val| None);
    assert!(
        report.invalid.is_empty(),
        "an unknown key is a typo in the key, not an unusable value: {:?}",
        report.invalid
    );

    let mut doc = load_test_doc();
    doc.selection = SelectionState::None;
    let report = apply_kvs(&mut doc, &one_pair(), |_v, _k, _val| Some(Outcome::Applied));
    assert!(
        report.invalid.is_empty(),
        "having nothing selected is not an unusable value: {:?}",
        report.invalid
    );
}
