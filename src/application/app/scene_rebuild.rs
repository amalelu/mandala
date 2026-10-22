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

use super::dispatch::cross_dispatch::RebuildContext;
use baumhard::mindmap::tree_builder::{FrameOverrides, InteractionModeOverrides};

/// Which of the two whole-canvas rebuild tiers a change owes,
/// **named rather than run**.
///
/// Same split `ReleaseRefresh` makes on the drag-release path
/// (`throttled_interaction/release.rs`, native-only, which is why
/// this is not an intra-doc link — the wasm32 doc leg would not
/// resolve it), and for the same reason: both
/// tiers need `&mut Renderer`, so a caller that *decides* a tier
/// cannot be exercised at the same time as one that *performs* it —
/// TEST_CONVENTIONS §T8 keeps live wgpu out of the harness. Handing
/// the decision back as a value is what lets a test ask "which tier
/// does a click that only moves the selection ask for?" without
/// standing up a device.
///
/// `ReleaseRefresh` is deliberately not reused for this: it carries a
/// third variant (`None`) that only a commit against a missing
/// document produces, and it runs through a `DrainContext`, which the
/// click path — it has no `ColorPickerState` — cannot build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::application::app) enum RebuildTier {
    /// [`rebuild_all`] — node tree plus every canvas role. Needed
    /// whenever node-tree text buffers are affected: a selection
    /// highlight to apply, shift, or clear, or a document mutation
    /// that could have changed anything at all.
    All,
    /// [`rebuild_scene_only`] — every canvas role, node tree reused.
    /// Enough when only the scene-level highlight cascade moves.
    SceneOnly,
}

impl RebuildTier {
    /// The tier a `prev` → `new` selection transition needs.
    ///
    /// [`Self::All`] when either side is a node-ish selection
    /// (node-tree highlights need to be applied, shifted, or
    /// cleared); [`Self::SceneOnly`] when both are edge-adjacent and
    /// only the scene-level highlight cascade moves.
    pub(in crate::application::app) fn for_selection_change(
        prev: &SelectionState,
        new: &SelectionState,
    ) -> Self {
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
        if touches_tree(prev) || touches_tree(new) {
            Self::All
        } else {
            Self::SceneOnly
        }
    }

    /// The tier a click owes, on both targets.
    ///
    /// A click does two things that can invalidate the canvas: it
    /// moves the selection, and — when it landed on a node carrying
    /// an `OnClick` binding — it fires custom mutations and document
    /// actions. The second is unbounded (a theme switch repaints
    /// everything), so a fired trigger forces [`Self::All`] and only
    /// a click that changed nothing but the selection gets to ask
    /// [`Self::for_selection_change`].
    ///
    /// Shared by `click::handle_click_core` and the browser's
    /// `run_wasm::event_mouse_click` release path, which had drifted
    /// in *opposite* directions: native rebuilt everything for every
    /// click outcome, and the browser ran the selection-delta tier
    /// even when a trigger had just mutated the document
    /// (CODE_CONVENTIONS §4 — the two are peers).
    pub(in crate::application::app) fn for_click(
        triggers_fired: bool,
        prev: &SelectionState,
        new: &SelectionState,
    ) -> Self {
        if triggers_fired {
            Self::All
        } else {
            Self::for_selection_change(prev, new)
        }
    }

    /// Run the tier against the live renderer, through the same
    /// [`RebuildContext`] bundle every other rebuild takes.
    ///
    /// `All` is `rebuild_all` without the connection-cache clear:
    /// a selection change moves no edge, so the cached samples stay
    /// valid (see `RebuildContext::rebuild_after_geometry_change`
    /// for the variant that clears them).
    pub(in crate::application::app) fn execute(self, rc: &mut RebuildContext<'_>) {
        match self {
            Self::All => rebuild_all(
                rc.document,
                &*rc.interaction_mode,
                rc.mindmap_tree,
                rc.app_scene,
                rc.renderer,
                rc.scene_cache,
            ),
            Self::SceneOnly => rebuild_scene_only(
                rc.document,
                &*rc.interaction_mode,
                rc.app_scene,
                rc.renderer,
                rc.scene_cache,
            ),
        }
    }
}

/// Post-selection-change rebuild with the right granularity —
/// [`RebuildTier::for_selection_change`] decided, then run.
///
/// Exists for every selection-change callsite that would
/// otherwise call `rebuild_all` unconditionally — under §4's
/// mobile budget a full rebuild on every edge-label / portal
/// click is wasted work on a large map. This helper makes the
/// right choice a one-liner so callers don't have to re-derive
/// the decision. Callers that need the tier as a *value* (because
/// something other than the selection also moved) reach for
/// `RebuildTier` directly instead.
pub(in crate::application::app) fn rebuild_after_selection_change(
    prev_selection: &SelectionState,
    rc: &mut RebuildContext<'_>,
) {
    RebuildTier::for_selection_change(prev_selection, &rc.document.selection).execute(rc);
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
/// **Six production sites build a tree and install it.** Three go
/// through here: [`rebuild_all`], `click::rebuild_all_with_mode`,
/// and `rebuild_selection_highlight` — a code span rather than a
/// link because that one is native-gated, and a link from this
/// cross-platform doc into a stripped item breaks the wasm32 doc leg
/// (CODE_CONVENTIONS §8). The last of those is also how
/// `drain_frame::drain_rect_select` repaints, so the rubber-band
/// preview inherits the overlay order rather than restating it.
/// Before this helper each open-coded its own subset — two silently
/// omitted the `NodeEdit` dim (a section drag mid-edit snapped every
/// other node's text back to full opacity while its border stayed
/// dimmed), and three omitted `reapply_active_toggles` (an active
/// toggle's visual vanished on any click-driven rebuild).
///
/// The other three install an **overlay-free** projection on
/// purpose, and each is safe for its own reason — which is why
/// "every site" above counts installs rather than callers of this
/// helper:
///
/// - `document::animations::tick`. `drain_animation_tick` calls
///   `rebuild_all` unconditionally on any advance, so the bare tree
///   exists for less than one frame and is superseded before it can
///   be drawn. That safety rests entirely on the call staying
///   unconditional; a future change that gates it must route the
///   animation rebuild through here.
/// - The two startup installers, `run_native_init` and
///   `run_wasm`'s init. At load there is no overlay to lose: the
///   selection is empty, the interaction mode is `Default`, and no
///   toggle is active — so the bare tree and the overlaid one are
///   the same tree. A future startup path that adopts a document
///   with any of those already set would have to come through here.
///
/// A seventh build, `console::commands::mutation`, is not an
/// installer: it builds a tree for the flat-apply path and discards
/// it, and the renderer rebuilds from the model on the next frame.
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

/// [`overlay_tree`] over a freshly-built tree — the shape every
/// overlay site wants today.
///
/// [`overlay_tree`] stays separate rather than being folded in
/// (CODE_CONVENTIONS §7 — preserve the seam): applying the overlays
/// to a tree that already exists is where a §B2 in-place consumer
/// attaches, and the rubber-band drain was one until it stopped
/// needing a bare tree to hit-test against.
///
/// # Costs
///
/// One `build_mindmap_tree` — benchmarked as
/// `section_tree_build_243_single_section` /
/// `section_tree_build_50_multi_section` and therefore hot-path by
/// §B7 — plus [`overlay_tree`]'s walks.
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
    let new_tree = build_overlaid_tree(doc, interaction_mode, highlight_entries_for(doc));
    renderer.rebuild_buffers_from_tree(&new_tree.tree);

    rebuild_scene_only(doc, interaction_mode, app_scene, renderer, scene_cache);
    renderer.set_mode_status_text(mode_status_line(interaction_mode, doc));

    *mindmap_tree = Some(new_tree);
}

