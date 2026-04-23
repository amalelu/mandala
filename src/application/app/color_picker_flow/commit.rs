//! Picker lifecycle terminals: cancel, close-standalone, commit
//! (single-target / selection fan-out) and the hover-preview stamp
//! that feeds `doc.color_picker_preview` during mouse-move.

use crate::application::document::{EdgeRef, MindMapDocument};
use crate::application::renderer::Renderer;

use super::super::rebuild_all;
use super::super::throttled_interaction::ColorPickerHoverInteraction;

/// Cancel the picker: clear the transient document preview and
/// close the modal. The committed model is untouched because the
/// new preview path never writes to it — the entire hover / cancel
/// flow is a pure scene-level substitution.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn cancel_color_picker(
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    use crate::application::color_picker::ColorPickerState;

    if matches!(state, ColorPickerState::Closed) {
        return;
    }
    *state = ColorPickerState::Closed;
    doc.color_picker_preview = None;
    renderer.rebuild_color_picker_overlay_buffers(app_scene, None);
    rebuild_all(doc, mindmap_tree, app_scene, renderer, scene_cache);
}

/// Close the standalone color picker without committing. Called by
/// the `color picker off` console command. Functionally identical to
/// `cancel_color_picker` — both close the picker and clear the
/// transient preview — but named distinctly because Standalone mode
/// has no "original" to cancel back to; the function exists so
/// call-sites read clearly.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn close_color_picker_standalone(
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    cancel_color_picker(state, doc, mindmap_tree, app_scene, renderer, scene_cache);
}

/// Commit the picker's currently-previewed HSV value via the regular
/// `set_edge_color` / `set_node_*_color` path — a single undo entry
/// is pushed and `ensure_glyph_connection` runs its fork-on-first-edit
/// only at this moment (never during hover). Close the modal.
///
/// The picker only commits concrete HSV hex values now that the
/// theme-variable chip row has been retired; theme-variable editing
/// lives elsewhere in the UI.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn commit_color_picker(
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    use crate::application::color_picker::{ColorPickerState, NodeColorAxis, PickerHandle};
    use baumhard::util::color::hsv_to_hex;

    let (handle, hue_deg, sat, val) = match state {
        ColorPickerState::Open {
            mode: crate::application::color_picker::PickerMode::Contextual { handle },
            hue_deg,
            sat,
            val,
            ..
        } => (handle.clone(), *hue_deg, *sat, *val),
        // Standalone mode has no bound target — commit is handled by
        // `commit_color_picker_to_selection` instead; this function
        // is Contextual-only. Being reached in Standalone mode means
        // the caller picked the wrong commit path.
        ColorPickerState::Open { .. } => {
            log::warn!(
                "commit_color_picker called in non-contextual mode; \
                 use commit_color_picker_to_selection for Standalone mode"
            );
            return;
        }
        ColorPickerState::Closed => return,
    };

    // Close the modal state first so the subsequent rebuilds don't
    // re-apply the preview.
    *state = ColorPickerState::Closed;
    doc.color_picker_preview = None;

    let hex = hsv_to_hex(hue_deg, sat, val);
    match handle {
        PickerHandle::Edge(index) => {
            let er = doc
                .mindmap
                .edges
                .get(index)
                .map(|e| EdgeRef::new(&e.from_id, &e.to_id, &e.edge_type));
            if let Some(er) = er {
                doc.set_edge_color(&er, Some(&hex));
            }
        }
        PickerHandle::Node { id, axis } => match axis {
            NodeColorAxis::Bg => {
                doc.set_node_bg_color(&id, hex);
            }
            NodeColorAxis::Text => {
                doc.set_node_text_color(&id, hex);
            }
            NodeColorAxis::Border => {
                doc.set_node_border_color(&id, hex);
            }
        },
    }

    renderer.rebuild_color_picker_overlay_buffers(app_scene, None);
    // `set_edge_color` / `set_node_*_color` mutate edge/node color
    // fields that `build_scene_with_cache` caches per-edge (body
    // glyph, color, font). Clear so the rebuild re-samples against
    // the committed model.
    scene_cache.clear();
    rebuild_all(doc, mindmap_tree, app_scene, renderer, scene_cache);
}

