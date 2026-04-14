//! Single source of truth for the picker's per-section
//! `GlyphArea` set — consumed by the initial-build path (which
//! wraps each area in a `GfxElement::GlyphArea` node) and by the
//! mutator path (which threads them through
//! [`crate::application::mutator_builder`] as
//! runtime values for the declarative spec). The two paths cannot
//! drift because they both read from the same [`PickerAreas`]
//! table.
//!
//! The section names ("title", "hue_ring", "hint", "sat_bar",
//! "val_bar", "preview", "hex") must match the
//! `mutator_spec.sections[*].section` strings in
//! `widgets/color_picker.json` — the spec's channel layout is
//! authoritative and the fn below fills the cells it asks for.

use std::collections::HashMap;

use baumhard::core::primitives::ColorFontRegions;
use baumhard::gfx_structs::area::{GlyphArea, OutlineStyle};
use glam::Vec2;

use super::color::{
    highlight_hovered_cell_color, highlight_selected_cell_color, rgb_to_cosmic_color,
};

/// One entry in the picker's tagged area list. Same payload as the
/// old `(usize, GlyphArea)` tuples plus the `(section, index)`
/// identity that lets the mutator path route by section name rather
/// than by channel math.
struct TaggedArea {
    section: &'static str,
    index: usize,
    channel: usize,
    area: GlyphArea,
}

/// All `GlyphArea`s the picker will emit on one apply cycle, in two
/// complementary forms:
///
/// - [`ordered`] is the channel-ascending `(channel, area)` list the
///   initial-build path needs to build its tree in the right walker
///   order.
/// - [`by_section`] is a `section → index → vec_index` lookup the
///   mutator path uses to resolve
///   `mutator_builder::SectionContext::area(section, index)` calls
///   without scanning or doing channel math.
///
/// Both share the same backing storage: `ordered[by_section[..][..]]`
/// is always the matching area.
///
/// [`ordered`]: PickerAreas::ordered
/// [`by_section`]: PickerAreas::by_section
pub(super) struct PickerAreas {
    pub(super) ordered: Vec<(usize, GlyphArea)>,
    by_section: HashMap<&'static str, Vec<Option<usize>>>,
}

impl PickerAreas {
    /// Resolve a `(section, index) → &GlyphArea` lookup. Panics if
    /// the section wasn't populated (the spec / builder disagree
    /// on what sections exist), since the picker apply path treats
    /// an absent section as a programming error rather than a
    /// recoverable state.
    pub(super) fn area(&self, section: &str, index: usize) -> &GlyphArea {
        let row = self
            .by_section
            .get(section)
            .unwrap_or_else(|| panic!("picker area lookup: unknown section {section:?}"));
        let vec_index = row
            .get(index)
            .and_then(|v| *v)
            .unwrap_or_else(|| panic!(
                "picker area lookup: section {section:?} index {index} was not populated \
                 (skipped or out-of-range)"
            ));
        &self.ordered[vec_index].1
    }
}

