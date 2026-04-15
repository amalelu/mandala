//! Keyboard dispatch for the open picker: Esc cancels (Contextual),
//! Enter commits (Contextual) or applies-to-selection (Standalone),
//! h/H ±15° hue, s/S ±0.1 sat, v/V ±0.1 val. Unmatched keys fall
//! through to normal keybind dispatch.

use winit::keyboard::Key;

use crate::application::document::MindMapDocument;
use crate::application::renderer::Renderer;

use super::commit::{
    apply_picker_preview, cancel_color_picker, commit_color_picker,
    commit_color_picker_to_selection,
};

/// Route a keystroke to the picker. Esc cancels (contextual only;
/// ignored in standalone), Enter commits, h/H ±15° hue, s/S ±0.1
/// sat, v/V ±0.1 val. Any other key falls through to normal
/// keybind dispatch.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn handle_color_picker_key(
    key_name: &Option<String>,
    logical_key: &Key,
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    picker_dirty: &mut bool,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) -> bool {
    use crate::application::color_picker::ColorPickerState;

    let name = key_name.as_deref();
    let is_standalone = state.is_standalone();
    match name {
        Some("escape") => {
            if is_standalone {
                // Standalone mode ignores Escape — the persistent
                // palette only closes via `color picker off` from
                // the console. Don't consume the key — let it
                // flow through to normal keybind dispatch so the
                // user can e.g. close the console if they've
                // summoned it.
                return false;
            }
            cancel_color_picker(state, doc, mindmap_tree, app_scene, renderer);
            return true;
        }
        Some("enter") => {
            if is_standalone {
                // Standalone: Enter behaves like clicking ࿕ —
                // applies the current HSV to the document
                // selection, stays open.
                commit_color_picker_to_selection(
                    state,
                    doc,
                    mindmap_tree,
                    app_scene,
                    renderer,
                );
                return true;
            }
            commit_color_picker(state, doc, mindmap_tree, app_scene, renderer);
            return true;
        }
        _ => {}
    }
    // Character keys: h/s/v nudges. Use logical_key to keep this
    // case-sensitive (uppercase = bigger nudge). Non-matching
    // characters fall through so the user can e.g. press `/` to
    // open the console while the Standalone palette is active.
    if let Key::Character(c) = logical_key {
        let s = c.as_str();
        let mut changed = false;
        if let ColorPickerState::Open { hue_deg, sat, val, .. } = state {
            match s {
                "h" => {
                    *hue_deg = (*hue_deg - 15.0).rem_euclid(360.0);
                    changed = true;
                }
                "H" => {
                    *hue_deg = (*hue_deg + 15.0).rem_euclid(360.0);
                    changed = true;
                }
                "s" => {
                    *sat = (*sat - 0.1).clamp(0.0, 1.0);
                    changed = true;
                }
                "S" => {
                    *sat = (*sat + 0.1).clamp(0.0, 1.0);
                    changed = true;
                }
                "v" => {
                    *val = (*val - 0.1).clamp(0.0, 1.0);
                    changed = true;
                }
                "V" => {
                    *val = (*val + 0.1).clamp(0.0, 1.0);
                    changed = true;
                }
                _ => {}
            }
        }
        if changed {
            apply_picker_preview(state, doc, picker_dirty);
            return true;
        }
        // Character key but not one of ours — fall through.
        return false;
    }
    // Any non-character key that didn't match an explicit arm
    // above (arrow keys, function keys, modifier-only, etc.) —
    // let it pass through to normal keybind dispatch.
    false
}
