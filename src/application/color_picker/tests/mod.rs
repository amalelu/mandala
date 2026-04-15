//! Picker unit tests, sharded by subject so each file stays small
//! and greppable. Shared fixtures live in [`fixtures`].
//!
//! - [`layout`] — position math (hue ring circle, sat/val bars,
//!   preview centering, center override, ink offsets, overlap).
//! - [`hit`] — `hit_test_picker` outcomes across the visible regions.
//! - [`cell_math`] — hue/sat/val cell ↔ value round-trips, hue wrap.
//! - [`crosshair`] — crosshair arm geometry, per-cell ink correction,
//!   symmetric advance.
//! - [`glyphs`] — hue-ring + arm glyph script grouping.
//! - [`sizing`] — screen-short-side targeting, size_scale monotonicity,
//!   safety clamp on tiny screens.
//! - [`hex`] — hex readout visibility + centering.
//! - [`state`] — channel ordering, dynamic-apply short-circuit key,
//!   resize-gesture math.

mod fixtures;

mod cell_math;
mod crosshair;
mod glyphs;
mod hex;
mod hit;
mod layout;
mod sizing;
mod state;
