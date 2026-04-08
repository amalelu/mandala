use crate::core::primitives::{
    Applicable, ApplyOperation, ColorFontRegion, ColorFontRegions, Flag, Range,
};
use crate::font::fonts::AppFont;
use crate::util::color::{Color, FloatRgba};

use crate::gfx_structs::util::regions::{RegionElementKeyPair, RegionIndexer, RegionParams};
use crate::util::geometry;
use crate::util::grapheme_chad::{
    count_grapheme_clusters, count_number_lines, delete_back_unicode, delete_front_unicode,
    find_nth_line_grapheme_range, insert_new_lines, insert_spaces, push_spaces,
    replace_graphemes_until_newline, split_off_graphemes,
};
use crate::util::ordered_vec2::OrderedVec2;
use crossbeam_channel::Sender;
use derivative::Derivative;
use glam::Vec2;
use log::debug;
use ordered_float::OrderedFloat;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::ops::{Add, AddAssign, Index, IndexMut, MulAssign, SubAssign};
use std::sync::Arc;
use strum_macros::{Display, EnumIter};
use crate::gfx_structs::util::hitbox::HitBox;
////////////////////////////////////////
///////////// GLYPH MODEL //////////////
//////////////////////////////////////

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Eq, Hash, EnumIter, Display)]
pub enum GlyphModelFieldType {
    GlyphMatrix,
    GlyphLine,
    GlyphLines,
    Flags,
    Layer,
    Position,
    Operation,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Eq)]
pub enum GlyphModelField {
    GlyphMatrix(GlyphMatrix),
    GlyphLine(usize, GlyphLine),
    GlyphLines(Vec<(usize, GlyphLine)>),
    Layer(usize),
    Position(OrderedVec2),
    Operation(ApplyOperation),
}

impl GlyphModelField {
    pub fn position(x: f32, y: f32) -> Self {
        Self::Position(OrderedVec2::new_f32(x, y))
    }

    pub fn variant(&self) -> GlyphModelFieldType {
        match self {
            GlyphModelField::GlyphMatrix(_) => GlyphModelFieldType::GlyphMatrix,
            GlyphModelField::GlyphLine(_, _) => GlyphModelFieldType::GlyphLine,
            GlyphModelField::Layer(_) => GlyphModelFieldType::Layer,
            GlyphModelField::Position(_) => GlyphModelFieldType::Position,
            GlyphModelField::GlyphLines(_) => GlyphModelFieldType::GlyphLines,
            GlyphModelField::Operation(_) => GlyphModelFieldType::Operation,
        }
    }

    #[inline]
    pub fn same_type(&self, other: &GlyphModelField) -> bool {
        self.variant() == other.variant()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GlyphLine {
    pub line: Vec<GlyphComponent>,
    pub ignore_initial_space: bool,
}

impl Index<usize> for GlyphLine {
    type Output = GlyphComponent;

    fn index(&self, index: usize) -> &Self::Output {
        self.line.get(index).unwrap()
    }
}

impl IndexMut<usize> for GlyphLine {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        self.line.get_mut(index).unwrap()
    }
}

impl SubAssign for GlyphLine {
    fn sub_assign(&mut self, rhs: Self) {
        self.perform_op(&rhs, GlyphLineOp::SubAssign);
    }
}

impl MulAssign for GlyphLine {
    fn mul_assign(&mut self, rhs: Self) {
        self.perform_op(&rhs, GlyphLineOp::MulAssign);
    }
}

impl AddAssign for GlyphLine {
    /// Using [GlyphLineOp::Assign] here intentionally
    fn add_assign(&mut self, rhs: Self) {
        self.perform_op(&rhs, GlyphLineOp::Assign);
    }
}

pub(crate) enum GlyphLineOp {
    Assign,
    AddAssign,
    MulAssign,
    SubAssign,
    Noop,
}

impl GlyphLine {
    pub fn new() -> Self {
        GlyphLine {
            line: vec![],
            ignore_initial_space: false,
        }
    }

    pub fn insert_at_index(&mut self, source: Vec<GlyphComponent>, index: usize) {
        // Ensure the index does not exceed the target's length to prevent panics
        let effective_index = index.min(self.line.len());

        // `splice` takes a range where to splice in the iterator of elements.
        // Since we're inserting at a specific index, the range starts and ends at `index`.
        // The second argument is the source Vec's into_iter(), which takes ownership of its items.
        self.line
            .splice(effective_index..effective_index, source.into_iter());
    }

