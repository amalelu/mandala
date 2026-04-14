//! Web (WASM) config-source plumbing: URL `?keybinds=` query param +
//! `localStorage` fallback. Not compiled on native.

use log::warn;

use super::config::KeybindConfig;

impl KeybindConfig {
    /// Load a config on WASM, with layered fallback: URL `?keybinds=<json>`
    /// query param (inline JSON, URL-encoded) > `localStorage` value under
    /// the `mandala_keybinds` key > hardcoded defaults.
    pub fn load_for_web() -> Self {
        if let Some(json) = read_keybinds_from_query() {
            match Self::from_json(&json) {
                Ok(cfg) => {
                    log::info!("loaded keybinds from URL query param");
                    return cfg;
                }
                Err(e) => warn!("keybinds query param parse failed: {}", e),
            }
        }
        if let Some(json) = read_keybinds_from_local_storage() {
            match Self::from_json(&json) {
                Ok(cfg) => {
                    log::info!("loaded keybinds from localStorage");
                    return cfg;
                }
                Err(e) => warn!("keybinds localStorage parse failed: {}", e),
            }
        }
        Self::default()
    }
}

fn read_keybinds_from_query() -> Option<String> {
    let window = web_sys::window()?;
    let search = window.location().search().ok()?;
    // Expect format: "?keybinds=<url-encoded-json>" or
    // "?map=foo&keybinds=<url-encoded-json>"
    let trimmed = search.trim_start_matches('?');
    for pair in trimmed.split('&') {
        if let Some(val) = pair.strip_prefix("keybinds=") {
            // Manual URL-decode: replace + with space, percent-decode the rest.
            let decoded = js_sys::decode_uri_component(val).ok()?;
            return decoded.as_string();
        }
    }
    None
}

fn read_keybinds_from_local_storage() -> Option<String> {
    let window = web_sys::window()?;
    let storage = window.local_storage().ok()??;
    storage.get_item("mandala_keybinds").ok()?
}
