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

/// Renderable glyph content for one tree node: a stacked matrix of
/// lines plus its placement metadata. Scene builders emit one per
/// model-bearing node.
#[derive(Derivative, Serialize, Deserialize, Debug, Clone)]
#[derivative(PartialEq, Eq)]
pub struct GlyphModel {
    /// The stacked glyph lines rendered by this model.
    pub glyph_matrix: GlyphMatrix,
    /// Draw order relative to sibling models. Zero is furthest from
    /// the camera; higher layers paint on top. Collisions at the
    /// same layer are undefined but non-fatal — the renderer must
    /// survive them without crashing.
    pub layer: usize,
    /// Origin anchor (top-left) in the parent container's coordinate
    /// space. `+x` right, `+y` down.
    pub position: OrderedVec2,
    /// Click-sensitive rectangles populated by the scene builder.
    /// Ignored for `PartialEq` — it's derived data, not identity.
    #[derivative(PartialEq = "ignore")]
    pub hitbox: HitBox,
}

impl GlyphModel {
    /// Empty model at origin with layer 0. O(1); no allocation
    /// beyond the default `GlyphMatrix`.
    pub fn new() -> Self {
        GlyphModel {
            glyph_matrix: GlyphMatrix::default(),
            layer: 0,
            position: OrderedVec2::new_f32(0.0, 0.0),
            hitbox: HitBox::new(),
        }
    }

    /// Borrow the hit-box. O(1).
    pub fn hitbox(&self) -> &HitBox {
        &self.hitbox
    }

    /// Mutable borrow of the hit-box — scene builders rewrite it on
    /// layout. O(1).
    pub fn hitbox_as_mut(&mut self) -> &mut HitBox {
        &mut self.hitbox
    }

    /// Append one [`GlyphLine`] to the matrix. O(1) amortised.
    pub fn add_line(&mut self, line: GlyphLine) {
        self.glyph_matrix.push(line);
    }

    /// Shift position left by `amount` pixels. O(1).
    pub fn nudge_left(&mut self, amount: &f32) {
        self.position.x -= amount;
    }

    /// Shift position right by `amount` pixels. O(1).
    pub fn nudge_right(&mut self, amount: &f32) {
        self.position.x += amount;
    }

    /// Shift position up (y decreases) by `amount` pixels. O(1).
    pub fn nudge_up(&mut self, amount: &f32) {
        self.position.y -= amount;
    }

    /// Shift position down (y increases) by `amount` pixels. O(1).
    pub fn nudge_down(&mut self, amount: &f32) {
        self.position.y += amount;
    }

    /// Teleport position to absolute `(x, y)`. O(1).
    pub fn move_to(&mut self, x: &f32, y: &f32) {
        self.position.x = OrderedFloat::from(*x);
        self.position.y = OrderedFloat::from(*y);
    }

    /// Rotate position clockwise around `pivot` by `degrees`. O(1).
    pub fn rotate(&mut self, pivot: &Vec2, degrees: &f32) {
        let new_position =
            geometry::clockwise_rotation_around_pivot(self.position.to_vec2(), *pivot, *degrees);
        self.position = OrderedVec2::from_vec2(new_position);
    }

    /// Insert `component` at `(line_num, at_idx)` replacing any
    /// overlapping content. Lines and columns are grown with
    /// whitespace padding as needed. O(line length) for the
    /// grapheme walk plus the splice.
    pub fn rude_insert(&mut self, component: &GlyphComponent, line_num: &usize, at_idx: &usize) {
        self.glyph_matrix
            .overriding_insert(*line_num, *at_idx, component);
    }

    /// Insert `component` at `(line_num, at_idx)` shifting existing
    /// graphemes to the right. O(line length) for the grapheme walk
    /// plus the shift.
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
