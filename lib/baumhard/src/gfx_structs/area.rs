// SPDX-License-Identifier: MPL-2.0

//! `GlyphArea` — the text-region element variant of a `GfxElement`.
//! Owns the text content, font scale, position, bounding size,
//! halo outline, and the `ColorFontRegions` set that carries per-
//! range color / font overrides. Mutations land here through
//! `apply_operation` (`DeltaGlyphArea`) or the higher-level
//! `GlyphAreaCommand`; the field and mutator vocabularies live in
//! the sibling `area_fields` and `area_mutators` modules and are
//! re-exported from this one so consumers import from a single
//! path.

pub use super::area_fields::*;
pub use super::area_mutators::*;

use crate::core::primitives::{ApplyOperation, ColorFontRegion, ColorFontRegions, Range};
use crate::font::fonts::AppFont;
use crate::gfx_structs::shape::NodeShape;
use crate::gfx_structs::util::hitbox::HitBox;
use crate::gfx_structs::zoom_visibility::ZoomVisibility;
use crate::util::color::FloatRgba;
use crate::util::geometry::clockwise_rotation_around_pivot;
use crate::util::grapheme_chad;
use crate::util::ordered_vec2::OrderedVec2;
use derivative::Derivative;
use glam::f32::Vec2;
use log::warn;

/// Asymmetric per-edge expansion in pixels. Used by
/// [`GlyphArea::background_padding`] to extend the fill rect
/// outward by an independent amount on each side. Default is
/// `0.0` everywhere — the fill coincides with the text rect, the
/// historical behavior. All fields stored as
/// [`ordered_float::OrderedFloat`] so the type is hashable and
/// `PartialEq`-able with float-bit equality, mirroring
/// [`OrderedVec2`].
///
/// Why per-edge rather than the previous symmetric `OrderedVec2`:
/// node borders are NOT symmetric — the top run sits
/// `font_size - corner_overlap` above the node rect, the bottom
/// run extends `1.5 * font_size - corner_overlap` below it. A
/// symmetric pad either over-extends top or under-extends bottom.
/// Carrying four independent values lets the producer
/// (`tree_builder::node::mindnode_to_glyph_area`) state the actual
/// outward extent on each side.
#[derive(Clone, Copy, Hash, Eq, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub struct EdgePadding {
    pub top: ordered_float::OrderedFloat<f32>,
    pub right: ordered_float::OrderedFloat<f32>,
    pub bottom: ordered_float::OrderedFloat<f32>,
    pub left: ordered_float::OrderedFloat<f32>,
}

impl EdgePadding {
    /// All-zero padding. Equivalent to `Default::default()`; named
    /// for use in `const` contexts and to read clearly at call
    /// sites that want "no inflation".
    pub const ZERO: EdgePadding = EdgePadding {
        top: ordered_float::OrderedFloat(0.0),
        right: ordered_float::OrderedFloat(0.0),
        bottom: ordered_float::OrderedFloat(0.0),
        left: ordered_float::OrderedFloat(0.0),
    };

    /// Construct from raw `f32`s.
    pub fn new(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        EdgePadding {
            top: ordered_float::OrderedFloat(top),
            right: ordered_float::OrderedFloat(right),
            bottom: ordered_float::OrderedFloat(bottom),
            left: ordered_float::OrderedFloat(left),
        }
    }

    /// `true` when every edge is exactly zero. Used by the
    /// renderer's per-frame fast-path to skip the pad arithmetic
    /// for unframed nodes (the common case).
    pub fn is_zero(&self) -> bool {
        self.top.0 == 0.0 && self.right.0 == 0.0 && self.bottom.0 == 0.0 && self.left.0 == 0.0
    }

    pub fn top(&self) -> f32 {
        self.top.0
    }
    pub fn right(&self) -> f32 {
        self.right.0
    }
    pub fn bottom(&self) -> f32 {
        self.bottom.0
    }
    pub fn left(&self) -> f32 {
        self.left.0
    }
}

