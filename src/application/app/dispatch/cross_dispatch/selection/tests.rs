// SPDX-License-Identifier: MPL-2.0

//! Pure-doc-mutation tests for the selection bucket's inner
//! `select_*_in` helpers — `select_all_in` / `deselect_all_in` /
//! `invert_selection_in` / `select_parent_in` / `select_child_in` /
//! `select_sibling_in`. The renderer-touching `apply_*` wrappers
//! are out of scope per `TEST_CONVENTIONS.md §T8` (no live wgpu
//! in unit tests); the tests below cover the cross-platform inner
//! functions that carry the actual logic. A regression in any of
//! these silently changes WASM and native behaviour identically —
//! the type-checker won't catch it.

#![cfg(test)]

use super::*;
use crate::application::document::tests_common::{load_test_doc, pinned_two_section_node};
use crate::application::document::{GraphemeRange, SectionSel, SectionSpan};

fn first_node_id(doc: &MindMapDocument) -> String {
    doc.mindmap
        .nodes
        .keys()
        .next()
        .expect("test fixture has nodes")
        .clone()
}

fn first_root_id(doc: &MindMapDocument) -> String {
    doc.mindmap
        .root_nodes()
        .first()
        .expect("test fixture has at least one root")
        .id
        .clone()
}

#[test]
fn select_all_in_picks_every_visible_node() {
    let mut doc = load_test_doc();
    let visible_count = doc
        .mindmap
        .nodes
        .values()
        .filter(|n| !doc.mindmap.is_hidden_by_fold(n))
        .count();
    assert!(visible_count > 0, "fixture has visible nodes");
    let changed = select_all_in(&mut doc);
    assert!(changed);
    let selected = doc.selection.selected_ids();
    assert_eq!(selected.len(), visible_count);
}

#[test]
fn select_all_in_excludes_folded_descendants() {
    let mut doc = load_test_doc();
    // Pick a non-leaf root and fold it.
    let root_id = first_root_id(&doc);
    let descendant_count = doc.mindmap.all_descendants(&root_id).len();
    assert!(descendant_count > 0, "test fixture root must have descendants",);
    doc.mindmap.nodes.get_mut(&root_id).unwrap().folded = true;
    let total_visible_before_fold = doc.mindmap.nodes.len();
    let _ = select_all_in(&mut doc);
    let selected = doc.selection.selected_ids();
    // The folded root itself is still visible; only its
    // descendants are hidden.
    assert!(selected.iter().any(|id| *id == root_id));
    assert_eq!(selected.len(), total_visible_before_fold - descendant_count);
}

#[test]
fn select_all_in_returns_false_when_no_visible_nodes() {
    let mut doc = MindMapDocument::new_blank(None);
    // Empty document → no visible nodes → no-op + false.
    assert!(!select_all_in(&mut doc));
    assert!(matches!(doc.selection, SelectionState::None));
}

#[test]
fn deselect_all_in_clears_selection() {
    let mut doc = load_test_doc();
    doc.selection = SelectionState::Single(first_node_id(&doc));
    assert!(deselect_all_in(&mut doc));
    assert!(matches!(doc.selection, SelectionState::None));
}

#[test]
fn deselect_all_in_returns_false_when_already_none() {
    let mut doc = load_test_doc();
    // Default selection is None.
    assert!(!deselect_all_in(&mut doc));
}

#[test]
fn invert_selection_in_skips_edge_selection() {
    let mut doc = load_test_doc();
    let edge = doc.mindmap.edges.first().expect("fixture has edges").clone();
    let er = crate::application::document::EdgeRef::new(&edge.from_id, &edge.to_id, &edge.edge_type);
    doc.selection = SelectionState::Edge(er.clone());
    // Edge selections are NOT invertable — the helper preserves
    // them (selecting "every visible node" via inversion would
    // be unintuitive).
    assert!(!invert_selection_in(&mut doc));
    assert!(matches!(doc.selection, SelectionState::Edge(_)));
}

