//! AST types for the mutator-tree DSL. See `super` for the high-level
//! tour; this file is the type-level wire format the JSON parses into.

use baumhard::core::primitives::ApplyOperation;
use baumhard::gfx_structs::mutator::Instruction;
use baumhard::gfx_structs::predicate::Predicate;
use baumhard::util::ordered_vec2::OrderedVec2;
use serde::Deserialize;

/// One node in the mutator-tree DSL. Variants map 1:1 to `GfxMutator`
/// constructors; `Repeat` is a compact sugar for "expand to N children
/// at consecutive channels".
#[derive(Debug, Clone, Deserialize)]
pub enum MutatorNode {
    /// `GfxMutator::Void` — no mutation, just structural grouping.
    /// Children are expanded in declaration order.
    Void {
        channel: usize,
        #[serde(default)]
        children: Vec<MutatorNode>,
    },
    /// `GfxMutator::Single` — one mutation on one channel.
    Single {
        channel: ChannelSrc,
        mutation: MutationSrc,
    },
    /// `GfxMutator::Macro` — flat batch of `Mutation`s on one channel.
    /// Macros can't contain other Macros — that's `Mutation`-level
    /// nesting, which this AST doesn't model.
    Macro {
        channel: usize,
        mutations: MutationListSrc,
    },
    /// `GfxMutator::Instruction` — recursive evaluation driver
    /// (`RepeatWhile` etc.) wrapping inner children.
    Instruction {
        channel: usize,
        instruction: InstructionSpec,
        #[serde(default = "MutationSrc::none_default")]
        mutation: MutationSrc,
        #[serde(default)]
        children: Vec<MutatorNode>,
    },
    /// Compact "N consecutive children with the same template" — the
    /// "24 children of X" idiom. Expands at apply time into
    /// `count - skip_indices.len()` children on channels
    /// `[channel_base + i for i in 0..count if !skip_indices.contains(i)]`.
    /// The `template`'s `ChannelSrc` should be `SectionIndex` so the
    /// builder threads the per-iteration channel through.
    Repeat {
        section: String,
        channel_base: usize,
        count: CountSrc,
        #[serde(default)]
        skip_indices: Vec<usize>,
        template: Box<MutatorNode>,
    },
}

/// Where a `Single`'s channel comes from. Inside a `Repeat`,
/// `SectionIndex` resolves to `channel_base + iter_index`.
#[derive(Debug, Clone, Deserialize)]
pub enum ChannelSrc {
    Literal(usize),
    SectionIndex,
}

/// Static or runtime-supplied cell count.
#[derive(Debug, Clone, Deserialize)]
pub enum CountSrc {
    Literal(usize),
    Runtime(String),
}

/// Where a single `Mutation` comes from.
#[derive(Debug, Clone, Deserialize)]
pub enum MutationSrc {
    /// `Mutation::AreaDelta` whose fields are filled at apply time —
    /// bare `CellField` variants pull from the area lookup; tagged
    /// variants are baked-in literals.
    AreaDelta(Vec<CellField>),
    /// Entirely runtime-supplied single `Mutation`. The section
    /// context is asked for it keyed by the enclosing section's name
    /// (or `""` if not inside a `Repeat`).
    Runtime,
    /// `Mutation::None` literal.
    None,
}

impl MutationSrc {
    pub(super) fn none_default() -> Self {
        MutationSrc::None
    }
}

/// Where a `Macro`'s `Vec<Mutation>` comes from.
#[derive(Debug, Clone, Deserialize)]
pub enum MutationListSrc {
    /// Entirely runtime-supplied — the section context returns the
    /// list keyed by the label (a free-form name the consumer
    /// disambiguates on).
    Runtime(String),
}

/// Per-cell `AreaDelta` field slot. Bare variants = "supplied at
/// runtime by the area lookup"; tagged variants = baked-in literals
/// reused for every cell.
#[derive(Debug, Clone, Deserialize)]
#[allow(non_camel_case_types)]
pub enum CellField {
    Text,
    position,
    bounds,
    scale,
    line_height,
    ColorFontRegions,
    Outline,
    Operation(ApplyOperation),
}

/// Serializable shadow of `baumhard::gfx_structs::mutator::Instruction`.
/// `RepeatWhileAlwaysTrue` is spelled out as a named variant to avoid
/// forcing every caller to serialize a full always-true `Predicate`.
#[derive(Debug, Clone, Deserialize)]
pub enum InstructionSpec {
    /// `Instruction::RepeatWhile(Predicate::always_true())`.
    RepeatWhileAlwaysTrue,
    RepeatWhile(Predicate),
    RotateWhile(f32, Predicate),
    SpatialDescend(OrderedVec2),
}

impl InstructionSpec {
    pub(super) fn into_instruction(self) -> Instruction {
        match self {
            InstructionSpec::RepeatWhileAlwaysTrue => {
                Instruction::RepeatWhile(Predicate::always_true())
            }
            InstructionSpec::RepeatWhile(p) => Instruction::RepeatWhile(p),
            InstructionSpec::RotateWhile(a, p) => Instruction::RotateWhile(a, p),
            InstructionSpec::SpatialDescend(point) => Instruction::SpatialDescend(point),
        }
    }
}
