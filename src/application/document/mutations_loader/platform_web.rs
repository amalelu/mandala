// SPDX-License-Identifier: MPL-2.0

//! Web user-source plumbing for custom mutations: URL
//! `?mutations=` query param > `localStorage` under the
//! `mandala_mutations` key > empty. Not compiled on native.
//!
//! The query/storage reads, the size cap, and the fallback walk all
//! belong to `crate::application::user_config::web_storage` — the
//! keybinds and macros web loaders name their layers against the
//! same driver, so the three cannot drift.

use baumhard::mindmap::custom_mutation::CustomMutation;

use crate::application::user_config::web_storage::load_web_layered;

/// Load user mutations on WASM, with layered fallback: URL
/// `?mutations=<json>` query param > `localStorage` under the
/// `mandala_mutations` key > empty. Never fails — missing or invalid
/// sources are logged and the next layer is tried.
pub fn load_user() -> Vec<CustomMutation> {
    match load_web_layered(
        "mutations",
        "mutations",
        "mandala_mutations",
        super::parse_mutations_json,
    ) {
        Some((v, source)) => {
            log::info!("loaded {} user mutations from {}", v.len(), source);
            v
        }
        None => Vec::new(),
    }
}
