use crate::font::fonts::AppFont;
use crate::util::color::Color;
use crate::gfx_structs::model::{GlyphComponent, GlyphLine, GlyphMatrix};
use crate::core::primitives::{ColorFontRegions, Range};

/// The tests are written in a non-test-annotated function and then wrapped by an annotated test function
/// So that they can be reused for benchmarking

#[test]
pub fn test_matrix_place_in_1() {
   matrix_place_in_1();
}

pub fn matrix_place_in_1() {
   let mut matrix = GlyphMatrix::new();
   matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "##########",
      AppFont::Evilz,
      Color::black(),
   )));
   matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "##########",
      AppFont::Evilz,
      Color::black(),
   )));
   matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "##########",
      AppFont::Evilz,
      Color::black(),
   )));

   let mut regions = ColorFontRegions::new_empty();
   let mut my_string = String::new();

   matrix.place_in(&mut my_string, &mut regions, (10, 10));
   assert_matrix_place_in(&my_string, &regions);
   // Just asserting that the operation is idempotent
   matrix.place_in(&mut my_string, &mut regions, (10, 10));
   assert_matrix_place_in(&my_string, &regions);
   matrix.place_in(&mut my_string, &mut regions, (10, 10));
   assert_matrix_place_in(&my_string, &regions);
   matrix.place_in(&mut my_string, &mut regions, (10, 10));
   assert_matrix_place_in(&my_string, &regions);
   matrix.place_in(&mut my_string, &mut regions, (10, 10));
   assert_matrix_place_in(&my_string, &regions);
}

fn assert_matrix_place_in(my_string: &String, regions: &ColorFontRegions) {
   assert_eq!(
      my_string,
      "\n\n\n\n\n\n\n\n\n\n          ##########\n          ##########\n          ##########"
   );
   assert_eq!(regions.num_regions(), 3);
   let first_region = regions.regions.first();
   assert_eq!(first_region.is_some(), true);
   let unwrapped_first_region = first_region.unwrap();
   assert_eq!(unwrapped_first_region.range, Range::new(20, 30));
   assert_eq!(
      unwrapped_first_region.color.unwrap(),
      Color::black().to_float()
   );
   assert_eq!(unwrapped_first_region.font.unwrap(), AppFont::Evilz);
   let second_region = regions.get(Range::new(41, 51));
   assert_eq!(second_region.is_some(), true);
   let unwrapped_second_region = second_region.unwrap();
   assert_eq!(unwrapped_second_region.range, Range::new(41, 51));
   assert_eq!(
      unwrapped_second_region.color.unwrap(),
      Color::black().to_float()
   );
   assert_eq!(unwrapped_second_region.font.unwrap(), AppFont::Evilz);
   let third_region = regions.get(Range::new(62, 72));
   assert_eq!(third_region.is_some(), true);
   let unwrapped_third_region = third_region.unwrap();
   assert_eq!(unwrapped_third_region.range, Range::new(62, 72));
   assert_eq!(
      unwrapped_third_region.color.unwrap(),
      Color::black().to_float()
   );
   assert_eq!(unwrapped_third_region.font.unwrap(), AppFont::Evilz);
}

#[test]
pub fn test_matrix_place_in_2() {
   matrix_place_in_2();
}

pub fn matrix_place_in_2() {
   let mut matrix_a = GlyphMatrix::new();
   matrix_a.push(GlyphLine::new_with(GlyphComponent::text(
      "##########",
      AppFont::Evilz,
      Color::black(),
   )));
   matrix_a.push(GlyphLine::new_with(GlyphComponent::text(
      "##########",
      AppFont::Evilz,
      Color::black(),
   )));
   matrix_a.push(GlyphLine::new_with(GlyphComponent::text(
      "##########",
      AppFont::Evilz,
      Color::black(),
   )));

   let mut matrix_b = GlyphMatrix::new();
   matrix_b.push(GlyphLine::new_with(GlyphComponent::text(
      "@@@@@@@@@@",
      AppFont::AppleTea,
      Color::white(),
   )));
   matrix_b.push(GlyphLine::new_with(GlyphComponent::text(
      "@@@@@@@@@@",
      AppFont::AppleTea,
      Color::white(),
   )));
   matrix_b.push(GlyphLine::new_with(GlyphComponent::text(
      "@@@@@@@@@@",
      AppFont::AppleTea,
      Color::white(),
   )));

   let mut regions = ColorFontRegions::new_empty();
   let mut my_string = String::new();
   matrix_a.place_in(&mut my_string, &mut regions, (0, 0));
   assert_eq!(my_string, "##########\n##########\n##########");
   assert_eq!(regions.num_regions(), 3);
   {
      let _region_1 = regions.get(Range::new(0, 10)).unwrap();
      let _region_2 = regions.get(Range::new(11, 21)).unwrap();
      let _region_3 = regions.get(Range::new(22, 32)).unwrap();
   }
   matrix_b.place_in(&mut my_string, &mut regions, (10, 0));
   assert_eq!(
      my_string,
      "##########@@@@@@@@@@\n##########@@@@@@@@@@\n##########@@@@@@@@@@"
   );
   assert_eq!(regions.num_regions(), 6);
   {
      // Corresponds to previous
      let _region_1 = regions.get(Range::new(0, 10)).unwrap();
      let _region_2 = regions.get(Range::new(21, 31)).unwrap();
      let _region_3 = regions.get(Range::new(42, 52)).unwrap();
      // New regions
      let _region_4 = regions.get(Range::new(10, 20)).unwrap();
      let _region_5 = regions.get(Range::new(31, 41)).unwrap();
      let _region_6= regions.get(Range::new(52, 62)).unwrap();
   }
   matrix_a.place_in(&mut my_string, &mut regions, (10, 3));
   assert_eq!(my_string, "##########@@@@@@@@@@\n##########@@@@@@@@@@\n##########@@@@@@@@@@\n          ##########\n          ##########\n          ##########");
   assert_eq!(regions.num_regions(), 9);
   {
      // Corresponds to previous
      let _region_1 = regions.get(Range::new(0, 10)).unwrap();
      let _region_2 = regions.get(Range::new(21, 31)).unwrap();
      let _region_3 = regions.get(Range::new(42, 52)).unwrap();
      let _region_4 = regions.get(Range::new(10, 20)).unwrap();
      let _region_5 = regions.get(Range::new(31, 41)).unwrap();
      let _region_6 = regions.get(Range::new(52, 62)).unwrap();
      // new regions
      let _region_7 = regions.get(Range::new(73, 83)).unwrap();
      let _region_8 = regions.get(Range::new(94, 104)).unwrap();
      let _region_9 = regions.get(Range::new(115, 125)).unwrap();
   }
   matrix_b.place_in(&mut my_string, &mut regions, (0, 3));
   assert_eq!(my_string, "##########@@@@@@@@@@\n##########@@@@@@@@@@\n##########@@@@@@@@@@\n@@@@@@@@@@##########\n@@@@@@@@@@##########\n@@@@@@@@@@##########");
   assert_eq!(regions.num_regions(), 12);
   {
      // Corresponds to previous
      let _region_1 = regions.get(Range::new(0, 10)).unwrap();
      let _region_2 = regions.get(Range::new(21, 31)).unwrap();
      let _region_3 = regions.get(Range::new(42, 52)).unwrap();
      let _region_4 = regions.get(Range::new(10, 20)).unwrap();
      let _region_5 = regions.get(Range::new(31, 41)).unwrap();
      let _region_6 = regions.get(Range::new(52, 62)).unwrap();
      let _region_7 = regions.get(Range::new(73, 83)).unwrap();
      let _region_8 = regions.get(Range::new(94, 104)).unwrap();
      let _region_9 = regions.get(Range::new(115, 125)).unwrap();
      // new regions
      let _region_10 = regions.get(Range::new(63, 73)).unwrap();
      let _region_11 = regions.get(Range::new(84, 94)).unwrap();
      let _region_12 = regions.get(Range::new(105, 115)).unwrap();
   }
}

