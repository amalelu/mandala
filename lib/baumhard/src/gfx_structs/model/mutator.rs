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
