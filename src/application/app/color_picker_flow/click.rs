//! Click dispatch + gesture end: LMB/RMB hit-test routing, wheel
//! commit, standalone selection commit, drag-anchor gesture start,
//! and mouse-up gesture release.

use winit::event::MouseButton;

use crate::application::document::MindMapDocument;
use crate::application::renderer::Renderer;

use super::commit::{
    apply_picker_preview, cancel_color_picker, commit_color_picker,
    commit_color_picker_to_selection,
};

/// Click handler for the picker. Semantics:
///
/// - **Hue / SatCell / ValCell** — select the hovered value.
///   Copies the cell's HSV component into the picker's selected
///   `hue_deg`/`sat`/`val` and clears `hover_preview`. The wheel
///   stays open — users can click around freely to build up a
///   color before committing.
/// - **Commit** (࿕) —
///   - Contextual: commit current HSV to the bound target, close.
///   - Standalone: apply current HSV to each item in the document
///     selection; stay open. If the selection is empty, trigger the
///     error-flash animation hook.
/// - **DragAnchor** —
///   - LMB → start a wheel-move gesture (translates `center_override`).
///   - RMB → start a wheel-resize gesture (mutates `size_scale`).
///   The mouse-up event ends either gesture via
///   `end_color_picker_gesture`.
/// - **Outside** —
///   - Contextual: cancel (restore original), close.
///   - Standalone: ignored (the persistent palette only closes via
///     `color picker off`).
///
/// `button` is `MouseButton::Left` or `MouseButton::Right`. The
/// caller (the `WindowEvent::MouseInput` branch) filters out other
/// buttons before reaching here.
///
/// Returns `true` if the click was consumed by the picker and the
/// caller should stop dispatching it. Returns `false` when the
/// click should fall through to normal canvas dispatch — the only
/// such case today is a Standalone-mode outside-backdrop click,
/// where the persistent palette needs to coexist with the user
/// interacting with the canvas underneath it.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn handle_color_picker_click(
    cursor_pos: (f64, f64),
    button: MouseButton,
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    picker_dirty: &mut bool,
) -> bool {
    use crate::application::color_picker::{
        hit_test_picker, hue_slot_to_degrees, sat_cell_to_value, val_cell_to_value,
        ColorPickerState, PickerGesture, PickerHit,
    };

    let hit = if let ColorPickerState::Open { layout: Some(layout), .. } = state {
        hit_test_picker(layout, cursor_pos.0 as f32, cursor_pos.1 as f32)
    } else {
        return false;
    };

    // RMB outside the DragAnchor region is a no-op for now — only
    // the empty region of the wheel disk (inside the circle, off
    // every interactive glyph) acts as a resize handle. That keeps
    // the gesture predictable: RMB on a hue/sat/val cell or a chip
    // doesn't accidentally resize while the user is also reading
    // the live preview. In Standalone mode we return `false` so
    // the RMB can reach any future right-click menu on the canvas.
    if button == MouseButton::Right && !matches!(hit, PickerHit::DragAnchor) {
        return !state.is_standalone();
    }

    let is_standalone = state.is_standalone();

    match hit {
        PickerHit::Outside => {
            if is_standalone {
                // Standalone mode: the persistent palette only
                // closes via `color picker off`. Don't consume the
                // click — let it flow through to the canvas so the
                // user can still select nodes, create edges, etc.
                return false;
            }
            // Contextual mode: click outside cancels.
            cancel_color_picker(state, doc, mindmap_tree, app_scene, renderer);
        }
        PickerHit::Hue(slot) => {
            if let ColorPickerState::Open { hue_deg, hover_preview, .. } = state {
                *hue_deg = hue_slot_to_degrees(slot);
                *hover_preview = None;
            }
            apply_picker_preview(state, doc, picker_dirty);
        }
        PickerHit::SatCell(i) => {
            if let ColorPickerState::Open { sat, hover_preview, .. } = state {
                *sat = sat_cell_to_value(i);
                *hover_preview = None;
            }
            apply_picker_preview(state, doc, picker_dirty);
        }
        PickerHit::ValCell(i) => {
            if let ColorPickerState::Open { val, hover_preview, .. } = state {
                *val = val_cell_to_value(i);
                *hover_preview = None;
            }
            apply_picker_preview(state, doc, picker_dirty);
        }
        PickerHit::Commit => {
            if is_standalone {
                // Standalone mode: apply the current HSV to each
                // item in the selection. Stay open.
                commit_color_picker_to_selection(
                    state,
                    doc,
                    mindmap_tree,
                    app_scene,
                    renderer,
                );
            } else {
                // Contextual mode: commit to the bound target,
                // close.
                commit_color_picker(state, doc, mindmap_tree, app_scene, renderer);
            }
        }
        PickerHit::DragAnchor => {
            // Start a gesture from anywhere inside the wheel disk
            // that's not on an interactive glyph (hit_test_picker
            // now gates on the circle, not the backdrop rect).
            // LMB → move (translate center_override); RMB → resize
            // (mutate size_scale). The two gestures are mutually
            // exclusive by construction — `gesture` only holds one
            // variant.
            if let ColorPickerState::Open {
                layout: Some(layout),
                gesture,
                size_scale,
                ..
            } = state
            {
                let cursor = (cursor_pos.0 as f32, cursor_pos.1 as f32);
                *gesture = Some(match button {
                    MouseButton::Left => PickerGesture::Move {
                        grab_offset: (
                            layout.center.0 - cursor.0,
                            layout.center.1 - cursor.1,
                        ),
                    },
                    MouseButton::Right => {
                        // Floor the anchor radius so a grab very
                        // near the wheel center doesn't make a 1px
                        // cursor move into a 100% scale change.
                        // `font_size * 3.0` is comfortably outside
                        // the central ࿕ commit button's hit
                        // radius (`preview_size * 0.45`), so the
                        // floor is rarely hit in practice anyway.
                        let dx = cursor.0 - layout.center.0;
                        let dy = cursor.1 - layout.center.1;
                        let raw_r = (dx * dx + dy * dy).sqrt();
                        let anchor_radius = raw_r.max(layout.font_size * 3.0);
                        PickerGesture::Resize {
                            anchor_radius,
                            anchor_scale: *size_scale,
                            anchor_center: layout.center,
                        }
                    }
                    // Other buttons can't reach here — caller
                    // filters to Left/Right before dispatching.
                    _ => return false,
                });
            }
        }
    }
    true
}

/// End an active picker gesture. Called on mouse-up while the
/// picker is open. Returns `true` if a gesture was active and the
/// caller should treat the release as consumed. Returns `false`
/// when no gesture was active (e.g. Standalone-mode press that
/// fell through to the canvas) so the release also falls through.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn end_color_picker_gesture(
    state: &mut crate::application::color_picker::ColorPickerState,
) -> bool {
    use crate::application::color_picker::ColorPickerState;
    if let ColorPickerState::Open { gesture, .. } = state {
        let was_active = gesture.is_some();
        *gesture = None;
        was_active
    } else {
        false
    }
}
