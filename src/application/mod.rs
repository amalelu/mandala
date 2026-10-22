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
//! - `ipc` — the opt-in `--ipc` socket an agent drives the
//!   running app through (native-only, so a plain code-span).
//! - [`scene_host`] — canvas vs overlay scene-tree slot
//!   routing (the boundary between renderer and the modal
//!   editors).
//! - [`source_tier`] — the `App < User < Map < Inline`
//!   provenance ladder both the mutation and macro registries
//!   resolve and gate against.
//! - [`user_config`] — XDG / web-storage shared loader
//!   plumbing for keybinds / mutations / macros.
//! - [`frame_throttle`] — per-frame cap math.
//! - [`common`] — small shared types: `RenderDecree`,
//!   `FpsDisplayMode`, and the `now_ms` clock.
//!
//! The crate's binary entry point (`src/main.rs`) constructs
//! [`app::Application`] and calls [`app::Application::run`];
//! everything else hangs off that.

// Five subsystems below carry a wasm32-only `allow(dead_code)`.
// Each is cross-platform *code* whose *driver* is native-gated (see
// CLAUDE.md "Dual-target status"), so on `wasm32-unknown-unknown`
// the module compiles with nothing calling into it and the
// dead-code lint fires on essentially all of it. The allow is
// scoped to wasm32 so the host lint — the one that can still see
// the driver — stays fully armed, and it sits at the module
// boundary rather than on hundreds of items because the answer is
// the same for every item and stops being true for all of them at
// once, when parity lands.

pub(crate) mod app;
pub(crate) mod clipboard;
// Picker layout math + state. Driver: `app::color_picker_flow`,
// native-gated pending the browser modal shell.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) mod color_picker;
// Picker glyph-area builders. Same driver, same gate, as
// [`color_picker`].
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) mod color_picker_overlay;
pub(crate) mod common;
// Console verbs, parser, completion. Driver: the modal shell in
// `app::console_input`, native-gated; parity is the named next
// step in CONCEPTS §6 "Console".
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) mod console;
pub(crate) mod document;
// Local socket control surface for agents and tooling. Native-only by
// transport nature — browsers cannot serve local sockets; see
// CLAUDE.md "Dual-target status".
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod ipc;
// Adaptive drain-rate math. Driver: `app::throttled_interaction`,
// native-gated with the drag state machine it paces.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
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
// Widget spec data (today: the color picker's geometry spec).
// Driver: [`color_picker`], native-gated.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) mod widgets;
