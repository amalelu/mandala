//! Single source of truth for the picker's GlyphArea content, keyed
//! by stable channels. Both
//! [`super::tree_builder::build_color_picker_overlay_tree`] (the
//! initial-build path) and
//! [`super::tree_builder::build_color_picker_overlay_mutator`] (the
//! in-place update path) consume this so they can never drift.

use baumhard::gfx_structs::area::{GlyphArea, OutlineStyle};

use super::color::{
    highlight_hovered_cell_color, highlight_selected_cell_color, rgb_to_cosmic_color,
};
use super::make_area::make_area;

/// Single source of truth for the picker's GlyphArea content, keyed
/// by stable channels. Both [`super::tree_builder::build_color_picker_overlay_tree`]
/// (the initial-build path) and
/// [`super::tree_builder::build_color_picker_overlay_mutator`]
/// (the in-place update path) consume this so they can never drift.
///
/// **Channel ordering invariant**: the returned vec must be sorted
/// by ascending channel — Baumhard's `align_child_walks` pairs
/// mutator children against target children by ascending channel
/// and breaks alignment if the order is violated. The constants in
/// `color_picker.rs` (PICKER_CHANNEL_*) are already chosen to
/// preserve this invariant in the natural insertion order
/// (title → hue ring → hint → sat → val → preview → hex → chips).
///
/// **Stable element count**: hex is always emitted (with empty
/// text when invisible) so the channel set doesn't shift when the
/// cursor crosses the backdrop boundary. Empty-text areas are
/// skipped by the walker without shaping.
pub(super) fn picker_glyph_areas(
    geometry: &crate::application::color_picker::ColorPickerOverlayGeometry,
    layout: &crate::application::color_picker::ColorPickerLayout,
) -> Vec<(usize, GlyphArea)> {
    use crate::application::color_picker::{
        arm_bottom_font, arm_bottom_glyphs, arm_left_glyphs, arm_right_glyphs, arm_top_glyphs,
        center_preview_glyph, hue_ring_glyphs, hue_slot_to_degrees, sat_cell_to_value,
        val_cell_to_value, CROSSHAIR_CENTER_CELL, PickerHit, PICKER_CHANNEL_HEX,
        PICKER_CHANNEL_HINT, PICKER_CHANNEL_HUE_RING_BASE, PICKER_CHANNEL_PREVIEW,
        PICKER_CHANNEL_SAT_BASE, PICKER_CHANNEL_TITLE, PICKER_CHANNEL_VAL_BASE,
        SAT_CELL_COUNT, VAL_CELL_COUNT,
    };
    use crate::application::widgets::color_picker_widget::load_spec;
    use baumhard::util::color::{hsv_to_hex, hsv_to_rgb};

    let spec = load_spec();
    let hover_scale: f32 = spec.geometry.hover_scale;

    // Outline style for every picker glyph. Sized at the spec's
    // `font_max` baseline and scaled linearly to the actual layout
    // `font_size` so a shrunk picker gets a proportionally thinner
    // outline. The walker (`walk_tree_into_buffers`) reads
    // `area.outline` and stamps 8 copies at the offsets yielded by
    // `OutlineStyle::offsets` — the stamp count is canonical inside
    // baumhard, so there's no `samples` knob here.
    let outline = if spec.geometry.outline_px > 0.0 {
        Some(OutlineStyle {
            color: [0, 0, 0, 255],
            px: spec.geometry.outline_px * (layout.font_size / spec.geometry.font_max),
        })
    } else {
        None
    };

    let font_size = layout.font_size;
    let ring_font_size = layout.ring_font_size;
    let cell_font_size = layout.cell_font_size;
    // Widen box reservations past the base glyph so hover-grow has
    // room to render at HOVER_SCALE without clipping neighbors, and
    // SMP glyphs (Egyptian hieroglyphs especially) shape without
    // hitting the right bound.
    let ring_box_w = ring_font_size * spec.geometry.ring_box_scale;
    let cell_box_w =
        (layout.cell_advance * spec.geometry.cell_box_scale).max(cell_font_size * 1.5);

    // Non-wheel chrome (title, hint, hex readout) tracks the
    // picker's current HSV preview color. This means the text
    // "carries" the selected color out of the wheel and into the
    // surrounding copy — confirming at a glance what the user is
    // about to commit. Halo contrast handles legibility.
    let preview_rgb = hsv_to_rgb(geometry.hue_deg, geometry.sat, geometry.val);
    let preview_color = rgb_to_cosmic_color(preview_rgb);

    let mut out: Vec<(usize, GlyphArea)> = Vec::with_capacity(80);

    // Title.
    let is_standalone = geometry.target_label.is_empty();
    let title_text = if is_standalone {
        spec.title_template_standalone.clone()
    } else {
        spec.title_template_contextual
            .replace("{target_label}", geometry.target_label)
    };
    out.push((
        PICKER_CHANNEL_TITLE,
        make_area(
            &title_text,
            preview_color,
            font_size,
            font_size,
            layout.title_pos,
            (font_size * 24.0, font_size * 1.5),
            false,
            None,
            outline,
        ),
    ));

    // Hue ring.
    for (i, &ring_glyph) in hue_ring_glyphs().iter().enumerate() {
        let hue = hue_slot_to_degrees(i);
        let rgb = hsv_to_rgb(hue, 1.0, 1.0);
        let is_hovered = matches!(geometry.hovered_hit, Some(PickerHit::Hue(h)) if h == i);
        let color = if is_hovered {
            highlight_hovered_cell_color(rgb)
        } else {
            rgb_to_cosmic_color(rgb)
        };
        let scale = if is_hovered { hover_scale } else { 1.0 };
        let pos = layout.hue_slot_positions[i];
        let fs = ring_font_size * scale;
        let bw = ring_box_w * scale;
        out.push((
            PICKER_CHANNEL_HUE_RING_BASE + i,
            make_area(
                ring_glyph,
                color,
                fs,
                fs,
                (pos.0 - bw * 0.5, pos.1 - fs * 0.5),
                (bw, fs * 1.5),
                true,
                None,
                outline,
            ),
        ));
    }

    // Hint footer. Contextual mode includes "Esc cancel" because
    // Esc exits the modal picker; Standalone mode omits it because
    // the persistent palette only closes via `color picker off`
    // from the console, and showing a dead affordance is worse
    // than hiding it.
    let hint_text = if is_standalone {
        spec.hint_text_standalone.as_str()
    } else {
        spec.hint_text_contextual.as_str()
    };
    out.push((
        PICKER_CHANNEL_HINT,
        make_area(
            hint_text,
            preview_color,
            font_size * 0.85,
            font_size * 0.85,
            layout.hint_pos,
            (font_size * 30.0, font_size * 1.5),
            false,
            None,
            outline,
        ),
    ));

    // Sat / val bars (skip centre cell — that's the preview glyph slot).
    let current_sat_cell = (geometry.sat * (SAT_CELL_COUNT as f32 - 1.0))
        .round()
        .clamp(0.0, (SAT_CELL_COUNT - 1) as f32) as usize;
    let current_val_cell = ((1.0 - geometry.val) * (VAL_CELL_COUNT as f32 - 1.0))
        .round()
        .clamp(0.0, (VAL_CELL_COUNT - 1) as f32) as usize;

    for i in 0..SAT_CELL_COUNT {
        if i == CROSSHAIR_CENTER_CELL {
            continue;
        }
        let cell_sat = sat_cell_to_value(i);
        let base_rgb = hsv_to_rgb(geometry.hue_deg, cell_sat, geometry.val);
        let is_hovered = matches!(geometry.hovered_hit, Some(PickerHit::SatCell(h)) if h == i);
        let color = if is_hovered {
            highlight_hovered_cell_color(base_rgb)
        } else if i == current_sat_cell {
            highlight_selected_cell_color(base_rgb)
        } else {
            rgb_to_cosmic_color(base_rgb)
        };
        let glyph = if i < CROSSHAIR_CENTER_CELL {
            arm_left_glyphs()[i]
        } else {
            arm_right_glyphs()[i - CROSSHAIR_CENTER_CELL - 1]
        };
        let scale = if is_hovered { hover_scale } else { 1.0 };
        let (cx, cy) = layout.sat_cell_positions[i];
        let fs = cell_font_size * scale;
        let bw = cell_box_w * scale;
        out.push((
            PICKER_CHANNEL_SAT_BASE + i,
            make_area(
                glyph,
                color,
                fs,
                fs,
                (cx - bw * 0.5, cy - fs * 0.5),
                (bw, fs * 1.5),
                true,
                None,
                outline,
            ),
        ));
    }
    for i in 0..VAL_CELL_COUNT {
        if i == CROSSHAIR_CENTER_CELL {
            continue;
        }
        let cell_val = val_cell_to_value(i);
        let base_rgb = hsv_to_rgb(geometry.hue_deg, geometry.sat, cell_val);
        let is_hovered = matches!(geometry.hovered_hit, Some(PickerHit::ValCell(h)) if h == i);
        let color = if is_hovered {
            highlight_hovered_cell_color(base_rgb)
        } else if i == current_val_cell {
            highlight_selected_cell_color(base_rgb)
        } else {
            rgb_to_cosmic_color(base_rgb)
        };
        // Pin Egyptian hieroglyph font on the bottom arm — see the
        // walker's family-name fix for context.
        let (glyph, font) = if i < CROSSHAIR_CENTER_CELL {
            (arm_top_glyphs()[i], None)
        } else {
            (
                arm_bottom_glyphs()[i - CROSSHAIR_CENTER_CELL - 1],
                arm_bottom_font(),
            )
        };
        let scale = if is_hovered { hover_scale } else { 1.0 };
        let (cx, cy) = layout.val_cell_positions[i];
        let fs = cell_font_size * scale;
        let bw = cell_box_w * scale;
        out.push((
            PICKER_CHANNEL_VAL_BASE + i,
            make_area(
                glyph,
                color,
                fs,
                fs,
                (cx - bw * 0.5, cy - fs * 0.5),
                (bw, fs * 1.5),
                true,
                font,
                outline,
            ),
        ));
    }

    // Centre preview glyph ࿕ (right-facing Tibetan svasti — the
    // spiritual "four roads meeting" symbol). Acts as the commit
    // button; hovering brightens it.
    let preview_size = layout.preview_size;
    let commit_hovered = matches!(geometry.hovered_hit, Some(PickerHit::Commit));
    let commit_color = if commit_hovered {
        highlight_hovered_cell_color(preview_rgb)
    } else {
        preview_color
    };
    let preview_scale_f = if commit_hovered { hover_scale } else { 1.0 };
    let scaled_preview = preview_size * preview_scale_f;
    // Pin the Tibetan font for the ࿕ glyph (U+0FD5) — cosmic-text's
    // default fallback isn't reliable for it, and we already pin
    // specific fonts for the Egyptian arm via the same pattern.
    let center_font = Some(baumhard::font::fonts::AppFont::NotoSerifTibetanRegular);
    // `layout.preview_pos` is the top-left of a `preview_size ×
    // preview_size` box whose centre is the wheel intersection
    // (already ink-corrected). The render box is `scaled_preview *
    // 1.5` to give hover-grow and sub-pixel slack; centre it
    // symmetrically on that same point so `Align::Center` lands the
    // glyph's advance-centre exactly where the layout intended.
    // Previous revision positioned the 1.5× box as if it were 1.0×,
    // extending 0.5× to the right only and drifting the ࿕ right of
    // centre by `preview_size/4` — ~15 px at the spec's 3× preview
    // scale.
    let preview_glyph_center = (
        layout.preview_pos.0 + preview_size * 0.5,
        layout.preview_pos.1 + preview_size * 0.5,
    );
    let preview_box_w = scaled_preview * 1.5;
    let preview_box_h = scaled_preview * 1.5;
    out.push((
        PICKER_CHANNEL_PREVIEW,
        make_area(
            center_preview_glyph(),
            commit_color,
            scaled_preview,
            scaled_preview,
            (
                preview_glyph_center.0 - preview_box_w * 0.5,
                preview_glyph_center.1 - preview_box_h * 0.5,
            ),
            (preview_box_w, preview_box_h),
            true,
            center_font,
            outline,
        ),
    ));

    // Hex readout — always emitted at a stable channel so the
    // mutator path doesn't have to handle a flickering element.
    // Empty text when invisible; the walker shapes nothing.
    let (hex_text, hex_pos, hex_bounds) = match layout.hex_pos {
        Some(anchor) => (
            hsv_to_hex(geometry.hue_deg, geometry.sat, geometry.val),
            anchor,
            (font_size * 8.0, font_size * 1.5),
        ),
        None => (String::new(), (0.0, 0.0), (0.0, 0.0)),
    };
    out.push((
        PICKER_CHANNEL_HEX,
        make_area(
            &hex_text,
            preview_color,
            font_size,
            font_size,
            hex_pos,
            hex_bounds,
            false,
            None,
            outline,
        ),
    ));

    out
}