#[test]
pub fn test_matrix_place_in_3() {
   matrix_place_in_3();
}

pub fn matrix_place_in_3() {
   let mut matrix_a = GlyphMatrix::new();
   matrix_a.push(GlyphLine::new_with(GlyphComponent::text(
      "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻",
      AppFont::AlphaMusicMan,
      Color::black(),
   )));
   matrix_a.push(GlyphLine::new_with(GlyphComponent::text(
      "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻",
      AppFont::Evilz,
      Color::black(),
   )));
   matrix_a.push(GlyphLine::new_with(GlyphComponent::text(
      "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻",
      AppFont::AliceInWonderland,
      Color::black(),
   )));

   let mut matrix_b = GlyphMatrix::new();
   matrix_b.push(GlyphLine::new_with(GlyphComponent::text(
      "@@@@@@@@@@",
      AppFont::AppleTea,
      Color::white(),
   )));
   matrix_b.push(GlyphLine::new_with(GlyphComponent::text(
      "@@@@@@@@@@",
      AppFont::AppleTea,
      Color::white(),
   )));
   matrix_b.push(GlyphLine::new_with(GlyphComponent::text(
      "@@@@@@@@@@",
      AppFont::AppleTea,
      Color::white(),
   )));

   let mut regions = ColorFontRegions::new_empty();
   let mut my_string = String::new();
   matrix_a.place_in(&mut my_string, &mut regions, (0, 0));
   assert_eq!(my_string, "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻");
   assert_eq!(regions.num_regions(), 3);
   {
      let _region_1 = regions.get(Range::new(0, 10)).unwrap();
      let _region_2 = regions.get(Range::new(11, 21)).unwrap();
      let _region_3 = regions.get(Range::new(22, 32)).unwrap();
   }
   matrix_b.place_in(&mut my_string, &mut regions, (10, 0));
   assert_eq!(
      my_string,
      "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@"
   );
   assert_eq!(regions.num_regions(), 6);
   {
      // Corresponds to previous
      let _region_1 = regions.get(Range::new(0, 10)).unwrap();
      let _region_2 = regions.get(Range::new(21, 31)).unwrap();
      let _region_3 = regions.get(Range::new(42, 52)).unwrap();
      // New regions
      let _region_4 = regions.get(Range::new(10, 20)).unwrap();
      let _region_5 = regions.get(Range::new(31, 41)).unwrap();
      let _region_6 = regions.get(Range::new(52, 62)).unwrap();
   }
   matrix_a.place_in(&mut my_string, &mut regions, (10, 3));
   assert_eq!(my_string,
              "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n          🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n          🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n          🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻");
   assert_eq!(regions.num_regions(), 9);
   {
      // Corresponds to previous
      let _region_1 = regions.get(Range::new(0, 10)).unwrap();
      let _region_2 = regions.get(Range::new(21, 31)).unwrap();
      let _region_3 = regions.get(Range::new(42, 52)).unwrap();
      let _region_4 = regions.get(Range::new(10, 20)).unwrap();
      let _region_5 = regions.get(Range::new(31, 41)).unwrap();
      let _region_6 = regions.get(Range::new(52, 62)).unwrap();
      // new regions
      let _region_7 = regions.get(Range::new(73, 83)).unwrap();
      let _region_8 = regions.get(Range::new(94, 104)).unwrap();
      let _region_9 = regions.get(Range::new(115, 125)).unwrap();
   }
   matrix_b.place_in(&mut my_string, &mut regions, (0, 3));
   assert_eq!(my_string, "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻@@@@@@@@@@\n@@@@@@@@@@🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n@@@@@@@@@@🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻\n@@@@@@@@@@🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻");
   assert_eq!(regions.num_regions(), 12);
   {
      // Corresponds to previous
      let _region_1 = regions.get(Range::new(0, 10)).unwrap();
      let _region_2 = regions.get(Range::new(21, 31)).unwrap();
      let _region_3 = regions.get(Range::new(42, 52)).unwrap();
      let _region_4 = regions.get(Range::new(10, 20)).unwrap();
      let _region_5 = regions.get(Range::new(31, 41)).unwrap();
      let _region_6 = regions.get(Range::new(52, 62)).unwrap();
      let _region_7 = regions.get(Range::new(73, 83)).unwrap();
      let _region_8 = regions.get(Range::new(94, 104)).unwrap();
      let _region_9 = regions.get(Range::new(115, 125)).unwrap();
      // new regions
      let _region_10 = regions.get(Range::new(63, 73)).unwrap();
      let _region_11 = regions.get(Range::new(84, 94)).unwrap();
      let _region_12 = regions.get(Range::new(105, 115)).unwrap();
   }
}

