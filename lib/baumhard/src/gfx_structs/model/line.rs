// SPDX-License-Identifier: MPL-2.0

//! `GlyphLine` — a horizontal run of [`GlyphComponent`]s forming one
//! visual line in a [`crate::gfx_structs::model::GlyphMatrix`]. Carries
//! the `overriding_insert` / `expanding_insert` logic used by the
//! in-place glyph-matrix mutation paths.

use super::component::GlyphComponent;
use crate::util::grapheme_chad::split_off_graphemes;
use serde::{Deserialize, Serialize};
use std::ops::{AddAssign, Index, IndexMut, MulAssign, SubAssign};

/// One visual line in a [`crate::gfx_structs::model::GlyphMatrix`] —
/// a vector of [`GlyphComponent`] runs. `ignore_initial_space`
/// controls how `*Assign` operators treat leading whitespace in the
/// rhs during matrix composition.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlyphLine {
    /// Ordered list of colored/fonted text runs that together form
    /// the line.
    pub line: Vec<GlyphComponent>,
    /// When `true`, leading whitespace in the rhs of an `*Assign` op
    /// is transparent instead of opaque: the all-whitespace runs in
    /// front are skipped, and the first run carrying content paints at
    /// its own grapheme offset (indent counted, indent not written).
    /// An rhs that is *entirely* whitespace therefore paints nothing.
    ///
    /// The flag rides on the wire — `GlyphLine` is a
    /// `ModelDelta`/`GlyphMatrix` payload, so a hand-authored
    /// `.mindmap.json` mutation can set it (see `format/mutations.md`).
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
    /// Using `GlyphLineOp::Assign` here intentionally
    fn add_assign(&mut self, rhs: Self) {
        self.perform_op(&rhs, GlyphLineOp::Assign);
    }
}

/// How [`GlyphLine::perform_op`] combines the *color* of a leading
/// rhs run with the lhs run it lands on. Every arm here is
/// constructed by one of the three `*Assign` impls above; there is no
/// fourth. An `AddAssign` arm existed once and was unreachable —
/// `AddAssign for GlyphLine` deliberately assigns rather than adds —
/// as did a `Noop` arm, which `ApplyOperation::apply_ref`
/// short-circuits before it can ever reach a line. Both were deleted
/// per CODE_CONVENTIONS.md §5 (no dead code) and §10 (delete rather
/// than deprecate); neither is `pub`, so nothing outside the crate
/// could name them.
pub(crate) enum GlyphLineOp {
    /// The rhs color replaces the lhs color outright. Used by both
    /// `Assign`-style composition and `AddAssign for GlyphLine`.
    Assign,
    /// The two colors multiply per channel.
    MulAssign,
    /// The rhs color is subtracted per channel from the lhs color.
    SubAssign,
}

impl GlyphLine {
    /// Empty line. O(1).
    pub fn new() -> Self {
        GlyphLine {
            line: vec![],
            ignore_initial_space: false,
        }
    }

    /// Splice `source` into the component vector at `index`,
    /// clamped to the line's current length. O(source.len() +
    /// line.len() - index) for the shift.
    pub fn insert_at_index(&mut self, source: Vec<GlyphComponent>, index: usize) {
        // Ensure the index does not exceed the target's length to prevent panics
        let effective_index = index.min(self.line.len());

        // `splice` takes a range where to splice in the iterator of elements.
        // Since we're inserting at a specific index, the range starts and ends at `index`.
        // The second argument is the source Vec's into_iter(), which takes ownership of its items.
        self.line.splice(effective_index..effective_index, source);
    }

    /// Line containing one component. O(1).
    pub fn new_with(component: GlyphComponent) -> Self {
        let mut new = GlyphLine::new();
        new.push(component);
        new
    }

    /// Line wrapping a pre-built component vector plus the
    /// `ignore_initial_space` flag. O(1); the vector is moved, not
    /// cloned.
    pub fn new_with_vec(comps: Vec<GlyphComponent>, ignore_initial_space: bool) -> Self {
        GlyphLine {
            line: comps,
            ignore_initial_space,
        }
    }

