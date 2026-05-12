// SPDX-License-Identifier: MPL-2.0

//! Scene rebuilders: turn (document, selection) into the per-role
//! Baumhard trees the renderer uses. `rebuild_all` walks every
//! canvas role; `rebuild_scene_only` skips the node tree;
//! `rebuild_after_selection_change` picks the right granularity for
//! a selection delta. Each `update_*_tree` dispatches between full
//! rebuild and §B2 in-place mutator via `AppScene`'s signature.

use crate::application::document::{apply_tree_highlights, MindMapDocument, SelectionState, HIGHLIGHT_COLOR};
use crate::application::renderer::Renderer;

/// Pure predicate for [`rebuild_after_selection_change`]'s
/// dispatch. Returns `true` when transitioning from `prev` to
/// `new` requires a full `rebuild_all` (node-tree highlights
/// need to be applied, shifted, or cleared). Returns `false`
/// when both selections are edge-adjacent and only the scene-
/// level highlight cascade moves.
///
/// Factored out of the helper so the decision is unit-testable
/// without renderer / scene-host setup — the full
/// `rebuild_after_selection_change` is an integration surface
/// over wgpu state.
pub(in crate::application::app) fn selection_change_touches_tree(
    prev: &SelectionState,
    new: &SelectionState,
) -> bool {
    fn touches_tree(sel: &SelectionState) -> bool {
        // `Section` joins `Single` / `Multi` because section-area
        // highlights are stamped through the node-tree's
        // `ColorFontRegions` (see `apply_tree_highlights`); a
        // Section-selection transition that goes through
        // `rebuild_scene_only` would leave the prior cyan stamp
        // un-cleared (or never apply a fresh one), leaking a
        // stale highlight.
        matches!(
            sel,
            SelectionState::Single(_)
                | SelectionState::Multi(_)
                | SelectionState::Section(_)
                | SelectionState::MultiSection(_)
                | SelectionState::SectionRange { .. }
        )
    }
    touches_tree(prev) || touches_tree(new)
}

/// Post-selection-change rebuild with the right granularity.
/// Picks `rebuild_all` when either the previous or new selection
/// is `Single` / `Multi` (node-tree highlights need reapplying or
/// clearing), `rebuild_scene_only` otherwise (edge-adjacent
/// selection changes only move scene-level highlight cascades,
/// not node text-buffer region colors).
///
/// Exists for every selection-change callsite that would
/// otherwise call `rebuild_all` unconditionally — under §4's
/// mobile budget a full rebuild on every edge-label / portal
/// click is wasted work on a large map. This helper makes the
/// right choice a one-liner so callers don't have to re-derive
/// the decision.
pub(in crate::application::app) fn rebuild_after_selection_change(
    prev_selection: &SelectionState,
    doc: &MindMapDocument,
    interaction_mode: &super::InteractionMode,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    if selection_change_touches_tree(prev_selection, &doc.selection) {
        rebuild_all(
            doc,
            interaction_mode,
            mindmap_tree,
            app_scene,
            renderer,
            scene_cache,
        );
    } else {
        rebuild_scene_only(doc, interaction_mode, app_scene, renderer, scene_cache);
    }
}

#[cfg(test)]
mod tests {
    use super::selection_change_touches_tree;
    use crate::application::document::{EdgeLabelSel, EdgeRef, PortalLabelSel, SelectionState};
    use baumhard::mindmap::scene_cache::EdgeKey;

    fn edge_ref() -> EdgeRef {
        EdgeRef::new("a", "b", "cross_link")
    }
    fn portal() -> PortalLabelSel {
        PortalLabelSel {
            edge_key: EdgeKey::new("a", "b", "cross_link"),
            endpoint_node_id: "a".into(),
        }
    }

    #[test]
    fn edge_adjacent_to_edge_adjacent_is_scene_only() {
        // Any pair of edge-adjacent variants (Edge / EdgeLabel /
        // PortalLabel / PortalText) transitions without touching
        // node text buffers — scene-only is correct.
        let variants = [
            SelectionState::Edge(edge_ref()),
            SelectionState::EdgeLabel(EdgeLabelSel::new(edge_ref())),
            SelectionState::PortalLabel(portal()),
            SelectionState::PortalText(portal()),
        ];
        for prev in &variants {
            for new in &variants {
                assert!(
                    !selection_change_touches_tree(prev, new),
                    "{:?} -> {:?} should be scene-only",
                    prev,
                    new
                );
            }
        }
    }

    #[test]
    fn transition_into_node_selection_needs_full_rebuild() {
        // Edge-adjacent -> Single / Multi: the new node must have
        // its highlight color region applied to its text buffer.
        let prev = SelectionState::EdgeLabel(EdgeLabelSel::new(edge_ref()));
        assert!(selection_change_touches_tree(
            &prev,
            &SelectionState::Single("n".into())
        ));
        assert!(selection_change_touches_tree(
            &prev,
            &SelectionState::Multi(vec!["a".into(), "b".into()])
        ));
    }

    #[test]
    fn transition_out_of_node_selection_needs_full_rebuild() {
        // Single / Multi -> Edge-adjacent: the previous node's
        // highlight must be CLEARED from its text buffer. Scene-
        // only would leave the stale highlight stuck.
        for prev in [
            SelectionState::Single("n".into()),
            SelectionState::Multi(vec!["a".into(), "b".into()]),
        ] {
            assert!(selection_change_touches_tree(
                &prev,
                &SelectionState::Edge(edge_ref())
            ));
            assert!(selection_change_touches_tree(
                &prev,
                &SelectionState::EdgeLabel(EdgeLabelSel::new(edge_ref()))
            ));
            assert!(selection_change_touches_tree(
                &prev,
                &SelectionState::PortalLabel(portal())
            ));
            assert!(selection_change_touches_tree(
                &prev,
                &SelectionState::PortalText(portal())
            ));
            assert!(selection_change_touches_tree(&prev, &SelectionState::None));
        }
    }

