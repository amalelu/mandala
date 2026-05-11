// SPDX-License-Identifier: MPL-2.0

//! Cross-platform `Action` arm bodies, grouped by concept.
//!
//! Each function in this directory implements one or more
//! `Action::*` variant bodies in a form callable from BOTH the
//! native dispatcher ([`super::action_core`] +
//! [`super::native::dispatch_action`]) and the WASM dispatcher
//! (`super::super::run_wasm` — cfg-gated, hence the plain
//! code-span). The split between native and WASM exists because
//! the two dispatchers carry different context types — native has
//! 21 fields including console / picker / interaction_mode / modifiers;
//! WASM has 9 fields, a strict subset. Arms whose bodies touch
//! only the shared subset live here; native-only arms stay in
//! [`super::native`].
//!
//! This is the Track-C path documented in
//! `WASM_CONVERGENCE.md`: incrementally lift arm bodies as they
//! turn out to need only cross-platform state, without waiting
//! for a full context-type unification. Each migration removes
//! duplication and the "keep in sync" maintenance tax that
//! mirror arms (Path A1) carry.
//!
//! Helpers take a [`RebuildContext`] when they need the rebuild
//! plumbing, or just `&mut Renderer` for renderer-only operations.
//! Both dispatchers construct the right shape at the call site.
//!
//! ## Per-concept layout
//!
//! - [`lifecycle`]: undo + create / orphan / delete / edit on the
//!   current selection, plus the cross-platform clipboard arms.
//! - [`edges`]: anchor / body-glyph / cap / type / display-mode /
//!   reset edits and edge-label text / position edits.
//! - [`style`]: border / color / font / spacing edits applied to
//!   the current selection.
//! - [`fps`]: renderer-side FPS overlay toggles (no document
//!   mutation).
//! - [`camera`]: every zoom-related arm (camera-state zoom
//!   step / reset / fit-to-tree + per-element zoom-visibility
//!   window edits), pan nudges, centre-on-selection, and
//!   jump-to-root.
//! - [`selection`]: selection-changing arms (`SelectAll`,
//!   `DeselectAll`, `InvertSelection`, `SelectParent`,
//!   `SelectChild`, `SelectNext/PrevSibling`) + their pure-doc
//!   inner functions.
//!
//! `mod.rs` itself only carries the shared types
//! ([`RebuildContext`], [`DispatchOutcome`]), the two
//! cross-cutting helpers ([`apply_with_rebuild`],
//! [`apply_keybind_custom_mutation`]), and the re-exports that
//! keep the existing `super::cross_dispatch::apply_*` import
//! surface working unchanged.

use crate::application::common::{FpsDisplayMode, RenderDecree};
use crate::application::document::MindMapDocument;
use crate::application::renderer::Renderer;
use crate::application::scene_host::AppScene;
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::GfxMutator;
use baumhard::gfx_structs::tree::Tree;
use baumhard::mindmap::scene_cache::{EdgeKey, SceneConnectionCache};
use baumhard::mindmap::tree_builder::MindMapTree;
use glam::Vec2;
use std::collections::HashMap;

use super::super::scene_rebuild::rebuild_all;

mod camera;
mod edges;
mod fps;
mod lifecycle;
mod selection;
mod style;

// Re-exports so the existing `super::cross_dispatch::apply_*`
// import surface (used by `action_core.rs`, `native.rs`, the
// WASM dispatchers, and `tests_mutations` in `document/`) keeps
// working unchanged.
pub(in crate::application::app) use camera::*;
pub(in crate::application::app) use edges::*;
pub(in crate::application::app) use fps::*;
pub(in crate::application::app) use lifecycle::*;
pub(in crate::application::app) use selection::*;
pub(in crate::application::app) use style::*;

