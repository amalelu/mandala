use crate::core::primitives::{
    Applicable, ApplyOperation, ColorFontRegion, ColorFontRegions, Range,
};
use crate::font::fonts::AppFont;
use crate::gfx_structs::util::regions::{RegionIndexer, RegionParams};
use crate::util::color::FloatRgba;
use crate::util::grapheme_chad;
use crate::util::ordered_vec2::OrderedVec2;
use derivative::Derivative;
use glam::f32::Vec2;
use ordered_float::OrderedFloat;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::ops::Add;
use std::sync::Arc;
use strum_macros::{Display, EnumIter};
use crate::gfx_structs::util::hitbox::HitBox;

/// This is for use in HashMaps and Sets
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum GlyphAreaFieldType {
    Text,
    Scale,
    LineHeight,
    Flags,
    Position,
    Bounds,
    ColorFontRegions,
    ApplyOperation,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub enum GlyphAreaField {
    Text(String),
    Scale(OrderedFloat<f32>),
    LineHeight(OrderedFloat<f32>),
    Position(OrderedVec2),
    Bounds(OrderedVec2),
    ColorFontRegions(ColorFontRegions),
    Operation(ApplyOperation),
}

impl Add for GlyphAreaField {
    type Output = GlyphAreaField;

    fn add(self, rhs: Self) -> Self::Output {
        {
            match self {
                GlyphAreaField::Text(txt) => {
                    if let GlyphAreaField::Text(other) = rhs {
                        return GlyphAreaField::Text(txt + &other);
                    }
                }
                GlyphAreaField::Scale(scale) => {
                    if let GlyphAreaField::Scale(other) = rhs {
                        return GlyphAreaField::Scale(scale + other);
                    }
                }
                GlyphAreaField::Position(this) => {
                    if let GlyphAreaField::Position(other) = rhs {
                        return GlyphAreaField::Position(OrderedVec2::new(
                            this.x + other.x,
                            this.y + other.y,
                        ));
                    }
                }
                GlyphAreaField::Bounds(this) => {
                    if let GlyphAreaField::Bounds(other) = rhs {
                        return GlyphAreaField::Bounds(OrderedVec2::new(
                            this.x + other.x,
                            this.y + other.y,
                        ));
                    }
                }
                GlyphAreaField::ColorFontRegions(regions) => {
                    if let GlyphAreaField::ColorFontRegions(other) = rhs {
                        let mut color_font_regions = ColorFontRegions::new_empty();
                        for region in regions.regions {
                            color_font_regions.submit_region(region);
                        }
                        for region in other.regions {
                            color_font_regions.submit_region(region);
                        }
                        return GlyphAreaField::ColorFontRegions(color_font_regions);
                    }
                }
                GlyphAreaField::LineHeight(height) => {
                    if let GlyphAreaField::LineHeight(other_height) = rhs {
                        return GlyphAreaField::LineHeight(height + other_height);
                    }
                }
                GlyphAreaField::Operation(_) => {}
            }
        }
        panic!("Tried to add different types of GlyphBlockFields");
    }
}

impl GlyphAreaField {
    pub fn scale(s: f32) -> Self {
        GlyphAreaField::Scale(OrderedFloat::from(s))
    }

    pub fn line_height(line_height: f32) -> Self {
        GlyphAreaField::LineHeight(OrderedFloat::from(line_height))
    }

    pub fn bounds(x: f32, y: f32) -> Self {
        GlyphAreaField::Bounds(OrderedVec2::new_f32(x, y))
    }

    pub fn position(x: f32, y: f32) -> Self {
        GlyphAreaField::Position(OrderedVec2::new_f32(x, y))
    }
    pub const fn variant(&self) -> GlyphAreaFieldType {
        match self {
            GlyphAreaField::Text(_) => GlyphAreaFieldType::Text,
            GlyphAreaField::Scale(_) => GlyphAreaFieldType::Scale,
            GlyphAreaField::Position(_) => GlyphAreaFieldType::Position,
            GlyphAreaField::Bounds(_) => GlyphAreaFieldType::Bounds,
            GlyphAreaField::ColorFontRegions(_) => GlyphAreaFieldType::ColorFontRegions,
            GlyphAreaField::LineHeight(_) => GlyphAreaFieldType::LineHeight,
            GlyphAreaField::Operation(_) => GlyphAreaFieldType::ApplyOperation,
        }
    }

    #[inline]
    pub fn same_type(&self, other: &GlyphAreaField) -> bool {
        self.variant() == other.variant()
    }
}

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
    #[derivative(PartialEq = "ignore")]
    pub hitbox: HitBox,
    #[serde(skip)]
    #[derivative(PartialEq = "ignore")]
    screen_region_params: Option<Arc<RegionParams>>,
    #[serde(skip)]
    #[derivative(PartialEq = "ignore")]
    tree_index: Option<Arc<RegionIndexer>>,
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
            hitbox: HitBox::new(),
            screen_region_params: None,
            tree_index: None,
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
            hitbox: HitBox::new(),
            screen_region_params: None,
            tree_index: None,
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
        let mut should_update_index = false;

        if delta.position().is_some() {
            let position = OrderedVec2::from_vec2(delta.position().unwrap());
            operation.apply(&mut self.position.x, position.x);
            operation.apply(&mut self.position.y, position.y);
            should_update_index = true;
        }

        if delta.bounds().is_some() {
            let bounds = OrderedVec2::from_vec2(delta.bounds().unwrap());
            operation.apply(&mut self.render_bounds.x, bounds.x);
            operation.apply(&mut self.render_bounds.y, bounds.y);
            should_update_index = true;
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
        if should_update_index {
            self.update_index_if_relevant()
        }
    }

    pub(crate) fn update_index_if_relevant(&mut self) {
        if self.tree_index.is_some() && self.screen_region_params.is_some() {
            // TODO update ..
            //self.tree_index.unwrap()
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
        self.update_index_if_relevant();
    }

    pub fn nudge_right(&mut self, nudge: f32) {
        self.position.x += nudge;
        self.update_index_if_relevant();
    }

    pub fn nudge_left(&mut self, nudge: f32) {
        self.position.x -= nudge;
        self.update_index_if_relevant();
    }

    pub fn nudge_up(&mut self, nudge: f32) {
        self.position.y -= nudge;
        self.update_index_if_relevant();
    }

    pub fn nudge_down(&mut self, nudge: f32) {
        self.position.y += nudge;
        self.update_index_if_relevant();
    }

    pub fn grow_font(&mut self, value: &f32) {
        self.scale += value;
    }

    pub fn shrink_font(&mut self, value: &f32) {
        self.scale -= value;
    }

    pub fn set_bounds(&mut self, bounds: (f32, f32)) {
        self.render_bounds = OrderedVec2::new_f32(bounds.0, bounds.1);
        self.update_index_if_relevant();
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
        self.update_index_if_relevant();
    }

    pub fn rotate(&mut self, pivot: Vec2, angle: f32) {
        self.position =
            OrderedVec2::from_vec2(Vec2::from_angle(angle).rotate(self.position.to_vec2() - pivot));
    }
}

////////////////////////////////////////
/////// GlyphAreaCommand Mutator ///////
///////////////////////////////////////

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize, Eq, Hash, EnumIter, Display)]
pub enum GlyphAreaCommandType {
    PopFront,
    PopBack,
    NudgeLeft,
    NudgeRight,
    NudgeDown,
    NudgeUp,
    MoveTo,
    GrowFont,
    ShrinkFont,
    SetFontSize,
    SetLineHeight,
    GrowLineHeight,
    ShrinkLineHeight,
    SetBounds,
    SetRegionFont,
    SetRegionColor,
    DeleteColorFontRegion,
    ChangeRegionRange,
}

#[derive(Clone, Debug, Copy, Serialize, Deserialize)]
pub enum GlyphAreaCommand {
    PopFront(usize),
    PopBack(usize),
    NudgeLeft(f32),
    NudgeRight(f32),
    NudgeDown(f32),
    NudgeUp(f32),
    MoveTo(f32, f32),
    GrowFont(f32),
    ShrinkFont(f32),
    SetFontSize(f32),
    SetLineHeight(f32),
    GrowLineHeight(f32),
    ShrinkLineHeight(f32),
    SetBounds(f32, f32),
    SetRegionFont(Range, AppFont),
    SetRegionColor(Range, FloatRgba),
    DeleteColorFontRegion(Range),
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
}