    pub fn new_with(component: GlyphComponent) -> Self {
        let mut new = GlyphLine::new();
        new.push(component);
        new
    }

    pub fn new_with_vec(comps: Vec<GlyphComponent>, ignore_initial_space: bool) -> Self {
        GlyphLine {
            line: comps,
            ignore_initial_space,
        }
    }

    /// This is mainly for the *Assign impl's
    pub(crate) fn perform_op(&mut self, rhs: &Self, operation: GlyphLineOp) {
        let mut begin_comp: usize = 0;
        if rhs.ignore_initial_space {
            // We want to find the index where we should begin to copy
            for (i, comp) in rhs.line.iter().enumerate() {
                if comp.contains_non_space() {
                    begin_comp = i + 1; // We'll handle this one
                    let comp_index = comp
                        .index_of_first_non_space_char()
                        .expect("We already confirmed that this should exist!");
                    let rhs_comp = rhs.get(i).unwrap();
                    let to_insert = rhs_comp.text.clone().split_off(comp_index);
                    let index_of_comp = rhs.index_of_component(i);
                    let font = rhs_comp.font;
                    let color;

                    match operation {
                        GlyphLineOp::AddAssign => {
                            if i < self.line.len() {
                                color = self.line[i].color + rhs_comp.color;
                            } else {
                                color = rhs_comp.color;
                            }
                        }
                        GlyphLineOp::Assign => {
                            color = rhs_comp.color;
                        }
                        GlyphLineOp::SubAssign => {
                            color = self.line[i].color - rhs_comp.color;
                        }
                        GlyphLineOp::MulAssign => {
                            color = self.line[i].color * rhs_comp.color;
                        }
                        GlyphLineOp::Noop => {
                            continue;
                        }
                    }
                    self.overriding_insert(
                        index_of_comp + comp_index,
                        &GlyphComponent::text(to_insert.as_str(), font, color),
                    );
                    break;
                } else {
                    continue;
                }
            }
        }
        for i in begin_comp..rhs.line.len() {
            if self.line.get(i).is_some() {
                let index_of_comp = self.index_of_component(i);
                self.overriding_insert(index_of_comp, &rhs.get(i).unwrap().clone());
            } else {
                self.line.insert(i, rhs.line[i].clone());
            }
        }
    }

    pub fn push(&mut self, glyph: GlyphComponent) {
        self.line.push(glyph);
    }

    pub fn get(&self, i: usize) -> Option<&GlyphComponent> {
        self.line.get(i)
    }

    pub fn last_component(&self) -> Option<&GlyphComponent> {
        self.line.last()
    }

    pub fn last_comp_mut(&mut self) -> Option<&mut GlyphComponent> {
        self.line.last_mut()
    }

    pub fn component_of_index(&self, index: usize) -> usize {
        let mut head = 0;
        for (i, comp) in self.line.iter().enumerate() {
            if head + comp.length() > index {
                return i;
            } else {
                head += comp.length();
            }
        }
        self.line.len()
    }

    pub fn index_of_component(&self, index: usize) -> usize {
        let mut idx = 0;
        for (i, comp) in self.line.iter().enumerate() {
            if i == index {
                return idx;
            }
            idx += comp.length();
        }
        panic!(
            "Index out of range, component {}, external idx stops at {}",
            index, idx
        );
    }

    pub fn get_mut(&mut self, i: usize) -> Option<&mut GlyphComponent> {
        self.line.get_mut(i)
    }

    #[inline]
    fn seek_comp_begin(
        e_idx_head: usize,
        begin: usize,
        end: usize,
        e_begin_comp: usize,
        comp: &mut GlyphComponent,
        comp_index: usize,
        idx_comp_drain_begin: &mut usize,
        idx_insert: &mut usize,
        should_overwrite: &mut bool,
    ) -> bool {
        let comp_len = comp.length();
        if e_idx_head == begin {
            // This whole comp can be spared
            *idx_comp_drain_begin = comp_index + 2; // next will be used
            *idx_insert = comp_index + 1; // insert into next
            *should_overwrite = false;
            return true;
        } else if e_begin_comp == begin && (end - begin) >= comp_len {
            // This whole comp will be replaced, so we can hijack
            *idx_insert = comp_index;
            *idx_comp_drain_begin = comp_index + 1;
            *should_overwrite = true;
            return true;
        } else if e_begin_comp == begin {
            // We're resizing, but insertion is done in the very front, so we need to shift to the right
            // and the insertion part does not completely override the existing component
            *idx_insert = comp_index;
            *should_overwrite = false;
            *idx_comp_drain_begin = comp_index + 2;
            comp.discard_front(end - begin);
            return true;
        } else if e_idx_head > begin {
            // means we resize, so this one can't be hijacked
            // but that means we can't drain next component either
            // because we need that spot for insertion
            *idx_comp_drain_begin = comp_index + 2;
            *idx_insert = comp_index + 1;
            *should_overwrite = true;
            comp.discard_back(comp_len - (begin - e_begin_comp));
            return true;
        }
        false
    }

