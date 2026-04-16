pub use super::area_fields::*;
pub use super::area_mutators::*;

use crate::core::primitives::{
    ApplyOperation, ColorFontRegion, ColorFontRegions, Range,
};
use crate::font::fonts::AppFont;
use crate::util::color::FloatRgba;
use crate::util::grapheme_chad;
use crate::util::ordered_vec2::OrderedVec2;
use derivative::Derivative;
use glam::f32::Vec2;
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use crate::gfx_structs::util::hitbox::HitBox;

/// One GlyphArea will be translated to one TextArea, and its properties mirror that of the TextArea
/// The translation from GlyphArea into TextArea will happen each time there is a modification
/// So that the Renderer has to update its buffers and caches. This translation is very fast, since
/// the fields are basically 1:1.
#[derive(Derivative, Serialize, Deserialize, Clone, Debug)]
#[derivative(Eq, PartialEq)]
pub struct GlyphArea {
    pub text: String,
    pub scale: OrderedFloat<f32>,
    pub line_height: OrderedFloat<f32>,
    pub position: OrderedVec2,
    pub render_bounds: OrderedVec2,
    pub regions: ColorFontRegions,
    /// Solid background fill drawn behind the text glyphs by the
    /// renderer. `None` means no fill — the element draws as text
    /// only, letting the canvas show through. Stored as 4×u8 RGBA
    /// so it's cheap to hash, copy, and ship to the GPU. Mutations
    /// can modify this directly through the tree walker; per-frame
    /// rendering reads it during `rebuild_buffers_from_tree`.
    #[serde(default)]
    pub background_color: Option<[u8; 4]>,
    /// When `true`, the renderer shapes this area's text with
    /// `cosmic_text::Align::Center` so cross-script glyphs whose
    /// per-glyph advance varies (e.g. the picker's Devanagari /
    /// Hebrew / Tibetan hue-ring cells) sit centred in their
    /// box. Default `false` — text starts at the box's left edge,
    /// matching ordinary mindmap node text.
    #[serde(default)]
    pub align_center: bool,
    /// Optional black-or-colored halo drawn behind the area's
    /// glyphs. When `Some`, the renderer's tree walker emits N
    /// extra shaped buffers at offset positions before the main
    /// one — see [`OutlineStyle`] for the cost trade-off. `None`
    /// (the default) skips the halo entirely; ordinary mindmap
    /// nodes that render against an opaque-enough background
    /// don't need one.
    #[serde(default)]
    pub outline: Option<OutlineStyle>,
    #[derivative(PartialEq = "ignore")]
    pub hitbox: HitBox,
}

impl Hash for GlyphArea {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);
        self.scale.to_bits().hash(state);
        self.line_height.to_bits().hash(state);
        self.position.x().to_bits().hash(state);
        self.position.y().to_bits().hash(state);
        self.render_bounds.x().to_bits().hash(state);
        self.render_bounds.y().to_bits().hash(state);
        self.regions.hash(state);
        self.background_color.hash(state);
        self.align_center.hash(state);
        self.outline.hash(state);
    }
}

impl GlyphArea {
    pub fn new(scale: f32, line_height: f32, position: Vec2, bounds: Vec2) -> Self {
        GlyphArea {
            text: "".to_string(),
            scale: OrderedFloat::from(scale),
            line_height: OrderedFloat::from(line_height),
            position: OrderedVec2::from_vec2(position),
            render_bounds: OrderedVec2::from_vec2(bounds),
            regions: ColorFontRegions::default(),
            background_color: None,
            align_center: false,
            outline: None,
            hitbox: HitBox::new(),
        }
    }
    pub fn new_with_str(
        text: &str,
        scale: f32,
        line_height: f32,
        position: Vec2,
        bounds: Vec2,
    ) -> Self {
        GlyphArea {
            text: text.to_string(),
            scale: OrderedFloat::from(scale),
            line_height: OrderedFloat::from(line_height),
            position: OrderedVec2::from_vec2(position),
            render_bounds: OrderedVec2::from_vec2(bounds),
            regions: ColorFontRegions::default(),
            background_color: None,
            align_center: false,
            outline: None,
            hitbox: HitBox::new(),
        }
    }

    pub fn hitbox(&self) -> &HitBox {
        &self.hitbox
    }

    pub fn hitbox_as_mut(&mut self) -> &mut HitBox {
        &mut self.hitbox
    }

