// SPDX-License-Identifier: MPL-2.0

//! Keyboard-event dispatch. Routes `KeyboardInput` (Pressed only)
//! through the modal-steal ladder (console, color picker, label /
//! portal / node text editors), then the action table, then the
//! custom-mutation key bindings.

#![cfg(not(target_arch = "wasm32"))]

use winit::event_loop::ActiveEventLoop;

use crate::application::platform::input::Key;

use super::color_picker_flow::handle_color_picker_key;
use super::console_input::handle_console_key;
use super::input_context::InputHandlerContext;
use super::label_edit::{
    handle_label_edit_key, handle_portal_text_edit_key, open_label_edit, open_portal_text_edit,
};
use super::text_edit::handle_text_edit_key;
use crate::application::document::SelectionState;
use crate::application::keybinds::Action;

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
    if ctx.color_picker_state.is_open() {
        let consumed = if let Some(doc) = ctx.document.as_mut() {
            handle_color_picker_key(
                &key_name,
                ctx.modifiers.control_key(),
                ctx.modifiers.shift_key(),
                ctx.modifiers.alt_key(),
                ctx.keybinds,
                ctx.color_picker_state,
                doc,
                ctx.interaction_mode,
                ctx.mindmap_tree,
                ctx.picker_hover,
                ctx.app_scene,
                ctx.renderer,
                ctx.scene_cache,
            )
        } else {
            false
        };
        if consumed {
            return;
        }
    }

    // Inline label edit modal. Steals keys the same way
    // the console does. Escape discards, Enter commits,
    // Backspace pops, character keys append.
    //
    // Commit/cancel pre-filter: dispatch through the funnel
    // (`Action::LabelEditCommit` / `LabelEditCancel`) BEFORE
    // calling the modal handler — `dispatch_action`'s arm body
    // owns the close-and-rebuild path. The modal handler retains
    // the literal-Key character insertion + cursor primitives
    // (CODE_CONVENTIONS §3 carve-out for Key payloads).
    if ctx.label_edit_state.is_open() {
        let action = key_name.as_deref().and_then(|n| {
            ctx.keybinds.action_for_context(
                crate::application::keybinds::InputContext::LabelEdit,
                n,
                ctx.modifiers.control_key(),
                ctx.modifiers.shift_key(),
                ctx.modifiers.alt_key(),
            )
        });
        if let Some(modal_action @ (Action::LabelEditCommit | Action::LabelEditCancel)) = action {
            let _ = super::dispatch::dispatch_action(modal_action, ctx, None);
            return;
        }
        if let Some(doc) = ctx.document.as_mut() {
            handle_label_edit_key(
                &key_name,
                &logical_key,
                ctx.modifiers.control_key(),
                ctx.modifiers.shift_key(),
                ctx.modifiers.alt_key(),
                ctx.keybinds,
                ctx.label_edit_state,
                doc,
                ctx.mindmap_tree,
                ctx.app_scene,
                ctx.renderer,
                ctx.scene_cache,
            );
        }
        return;
    }

    // Inline portal-text edit modal — parallel to the
    // edge label editor but keyed to
    // `(edge_ref, endpoint_node_id)`. Same keystroke
    // routing via `InputContext::LabelEdit` and the same
    // `LabelEditCommit/Cancel` Actions (the dispatch arm
    // picks the open state).
    if ctx.portal_text_edit_state.is_open() {
        let action = key_name.as_deref().and_then(|n| {
            ctx.keybinds.action_for_context(
                crate::application::keybinds::InputContext::LabelEdit,
                n,
                ctx.modifiers.control_key(),
                ctx.modifiers.shift_key(),
                ctx.modifiers.alt_key(),
            )
        });
        if let Some(modal_action @ (Action::LabelEditCommit | Action::LabelEditCancel)) = action {
            let _ = super::dispatch::dispatch_action(modal_action, ctx, None);
            return;
        }
        if let Some(doc) = ctx.document.as_mut() {
            handle_portal_text_edit_key(
                &key_name,
                &logical_key,
                ctx.modifiers.control_key(),
                ctx.modifiers.shift_key(),
                ctx.modifiers.alt_key(),
                ctx.keybinds,
                ctx.portal_text_edit_state,
                doc,
                ctx.interaction_mode,
                ctx.mindmap_tree,
                ctx.app_scene,
                ctx.renderer,
                ctx.scene_cache,
            );
        }
        return;
    }

    // Inline node text editor. Steals keys the same way
    // the console / label-edit modals do. Enter and Tab
    // are literal characters inside the editor — this is
    // a multi-line paragraph editor, not an outliner.
    // Esc cancels; commit is via click-outside in the
    // mouse handler. Pre-filter commit/cancel through the
    // funnel like LabelEdit above.
    if ctx.text_edit_state.is_open() {
        let action = key_name.as_deref().and_then(|n| {
            ctx.keybinds.action_for_context(
                crate::application::keybinds::InputContext::TextEdit,
                n,
                ctx.modifiers.control_key(),
                ctx.modifiers.shift_key(),
                ctx.modifiers.alt_key(),
            )
        });
        if let Some(modal_action @ (Action::TextEditCommit | Action::TextEditCancel)) = action {
            let _ = super::dispatch::dispatch_action(modal_action, ctx, None);
            return;
        }
        if let Some(doc) = ctx.document.as_mut() {
            handle_text_edit_key(
                &key_name,
                &logical_key,
                ctx.modifiers.control_key(),
                ctx.modifiers.shift_key(),
                ctx.modifiers.alt_key(),
                ctx.keybinds,
                ctx.text_edit_state,
                doc,
                ctx.mindmap_tree,
                ctx.app_scene,
                ctx.renderer,
                ctx.scene_cache,
            );
        }
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
                // `apply_document_actions`).
                let _ = super::dispatch::dispatch_custom_mutation_for_key(
                    ctx,
                    k,
                    ctx.modifiers.control_key(),
                    ctx.modifiers.shift_key(),
                    ctx.modifiers.alt_key(),
                );
            }
        }
    }
}

