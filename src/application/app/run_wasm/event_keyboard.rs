// SPDX-License-Identifier: MPL-2.0

//! `WindowEvent::KeyboardInput` arm (Pressed only). Routes
//! through the editor-steal pre-filter (`TextEditCommit` /
//! `TextEditCancel` first, then the modal `handle_text_edit_key`
//! body), then the document-context Action lookup, then the Macro
//! fallback, then the custom-mutation tier. That is the same
//! Action -> Macro -> CustomMutation chain native runs in
//! `event_keyboard::handle_keyboard_input`; the WASM side has no
//! console / color-picker / label / portal edit modals so only the
//! pre-filter ladder is shorter.

#![cfg(target_arch = "wasm32")]

use crate::application::platform::input::Key;

use super::WasmMacroDispatchTarget;
use crate::application::app::dispatch;
use crate::application::app::text_edit::handle_text_edit_key;
use crate::application::keybinds::Action;

impl super::WasmApp {
    pub(super) fn handle_keyboard_input(&mut self, logical_key: Key) {
        let key_name = crate::application::keybinds::key_to_name(&logical_key);

        let mut input_borrow = self.input.borrow_mut();
        let mut renderer_borrow = self.renderer.borrow_mut();
        let (Some(input), Some(renderer)) = (input_borrow.as_mut(), renderer_borrow.as_mut()) else {
            return;
        };

        // Editor keyboard-steal: if open, route all keys
        // to the editor so hotkeys don't collide with typed text.
        //
        // Commit/cancel pre-filter: dispatch through the funnel
        // (`Action::TextEditCommit` / `TextEditCancel`) BEFORE
        // calling the modal handler. Mirrors the native shape
        // at `event_keyboard.rs`.
        if input.text_edit_state.is_open() {
            let action = key_name.as_deref().and_then(|n| {
                self.keybinds.action_for_context(
                    crate::application::keybinds::InputContext::TextEdit,
                    n,
                    input.modifiers.control_key(),
                    input.modifiers.shift_key(),
                    input.modifiers.alt_key(),
                )
            });
            if let Some(modal_action @ (Action::TextEditCommit | Action::TextEditCancel)) = &action {
                let mut core = input.input_context_core(renderer, &self.keybinds);
                let _ = dispatch::action_core::dispatch_compatible(modal_action, &mut core, None);
                self.suppress_keys.set(input.text_edit_state.is_open());
                return;
            }
            handle_text_edit_key(
                &key_name,
                &logical_key,
                input.modifiers.control_key(),
                input.modifiers.shift_key(),
                input.modifiers.alt_key(),
                &self.keybinds,
                &mut input.text_edit_state,
                &mut input.document,
                &mut input.mindmap_tree,
                &mut input.app_scene,
                renderer,
                &mut input.scene_cache,
            );
            self.suppress_keys.set(input.text_edit_state.is_open());
            return;
        }

        // Hotkey dispatch via keybinds.
        let action = key_name.as_deref().and_then(|k| {
            self.keybinds.action_for_context(
                crate::application::keybinds::InputContext::Document,
                k,
                input.modifiers.control_key(),
                input.modifiers.shift_key(),
                input.modifiers.alt_key(),
            )
        });
        // WASM dispatch ladder. Action body lives in the
        // unified `dispatch_action_core::dispatch_compatible`
        // (Track C) — both the keyboard path here and the
        // `WasmMacroDispatchTarget::dispatch_action` impl
        // reach the same body. Native's `dispatch::dispatch_action`
        // delegates to it as well; one source of truth for
        // every Compatible arm across both targets.
        //
        // After the action lookup completes, fall through to
        // macro lookup — the same Action → Macro →
        // CustomMutation chain native uses (`event_keyboard.rs`).
        if let Some(a) = action.clone() {
            // Pin "did the user just trigger an EditSelection?"
            // before the dispatch — `self.suppress_keys` is
            // ONLY updated for that pair (pre-Track-B, the
            // suppress call lived inside the EditSelection
            // pre-filter arm). Other Compatible Actions don't
            // touch suppress.
            let was_edit_selection = matches!(a, Action::EditSelection | Action::EditSelectionClean);
            let _ = {
                let mut core = input.input_context_core(renderer, &self.keybinds);
                dispatch::action_core::dispatch_compatible(&a, &mut core, None)
            };
            if was_edit_selection {
                // Mirror pre-Track-B `set(is_open())`: flip
                // suppress to whatever the modal state ended
                // up at — true if the editor opened, false if
                // it didn't (e.g. selection wasn't Single).
                // Always-set, NOT gated on dispatch outcome.
                // Track-C-Commit-3 incorrectly gated on
                // `Handled` which left suppress stuck-true on a
                // non-Single EditSelection (Unhandled outcome);
                // restored here per the design reviewer's flag.
                self.suppress_keys.set(input.text_edit_state.is_open());
            }
        } else if let Some(k) = key_name.as_deref() {
            // No built-in Action bound to this combo — fall through
            // the rest of native's chain: Action → Macro →
            // CustomMutation (CONCEPTS §5). Privilege gating runs
            // inside `dispatch::macro_core::dispatch_macro` so a
            // hostile Map / Inline tier macro can't slip destructive
            // Actions or ConsoleLine past.
            let (ctrl, shift, alt) = (
                input.modifiers.control_key(),
                input.modifiers.shift_key(),
                input.modifiers.alt_key(),
            );
            let macro_id = self.keybinds.macro_for(k, ctrl, shift, alt).map(str::to_string);
            // A macro bound to an id that isn't in the registry
            // (typo'd config, half-loaded macros file) returns false
            // from `dispatch_macro`; fall through to the
            // custom-mutation tier so the keystroke still has a
            // chance to do something, exactly as native does.
            let macro_handled = if let Some(id) = macro_id {
                let mut target = WasmMacroDispatchTarget {
                    input,
                    renderer,
                    keybinds: &self.keybinds,
                };
                dispatch::macro_core::dispatch_macro(&id, &mut target)
            } else {
                false
            };
            if !macro_handled {
                // Third tier. Pre-unification the browser's chain
                // stopped at Macro, so a `custom_mutation_bindings`
                // entry worked on the desktop and was silently dead
                // on the web.
                //
                // **Instant mutations only.** An entry carrying
                // `timing.duration_ms > 0` takes
                // `apply_keybind_custom_mutation`'s `start_animation`
                // branch, which only *queues* the envelope. The one
                // site that advances it is
                // `drain_frame::drain_animation_tick`, and
                // `drain_frame.rs` is `#[cfg(not(target_arch =
                // "wasm32"))]` — the browser has no per-frame drain
                // loop, so an animated mutation started here never
                // lerps, never completes, and never reaches the
                // completion `apply_custom_mutation` / undo push. The
                // user gets `apply_document_actions`' side effects
                // without the mutation.
                //
                // Registered in CLAUDE.md's "Dual-target status" with
                // the parity trajectory. Pre-existing on the click
                // path (`click_triggers.rs` calls
                // `start_animation_at` the same way); this tier
                // widens the surface from click triggers to every
                // bound keystroke, which is why it is stated here
                // rather than left implicit.
                let mut core = input.input_context_core(renderer, &self.keybinds);
                let _ = dispatch::dispatch_custom_mutation_for_key(&mut core, k, ctrl, shift, alt);
            }
        }
    }
}
