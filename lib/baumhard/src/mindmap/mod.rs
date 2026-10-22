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
/// The one-node `MindMap` a rejected load is rendered as, so a
/// loader message reaches the canvas instead of only `stderr`.
pub mod placard;
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
/// Keys the model has no field for — captured at load, warned about
/// once, and written back untouched at save.
pub mod unknown_keys;

mod border_tests;
#[cfg(test)]
pub(crate) mod test_helpers;

use crate::util::color::Color;
use crate::util::color_conversion::hex_to_color;

/// The cyan "this is active" affordance, in the `#RRGGBB` notation
/// the model speaks — the one place the project writes this color
/// down. Applied at scene / tree emission time (selected edges,
/// edge handles, portal markers, section frames, portal mutator
/// output) and, through [`SELECTION_HIGHLIGHT`] below, by every
/// app-side reading of the same color.
///
/// `pub` rather than `pub(crate)` on purpose. While it was
/// crate-private the app had no way to *reach* the canonical value
/// and duplicated it instead — as `[0.0, 0.9, 1.0, 1.0]` in the
/// document layer (which quantises to 229.5, not 229) and as
/// `Color::rgba(0, 230, 255, ..)` twice in the renderer. Three
/// spellings of one color, and only one of them was this one.
pub const SELECTION_HIGHLIGHT_HEX: &str = "#00E5FF";

/// [`SELECTION_HIGHLIGHT_HEX`] in baumhard's byte-packed [`Color`],
/// parsed at compile time. Every non-hex reading of the selection
/// color derives from here — the app's float-RGBA
/// `document::types::HIGHLIGHT_COLOR` via [`Color::to_float`], the
/// renderer's two cosmic-text colors via
/// [`crate::font::color::cosmic_color_from_color`] — so no consumer
/// has a reason to write the channels out again.
pub const SELECTION_HIGHLIGHT: Color = match hex_to_color(SELECTION_HIGHLIGHT_HEX) {
    Some(color) => color,
    // Unreachable for a well-formed literal, and a compile error
    // rather than a run-time panic if the literal above is ever
    // edited into something `hex_to_color` rejects.
    None => panic!("SELECTION_HIGHLIGHT_HEX must be a hex color literal"),
};