/// Rebuild the node tree with the current highlights and install
/// it — the third tier, below [`rebuild_scene_only`]: node text
/// buffers only, no canvas roles, no mode-status line.
///
/// What a change to *which nodes are highlighted* actually needs,
/// and nothing more. Two callers: the threshold-cross demote that
/// narrows a `Section` selection to its owning node before a drag,
/// and the rubber-band drain, whose preview set is a highlight
/// change by construction.
///
/// Native-only because both of those are: the threshold cross and
/// the per-frame drain are `DragState` machinery, which the browser
/// has no counterpart for. The tier itself is not native by nature —
/// whichever browser-side gesture first needs a highlight-only
/// repaint takes the `cfg` off. See CLAUDE.md's "Dual-target status"
/// registry.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn rebuild_selection_highlight(
    doc: &MindMapDocument,
    interaction_mode: &super::InteractionMode,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &mut Renderer,
) {
    if mindmap_tree.is_none() {
        return;
    }
    // Every overlay, in the one correct order — see
    // [`build_overlaid_tree`]. `interaction_mode` is threaded in for
    // the `NodeEdit` dim; without it a section drag mid-edit snapped
    // every other node's text back to full opacity while its border
    // stayed dimmed.
    let new_tree = build_overlaid_tree(doc, interaction_mode, highlight_entries_for(doc));
    renderer.rebuild_buffers_from_tree(&new_tree.tree);
    *mindmap_tree = Some(new_tree);
}

