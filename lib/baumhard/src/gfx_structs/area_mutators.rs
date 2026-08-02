// SPDX-License-Identifier: MPL-2.0

//! Command and delta mutators for `GlyphArea` — the two `Applicable`
//! implementations that the tree walker dispatches through.

use crate::core::primitives::{Applicable, ApplyOperation, ColorFontRegions, Range};
use crate::font::fonts::AppFont;
use crate::gfx_structs::delta::Delta;
use crate::gfx_structs::shape::NodeShape;
use crate::util::color::FloatRgba;
use glam::f32::Vec2;
use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumDiscriminants, EnumIter};

use super::area::GlyphArea;
use super::area_fields::{GlyphAreaField, GlyphAreaFieldType, OutlineStyle};
use super::zoom_visibility::ZoomVisibility;

////////////////////////////////////////
/////// GlyphAreaCommand Mutator ///////
///////////////////////////////////////

/// Imperative mutation command applied to a [`GlyphArea`] via its
/// [`Applicable`] impl. Unlike [`DeltaGlyphArea`] (which is
/// arithmetic — `Add`/`Assign`/`Subtract`), a command performs a
/// single named operation whose semantics are fixed. All variants
/// are O(1) except the `ColorFontRegion`-touching ones, which are
/// O(n) in the number of existing regions.
///
/// The payload-free `GlyphAreaCommandType` tag is derived from this
/// enum, so a new command cannot ship without its tag.
#[derive(Clone, Debug, Copy, Serialize, Deserialize, PartialEq, EnumDiscriminants)]
#[strum_discriminants(name(GlyphAreaCommandType))]
#[strum_discriminants(derive(Hash, Serialize, Deserialize, EnumIter, Display))]
#[strum_discriminants(doc = "Payload-free tag for [`GlyphAreaCommand`], derived by strum. Keys")]
#[strum_discriminants(doc = "the `HashSet`/`HashMap` look-ups where a caller needs to know")]
#[strum_discriminants(doc = "*which* command was scheduled but not its parameters.")]
#[serde(deny_unknown_fields)]
pub enum GlyphAreaCommand {
    /// Remove `n` grapheme clusters from the front of the text.
    PopFront(usize),
    /// Remove `n` grapheme clusters from the back of the text.
    PopBack(usize),
    /// Shift position left by the given pixel delta.
    NudgeLeft(f32),
    /// Shift position right by the given pixel delta.
    NudgeRight(f32),
    /// Shift position down by the given pixel delta.
    NudgeDown(f32),
    /// Shift position up by the given pixel delta.
    NudgeUp(f32),
    /// Teleport position to absolute `(x, y)`.
    MoveTo(f32, f32),
    /// Rotate position around `pivot` by `degrees` (clockwise). The
    /// area-side twin of
    /// [`GlyphModelCommand::Rotate`](crate::gfx_structs::model::GlyphModelCommand::Rotate);
    /// both bottom out in `clockwise_rotation_around_pivot`.
    Rotate {
        /// Pivot in world coordinates. On the wire this is a
        /// two-element `[x, y]` array — `glam` serializes `Vec2` as a
        /// sequence and its deserializer rejects the
        /// `{"x": …, "y": …}` object form. See `format/mutations.md`.
        pivot: Vec2,
        /// Rotation angle in degrees.
        degrees: f32,
    },
    /// Increase font scale by a delta.
    GrowFont(f32),
    /// Decrease font scale by a delta.
    ShrinkFont(f32),
    /// Replace font scale with an absolute value.
    SetFontSize(f32),
    /// Replace the line-height multiplier.
    SetLineHeight(f32),
    /// Increase line-height by a delta.
    GrowLineHeight(f32),
    /// Decrease line-height by a delta.
    ShrinkLineHeight(f32),
    /// Replace render bounds with absolute `(w, h)`.
    SetBounds(f32, f32),
    /// Assign a font to the given character range. O(n) in region count.
    SetRegionFont(Range, AppFont),
    /// Assign a color to the given character range. O(n) in region count.
    SetRegionColor(Range, FloatRgba),
    /// Remove the color/font region at the given character range.
    /// O(n) in region count.
    DeleteColorFontRegion(Range),
    /// Move an existing region from `current` to `new` range. O(n) in
    /// region count.
    ChangeRegionRange(Range, Range),
}

