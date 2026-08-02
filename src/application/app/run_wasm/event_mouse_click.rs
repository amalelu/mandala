// SPDX-License-Identifier: MPL-2.0

//! `WindowEvent::MouseInput` arm (left button only). Splits into
//! Pressed and Released paths via [`super::WasmApp::handle_mouse_input`]:
//!
//! - **Pressed** ([`super::WasmApp::handle_mouse_pressed`]): runs the cross-platform
//!   `compute_click_hit` priority chain (node > portal text > portal
//!   icon > edge label > empty), distinguishes single vs double
//!   click, commits double-click outcomes immediately, and otherwise
//!   stashes a [`super::PendingClick`] plus a `LastClick` snapshot
//!   for the eventual release.
//! - **Released** ([`super::WasmApp::handle_mouse_released`]): consumes the pending
//!   click — click-outside on an open editor commits the edit via
//!   the funnel, otherwise the pending tag becomes a fresh
//!   `SelectionState` and the scene is rebuilt through
//!   `rebuild_after_selection_change`.
//!
//! Mirrors the native shape at `event_mouse_click.rs`; the WASM
//! variant has no console / color-picker / drag-state intercepts so
//! the early-return ladder is shorter.

#![cfg(target_arch = "wasm32")]

use crate::application::platform::input::ElementState;

use super::PendingClick;
use crate::application::app::click_triggers::fire_onclick_triggers;
use crate::application::app::dispatch::DispatchOutcome;
use crate::application::app::scene_rebuild::rebuild_after_selection_change;
use crate::application::app::{
    compute_click_hit, dispatch, is_double_click, now_ms, ClickHitParts, LastClick,
};
use crate::application::document::{EdgeLabelSel, EdgeRef, PortalLabelSel, SectionSel, SelectionState};
use crate::application::keybinds::Action;
use std::sync::atomic::AtomicBool;

/// One-shot latch for the double-click input class. See
/// [`crate::application::app::warn_unhandled_native_only_once`]; per-class so a dead
/// double-click binding and a dead wheel binding each get their own
/// line in the console.
static WARNED_NATIVE_ONLY_DOUBLE_CLICK: AtomicBool = AtomicBool::new(false);

impl super::WasmApp {
    pub(super) fn handle_mouse_input(&mut self, state: ElementState) {
        if state == ElementState::Pressed {
            self.handle_mouse_pressed();
        } else {
            self.handle_mouse_released();
        }
    }

    /// WASM right-button handler.
    ///
    /// `DragState::PendingRight` and the `Action::FastResizeStart`
    /// gesture pipeline are `cfg(not(target_arch = "wasm32"))` —
    /// the throttled-drag machinery doesn't exist on WASM yet
    /// (pending §6.6 touch parity / `TwoFingerDrag`). What this
    /// shim *does* do is prevent the right-button event from being
    /// silently dropped, and surface a one-time warn so a user
    /// who tries Ctrl+RightDrag on WASM sees evidence in the
    /// console rather than wondering why the binding is dead.
    /// Single-press `RightClick` actions stay unreachable on
    /// WASM until the dispatch table grows a path for non-Action
    /// MouseGesture lookups; that's tracked alongside the touch
    /// parity work since both want the same `MouseGesture` →
    /// dispatch wiring.
    pub(super) fn handle_right_button(&mut self, state: ElementState) {
        use std::sync::atomic::{AtomicBool, Ordering};
        static WARNED: AtomicBool = AtomicBool::new(false);
        if state == ElementState::Released
            && !WARNED.swap(true, Ordering::Relaxed)
        {
            log::warn!(
                "right-button gesture on WASM is currently a no-op \
                 (Action::FastResizeStart is NativeOnly until §6.6 \
                 touch parity ships TwoFingerDrag); browser context \
                 menu suppressed via canvas oncontextmenu, \
                 Shift+RightClick bypasses the suppression"
            );
        }
    }

