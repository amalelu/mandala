//! Single source of truth for the picker's per-section
//! `GlyphArea` set, plus the tree- and mutator-builders that wrap it
//! into the shapes the renderer registers. Everything that turns
//! `(geometry, layout)` into a registered overlay or an in-place
//! mutator lives here; the initial-build path and the §B2 mutator
//! paths cannot drift because they all read from the same
//! [`PickerAreas`] table built by [`compute_picker_areas`].
//!
//! The section names ("title", "hue_ring", "hint", "sat_bar",
//! "val_bar", "preview", "hex") must match the
//! `mutator_spec.sections[*].section` strings in
//! `widgets/color_picker.json` — the spec's channel layout is
//! authoritative and the fn below fills the cells it asks for.

use baumhard::core::primitives::ColorFontRegions;
use baumhard::gfx_structs::area::{GlyphArea, OutlineStyle};
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::GfxMutator;
use baumhard::gfx_structs::tree::{MutatorTree, Tree};
use glam::Vec2;

use super::color::{
    highlight_hovered_cell_color, highlight_selected_cell_color, rgb_to_cosmic_color,
};
use super::glyph_model::glyph_model_from_picker_area;
use crate::application::color_picker::{HUE_SLOT_COUNT, SAT_CELL_COUNT, VAL_CELL_COUNT};
use crate::application::mutator_builder::{self, SectionContext};

/// All `GlyphArea`s the picker will emit on one apply cycle.
///
/// `ordered` is the channel-ascending `(channel, area)` list the
/// initial-build path walks to seat each cell at the right channel.
/// The per-section `[Option<usize>; N]` arrays index into `ordered` so
/// the mutator path can resolve
/// `mutator_builder::SectionContext::area(section, index)` calls
/// without scanning or doing channel math. The arrays are inline on
/// the struct (no per-frame heap allocation) — sized at compile time
/// from the per-section constants in `color_picker.rs`. `None` slots
/// mark intentionally-skipped indices (e.g. the centre crosshair cell
/// at `sat_bar[8]` / `val_bar[8]`); calling `area` on a skipped slot
/// is a programming error.
pub(super) struct PickerAreas {
    pub(super) ordered: Vec<(usize, GlyphArea)>,
    title: [Option<usize>; 1],
    hue_ring: [Option<usize>; HUE_SLOT_COUNT],
    hint: [Option<usize>; 1],
    sat_bar: [Option<usize>; SAT_CELL_COUNT],
    val_bar: [Option<usize>; VAL_CELL_COUNT],
    preview: [Option<usize>; 1],
    hex: [Option<usize>; 1],
}

/// Compile-time enum mirror of the picker's JSON section names. Lets
/// `PickerAreas::area` translate the spec's `&str` section key into a
/// branch on a known-shape array without a HashMap. The `from_name`
/// match panics on an unknown section since the JSON / Rust drift
/// would be a programming error, not a recoverable state.
#[derive(Copy, Clone, Debug)]
enum PickerSection {
    Title,
    HueRing,
    Hint,
    SatBar,
    ValBar,
    Preview,
    Hex,
}

impl PickerSection {
    fn from_name(name: &str) -> Self {
        match name {
            "title" => PickerSection::Title,
            "hue_ring" => PickerSection::HueRing,
            "hint" => PickerSection::Hint,
            "sat_bar" => PickerSection::SatBar,
            "val_bar" => PickerSection::ValBar,
            "preview" => PickerSection::Preview,
            "hex" => PickerSection::Hex,
            other => panic!("picker area lookup: unknown section {other:?}"),
        }
    }
}