    #[test]
    fn none_to_edge_adjacent_is_scene_only() {
        // A fresh click on an edge label when nothing was
        // selected: no tree highlight to clear, no new one to
        // apply. Scene-only is correct.
        for new in [
            SelectionState::Edge(edge_ref()),
            SelectionState::EdgeLabel(EdgeLabelSel::new(edge_ref())),
            SelectionState::PortalLabel(portal()),
            SelectionState::PortalText(portal()),
        ] {
            assert!(
                !selection_change_touches_tree(&SelectionState::None, &new),
                "None -> {:?} should be scene-only",
                new
            );
        }
    }

    #[test]
    fn node_to_node_needs_full_rebuild() {
        // Node -> node: old highlight clears, new highlight
        // applies. Full rebuild in both directions.
        assert!(selection_change_touches_tree(
            &SelectionState::Single("a".into()),
            &SelectionState::Single("b".into())
        ));
    }

    /// Construction-side panic guard for the load-time pre-warm.
    /// `warm_handle_tree_arenas` runs synchronously before the
    /// window is visible, so a panic here aborts startup. The
    /// synthetic data uses stub `EdgeKey`s and empty `node_id`s;
    /// if any future change to those constructors adds validation
    /// that rejects the stubs, we want the failure to land in
    /// `cargo test` rather than on the user's first launch.
    #[test]
    fn warm_handle_tree_arenas_does_not_panic_on_fresh_scene() {
        let mut app_scene = crate::application::scene_host::AppScene::new();
        super::warm_handle_tree_arenas(&mut app_scene);
        // Sanity: all three handle roles have a registered tree
        // (the synthetic stamp). Caller is responsible for any
        // empty re-stamp that follows; this test only verifies
        // the warm itself didn't blow up.
        assert!(app_scene
            .canvas_id(crate::application::scene_host::CanvasRole::NodeResizeHandles)
            .is_some());
        assert!(app_scene
            .canvas_id(crate::application::scene_host::CanvasRole::SectionResizeHandles)
            .is_some());
        assert!(app_scene
            .canvas_id(crate::application::scene_host::CanvasRole::EdgeHandles)
            .is_some());
    }

    /// `mode_status_line` returns `None` for `Default` mode — no
    /// status overlay when the user has nothing modal active.
    #[test]
    fn test_mode_status_line_default_is_none() {
        use crate::application::app::InteractionMode;
        use crate::application::document::tests_common::pinned_two_section_node;
        let (doc, _id) = pinned_two_section_node();
        assert_eq!(super::mode_status_line(&InteractionMode::Default, &doc), None);
    }

    /// NodeEdit on a single-section node renders the short form
    /// (just `editing: <id>`) — section count [1 of 1] would be
    /// noise for migrated maps where every node has exactly one
    /// section.
    #[test]
    fn test_mode_status_line_node_edit_single_section_uses_short_form() {
        use crate::application::app::InteractionMode;
        use crate::application::document::tests_common::pinned_two_section_node;
        let (mut doc, id) = pinned_two_section_node();
        // Drop section 1 to make the node single-section.
        doc.mindmap.nodes.get_mut(&id).unwrap().sections.truncate(1);
        let mode = InteractionMode::NodeEdit { node_id: id.clone() };
        let line = super::mode_status_line(&mode, &doc).expect("text expected");
        assert!(line.starts_with("editing: "), "got {:?}", line);
        assert!(line.contains(&id), "got {:?}", line);
        assert!(
            !line.contains("section ["),
            "single-section must skip the [N of M] suffix; got {:?}",
            line
        );
    }