#[test]
fn invert_selection_in_inverts_node_selection() {
    let mut doc = load_test_doc();
    let pivot = first_node_id(&doc);
    doc.selection = SelectionState::Single(pivot.clone());
    assert!(invert_selection_in(&mut doc));
    // Pivot is no longer in the selection.
    assert!(!doc.selection.selected_ids().iter().any(|id| **id == pivot));
    // Every other visible node IS in the selection.
    let expected = doc
        .mindmap
        .nodes
        .values()
        .filter(|n| n.id != pivot && !doc.mindmap.is_hidden_by_fold(n))
        .count();
    assert_eq!(doc.selection.selected_ids().len(), expected);
}

#[test]
fn select_parent_in_walks_up_one_level() {
    let mut doc = load_test_doc();
    // Pick a non-root node to start.
    let child_id = doc
        .mindmap
        .nodes
        .values()
        .find(|n| n.parent_id.is_some())
        .expect("fixture has a non-root node")
        .id
        .clone();
    let parent_id = doc.mindmap.nodes[&child_id].parent_id.clone().unwrap();
    doc.selection = SelectionState::Single(child_id);
    assert!(select_parent_in(&mut doc));
    assert!(matches!(
        doc.selection,
        SelectionState::Single(ref s) if s == &parent_id
    ));
}

#[test]
fn select_parent_in_no_op_at_root() {
    let mut doc = load_test_doc();
    let root_id = first_root_id(&doc);
    doc.selection = SelectionState::Single(root_id.clone());
    // Roots have no parent — no-op + false.
    assert!(!select_parent_in(&mut doc));
    assert!(matches!(
        doc.selection,
        SelectionState::Single(ref s) if s == &root_id
    ));
}

/// `select_parent_in` walks up from a `SectionRange` selection
/// to the section's owning node's parent — same shape as the
/// `Section` arm. Pins the missed-arm fix from the N4-C.a
/// review (N4-C.a's wildcard `_ => return false` would have
/// no-op'd here).
#[test]
fn select_parent_in_walks_up_from_section_range() {
    use crate::application::document::SectionSel;
    let mut doc = load_test_doc();
    let child_id = doc
        .mindmap
        .nodes
        .values()
        .find(|n| n.parent_id.is_some())
        .expect("fixture has a non-root node")
        .id
        .clone();
    let parent_id = doc.mindmap.nodes[&child_id].parent_id.clone().unwrap();
    doc.selection = SelectionState::SectionRange {
        sel: SectionSel::new(&child_id, 0),
        section_span: SectionSpan::single(0),
        grapheme_range: GraphemeRange::new(1, 3),
    };
    assert!(select_parent_in(&mut doc));
    assert!(matches!(
        doc.selection,
        SelectionState::Single(ref s) if s == &parent_id
    ));
}

#[test]
fn select_parent_in_no_op_for_multi_selection() {
    let mut doc = load_test_doc();
    let ids: Vec<String> = doc.mindmap.nodes.keys().take(2).cloned().collect();
    doc.selection = SelectionState::from_ids(ids);
    assert!(!select_parent_in(&mut doc));
}

#[test]
fn select_child_in_steps_into_first_visible_child() {
    let mut doc = load_test_doc();
    let parent_id = doc
        .mindmap
        .nodes
        .values()
        .find(|n| !doc.mindmap.children_of(&n.id).is_empty())
        .expect("fixture has a parent node")
        .id
        .clone();
    let expected_child = doc
        .mindmap
        .children_of(&parent_id)
        .into_iter()
        .find(|c| !doc.mindmap.is_hidden_by_fold(c))
        .expect("at least one visible child")
        .id
        .clone();
    doc.selection = SelectionState::Single(parent_id);
    assert!(select_child_in(&mut doc));
    assert!(matches!(
        doc.selection,
        SelectionState::Single(ref s) if s == &expected_child
    ));
}