impl Applicable<GlyphArea> for GlyphAreaCommand {
    fn apply_to(&self, target: &mut GlyphArea) {
        match self {
            GlyphAreaCommand::PopFront(pop_count) => target.pop_front(*pop_count),
            GlyphAreaCommand::PopBack(pop_count) => target.pop_back(*pop_count),
            GlyphAreaCommand::MoveTo(x, y) => {
                target.set_position((*x, *y));
            }
            GlyphAreaCommand::Rotate { pivot, degrees } => {
                target.rotate(*pivot, *degrees);
            }
            GlyphAreaCommand::NudgeLeft(value) => {
                target.nudge_left(*value);
            }
            GlyphAreaCommand::NudgeRight(value) => {
                target.nudge_right(*value);
            }
            GlyphAreaCommand::NudgeDown(value) => {
                target.nudge_down(*value);
            }
            GlyphAreaCommand::NudgeUp(value) => {
                target.nudge_up(*value);
            }
            GlyphAreaCommand::GrowFont(value) => {
                target.grow_font(value);
            }
            GlyphAreaCommand::ShrinkFont(value) => {
                target.shrink_font(value);
            }
            GlyphAreaCommand::SetBounds(x, y) => {
                target.set_bounds((*x, *y));
            }
            GlyphAreaCommand::DeleteColorFontRegion(range) => {
                target.delete_color_font_region(range);
            }
            GlyphAreaCommand::ChangeRegionRange(current_range, new_range) => {
                target.change_region_range(current_range, new_range);
            }
            GlyphAreaCommand::SetRegionFont(range, font) => {
                target.set_region_font(range, font);
            }
            GlyphAreaCommand::SetRegionColor(range, color) => {
                target.set_region_color(range, color);
            }
            GlyphAreaCommand::SetFontSize(font_size) => {
                target.set_font_size(font_size);
            }
            GlyphAreaCommand::SetLineHeight(line_height) => {
                target.set_line_height(line_height);
            }
            GlyphAreaCommand::GrowLineHeight(line_height) => {
                target.grow_line_height(line_height);
            }
            GlyphAreaCommand::ShrinkLineHeight(line_height) => {
                target.shrink_line_height(line_height);
            }
        }
    }
}

////////////////////////////////////////
/////// DeltaGlyphArea Mutator ////////
///////////////////////////////////////

/// A field-set delta targeting a [`GlyphArea`]. The map keys guarantee
/// at most one entry per field type; the value carries the new payload
/// and (via the co-located `Operation` entry) the arithmetic semantics
/// to apply. Applied via [`Applicable::apply_to`], which dispatches
/// into [`GlyphArea::apply_operation`].
///
/// Storage, `new()`, the operation lookup, and the additive merge come
/// from the shared [`Delta`] container — the area and model deltas are
/// the same container over different vocabularies, not two hand-kept
/// twins. The accessors below are what *is* area-specific.
pub type DeltaGlyphArea = Delta<GlyphAreaField>;

impl Applicable<GlyphArea> for DeltaGlyphArea {
    fn apply_to(&self, target: &mut GlyphArea) {
        target.apply_operation(self)
    }
}

impl DeltaGlyphArea {
    /// Build a full-coverage `Assign` delta that mirrors every
    /// per-glyph field of `area` the in-place mutator path needs to
    /// re-stamp on a tree leaf. Emits `Text`, `position`, `bounds`,
    /// `scale`, `line_height`, `ColorFontRegions`, `Outline`, and
    /// `ZoomVisibility` (the latter required per `lib/baumhard/CONVENTIONS.md`
    /// §B2 — without it a mutator rebuild silently resets each
    /// element's authored zoom window to `Default`), with
    /// `ApplyOperation::Assign` as the global mode.
    ///
    /// Single source of truth for the per-leaf delta shape every
    /// `tree_builder/*::build_*_mutator_tree` function needs; lifting
    /// it to baumhard means any consumer (border, connection,
    /// connection_label, edge_handle, portal — and any future
    /// renderable element type) shares one definition of "what fields
    /// need to be re-asserted to keep the leaf in sync with its
    /// source area." Adding a new per-leaf field becomes a one-line
    /// change here, fanning out to every consumer.
    ///
    /// Cost: clones the area's text, regions, and outline; one
    /// 9-entry `FxHashMap`. No font-system access, no shaping.
    pub fn full_assign_from(area: &GlyphArea) -> DeltaGlyphArea {
        DeltaGlyphArea::new(vec![
            GlyphAreaField::Text(area.text.clone()),
            GlyphAreaField::position(area.position.x.0, area.position.y.0),
            GlyphAreaField::bounds(area.render_bounds.x.0, area.render_bounds.y.0),
            GlyphAreaField::scale(area.scale.0),
            GlyphAreaField::line_height(area.line_height.0),
            GlyphAreaField::ColorFontRegions(area.regions.clone()),
            GlyphAreaField::Outline(area.outline),
            GlyphAreaField::ZoomVisibility(area.zoom_visibility),
            GlyphAreaField::Operation(ApplyOperation::Assign),
        ])
    }