    /// NodeEdit on a multi-section node renders `editing: <id> — section [N of M]`
    /// where N is the active section idx + 1 (selection-derived) and M is
    /// the total section count.
    #[test]
    fn test_mode_status_line_node_edit_multi_section_renders_n_of_m() {
        use crate::application::app::InteractionMode;
        use crate::application::document::tests_common::pinned_two_section_node;
        use crate::application::document::{SectionSel, SelectionState};
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel::new(&id, 1));
        let mode = InteractionMode::NodeEdit { node_id: id.clone() };
        let line = super::mode_status_line(&mode, &doc).expect("text expected");
        assert!(line.contains(&id), "got {:?}", line);
        assert!(line.contains("section [2 of 2]"), "got {:?}", line);
    }

    /// Resize-mode status line spells out the target — node form is
    /// just the id, section form is `node[idx]`.
    #[test]
    fn test_mode_status_line_resize_node_target_renders_id() {
        use crate::application::app::interaction_mode::ResizeTarget;
        use crate::application::app::InteractionMode;
        use crate::application::document::tests_common::pinned_two_section_node;
        let (doc, id) = pinned_two_section_node();
        let mode = InteractionMode::Resize {
            target: ResizeTarget::Node(id.clone()),
        };
        let line = super::mode_status_line(&mode, &doc).expect("text expected");
        assert!(line.starts_with("resize: "), "got {:?}", line);
        assert!(line.contains(&id), "got {:?}", line);
    }

    #[test]
    fn test_mode_status_line_resize_section_target_renders_indexed_form() {
        use crate::application::app::interaction_mode::ResizeTarget;
        use crate::application::app::InteractionMode;
        use crate::application::document::tests_common::pinned_two_section_node;
        let (doc, id) = pinned_two_section_node();
        let mode = InteractionMode::Resize {
            target: ResizeTarget::Section {
                node_id: id.clone(),
                section_idx: 1,
            },
        };
        let line = super::mode_status_line(&mode, &doc).expect("text expected");
        assert!(line.contains(&format!("{}[1]", id)), "got {:?}", line);
    }

    /// NodeEdit on a multi-section node with a `Single` selection
    /// (the user entered NodeEdit but hasn't picked a section yet)
    /// renders `[- of N]` rather than the misleading `[1 of N]`
    /// fabricated index. Pin from the Opus review TIER2.
    #[test]
    fn test_mode_status_line_node_edit_single_no_section_picked_renders_dash() {
        use crate::application::app::InteractionMode;
        use crate::application::document::tests_common::pinned_two_section_node;
        use crate::application::document::SelectionState;
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Single(id.clone());
        let mode = InteractionMode::NodeEdit { node_id: id.clone() };
        let line = super::mode_status_line(&mode, &doc).expect("text expected");
        assert!(line.contains("section [- of 2]"), "got {:?}", line);
    }

    /// NodeEdit on a multi-section node with a Section selection
    /// pointing at a *different* node (drift, e.g. user clicked a
    /// sibling) — same `[- of N]` outcome because the selection
    /// owner doesn't match the active NodeEdit target.
    #[test]
    fn test_mode_status_line_node_edit_section_owner_mismatch_renders_dash() {
        use crate::application::app::InteractionMode;
        use crate::application::document::tests_common::pinned_two_section_node;
        use crate::application::document::{SectionSel, SelectionState};
        let (mut doc, id) = pinned_two_section_node();
        // Selection points at a different (synthetic) node id.
        doc.selection = SelectionState::Section(SectionSel {
            node_id: "other-node".to_string(),
            section_idx: 0,
        });
        let mode = InteractionMode::NodeEdit { node_id: id.clone() };
        let line = super::mode_status_line(&mode, &doc).expect("text expected");
        assert!(line.contains("section [- of 2]"), "got {:?}", line);
    }

    /// Reparent mode with one source renders the singular form
    /// "1 source" — pluralization branch boundary.
    #[test]
    fn test_mode_status_line_reparent_singular_source() {
        use crate::application::app::InteractionMode;
        use crate::application::document::tests_common::load_test_doc;
        let doc = load_test_doc();
        let mode = InteractionMode::Reparent {
            sources: vec!["only-source".to_string()],
        };
        let line = super::mode_status_line(&mode, &doc).expect("text expected");
        assert!(line.contains("1 source "), "got {:?}", line);
        assert!(
            !line.contains("1 sources"),
            "singular form must not pluralize: {:?}",
            line
        );
    }

    /// Reparent mode with two+ sources renders the plural form.
    #[test]
    fn test_mode_status_line_reparent_plural_sources() {
        use crate::application::app::InteractionMode;
        use crate::application::document::tests_common::load_test_doc;
        let doc = load_test_doc();
        let mode = InteractionMode::Reparent {
            sources: vec!["a".to_string(), "b".to_string()],
        };
        let line = super::mode_status_line(&mode, &doc).expect("text expected");
        assert!(line.contains("2 sources"), "plural form must render: {:?}", line);
    }

    /// Connect mode renders the source node id in the status line.
    #[test]
    fn test_mode_status_line_connect_renders_source() {
        use crate::application::app::InteractionMode;
        use crate::application::document::tests_common::load_test_doc;
        let doc = load_test_doc();
        let mode = InteractionMode::Connect {
            source: "src-node-42".to_string(),
        };
        let line = super::mode_status_line(&mode, &doc).expect("text expected");
        assert!(line.starts_with("connect: "), "got {:?}", line);
        assert!(line.contains("src-node-42"), "got {:?}", line);
    }
}

pub(in crate::application::app) fn rebuild_all(
    doc: &MindMapDocument,
    interaction_mode: &super::InteractionMode,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    let mut new_tree = doc.build_tree();
    apply_tree_highlights(&mut new_tree, selection_highlight_entries(&doc.selection));
    renderer.rebuild_buffers_from_tree(&new_tree.tree);

    rebuild_scene_only(doc, interaction_mode, app_scene, renderer, scene_cache);
    renderer.set_mode_status_text(mode_status_line(interaction_mode, doc));

    *mindmap_tree = Some(new_tree);
}

