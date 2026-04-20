//! Scene rebuilders: turn the document model + selection into the
//! per-role baumhard trees the renderer registers, and dispatch
//! between full rebuild and §B2 in-place mutator paths via
//! `scene_host`'s canvas-signature comparator.
//!
//! `rebuild_all` is the post-mutation entry point — it rebuilds the
//! node tree from the model and walks every canvas role
//! (`update_*_tree`). `rebuild_scene_only` skips the node-tree
//! rebuild for paths that only changed scene data (selection,
//! preview overrides) without touching the model.

use crate::application::document::{
    apply_tree_highlights, MindMapDocument, HIGHLIGHT_COLOR,
};
use crate::application::renderer::Renderer;

pub(in crate::application::app) fn rebuild_all(
    doc: &MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    let mut new_tree = doc.build_tree();
    apply_tree_highlights(
        &mut new_tree,
        doc.selection
            .selected_ids()
            .into_iter()
            .map(|id| (id, HIGHLIGHT_COLOR)),
    );
    renderer.rebuild_buffers_from_tree(&new_tree.tree);

    rebuild_scene_only(doc, app_scene, renderer);

    *mindmap_tree = Some(new_tree);
}

/// Narrower cousin of `rebuild_all` that rebuilds only the flat
/// scene pipeline (connections, borders, edge handles, labels,
/// portals) — NOT the tree (node text buffers, node backgrounds).
/// Used by the glyph-wheel color picker's hover path: a per-frame
/// color preview doesn't change node text, borders, or positions,
/// so the tree rebuild is wasted work. Halves the hot-path cost vs
/// `rebuild_all` on maps with many nodes.
pub(in crate::application::app) fn rebuild_scene_only(
    doc: &MindMapDocument,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    let scene = doc.build_scene_with_selection(renderer.camera_zoom());
    update_connection_tree(&scene, app_scene);
    update_border_tree_static(doc, app_scene);
    update_portal_tree(doc, &std::collections::HashMap::new(), app_scene, renderer);
    update_edge_handle_tree(&scene, app_scene);
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
/// [`CanvasRole::Borders`]. Caller must follow with
/// [`flush_canvas_scene_buffers`] before the next render.
pub(in crate::application::app) fn update_border_tree_static(
    doc: &MindMapDocument,
    app_scene: &mut crate::application::scene_host::AppScene,
) {
    update_border_tree_with_offsets(doc, &std::collections::HashMap::new(), app_scene);
}

/// Build or in-place update the border tree under
/// [`CanvasRole::Borders`].
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
/// [`CanvasRole::Portals`]. Selection-cyan and color-preview
/// override rules mirror `scene_builder::build_scene`. Hands the
/// AABB-keyed hitbox map back to the renderer so the legacy
/// `Renderer::hit_test_portal` keeps working until hit-test
/// routing migrates to [`Scene::component_at`].
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
/// Mirrors the canonical pattern from the picker (commit
/// `ceaeeb4`), now applied to a nested-channel tree.
pub(in crate::application::app) fn update_portal_tree(
    doc: &MindMapDocument,
    offsets: &std::collections::HashMap<String, (f32, f32)>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    use crate::application::document::ColorPickerPreview;
    use crate::application::scene_host::{hash_canvas_signature, CanvasDispatch, CanvasRole};
    use baumhard::mindmap::scene_builder::SelectedPortalLabel;
    use baumhard::mindmap::tree_builder::{
        build_portal_mutator_tree_from_pairs, build_portal_tree_from_pairs,
        portal_identity_sequence, portal_pair_data, PortalColorPreviewRef, SelectedEdgeRef,
    };

    let selected_owned = doc
        .selection
        .selected_edge()
        .map(|e| (e.from_id.clone(), e.to_id.clone(), e.edge_type.clone()));
    let selected: Option<SelectedEdgeRef> = selected_owned
        .as_ref()
        .map(|(f, t, ty)| (f.as_str(), t.as_str(), ty.as_str()));
    let selected_portal_label: Option<SelectedPortalLabel> =
        doc.selection.selected_portal_label_scene_ref();

    // The picker preview fans out to the portal pass whenever the
    // previewed edge is portal-mode. No separate Portal variant on
    // `ColorPickerPreview` — the `Edge` key is enough.
    let preview: Option<PortalColorPreviewRef> = match &doc.color_picker_preview {
        Some(ColorPickerPreview::Edge { key, color }) => Some(PortalColorPreviewRef {
            edge_key: key,
            color: color.as_str(),
        }),
        _ => None,
    };

    // Portal text-edit preview mirrors the existing
    // `label_edit_preview`: when the inline portal-text editor is
    // open, its buffer substitutes for the committed
    // `PortalEndpointState.text` on the named endpoint so edits
    // render live.
    let portal_text_edit = doc
        .portal_text_edit_preview
        .as_ref()
        .map(|(key, endpoint, buffer)| {
            baumhard::mindmap::scene_builder::PortalTextEditOverride {
                edge_key: key,
                endpoint_node_id: endpoint.as_str(),
                buffer: buffer.as_str(),
            }
        });

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
            renderer.set_portal_hitboxes(result.hitboxes);
            app_scene.apply_canvas_mutator(CanvasRole::Portals, &result.mutator);
        }
        CanvasDispatch::FullRebuild => {
            let result = build_portal_tree_from_pairs(&pairs);
            renderer.set_portal_hitboxes(result.hitboxes);
            app_scene.register_canvas(CanvasRole::Portals, result.tree, glam::Vec2::ZERO);
            app_scene.set_canvas_signature(CanvasRole::Portals, signature);
        }
    }
}

