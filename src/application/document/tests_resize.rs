// SPDX-License-Identifier: MPL-2.0

//! Tests for the mode-driven resize-handle gate that landed in
//! Batch 2 of `SECTIONS_BORDERS_RESIZE_PLAN.md`.
//!
//! Pre-Batch-2 the scene-builder gate at
//! `MindMapDocument::frame_overrides` read `SelectionState`
//! directly: `Single(node)` → 8 node handles, `Section(s)` →
//! 8 section handles. That auto-emission produced the user-facing
//! "we often find ourselves accidentally resizing when we only want
//! to move nodes around" bug. Batch 2 moved the gate to
//! `InteractionModeOverrides` (populated from `InteractionMode`), so
//! handles emit only when the active mode is `Resize { target }`.
//!
//! These tests pin the contract on both ends:
//! - `Default` mode + any selection → zero handles.
//! - `Resize { Node(id) }` → 8 node handles, no section handles.
//! - `Resize { Section { .. } }` (Some-sized) → 8 section handles, no node handles.
//! - `Resize { Section { .. } }` (None-sized / fill-parent) → zero handles
//!   (filtered by the scene builder regardless of override).

use super::tests_common::{first_testament_node_id, load_test_doc, pinned_two_section_node};
use super::{InteractionModeOverrides, SelectionState};

/// Regression for the auto-anchor-on-selection UX bug fixed in
/// Batch 2: a `Single`-selection in `Default` mode (i.e. callers
/// that pass `InteractionModeOverrides::none()`) emits zero handles.
#[test]
fn test_default_mode_with_single_selection_emits_no_resize_handles() {
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    doc.selection = SelectionState::Single(id);
    let scene = crate::application::document::tests_common::project_roles(&doc, 1.0, InteractionModeOverrides::none());
    assert!(
        scene.node_resize_handles.is_empty(),
        "Default mode + Single selection must NOT emit node resize handles"
    );
    assert!(
        scene.section_resize_handles.is_empty(),
        "Default mode + Single selection must NOT emit section resize handles"
    );
}

/// `Default` mode + a `Section` selection — same regression as
/// above for the section-handle path. Pre-Batch-2 the gate at
/// `selected_section()` would have armed 8 section handles for any
/// `Some`-sized section selection.
#[test]
fn test_default_mode_with_section_selection_emits_no_resize_handles() {
    let (mut doc, id) = pinned_two_section_node();
    doc.selection = SelectionState::Section(super::SectionSel {
        node_id: id,
        section_idx: 1,
    });
    let scene = crate::application::document::tests_common::project_roles(&doc, 1.0, InteractionModeOverrides::none());
    assert!(
        scene.node_resize_handles.is_empty(),
        "Default mode + Section selection must NOT emit node resize handles"
    );
    assert!(
        scene.section_resize_handles.is_empty(),
        "Default mode + Section selection must NOT emit section resize handles"
    );
}

/// `Resize { Node(id) }` mode emits the 8 node handles for the
/// targeted node and zero section handles. Sides cover the full
/// NW/N/NE/E/SE/S/SW/W set (checked by counting unique sides).
#[test]
fn test_resize_mode_node_target_emits_eight_node_handles() {
    let doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    let scene = crate::application::document::tests_common::project_roles(&doc, 
        1.0,
        InteractionModeOverrides {
            node: Some(id.as_str()),
            section: None,
            node_edit_for: None,
            focused_section: None,
        },
    );
    assert_eq!(
        scene.node_resize_handles.len(),
        8,
        "Resize mode with Node target must emit exactly 8 handles"
    );
    assert!(
        scene.section_resize_handles.is_empty(),
        "Node-target Resize mode must not emit section handles"
    );
    // Pin that all 8 sides are distinct — guards against a future
    // refactor that emits e.g. 4 corners + 4 corners with bad math.
    let mut sides: Vec<_> = scene.node_resize_handles.iter().map(|h| h.side).collect();
    sides.sort_by_key(|s| format!("{:?}", s));
    sides.dedup_by_key(|s| format!("{:?}", s));
    assert_eq!(sides.len(), 8, "All 8 ResizeHandleSide variants must appear");
}

/// `Resize { Section { .. } }` mode targeting a `Some`-sized
/// section emits the 8 section handles for that section and zero
/// node handles. Symmetric to the node test above — closes the
/// asymmetric coverage flagged by the test review.
#[test]
fn test_resize_mode_section_target_emits_eight_section_handles() {
    let (doc, id) = pinned_two_section_node();
    let scene = crate::application::document::tests_common::project_roles(&doc, 
        1.0,
        InteractionModeOverrides {
            node: None,
            section: Some((id.as_str(), 1)),
            node_edit_for: None,
            focused_section: None,
        },
    );
    assert_eq!(
        scene.section_resize_handles.len(),
        8,
        "Resize mode with Section target must emit exactly 8 section handles"
    );
    assert!(
        scene.node_resize_handles.is_empty(),
        "Section-target Resize mode must not emit node handles"
    );
    // Each handle carries the correct (node_id, section_idx) pair.
    for h in &scene.section_resize_handles {
        assert_eq!(h.node_id, id);
        assert_eq!(h.section_idx, 1);
    }
}

/// Fill-parent sections (`size == None`) emit zero handles even
/// when the override targets them — the scene builder filters them
/// internally because there's no own AABB to stretch. Pins this
/// invariant so the override surface can't accidentally light up
/// handles on a section that has nothing to resize.
#[test]
fn test_resize_mode_section_target_fill_parent_emits_no_handles() {
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    // Force section 0 to fill-parent.
    if let Some(node) = doc.mindmap.nodes.get_mut(&id) {
        if let Some(s) = node.sections.first_mut() {
            s.size = None;
        }
    }
    let scene = crate::application::document::tests_common::project_roles(&doc, 
        1.0,
        InteractionModeOverrides {
            node: None,
            section: Some((id.as_str(), 0)),
            node_edit_for: None,
            focused_section: None,
        },
    );
    assert!(
        scene.section_resize_handles.is_empty(),
        "Fill-parent (None-sized) section must emit zero handles"
    );
}
