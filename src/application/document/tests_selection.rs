// SPDX-License-Identifier: MPL-2.0

//! `SelectionState` accessor + collapser coverage. Each of the
//! four edge-adjacent variants (`Edge`, `EdgeLabel`, `PortalLabel`,
//! `PortalText`) has a narrow accessor and a collapser
//! (`selected_edge_or_portal_edge`, `selected_portal_endpoint`)
//! that widens across the mutually-exclusive group. This file
//! pins the narrow-vs-wide semantics so a future refactor can't
//! quietly make one variant report through another's accessor.

use super::types::{EdgeLabelSel, EdgeRef, PortalLabelSel, SectionSel, SelectionState};
use baumhard::mindmap::scene_cache::EdgeKey;

fn edge_ref() -> EdgeRef {
    EdgeRef::new("a", "b", "cross_link")
}

fn portal_sel() -> PortalLabelSel {
    PortalLabelSel {
        edge_key: EdgeKey::new("a", "b", "cross_link"),
        endpoint_node_id: "a".to_string(),
    }
}

#[test]
fn selected_edge_narrow_accessor_rejects_sub_part_variants() {
    // `selected_edge` returns Some only for the whole-edge
    // selection. The three sub-part variants share the owning
    // edge but are distinct selection states.
    assert!(SelectionState::Edge(edge_ref()).selected_edge().is_some());
    assert!(SelectionState::EdgeLabel(EdgeLabelSel::new(edge_ref()))
        .selected_edge()
        .is_none());
    assert!(SelectionState::PortalLabel(portal_sel())
        .selected_edge()
        .is_none());
    assert!(SelectionState::PortalText(portal_sel()).selected_edge().is_none());
}

#[test]
fn selected_edge_label_only_matches_edge_label_variant() {
    assert!(SelectionState::EdgeLabel(EdgeLabelSel::new(edge_ref()))
        .selected_edge_label()
        .is_some());
    assert!(SelectionState::Edge(edge_ref()).selected_edge_label().is_none());
    assert!(SelectionState::PortalLabel(portal_sel())
        .selected_edge_label()
        .is_none());
    assert!(SelectionState::PortalText(portal_sel())
        .selected_edge_label()
        .is_none());
}

#[test]
fn selected_portal_label_and_text_are_narrowly_scoped() {
    // Despite `PortalLabel` and `PortalText` sharing the
    // `PortalLabelSel` inner type, each accessor matches only
    // its own variant. Crossing the two would defeat the
    // purpose of having separate variants for icon vs text.
    assert!(SelectionState::PortalLabel(portal_sel())
        .selected_portal_label()
        .is_some());
    assert!(SelectionState::PortalLabel(portal_sel())
        .selected_portal_text()
        .is_none());
    assert!(SelectionState::PortalText(portal_sel())
        .selected_portal_text()
        .is_some());
    assert!(SelectionState::PortalText(portal_sel())
        .selected_portal_label()
        .is_none());
}

#[test]
fn selected_edge_or_portal_edge_collapses_all_four_variants() {
    // Every edge-adjacent variant reports its owning edge ref
    // through the collapser. Non-edge variants (`None`,
    // `Single`, `Multi`) report `None`.
    let er = edge_ref();
    let cases = [
        SelectionState::Edge(er.clone()),
        SelectionState::EdgeLabel(EdgeLabelSel::new(er.clone())),
        SelectionState::PortalLabel(portal_sel()),
        SelectionState::PortalText(portal_sel()),
    ];
    for sel in cases {
        assert_eq!(
            sel.selected_edge_or_portal_edge(),
            Some(er.clone()),
            "every edge-adjacent variant collapses to the owning edge"
        );
    }
    assert!(SelectionState::None.selected_edge_or_portal_edge().is_none());
    assert!(SelectionState::Single("n".into())
        .selected_edge_or_portal_edge()
        .is_none());
    assert!(SelectionState::Multi(vec!["a".into(), "b".into()])
        .selected_edge_or_portal_edge()
        .is_none());
}