impl PickerAreas {
    /// Resolve a `(section, index) → &GlyphArea` lookup. Panics if
    /// the section wasn't populated (the spec / builder disagree
    /// on what sections exist) or the requested index was deliberately
    /// skipped (e.g. the crosshair centre slot at sat_bar[8]) —
    /// the picker apply path treats both as a programming error
    /// rather than a recoverable state.
    pub(super) fn area(&self, section: &str, index: usize) -> &GlyphArea {
        let slot: Option<usize> = match PickerSection::from_name(section) {
            PickerSection::Title => self.title.get(index).copied().flatten(),
            PickerSection::HueRing => self.hue_ring.get(index).copied().flatten(),
            PickerSection::Hint => self.hint.get(index).copied().flatten(),
            PickerSection::SatBar => self.sat_bar.get(index).copied().flatten(),
            PickerSection::ValBar => self.val_bar.get(index).copied().flatten(),
            PickerSection::Preview => self.preview.get(index).copied().flatten(),
            PickerSection::Hex => self.hex.get(index).copied().flatten(),
        };
        let vec_index = slot.unwrap_or_else(|| {
            panic!(
                "picker area lookup: section {section:?} index {index} was not populated \
                 (skipped or out-of-range)"
            )
        });
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

    // Build directly into the final two-form layout — no intermediate
    // `Vec<TaggedArea>`, no HashMap. The picker emits a fixed 60 cells
    // across 7 sections, all sized at compile time, so the per-section
    // index arrays sit on the stack as `Option<usize>` slots and the
    // single `Vec<(usize, GlyphArea)>` holds the channel-ascending
    // payload.
    let mut areas = PickerAreas {
        ordered: Vec::with_capacity(60),
        title: [None; 1],
        hue_ring: [None; HUE_SLOT_COUNT],
        hint: [None; 1],
        sat_bar: [None; SAT_CELL_COUNT],
        val_bar: [None; VAL_CELL_COUNT],
        preview: [None; 1],
        hex: [None; 1],
    };

    // Closure-free helper: push a built area into `ordered` and stamp
    // its position into the matching per-section slot.
    fn push(
        areas: &mut PickerAreas,
        section: PickerSection,
        index: usize,
        channel: usize,
        area: GlyphArea,
    ) {
        let vec_index = areas.ordered.len();
        areas.ordered.push((channel, area));
        let slot = match section {
            PickerSection::Title => areas.title.get_mut(index),
            PickerSection::HueRing => areas.hue_ring.get_mut(index),
            PickerSection::Hint => areas.hint.get_mut(index),
            PickerSection::SatBar => areas.sat_bar.get_mut(index),
            PickerSection::ValBar => areas.val_bar.get_mut(index),
            PickerSection::Preview => areas.preview.get_mut(index),
            PickerSection::Hex => areas.hex.get_mut(index),
        };
        *slot.expect("picker area builder: index past compile-time section size") = Some(vec_index);
    }

    // Title.
    let is_standalone = geometry.target_label.is_empty();
    let title_text = if is_standalone {
        spec.title_template_standalone.clone()
    } else {
        spec.title_template_contextual
            .replace("{target_label}", geometry.target_label)
    };
    push(
        &mut areas,
        PickerSection::Title,
        0,
        picker_channel("title", 0),
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
    );

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
        push(
            &mut areas,
            PickerSection::HueRing,
            i,
            picker_channel("hue_ring", i),
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
        );
    }

    // Hint footer.
    let hint_text = if is_standalone {
        spec.hint_text_standalone.as_str()
    } else {
        spec.hint_text_contextual.as_str()
    };
    push(
        &mut areas,
        PickerSection::Hint,
        0,
        picker_channel("hint", 0),
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
    );

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
        push(
            &mut areas,
            PickerSection::SatBar,
            i,
            picker_channel("sat_bar", i),
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
        );
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
        push(
            &mut areas,
            PickerSection::ValBar,
            i,
            picker_channel("val_bar", i),
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
        );
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
    push(
        &mut areas,
        PickerSection::Preview,
        0,
        picker_channel("preview", 0),
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
    );

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
    push(
        &mut areas,
        PickerSection::Hex,
        0,
        picker_channel("hex", 0),
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
    );