impl Default for EdgePadding {
    fn default() -> Self {
        EdgePadding::ZERO
    }
}
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

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
    /// Per-character color / font runs layered over the base text.
    pub regions: ColorFontRegions,
    /// Solid background fill drawn behind the text glyphs by the
    /// renderer. `None` means no fill — the element draws as text
    /// only, letting the canvas show through. Stored as 4×u8 RGBA
    /// so it's cheap to hash, copy, and ship to the GPU. Mutations
    /// can modify this directly through the tree walker; per-frame
    /// rendering reads it during `rebuild_buffers_from_tree`.
    #[serde(default)]
    pub background_color: Option<[u8; 4]>,
    /// Asymmetric outward expansion of the background fill rect
    /// beyond `(position, render_bounds)`. The renderer draws the
    /// fill at `(position.x - left, position.y - top)` with size
    /// `(render_bounds.x + left + right, render_bounds.y + top + bottom)`;
    /// text shaping uses the unmodified `position` / `render_bounds`,
    /// so this field doesn't affect layout. Default `EdgePadding::ZERO`
    /// — the background coincides with the text bounds, the historical
    /// behavior. Used by mindmap nodes to extend the fill behind
    /// border glyphs that sit outside the text rect so the border
    /// draws against the node's background color rather than the
    /// canvas underneath. Per-edge rather than symmetric because
    /// node borders are inherently asymmetric — the top run sits
    /// `font_size - corner_overlap` above the rect and the bottom
    /// run extends `1.5 * font_size - corner_overlap` below it.
    #[serde(default)]
    pub background_padding: EdgePadding,
    /// When `true`, the renderer shapes this area's text with
    /// `cosmic_text::Align::Center` so cross-script glyphs whose
    /// per-glyph advance varies (e.g. the picker's Devanagari /
    /// Hebrew / Tibetan hue-ring cells) sit centered in their
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
    /// Background / hit-test shape of the area. Default
    /// [`NodeShape::Rectangle`] matches the historical behavior
    /// where every node fills its bounding box. Shared between the
    /// renderer's rect SDF pipeline (drawn fill) and the BVH
    /// descent (point-in-shape hit test), so changing this field
    /// automatically moves both visuals and input together.
    ///
    /// Round-trip fidelity for unknown-spelling shapes is owned by
    /// the format layer (`NodeStyle.shape: String` in
    /// `crate::mindmap::model::node`), *not* by this field: the
    /// string-to-enum conversion in
    /// [`NodeShape::from_style_string`](crate::gfx_structs::shape::NodeShape::from_style_string)
    /// collapses unknowns to `Rectangle`, but the original string
    /// is preserved on the `MindNode` and rewritten on save.
    #[serde(default)]
    pub shape: NodeShape,
    /// Optional `[min, max]` window on `camera.zoom` controlling
    /// whether this area is drawn. Default
    /// [`ZoomVisibility::unbounded`] renders at every zoom — the
    /// historical posture. When a bound is set, the renderer's
    /// final cull skips this area whenever the current zoom falls
    /// outside the window (see
    /// [`ZoomVisibility::contains`]). Orthogonal to `scale` and
    /// any per-builder font-size clamps: this gates *presence*,
    /// not size.
    #[serde(default, skip_serializing_if = "ZoomVisibility::is_default")]
    pub zoom_visibility: ZoomVisibility,
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
        self.background_padding.hash(state);
        self.align_center.hash(state);
        self.outline.hash(state);
        self.shape.hash(state);
        self.zoom_visibility.hash(state);
    }
}