    /// Borrow the delta's color/font region payload, if any. O(1).
    pub fn color_font_regions(&self) -> Option<&ColorFontRegions> {
        match self.get(GlyphAreaFieldType::ColorFontRegions)? {
            GlyphAreaField::ColorFontRegions(regions) => Some(regions),
            _ => None,
        }
    }

    /// Return the delta's position payload as a `Vec2`, if any. O(1).
    pub fn position(&self) -> Option<Vec2> {
        match self.get(GlyphAreaFieldType::Position)? {
            GlyphAreaField::Position(position) => Some(position.to_vec2()),
            _ => None,
        }
    }

    /// Return the delta's scale payload, if any. O(1).
    pub fn scale(&self) -> Option<f32> {
        match self.get(GlyphAreaFieldType::Scale)? {
            GlyphAreaField::Scale(scale) => Some(scale.0),
            _ => None,
        }
    }

    /// Return the delta's line-height payload, if any. O(1).
    pub fn line_height(&self) -> Option<f32> {
        match self.get(GlyphAreaFieldType::LineHeight)? {
            GlyphAreaField::LineHeight(line_height) => Some(line_height.0),
            _ => None,
        }
    }

    /// Borrow the delta's text payload, if any. O(1) — no copy of
    /// the string.
    pub fn text_ref(&self) -> Option<&str> {
        match self.get(GlyphAreaFieldType::Text)? {
            GlyphAreaField::Text(text) => Some(text),
            _ => None,
        }
    }

    /// Return the delta's bounds payload as a `Vec2`, if any. O(1).
    pub fn bounds(&self) -> Option<Vec2> {
        match self.get(GlyphAreaFieldType::Bounds)? {
            GlyphAreaField::Bounds(bounds) => Some(bounds.to_vec2()),
            _ => None,
        }
    }

    /// Returns the delta's [`OutlineStyle`] payload if one was set
    /// on construction. Outer `Option` distinguishes "no outline
    /// field in this delta" (returns `None`) from "the delta
    /// explicitly clears the outline" (returns `Some(None)`); the
    /// latter is how a mutator removes a previously-set halo.
    pub fn outline(&self) -> Option<Option<OutlineStyle>> {
        match self.get(GlyphAreaFieldType::Outline)? {
            GlyphAreaField::Outline(outline) => Some(*outline),
            _ => None,
        }
    }

    /// Returns the delta's [`NodeShape`] payload if one was set on
    /// construction. `None` means the delta leaves the area's
    /// `shape` field alone; `Some(shape)` means it replaces (under
    /// Assign/Add) or resets-to-rectangle (under Subtract). O(1).
    pub fn shape(&self) -> Option<NodeShape> {
        match self.get(GlyphAreaFieldType::Shape)? {
            GlyphAreaField::Shape(shape) => Some(*shape),
            _ => None,
        }
    }

