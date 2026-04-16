//! Configurable keybindings.
//!
//! This module holds the configuration for every keyboard-driven action the
//! app supports, so users can rebind them without recompiling. The design
//! favors flexibility and forgiving loading:
//!
//! - Start with hardcoded defaults (`KeybindConfig::default`).
//! - Overlay a JSON config file if one is found or explicitly specified.
//! - On desktop, the file path is either supplied via CLI (`--keybinds
//!   <path>`) or looked up at a conventional location
//!   (`$XDG_CONFIG_HOME/mandala/keybinds.json`, or
//!   `$HOME/.config/mandala/keybinds.json`).
//! - On WASM, the config is read from a URL query param (`?keybinds=<json>`)
//!   or from `localStorage` under the key `mandala_keybinds`.
//! - Any failure to load a layer is logged and the layer is skipped — the
//!   app never crashes for a bad keybinds file.
//!
//! Partial configs are supported via serde's `default` attribute: an unset
//! field falls back to its hardcoded default, so a user can override a
//! single action without respecifying everything.
//!
//! Module split:
//! - [`action`] — the abstract `Action` enum dispatched by the event loop.
//! - [`bind`] — `KeyBind` parser/matcher + `normalize_key_name` /
//!   `key_to_name` shims.
//! - [`config`] — the user-editable `KeybindConfig` + JSON loader.
//! - [`resolved`] — the fast-lookup `ResolvedKeybinds` table.
//! - [`platform_desktop`] / [`platform_web`] — per-target config-source
//!   plumbing (cfg-gated).

mod action;
mod bind;
mod config;
mod context;
mod resolved;

#[cfg(not(target_arch = "wasm32"))]
mod platform_desktop;
#[cfg(target_arch = "wasm32")]
mod platform_web;

#[cfg(test)]
mod tests;

pub use action::Action;
// `normalize_key_name` and `KeyBind` are referenced from the in-crate
// test block (`tests`) and are part of the keybinds public surface
// for any future external consumer; cargo check (without --tests)
// flags them as unused, allow + retain.
#[allow(unused_imports)]
pub use bind::{key_to_name, normalize_key_name, KeyBind};
pub use config::KeybindConfig;
pub use context::InputContext;
pub use resolved::ResolvedKeybinds;