/// Compute the mode-status overlay line for the active interaction
/// mode. Returns `None` for `Default` (no overlay), `Some(text)` for
/// every other mode. Pure derivation from `(mode, doc, selection)`
/// — pulled into a helper so [`rebuild_all`] can pass the result
/// straight to `Renderer::set_mode_status_text`.
///
/// Format pinned in §3.5 of `SECTIONS_BORDERS_RESIZE_PLAN.md`.
pub(in crate::application::app) fn mode_status_line(
    interaction_mode: &super::InteractionMode,
    doc: &MindMapDocument,
) -> Option<String> {
    use super::InteractionMode;
    match interaction_mode {
        InteractionMode::Default => None,
        InteractionMode::NodeEdit { node_id } => {
            let total = doc
                .mindmap
                .nodes
                .get(node_id)
                .map(|n| n.sections.len())
                .unwrap_or(0);
            // Active section index, 1-indexed for display. `None` when
            // selection isn't narrowed to a section of the active
            // NodeEdit node (Single selection on the node, drift to
            // an edge / portal, etc.). Pre-fix this defaulted to 1
            // and showed `[1 of N]` even when no section was picked
            // — misleading. Now we render `[- of N]` so the user
            // sees they still need to pick a section.
            let active_idx_display = doc
                .selection
                .selected_section()
                .filter(|s| s.node_id == *node_id)
                .map(|s| {
                    // Clamp against `total` so a stale selection
                    // pointing past the section count after a custom
                    // mutation doesn't render `[N+1 of N]`.
                    (s.section_idx + 1).min(total.max(1))
                });
            if total <= 1 {
                // Single-section (or stale-mode-after-deletion node
                // with `total == 0`): short form. The status bar
                // can't say anything more meaningful than the node id.
                Some(format!("editing: {}", node_id))
            } else {
                let idx_label = match active_idx_display {
                    Some(n) => n.to_string(),
                    None => "-".to_string(),
                };
                Some(format!(
                    "editing: {} \u{2014} section [{} of {}]",
                    node_id, idx_label, total
                ))
            }
        }
        InteractionMode::Resize { target } => {
            use super::interaction_mode::ResizeTarget;
            let target_label = match target {
                ResizeTarget::Node(id) => id.clone(),
                ResizeTarget::Section { node_id, section_idx } => {
                    format!("{}[{}]", node_id, section_idx)
                }
            };
            Some(format!("resize: {} \u{2014} drag a corner or edge", target_label))
        }
        InteractionMode::Reparent { sources } => {
            let count = sources.len();
            Some(format!(
                "reparent: {} source{} \u{2014} click a target node",
                count,
                if count == 1 { "" } else { "s" },
            ))
        }
        InteractionMode::Connect { source } => {
            Some(format!("connect: {} \u{2014} click a target node", source))
        }
    }
}

/// Map a [`SelectionState`] to the highlight entries
/// `apply_tree_highlights` consumes — one
/// `(node_id, only_section_idx, color)` triple per highlighted
/// region. Single / Multi yield whole-node entries (None
/// section index — every section of the named node tints).
/// Section yields a single entry restricted to the targeted
/// section. MultiSection yields one entry per `(node_id,
/// section_idx)` pair, so a multi-section set highlights
/// only the selected sections (and a multi-section set on
/// one node tints just those sections, leaving sibling
/// sections untouched).
pub(in crate::application::app) fn selection_highlight_entries(
    selection: &SelectionState,
) -> Vec<(&str, Option<usize>, [f32; 4])> {
    match selection {
        SelectionState::Section(s) => {
            vec![(s.node_id.as_str(), Some(s.section_idx), HIGHLIGHT_COLOR)]
        }
        SelectionState::MultiSection(secs) => secs
            .iter()
            .map(|s| (s.node_id.as_str(), Some(s.section_idx), HIGHLIGHT_COLOR))
            .collect(),
        // Range-aware sub-grapheme highlight is deferred to a
        // future tier; for now narrow the highlight to the
        // owning section (same shape as `Section`) so the user
        // can see which section their range targets.
        SelectionState::SectionRange { sel, .. } => {
            vec![(sel.node_id.as_str(), Some(sel.section_idx), HIGHLIGHT_COLOR)]
        }
        _ => selection
            .selected_ids()
            .into_iter()
            .map(|id| (id, None, HIGHLIGHT_COLOR))
            .collect(),
    }
}

/// Narrower cousin of `rebuild_all` that rebuilds only the flat
/// scene pipeline (connections, borders, edge handles, labels,
/// portals) — NOT the tree (node text buffers, node backgrounds).
/// Used by the glyph-wheel color picker's hover path: a per-frame
/// color preview doesn't change node text, borders, or positions,
/// so the tree rebuild is wasted work. Halves the hot-path cost vs
/// `rebuild_all` on maps with many nodes.
///
/// Uses the cache-aware `build_scene_with_cache` entry point so
/// unchanged edge geometry (`sample_path` samples) is reused from
/// the persistent `SceneConnectionCache`. This matches what the
/// drag drains (`MovingNode`, `EdgeHandle`, `EdgeLabel`,
/// `PortalLabel`) already do — every throttled consumer that
/// reaches this helper now inherits the same optimization.
pub(in crate::application::app) fn rebuild_scene_only(
    doc: &MindMapDocument,
    interaction_mode: &super::InteractionMode,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    let scene = doc.build_scene_with_cache(
        &std::collections::HashMap::new(),
        scene_cache,
        renderer.camera_zoom(),
        interaction_mode.resize_handle_overrides(),
    );
    update_connection_tree(&scene, app_scene);
    update_border_tree_static(doc, app_scene);
    update_portal_tree(doc, &std::collections::HashMap::new(), app_scene, renderer);
    update_edge_handle_tree(&scene, app_scene);
    update_section_resize_handle_tree(&scene, app_scene);
    update_node_resize_handle_tree(&scene, app_scene);
    update_section_frame_tree(&scene, app_scene);
    update_connection_label_tree(&scene, app_scene, renderer);
    flush_canvas_scene_buffers(app_scene, renderer);
}

// =====================================================================
// Canvas-tree update helpers.
//
// Each helper builds a baumhard tree for one canvas role and
// registers it into `AppScene`'s canvas sub-scene. **They do not
// re-walk the scene into renderer buffers** — that's the caller's
// responsibility, via `flush_canvas_scene_buffers`. Folding the
// flush into each helper would cost N tree walks per
// rebuild_scene_only call (one per role) when 1 suffices.
// =====================================================================

/// Build the border tree (no drag offsets) and register it under
/// [`crate::application::scene_host::CanvasRole::Borders`]. Caller
/// must follow with [`flush_canvas_scene_buffers`] before the next
/// render.
pub(in crate::application::app) fn update_border_tree_static(
    doc: &MindMapDocument,
    app_scene: &mut crate::application::scene_host::AppScene,
) {
    update_border_tree_with_offsets(doc, &std::collections::HashMap::new(), app_scene);
}

