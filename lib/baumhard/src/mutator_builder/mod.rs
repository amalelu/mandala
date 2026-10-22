// SPDX-License-Identifier: MPL-2.0

//! Declarative mutator-tree DSL.
//!
//! A `MutatorNode` is a serde-friendly AST mirroring the four
//! `GfxMutator` variants plus a `Repeat` wrapper for "N consecutive
//! children with the same template" (24 hue cells,
//! runtime-count rows, descendants-of). `build` walks it recursively
//! and consults a `SectionContext` for runtime values — per-cell
//! `GlyphArea`s, counts, mutation lists. Custom mutations carry
//! their payload as a `MutatorNode`; the color picker, console
//! overlay, and user widgets all reach for the same builder.

mod ast;
mod build;
mod context;

/// `#[allow(clippy::unwrap_used)]` because this is a tests tree
/// (§T2.2): it carries no `cfg(test)` gate — the criterion harness
/// imports its bodies — so the crate-root `warn(clippy::unwrap_used)`
/// sees it as shipped code. `unwrap()` in a test is the correct
/// spelling; the gate that matters for shipped code is
/// `util::unwrap_posture`, which excludes these trees by path.
#[allow(clippy::unwrap_used)]
pub mod tests;

// `InstructionSpec` and `MutationListSrc` are retained as `pub`
// re-exports even when no in-tree consumer uses them — they cover
// scope-topology follow-ups (recursive `Macro` / `Instruction` via
// `MutationListSrc` / `InstructionSpec`) on the named trajectory
// (§7 seam preservation).
#[allow(unused_imports)]
pub use ast::{CellField, ChannelSrc, CountSrc, InstructionSpec, MutationListSrc, MutationSrc, MutatorNode};
pub use build::{build, iter_section_channels};
pub use context::SectionContext;