/// Abstract surface every cross-platform `Action` arm + the scene
/// rebuild plumbing uses on the rendering side. Native binds to
/// the real [`Renderer`]; headless binds to a no-op host with a
/// virtual camera so `dispatch_compatible` and `rebuild_all` work
/// against a GPU-less app.
///
/// **Scope.** Strictly the methods reached from
/// [`super::action_core::dispatch_compatible`], its helpers
/// ([`apply_zoom_step`], [`apply_pan_camera`],
/// [`apply_toggle_fps`], …), and
/// [`super::super::scene_rebuild::rebuild_all`]. Modal flows
/// (color picker, label editor, console) keep using `&mut Renderer`
/// directly because they're native-only and unreachable from
/// headless dispatch.
///
/// **Naming.** Camera reads are `&self`, camera writes are
/// `&mut self`. Each method mirrors a `Renderer` method 1:1 so
/// the impl is a thin forward; a future widening can absorb
/// additional methods without changing call sites.
pub(in crate::application::app) trait RebuildHost {
    // ── Camera reads ────────────────────────────────────────────
    fn camera_zoom(&self) -> f32;
    fn surface_width(&self) -> u32;
    fn surface_height(&self) -> u32;
    fn fps_display_mode(&self) -> FpsDisplayMode;

    // ── Camera writes ───────────────────────────────────────────
    fn process_decree(&mut self, decree: RenderDecree);
    fn set_camera_center(&mut self, target: Vec2);
    fn fit_camera_to_tree(&mut self, tree: &Tree<GfxElement, GfxMutator>);
    fn set_fps_display(&mut self, mode: FpsDisplayMode);

    // ── Rebuild plumbing ────────────────────────────────────────
    fn rebuild_buffers_from_tree(&mut self, tree: &Tree<GfxElement, GfxMutator>);
    fn rebuild_canvas_scene_buffers(&mut self, app_scene: &mut AppScene);
    fn set_mode_status_text(&mut self, text: Option<String>);
    fn set_portal_icon_hitboxes(
        &mut self,
        hitboxes: HashMap<(EdgeKey, String), (Vec2, Vec2)>,
    );
    fn set_portal_text_hitboxes(
        &mut self,
        hitboxes: HashMap<(EdgeKey, String), (Vec2, Vec2)>,
    );
    fn set_connection_label_hitboxes(&mut self, hitboxes: HashMap<EdgeKey, (Vec2, Vec2)>);

    // ── Geometry ────────────────────────────────────────────────
    fn screen_to_canvas(&self, screen_x: f32, screen_y: f32) -> Vec2;
    fn reshape_buffer_for(
        &mut self,
        arena_id: indextree::NodeId,
        tree: &Tree<GfxElement, GfxMutator>,
    );
}

impl RebuildHost for Renderer {
    fn camera_zoom(&self) -> f32 {
        Renderer::camera_zoom(self)
    }
    fn surface_width(&self) -> u32 {
        Renderer::surface_width(self)
    }
    fn surface_height(&self) -> u32 {
        Renderer::surface_height(self)
    }
    fn fps_display_mode(&self) -> FpsDisplayMode {
        Renderer::fps_display_mode(self)
    }
    fn process_decree(&mut self, decree: RenderDecree) {
        Renderer::process_decree(self, decree)
    }
    fn set_camera_center(&mut self, target: Vec2) {
        Renderer::set_camera_center(self, target)
    }
    fn fit_camera_to_tree(&mut self, tree: &Tree<GfxElement, GfxMutator>) {
        Renderer::fit_camera_to_tree(self, tree)
    }
    fn set_fps_display(&mut self, mode: FpsDisplayMode) {
        Renderer::set_fps_display(self, mode)
    }
    fn rebuild_buffers_from_tree(&mut self, tree: &Tree<GfxElement, GfxMutator>) {
        Renderer::rebuild_buffers_from_tree(self, tree)
    }
    fn rebuild_canvas_scene_buffers(&mut self, app_scene: &mut AppScene) {
        Renderer::rebuild_canvas_scene_buffers(self, app_scene)
    }
    fn set_mode_status_text(&mut self, text: Option<String>) {
        Renderer::set_mode_status_text(self, text)
    }
    fn set_portal_icon_hitboxes(
        &mut self,
        hitboxes: HashMap<(EdgeKey, String), (Vec2, Vec2)>,
    ) {
        Renderer::set_portal_icon_hitboxes(self, hitboxes)
    }
    fn set_portal_text_hitboxes(
        &mut self,
        hitboxes: HashMap<(EdgeKey, String), (Vec2, Vec2)>,
    ) {
        Renderer::set_portal_text_hitboxes(self, hitboxes)
    }
    fn set_connection_label_hitboxes(&mut self, hitboxes: HashMap<EdgeKey, (Vec2, Vec2)>) {
        Renderer::set_connection_label_hitboxes(self, hitboxes)
    }
    fn screen_to_canvas(&self, screen_x: f32, screen_y: f32) -> Vec2 {
        Renderer::screen_to_canvas(self, screen_x, screen_y)
    }
    fn reshape_buffer_for(
        &mut self,
        arena_id: indextree::NodeId,
        tree: &Tree<GfxElement, GfxMutator>,
    ) {
        Renderer::reshape_buffer_for(self, arena_id, tree)
    }
}

