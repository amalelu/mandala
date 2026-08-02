// SPDX-License-Identifier: MPL-2.0

//! Click-target handlers for the native event loop: default click,
//! reparent-target click, connect-target click, plus the
//! mode-aware variant of `rebuild_all`. WASM is gated out at the
//! parent module's `#[cfg]`.

#![cfg(not(target_arch = "wasm32"))]

use baumhard::mindmap::custom_mutation::PlatformContext;
use baumhard::mindmap::tree_builder::PortalPart;

use super::click_triggers::fire_onclick_triggers;
use super::scene_rebuild::{build_overlaid_tree, rebuild_all, rebuild_scene_only};
use super::{now_ms, InteractionMode, EDGE_HIT_TOLERANCE_PX};
use crate::application::document::{
    hit_test_edge, MindMapDocument, SectionSel, SelectionState, REPARENT_SOURCE_COLOR,
    REPARENT_TARGET_COLOR,
};
use crate::application::renderer::Renderer;

/// Handle a click event: update selection, rebuild tree with highlight.
/// When the node hit test misses, falls through to edge hit testing so
/// the user can click on a connection path to select it. If the clicked
/// node has an `OnClick` trigger binding, the bound custom mutation fires
/// (both node mutations and any document actions) after the selection
/// update.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn handle_click(
    hit: Option<String>,
    hit_section: Option<usize>,
    cursor_pos: (f64, f64),
    shift_pressed: bool,
    document: &mut Option<MindMapDocument>,
    interaction_mode: &InteractionMode,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    let doc = match document.as_mut() {
        Some(d) => d,
        None => return,
    };

    // OnClick triggers fire before the selection update so that
    // document actions (theme switches etc.) take effect before
    // the scene rebuild below picks up the new state.
    if let Some(id) = hit.as_ref() {
        fire_onclick_triggers(
            doc, mindmap_tree, scene_cache, id, hit_section,
            PlatformContext::Desktop, now_ms() as u64,
        );
    }

    // Update selection state
    match (&hit, shift_pressed) {
        (Some(id), shift) => {
            doc.selection = compute_node_click_selection(
                &doc.selection,
                id,
                hit_section,
                shift,
                interaction_mode,
            );
        }
        (None, false) => {
            // Node miss — fall through: first try portal markers
            // (label glyphs attached to their endpoint nodes),
            // then edge hit testing, then finally deselect. A
            // portal-marker click selects the specific label
            // via `SelectionState::PortalLabel { .. }` so wheel
            // / copy / paste / cut / drag all operate on just
            // that endpoint's state; double-click is handled
            // separately by the event loop and pans the camera
            // to the opposite endpoint.
            let canvas_pos = renderer.screen_to_canvas(cursor_pos.0 as f32, cursor_pos.1 as f32);
            // One BVH descent over the portal tree names both the
            // endpoint and which of its two sibling leaves — icon
            // or text — the click landed on, so the sub-part
            // precedence is decided by geometry (smallest area
            // wins) rather than by the order two side maps happen
            // to be scanned in.
            if let Some(hit) = app_scene.portal_at(canvas_pos) {
                let sel = crate::application::document::PortalLabelSel {
                    edge_key: hit.edge_key,
                    endpoint_node_id: hit.endpoint_node_id,
                };
                doc.selection = match hit.part {
                    PortalPart::Text => SelectionState::PortalText(sel),
                    PortalPart::Icon => SelectionState::PortalLabel(sel),
                };
            } else {
                let tolerance = EDGE_HIT_TOLERANCE_PX * renderer.canvas_per_pixel();
                let edge_hit = hit_test_edge(canvas_pos, &doc.mindmap, tolerance);
                doc.selection = match edge_hit {
                    Some(edge_ref) => SelectionState::Edge(edge_ref),
                    None => SelectionState::None,
                };
            }
        }
        (None, true) => {
            // Shift+click on empty space: keep current selection (no edge
            // hit test — shift is reserved for multi-node).
        }
    }

    // Rebuild tree with selection highlight applied
    rebuild_all(doc, interaction_mode, mindmap_tree, app_scene, renderer, scene_cache);
}

