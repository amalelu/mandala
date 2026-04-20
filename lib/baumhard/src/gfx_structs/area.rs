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

/// A text-region element. One `GlyphArea` corresponds to one
/// `glyphon::TextArea` in the renderer; every field here maps
/// ~1:1 onto the shaped buffer the renderer re-derives after any
/// modification. The translation is deliberately cheap — mutation
/// drives renderer rebuilds, so mutations stay light.
#[derive(Derivative, Serialize, Deserialize, Clone, Debug)]
#[derivative(Eq, PartialEq)]
pub struct GlyphArea {
    /// UTF-8 text laid out in this region.
    pub text: String,
    /// Font size in points. Drives cosmic-text `Metrics::font_size`.
    pub scale: OrderedFloat<f32>,
    /// Line-height multiplier applied to `scale` for vertical spacing.
    pub line_height: OrderedFloat<f32>,
    /// World-space anchor (top-left) of the area.
    pub position: OrderedVec2,
    /// Width / height the renderer is free to shape into. Zero disables
    /// rendering entirely.
    pub render_bounds: OrderedVec2,
    /// Per-character colour / font runs layered over the base text.
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
    /// Click-sensitive extents. Ignored for `PartialEq` because
    /// hit-boxes are scene-builder output, not persistent identity.
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
    /// Construct an empty-text area with the given metrics and
    /// placement. Regions and hitbox start empty; `align_center`,
    /// `background_color`, and `outline` default off. O(1); one
    /// heap allocation for the empty `text` String.
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
    /// Construct an area pre-populated with `text`. Mirrors `new` but
    /// skips the empty-string detour. O(n) in `text.len()` for the
    /// owning copy.
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

    /// Borrow the hit-test rectangle bag. O(1).
    pub fn hitbox(&self) -> &HitBox {
        &self.hitbox
    }

    /// Mutable borrow of the hit-test rectangle bag — scene builders
    /// use it to rewrite the click areas on layout. O(1).
    pub fn hitbox_as_mut(&mut self) -> &mut HitBox {
        &mut self.hitbox
    }

