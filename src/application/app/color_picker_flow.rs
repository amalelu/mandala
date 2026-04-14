//! Glyph-wheel color picker flow: open / commit / cancel / per-frame
//! mouse + keyboard handlers + the §B2 dispatcher
//! (`rebuild_color_picker_overlay`) the event loop calls each frame.
//!
//! Pulled out of `app/mod.rs` so the picker's HSV / handle-resolution
//! logic doesn't bloat the event loop. Public surface stays
//! `pub(super)` — `console_input` calls
//! `open_color_picker_contextual` / `_standalone` /
//! `cancel_color_picker` / `close_color_picker_standalone`, and the
//! event loop calls every other entry point in the file.

use glam::Vec2;
use winit::event::MouseButton;
use winit::keyboard::Key;

use crate::application::common::InputMode;
use crate::application::document::{EdgeRef, MindMapDocument};
use crate::application::renderer::Renderer;

use super::{rebuild_all, rebuild_scene_only};

// =====================================================================
// Glyph-wheel color picker handlers
// =====================================================================

/// Open the color picker in contextual mode, bound to the given
/// target. Resolves the target ref to a concrete handle, seeds HSV
/// from the target's currently-displayed color, and shows the
/// modal-style wheel. Commit writes to the bound target; Esc and
/// outside-click cancel (restore the original). See
/// [`open_color_picker_standalone`] for the persistent-palette flavor
/// that writes to the document's current selection.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn open_color_picker_contextual(
    target: crate::application::color_picker::ColorTarget,
    doc: &mut MindMapDocument,
    state: &mut crate::application::color_picker::ColorPickerState,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    use crate::application::color_picker::{current_hsv_at, PickerMode};

    // Resolve the target to a picker handle up front. If the
    // edge / portal / node was deleted between the open trigger
    // and Enter being pressed, warn and bail — the picker never
    // opens. Should never happen because the dispatcher runs
    // synchronously, but defensive.
    let handle = match target.resolve(doc) {
        Some(h) => h,
        None => {
            log::warn!("color picker: target ref did not resolve; ignoring open");
            return;
        }
    };

    // Seed HSV from the currently-displayed (possibly theme-resolved)
    // color so the picker opens right where the user already is.
    let hsv = current_hsv_at(doc, &handle);

    // Seed the document preview so the initial render already shows
    // the same HSV the picker opened at. Overwritten on the next
    // hover frame, but this avoids a one-frame flash of the original
    // color when the picker opens. Nodes don't have a scene-builder
    // preview path yet (commit-only for the first version of the
    // `color bg/text/border` picker flow), so the helper is a no-op
    // for the Node arm.
    seed_initial_preview(doc, &handle, hsv.0, hsv.1, hsv.2);

    open_picker_inner(
        PickerMode::Contextual { handle },
        hsv,
        doc,
        state,
        app_scene,
        renderer,
    );
}

/// Open the color picker in standalone mode — a persistent palette
/// that applies the current HSV to the document's selection on ࿕
/// click and stays open until dismissed via `color picker off`.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn open_color_picker_standalone(
    doc: &mut MindMapDocument,
    state: &mut crate::application::color_picker::ColorPickerState,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    use crate::application::color_picker::PickerMode;

    // Seed to a plausible starting color (red, full saturation, full
    // value). The user will nudge within seconds, so the exact seed
    // doesn't matter much — red-at-the-top matches the hue ring's
    // 12-o'clock slot so the wheel opens with the ring's "start" cell
    // highlighted.
    let hsv = (0.0_f32, 1.0_f32, 1.0_f32);
    open_picker_inner(PickerMode::Standalone, hsv, doc, state, app_scene, renderer);
}