#[test]
fn selected_portal_endpoint_covers_icon_and_text_only() {
    // Portal-scope collapser widens PortalLabel + PortalText
    // (the two portal sub-selections) into the shared
    // `PortalLabelSel`. Non-portal variants — including the
    // other edge-adjacent `Edge` / `EdgeLabel` — report None.
    assert!(SelectionState::PortalLabel(portal_sel())
        .selected_portal_endpoint()
        .is_some());
    assert!(SelectionState::PortalText(portal_sel())
        .selected_portal_endpoint()
        .is_some());
    assert!(SelectionState::Edge(edge_ref())
        .selected_portal_endpoint()
        .is_none());
    assert!(SelectionState::EdgeLabel(EdgeLabelSel::new(edge_ref()))
        .selected_portal_endpoint()
        .is_none());
    assert!(SelectionState::None.selected_portal_endpoint().is_none());
}

#[test]
fn selected_portal_label_scene_ref_covers_icon_and_text() {
    // Both portal sub-variants produce a scene ref so the
    // highlight cascade treats them as one endpoint target
    // on selection. Non-portal selections produce None.
    assert!(SelectionState::PortalLabel(portal_sel())
        .selected_portal_label_scene_ref()
        .is_some());
    assert!(SelectionState::PortalText(portal_sel())
        .selected_portal_label_scene_ref()
        .is_some());
    assert!(SelectionState::Edge(edge_ref())
        .selected_portal_label_scene_ref()
        .is_none());
    assert!(SelectionState::EdgeLabel(EdgeLabelSel::new(edge_ref()))
        .selected_portal_label_scene_ref()
        .is_none());
}

#[test]
fn is_selected_and_selected_ids_ignore_all_edge_adjacent_variants() {
    // Node-scope accessors (`is_selected`, `selected_ids`) only
    // consider node selections; the four edge-adjacent variants
    // all report "nothing".
    let edge_adjacent = [
        SelectionState::Edge(edge_ref()),
        SelectionState::EdgeLabel(EdgeLabelSel::new(edge_ref())),
        SelectionState::PortalLabel(portal_sel()),
        SelectionState::PortalText(portal_sel()),
    ];
    for sel in edge_adjacent {
        assert!(!sel.is_selected("a"));
        assert!(sel.selected_ids().is_empty());
    }
}

// ── Node-scope accessors + constructors ──────────────────────────

#[test]
fn test_selection_state_is_selected() {
    let none = SelectionState::None;
    assert!(!none.is_selected("123"));

    let single = SelectionState::Single("123".to_string());
    assert!(single.is_selected("123"));
    assert!(!single.is_selected("456"));

    let multi = SelectionState::Multi(vec!["123".to_string(), "456".to_string()]);
    assert!(multi.is_selected("123"));
    assert!(multi.is_selected("456"));
    assert!(!multi.is_selected("789"));

    // Section selection counts as "this owning node is selected" —
    // every per-node consumer (highlight, drag, chrome) gets the
    // natural answer.
    let section = SelectionState::Section(SectionSel::new("123", 1));
    assert!(section.is_selected("123"));
    assert!(!section.is_selected("456"));
    assert_eq!(section.selected_ids(), vec!["123"]);
    let s = section
        .selected_section()
        .expect("Section variant carries SectionSel");
    assert_eq!(s.node_id, "123");
    assert_eq!(s.section_idx, 1);
    // Other selection variants return `None` from
    // `selected_section()`.
    assert!(none.selected_section().is_none());
    assert!(single.selected_section().is_none());
}

#[test]
fn test_selection_state_from_ids_empty_is_none() {
    assert!(matches!(SelectionState::from_ids(vec![]), SelectionState::None));
}

#[test]
fn test_selection_state_from_ids_single_element_is_single() {
    match SelectionState::from_ids(vec!["alpha".to_string()]) {
        SelectionState::Single(id) => assert_eq!(id, "alpha"),
        other => panic!("expected Single, got {other:?}"),
    }
}

#[test]
fn test_selection_state_from_ids_two_elements_is_multi_preserving_order() {
    match SelectionState::from_ids(vec!["a".to_string(), "b".to_string()]) {
        SelectionState::Multi(ids) => assert_eq!(ids, vec!["a".to_string(), "b".to_string()]),
        other => panic!("expected Multi, got {other:?}"),
    }
}

