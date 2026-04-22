//! Keyboard dispatch for the open picker. Routes keystrokes through
//! the contextual keybind resolver (`InputContext::ColorPicker`) so
//! all picker keys are user-customizable. Unmatched keys fall
//! through to the Document context via `InputContext::falls_through`.

use crate::application::clipboard;
use crate::application::color_picker::ColorPickerState;
use crate::application::console::traits::{ClipboardContent, HandlesCopy, HandlesCut, HandlesPaste, Outcome};
use crate::application::document::MindMapDocument;
use crate::application::keybinds::{Action, InputContext, ResolvedKeybinds};
use crate::application::renderer::Renderer;

use super::commit::{
    apply_picker_preview, cancel_color_picker, commit_color_picker,
    commit_color_picker_to_selection,
};
use super::super::throttled_interaction::ColorPickerHoverInteraction;

/// Route a keystroke to the picker via `action_for_context`. Returns
/// `true` if the key was consumed, `false` to let it fall through
/// to the event loop's normal dispatch.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn handle_color_picker_key(
    key_name: &Option<String>,
    ctrl: bool,
    shift: bool,
    alt: bool,
    keybinds: &ResolvedKeybinds,
    state: &mut ColorPickerState,
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    picker_hover: &mut ColorPickerHoverInteraction,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) -> bool {
    let name = match key_name.as_deref() {
        Some(n) => n,
        None => return false,
    };

    let action = keybinds.action_for_context(
        InputContext::ColorPicker, name, ctrl, shift, alt,
    );

    match action {
        Some(Action::Copy) => {
            if let ClipboardContent::Text(hex) = state.clipboard_copy() {
                clipboard::write_clipboard(&hex);
            }
            true
        }
        Some(Action::Paste) => {
            if let Some(text) = clipboard::read_clipboard() {
                if let Outcome::Applied = state.clipboard_paste(&text) {
                    apply_picker_preview(state, doc, picker_hover);
                }
            }
            true
        }
        Some(Action::Cut) => {
            if let ClipboardContent::Text(hex) = state.clipboard_cut() {
                clipboard::write_clipboard(&hex);
            }
            true
        }
        Some(Action::PickerCancel) => {
            if state.is_standalone() {
                return false;
            }
            cancel_color_picker(state, doc, mindmap_tree, app_scene, renderer);
            true
        }
        Some(Action::PickerCommit) => {
            if state.is_standalone() {
                commit_color_picker_to_selection(
                    state, doc, mindmap_tree, app_scene, renderer,
                );
            } else {
                commit_color_picker(state, doc, mindmap_tree, app_scene, renderer);
            }
            true
        }
        Some(Action::PickerNudgeHueDown) => {
            nudge_picker(state, doc, picker_hover, |h, _, _| {
                *h = (*h - 15.0).rem_euclid(360.0);
            })
        }
        Some(Action::PickerNudgeHueUp) => {
            nudge_picker(state, doc, picker_hover, |h, _, _| {
                *h = (*h + 15.0).rem_euclid(360.0);
            })
        }
        Some(Action::PickerNudgeSatDown) => {
            nudge_picker(state, doc, picker_hover, |_, s, _| {
                *s = (*s - 0.1).clamp(0.0, 1.0);
            })
        }
        Some(Action::PickerNudgeSatUp) => {
            nudge_picker(state, doc, picker_hover, |_, s, _| {
                *s = (*s + 0.1).clamp(0.0, 1.0);
            })
        }
        Some(Action::PickerNudgeValDown) => {
            nudge_picker(state, doc, picker_hover, |_, _, v| {
                *v = (*v - 0.1).clamp(0.0, 1.0);
            })
        }
        Some(Action::PickerNudgeValUp) => {
            nudge_picker(state, doc, picker_hover, |_, _, v| {
                *v = (*v + 0.1).clamp(0.0, 1.0);
            })
        }
        Some(_) => {
            // A Document-level action fell through — let the event
            // loop handle it via normal dispatch.
            false
        }
        None => false,
    }
}

fn nudge_picker(
    state: &mut ColorPickerState,
    doc: &mut MindMapDocument,
    picker_hover: &mut ColorPickerHoverInteraction,
    f: impl FnOnce(&mut f32, &mut f32, &mut f32),
) -> bool {
    if let ColorPickerState::Open { hue_deg, sat, val, hover_preview, .. } = state {
        f(hue_deg, sat, val);
        *hover_preview = None;
        apply_picker_preview(state, doc, picker_hover);
        true
    } else {
        false
    }
}
