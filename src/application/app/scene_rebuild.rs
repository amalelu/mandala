// SPDX-License-Identifier: MPL-2.0

//! Scene rebuilders: turn (document, interaction mode) into the
//! per-role Baumhard trees the renderer walks. `rebuild_all` also
//! rebuilds the node tree; `rebuild_scene_only` refreshes every
//! canvas role but leaves the node tree alone;
//! `rebuild_after_selection_change` picks the right granularity for
//! a selection delta.
//!
//! [`CanvasFrame`] is where the per-role work lives: it owns one
//! rebuild's shared inputs (the fold-hidden set, the assembled
//! [`FrameOverrides`]) and exposes one `update_*` per role, each
//! dispatching between full rebuild and §B2 in-place mutator via
//! `AppScene`'s signature. Callers pick only the roles their
//! interaction can actually change.

use crate::application::document::{
    apply_inactive_node_dimming, apply_tree_highlights, MindMapDocument, SelectionState, HIGHLIGHT_COLOR,
};
use crate::application::renderer::Renderer;
use baumhard::mindmap::tree_builder::{FrameOverrides, InteractionModeOverrides};

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
        rebuild_all(doc, interaction_mode, mindmap_tree, app_scene, renderer, scene_cache);
    } else {
        rebuild_scene_only(doc, interaction_mode, app_scene, renderer, scene_cache);
    }
}

