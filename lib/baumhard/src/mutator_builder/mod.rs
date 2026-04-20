//! Declarative mutator-tree DSL.
//!
//! A `MutatorNode` is a serde-friendly AST mirroring the four
//! `GfxMutator` variants plus a `Repeat` wrapper for "N consecutive
//! children with the same template" (the picker's "24 hue cells", the
//! console's runtime-count rows, the `RepeatWhile`-over-descendants
//! shape every scope topology wants). `build` walks it recursively
//! and consults a `SectionContext` for runtime values — per-cell
//! `GlyphArea`s, runtime counts, runtime mutation lists.
//!
//! **Why this lives in Baumhard.** The AST's named trajectory includes
//! "scope topologies in `lib/baumhard` (recursive `Macro` /
//! `Instruction` via `MutationListSrc` / `InstructionSpec`)". Custom
//! mutations use it to carry their payload, which is a Baumhard-level
//! concern — the `CustomMutation` carrier lives in
//! [`crate::mindmap::custom_mutation`] and folds a `MutatorNode` into
//! its identity + metadata envelope. The color picker, console
//! overlay, and any future user-authored widget reach for the same
//! builder; application code supplies the `SectionContext` impl.

mod ast;
mod build;
mod context;

pub mod tests;

// Re-exports form the crate-wide API of the DSL. Some variants
// (`InstructionSpec`, `MutationListSrc`) aren't exercised by every
// consumer; they're retained because the design covers scope-topology
// follow-ups that do use them.
#[allow(unused_imports)]
pub use ast::{
    CellField, ChannelSrc, CountSrc, InstructionSpec, MutationListSrc, MutationSrc, MutatorNode,
};
pub use build::{build, iter_section_channels};
pub use context::SectionContext;