    fn handle_mouse_pressed(&mut self) {
        // --- Left mouse Pressed ---
        let mut input_borrow = self.input.borrow_mut();
        let Some(input) = input_borrow.as_mut() else {
            return;
        };

        // Compute canvas position via renderer
        let canvas_pos = {
            let renderer_borrow = self.renderer.borrow();
            match renderer_borrow.as_ref() {
                Some(r) => r.screen_to_canvas(input.cursor_pos.0 as f32, input.cursor_pos.1 as f32),
                None => return,
            }
        };

        // Hit test against nodes + portal sub-parts + edge
        // labels. Cross-platform helper — the priority chain
        // is byte-identical to native (`compute_click_hit`
        // in `app/mod.rs`), so the previously-duplicated
        // hit-routing block now lives in one place.
        let now = now_ms();
        let parts = compute_click_hit(canvas_pos, input.mindmap_tree.as_mut(), &mut input.app_scene);
        let ClickHitParts {
            click_hit,
            hit_node,
            hit_section_idx,
            portal_text_hit,
            portal_icon_hit,
            edge_label_hit,
        } = parts;
        // Same guard native runs, over the same predicate: a
        // double-click on the element already under edit must not
        // re-open the editor, because re-opening re-seeds the buffer
        // from the committed model value and silently destroys the
        // in-progress edit. WASM has no single-line editor, so its
        // half of the predicate is `false` here.
        let already_editing_same_target = crate::application::app::already_editing_same_target(
            input.text_edit_state.node_id(),
            hit_node.as_deref(),
            false,
        );
        let is_dblclick = !already_editing_same_target
            && input
                .last_click
                .as_ref()
                .map(|prev| is_double_click(prev, now, input.cursor_pos, &click_hit))
                .unwrap_or(false);

        if is_dblclick {
            input.last_click = None;

            let mut renderer_borrow = self.renderer.borrow_mut();
            let Some(renderer) = renderer_borrow.as_mut() else {
                return;
            };

            // Look up which Action the user has bound to
            // `DoubleClick` — the same `action_for_gesture` lookup
            // native runs, with the same modifier fallback, so
            // rebinding or unbinding `double_click_activate` takes
            // effect in the browser too. Pre-unification this side
            // hardcoded the `DoubleClickActivate` body and the
            // keybind table was never consulted.
            let dblclick_name = crate::application::keybinds::MouseGesture::DoubleClick.key_name();
            let action = self.keybinds.action_for_gesture(
                dblclick_name,
                input.modifiers.control_key(),
                input.modifiers.shift_key(),
                input.modifiers.alt_key(),
            );
            if let Some(a) = action {
                let dispatch_hit = dispatch::DispatchHit {
                    click_hit: click_hit.clone(),
                    canvas_pos,
                };
                let outcome = {
                    let mut core = input.input_context_core(renderer, &self.keybinds);
                    dispatch::action_core::dispatch_compatible(&a, &mut core, Some(&dispatch_hit))
                };
                // `Unhandled` here means the browser could not finish
                // the gesture, and it arrives for two very different
                // reasons that must not be reported the same way.
                // Which one it is depends on the **Action** as well as
                // the hit — a `NativeOnly` Action bound to the
                // double-click returns `Unhandled` having run nothing,
                // and its hit can perfectly well be an edge label — so
                // the split is decided by the cross-platform
                // classifier rather than re-derived here.
                if matches!(outcome, DispatchOutcome::Unhandled) {
                    match dispatch::classify_unhandled_pointer_dispatch(&a, &dispatch_hit.click_hit) {
                        // The sanctioned carve-out: the label is
                        // selected and the scene rebuilt; only the
                        // single-line editor open is missing, because
                        // the browser has no single-line editor yet
                        // (CLAUDE.md "Dual-target status"). Expected,
                        // so `debug!` — it must not warn on every
                        // edge-label double-click.
                        dispatch::UnhandledPointerDispatch::EdgeLabelEditorIsNativeOnly(edge_ref) => {
                            log::debug!(
                                "run_wasm: edge-label double-click on {:?} selected the label; \
                                 the single-line editor is native-only \
                                 (DoubleClickResidual::OpenEdgeLabelEditor)",
                                edge_ref,
                            )
                        }
                        // Anything else is a `NativeOnly` Action the
                        // user bound to `DoubleClick`. Newly reachable:
                        // before this PR the browser hardcoded the
                        // double-click body and never consulted the
                        // table at all.
                        dispatch::UnhandledPointerDispatch::NativeOnlyBinding => {
                            crate::application::app::warn_unhandled_native_only_once(
                                &WARNED_NATIVE_ONLY_DOUBLE_CLICK,
                                dblclick_name,
                                &a,
                                "Rebind double_click_activate to a Compatible action, or wait \
                                 for the browser port of the native-only body \
                                 (work_plans/WASM_CONVERGENCE.md).",
                            );
                        }
                    }
                }
            }
            // No Action bound to DoubleClick: silently no-op. Either
            // way the double-click consumed `last_click`; we don't
            // fall through to the single-click pending-selection path.
            self.suppress_keys.set(input.text_edit_state.is_open());
            return;
        }

        input.pending_click = if let Some(id) = hit_node.clone() {
            // Multi-section nodes opt into per-section click
            // routing — the press records the section index so
            // mouse-up commits a `SelectionState::Section`
            // selection. Single-section nodes route through
            // `Node` to preserve today's whole-node click
            // semantic on every migrated map.
            match hit_section_idx {
                Some(section_idx) => PendingClick::Section {
                    node_id: id,
                    section_idx,
                },
                None => PendingClick::Node(id),
            }
        } else if let Some((key, endpoint)) = portal_text_hit.clone() {
            // Portal **text** click — committed to
            // `SelectionState::PortalText` on mouse-up.
            PendingClick::PortalText {
                edge_key: key,
                endpoint_node_id: endpoint,
            }
        } else if let Some((key, endpoint)) = portal_icon_hit.clone() {
            // Portal **icon** click — committed to
            // `SelectionState::PortalLabel` on mouse-up.
            // Double-click already fired above so a
            // pending marker click can only mean "select
            // this label".
            PendingClick::PortalMarker {
                edge_key: key,
                endpoint_node_id: endpoint,
            }
        } else if let Some(key) = edge_label_hit.clone() {
            // Edge label click — committed to
            // `SelectionState::EdgeLabel` on mouse-up.
            PendingClick::EdgeLabel(key)
        } else {
            PendingClick::Empty
        };
        input.last_click = Some(LastClick {
            time: now,
            screen_pos: input.cursor_pos,
            hit: click_hit,
        });
    }