#[test]
pub fn test_matrix_add_assign_2() {
   matrix_add_assign_2();
}

pub fn matrix_add_assign_2() {
   let mut matrix = GlyphMatrix::new();

   matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "##########",
      AppFont::Evilz,
      Color::black(),
   )));
   matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "##########",
      AppFont::Evilz,
      Color::black(),
   )));
   matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "##########",
      AppFont::Evilz,
      Color::black(),
   )));
   matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "##########",
      AppFont::Evilz,
      Color::black(),
   )));
   matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "##########",
      AppFont::Evilz,
      Color::black(),
   )));
   matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "##########",
      AppFont::Evilz,
      Color::black(),
   )));
   matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "##########",
      AppFont::Evilz,
      Color::black(),
   )));
   matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "##########",
      AppFont::Evilz,
      Color::black(),
   )));
   matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "##########",
      AppFont::Evilz,
      Color::black(),
   )));
   matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "##########",
      AppFont::Evilz,
      Color::black(),
   )));

   let mut modifier_matrix = GlyphMatrix::new();
   modifier_matrix.push(GlyphLine::new());
   modifier_matrix.push(GlyphLine::new());
   modifier_matrix.push(GlyphLine::new());

   modifier_matrix.push(GlyphLine::new_with_vec(
      vec![
         GlyphComponent::space(3),
         GlyphComponent::text("HELP", AppFont::Evilz, Color::black()),
      ],
      true,
   ));

   modifier_matrix.push(GlyphLine::new_with_vec(
      vec![
         GlyphComponent::space(3),
         GlyphComponent::text("HELP", AppFont::Evilz, Color::black()),
      ],
      true,
   ));

   modifier_matrix.push(GlyphLine::new_with_vec(
      vec![
         GlyphComponent::space(3),
         GlyphComponent::text("HELP", AppFont::Evilz, Color::black()),
      ],
      true,
   ));

   modifier_matrix.push(GlyphLine::new_with_vec(
      vec![
         GlyphComponent::space(3),
         GlyphComponent::text("HELP", AppFont::Evilz, Color::black()),
      ],
      true,
   ));

   matrix += modifier_matrix;

   assert_eq!(matrix.get(0).unwrap().get(0).unwrap().text, "##########");
   assert_eq!(matrix.get(1).unwrap().get(0).unwrap().text, "##########");
   assert_eq!(matrix.get(2).unwrap().get(0).unwrap().text, "##########");

   assert_eq!(matrix.get(3).unwrap().get(0).unwrap().text, "###");
   assert_eq!(matrix.get(3).unwrap().get(1).unwrap().text, "HELP");
   assert_eq!(matrix.get(3).unwrap().get(2).unwrap().text, "###");

   assert_eq!(matrix.get(4).unwrap().get(0).unwrap().text, "###");
   assert_eq!(matrix.get(4).unwrap().get(1).unwrap().text, "HELP");
   assert_eq!(matrix.get(4).unwrap().get(2).unwrap().text, "###");

   assert_eq!(matrix.get(5).unwrap().get(0).unwrap().text, "###");
   assert_eq!(matrix.get(5).unwrap().get(1).unwrap().text, "HELP");
   assert_eq!(matrix.get(5).unwrap().get(2).unwrap().text, "###");

   assert_eq!(matrix.get(6).unwrap().get(0).unwrap().text, "###");
   assert_eq!(matrix.get(6).unwrap().get(1).unwrap().text, "HELP");
   assert_eq!(matrix.get(6).unwrap().get(2).unwrap().text, "###");

   assert_eq!(matrix.get(7).unwrap().get(0).unwrap().text, "##########");
   assert_eq!(matrix.get(8).unwrap().get(0).unwrap().text, "##########");
   assert_eq!(matrix.get(9).unwrap().get(0).unwrap().text, "##########");
}

#[test]
pub fn test_matrix_add_assign_1() {
   matrix_add_assign_1();
}

pub fn matrix_add_assign_1() {
   let mut matrix = create_default_matrix();

   let mut modifier_matrix = GlyphMatrix::new();
   modifier_matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "HELP",
      AppFont::Evilz,
      Color::black(),
   )));
   modifier_matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "HELP",
      AppFont::Evilz,
      Color::black(),
   )));
   modifier_matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "HELP",
      AppFont::Evilz,
      Color::black(),
   )));
   modifier_matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "HELP",
      AppFont::Evilz,
      Color::black(),
   )));

   matrix += modifier_matrix;

   assert_eq!(matrix.get(0).unwrap().get(0).unwrap().text, "HELP");
   assert_eq!(matrix.get(0).unwrap().get(1).unwrap().text, "######");
   assert_eq!(matrix.get(1).unwrap().get(0).unwrap().text, "HELP");
   assert_eq!(matrix.get(1).unwrap().get(1).unwrap().text, "######");
   assert_eq!(matrix.get(2).unwrap().get(0).unwrap().text, "HELP");
   assert_eq!(matrix.get(2).unwrap().get(1).unwrap().text, "######");
   assert_eq!(matrix.get(3).unwrap().get(0).unwrap().text, "HELP");
   assert_eq!(matrix.get(3).unwrap().get(1).unwrap().text, "######");
   assert_eq!(matrix.get(4).unwrap().get(0).unwrap().text, "##########");
   assert_eq!(matrix.get(5).unwrap().get(0).unwrap().text, "##########");
   assert_eq!(matrix.get(6).unwrap().get(0).unwrap().text, "##########");
   assert_eq!(matrix.get(7).unwrap().get(0).unwrap().text, "##########");
   assert_eq!(matrix.get(8).unwrap().get(0).unwrap().text, "##########");
   assert_eq!(matrix.get(9).unwrap().get(0).unwrap().text, "##########");
}

