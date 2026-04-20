//! Mindmap data model, loader/saver, and the builders that project a
//! `MindMap` into the Baumhard render tree and flat scene. Borders,
//! connections, portal labels, and edge handles all descend from the
//! types declared under `model` and materialise through `tree_builder`
//! / `scene_builder`.

/// Timing envelope, easing, and lerp helpers for animated
/// `CustomMutation`s — what starts an animation when a mutation
/// carries an `AnimationTiming`.
pub mod animation;
/// Mindmap data model — `MindMap`, `MindNode`, `MindEdge`, palettes,
/// canvas. What the loader deserializes and the document layer
/// mutates.
pub mod model;
/// `.mindmap.json` loader and saver — the serialization boundary.
pub mod loader;
/// Per-node glyph-border configuration plus the geometry constants
/// shared by the renderer and the border tree builder.
pub mod border;
/// Connection-path geometry: anchor resolution, straight/cubic
/// Bezier construction, arc-length sampling, point-to-path distance.
pub mod connection;
/// `CustomMutation` carrier — identity, metadata, and the
/// `MutatorNode` payload dispatched on triggers and on console
/// `mutation apply`.
pub mod custom_mutation;
/// Portal-label geometry: point ↔ `border_t` on a node's
/// rectangular border, plus the directional default orientation.
pub mod portal_geometry;
/// Mindmap → `Scene` builder for flat elements (connections,
/// borders, portals).
pub mod scene_builder;
/// Per-edge cache of connection glyph geometry — keeps the scene
/// builder from re-sampling every visible edge every drag frame.
pub mod scene_cache;
/// `MindMap` → `Tree<GfxElement, GfxMutator>` builder with
/// per-canvas-role sub-builders (nodes, borders, portals,
/// connections, edge handles).
pub mod tree_builder;

/// Cyan selection highlight applied at scene / tree emission time
/// (selected edges, edge handles, portal markers, portal mutator
/// output). The app crate's `document::types::HIGHLIGHT_COLOR` is
/// the approximately-matching float-RGBA form used by the selection
/// machinery upstream.
pub(crate) const SELECTION_HIGHLIGHT_HEX: &str = "#00E5FF";
