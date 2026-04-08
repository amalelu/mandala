use std::cmp::Ordering;
use std::collections::BTreeSet;

use std::hash::{Hash, Hasher};
use std::ops::{AddAssign, MulAssign, SubAssign};
use log::{debug};
use serde::{Deserialize, Serialize};

use crate::font::fonts::AppFont;
use crate::util::color::FloatRgba;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ColorFontRegion {
    pub range: Range,
    pub font: Option<AppFont>,
    pub color: Option<FloatRgba>,
}

impl Hash for ColorFontRegion {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // We only hash the range here so that we can use the range as a key to get
        self.range.hash(state);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ColorFontRegionField {
    Range(Range),
    Font(AppFont),
    Color(FloatRgba),
    This,
}

impl PartialOrd for ColorFontRegion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.range.partial_cmp(&other.range)
    }
}

impl Eq for ColorFontRegion {}

impl PartialEq for ColorFontRegion {
    fn eq(&self, other: &Self) -> bool {
        self.range == other.range
    }
}

impl Ord for ColorFontRegion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.range.cmp(&other.range)
    }
}

impl ColorFontRegion {
    pub fn new(range: Range, font: Option<AppFont>, color: Option<FloatRgba>) -> Self {
        ColorFontRegion { range, font, color }
    }
    pub fn new_key_only(range: Range) -> Self {
        ColorFontRegion {
            range,
            font: None,
            color: None,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, Hash)]
pub struct ColorFontRegions {
    pub regions: BTreeSet<ColorFontRegion>,
}

impl ColorFontRegions {
    pub fn new_from(source: Vec<ColorFontRegion>) -> Self {
        ColorFontRegions {
            regions: source.into_iter().collect(),
        }
    }
    pub fn new_empty() -> Self {
        ColorFontRegions {
            regions: BTreeSet::new(),
        }
    }

    pub fn all_regions(&self) -> Vec<&ColorFontRegion> {
        self.regions.iter().collect()
    }

    pub fn num_regions(&self) -> usize {
        self.regions.len()
    }

    pub fn submit_region(&mut self, region: ColorFontRegion) {
        if region.range.start > region.range.end {
            panic!("Range start is higher than range end!");
        }
        if self.regions.contains(&region) {
            self.regions.remove(&region);
        }
        self.regions.insert(region);
    }

    /// If there is a region in the given range, it will be split in two at the start of the range
    /// and the second half will be pushed forward to the end of the range
    pub fn split_and_separate(&mut self, range: Range) {
        let mut copy_of_regions: Vec<_> = self.regions.iter().copied().collect();
        let mut cloned_regions: Vec<ColorFontRegion> = Vec::new();
        for region in &mut copy_of_regions {
            if range.overlaps(&region.range) {
                let mut right_part = region.clone();
                right_part.range.start = range.end;
                right_part.range.end += range.magnitude();
                cloned_regions.push(right_part);
                region.range.end = range.start;
            }
            if region.range.start >= range.end {
                region.range.push_right(range.magnitude());
            }
        }
        self.regions.clear();
        self.regions.extend(copy_of_regions);
        self.regions.extend(cloned_regions);
    }

    pub fn shift_regions_after(&mut self, idx: usize, magnitude: usize) {
        let mut copy_of_regions: Vec<_> = self.regions.iter().copied().collect();
        for region in &mut copy_of_regions {
            if region.range.start > idx {
                region.range.start += magnitude;
                region.range.end += magnitude;
            }
        }
        self.regions.clear();
        self.regions.extend(copy_of_regions);
    }

    pub fn replace_regions(&mut self, regions: &Self) {
        self.regions.clear();
        for region in &regions.regions {
            self.regions.insert(*region);
        }
    }

    pub fn set_or_insert(&mut self, region: &ColorFontRegion) {
        if self.regions.contains(region) {
            let mut new_region = self.regions.get(region).unwrap().clone();
            if region.color.is_some() {
                new_region.color = region.color;
            }
            if region.font.is_some() {
                new_region.font = region.font;
            }
            self.submit_region(new_region);
        } else {
            self.regions.insert(*region);
        }
    }

    pub fn get(&self, range: Range) -> Option<&ColorFontRegion> {
        self.regions.get(&ColorFontRegion::new_key_only(range))
    }

    pub fn hard_get(&self, range: Range) -> ColorFontRegion {
        debug!("Doing a hard_get on node, regions will follow:");
        for r in self.regions.iter() {
            debug!("{} - {}", r.range.start, r.range.end);
        }
        *self
            .regions
            .get(&ColorFontRegion::new_key_only(range))
            .expect(
                "You tried real hard to get that region, but it didn't exist.. (seriously, fuck you)",
            )
    }

    pub fn remove_range(&mut self, range: Range) -> bool {
        self.remove(&ColorFontRegion::new_key_only(range))
    }

