//! Picker open paths: Contextual (bound target) and Standalone
//! (persistent palette). The shared `open_picker_inner` performs the
//! one-time font-system measurement pass that seeds
//! `max_cell_advance`, `max_ring_advance`, and the per-glyph ink
//! offsets the pure-function layout later consumes.

use crate::application::document::MindMapDocument;
use crate::application::renderer::Renderer;

use super::super::rebuild_scene_only;
use super::rebuild::rebuild_color_picker_overlay;

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
        hover_preview: None,
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
        PickerHandle::Node { .. } => {
            // Node preview not yet plumbed through the scene
            // builder — commit-only for v1. The picker still opens
            // and lets the user pick + commit; it just doesn't
            // hover-preview on the underlying node.
        }
    }
}