    #[inline]
    fn seek_comp_end(
        e_idx_head: usize,
        end: usize,
        e_begin_comp: usize,
        comp: &mut GlyphComponent,
        comp_index: usize,
        idx_comp_drain_end: &mut usize,
    ) -> bool {
        if e_idx_head == end {
            // This whole comp will be overridden
            *idx_comp_drain_end = comp_index + 1;
            return true;
        } else if e_begin_comp == end {
            *idx_comp_drain_end = comp_index;
            return true;
        } else if e_idx_head > end {
            // needs resize, so this shouldn't be overridden, stop the drain before this one
            *idx_comp_drain_end = comp_index;
            comp.discard_front(end - e_begin_comp);
            return true;
        }
        false
    }

    #[inline]
    fn split_and_resize(
        begin: usize,
        end: usize,
        comp_idx: usize,
        comp_begin_e_idx: usize,
        line: &mut Vec<GlyphComponent>,
    ) {
        // Given a component where the insert
        // begins and ends in the middle of it:
        //
        // b = begin_index, e = end_index
        // 1. [..-i..][#############][..+i..]
        // 2. ######b<-new_item->e#######
        // 3. [######][new_item][#######]
        //     ^orig    ^item    ^new
        //
        // 4. [######][new_item][...##]
        //                        ^discard_front(e-b)
        let split_index = begin - comp_begin_e_idx;
        let mut cloned_comp = line
            .get(comp_idx)
            .expect("Yeah we expected this one")
            .clone();
        let split_str = split_off_graphemes(&mut line.get_mut(comp_idx).unwrap().text, split_index);
        cloned_comp.text = split_str;
        cloned_comp.discard_front(end - begin);
        line.insert(comp_idx + 1, cloned_comp);
    }

    pub fn length(&self) -> usize {
        self.line.iter().map(|comp| comp.length()).sum()
    }

    #[inline]
    fn split_component_at(comp_idx: usize, split_at: usize, line: &mut Vec<GlyphComponent>) {
        let split_off_comp = line.get_mut(comp_idx).unwrap().split_off(split_at);
        line.insert(comp_idx + 1, split_off_comp);
    }

    pub fn expanding_insert(&mut self, begin: usize, item: &GlyphComponent) {
        // We have two index types here; component index and "external index"
        // [[A,B,C], [D,E,F], [G,H]]
        //   1,2,3    4,5,6    7,8 <-- e_idx
        //     1        2       3 <-- comp_idx

        if self.length() <= begin {
            let spaces_we_need_to_add = begin - self.length();
            if self.line.len() > 0 {
                self.last_comp_mut()
                    .unwrap()
                    .space_back(spaces_we_need_to_add);
            } else {
                if spaces_we_need_to_add > 0 {
                    self.push(GlyphComponent::space(spaces_we_need_to_add));
                }
            }
            self.push(item.clone());
            return;
        }

        // the external index is our insertion point, which is likely part of a component
        // This component then
        // (a) has to be split, and the new component must be sandwiched between them
        // (b) Or if at the first index of a component, insert at that components index
        // (c) Or if the last index, insert at that index + 1
        let comp_at_insert = self.component_of_index(begin);
        let index_of_comp_at_insert = self.index_of_component(comp_at_insert);
        // check if (b)
        if index_of_comp_at_insert == begin {
            self.line.insert(comp_at_insert, item.clone());
            return;
        }
        let end_index_of_comp_at_insert =
            index_of_comp_at_insert + self.line.get(comp_at_insert).unwrap().length();
        // check if (c)
        if end_index_of_comp_at_insert == begin {
            self.line.insert(comp_at_insert + 1, item.clone());
            return;
        }
        // It must be (a)
        Self::split_component_at(
            comp_at_insert,
            begin - index_of_comp_at_insert,
            &mut self.line,
        );
        self.line.insert(comp_at_insert + 1, item.clone());
    }

