// SPDX-License-Identifier: MPL-2.0

//! Unit tests for the trait dispatcher's data types — color value
//! parsing, outcome helper, and selection materialization.

use super::*;
use crate::application::console::constants::{VAR_ACCENT, VAR_EDGE, VAR_FG};
use crate::application::document::{GraphemeRange, SectionSel, SectionSpan, SelectionState};

#[test]
fn test_parse_hex_ok() {
    assert_eq!(ColorValue::parse("#123").unwrap(), ColorValue::Hex("#123".into()));
    assert_eq!(
        ColorValue::parse("#009c15").unwrap(),
        ColorValue::Hex("#009c15".into())
    );
    assert_eq!(
        ColorValue::parse("#009c15ff").unwrap(),
        ColorValue::Hex("#009c15ff".into())
    );
}

#[test]
fn test_parse_hex_rejects_bad_length() {
    assert!(ColorValue::parse("#12").is_err());
    assert!(ColorValue::parse("#12345").is_err());
    assert!(ColorValue::parse("#zzzzzz").is_err());
}

#[test]
fn test_parse_var_tokens() {
    assert_eq!(ColorValue::parse("accent").unwrap(), ColorValue::Var(VAR_ACCENT));
    assert_eq!(ColorValue::parse("ACCENT").unwrap(), ColorValue::Var(VAR_ACCENT));
    assert_eq!(ColorValue::parse("fg").unwrap(), ColorValue::Var(VAR_FG));
    assert_eq!(ColorValue::parse("edge").unwrap(), ColorValue::Var(VAR_EDGE));
}

#[test]
fn test_parse_reset() {
    assert_eq!(ColorValue::parse("reset").unwrap(), ColorValue::Reset);
}

#[test]
fn test_parse_unknown_is_error() {
    assert!(ColorValue::parse("bogus").is_err());
}

#[test]
fn test_selection_targets_for_each_variant() {
    use crate::application::document::EdgeRef;
    assert!(selection_targets(&SelectionState::None).is_empty());

    let ids = vec!["a".to_string(), "b".to_string()];
    let out = selection_targets(&SelectionState::Multi(ids.clone()));
    assert_eq!(out.len(), 2);

    let er = EdgeRef::new("a", "b", "cross_link");
    let out = selection_targets(&SelectionState::Edge(er));
    assert!(matches!(out.as_slice(), [TargetId::Edge(_)]));
}

/// `selection_targets` fans `MultiSection` out to one
/// `TargetId::Section` per entry — each per-section verb
/// (color text, font size / family) applies to every section.
#[test]
fn test_selection_targets_multisection_fans_out_per_entry() {
    use crate::application::document::SectionSel;
    let secs = vec![
        SectionSel::new("a", 0),
        SectionSel::new("a", 2),
        SectionSel::new("b", 1),
    ];
    let out = selection_targets(&SelectionState::MultiSection(secs.clone()));
    assert_eq!(out.len(), 3);
    for (i, target) in out.iter().enumerate() {
        match target {
            TargetId::Section {
                node_id,
                section_idx,
                range,
            } => {
                assert_eq!(node_id, &secs[i].node_id);
                assert_eq!(*section_idx, secs[i].section_idx);
                assert!(range.is_none(), "MultiSection fan-out has no sub-range");
            }
            _ => panic!("expected TargetId::Section, got non-section variant"),
        }
    }
}

/// `selection_targets` fans `SelectionState::SectionRange` out
/// to a single `TargetId::Section { range: Some(_), .. }` — the
/// range threads into the dispatcher's `TargetView::Section`
/// arm, where range-aware setters consult it.
#[test]
fn test_selection_targets_section_range_carries_range() {
    let sel = SelectionState::SectionRange {
        sel: SectionSel::new("a", 1),
        section_span: SectionSpan::single(1),
        grapheme_range: GraphemeRange::new(3, 7),
    };
    let out = selection_targets(&sel);
    assert_eq!(out.len(), 1);
    match &out[0] {
        TargetId::Section {
            node_id,
            section_idx,
            range,
        } => {
            assert_eq!(node_id, "a");
            assert_eq!(*section_idx, 1);
            assert_eq!(*range, Some(GraphemeRange::new(3, 7)));
        }
        _ => panic!("expected TargetId::Section"),
    }
}

/// `Section` and `MultiSection` continue to fan out with
/// `range: None` — pin the back-compat invariant the trait
/// dispatcher relies on.
#[test]
fn test_selection_targets_section_carries_no_range() {
    let sel = SelectionState::Section(SectionSel::new("a", 0));
    let out = selection_targets(&sel);
    assert_eq!(out.len(), 1);
    match &out[0] {
        TargetId::Section { range, .. } => assert!(range.is_none()),
        _ => panic!("expected TargetId::Section"),
    }
}