/// Build or in-place update the border tree under
/// [`crate::application::scene_host::CanvasRole::Borders`].
///
/// **§B2 dispatch.** The hot path this closes: when the color
/// picker is open, every throttled `AboutToWait` drain calls
/// `rebuild_scene_only`, which runs this function. Pre-dispatch,
/// that meant a fresh `Tree<GfxElement, GfxMutator>` allocation
/// per picker-hover frame plus a full canvas-scene buffer
/// re-shape — O(n_borders × per-glyph shape cost). With the
/// identity-sequence dispatch below, hover takes the in-place
/// mutator path (which walks the same per-node Void + 4 runs but
/// only overwrites variable fields) and the arena is reused.
///
/// Structural identity: the sorted sequence of bordered
/// (non-folded, `show_frame = true`) node IDs. Drag, text-edit,
/// color-preview, and preset-swap all leave this stable. Adding
/// / removing a framed node, folding an ancestor, or toggling
/// `show_frame` shifts the sequence and the dispatcher takes the
/// full rebuild.
pub(in crate::application::app) fn update_border_tree_with_offsets(
    doc: &MindMapDocument,
    offsets: &std::collections::HashMap<String, (f32, f32)>,
    app_scene: &mut crate::application::scene_host::AppScene,
) {
    use crate::application::scene_host::{hash_canvas_signature, CanvasDispatch, CanvasRole};
    use baumhard::mindmap::tree_builder::{
        border_identity_sequence, border_node_data, build_border_mutator_tree_from_nodes,
        build_border_tree_from_nodes,
    };

    let nodes = border_node_data(&doc.mindmap, offsets);
    let signature = hash_canvas_signature(&border_identity_sequence(&nodes));

    match app_scene.canvas_dispatch(CanvasRole::Borders, signature) {
        CanvasDispatch::InPlaceMutator => {
            let mutator = build_border_mutator_tree_from_nodes(&nodes);
            app_scene.apply_canvas_mutator(CanvasRole::Borders, &mutator);
        }
        CanvasDispatch::FullRebuild => {
            let tree = build_border_tree_from_nodes(&nodes);
            app_scene.register_canvas(CanvasRole::Borders, tree, glam::Vec2::ZERO);
            app_scene.set_canvas_signature(CanvasRole::Borders, signature);
        }
    }
}

/// Build or in-place update the portal tree under
/// [`crate::application::scene_host::CanvasRole::Portals`].
/// Selection-cyan and color-preview override rules mirror
/// `scene_builder::build_scene`. Hands the AABB-keyed hitbox map
/// back to the renderer so the legacy `Renderer::hit_test_portal`
/// keeps working until hit-test routing migrates to
/// [`baumhard::gfx_structs::scene::Scene::component_at`].
///
/// **§B2 dispatch.** Drag, color-preview, and selection toggle
/// all leave the visible-portal *identity sequence* unchanged —
/// the same pairs in the same order, only their positions /
/// colors / regions move. For those continuous interactions we
/// take the in-place mutator path
/// (`build_portal_mutator_tree_from_pairs` →
/// `apply_canvas_mutator`), which reuses the existing tree arena
/// instead of allocating a new one each frame. When portals are
/// added, removed, or a fold reveals/hides an endpoint, the
/// identity sequence shifts and we fall back to a full rebuild.
/// Mirrors the canonical in-place mutator pattern from the picker,
/// now applied to a nested-channel tree.
pub(in crate::application::app) fn update_portal_tree(
    doc: &MindMapDocument,
    offsets: &std::collections::HashMap<String, (f32, f32)>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    use crate::application::scene_host::{hash_canvas_signature, CanvasDispatch, CanvasRole};
    use baumhard::mindmap::scene_builder::SelectedPortalLabel;
    use baumhard::mindmap::tree_builder::{
        build_portal_mutator_tree_from_pairs, build_portal_tree_from_pairs, portal_identity_sequence,
        portal_pair_data, PortalColorPreviewRef, SelectedEdgeRef,
    };

    let selected_owned = doc
        .selection
        .selected_edge()
        .map(|e| (e.from_id.clone(), e.to_id.clone(), e.edge_type.clone()));
    let selected: Option<SelectedEdgeRef> = selected_owned
        .as_ref()
        .map(|(f, t, ty)| (f.as_str(), t.as_str(), ty.as_str()));
    let selected_portal_label: Option<SelectedPortalLabel> = doc.selection.selected_portal_label_scene_ref();

    // The picker preview fans out to the portal pass whenever the
    // previewed edge is portal-mode. `ColorPickerPreview` is a
    // struct (one shape, one preview) — no Portal variant needed,
    // the edge `key` is enough to fan out.
    let preview: Option<PortalColorPreviewRef> =
        doc.color_picker_preview.as_ref().map(|p| PortalColorPreviewRef {
            edge_key: &p.key,
            color: p.color.as_str(),
        });

    // Portal text-edit preview mirrors the existing
    // `label_edit_preview`: when the inline portal-text editor is
    // open, its buffer substitutes for the committed
    // `PortalEndpointState.text` on the named endpoint so edits
    // render live.
    let portal_text_edit = doc
        .portal_text_edit_preview
        .as_ref()
        .map(
            |(key, endpoint, buffer)| baumhard::mindmap::scene_builder::PortalTextEditOverride {
                edge_key: key,
                endpoint_node_id: endpoint.as_str(),
                buffer: buffer.as_str(),
            },
        );

    let pairs = portal_pair_data(
        &doc.mindmap,
        offsets,
        selected,
        selected_portal_label,
        preview,
        portal_text_edit,
        renderer.camera_zoom(),
    );
    let signature = hash_canvas_signature(&portal_identity_sequence(&pairs));

    match app_scene.canvas_dispatch(CanvasRole::Portals, signature) {
        CanvasDispatch::InPlaceMutator => {
            let result = build_portal_mutator_tree_from_pairs(&pairs);
            renderer.set_portal_icon_hitboxes(result.icon_hitboxes);
            renderer.set_portal_text_hitboxes(result.text_hitboxes);
            app_scene.apply_canvas_mutator(CanvasRole::Portals, &result.mutator);
        }
        CanvasDispatch::FullRebuild => {
            let result = build_portal_tree_from_pairs(&pairs);
            renderer.set_portal_icon_hitboxes(result.icon_hitboxes);
            renderer.set_portal_text_hitboxes(result.text_hitboxes);
            app_scene.register_canvas(CanvasRole::Portals, result.tree, glam::Vec2::ZERO);
            app_scene.set_canvas_signature(CanvasRole::Portals, signature);
        }
    }
}