/// Apply the current picker HSV to the document's transient color
/// preview, then rebuild only the scene (not the node tree, which
/// didn't change) + the picker overlay. Hot path: no ref resolution,
/// no model mutation, no snapshot. The scene builder reads the
/// preview via `doc.color_picker_preview` and substitutes it in
/// during emission.
///
/// Marks `picker_hover.dirty` so the per-frame throttle picks up
/// the change on its next drain — every mouse-move on the wheel
/// routes through here, and unguarded rebuilds would re-shape
/// every border / connection / portal on the map at ~120 Hz.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn apply_picker_preview(
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    picker_hover: &mut ColorPickerHoverInteraction,
) {
    use crate::application::color_picker::{ColorPickerState, PickerHandle};
    use crate::application::document::ColorPickerPreview;
    use baumhard::util::color::hsv_to_hex;

    let (handle, eff_hue, eff_sat, eff_val) = match state {
        ColorPickerState::Open {
            mode,
            hue_deg,
            sat,
            val,
            hover_preview,
            ..
        } => {
            let handle = match mode {
                crate::application::color_picker::PickerMode::Contextual { handle } => {
                    Some(handle.clone())
                }
                // Standalone mode has no bound target — nothing to
                // preview on the scene. The ࿕ glyph in the wheel
                // still shows the current HSV (rendered by the picker
                // overlay itself), so the user gets immediate
                // feedback without needing doc.color_picker_preview.
                crate::application::color_picker::PickerMode::Standalone => None,
            };
            let (eh, es, ev) = hover_preview.unwrap_or((*hue_deg, *sat, *val));
            (handle, eh, es, ev)
        }
        ColorPickerState::Closed => return,
    };
    let hex = hsv_to_hex(eff_hue, eff_sat, eff_val);
    if let Some(handle) = handle {
        match handle {
            PickerHandle::Edge(index) => {
                if let Some(edge) = doc.mindmap.edges.get(index) {
                    let key = baumhard::mindmap::scene_cache::EdgeKey::from_edge(edge);
                    doc.color_picker_preview =
                        Some(ColorPickerPreview::Edge { key, color: hex });
                }
            }
            PickerHandle::Node { .. } => {
                // Node preview lives on the tree pipeline, not the
                // scene pipeline — not yet wired. Commit-only for v1.
            }
        }
    }
    // Scene + picker rebuilds are deferred to the `AboutToWait`
    // drain via `picker_hover.dirty`. Mouse moves come in at
    // ~120Hz on modern hardware; without this gate every event
    // would re-shape every border / connection / portal on the
    // map plus the picker overlay. The drain is gated by
    // `picker_hover.throttle` (the same `MutationFrequencyThrottle`
    // type the drag path uses), which self-tunes to keep the
    // per-frame work under the refresh budget.
    picker_hover.dirty = true;
    // Additionally flag the canvas dirty: `doc.color_picker_preview`
    // drives a per-edge color override that the scene builder reads
    // during emission. Only `apply_picker_preview` writes to that
    // preview — gesture-only paths (Move / Resize in `mouse.rs`)
    // leave it clear, which is what lets the drain skip
    // `rebuild_scene_only` during a wheel drag. Keyboard nudges,
    // however, land here even mid-drag; they must still trigger the
    // canvas rebuild so the targeted edge repaints.
    picker_hover.canvas_dirty = true;
}

/// Commit the picker's current HSV to every colorable item in the
/// document's current selection. Standalone mode's core gesture.
///
/// Dispatches through the `AcceptsWheelColor` trait: each component
/// type declares its own default color channel (nodes → bg, edges →
/// their single color field). The picker doesn't decide — the
/// component does. Empty selection → fire the error-flash animation
/// hook and do nothing.
///
/// Multi-select applies in a single pass — one undo entry per item
/// (grouped undo is a future refinement when `UndoAction::Group`
/// lands in the document layer).
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn commit_color_picker_to_selection(
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    use crate::application::color_picker::{request_error_flash, ColorPickerState, FlashKind};
    use crate::application::console::traits::{
        selection_targets, view_for, AcceptsWheelColor, ColorValue, Outcome,
    };
    use baumhard::util::color::hsv_to_hex;

    let (hue_deg, sat, val) = match state {
        ColorPickerState::Open {
            hue_deg, sat, val, ..
        } => (*hue_deg, *sat, *val),
        ColorPickerState::Closed => return,
    };
    let color = ColorValue::Hex(hsv_to_hex(hue_deg, sat, val));

    let targets = selection_targets(&doc.selection);
    if targets.is_empty() {
        // The user pressed ࿕ with nothing selected. Fire the
        // animation hook (no-op stub today; picks up when the
        // animation pipeline lands) so the wheel flashes red.
        request_error_flash(state, FlashKind::Error);
        return;
    }

    // Fan out across the selection, letting each component decide
    // which channel the wheel color lands on. A fresh `TargetView`
    // per iteration so no two views alias the doc borrow.
    let mut any_accepted = false;
    for tid in &targets {
        let mut view = view_for(doc, tid);
        match view.apply_wheel_color(color.clone()) {
            Outcome::Applied | Outcome::Unchanged => any_accepted = true,
            Outcome::NotApplicable | Outcome::Invalid(_) => {}
        }
    }

    if any_accepted {
        // Same rationale as `commit_color_picker`: the wheel-color
        // writes land on cached edge fields, so clear before the
        // rebuild.
        scene_cache.clear();
        // Rebuild the whole scene so the newly-colored items repaint
        // next frame. The picker itself stays open — no state change
        // needed on `state`.
        rebuild_all(doc, mindmap_tree, app_scene, renderer, scene_cache);
    }
}
