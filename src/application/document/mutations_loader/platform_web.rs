//! Web user-source plumbing: URL `?mutations=` query param +
//! `localStorage` fallback. Not compiled on native. Mirrors
//! `keybinds::platform_web` so a future session wiring up web-side
//! write-back has a consistent shape to extend.

use log::warn;

use baumhard::mindmap::custom_mutation::CustomMutation;

/// Load user mutations on WASM, with layered fallback: URL
/// `?mutations=<json>` query param > `localStorage` under the
/// `mandala_mutations` key > empty. Never fails — missing or invalid
/// sources are logged and the next layer is tried.
pub fn load_user() -> Vec<CustomMutation> {
    if let Some(json) = read_from_query() {
        match super::parse_mutations_json(&json) {
            Ok(v) => {
                log::info!("loaded {} user mutations from URL query param", v.len());
                return v;
            }
            Err(e) => warn!("mutations query param parse failed: {}", e),
        }
    }
    if let Some(json) = read_from_local_storage() {
        match super::parse_mutations_json(&json) {
            Ok(v) => {
                log::info!("loaded {} user mutations from localStorage", v.len());
                return v;
            }
            Err(e) => warn!("mutations localStorage parse failed: {}", e),
        }
    }
    Vec::new()
}

fn read_from_query() -> Option<String> {
    let window = web_sys::window()?;
    let search = window.location().search().ok()?;
    let trimmed = search.trim_start_matches('?');
    for pair in trimmed.split('&') {
        if let Some(val) = pair.strip_prefix("mutations=") {
            let decoded = js_sys::decode_uri_component(val).ok()?;
            return decoded.as_string();
        }
    }
    None
}

fn read_from_local_storage() -> Option<String> {
    let window = web_sys::window()?;
    let storage = window.local_storage().ok()??;
    storage.get_item("mandala_mutations").ok()?
}
