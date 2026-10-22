// SPDX-License-Identifier: MPL-2.0

//! Baumhard — glyph-oriented rendering primitives for Mandala.
//!
//! Owns the GPU-adjacent data model: the `Tree<GfxElement,
//! GfxMutator>` that underpins every glyph layout, the mindmap
//! model and scene builder, shader entry points, and the
//! declarative mutator-builder DSL.
//!
//! Prescriptive rules (mutation-not-rebuild, arena discipline,
//! benchmark-reuse, no-unsafe) live in
//! `lib/baumhard/CONVENTIONS.md` — read them before touching this
//! crate.

// CODE_CONVENTIONS §9 closes with "Bare `unwrap()` outside tests is
// a bug", and this is the half of that rule an editor can tell you
// about while you type. `util::unwrap_posture` is the other half —
// it reads the workspace's source text and fails every `./test.sh`
// run; the lint fails only `./test.sh --lint`, a hard gate since
// clippy's baseline reached zero, but one you have to ask for. Two
// mechanisms rather than one because they disagree usefully: the lint sees
// post-expansion code the text scan cannot read, and the scan sees
// the `pub mod tests;` trees the lint has to be told about.
//
// The `cfg_attr` is what keeps the lint off test code. A
// `#[cfg(test)] mod` does not exist in the build where the lint is
// live, and in the build where it does exist the whole crate is
// allowed — so `unwrap()` stays the right spelling in a test and a
// bug everywhere else.
#![warn(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

/// Low-level primitives: colour regions, outlines, apply-operations,
/// and pure-data value types.
pub mod core;
/// Font loading, shaping, and glyph-metric lookups backed by
/// cosmic-text. Owns the long-lived font cache.
pub mod font;
/// On-disk format primitives — the JSON loader facade for
/// non-mindmap configs (keybinds, user macros, embedded widget
/// specs). The mindmap format itself lives under
/// [`mindmap::loader`].
pub mod format;
/// GPU-facing structs: `GfxElement`, `GfxMutator`, `GlyphArea`,
/// `Tree`/`MutatorTree`, predicates, and the instruction vocabulary.
pub mod gfx_structs;
/// `.mindmap.json` data model, loaders, scene/tree builders, and
/// the `CustomMutation` carrier.
pub mod mindmap;
/// Declarative mutator-tree DSL: `MutatorNode` AST + `SectionContext`
/// runtime look-up + `build` walker.
pub mod mutator_builder;
/// Shared math, container, and formatting helpers across the crate.
pub mod util;
