//! Glyph-model mutators: `DeltaGlyphModel` (field-level deltas) and
//! `GlyphModelCommand` (high-level commands like nudge / rotate /
//! insert). Both implement [`Applicable<GlyphModel>`] and route through
//! [`GlyphModel`]'s internal `apply_operation` or its public methods.

use super::glyph_model::GlyphModel;
use super::line::GlyphLine;
use super::matrix::GlyphMatrix;
use super::component::GlyphComponent;

use crate::core::primitives::{Applicable, ApplyOperation};
use crate::util::ordered_vec2::OrderedVec2;
use glam::Vec2;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumIter};

/// Payload-free discriminant for [`GlyphModelField`]. Used as a
/// HashMap/HashSet key in [`DeltaGlyphModel`]; cheap to hash and
/// compare.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Eq, Hash, EnumIter, Display)]
pub enum GlyphModelFieldType {
    /// Tag for [`GlyphModelField::GlyphMatrix`].
    GlyphMatrix,
    /// Tag for [`GlyphModelField::GlyphLine`].
    GlyphLine,
    /// Tag for [`GlyphModelField::GlyphLines`].
    GlyphLines,
    /// Reserved for flag deltas; no matching field today
    /// (§6 seam).
    Flags,
    /// Tag for [`GlyphModelField::Layer`].
    Layer,
    /// Tag for [`GlyphModelField::Position`].
    Position,
    /// Tag for [`GlyphModelField::Operation`] (the control variant).
    Operation,
}

/// One field of a [`GlyphModel`] plus its new value. The delta
/// pipeline selects the variant to know which field to touch; the
/// `Operation` variant is the control knob that picks
/// `Assign`/`Add`/`Subtract` for the rest of the delta.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Eq)]
pub enum GlyphModelField {
    /// Replace the entire matrix.
    GlyphMatrix(GlyphMatrix),
    /// Replace one line at `line_num` with `GlyphLine`.
    GlyphLine(usize, GlyphLine),
    /// Replace many lines in one go, identified by `(line_num, line)`.
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

    /// Discriminant tag for this field. O(1).
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

    /// Whether `self` and `other` carry the same variant (ignoring
    /// payload). O(1).
    #[inline]
    pub fn same_type(&self, other: &GlyphModelField) -> bool {
        self.variant() == other.variant()
    }
}

////////////////////////////////////////
/////// DeltaGlyphModel Mutator ///////
//////////////////////////////////////

/// Field-set delta for a [`GlyphModel`]. One entry per touched field
/// type; the companion `Operation` entry carries the global
/// arithmetic mode.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeltaGlyphModel {
    /// Keyed by field variant so duplicates collapse.
    pub fields: FxHashMap<GlyphModelFieldType, GlyphModelField>,
}

impl DeltaGlyphModel {
    /// Build a delta from a list of field payloads. Later entries
    /// of the same variant win. O(n) in `fields.len()`.
    pub fn new(fields: Vec<GlyphModelField>) -> DeltaGlyphModel {
        let mut field_map = FxHashMap::default();
        for field in fields {
            field_map.insert(field.variant(), field.clone());
        }
        DeltaGlyphModel { fields: field_map }
    }

    /// Clone the matrix payload if present. O(matrix size).
    pub fn glyph_matrix(&self) -> Option<GlyphMatrix> {
        if let Some(GlyphModelField::GlyphMatrix(matrix)) =
            self.fields.get(&GlyphModelFieldType::GlyphMatrix)
        {
            Some(matrix.clone())
        } else {
            None
        }
    }

    /// Layer payload, if any. O(1).
    pub fn layer(&self) -> Option<usize> {
        if let Some(GlyphModelField::Layer(layer)) = self.fields.get(&GlyphModelFieldType::Layer) {
            Some(*layer)
        } else {
            None
        }
    }

    /// Clone the single-line payload if present. O(line length).
    pub fn glyph_line(&self) -> Option<(usize, GlyphLine)> {
        if let Some(GlyphModelField::GlyphLine(line_num, glyph_line)) =
            self.fields.get(&GlyphModelFieldType::GlyphLine)
        {
            Some((*line_num, glyph_line.clone()))
        } else {
            None
        }
    }

    /// Clone the multi-line payload if present. O(sum of line sizes).
    pub fn glyph_lines(&self) -> Option<Vec<(usize, GlyphLine)>> {
        if let Some(GlyphModelField::GlyphLines(lines)) =
            self.fields.get(&GlyphModelFieldType::GlyphLines)
        {
            Some(lines.clone())
        } else {
            None
        }
    }

    /// Position payload, if any. O(1).
    pub fn position(&self) -> Option<OrderedVec2> {
        if let Some(GlyphModelField::Position(vec)) =
            self.fields.get(&GlyphModelFieldType::Position)
        {
            Some(*vec)
        } else {
            None
        }
    }

    /// Global arithmetic mode (`Assign` / `Add` / `Subtract`), or
    /// `Noop` when no `Operation` entry is present. O(1).
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

/// Payload-free discriminant for [`GlyphModelCommand`]. Keyable in
/// HashMap/HashSet for command-dispatch tables.
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize, Eq, Hash, EnumIter, Display)]
pub enum GlyphModelCommandType {
    /// Tag for `GlyphModelCommand::NudgeLeft`.
    NudgeLeft,
    /// Tag for `GlyphModelCommand::NudgeRight`.
    NudgeRight,
    /// Tag for `GlyphModelCommand::NudgeDown`.
    NudgeDown,
    /// Tag for `GlyphModelCommand::NudgeUp`.
    NudgeUp,
    /// Tag for `GlyphModelCommand::MoveTo`.
    MoveTo,
    /// Reserved for a future flag-setting command (§6 seam).
    SetFlag,
    /// Tag for `GlyphModelCommand::Rotate`.
    Rotate,
    /// Tag for `GlyphModelCommand::RudeInsert`.
    RudeInsert,
    /// Tag for `GlyphModelCommand::PoliteInsert`.
    PoliteInsert,
}

/// Imperative command targeting a [`GlyphModel`]. Unlike
/// [`DeltaGlyphModel`] (arithmetic), each variant performs one
/// fixed operation. All variants are O(1) on position; the insert
/// variants are O(line length).
#[derive(Clone, Debug, Serialize, Deserialize)]
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
        /// Pivot in world coordinates.
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

impl GlyphModelCommand {
    /// Discriminant tag for this command. O(1).
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

    /// Whether `self` and `other` share the same variant (ignoring
    /// payload). O(1).
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