/// Stamp **every render-layer overlay** onto an already-built node
/// tree, in the one order that is correct.
///
/// The three overlays are deliberately *not* part of
/// [`MindMapDocument::build_tree`](crate::application::document::MindMapDocument::build_tree):
/// that projection is what the `Persistent` custom-mutation apply
/// path syncs back to the model, so anything baked into it would be
/// written to disk (see `document/custom/mod.rs`, which rebuilds a
/// fresh `build_tree()` and diffs against it). They therefore have
/// to be re-applied at every site that rebuilds the tree — and
/// "every site" is the whole problem this function exists to solve.
///
/// Five sites rebuild the tree. Four install it as the live
/// `mindmap_tree` and must overlay it: [`rebuild_all`],
/// `click::rebuild_all_with_mode`,
/// `event_cursor_moved::rebuild_selection_highlight`, and
/// `drain_frame::drain_selecting_rect`. Before this helper each
/// open-coded its own subset — two silently omitted the `NodeEdit`
/// dim (a section drag mid-edit snapped every other node's text
/// back to full opacity while its border stayed dimmed), and three
/// omitted `reapply_active_toggles` (an active toggle's visual
/// vanished on any click-driven rebuild).
///
/// The fifth, `document::animations::tick`, deliberately installs an
/// **overlay-free** projection: `drain_animation_tick` calls
/// `rebuild_all` unconditionally on any advance, so the bare tree
/// exists for less than one frame and is superseded before it can be
/// drawn. That safety rests entirely on the call staying
/// unconditional; a future change that gates it must route the
/// animation rebuild through here.
///
/// Order is load-bearing:
///
/// 1. **Dim** — a base-layer alpha scale over every node that is
///    not the active `NodeEdit` target.
/// 2. **Highlights** — absolute `SetRegionColor` writes, so a
///    deliberate selection tint overwrites the dim rather than
///    compounding with it. A node that is both inactive and
///    selected must read as *selected*.
/// 3. **Active toggles** — re-stamped last so a toggle-on's visual
///    survives a rebuild-from-model (CONCEPTS §4).
///
/// # Costs
///
/// One `walk_tree_from` per section of each overlaid node. The dim
/// is a walk-free early return outside `NodeEdit`, and
/// `reapply_active_toggles` early-returns when no toggle is active,
/// so the steady-state cost is the highlight stamp alone.
pub(in crate::application::app) fn overlay_tree<'a, I>(
    tree: &mut baumhard::mindmap::tree_builder::MindMapTree,
    doc: &MindMapDocument,
    interaction_mode: &super::InteractionMode,
    highlights: I,
) where
    I: IntoIterator<Item = (&'a str, Option<usize>, [f32; 4])>,
{
    apply_inactive_node_dimming(tree, interaction_mode.node_edit_for());
    apply_tree_highlights(tree, highlights);
    doc.reapply_active_toggles(tree);
}

/// [`overlay_tree`] over a freshly-built tree — the shape three of
/// the four overlay sites want.
///
/// The fourth, `drain_frame::drain_selecting_rect`, needs the bare
/// tree first (its rect hit-test runs against it) and so calls
/// [`overlay_tree`] in place rather than building a second one.
///
/// # Costs
///
/// One `build_mindmap_tree` — benchmarked as `do_build_mindmap_tree`
/// and therefore hot-path by §B7 — plus [`overlay_tree`]'s walks.
pub(in crate::application::app) fn build_overlaid_tree<'a, I>(
    doc: &MindMapDocument,
    interaction_mode: &super::InteractionMode,
    highlights: I,
) -> baumhard::mindmap::tree_builder::MindMapTree
where
    I: IntoIterator<Item = (&'a str, Option<usize>, [f32; 4])>,
{
    let mut tree = doc.build_tree();
    overlay_tree(&mut tree, doc, interaction_mode, highlights);
    tree
}

pub(in crate::application::app) fn rebuild_all(
    doc: &MindMapDocument,
    interaction_mode: &super::InteractionMode,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    let new_tree = build_overlaid_tree(
        doc,
        interaction_mode,
        selection_highlight_entries(&doc.selection),
    );
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
            Some(format!(
                "resize: {} \u{2014} drag a corner or edge",
                target_label
            ))
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
            Some(format!(
                "connect: {} \u{2014} click a target node",
                source
            ))
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

/// Narrower cousin of `rebuild_all` that rebuilds every canvas
/// role (connections, borders, portals, labels, section frames,
/// handles) but **not** the node tree (node text buffers, node
/// backgrounds).
///
/// Used by the glyph-wheel color picker's hover path: a per-frame
/// color preview doesn't change node text, borders, or positions,
/// so the node-tree rebuild is wasted work. Halves the hot-path
/// cost vs `rebuild_all` on maps with many nodes.
///
/// Threads the persistent `SceneConnectionCache` so unchanged edge
/// geometry (`sample_path` samples) is reused. This matches what
/// the drag drains (`MovingNode`, `EdgeHandle`, `EdgeLabel`,
/// `PortalLabel`) already do — every throttled consumer that
/// reaches this helper inherits the same optimization.
pub(in crate::application::app) fn rebuild_scene_only(
    doc: &MindMapDocument,
    interaction_mode: &super::InteractionMode,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    let offsets = std::collections::HashMap::new();
    let frame = CanvasFrame::new(
        doc,
        &offsets,
        interaction_mode.resize_handle_overrides(),
        renderer.camera_zoom(),
    );
    frame.update_all(scene_cache, app_scene);
    flush_canvas_scene_buffers(app_scene, renderer);
}

/// Re-project the **zoom-dependent** canvas roles after a camera
/// change (pan / zoom / surface resize).
///
/// Connections, their labels, portals and edge handles all size and
/// position against the current camera; borders, section frames and
/// resize handles are canvas-space and zoom-independent, so
/// re-projecting them here would be pure waste on every scroll tick
/// (§4's mobile budget). That is the whole difference from
/// [`rebuild_scene_only`], which does all of them.
///
/// The connection sample cache is cleared first: effective font size
/// depends on zoom, so every cached pre-clip sample is stale.
/// `ensure_zoom` inside the connection pass would also catch it; the
/// explicit clear keeps the ordering readable next to the rebuild.
///
/// Both targets reach this. Native calls it from
/// `drain_frame::drain_camera_geometry_rebuild` once per frame under
/// the renderer's dirty flag; WASM has no per-frame drain and calls
/// it from the wheel handler under the same flag.
pub(in crate::application::app) fn rebuild_camera_geometry(
    doc: &MindMapDocument,
    interaction_mode: &super::InteractionMode,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    scene_cache.clear();
    let offsets = std::collections::HashMap::new();
    let frame = CanvasFrame::new(
        doc,
        &offsets,
        interaction_mode.resize_handle_overrides(),
        renderer.camera_zoom(),
    );
    frame.update_connection_trees(scene_cache, app_scene);
    frame.update_connection_label_tree(app_scene);
    frame.update_portal_tree(app_scene);
    flush_canvas_scene_buffers(app_scene, renderer);
}

/// Project every canvas role once at load, and warm the handle-tree
/// allocators.
///
/// Both runtimes ran this as a line-for-line copy in their init
/// paths, differing only in local variable names. It is the one
/// startup body where a divergence would be invisible — the map still
/// draws either way; only the first interaction gets slower on
/// whichever target fell behind.
///
/// Three things happen, in this order:
///
/// 1. **Project every role.** Populates the per-edge Bezier-sample
///    cache so first interactions don't pay the full cost. Reads
///    `renderer.camera_zoom()`, so the caller must have settled the
///    camera (`fit_camera_to_tree`) first — otherwise connection
///    glyphs size against the default-init zoom rather than the
///    real one. Init runs before any interaction, so the mode is
///    `Default` and no resize handles emit; the explicit
///    [`InteractionModeOverrides::none`] makes the warm projection
///    match the first post-init frame's shape.
/// 2. **Register the handle-tree roles** with their fresh-load
///    (empty-slice) signatures, then warm their arenas with
///    synthetic 8-element slices. The first real selection still
///    takes a full rebuild — its signature differs from the empty
///    one — but the cosmic-text `BufferLine` pools and arena bumpers
///    inside `build_handle_tree` are warm by then. Registration also
///    lets §B2 dispatch find the roles at all; without it the first
///    interaction would force a register-and-rebuild and the second
///    another rebuild.
/// 3. **Re-stamp the empty-handle signatures**, so the canvas state
///    at load-end is the empty-handles state rather than the
///    synthetic 8-handle one. The caller's later `rebuild_all` would
///    also do this, but re-stamping here means correctness does not
///    depend on that call — if a future change makes it conditional
///    or moves it, the canvas state stays well-defined.
///
/// [`InteractionModeOverrides::none`]:
///     crate::application::document::InteractionModeOverrides::none
pub(in crate::application::app) fn warm_scene_at_load(
    doc: &MindMapDocument,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    let init_offsets = std::collections::HashMap::new();
    let frame = CanvasFrame::new(
        doc,
        &init_offsets,
        crate::application::document::InteractionModeOverrides::none(),
        renderer.camera_zoom(),
    );
    frame.update_connection_trees(scene_cache, app_scene);
    frame.update_border_tree(app_scene);
    frame.update_portal_tree(app_scene);
    frame.update_connection_label_tree(app_scene);
    frame.update_section_frame_tree(app_scene);
    frame.update_section_resize_handle_tree(app_scene);
    frame.update_node_resize_handle_tree(app_scene);
    warm_handle_tree_arenas(app_scene);
    update_edge_handle_tree_from_slice(&[], app_scene);
    frame.update_section_resize_handle_tree(app_scene);
    frame.update_node_resize_handle_tree(app_scene);
    flush_canvas_scene_buffers(app_scene, renderer);
}

// =====================================================================
// Canvas-role projection.
//
// `CanvasFrame` owns the per-frame inputs every role shares and
// exposes one `update_*` per role. **None of them re-walk the
// registered trees into renderer buffers** — that's the caller's
// responsibility, via `flush_canvas_scene_buffers`. Folding the
// flush into each method would cost N tree walks per rebuild (one
// per role touched) when 1 suffices.
// =====================================================================

/// One rebuild's shared projection inputs, plus a method per
/// canvas role.
///
/// Every `update_*` on this type resolves its role's element data
/// from `(document, offsets, overrides, hidden)` and dispatches it
/// through `AppScene`'s signature check. Grouping them here is what
/// lets the expensive shared inputs — the fold-hidden set, the
/// assembled [`FrameOverrides`], and (for connections) the clip
/// AABBs — be computed once per frame instead of once per role,
/// while each call site still picks only the roles its interaction
/// can actually change.
///
/// The subsets matter: a scroll-wheel zoom moves connections,
/// labels, portals, and edge handles but cannot move a border or a
/// resize handle (both are canvas-space and zoom-independent), so
/// `drain_camera_geometry_rebuild` calls four methods rather than
/// [`Self::update_all`]. Under §4's mobile budget those skips are
/// the difference between a smooth zoom and a stuttering one on a
/// large map.
pub(in crate::application::app) struct CanvasFrame<'a> {
    doc: &'a MindMapDocument,
    /// In-flight drag deltas keyed by node id. Empty on every
    /// non-drag rebuild.
    offsets: &'a std::collections::HashMap<String, (f32, f32)>,
    overrides: FrameOverrides<'a>,
    /// `MindMap::fold_hidden_set` for this frame — borrowed from
    /// `doc.mindmap`, shared by every pass below.
    hidden: std::collections::HashSet<&'a str>,
    camera_zoom: f32,
}

impl<'a> CanvasFrame<'a> {
    /// Assemble the shared inputs for one rebuild.
    ///
    /// # Costs
    ///
    /// One `fold_hidden_set` walk (O(nodes × ancestor depth)) plus
    /// the borrow-only override assembly. Nothing is projected until
    /// an `update_*` method is called.
    pub(in crate::application::app) fn new(
        doc: &'a MindMapDocument,
        offsets: &'a std::collections::HashMap<String, (f32, f32)>,
        mode_overrides: InteractionModeOverrides<'a>,
        camera_zoom: f32,
    ) -> Self {
        Self {
            doc,
            offsets,
            overrides: doc.frame_overrides(mode_overrides),
            hidden: doc.mindmap.fold_hidden_set(),
            camera_zoom,
        }
    }

    /// Run the cache-aware connection pass once and dispatch its
    /// two outputs to the `Connections` and `EdgeHandles` canvas
    /// roles.
    ///
    /// The pass emits both because grab-handles are resolved from
    /// the same live, offset-applied endpoint geometry the samples
    /// are — splitting them would mean walking `map.edges` twice
    /// and re-deriving the selected edge's anchors.
    ///
    /// **§B2 dispatch.** Selection toggle, color preview, and theme
    /// switches change only per-glyph fields (color regions, body
    /// glyph) without altering the per-edge structural shape (cap
    /// presence, body-glyph count). For those calls we take the
    /// in-place mutator path. Endpoint drag resamples the path and
    /// the body-glyph count typically shifts every few pixels — the
    /// identity sequence drops the equality and we fall back to a
    /// full rebuild. Handles dispatch the same way against
    /// `handle_identity_sequence`: a steady drag keeps the
    /// kind-derived channel set constant and reuses the arena; a
    /// midpoint drag that spawns a control point shifts it and
    /// forces a rebuild.
    pub(in crate::application::app) fn update_connection_trees(
        &self,
        cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
        app_scene: &mut crate::application::scene_host::AppScene,
    ) {
        use crate::application::scene_host::{hash_canvas_signature, CanvasDispatch, CanvasRole};
        use baumhard::mindmap::tree_builder::{
            build_connection_elements, build_connection_mutator_tree, build_connection_tree,
            connection_identity_sequence, node_clip_aabbs,
        };

        // Sample spacing is a function of the effective font size,
        // which is a function of zoom — flush stale samples before
        // the pass reads them.
        cache.ensure_zoom(self.camera_zoom);
        let node_aabbs = node_clip_aabbs(
            &self.doc.mindmap,
            self.offsets,
            self.overrides.border,
            &self.hidden,
        );
        let (connections, edge_handles) = build_connection_elements(
            &self.doc.mindmap,
            self.offsets,
            &node_aabbs,
            self.overrides.selection.edge,
            self.overrides.edge_color,
            cache,
            self.camera_zoom,
            &self.hidden,
        );

        let signature = hash_canvas_signature(&connection_identity_sequence(&connections));
        match app_scene.canvas_dispatch(CanvasRole::Connections, signature) {
            CanvasDispatch::InPlaceMutator => {
                let mutator = build_connection_mutator_tree(&connections);
                app_scene.apply_canvas_mutator(CanvasRole::Connections, &mutator);
            }
            CanvasDispatch::FullRebuild => {
                let tree = build_connection_tree(&connections);
                app_scene.register_canvas(CanvasRole::Connections, tree, glam::Vec2::ZERO);
                app_scene.set_canvas_signature(CanvasRole::Connections, signature);
            }
        }
        update_edge_handle_tree_from_slice(&edge_handles, app_scene);
    }

    /// Build or in-place update the border tree under
    /// [`crate::application::scene_host::CanvasRole::Borders`].
    ///
    /// **§B2 dispatch.** The hot path this closes: when the color
    /// picker is open, every throttled `AboutToWait` drain calls
    /// `rebuild_scene_only`, which runs this. Pre-dispatch, that
    /// meant a fresh `Tree<GfxElement, GfxMutator>` allocation per
    /// picker-hover frame plus a full canvas-scene buffer re-shape
    /// — O(n_borders × per-glyph shape cost). With the
    /// identity-sequence dispatch below, hover takes the in-place
    /// mutator path (which walks the same per-node Void + 4 runs but
    /// only overwrites variable fields) and the arena is reused.
    ///
    /// Structural identity: the sorted sequence of bordered
    /// (non-folded, rectangular, `show_frame = true` or
    /// force-shown by preview) node IDs. Drag, text-edit,
    /// color-preview, `NodeEdit` dimming, and preset-swap all leave
    /// this stable. Adding / removing a framed node, folding an
    /// ancestor, or toggling `show_frame` shifts the sequence and
    /// the dispatcher takes the full rebuild.
    pub(in crate::application::app) fn update_border_tree(
        &self,
        app_scene: &mut crate::application::scene_host::AppScene,
    ) {
        use crate::application::scene_host::{hash_canvas_signature, CanvasDispatch, CanvasRole};
        use baumhard::mindmap::tree_builder::{
            border_identity_sequence, border_node_data, build_border_mutator_tree_from_nodes,
            build_border_tree_from_nodes, BorderChromeOverrides,
        };

        let nodes = border_node_data(
            &self.doc.mindmap,
            self.offsets,
            BorderChromeOverrides {
                preview: self.overrides.border,
                node_edit_for: self.overrides.selection.node_edit_for,
            },
            &self.hidden,
        );
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
    /// Re-stamps the role's
    /// [`PortalHitIndex`](baumhard::mindmap::tree_builder::PortalHitIndex)
    /// alongside the tree so
    /// [`AppScene::portal_at`](crate::application::scene_host::AppScene::portal_at)
    /// can name a BVH hit. The index holds identity only; the
    /// clickable rectangles are the tree's own `GlyphArea`s.
    ///
    /// **§B2 dispatch.** Drag, color-preview, portal-text edits, and
    /// selection toggle all leave the visible-portal *identity
    /// sequence* unchanged — the same pairs in the same order, only
    /// their positions / colors / regions move. For those continuous
    /// interactions we take the in-place mutator path, which reuses
    /// the existing tree arena instead of allocating a new one each
    /// frame. When portals are added, removed, or a fold
    /// reveals/hides an endpoint, the identity sequence shifts and
    /// we fall back to a full rebuild.
    pub(in crate::application::app) fn update_portal_tree(
        &self,
        app_scene: &mut crate::application::scene_host::AppScene,
    ) {
        use crate::application::scene_host::{hash_canvas_signature, CanvasDispatch, CanvasRole};
        use baumhard::mindmap::tree_builder::{
            build_portal_mutator_tree_from_pairs, build_portal_tree_from_pairs, portal_identity_sequence,
            portal_pair_data,
        };

        let portal_text_edit = self.doc.portal_text_edit_preview.as_ref().map(
            |(key, endpoint, buffer)| baumhard::mindmap::tree_builder::PortalTextEditOverride {
                edge_key: key,
                endpoint_node_id: endpoint.as_str(),
                buffer: buffer.as_str(),
            },
        );

        let pairs = portal_pair_data(
            &self.doc.mindmap,
            self.offsets,
            self.overrides.selection.edge,
            self.overrides.selection.portal_label,
            self.overrides.portal_color,
            portal_text_edit,
            self.camera_zoom,
            &self.hidden,
        );
        let signature = hash_canvas_signature(&portal_identity_sequence(&pairs));

        match app_scene.canvas_dispatch(CanvasRole::Portals, signature) {
            CanvasDispatch::InPlaceMutator => {
                let result = build_portal_mutator_tree_from_pairs(&pairs);
                app_scene.set_portal_hit_index(result.hit_index);
                app_scene.apply_canvas_mutator(CanvasRole::Portals, &result.mutator);
            }
            CanvasDispatch::FullRebuild => {
                let result = build_portal_tree_from_pairs(&pairs);
                app_scene.set_portal_hit_index(result.hit_index);
                app_scene.register_canvas(CanvasRole::Portals, result.tree, glam::Vec2::ZERO);
                app_scene.set_canvas_signature(CanvasRole::Portals, signature);
            }
        }
    }

    /// Build or in-place update the connection-label tree under
    /// [`crate::application::scene_host::CanvasRole::ConnectionLabels`].
    /// Re-stamps the role's
    /// [`ConnectionLabelHitIndex`](baumhard::mindmap::tree_builder::ConnectionLabelHitIndex)
    /// alongside the tree so
    /// [`AppScene::edge_label_at`](crate::application::scene_host::AppScene::edge_label_at)
    /// can name a BVH hit.
    ///
    /// **§B2 dispatch.** Inline label edits (the hot path), color
    /// changes, and label movement keep the structural identity (the
    /// per-edge `EdgeKey` sequence) stable; the in-place mutator path
    /// runs and the arena is reused. Adding or removing a label, or
    /// selection-edge reorderings, change the identity and trigger a
    /// full rebuild.
    pub(in crate::application::app) fn update_connection_label_tree(
        &self,
        app_scene: &mut crate::application::scene_host::AppScene,
    ) {
        use crate::application::scene_host::{hash_canvas_signature, CanvasDispatch, CanvasRole};
        use baumhard::mindmap::scene_cache::EdgeKey;
        use baumhard::mindmap::tree_builder::{
            build_connection_label_mutator_tree, build_connection_label_tree, build_label_elements,
            connection_label_identity_sequence,
        };

        // A whole-edge selection paints the label too — "selected"
        // reads the same way on every sub-part of an edge — so the
        // label pass takes either the label sub-selection or the
        // whole-edge selection, whichever is set.
        let highlight_key: Option<EdgeKey> = self
            .overrides
            .selection
            .edge_label
            .clone()
            .or_else(|| {
                self.overrides
                    .selection
                    .edge
                    .map(|(f, t, ty)| EdgeKey::new(f, t, ty))
            });
        let elements = build_label_elements(
            &self.doc.mindmap,
            self.offsets,
            self.overrides.selection.label_edit,
            self.overrides.edge_color,
            highlight_key.as_ref(),
            self.camera_zoom,
            &self.hidden,
        );

        let signature = hash_canvas_signature(&connection_label_identity_sequence(&elements));
        match app_scene.canvas_dispatch(CanvasRole::ConnectionLabels, signature) {
            CanvasDispatch::InPlaceMutator => {
                let result = build_connection_label_mutator_tree(&elements);
                app_scene.set_connection_label_hit_index(result.hit_index);
                app_scene.apply_canvas_mutator(CanvasRole::ConnectionLabels, &result.mutator);
            }
            CanvasDispatch::FullRebuild => {
                let result = build_connection_label_tree(&elements);
                app_scene.set_connection_label_hit_index(result.hit_index);
                app_scene.register_canvas(CanvasRole::ConnectionLabels, result.tree, glam::Vec2::ZERO);
                app_scene.set_canvas_signature(CanvasRole::ConnectionLabels, signature);
            }
        }
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
    /// output, which captures `(node_id, section_idx, focused,
    /// resolved style axes, position, bounds, palette cycle)`. Any
    /// visible change — preset, pattern, corner, color, focus
    /// toggle, node move — moves the signature, so the dispatch
    /// triggers a full rebuild correctly.
    ///
    /// There's no §B2 in-place mutator path: section-frame style
    /// changes reshape the glyph runs entirely, and the focus toggle
    /// swaps preset glyph sets. A delta-style mutator would have to
    /// re-stamp every field of every run anyway, so `FullRebuild` is
    /// the only meaningful path. The matching `InPlaceMutator` arm
    /// short-circuits to a no-op: the registered tree is already
    /// correct, no work needed.
    pub(in crate::application::app) fn update_section_frame_tree(
        &self,
        app_scene: &mut crate::application::scene_host::AppScene,
    ) {
        use crate::application::scene_host::{CanvasDispatch, CanvasRole};
        use baumhard::mindmap::tree_builder::{
            build_section_frame_tree, build_section_frames, section_frame_identity_sequence,
        };

        let elements = build_section_frames(
            &self.doc.mindmap,
            self.offsets,
            self.overrides.selection.node_edit_for,
            self.overrides.selection.focused_section,
            self.overrides.border,
            &self.hidden,
        );
        let signature = section_frame_identity_sequence(&elements);
        match app_scene.canvas_dispatch(CanvasRole::SectionFrames, signature) {
            CanvasDispatch::InPlaceMutator => {
                // Signature matched the registered tree — nothing
                // changed since the last rebuild, so the registered
                // tree is already correct. Early-return saves the
                // tree-allocation + register_canvas slab churn that
                // would otherwise fire on every NodeEdit-mode rebuild.
            }
            CanvasDispatch::FullRebuild => {
                let tree = build_section_frame_tree(&elements);
                app_scene.register_canvas(CanvasRole::SectionFrames, tree, glam::Vec2::ZERO);
                app_scene.set_canvas_signature(CanvasRole::SectionFrames, signature);
            }
        }
    }

    /// Emit the selected node's eight resize handles (or none) and
    /// dispatch them to
    /// [`crate::application::scene_host::CanvasRole::NodeResizeHandles`].
    /// Selection-gated 0 ↔ 8 transitions take the full-rebuild arm;
    /// a steady drag stays on the in-place mutator arm.
    pub(in crate::application::app) fn update_node_resize_handle_tree(
        &self,
        app_scene: &mut crate::application::scene_host::AppScene,
    ) {
        let elements = baumhard::mindmap::tree_builder::build_selected_node_handles(
            &self.doc.mindmap,
            self.offsets,
            self.overrides.selection.selected_node_for_resize,
            &self.hidden,
        );
        update_node_resize_handle_tree_from_slice(&elements, app_scene);
    }

    /// Emit the selected section's eight resize handles (or none)
    /// and dispatch them to
    /// [`crate::application::scene_host::CanvasRole::SectionResizeHandles`].
    /// A `None`-sized (fill-parent) section emits zero handles —
    /// there is no per-section AABB to stretch.
    pub(in crate::application::app) fn update_section_resize_handle_tree(
        &self,
        app_scene: &mut crate::application::scene_host::AppScene,
    ) {
        let elements = baumhard::mindmap::tree_builder::build_selected_section_handles(
            &self.doc.mindmap,
            self.offsets,
            self.overrides.selection.selected_section,
            &self.hidden,
        );
        update_section_resize_handle_tree_from_slice(&elements, app_scene);
    }

    /// Refresh every canvas role. Callers that know their
    /// interaction can only move a subset should call the
    /// individual methods instead — see the type-level note.
    ///
    /// Does **not** flush the renderer's canvas-scene buffers:
    /// that's one walk over all the registered trees, so it belongs
    /// once at the end of the caller's batch (see
    /// [`flush_canvas_scene_buffers`]).
    pub(in crate::application::app) fn update_all(
        &self,
        cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
        app_scene: &mut crate::application::scene_host::AppScene,
    ) {
        self.update_connection_trees(cache, app_scene);
        self.update_border_tree(app_scene);
        self.update_portal_tree(app_scene);
        self.update_section_resize_handle_tree(app_scene);
        self.update_node_resize_handle_tree(app_scene);
        self.update_section_frame_tree(app_scene);
        self.update_connection_label_tree(app_scene);
    }
}

/// Dispatch a pre-built edge-handle slice to
/// [`crate::application::scene_host::CanvasRole::EdgeHandles`].
///
/// [`CanvasFrame::update_connection_trees`] is the normal caller —
/// the connection pass emits handles alongside the samples. This
/// slice form also serves the load-time pre-warm, which feeds
/// synthetic 8-handle data through the dispatch path so the
/// handle-tree builder's arena allocates from warm pools.
pub(in crate::application::app) fn update_edge_handle_tree_from_slice(
    elements: &[baumhard::mindmap::tree_builder::EdgeHandleElement],
    app_scene: &mut crate::application::scene_host::AppScene,
) {
    update_handle_canvas_role(
        crate::application::scene_host::CanvasRole::EdgeHandles,
        elements,
        app_scene,
    );
}

/// Dispatch a pre-built node-resize-handle slice to
/// [`crate::application::scene_host::CanvasRole::NodeResizeHandles`].
/// Used by the resize drain to refresh handle positions per-frame
/// against the in-progress AABB, which is not yet in the model and
/// so cannot come from [`CanvasFrame::update_node_resize_handle_tree`].
pub(in crate::application::app) fn update_node_resize_handle_tree_from_slice(
    elements: &[baumhard::mindmap::tree_builder::NodeResizeHandleElement],
    app_scene: &mut crate::application::scene_host::AppScene,
) {
    update_handle_canvas_role(
        crate::application::scene_host::CanvasRole::NodeResizeHandles,
        elements,
        app_scene,
    );
}

/// Dispatch a pre-built section-resize-handle slice to
/// [`crate::application::scene_host::CanvasRole::SectionResizeHandles`].
/// Sibling of [`update_node_resize_handle_tree_from_slice`], same
/// in-progress-AABB rationale.
pub(in crate::application::app) fn update_section_resize_handle_tree_from_slice(
    elements: &[baumhard::mindmap::tree_builder::SectionResizeHandleElement],
    app_scene: &mut crate::application::scene_host::AppScene,
) {
    update_handle_canvas_role(
        crate::application::scene_host::CanvasRole::SectionResizeHandles,
        elements,
        app_scene,
    );
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
/// pass with the live element slices) so the active canvas state
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
    use baumhard::mindmap::tree_builder::{
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
        assert!(!line.contains("section ["), "single-section must skip the [N of M] suffix; got {:?}", line);
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
            target: ResizeTarget::Section { node_id: id.clone(), section_idx: 1 },
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
        assert!(!line.contains("1 sources"), "singular form must not pluralize: {:?}", line);
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

    // ── build_overlaid_tree: the single render-overlay funnel ────
    //
    // Four call sites rebuild the node tree, and before this helper
    // two of them omitted the `NodeEdit` dim. The helper is pure
    // over `(doc, mode, highlights)` — no `Renderer`, no `AppScene`
    // — precisely so the composition can be pinned here instead of
    // being verifiable only by reading four call sites.

    use crate::application::document::tests_common::load_test_doc;
    use crate::application::document::HIGHLIGHT_COLOR as CYAN;

    /// Alpha of the first region on a node's first section area.
    fn first_alpha(tree: &baumhard::mindmap::tree_builder::MindMapTree, mind_id: &str) -> f32 {
        let sid = tree.section_arena_id(mind_id, 0).expect("section area");
        tree.tree.arena.get(sid).unwrap().get().glyph_area().unwrap().regions.all_regions()[0]
            .color
            .unwrap()[3]
    }

    fn some_other_node_than(doc: &crate::application::document::MindMapDocument, id: &str) -> String {
        doc.mindmap
            .nodes
            .keys()
            .find(|k| k.as_str() != id)
            .expect("fixture has more than one node")
            .clone()
    }

    /// Default mode: no dim, and the highlighted node gets the tint.
    /// The baseline the other two cases move against.
    #[test]
    fn test_build_overlaid_tree_default_mode_only_highlights() {
        let doc = load_test_doc();
        let other = some_other_node_than(&doc, "0");
        let tree = super::build_overlaid_tree(
            &doc,
            &crate::application::app::InteractionMode::Default,
            std::iter::once(("0", None, CYAN)),
        );
        assert!(
            (first_alpha(&tree, &other) - 1.0).abs() < 1e-3,
            "Default mode must not dim anything"
        );
        assert!((first_alpha(&tree, "0") - CYAN[3]).abs() < 1e-3);
    }

    /// NodeEdit: every node but the active one dims — the property
    /// `rebuild_selection_highlight` and `drain_selecting_rect`
    /// used to drop, leaving text at full opacity over a
    /// half-alpha border for the duration of a drag.
    #[test]
    fn test_build_overlaid_tree_node_edit_dims_inactive_nodes() {
        let doc = load_test_doc();
        let other = some_other_node_than(&doc, "0");
        let before = first_alpha(&super::build_overlaid_tree(
            &doc,
            &crate::application::app::InteractionMode::Default,
            std::iter::empty(),
        ), &other);
        let tree = super::build_overlaid_tree(
            &doc,
            &crate::application::app::InteractionMode::NodeEdit { node_id: "0".into() },
            std::iter::empty(),
        );
        assert!(
            (first_alpha(&tree, "0") - before).abs() < 1e-3,
            "the active NodeEdit node keeps full alpha"
        );
        assert!(
            (first_alpha(&tree, &other)
                - before * baumhard::mindmap::tree_builder::INACTIVE_NODE_ALPHA_MULTIPLIER)
                .abs()
                < 1e-2,
            "every other node dims"
        );
    }

    /// The split `overlay_tree` / `build_overlaid_tree` exists so
    /// `drain_selecting_rect` can hit-test a bare tree and then
    /// overlay it in place, instead of building the tree twice per
    /// rubber-band frame. That is only sound while the two produce
    /// the same result — pin it, so a future overlay added to one
    /// and not the other fails here rather than as a flicker during
    /// a drag.
    #[test]
    fn test_overlay_tree_in_place_matches_build_overlaid_tree() {
        let doc = load_test_doc();
        let other = some_other_node_than(&doc, "0");
        let mode = crate::application::app::InteractionMode::NodeEdit { node_id: "0".into() };

        let built = super::build_overlaid_tree(&doc, &mode, std::iter::once(("0", None, CYAN)));

        let mut in_place = doc.build_tree();
        super::overlay_tree(&mut in_place, &doc, &mode, std::iter::once(("0", None, CYAN)));

        for id in ["0", other.as_str()] {
            assert!(
                (first_alpha(&built, id) - first_alpha(&in_place, id)).abs() < 1e-6,
                "{id}: in-place overlay diverged from build-then-overlay"
            );
        }
    }

    /// Ordering: dim is a base layer, the selection tint is stamped
    /// on top of it. A node that is both inactive and selected must
    /// read as *selected*, at full highlight alpha.
    #[test]
    fn test_build_overlaid_tree_highlight_beats_the_dim() {
        let doc = load_test_doc();
        let other = some_other_node_than(&doc, "0");
        let tree = super::build_overlaid_tree(
            &doc,
            &crate::application::app::InteractionMode::NodeEdit { node_id: "0".into() },
            std::iter::once((other.as_str(), None, CYAN)),
        );
        assert!(
            (first_alpha(&tree, &other) - CYAN[3]).abs() < 1e-3,
            "an inactive-but-selected node reads as selected, not faded"
        );
    }
}