impl GlyphArea {
    /// Hash everything a renderer would draw this area from —
    /// including the region colors and font pins that [`Hash`]
    /// above cannot reach.
    ///
    /// `Hash` has to agree with this type's `PartialEq`, and that
    /// equality runs through [`ColorFontRegions`], whose element
    /// identity is the [`Range`] alone. So a recolored span leaves
    /// both `==` and `hash` unmoved — which is exactly what a
    /// color-picker hover produces. Anything folding "would a rebuild of this area
    /// draw differently?" into one integer must call this instead;
    /// the canvas-role dispatchers in `scene_rebuild` do, to decide
    /// whether the registered tree is already correct.
    ///
    /// Floats go in as `to_bits`, so `-0.0` and `0.0` are distinct
    /// values here while `==` calls them equal. That is the safe
    /// direction for a reuse decision: it can only cost a redundant
    /// update, never a skipped one.
    ///
    /// **`hitbox` is deliberately excluded**, matching `PartialEq`
    /// (which marks it `ignore`) and `Hash` (which omits it): click
    /// extents are scene-builder output rather than drawn state. A
    /// consumer that skips work on a hash match therefore skips
    /// re-stamping hit boxes too, so it owes its own proof that its
    /// builder writes none — see
    /// `test_border_and_portal_areas_carry_no_hitbox`.
    ///
    /// Cost: O(text bytes + region count); no allocation. The body
    /// destructures `self`, so a field added to this struct will not
    /// compile until it is accounted for here.
    pub fn hash_content<H: Hasher>(&self, state: &mut H) {
        let GlyphArea {
            text,
            scale,
            line_height,
            position,
            render_bounds,
            regions,
            background_color,
            background_padding,
            align_center,
            outline,
            shape,
            zoom_visibility,
            hitbox: _,
        } = self;
        text.hash(state);
        scale.to_bits().hash(state);
        line_height.to_bits().hash(state);
        position.x().to_bits().hash(state);
        position.y().to_bits().hash(state);
        render_bounds.x().to_bits().hash(state);
        render_bounds.y().to_bits().hash(state);
        regions.hash_content(state);
        background_color.hash(state);
        background_padding.hash(state);
        align_center.hash(state);
        outline.hash(state);
        shape.hash(state);
        zoom_visibility.hash(state);
    }