/// Cut on a `TargetView::Section { range: Some(_), .. }` returns
/// the in-range graphemes as plain text and removes them from
/// the section's text.
#[test]
fn test_section_range_cut_returns_in_range_text_and_shrinks_section() {
    use crate::application::document::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();
    doc.set_section_text(&id, 0, "abcdefgh".into());
    let mut view = TargetView::Section {
        doc: &mut doc,
        id: id.clone(),
        section_idx: 0,
        range: Some(GraphemeRange::new(2, 5)),
    };
    let outcome = view.clipboard_cut();
    let ClipboardContent::Text(text) = outcome else {
        panic!("expected Text, got {:?}", outcome);
    };
    assert_eq!(text, "cde");
    assert_eq!(doc.mindmap.nodes[&id].sections[0].text, "abfgh");
}

/// **Paste on `TargetView::Section { range: Some(_), .. }`
/// replaces the in-range graphemes with the clipboard
/// content.**
#[test]
fn test_section_range_paste_replaces_in_range_text() {
    use crate::application::document::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();
    doc.set_section_text(&id, 0, "abcdefgh".into());
    let mut view = TargetView::Section {
        doc: &mut doc,
        id: id.clone(),
        section_idx: 0,
        range: Some(GraphemeRange::new(2, 5)),
    };
    let outcome = view.clipboard_paste("XYZW");
    assert!(matches!(outcome, Outcome::Applied));
    assert_eq!(doc.mindmap.nodes[&id].sections[0].text, "abXYZWfgh");
}

/// Range-aware Cut on a multi-byte grapheme cluster — pin that
/// the byte/grapheme indexing handles emoji correctly. The
/// regional-indicator pair `🇺🇸` is one extended grapheme but
/// 8 UTF-8 bytes; cutting it must not split mid-cluster.
#[test]
fn test_section_range_cut_handles_emoji_grapheme() {
    use crate::application::document::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();
    doc.set_section_text(&id, 0, "a\u{1F1FA}\u{1F1F8}b".into()); // "a🇺🇸b" — 3 graphemes
    let mut view = TargetView::Section {
        doc: &mut doc,
        id: id.clone(),
        section_idx: 0,
        range: Some(GraphemeRange::new(1, 2)),
    };
    let outcome = view.clipboard_cut();
    let ClipboardContent::Text(text) = outcome else {
        panic!("expected Text, got {:?}", outcome);
    };
    assert_eq!(text, "\u{1F1FA}\u{1F1F8}");
    assert_eq!(doc.mindmap.nodes[&id].sections[0].text, "ab");
}

/// Range-aware Cut with `range_start >= clamped_end` (degenerate
/// empty range) returns Empty without modifying the section.
#[test]
fn test_section_range_cut_empty_range_returns_empty_no_change() {
    use crate::application::document::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();
    doc.set_section_text(&id, 0, "abcde".into());
    let before = doc.mindmap.nodes[&id].sections[0].text.clone();
    let mut view = TargetView::Section {
        doc: &mut doc,
        id: id.clone(),
        section_idx: 0,
        range: Some(GraphemeRange::new(3, 3)),
    };
    let outcome = view.clipboard_cut();
    assert!(matches!(outcome, ClipboardContent::Empty));
    assert_eq!(doc.mindmap.nodes[&id].sections[0].text, before);
}

/// Range-aware Paste with `range_end > total` clamps end to total.
#[test]
fn test_section_range_paste_clamps_end_past_total() {
    use crate::application::document::tests_common::pinned_two_section_node;
    let (mut doc, id) = pinned_two_section_node();
    doc.set_section_text(&id, 0, "abcde".into());
    let mut view = TargetView::Section {
        doc: &mut doc,
        id: id.clone(),
        section_idx: 0,
        range: Some(GraphemeRange::new(3, 999)),
    };
    let outcome = view.clipboard_paste("XY");
    assert!(matches!(outcome, Outcome::Applied));
    assert_eq!(doc.mindmap.nodes[&id].sections[0].text, "abcXY");
}

/// **Dispatcher routes range to the range-aware setter.** A
/// `TargetView::Section { range: Some(_), .. }` color write
/// must hit `set_section_text_color_range` (which only mutates
/// in-range runs), not the whole-section setter. Pin by
/// constructing a `Section` selection extended with a sub-range,
/// dispatching `apply_wheel_color`, and asserting only the
/// in-range runs changed color.
#[test]
fn test_section_range_dispatches_to_range_aware_color_setter() {
    use crate::application::document::tests_common::pinned_two_section_node;
    use crate::application::document::SelectionState;

    let (mut doc, id) = pinned_two_section_node();
    // Set up a known 10-grapheme section with a single run.
    {
        let n = doc.mindmap.nodes.get_mut(&id).unwrap();
        let s = &mut n.sections[0];
        s.text = "abcdefghij".into();
        s.text_runs.clear();
        s.text_runs.push(baumhard::mindmap::model::TextRun {
            start: 0,
            end: 10,
            bold: false,
            italic: false,
            underline: false,
            font: "LiberationSans".into(),
            size_pt: 14.0,
            color: "#ffffff".into(),
            hyperlink: None,
        });
    }
    doc.selection = SelectionState::SectionRange {
        sel: SectionSel::new(&id, 0),
        section_span: SectionSpan::single(0),
        grapheme_range: GraphemeRange::new(3, 7),
    };

    let targets = selection_targets(&doc.selection);
    assert_eq!(targets.len(), 1);
    for tid in &targets {
        let mut view = view_for(&mut doc, tid);
        let outcome = view.set_text_color(ColorValue::Hex("#abcdef".into()));
        assert!(matches!(outcome, Outcome::Applied));
    }

    // The mutation should split the original [0..10) run into
    // three: [0..3 white, 3..7 #abcdef, 7..10 white].
    let runs = &doc.mindmap.nodes.get(&id).unwrap().sections[0].text_runs;
    assert_eq!(runs.len(), 3, "expected three runs after range carve-out");
    assert_eq!(runs[0].color, "#ffffff");
    assert_eq!(runs[1].color, "#abcdef");
    assert_eq!(runs[1].start, 3);
    assert_eq!(runs[1].end, 7);
    assert_eq!(runs[2].color, "#ffffff");
}

