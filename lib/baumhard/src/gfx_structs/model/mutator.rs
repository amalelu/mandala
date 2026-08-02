// SPDX-License-Identifier: MPL-2.0

//! Glyph-model mutators: `DeltaGlyphModel` (field-level deltas) and
//! `GlyphModelCommand` (high-level commands like nudge / rotate /
//! insert). Both implement
//! [`Applicable`](crate::core::primitives::Applicable)`<GlyphModel>`
//! and route through
//! [`GlyphModel`](crate::gfx_structs::model::GlyphModel)'s internal
//! `apply_operation` or its public methods.

use super::component::GlyphComponent;
use super::glyph_model::GlyphModel;
use super::line::GlyphLine;
use super::matrix::GlyphMatrix;

use crate::core::primitives::{Applicable, ApplyOperation};
use crate::gfx_structs::delta::{Delta, DeltaField};
use crate::util::ordered_vec2::OrderedVec2;
use glam::Vec2;
use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumDiscriminants, EnumIter};

/// One field of a [`GlyphModel`] plus its new value. The delta
/// pipeline selects the variant to know which field to touch; the
/// `Operation` variant is the control knob that picks
/// `Assign`/`Add`/`Subtract` for the rest of the delta.
///
/// The payload-free `GlyphModelFieldType` tag is *derived* from this
/// enum, so the two can never drift: adding a variant here adds its
/// tag, and the only other edit a new model mutation needs is its
/// branch in `GlyphModel::apply_operation` (`CONVENTIONS.md` §B4).
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Eq, EnumDiscriminants)]
#[strum_discriminants(name(GlyphModelFieldType))]
#[strum_discriminants(derive(Hash, Serialize, Deserialize, EnumIter, Display))]
#[strum_discriminants(doc = "Payload-free tag for [`GlyphModelField`], derived by strum so it")]
#[strum_discriminants(doc = "always lists exactly the field variants that exist. Keys the")]
#[strum_discriminants(doc = "[`DeltaGlyphModel`] map, where using the tag rather than the")]
#[strum_discriminants(doc = "field avoids hashing the payload (whole glyph matrices) on")]
#[strum_discriminants(doc = "every lookup.")]
pub enum GlyphModelField {
    /// Replace the entire matrix. Operation semantics: `Assign` and
    /// `Add` both overwrite the matrix (matrix addition is defined as
    /// per-line overwrite via the `AddAssign` impl); `Subtract`
    /// and `Multiply` use the matching `*Assign` impls; `Delete`
    /// clears the matrix to its default empty state.
    GlyphMatrix(GlyphMatrix),
    /// Replace one line at `line_num` with `GlyphLine`.
    /// `Assign`/`Add` grow missing lines with empty rows as needed.
    /// `Subtract`/`Multiply`/`Delete` apply only to existing lines and
    /// no-op for absent rows. Operation semantics otherwise mirror
    /// `GlyphMatrix`: `Assign`/`Add` overwrite the target line,
    /// `Subtract`/`Multiply` use the line's `*Assign` impls, `Delete`
    /// clears the line.
    GlyphLine(usize, GlyphLine),
    /// Replace many lines in one go, identified by `(line_num, line)`.
    /// Each line is applied independently with the delta's operation;
    /// growth and destructive no-op semantics per line match
    /// [`GlyphModelField::GlyphLine`].
    GlyphLines(Vec<(usize, GlyphLine)>),
    /// Replace the draw-order layer.
    Layer(usize),
    /// Replace the position anchor.
    Position(OrderedVec2),
    /// Control variant: selects how sibling fields compose.
    Operation(ApplyOperation),
}

impl GlyphModelField {
    /// Construct a `Position` field from `(x, y)` pixels. O(1).
    pub fn position(x: f32, y: f32) -> Self {
        Self::Position(OrderedVec2::new_f32(x, y))
    }
}

impl DeltaField for GlyphModelField {
    const OPERATION_KEY: GlyphModelFieldType = GlyphModelFieldType::Operation;

    #[inline]
    fn as_operation(&self) -> Option<ApplyOperation> {
        match self {
            GlyphModelField::Operation(operation) => Some(*operation),
            _ => None,
        }
    }
}

////////////////////////////////////////
/////// DeltaGlyphModel Mutator ///////
//////////////////////////////////////

