// SPDX-License-Identifier: MPL-2.0

//! The layered-fallback driver every user-tier config loader runs on.
//!
//! Each user-tier config (`keybinds.json`, `mutations.json`,
//! `macros.json`) is looked up across an ordered list of candidate
//! sources — explicit CLI path before the XDG path on native, URL
//! query param before `localStorage` on web — and the first source
//! that yields a payload which fits the size cap *and* parses wins.
//! A source that is simply absent is skipped silently; a source that
//! is present but unreadable, oversized, or malformed is logged and
//! the walk continues. That policy is identical for every config and
//! every platform, so it lives here exactly once.
//!
//! The driver itself is platform-neutral: it only knows how to walk
//! [`ConfigLayer`]s. The filesystem-backed layers are built by
//! `super::desktop::load_desktop_layered` on top of
//! `super::read_capped`; the query-param / `localStorage` layers are
//! built by `super::web_storage::load_web_layered`. Both of those
//! wrappers are cfg-gated to their target, so this header names them
//! in plain backticks rather than intra-doc links. That split keeps
//! the precedence logic testable on native (TEST_CONVENTIONS §T9)
//! even though half the layers only exist in a browser.

use super::check_cap;

/// Result of asking one layer for its payload:
///
/// - `None` — the layer is not present at all (no CLI path given, no
///   such file, no query param). Not an error; the walk moves on
///   without logging.
/// - `Some(Err(msg))` — the layer is present but could not be turned
///   into a payload (stat failed, read failed, over the size cap).
///   Logged, then the walk moves on.
/// - `Some(Ok(payload))` — the raw JSON text for this layer.
pub type LayerPayload = Option<Result<String, String>>;

/// One candidate source in a layered user-config lookup.
///
/// `source` is the human-readable name the loader's log lines use —
/// a resolved path on native, `"URL query param"` / `"localStorage"`
/// on web. It is what [`load_layered`] hands back to the caller so
/// the caller's success log can name the winning layer.
///
/// `fetch` is deliberately lazy: a lower-precedence layer must not
/// touch the filesystem (or the JS storage backend) when a
/// higher-precedence layer already won.
pub struct ConfigLayer<'s, 'f> {
    /// Human-readable name of this source, used in log lines.
    pub source: &'s str,
    /// Lazy payload fetch. See [`LayerPayload`] for the tri-state.
    pub fetch: &'f dyn Fn() -> LayerPayload,
}

