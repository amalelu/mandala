use crate::core::primitives::{ColorFontRegionField, Flag, Flaggable, Range};
use crate::gfx_structs::area::{GlyphArea, GlyphAreaField};
use crate::gfx_structs::model::{GlyphModel, GlyphModelField};
use crate::gfx_structs::mutator::GlyphTreeEventInstance;
use crate::gfx_structs::tree::{BranchChannel, EventSubscriber, TreeEventConsumer, TreeNode};
use crate::util::color::FloatRgba;
use crate::util::geometry::clockwise_rotation_around_pivot;
use crate::util::ordered_vec2::OrderedVec2;
use glam::Vec2;
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};
use std::fmt::{Debug, Formatter};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum GfxElementField {
    GlyphArea(GlyphAreaField),
    GlyphModel(GlyphModelField),
    Region(Range, ColorFontRegionField),
    Channel(usize),
    Id(usize),
    Flag(Flag),
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum GfxElementType {
    GlyphArea,
    GlyphModel,
    Void,
}

pub enum GfxElement {
    GlyphArea {
        glyph_area: Box<GlyphArea>,
        flags: FxHashSet<Flag>,
        channel: usize,
        unique_id: usize,
        event_subscribers: Vec<EventSubscriber>,
    },
    /// A [GlyphModel] typically needs a container (such as [GlyphArea]) in order to make sense
    GlyphModel {
        glyph_model: Box<GlyphModel>,
        flags: FxHashSet<Flag>,
        channel: usize,
        unique_id: usize,
        event_subscribers: Vec<EventSubscriber>,
    },
    /// A Void element is simply a tool to create trees where certain nodes are ignored
    /// For the purpose of following some pattern, for example conforming to an existing mutator tree
    Void {
        channel: usize,
        unique_id: usize,
        event_subscribers: Vec<EventSubscriber>,
        flags: FxHashSet<Flag>,
    },
}

impl GfxElement {
    pub fn new_area_non_indexed(section: GlyphArea, channel: usize) -> GfxElement {
        Self::new_area_non_indexed_with_id(section, channel, 0)
    }
    pub fn new_area_non_indexed_with_id(
        section: GlyphArea,
        channel: usize,
        unique_id: usize,
    ) -> GfxElement {
        GfxElement::GlyphArea {
            glyph_area: Box::new(section),
            flags: Default::default(),
            channel,
            unique_id,
            event_subscribers: vec![],
        }
    }

    pub fn new_void(channel: usize) -> GfxElement {
        Self::new_void_with_id(channel, 0)
    }

    pub fn new_void_with_id(channel: usize, unique_id: usize) -> GfxElement {
        GfxElement::Void {
            channel,
            unique_id,
            event_subscribers: vec![],
            flags: Default::default(),
        }
    }
    pub fn new_model_non_indexed(model: GlyphModel, channel: usize, unique_id: usize) -> Self {
        Self::new_model_non_indexed_with_id(model, channel, unique_id)
    }

    pub fn new_model_non_indexed_with_id(
        model: GlyphModel,
        channel: usize,
        unique_id: usize,
    ) -> Self {
        GfxElement::GlyphModel {
            glyph_model: Box::new(model),
            flags: Default::default(),
            channel,
            unique_id,
            event_subscribers: vec![],
        }
    }

    pub fn new_model_blank(channel: usize, unique_id: usize) -> GfxElement {
        Self::new_model_blank_with_id(channel, unique_id)
    }

    pub fn new_model_blank_with_id(channel: usize, unique_id: usize) -> GfxElement {
        GfxElement::GlyphModel {
            glyph_model: Box::new(GlyphModel::new()),
            flags: Default::default(),
            channel,
            unique_id,
            event_subscribers: vec![],
        }
    }

    pub fn subscribers_mut(&mut self) -> &mut Vec<EventSubscriber> {
        match self {
            GfxElement::GlyphArea {
                event_subscribers, ..
            } => event_subscribers,
            GfxElement::GlyphModel {
                event_subscribers, ..
            } => event_subscribers,
            GfxElement::Void {
                event_subscribers, ..
            } => event_subscribers,
        }
    }

    pub fn subscribers_as_ref(&self) -> &Vec<EventSubscriber> {
        match self {
            GfxElement::GlyphArea {
                event_subscribers, ..
            } => event_subscribers.as_ref(),
            GfxElement::GlyphModel {
                event_subscribers, ..
            } => event_subscribers.as_ref(),
            GfxElement::Void {
                event_subscribers, ..
            } => event_subscribers.as_ref(),
        }
    }

    pub fn set_unique_id(&mut self, id: usize) {
        match self {
            GfxElement::GlyphArea { unique_id, .. } => *unique_id = id,
            GfxElement::GlyphModel { unique_id, .. } => *unique_id = id,
            GfxElement::Void { unique_id, .. } => *unique_id = id,
        }
    }

    pub fn get_type(&self) -> GfxElementType {
        match self {
            GfxElement::GlyphArea { .. } => GfxElementType::GlyphArea,
            GfxElement::Void { .. } => GfxElementType::Void,
            GfxElement::GlyphModel { .. } => GfxElementType::GlyphModel,
        }
    }

    pub fn glyph_area_mut(&mut self) -> Option<&mut GlyphArea> {
        match self {
            GfxElement::GlyphArea {
                glyph_area: section,
                ..
            } => Some(section.as_mut()),
            GfxElement::Void { .. } => None,
            GfxElement::GlyphModel { .. } => None,
        }
    }

    pub fn glyph_area(&self) -> Option<&GlyphArea> {
        match self {
            GfxElement::GlyphArea {
                glyph_area: section,
                ..
            } => Some(section),
            GfxElement::Void { .. } => None,
            GfxElement::GlyphModel { .. } => None,
        }
    }

    pub fn glyph_model(&self) -> Option<&GlyphModel> {
        match self {
            GfxElement::GlyphModel { glyph_model, .. } => Some(glyph_model),
            _ => None,
        }
    }

    pub fn glyph_model_mut(&mut self) -> Option<&mut GlyphModel> {
        match self {
            GfxElement::GlyphModel { glyph_model, .. } => Some(glyph_model),
            _ => None,
        }
    }

    pub fn unique_id(&self) -> usize {
        match self {
            GfxElement::GlyphArea { unique_id, .. } => *unique_id,
            GfxElement::Void { unique_id, .. } => *unique_id,
            GfxElement::GlyphModel { unique_id, .. } => *unique_id,
        }
    }

    pub fn position(&self) -> Vec2 {
        match self {
            GfxElement::GlyphArea {
                glyph_area: section,
                ..
            } => section.position.to_vec2(),
            GfxElement::Void { .. } => Vec2::NAN,
            GfxElement::GlyphModel { glyph_model, .. } => glyph_model.position.to_vec2(),
        }
    }

    pub fn color_at_region(&self, range: Range) -> Option<FloatRgba> {
        match self {
            GfxElement::GlyphArea { glyph_area, .. } => glyph_area
                .regions
                .get(range)
                .and_then(|region| region.color),
            GfxElement::GlyphModel { .. } => panic!("Not yet implemented"),
            GfxElement::Void { .. } => None,
        }
    }

    pub fn set_position(&mut self, position: Vec2) {
        match self {
            GfxElement::GlyphArea {
                ref mut glyph_area, ..
            } => {
                glyph_area.position = OrderedVec2::from_vec2(position);
            }
            GfxElement::Void { .. } => {
                // Nothing needs doing
            }
            GfxElement::GlyphModel {
                ref mut glyph_model,
                ..
            } => {
                glyph_model.position = OrderedVec2::from_vec2(position);
            }
        }
    }

    pub fn rotate(&mut self, pivot: Vec2, degrees: f32) {
        let position = self.position();
        if !position.is_nan() {
            self.set_position(clockwise_rotation_around_pivot(position, pivot, degrees));
        }
    }
}

impl TreeNode for GfxElement {
    fn void() -> Self {
        Self::new_void_with_id(0, 0)
    }
}

impl Flaggable for GfxElement {
    fn flag_is_set(&self, flag: Flag) -> bool {
        match self {
            GfxElement::GlyphArea { flags, .. } => flags.contains(&flag),
            GfxElement::GlyphModel { flags, .. } => flags.contains(&flag),
            GfxElement::Void { .. } => false,
        }
    }

    fn set_flag(&mut self, flag: Flag) {
        match self {
            GfxElement::GlyphArea { flags, .. } => {
                flags.insert(flag);
            }
            GfxElement::GlyphModel { flags, .. } => {
                flags.insert(flag);
            }
            GfxElement::Void { .. } => {}
        }
    }

    fn clear_flag(&mut self, flag: Flag) {
        match self {
            GfxElement::GlyphArea { flags, .. } => {
                flags.remove(&flag);
            }
            GfxElement::GlyphModel { flags, .. } => {
                flags.remove(&flag);
            }
            GfxElement::Void { .. } => {}
        }
    }
}

impl Debug for GfxElement {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        //todo
        Ok(())
    }
}