#[test]
pub fn test_matrix_mul_assign_1() {
   matrix_mul_assign_1();
}

pub fn matrix_mul_assign_1() {
   let mut matrix = create_default_matrix();

   let mut modifier_matrix = GlyphMatrix::new();
   modifier_matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "HELP",
      AppFont::Evilz,
      Color::black(),
   )));
   modifier_matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "HELP",
      AppFont::Evilz,
      Color::black(),
   )));
   modifier_matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "HELP",
      AppFont::Evilz,
      Color::black(),
   )));
   modifier_matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "HELP",
      AppFont::Evilz,
      Color::black(),
   )));

   matrix *= modifier_matrix;

   assert_eq!(matrix.get(0).unwrap().get(0).unwrap().text, "HELP");
   assert_eq!(matrix.get(0).unwrap().get(1).unwrap().text, "######");
   assert_eq!(matrix.get(1).unwrap().get(0).unwrap().text, "HELP");
   assert_eq!(matrix.get(1).unwrap().get(1).unwrap().text, "######");
   assert_eq!(matrix.get(2).unwrap().get(0).unwrap().text, "HELP");
   assert_eq!(matrix.get(2).unwrap().get(1).unwrap().text, "######");
   assert_eq!(matrix.get(3).unwrap().get(0).unwrap().text, "HELP");
   assert_eq!(matrix.get(3).unwrap().get(1).unwrap().text, "######");
   assert_eq!(matrix.get(4).unwrap().get(0).unwrap().text, "##########");
   assert_eq!(matrix.get(5).unwrap().get(0).unwrap().text, "##########");
   assert_eq!(matrix.get(6).unwrap().get(0).unwrap().text, "##########");
   assert_eq!(matrix.get(7).unwrap().get(0).unwrap().text, "##########");
   assert_eq!(matrix.get(8).unwrap().get(0).unwrap().text, "##########");
   assert_eq!(matrix.get(9).unwrap().get(0).unwrap().text, "##########");
}

fn create_default_matrix() -> GlyphMatrix {
   let mut matrix = GlyphMatrix::new();

   matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "##########",
      AppFont::Evilz,
      Color::white(),
   )));
   matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "##########",
      AppFont::Evilz,
      Color::white(),
   )));
   matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "##########",
      AppFont::Evilz,
      Color::white(),
   )));
   matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "##########",
      AppFont::Evilz,
      Color::white(),
   )));
   matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "##########",
      AppFont::Evilz,
      Color::white(),
   )));
   matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "##########",
      AppFont::Evilz,
      Color::white(),
   )));
   matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "##########",
      AppFont::Evilz,
      Color::white(),
   )));
   matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "##########",
      AppFont::Evilz,
      Color::white(),
   )));
   matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "##########",
      AppFont::Evilz,
      Color::white(),
   )));
   matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "##########",
      AppFont::Evilz,
      Color::white(),
   )));
   matrix.push(GlyphLine::new_with(GlyphComponent::text(
      "##########",
      AppFont::Evilz,
      Color::white(),
   )));

   return matrix;
}

#[test]
pub fn test_line_add_assign_1() {
   line_add_assign_1();
}

// Pretty straight forward, simple test case
pub fn line_add_assign_1() {
   let mut glyph_line = GlyphLine::new();
   glyph_line.push(GlyphComponent::text(
      "##########",
      AppFont::AppleTea,
      Color::black(),
   ));
   glyph_line.push(GlyphComponent::text(
      "@@@@@@@@@@",
      AppFont::AppleTea,
      Color::black(),
   ));
   glyph_line.push(GlyphComponent::text(
      "**********",
      AppFont::AppleTea,
      Color::black(),
   ));

   let mut modifier_line = GlyphLine::new();
   modifier_line.push(GlyphComponent::text(
      "!!!!!!!!!!!",
      AppFont::AliceInWonderland,
      Color::white(),
   ));
   glyph_line += modifier_line;
   assert_eq!(glyph_line.get(0).unwrap().text, "!!!!!!!!!!!");
   assert_eq!(glyph_line.get(1).unwrap().text, "@@@@@@@@@");
   assert_eq!(glyph_line.get(2).unwrap().text, "**********");
}

#[test]
pub fn test_line_add_assign_2() {
   line_add_assign_2();
}

// Here we test the ability to ignore initial whitespace, while also respecting the indices
pub fn line_add_assign_2() {
   let mut glyph_line = GlyphLine::new();
   glyph_line.push(GlyphComponent::text(
      "##########",
      AppFont::AppleTea,
      Color::black(),
   ));
   glyph_line.push(GlyphComponent::text(
      "@@@@@@@@@@",
      AppFont::AppleTea,
      Color::black(),
   ));
   glyph_line.push(GlyphComponent::text(
      "**********",
      AppFont::AppleTea,
      Color::black(),
   ));

   let mut modifier_line = GlyphLine::new();
   modifier_line.ignore_initial_space = true;
   modifier_line.push(GlyphComponent::space(20));
   modifier_line.push(GlyphComponent::text(
      "!!!!!!!!!!",
      AppFont::AliceInWonderland,
      Color::white(),
   ));
   glyph_line += modifier_line;
   assert_eq!(glyph_line.get(0).unwrap().text, "##########");
   assert_eq!(glyph_line.get(1).unwrap().text, "@@@@@@@@@@");
   assert_eq!(glyph_line.get(2).unwrap().text, "!!!!!!!!!!");
}