    pub fn overriding_insert(&mut self, begin: usize, item: &GlyphComponent) {
        // We have two index types here; component index and "external index"
        // [[A,B,C], [D,E,F], [G,H]]
        //   1,2,3    4,5,6    7,8 <-- e_idx
        //     1        2       3 <-- comp_idx
        let self_len = self.length();
        let item_len = item.length();
        let end = begin + item_len;
        let mut idx_comp_drain_begin: usize = 0;
        let mut needs_comp_begin = true;
        let mut idx_comp_drain_end = self.line.len();
        let mut idx_insert_comp: usize = 0;
        let mut e_idx_head: usize = 0;
        let mut e_begin_comp: usize = 0;
        // In the case where insertion index is at the end, or beyond the end (delta > 0)
        let mut override_at_index: bool = false;
        let mut split_and_adjust: bool = false;
        let to_insert: GlyphComponent;
        let mut delta_head = 0;

        // If the insertion is at the end, the case is simple
        if self_len <= begin {
            delta_head = begin - self_len;
            to_insert = item.clone();
            idx_insert_comp = self.line.len();
        } else {
            to_insert = item.clone();
            // If not then a bit more complex
            for (i, comp) in self.line.iter_mut().enumerate() {
                e_begin_comp = e_idx_head;
                e_idx_head += comp.length();

                if e_idx_head > end && needs_comp_begin && begin > e_begin_comp {
                    // in this case the whole range is within a single component
                    split_and_adjust = true;
                    idx_insert_comp = i + 1;
                    break;
                } else if needs_comp_begin {
                    let found_begin = Self::seek_comp_begin(
                        e_idx_head,
                        begin,
                        end,
                        e_begin_comp,
                        comp,
                        i,
                        &mut idx_comp_drain_begin,
                        &mut idx_insert_comp,
                        &mut override_at_index,
                    );
                    if found_begin {
                        needs_comp_begin = false;
                    }
                } else {
                    let found_end = Self::seek_comp_end(
                        e_idx_head,
                        end,
                        e_begin_comp,
                        comp,
                        i,
                        &mut idx_comp_drain_end,
                    );
                    if found_end {
                        break;
                    }
                }
            }

            if split_and_adjust {
                let split_comp_index = idx_insert_comp - 1;
                Self::split_and_resize(begin, end, split_comp_index, e_begin_comp, &mut self.line);

                self.line.insert(idx_insert_comp, to_insert);
                self.add_space_delta(idx_insert_comp, delta_head);
                return;
            }

            if idx_comp_drain_end > idx_comp_drain_begin {
                let to_drain = idx_comp_drain_end - idx_comp_drain_begin;
                if to_drain > 0 {
                    // remove the overridden ones
                    self.line.drain(idx_comp_drain_begin..idx_comp_drain_end);
                }
            }
        }
        if self.line.get(idx_insert_comp).is_some()
            && idx_comp_drain_end > idx_insert_comp
            && override_at_index
        {
            self.line[idx_insert_comp] = to_insert;
        } else {
            self.line.insert(idx_insert_comp, to_insert);
        }
        self.add_space_delta(idx_insert_comp, delta_head);
    }