    /// Returns the delta's [`ZoomVisibility`] payload if one was
    /// set on construction. `None` means the delta leaves the
    /// area's `zoom_visibility` alone; `Some(window)` means it
    /// replaces (under Assign/Add) or resets-to-unbounded (under
    /// Subtract). O(1).
    pub fn zoom_visibility(&self) -> Option<ZoomVisibility> {
        match self.get(GlyphAreaFieldType::ZoomVisibility)? {
            GlyphAreaField::ZoomVisibility(window) => Some(*window),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::primitives::ColorFontRegions;
    use crate::gfx_structs::area::GlyphArea;
    use crate::gfx_structs::shape::NodeShape;
    use crate::gfx_structs::zoom_visibility::ZoomVisibility;
    use glam::Vec2;

    /// `full_assign_from` emits exactly the nine fields the
    /// in-place mutator path needs (Text / position / bounds /
    /// scale / line_height / regions / Outline / ZoomVisibility /
    /// Operation(Assign)). Locks the contract against a future
    /// silent omission of any per-leaf field — `lib/baumhard/CONVENTIONS.md
    /// §B2`.
    #[test]
    fn full_assign_from_emits_all_nine_fields() {
        let area = GlyphArea::new_with_str("test", 12.0, 12.0, Vec2::new(1.0, 2.0), Vec2::new(100.0, 50.0));
        let delta = DeltaGlyphArea::full_assign_from(&area);
        assert!(delta.fields.contains_key(&GlyphAreaFieldType::Text));
        assert!(delta.fields.contains_key(&GlyphAreaFieldType::Position));
        assert!(delta.fields.contains_key(&GlyphAreaFieldType::Bounds));
        assert!(delta.fields.contains_key(&GlyphAreaFieldType::Scale));
        assert!(delta.fields.contains_key(&GlyphAreaFieldType::LineHeight));
        assert!(delta.fields.contains_key(&GlyphAreaFieldType::ColorFontRegions));
        assert!(delta.fields.contains_key(&GlyphAreaFieldType::Outline));
        assert!(delta.fields.contains_key(&GlyphAreaFieldType::ZoomVisibility));
        assert!(delta.fields.contains_key(&GlyphAreaFieldType::Operation));
        assert_eq!(delta.operation_variant(), ApplyOperation::Assign);
    }

    /// The fields the helper emits round-trip through the
    /// `apply_to` path: applying the delta to a fresh `GlyphArea`
    /// reproduces the source's per-leaf state.
    #[test]
    fn full_assign_from_round_trips_through_apply_to() {
        use crate::core::primitives::Applicable;

        let mut source = GlyphArea::new_with_str(
            "round-trip",
            14.0,
            16.0,
            Vec2::new(7.0, 11.0),
            Vec2::new(200.0, 80.0),
        );
        source.zoom_visibility = ZoomVisibility::try_new(Some(0.5), Some(2.0)).unwrap();
        source.regions = ColorFontRegions::single_span(
            crate::util::grapheme_chad::count_grapheme_clusters("round-trip"),
            Some([1.0, 0.5, 0.25, 1.0]),
            None,
        );

        let delta = DeltaGlyphArea::full_assign_from(&source);
        let mut target = GlyphArea::new(0.0, 0.0, Vec2::ZERO, Vec2::ZERO);
        // Pre-condition: target shape differs from source shape so
        // any field that fails to overwrite would still be visible
        // in the post-state.
        target.shape = NodeShape::Ellipse;
        delta.apply_to(&mut target);

        assert_eq!(target.text, source.text);
        assert_eq!(target.scale, source.scale);
        assert_eq!(target.line_height, source.line_height);
        assert_eq!(target.position, source.position);
        assert_eq!(target.render_bounds, source.render_bounds);
        assert_eq!(target.regions, source.regions);
        assert_eq!(target.outline, source.outline);
        assert_eq!(target.zoom_visibility, source.zoom_visibility);
        // `Shape` is intentionally NOT in the full-assign field set —
        // it's policy, not per-leaf identity. Verify it stayed at
        // the pre-apply value (Ellipse) rather than being reset.
        assert_eq!(target.shape, NodeShape::Ellipse);
    }

    /// Authored zoom-visibility windows survive the round-trip.
    /// Locks the §B2 latent-bug fix that made `edge_handle.rs`'s
    /// mutator stop omitting `ZoomVisibility` from its delta.
    #[test]
    fn full_assign_from_preserves_authored_zoom_window() {
        use crate::core::primitives::Applicable;

        let mut source = GlyphArea::new(12.0, 12.0, Vec2::ZERO, Vec2::new(10.0, 10.0));
        source.zoom_visibility = ZoomVisibility::try_new(Some(1.5), Some(3.0)).unwrap();

        let delta = DeltaGlyphArea::full_assign_from(&source);
        let mut target = GlyphArea::new(0.0, 0.0, Vec2::ZERO, Vec2::ZERO);
        // Target starts with unbounded zoom — if the helper omits
        // ZoomVisibility from the delta, this assert fails.
        assert!(target.zoom_visibility.is_default());
        delta.apply_to(&mut target);
        assert_eq!(target.zoom_visibility, source.zoom_visibility);
    }
}
