use glam::Vec2;
use crate::font::fonts;
use crate::gfx_structs::element::{GfxElement, GfxElementField};
use crate::gfx_structs::area::{GlyphArea, GlyphAreaCommand, GlyphAreaField};
use crate::gfx_structs::tree::{MutatorTree, Tree};
use crate::gfx_structs::mutator::{GfxMutator, Mutation};
use crate::gfx_structs::mutator::Instruction::RepeatWhile;
use crate::gfx_structs::predicate::{Comparator, Predicate};
use crate::gfx_structs::tree_walker::{walk_tree, walk_tree_from};
use crate::util::ordered_vec2::OrderedVec2;

#[test]
pub fn test_repeat_while_skip_while() {
   repeat_while_skip_while();
}

pub fn repeat_while_skip_while() {
   // This is necessary to initialize lazy statics
   fonts::init();

   let root_element = GfxElement::new_area_non_indexed_with_id(
      GlyphArea::new_with_str(
         "root",
         1.0,
         10.0,
         Vec2::new(50.0, 50.0),
         Vec2::new(500.0, 500.0),
      ),
      0,
      0,
   );
   let mut mutator: MutatorTree<GfxMutator> = MutatorTree::new();
   let mut model: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed_with(root_element);


   let head_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
      GlyphArea::new(1.0, 10.0, Vec2::new(100.0, 100.0), Vec2::new(100.0, 100.0)),
      0,
      1,
   ));

   let neck_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
      GlyphArea::new(1.0, 10.0, Vec2::new(100.0, 110.0), Vec2::new(100.0, 100.0)),
      0,
      2,
   ));

   let torso_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
      GlyphArea::new(1.0, 10.0, Vec2::new(100.0, 120.0), Vec2::new(100.0, 100.0)),
      0,
      3,
   ));

   /////////////// Arms ///////////////

   let left_shoulder_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
      GlyphArea::new(1.0, 10.0, Vec2::new(90.0, 120.0), Vec2::new(100.0, 100.0)),
      0,
      4,
   ));

   let right_shoulder_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
      GlyphArea::new(1.0, 10.0, Vec2::new(110.0, 120.0), Vec2::new(100.0, 100.0)),
      0,
      5,
   ));

   let left_upper_arm_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
      GlyphArea::new(1.0, 10.0, Vec2::new(90.0, 130.0), Vec2::new(100.0, 100.0)),
      0,
      6,
   ));

   let right_upper_arm_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
      GlyphArea::new(1.0, 10.0, Vec2::new(110.0, 130.0), Vec2::new(100.0, 100.0)),
      0,
      7,
   ));

   let left_lower_arm_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
      GlyphArea::new(1.0, 10.0, Vec2::new(90.0, 140.0), Vec2::new(100.0, 100.0)),
      0,
      8,
   ));

   let right_lower_arm_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
      GlyphArea::new(1.0, 10.0, Vec2::new(110.0, 140.0), Vec2::new(100.0, 100.0)),
      0,
      9,
   ));

   /////////////// Legs ///////////////

   let left_thigh_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
      GlyphArea::new(1.0, 10.0, Vec2::new(95.0, 130.0), Vec2::new(100.0, 100.0)),
      0,
      10,
   ));

   let right_thigh_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
      GlyphArea::new(1.0, 10.0, Vec2::new(105.0, 130.0), Vec2::new(100.0, 100.0)),
      0,
      11,
   ));

   let left_knee_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
      GlyphArea::new(1.0, 10.0, Vec2::new(95.0, 140.0), Vec2::new(100.0, 100.0)),
      0,
      12,
   ));

   let right_knee_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
      GlyphArea::new(1.0, 10.0, Vec2::new(105.0, 140.0), Vec2::new(100.0, 100.0)),
      0,
      13,
   ));

   let left_lower_leg_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
      GlyphArea::new(1.0, 10.0, Vec2::new(95.0, 150.0), Vec2::new(100.0, 100.0)),
      0,
      14,
   ));

   let right_lower_leg_id = model.arena.new_node(GfxElement::new_area_non_indexed_with_id(
      GlyphArea::new(1.0, 10.0, Vec2::new(105.0, 150.0), Vec2::new(100.0, 100.0)),
      0,
      15,
   ));

   model.root.append(head_id, &mut model.arena);
   head_id.append(neck_id, &mut model.arena);
   neck_id.append(torso_id, &mut model.arena);

   torso_id.append(left_shoulder_id, &mut model.arena);
   torso_id.append(right_shoulder_id, &mut model.arena);
   torso_id.append(left_thigh_id, &mut model.arena);
   torso_id.append(right_thigh_id, &mut model.arena);

   left_shoulder_id.append(left_upper_arm_id, &mut model.arena);
   left_upper_arm_id.append(left_lower_arm_id, &mut model.arena);

   right_shoulder_id.append(right_upper_arm_id, &mut model.arena);
   right_upper_arm_id.append(right_lower_arm_id, &mut model.arena);

   left_thigh_id.append(left_knee_id, &mut model.arena);
   left_knee_id.append(left_lower_leg_id, &mut model.arena);

   right_thigh_id.append(right_knee_id, &mut model.arena);
   right_knee_id.append(right_lower_leg_id, &mut model.arena);

   // Now make the mutator
   let mut predicate = Predicate::new();
   predicate.fields.push((
      GfxElementField::GlyphArea(GlyphAreaField::Position(OrderedVec2::new_f32(105.0, 150.0))),
      Comparator::not_equals(),
   ));
   // Root
   let instruction_node_id = mutator.arena.new_node(GfxMutator::Instruction {
      instruction: RepeatWhile(predicate),
      channel: 0,
      mutation: Mutation::None,
   });
   let void_node = mutator.arena.new_node(GfxMutator::new_void(0));
   // now make the node to be applied
   let applicable_node = mutator.arena.new_node(GfxMutator::Single {
      mutation: Mutation::AreaCommand(Box::new(GlyphAreaCommand::NudgeDown(10.0))),
      channel: 0,
   });

   mutator.root.append(instruction_node_id, &mut mutator.arena);
   instruction_node_id.append(void_node, &mut mutator.arena);
   void_node.append(applicable_node, &mut mutator.arena);

   walk_tree(&mut model, &mutator);

   assert_eq!(
      model
         .arena
         .get(right_lower_leg_id)
         .unwrap()
         .get()
         .position()
         .y,
      160.0
   );
   assert_eq!(
      model
         .arena
         .get(right_lower_leg_id)
         .unwrap()
         .get()
         .position()
         .x,
      105.0
   );

   assert_eq!(
      model
         .arena
         .get(left_lower_leg_id)
         .unwrap()
         .get()
         .position()
         .y,
      150.0
   );
   assert_eq!(
      model
         .arena
         .get(left_lower_leg_id)
         .unwrap()
         .get()
         .position()
         .x,
      95.0
   );

   assert_eq!(
      model.arena.get(left_knee_id).unwrap().get().position().y,
      140.0
   );
   assert_eq!(
      model.arena.get(left_knee_id).unwrap().get().position().x,
      95.0
   );

   assert_eq!(
      model.arena.get(left_thigh_id).unwrap().get().position().y,
      130.0
   );
   assert_eq!(
      model.arena.get(left_thigh_id).unwrap().get().position().x,
      95.0
   );

   assert_eq!(
      model.arena.get(right_knee_id).unwrap().get().position().y,
      140.0
   );
   assert_eq!(
      model.arena.get(right_knee_id).unwrap().get().position().x,
      105.0
   );

   assert_eq!(
      model.arena.get(right_thigh_id).unwrap().get().position().y,
      130.0
   );
   assert_eq!(
      model.arena.get(right_thigh_id).unwrap().get().position().x,
      105.0
   );

   assert_eq!(
      model
         .arena
         .get(left_lower_arm_id)
         .unwrap()
         .get()
         .position()
         .y,
      140.0
   );
   assert_eq!(
      model
         .arena
         .get(left_lower_arm_id)
         .unwrap()
         .get()
         .position()
         .x,
      90.0
   );

   assert_eq!(
      model
         .arena
         .get(left_upper_arm_id)
         .unwrap()
         .get()
         .position()
         .y,
      130.0
   );
   assert_eq!(
      model
         .arena
         .get(left_upper_arm_id)
         .unwrap()
         .get()
         .position()
         .x,
      90.0
   );

   assert_eq!(
      model
         .arena
         .get(left_shoulder_id)
         .unwrap()
         .get()
         .position()
         .y,
      120.0
   );
   assert_eq!(
      model
         .arena
         .get(left_shoulder_id)
         .unwrap()
         .get()
         .position()
         .x,
      90.0
   );

   assert_eq!(
      model
         .arena
         .get(right_lower_arm_id)
         .unwrap()
         .get()
         .position()
         .y,
      140.0
   );
   assert_eq!(
      model
         .arena
         .get(right_lower_arm_id)
         .unwrap()
         .get()
         .position()
         .x,
      110.0
   );

   assert_eq!(
      model
         .arena
         .get(right_upper_arm_id)
         .unwrap()
         .get()
         .position()
         .y,
      130.0
   );
   assert_eq!(
      model
         .arena
         .get(right_upper_arm_id)
         .unwrap()
         .get()
         .position()
         .x,
      110.0
   );

   assert_eq!(
      model
         .arena
         .get(right_shoulder_id)
         .unwrap()
         .get()
         .position()
         .y,
      120.0
   );
   assert_eq!(
      model
         .arena
         .get(right_shoulder_id)
         .unwrap()
         .get()
         .position()
         .x,
      110.0
   );

   assert_eq!(model.arena.get(torso_id).unwrap().get().position().y, 120.0);
   assert_eq!(model.arena.get(torso_id).unwrap().get().position().x, 100.0);

   assert_eq!(model.arena.get(neck_id).unwrap().get().position().y, 110.0);
   assert_eq!(model.arena.get(neck_id).unwrap().get().position().x, 100.0);

   assert_eq!(model.arena.get(head_id).unwrap().get().position().y, 100.0);
   assert_eq!(model.arena.get(head_id).unwrap().get().position().x, 100.0);
}