/// Pure inner helper for the keybind-triggered custom-mutation path.
/// Runs the same animation-aware apply + always-`apply_document_actions`
/// sequence the click-trigger path at `click.rs:35-64` uses, but
/// without touching the renderer. Returns `true` when the mutation
/// was applied.
///
/// Cross-platform: takes only `MindMapDocument`, `Option<MindMapTree>`,
/// `SceneConnectionCache`, `CustomMutation`, `node_id`, `now_ms`.
/// The caller is responsible for the post-apply scene rebuild
/// (`rebuild_after_geometry_change` for both targets). Lifted from
/// `dispatch.rs` (native-gated) for Track B so the WASM macro
/// dispatch path can reach the same animation-envelope contract
/// native uses.
///
/// `pub(crate)` because the parity tests in
/// `crate::application::document::tests_mutations` import it
/// (via the back-compat re-export in `super::dispatch`).
pub(crate) fn apply_keybind_custom_mutation(
    doc: &mut crate::application::document::MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
    cm: &baumhard::mindmap::custom_mutation::CustomMutation,
    node_id: &str,
    now_ms: u64,
) -> bool {
    if cm.timing.as_ref().is_some_and(|t| t.duration_ms > 0) {
        doc.start_animation(cm, node_id, now_ms);
    } else if let Some(tree) = mindmap_tree.as_mut() {
        doc.apply_custom_mutation(cm, node_id, Some(tree));
        scene_cache.clear();
    } else {
        // No tree available and no animation requested — nothing to apply.
        return false;
    }
    // Phase-7 parity: always invoke document actions, regardless of
    // whether the mutation animated or applied directly.
    doc.apply_document_actions(cm);
    true
}

/// Outcome of a `dispatch_action` call. The two variants let
/// callers branch on whether the dispatcher recognized and ran
/// the action — used by the keyboard handler to decide whether
/// to fall through to macro / custom-mutation lookup, and by the
/// mouse handler to decide whether the gesture consumed the
/// event.
///
/// Lives in `cross_dispatch` rather than `dispatch` because
/// `dispatch.rs` is `#[cfg(not(target_arch = "wasm32"))]`-gated
/// but the macro dispatch trait `MacroDispatchTarget` needs to
/// return `DispatchOutcome` from both targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::application::app) enum DispatchOutcome {
    /// The action was recognized and its body ran.
    Handled,
    /// The action's variant has no body in the current dispatcher
    /// (e.g. context-mismatched, or scaffolded ahead of its arm).
    /// Caller may fall through to lower-priority resolution.
    Unhandled,
}

/// Borrowed bundle of the shared rebuild plumbing — the minimum
/// surface every cross-platform mutating Action arm needs.
/// Constructed at the call site from whichever larger context
/// (`InputHandlerContext` on native, `WasmInputState` on WASM)
/// the dispatcher inherits.
pub(in crate::application::app) struct RebuildContext<'a> {
    pub document: &'a mut MindMapDocument,
    pub mindmap_tree: &'a mut Option<MindMapTree>,
    pub app_scene: &'a mut AppScene,
    pub host: &'a mut dyn RebuildHost,
    pub scene_cache: &'a mut SceneConnectionCache,
    /// Active interaction mode. Mutable because `EnterResizeMode` /
    /// `ExitMode` / `EnterReparentMode` etc. dispatch arms write
    /// through this field; read-only consumers reborrow as `&*` to
    /// pass to scene-rebuild helpers. Threaded through from the
    /// caller's `InputContextCore::interaction_mode`.
    pub interaction_mode: &'a mut super::super::InteractionMode,
}