#[test]
fn select_child_in_no_op_for_leaf() {
    let mut doc = load_test_doc();
    // Find a node with no children.
    let leaf_id = doc
        .mindmap
        .nodes
        .values()
        .find(|n| doc.mindmap.children_of(&n.id).is_empty())
        .expect("fixture has a leaf")
        .id
        .clone();
    doc.selection = SelectionState::Single(leaf_id);
    assert!(!select_child_in(&mut doc));
}

/// Section selection collapses to the owning node first, then
/// walks. `select_parent_in` from a `Section` whose owning node
/// has a parent must produce `Single(parent_id)`.
#[test]
fn select_parent_in_handles_section_selection() {
    use crate::application::document::SectionSel;
    let mut doc = load_test_doc();
    let child_id = doc
        .mindmap
        .nodes
        .values()
        .find(|n| n.parent_id.is_some())
        .expect("fixture has a non-root node")
        .id
        .clone();
    let parent_id = doc.mindmap.nodes[&child_id].parent_id.clone().unwrap();
    doc.selection = SelectionState::Section(SectionSel::new(child_id, 0));
    assert!(select_parent_in(&mut doc));
    assert!(matches!(
        doc.selection,
        SelectionState::Single(ref s) if s == &parent_id
    ));
}

/// `select_child_in` on a `Section` must step into the section's
/// owning node's first visible child.
#[test]
fn select_child_in_handles_section_selection() {
    use crate::application::document::SectionSel;
    let mut doc = load_test_doc();
    let parent_id = doc
        .mindmap
        .nodes
        .values()
        .find(|n| !doc.mindmap.children_of(&n.id).is_empty())
        .expect("fixture has a parent node")
        .id
        .clone();
    let expected_child = doc
        .mindmap
        .children_of(&parent_id)
        .into_iter()
        .find(|c| !doc.mindmap.is_hidden_by_fold(c))
        .expect("at least one visible child")
        .id
        .clone();
    doc.selection = SelectionState::Section(SectionSel::new(parent_id, 0));
    assert!(select_child_in(&mut doc));
    assert!(matches!(
        doc.selection,
        SelectionState::Single(ref s) if s == &expected_child
    ));
}

/// `select_sibling_in` on a `Section` walks between sections of
/// the same node first; once sections are exhausted, falls
/// through to the next mind-node sibling. Single-section nodes
/// fall through immediately. The fall-through case keeps the
/// pre-tier-D behaviour for migration-default nodes.
#[test]
fn select_sibling_in_handles_section_selection() {
    use crate::application::document::SectionSel;
    let mut doc = load_test_doc();
    let (start_id, expected_next) = doc
        .mindmap
        .nodes
        .values()
        .filter_map(|n| {
            let parent = n.parent_id.as_ref()?;
            let siblings = doc.mindmap.children_of(parent);
            if siblings.len() < 2 {
                return None;
            }
            let idx = siblings.iter().position(|s| s.id == n.id)?;
            let next = siblings.get(idx + 1)?.id.clone();
            Some((n.id.clone(), next))
        })
        .next()
        .expect("fixture has at least one node with a next sibling");
    doc.selection = SelectionState::Section(SectionSel::new(start_id, 0));
    assert!(select_sibling_in(&mut doc, true));
    assert!(matches!(
        doc.selection,
        SelectionState::Single(ref s) if s == &expected_next
    ));
}

