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

use crate::application::user_config::web_storage::load_web_storage_only;

/// Load user mutations on WASM: `localStorage` under the
/// `mandala_mutations` key, then empty. Never fails — a missing or
/// invalid source is logged and the default is used.
///
/// **No `?mutations=` layer**, for the reason given on
/// [`load_web_storage_only`]:
/// a query param is owned by whoever composed the link, so it must not
/// reach `SourceTier::User`. A user-tier custom mutation outranks the
/// map's own by precedence and is not screened by the macro-step
/// gates, which govern `MacroStep`s rather than mutations.
pub fn load_user() -> Vec<CustomMutation> {
    match load_web_storage_only("mutations", "mandala_mutations", super::parse_mutations_json) {
        Some((v, source)) => {
            log::info!("loaded {} user mutations from {}", v.len(), source);
            v
        }
        None => Vec::new(),
    }
}
