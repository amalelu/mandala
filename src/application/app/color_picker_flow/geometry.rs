//! `compute_picker_geometry` — the pre-rebuild pass that turns the
//! state's HSV + measurement values into a
//! `ColorPickerOverlayGeometry`, runs the pure-function layout, and
//! reports whether the layout changed since the last rebuild so the
//! dispatcher can pick between the layout-phase and dynamic-phase
//! mutators.

/// Build geometry from the current picker state and report whether
/// the layout has changed since the previous call. Internal helper —
/// the bool lets `rebuild_color_picker_overlay` pick between the
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
///
/// Takes `surface_size` as a plain `(width, height)` tuple in pixels
/// rather than a `&Renderer`: per `CODE_CONVENTIONS.md §3`, platform-
/// shared layout math must be reachable without a wgpu instance, and
/// the pure layout fn `compute_color_picker_layout` only needs the
/// two screen dimensions.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn compute_picker_geometry(
    state: &mut crate::application::color_picker::ColorPickerState,
    surface_size: (f32, f32),
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
    // (with its fixed-size cell-position arrays) every hover.
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
            hover_preview,
            arm_top_ink_offsets,
            arm_bottom_ink_offsets,
            arm_left_ink_offsets,
            arm_right_ink_offsets,
            preview_ink_offset,
            ..
        } => {
            let (eff_hue, eff_sat, eff_val) =
                hover_preview.unwrap_or((*hue_deg, *sat, *val));
            (
            match mode {
                crate::application::color_picker::PickerMode::Contextual { handle } => {
                    handle.label()
                }
                crate::application::color_picker::PickerMode::Standalone => "",
            },
            eff_hue,
            eff_sat,
            eff_val,
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
        )},
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
    let (surface_w, surface_h) = surface_size;
    let layout = compute_color_picker_layout(&geometry, surface_w, surface_h);
    let layout_changed = if let ColorPickerState::Open { layout: cached, .. } = state {
        let changed = cached.as_ref().map_or(true, |c| c != &layout);
        *cached = Some(layout);
        changed
    } else {
        true
    };

    Some((geometry, layout_changed))
}
