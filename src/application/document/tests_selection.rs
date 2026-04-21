//! `SelectionState` accessor + collapser coverage. Each of the
//! four edge-adjacent variants (`Edge`, `EdgeLabel`, `PortalLabel`,
//! `PortalText`) has a narrow accessor and a collapser
//! (`selected_edge_or_portal_edge`, `selected_portal_endpoint`)
//! that widens across the mutually-exclusive group. This file
//! pins the narrow-vs-wide semantics so a future refactor can't
//! quietly make one variant report through another's accessor.

use super::types::{EdgeLabelSel, EdgeRef, PortalLabelSel, SelectionState};
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
    assert!(SelectionState::PortalText(portal_sel())
        .selected_edge()
        .is_none());
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
    assert!(SelectionState::Edge(edge_ref()).selected_portal_endpoint().is_none());
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