/// Shared picker-open core: measures glyph advances (one font-system
/// lock per open, amortized across the whole session) and writes the
/// `Open` state. Split out so Contextual and Standalone modes share a
/// single measurement pass and state-init shape.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn open_picker_inner(
    mode: crate::application::color_picker::PickerMode,
    (hue_deg, sat, val): (f32, f32, f32),
    doc: &mut MindMapDocument,
    state: &mut crate::application::color_picker::ColorPickerState,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    use crate::application::color_picker::{
        arm_bottom_font, arm_bottom_glyphs, arm_left_glyphs, arm_right_glyphs, arm_top_glyphs,
        center_preview_glyph, hue_ring_font_scale, hue_ring_glyphs, ColorPickerState,
    };

    // Measure the widest shaped advance across every crosshair-arm
    // glyph and every hue-ring glyph. These become the spacing units
    // the layout pure-fn uses for cell and ring-slot positions —
    // measuring once here avoids per-hover font-system traffic and
    // keeps `compute_color_picker_layout` pure. Both measurements
    // happen behind the font-system write lock, which is also what
    // the renderer's buffer builders need, so we grab it once.
    // Measurement font size: pick the spec's `font_max` so the
    // ratios captured here are accurate across the full
    // `[font_min, font_max]` range the layout fn might pick. The
    // ratios `max_cell_advance / measurement_font_size` and
    // `max_ring_advance / (measurement_font_size * ring_scale)` are
    // dimensionless and stable across font sizes (cosmic-text
    // shapes proportionally), so the layout can scale them to
    // whatever font_size it derives from the window-size formula.
    let geom = &crate::application::widgets::color_picker_widget::load_spec().geometry;
    let measurement_font_size: f32 = geom.font_max;
    let ring_font_size = measurement_font_size * hue_ring_font_scale();
    let (
        max_cell_advance,
        max_ring_advance,
        arm_top_ink_offsets,
        arm_bottom_ink_offsets,
        arm_left_ink_offsets,
        arm_right_ink_offsets,
        preview_ink_offset,
    ) = {
        let mut font_system = baumhard::font::fonts::FONT_SYSTEM
            .write()
            .expect("Failed to acquire font_system lock");
        let mut crosshair: Vec<&str> = Vec::with_capacity(40);
        crosshair.extend(arm_top_glyphs().iter().copied());
        crosshair.extend(arm_bottom_glyphs().iter().copied());
        crosshair.extend(arm_left_glyphs().iter().copied());
        crosshair.extend(arm_right_glyphs().iter().copied());
        let cell = crate::application::renderer::measure_max_glyph_advance(
            &mut font_system,
            &crosshair,
            measurement_font_size,
        );
        let ring_glyphs: Vec<&str> = hue_ring_glyphs().iter().copied().collect();
        let ring = crate::application::renderer::measure_max_glyph_advance(
            &mut font_system,
            &ring_glyphs,
            ring_font_size,
        );
        // Per-glyph ink-center-vs-em-box-center offset, both axes.
        // The picker renders each cell with `Align::Center` in a
        // bounds box of width `cell_box_w` and height
        // `cell_font_size * 1.5`. For the ink to land on the
        // crosshair radius (not the em-box centre) we measure each
        // glyph's actual ink rectangle and store a dimensionless
        // (dx, dy) — `compute_color_picker_layout` subtracts those
        // per-cell. A single per-arm correction can't pull this
        // off: both sidebearings (drives x) and baseline-relative
        // ink extent (drives y) vary glyph-to-glyph within an arm.
        let mut swash_cache = cosmic_text::SwashCache::new();
        use baumhard::font::fonts::measure_glyph_ink_bounds;
        use crate::application::color_picker::CROSSHAIR_CENTER_CELL;
        let mut arm_ink_offsets = |glyphs: &[&str], font: Option<baumhard::font::fonts::AppFont>|
            -> [(f32, f32); CROSSHAIR_CENTER_CELL] {
            // Spec-load tests assert each arm has exactly
            // `CROSSHAIR_CENTER_CELL` glyphs; index directly so a
            // runtime spec mismatch panics here instead of silently
            // shipping zero offsets for the missing entries.
            let mut out = [(0.0_f32, 0.0_f32); CROSSHAIR_CENTER_CELL];
            for (i, slot) in out.iter_mut().enumerate() {
                let b = measure_glyph_ink_bounds(
                    &mut font_system,
                    &mut swash_cache,
                    font,
                    glyphs[i],
                    measurement_font_size,
                );
                let dx = b.x_offset_from_advance_center() / measurement_font_size;
                let dy = b.y_offset_from_box_center(measurement_font_size, 1.5)
                    / measurement_font_size;
                *slot = (dx, dy);
            }
            out
        };
        let arm_top = arm_ink_offsets(arm_top_glyphs(), None);
        let arm_bottom = arm_ink_offsets(arm_bottom_glyphs(), arm_bottom_font());
        let arm_left = arm_ink_offsets(arm_left_glyphs(), None);
        let arm_right = arm_ink_offsets(arm_right_glyphs(), None);
        // Preview ࿕ — full (dx, dy) correction at the preview's own
        // box height (also `1.5 * font_size`).
        let preview_bounds = measure_glyph_ink_bounds(
            &mut font_system,
            &mut swash_cache,
            Some(baumhard::font::fonts::AppFont::NotoSerifTibetanRegular),
            center_preview_glyph(),
            measurement_font_size,
        );
        let preview = (
            preview_bounds.x_offset_from_advance_center() / measurement_font_size,
            preview_bounds.y_offset_from_box_center(measurement_font_size, 1.5)
                / measurement_font_size,
        );
        (cell, ring, arm_top, arm_bottom, arm_left, arm_right, preview)
    };

    *state = ColorPickerState::Open {
        mode,
        hue_deg,
        sat,
        val,
        last_cursor_pos: None,
        max_cell_advance,
        max_ring_advance,
        measurement_font_size,
        arm_top_ink_offsets,
        arm_bottom_ink_offsets,
        arm_left_ink_offsets,
        arm_right_ink_offsets,
        preview_ink_offset,
        layout: None,
        center_override: None,
        size_scale: 1.0,
        gesture: None,
        hovered_hit: None,
        pending_error_flash: false,
        last_dynamic_apply: None,
    };

    rebuild_color_picker_overlay(state, doc, app_scene, renderer);
    rebuild_scene_only(doc, app_scene, renderer);
}