    /// Apply one [`DeltaGlyphArea`] to this area. The delta's
    /// `ApplyOperation` governs whether each field is assigned,
    /// added, or subtracted. Costs: O(k) in the number of fields
    /// the delta touches; region set operations are O(n) in the
    /// existing region count.
    pub fn apply_operation(&mut self, delta: &DeltaGlyphArea) {
        let operation = delta.operation_variant();

        if let Some(position) = delta.position() {
            let position = OrderedVec2::from_vec2(position);
            operation.apply(&mut self.position.x, position.x);
            operation.apply(&mut self.position.y, position.y);
        }

        if let Some(bounds) = delta.bounds() {
            let bounds = OrderedVec2::from_vec2(bounds);
            operation.apply(&mut self.render_bounds.x, bounds.x);
            operation.apply(&mut self.render_bounds.y, bounds.y);
        }

        if let Some(line_height) = delta.line_height() {
            operation.apply(&mut self.line_height, OrderedFloat::from(line_height));
        }

        if let Some(scale) = delta.scale() {
            operation.apply(&mut self.scale, OrderedFloat::from(scale));
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

        if let Some(text) = delta.text_ref() {
            match operation {
                ApplyOperation::Assign => self.text = text.to_string(),
                ApplyOperation::Add => self.text += text,
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

    /// Remove `pop_count` grapheme clusters from the front of the
    /// text. O(n) in the text length for the shift.
    pub fn pop_front(&mut self, pop_count: usize) {
        grapheme_chad::delete_front_unicode(&mut self.text, pop_count);
    }

    /// Remove `pop_count` grapheme clusters from the back of the
    /// text. O(n) grapheme walk to find the cut point.
    pub fn pop_back(&mut self, pop_count: usize) {
        grapheme_chad::delete_back_unicode(&mut self.text, pop_count)
    }

    /// Translate position by `(x, y)`. O(1).
    pub fn move_position(&mut self, x: f32, y: f32) {
        self.position.x += x;
        self.position.y += y;
    }

    /// Nudge position right by `nudge` pixels. O(1).
    pub fn nudge_right(&mut self, nudge: f32) {
        self.position.x += nudge;
    }

    /// Nudge position left by `nudge` pixels. O(1).
    pub fn nudge_left(&mut self, nudge: f32) {
        self.position.x -= nudge;
    }

    /// Nudge position up (y decreases in screen space) by `nudge`
    /// pixels. O(1).
    pub fn nudge_up(&mut self, nudge: f32) {
        self.position.y -= nudge;
    }

    /// Nudge position down (y increases in screen space) by `nudge`
    /// pixels. O(1).
    pub fn nudge_down(&mut self, nudge: f32) {
        self.position.y += nudge;
    }

    /// Add `value` to the font scale. O(1).
    pub fn grow_font(&mut self, value: &f32) {
        self.scale += value;
    }

    /// Subtract `value` from the font scale. O(1).
    pub fn shrink_font(&mut self, value: &f32) {
        self.scale -= value;
    }

    /// Replace the render bounds with `(width, height)`. O(1).
    pub fn set_bounds(&mut self, bounds: (f32, f32)) {
        self.render_bounds = OrderedVec2::new_f32(bounds.0, bounds.1);
    }

    /// Remove the colour/font region at `range`, if any. O(n) in
    /// the existing region count.
    pub fn delete_color_font_region(&mut self, range: &Range) {
        self.regions.remove_range(*range);
    }

    /// Move an existing region's span from `current_range` to
    /// `new_range`. O(n) in region count.
    ///
    /// # Panics
    /// Panics if no region exists at `current_range`.
    pub fn change_region_range(&mut self, current_range: &Range, new_range: &Range) {
        let mut current = *self.regions.get(*current_range).expect("No region found");
        current.range = *new_range;
        self.regions.remove_range(*current_range);
        self.regions.submit_region(current);
    }

    /// Assign `font` to the character `range`, creating or updating
    /// the matching region. O(n) in region count.
    pub fn set_region_font(&mut self, range: &Range, font: &AppFont) {
        self.regions
            .set_or_insert(&ColorFontRegion::new(*range, Some(*font), None));
    }

    /// Assign `color` to the character `range`, creating or updating
    /// the matching region. O(n) in region count.
    pub fn set_region_color(&mut self, range: &Range, color: &FloatRgba) {
        self.regions
            .set_or_insert(&ColorFontRegion::new(*range, None, Some(*color)));
    }

    /// Replace the font scale with `size`. O(1).
    pub fn set_font_size(&mut self, size: &f32) {
        self.scale = OrderedFloat::from(*size);
    }

    /// Replace the line-height multiplier with `line_height`. O(1).
    pub fn set_line_height(&mut self, line_height: &f32) {
        self.line_height = OrderedFloat::from(*line_height);
    }

    /// Add `line_height` to the current line-height multiplier. O(1).
    pub fn grow_line_height(&mut self, line_height: &f32) {
        self.line_height += line_height;
    }

    /// Subtract `line_height` from the current line-height
    /// multiplier. O(1).
    pub fn shrink_line_height(&mut self, line_height: &f32) {
        self.line_height -= line_height;
    }

    /// Position as a plain `Vec2`. O(1).
    pub fn position(&self) -> Vec2 {
        self.position.to_vec2()
    }

    /// Replace the position with `to_set`. O(1).
    pub fn set_position(&mut self, to_set: (f32, f32)) {
        self.position = OrderedVec2::new_f32(to_set.0, to_set.1);
    }

    /// Rotate this area's position around `pivot` by `angle` radians.
    /// Note this uses `Vec2::from_angle(angle).rotate(...)` so the
    /// angle parameter is radians, not degrees. O(1).
    pub fn rotate(&mut self, pivot: Vec2, angle: f32) {
        self.position =
            OrderedVec2::from_vec2(Vec2::from_angle(angle).rotate(self.position.to_vec2() - pivot));
    }
}