/// Build or in-place update the connection tree under
/// [`crate::application::scene_host::CanvasRole::Connections`].
///
/// **§B2 dispatch.** Selection toggle, color preview, and theme
/// switches change only per-glyph fields (color regions, body
/// glyph) without altering the per-edge structural shape (cap
/// presence, body-glyph count). For those calls we take the
/// in-place mutator path. Endpoint drag resamples the path and
/// the body-glyph count typically shifts every few pixels — the
/// identity sequence drops the equality and we fall back to a
/// full rebuild. The dispatcher hashes
/// `connection_identity_sequence` to make the choice.
pub(in crate::application::app) fn update_connection_tree(
    scene: &baumhard::mindmap::scene_builder::RenderScene,
    app_scene: &mut crate::application::scene_host::AppScene,
) {
    use crate::application::scene_host::{hash_canvas_signature, CanvasDispatch, CanvasRole};
    use baumhard::mindmap::tree_builder::{
        build_connection_mutator_tree, build_connection_tree, connection_identity_sequence,
    };

    let signature = hash_canvas_signature(&connection_identity_sequence(&scene.connection_elements));
    match app_scene.canvas_dispatch(CanvasRole::Connections, signature) {
        CanvasDispatch::InPlaceMutator => {
            let mutator = build_connection_mutator_tree(&scene.connection_elements);
            app_scene.apply_canvas_mutator(CanvasRole::Connections, &mutator);
        }
        CanvasDispatch::FullRebuild => {
            let tree = build_connection_tree(&scene.connection_elements);
            app_scene.register_canvas(CanvasRole::Connections, tree, glam::Vec2::ZERO);
            app_scene.set_canvas_signature(CanvasRole::Connections, signature);
        }
    }
}

/// Build or in-place update the connection-label tree under
/// [`crate::application::scene_host::CanvasRole::ConnectionLabels`].
/// Threads the per-edge AABB
/// hitbox map back to the renderer so `hit_test_edge_label`
/// keeps working.
///
/// **§B2 dispatch.** Inline label edits (the hot path),
/// color changes, and label movement keep the structural identity
/// (the per-edge `EdgeKey` sequence) stable; the in-place mutator
/// path runs and the arena is reused. Adding or removing a label,
/// or selection-edge reorderings, change the identity and
/// trigger a full rebuild.
pub(in crate::application::app) fn update_connection_label_tree(
    scene: &baumhard::mindmap::scene_builder::RenderScene,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    use crate::application::scene_host::{hash_canvas_signature, CanvasDispatch, CanvasRole};
    use baumhard::mindmap::tree_builder::{
        build_connection_label_mutator_tree, build_connection_label_tree, connection_label_identity_sequence,
    };

    let signature = hash_canvas_signature(&connection_label_identity_sequence(
        &scene.connection_label_elements,
    ));
    match app_scene.canvas_dispatch(CanvasRole::ConnectionLabels, signature) {
        CanvasDispatch::InPlaceMutator => {
            let result = build_connection_label_mutator_tree(&scene.connection_label_elements);
            renderer.set_connection_label_hitboxes(result.hitboxes);
            app_scene.apply_canvas_mutator(CanvasRole::ConnectionLabels, &result.mutator);
        }
        CanvasDispatch::FullRebuild => {
            let result = build_connection_label_tree(&scene.connection_label_elements);
            renderer.set_connection_label_hitboxes(result.hitboxes);
            app_scene.register_canvas(CanvasRole::ConnectionLabels, result.tree, glam::Vec2::ZERO);
            app_scene.set_canvas_signature(CanvasRole::ConnectionLabels, signature);
        }
    }
}

/// Build or in-place update the edge-handle tree under
/// [`crate::application::scene_host::CanvasRole::EdgeHandles`].
///
/// **§B2 dispatch.** Dragging a handle moves only its position;
/// the handle set's *identity sequence* (the
/// kind-derived channels emitted by
/// [`baumhard::mindmap::tree_builder::handle_identity_sequence`])
/// stays constant for the duration of one drag. We take the in-place
/// mutator path under that condition, reusing the existing arena
/// instead of allocating a fresh one each frame. When the handle
/// set's structure shifts — selection moves to a different edge
/// shape, or a midpoint drag spawns a control point — the identity
/// sequence changes and we fall back to a full rebuild. Mirrors the
/// dispatch shape used in `update_portal_tree`.
pub(in crate::application::app) fn update_edge_handle_tree(
    scene: &baumhard::mindmap::scene_builder::RenderScene,
    app_scene: &mut crate::application::scene_host::AppScene,
) {
    update_handle_canvas_role(
        crate::application::scene_host::CanvasRole::EdgeHandles,
        &scene.edge_handles,
        app_scene,
    );
}