#[test]
pub fn test_line_add_assign_3() {
   line_add_assign_3();
}

// Here we test the ability to handle emojis
pub fn line_add_assign_3() {
   let mut glyph_line = GlyphLine::new();
   glyph_line.push(GlyphComponent::text(
      "##########",
      AppFont::AppleTea,
      Color::black(),
   ));
   glyph_line.push(GlyphComponent::text(
      "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻",
      AppFont::AppleTea,
      Color::black(),
   ));
   glyph_line.push(GlyphComponent::text(
      "**********",
      AppFont::AppleTea,
      Color::black(),
   ));

   let mut modifier_line = GlyphLine::new();
   modifier_line.ignore_initial_space = true;
   modifier_line.push(GlyphComponent::space(20));
   modifier_line.push(GlyphComponent::text(
      "🍕🍕🍕🍕🍕🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻",
      AppFont::AliceInWonderland,
      Color::white(),
   ));
   glyph_line += modifier_line;
   assert_eq!(glyph_line.get(0).unwrap().text, "##########");
   assert_eq!(
      glyph_line.get(1).unwrap().text,
      "🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻"
   );
   assert_eq!(
      glyph_line.get(2).unwrap().text,
      "🍕🍕🍕🍕🍕🙏🏻🙏🏻🙏🏻🙏🏻🙏🏻"
   );
}

#[test]
pub fn test_line_add_assign_4() {
   line_add_assign_4();
}

pub fn line_add_assign_4() {
   let mut glyph_line = GlyphLine::new();
   glyph_line.push(GlyphComponent::text(
      "##############################",
      AppFont::AppleTea,
      Color::black(),
   ));

   let mut modifier_line = GlyphLine::new();
   modifier_line.ignore_initial_space = true;
   modifier_line.push(GlyphComponent::space(10));
   modifier_line.push(GlyphComponent::text(
      "!!!!!!!!!!",
      AppFont::AliceInWonderland,
      Color::white(),
   ));
   glyph_line += modifier_line;
   assert_eq!(glyph_line.get(0).unwrap().text, "##########");
   assert_eq!(glyph_line.get(1).unwrap().text, "!!!!!!!!!!");
   assert_eq!(glyph_line.get(2).unwrap().text, "##########");
}
#[test]
pub fn test_component_of_index() {
   component_of_index();
}

pub fn component_of_index() {
   let mut glyph_line = GlyphLine::new();
   glyph_line.push(GlyphComponent::space(10)); //0
   glyph_line.push(GlyphComponent::space(10)); //1
   glyph_line.push(GlyphComponent::space(10)); //2
   let a = glyph_line.component_of_index(0);
   assert_eq!(a, 0);
   let b = glyph_line.component_of_index(9);
   assert_eq!(b, 0);
   let c = glyph_line.component_of_index(10);
   assert_eq!(c, 1);
   let d = glyph_line.component_of_index(15);
   assert_eq!(d, 1);
   let e = glyph_line.component_of_index(20);
   assert_eq!(e, 2);
   let f = glyph_line.component_of_index(29);
   assert_eq!(f, 2);
}

#[test]
pub fn test_index_of_component() {
   index_of_component();
}

pub fn index_of_component() {
   let mut glyph_line = GlyphLine::new();
   glyph_line.push(GlyphComponent::space(10)); //0
   glyph_line.push(GlyphComponent::space(10)); //1
   glyph_line.push(GlyphComponent::space(10)); //2
   let a = glyph_line.index_of_component(0);
   assert_eq!(a, 0);
   let b = glyph_line.index_of_component(1);
   assert_eq!(b, 10);
   let c = glyph_line.index_of_component(2);
   assert_eq!(c, 20);
}

#[test]
pub fn test_expanding_insert_1() {
   expanding_insert_1();
}

pub fn expanding_insert_1() {
   let mut glyph_line = GlyphLine::new();
   glyph_line.push(GlyphComponent::space(10)); //0
   glyph_line.push(GlyphComponent::text(
      "12",
      AppFont::AppleTea,
      Color::black(),
   )); //1
   glyph_line.push(GlyphComponent::space(10)); //2
   assert_eq!(glyph_line.line.len(), 3);
   // This should now insert itself between the 1 and the 2
   glyph_line.expanding_insert(
      11,
      &GlyphComponent::text("onetwo", AppFont::Evilz, Color::black()),
   );
   // Two space comps + "1", and "2", and "onetwo" = 5
   assert_eq!(glyph_line.line.len(), 5);
   assert_eq!(glyph_line.get(1).unwrap().text, "1");
   assert_eq!(glyph_line.get(1).unwrap().font, AppFont::AppleTea);
   assert_eq!(glyph_line.get(2).unwrap().text, "onetwo");
   assert_eq!(glyph_line.get(2).unwrap().font, AppFont::Evilz);
   assert_eq!(glyph_line.get(3).unwrap().text, "2");
   assert_eq!(glyph_line.get(3).unwrap().font, AppFont::AppleTea);
}

#[test]
pub fn test_expanding_insert_2() {
   expanding_insert_2();
}

pub fn expanding_insert_2() {
   let mut glyph_line = GlyphLine::new();
   glyph_line.push(GlyphComponent::space(10)); //0
   glyph_line.push(GlyphComponent::text(
      "12",
      AppFont::AppleTea,
      Color::black(),
   )); //1
   glyph_line.push(GlyphComponent::space(10)); //2
   assert_eq!(glyph_line.line.len(), 3);

   glyph_line.expanding_insert(
      10,
      &GlyphComponent::text("onetwo", AppFont::Evilz, Color::black()),
   );

   assert_eq!(glyph_line.line.len(), 4);
   assert_eq!(glyph_line.get(1).unwrap().text, "onetwo");
   assert_eq!(glyph_line.get(1).unwrap().font, AppFont::Evilz);
   assert_eq!(glyph_line.get(2).unwrap().text, "12");
   assert_eq!(glyph_line.get(2).unwrap().font, AppFont::AppleTea);
}