/// On edge / portal label selections, intercept printable
/// characters and route them to the inline editor: open the
/// right editor for the current selection, then replay the
/// keystroke into it so the typed character lands as the first
/// edit. Mirrors the `EditSelectionClean`-on-node flow.
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
    let opened = match doc.selection.clone() {
        SelectionState::EdgeLabel(s) => {
            open_label_edit(
                &s.edge_ref,
                doc,
                ctx.label_edit_state,
                ctx.app_scene,
                ctx.renderer,
            );
            ctx.label_edit_state.is_open()
        }
        SelectionState::PortalLabel(s) | SelectionState::PortalText(s) => {
            let er = s.edge_ref();
            open_portal_text_edit(
                &er,
                &s.endpoint_node_id,
                doc,
                ctx.portal_text_edit_state,
                ctx.app_scene,
                ctx.renderer,
            );
            ctx.portal_text_edit_state.is_open()
        }
        _ => return false,
    };
    if opened {
        if ctx.label_edit_state.is_open() {
            handle_label_edit_key(
                key_name,
                logical_key,
                ctx.modifiers.control_key(),
                ctx.modifiers.shift_key(),
                ctx.modifiers.alt_key(),
                ctx.keybinds,
                ctx.label_edit_state,
                doc,
                ctx.mindmap_tree,
                ctx.app_scene,
                ctx.renderer,
                ctx.scene_cache,
            );
        } else if ctx.portal_text_edit_state.is_open() {
            handle_portal_text_edit_key(
                key_name,
                logical_key,
                ctx.modifiers.control_key(),
                ctx.modifiers.shift_key(),
                ctx.modifiers.alt_key(),
                ctx.keybinds,
                ctx.portal_text_edit_state,
                doc,
                ctx.interaction_mode,
                ctx.mindmap_tree,
                ctx.app_scene,
                ctx.renderer,
                ctx.scene_cache,
            );
        }
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
