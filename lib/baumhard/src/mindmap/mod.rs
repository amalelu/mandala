// SPDX-License-Identifier: MPL-2.0

//! Mindmap data model, loader/saver, and the builders that project
//! a `MindMap` into the Baumhard render tree. Borders, connections,
//! portal labels, section frames, and every kind of handle descend
//! from the types declared under `model` and materialize through
//! `tree_builder`.

/// Timing envelope, easing, and lerp helpers for animated
/// `CustomMutation`s.
pub mod animation;
/// Per-node glyph-border configuration plus geometry constants
/// shared by the renderer and the border tree builder.
pub mod border;
/// Border-side pattern syntax — parser and grapheme-aware fitter
/// for `CustomBorderGlyphs.{top, bottom, left, right}` strings.
pub mod border_pattern;
/// Connection-path geometry: anchor resolution, straight/cubic
/// Bezier construction, arc-length sampling, point-to-path distance.
pub mod connection;
/// `CustomMutation` carrier — identity, metadata, and the
/// `MutatorNode` payload.
pub mod custom_mutation;
/// `.mindmap.json` loader and saver — the serialization boundary.
pub mod loader;
/// Mindmap data model — `MindMap`, `MindNode`, `MindEdge`, palettes,
/// canvas.
pub mod model;
/// Portal-label geometry: point ↔ `border_t` on a node's rectangular
/// border, plus the directional default orientation.
pub mod portal_geometry;
/// Per-edge cache of connection glyph geometry — keeps the
/// connection pass from re-sampling every visible edge every drag
/// frame.
pub mod scene_cache;
/// `MindMap` → `Tree<GfxElement, GfxMutator>` builder with
/// per-canvas-role sub-builders.
pub mod tree_builder;

#[cfg(test)]
pub(crate) mod test_helpers;
mod border_tests;

/// Cyan selection highlight applied at scene / tree emission time
/// (selected edges, edge handles, portal markers, portal mutator
/// output). The app crate's `document::types::HIGHLIGHT_COLOR` is
/// the approximately-matching float-RGBA form used by the selection
/// machinery upstream.
pub(crate) const SELECTION_HIGHLIGHT_HEX: &str = "#00E5FF";