/// Helper: write the initial HSV into `doc.color_picker_preview` on
/// picker open so the first rendered frame already shows the
/// previewed color instead of the model's stored one.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn seed_initial_preview(
    doc: &mut MindMapDocument,
    handle: &crate::application::color_picker::PickerHandle,
    hue_deg: f32,
    sat: f32,
    val: f32,
) {
    use crate::application::color_picker::PickerHandle;
    use crate::application::document::ColorPickerPreview;
    use baumhard::util::color::hsv_to_hex;

    let hex = hsv_to_hex(hue_deg, sat, val);
    match handle {
        PickerHandle::Edge(index) => {
            if let Some(edge) = doc.mindmap.edges.get(*index) {
                let key = baumhard::mindmap::scene_cache::EdgeKey::from_edge(edge);
                doc.color_picker_preview = Some(ColorPickerPreview::Edge { key, color: hex });
            }
        }
        PickerHandle::Portal(index) => {
            if let Some(portal) = doc.mindmap.portals.get(*index) {
                let key = baumhard::mindmap::scene_builder::PortalRefKey::from_portal(portal);
                doc.color_picker_preview = Some(ColorPickerPreview::Portal { key, color: hex });
            }
        }
        PickerHandle::Node { .. } => {
            // Node preview not yet plumbed through the scene
            // builder — commit-only for v1. The picker still opens
            // and lets the user pick + commit; it just doesn't
            // hover-preview on the underlying node.
        }
    }
}