impl PartialEq for GfxElement {
    fn eq(&self, other: &Self) -> bool {
        if self.get_type() == other.get_type() {
            return match self.get_type() {
                GfxElementType::GlyphArea => {
                    *(self.glyph_area().unwrap()) == *(other.glyph_area().unwrap())
                }
                GfxElementType::GlyphModel => {
                    *self.glyph_model().unwrap() == *other.glyph_model().unwrap()
                }
                GfxElementType::Void => true,
            };
        }
        false
    }
}

impl Clone for GfxElement {
    fn clone(&self) -> Self {
        match self.get_type() {
            GfxElementType::GlyphArea => {
                let mut output = GfxElement::new_area_non_indexed_with_id(
                    self.glyph_area().unwrap().clone(),
                    self.channel(),
                    self.unique_id(),
                );

                *output.subscribers_mut() = self.subscribers_as_ref().clone();
                output
            }
            GfxElementType::GlyphModel => {
                let mut output = GfxElement::new_model_non_indexed_with_id(
                    self.glyph_model().unwrap().clone(),
                    self.channel(),
                    self.unique_id(),
                );

                *output.subscribers_mut() = self.subscribers_as_ref().clone();
                output
            }
            GfxElementType::Void => {
                let mut output = GfxElement::new_void_with_id(self.channel(), self.unique_id());
                *output.subscribers_mut() = self.subscribers_as_ref().clone();
                output
            }
        }
    }
}

impl TreeEventConsumer for GfxElement {
    fn accept_event(&mut self, event: &GlyphTreeEventInstance) {
        let subscribers = self.subscribers_as_ref().clone();
        for sub in subscribers {
            sub.lock()
                .expect("Failed to acquire lock for EventSubscriber")(
                self, event.clone()
            );
        }
    }
}

impl BranchChannel for GfxElement {
    fn channel(&self) -> usize {
        match self {
            GfxElement::GlyphArea { channel, .. } => *channel,
            GfxElement::Void { channel, .. } => *channel,
            GfxElement::GlyphModel { channel, .. } => *channel,
        }
    }
}

impl Default for GfxElement {
    fn default() -> Self {
        Self::void()
    }
}