/// Compute the full per-section picker area table for the given
/// `(geometry, layout)` state. Both the initial-build path and the
/// mutator path route through this to avoid drift — see the
/// module-level docs.
///
/// **Channel ordering invariant**: `ordered` is channel-ascending.
/// Baumhard's `align_child_walks` pairs mutator children with target
/// children by ascending channel, so the insertion order here and
/// the mutator-builder's traversal order of the spec must agree.
pub(super) fn compute_picker_areas(
    geometry: &crate::application::color_picker::ColorPickerOverlayGeometry,
    layout: &crate::application::color_picker::ColorPickerLayout,
) -> PickerAreas {
    use crate::application::color_picker::{
        arm_bottom_font, arm_bottom_glyphs, arm_left_glyphs, arm_right_glyphs, arm_top_glyphs,
        center_preview_glyph, hue_ring_glyphs, hue_slot_to_degrees, picker_channel,
        sat_cell_to_value, val_cell_to_value, PickerHit, CROSSHAIR_CENTER_CELL, SAT_CELL_COUNT,
        VAL_CELL_COUNT,
    };
    use crate::application::widgets::color_picker_widget::load_spec;
    use baumhard::util::color::{hsv_to_hex, hsv_to_rgb};

    let spec = load_spec();
    let hover_scale: f32 = spec.geometry.hover_scale;

    // Outline style for every picker glyph. Sized at the spec's
    // `font_max` baseline and scaled linearly to the actual layout
    // `font_size`.
    let outline = if spec.geometry.outline_px > 0.0 {
        Some(OutlineStyle {
            color: [0, 0, 0, 255],
            px: spec.geometry.outline_px * (layout.font_size / spec.geometry.font_max),
        })
    } else {
        None
    };

    fn make_area(
        text: &str,
        color: cosmic_text::Color,
        font_size: f32,
        line_height: f32,
        pos: (f32, f32),
        bounds: (f32, f32),
        centered: bool,
        font: Option<baumhard::font::fonts::AppFont>,
        outline: Option<OutlineStyle>,
    ) -> GlyphArea {
        let mut area = GlyphArea::new_with_str(
            text,
            font_size,
            line_height,
            Vec2::new(pos.0, pos.1),
            Vec2::new(bounds.0, bounds.1),
        );
        area.align_center = centered;
        area.outline = outline;
        let rgba = [
            color.r() as f32 / 255.0,
            color.g() as f32 / 255.0,
            color.b() as f32 / 255.0,
            color.a() as f32 / 255.0,
        ];
        area.regions = ColorFontRegions::single_span(
            baumhard::util::grapheme_chad::count_grapheme_clusters(text),
            Some(rgba),
            font,
        );
        area
    }

    let font_size = layout.font_size;
    let ring_font_size = layout.ring_font_size;
    let cell_font_size = layout.cell_font_size;
    let ring_box_w = ring_font_size * spec.geometry.ring_box_scale;
    let cell_box_w =
        (layout.cell_advance * spec.geometry.cell_box_scale).max(cell_font_size * 1.5);

    let preview_rgb = hsv_to_rgb(geometry.hue_deg, geometry.sat, geometry.val);
    let preview_color = rgb_to_cosmic_color(preview_rgb);

    let mut tagged: Vec<TaggedArea> = Vec::with_capacity(80);

    // Title.
    let is_standalone = geometry.target_label.is_empty();
    let title_text = if is_standalone {
        spec.title_template_standalone.clone()
    } else {
        spec.title_template_contextual
            .replace("{target_label}", geometry.target_label)
    };
    tagged.push(TaggedArea {
        section: "title",
        index: 0,
        channel: picker_channel("title", 0),
        area: make_area(
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
    });

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
        tagged.push(TaggedArea {
            section: "hue_ring",
            index: i,
            channel: picker_channel("hue_ring", i),
            area: make_area(
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
        });
    }

    // Hint footer.
    let hint_text = if is_standalone {
        spec.hint_text_standalone.as_str()
    } else {
        spec.hint_text_contextual.as_str()
    };
    tagged.push(TaggedArea {
        section: "hint",
        index: 0,
        channel: picker_channel("hint", 0),
        area: make_area(
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
    });

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
        tagged.push(TaggedArea {
            section: "sat_bar",
            index: i,
            channel: picker_channel("sat_bar", i),
            area: make_area(
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
        });
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
        tagged.push(TaggedArea {
            section: "val_bar",
            index: i,
            channel: picker_channel("val_bar", i),
            area: make_area(
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
        });
    }

    // Centre preview glyph ࿕.
    let preview_size = layout.preview_size;
    let commit_hovered = matches!(geometry.hovered_hit, Some(PickerHit::Commit));
    let commit_color = if commit_hovered {
        highlight_hovered_cell_color(preview_rgb)
    } else {
        preview_color
    };
    let preview_scale_f = if commit_hovered { hover_scale } else { 1.0 };
    let scaled_preview = preview_size * preview_scale_f;
    let center_font = Some(baumhard::font::fonts::AppFont::NotoSerifTibetanRegular);
    let preview_glyph_center = (
        layout.preview_pos.0 + preview_size * 0.5,
        layout.preview_pos.1 + preview_size * 0.5,
    );
    let preview_box_w = scaled_preview * 1.5;
    let preview_box_h = scaled_preview * 1.5;
    tagged.push(TaggedArea {
        section: "preview",
        index: 0,
        channel: picker_channel("preview", 0),
        area: make_area(
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
    });

    // Hex readout — always emitted at a stable channel so the mutator
    // path doesn't have to handle a flickering element. Empty text
    // when invisible.
    let (hex_text, hex_pos, hex_bounds) = match layout.hex_pos {
        Some(anchor) => (
            hsv_to_hex(geometry.hue_deg, geometry.sat, geometry.val),
            anchor,
            (font_size * 8.0, font_size * 1.5),
        ),
        None => (String::new(), (0.0, 0.0), (0.0, 0.0)),
    };
    tagged.push(TaggedArea {
        section: "hex",
        index: 0,
        channel: picker_channel("hex", 0),
        area: make_area(
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
    });

    // Bake into the two-form result. `tagged` was built in the
    // canonical insertion order (matches the spec's section order),
    // so `ordered` is channel-ascending already.
    let mut ordered: Vec<(usize, GlyphArea)> = Vec::with_capacity(tagged.len());
    let mut by_section: HashMap<&'static str, Vec<Option<usize>>> = HashMap::new();
    for TaggedArea {
        section,
        index,
        channel,
        area,
    } in tagged
    {
        let vec_index = ordered.len();
        ordered.push((channel, area));
        let row = by_section.entry(section).or_default();
        if row.len() <= index {
            row.resize(index + 1, None);
        }
        row[index] = Some(vec_index);
    }

    PickerAreas { ordered, by_section }
}

/// Backward-compat shim — returns the channel-ordered list for
/// callers that don't need the section-keyed lookup (initial-build
/// path + some existing tests).
#[cfg(test)]
pub(super) fn picker_glyph_areas(
    geometry: &crate::application::color_picker::ColorPickerOverlayGeometry,
    layout: &crate::application::color_picker::ColorPickerLayout,
) -> Vec<(usize, GlyphArea)> {
    compute_picker_areas(geometry, layout).ordered
}
