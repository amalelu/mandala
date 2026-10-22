// SPDX-License-Identifier: MPL-2.0

//! Tree-builder tests sharded by subject so each file stays small
//! and greppable. Shared fixtures (`test_map_path`, `synthetic_node`,
//! `synthetic_map`, `mk_chain_map`, `mk_star_map`, `synthetic_portal`,
//! `glyph_area_of`) live in [`fixtures`].
//!
//! - [`node_basic`] — `build_mindmap_tree` structure, root-node count,
//!   glyph_area properties, color-region projection, parent/child
//!   hierarchy, unique IDs, element type invariants.
//! - [`node_scale`] — large-N regression guards (1000-node chain,
//!   500-wide fan-out, deep-chain stack safety).
//! - [`node_background`] — `GlyphArea::background_color` resolution
//!   across hex, empty, transparent, theme-var, malformed, 3-digit.
//! - [`border`] — border tree: void-per-framed, filters, drag
//!   offset, theme var, channel stability, mutator round-trip.
//! - [`canvas_hit`] — canvas click routing over the portal and
//!   connection-label trees: overlap resolution by smallest area,
//!   node-id independence, and a differential sweep against a
//!   linear-scan oracle over the same rectangles.
//! - [`portal`] — portal tree: marker pairs, fold filter,
//!   selection highlight, ascending channels, mutator round-trip,
//!   structural signature.
//! - [`connection`] — connection tree (edges + labels): per-edge
//!   voids, cap filters, identity drift, mutator round-trips.
//! - [`edge_handle`] — edge-handle tree: channel ordering, mutator
//!   round-trip, identity shift on midpoint→CP transitions.
//! - [`sections`] — post-section-refactor invariants: container vs.
//!   section-area vs. section-model layering, `Flag::SectionRoot`,
//!   `section_map` round-trips, multi-section iteration order, and
//!   the `owning_mind_id` climb across the three layers.
//! - [`section_text`] — per-section text projection: which
//!   sections reach a `GlyphArea`, the section AABB math, and the
//!   empty-section skip path.
//! - [`node_clip`] — the connection pass's clip AABBs: raw rect
//!   for frameless nodes, border-expanded rect for framed ones.
//! - [`point_inside`] — `point_inside_any_node` boundary coverage.
//! - [`themes`] — theme-variable resolution across background /
//!   frame / connection / missing-var fall-through.
//! - [`connection_clipping`] — connection glyph clipping in-node /
//!   in-frame / cap survival / cap clip.
//! - [`connection_cache`] — `SceneConnectionCache` integration
//!   (hit, miss, drag stability, eviction, fold, selection
//!   stability).
//! - [`connection_label_emit`] — connection-label emission
//!   (present / missing / position_t / color inheritance).
//! - [`edge_handle_emit`] — edge-handle emission (straight /
//!   curved / cubic / canvas-absolute).
//! - [`portal_emit`] — portal pair-data emission (two-per-pair /
//!   missing endpoint / fold filter / theme var / selection
//!   highlight / anchor / drag offset).
//! - [`section_frame_emit`] — section-frame emission under
//!   `NodeEdit` (gating, cascade, focused variant, preview).
//! - [`node_edit`] — the inactive-node dimming affordance across
//!   the border pass and the app-facing alpha constant.
//! - [`palette_cascade`] — the palette cascade as the projection
//!   passes see it: fill / frame / text / title from
//!   `map.palettes`, `starts_at_root`, level clamping,
//!   `connections_colored`, and the palette edit that moves them
//!   all at once.
//! - [`canvas_signature`] — the border and portal dispatch
//!   signatures: which inputs each covers, checked against the
//!   tree the signature guards rather than against itself.
//! - [`edge_path`] — the per-frame `EdgePathCache` the sampler, the
//!   grab-handles and the label layout share: one build per edge,
//!   none for an edge no pass asks about, and the offset-applied
//!   endpoint rects underneath it.

mod fixtures;

mod border;
mod canvas_hit;
mod canvas_signature;
mod connection;
mod connection_cache;
mod connection_clipping;
mod connection_label_emit;
mod edge_handle;
mod edge_handle_emit;
mod edge_path;
mod node_background;
mod node_basic;
mod node_clip;
mod node_edit;
mod node_scale;
mod palette_cascade;
mod point_inside;
mod portal;
mod portal_emit;
mod section_frame;
mod section_frame_emit;
mod section_text;
mod sections;
mod themes;