    #[inline]
    fn add_space_delta(&mut self, idx_insert_comp: usize, delta_head: usize) {
        if delta_head > 0 {
            // We need to check if the previous component is also just space
            if idx_insert_comp > 0 {
                let previous = self
                    .line
                    .get_mut(idx_insert_comp - 1)
                    .expect("No previous component exists, this is an invalid state");
                if !previous.contains_non_space() {
                    // This is all space alright
                    previous.space_back(delta_head);
                    return;
                }
            }
            self.line
                .insert(idx_insert_comp, GlyphComponent::space(delta_head));
        }
    }
}

impl Default for GlyphLine {
    fn default() -> Self {
        GlyphLine::new()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GlyphMatrix {
    pub matrix: Vec<GlyphLine>,
}

impl Index<usize> for GlyphMatrix {
    type Output = GlyphLine;

    fn index(&self, index: usize) -> &Self::Output {
        self.matrix.get(index).unwrap()
    }
}

impl IndexMut<usize> for GlyphMatrix {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        let matrix = &mut self.matrix;
        let matrix_len = matrix.len();

        if matrix.get_mut(index).is_some() {
            return matrix.get_mut(index).unwrap();
        }
        if index > matrix_len {
            let matrix_delta = index - matrix_len;
            for _ in 0..matrix_delta {
                matrix.push(GlyphLine::new());
            }
        }
        matrix.push(GlyphLine::new());
        matrix.get_mut(index).unwrap()
    }
}

impl SubAssign for GlyphMatrix {
    fn sub_assign(&mut self, rhs: Self) {
        for (i, line) in (&rhs.matrix).iter().enumerate() {
            debug!("Looking at rhs line {}", i);
            if self.get(i).is_none() {
                // There's no way we can have negative glyphs, for now at least.
                // I suppose it depends on what we want a subtraction operation to
                // mean on a GlyphMatrix
                debug!("line {} does not exist in self, so falls away", i);
                continue;
            } else {
                self.get_mut(i).unwrap().sub_assign(line.clone())
            }
        }
    }
}

impl MulAssign for GlyphMatrix {
    fn mul_assign(&mut self, rhs: Self) {
        // wtf does it mean to multiply two glyphmatrices
        // Well let's see
        for (i, line) in (&rhs.matrix).iter().enumerate() {
            debug!("Looking at rhs line {}", i);
            if self.get(i).is_none() {
                // Don't copy over lines that don't exist in self
                // because that would be considered multiplication by 0
                debug!("line {} does not exist in self, so falls away", i);
                continue;
            } else {
                self.get_mut(i).unwrap().mul_assign(line.clone())
            }
        }
    }
}

impl AddAssign for GlyphMatrix {
    fn add_assign(&mut self, rhs: Self) {
        // rhs might be dimensionally bigger than self.
        // This will be an overriding add_assign
        for (i, line) in (&rhs.matrix).iter().enumerate() {
            debug!("Looking at rhs line {}", i);
            if self.get(i).is_none() {
                self.matrix.insert(i, line.clone());
                debug!("Cloned line {} from rhs into self", i);
                continue;
            } else {
                self.get_mut(i).unwrap().add_assign(line.clone())
            }
        }
    }
}

impl GlyphMatrix {
    pub fn new() -> Self {
        GlyphMatrix { matrix: vec![] }
    }
    pub fn push(&mut self, line: GlyphLine) {
        self.matrix.push(line);
    }

    pub fn get(&self, line_num: usize) -> Option<&GlyphLine> {
        self.matrix.get(line_num)
    }

    pub fn get_mut(&mut self, line_num: usize) -> Option<&mut GlyphLine> {
        self.matrix.get_mut(line_num)
    }

    pub fn expanding_insert(&mut self, line_num: usize, idx: usize, component: &GlyphComponent) {
        self.expand_to_line(line_num, idx);
        self.matrix[line_num].expanding_insert(idx, component);
    }

    pub fn overriding_insert(&mut self, line_num: usize, idx: usize, component: &GlyphComponent) {
        self.expand_to_line(line_num, idx);
        self.matrix[line_num].overriding_insert(idx, component);
    }

    pub fn place_in(
        &self,
        string: &mut String,
        regions: &mut ColorFontRegions,
        offset: (usize, usize),
    ) {
        // Ensure that there's enough lines present in the string
        let num_lines = count_number_lines(&string);
        let needed_lines = self.matrix.len() + offset.1;

        if needed_lines > num_lines {
            insert_new_lines(string, needed_lines - num_lines);
        }

        for (line_num, line) in self.matrix.iter().enumerate() {
            let graph_line_start_index: usize;
            {
                // If there's an x-offset, then we also need to ensure that each line is at least the length of that;
                let target_line_grapheme_range =
                    find_nth_line_grapheme_range(string, line_num + offset.1);
                if let Some(line_graph_range) = target_line_grapheme_range {
                    let target_line_len = line_graph_range.1 - line_graph_range.0;
                    graph_line_start_index = line_graph_range.0;
                    if target_line_len < offset.0 {
                        insert_spaces(string, line_graph_range.1, offset.0 - target_line_len);
                    }
                } else {
                    // Important that this is done before pushing spaces
                    graph_line_start_index = count_grapheme_clusters(&string);
                    push_spaces(string, offset.0);
                }
            }

            // Then copy the contents of each line into the corresponding line in the target String
            // todo: It would be nice if the following logic also applied, but right now it does not:
            // if the source is space, and the target is non-space, then let it remain unchanged
            // Otherwise prioritize the one being copied over
            let mut comp_head = graph_line_start_index + offset.0;
            for component in line.line.iter() {
                let region_shift =
                    replace_graphemes_until_newline(string, comp_head, &component.text);
                if let Some(t) = region_shift {
                    regions.shift_regions_after(t.0, t.1);
                }
                regions.submit_region(ColorFontRegion::new(
                    Range::new(comp_head, comp_head + component.length()),
                    Some(component.font),
                    Some(component.color.to_float()),
                ));
                comp_head += &component.length();
            }
        }
    }