/// Forward sibling walk inside a multi-section node steps to the
/// next *section* before falling through to the next mind-node
/// sibling. Pins the keyboard reach into every section a multi-
/// section author authored — pre-fix the only way to select
/// `Section(N, 1)` from `Section(N, 0)` was via click.
#[test]
fn select_sibling_in_walks_between_sections_forward() {
    use crate::application::document::SectionSel;
    use baumhard::mindmap::model::MindSection;
    let mut doc = load_test_doc();
    let nid = first_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections
            .push(MindSection::new_default("second".into(), Vec::new()));
        node.sections
            .push(MindSection::new_default("third".into(), Vec::new()));
    }
    doc.selection = SelectionState::Section(SectionSel::new(nid.clone(), 0));
    assert!(select_sibling_in(&mut doc, true));
    assert!(matches!(
        doc.selection,
        SelectionState::Section(ref s) if s.node_id == nid && s.section_idx == 1
    ));
    assert!(select_sibling_in(&mut doc, true));
    assert!(matches!(
        doc.selection,
        SelectionState::Section(ref s) if s.node_id == nid && s.section_idx == 2
    ));
}

/// `select_sibling_in` from a `SectionRange` selection walks
/// between sections like `Section` does — and **drops the
/// sub-range** on transition (the result is `Section`, not a
/// new `SectionRange`). Pins the documented "moving to a
/// sibling section is a fresh selection, not a range
/// carry-over" semantic.
#[test]
fn select_sibling_in_drops_section_range_on_walk() {
    use crate::application::document::SectionSel;
    use baumhard::mindmap::model::MindSection;
    let mut doc = load_test_doc();
    let nid = first_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections
            .push(MindSection::new_default("second".into(), Vec::new()));
    }
    doc.selection = SelectionState::SectionRange {
        sel: SectionSel::new(nid.clone(), 0),
        section_span: SectionSpan::single(0),
        grapheme_range: GraphemeRange::new(1, 3),
    };
    assert!(select_sibling_in(&mut doc, true));
    assert!(
        matches!(
            doc.selection,
            SelectionState::Section(ref s) if s.node_id == nid && s.section_idx == 1
        ),
        "sibling walk must demote SectionRange to Section, dropping the sub-range"
    );
}

/// Backward sibling walk on `Section(N, K>0)` steps to
/// `Section(N, K-1)`; on `Section(N, 0)` falls through to the
/// previous mind-node sibling (or no-op at root).
#[test]
fn select_sibling_in_walks_between_sections_backward() {
    use crate::application::document::SectionSel;
    use baumhard::mindmap::model::MindSection;
    let mut doc = load_test_doc();
    let nid = first_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&nid).unwrap();
        node.sections
            .push(MindSection::new_default("second".into(), Vec::new()));
    }
    doc.selection = SelectionState::Section(SectionSel::new(nid.clone(), 1));
    assert!(select_sibling_in(&mut doc, false));
    assert!(matches!(
        doc.selection,
        SelectionState::Section(ref s) if s.node_id == nid && s.section_idx == 0
    ));
}

/// `invert_selection_in` treats `Section(s)` like `Single(s.node_id)`:
/// the owning node drops out of the inversion, every other visible
/// node is selected.
#[test]
fn invert_selection_in_handles_section_selection() {
    use crate::application::document::SectionSel;
    let mut doc = load_test_doc();
    let pivot = first_node_id(&doc);
    doc.selection = SelectionState::Section(SectionSel::new(pivot.clone(), 0));
    assert!(invert_selection_in(&mut doc));
    assert!(!doc.selection.selected_ids().iter().any(|id| **id == pivot));
    let expected = doc
        .mindmap
        .nodes
        .values()
        .filter(|n| n.id != pivot && !doc.mindmap.is_hidden_by_fold(n))
        .count();
    assert_eq!(doc.selection.selected_ids().len(), expected);
}

#[test]
fn select_sibling_in_walks_visible_neighbour() {
    let mut doc = load_test_doc();
    // Find a node with at least one sibling.
    let (start_id, _next_id) = doc
        .mindmap
        .nodes
        .values()
        .filter_map(|n| {
            let parent = n.parent_id.as_ref()?;
            let siblings = doc.mindmap.children_of(parent);
            if siblings.len() < 2 {
                return None;
            }
            let idx = siblings.iter().position(|s| s.id == n.id)?;
            let next = siblings.get(idx + 1)?.id.clone();
            Some((n.id.clone(), next))
        })
        .next()
        .expect("fixture has at least one node with a next sibling");
    doc.selection = SelectionState::Single(start_id);
    assert!(select_sibling_in(&mut doc, true));
    // Walking back returns to the previous sibling.
    assert!(select_sibling_in(&mut doc, false));
}

