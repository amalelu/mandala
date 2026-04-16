//! Command and delta mutators for `GlyphArea` — the two `Applicable`
//! implementations that the tree walker dispatches through.

use crate::core::primitives::{
    Applicable, ApplyOperation, ColorFontRegions, Range,
};
use crate::font::fonts::AppFont;
use crate::util::color::FloatRgba;
use glam::f32::Vec2;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::ops::Add;
use strum_macros::{Display, EnumIter};

use super::area::GlyphArea;
use super::area_fields::{GlyphAreaField, GlyphAreaFieldType, OutlineStyle};

////////////////////////////////////////
/////// GlyphAreaCommand Mutator ///////
///////////////////////////////////////

/// Tag enum for [`GlyphAreaCommand`] — identifies the command kind
/// without carrying payload. Used as a key in `HashSet`/`HashMap`
/// look-ups where the caller needs to know *which* command was
/// scheduled but not its parameters.
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize, Eq, Hash, EnumIter, Display)]
pub enum GlyphAreaCommandType {
    /// Remove grapheme clusters from the front of the text.
    PopFront,
    /// Remove grapheme clusters from the back of the text.
    PopBack,
    /// Shift position left by a pixel delta.
    NudgeLeft,
    /// Shift position right by a pixel delta.
    NudgeRight,
    /// Shift position down by a pixel delta.
    NudgeDown,
    /// Shift position up by a pixel delta.
    NudgeUp,
    /// Teleport position to an absolute (x, y).
    MoveTo,
    /// Increase font scale by a delta.
    GrowFont,
    /// Decrease font scale by a delta.
    ShrinkFont,
    /// Replace font scale with an absolute value.
    SetFontSize,
    /// Replace the line-height multiplier.
    SetLineHeight,
    /// Increase line-height by a delta.
    GrowLineHeight,
    /// Decrease line-height by a delta.
    ShrinkLineHeight,
    /// Replace render bounds with absolute (w, h).
    SetBounds,
    /// Assign a font to a character range.
    SetRegionFont,
    /// Assign a colour to a character range.
    SetRegionColor,
    /// Remove the colour/font region at a character range.
    DeleteColorFontRegion,
    /// Move an existing region's span to a new character range.
    ChangeRegionRange,
}

/// Imperative mutation command applied to a [`GlyphArea`] via its
/// [`Applicable`] impl. Unlike [`DeltaGlyphArea`] (which is
/// arithmetic — `Add`/`Assign`/`Subtract`), a command performs a
/// single named operation whose semantics are fixed. All variants
/// are O(1) except the `ColorFontRegion`-touching ones, which are
/// O(n) in the number of existing regions.
#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
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
    /// Assign a colour to the given character range. O(n) in region count.
    SetRegionColor(Range, FloatRgba),
    /// Remove the colour/font region at the given character range.
    /// O(n) in region count.
    DeleteColorFontRegion(Range),
    /// Move an existing region from `current` to `new` range. O(n) in
    /// region count.
    ChangeRegionRange(Range, Range),
}

impl GlyphAreaCommand {
    pub fn variant(&self) -> GlyphAreaCommandType {
        match self {
            GlyphAreaCommand::PopFront(_) => GlyphAreaCommandType::PopFront,
            GlyphAreaCommand::PopBack(_) => GlyphAreaCommandType::PopBack,
            GlyphAreaCommand::NudgeLeft(_) => GlyphAreaCommandType::NudgeLeft,
            GlyphAreaCommand::NudgeRight(_) => GlyphAreaCommandType::NudgeRight,
            GlyphAreaCommand::NudgeDown(_) => GlyphAreaCommandType::NudgeDown,
            GlyphAreaCommand::NudgeUp(_) => GlyphAreaCommandType::NudgeUp,
            GlyphAreaCommand::MoveTo(_, _) => GlyphAreaCommandType::MoveTo,
            GlyphAreaCommand::GrowFont(_) => GlyphAreaCommandType::GrowFont,
            GlyphAreaCommand::ShrinkFont(_) => GlyphAreaCommandType::ShrinkFont,
            GlyphAreaCommand::SetFontSize(_) => GlyphAreaCommandType::SetFontSize,
            GlyphAreaCommand::SetLineHeight(_) => GlyphAreaCommandType::SetLineHeight,
            GlyphAreaCommand::GrowLineHeight(_) => GlyphAreaCommandType::GrowLineHeight,
            GlyphAreaCommand::ShrinkLineHeight(_) => GlyphAreaCommandType::ShrinkLineHeight,
            GlyphAreaCommand::SetBounds(_, _) => GlyphAreaCommandType::SetBounds,
            GlyphAreaCommand::SetRegionFont(_, _) => GlyphAreaCommandType::SetRegionFont,
            GlyphAreaCommand::SetRegionColor(_, _) => GlyphAreaCommandType::SetRegionColor,
            GlyphAreaCommand::DeleteColorFontRegion(_) => {
                GlyphAreaCommandType::DeleteColorFontRegion
            }
            GlyphAreaCommand::ChangeRegionRange { .. } => GlyphAreaCommandType::ChangeRegionRange,
        }
    }
}