/// Rebuild tree, connections, and borders like `rebuild_all`, but additionally
/// overlays reparent-mode highlights on top of the normal selection highlight.
/// `hovered_node` is the node currently under the cursor (highlighted green as
/// the drop target) when in reparent mode; it is ignored in Normal mode.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn rebuild_all_with_mode(
    doc: &MindMapDocument,
    interaction_mode: &InteractionMode,
    hovered_node: Option<&str>,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    // Build a single flat list of (mind_node_id, color) pairs that
    // `apply_tree_highlights` applies via baumhard's mutator/walker.
    // Order matters: later entries override earlier ones via the
    // repeated `SetRegionColor` mutation, so selection (cyan) is
    // listed first, then mode-specific source (orange), then the
    // hovered target (green). This matches the previous behavior
    // where reparent_source_highlight was documented to override
    // selection_highlight on conflict.
    // Highlight tuples are `(node_id, section_idx?, color)`. A
    // Section / MultiSection narrow the highlight to the
    // selected sections only; mode-driven Reparent / Connect
    // highlights always paint every section (the gesture is
    // whole-node). Routes through the canonical
    // `selection_highlight_entries` helper so the three
    // selection-rebuild sites (here, `rebuild_all`, and the
    // threshold-cross promotion's `rebuild_selection_highlight`)
    // share one mapping.
    let mut highlights = super::scene_rebuild::selection_highlight_entries(&doc.selection);
    match interaction_mode {
        InteractionMode::Reparent { sources } => {
            for s in sources {
                highlights.push((s.as_str(), None, REPARENT_SOURCE_COLOR));
            }
            if let Some(h) = hovered_node {
                if !sources.iter().any(|s| s == h) {
                    highlights.push((h, None, REPARENT_TARGET_COLOR));
                }
            }
        }
        InteractionMode::Connect { source } => {
            highlights.push((source.as_str(), None, REPARENT_SOURCE_COLOR));
            if let Some(h) = hovered_node {
                if h != source {
                    highlights.push((h, None, REPARENT_TARGET_COLOR));
                }
            }
        }
        // Default / NodeEdit / Resize don't contribute selection-
        // tinting highlights. NodeEdit dimming is a separate overlay
        // `build_overlaid_tree` stamps; Resize tinting rides on the
        // handle trees.
        InteractionMode::Default | InteractionMode::NodeEdit { .. } | InteractionMode::Resize { .. } => {}
    }
    let new_tree = build_overlaid_tree(doc, interaction_mode, highlights);
    renderer.rebuild_buffers_from_tree(&new_tree.tree);

    rebuild_scene_only(doc, interaction_mode, app_scene, renderer, scene_cache);
    renderer
        .set_mode_status_text(super::scene_rebuild::mode_status_line(interaction_mode, doc));

    *mindmap_tree = Some(new_tree);
}

