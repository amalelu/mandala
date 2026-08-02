// SPDX-License-Identifier: MPL-2.0

//! `GlyphComponent` + `GlyphComponentField` — the leaf of the glyph
//! model hierarchy: one contiguous run of text sharing a font and a
//! color. A `GlyphLine` is a `Vec<GlyphComponent>`; a `GlyphMatrix`
//! is a `Vec<GlyphLine>`; a `GlyphModel` wraps the matrix.

use crate::font::fonts::AppFont;
use crate::util::color::{Color, FloatRgba};
use crate::util::grapheme_chad::{
    count_grapheme_clusters, delete_back_unicode, delete_front_unicode, first_non_whitespace_grapheme,
    split_off_graphemes,
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
    /// Color assignment.
    Color(FloatRgba),
}

/// The leaf: one run of text rendered in a single font and color.
/// Stacks into a [`crate::gfx_structs::model::GlyphLine`], which
/// stacks into a [`crate::gfx_structs::model::GlyphMatrix`], which
/// belongs to a [`crate::gfx_structs::model::GlyphModel`].
#[derive(Serialize, Debug, Eq, PartialEq, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct GlyphComponent {
    /// The text run — may contain multi-byte / multi-grapheme clusters.
    pub text: String,
    /// Font used for this run.
    pub font: AppFont,
    /// RGBA color (u8 per channel).
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
        // Per-channel wrapping multiply via Color's `*` impl —
        // shares `Color::channel_apply` with the standalone
        // wrapping arithmetic so the semantics can't drift.
        self.color = self.color * rhs.color;
    }
}

impl AddAssign for GlyphComponent {
    fn add_assign(&mut self, rhs: Self) {
        if !rhs.text.is_empty() {
            // Append in place: the old `self.text.clone() + &rhs.text`
            // allocated a second String for every concatenation on a
            // hot path (§B7).
            self.text.push_str(&rhs.text);
        }
        if self.font == AppFont::Any {
            self.font = rhs.font;
        }
        // Per-channel wrapping add via Color's `+` impl — same
        // shared `channel_apply` as `mul_assign`.
        self.color = self.color + rhs.color;
    }
}

impl Add for GlyphComponent {
    type Output = Self;

    fn add(mut self, rhs: Self) -> Self::Output {
        // `self` is already owned — cloning it first was a wasted
        // allocation of the whole text run.
        self += rhs;
        self
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

    /// Build an invisible-color spacer of `n` ASCII spaces. Used by
    /// the matrix painter to pad lines. O(n) for the repeat.
    pub fn space(n: usize) -> Self {
        GlyphComponent {
            text: " ".repeat(n),
            font: AppFont::Any,
            color: Color::invisible(),
        }
    }

    /// Split off the graphemes at-and-after `at_idx` into a new
    /// component (inheriting this component's font / color). O(n)
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
    /// character.
    ///
    /// Defined as "[`Self::index_of_first_non_space_grapheme`] found
    /// something" rather than as a second whitespace scan, so the
    /// predicate and the index it guards cannot disagree about what
    /// counts as content.
    ///
    /// Costs: O(n) grapheme walk that short-circuits at the first
    /// content cluster; no allocation.
    pub fn contains_non_space(&self) -> bool {
        self.index_of_first_non_space_grapheme().is_some()
    }

    /// Grapheme-cluster index of the first cluster carrying
    /// non-whitespace content, or `None` if the run is all whitespace.
    ///
    /// The unit is grapheme clusters — the same unit
    /// [`Self::length`], [`Self::split_off`], and every `GlyphLine`
    /// offset speak in — so the result composes with them directly.
    /// It was a `char` ordinal, which agreed with neither the byte
    /// offset it was sliced with nor the cluster offset it was added
    /// to (CONVENTIONS §B3).
    ///
    /// Costs: delegates to
    /// [`first_non_whitespace_grapheme`]; O(n) grapheme walk that
    /// short-circuits at the first content cluster, no allocation.
    pub fn index_of_first_non_space_grapheme(&self) -> Option<usize> {
        first_non_whitespace_grapheme(&self.text)
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
