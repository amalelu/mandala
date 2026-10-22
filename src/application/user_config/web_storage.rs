// SPDX-License-Identifier: MPL-2.0

//! WASM-side `?<name>=` query-param + `localStorage` access. The
//! three user-tier JSON loaders (keybinds, mutations, macros) all
//! reach for the same shape: pull the named string from the URL,
//! fall back to a localStorage key. Same shape; same machinery —
//! [`load_web_layered`] is that shape, wired onto the shared
//! [`super::layered::load_layered`] driver.
//!
//! **The query-param layer is built but not reached.** All three
//! browser loaders (`keybinds::platform_web`,
//! `macros::loader::platform_web`,
//! `document::mutations_loader::platform_web`) call
//! [`load_web_storage_only`], so `?keybinds=<json>` and its siblings
//! do nothing today. #41 found the two-layer chain dead and kept it
//! rather than delete a working implementation: switching the three
//! call sites over is a decision about what URL surface the app
//! offers, not a cleanup. Until then this module documents a
//! capability the app does not have, and that is recorded here
//! rather than left for the next reader to discover.

use super::{load_layered, ConfigLayer};

/// Name the query-param layer answers to in log lines. Unreached —
/// see the module header.
#[allow(dead_code)]
const QUERY_PARAM_SOURCE: &str = "URL query param";

/// Name the `localStorage` layer answers to in log lines.
const LOCAL_STORAGE_SOURCE: &str = "localStorage";

/// The browser's user-tier fallback chain: `?<param_name>=<json>`
/// first, then `localStorage[storage_key]`, then nothing.
///
/// Returns the parsed value plus the name of the layer that produced
/// it, so the caller's success log can say where the config came
/// from. `None` means no layer produced anything usable; every
/// failure along the way has already been logged by the shared
/// driver, so the caller only has to substitute its defaults.
///
/// Costs: at most one `window.location` read and one `localStorage`
/// round-trip, both skipped once a higher layer wins.
/// Unreached today — see the module header. The consumers in
/// waiting are the three `platform_web` loaders, each of which calls
/// [`load_web_storage_only`] instead.
#[allow(dead_code)]
pub fn load_web_layered<T>(
    label: &str,
    param_name: &str,
    storage_key: &str,
    parse: impl Fn(&str) -> Result<T, String>,
) -> Option<(T, &'static str)> {
    let query = || read_query_param(param_name).map(Ok);
    let storage = || read_local_storage(storage_key).map(Ok);
    load_layered(
        label,
        [
            ConfigLayer {
                source: QUERY_PARAM_SOURCE,
                fetch: &query,
            },
            ConfigLayer {
                source: LOCAL_STORAGE_SOURCE,
                fetch: &storage,
            },
        ],
        parse,
    )
}

/// The browser's fallback chain **without** the query-param layer:
/// `localStorage[storage_key]`, then nothing.
///
/// **A URL query param is not the user's own configuration.** The
/// trusted [`SourceTier::User`](crate::application::source_tier::SourceTier::User)
/// is justified by "the user owns the file" — true of an XDG config
/// on native, and true of `localStorage`, which only the user's own
/// browser session writes. It is *not* true of `?name=<json>`, which
/// is owned by whoever composed the link.
///
/// That distinction did not exist, and on the web the same URL carried
/// both halves of the trust boundary: `?map=` supplied the document at
/// the untrusted `Map` tier while `?macros=` supplied macros at the
/// fully trusted `User` tier — where
/// [`SourceTier::allows_console_line`](crate::application::source_tier::SourceTier::allows_console_line)
/// permits `MacroStep::ConsoleLine`, an arbitrary console verb, and
/// `allows_action` permits every destructive `Action`. One transport,
/// two trust levels, decided by nothing but which parameter name the
/// sender chose.
///
/// So the configs that can *execute* use this chain. The blast radius
/// was bounded by the browser sandbox — no filesystem, no process —
/// but "anyone who can send you a link can run editor commands against
/// your open document" is not a boundary worth keeping.
pub fn load_web_storage_only<T>(
    label: &str,
    storage_key: &str,
    parse: impl Fn(&str) -> Result<T, String>,
) -> Option<(T, &'static str)> {
    let storage = || read_local_storage(storage_key).map(Ok);
    load_layered(
        label,
        [ConfigLayer {
            source: LOCAL_STORAGE_SOURCE,
            fetch: &storage,
        }],
        parse,
    )
}

/// Read a URL query parameter by name, percent-decoding the value.
/// Returns `None` if `web_sys::window()`/location is missing, the
/// query string is empty, the param is absent, or the decode fails.
/// Recognizes `?name=value` and `&name=value` (interior).
///
/// Allocates the formatted prefix and the decoded `String` on the
/// success path. O(n) over the query-string length.
#[allow(dead_code)]
pub fn read_query_param(name: &str) -> Option<String> {
    let window = web_sys::window()?;
    let search = window.location().search().ok()?;
    let trimmed = search.trim_start_matches('?');
    let prefix = format!("{}=", name);
    for pair in trimmed.split('&') {
        if let Some(val) = pair.strip_prefix(prefix.as_str()) {
            return js_sys::decode_uri_component(val).ok()?.as_string();
        }
    }
    None
}

/// Read a value from the browser's `localStorage` by key. Returns
/// `None` if the storage API is unavailable (cookies disabled,
/// sandboxed iframe, etc.) or the key is unset.
///
/// O(1); one round-trip into the JS storage backend.
pub fn read_local_storage(key: &str) -> Option<String> {
    let window = web_sys::window()?;
    let storage = window.local_storage().ok()??;
    storage.get_item(key).ok()?
}