#[test]
pub fn test_expanding_insert_3() {
   expanding_insert_3();
}

pub fn expanding_insert_3() {
   let mut glyph_line = GlyphLine::new();
   glyph_line.push(GlyphComponent::space(10)); //0
   glyph_line.push(GlyphComponent::text(
      "12",
      AppFont::AppleTea,
      Color::black(),
   )); //1
   glyph_line.push(GlyphComponent::space(10)); //2
   assert_eq!(glyph_line.line.len(), 3);

   glyph_line.expanding_insert(
      12,
      &GlyphComponent::text("onetwo", AppFont::Evilz, Color::black()),
   );

   assert_eq!(glyph_line.line.len(), 4);

   assert_eq!(glyph_line.get(1).unwrap().text, "12");
   assert_eq!(glyph_line.get(1).unwrap().font, AppFont::AppleTea);
   assert_eq!(glyph_line.get(2).unwrap().text, "onetwo");
   assert_eq!(glyph_line.get(2).unwrap().font, AppFont::Evilz);
}

#[test]
pub fn test_expanding_insert_4() {
   expanding_insert_4();
}

pub fn expanding_insert_4() {
   let mut glyph_line = GlyphLine::new();
   glyph_line.expanding_insert(
      0,
      &GlyphComponent::text("123", AppFont::African, Color::black()),
   );
   assert_eq!(glyph_line.line.len(), 1);
   assert_eq!(glyph_line.line.get(0).unwrap().text, "123");
}

#[test]
pub fn test_expanding_insert_5() {
   expanding_insert_5();
}

pub fn expanding_insert_5() {
   let mut glyph_line = GlyphLine::new();
   glyph_line.expanding_insert(
      10,
      &GlyphComponent::text("123", AppFont::African, Color::black()),
   );
   assert_eq!(glyph_line.line.len(), 2);
   assert_eq!(
      glyph_line.line.get(0).unwrap().text,
      GlyphComponent::space(10).text
   );
   assert_eq!(glyph_line.line.get(1).unwrap().text, "123");
}

#[test]
pub fn test_expanding_insert_6() {
   expanding_insert_6();
}

pub fn expanding_insert_6() {
   let mut glyph_line = GlyphLine::new();
   glyph_line.push(GlyphComponent::space(5));
   glyph_line.expanding_insert(
      10,
      &GlyphComponent::text("123", AppFont::African, Color::black()),
   );
   assert_eq!(glyph_line.line.len(), 2);
   assert_eq!(
      glyph_line.line.get(0).unwrap().text,
      GlyphComponent::space(10).text
   );
   assert_eq!(glyph_line.line.get(1).unwrap().text, "123");
}

#[test]
pub fn test_expanding_insert_7() {
   expanding_insert_7();
}

pub fn expanding_insert_7() {
   let mut glyph_line = GlyphLine::new();
   glyph_line.push(GlyphComponent::space(10));
   glyph_line.expanding_insert(
      10,
      &GlyphComponent::text("123", AppFont::African, Color::black()),
   );
   assert_eq!(glyph_line.line.len(), 2);
   assert_eq!(
      glyph_line.line.get(0).unwrap().text,
      GlyphComponent::space(10).text
   );
   assert_eq!(glyph_line.line.get(1).unwrap().text, "123");
}

#[test]
pub fn test_overriding_insert_1() {
   overriding_insert_1();
}

pub fn overriding_insert_1() {
   let mut glyph_line = GlyphLine::new();
   glyph_line.push(GlyphComponent::space(20)); //0
   glyph_line.push(GlyphComponent::space(20)); //1
   glyph_line.push(GlyphComponent::space(20)); //2
   assert_eq!(glyph_line.line.len(), 3);
   glyph_line.overriding_insert(10, &GlyphComponent::space(40));
   assert_eq!(glyph_line.line.len(), 3);
   assert_eq!(glyph_line.get(0).unwrap().length(), 10);
   assert_eq!(glyph_line.get(1).unwrap().length(), 40);
   assert_eq!(glyph_line.get(2).unwrap().length(), 10);
}
#[test]
pub fn test_overriding_insert_2() {
   overriding_insert_2();
}

pub fn overriding_insert_2() {
   let mut glyph_line = GlyphLine::new();
   glyph_line.push(GlyphComponent::space(20)); //0
   glyph_line.push(GlyphComponent::space(20)); //1
   glyph_line.push(GlyphComponent::space(20)); //2
   glyph_line.push(GlyphComponent::space(20)); //3
   glyph_line.push(GlyphComponent::space(20)); //4
   glyph_line.push(GlyphComponent::space(20)); //5
   assert_eq!(glyph_line.line.len(), 6);
   glyph_line.overriding_insert(10, &GlyphComponent::space(100));
   assert_eq!(glyph_line.line.len(), 3);
   assert_eq!(glyph_line.get(0).unwrap().length(), 10);
   assert_eq!(glyph_line.get(1).unwrap().length(), 100);
   assert_eq!(glyph_line.get(2).unwrap().length(), 10);
}

#[test]
pub fn test_overriding_insert_3() {
   overriding_insert_3();
}

pub fn overriding_insert_3() {
   let mut glyph_line = GlyphLine::new();
   glyph_line.push(GlyphComponent::space(20)); //0
   glyph_line.push(GlyphComponent::space(20)); //1
   glyph_line.push(GlyphComponent::space(20)); //2
   assert_eq!(glyph_line.line.len(), 3);
   glyph_line.overriding_insert(60, &GlyphComponent::space(40));
   assert_eq!(glyph_line.line.len(), 4);
   assert_eq!(glyph_line.get(0).unwrap().length(), 20);
   assert_eq!(glyph_line.get(1).unwrap().length(), 20);
   assert_eq!(glyph_line.get(2).unwrap().length(), 20);
   assert_eq!(glyph_line.get(3).unwrap().length(), 40);
}

