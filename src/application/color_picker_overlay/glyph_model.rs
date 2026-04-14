//! `GlyphModel` mirror for a picker `GlyphArea`. Peer file to the
//! picker's `GlyphArea`-side construction inside
//! [`super::picker_glyph_areas`] — both are baumhard `GfxElement`
//! variants, both get stamped into the picker overlay tree by
//! [`super::picker_glyph_areas::build_color_picker_overlay_tree`].

use baumhard::font::fonts::AppFont;
use baumhard::gfx_structs::area::GlyphArea;
use glam::Vec2;

/// Build a `GlyphModel` mirroring a picker `GlyphArea`'s text +
/// dominant color/font, used as the model child attached to each
/// picker GlyphArea by
/// [`super::picker_glyph_areas::build_color_picker_overlay_tree`].
///
/// Establishes the architectural pattern the user requested in the
/// color-picker restructure: every picker piece is a (GlyphArea
/// view + GlyphModel source-of-truth) pair. Today the renderer only
/// reads the `GlyphArea` (a `GlyphModel` "is essentially stamped
/// into a GlyphArea" — the matrix's text + regions are produced by
/// `make_area` directly rather than by `GlyphMatrix::place_in`), so
/// the model node is structural — present in the tree, ignored by
/// the renderer's `walk_tree_into_buffers` (which skips
/// `GlyphModel` and `Void` variants). It exists so future per-glyph
/// mutation / animation work can target the model and re-stamp into
/// the parent area without rebuilding the arena.
///
/// **Mutator-path interaction**: the §B2 `apply_color_picker_overlay_mutator`
/// path stays GlyphArea-only — it does not produce mutator children
/// for these model nodes. That's safe because Baumhard's
/// `align_child_walks` returns immediately when the mutator node
/// has no children (`tree_walker.rs:237-240`), so the GlyphModel
/// child is never traversed by the in-place update path.
///
/// Picker GlyphAreas use `ColorFontRegions::single_span` (one region
/// covering all chars), so the first region carries the
/// authoritative color + optional font pin for the whole text. We
/// pull both, drop them into a single-component `GlyphLine`, and
/// position the model at the same screen-space anchor as the area
/// so the mirrored matrix is geometrically consistent with the
/// rendered ink. Float-to-byte color conversion uses `.round()`
/// (not truncation) so the round-trip through float regions →
/// byte components → float regions is symmetric to the picker's
/// own byte→float conversion in `make_area`.
#[inline]
pub(super) fn glyph_model_from_picker_area(
    area: &GlyphArea,
) -> baumhard::gfx_structs::model::GlyphModel {
    use baumhard::gfx_structs::model::{GlyphComponent, GlyphLine, GlyphModel};
    use baumhard::util::color::Color as BaumhardColor;
    use baumhard::util::ordered_vec2::OrderedVec2;

    let mut model = GlyphModel::new();
    model.position = OrderedVec2::from_vec2(Vec2::new(area.position.x.0, area.position.y.0));

    if area.text.is_empty() {
        return model;
    }

    let regions = area.regions.all_regions();
    let (font, color) = match regions.first() {
        Some(r) => {
            let font = r.font.unwrap_or(AppFont::Any);
            let color = r
                .color
                .map(|fc| {
                    BaumhardColor::new_u8(&[
                        (fc[0].clamp(0.0, 1.0) * 255.0).round() as u8,
                        (fc[1].clamp(0.0, 1.0) * 255.0).round() as u8,
                        (fc[2].clamp(0.0, 1.0) * 255.0).round() as u8,
                        (fc[3].clamp(0.0, 1.0) * 255.0).round() as u8,
                    ])
                })
                .unwrap_or_else(BaumhardColor::black);
            (font, color)
        }
        None => (AppFont::Any, BaumhardColor::black()),
    };

    model.add_line(GlyphLine::new_with(GlyphComponent::text(
        &area.text,
        font,
        color,
    )));
    model
}
