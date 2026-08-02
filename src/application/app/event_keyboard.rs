// SPDX-License-Identifier: MPL-2.0

//! Keyboard-event dispatch. Routes `KeyboardInput` (Pressed only)
//! through the modal-steal ladder (console, color picker, label /
//! portal / node text editors), then the action table, then the
//! custom-mutation key bindings.

#![cfg(not(target_arch = "wasm32"))]

use winit::event_loop::ActiveEventLoop;

use crate::application::platform::input::Key;

use super::color_picker_flow::{handle_color_picker_clipboard_key, picker_op_for};
use super::console_input::handle_console_key;
use super::input_context::InputHandlerContext;
use super::modal_editor::{steal_key_for_modal, ModalEditor};
use super::single_line_edit::{
    handle_single_line_edit_key, open_single_line_edit, resolve_single_line_target,
};

pub(super) fn handle_keyboard_input(
    logical_key: Key,
    _event_loop: &ActiveEventLoop,
    ctx: &mut InputHandlerContext<'_>,
) {
    let key_name = crate::application::keybinds::key_to_name(&logical_key);

    // When the console is open, it steals all
    // keyboard input. Character keys insert at the
    // cursor, Tab triggers completion, Up/Down walks
    // history, Enter parses + executes, Escape
    // closes. Regular hotkeys are suppressed until
    // the console closes.
    if ctx.console_state.is_open() {
        handle_console_key(&key_name, &logical_key, ctx);
        return;
    }

    // Glyph-wheel color picker key handling.
    // Mutually exclusive with console and label-edit
    // for the keys it claims (Esc, Enter, h/s/v/
    // H/S/V). Any other key — notably the console
    // trigger `/` — falls through so the Standalone
    // persistent palette doesn't deadlock the user
    // out of the normal keybind dispatch.
    //
    // Commit / cancel / nudge pre-filter: dispatch through the
    // funnel BEFORE the modal handler runs, exactly like the
    // `LabelEdit*` and `TextEdit*` blocks below. `picker_op_for`
    // is the shared predicate the `dispatch_action` arm guards
    // on, so the two sides cannot drift. Only the clipboard verbs
    // (which carry a hex payload no `Action` body can express)
    // stay modal-local.
    if ctx.color_picker_state.is_open() {
        let picker_action = key_name.as_deref().and_then(|n| {
            ctx.keybinds.action_for_context(
                crate::application::keybinds::InputContext::ColorPicker,
                n,
                ctx.modifiers.control_key(),
                ctx.modifiers.shift_key(),
                ctx.modifiers.alt_key(),
            )
        });
        if picker_action.as_ref().is_some_and(|a| picker_op_for(a).is_some()) {
            // Funnel-owned: commit / cancel / nudge.
            // `Unhandled` comes back for the Standalone-cancel and
            // no-document cases — the picker declined the key, so
            // it falls through to Document dispatch below.
            if let Some(modal_action) = picker_action {
                if matches!(
                    super::dispatch::dispatch_action(modal_action, ctx, None),
                    super::dispatch::DispatchOutcome::Handled
                ) {
                    return;
                }
            }
        } else {
            let consumed = if let Some(doc) = ctx.document.as_mut() {
                handle_color_picker_clipboard_key(
                    picker_action.as_ref(),
                    ctx.color_picker_state,
                    doc,
                    ctx.picker_hover,
                )
            } else {
                false
            };
            if consumed {
                return;
            }
        }
    }

    // Inline text-editor modals. The single-line editor (edge
    // label / portal caption) and the multi-line node text editor
    // steal keys the same way the console does, and they steal
    // them the *same* way as each other: commit / cancel are
    // pre-filtered through the funnel — `dispatch_action`'s arm
    // body owns the close-and-rebuild path — and everything else
    // goes to the editor's own handler, which keeps the
    // literal-Key character insertion and the rebindable cursor
    // primitives (CODE_CONVENTIONS §3 carve-out for Key payloads).
    // Inside the node editor Enter and Tab are literal characters:
    // it is a multi-line paragraph editor, not an outliner.
    if let Some(modal) = ModalEditor::stealing(ctx) {
        steal_key_for_modal(modal, &key_name, &logical_key, ctx);
        return;
    }

    // When NodeEdit mode is active and no other modal stole the
    // key, try the NodeEdit context first. The cascade falls
    // through to Document on miss (per
    // `InputContext::NodeEdit::falls_through() == true`), so
    // unmatched keys reach the global Action set normally.
    // `action_for_context` itself does the fallthrough; we just
    // pick the right starting context.
    let starting_context = if matches!(ctx.interaction_mode, super::InteractionMode::NodeEdit { .. }) {
        crate::application::keybinds::InputContext::NodeEdit
    } else {
        crate::application::keybinds::InputContext::Document
    };
    let action = key_name.as_deref().and_then(|k| {
        ctx.keybinds.action_for_context(
            starting_context,
            k,
            ctx.modifiers.control_key(),
            ctx.modifiers.shift_key(),
            ctx.modifiers.alt_key(),
        )
    });

    // Type-to-edit on edge / portal label selections: only fires
    // when no action claims the key, so rebound printables go to
    // dispatch first.
    if action.is_none() && try_type_to_edit(&logical_key, &key_name, ctx) {
        return;
    }

    if let Some(a) = action {
        // Action body lives in `super::dispatch::dispatch_action`.
        let _ = super::dispatch::dispatch_action(a, ctx, None);
    } else {
        // No built-in action matched — try ctx.macros first, then custom
        // mutations. Resolution order is documented in CONCEPTS.md §5
        // "Action dispatch": Action -> Macro -> CustomMutation.
        if let Some(k) = key_name.as_deref() {
            let macro_id = ctx
                .keybinds
                .macro_for(
                    k,
                    ctx.modifiers.control_key(),
                    ctx.modifiers.shift_key(),
                    ctx.modifiers.alt_key(),
                )
                .map(|s| s.to_string());
            // If a macro is bound but its id isn't in the registry
            // (typo'd config, half-loaded ctx.macros file, etc.),
            // `dispatch_macro` returns false. Fall through to the
            // custom-mutation tier so the keystroke still has a
            // chance to do something — better UX than swallowing
            // silently.
            let macro_handled = if let Some(id) = macro_id {
                super::dispatch::dispatch_macro(&id, ctx)
            } else {
                false
            };
            if !macro_handled {
                // Custom mutation fall-through (Phase-7 parity:
                // animation-timing aware, always invokes
                // `apply_document_actions`). The tier body is
                // cross-platform and takes the shared core, so the
                // WASM keyboard handler runs the same three-tier
                // chain.
                let (ctrl, shift, alt) = (
                    ctx.modifiers.control_key(),
                    ctx.modifiers.shift_key(),
                    ctx.modifiers.alt_key(),
                );
                let (mut core, _) = ctx.split_borrow();
                let _ = super::dispatch::dispatch_custom_mutation_for_key(&mut core, k, ctrl, shift, alt);
            }
        }
    }
}