#[test]
pub fn test_overriding_insert_4() {
   overriding_insert_4();
}

pub fn overriding_insert_4() {
   let mut glyph_line = GlyphLine::new();
   glyph_line.push(GlyphComponent::space(20)); //0
   glyph_line.push(GlyphComponent::space(20)); //1
   glyph_line.push(GlyphComponent::space(20)); //2
   assert_eq!(glyph_line.line.len(), 3);
   glyph_line.overriding_insert(0, &GlyphComponent::space(40));
   assert_eq!(glyph_line.line.len(), 2);
   assert_eq!(glyph_line.get(0).unwrap().length(), 40);
   assert_eq!(glyph_line.get(1).unwrap().length(), 20);
}

#[test]
pub fn test_overriding_insert_5() {
   overriding_insert_5();
}

pub fn overriding_insert_5() {
   let mut glyph_line = GlyphLine::new();
   glyph_line.push(GlyphComponent::space(20)); //0
   glyph_line.push(GlyphComponent::space(20)); //1
   glyph_line.push(GlyphComponent::space(20)); //2
   assert_eq!(glyph_line.line.len(), 3);
   glyph_line.overriding_insert(0, &GlyphComponent::space(20));
   assert_eq!(glyph_line.line.len(), 3);
   assert_eq!(glyph_line.get(0).unwrap().length(), 20);
   assert_eq!(glyph_line.get(1).unwrap().length(), 20);
   assert_eq!(glyph_line.get(2).unwrap().length(), 20);
}

#[test]
pub fn test_overriding_insert_6() {
   overriding_insert_6();
}

pub fn overriding_insert_6() {
   let mut glyph_line = GlyphLine::new();
   glyph_line.push(GlyphComponent::space(20)); //0
   glyph_line.push(GlyphComponent::space(20)); //1
   glyph_line.push(GlyphComponent::space(20)); //2
   assert_eq!(glyph_line.line.len(), 3);
   for _i in 0..3639 {
      glyph_line.overriding_insert(0, &GlyphComponent::space(20));
   }
   assert_eq!(glyph_line.line.len(), 3);
   assert_eq!(glyph_line.get(0).unwrap().length(), 20);
   assert_eq!(glyph_line.get(1).unwrap().length(), 20);
   assert_eq!(glyph_line.get(2).unwrap().length(), 20);
}

#[test]
pub fn test_overriding_insert_7() {
   overriding_insert_7();
}

pub fn overriding_insert_7() {
   let mut glyph_line = GlyphLine::new();
   glyph_line.push(GlyphComponent::space(1)); //0
   glyph_line.push(GlyphComponent::space(1)); //1
   glyph_line.push(GlyphComponent::space(1)); //2
   assert_eq!(glyph_line.line.len(), 3);
   glyph_line.overriding_insert(3, &GlyphComponent::space(1));
   assert_eq!(glyph_line.line.len(), 4);

   glyph_line.overriding_insert(4, &GlyphComponent::space(1));
   assert_eq!(glyph_line.line.len(), 5);

   glyph_line.overriding_insert(5, &GlyphComponent::space(1));
   assert_eq!(glyph_line.line.len(), 6);

   glyph_line.overriding_insert(6, &GlyphComponent::space(1));
   assert_eq!(glyph_line.line.len(), 7);

   glyph_line.overriding_insert(7, &GlyphComponent::space(1));
   assert_eq!(glyph_line.line.len(), 8);

   glyph_line.overriding_insert(8, &GlyphComponent::space(1));
   assert_eq!(glyph_line.line.len(), 9);

   assert_eq!(glyph_line.get(0).unwrap().length(), 1);
   assert_eq!(glyph_line.get(1).unwrap().length(), 1);
   assert_eq!(glyph_line.get(2).unwrap().length(), 1);
   assert_eq!(glyph_line.get(3).unwrap().length(), 1);
   assert_eq!(glyph_line.get(4).unwrap().length(), 1);
   assert_eq!(glyph_line.get(5).unwrap().length(), 1);
   assert_eq!(glyph_line.get(6).unwrap().length(), 1);
   assert_eq!(glyph_line.get(7).unwrap().length(), 1);
   assert_eq!(glyph_line.get(8).unwrap().length(), 1);
}

#[test]
pub fn test_overriding_insert_8() {
   overriding_insert_8();
}

pub fn overriding_insert_8() {
   let mut glyph_line = GlyphLine::new();

   glyph_line.push(GlyphComponent::text(
      "0123456789",
      AppFont::AppleTea,
      Color::black(),
   ));
   glyph_line.push(GlyphComponent::text(
      "abcdefghij",
      AppFont::AliceInWonderland,
      Color::white(),
   ));
   assert_eq!(glyph_line.line.len(), 2);
   glyph_line.overriding_insert(
      10,
      &GlyphComponent::text("x🙏🏻🍕", AppFont::Any, Color::black()),
   );
   assert_eq!(glyph_line.line.len(), 3);
   assert_eq!(glyph_line.get(0).unwrap().as_str(), "0123456789");
   assert_eq!(glyph_line.get(1).unwrap().as_str(), "x🙏🏻🍕");
   assert_eq!(glyph_line.get(2).unwrap().as_str(), "defghij");
   assert_eq!(glyph_line.get(0).unwrap().font, AppFont::AppleTea);
   assert_eq!(glyph_line.get(1).unwrap().font, AppFont::Any);
   assert_eq!(glyph_line.get(2).unwrap().font, AppFont::AliceInWonderland);
}

#[test]
pub fn test_overriding_insert_9() {
   overriding_insert_9();
}