impl Applicable<GlyphArea> for GlyphAreaCommand {
    fn apply_to(&self, target: &mut GlyphArea) {
        match self {
            GlyphAreaCommand::PopFront(pop_count) => target.pop_front(*pop_count),
            GlyphAreaCommand::PopBack(pop_count) => target.pop_back(*pop_count),
            GlyphAreaCommand::MoveTo(x, y) => {
                target.set_position((*x, *y));
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

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct DeltaGlyphArea {
    pub fields: FxHashMap<GlyphAreaFieldType, GlyphAreaField>,
}

impl Applicable<GlyphArea> for DeltaGlyphArea {
    fn apply_to(&self, target: &mut GlyphArea) {
        target.apply_operation(&self)
    }
}

impl Add for DeltaGlyphArea {
    type Output = DeltaGlyphArea;

    fn add(self, rhs: Self) -> Self::Output {
        let mut fields = FxHashMap::default();
        for (key, value) in self.fields {
            if let Some(other_value) = rhs.fields.get(&key) {
                // If both sides have the same field, add them together
                fields.insert(key, value + other_value.clone());
            } else {
                // If only one side has the field, just copy it over
                fields.insert(key, value);
            }
        }
        // Copy over any fields that are only in the rhs
        for (key, value) in rhs.fields {
            if !fields.contains_key(&key) {
                fields.insert(key, value);
            }
        }
        DeltaGlyphArea { fields }
    }
}

impl DeltaGlyphArea {
    pub fn new(fields: Vec<GlyphAreaField>) -> DeltaGlyphArea {
        let mut field_map = FxHashMap::default();
        for field in fields {
            field_map.insert(field.variant(), field.clone());
        }

        DeltaGlyphArea { fields: field_map }
    }

    pub fn operation_variant(&self) -> ApplyOperation {
        if let Some(GlyphAreaField::Operation(operation)) =
            self.fields.get(&GlyphAreaFieldType::ApplyOperation)
        {
            *operation
        } else {
            ApplyOperation::Noop
        }
    }

    pub fn color_font_regions(&self) -> Option<&ColorFontRegions> {
        if let Some(GlyphAreaField::ColorFontRegions(color_font_regions)) =
            self.fields.get(&GlyphAreaFieldType::ColorFontRegions)
        {
            Some(color_font_regions)
        } else {
            None
        }
    }

    pub fn position(&self) -> Option<Vec2> {
        if let Some(GlyphAreaField::Position(x)) = self.fields.get(&GlyphAreaFieldType::Position) {
            Some(x.to_vec2())
        } else {
            None
        }
    }

    pub fn scale(&self) -> Option<f32> {
        if let Some(GlyphAreaField::Scale(scale)) = self.fields.get(&GlyphAreaFieldType::Scale) {
            Some(scale.0)
        } else {
            None
        }
    }

    pub fn line_height(&self) -> Option<f32> {
        if let Some(GlyphAreaField::LineHeight(line_height)) =
            self.fields.get(&GlyphAreaFieldType::LineHeight)
        {
            Some(line_height.0)
        } else {
            None
        }
    }

    pub fn text_ref(&self) -> Option<&str> {
        if let Some(GlyphAreaField::Text(text)) = self.fields.get(&GlyphAreaFieldType::Text) {
            Some(text)
        } else {
            None
        }
    }

    pub fn bounds(&self) -> Option<Vec2> {
        if let Some(GlyphAreaField::Bounds(x)) = self.fields.get(&GlyphAreaFieldType::Bounds) {
            Some(x.to_vec2())
        } else {
            None
        }
    }

    /// Returns the delta's [`OutlineStyle`] payload if one was set
    /// on construction. Outer `Option` distinguishes "no outline
    /// field in this delta" (returns `None`) from "the delta
    /// explicitly clears the outline" (returns `Some(None)`); the
    /// latter is how a mutator removes a previously-set halo.
    pub fn outline(&self) -> Option<Option<OutlineStyle>> {
        if let Some(GlyphAreaField::Outline(outline)) =
            self.fields.get(&GlyphAreaFieldType::Outline)
        {
            Some(*outline)
        } else {
            None
        }
    }
}
