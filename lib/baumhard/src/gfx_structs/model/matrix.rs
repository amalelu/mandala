//! `GlyphMatrix` — a column-of-[`GlyphLine`]s wrapper. The `place_in`
//! method paints the matrix onto a target `String` + `ColorFontRegions`
//! at an offset, the workhorse of the scene-builder's glyph placement
//! path.

use super::component::GlyphComponent;
use super::line::GlyphLine;
use crate::core::primitives::{ColorFontRegion, ColorFontRegions, Range};
use crate::util::grapheme_chad::{
    count_grapheme_clusters, count_number_lines, find_nth_line_grapheme_range, insert_new_lines,
    insert_spaces, push_spaces, replace_graphemes_until_newline,
};
use log::debug;
use serde::{Deserialize, Serialize};
use std::ops::{AddAssign, Index, IndexMut, MulAssign, SubAssign};

/// Stacked collection of [`GlyphLine`]s rendered top-to-bottom.
/// Wraps a single `Vec<GlyphLine>`; indexing past the end of the
/// matrix auto-expands with empty lines, so callers can write into
/// arbitrary coordinates without pre-sizing.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GlyphMatrix {
    /// Ordered lines. Index 0 is the top-most visual line.
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
    /// Empty matrix. O(1), no allocation.
    pub fn new() -> Self {
        GlyphMatrix { matrix: vec![] }
    }
    /// Append a line. O(1) amortised.
    pub fn push(&mut self, line: GlyphLine) {
        self.matrix.push(line);
    }

    /// Borrow the line at `line_num`. O(1).
    pub fn get(&self, line_num: usize) -> Option<&GlyphLine> {
        self.matrix.get(line_num)
    }

    /// Mutable borrow of the line at `line_num`. O(1).
    pub fn get_mut(&mut self, line_num: usize) -> Option<&mut GlyphLine> {
        self.matrix.get_mut(line_num)
    }

    /// Expanding-insert at `(line_num, idx)`: grows rows / columns
    /// with blank space as needed, then delegates to
    /// [`GlyphLine::expanding_insert`]. O(line growth + grapheme
    /// walk).
    pub fn expanding_insert(&mut self, line_num: usize, idx: usize, component: &GlyphComponent) {
        self.expand_to_line(line_num, idx);
        self.matrix[line_num].expanding_insert(idx, component);
    }

    /// Overriding-insert at `(line_num, idx)`: grows rows / columns
    /// with blank space as needed, then delegates to
    /// [`GlyphLine::overriding_insert`]. O(line growth + grapheme
    /// walk).
    pub fn overriding_insert(&mut self, line_num: usize, idx: usize, component: &GlyphComponent) {
        self.expand_to_line(line_num, idx);
        self.matrix[line_num].overriding_insert(idx, component);
    }

    /// Paint this matrix into the caller-owned `string` +
    /// `regions` pair, offset by `(cols, rows)` graphemes.
    /// The target `string` is padded with newlines / spaces as
    /// needed so every source component lands on the intended
    /// grapheme cell. `regions` gains one `ColorFontRegion` per
    /// painted component so the renderer can colour/fontify the
    /// right spans.
    ///
    /// # Costs
    ///
    /// O(total painted graphemes + existing text size) — the walk
    /// over source components is linear, but each
    /// `replace_graphemes_until_newline` call is O(line length).
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

            // Copy each component into the target line. Source
            // always wins the cell — a future refinement where
            // whitespace source preserves non-whitespace target is
            // not yet implemented.
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