    fn expand_to_line(&mut self, line_num: usize, idx: usize) {
        let matrix_len = self.matrix.len();

        if matrix_len <= line_num {
            let line_delta = line_num - matrix_len;
            for _ in 0..line_delta {
                self.matrix.push(GlyphLine::new());
            }
            let line: GlyphLine;
            if idx > 0 {
                line = GlyphLine::new_with(GlyphComponent::space(idx));
            } else {
                line = GlyphLine::new();
            }
            self.matrix.push(line);
        }
    }
}

impl Default for GlyphMatrix {
    fn default() -> Self {
        GlyphMatrix::new()
    }
}

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
    #[serde(skip)]
    #[derivative(PartialEq = "ignore")]
    screen_region_params: Option<Arc<RegionParams>>,
    #[serde(skip)]
    #[derivative(PartialEq = "ignore")]
    tree_index: Option<Arc<RegionIndexer>>,
}

impl GlyphModel {
    /// Creates a new [GlyphModel] with an empty Vec and layer set to 0, at (0,0)
    pub fn new() -> Self {
        GlyphModel {
            glyph_matrix: GlyphMatrix::default(),
            layer: 0,
            position: OrderedVec2::new_f32(0.0, 0.0),
            hitbox: HitBox::new(),
            screen_region_params: None,
            tree_index: None,
        }
    }

    pub fn update_index_if_relevant(&mut self) {
        if self.tree_index.is_some() && self.screen_region_params.is_some() {
            // TODO update ..
            //self.tree_index.unwrap()
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
        //update_index_if_relevant??
    }

    pub fn nudge_left(&mut self, amount: &f32) {
        self.position.x -= amount;
        self.update_index_if_relevant();
    }

    pub fn nudge_right(&mut self, amount: &f32) {
        self.position.x += amount;
        self.update_index_if_relevant();
    }

    pub fn nudge_up(&mut self, amount: &f32) {
        self.position.y -= amount;
        self.update_index_if_relevant();
    }

    pub fn nudge_down(&mut self, amount: &f32) {
        self.position.y += amount;
        self.update_index_if_relevant();
    }

    pub fn move_to(&mut self, x: &f32, y: &f32) {
        self.position.x = OrderedFloat::from(*x);
        self.position.y = OrderedFloat::from(*y);
        self.update_index_if_relevant();
    }

    pub fn rotate(&mut self, pivot: &Vec2, degrees: &f32) {
        let new_position =
            geometry::clockwise_rotation_around_pivot(self.position.to_vec2(), *pivot, *degrees);
        self.position = OrderedVec2::from_vec2(new_position);
        self.update_index_if_relevant();
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

    fn apply_operation(&mut self, delta: &DeltaGlyphModel) {
        let mut should_update_index = false;
        let operation = delta.operation_variant();
        if let Some(position_delta) = delta.position() {
            operation.apply(&mut self.position.x, position_delta.x);
            operation.apply(&mut self.position.y, position_delta.y);
            should_update_index = true;
        }

        if let Some(delta_layer) = delta.layer() {
            operation.apply(&mut self.layer, delta_layer);
            // We should be using layers in the indices somehow
            should_update_index = true;
        }

        if let Some(glyph_matrix) = delta.glyph_matrix() {
            operation.apply(&mut self.glyph_matrix, glyph_matrix);
            //should or should not update? Probably should
        }
        if should_update_index {
            self.update_index_if_relevant();
        }
    }
}

/// Each variant represents a field in GlyphComponent, used for mutators
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum GlyphComponentField {
    Text(String),
    Font(AppFont),
    Color(FloatRgba),
}

#[derive(Serialize, Debug, Eq, PartialEq, Deserialize, Clone)]
pub struct GlyphComponent {
    pub text: String,
    pub font: AppFont,
    pub color: Color,
}

impl Hash for GlyphComponent {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);
        self.font.hash(state);
        self.color.rgba.hash(state);
    }
}

impl MulAssign for GlyphComponent {
    fn mul_assign(&mut self, rhs: Self) {
        if !rhs.text.is_empty() {
            self.text = rhs.text.clone();
        }
        if rhs.font != AppFont::Any {
            self.font = rhs.font;
        }
        // Colors
        let mut result = self.color[0].wrapping_mul(rhs.color[0]);
        self.color[0] = result;
        result = self.color[1].wrapping_mul(rhs.color[1]);
        self.color[1] = result;
        result = self.color[2].wrapping_mul(rhs.color[2]);
        self.color[2] = result;
        result = self.color[3].wrapping_mul(rhs.color[3]);
        self.color[3] = result;
    }
}

impl AddAssign for GlyphComponent {
    fn add_assign(&mut self, rhs: Self) {
        if !rhs.text.is_empty() {
            self.text = self.text.clone() + &*rhs.text;
        }
        if self.font == AppFont::Any {
            self.font = rhs.font;
        }
        let mut result = self.color[0].wrapping_add(rhs.color[0]);
        self.color[0] = result;
        result = self.color[1].wrapping_add(rhs.color[1]);
        self.color[1] = result;
        result = self.color[2].wrapping_add(rhs.color[2]);
        self.color[2] = result;
        result = self.color[3].wrapping_add(rhs.color[3]);
        self.color[3] = result;
    }
}

impl Add for GlyphComponent {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        let mut output = self.clone();
        output += rhs;
        output
    }
}

