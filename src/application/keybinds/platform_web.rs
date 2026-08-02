// SPDX-License-Identifier: MPL-2.0

//! Web (WASM) config-source plumbing for keybinds: URL `?keybinds=`
//! query param > `localStorage` under the `mandala_keybinds` key >
//! hardcoded defaults. Not compiled on native.
//!
//! The query/storage reads, the size cap, and the fallback walk all
//! belong to `crate::application::user_config::web_storage` — the
//! mutations and macros web loaders name their layers against the
//! same driver, so the three cannot drift.

use super::config::KeybindConfig;
use crate::application::user_config::web_storage::load_web_layered;

impl KeybindConfig {
    /// Load a config on WASM, with layered fallback: URL `?keybinds=<json>`
    /// query param (inline JSON, URL-encoded) > `localStorage` value under
    /// the `mandala_keybinds` key > hardcoded defaults. Each layer's
    /// payload is bounded by `MAX_USER_PAYLOAD_BYTES` — an oversized
    /// blob is logged and skipped without invoking serde.
    pub fn load_for_web() -> Self {
        match load_web_layered("keybinds", "keybinds", "mandala_keybinds", Self::from_json) {
            Some((cfg, source)) => {
                log::info!("loaded keybinds from {}", source);
                cfg
            }
            None => Self::default(),
        }
    }
}
