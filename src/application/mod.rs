// SPDX-License-Identifier: MPL-2.0

//! Application-crate root. The three load-bearing modules
//! follow the model/view/dispatch boundary documented in
//! `CODE_CONVENTIONS.md §3`:
//!
//! - [`document`] — `MindMapDocument` + mutation / undo cascade.
//!   Pure data + transformations, no GPU.
//! - [`renderer`] — wgpu pipelines + cosmic-text integration.
//!   Pure presentation, no document mutation.
//! - [`app`] — event loop, modal state machines, and the
//!   single dispatch funnel every user-driven `Action` flows
//!   through (§3 "Single dispatch funnel").
//!
//! Orthogonal subsystems land at the dispatch / authoring
//! seams rather than under any one of the three above:
//!
//! - [`keybinds`] — config-loaded `Action` table, the input
//!   layer the funnel resolves against.
//! - [`console`] — the verb parser + completion + execution
//!   layer (modal-input carve-out per §3).
//! - [`macros`] — multi-step replay registry; tier-gated
//!   privilege model so untrusted-source macros can't sneak
//!   destructive verbs past the funnel.
//! - [`color_picker`] / [`color_picker_overlay`] — glyph-wheel
//!   HSV picker (modal-input carve-out).
//! - [`clipboard`] — cross-platform copy / paste plumbing.
//! - [`scene_host`] — canvas vs overlay scene-tree slot
//!   routing (the boundary between renderer and the modal
//!   editors).
//! - [`source_tier`] — the `App < User < Map < Inline`
//!   provenance ladder both the mutation and macro registries
//!   resolve and gate against.
//! - [`user_config`] — XDG / web-storage shared loader
//!   plumbing for keybinds / mutations / macros.
//! - [`frame_throttle`] — per-frame cap math.
//! - [`common`] — small shared types: `RedrawMode`,
//!   `RenderDecree`, `FpsDisplayMode`, `PollTimer`,
//!   `StopWatch`.
//!
//! The crate's binary entry point (`src/main.rs`) constructs
//! [`app::Application`] and calls [`app::Application::run`];
//! everything else hangs off that.

pub(crate) mod app;
pub(crate) mod clipboard;
pub(crate) mod color_picker;
pub(crate) mod color_picker_overlay;
pub(crate) mod common;
pub(crate) mod console;
pub(crate) mod document;
pub(crate) mod frame_throttle;
pub(crate) mod keybinds;
pub(crate) mod macros;
/// Platform-input value types — the inward seam over winit's
/// `Key` / `Modifiers` / `MouseButton` / `CursorIcon` / etc. so
/// only the bootstrap (`app/run_native.rs`, `app/run_wasm/`)
/// imports `winit::*`.
pub(crate) mod platform;
pub mod renderer;
pub(crate) mod scene_host;
pub(crate) mod source_tier;
pub(crate) mod user_config;
pub(crate) mod widgets;
