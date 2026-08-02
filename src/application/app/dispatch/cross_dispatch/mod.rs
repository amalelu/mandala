// SPDX-License-Identifier: MPL-2.0

//! Cross-platform `Action` arm bodies, grouped by concept.
//!
//! Each function in this directory implements one or more
//! `Action::*` variant bodies in a form callable from BOTH the
//! native dispatcher ([`super::action_core`] +
//! `super::native::dispatch_action`) and the WASM dispatcher
//! (`super::super::run_wasm`). Both of those are plain
//! code-spans: each is cfg-gated to one target, and rustdoc
//! resolves links against the active target's module tree, so
//! either link breaks the other target's doc build.
//!
//! The split between native and WASM exists because the two
//! dispatchers carry different context types — native has 21 fields
//! including console / picker / interaction_mode / modifiers; WASM
//! has 9 fields, a strict subset. Arms whose bodies touch
//! only the shared subset live here; native-only arms stay in
//! `super::native`.
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
//!   window edits), pan nudges, center-on-selection, and
//!   jump-to-root.
//! - [`selection`]: selection-changing arms (`SelectAll`,
//!   `DeselectAll`, `InvertSelection`, `SelectParent`,
//!   `SelectChild`, `SelectNext/PrevSibling`) + their pure-doc
//!   inner functions.
//! - [`mod@pointer`]: the pointer-gesture payload
//!   ([`DispatchHit`]) and the double-click behavior — a pure
//!   route resolver plus the apply half that matches on it.
//!
//! `mod.rs` itself only carries the shared types
//! ([`RebuildContext`], [`DispatchOutcome`]), the two
//! cross-cutting helpers ([`apply_with_rebuild`],
//! [`apply_keybind_custom_mutation`]), and the re-exports that
//! keep the existing `super::cross_dispatch::apply_*` import
//! surface working unchanged.

use crate::application::document::MindMapDocument;
use crate::application::renderer::Renderer;
use crate::application::scene_host::AppScene;
use baumhard::mindmap::scene_cache::SceneConnectionCache;
use baumhard::mindmap::tree_builder::MindMapTree;

use super::super::scene_rebuild::rebuild_all;

mod camera;
mod edges;
mod fps;
mod lifecycle;
mod pointer;
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
pub(in crate::application::app) use pointer::*;
pub(in crate::application::app) use selection::*;
pub(in crate::application::app) use style::*;

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
        // Queues the envelope only. Advancing it is
        // `drain_frame::drain_animation_tick`, which is native-only —
        // so on WASM this branch starts an animation that never ticks.
        // A sanctioned carve-out, registered in CLAUDE.md's
        // "Dual-target status". This function is the keystroke tier's
        // entry (`run_wasm/event_keyboard.rs` on the browser side,
        // `native.rs` / `lifecycle.rs` on the desktop's).
        //
        // The **click-trigger** path does not come through here:
        // `click_triggers::fire_onclick_triggers` carries its own copy
        // of this animated-vs-instant routing and calls
        // `start_animation_at` (section-aware) instead. Same stall,
        // reached by a second body. Named pre-existing duplication,
        // not unified in this PR because the two copies genuinely
        // differ in the no-tree case — this one returns `false` and
        // skips `apply_document_actions`, the trigger loop applies
        // them anyway — so collapsing them is a behavior decision, not
        // a refactor. Whoever takes it owns picking which of the two
        // is right.
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
    pub renderer: &'a mut Renderer,
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
            self.renderer,
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
            self.renderer,
            self.scene_cache,
        );
    }
}

// ── RebuildContext construction macro ───────────────────────────

/// Build a [`RebuildContext`] from a context-like struct (native
/// `super::super::input_context::InputHandlerContext` (native-gated,
/// hence the plain code-span) or WASM
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
            renderer: $ctx.renderer,
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