/// Same as [`update_edge_handle_tree`] but takes the element slice
/// directly. Used by the load-time pre-warm to feed synthetic
/// 8-handle data through the dispatch path so the handle-tree
/// builder's arena allocates from warm pools — without forcing
/// a full `RenderScene` build that already happens upstream.
pub(in crate::application::app) fn update_edge_handle_tree_from_slice(
    elements: &[baumhard::mindmap::scene_builder::EdgeHandleElement],
    app_scene: &mut crate::application::scene_host::AppScene,
) {
    update_handle_canvas_role(
        crate::application::scene_host::CanvasRole::EdgeHandles,
        elements,
        app_scene,
    );
}

/// Build or in-place update the section-resize-handle tree under
/// [`crate::application::scene_host::CanvasRole::SectionResizeHandles`].
/// Sibling of `update_edge_handle_tree`; same §B2 dispatch shape.
///
/// Resize handle counts are 0 (no Section selection / fill-parent
/// section) or 8 (Some-sized selected section), so the identity-
/// sequence-based dispatch flips between the two on selection
/// transitions and stays stable during a resize drag.
pub(in crate::application::app) fn update_section_resize_handle_tree(
    scene: &baumhard::mindmap::scene_builder::RenderScene,
    app_scene: &mut crate::application::scene_host::AppScene,
) {
    update_section_resize_handle_tree_from_slice(&scene.section_resize_handles, app_scene);
}

/// Build or in-place update the node-resize-handle tree under
/// [`crate::application::scene_host::CanvasRole::NodeResizeHandles`].
/// Sibling of `update_section_resize_handle_tree`; same §B2
/// dispatch. Selection-gated 0 ↔ 8 transitions take the full-
/// rebuild arm; a steady drag stays on the in-place mutator arm.
pub(in crate::application::app) fn update_node_resize_handle_tree(
    scene: &baumhard::mindmap::scene_builder::RenderScene,
    app_scene: &mut crate::application::scene_host::AppScene,
) {
    update_node_resize_handle_tree_from_slice(&scene.node_resize_handles, app_scene);
}

/// Same as [`update_node_resize_handle_tree`] but takes the
/// element slice directly. Used by the resize drain to refresh
/// handle positions per-frame against the in-progress AABB
/// without round-tripping through a full `RenderScene` build.
pub(in crate::application::app) fn update_node_resize_handle_tree_from_slice(
    elements: &[baumhard::mindmap::scene_builder::NodeResizeHandleElement],
    app_scene: &mut crate::application::scene_host::AppScene,
) {
    update_handle_canvas_role(
        crate::application::scene_host::CanvasRole::NodeResizeHandles,
        elements,
        app_scene,
    );
}

/// Same as [`update_section_resize_handle_tree`] but takes the
/// element slice directly. Used by the resize drain to refresh
/// handle positions per-frame against the in-progress AABB
/// without round-tripping through a full `RenderScene` build.
pub(in crate::application::app) fn update_section_resize_handle_tree_from_slice(
    elements: &[baumhard::mindmap::scene_builder::SectionResizeHandleElement],
    app_scene: &mut crate::application::scene_host::AppScene,
) {
    update_handle_canvas_role(
        crate::application::scene_host::CanvasRole::SectionResizeHandles,
        elements,
        app_scene,
    );
}

/// Build or in-place register the section-frame tree under
/// [`crate::application::scene_host::CanvasRole::SectionFrames`].
/// Empty input → a trivial tree (one void root, no children),
/// which is fine: the renderer skips empty trees during canvas
/// flush.
///
/// Section-frame visibility is mode-driven (NodeEdit on / off),
/// not gesture-driven. The dispatch's structural signature is
/// the [`baumhard::mindmap::tree_builder::section_frame_identity_sequence`]
/// output, which captures
/// `(node_id, section_idx, focused, color, per-side rendered
/// text)`. Any visible style change — preset, pattern, corner,
/// color, focus toggle — moves the signature, so the dispatch
/// triggers a full rebuild correctly.
///
/// There's no §B2 in-place mutator path: section-frame style
/// changes (e.g. an author edits a per-side `SidePattern`)
/// reshape the glyph runs entirely, and the focus toggle swaps
/// preset glyph sets. A delta-style mutator would have to
/// re-stamp every field of every run anyway, so the
/// `FullRebuild` arm is the only meaningful path. The matching
/// `InPlaceMutator` arm short-circuits to a no-op: the
/// registered tree is already correct, no work needed.
pub(in crate::application::app) fn update_section_frame_tree(
    scene: &baumhard::mindmap::scene_builder::RenderScene,
    app_scene: &mut crate::application::scene_host::AppScene,
) {
    use crate::application::scene_host::{CanvasDispatch, CanvasRole};
    use baumhard::mindmap::tree_builder::{build_section_frame_tree, section_frame_identity_sequence};

    let signature = section_frame_identity_sequence(&scene.section_frames);
    match app_scene.canvas_dispatch(CanvasRole::SectionFrames, signature) {
        CanvasDispatch::InPlaceMutator => {
            // Signature matched the registered tree — nothing
            // changed since the last rebuild, so the registered
            // tree is already correct. Early-return saves the
            // tree-allocation + register_canvas slab churn that
            // would otherwise fire on every NodeEdit-mode rebuild.
        }
        CanvasDispatch::FullRebuild => {
            let tree = build_section_frame_tree(&scene.section_frames);
            app_scene.register_canvas(CanvasRole::SectionFrames, tree, glam::Vec2::ZERO);
            app_scene.set_canvas_signature(CanvasRole::SectionFrames, signature);
        }
    }
}