/// The highlight entries every node-tree rebuild stamps: the
/// rubber-band preview when one is live, the selection otherwise.
///
/// The preview *replaces* rather than adds to the selection's
/// entries. A rubber-band release writes `SelectionState::from_ids`
/// over the selection outright — shift included, since shift is what
/// starts the gesture rather than what extends it — so leaving the
/// pre-drag selection painted underneath would show the user a set
/// that is about to be discarded. This is the rule the drain used to
/// apply by building a private tree for itself out of the preview
/// alone; stating it here is what lets *every* rebuild path
/// reproduce it, including the animation tick's, which used to wipe
/// the preview for a frame by rebuilding from `doc.selection`.
pub(in crate::application::app) fn highlight_entries_for(
    doc: &MindMapDocument,
) -> Vec<(&str, Option<usize>, [f32; 4])> {
    match doc.rect_select_preview() {
        Some(ids) => ids
            .iter()
            .map(|id| (id.as_str(), None, HIGHLIGHT_COLOR))
            .collect(),
        None => selection_highlight_entries(&doc.selection),
    }
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
///
/// Reached through [`highlight_entries_for`], which is where the
/// rubber-band preview gets to speak instead. Call this one
/// directly only when the selection is genuinely the subject.
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
    let mut frame = CanvasFrame::new(
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
    let mut frame = CanvasFrame::new(
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
    let mut frame = CanvasFrame::new(
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
    /// This frame's connection paths, built at most once per edge and
    /// shared by the three passes that want them: the connection
    /// sampler, the selected edge's grab-handles, and the label
    /// layout. No path is built until a pass asks (#36 item 6) —
    /// though the slot table itself is sized here, so a frame that
    /// builds no path still allocates one `Option` per edge.
    ///
    /// It lives on the frame because the frame is exactly its
    /// validity window — a path is a pure function of `doc.mindmap`
    /// and `offsets`, both of which this type holds by shared
    /// reference for as long as it exists.
    paths: baumhard::mindmap::tree_builder::EdgePathCache<'a>,
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
            paths: baumhard::mindmap::tree_builder::EdgePathCache::new(&doc.mindmap, offsets),
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
        &mut self,
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
        // Destructured so the shared `paths` memo can be mut-borrowed
        // beside the frame's other fields, which the pass reads.
        let Self {
            doc,
            offsets,
            overrides,
            hidden,
            paths,
            camera_zoom,
        } = self;
        let node_aabbs = node_clip_aabbs(&doc.mindmap, offsets, overrides.border, hidden);
        let (connections, edge_handles) = build_connection_elements(
            &doc.mindmap,
            offsets,
            &node_aabbs,
            overrides.selection.edge,
            overrides.edge_color,
            cache,
            paths,
            *camera_zoom,
            hidden,
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
    /// **§B2 dispatch, on two signatures.** The hot path this
    /// closes: when the color picker is open, every throttled
    /// `AboutToWait` drain calls `rebuild_scene_only`, which runs
    /// this. Pre-dispatch, that meant a fresh
    /// `Tree<GfxElement, GfxMutator>` allocation per picker-hover
    /// frame plus a full canvas-scene buffer re-shape —
    /// O(n_borders × per-glyph shape cost). With the structural
    /// dispatch below, hover takes the in-place mutator path (which
    /// walks the same per-node Void + 8 runs but only overwrites
    /// variable fields) and the arena is reused.
    ///
    /// Structural signature
    /// ([`border_structure_signature`](baumhard::mindmap::tree_builder::border_structure_signature)):
    /// the sorted sequence of bordered (non-folded, rectangular,
    /// `show_frame = true` or force-shown by preview) node IDs and
    /// their channels. Drag, text-edit, color-preview, `NodeEdit`
    /// dimming, and preset-swap all leave this stable. Adding /
    /// removing a framed node, folding an ancestor, or toggling
    /// `show_frame` shifts it and the dispatcher takes the full
    /// rebuild.
    ///
    /// Content signature
    /// ([`border_content_signature`](baumhard::mindmap::tree_builder::border_content_signature)):
    /// every field of every [`BorderNodeData`](baumhard::mindmap::tree_builder::BorderNodeData).
    /// A stable structure only says a mutator would *align*, not
    /// that it would *write* anything, and the great majority of
    /// rebuilds are triggered by something in another role — a
    /// click on an edge label re-runs every canvas role. On a
    /// content match the mutator is neither built nor applied, and
    /// the work that removes is visible in the diff: one
    /// `MutatorTree` arena, one
    /// [`border_run_specs`](baumhard::mindmap::border::border_run_specs)
    /// call per framed node — each of which takes the `FONT_SYSTEM`
    /// write guard — eight `GlyphArea` + `DeltaGlyphArea` pairs per
    /// framed node, and the walk that applies them.
    ///
    /// The data pass itself still runs unconditionally: the
    /// signature is computed *from* its output, so there is nothing
    /// cheaper to decide on. Section frames have the same shape.
    pub(in crate::application::app) fn update_border_tree(
        &self,
        app_scene: &mut crate::application::scene_host::AppScene,
    ) {
        use crate::application::scene_host::{CanvasContentDispatch, CanvasRole, CanvasSignatures};
        use baumhard::mindmap::tree_builder::{
            border_content_signature, border_node_data, border_structure_signature,
            build_border_mutator_tree_from_nodes, build_border_tree_from_nodes, BorderChromeOverrides,
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
        let signatures = CanvasSignatures {
            structure: border_structure_signature(&nodes),
            content: border_content_signature(&nodes),
        };

        match app_scene.canvas_dispatch_with_content(CanvasRole::Borders, &signatures) {
            CanvasContentDispatch::Skip => {
                // Every field the border tree is built from is
                // unchanged, so the registered tree already holds
                // what the mutator would have written.
            }
            CanvasContentDispatch::InPlaceMutator => {
                let mutator = build_border_mutator_tree_from_nodes(&nodes);
                app_scene.apply_canvas_mutator(CanvasRole::Borders, &mutator);
            }
            CanvasContentDispatch::FullRebuild => {
                let tree = build_border_tree_from_nodes(&nodes);
                app_scene.register_canvas(CanvasRole::Borders, tree, glam::Vec2::ZERO);
                app_scene.set_canvas_signature(CanvasRole::Borders, signatures.structure);
            }
        }
        // Outside the match on purpose: correct after all three
        // outcomes, so no arm can be the one that forgets and
        // leaves a later frame skipping against a stale value.
        app_scene.set_canvas_content_signature(CanvasRole::Borders, signatures.content);
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
    /// **§B2 dispatch, on two signatures.** Drag, color-preview,
    /// portal-text edits, and selection toggle all leave the
    /// visible-portal *structural signature* unchanged — the same
    /// pairs in the same order, only their positions / colors /
    /// regions move. For those continuous interactions we take the
    /// in-place mutator path, which reuses the existing tree arena
    /// instead of allocating a new one each frame. When portals are
    /// added, removed, or a fold reveals/hides an endpoint, the
    /// structural signature shifts and we fall back to a full
    /// rebuild.
    ///
    /// A stable structure only says a mutator would *align*, not
    /// that it would *write* anything — and every canvas role is
    /// re-run by any interaction, including ones that cannot touch
    /// a portal. So a second, finer
    /// [`portal_content_signature`](baumhard::mindmap::tree_builder::portal_content_signature)
    /// over every field of every
    /// [`PortalPairData`](baumhard::mindmap::tree_builder::PortalPairData)
    /// decides whether the in-place arm does anything at all. On a
    /// match the mutator is neither built nor applied and the hit
    /// index is not re-stamped — it is a pure function of the same
    /// pair data, so the stamped one already matches. The work that
    /// removes is visible in the diff: one `MutatorTree` arena,
    /// four `DeltaGlyphArea::full_assign_from` clones per visible
    /// pair, one [`PortalHitIndex`](baumhard::mindmap::tree_builder::PortalHitIndex)
    /// with five `String` clones per pair — an `EdgeKey`'s three
    /// plus the two endpoint ids — and the walk that applies the
    /// mutator.
    ///
    /// The data pass itself still runs unconditionally: the
    /// signatures are computed *from* its output, so there is
    /// nothing cheaper to decide on.
    pub(in crate::application::app) fn update_portal_tree(
        &self,
        app_scene: &mut crate::application::scene_host::AppScene,
    ) {
        use crate::application::scene_host::{CanvasContentDispatch, CanvasRole, CanvasSignatures};
        use baumhard::mindmap::tree_builder::{
            build_portal_mutator_tree_from_pairs, build_portal_tree_from_pairs, portal_content_signature,
            portal_pair_data, portal_structure_signature,
        };

        let portal_text_edit = self
            .doc
            .portal_text_edit_preview
            .as_ref()
            .map(
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
        let signatures = CanvasSignatures {
            structure: portal_structure_signature(&pairs),
            content: portal_content_signature(&pairs),
        };

        match app_scene.canvas_dispatch_with_content(CanvasRole::Portals, &signatures) {
            CanvasContentDispatch::Skip => {
                // Every field the portal tree and its hit index are
                // built from is unchanged, so both already hold what
                // this pass would have written.
            }
            CanvasContentDispatch::InPlaceMutator => {
                let result = build_portal_mutator_tree_from_pairs(&pairs);
                app_scene.set_portal_hit_index(result.hit_index);
                app_scene.apply_canvas_mutator(CanvasRole::Portals, &result.mutator);
            }
            CanvasContentDispatch::FullRebuild => {
                let result = build_portal_tree_from_pairs(&pairs);
                app_scene.set_portal_hit_index(result.hit_index);
                app_scene.register_canvas(CanvasRole::Portals, result.tree, glam::Vec2::ZERO);
                app_scene.set_canvas_signature(CanvasRole::Portals, signatures.structure);
            }
        }
        // Outside the match on purpose: correct after all three
        // outcomes, so no arm can be the one that forgets and
        // leaves a later frame skipping against a stale value.
        app_scene.set_canvas_content_signature(CanvasRole::Portals, signatures.content);
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
        &mut self,
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
        let highlight_key: Option<EdgeKey> = self.overrides.selection.edge_label.clone().or_else(|| {
            self.overrides
                .selection
                .edge
                .map(|(f, t, ty)| EdgeKey::new(f, t, ty))
        });
        let elements = build_label_elements(
            &self.doc.mindmap,
            self.overrides.selection.label_edit,
            self.overrides.edge_color,
            highlight_key.as_ref(),
            &mut self.paths,
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
    /// not gesture-driven. The dispatch keys on the
    /// [`baumhard::mindmap::tree_builder::section_frame_content_signature`]
    /// output, which captures `(node_id, section_idx, focused,
    /// resolved style axes, position, bounds, palette cycle)`. Any
    /// visible change — preset, pattern, corner, color, focus
    /// toggle, node move — moves the signature, so the dispatch
    /// triggers a full rebuild correctly. Borders and portals split
    /// that one question into two, because they have an in-place
    /// mutator arm to protect; this role does not, so one
    /// content-covering signature serves.
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
            build_section_frame_tree, build_section_frames, section_frame_content_signature,
        };

        let elements = build_section_frames(
            &self.doc.mindmap,
            self.offsets,
            self.overrides.selection.node_edit_for,
            self.overrides.selection.focused_section,
            self.overrides.border,
            &self.hidden,
        );
        let signature = section_frame_content_signature(&elements);
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
        &mut self,
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

/// Walk every canvas-scene tree once and bring the renderer's
/// canvas-scene shaped output back in step with it. Call this
/// **once** after a batch of `update_*_tree` invocations — calling
/// it inside each helper would multiply the per-frame *walk* by the
/// number of roles touched.
///
/// The walk is all it would multiply: the pass underneath re-shapes
/// only the elements whose shaping inputs moved, so the roles a
/// caller left alone cost the walk and the comparison and nothing
/// else. What that buys the callers of this function is that
/// projecting a subset of the roles now *stays* a subset all the way
/// to the GPU — before it, every one of them paid a full re-shape of
/// every registered role no matter how little it had changed.
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
    use baumhard::mindmap::scene_cache::EdgeKey;
    use baumhard::mindmap::tree_builder::{
        EdgeHandleElement, EdgeHandleKind, NodeResizeHandleElement, ResizeHandleSide,
        SectionResizeHandleElement,
    };

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
    use super::{highlight_entries_for, selection_highlight_entries, RebuildTier};
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
    fn test_selection_tier_edge_adjacent_to_edge_adjacent_is_scene_only() {
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
                assert_eq!(
                    RebuildTier::for_selection_change(prev, new),
                    RebuildTier::SceneOnly,
                    "{:?} -> {:?} should be scene-only",
                    prev,
                    new
                );
            }
        }
    }

    #[test]
    fn test_selection_tier_into_node_selection_needs_full_rebuild() {
        // Edge-adjacent -> Single / Multi: the new node must have
        // its highlight color region applied to its text buffer.
        let prev = SelectionState::EdgeLabel(EdgeLabelSel::new(edge_ref()));
        assert_eq!(
            RebuildTier::for_selection_change(&prev, &SelectionState::Single("n".into())),
            RebuildTier::All
        );
        assert_eq!(
            RebuildTier::for_selection_change(&prev, &SelectionState::Multi(vec!["a".into(), "b".into()])),
            RebuildTier::All
        );
    }

    #[test]
    fn test_selection_tier_out_of_node_selection_needs_full_rebuild() {
        // Single / Multi -> Edge-adjacent: the previous node's
        // highlight must be CLEARED from its text buffer. Scene-
        // only would leave the stale highlight stuck.
        for prev in [
            SelectionState::Single("n".into()),
            SelectionState::Multi(vec!["a".into(), "b".into()]),
        ] {
            for new in [
                SelectionState::Edge(edge_ref()),
                SelectionState::EdgeLabel(EdgeLabelSel::new(edge_ref())),
                SelectionState::PortalLabel(portal()),
                SelectionState::PortalText(portal()),
                SelectionState::None,
            ] {
                assert_eq!(
                    RebuildTier::for_selection_change(&prev, &new),
                    RebuildTier::All,
                    "{:?} -> {:?} must clear the node highlight",
                    prev,
                    new
                );
            }
        }
    }

    #[test]
    fn test_selection_tier_none_to_edge_adjacent_is_scene_only() {
        // A fresh click on an edge label when nothing was
        // selected: no tree highlight to clear, no new one to
        // apply. Scene-only is correct.
        for new in [
            SelectionState::Edge(edge_ref()),
            SelectionState::EdgeLabel(EdgeLabelSel::new(edge_ref())),
            SelectionState::PortalLabel(portal()),
            SelectionState::PortalText(portal()),
        ] {
            assert_eq!(
                RebuildTier::for_selection_change(&SelectionState::None, &new),
                RebuildTier::SceneOnly,
                "None -> {:?} should be scene-only",
                new
            );
        }
    }

    #[test]
    fn test_selection_tier_node_to_node_needs_full_rebuild() {
        // Node -> node: old highlight clears, new highlight
        // applies. Full rebuild in both directions.
        assert_eq!(
            RebuildTier::for_selection_change(
                &SelectionState::Single("a".into()),
                &SelectionState::Single("b".into())
            ),
            RebuildTier::All
        );
    }

    /// While a rubber-band drag is in flight its covered set is what
    /// every node-tree rebuild paints, and the pre-drag selection's
    /// highlight stands down — the release writes
    /// `SelectionState::from_ids` over the selection outright, so
    /// leaving it painted underneath would show the user a set that
    /// is about to be discarded.
    ///
    /// The fixture is chosen so the two answers cannot be confused:
    /// the selection names a node the preview does not, and the
    /// preview names two the selection does not.
    ///
    /// Fails when `highlight_entries_for` stops consulting
    /// `rect_select_preview` — which is how the preview used to be
    /// invisible to every rebuild but the rubber-band drain's own,
    /// so an animation tick landing mid-gesture wiped it for a
    /// frame.
    #[test]
    fn test_a_live_rubber_band_preview_replaces_the_selection_highlight() {
        let mut doc = crate::application::document::tests_common::load_test_doc();
        doc.selection = SelectionState::Single("selected-node".to_string());
        doc.set_rect_select_preview(vec!["covered-a".to_string(), "covered-b".to_string()]);
        let ids: Vec<&str> = highlight_entries_for(&doc)
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        assert_eq!(
            ids,
            vec!["covered-a", "covered-b"],
            "the covered set is what the user is reading"
        );

        doc.take_rect_select_preview();
        let ids: Vec<&str> = highlight_entries_for(&doc)
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        assert_eq!(
            ids,
            vec!["selected-node"],
            "and the selection comes back the instant the gesture ends"
        );
    }

    /// The preview paints whole nodes, never a section narrowing —
    /// `rect_select` answers in node ids and the release routes
    /// through `SelectionState::from_ids`, which cannot produce a
    /// `Section`. A `Some(idx)` here would tint one section of a
    /// covered node and leave its siblings plain.
    #[test]
    fn test_a_rubber_band_preview_entry_never_narrows_to_a_section() {
        let mut doc = crate::application::document::tests_common::load_test_doc();
        doc.selection = SelectionState::Section(crate::application::document::SectionSel {
            node_id: "selected-node".to_string(),
            section_idx: 3,
        });
        assert_eq!(
            selection_highlight_entries(&doc.selection)[0].1,
            Some(3),
            "precondition: the selection this replaces does narrow, or the row below \
             proves nothing"
        );
        doc.set_rect_select_preview(vec!["covered-a".to_string()]);
        assert_eq!(highlight_entries_for(&doc)[0].1, None);
    }

    /// A fired `OnClick` trigger forces the full tier no matter how
    /// small the selection delta is, on both targets. The trigger
    /// runs custom mutations and document actions — a theme switch
    /// repaints every node — so the selection delta says nothing
    /// about what changed.
    ///
    /// Fails on the input below when `for_click` stops consulting
    /// `triggers_fired`: the pair of edge-adjacent selections is one
    /// `for_selection_change` answers `SceneOnly` for, so the `All`
    /// here can only come from the flag. That is the browser's
    /// pre-fix shape — `run_wasm::event_mouse_click` fired triggers
    /// and then called `rebuild_after_selection_change` regardless.
    #[test]
    fn test_click_tier_is_full_when_a_trigger_fired_however_small_the_selection_delta() {
        let edge_adjacent = SelectionState::EdgeLabel(EdgeLabelSel::new(edge_ref()));
        assert_eq!(
            RebuildTier::for_selection_change(&edge_adjacent, &edge_adjacent),
            RebuildTier::SceneOnly,
            "precondition: this transition must be the cheap tier, or the row below proves nothing"
        );
        assert_eq!(
            RebuildTier::for_click(true, &edge_adjacent, &edge_adjacent),
            RebuildTier::All,
            "a fired trigger can have changed anything"
        );
    }

    /// The complement: with no trigger, a click is exactly its
    /// selection delta. Without this row the fix could be "answer
    /// `All` always", which is the native pre-fix shape — every
    /// click ran `rebuild_all`, including an edge-label -> edge-label
    /// selection change that cannot touch a node text buffer.
    #[test]
    fn test_click_tier_with_no_trigger_is_the_selection_delta_alone() {
        let from = SelectionState::EdgeLabel(EdgeLabelSel::new(edge_ref()));
        let to = SelectionState::PortalLabel(portal());
        assert_eq!(
            RebuildTier::for_click(false, &from, &to),
            RebuildTier::SceneOnly,
            "a click that only moves between edge-adjacent selections owes no node-tree rebuild"
        );
        assert_eq!(
            RebuildTier::for_click(false, &from, &SelectionState::Single("n".into())),
            RebuildTier::All,
            "...and one that lands on a node still owes one"
        );
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
        tree.tree
            .arena
            .get(sid)
            .unwrap()
            .get()
            .glyph_area()
            .unwrap()
            .regions
            .all_regions()[0]
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
    /// the rebuild sites used to drop before they shared one
    /// overlay body, leaving text at full opacity over a half-alpha
    /// border for the duration of a drag.
    #[test]
    fn test_build_overlaid_tree_node_edit_dims_inactive_nodes() {
        let doc = load_test_doc();
        let other = some_other_node_than(&doc, "0");
        let before = first_alpha(
            &super::build_overlaid_tree(
                &doc,
                &crate::application::app::InteractionMode::Default,
                std::iter::empty(),
            ),
            &other,
        );
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

    /// The split `overlay_tree` / `build_overlaid_tree` is the seam
    /// a §B2 in-place consumer attaches to: overlaying a tree that
    /// already exists, instead of building a fresh one to overlay.
    /// The seam is only worth anything while the two produce the
    /// same result — pin it, so a future overlay added to one and
    /// not the other fails here rather than as a flicker on screen.
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

// -----------------------------------------------------------------
// Canvas-pass granularity
//
// `flush_canvas_scene_buffers` used to re-shape every buffer of
// every registered canvas role, so a caller that re-projected one
// role still paid for all eight. It no longer does, and this is
// where that is asserted end to end: real roles projected by the
// real `CanvasFrame` off the canonical fixture, then the real pass
// run over them.
//
// The pass lives on `Renderer`, which no test may construct
// (§T8) — but it takes a `&Scene`, so it can be run here without
// one. That is what `renderer`'s `#[cfg(test)]` reach-through
// exists for.
// -----------------------------------------------------------------

#[cfg(test)]
mod canvas_pass_granularity {
    use crate::application::app::InteractionMode;
    use crate::application::document::{tests_common::load_test_doc, EdgeRef, MindMapDocument};
    use crate::application::renderer::{refresh, ScenePassKind, ShapedSceneElement};
    use crate::application::scene_host::{AppScene, CanvasRole};
    use baumhard::gfx_structs::scene::SceneTreeId;
    use baumhard::mindmap::scene_cache::SceneConnectionCache;
    use glam::Vec2;

    /// A labeled edge of the canonical fixture, as
    /// `(from_id, to_id, edge_type)`. That it still carries a label
    /// — and so still emits an element the label role can move — is
    /// asserted rather than assumed: `apply_edge_label_drag` returns
    /// `false` for an edge it could not move, and the test refuses
    /// to continue on that.
    const LABELED_EDGE: (&str, &str, &str) = ("1", "1.2", "parent_child");

    /// Project every canvas role, exactly as `rebuild_scene_only`
    /// does — the frame is rebuilt per call because it borrows the
    /// document.
    fn project_all_roles(doc: &MindMapDocument, app_scene: &mut AppScene, cache: &mut SceneConnectionCache) {
        let offsets = std::collections::HashMap::new();
        super::CanvasFrame::new(
            doc,
            &offsets,
            InteractionMode::Default.resize_handle_overrides(),
            1.0,
        )
        .update_all(cache, app_scene);
    }

    /// Walk positions each named role contributes, one `refresh` per
    /// role into a cache of its own.
    ///
    /// Used as a precondition, never as a claim. A canvas role with
    /// nothing to draw is still a registered tree, and its `Void`
    /// root is one walk position that shapes to nothing and matches
    /// `(None, None)` for free — so a "none of these re-shaped"
    /// assertion covering it is true of that role for a reason that
    /// has nothing to do with the reuse rule. Which roles carry a
    /// given aggregate zero is therefore part of what that zero
    /// means, and the aggregate cannot show it.
    fn walked_per_role(app_scene: &AppScene, roles: &[CanvasRole]) -> Vec<(CanvasRole, usize)> {
        roles
            .iter()
            .map(|role| {
                let id = app_scene
                    .canvas_id(*role)
                    .expect("every canvas role is registered by update_all");
                let mut shaped: Vec<ShapedSceneElement> = Vec::new();
                let counts = refresh(
                    app_scene.canvas_scene(),
                    &[id],
                    &mut shaped,
                    ScenePassKind::Canvas,
                );
                (*role, counts.walked)
            })
            .collect()
    }

    /// Project the connection-label role alone — what the edge-label
    /// drag drain does on every drained frame
    /// (`throttled_interaction/edge_label.rs`), because a label move
    /// cannot touch any other role.
    fn project_label_role_only(doc: &MindMapDocument, app_scene: &mut AppScene) {
        let offsets = std::collections::HashMap::new();
        super::CanvasFrame::new(
            doc,
            &offsets,
            InteractionMode::Default.resize_handle_overrides(),
            1.0,
        )
        .update_connection_label_tree(app_scene);
    }

    /// A drained frame of an edge-label drag re-shapes the label it
    /// moved, and nothing else on the canvas.
    ///
    /// The claim is asserted the way it is actually load-bearing —
    /// as a zero. The seven roles the drain does not re-project are
    /// walked into their own cache, and after the drag that pass
    /// re-shapes **none** of them; before the reuse rule it
    /// re-shaped every one, because the flush cleared and re-walked
    /// the whole canvas. Nothing here computes an expected value
    /// with the code under test: the zero is a zero, and the count
    /// on the label side is a literal that is meant to be re-read if
    /// the label projection ever changes what one move touches.
    ///
    /// The idempotence step before the drag is not ceremony. It is
    /// what makes the zero afterwards mean anything: if projecting
    /// the same document twice already produced differing elements,
    /// a later "nothing changed" would be measuring the fixture
    /// rather than the pass.
    #[test]
    fn test_edge_label_drain_reshapes_only_the_label_it_moved() {
        baumhard::font::fonts::init();

        let mut doc = load_test_doc();
        let mut app_scene = AppScene::new();
        let mut cache = SceneConnectionCache::new();
        project_all_roles(&doc, &mut app_scene, &mut cache);

        let all_ids = app_scene.canvas_ids_in_layer_order();
        let label_id = app_scene
            .canvas_id(CanvasRole::ConnectionLabels)
            .expect("the label role is registered by update_all");
        let other_ids: Vec<SceneTreeId> = all_ids.iter().copied().filter(|id| *id != label_id).collect();
        assert_eq!(
            other_ids.len(),
            all_ids.len() - 1,
            "exactly one of the registered roles is the label role"
        );
        assert_eq!(all_ids.len(), 8, "every canvas role is registered by update_all");

        // Three caches: the whole canvas as production keeps it, and
        // the two halves the claim is about.
        let mut all: Vec<ShapedSceneElement> = Vec::new();
        let mut labels: Vec<ShapedSceneElement> = Vec::new();
        let mut others: Vec<ShapedSceneElement> = Vec::new();
        let cold = refresh(
            app_scene.canvas_scene(),
            &all_ids,
            &mut all,
            ScenePassKind::Canvas,
        );
        assert_eq!(
            cold.reshaped, cold.walked,
            "an empty cache shapes everything it walks"
        );
        let cold_labels = refresh(
            app_scene.canvas_scene(),
            &[label_id],
            &mut labels,
            ScenePassKind::Canvas,
        );
        refresh(
            app_scene.canvas_scene(),
            &other_ids,
            &mut others,
            ScenePassKind::Canvas,
        );
        assert!(
            cold.walked > 10 * cold_labels.walked,
            "the label role must be a small part of the canvas, or \"only the labels\" says \
             nothing: {} label positions of {} walked",
            cold_labels.walked,
            cold.walked
        );

        // Precondition: projecting the same document twice is
        // element-for-element stable, so the zeroes below are the
        // pass's doing and not the fixture's.
        project_all_roles(&doc, &mut app_scene, &mut cache);
        let idempotent = refresh(
            app_scene.canvas_scene(),
            &all_ids,
            &mut all,
            ScenePassKind::Canvas,
        );
        assert_eq!(
            idempotent.reshaped, 0,
            "re-projecting an unchanged document must not re-shape a single element"
        );

        // The drag: one label moves, and only the label role is
        // re-projected — the two statements the drain makes.
        let edge_ref = EdgeRef::new(LABELED_EDGE.0, LABELED_EDGE.1, LABELED_EDGE.2);
        assert!(
            crate::application::app::edge_label_drag::apply_edge_label_drag(
                &mut doc,
                &edge_ref,
                Vec2::new(120.0, 260.0),
            ),
            "the fixture edge must actually accept the drag, or nothing below is being measured"
        );
        project_label_role_only(&doc, &mut app_scene);

        // What the zero below is worth, role by role. Two of the
        // seven carry it on this fixture and five hold a bare `Void`
        // root apiece — nothing is selected, so no handles emit, and
        // Default mode emits no section frames. Named rather than
        // left to the aggregate, which cannot show the difference.
        let untouched_roles = [
            CanvasRole::Connections,
            CanvasRole::Borders,
            CanvasRole::Portals,
            CanvasRole::EdgeHandles,
            CanvasRole::SectionFrames,
            CanvasRole::NodeResizeHandles,
            CanvasRole::SectionResizeHandles,
        ];
        let per_role = walked_per_role(&app_scene, &untouched_roles);
        // Identity, not cardinality, and identity *alone*: a length
        // check would let the literal swap one role for another and
        // go on describing a set the pass below does not walk, while
        // a dedup check next to this one could never fire on its own
        // — seven slots holding a repeat is seven slots missing a
        // role, which is a set difference this assertion already
        // sees.
        assert_eq!(
            untouched_roles
                .iter()
                .map(|role| {
                    app_scene
                        .canvas_id(*role)
                        .expect("every canvas role is registered by update_all")
                })
                .collect::<std::collections::HashSet<_>>(),
            other_ids
                .iter()
                .copied()
                .collect::<std::collections::HashSet<_>>(),
            "the roles named above must be exactly the ones the label role is not, so the \
             per-role counts describe the set the pass below walks: {untouched_roles:?}"
        );
        assert!(
            per_role.iter().filter(|(_, walked)| *walked > 1).count() >= 2,
            "at least two of the untouched roles must hold more than their `Void` root, or the \
             zero below is mostly a statement about empty trees: {per_role:?}"
        );

        let untouched = refresh(
            app_scene.canvas_scene(),
            &other_ids,
            &mut others,
            ScenePassKind::Canvas,
        );
        assert_eq!(
            untouched.reshaped, 0,
            "a label move re-projects no other role, so the other {} walked positions must \
             keep the buffers they already have",
            untouched.walked
        );

        let moved = refresh(
            app_scene.canvas_scene(),
            &[label_id],
            &mut labels,
            ScenePassKind::Canvas,
        );
        assert_eq!(
            moved.reshaped, 1,
            "one label moved, so one of the label role's {} walked positions may re-shape",
            moved.walked
        );

        // And the same, through the one cache production actually
        // keeps: the whole canvas walked, one element re-shaped.
        let production = refresh(
            app_scene.canvas_scene(),
            &all_ids,
            &mut all,
            ScenePassKind::Canvas,
        );
        assert_eq!(production.walked, cold.walked);
        assert_eq!(
            production.reshaped, 1,
            "the whole-canvas pass a drained frame runs re-shapes the moved label and nothing \
             else, out of {} walked positions",
            production.walked
        );
    }

    /// A camera move re-projects four of the eight roles, and the
    /// four it leaves alone keep every buffer they hold.
    ///
    /// `rebuild_camera_geometry` is the other production caller that
    /// deliberately narrows to a subset, and it narrows for a reason
    /// worth pinning: borders, section frames and the two
    /// resize-handle roles are canvas-space and zoom-independent.
    /// Before the reuse rule that narrowing stopped at the
    /// projection and the flush re-shaped all eight anyway.
    ///
    /// **What the fixture supplies is one of those four**, and the
    /// test says so out loud rather than letting the aggregate imply
    /// four roles' worth of coverage: at rest, with nothing selected
    /// and no `NodeEdit` mode, the section-frame role and both
    /// resize-handle roles hold a bare `Void` root each, so the zero
    /// below is carried by `Borders` and says only "an empty role
    /// stays empty" about the other three. The per-role precondition
    /// is what fires the day a fixture change empties `Borders` too.
    #[test]
    fn test_camera_geometry_rebuild_leaves_the_zoom_independent_roles_shaped() {
        baumhard::font::fonts::init();

        let doc = load_test_doc();
        let mut app_scene = AppScene::new();
        let mut cache = SceneConnectionCache::new();
        project_all_roles(&doc, &mut app_scene, &mut cache);

        let zoom_independent = [
            CanvasRole::Borders,
            CanvasRole::SectionFrames,
            CanvasRole::NodeResizeHandles,
            CanvasRole::SectionResizeHandles,
        ];
        let ids: Vec<SceneTreeId> = zoom_independent
            .iter()
            .map(|role| {
                app_scene
                    .canvas_id(*role)
                    .expect("every canvas role is registered by update_all")
            })
            .collect();

        // `EdgeHandles` belongs here: `update_connection_trees` ends
        // by dispatching the handle slice the connection pass emitted
        // alongside its samples, so a camera move re-projects it too
        // — which is what makes the count in the name four rather
        // than three. It walks only its `Void` root at rest, so it
        // contributes nothing to the assertion; listing it anyway is
        // what stops a later reader filing it under zoom-independent.
        let moved_roles = [
            CanvasRole::Connections,
            CanvasRole::ConnectionLabels,
            CanvasRole::Portals,
            CanvasRole::EdgeHandles,
        ];
        let moved_ids: Vec<SceneTreeId> = moved_roles
            .iter()
            .map(|role| {
                app_scene
                    .canvas_id(*role)
                    .expect("every canvas role is registered by update_all")
            })
            .collect();

        // Which of the four the fixture actually exercises. See the
        // docstring: one, and the test refuses to imply otherwise.
        let per_role = walked_per_role(&app_scene, &zoom_independent);
        assert!(
            per_role[0].1 > 50,
            "`Borders` is the role that carries this test's zero; if it empties, the assertion \
             below stops proving anything: {per_role:?}"
        );
        assert!(
            per_role[1..].iter().all(|(_, walked)| *walked == 1),
            "the other three zoom-independent roles are expected to hold only their `Void` root \
             at rest — if one of them has filled, this test now covers it and the docstring \
             needs re-reading: {per_role:?}"
        );

        let mut shaped: Vec<ShapedSceneElement> = Vec::new();
        let mut moved_shaped: Vec<ShapedSceneElement> = Vec::new();
        let cold = refresh(app_scene.canvas_scene(), &ids, &mut shaped, ScenePassKind::Canvas);
        refresh(
            app_scene.canvas_scene(),
            &moved_ids,
            &mut moved_shaped,
            ScenePassKind::Canvas,
        );
        assert!(
            cold.walked > 50,
            "the zoom-independent roles must carry real content, or the zero below is empty: \
             {} positions",
            cold.walked
        );

        // What `rebuild_camera_geometry` re-projects, at a zoom the
        // connection pass has to resample for.
        let offsets = std::collections::HashMap::new();
        cache.clear();
        let mut frame = super::CanvasFrame::new(
            &doc,
            &offsets,
            InteractionMode::Default.resize_handle_overrides(),
            2.5,
        );
        frame.update_connection_trees(&mut cache, &mut app_scene);
        frame.update_connection_label_tree(&mut app_scene);
        frame.update_portal_tree(&mut app_scene);

        // The zoom has to have actually moved something, or the zero
        // below is a camera change that never happened.
        let moved = refresh(
            app_scene.canvas_scene(),
            &moved_ids,
            &mut moved_shaped,
            ScenePassKind::Canvas,
        );
        assert!(
            moved.reshaped > 0,
            "re-projecting {moved_roles:?} at a different zoom must move something, or this \
             test is measuring a camera that stood still"
        );

        let after = refresh(app_scene.canvas_scene(), &ids, &mut shaped, ScenePassKind::Canvas);
        assert_eq!(
            (after.walked, after.reshaped),
            (cold.walked, 0),
            "the four zoom-independent roles were not re-projected, so none of them re-shapes"
        );
    }
}

// -----------------------------------------------------------------
// The border / portal roles' third dispatch outcome: do nothing.
//
// `border_content_signature` / `portal_content_signature` are
// tested in baumhard against the trees they guard
// (`tree_builder::tests::canvas_signature`). What those cannot see
// is whether this file actually consults them, so these tests
// observe the *effect* rather than the decision: they write a
// sentinel into the registered tree between two projections and
// ask whether it survived.
// -----------------------------------------------------------------

#[cfg(test)]
mod content_signature_dispatch {
    use std::collections::HashMap;

    use crate::application::app::InteractionMode;
    use crate::application::document::{tests_common::load_test_doc, MindMapDocument};
    use crate::application::scene_host::{AppScene, CanvasRole, CanvasSignatures};
    use baumhard::core::primitives::ApplyOperation;
    use baumhard::gfx_structs::area::{DeltaGlyphArea, GlyphAreaField};
    use baumhard::gfx_structs::mutator::{GfxMutator, Mutation};
    use baumhard::gfx_structs::tree::MutatorTree;

    /// Text no border run or portal marker can hold, written into
    /// the registered tree so a later projection either overwrites
    /// it (work happened) or does not (the skip fired).
    const SENTINEL: &str = "\u{2620}stale-sentinel\u{2620}";

    type Offsets = HashMap<String, (f32, f32)>;

    fn project_border(doc: &MindMapDocument, app_scene: &mut AppScene, offsets: &Offsets) {
        super::CanvasFrame::new(
            doc,
            offsets,
            InteractionMode::Default.resize_handle_overrides(),
            1.0,
        )
        .update_border_tree(app_scene);
    }

    fn project_portal(doc: &MindMapDocument, app_scene: &mut AppScene, offsets: &Offsets) {
        super::CanvasFrame::new(
            doc,
            offsets,
            InteractionMode::Default.resize_handle_overrides(),
            1.0,
        )
        .update_portal_tree(app_scene);
    }

    /// A mutator that assigns [`SENTINEL`] to the leaf at the given
    /// channel path under the root. Built by hand rather than
    /// through a role's own builder so it cannot be confused with
    /// the work the dispatcher does or does not do.
    fn sentinel_mutator(channel_path: &[usize]) -> MutatorTree<GfxMutator> {
        let (leaf_channel, void_channels) = channel_path
            .split_last()
            .expect("a sentinel path names at least the leaf");
        let mut mt: MutatorTree<GfxMutator> = MutatorTree::new_with(GfxMutator::new_void(0));
        let mut parent = mt.root;
        for channel in void_channels {
            parent = parent.append_value(GfxMutator::new_void(*channel), &mut mt.arena);
        }
        let delta = DeltaGlyphArea::new(vec![
            GlyphAreaField::Text(SENTINEL.to_string()),
            GlyphAreaField::Operation(ApplyOperation::Assign),
        ]);
        parent.append_value(
            GfxMutator::new(Mutation::AreaDelta(Box::new(delta)), *leaf_channel),
            &mut mt.arena,
        );
        mt
    }

    /// Whether any glyph area in a role's registered tree still
    /// carries the sentinel.
    fn sentinel_survives(app_scene: &AppScene, role: CanvasRole) -> bool {
        let id = app_scene
            .canvas_id(role)
            .expect("the role is registered by its first projection");
        let tree = app_scene
            .canvas_scene()
            .tree(id)
            .expect("a registered role has a tree");
        tree.root.descendants(&tree.arena).any(|node| {
            tree.arena
                .get(node)
                .and_then(|n| n.get().glyph_area())
                .is_some_and(|area| area.text == SENTINEL)
        })
    }

    /// A canvas-space node id whose drag offset moves the border
    /// tree: the first node the border data pass emits, taken from
    /// that pass rather than hardcoded so the fixture cannot drift
    /// out from under the test.
    fn first_framed_node(doc: &MindMapDocument) -> String {
        let hidden = doc.mindmap.fold_hidden_set();
        let nodes = baumhard::mindmap::tree_builder::border_node_data(
            &doc.mindmap,
            &HashMap::new(),
            baumhard::mindmap::tree_builder::BorderChromeOverrides::default(),
            &hidden,
        );
        nodes
            .first()
            .map(|n| n.node_id.clone())
            .expect("the canonical fixture must carry at least one framed node")
    }

    /// The canonical fixture carries **no** portal-mode edge, so
    /// the portal tests build their document by flipping one of its
    /// ordinary edges into portal mode.
    ///
    /// Written as a fixture step rather than left implicit because
    /// the first draft of these tests used `load_test_doc` directly
    /// and the precondition below caught it: an absent portal role
    /// would have made "nothing was written" true for a reason that
    /// has nothing to do with the dispatch.
    fn portal_doc() -> MindMapDocument {
        let mut doc = load_test_doc();
        let hidden: Vec<String> = doc
            .mindmap
            .fold_hidden_set()
            .iter()
            .map(|id| (*id).to_string())
            .collect();
        let nodes = &doc.mindmap.nodes;
        let flipped = doc
            .mindmap
            .edges
            .iter()
            .position(|e| {
                e.visible
                    && nodes.contains_key(&e.from_id)
                    && nodes.contains_key(&e.to_id)
                    && !hidden.contains(&e.from_id)
                    && !hidden.contains(&e.to_id)
            })
            .expect("the canonical fixture must carry one visible edge between two visible nodes");
        doc.mindmap.edges[flipped].display_mode =
            Some(baumhard::mindmap::model::DISPLAY_MODE_PORTAL.to_string());
        doc
    }

    /// The near-side node of the first visible portal pair, taken
    /// from the portal data pass for the same reason.
    fn first_portal_endpoint(doc: &MindMapDocument) -> String {
        let hidden = doc.mindmap.fold_hidden_set();
        let pairs = baumhard::mindmap::tree_builder::portal_pair_data(
            &doc.mindmap,
            &HashMap::new(),
            None,
            None,
            None,
            None,
            1.0,
            &hidden,
        );
        pairs
            .first()
            .map(|p| p.endpoints[0].endpoint_node_id.clone())
            .expect("the canonical fixture must carry at least one visible portal pair")
    }

    /// The pair of signatures the border role computes for a
    /// document, in the order `update_border_tree` computes them:
    /// `(structure, content)`.
    fn border_signatures(doc: &MindMapDocument, offsets: &Offsets) -> CanvasSignatures {
        use baumhard::mindmap::tree_builder::{
            border_content_signature, border_node_data, border_structure_signature, BorderChromeOverrides,
        };
        let nodes = border_node_data(
            &doc.mindmap,
            offsets,
            BorderChromeOverrides::default(),
            &doc.mindmap.fold_hidden_set(),
        );
        CanvasSignatures {
            structure: border_structure_signature(&nodes),
            content: border_content_signature(&nodes),
        }
    }

    /// The portal equivalent of [`border_signatures`].
    fn portal_signatures(doc: &MindMapDocument, offsets: &Offsets) -> CanvasSignatures {
        use baumhard::mindmap::tree_builder::{
            portal_content_signature, portal_pair_data, portal_structure_signature,
        };
        let pairs = portal_pair_data(
            &doc.mindmap,
            offsets,
            None,
            None,
            None,
            None,
            1.0,
            &doc.mindmap.fold_hidden_set(),
        );
        CanvasSignatures {
            structure: portal_structure_signature(&pairs),
            content: portal_content_signature(&pairs),
        }
    }

    /// **Which** signature the rebuild path hands to dispatch, read
    /// back from where it was recorded.
    ///
    /// This is the assertion the whole two-signature design rests
    /// on, and until it existed nothing in the suite could tell the
    /// design from its opposite. Collapsing the two — dispatching
    /// and recording on `content` for both roles — still renders
    /// correctly; it only sends every color-picker hover frame down
    /// the full-rebuild arm and reallocates the arena the in-place
    /// path exists to reuse. The whole suite stayed green under
    /// that revert.
    ///
    /// Read back through `canvas_signature` rather than through the
    /// tree handle: `Scene::insert` hands out reusable slab
    /// indices, so a `canvas_id` that did not change is not
    /// evidence that no rebuild happened.
    ///
    /// **Failing inputs, run rather than reasoned about.** Two
    /// redden this test: recording the wrong half —
    /// `set_canvas_signature(CanvasRole::Borders, signatures.content)`
    /// on the rebuild arm — and swapping the two fields where
    /// `CanvasSignatures` is built, which reaches the same record
    /// through the struct. The reviewed revert, which does both,
    /// reddens it too.
    ///
    /// **What it does *not* catch is worth naming, because the
    /// first version of this sentence named exactly that and was
    /// wrong.** Making the *dispatch* read the wrong field while
    /// the record stays right — `canvas_dispatch(role,
    /// signatures.content)` inside `canvas_dispatch_with_content`
    /// — leaves this test green: the stored value is still the
    /// structural signature, which is all this test reads. The
    /// skip simply stops firing, and
    /// [`test_border_dispatch_writes_nothing_when_no_input_moved`]
    /// is what sees that.
    ///
    /// So the two tests are a **pair**, and neither is redundant:
    /// this one guards what is *recorded*, its sibling guards what
    /// is *dispatched on*, and the four plants between them are
    /// covered only by both.
    #[test]
    fn test_border_dispatch_records_the_structural_signature_not_the_content_one() {
        baumhard::font::fonts::init();
        let doc = load_test_doc();
        let mut app_scene = AppScene::new();
        let offsets = Offsets::new();

        let signatures = border_signatures(&doc, &offsets);
        assert_ne!(
            signatures.structure, signatures.content,
            "the fixture must produce two different signatures, or 'the stored one is not \
             the content one' is true of any implementation"
        );

        project_border(&doc, &mut app_scene, &offsets);
        assert_eq!(
            app_scene.canvas_signature(CanvasRole::Borders),
            Some(signatures.structure),
            "the border role must dispatch and record on the structural signature — the \
             subset that says a mutator can align, not the one that says it has something \
             to write"
        );
        assert_ne!(
            app_scene.canvas_signature(CanvasRole::Borders),
            Some(signatures.content),
            "recording the content signature here is the collapse this design exists to \
             avoid: every color change would then take the full-rebuild arm"
        );

        // A drag moves content and leaves structure alone, so the
        // in-place arm runs. It must not overwrite the recorded
        // structural signature with anything either.
        let mut dragged = Offsets::new();
        dragged.insert(first_framed_node(&doc), (17.0, -9.0));
        let dragged_signatures = border_signatures(&doc, &dragged);
        assert_eq!(
            dragged_signatures.structure, signatures.structure,
            "a drag offset must leave the structural signature alone, or this step is \
             exercising the rebuild arm rather than the in-place one"
        );
        assert_ne!(
            dragged_signatures.content, signatures.content,
            "a drag offset must move the content signature"
        );

        project_border(&doc, &mut app_scene, &dragged);
        assert_eq!(
            app_scene.canvas_signature(CanvasRole::Borders),
            Some(signatures.structure),
            "the in-place arm must leave the recorded structural signature as it found it"
        );
    }

    /// The portal half of the same claim, with the same pairing:
    /// this test reads what was *recorded*, and
    /// [`test_portal_dispatch_writes_nothing_when_no_input_moved`]
    /// sees what was *dispatched on*.
    ///
    /// Failing input: record `signatures.content` on the rebuild
    /// arm of `update_portal_tree`, or swap the two fields where
    /// its `CanvasSignatures` is built.
    #[test]
    fn test_portal_dispatch_records_the_structural_signature_not_the_content_one() {
        baumhard::font::fonts::init();
        let doc = portal_doc();
        let mut app_scene = AppScene::new();
        let offsets = Offsets::new();

        let signatures = portal_signatures(&doc, &offsets);
        assert_ne!(
            signatures.structure, signatures.content,
            "the fixture must produce two different signatures, or 'the stored one is not \
             the content one' is true of any implementation"
        );

        project_portal(&doc, &mut app_scene, &offsets);
        assert_eq!(
            app_scene.canvas_signature(CanvasRole::Portals),
            Some(signatures.structure),
            "the portal role must dispatch and record on the structural signature"
        );
        assert_ne!(
            app_scene.canvas_signature(CanvasRole::Portals),
            Some(signatures.content),
            "recording the content signature here would send every portal color hover and \
             every inline-edit keystroke down the full-rebuild arm"
        );

        let mut dragged = Offsets::new();
        dragged.insert(first_portal_endpoint(&doc), (23.0, 11.0));
        let dragged_signatures = portal_signatures(&doc, &dragged);
        assert_eq!(
            dragged_signatures.structure, signatures.structure,
            "a drag offset must leave the structural signature alone, or this step is \
             exercising the rebuild arm rather than the in-place one"
        );
        assert_ne!(
            dragged_signatures.content, signatures.content,
            "a drag offset must move the content signature"
        );

        project_portal(&doc, &mut app_scene, &dragged);
        assert_eq!(
            app_scene.canvas_signature(CanvasRole::Portals),
            Some(signatures.structure),
            "the in-place arm must leave the recorded structural signature as it found it"
        );
    }

    /// A second projection with nothing changed writes nothing.
    ///
    /// The sentinel is applied to the registered border tree
    /// between the two projections; if `update_border_tree` still
    /// built and applied its mutator, the run's `Text::Assign`
    /// would overwrite it. It survives, so the mutator was never
    /// built.
    ///
    /// **Failing inputs.** Anything that stops the `Skip` arm
    /// firing: make `AppScene::canvas_dispatch_with_content` never
    /// return [`CanvasContentDispatch::Skip`](crate::application::scene_host::CanvasContentDispatch::Skip);
    /// have it ask its first question on `signatures.content`
    /// instead of `signatures.structure`, so the two comparisons
    /// collapse into one; or delete the
    /// `set_canvas_content_signature` call below the match, since
    /// nothing is then recorded to match against. Each overwrites
    /// the sentinel and fires this assertion.
    ///
    /// This test is the half of the pair that watches the
    /// *dispatch*; its sibling
    /// [`test_border_dispatch_records_the_structural_signature_not_the_content_one`]
    /// watches what gets *recorded*, and the middle plant above is
    /// invisible to that one.
    #[test]
    fn test_border_dispatch_writes_nothing_when_no_input_moved() {
        baumhard::font::fonts::init();
        let doc = load_test_doc();
        let mut app_scene = AppScene::new();
        let offsets = Offsets::new();

        project_border(&doc, &mut app_scene, &offsets);
        app_scene.apply_canvas_mutator(CanvasRole::Borders, &sentinel_mutator(&[1, 1]));
        assert!(
            sentinel_survives(&app_scene, CanvasRole::Borders),
            "the sentinel never reached the border tree, so what follows would be true \
             of an empty tree as much as of a skipped update"
        );

        project_border(&doc, &mut app_scene, &offsets);
        assert!(
            sentinel_survives(&app_scene, CanvasRole::Borders),
            "the border role rewrote its runs although every input it reads was unchanged"
        );
    }

    /// The skip is not a stall: move one input and the same
    /// projection writes again.
    ///
    /// A drag offset on a framed node moves `pos_x` / `pos_y` on
    /// that node's [`BorderNodeData`](baumhard::mindmap::tree_builder::BorderNodeData)
    /// while leaving the framed-node sequence alone, so this is the
    /// in-place arm doing its job. Which arm ran is not asserted
    /// here — that is what the structural-signature test in
    /// baumhard pins; what is asserted is that the update happened.
    ///
    /// Failing input: drop `pos_x` / `pos_y` from
    /// `border_content_signature`, and the second projection skips
    /// a frame that moved.
    #[test]
    fn test_border_dispatch_writes_again_when_a_drag_offset_moves() {
        baumhard::font::fonts::init();
        let doc = load_test_doc();
        let mut app_scene = AppScene::new();
        let dragged = first_framed_node(&doc);

        project_border(&doc, &mut app_scene, &Offsets::new());
        app_scene.apply_canvas_mutator(CanvasRole::Borders, &sentinel_mutator(&[1, 1]));
        assert!(
            sentinel_survives(&app_scene, CanvasRole::Borders),
            "the sentinel never reached the border tree"
        );

        let mut offsets = Offsets::new();
        offsets.insert(dragged, (17.0, -9.0));
        project_border(&doc, &mut app_scene, &offsets);
        assert!(
            !sentinel_survives(&app_scene, CanvasRole::Borders),
            "the border role skipped an update although a node had moved"
        );
    }

    /// The portal half of the same pair. The sentinel sits three
    /// levels down — pair void, endpoint void, icon slot — matching
    /// the portal tree's shape.
    ///
    /// Failing input: the same three, reaching the portal role —
    /// they all live in `canvas_dispatch_with_content`, which both
    /// roles share, except the `set_canvas_content_signature` call,
    /// which is per role.
    #[test]
    fn test_portal_dispatch_writes_nothing_when_no_input_moved() {
        baumhard::font::fonts::init();
        let doc = portal_doc();
        let mut app_scene = AppScene::new();
        let offsets = Offsets::new();

        project_portal(&doc, &mut app_scene, &offsets);
        app_scene.apply_canvas_mutator(CanvasRole::Portals, &sentinel_mutator(&[1, 1, 1]));
        assert!(
            sentinel_survives(&app_scene, CanvasRole::Portals),
            "the sentinel never reached the portal tree, so what follows would be true \
             of an empty tree as much as of a skipped update"
        );

        project_portal(&doc, &mut app_scene, &offsets);
        assert!(
            sentinel_survives(&app_scene, CanvasRole::Portals),
            "the portal role rewrote its markers although every input it reads was unchanged"
        );
    }

    /// The portal half of the moved-input pair.
    #[test]
    fn test_portal_dispatch_writes_again_when_a_drag_offset_moves() {
        baumhard::font::fonts::init();
        let doc = portal_doc();
        let mut app_scene = AppScene::new();
        let dragged = first_portal_endpoint(&doc);

        project_portal(&doc, &mut app_scene, &Offsets::new());
        app_scene.apply_canvas_mutator(CanvasRole::Portals, &sentinel_mutator(&[1, 1, 1]));
        assert!(
            sentinel_survives(&app_scene, CanvasRole::Portals),
            "the sentinel never reached the portal tree"
        );

        let mut offsets = Offsets::new();
        offsets.insert(dragged, (23.0, 11.0));
        project_portal(&doc, &mut app_scene, &offsets);
        assert!(
            !sentinel_survives(&app_scene, CanvasRole::Portals),
            "the portal role skipped an update although an endpoint had moved"
        );
    }
}