/// Field-set delta for a [`GlyphModel`]. One entry per touched field
/// type; the companion `Operation` entry carries the global
/// arithmetic mode.
///
/// Storage, `new()`, the operation lookup, and the additive merge come
/// from the shared [`Delta`] container — the model and area deltas are
/// the same container over different vocabularies. The accessors below
/// are what *is* model-specific, and they **borrow**: a delta is
/// applied through a shared reference, and deep-copying a whole
/// [`GlyphMatrix`] just to read it made `apply_to` pay for a payload
/// even under `Noop` / `Delete` (`CONVENTIONS.md` §B7 names this a hot
/// loop).
pub type DeltaGlyphModel = Delta<GlyphModelField>;

impl DeltaGlyphModel {
    /// Borrow the matrix payload if present. O(1) — no copy of the
    /// matrix.
    pub fn glyph_matrix(&self) -> Option<&GlyphMatrix> {
        match self.get(GlyphModelFieldType::GlyphMatrix)? {
            GlyphModelField::GlyphMatrix(matrix) => Some(matrix),
            _ => None,
        }
    }

    /// Layer payload, if any. O(1).
    pub fn layer(&self) -> Option<usize> {
        match self.get(GlyphModelFieldType::Layer)? {
            GlyphModelField::Layer(layer) => Some(*layer),
            _ => None,
        }
    }

    /// Borrow the single-line payload if present, with its target
    /// line number. O(1) — no copy of the line.
    pub fn glyph_line(&self) -> Option<(usize, &GlyphLine)> {
        match self.get(GlyphModelFieldType::GlyphLine)? {
            GlyphModelField::GlyphLine(line_num, glyph_line) => Some((*line_num, glyph_line)),
            _ => None,
        }
    }

    /// Borrow the multi-line payload if present. O(1) — no copy of
    /// the lines.
    pub fn glyph_lines(&self) -> Option<&[(usize, GlyphLine)]> {
        match self.get(GlyphModelFieldType::GlyphLines)? {
            GlyphModelField::GlyphLines(lines) => Some(lines),
            _ => None,
        }
    }

    /// Position payload, if any. O(1).
    pub fn position(&self) -> Option<OrderedVec2> {
        match self.get(GlyphModelFieldType::Position)? {
            GlyphModelField::Position(position) => Some(*position),
            _ => None,
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

/// Imperative command targeting a [`GlyphModel`]. Unlike
/// [`DeltaGlyphModel`] (arithmetic), each variant performs one
/// fixed operation. All variants are O(1) on position; the insert
/// variants are O(line length).
///
/// The payload-free `GlyphModelCommandType` tag is derived from this
/// enum, so a new command cannot ship without its tag.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, EnumDiscriminants)]
#[strum_discriminants(name(GlyphModelCommandType))]
#[strum_discriminants(derive(Hash, Serialize, Deserialize, EnumIter, Display))]
#[strum_discriminants(doc = "Payload-free tag for [`GlyphModelCommand`], derived by strum.")]
#[strum_discriminants(doc = "Keyable in `HashMap`/`HashSet` for command-dispatch tables.")]
#[serde(deny_unknown_fields)]
pub enum GlyphModelCommand {
    /// Shift position left by the given pixels.
    NudgeLeft(f32),
    /// Shift position right by the given pixels.
    NudgeRight(f32),
    /// Shift position down by the given pixels.
    NudgeDown(f32),
    /// Shift position up by the given pixels.
    NudgeUp(f32),
    /// Teleport position to absolute `(x, y)`.
    MoveTo(f32, f32),
    /// Rotate position around `pivot` by `degrees` (clockwise).
    Rotate {
        /// Pivot in world coordinates. On the wire this is a
        /// two-element `[x, y]` array — `glam` serializes `Vec2` as a
        /// sequence and its deserializer rejects the
        /// `{"x": …, "y": …}` object form. See `format/mutations.md`.
        pivot: Vec2,
        /// Rotation angle in degrees.
        degrees: f32,
    },
    /// Insert `component` at `(line_num, at_idx)`, overwriting any
    /// overlapping content.
    RudeInsert {
        /// Content to insert.
        component: GlyphComponent,
        /// Target line index.
        line_num: usize,
        /// Grapheme-index inside the line.
        at_idx: usize,
    },
    /// Insert `component` at `(line_num, at_idx)`, shifting existing
    /// graphemes right.
    PoliteInsert {
        /// Content to insert.
        component: GlyphComponent,
        /// Target line index.
        line_num: usize,
        /// Grapheme-index inside the line.
        at_idx: usize,
    },
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