/// Generic §B2 dispatch for any handle-bearing canvas role —
/// edge handles, section resize handles, node resize handles.
/// Each role's `update_*_handle_tree*` wrapper picks the
/// `CanvasRole` and the element slice; this fn routes through
/// the trait-generic `build_handle_tree` /
/// `build_handle_mutator_tree` / `handle_identity_sequence` so
/// the §5-flagged triplication of three-near-identical
/// per-domain dispatchers collapses to one source of truth.
fn update_handle_canvas_role<E: baumhard::mindmap::tree_builder::HandleVisual>(
    role: crate::application::scene_host::CanvasRole,
    elements: &[E],
    app_scene: &mut crate::application::scene_host::AppScene,
) {
    use crate::application::scene_host::{hash_canvas_signature, CanvasDispatch};
    use baumhard::mindmap::tree_builder::{
        build_handle_mutator_tree, build_handle_tree, handle_identity_sequence,
    };

    let signature = hash_canvas_signature(&handle_identity_sequence(elements));
    match app_scene.canvas_dispatch(role, signature) {
        CanvasDispatch::InPlaceMutator => {
            let mutator = build_handle_mutator_tree(elements);
            app_scene.apply_canvas_mutator(role, &mutator);
        }
        CanvasDispatch::FullRebuild => {
            let tree = build_handle_tree(elements);
            app_scene.register_canvas(role, tree, glam::Vec2::ZERO);
            app_scene.set_canvas_signature(role, signature);
        }
    }
}

/// Walk every canvas-scene tree once and rebuild the renderer's
/// `canvas_scene_buffers`. Call this **once** after a batch of
/// `update_*_tree` invocations — calling it inside each helper
/// would multiply the per-frame shaping cost by the number of
/// roles touched.
pub(in crate::application::app) fn flush_canvas_scene_buffers(
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    renderer.rebuild_canvas_scene_buffers(app_scene);
}

/// Feed the three handle-tree canvas roles synthetic 8-element
/// slices once so the handle-tree builder's arena allocates from
/// cold pools at load rather than on the user's first selection.
/// Caller is responsible for re-stamping the load-time empty
/// signature afterwards (e.g. another `update_*_handle_tree`
/// pass with the live `RenderScene`) so the active canvas state
/// at load-end matches what's actually on screen — the synthetic
/// stamp leaves the 8-handle signature in place, which would
/// otherwise force an immediate FullRebuild on every drain that
/// queries the role.
///
/// The synthetic data is not sound for rendering — positions are
/// `(0, 0)`, `node_id` is empty, the edge key is a stub — but
/// `build_handle_tree` only reads `HandleVisual` trait methods
/// (channel / glyph / color / position / font_size_pt), which
/// the synthetic elements satisfy. No glyph rendering happens
/// until `flush_canvas_scene_buffers`, which the caller invokes
/// after the empty re-stamp.
pub(in crate::application::app) fn warm_handle_tree_arenas(
    app_scene: &mut crate::application::scene_host::AppScene,
) {
    use baumhard::mindmap::scene_builder::{
        EdgeHandleElement, EdgeHandleKind, NodeResizeHandleElement, ResizeHandleSide,
        SectionResizeHandleElement,
    };
    use baumhard::mindmap::scene_cache::EdgeKey;

    let sides = [
        ResizeHandleSide::NW,
        ResizeHandleSide::N,
        ResizeHandleSide::NE,
        ResizeHandleSide::E,
        ResizeHandleSide::SE,
        ResizeHandleSide::S,
        ResizeHandleSide::SW,
        ResizeHandleSide::W,
    ];

    let node_handles: Vec<NodeResizeHandleElement> = sides
        .iter()
        .map(|&side| NodeResizeHandleElement {
            node_id: String::new(),
            side,
            position: (0.0, 0.0),
            glyph: String::from("\u{25C7}"), // ◇
            color: String::from("#000000"),
            font_size_pt: 12.0,
        })
        .collect();
    update_node_resize_handle_tree_from_slice(&node_handles, app_scene);

    let section_handles: Vec<SectionResizeHandleElement> = sides
        .iter()
        .map(|&side| SectionResizeHandleElement {
            node_id: String::new(),
            section_idx: 0,
            side,
            position: (0.0, 0.0),
            glyph: String::from("\u{25C7}"), // ◇
            color: String::from("#000000"),
            font_size_pt: 12.0,
        })
        .collect();
    update_section_resize_handle_tree_from_slice(&section_handles, app_scene);

    // Edge handles: a typical selected edge produces 2-5 handles
    // (anchor-from, anchor-to, control points, midpoint). 5 is
    // a reasonable upper-bound warm size — the arena's bump
    // capacity scales with the largest count it sees.
    let edge_kinds = [
        EdgeHandleKind::AnchorFrom,
        EdgeHandleKind::AnchorTo,
        EdgeHandleKind::ControlPoint(0),
        EdgeHandleKind::ControlPoint(1),
        EdgeHandleKind::Midpoint,
    ];
    let edge_handles: Vec<EdgeHandleElement> = edge_kinds
        .iter()
        .map(|&kind| EdgeHandleElement {
            edge_key: EdgeKey::new("a", "b", "parent_child"),
            kind,
            position: (0.0, 0.0),
            glyph: String::from("\u{25C6}"), // ◆
            color: String::from("#000000"),
            font_size_pt: 12.0,
        })
        .collect();
    update_edge_handle_tree_from_slice(&edge_handles, app_scene);
}