    pub fn remove(&mut self, region: &ColorFontRegion) -> bool {
        self.regions.remove(region)
    }
}

impl Default for ColorFontRegions {
    fn default() -> Self {
        ColorFontRegions::new_empty()
    }
}

use strum_macros::{EnumString, Display};
use crate::util::ordered_vec2::OrderedVec2;

#[derive(Clone, Copy, Eq, PartialEq, Debug, EnumString, Display, Serialize, Deserialize)]
pub enum ApplyOperation {
    Add,
    Assign,
    Delete,
    Subtract,
    Multiply,
    Noop,
}

impl ApplyOperation {
    pub fn apply<T: AddAssign<T> + MulAssign<T> + SubAssign<T> + Default>(
        &self,
        lhs: &mut T,
        rhs: T,
    ) {
        match self {
            ApplyOperation::Add => *lhs += rhs,
            ApplyOperation::Assign => *lhs = rhs,
            ApplyOperation::Subtract => *lhs -= rhs,
            ApplyOperation::Multiply => *lhs *= rhs,
            ApplyOperation::Noop => {}
            ApplyOperation::Delete => *lhs = T::default(),
        }
    }
}

#[derive(Clone, Copy, PartialOrd, Ord, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct Range {
    pub start: usize,
    pub end: usize,
}

impl Range {
    pub fn tup(range: (usize, usize)) -> Self {
        Range {
            start: range.0,
            end: range.1,
        }
    }
    pub fn new(start: usize, end: usize) -> Self {
        Range { start, end }
    }

    pub fn to_rust_range(&self) -> std::ops::Range<usize> {
        self.start..self.end
    }

    pub fn magnitude(&self) -> usize {
        self.end - self.start
    }

    pub fn push_right(&mut self, n: usize) {
        self.start += n;
        self.end += n;
    }

    pub fn push_left(&mut self, n: usize) {
        self.start -= n;
        self.end -= n;
    }

    pub fn overlaps(&self, other: &Self) -> bool {
        if self.start >= other.end || other.start >= self.end {
            return false;
        }
        true
    }
}

pub trait Flaggable {
    fn flag_is_set(&self, flag: Flag) -> bool;
    fn set_flag(&mut self, flag: Flag);
    fn clear_flag(&mut self, flag: Flag);
}

pub trait Applicable<T: Clone> {
    fn apply_to(&self, target: &mut T);
}

// This was created to allow for integrated UI functionality
// We will probably need a lot more flags in order to support
// A complete UI experience
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Hash, Eq, PartialEq)]
pub enum Flag {
    Focused,
    Mutable,
    Anchored(AnchorBox),
    /// If set in an element, all mutations should also create a corresponding event
    MutationEvents,
}

#[derive(Clone, Copy, Hash, Eq, PartialEq, Debug, Serialize, Deserialize)]
pub enum AnchorBox {
    Single(Anchor),
    Dual(Anchor, Anchor),
    Trio(Anchor, Anchor, Anchor),
    Full(Anchor, Anchor, Anchor, Anchor),
}

#[derive(Clone, Copy, Hash, Eq, PartialEq, Debug, Serialize, Deserialize)]
pub struct Anchor {
    target: AnchorTarget,
    self_point: AnchorPoint,
    target_point: AnchorPoint,
}

impl Anchor {
    pub fn new(target: AnchorTarget, target_point: AnchorPoint, self_point: AnchorPoint) -> Self {
        Anchor {
            target,
            self_point,
            target_point,
        }
    }

    pub fn on_parent(parent_point: AnchorPoint, self_point: AnchorPoint) -> Self {
        Anchor {
            target: AnchorTarget::Parent { generation_offset: 0 },
            self_point,
            target_point: parent_point,
        }
    }

    pub fn on_window(window_point: AnchorPoint, self_point: AnchorPoint) -> Self {
        Anchor {
            target: AnchorTarget::Window,
            self_point,
            target_point: window_point,
        }
    }

    pub fn in_world(world_point: AnchorPoint, self_point: AnchorPoint) -> Self {
        Anchor {
            target: AnchorTarget::World,
            self_point,
            target_point: world_point,
        }
    }
}

impl Default for Anchor {
    fn default() -> Self {
        Anchor::new(AnchorTarget::Parent { generation_offset: 0 }, AnchorPoint::Center(0), AnchorPoint::Center(0))
    }
}

#[derive(Clone, Copy, Hash, Eq, PartialEq, Debug, Serialize, Deserialize)]
pub enum AnchorTarget {
    Parent { generation_offset: usize },
    Child { child_num: usize },
    Window,
    Display,
    World,
}

#[derive(Clone, Copy, Hash, Eq, PartialEq, Debug, Serialize, Deserialize)]
pub enum AnchorPoint {
    TopLeft(i16),
    TopCenter(i16),
    TopRight(i16),
    BotLeft(i16),
    BotCenter(i16),
    BotRight(i16),
    LeftCenter(i16),
    RightCenter(i16),
    Center(i16),
}

pub trait Positioned {
    fn position(&self) -> OrderedVec2;
}

pub trait Bounded {
    fn bounds(&self) -> OrderedVec2;
}