    /// Construct an empty-text area with the given metrics and
    /// placement. Regions and hitbox start empty; `align_center`,
    /// `background_color`, `background_padding`, and `outline`
    /// default off; `shape` defaults to `Rectangle`;
    /// `zoom_visibility` to unbounded. O(1); one heap allocation
    /// for the empty `text` String.
    pub fn new(scale: f32, line_height: f32, position: Vec2, bounds: Vec2) -> Self {
        GlyphArea {
            text: "".to_string(),
            scale: OrderedFloat::from(scale),
            line_height: OrderedFloat::from(line_height),
            position: OrderedVec2::from_vec2(position),
            render_bounds: OrderedVec2::from_vec2(bounds),
            regions: ColorFontRegions::default(),
            background_color: None,
            background_padding: EdgePadding::ZERO,
            align_center: false,
            outline: None,
            shape: NodeShape::Rectangle,
            zoom_visibility: ZoomVisibility::unbounded(),
            hitbox: HitBox::new(),
        }
    }
    /// Construct an area pre-populated with `text`. Mirrors `new` but
    /// skips the empty-string detour. O(n) in `text.len()` for the
    /// owning copy.
    pub fn new_with_str(text: &str, scale: f32, line_height: f32, position: Vec2, bounds: Vec2) -> Self {
        GlyphArea {
            text: text.to_string(),
            scale: OrderedFloat::from(scale),
            line_height: OrderedFloat::from(line_height),
            position: OrderedVec2::from_vec2(position),
            render_bounds: OrderedVec2::from_vec2(bounds),
            regions: ColorFontRegions::default(),
            background_color: None,
            background_padding: EdgePadding::ZERO,
            align_center: false,
            outline: None,
            shape: NodeShape::Rectangle,
            zoom_visibility: ZoomVisibility::unbounded(),
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

        // The delta path writes the same two shaper metrics the
        // setters do, so it takes the same clamp. Applying into a
        // local first keeps the operation semantics (`Add` still
        // adds, `Multiply` still multiplies) while the *result* is
        // what gets bounded — otherwise a `GlyphAreaField::Scale`
        // delta, which a map-carried CustomMutation can carry, walks
        // straight past the invariant the setters exist to hold.
        if let Some(line_height) = delta.line_height() {
            let mut next = self.line_height;
            operation.apply(&mut next, OrderedFloat::from(line_height));
            self.set_line_height_clamped(next.into_inner());
        }

        if let Some(scale) = delta.scale() {
            let mut next = self.scale;
            operation.apply(&mut next, OrderedFloat::from(scale));
            self.set_scale_clamped(next.into_inner());
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
                ApplyOperation::Delete => self.regions = ColorFontRegions::default(),
                ApplyOperation::Multiply => {
                    warn!("gfx_structs::area: Multiply is not defined for ColorFontRegions; ignoring")
                }
                ApplyOperation::Noop => {}
            }
        }

        if let Some(text) = delta.text_ref() {
            match operation {
                ApplyOperation::Assign => {
                    self.text.clear();
                    self.text.push_str(text);
                }
                ApplyOperation::Add => self.text += text,
                ApplyOperation::Delete => self.text.clear(),
                ApplyOperation::Subtract | ApplyOperation::Multiply => {
                    warn!(
                        "gfx_structs::area: {:?} is not defined for text; ignoring",
                        operation
                    )
                }
                ApplyOperation::Noop => {}
            }
        }

        // Halo state is on/off; "merging" two halos isn't
        // meaningful (see `Add` impl on `GlyphAreaField`). Both
        // Assign and Add overwrite; Subtract clears.
        apply_overwrite_or_reset(operation, delta.outline(), &mut self.outline, None);

        // Shapes don't compose (you can't "add" an ellipse to a
        // rectangle); Assign and Add both overwrite, Subtract
        // resets to the default rectangle.
        apply_overwrite_or_reset(operation, delta.shape(), &mut self.shape, NodeShape::Rectangle);

        // Zoom windows don't compose arithmetically (combining
        // "zoomed in only" with "zoomed out only" yields nothing
        // sensible); Assign and Add overwrite, Subtract restores
        // the unbounded default.
        apply_overwrite_or_reset(
            operation,
            delta.zoom_visibility(),
            &mut self.zoom_visibility,
            ZoomVisibility::unbounded(),
        );
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
        self.set_scale_clamped(self.scale.into_inner() + value);
    }

    /// Write `scale`, forced into the shaper's usable range.
    ///
    /// **Every mutator that touches the font scale lands here**, and
    /// that is the point. The scale becomes a `cosmic_text::Metrics`
    /// term, where zero trips an `assert!` that aborts the process
    /// and a huge value asks the rasterizer for a glyph the atlas
    /// cannot hold. The document loader screens the numbers it can
    /// see (`mindmap::model::validate`), but it only sees *authored
    /// geometry* — a `CustomMutation` payload carried by the same
    /// file writes here later, on a click, and so do console verbs,
    /// macros, and IPC. Clamping at the field rather than at each
    /// caller is what makes the invariant hold for writers that do
    /// not exist yet.
    ///
    /// Cost: O(1).
    fn set_scale_clamped(&mut self, value: f32) {
        self.scale = OrderedFloat::from(crate::font::fonts::clamp_font_metric(
            value,
            self.scale.into_inner(),
        ));
    }

    /// Write `line_height`, forced into the same range as
    /// [`Self::set_scale_clamped`] and for the same reason — it is
    /// the other half of `Metrics::new`, and the half the
    /// zero-line-height assert actually names.
    ///
    /// Cost: O(1).
    fn set_line_height_clamped(&mut self, value: f32) {
        self.line_height = OrderedFloat::from(crate::font::fonts::clamp_font_metric(
            value,
            self.line_height.into_inner(),
        ));
    }

    /// Subtract `value` from the font scale. O(1).
    pub fn shrink_font(&mut self, value: &f32) {
        self.set_scale_clamped(self.scale.into_inner() - value);
    }

