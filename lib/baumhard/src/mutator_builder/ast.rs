// SPDX-License-Identifier: MPL-2.0

//! Typed AST for the mutator-tree DSL. The variants here are what
//! [`mutator_builder::build`](super::build) walks to produce a
//! [`MutatorTree<GfxMutator>`](crate::gfx_structs::tree::MutatorTree)
//! ready for `apply_to`. Every JSON-loaded mutator (custom mutations,
//! procedural animation defs in `lib/baumhard/src/mindmap/`) round-trips
//! through these types via serde, and the procedural-builder code paths
//! in the app crate construct the same shapes directly. See `super` for
//! the high-level tour and CONVENTIONS §B2 for why the tree mutates
//! rather than rebuilds.

use crate::core::primitives::ApplyOperation;
use crate::gfx_structs::mutator::{Instruction, Mutation};
use crate::gfx_structs::predicate::Predicate;
use crate::util::ordered_vec2::OrderedVec2;
use serde::{Deserialize, Serialize};

/// One node in the mutator-tree DSL. Variants map 1:1 to `GfxMutator`
/// constructors; `Repeat` is a compact sugar for "expand to N children
/// at consecutive channels".
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum MutatorNode {
    /// `GfxMutator::Void` — no mutation, just structural grouping.
    /// Children are expanded in declaration order.
    Void {
        /// Branch-routing channel index. The walker descends only
        /// where the target tree's channel matches.
        channel: usize,
        /// Inner mutator nodes expanded in declaration order under
        /// this grouping. Empty by default so a bare structural
        /// `Void` need not declare an empty list.
        #[serde(default)]
        children: Vec<MutatorNode>,
    },
    /// `GfxMutator::Single` — one mutation on one channel.
    Single {
        /// Channel source — literal index or per-iteration index
        /// resolved by an enclosing [`MutatorNode::Repeat`].
        channel: ChannelSrc,
        /// Mutation payload — literal `Mutation`, runtime-fetched
        /// from a [`SectionContext`](super::context::SectionContext),
        /// or `None` placeholder.
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
        /// Branch-routing channel index for the whole macro batch.
        channel: usize,
        /// Source of the flat `Vec<Mutation>` — literal payload
        /// or runtime-fetched by label.
        mutations: MutationListSrc,
        /// Optional inner nodes expanded after the macro. The
        /// `SelfAndDescendants` shape uses one
        /// `Instruction(RepeatWhile)` child to walk descendants.
        /// Empty default keeps the common flat-Macro case terse.
        #[serde(default)]
        children: Vec<MutatorNode>,
    },
    /// `GfxMutator::Instruction` — recursive evaluation driver
    /// (`RepeatWhile` etc.) wrapping inner children.
    Instruction {
        /// Branch-routing channel index. The walker descends only
        /// where the target tree's channel matches.
        channel: usize,
        /// Which `Instruction` variant drives the recursive walk
        /// (`RepeatWhile`, `RotateWhile`, `SpatialDescend`,
        /// `MapChildren`).
        instruction: InstructionSpec,
        /// Optional per-step mutation applied to the current
        /// target before descending. Defaults to
        /// [`MutationSrc::None`] so `Instruction` nodes that only
        /// drive walking need not carry a payload.
        #[serde(default = "MutationSrc::none_default")]
        mutation: MutationSrc,
        /// Inner mutator nodes evaluated against each visited
        /// target during the walk. Empty default for terse JSON.
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
        /// Free-form label the [`SectionContext`](super::context::SectionContext)
        /// keys per-iteration runtime data on (mutation lookups,
        /// runtime counts, area lookups).
        section: String,
        /// First channel in the iterated range. Iteration `i`
        /// (zero-indexed) maps to channel `channel_base + i`.
        channel_base: usize,
        /// How many iterations to expand — literal at AST time or
        /// runtime-fetched by section label.
        count: CountSrc,
        /// Iteration indices to skip. The expanded child set is
        /// `count - skip_indices.len()` nodes. Empty by default.
        #[serde(default)]
        skip_indices: Vec<usize>,
        /// One template node cloned per iteration; its
        /// [`ChannelSrc::SectionIndex`] entries resolve to
        /// `channel_base + iter_index` at build time.
        template: Box<MutatorNode>,
    },
}

/// Where a `Single`'s channel comes from. Inside a `Repeat`,
/// `SectionIndex` resolves to `channel_base + iter_index`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ChannelSrc {
    /// A baked-in channel index.
    Literal(usize),
    /// The iteration's channel (`channel_base + iter_index`). Only
    /// meaningful inside a [`MutatorNode::Repeat`] template.
    SectionIndex,
}

/// Static or runtime-supplied cell count.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
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
#[non_exhaustive]
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
//
// The lowercase variants (`position`, `bounds`, `scale`, `line_height`)
// match the snake_case JSON tags consumers emit for runtime-supplied
// fields; renaming them would break the on-the-wire contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(non_camel_case_types)]
#[non_exhaustive]
pub enum CellField {
    /// Pull the cell's text payload from the per-section area
    /// lookup at apply time.
    Text,
    /// Pull the cell's canvas-space position
    /// (`OrderedVec2`) from the area lookup.
    position,
    /// Pull the cell's render bounds (width, height in canvas
    /// pixels) from the area lookup.
    bounds,
    /// Pull the cell's font-size scalar from the area lookup.
    scale,
    /// Pull the cell's line-height multiplier from the area lookup.
    line_height,
    /// Pull the cell's [`ColorFontRegions`](crate::core::primitives::ColorFontRegions)
    /// styled-span set from the area lookup.
    ColorFontRegions,
    /// Pull the cell's optional halo outline payload from the
    /// area lookup.
    Outline,
    /// Bake an [`ApplyOperation`] literal into the delta — every
    /// cell built from this template uses the same arithmetic
    /// (`Add`, `Assign`, `Subtract`, `Delete`, ...).
    Operation(ApplyOperation),
}

/// Serializable shadow of [`Instruction`].
///
/// The variants are intentionally 1:1 with [`Instruction`] except for
/// `RepeatWhileAlwaysTrue`, which is sugar: it serializes as a plain
/// named variant instead of forcing every JSON author to write out a
/// full always-true [`Predicate`]. The seam is preserved — a later
/// format revision can collapse the sugar into a serde alias without
/// breaking existing documents.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
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
            InstructionSpec::RepeatWhileAlwaysTrue => Instruction::RepeatWhile(Predicate::always_true()),
            InstructionSpec::RepeatWhile(p) => Instruction::RepeatWhile(p),
            InstructionSpec::RotateWhile(a, p) => Instruction::RotateWhile(a, p),
            InstructionSpec::SpatialDescend(point) => Instruction::SpatialDescend(point),
            InstructionSpec::MapChildren => Instruction::MapChildren,
        }
    }
}
