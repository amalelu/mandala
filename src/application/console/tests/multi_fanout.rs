// SPDX-License-Identifier: MPL-2.0

//! Multi-selection fanout + trait dispatcher aggregation tests.
//! Verifies that commands invoked against `Multi(ids)` reach every
//! selected target and that NotApplicable propagates correctly when
//! the bound trait doesn't match the target type.

use super::fixtures::{first_node_id, load_test_doc, run, two_testament_node_ids};
use crate::application::console::ExecResult;
use crate::application::document::SelectionState;

/// A `color bg=#abc` run against a `Multi(ids)` selection must
/// mutate *every* selected node and push one undo entry per node —
/// the fanout contract.
#[test]
fn test_color_bg_fans_out_across_multi_selection() {
    let mut doc = load_test_doc();
    let (a, b) = two_testament_node_ids(&doc);
    doc.selection = SelectionState::Multi(vec![a.clone(), b.clone()]);
    doc.undo_stack.clear();
    let _ = run("color bg=#402030", &mut doc);
    // Through the cascade: the fixture's nodes are themed, so the
    // fill they paint is not `style.background_color`.
    for id in [&a, &b] {
        let node = doc.mindmap.nodes.get(id).unwrap();
        assert_eq!(doc.mindmap.node_background_color(node), "#402030");
    }
    // One undo per node — the dispatcher doesn't batch.
    assert_eq!(doc.undo_stack.len(), 2);
}

/// `label text=...` with a node selection — HasLabel only matches
/// Edge, so the dispatcher should report "not applicable" and leave
/// the node untouched.
#[test]
fn test_label_text_kv_not_applicable_on_node_selection() {
    let mut doc = load_test_doc();
    let nid = first_node_id(&doc);
    doc.selection = SelectionState::Single(nid);
    let result = run(r#"label text="hello""#, &mut doc);
    // Dispatcher reports the single pair as NotApplicable; with
    // nothing applied it turns the report into an Err.
    assert!(matches!(result, ExecResult::Err(_)), "got {:?}", result);
}