    /// Replace the render bounds with `(width, height)`. O(1).
    pub fn set_bounds(&mut self, bounds: (f32, f32)) {
        self.render_bounds = OrderedVec2::new_f32(bounds.0, bounds.1);
    }

    /// Remove the color/font region at `range`, if any. O(n) in
    /// the existing region count.
    pub fn delete_color_font_region(&mut self, range: &Range) {
        self.regions.remove_range(*range);
    }

    /// Move an existing region's span from `current_range` to
    /// `new_range`. O(n) in region count.
    ///
    /// If no region exists at `current_range`, the call is a no-op
    /// after logging a warning — interactive mutation paths must not
    /// abort the editor over a stale range (CODE_CONVENTIONS.md §9).
    pub fn change_region_range(&mut self, current_range: &Range, new_range: &Range) {
        let Some(mut current) = self.regions.get(*current_range).copied() else {
            warn!(
                "change_region_range skipped: no region at {}..{}",
                current_range.start, current_range.end
            );
            return;
        };
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
        self.set_scale_clamped(*size);
    }

    /// Replace the line-height multiplier with `line_height`. O(1).
    pub fn set_line_height(&mut self, line_height: &f32) {
        self.set_line_height_clamped(*line_height);
    }

    /// Add `line_height` to the current line-height multiplier. O(1).
    pub fn grow_line_height(&mut self, line_height: &f32) {
        self.set_line_height_clamped(self.line_height.into_inner() + line_height);
    }

    /// Subtract `line_height` from the current line-height
    /// multiplier. O(1).
    pub fn shrink_line_height(&mut self, line_height: &f32) {
        self.set_line_height_clamped(self.line_height.into_inner() - line_height);
    }

    /// Position as a plain `Vec2`. O(1).
    pub fn position(&self) -> Vec2 {
        self.position.to_vec2()
    }

    /// Replace the position with `to_set`. O(1).
    pub fn set_position(&mut self, to_set: (f32, f32)) {
        self.position = OrderedVec2::new_f32(to_set.0, to_set.1);
    }

    /// Rotate this area's position clockwise around `pivot` by
    /// `degrees`.
    ///
    /// Shares [`clockwise_rotation_around_pivot`] with its two
    /// siblings — [`GfxElement::rotate`](crate::gfx_structs::element::GfxElement::rotate)
    /// and [`GlyphModel::rotate`](crate::gfx_structs::model::GlyphModel::rotate)
    /// — so all three agree on the unit (degrees, not radians) and the
    /// direction (clockwise in screen space, where `+y` points down).
    /// The rotation is about `pivot`, so rotating about the area's own
    /// position is the identity.
    ///
    /// Costs: O(1) — one `Mat3` construction and one point transform.
    /// No allocation.
    pub fn rotate(&mut self, pivot: Vec2, degrees: f32) {
        self.position = OrderedVec2::from_vec2(clockwise_rotation_around_pivot(
            self.position.to_vec2(),
            pivot,
            degrees,
        ));
    }
}

/// Apply the "overwrite-on-Assign/Add, reset-on-Subtract,
/// no-op otherwise" policy to a single field. The three
/// non-arithmetic per-leaf fields (`outline`, `shape`,
/// `zoom_visibility`) all share this contract — the values
/// don't compose under addition (you can't "add" two halos /
/// shapes / zoom windows), so the apply path treats `Add` and
/// `Assign` identically and uses `Subtract` as the canonical
/// "remove the override" signal.
///
/// `delta` is `None` when the corresponding [`DeltaGlyphArea`]
/// field wasn't set on construction; the helper short-circuits
/// in that case so the target field is untouched.
fn apply_overwrite_or_reset<T>(op: ApplyOperation, delta: Option<T>, target: &mut T, reset: T) {
    let Some(value) = delta else { return };
    match op {
        ApplyOperation::Assign | ApplyOperation::Add => *target = value,
        ApplyOperation::Subtract => *target = reset,
        _ => {}
    }
}