// ── apply_to_targets aggregation paths ─────────────────────────
//
// These pin the four outcome shapes the dispatcher folds into a
// `DispatchReport`: empty selection, all-NotApplicable, mixed
// Applied/Unchanged, and Invalid. The font command's
// happy-path / not-applicable cases live in `commands::font::tests`;
// here we cover the dispatcher mechanics directly via a stub
// closure that doesn't touch the doc.

use crate::application::document::tests_common::{
    first_n_testament_node_ids as first_n_node_ids, load_test_doc as fresh_doc,
};

#[test]
fn test_apply_to_targets_empty_selection_reports_no_target() {
    let mut doc = fresh_doc();
    doc.selection = SelectionState::None;
    let report = apply_to_targets(&mut doc, |_| Outcome::Applied);
    assert!(report.all_failed);
    assert!(!report.any_applied);
    assert_eq!(report.messages.len(), 1);
    assert!(report.messages[0].contains("no target"));
}

#[test]
fn test_apply_to_targets_multi_node_fanout_aggregates_applied() {
    let mut doc = fresh_doc();
    let ids = first_n_node_ids(&doc, 3);
    assert_eq!(ids.len(), 3);
    doc.selection = SelectionState::Multi(ids);

    let mut calls = 0;
    let report = apply_to_targets(&mut doc, |_view| {
        calls += 1;
        Outcome::Applied
    });
    assert_eq!(calls, 3, "op called once per multi-selected node");
    assert!(report.any_applied);
    assert!(!report.all_failed);
    assert!(report.messages.is_empty());
}

#[test]
fn test_apply_to_targets_all_not_applicable_surfaces_label() {
    let mut doc = fresh_doc();
    let ids = first_n_node_ids(&doc, 2);
    doc.selection = SelectionState::Multi(ids);
    let report = apply_to_targets(&mut doc, |_| Outcome::NotApplicable);
    assert!(!report.any_applied);
    assert!(report.all_failed);
    assert_eq!(report.messages.len(), 1);
    assert!(report.messages[0].contains("not applicable to nodes"));
}

#[test]
fn test_apply_to_targets_invalid_surfaces_per_target_messages() {
    let mut doc = fresh_doc();
    let ids = first_n_node_ids(&doc, 2);
    doc.selection = SelectionState::Multi(ids);
    let mut idx = 0;
    let report = apply_to_targets(&mut doc, |_| {
        idx += 1;
        Outcome::Invalid(format!("bad-{}", idx))
    });
    assert!(!report.any_applied);
    assert!(report.all_failed);
    // Both invalid messages survive; order matches call order.
    assert_eq!(report.messages.len(), 2);
    assert!(report.messages.iter().any(|m| m.contains("bad-1")));
    assert!(report.messages.iter().any(|m| m.contains("bad-2")));
}

#[test]
fn test_apply_to_targets_mixed_applied_and_unchanged_succeeds_silently() {
    let mut doc = fresh_doc();
    let ids = first_n_node_ids(&doc, 3);
    doc.selection = SelectionState::Multi(ids);
    let mut tick = 0;
    let report = apply_to_targets(&mut doc, |_| {
        tick += 1;
        if tick == 1 {
            Outcome::Applied
        } else {
            Outcome::Unchanged
        }
    });
    // At least one Applied → success, no "already set" message
    // (that's reserved for "every target reported Unchanged").
    assert!(report.any_applied);
    assert!(!report.all_failed);
    assert!(report.messages.is_empty());
}

#[test]
fn test_apply_to_targets_all_unchanged_reports_already_set() {
    let mut doc = fresh_doc();
    let ids = first_n_node_ids(&doc, 2);
    doc.selection = SelectionState::Multi(ids);
    let report = apply_to_targets(&mut doc, |_| Outcome::Unchanged);
    // No Applied means `any_applied = false`; "already set" surfaces as a
    // success-with-message — `all_failed` stays false because at
    // least one pair (the unchanged path) succeeded.
    assert!(!report.any_applied);
    assert!(!report.all_failed);
    assert_eq!(report.messages.len(), 1);
    assert!(report.messages[0].contains("already set"));
}