// ── MultiSection collapse-to-first nav ───────────────────────────

/// Up-arrow on a MultiSection collapses to the first section's
/// owning node and walks to its parent (or no-ops at root).
#[test]
fn select_parent_in_collapses_multisection_to_first_owning_node() {
    let (mut doc, id) = pinned_two_section_node();
    // Two-section node has no parent in the fixture; pin that
    // up-arrow on MultiSection no-ops cleanly (rather than
    // silently no-op'ing for the wrong reason).
    doc.selection = SelectionState::MultiSection(vec![SectionSel::new(&id, 0), SectionSel::new(&id, 1)]);
    assert!(!select_parent_in(&mut doc));
}

/// Down-arrow on a MultiSection produces the same post-state as
/// down-arrow on `Single(first_section.node_id)` — pinning the
/// "collapse to first owning node" contract. Uses load_test_doc
/// (which has the testament hierarchy with parent/child relations)
/// and selects a node with at least one visible child so the
/// collapse + walk path is actually exercised.
#[test]
fn select_child_in_multisection_matches_single_on_first_owning_node() {
    let mut doc_a = load_test_doc();
    let mut doc_b = load_test_doc();
    let parent_with_child = doc_a
        .mindmap
        .nodes
        .values()
        .filter_map(|n| {
            doc_a
                .mindmap
                .children_of(&n.id)
                .iter()
                .find(|c| !doc_a.mindmap.is_hidden_by_fold(c))
                .map(|c| (n.id.clone(), c.id.clone()))
        })
        .min()
        .expect("fixture has at least one node with a visible child");
    let parent_id = parent_with_child.0;
    let expected_child = parent_with_child.1;
    doc_a.selection = SelectionState::MultiSection(vec![
        SectionSel::new(&parent_id, 0),
        SectionSel::new(&parent_id, 1),
    ]);
    doc_b.selection = SelectionState::Single(parent_id);
    assert!(select_child_in(&mut doc_a));
    assert!(select_child_in(&mut doc_b));
    let landed_a = match &doc_a.selection {
        SelectionState::Single(id) => id.clone(),
        other => panic!("expected Single, got {:?}", other),
    };
    let landed_b = match &doc_b.selection {
        SelectionState::Single(id) => id.clone(),
        other => panic!("expected Single, got {:?}", other),
    };
    assert_eq!(landed_a, expected_child);
    assert_eq!(landed_b, expected_child);
}

/// Sibling navigation on a MultiSection collapses to the first
/// section's owning node and walks the node's siblings.
/// Deterministic by `min()` so the test doesn't pick whichever
/// HashMap iteration surfaced first.
#[test]
fn select_sibling_in_collapses_multisection_to_first_owning_node() {
    let mut doc = load_test_doc();
    let (start_id, expected_next) = doc
        .mindmap
        .nodes
        .values()
        .filter_map(|n| {
            let parent_id = n.parent_id.clone()?;
            let siblings = doc.mindmap.children_of(&parent_id);
            if siblings.len() < 2 {
                return None;
            }
            let idx = siblings.iter().position(|s| s.id == n.id)?;
            let next = siblings.get(idx + 1)?.id.clone();
            Some((n.id.clone(), next))
        })
        .min()
        .expect("fixture has at least one node with a next sibling");
    doc.selection =
        SelectionState::MultiSection(vec![SectionSel::new(&start_id, 0), SectionSel::new(&start_id, 1)]);
    assert!(select_sibling_in(&mut doc, true));
    let landed = match &doc.selection {
        SelectionState::Single(id) => id.clone(),
        other => panic!("expected Single, got {:?}", other),
    };
    assert_eq!(landed, expected_next);
}