pub fn overriding_insert_9() {
   let mut glyph_line = GlyphLine::new();

   glyph_line.push(GlyphComponent::text(
      "0123456789",
      AppFont::AppleTea,
      Color::black(),
   ));
   glyph_line.push(GlyphComponent::text(
      "abcdefghij",
      AppFont::AliceInWonderland,
      Color::white(),
   ));
   assert_eq!(glyph_line.line.len(), 2);
   glyph_line.overriding_insert(
      7,
      &GlyphComponent::text("x🙏🏻🍕", AppFont::Any, Color::black()),
   );
   assert_eq!(glyph_line.line.len(), 3);
   assert_eq!(glyph_line.get(0).unwrap().as_str(), "0123456");
   assert_eq!(glyph_line.get(1).unwrap().as_str(), "x🙏🏻🍕");
   assert_eq!(glyph_line.get(2).unwrap().as_str(), "abcdefghij");
   assert_eq!(glyph_line.get(0).unwrap().font, AppFont::AppleTea);
   assert_eq!(glyph_line.get(1).unwrap().font, AppFont::Any);
   assert_eq!(glyph_line.get(2).unwrap().font, AppFont::AliceInWonderland);
   assert_eq!(glyph_line.get(0).unwrap().color, Color::black());
   assert_eq!(glyph_line.get(1).unwrap().color, Color::black());
   assert_eq!(glyph_line.get(2).unwrap().color, Color::white());
}

#[test]
pub fn test_overriding_insert_10() {
   overriding_insert_10();
}

pub fn overriding_insert_10() {
   let mut glyph_line = GlyphLine::new();

   glyph_line.push(GlyphComponent::text(
      "0123456789",
      AppFont::AppleTea,
      Color::black(),
   ));
   glyph_line.push(GlyphComponent::text(
      "abcdefghij",
      AppFont::AliceInWonderland,
      Color::white(),
   ));
   glyph_line.push(GlyphComponent::text(
      "Ook? Ook! Ook? Ook.",
      AppFont::Evilz,
      Color::black(),
   ));
   assert_eq!(glyph_line.line.len(), 3);
   glyph_line.overriding_insert(
      7,
      &GlyphComponent::text("Nanananananananana", AppFont::Any, Color::black()),
   );

   assert_eq!(glyph_line.line.len(), 3);
   assert_eq!(glyph_line.get(0).unwrap().as_str(), "0123456");
   assert_eq!(glyph_line.get(1).unwrap().as_str(), "Nanananananananana");
   assert_eq!(glyph_line.get(2).unwrap().as_str(), "Ook! Ook? Ook.");
   assert_eq!(glyph_line.get(0).unwrap().font, AppFont::AppleTea);
   assert_eq!(glyph_line.get(1).unwrap().font, AppFont::Any);
   assert_eq!(glyph_line.get(2).unwrap().font, AppFont::Evilz);
   assert_eq!(glyph_line.get(0).unwrap().color, Color::black());
   assert_eq!(glyph_line.get(1).unwrap().color, Color::black());
   assert_eq!(glyph_line.get(2).unwrap().color, Color::black());
}

#[test]
pub fn test_overriding_insert_11() {
   overriding_insert_11();
}

pub fn overriding_insert_11() {
   let mut glyph_line = GlyphLine::new();

   glyph_line.push(GlyphComponent::text(
      "0123456789",
      AppFont::AppleTea,
      Color::black(),
   ));
   assert_eq!(glyph_line.line.len(), 1);
   glyph_line.overriding_insert(
      10,
      &GlyphComponent::text("10", AppFont::Any, Color::black()),
   );
   assert_eq!(glyph_line.line.len(), 2);
   assert_eq!(glyph_line.get(0).unwrap().as_str(), "0123456789");
   assert_eq!(glyph_line.get(1).unwrap().as_str(), "10");
}

#[test]
pub fn test_overriding_insert_12() {
   overriding_insert_12();
}

pub fn overriding_insert_12() {
   let mut glyph_line = GlyphLine::new();

   glyph_line.push(GlyphComponent::text(
      "0123456789",
      AppFont::AppleTea,
      Color::black(),
   ));
   assert_eq!(glyph_line.line.len(), 1);
   glyph_line.overriding_insert(
      11,
      &GlyphComponent::text("10", AppFont::AlphaMusicMan, Color::black()),
   );
   assert_eq!(glyph_line.line.len(), 3);
   assert_eq!(glyph_line.get(0).unwrap().as_str(), "0123456789");
   assert_eq!(glyph_line.get(1).unwrap().as_str(), " ");
   assert_eq!(glyph_line.get(2).unwrap().as_str(), "10");
   assert_eq!(glyph_line.get(0).unwrap().font, AppFont::AppleTea);
   assert_eq!(glyph_line.get(1).unwrap().font, AppFont::Any);
   assert_eq!(glyph_line.get(1).unwrap().color, Color::invisible());
   assert_eq!(glyph_line.get(2).unwrap().font, AppFont::AlphaMusicMan);
   assert_eq!(glyph_line.get(2).unwrap().color, Color::black());
}

#[test]
pub fn test_overriding_insert_13() {
   overriding_insert_13();
}

pub fn overriding_insert_13() {
   let mut glyph_line = GlyphLine::new();

   glyph_line.push(GlyphComponent::text(
      "0123456789",
      AppFont::AppleTea,
      Color::black(),
   ));
   assert_eq!(glyph_line.line.len(), 1);
   glyph_line.overriding_insert(
      15,
      &GlyphComponent::text("10", AppFont::AppleTea, Color::black()),
   );
   assert_eq!(glyph_line.line.len(), 3);
   assert_eq!(glyph_line.get(0).unwrap().as_str(), "0123456789");
   assert_eq!(glyph_line.get(1).unwrap().as_str(), "     ");
   assert_eq!(glyph_line.get(2).unwrap().as_str(), "10");
   assert_eq!(glyph_line.get(0).unwrap().font, AppFont::AppleTea);
   assert_eq!(glyph_line.get(1).unwrap().font, AppFont::Any);
   assert_eq!(glyph_line.get(1).unwrap().color, Color::invisible());
   assert_eq!(glyph_line.get(2).unwrap().font, AppFont::AppleTea);
   assert_eq!(glyph_line.get(2).unwrap().color, Color::black());
}