    pub fn apply_operation(&mut self, delta: &DeltaGlyphArea) {
        let operation = delta.operation_variant();

        if delta.position().is_some() {
            let position = OrderedVec2::from_vec2(delta.position().unwrap());
            operation.apply(&mut self.position.x, position.x);
            operation.apply(&mut self.position.y, position.y);
        }

        if delta.bounds().is_some() {
            let bounds = OrderedVec2::from_vec2(delta.bounds().unwrap());
            operation.apply(&mut self.render_bounds.x, bounds.x);
            operation.apply(&mut self.render_bounds.y, bounds.y);
        }

        if delta.line_height().is_some() {
            operation.apply(
                &mut self.line_height,
                OrderedFloat::from(delta.line_height().unwrap()),
            );
        }

        if delta.scale().is_some() {
            operation.apply(&mut self.scale, OrderedFloat::from(delta.scale().unwrap()));
        }

        if let Some(x) = delta.color_font_regions() {
            match operation {
                // For add, we add the regions in the delta to the regions in the self
                ApplyOperation::Add => {
                    for delta_region in &x.regions {
                        self.regions.submit_region(*delta_region);
                    }
                }
                // For assign, we remove the self regions, and insert the delta's
                ApplyOperation::Assign => self.regions.replace_regions(x),
                ApplyOperation::Subtract => {
                    for delta_region in &x.regions {
                        self.regions.remove(delta_region);
                    }
                }
                _ => {}
            }
        }

        if delta.text_ref().is_some() {
            match operation {
                ApplyOperation::Assign => {
                    self.text = delta.text_ref().unwrap().to_string();
                }
                ApplyOperation::Add => self.text += delta.text_ref().unwrap(),
                _ => {}
            }
        }

        if let Some(outline) = delta.outline() {
            match operation {
                // For Add / Assign we just take the delta's value —
                // halo state is on/off; "merging" two halos isn't
                // meaningful (see `Add` impl on `GlyphAreaField`).
                ApplyOperation::Assign | ApplyOperation::Add => {
                    self.outline = outline;
                }
                // Subtract clears the halo entirely. The delta's
                // payload doesn't matter for this branch — the
                // semantic is "remove what's there".
                ApplyOperation::Subtract => {
                    self.outline = None;
                }
                _ => {}
            }
        }
    }

    pub fn pop_front(&mut self, pop_count: usize) {
        grapheme_chad::delete_front_unicode(&mut self.text, pop_count);
    }

    pub fn pop_back(&mut self, pop_count: usize) {
        grapheme_chad::delete_back_unicode(&mut self.text, pop_count)
    }

    pub fn move_position(&mut self, x: f32, y: f32) {
        self.position.x += x;
        self.position.y += y;
    }

    pub fn nudge_right(&mut self, nudge: f32) {
        self.position.x += nudge;
    }

    pub fn nudge_left(&mut self, nudge: f32) {
        self.position.x -= nudge;
    }

    pub fn nudge_up(&mut self, nudge: f32) {
        self.position.y -= nudge;
    }

    pub fn nudge_down(&mut self, nudge: f32) {
        self.position.y += nudge;
    }

    pub fn grow_font(&mut self, value: &f32) {
        self.scale += value;
    }

    pub fn shrink_font(&mut self, value: &f32) {
        self.scale -= value;
    }

    pub fn set_bounds(&mut self, bounds: (f32, f32)) {
        self.render_bounds = OrderedVec2::new_f32(bounds.0, bounds.1);
    }

    pub fn delete_color_font_region(&mut self, range: &Range) {
        self.regions.remove_range(*range);
    }

    pub fn change_region_range(&mut self, current_range: &Range, new_range: &Range) {
        let mut current = *self.regions.get(*current_range).expect("No region found");
        current.range = *new_range;
        self.regions.remove_range(*current_range);
        self.regions.submit_region(current);
    }

    pub fn set_region_font(&mut self, range: &Range, font: &AppFont) {
        self.regions
            .set_or_insert(&ColorFontRegion::new(*range, Some(*font), None));
    }

    pub fn set_region_color(&mut self, range: &Range, color: &FloatRgba) {
        self.regions
            .set_or_insert(&ColorFontRegion::new(*range, None, Some(*color)));
    }

    pub fn set_font_size(&mut self, size: &f32) {
        self.scale = OrderedFloat::from(*size);
    }

    pub fn set_line_height(&mut self, line_height: &f32) {
        self.line_height = OrderedFloat::from(*line_height);
    }

    pub fn grow_line_height(&mut self, line_height: &f32) {
        self.line_height += line_height;
    }

    pub fn shrink_line_height(&mut self, line_height: &f32) {
        self.line_height -= line_height;
    }

    pub fn position(&self) -> Vec2 {
        self.position.to_vec2()
    }

    pub fn set_position(&mut self, to_set: (f32, f32)) {
        self.position = OrderedVec2::new_f32(to_set.0, to_set.1);
    }

    pub fn rotate(&mut self, pivot: Vec2, angle: f32) {
        self.position =
            OrderedVec2::from_vec2(Vec2::from_angle(angle).rotate(self.position.to_vec2() - pivot));
    }
}