impl GlyphComponent {
    pub fn text(text: &str, font: AppFont, color: Color) -> Self {
        GlyphComponent {
            text: text.to_string(),
            font,
            color,
        }
    }

    pub fn space(n: usize) -> Self {
        GlyphComponent {
            text: " ".repeat(n),
            font: AppFont::Any,
            color: Color::invisible(),
        }
    }

    pub fn split_off(&mut self, at_idx: usize) -> Self {
        let split_str = split_off_graphemes(&mut self.text, at_idx);
        GlyphComponent {
            text: split_str,
            font: self.font,
            color: self.color,
        }
    }

    pub fn space_front(&mut self, n: usize) {
        self.pad_front(" ", n);
    }

    pub fn space_back(&mut self, n: usize) {
        self.pad_back(" ", n);
    }

    pub fn pad_front(&mut self, pad: &str, n: usize) {
        let padding = pad.repeat(n);
        self.text.insert_str(0, &padding);
    }

    pub fn pad_back(&mut self, pad: &str, n: usize) {
        let padding = pad.repeat(n);
        self.text.push_str(&padding);
    }

    pub fn contains_non_space(&self) -> bool {
        self.text.chars().any(|c| !c.is_whitespace())
    }

    pub fn index_of_first_non_space_char(&self) -> Option<usize> {
        self.text
            .chars()
            .enumerate()
            .find(|&(_, c)| !c.is_whitespace())
            .map(|(i, _)| i)
    }

    pub fn as_str(&self) -> &str {
        self.text.as_str()
    }

    /// Returns number of grapheme clusters
    pub fn length(&self) -> usize {
        count_grapheme_clusters(&self.text)
    }

    /// This works on unicode grapheme clusters
    pub fn discard_front(&mut self, num: usize) {
        delete_front_unicode(&mut self.text, num);
    }

    /// This works on unicode grapheme clusters
    pub fn discard_back(&mut self, num: usize) {
        delete_back_unicode(&mut self.text, num);
    }
}

////////////////////////////////////////
/////// DeltaGlyphModel Mutator ///////
//////////////////////////////////////

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeltaGlyphModel {
    pub fields: FxHashMap<GlyphModelFieldType, GlyphModelField>,
}

impl DeltaGlyphModel {
    pub fn new(fields: Vec<GlyphModelField>) -> DeltaGlyphModel {
        let mut field_map = FxHashMap::default();
        for field in fields {
            field_map.insert(field.variant(), field.clone());
        }
        DeltaGlyphModel { fields: field_map }
    }

    pub fn glyph_matrix(&self) -> Option<GlyphMatrix> {
        if let Some(GlyphModelField::GlyphMatrix(matrix)) =
            self.fields.get(&GlyphModelFieldType::GlyphMatrix)
        {
            Some(matrix.clone())
        } else {
            None
        }
    }

    pub fn layer(&self) -> Option<usize> {
        if let Some(GlyphModelField::Layer(layer)) = self.fields.get(&GlyphModelFieldType::Layer) {
            Some(*layer)
        } else {
            None
        }
    }

