// SPDX-License-Identifier: MPL-2.0

//! WASM-side `?<name>=` query-param + `localStorage` access. The
//! three user-tier JSON loaders (keybinds, mutations, macros) all
//! reach for the same shape: pull the named string from the URL,
//! fall back to a localStorage key. Same shape; same machinery —
//! [`load_web_layered`] is that shape, wired onto the shared
//! [`super::layered::load_layered`] driver.

use super::{load_layered, ConfigLayer};

/// Name the query-param layer answers to in log lines.
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

/// Read a URL query parameter by name, percent-decoding the value.
/// Returns `None` if `web_sys::window()`/location is missing, the
/// query string is empty, the param is absent, or the decode fails.
/// Recognizes `?name=value` and `&name=value` (interior).
///
/// Allocates the formatted prefix and the decoded `String` on the
/// success path. O(n) over the query-string length.
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
