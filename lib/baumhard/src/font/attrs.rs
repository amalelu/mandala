//! Attribute-list construction for cosmic-text spans.
//!
//! Bridges baumhard's `ColorFontRegions` (the model-level
//! representation of styled text runs) into a cosmic-text `AttrsList`
//! that the renderer can hand to `Editor::insert_string`. Lives in
//! `font/` so all cosmic-text styling goes through a single blessed
//! module — see `CODE_CONVENTIONS.md` §2 and `CONVENTIONS.md` §B4.

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
/// unknown or unresolvable font id falls back to `Family::Monospace`
/// with a warning — this function runs inside the renderer's frame
/// loop and a corrupt save must not abort it.
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

        // Resolve the font family. Both miss paths (compiled-id map
        // miss, fontdb face miss) fall back to Monospace with a
        // warning — consistent with §4's "degrade the frame, not
        // abort the process" rule.
        let family = resolve_font_family(region.font.as_ref(), font_system);
        attrs = match family {
            Some(ref name) => attrs.family(Family::Name(name.as_str())),
            None => attrs.family(Family::Monospace),
        };
        attr_list.add_span(region.range.to_rust_range(), &attrs);
    }
    attr_list
}

/// Look up the font-family name for a compiled font id. Returns
/// `None` (monospace fallback) with a warning on any miss.
fn resolve_font_family(
    font_id: Option<&crate::font::fonts::AppFont>,
    font_system: &mut FontSystem,
) -> Option<String> {
    let font_id = font_id?;
    let face_ids = match COMPILED_FONT_ID_MAP.get(font_id) {
        Some(ids) if !ids.is_empty() => ids,
        _ => {
            warn!("attrs_list_from_regions: unknown font id {font_id:?}, falling back to Monospace");
            return None;
        }
    };
    match font_system.db().face(face_ids[0]) {
        Some(face) => Some(face.families[0].0.clone()),
        None => {
            warn!("attrs_list_from_regions: fontdb face miss for {font_id:?}, falling back to Monospace");
            None
        }
    }
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