    pub fn glyph_line(&self) -> Option<(usize, GlyphLine)> {
        if let Some(GlyphModelField::GlyphLine(line_num, glyph_line)) =
            self.fields.get(&GlyphModelFieldType::GlyphLine)
        {
            Some((*line_num, glyph_line.clone()))
        } else {
            None
        }
    }

    pub fn glyph_lines(&self) -> Option<Vec<(usize, GlyphLine)>> {
        if let Some(GlyphModelField::GlyphLines(lines)) =
            self.fields.get(&GlyphModelFieldType::GlyphLines)
        {
            Some(lines.clone())
        } else {
            None
        }
    }

    pub fn position(&self) -> Option<OrderedVec2> {
        if let Some(GlyphModelField::Position(vec)) =
            self.fields.get(&GlyphModelFieldType::Position)
        {
            Some(*vec)
        } else {
            None
        }
    }

    pub fn operation_variant(&self) -> ApplyOperation {
        if let Some(GlyphModelField::Operation(operation)) =
            self.fields.get(&GlyphModelFieldType::Operation)
        {
            *operation
        } else {
            ApplyOperation::Noop
        }
    }
}

impl Applicable<GlyphModel> for DeltaGlyphModel {
    fn apply_to(&self, target: &mut GlyphModel) {
        target.apply_operation(self);
    }
}

////////////////////////////////////////
////// GlyphModelCommand Mutator //////
//////////////////////////////////////

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize, Eq, Hash, EnumIter, Display)]
pub enum GlyphModelCommandType {
    NudgeLeft,
    NudgeRight,
    NudgeDown,
    NudgeUp,
    MoveTo,
    SetFlag,
    Rotate,
    RudeInsert,
    PoliteInsert,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum GlyphModelCommand {
    NudgeLeft(f32),
    NudgeRight(f32),
    NudgeDown(f32),
    NudgeUp(f32),
    MoveTo(f32, f32),
    Rotate {
        pivot: Vec2,
        degrees: f32,
    },
    /// Inserts and overrides existing characters
    RudeInsert {
        component: GlyphComponent,
        line_num: usize,
        at_idx: usize,
    },
    /// Inserts and pushes existing characters forward
    PoliteInsert {
        component: GlyphComponent,
        line_num: usize,
        at_idx: usize,
    },
}

impl GlyphModelCommand {
    pub fn variant(&self) -> GlyphModelCommandType {
        match self {
            GlyphModelCommand::NudgeLeft(_) => GlyphModelCommandType::NudgeLeft,
            GlyphModelCommand::NudgeRight(_) => GlyphModelCommandType::NudgeRight,
            GlyphModelCommand::NudgeDown(_) => GlyphModelCommandType::NudgeDown,
            GlyphModelCommand::NudgeUp(_) => GlyphModelCommandType::NudgeUp,
            GlyphModelCommand::MoveTo(_, _) => GlyphModelCommandType::MoveTo,
            GlyphModelCommand::Rotate { .. } => GlyphModelCommandType::Rotate,
            GlyphModelCommand::RudeInsert { .. } => GlyphModelCommandType::RudeInsert,
            GlyphModelCommand::PoliteInsert { .. } => GlyphModelCommandType::PoliteInsert,
        }
    }

    #[inline]
    pub fn same_type(&self, other: &Self) -> bool {
        self.variant() == other.variant()
    }
}

impl Applicable<GlyphModel> for GlyphModelCommand {
    fn apply_to(&self, target: &mut GlyphModel) {
        match self {
            GlyphModelCommand::NudgeLeft(amount) => target.nudge_left(amount),
            GlyphModelCommand::NudgeRight(amount) => target.nudge_right(amount),
            GlyphModelCommand::NudgeDown(amount) => target.nudge_down(amount),
            GlyphModelCommand::NudgeUp(amount) => target.nudge_up(amount),
            GlyphModelCommand::MoveTo(x, y) => target.move_to(x, y),
            GlyphModelCommand::Rotate { pivot, degrees } => target.rotate(pivot, degrees),
            GlyphModelCommand::RudeInsert {
                component,
                line_num,
                at_idx,
            } => target.rude_insert(component, line_num, at_idx),
            GlyphModelCommand::PoliteInsert {
                component,
                line_num,
                at_idx,
            } => {
                target.expanding_insert(component, line_num, at_idx);
            }
        }
    }
}