/// Build geometry from the current picker state and report whether
/// the layout has changed since the previous call. Internal helper —
/// the bool lets [`rebuild_color_picker_overlay`] pick between the
/// layout-phase mutator (full per-cell field set; viewport resize,
/// RMB drag-to-resize, drag-to-move repositioning) and the cheaper
/// dynamic-phase mutator (color + hover scale + hex text only). Also
/// caches the freshly-computed `ColorPickerLayout` back into the
/// state so the mouse hit-test can read it without re-running the
/// layout pure fn.
///
/// The "first call after open" case (no cached layout yet) reports
/// `changed = true` so the layout phase fires once before any
/// dynamic frames land — without it the dynamic mutator would only
/// write the per-frame fields onto a tree whose static fields are
/// still at their initial-build values, which is correct on this
/// path (the initial build IS the layout phase) but the bool keeps
/// the dispatch readable.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn compute_picker_geometry(
    state: &mut crate::application::color_picker::ColorPickerState,
    renderer: &Renderer,
) -> Option<(
    crate::application::color_picker::ColorPickerOverlayGeometry,
    bool,
)> {
    use crate::application::color_picker::{
        compute_color_picker_layout, ColorPickerOverlayGeometry, ColorPickerState,
    };
    use baumhard::util::color::hsv_to_hex;

    // Extract only the fields `compute_color_picker_layout` needs,
    // plus a copy of the backdrop tuple from the cached layout for
    // the cursor-inside-backdrop check. Copying just the 4 floats is
    // ~200 bytes cheaper than cloning the whole ColorPickerLayout
    // (with its fixed-size cell-position arrays) every
    // hover.
    let (
        target_label,
        hue_deg,
        sat,
        val,
        last_cursor_pos,
        max_cell_advance,
        max_ring_advance,
        measurement_font_size,
        size_scale,
        cached_backdrop,
        center_override,
        hovered_hit,
        arm_top_ink_offsets,
        arm_bottom_ink_offsets,
        arm_left_ink_offsets,
        arm_right_ink_offsets,
        preview_ink_offset,
    ) = match state {
        ColorPickerState::Closed => return None,
        ColorPickerState::Open {
            mode,
            hue_deg,
            sat,
            val,
            last_cursor_pos,
            max_cell_advance,
            max_ring_advance,
            measurement_font_size,
            layout,
            center_override,
            size_scale,
            hovered_hit,
            arm_top_ink_offsets,
            arm_bottom_ink_offsets,
            arm_left_ink_offsets,
            arm_right_ink_offsets,
            preview_ink_offset,
            ..
        } => (
            match mode {
                crate::application::color_picker::PickerMode::Contextual { handle } => {
                    handle.label()
                }
                crate::application::color_picker::PickerMode::Standalone => "",
            },
            *hue_deg,
            *sat,
            *val,
            *last_cursor_pos,
            *max_cell_advance,
            *max_ring_advance,
            *measurement_font_size,
            *size_scale,
            layout.as_ref().map(|l| l.backdrop),
            *center_override,
            *hovered_hit,
            *arm_top_ink_offsets,
            *arm_bottom_ink_offsets,
            *arm_left_ink_offsets,
            *arm_right_ink_offsets,
            *preview_ink_offset,
        ),
    };

    // Hex readout is visible when the cursor is inside the backdrop.
    // Without a cached layout from a previous rebuild we can't
    // hit-test the backdrop, so the first rebuild lands without the
    // hex showing; it appears on the first hover rebuild after the
    // cursor enters the window.
    let hex_visible = match (last_cursor_pos, cached_backdrop) {
        (Some((cx, cy)), Some((bl, bt, bw, bh))) => {
            cx >= bl && cx <= bl + bw && cy >= bt && cy <= bt + bh
        }
        _ => false,
    };

    let geometry = ColorPickerOverlayGeometry {
        target_label,
        hue_deg,
        sat,
        val,
        preview_hex: hsv_to_hex(hue_deg, sat, val),
        hex_visible,
        max_cell_advance,
        max_ring_advance,
        measurement_font_size,
        size_scale,
        center_override,
        hovered_hit,
        arm_top_ink_offsets,
        arm_bottom_ink_offsets,
        arm_left_ink_offsets,
        arm_right_ink_offsets,
        preview_ink_offset,
    };

    // Cache the layout into the state so the mouse hit-test can use
    // it, and report whether it actually changed since the last
    // rebuild — `true` on the first call after open (no cached layout
    // yet) and on any layout-affecting change (resize, size_scale,
    // center_override, ink-offset measurement update).
    let layout = compute_color_picker_layout(
        &geometry,
        renderer.surface_width() as f32,
        renderer.surface_height() as f32,
    );
    let layout_changed = if let ColorPickerState::Open { layout: cached, .. } = state {
        let changed = cached.as_ref().map_or(true, |c| c != &layout);
        *cached = Some(layout);
        changed
    } else {
        true
    };

    Some((geometry, layout_changed))
}