/// Walk `layers` in descending precedence and return the first
/// payload that clears the size cap and parses, along with the
/// `source` of the layer that produced it.
///
/// `label` names the config being loaded (`"keybinds"`,
/// `"mutations"`, `"macros"`) and prefixes every warning so a user
/// staring at a log can tell which file misbehaved. Every failure is
/// non-fatal: this is a startup path that must always produce
/// *something*, so the caller substitutes its own defaults when the
/// return is `None`.
///
/// Costs: one `fetch` call per layer until one wins, plus one
/// `parse` per layer that produced an in-cap payload. Nothing is
/// allocated by the driver itself beyond the warning strings on the
/// failing branches.
pub fn load_layered<'s, 'f, T, I>(
    label: &str,
    layers: I,
    parse: impl Fn(&str) -> Result<T, String>,
) -> Option<(T, &'s str)>
where
    I: IntoIterator<Item = ConfigLayer<'s, 'f>>,
{
    for layer in layers {
        let payload = match (layer.fetch)() {
            None => continue,
            Some(Err(e)) => {
                log::warn!("{}: {} load failed: {}", label, layer.source, e);
                continue;
            }
            Some(Ok(p)) => p,
        };
        // The cap is re-checked here even for filesystem layers,
        // which already stat-checked before reading: a web layer has
        // no stat to check against, so this is the only guard it
        // gets, and running it for both keeps one rule in one place.
        if let Err(e) = check_cap(payload.len() as u64) {
            log::warn!("{}: {} load failed: {}", label, layer.source, e);
            continue;
        }
        match parse(&payload) {
            Ok(v) => return Some((v, layer.source)),
            Err(e) => log::warn!("{}: {} parse failed: {}", label, layer.source, e),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::user_config::MAX_USER_PAYLOAD_BYTES;
    use std::cell::Cell;

    /// Trivial parser: accepts any payload that does not start with
    /// `!`, and returns it uppercased so the test can tell parsed
    /// output from raw payload.
    fn parse(src: &str) -> Result<String, String> {
        if src.starts_with('!') {
            Err("payload rejected by parser".to_string())
        } else {
            Ok(src.to_ascii_uppercase())
        }
    }

    #[test]
    fn test_layered_first_present_layer_wins() {
        let high = || Some(Ok("high".to_string()));
        let low = || Some(Ok("low".to_string()));
        let got = load_layered(
            "test",
            [
                ConfigLayer {
                    source: "high",
                    fetch: &high,
                },
                ConfigLayer {
                    source: "low",
                    fetch: &low,
                },
            ],
            parse,
        );
        assert_eq!(got, Some(("HIGH".to_string(), "high")));
    }

    #[test]
    fn test_layered_absent_layer_falls_through() {
        let high = || None;
        let low = || Some(Ok("low".to_string()));
        let got = load_layered(
            "test",
            [
                ConfigLayer {
                    source: "high",
                    fetch: &high,
                },
                ConfigLayer {
                    source: "low",
                    fetch: &low,
                },
            ],
            parse,
        );
        assert_eq!(got, Some(("LOW".to_string(), "low")));
    }

    #[test]
    fn test_layered_unreadable_layer_falls_through() {
        let high = || Some(Err("stat: no such file".to_string()));
        let low = || Some(Ok("low".to_string()));
        let got = load_layered(
            "test",
            [
                ConfigLayer {
                    source: "high",
                    fetch: &high,
                },
                ConfigLayer {
                    source: "low",
                    fetch: &low,
                },
            ],
            parse,
        );
        assert_eq!(got, Some(("LOW".to_string(), "low")));
    }

    #[test]
    fn test_layered_unparseable_layer_falls_through() {
        let high = || Some(Ok("!malformed".to_string()));
        let low = || Some(Ok("low".to_string()));
        let got = load_layered(
            "test",
            [
                ConfigLayer {
                    source: "high",
                    fetch: &high,
                },
                ConfigLayer {
                    source: "low",
                    fetch: &low,
                },
            ],
            parse,
        );
        assert_eq!(got, Some(("LOW".to_string(), "low")));
    }

    #[test]
    fn test_layered_oversized_layer_falls_through() {
        let blob = " ".repeat(MAX_USER_PAYLOAD_BYTES + 1);
        let high = || Some(Ok(blob.clone()));
        let low = || Some(Ok("low".to_string()));
        let got = load_layered(
            "test",
            [
                ConfigLayer {
                    source: "high",
                    fetch: &high,
                },
                ConfigLayer {
                    source: "low",
                    fetch: &low,
                },
            ],
            parse,
        );
        assert_eq!(got, Some(("LOW".to_string(), "low")));
    }

    #[test]
    fn test_layered_all_layers_exhausted_returns_none() {
        let high = || None;
        let low = || Some(Err("read: permission denied".to_string()));
        let got = load_layered(
            "test",
            [
                ConfigLayer {
                    source: "high",
                    fetch: &high,
                },
                ConfigLayer {
                    source: "low",
                    fetch: &low,
                },
            ],
            parse,
        );
        assert_eq!(got, None);
    }

    /// A winning layer must short-circuit the walk — a lower layer's
    /// `fetch` must never run, or every startup would pay for an
    /// XDG stat it does not need.
    #[test]
    fn test_layered_lower_layer_is_not_fetched_when_higher_wins() {
        let low_hits = Cell::new(0u32);
        let high = || Some(Ok("high".to_string()));
        let low = || {
            low_hits.set(low_hits.get() + 1);
            Some(Ok("low".to_string()))
        };
        let got = load_layered(
            "test",
            [
                ConfigLayer {
                    source: "high",
                    fetch: &high,
                },
                ConfigLayer {
                    source: "low",
                    fetch: &low,
                },
            ],
            parse,
        );
        assert_eq!(got, Some(("HIGH".to_string(), "high")));
        assert_eq!(low_hits.get(), 0, "lower layer must not be fetched");
    }

    /// The layer list is walked in the order given — the driver
    /// never reorders, so precedence is entirely the caller's
    /// declaration order.
    #[test]
    fn test_layered_empty_layer_list_returns_none() {
        let got = load_layered("test", Vec::<ConfigLayer<'_, '_>>::new(), parse);
        assert!(got.is_none());
    }
}