impl<'a> RebuildContext<'a> {
    /// Trigger a full scene rebuild after a **geometry-changing**
    /// document mutation (border / color / font / spacing / edge
    /// type / etc.). Clears the connection sample cache because
    /// edge geometry may have shifted, then rebuilds tree +
    /// app-scene + renderer buffers.
    ///
    /// Use [`Self::rebuild_after_selection_change`] instead when
    /// the only thing that changed is `doc.selection`. Selection
    /// changes don't move edges, so the cached `sample_path`
    /// samples remain valid; clearing the cache forces a
    /// thousand-edge re-sample on every keyboard navigation
    /// keystroke for nothing.
    pub fn rebuild_after_geometry_change(&mut self) {
        self.scene_cache.clear();
        rebuild_all(
            self.document,
            &*self.interaction_mode,
            self.mindmap_tree,
            self.app_scene,
            self.host,
            self.scene_cache,
        );
    }

    /// Trigger a scene rebuild after a **selection-only** mutation
    /// (`SelectAll`, `JumpToRoot`, `SelectParent`, etc.). Skips
    /// the connection-sample cache clear because edge geometry
    /// hasn't changed — the cache stays valid and per-edge
    /// `sample_path` work is reused on the rebuild. Saves a
    /// noticeable amount of work on dense maps where every key
    /// nav would otherwise force a full re-sample.
    pub fn rebuild_after_selection_change(&mut self) {
        rebuild_all(
            self.document,
            &*self.interaction_mode,
            self.mindmap_tree,
            self.app_scene,
            self.host,
            self.scene_cache,
        );
    }
}

// ── RebuildContext construction macro ───────────────────────────

/// Build a [`RebuildContext`] from a context-like struct (native
/// [`super::super::input_context::InputHandlerContext`] or WASM
/// `WasmInputState`) plus an already-unwrapped
/// `&mut MindMapDocument`. Expands inline so the borrow-checker
/// accepts the disjoint per-field borrows; a `fn rebuild_ctx(&mut
/// self, doc)` builder would conflict with the active `doc` borrow
/// the caller's `if let Some(doc) = ctx.document.as_mut()` already
/// holds (re-borrowing `*ctx` while `doc` is live).
///
/// Both dispatchers compress the 6-line struct literal at every
/// rebuilding arm into a single `rebuild_ctx!(ctx, doc)` call.
macro_rules! rebuild_ctx {
    ($ctx:expr, $doc:expr) => {
        $crate::application::app::dispatch::cross_dispatch::RebuildContext {
            document: $doc,
            mindmap_tree: $ctx.mindmap_tree,
            app_scene: $ctx.app_scene,
            host: $ctx.host,
            scene_cache: $ctx.scene_cache,
            interaction_mode: $ctx.interaction_mode,
        }
    };
}
pub(in crate::application::app) use rebuild_ctx;

// ── Generic apply-then-rebuild ──────────────────────────────────

/// Run `apply` against the document and trigger a scene rebuild
/// when it returns `true`. Wraps the canonical "call mutation
/// core, conditionally rebuild" shape every parametric `Action`
/// arm uses so both dispatchers can express it as a one-liner.
///
/// Same arm shape as e.g.
/// ```text
/// apply_with_rebuild(&mut rc, |doc|
///     apply_anchor_to_selection(doc, Some(from), Some(to))
/// );
/// ```
pub(in crate::application::app) fn apply_with_rebuild<F>(rc: &mut RebuildContext<'_>, apply: F)
where
    F: FnOnce(&mut MindMapDocument) -> bool,
{
    if apply(rc.document) {
        rc.rebuild_after_geometry_change();
    }
}