/// Build or in-place update the connection tree under
/// [`CanvasRole::Connections`].
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

    let signature =
        hash_canvas_signature(&connection_identity_sequence(&scene.connection_elements));
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
/// [`CanvasRole::ConnectionLabels`]. Threads the per-edge AABB
/// hitbox map back to the renderer so `hit_test_edge_label`
/// keeps working.
///
/// **§B2 dispatch.** Inline label edits (Phase 2.1's hot path),
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
        build_connection_label_mutator_tree, build_connection_label_tree,
        connection_label_identity_sequence,
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
            app_scene.register_canvas(
                CanvasRole::ConnectionLabels,
                result.tree,
                glam::Vec2::ZERO,
            );
            app_scene.set_canvas_signature(CanvasRole::ConnectionLabels, signature);
        }
    }
}

/// Build or in-place update the edge-handle tree under
/// [`CanvasRole::EdgeHandles`].
///
/// **§B2 dispatch.** Dragging a handle moves only its position;
/// the handle set's *identity sequence* (the
/// kind-derived channels emitted by
/// [`baumhard::mindmap::tree_builder::edge_handle_identity_sequence`])
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
    use crate::application::scene_host::{hash_canvas_signature, CanvasDispatch, CanvasRole};
    use baumhard::mindmap::tree_builder::{
        build_edge_handle_mutator_tree, build_edge_handle_tree,
        edge_handle_identity_sequence,
    };

    let signature = hash_canvas_signature(&edge_handle_identity_sequence(&scene.edge_handles));
    match app_scene.canvas_dispatch(CanvasRole::EdgeHandles, signature) {
        CanvasDispatch::InPlaceMutator => {
            let mutator = build_edge_handle_mutator_tree(&scene.edge_handles);
            app_scene.apply_canvas_mutator(CanvasRole::EdgeHandles, &mutator);
        }
        CanvasDispatch::FullRebuild => {
            let tree = build_edge_handle_tree(&scene.edge_handles);
            app_scene.register_canvas(CanvasRole::EdgeHandles, tree, glam::Vec2::ZERO);
            app_scene.set_canvas_signature(CanvasRole::EdgeHandles, signature);
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