    /// Composition workhorse behind the three `*Assign` impls: paint
    /// every run of `rhs` onto `self` at **the grapheme offset that
    /// run occupies inside `rhs`**, with `operation` deciding how the
    /// two colors combine.
    ///
    /// That offset — the rhs's own column, not the lhs's run ordinal
    /// — is the whole contract. `self` and `rhs` never have to agree
    /// on where their run boundaries fall, and after the first paint
    /// they generally do not: `overriding_insert` splits and merges
    /// `self`'s runs as it goes. Indexing `self` by an `rhs` ordinal
    /// therefore drifts a little further with each run painted, which
    /// scrambled the order of any surplus runs and mislaid them by
    /// however much the partitions had diverged.
    ///
    /// When `rhs.ignore_initial_space` is set, `rhs`'s leading
    /// whitespace is transparent rather than opaque: the all-space
    /// runs in front are skipped entirely, and the first run carrying
    /// content is painted at its own offset with its indent counted
    /// in but not written, so the rhs shows through onto the lhs
    /// instead of blanking it. Runs after that one paint at their own
    /// offsets in the same way.
    ///
    /// Every offset here is a grapheme-cluster count:
    /// [`GlyphComponent::index_of_first_non_space_grapheme`],
    /// [`Self::index_of_component`], [`GlyphComponent::length`], and
    /// [`split_off_graphemes`] all speak that one unit. The path used
    /// to mix three — a `char` ordinal, fed to a byte-indexed
    /// `String::split_off` (a panic on any multi-byte leading space
    /// such as U+3000), then reused as a cluster offset (CONVENTIONS
    /// §B3).
    ///
    /// Nothing here can panic on a shape mismatch: every `self`
    /// access is guarded, and painting past the end of `self` is
    /// `overriding_insert`'s whitespace-padding case rather than an
    /// out-of-bounds insert.
    ///
    /// Costs: O(rhs runs), each doing one `overriding_insert` — an
    /// O(line length) grapheme walk plus a splice — over a running
    /// offset that costs one grapheme walk per run. Clones the text
    /// of each painted run once.
    pub(crate) fn perform_op(&mut self, rhs: &Self, operation: GlyphLineOp) {
        let mut begin_comp: usize = 0;
        if rhs.ignore_initial_space {
            // An rhs that is nothing but indent paints nothing at all:
            // the flag's contract is that leading whitespace is
            // transparent, and every run of an all-whitespace rhs is
            // leading whitespace. The loop below raises this back to
            // `i + 1` as soon as it finds a run with content.
            begin_comp = rhs.line.len();
            for (i, rhs_comp) in rhs.line.iter().enumerate() {
                // All-whitespace runs in front of the content paint
                // nothing at all.
                let Some(split_at) = rhs_comp.index_of_first_non_space_grapheme() else {
                    continue;
                };
                begin_comp = i + 1;

                let mut content = rhs_comp.text.clone();
                let to_insert = split_off_graphemes(&mut content, split_at);

                let lhs_color = self.line.get(i).map(|comp| comp.color);
                let color = match operation {
                    GlyphLineOp::Assign => rhs_comp.color,
                    GlyphLineOp::SubAssign => lhs_color.map_or(rhs_comp.color, |lhs| lhs - rhs_comp.color),
                    GlyphLineOp::MulAssign => lhs_color.map_or(rhs_comp.color, |lhs| lhs * rhs_comp.color),
                };

                self.overriding_insert(
                    rhs.index_of_component(i) + split_at,
                    &GlyphComponent::text(to_insert.as_str(), rhs_comp.font, color),
                );
                break;
            }
        }
        // Where run `begin_comp` starts inside `rhs`. Skipped leading
        // whitespace still occupies columns, so it counts toward the
        // offset even though it painted nothing.
        let mut rhs_offset: usize = rhs.line.iter().take(begin_comp).map(GlyphComponent::length).sum();
        for run in rhs.line.iter().skip(begin_comp) {
            // `overriding_insert` clones the component itself, so
            // cloning here too was a second copy of the run (§B7). It
            // also pads with whitespace when `rhs_offset` is past the
            // end of `self`, which is why a shorter lhs needs no
            // special case — and cannot panic the way the old
            // `Vec::insert(i, …)` did.
            self.overriding_insert(rhs_offset, run);
            rhs_offset += run.length();
        }
    }

    /// Append a component to the end. O(1) amortized.
    pub fn push(&mut self, glyph: GlyphComponent) {
        self.line.push(glyph);
    }

    /// Borrow the component at position `i`. O(1).
    pub fn get(&self, i: usize) -> Option<&GlyphComponent> {
        self.line.get(i)
    }

