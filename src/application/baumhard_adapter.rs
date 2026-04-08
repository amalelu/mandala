use cosmic_text::{Attrs, AttrsList, Color, Family, FontSystem, Style};
use log::error;
use baumhard::util::color;
use baumhard::font::fonts;
use baumhard::core::primitives::ColorFontRegions;

pub fn to_cosmic_text(source: &ColorFontRegions, font_system: &mut FontSystem) -> AttrsList {
   let mut attr_list = AttrsList::new(Attrs::new());
   for region in &source.regions {
      let mut attrs = Attrs::new();
      attrs = attrs.style(Style::Normal);
      if region.color.is_some() {
         let color = color::convert_f32_to_u8(region.color.as_ref().unwrap());
         attrs = attrs.color(Color::rgba(color[0], color[1], color[2], color[3]));
      }
      if region.font.is_some() {
         let font = fonts::COMPILED_FONT_ID_MAP
            .get(region.font.as_ref().unwrap())
            .unwrap();
         let face = font_system.db().face(font[0]).unwrap();
         let derp = face.families[0].0.as_ref();
         error!("{}", derp);
         attrs = attrs.family(Family::Name(derp));
      } else {
         attrs = attrs.family(Family::Monospace);
      }
      attr_list.add_span(region.range.to_rust_range(), attrs);
   }
   attr_list
}