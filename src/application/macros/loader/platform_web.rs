// SPDX-License-Identifier: MPL-2.0

//! Web user-tier macro loader: URL `?macros=<urlencoded-json>`
//! query param > `localStorage` under the `mandala_macros` key >
//! empty. Not compiled on native.
//!
//! The query/storage reads, the size cap, and the fallback walk all
//! belong to `crate::application::user_config::web_storage` — the
//! keybinds and mutations web loaders name their layers against the
//! same driver, so the three cannot drift. Never panics: missing or
//! invalid sources are logged by the driver and the next layer is
//! tried.

use super::Macro;
use crate::application::user_config::web_storage::load_web_layered;

/// Load user macros on WASM with layered fallback. Same shape
/// (and same name) as the native sibling so the platform-routed
/// `pub use` at `super::load_user_macros` resolves on both targets.
pub fn load_user_macros() -> Vec<Macro> {
    match load_web_layered(
        "macros",
        "macros",
        "mandala_macros",
        super::parse_user_macros_json,
    ) {
        Some((v, source)) => {
            if !v.is_empty() {
                log::info!("macros: loaded {} user macro(s) from {}", v.len(), source);
            }
            v
        }
        None => Vec::new(),
    }
}
