//! `GlyphModel` — the renderable glyph matrix + its positional
//! metadata (layer, origin, hit box). Scene builders emit one per
//! tree-node.

use super::component::GlyphComponent;
use super::line::GlyphLine;
use super::matrix::GlyphMatrix;
use super::mutator::DeltaGlyphModel;
use crate::gfx_structs::util::hitbox::HitBox;
use crate::util::geometry;
use crate::util::ordered_vec2::OrderedVec2;
use derivative::Derivative;
use glam::Vec2;
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};

#[derive(Derivative, Serialize, Deserialize, Debug, Clone)]
#[derivative(PartialEq, Eq)]
pub struct GlyphModel {
    pub glyph_matrix: GlyphMatrix,
    /// ## FYI
    /// Starting from 0 as the lowest level, the layer places the [GlyphModel] in relation to other objects
    /// The higher the layer value, the closer to the camera it should be considered
    /// So the objects with the highest layer value will always be painted on top of any other objects
    /// If two objects have the same layer, then it is undefined what should happen if they collide
    /// But the program should not crash, because this is something that will happen quite often
    /// although collision logic should then take place and separate them
    pub layer: usize,
    /// Origin is (0,0) from the top left corner in its parent container,
    /// increasing x goes to the right while increasing y goes downwards
    pub position: OrderedVec2,
    #[derivative(PartialEq = "ignore")]
    pub hitbox: HitBox,
}

impl GlyphModel {
    /// Creates a new [GlyphModel] with an empty Vec and layer set to 0, at (0,0)
    pub fn new() -> Self {
        GlyphModel {
            glyph_matrix: GlyphMatrix::default(),
            layer: 0,
            position: OrderedVec2::new_f32(0.0, 0.0),
            hitbox: HitBox::new(),
        }
    }

    pub fn hitbox(&self) -> &HitBox {
        &self.hitbox
    }

    pub fn hitbox_as_mut(&mut self) -> &mut HitBox {
        &mut self.hitbox
    }

    pub fn add_line(&mut self, line: GlyphLine) {
        self.glyph_matrix.push(line);
    }

    pub fn nudge_left(&mut self, amount: &f32) {
        self.position.x -= amount;
    }

    pub fn nudge_right(&mut self, amount: &f32) {
        self.position.x += amount;
    }

    pub fn nudge_up(&mut self, amount: &f32) {
        self.position.y -= amount;
    }

    pub fn nudge_down(&mut self, amount: &f32) {
        self.position.y += amount;
    }

    pub fn move_to(&mut self, x: &f32, y: &f32) {
        self.position.x = OrderedFloat::from(*x);
        self.position.y = OrderedFloat::from(*y);
    }

    pub fn rotate(&mut self, pivot: &Vec2, degrees: &f32) {
        let new_position =
            geometry::clockwise_rotation_around_pivot(self.position.to_vec2(), *pivot, *degrees);
        self.position = OrderedVec2::from_vec2(new_position);
    }

    pub fn rude_insert(&mut self, component: &GlyphComponent, line_num: &usize, at_idx: &usize) {
        self.glyph_matrix
            .overriding_insert(*line_num, *at_idx, component);
    }

    pub fn expanding_insert(
        &mut self,
        component: &GlyphComponent,
        line_num: &usize,
        at_idx: &usize,
    ) {
        self.glyph_matrix
            .expanding_insert(*line_num, *at_idx, component);
    }

    pub(super) fn apply_operation(&mut self, delta: &DeltaGlyphModel) {
        let operation = delta.operation_variant();
        if let Some(position_delta) = delta.position() {
            operation.apply(&mut self.position.x, position_delta.x);
            operation.apply(&mut self.position.y, position_delta.y);
        }

        if let Some(delta_layer) = delta.layer() {
            operation.apply(&mut self.layer, delta_layer);
        }

        if let Some(glyph_matrix) = delta.glyph_matrix() {
            operation.apply(&mut self.glyph_matrix, glyph_matrix);
        }
    }
}
