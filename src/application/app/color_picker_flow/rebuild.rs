// SPDX-License-Identifier: MPL-2.0

//! Picker overlay rebuild dispatcher. The event loop calls this each
//! frame the picker is open; it picks between first-build, layout-phase
//! mutator, dynamic-phase mutator, and unregister paths by comparing
//! the current geometry against the cached layout.

use crate::application::document::MindMapDocument;
use crate::application::renderer::Renderer;

use super::geometry::compute_picker_geometry;

/// Picker overlay update entry point. Dispatches between the
/// initial-build path and the §B2-compliant in-place mutator paths:
///
/// - **Closed** (`compute_picker_geometry` returns `None`): unregister
///   the overlay tree by passing `None` to the buffer rebuild.
/// - **First open** (no tree registered): build a fresh tree via
///   `Renderer::rebuild_color_picker_overlay_buffers`. The initial
///   build *is* the layout phase, so dynamic frames after this can
///   safely target the just-built static fields.
/// - **Layout changed** (resize, RMB drag-to-resize, drag-to-move
///   repositioning): apply the layout-phase mutator —
///   `Renderer::apply_color_picker_overlay_mutator` — which writes
///   every variable field on every cell via `Assign` deltas.
/// - **Layout unchanged** (per-frame hover / HSV / chip / drag-Move
///   without geometry change): apply the dynamic-phase mutator —
///   `Renderer::apply_color_picker_overlay_dynamic_mutator` — which
///   writes only the fields that genuinely move per frame
///   (`ColorFontRegions`, `scale`, hex `Text`).
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn rebuild_color_picker_overlay(
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &MindMapDocument,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    use crate::application::color_picker::{ColorPickerState, PickerDynamicApplyKey};
    use crate::application::scene_host::OverlayRole;
    let surface_size = (renderer.surface_width() as f32, renderer.surface_height() as f32);
    // Selection hint surfaces "section K of <node>" / "node <id>" /
    // "{count} nodes" / "edge" / "(no selection)" in the Standalone
    // picker title so the user can see at a glance what a wheel
    // commit will land on. Contextual mode skips this — the title
    // already shows the bound target's label
    // (`PickerHandle::label`).
    let selection_hint = if state.is_standalone() {
        Some(standalone_selection_hint(&doc.selection))
    } else {
        None
    };
    let Some((geometry, layout_changed)) = compute_picker_geometry(state, surface_size, selection_hint)
    else {
        renderer.rebuild_color_picker_overlay_buffers(app_scene, None);
        return;
    };
    // Compute the key the dynamic path would write against, for the
    // state-change short-circuit below. Captured here while we still
    // own `geometry` — the dispatch branches consume it.
    let apply_key = PickerDynamicApplyKey {
        hue_deg: geometry.hue_deg,
        sat: geometry.sat,
        val: geometry.val,
        hovered_hit: geometry.hovered_hit,
        hex_visible: geometry.hex_visible,
    };
    // Split the Open variant into disjoint field borrows so we can
    // read `layout` and write `last_dynamic_apply` concurrently.
    let ColorPickerState::Open {
        layout: state_layout,
        last_dynamic_apply,
        ..
    } = state
    else {
        return;
    };
    let Some(layout) = state_layout.as_ref() else {
        return;
    };
    let registered = app_scene.overlay_id(OverlayRole::ColorPicker).is_some();
    if registered {
        if layout_changed {
            renderer.apply_color_picker_overlay_mutator(app_scene, &geometry, layout);
            // Layout rewrite stamps every field on every cell; seed
            // the short-circuit cache with the just-applied key.
            *last_dynamic_apply = Some(apply_key);
        } else {
            // Dynamic-apply short-circuit: nothing observable the
            // dynamic spec touches has changed since the last apply,
            // so its output is still correct. Cheap bail-out — cursor
            // moves within one cell trigger this routinely.
            if *last_dynamic_apply == Some(apply_key) {
                return;
            }
            renderer.apply_color_picker_overlay_dynamic_mutator(app_scene, &geometry, layout);
            *last_dynamic_apply = Some(apply_key);
        }
    } else {
        renderer.rebuild_color_picker_overlay_buffers(app_scene, Some((&geometry, layout)));
        // First build doubles as the layout phase; seed the cache so
        // the next stable-geometry frame short-circuits.
        *last_dynamic_apply = Some(apply_key);
    }
}

/// One-line selection identity for the Standalone picker title. The
/// shapes are deliberately narrow:
/// - `Section` → `"section K of <node_id>"`
/// - `Single`  → `"node <id>"`
/// - `Multi`   → `"<count> nodes"`
/// - `Edge`    → `"edge"`
/// - `EdgeLabel` / `PortalLabel` / `PortalText` → kind-only label
/// - `None`    → `"(no selection)"`
///
/// The Standalone wheel commit fans out via `selection_targets` →
/// `apply_wheel_color`, so what the user sees in the title bar is
/// exactly what will receive the commit. Mirrors
/// `PickerHandle::label` for the bound-target case but adds the
/// indices the dynamic shape needs to make a section commit
/// distinguishable from its owning-node commit at a glance.
#[cfg(not(target_arch = "wasm32"))]
fn standalone_selection_hint(selection: &crate::application::document::SelectionState) -> String {
    use crate::application::document::SelectionState;
    match selection {
        SelectionState::None => "(no selection)".to_string(),
        SelectionState::Single(id) => format!("node {}", id),
        SelectionState::Multi(ids) => format!("{} nodes", ids.len()),
        SelectionState::Section(s) => format!("section {} of {}", s.section_idx, s.node_id),
        SelectionState::MultiSection(secs) => format!("{} sections", secs.len()),
        SelectionState::SectionRange { sel, range } => {
            format!(
                "section {} of {} range {}..{}",
                sel.section_idx, sel.node_id, range.0, range.1
            )
        }
        SelectionState::Edge(_) => "edge".to_string(),
        SelectionState::EdgeLabel(_) => "edge label".to_string(),
        SelectionState::PortalLabel(_) => "portal label".to_string(),
        SelectionState::PortalText(_) => "portal text".to_string(),
    }
}