    areas
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

/// Build the color-picker overlay tree from a geometry +
/// pre-computed layout. Iterates the
/// [channel-ascending][PickerAreas::ordered] list so the registered
/// tree's channels match what the mutator path will later target.
///
/// Tree shape (each GlyphArea has a paired GlyphModel child built by
/// [`glyph_model_from_picker_area`]):
///
/// ```text
/// Void (root)
/// ├── GlyphArea title bar / hue ring / hint / sat bar / val bar / preview / hex
/// │   └── GlyphModel mirror
/// └── …
/// ```
///
/// **Performance note**: this rebuilds every glyph on every
/// `rebuild_color_picker_overlay_buffers` call, which is reserved
/// for picker open / close and tree-shape changes. Per-frame
/// updates go through [`build_color_picker_overlay_dynamic_mutator`]
/// — same arena, slim per-cell delta.
pub(super) fn build_color_picker_overlay_tree(
    geometry: &crate::application::color_picker::ColorPickerOverlayGeometry,
    layout: &crate::application::color_picker::ColorPickerLayout,
) -> Tree<GfxElement, GfxMutator> {
    let areas = compute_picker_areas(geometry, layout);
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    for (channel, area) in &areas.ordered {
        let model = glyph_model_from_picker_area(area);
        let area_element = GfxElement::new_area_non_indexed_with_id(area.clone(), *channel, *channel);
        let area_id = tree.arena.new_node(area_element);
        tree.root.append(area_id, &mut tree.arena);
        let model_element =
            GfxElement::new_model_non_indexed_with_id(model, *channel, *channel);
        let model_id = tree.arena.new_node(model_element);
        area_id.append(model_id, &mut tree.arena);
    }
    tree
}

/// Build a [`MutatorTree`] that updates an already-registered picker
/// tree to the current `(geometry, layout)` state without rebuilding
/// the arena. The tree shape is declared in
/// `widgets/color_picker.json`'s `mutator_spec` (the **layout** spec
/// — full per-cell field set); the per-cell `GlyphArea` values come
/// from [`PickerAreas`] via a [`PickerSectionContext`] adapter.
///
/// This is the §B2 "mutation, not rebuild" path for layout-change
/// events: initial open, viewport resize, and RMB size_scale drag.
/// Hover / HSV / chip frames go through
/// [`build_color_picker_overlay_dynamic_mutator`] instead — same
/// channel layout, slimmer per-section field lists.
pub(super) fn build_color_picker_overlay_mutator(
    geometry: &crate::application::color_picker::ColorPickerOverlayGeometry,
    layout: &crate::application::color_picker::ColorPickerLayout,
) -> MutatorTree<GfxMutator> {
    let spec = crate::application::widgets::color_picker_widget::load_spec();
    let areas = compute_picker_areas(geometry, layout);
    let ctx = PickerSectionContext { areas: &areas };
    mutator_builder::build(&spec.mutator_spec, &ctx)
}

/// Per-frame [`MutatorTree`] for the picker — the **dynamic** phase.
/// Walked from `widgets/color_picker.json`'s `dynamic_mutator_spec`,
/// which carries the same channel layout as `mutator_spec` but only
/// the per-section `CellField`s that actually change between hover /
/// HSV / drag frames (color, hover scale, hex text). Position,
/// bounds, line_height, and outline come from the layout phase and
/// stay untouched here.
///
/// **Cost note**: this still routes through [`compute_picker_areas`]
/// today, so the per-frame work is dominated by full GlyphArea
/// construction even though the mutator only reads a subset of
/// fields. Closing that gap is the next consolidation step (slim
/// per-section context + `SectionContext::dynamic_field`); pinned by
/// the `dynamic_mutator_spec_per_section_fields_are_slim` test in
/// `widgets::color_picker_widget`.
pub(super) fn build_color_picker_overlay_dynamic_mutator(
    geometry: &crate::application::color_picker::ColorPickerOverlayGeometry,
    layout: &crate::application::color_picker::ColorPickerLayout,
) -> MutatorTree<GfxMutator> {
    let spec = crate::application::widgets::color_picker_widget::load_spec();
    let areas = compute_picker_areas(geometry, layout);
    let ctx = PickerSectionContext { areas: &areas };
    mutator_builder::build(&spec.dynamic_mutator_spec, &ctx)
}

/// Adapter implementing [`SectionContext`] on top of a precomputed
/// [`PickerAreas`] table. Only `area(section, index)` is wired — the
/// picker spec uses no runtime counts, runtime mutations, or macros,
/// so the other trait methods keep their `unreachable!()` defaults.
struct PickerSectionContext<'a> {
    areas: &'a PickerAreas,
}

impl<'a> SectionContext for PickerSectionContext<'a> {
    fn area(&self, section: &str, index: usize) -> &GlyphArea {
        self.areas.area(section, index)
    }
}
