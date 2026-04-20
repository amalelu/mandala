//! AST types for the mutator-tree DSL. See `super` for the high-level
//! tour; this file is the type-level wire format that JSON parses into.

use crate::core::primitives::ApplyOperation;
use crate::gfx_structs::mutator::{Instruction, Mutation};
use crate::gfx_structs::predicate::Predicate;
use crate::util::ordered_vec2::OrderedVec2;
use serde::{Deserialize, Serialize};

/// One node in the mutator-tree DSL. Variants map 1:1 to `GfxMutator`
/// constructors; `Repeat` is a compact sugar for "expand to N children
/// at consecutive channels".
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Macros can't nest their own mutation list — that's
    /// `Mutation`-level nesting, which this AST doesn't model.
    /// `children` lets a Macro carry child mutator nodes in the
    /// arena (for the `SelfAndDescendants` scope shape: Macro at root
    /// applies to the anchor, with an `Instruction(RepeatWhile)`
    /// child walking descendants). Defaults to empty so the overwhelming
    /// "flat Macro" case stays terse.
    Macro {
        channel: usize,
        mutations: MutationListSrc,
        #[serde(default)]
        children: Vec<MutatorNode>,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChannelSrc {
    /// A baked-in channel index.
    Literal(usize),
    /// The iteration's channel (`channel_base + iter_index`). Only
    /// meaningful inside a [`MutatorNode::Repeat`] template.
    SectionIndex,
}

/// Static or runtime-supplied cell count.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CountSrc {
    /// Count baked into the AST at deserialize time.
    Literal(usize),
    /// Count fetched from
    /// [`SectionContext::count`](crate::mutator_builder::context::SectionContext::count)
    /// at apply time under the given label.
    Runtime(String),
}

/// Where a single `Mutation` comes from.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum MutationListSrc {
    /// A baked-in `Vec<Mutation>` serialized alongside the AST. The
    /// overwhelming case for [`crate::mindmap::custom_mutation`]
    /// entries that ship pure data from a JSON file — no runtime
    /// context is consulted.
    Literal(Vec<Mutation>),
    /// Entirely runtime-supplied — the section context returns the
    /// list keyed by the label (a free-form name the consumer
    /// disambiguates on). Used by consumers whose `Vec<Mutation>`
    /// depends on scene state (e.g. size-aware layouts).
    Runtime(String),
}

/// Per-cell `AreaDelta` field slot. Bare variants = "supplied at
/// runtime by the area lookup"; tagged variants = baked-in literals
/// reused for every cell.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Serializable shadow of [`Instruction`].
/// `RepeatWhileAlwaysTrue` is spelled out as a named variant to avoid
/// forcing every caller to serialize a full always-true `Predicate`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InstructionSpec {
    /// `Instruction::RepeatWhile(Predicate::always_true())`.
    RepeatWhileAlwaysTrue,
    /// `Instruction::RepeatWhile(predicate)`.
    RepeatWhile(Predicate),
    /// `Instruction::RotateWhile(angle, predicate)`.
    RotateWhile(f32, Predicate),
    /// `Instruction::SpatialDescend(point)`.
    SpatialDescend(OrderedVec2),
    /// `Instruction::MapChildren` — unit variant, no payload. Pairs
    /// this instruction node's mutator children with the current
    /// target's children by sibling position (zip), independent of
    /// channel. The opt-in alternative to channel-based alignment for
    /// per-index targeting.
    MapChildren,
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
            InstructionSpec::MapChildren => Instruction::MapChildren,
        }
    }
}
