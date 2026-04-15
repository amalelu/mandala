//! Scene-builder tests sharded by subject so each file stays small
//! and greppable. Shared fixtures (`test_map_path`, `synthetic_node`,
//! `synthetic_edge`, `synthetic_map`, `themed_node`, `two_node_edge_map`,
//! `synthetic_portal`) live in [`fixtures`].
//!
//! - [`point_inside`] — `point_inside_any_node` boundary coverage.
//! - [`themes`] — theme-variable resolution across background /
//!   frame / connection / missing-var fall-through.
//! - [`clipping`] — connection glyph clipping in-node / in-frame /
//!   cap survival / cap clip.
//! - [`cache`] — `SceneConnectionCache` integration (hit, miss,
//!   drag stability, eviction, fold, selection stability).
//! - [`edge_handles`] — Session 6C edge-handle emission (straight /
//!   curved / cubic / canvas-absolute).
//! - [`labels`] — Session 6D connection-label emission (present /
//!   missing / position_t / color inheritance).
//! - [`portals`] — portal element emission (two-per-pair / missing
//!   endpoint / fold filter / theme var / selection highlight /
//!   anchor / drag offset).

mod fixtures;

mod cache;
mod clipping;
mod edge_handles;
mod labels;
mod point_inside;
mod portals;
mod themes;
