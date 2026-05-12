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