#[test]
fn test_selection_state_from_ids_many_elements_is_multi_preserving_order() {
    let input = vec!["a".to_string(), "b".to_string(), "c".to_string(), "d".to_string()];
    match SelectionState::from_ids(input.clone()) {
        SelectionState::Multi(ids) => assert_eq!(ids, input),
        other => panic!("expected Multi, got {other:?}"),
    }
}

// ── SelectionState::MultiSection (N3) ────────────────────────────

#[test]
fn test_selection_state_from_sections_empty_is_none() {
    let sel = SelectionState::from_sections(vec![]);
    assert!(matches!(sel, SelectionState::None));
}

#[test]
fn test_selection_state_from_sections_one_is_section() {
    let sel = SelectionState::from_sections(vec![SectionSel::new("0", 1)]);
    assert!(matches!(sel, SelectionState::Section(_)));
}

#[test]
fn test_selection_state_from_sections_many_is_multisection_preserving_order() {
    let secs = vec![
        SectionSel::new("0", 1),
        SectionSel::new("1", 0),
        SectionSel::new("2", 3),
    ];
    match SelectionState::from_sections(secs.clone()) {
        SelectionState::MultiSection(out) => assert_eq!(out, secs),
        other => panic!("expected MultiSection, got {:?}", other),
    }
}

#[test]
fn test_multisection_is_selected_matches_any_section_node() {
    let sel = SelectionState::MultiSection(vec![SectionSel::new("a", 0), SectionSel::new("b", 2)]);
    assert!(sel.is_selected("a"));
    assert!(sel.is_selected("b"));
    assert!(!sel.is_selected("c"));
}

#[test]
fn test_multisection_selected_ids_dedups_per_node() {
    // Two sections of node "a" + one section of node "b" → unique
    // node-ids = [a, b]. First-seen wins on order.
    let sel = SelectionState::MultiSection(vec![
        SectionSel::new("a", 0),
        SectionSel::new("a", 1),
        SectionSel::new("b", 0),
    ]);
    assert_eq!(sel.selected_ids(), vec!["a", "b"]);
}

#[test]
fn test_multisection_selected_section_returns_none() {
    // `selected_section()` is the single-target accessor — it
    // returns None for MultiSection so verbs that need a single
    // section target route through `selected_sections()` instead.
    let sel = SelectionState::MultiSection(vec![SectionSel::new("a", 0), SectionSel::new("a", 1)]);
    assert!(sel.selected_section().is_none());
}

#[test]
fn test_multisection_selected_sections_returns_all() {
    let secs = vec![SectionSel::new("a", 0), SectionSel::new("b", 1)];
    let sel = SelectionState::MultiSection(secs.clone());
    assert_eq!(sel.selected_sections(), secs.as_slice());
}

#[test]
fn test_section_selected_sections_returns_singleton() {
    let s = SectionSel::new("a", 1);
    let sel = SelectionState::Section(s.clone());
    assert_eq!(sel.selected_sections(), &[s]);
}

#[test]
fn test_other_selections_have_empty_selected_sections() {
    assert_eq!(SelectionState::None.selected_sections(), &[]);
    assert_eq!(SelectionState::Single("a".into()).selected_sections(), &[]);
    assert_eq!(
        SelectionState::Multi(vec!["a".into(), "b".into()]).selected_sections(),
        &[]
    );
    assert_eq!(
        SelectionState::Edge(EdgeRef::new("a", "b", "child")).selected_sections(),
        &[]
    );
}

/// `from_sections` deduplicates by `(node_id, section_idx)`
/// in first-seen order — pins the uniqueness invariant the
/// downstream consumers (selection_targets, font fan-out,
/// highlight pipeline) implicitly assume.
#[test]
fn test_from_sections_dedups_by_node_and_idx() {
    let secs = vec![
        SectionSel::new("a", 0),
        SectionSel::new("a", 0), // duplicate of above
        SectionSel::new("a", 1),
        SectionSel::new("a", 0), // another duplicate of first
        SectionSel::new("b", 0),
    ];
    match SelectionState::from_sections(secs) {
        SelectionState::MultiSection(out) => {
            assert_eq!(out.len(), 3);
            assert_eq!(out[0], SectionSel::new("a", 0));
            assert_eq!(out[1], SectionSel::new("a", 1));
            assert_eq!(out[2], SectionSel::new("b", 0));
        }
        other => panic!("expected MultiSection, got {:?}", other),
    }
}

