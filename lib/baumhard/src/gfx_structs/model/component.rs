//! `GlyphComponent` + `GlyphComponentField` — the leaf of the glyph
//! model hierarchy: one contiguous run of text sharing a font and a
//! colour. A `GlyphLine` is a `Vec<GlyphComponent>`; a `GlyphMatrix`
//! is a `Vec<GlyphLine>`; a `GlyphModel` wraps the matrix.

use crate::font::fonts::AppFont;
use crate::util::color::{Color, FloatRgba};
use crate::util::grapheme_chad::{
    count_grapheme_clusters, delete_back_unicode, delete_front_unicode, split_off_graphemes,
};
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::ops::{Add, AddAssign, MulAssign};

/// Field-level delta vocabulary for [`GlyphComponent`]. Each variant
/// carries the replacement (or addend) for one field; the mutator
/// pipeline picks the variant to know *which* part to touch.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum GlyphComponentField {
    /// New text run.
    Text(String),
    /// Font assignment.
    Font(AppFont),
    /// Colour assignment.
    Color(FloatRgba),
}

/// The leaf: one run of text rendered in a single font and colour.
/// Stacks into a [`crate::gfx_structs::model::GlyphLine`], which
/// stacks into a [`crate::gfx_structs::model::GlyphMatrix`], which
/// belongs to a [`crate::gfx_structs::model::GlyphModel`].
#[derive(Serialize, Debug, Eq, PartialEq, Deserialize, Clone)]
pub struct GlyphComponent {
    /// The text run — may contain multi-byte / multi-grapheme clusters.
    pub text: String,
    /// Font used for this run.
    pub font: AppFont,
    /// RGBA colour (u8 per channel).
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
    /// Build a component from `(text, font, color)`. O(n) in
    /// `text.len()` for the owning copy.
    pub fn text(text: &str, font: AppFont, color: Color) -> Self {
        GlyphComponent {
            text: text.to_string(),
            font,
            color,
        }
    }

    /// Build an invisible-colour spacer of `n` ASCII spaces. Used by
    /// the matrix painter to pad lines. O(n) for the repeat.
    pub fn space(n: usize) -> Self {
        GlyphComponent {
            text: " ".repeat(n),
            font: AppFont::Any,
            color: Color::invisible(),
        }
    }

    /// Split off the graphemes at-and-after `at_idx` into a new
    /// component (inheriting this component's font / colour). O(n)
    /// in `at_idx` for the grapheme walk.
    pub fn split_off(&mut self, at_idx: usize) -> Self {
        let split_str = split_off_graphemes(&mut self.text, at_idx);
        GlyphComponent {
            text: split_str,
            font: self.font,
            color: self.color,
        }
    }

    /// Prepend `n` ASCII spaces to the text. O(n) for the alloc +
    /// O(existing.len()) for the shift.
    pub fn space_front(&mut self, n: usize) {
        self.pad_front(" ", n);
    }

    /// Append `n` ASCII spaces to the text. O(n).
    pub fn space_back(&mut self, n: usize) {
        self.pad_back(" ", n);
    }

    /// Prepend `n` repetitions of `pad` to the text. O(n·|pad|) +
    /// O(existing.len()).
    pub fn pad_front(&mut self, pad: &str, n: usize) {
        let padding = pad.repeat(n);
        self.text.insert_str(0, &padding);
    }

    /// Append `n` repetitions of `pad` to the text. O(n·|pad|).
    pub fn pad_back(&mut self, pad: &str, n: usize) {
        let padding = pad.repeat(n);
        self.text.push_str(&padding);
    }

    /// True when the text contains at least one non-whitespace
    /// character. O(n) in the text length.
    pub fn contains_non_space(&self) -> bool {
        self.text.chars().any(|c| !c.is_whitespace())
    }

    /// Index of the first non-whitespace character (by `char`
    /// iteration, not grapheme), or `None` if the run is all
    /// whitespace. O(n).
    pub fn index_of_first_non_space_char(&self) -> Option<usize> {
        self.text
            .chars()
            .enumerate()
            .find(|&(_, c)| !c.is_whitespace())
            .map(|(i, _)| i)
    }

    /// Borrow the text as a `&str`. O(1).
    pub fn as_str(&self) -> &str {
        self.text.as_str()
    }

    /// Grapheme-cluster count of the text. O(n) grapheme walk.
    pub fn length(&self) -> usize {
        count_grapheme_clusters(&self.text)
    }

    /// Drop `num` grapheme clusters from the front of the text. O(n)
    /// grapheme walk plus O(text.len()) shift.
    pub fn discard_front(&mut self, num: usize) {
        delete_front_unicode(&mut self.text, num);
    }

    /// Drop `num` grapheme clusters from the back of the text. O(n)
    /// grapheme walk.
    pub fn discard_back(&mut self, num: usize) {
        delete_back_unicode(&mut self.text, num);
    }
}