/// On edge / portal label selections, intercept printable
/// characters and route them to the inline editor: open the
/// right editor for the current selection, then replay the
/// keystroke into it so the typed character lands as the first
/// edit.
///
/// Mirrors the **shape** of the `EditSelectionClean`-on-node flow
/// (open the editor, replay the keystroke into it) but not its
/// buffer contract: this path seeds the existing text and the
/// typed character appends at the cursor. See the `clean = false`
/// note on the `open_single_line_edit` call below — switching
/// to an empty buffer would change what typing over a selected
/// label does, which is a product decision, not a funnel fix.
///
/// Returns `true` when the keystroke was consumed (editor
/// opened or selection was stale and the keystroke was dropped
/// to avoid running Document-level dispatch with a phantom
/// target). Returns `false` to let the caller fall through to
/// the macro / custom-mutation tier.
///
/// Caller already checked `action.is_none()` so rebinding any
/// printable to a Document action keeps that binding alive
/// even when an edge label is selected.
fn try_type_to_edit(logical_key: &Key, key_name: &Option<String>, ctx: &mut InputHandlerContext<'_>) -> bool {
    if ctx.modifiers.control_key() || ctx.modifiers.alt_key() {
        return false;
    }
    let Key::Character(ref c) = *logical_key else {
        return false;
    };
    // Reject empty payloads and pure-control payloads up
    // front so single-char shortcuts that the keybind table
    // hasn't claimed don't accidentally open an editor.
    if !c.as_str().chars().any(|ch| !ch.is_control()) {
        return false;
    }
    let Some(doc) = ctx.document.as_mut() else {
        return false;
    };
    // Same selection → editor mapping the `EditSelection` /
    // `LabelEditOnSelection` dispatch arms use.
    //
    // `clean = false`: the editor seeds with the existing text and
    // the typed character appends at the cursor. This is the
    // historical behavior and it is deliberately NOT
    // `EditSelectionClean`'s empty-buffer contract — retyping over
    // a selected label would be a separate, user-visible product
    // decision.
    let Some(target) = resolve_single_line_target(&doc.selection) else {
        return false;
    };
    open_single_line_edit(
        target,
        false,
        doc,
        ctx.single_line_edit_state,
        ctx.app_scene,
        ctx.renderer,
    );
    if ctx.single_line_edit_state.is_open() {
        handle_single_line_edit_key(
            key_name,
            logical_key,
            ctx.modifiers.control_key(),
            ctx.modifiers.shift_key(),
            ctx.modifiers.alt_key(),
            ctx.keybinds,
            ctx.single_line_edit_state,
            doc,
            ctx.interaction_mode,
            ctx.mindmap_tree,
            ctx.app_scene,
            ctx.renderer,
            ctx.scene_cache,
        );
        return true;
    }
    // `open_*` silently returned — the selection's target
    // evaporated (edge deleted by a background undo, portal
    // edge flipped to line mode, etc). Drop the keystroke
    // rather than falling through to action dispatch with a
    // stale selection — the user's mental model was "I'm about
    // to type into this selected thing", not "trigger a
    // Document action".
    log::warn!(
        "type-to-edit: selected edge / portal endpoint vanished before editor could open; keystroke dropped"
    );
    true
}