/// `from_sections` of `[a/0, a/0, a/0]` — all duplicates of one
/// — collapses to `Section(a/0)` after dedup. Pins that the
/// many → one collapse fires correctly when the input has
/// many entries that all dedup down to a single unique entry.
#[test]
fn test_from_sections_all_duplicates_collapses_to_section() {
    let secs = vec![
        SectionSel::new("a", 0),
        SectionSel::new("a", 0),
        SectionSel::new("a", 0),
    ];
    assert!(matches!(
        SelectionState::from_sections(secs),
        SelectionState::Section(_)
    ));
}

/// `dedup_owning_node_ids` deduplicates owning-node ids across
/// every selection variant. Pins the helper that border / zoom
/// / topology Delete all route through.
#[test]
fn test_dedup_owning_node_ids_across_variants() {
    // Multi with duplicates — first-seen wins.
    let multi = SelectionState::Multi(vec!["a".into(), "a".into(), "b".into()]);
    assert_eq!(
        multi.dedup_owning_node_ids(),
        vec!["a".to_string(), "b".to_string()]
    );

    // MultiSection with two sections of one node + one of
    // another — dedup'd to two unique node ids.
    let multi_sec = SelectionState::MultiSection(vec![
        SectionSel::new("a", 0),
        SectionSel::new("a", 1),
        SectionSel::new("b", 0),
    ]);
    assert_eq!(
        multi_sec.dedup_owning_node_ids(),
        vec!["a".to_string(), "b".to_string()]
    );

    // Single → vec of one.
    let single = SelectionState::Single("x".into());
    assert_eq!(single.dedup_owning_node_ids(), vec!["x".to_string()]);

    // None → empty.
    assert!(SelectionState::None.dedup_owning_node_ids().is_empty());
}

/// `dedup_owning_node_ids` order-preserves first-seen across a
/// `MultiSection` set whose section ordering interleaves nodes —
/// pins the order property the section-shift+drag harvest relies
/// on (the snapshot's first entry is the visually-first selected
/// node).
#[test]
fn test_dedup_owning_node_ids_preserves_first_seen_order() {
    let sel = SelectionState::MultiSection(vec![
        SectionSel::new("b", 0),
        SectionSel::new("a", 1),
        SectionSel::new("b", 1), // dup of b — drops
        SectionSel::new("c", 0),
        SectionSel::new("a", 0), // dup of a — drops
    ]);
    assert_eq!(
        sel.dedup_owning_node_ids(),
        vec!["b".to_string(), "a".to_string(), "c".to_string()]
    );
}

// ── SelectionState::SectionRange ────────────────────────────────

#[test]
fn test_section_range_accessors_route_to_owning_section() {
    let target = SectionSel::new("a", 1);
    let sel = SelectionState::SectionRange {
        sel: target.clone(),
        range: (3, 7),
    };
    assert!(sel.is_selected("a"));
    assert!(!sel.is_selected("b"));
    assert_eq!(sel.selected_ids(), vec!["a"]);
    assert_eq!(sel.selected_sections(), &[target]);
    assert_eq!(sel.dedup_owning_node_ids(), vec!["a".to_string()]);
    assert_eq!(sel.selected_range(), Some((3, 7)));
    let s = sel.selected_section().expect("inner SectionSel");
    assert_eq!(s.node_id, "a");
    assert_eq!(s.section_idx, 1);
}

#[test]
fn test_other_variants_selected_range_is_none() {
    assert!(SelectionState::None.selected_range().is_none());
    assert!(SelectionState::Single("a".into()).selected_range().is_none());
    assert!(SelectionState::Section(SectionSel::new("a", 0))
        .selected_range()
        .is_none());
    assert!(
        SelectionState::MultiSection(vec![SectionSel::new("a", 0), SectionSel::new("b", 0)])
            .selected_range()
            .is_none()
    );
}
