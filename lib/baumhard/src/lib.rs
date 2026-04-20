//! Baumhard — glyph-oriented rendering primitives for Mandala.
//!
//! Baumhard owns the GPU-adjacent data model: the `Tree<GfxElement,
//! GfxMutator>` that underpins every glyph layout, the mindmap data
//! model and scene builder, the shader entry points, and the
//! declarative mutator-builder DSL that lets widgets and custom
//! mutations describe tree deltas as serializable JSON.
//!
//! The crate's prescriptive rules (mutation-not-rebuild, arena
//! discipline, benchmark-reuse, no-unsafe) live in
//! `lib/baumhard/CONVENTIONS.md` — read them before touching anything
//! under this crate.

/// Shared math, container, and formatting helpers used across the
/// crate. Kept thin; anything non-trivial graduates to a dedicated
/// module.
pub mod util;
/// Font loading, shaping, and glyph-metric lookups backed by
/// cosmic-text. Owns the long-lived font cache.
pub mod font;
/// GPU-facing structs: `GfxElement`, `GfxMutator`, `GlyphArea`, the
/// `Tree` / `MutatorTree` arenas, predicates, and the instruction
/// vocabulary that drives mutator evaluation.
pub mod gfx_structs;
/// Low-level primitives referenced by the higher-level structs:
/// colour regions, outlines, apply-operations, and similar pure-data
/// value types.
pub mod core;
/// WGSL shader modules and the thin Rust wrappers that expose their
/// entry-point names to the application.
pub mod shaders;
/// `.mindmap.json` data model, loaders, scene builders, tree bridge,
/// and the `CustomMutation` carrier. The most interesting logic in
/// the crate lives here.
pub mod mindmap;
/// Declarative mutator-tree DSL: `MutatorNode` AST + `SectionContext`
/// runtime look-up + `build` walker. Widgets and custom mutations
/// ship their payloads as this AST.
pub mod mutator_builder;

