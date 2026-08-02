// SPDX-License-Identifier: MPL-2.0

//! Shared loader scaffolding for user-tier JSON config files
//! (`keybinds.json`, `mutations.json`, `macros.json`).
//!
//! Every one of them answers the same three questions the same way,
//! so each answer lives here exactly once:
//!
//! - *How big may a payload be?* [`MAX_USER_PAYLOAD_BYTES`], enforced
//!   through [`check_cap`] — the single place the cap message is
//!   worded.
//! - *How is one candidate source turned into text?*
//!   `capped_read::read_capped` for the filesystem tier (native),
//!   `web_storage::read_query_param` / `read_local_storage` for the
//!   browser tiers.
//! - *How do candidate sources compose into a fallback chain?*
//!   [`layered::load_layered`] — with the same "absent is silent,
//!   broken is logged, first good one wins" policy on either target.
//!   Each platform names its own layers exactly once, in the
//!   composition wrapper the driver sits under:
//!   `desktop::load_desktop_layered` (explicit CLI path, then the XDG
//!   path) and `web_storage::load_web_layered` (URL query param, then
//!   `localStorage`).
//!
//! Adding a fourth user-tier config file is therefore a matter of
//! naming its filename / query-param / storage key and handing the
//! wrapper a parser; no new read, cap, or fallback code, and no new
//! layer construction.
//!
//! Native path resolution is `xdg::xdg_mandala_path`
//! (`$XDG_CONFIG_HOME/mandala/<file>.json`, or the `$HOME/.config`
//! fallback); the browser equivalents live in `web_storage`. The
//! per-target modules named above are all cfg-gated, so this header
//! uses plain backticks rather than intra-doc links for whichever
//! ones are not compiled.

#[cfg(not(target_arch = "wasm32"))]
pub mod capped_read;

#[cfg(not(target_arch = "wasm32"))]
pub mod desktop;

pub mod layered;

#[cfg(not(target_arch = "wasm32"))]
pub mod xdg;

#[cfg(target_arch = "wasm32")]
pub mod web_storage;

#[cfg(not(target_arch = "wasm32"))]
pub use capped_read::read_capped;
#[cfg(not(target_arch = "wasm32"))]
pub use desktop::load_desktop_layered;
pub use layered::{load_layered, ConfigLayer};

/// Upper bound on a user-tier JSON payload (file or web string),
/// in bytes. Real keybind / mutation / macro files are small —
/// a few KB at the high end. A multi-MB input is almost certainly
/// accidental or hostile, and loading it into memory + running serde
/// over it is wasted work. 1 MiB is generous (~1000x the largest
/// real file we ship).
pub const MAX_USER_PAYLOAD_BYTES: usize = 1 << 20;

/// The one place the size cap is worded and enforced.
///
/// Returns `Ok(())` when `byte_len` is within
/// [`MAX_USER_PAYLOAD_BYTES`] — the boundary itself is legal, the
/// comparison is `>` — and otherwise an `Err` the caller logs.
/// Takes `u64` rather than `usize` so a filesystem stat size feeds
/// in without a lossy cast on 32-bit targets.
///
/// It is deliberately silent: [`layered::load_layered`] owns the
/// warning, so it can name both the config and the layer that
/// tripped the cap rather than emitting a message with no context.
///
/// O(1); allocates the message only on the failing branch.
pub fn check_cap(byte_len: u64) -> Result<(), String> {
    if byte_len > MAX_USER_PAYLOAD_BYTES as u64 {
        Err(format!(
            "exceeds size cap ({} bytes > {} max); refusing to load",
            byte_len, MAX_USER_PAYLOAD_BYTES,
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_cap_accepts_exactly_the_cap() {
        assert!(check_cap(MAX_USER_PAYLOAD_BYTES as u64).is_ok());
    }

    #[test]
    fn test_check_cap_rejects_one_byte_over() {
        let err = check_cap(MAX_USER_PAYLOAD_BYTES as u64 + 1).unwrap_err();
        assert!(err.contains("exceeds size cap"), "unexpected message: {}", err);
    }

    #[test]
    fn test_check_cap_accepts_empty_payload() {
        assert!(check_cap(0).is_ok());
    }
}
