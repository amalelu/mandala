// SPDX-License-Identifier: MPL-2.0

//! Size-capped filesystem read for user-tier config files.
//!
//! Native-only, for the same reason `xdg` is: the filesystem tier of
//! a user config exists only on desktop. The browser's peer tiers
//! (`?<name>=` query param, `localStorage`) live in `web_storage`
//! and reach the same cap through [`super::check_cap`], so both
//! targets enforce one rule from one place.
//!
//! The cap is checked against the file's stat size *before*
//! `read_to_string` runs — the whole point is not to allocate a
//! multi-megabyte `String` and hand it to serde. The in-memory
//! re-check that [`super::layered::load_layered`] performs is the
//! web tier's guard, and is a no-op for anything this function
//! returns.

use std::path::Path;

use super::check_cap;

/// Read `path` into a `String`, refusing anything over
/// [`super::MAX_USER_PAYLOAD_BYTES`].
///
/// Errors are terse and path-free — `"stat: ..."`, `"read: ..."`,
/// or the shared cap message — because the caller already names the
/// file through its [`super::layered::ConfigLayer::source`], and
/// repeating the path in both halves of the log line reads as a
/// stutter.
///
/// Costs: one `stat` syscall, then one full read of the file on the
/// success path. Nothing is read when the cap rejects.
pub fn read_capped(path: &Path) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("stat: {}", e))?;
    check_cap(meta.len())?;
    std::fs::read_to_string(path).map_err(|e| format!("read: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    use baumhard::util::test_temp::TempDir;

    use crate::application::user_config::MAX_USER_PAYLOAD_BYTES;

    #[test]
    fn test_read_capped_small_file_round_trips() {
        let scratch = TempDir::new("read-capped-small");
        let p = scratch.join("small.json");
        std::fs::write(&p, "{\"a\": 1}").unwrap();
        assert_eq!(read_capped(&p).unwrap(), "{\"a\": 1}");
    }

    /// A file of exactly `MAX_USER_PAYLOAD_BYTES` is *within* the
    /// cap — the comparison is `>`, not `>=`. Pinned because an
    /// off-by-one here silently rejects the largest legal config.
    #[test]
    fn test_read_capped_file_exactly_at_cap_is_accepted() {
        let scratch = TempDir::new("read-capped-exact");
        let p = scratch.join("exact.json");
        std::fs::write(&p, vec![b' '; MAX_USER_PAYLOAD_BYTES]).unwrap();
        let got = read_capped(&p).expect("a file exactly at the cap must be readable");
        assert_eq!(got.len(), MAX_USER_PAYLOAD_BYTES);
    }

    /// One byte over the cap is rejected, and rejected *before* the
    /// read — the error text is the shared cap message, not a read
    /// error.
    #[test]
    fn test_read_capped_file_one_byte_over_cap_is_rejected() {
        let scratch = TempDir::new("read-capped-over");
        let p = scratch.join("over.json");
        std::fs::write(&p, vec![b' '; MAX_USER_PAYLOAD_BYTES + 1]).unwrap();
        let err = read_capped(&p).expect_err("one byte over the cap must be rejected");
        assert!(
            err.contains("exceeds size cap"),
            "expected the shared cap message, got: {}",
            err
        );
    }

    #[test]
    fn test_read_capped_missing_file_reports_stat_failure() {
        // The scratch directory exists and is empty, so the file is
        // absent by construction — nothing to clean up first.
        let scratch = TempDir::new("read-capped-missing");
        let p = scratch.join("definitely-absent.json");
        let err = read_capped(&p).expect_err("a missing file must be an error");
        assert!(err.starts_with("stat: "), "expected a stat error, got: {}", err);
    }

    /// A path that stats fine but cannot be read as text: a
    /// directory. Chosen over a chmod-000 file because the suite
    /// runs as root in CI containers, where permission bits are not
    /// enforced — this case fails identically for every uid.
    #[test]
    fn test_read_capped_unreadable_path_reports_read_failure() {
        let scratch = TempDir::new("read-capped-directory");
        let p = scratch.join("as-a-directory");
        std::fs::create_dir_all(&p).unwrap();
        let err = read_capped(&p).expect_err("a directory must not read as config text");
        assert!(err.starts_with("read: "), "expected a read error, got: {}", err);
    }
}
