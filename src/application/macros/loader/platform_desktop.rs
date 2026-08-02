// SPDX-License-Identifier: MPL-2.0

//! Native user-tier macro loader. Reads
//! `$XDG_CONFIG_HOME/mandala/macros.json` (fallback
//! `~/.config/mandala/macros.json`) and parses through the shared
//! [`super::parse_user_macros_json`].
//!
//! The read, the size cap, the layer construction, and the fallback
//! walk all belong to [`crate::application::user_config`]; this
//! module only names the file and the parser. The keybinds and
//! mutations desktop loaders name theirs the same way against the
//! same wrapper, which is also where the shared resilience posture
//! lives: the app boots with an empty user tier when the file is
//! absent or malformed, and warns on failure so the user notices.

use super::Macro;
use crate::application::user_config::load_desktop_layered;

/// Load the user-layer macros. Tier: `SourceTier::User`, assigned
/// at the call site in `run_native_init::build`.
///
/// Macros have no CLI override, so the explicit layer is `None` and
/// the XDG path is the only candidate — the one place this loader
/// differs from its keybinds / mutations peers.
///
/// Returns an empty `Vec` when the file is absent, oversized, or
/// malformed; failures log a warning so users notice but the app
/// still boots. Files larger than `MAX_USER_PAYLOAD_BYTES` are
/// rejected before `read_to_string` runs.
pub fn load_user_macros() -> Vec<Macro> {
    match load_desktop_layered("macros", "macros.json", None, super::parse_user_macros_json) {
        Some((v, source)) => {
            if !v.is_empty() {
                log::info!("macros: loaded {} user macro(s) from {}", v.len(), source);
            }
            v
        }
        None => Vec::new(),
    }
}
