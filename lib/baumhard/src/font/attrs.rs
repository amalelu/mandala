//! Attribute-list construction for cosmic-text spans.
//!
//! Bridges baumhard's `ColorFontRegions` (the model-level
//! representation of styled text runs) into a cosmic-text `AttrsList`
//! that the renderer can hand to `Editor::insert_string`. Lives in
//! `font/` because it's the canonical, blessed point of contact
//! between application data and cosmic-text — see
//! [`crate::CONVENTIONS`](../../CONVENTIONS.md) §2 / §B4.
//!
//! Used to be a hand-rolled `to_cosmic_text` helper in the app crate
//! (`src/application/baumhard_adapter.rs`) that reached straight into
//! `font_system.db().face(...)` and panicked on three different
//! `unwrap()`s. Centralised here so the app crate has zero direct
//! cosmic-text usage on the styling path.

use cosmic_text::{Attrs, AttrsList, Color, Family, FontSystem, Style};
use log::warn;

use crate::core::primitives::ColorFontRegions;
use crate::font::fonts::COMPILED_FONT_ID_MAP;
use crate::util::color::convert_f32_to_u8;

/// Build a cosmic-text `AttrsList` from a `ColorFontRegions` source.
///
/// One span is emitted per region. A region with `color = Some(rgba)`
/// gets that color; otherwise the span uses cosmic-text's default. A
/// region with `font = Some(id)` resolves to that font family; an
/// unknown font id falls back to `Family::Monospace` with a warning,
/// rather than panicking — this function runs inside the renderer's
/// frame loop and a corrupt save must not abort it.
///
/// Cost: O(n_regions) iteration plus one `font_system.db().face()`
/// lookup per region with a font id. The caller is expected to hold
/// the `FONT_SYSTEM` write lock for the same scope it uses the
/// returned list — that's how the renderer wires it today.
pub fn attrs_list_from_regions(
    source: &ColorFontRegions,
    font_system: &mut FontSystem,
) -> AttrsList {
    let mut attr_list = AttrsList::new(&Attrs::new());
    for region in &source.regions {
        let mut attrs = Attrs::new().style(Style::Normal);

        if let Some(color) = region.color.as_ref() {
            let rgba = convert_f32_to_u8(color);
            attrs = attrs.color(Color::rgba(rgba[0], rgba[1], rgba[2], rgba[3]));
        }

        // Resolve the font family lazily so a missing font id degrades
        // to monospace instead of panicking on .unwrap(). Both lookups
        // (compiled-id map miss, fontdb face miss) hit the same
        // fallback.
        let resolved_family: Option<String> = match region.font.as_ref() {
            Some(font_id) => {
                let face_ids = COMPILED_FONT_ID_MAP.get(font_id);
                let face_ids = match face_ids {
                    Some(ids) if !ids.is_empty() => ids,
                    _ => {
                        warn!(
                            "attrs_list_from_regions: unknown font id {:?}, falling back to Monospace",
                            font_id
                        );
                        attr_list.add_span(region.range.to_rust_range(), &attrs.family(Family::Monospace));
                        continue;
                    }
                };
                font_system
                    .db()
                    .face(face_ids[0])
                    .map(|face| face.families[0].0.clone())
            }
            None => None,
        };

        match resolved_family {
            Some(family) => {
                // Reborrow the family string for the span lifetime —
                // AttrsList copies internally on `add_span`, so the
                // local owns the storage just long enough.
                attrs = attrs.family(Family::Name(family.as_str()));
                attr_list.add_span(region.range.to_rust_range(), &attrs);
            }
            None => {
                attrs = attrs.family(Family::Monospace);
                attr_list.add_span(region.range.to_rust_range(), &attrs);
            }
        }
    }
    attr_list
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::primitives::{ColorFontRegion, Range};

    /// Empty regions produce an empty span list. The defaults stored
    /// inside `AttrsList` are not exposed via `spans()`, so an empty
    /// input gives a length-0 span list.
    #[test]
    fn test_attrs_list_from_empty_regions_yields_no_spans() {
        // We don't need to load fonts here because the function only
        // touches the FontSystem inside the per-region loop, which
        // never runs on an empty input.
        let regions = ColorFontRegions::new_empty();
        let mut fs = FontSystem::new();
        let list = attrs_list_from_regions(&regions, &mut fs);
        assert_eq!(list.spans().len(), 0);
    }

    /// A single region with a color and no font produces one span,
    /// with the color converted from f32 to u8 internally.
    #[test]
    fn test_attrs_list_from_single_color_region_emits_one_span() {
        let mut regions = ColorFontRegions::new_empty();
        regions.submit_region(ColorFontRegion::new(
            Range::new(0, 5),
            None,
            Some([1.0, 0.0, 0.0, 1.0]),
        ));
        let mut fs = FontSystem::new();
        let list = attrs_list_from_regions(&regions, &mut fs);
        assert_eq!(list.spans().len(), 1);
    }

    /// Two adjacent regions emit two spans. Guards against the
    /// inherited region pipeline collapsing distinct ranges into one.
    #[test]
    fn test_attrs_list_from_two_regions_emits_two_spans() {
        let mut regions = ColorFontRegions::new_empty();
        regions.submit_region(ColorFontRegion::new(
            Range::new(0, 5),
            None,
            Some([1.0, 0.0, 0.0, 1.0]),
        ));
        regions.submit_region(ColorFontRegion::new(
            Range::new(5, 10),
            None,
            Some([0.0, 1.0, 0.0, 1.0]),
        ));
        let mut fs = FontSystem::new();
        let list = attrs_list_from_regions(&regions, &mut fs);
        assert_eq!(list.spans().len(), 2);
    }
}