/// Picker overlay update entry point. Dispatches between the
/// initial-build path and the §B2-compliant in-place mutator paths:
///
/// - **Closed** (`compute_picker_geometry` returns `None`): unregister
///   the overlay tree by passing `None` to the buffer rebuild.
/// - **First open** (no tree registered): build a fresh tree via
///   [`Renderer::rebuild_color_picker_overlay_buffers`]. The initial
///   build *is* the layout phase, so dynamic frames after this can
///   safely target the just-built static fields.
/// - **Layout changed** (resize, RMB drag-to-resize, drag-to-move
///   repositioning): apply the layout-phase mutator —
///   [`Renderer::apply_color_picker_overlay_mutator`] — which writes
///   every variable field on every cell via `Assign` deltas.
/// - **Layout unchanged** (per-frame hover / HSV / chip / drag-Move
///   without geometry change): apply the dynamic-phase mutator —
///   [`Renderer::apply_color_picker_overlay_dynamic_mutator`] — which
///   writes only the fields that genuinely move per frame
///   (`ColorFontRegions`, `scale`, hex `Text`).
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn rebuild_color_picker_overlay(
    state: &mut crate::application::color_picker::ColorPickerState,
    _doc: &MindMapDocument,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    use crate::application::color_picker::{ColorPickerState, PickerDynamicApplyKey};
    use crate::application::scene_host::OverlayRole;
    let Some((geometry, layout_changed)) = compute_picker_geometry(state, renderer) else {
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
) {
    use crate::application::color_picker::ColorPickerState;

    if matches!(state, ColorPickerState::Closed) {
        return;
    }
    *state = ColorPickerState::Closed;
    doc.color_picker_preview = None;
    renderer.rebuild_color_picker_overlay_buffers(app_scene, None);
    rebuild_all(doc, mindmap_tree, app_scene, renderer);
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
) {
    cancel_color_picker(state, doc, mindmap_tree, app_scene, renderer);
}

/// Commit the picker's currently-previewed HSV value via the regular
/// `set_edge_color` / `set_portal_color` / `set_node_*_color` path —
/// a single undo entry is pushed and `ensure_glyph_connection` runs
/// its fork-on-first-edit only at this moment (never during hover).
/// Close the modal.
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
        PickerHandle::Portal(index) => {
            let pr = doc.mindmap.portals.get(index).map(|p| {
                crate::application::document::PortalRef::new(
                    p.label.clone(),
                    p.endpoint_a.clone(),
                    p.endpoint_b.clone(),
                )
            });
            if let Some(pr) = pr {
                doc.set_portal_color(&pr, &hex);
            }
        }
        PickerHandle::Node { id, axis } => {
            match axis {
                NodeColorAxis::Bg => {
                    doc.set_node_bg_color(&id, hex);
                }
                NodeColorAxis::Text => {
                    doc.set_node_text_color(&id, hex);
                }
                NodeColorAxis::Border => {
                    doc.set_node_border_color(&id, hex);
                }
            }
        }
    }

    renderer.rebuild_color_picker_overlay_buffers(app_scene, None);
    rebuild_all(doc, mindmap_tree, app_scene, renderer);
}

/// Apply the current picker HSV to the document's transient color
/// preview, then rebuild only the scene (not the node tree, which
/// didn't change) + the picker overlay. Hot path: no ref resolution,
/// no model mutation, no snapshot. The scene builder reads the
/// preview via `doc.color_picker_preview` and substitutes it in
/// during emission.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn apply_picker_preview(
    state: &mut crate::application::color_picker::ColorPickerState,
    doc: &mut MindMapDocument,
    picker_dirty: &mut bool,
) {
    use crate::application::color_picker::{ColorPickerState, PickerHandle};
    use crate::application::document::ColorPickerPreview;
    use baumhard::util::color::hsv_to_hex;

    let (handle, hue_deg, sat, val) = match state {
        ColorPickerState::Open {
            mode,
            hue_deg,
            sat,
            val,
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
            (handle, *hue_deg, *sat, *val)
        }
        ColorPickerState::Closed => return,
    };
    let hex = hsv_to_hex(hue_deg, sat, val);
    if let Some(handle) = handle {
        match handle {
            PickerHandle::Edge(index) => {
                if let Some(edge) = doc.mindmap.edges.get(index) {
                    let key = baumhard::mindmap::scene_cache::EdgeKey::from_edge(edge);
                    doc.color_picker_preview =
                        Some(ColorPickerPreview::Edge { key, color: hex });
                }
            }
            PickerHandle::Portal(index) => {
                if let Some(portal) = doc.mindmap.portals.get(index) {
                    let key = baumhard::mindmap::scene_builder::PortalRefKey::from_portal(portal);
                    doc.color_picker_preview =
                        Some(ColorPickerPreview::Portal { key, color: hex });
                }
            }
            PickerHandle::Node { .. } => {
                // Node preview lives on the tree pipeline, not the
                // scene pipeline — not yet wired. Commit-only for v1.
            }
        }
    }
    // Scene + picker rebuilds are deferred to the `AboutToWait`
    // drain via `picker_dirty`. Mouse moves come in at ~120Hz on
    // modern hardware; without this gate every event would
    // re-shape every border / connection / portal on the map
    // plus the picker overlay. The drain is gated by
    // `picker_throttle` (the same `MutationFrequencyThrottle`
    // type the drag path uses), which self-tunes to keep the
    // per-frame work under the refresh budget.
    *picker_dirty = true;
}

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