/// Pure selection-update helper for "click landed on a node."
///
/// Resolves the new [`SelectionState`] given the previous selection,
/// the click hit (node id + optional section index), the shift modifier,
/// and the current [`InteractionMode`]. Section routing is gated by
/// [`InteractionMode::click_resolves_to_section`]: outside `NodeEdit { id }`
/// (or in NodeEdit on a different node) every click on a multi-section
/// node folds to whole-node `Single` / `Multi`. Single-section nodes
/// always fold via `hit_test_target`'s short-circuit (they never
/// produce `hit_section = Some(_)`), so their click behavior is
/// unchanged from pre-Batch-3.
///
/// Plain click:
/// - `route_to_section` true → `Section { node_id, section_idx }`.
/// - else → `Single(node_id)`.
///
/// Shift+click, section-routed:
/// - `Section(s)` matching the new (node, idx) → `None` (toggle off).
/// - `Section(s)` mismatching → promote to `MultiSection`.
/// - `MultiSection` → toggle the (node, idx) pair in or out, narrowing
///   back to `Section` when one remains.
/// - any non-section starting state → start a fresh `Section`.
///
/// Shift+click, whole-node (route_to_section false):
/// - `Single(existing)` matching → `None` (toggle off).
/// - `Single(existing)` mismatching → `Multi(vec![existing, new])`.
/// - `Multi` → toggle id in or out, narrowing back to `Single`.
/// - any non-node starting state → fresh `Single`.
pub(super) fn compute_node_click_selection(
    existing: &SelectionState,
    hit_id: &str,
    hit_section: Option<usize>,
    shift_pressed: bool,
    interaction_mode: &InteractionMode,
) -> SelectionState {
    let route_to_section =
        hit_section.is_some() && interaction_mode.click_resolves_to_section(hit_id);

    if !shift_pressed {
        return if route_to_section {
            SelectionState::Section(SectionSel {
                node_id: hit_id.to_string(),
                section_idx: hit_section.expect("guarded above"),
            })
        } else {
            SelectionState::Single(hit_id.to_string())
        };
    }

    if route_to_section {
        let new_sec = SectionSel {
            node_id: hit_id.to_string(),
            section_idx: hit_section.expect("guarded above"),
        };
        return match existing {
            SelectionState::Section(prev) if prev == &new_sec => SelectionState::None,
            SelectionState::Section(prev) => {
                SelectionState::MultiSection(vec![prev.clone(), new_sec])
            }
            SelectionState::MultiSection(prev) => {
                let mut secs = prev.clone();
                if let Some(pos) = secs.iter().position(|s| s == &new_sec) {
                    secs.remove(pos);
                    SelectionState::from_sections(secs)
                } else {
                    secs.push(new_sec);
                    SelectionState::MultiSection(secs)
                }
            }
            _ => SelectionState::Section(new_sec),
        };
    }

    // Whole-node shift+click: existing behavior (toggle node in/out of Multi).
    match existing {
        SelectionState::None
        | SelectionState::Edge(_)
        | SelectionState::EdgeLabel(_)
        | SelectionState::PortalLabel(_)
        | SelectionState::PortalText(_)
        | SelectionState::Section(_)
        | SelectionState::MultiSection(_)
        | SelectionState::SectionRange { .. } => SelectionState::Single(hit_id.to_string()),
        SelectionState::Single(prev) => {
            if prev == hit_id {
                SelectionState::None
            } else {
                SelectionState::Multi(vec![prev.clone(), hit_id.to_string()])
            }
        }
        SelectionState::Multi(prev) => {
            let mut ids = prev.clone();
            if let Some(pos) = ids.iter().position(|i| i == hit_id) {
                ids.remove(pos);
                SelectionState::from_ids(ids)
            } else {
                ids.push(hit_id.to_string());
                SelectionState::Multi(ids)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::SectionSel;

    fn node_edit_for(id: &str) -> InteractionMode {
        InteractionMode::NodeEdit { node_id: id.to_string() }
    }

    fn sec(node_id: &str, idx: usize) -> SectionSel {
        SectionSel { node_id: node_id.to_string(), section_idx: idx }
    }

    // Plain click — section routing rules.

    #[test]
    fn test_plain_click_multi_section_in_node_edit_routes_to_section() {
        let result = compute_node_click_selection(
            &SelectionState::None, "n0", Some(2), false, &node_edit_for("n0"),
        );
        match result {
            SelectionState::Section(s) => assert_eq!(s, sec("n0", 2)),
            other => panic!("expected Section(n0,2), got {other:?}"),
        }
    }

    #[test]
    fn test_plain_click_multi_section_in_default_mode_folds_to_single() {
        let result = compute_node_click_selection(
            &SelectionState::None, "n0", Some(2), false, &InteractionMode::Default,
        );
        match result {
            SelectionState::Single(id) => assert_eq!(id, "n0"),
            other => panic!("expected Single(n0), got {other:?}"),
        }
    }

    #[test]
    fn test_plain_click_multi_section_in_node_edit_on_other_node_folds_to_single() {
        let result = compute_node_click_selection(
            &SelectionState::None, "n0", Some(2), false, &node_edit_for("n1"),
        );
        match result {
            SelectionState::Single(id) => assert_eq!(id, "n0"),
            other => panic!("expected Single(n0), got {other:?}"),
        }
    }

    #[test]
    fn test_plain_click_no_section_in_node_edit_returns_single() {
        // hit_section = None → always Single regardless of mode.
        let result = compute_node_click_selection(
            &SelectionState::None, "n0", None, false, &node_edit_for("n0"),
        );
        match result {
            SelectionState::Single(id) => assert_eq!(id, "n0"),
            other => panic!("expected Single(n0), got {other:?}"),
        }
    }

    // Shift+click — section routing rules.

    #[test]
    fn test_shift_click_same_section_in_node_edit_toggles_off() {
        let result = compute_node_click_selection(
            &SelectionState::Section(sec("n0", 1)),
            "n0", Some(1), true, &node_edit_for("n0"),
        );
        assert!(matches!(result, SelectionState::None), "got {result:?}");
    }

    #[test]
    fn test_shift_click_different_section_in_node_edit_promotes_to_multi_section() {
        let result = compute_node_click_selection(
            &SelectionState::Section(sec("n0", 0)),
            "n0", Some(1), true, &node_edit_for("n0"),
        );
        match result {
            SelectionState::MultiSection(secs) => {
                assert_eq!(secs, vec![sec("n0", 0), sec("n0", 1)]);
            }
            other => panic!("expected MultiSection, got {other:?}"),
        }
    }

    #[test]
    fn test_shift_click_section_outside_node_edit_falls_back_to_node_path() {
        // Default mode + hit_section=Some → folds to whole-node shift+click.
        // Starting from None: result is fresh Single.
        let result = compute_node_click_selection(
            &SelectionState::None, "n0", Some(1), true, &InteractionMode::Default,
        );
        match result {
            SelectionState::Single(id) => assert_eq!(id, "n0"),
            other => panic!("expected Single(n0), got {other:?}"),
        }
    }

    #[test]
    fn test_shift_click_multi_section_remove_narrows_to_single_section() {
        let prev = SelectionState::MultiSection(vec![sec("n0", 0), sec("n0", 1)]);
        let result = compute_node_click_selection(
            &prev, "n0", Some(1), true, &node_edit_for("n0"),
        );
        match result {
            SelectionState::Section(s) => assert_eq!(s, sec("n0", 0)),
            other => panic!("expected Section(n0,0), got {other:?}"),
        }
    }

    /// Cross-node MultiSection: starting from a `MultiSection` set
    /// containing sections of node A, shift-clicking a section of
    /// node B (while in `NodeEdit { B }`) extends the set with the
    /// new (node_id, section_idx) pair. The dedup-by-(node_id,
    /// section_idx) identity is the load-bearing invariant the
    /// docstring on `compute_node_click_selection` calls out.
    #[test]
    fn test_shift_click_extends_multi_section_across_distinct_nodes() {
        let prev = SelectionState::MultiSection(vec![sec("a", 0), sec("a", 1)]);
        let result = compute_node_click_selection(
            &prev, "b", Some(0), true, &node_edit_for("b"),
        );
        match result {
            SelectionState::MultiSection(secs) => {
                assert_eq!(secs.len(), 3, "got {secs:?}");
                assert!(secs.contains(&sec("a", 0)));
                assert!(secs.contains(&sec("a", 1)));
                assert!(secs.contains(&sec("b", 0)));
            }
            other => panic!("expected MultiSection of length 3, got {other:?}"),
        }
    }

    /// `SectionRange` as the *starting* state: shift+click on a
    /// node folds to fresh `Single` (the node-path takes the
    /// non-section branch in the match arm). Pins the explicit
    /// `SectionRange` arm in `compute_node_click_selection`.
    #[test]
    fn test_shift_click_node_from_section_range_collapses_to_single() {
        let prev = SelectionState::SectionRange {
            sel: sec("n0", 0),
            range: (1, 3),
        };
        let result = compute_node_click_selection(
            &prev, "n1", None, true, &InteractionMode::Default,
        );
        match result {
            SelectionState::Single(id) => assert_eq!(id, "n1"),
            other => panic!("expected Single(n1), got {other:?}"),
        }
    }

    /// Cross-node MultiSection toggle-off: shift-clicking a section
    /// already in the set removes only that pair, leaving the
    /// other-node members alone.
    #[test]
    fn test_shift_click_removes_cross_node_section_from_multi_section() {
        let prev =
            SelectionState::MultiSection(vec![sec("a", 0), sec("a", 1), sec("b", 0)]);
        let result = compute_node_click_selection(
            &prev, "b", Some(0), true, &node_edit_for("b"),
        );
        match result {
            SelectionState::MultiSection(secs) => {
                assert_eq!(secs.len(), 2, "got {secs:?}");
                assert!(secs.contains(&sec("a", 0)));
                assert!(secs.contains(&sec("a", 1)));
                assert!(!secs.contains(&sec("b", 0)));
            }
            other => panic!("expected MultiSection of length 2, got {other:?}"),
        }
    }

    // Plain click — non-section behavior stays intact.

    #[test]
    fn test_plain_click_overrides_existing_multi_with_single() {
        let prev = SelectionState::Multi(vec!["a".into(), "b".into()]);
        let result = compute_node_click_selection(
            &prev, "n0", None, false, &InteractionMode::Default,
        );
        match result {
            SelectionState::Single(id) => assert_eq!(id, "n0"),
            other => panic!("expected Single(n0), got {other:?}"),
        }
    }

    // Shift+click — whole-node toggle behavior stays intact.

    #[test]
    fn test_shift_click_same_single_node_toggles_off() {
        let result = compute_node_click_selection(
            &SelectionState::Single("n0".into()),
            "n0", None, true, &InteractionMode::Default,
        );
        assert!(matches!(result, SelectionState::None), "got {result:?}");
    }

    #[test]
    fn test_shift_click_different_single_node_promotes_to_multi() {
        let result = compute_node_click_selection(
            &SelectionState::Single("a".into()),
            "b", None, true, &InteractionMode::Default,
        );
        match result {
            SelectionState::Multi(ids) => {
                assert_eq!(ids, vec!["a".to_string(), "b".to_string()]);
            }
            other => panic!("expected Multi, got {other:?}"),
        }
    }
}