    /// Borrow the last component. O(1).
    pub fn last_component(&self) -> Option<&GlyphComponent> {
        self.line.last()
    }

    /// Mutable borrow of the last component. O(1).
    pub fn last_comp_mut(&mut self) -> Option<&mut GlyphComponent> {
        self.line.last_mut()
    }

    /// Component index that contains grapheme position `index`, or
    /// `line.len()` when `index` is past the last component.
    /// O(n) grapheme walk over components.
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

    /// Grapheme-index where component `index` begins. O(n) grapheme
    /// walk. Panics when `index >= line.len()`.
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

    /// Mutable component borrow at position `i`. O(1).
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
            // This whole comp can be spared: the insert starts exactly
            // on the boundary after it, so it goes into the next slot.
            //
            // `should_overwrite` is `true` because whether that next
            // slot is *consumed* by the insert is not knowable from
            // here — it depends on the item's length against a
            // component this call has not seen. So defer to the drain
            // window: the caller only honors the flag when
            // `idx_comp_drain_end > idx_insert`, which is precisely
            // "the run in the insertion slot was fully overridden".
            // Hard-coding `false` made the result depend on how the
            // line happened to be cut into runs — the same insert over
            // the same text overwrote a character when the line was one
            // run, and grew the line by one when a run boundary fell at
            // the insertion point.
            *idx_comp_drain_begin = comp_index + 2; // next will be used
            *idx_insert = comp_index + 1; // insert into next
            *should_overwrite = true;
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
        } else if e_begin_comp >= end {
            // This comp starts at or after the end of the insert, so
            // none of it is overridden and none of it is trimmed —
            // the drain stops before it.
            //
            // The `>` half of this used to fall through to the
            // `discard_front` branch below and underflow
            // `end - e_begin_comp`. It is reachable whenever the
            // insert is shorter than the run it starts on and another
            // run follows — `overriding_insert(0, one_cluster)` on any
            // multi-run line, which `GlyphModelCommand::RudeInsert`
            // hands straight to a `.mindmap.json` author.
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
        let mut cloned_comp = line.get(comp_idx).expect("Yeah we expected this one").clone();
        let split_str = split_off_graphemes(&mut line.get_mut(comp_idx).unwrap().text, split_index);
        cloned_comp.text = split_str;
        cloned_comp.discard_front(end - begin);
        line.insert(comp_idx + 1, cloned_comp);
    }

    /// Total grapheme count across every component. O(sum of
    /// component grapheme walks).
    pub fn length(&self) -> usize {
        self.line.iter().map(|comp| comp.length()).sum()
    }

    #[inline]
    fn split_component_at(comp_idx: usize, split_at: usize, line: &mut Vec<GlyphComponent>) {
        let split_off_comp = line.get_mut(comp_idx).unwrap().split_off(split_at);
        line.insert(comp_idx + 1, split_off_comp);
    }

    /// Insert `item` at grapheme position `begin`, pushing existing
    /// content to the right. Pads with whitespace when `begin`
    /// exceeds the current line length. O(n) grapheme walk + O(n)
    /// splice.
    pub fn expanding_insert(&mut self, begin: usize, item: &GlyphComponent) {
        // We have two index types here; component index and "external index"
        // [[A,B,C], [D,E,F], [G,H]]
        //   1,2,3    4,5,6    7,8 <-- e_idx
        //     1        2       3 <-- comp_idx

        if self.length() <= begin {
            let spaces_we_need_to_add = begin - self.length();
            if !self.line.is_empty() {
                self.last_comp_mut().unwrap().space_back(spaces_we_need_to_add);
            } else if spaces_we_need_to_add > 0 {
                self.push(GlyphComponent::space(spaces_we_need_to_add));
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
        Self::split_component_at(comp_at_insert, begin - index_of_comp_at_insert, &mut self.line);
        self.line.insert(comp_at_insert + 1, item.clone());
    }

    /// Insert `item` at grapheme position `begin`, overwriting the
    /// `item.length()` graphemes that already occupied that range.
    /// Pads with whitespace when `begin` exceeds the current line
    /// length. O(n) grapheme walk + O(n) splice/drain.
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
                    let found_end =
                        Self::seek_comp_end(e_idx_head, end, e_begin_comp, comp, i, &mut idx_comp_drain_end);
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
