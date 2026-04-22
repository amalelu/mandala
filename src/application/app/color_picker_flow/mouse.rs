//! Mouse-move dispatch: feed active drag gestures on one branch,
//! hit-test and update hover_preview / hovered_hit on the other.
//! Delegates to `apply_picker_preview` when the preview changes so
//! the document's transient preview stays in sync with the hover
//! color. The real `hue_deg`/`sat`/`val` only change on click or
//! keyboard nudge — hover is purely visual.

use crate::application::document::MindMapDocument;

use super::commit::apply_picker_preview;
use super::super::throttled_interaction::ColorPickerHoverInteraction;

/// Mouse-move handler for the picker. Branches on active-drag vs
/// hover:
///
/// - **Drag active**: translate the wheel so
///   `center = cursor + grab_offset`. Every layout position (ring,
///   bars, chips, backdrop) rebuilds against the new center via
///   `center_override`.
/// - **Hover**: hit-test the cursor, set `hover_preview` to the
///   hovered cell's HSV (visual preview only — no mutation of the
///   selected `hue_deg`/`sat`/`val`), and record `hovered_hit`
///   for the renderer's hover-grow effect.
///
/// Returns `true` when the picker consumed the move and the caller
/// should stop dispatching it. Returns `false` when the move
/// should fall through to normal canvas hover — the Standalone
/// palette with no active gesture and the cursor outside its
/// backdrop is the one case today, so the user can still see
/// button-node cursor changes on the canvas while the palette
/// floats above it.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn handle_color_picker_mouse_move(
    cursor_pos: (f64, f64),
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    picker_hover: &mut ColorPickerHoverInteraction,
) -> bool {
    use crate::application::color_picker::{
        hit_test_picker, hue_slot_to_degrees, sat_cell_to_value, val_cell_to_value,
        ColorPickerState, PickerHit,
    };

    // Always record the cursor position on the state before hit-
    // testing — `compute_picker_geometry` reads it to toggle
    // `hex_visible` based on "cursor inside backdrop". A move that
    // doesn't hit any interactive element still needs this update so
    // the hex readout can appear/disappear as the cursor crosses the
    // backdrop boundary.
    let cursor = (cursor_pos.0 as f32, cursor_pos.1 as f32);
    if let ColorPickerState::Open { last_cursor_pos, .. } = state {
        *last_cursor_pos = Some(cursor);
    }

    // Active gesture takes priority: while the wheel is being
    // dragged or resized, every cursor move feeds the gesture
    // instead of hit-testing for hover. The two gestures are
    // mutually exclusive — `gesture` holds at most one variant.
    if let ColorPickerState::Open {
        gesture: Some(g),
        center_override,
        size_scale,
        ..
    } = state
    {
        match *g {
            crate::application::color_picker::PickerGesture::Move { grab_offset } => {
                let new_center = (cursor.0 + grab_offset.0, cursor.1 + grab_offset.1);
                *center_override = Some(new_center);
            }
            crate::application::color_picker::PickerGesture::Resize {
                anchor_radius,
                anchor_scale,
                anchor_center,
            } => {
                // Multiplicative scale change: new_scale =
                // anchor_scale * (current_radius / anchor_radius),
                // floored on the input side at the same `font * 3`
                // anchor_radius cap so the ratio stays well-behaved
                // throughout the gesture. Clamps from the spec.
                let dx = cursor.0 - anchor_center.0;
                let dy = cursor.1 - anchor_center.1;
                let raw_r = (dx * dx + dy * dy).sqrt();
                let r_now = raw_r.max(anchor_radius * 0.1);
                let geom =
                    &crate::application::widgets::color_picker_widget::load_spec().geometry;
                *size_scale = (anchor_scale * (r_now / anchor_radius))
                    .clamp(geom.resize_scale_min, geom.resize_scale_max);
            }
        }
        picker_hover.dirty = true;
        return true;
    }

    let hit = if let ColorPickerState::Open { layout: Some(layout), .. } = state {
        hit_test_picker(layout, cursor.0, cursor.1)
    } else {
        // Picker closed, or open but the first rebuild hasn't happened
        // yet — no cached layout to hit-test against. The open path
        // always rebuilds before releasing control, so this branch is
        // only reachable during the ~1-line window between construction
        // and the first rebuild call.
        return false;
    };

    // Standalone mode + cursor outside the backdrop: don't consume
    // the move. The canvas underneath should still update its own
    // hover state (button-node cursor, etc.) — the persistent
    // palette is meant to coexist with ordinary canvas work, not
    // block it.
    if state.is_standalone() && matches!(hit, PickerHit::Outside) {
        return false;
    }

    // Only mark dirty when the picker's interactive state
    // actually moved. Mouse events arrive at ~120 Hz and the
    // user can drag many cursor pixels within the same hue
    // slot or sat/val cell; without this gate the throttle
    // runs full-canvas rebuilds for cursor jiggle that has no
    // visible effect, and the cross feels laggier than the
    // wheel because cells are smaller (more boundary crossings
    // per visit).
    let mut state_changed = false;
    if let ColorPickerState::Open {
        hue_deg,
        sat,
        val,
        hovered_hit,
        hover_preview,
        ..
    } = state
    {
        // Track hover changes for hover-grow. Any change in the
        // hit region (e.g. moving from hue slot 3 to slot 4, or
        // from a ring glyph onto the empty backdrop) flips the
        // hovered_hit and triggers a rebuild.
        let new_hover = match hit {
            PickerHit::Hue(_)
            | PickerHit::SatCell(_)
            | PickerHit::ValCell(_)
            | PickerHit::Commit => Some(hit),
            // DragAnchor / Outside are not hoverable targets —
            // they don't grow on hover.
            PickerHit::DragAnchor | PickerHit::Outside => None,
        };
        if *hovered_hit != new_hover {
            *hovered_hit = new_hover;
            state_changed = true;
        }

        // Compute the hover preview triple. The actual
        // `hue_deg`/`sat`/`val` only change on click or keyboard
        // nudge — hover just sets a transient preview the
        // rendering pipeline reads for visual feedback.
        let new_preview = match hit {
            PickerHit::Hue(slot) => {
                Some((hue_slot_to_degrees(slot), *sat, *val))
            }
            PickerHit::SatCell(i) => {
                Some((*hue_deg, sat_cell_to_value(i), *val))
            }
            PickerHit::ValCell(i) => {
                Some((*hue_deg, *sat, val_cell_to_value(i)))
            }
            PickerHit::Commit | PickerHit::DragAnchor | PickerHit::Outside => None,
        };
        if *hover_preview != new_preview {
            *hover_preview = new_preview;
            state_changed = true;
        }
    }

    // The hex readout's visibility depends on cursor position
    // crossing the backdrop boundary. We always update
    // `last_cursor_pos` above, so a subsequent `state_changed`
    // event will pick up the right `hex_visible` value. Pure
    // cursor wiggles inside the same cell don't redraw the hex
    // — which is fine: the readout was already showing the
    // current value.
    if state_changed {
        picker_hover.dirty = true;
        // Preview the updated HSV onto the (possibly contextual)
        // target so the map reflects the hover color live. No-op
        // in Standalone mode — no bound target — but the ࿕ glyph
        // in the wheel still shows the current HSV so the user
        // gets immediate color feedback on the wheel itself.
        apply_picker_preview(state, doc, picker_hover);
    }
    true
}
