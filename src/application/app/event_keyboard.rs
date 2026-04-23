//! Keyboard-event dispatch extracted from the native event loop
//! in [`super::run_native`]. Routes `WindowEvent::KeyboardInput`
//! (Pressed state only) through the modal-steal ladder
//! (console, color picker, label / portal / node text editors)
//! down to the action table and finally to the
//! custom-mutation key bindings. Persistent state flows in through
//! [`super::input_context::InputHandlerContext`].

#![cfg(not(target_arch = "wasm32"))]

use super::*;
use super::input_context::InputHandlerContext;
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::Key;

pub(super) fn handle_keyboard_input(
    logical_key: Key,
    _event_loop: &ActiveEventLoop,
    ctx: InputHandlerContext<'_>,
) {
    let InputHandlerContext {
        document,
        mindmap_tree,
        app_scene,
        renderer,
        scene_cache,
        app_mode,
        console_state,
        console_history,
        label_edit_state,
        portal_text_edit_state,
        text_edit_state,
        color_picker_state,
        last_click,
        hovered_node,
        cursor_pos,
        modifiers,
        picker_hover,
        keybinds,
        ..
    } = ctx;
    let cursor_pos = *cursor_pos;
    let key_name = crate::application::keybinds::key_to_name(&logical_key);

    // When the console is open, it steals all
    // keyboard input. Character keys insert at the
    // cursor, Tab triggers completion, Up/Down walks
    // history, Enter parses + executes, Escape
    // closes. Regular hotkeys are suppressed until
    // the console closes.
    if console_state.is_open() {
        handle_console_key(
            &key_name,
            &logical_key,
            modifiers.control_key(),
            modifiers.shift_key(),
            modifiers.alt_key(),
            console_state,
            console_history,
            label_edit_state,
            portal_text_edit_state,
            color_picker_state,
            document,
            mindmap_tree,
            app_scene,
            renderer,
            scene_cache,
            keybinds,
        );
        return;
    }

    // Glyph-wheel color picker key handling.
    // Mutually exclusive with console and label-edit
    // for the keys it claims (Esc, Enter, h/s/v/
    // H/S/V). Any other key — notably the console
    // trigger `/` — falls through so the Standalone
    // persistent palette doesn't deadlock the user
    // out of the normal keybind dispatch.
    if color_picker_state.is_open() {
        let consumed = if let Some(doc) = document.as_mut() {
            handle_color_picker_key(
                &key_name,
                modifiers.control_key(),
                modifiers.shift_key(),
                modifiers.alt_key(),
                keybinds,
                color_picker_state,
                doc,
                mindmap_tree,
                picker_hover,
                app_scene,
                renderer,
                scene_cache,
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
    if label_edit_state.is_open() {
        if let Some(doc) = document.as_mut() {
            handle_label_edit_key(
                &key_name,
                &logical_key,
                modifiers.control_key(),
                modifiers.shift_key(),
                modifiers.alt_key(),
                keybinds,
                label_edit_state,
                doc,
                mindmap_tree,
                app_scene,
                renderer,
                scene_cache,
            );
        }
        return;
    }

    // Inline portal-text edit modal — parallel to the
    // edge label editor but keyed to
    // `(edge_ref, endpoint_node_id)`. Same keystroke
    // routing via `InputContext::LabelEdit`.
    if portal_text_edit_state.is_open() {
        if let Some(doc) = document.as_mut() {
            handle_portal_text_edit_key(
                &key_name,
                &logical_key,
                modifiers.control_key(),
                modifiers.shift_key(),
                modifiers.alt_key(),
                keybinds,
                portal_text_edit_state,
                doc,
                mindmap_tree,
                app_scene,
                renderer,
                scene_cache,
            );
        }
        return;
    }

    // Inline node text editor. Steals keys the same way
    // the console / label-edit modals do. Enter and Tab
    // are literal characters inside the editor — this is
    // a multi-line paragraph editor, not an outliner.
    // Esc cancels; commit is via click-outside in the
    // mouse handler.
    if text_edit_state.is_open() {
        if let Some(doc) = document.as_mut() {
            handle_text_edit_key(
                &key_name,
                &logical_key,
                modifiers.control_key(),
                modifiers.shift_key(),
                modifiers.alt_key(),
                keybinds,
                text_edit_state,
                doc,
                mindmap_tree,
                app_scene,
                renderer,
                scene_cache,
            );
        }
        return;
    }

    let action = key_name.as_deref().and_then(|k| {
        keybinds.action_for_context(
            crate::application::keybinds::InputContext::Document,
            k,
            modifiers.control_key(),
            modifiers.shift_key(),
            modifiers.alt_key(),
        )
    });

    // Type-to-edit on edge / portal label selections: when an
    // editable selection is active, no editor is open, no action
    // claims the key (so custom mutations / keybind rebinds always
    // win), and the user types a printable character (no Ctrl /
    // Alt — Shift is OK so capital letters and shifted symbols
    // still type), open the right inline editor and replay the
    // keystroke through it so the typed character lands in the
    // buffer as the first edit. This makes the gesture symmetric
    // with what the node editor offers via `EditSelectionClean` /
    // typing on a freshly-selected node. The action-first check
    // means rebinding `'a'` to a Document action keeps that
    // binding alive even when an edge label is selected.
    if action.is_none()
        && !modifiers.control_key()
        && !modifiers.alt_key()
    {
        if let Key::Character(ref c) = logical_key {
            // Reject empty payloads and pure-control payloads up
            // front so single-char shortcuts that the keybind table
            // hasn't claimed don't accidentally open an editor.
            let has_printable = c.as_str().chars().any(|ch| !ch.is_control());
            if has_printable {
                if let Some(doc) = document.as_mut() {
                    let opened = match doc.selection.clone() {
                        SelectionState::EdgeLabel(s) => {
                            open_label_edit(
                                &s.edge_ref,
                                doc,
                                label_edit_state,
                                app_scene,
                                renderer,
                            );
                            label_edit_state.is_open()
                        }
                        SelectionState::PortalLabel(s)
                        | SelectionState::PortalText(s) => {
                            let er = s.edge_ref();
                            open_portal_text_edit(
                                &er,
                                &s.endpoint_node_id,
                                doc,
                                portal_text_edit_state,
                                app_scene,
                                renderer,
                            );
                            portal_text_edit_state.is_open()
                        }
                        _ => false,
                    };
                    if opened {
                        // Replay the typed character through the
                        // newly-opened editor so the first key
                        // ends up in the buffer instead of being
                        // swallowed by the open gesture.
                        if label_edit_state.is_open() {
                            handle_label_edit_key(
                                &key_name,
                                &logical_key,
                                modifiers.control_key(),
                                modifiers.shift_key(),
                                modifiers.alt_key(),
                                keybinds,
                                label_edit_state,
                                doc,
                                mindmap_tree,
                                app_scene,
                                renderer,
                                scene_cache,
                            );
                        } else if portal_text_edit_state.is_open() {
                            handle_portal_text_edit_key(
                                &key_name,
                                &logical_key,
                                modifiers.control_key(),
                                modifiers.shift_key(),
                                modifiers.alt_key(),
                                keybinds,
                                portal_text_edit_state,
                                doc,
                                mindmap_tree,
                                app_scene,
                                renderer,
                                scene_cache,
                            );
                        }
                        return;
                    }
                    // `open_*` silently returned without opening an
                    // editor — the selection's target evaporated
                    // (edge deleted by a background undo, portal
                    // edge flipped to line mode, etc). Log and drop
                    // the keystroke rather than falling through to
                    // action dispatch with a stale selection — the
                    // user's mental model was "I'm about to type
                    // into this selected thing", not "trigger a
                    // Document action".
                    log::warn!(
                        "type-to-edit: selected edge / portal endpoint \
                         vanished before editor could open; keystroke dropped"
                    );
                    return;
                }
            }
        }
    }

    match action {
        Some(Action::OpenConsole) => {
            // Toggle: already open → close. Otherwise
            // construct a fresh state seeded with the
            // persisted history. Rebuild the overlay
            // so the frame appears immediately.
            if console_state.is_open() {
                save_console_history(console_history);
                *console_state = ConsoleState::Closed;
                renderer.rebuild_console_overlay_buffers(app_scene, None);
            } else {
                *console_state = ConsoleState::open(console_history.clone());
                if let Some(doc) = document.as_ref() {
                    rebuild_console_overlay(console_state, doc, app_scene, renderer, keybinds);
                }
            }
        }
        Some(Action::Undo) => {
            if let Some(doc) = document.as_mut() {
                // Ctrl+Z during an animation fast-forwards to
                // completion so the animation's own undo entry
                // lands on the stack before we pop — Ctrl+Z
                // reverses the animation's effect in one
                // keystroke, matching the post-completion
                // Ctrl+Z behaviour.
                if doc.has_active_animations() {
                    doc.fast_forward_animations(mindmap_tree.as_mut());
                }
                if doc.undo() {
                    // Undo restores node positions / edge paths in
                    // place; cached connection samples key off those
                    // coordinates and would be served stale. Clear
                    // before the rebuild resamples.
                    scene_cache.clear();
                    rebuild_all(doc, mindmap_tree, app_scene, renderer, scene_cache);
                }
            }
        }
        Some(Action::CancelMode) => {
            if matches!(*app_mode, AppMode::Reparent { .. } | AppMode::Connect { .. }) {
                *app_mode = AppMode::Normal;
                *hovered_node = None;
                // Clear any stale click so a post-mode click
                // doesn't get retroactively paired with a pre-mode
                // click into a spurious double-click.
                *last_click = None;
                if let Some(doc) = document.as_ref() {
                    rebuild_all_with_mode(
                        doc,
                        app_mode,
                        hovered_node.as_deref(),
                        mindmap_tree,
                        app_scene,
                        renderer,
                        scene_cache,
                    );
                }
            }
        }
        Some(Action::EnterReparentMode) => {
            if let Some(doc) = document.as_ref() {
                let sel: Vec<String> = doc
                    .selection
                    .selected_ids()
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
                if !sel.is_empty() {
                    *app_mode = AppMode::Reparent { sources: sel };
                    *hovered_node = None;
                    *last_click = None;
                    rebuild_all_with_mode(
                        doc,
                        app_mode,
                        hovered_node.as_deref(),
                        mindmap_tree,
                        app_scene,
                        renderer,
                        scene_cache,
                    );
                }
            }
        }
        Some(Action::EnterConnectMode) => {
            if let Some(doc) = document.as_ref() {
                if let SelectionState::Single(source) = &doc.selection {
                    *app_mode = AppMode::Connect {
                        source: source.clone(),
                    };
                    *hovered_node = None;
                    *last_click = None;
                    rebuild_all_with_mode(
                        doc,
                        app_mode,
                        hovered_node.as_deref(),
                        mindmap_tree,
                        app_scene,
                        renderer,
                        scene_cache,
                    );
                }
            }
        }
        Some(Action::DeleteSelection) => {
            if let Some(doc) = document.as_mut() {
                if doc.apply_delete_selection() {
                    rebuild_all(doc, mindmap_tree, app_scene, renderer, scene_cache);
                }
            }
        }
        Some(Action::CreateOrphanNode) => {
            if let Some(doc) = document.as_mut() {
                let canvas_pos =
                    renderer.screen_to_canvas(cursor_pos.0 as f32, cursor_pos.1 as f32);
                doc.create_orphan_and_select(canvas_pos);
                rebuild_all(doc, mindmap_tree, app_scene, renderer, scene_cache);
            }
        }
        Some(Action::OrphanSelection) => {
            if let Some(doc) = document.as_mut() {
                if doc.apply_orphan_selection_with_undo() {
                    rebuild_all(doc, mindmap_tree, app_scene, renderer, scene_cache);
                }
            }
        }
        Some(a @ (Action::EditSelection | Action::EditSelectionClean)) => {
            // Open the text editor on the selected single node.
            // `EditSelectionClean` opens with an empty buffer
            // (the "clean slate" retype gesture);
            // `EditSelection` opens on the node's current text
            // with cursor at end. The text-editor steal at the
            // top of keyboard dispatch means these never fire
            // while the editor is already open, so
            // Enter/Backspace stay literal inside the editor.
            //
            // Portal-label selection routes to the inline
            // portal-text editor instead — same action,
            // different target type.
            let clean = matches!(a, Action::EditSelectionClean);
            if let Some(doc) = document.as_mut() {
                match doc.selection.clone() {
                    SelectionState::Single(id) => {
                        open_text_edit(
                            &id,
                            clean,
                            doc,
                            text_edit_state,
                            mindmap_tree,
                            app_scene,
                            renderer,
                        );
                    }
                    SelectionState::PortalLabel(s) | SelectionState::PortalText(s) => {
                        // Both portal sub-selections (icon + text)
                        // open the same per-endpoint text editor.
                        // The icon doesn't carry text of its own,
                        // so an `EditSelection` from a `PortalLabel`
                        // selection still authors the text label
                        // sitting next to it — matching the
                        // "edit the thing the selection refers to"
                        // intent on every other variant.
                        let er = s.edge_ref();
                        open_portal_text_edit(
                            &er,
                            &s.endpoint_node_id,
                            doc,
                            portal_text_edit_state,
                            app_scene,
                            renderer,
                        );
                    }
                    SelectionState::EdgeLabel(s) => {
                        // EdgeLabel selection → inline label editor
                        // on the owning edge's `MindEdge.label`.
                        // Same action surface as nodes / portals.
                        open_label_edit(
                            &s.edge_ref,
                            doc,
                            label_edit_state,
                            app_scene,
                            renderer,
                        );
                    }
                    _ => {}
                }
            }
        }
        Some(Action::Copy) | Some(Action::Cut) => {
            use crate::application::console::traits::{
                selection_targets, view_for, ClipboardContent, HandlesCopy, HandlesCut,
            };
            let is_cut = matches!(action, Some(Action::Cut));
            if let Some(doc) = document.as_mut() {
                let targets = selection_targets(&doc.selection);
                for tid in &targets {
                    let mut view = view_for(doc, tid);
                    let content = if is_cut {
                        view.clipboard_cut()
                    } else {
                        view.clipboard_copy()
                    };
                    if let ClipboardContent::Text(text) = content {
                        crate::application::clipboard::write_clipboard(&text);
                        break;
                    }
                }
            }
        }
        Some(Action::Paste) => {
            use crate::application::console::traits::{
                selection_targets, view_for, HandlesPaste, Outcome,
            };
            if let Some(text) = crate::application::clipboard::read_clipboard() {
                if let Some(doc) = document.as_mut() {
                    let targets = selection_targets(&doc.selection);
                    let mut any_applied = false;
                    for tid in &targets {
                        let mut view = view_for(doc, tid);
                        if let Outcome::Applied = view.clipboard_paste(&text) {
                            any_applied = true;
                        }
                    }
                    if any_applied {
                        rebuild_all(doc, mindmap_tree, app_scene, renderer, scene_cache);
                    }
                }
            }
        }
        Some(Action::SaveDocument) => {
            if let Some(doc) = document.as_mut() {
                save_document_to_bound_path(doc, console_state);
            }
        }
        Some(a) => {
            log::debug!("unhandled Document action: {:?}", a);
        }
        None => {
            // No built-in action matched — try the
            // user-defined `custom_mutation_bindings`.
            if let Some(id) = key_name.as_deref().and_then(|k| {
                keybinds
                    .custom_mutation_for(
                        k,
                        modifiers.control_key(),
                        modifiers.shift_key(),
                        modifiers.alt_key(),
                    )
                    .map(|s| s.to_string())
            }) {
                if let Some(doc) = document.as_mut() {
                    if let SelectionState::Single(nid) = doc.selection.clone() {
                        let mutation = doc.mutation_registry.get(&id).cloned();
                        if let (Some(m), Some(tree)) = (mutation, mindmap_tree.as_mut()) {
                            doc.apply_custom_mutation(&m, &nid, Some(tree));
                            scene_cache.clear();
                            rebuild_all(doc, mindmap_tree, app_scene, renderer, scene_cache);
                        }
                    }
                }
            }
        }
    }
}