    fn handle_mouse_released(&mut self) {
        // --- Left mouse Released ---
        let mut input_borrow = self.input.borrow_mut();
        let Some(input) = input_borrow.as_mut() else {
            return;
        };

        let pending = std::mem::replace(&mut input.pending_click, PendingClick::None);
        if matches!(pending, PendingClick::None) {
            return;
        }

        if input.text_edit_state.is_open() {
            let mut renderer_borrow = self.renderer.borrow_mut();
            let Some(renderer) = renderer_borrow.as_mut() else {
                return;
            };
            let release_canvas =
                renderer.screen_to_canvas(input.cursor_pos.0 as f32, input.cursor_pos.1 as f32);
            // Same predicate native's modal-release ladder runs.
            if crate::application::app::text_edit::release_stays_inside_edited_node(
                &input.text_edit_state,
                &mut input.mindmap_tree,
                release_canvas,
            ) {
                return;
            }

            // Click-outside: commit via the funnel
            // (`Action::TextEditCommit`). Track C lets WASM
            // call the same `dispatch_compatible` that
            // native uses for this Compatible Action.
            {
                let mut core = input.input_context_core(renderer, &self.keybinds);
                let _ = dispatch::action_core::dispatch_compatible(&Action::TextEditCommit, &mut core, None);
            }
            self.suppress_keys.set(false);
            // Fall through to the click-target selection
            // update below — same as native's
            // `event_mouse_click.rs` flow (commit then
            // `handle_click`). Without this, the
            // editor-commit path's `lift_anchor_to_section_range`
            // leaves `SelectionState::SectionRange` standing
            // even though the user clicked away to a different
            // node. Drop the renderer borrow first so the
            // re-borrow at the bottom of this function
            // doesn't panic.
            drop(renderer_borrow);
        }

        // Fire any `OnClick` triggers BEFORE the selection
        // update so document actions (theme switches etc.) take
        // effect before the scene rebuild below picks up the new
        // state. Pre-fix the WASM click handler **never**
        // dispatched triggers — every map's `OnClick`-bound
        // mutation (whole-node and per-section) was silently
        // dead on the web. Native does this in `click.rs`; the
        // WASM analogue mirrors that flow with
        // `PlatformContext::Web` filtering.
        //
        // Section-aware: when the click resolved to a specific
        // section, `find_triggered_mutations_at` walks the
        // section's `trigger_bindings` first, then the whole-
        // node bindings — same precedence native enforces.
        let (trigger_node_id, trigger_section_idx): (Option<String>, Option<usize>) = match &pending {
            PendingClick::Node(id) => (Some(id.clone()), None),
            PendingClick::Section { node_id, section_idx } => (Some(node_id.clone()), Some(*section_idx)),
            _ => (None, None),
        };
        if let Some(id) = trigger_node_id.as_ref() {
            fire_onclick_triggers(
                &mut input.document,
                &mut input.mindmap_tree,
                &mut input.scene_cache,
                id,
                trigger_section_idx,
                baumhard::mindmap::custom_mutation::PlatformContext::Web,
                now_ms() as u64,
            );
        }

        // Plain selection click. Snapshot the previous
        // selection so `rebuild_after_selection_change`
        // can pick between `rebuild_all` (needed when
        // either side is a node selection — tree
        // highlights must be applied or cleared) and the
        // cheaper `rebuild_scene_only` (edge-adjacent →
        // edge-adjacent transitions).
        let prev_selection = input.document.selection.clone();
        input.document.selection = match pending {
            PendingClick::Node(node_id) => SelectionState::Single(node_id),
            PendingClick::Section { node_id, section_idx } => {
                SelectionState::Section(SectionSel { node_id, section_idx })
            }
            PendingClick::PortalMarker {
                edge_key,
                endpoint_node_id,
            } => SelectionState::PortalLabel(PortalLabelSel {
                edge_key,
                endpoint_node_id,
            }),
            PendingClick::PortalText {
                edge_key,
                endpoint_node_id,
            } => SelectionState::PortalText(PortalLabelSel {
                edge_key,
                endpoint_node_id,
            }),
            PendingClick::EdgeLabel(edge_key) => {
                let er = EdgeRef::new(
                    edge_key.from_id.as_str(),
                    edge_key.to_id.as_str(),
                    edge_key.edge_type.as_str(),
                );
                SelectionState::EdgeLabel(EdgeLabelSel::new(er))
            }
            _ => SelectionState::None,
        };
        let mut renderer_borrow = self.renderer.borrow_mut();
        if let Some(renderer) = renderer_borrow.as_mut() {
            rebuild_after_selection_change(
                &prev_selection,
                &input.document,
                &input.interaction_mode,
                &mut input.mindmap_tree,
                &mut input.app_scene,
                renderer,
                &mut input.scene_cache,
            );
        }
    }
}