/// Mouse-move handler for the picker. Branches on active-drag vs
/// hover:
///
/// - **Drag active**: translate the wheel so
///   `center = cursor + grab_offset`. Every layout position (ring,
///   bars, chips, backdrop) rebuilds against the new center via
///   `center_override`.
/// - **Hover**: hit-test the cursor, update HSV / chip focus to match
///   the hovered glyph (live preview), and record
///   `hovered_hit` for the renderer's hover-grow effect.
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
    picker_dirty: &mut bool,
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
                let geom = &crate::application::widgets::color_picker_widget::load_spec()
                    .geometry;
                *size_scale = (anchor_scale * (r_now / anchor_radius))
                    .clamp(geom.resize_scale_min, geom.resize_scale_max);
            }
        }
        *picker_dirty = true;
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

        match hit {
            PickerHit::Hue(slot) => {
                let new_hue = hue_slot_to_degrees(slot);
                if (*hue_deg - new_hue).abs() > f32::EPSILON {
                    *hue_deg = new_hue;
                    state_changed = true;
                }
            }
            PickerHit::SatCell(i) => {
                let new_sat = sat_cell_to_value(i);
                if (*sat - new_sat).abs() > f32::EPSILON {
                    *sat = new_sat;
                    state_changed = true;
                }
            }
            PickerHit::ValCell(i) => {
                let new_val = val_cell_to_value(i);
                if (*val - new_val).abs() > f32::EPSILON {
                    *val = new_val;
                    state_changed = true;
                }
            }
            PickerHit::Commit | PickerHit::DragAnchor | PickerHit::Outside => {}
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
        *picker_dirty = true;
        // Preview the updated HSV onto the (possibly contextual)
        // target so the map reflects the hover color live. No-op
        // in Standalone mode — no bound target — but the ࿕ glyph
        // in the wheel still shows the current HSV so the user
        // gets immediate color feedback on the wheel itself.
        apply_picker_preview(state, doc, picker_dirty);
    }
    true
}

/// Click handler for the picker. Semantics:
///
/// - **Hue / SatCell / ValCell / Chip** — preview only. The
///   mouse-move handler already updated HSV on hover, so a click on
///   a glyph is effectively a no-op at the model layer; it's the
///   user affirming the current selection. Clicks here **do not**
///   commit and **do not** close the wheel — users can click around
///   freely and watch the preview update.
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
) -> bool {
    use crate::application::color_picker::{
        hit_test_picker, ColorPickerState, PickerGesture, PickerHit,
    };

    let hit = if let ColorPickerState::Open { layout: Some(layout), .. } = state {
        hit_test_picker(layout, cursor_pos.0 as f32, cursor_pos.1 as f32)
    } else {
        return false;
    };

    // RMB outside the DragAnchor region is a no-op for now — only
    // the empty backdrop area acts as a resize handle. That keeps
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
        PickerHit::Hue(_) | PickerHit::SatCell(_) | PickerHit::ValCell(_) => {
            // Preview-only: the mouse-move handler already updated
            // HSV as the cursor moved over the glyph, so clicking is
            // a no-op at the model layer. Users can click freely to
            // experiment without the picker closing.
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
            // Start a gesture from anywhere inside the backdrop
            // that's not on an interactive glyph. LMB → move
            // (translate center_override); RMB → resize (mutate
            // size_scale). The two gestures are mutually exclusive
            // by construction — `gesture` only holds one variant.
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

/// Commit the picker's current HSV to every colorable item in the
/// document's current selection. Standalone mode's core gesture.
///
/// Dispatches through the [`AcceptsWheelColor`] trait: each component
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
        // Rebuild the whole scene so the newly-colored items repaint
        // next frame. The picker itself stays open — no state change
        // needed on `state`.
        rebuild_all(doc, mindmap_tree, app_scene, renderer);
    }
}
