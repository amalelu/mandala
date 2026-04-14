//! Declarative mutator-tree DSL.
//!
//! A [`MutatorNode`] is a serde-friendly AST mirroring the four
//! `GfxMutator` variants plus a [`Repeat`] wrapper for "N consecutive
//! children with the same template" (the picker's "24 hue cells", the
//! console's runtime-count rows, …). [`build`] walks it recursively
//! and consults a [`SectionContext`] for runtime values — per-cell
//! `GlyphArea`s, runtime counts, runtime mutation lists.
//!
//! First consumer is the color picker overlay; the AST is shaped to
//! absorb the console overlay (runtime counts via [`CountSrc`]),
//! scope topologies in `lib/baumhard` (recursive `Macro` /
//! `Instruction` via [`MutationListSrc`] / [`InstructionSpec`]), and
//! future user-authored widgets without schema breaks.
//!
//! [`MutatorNode`]: ast::MutatorNode
//! [`Repeat`]: ast::MutatorNode::Repeat
//! [`SectionContext`]: context::SectionContext
//! [`CountSrc`]: ast::CountSrc
//! [`MutationListSrc`]: ast::MutationListSrc
//! [`InstructionSpec`]: ast::InstructionSpec
//! [`build`]: build::build

mod ast;
mod build;
mod context;

#[cfg(test)]
mod tests;

// Re-exports form the crate-wide API of the DSL. Some variants
// (`InstructionSpec`, `MutationListSrc`) aren't exercised by today's
// picker consumer; they're retained because the design also covers
// console / scope-topology follow-ups that do use them.
#[allow(unused_imports)]
pub use ast::{
    CellField, ChannelSrc, CountSrc, InstructionSpec, MutationListSrc, MutationSrc, MutatorNode,
};
pub use build::{build, iter_section_channels};
pub use context::SectionContext;
