//! Platform clipboard abstraction — thin wrapper over the system
//! clipboard so trait methods stay pure data transformations and the
//! event loop does the I/O at the boundary.
//!
//! Native builds use `arboard`; WASM stubs log and return nothing
//! (the browser clipboard API is async and requires a separate
//! integration pass).

/// Read text from the system clipboard. Returns `None` if the
/// clipboard is empty, inaccessible, or not supported on this
/// platform.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_clipboard() -> Option<String> {
    arboard::Clipboard::new()
        .ok()
        .and_then(|mut cb| cb.get_text().ok())
        .filter(|s| !s.is_empty())
}

/// Write text to the system clipboard. Silently ignores failures —
/// clipboard access can be denied by the OS or window manager, and
/// the interactive path must not panic (CODE_CONVENTIONS §7).
#[cfg(not(target_arch = "wasm32"))]
pub fn write_clipboard(text: &str) {
    if let Ok(mut cb) = arboard::Clipboard::new() {
        let _ = cb.set_text(text);
    }
}

#[cfg(target_arch = "wasm32")]
pub fn read_clipboard() -> Option<String> {
    log::debug!("clipboard read not yet supported on WASM");
    None
}

#[cfg(target_arch = "wasm32")]
pub fn write_clipboard(_text: &str) {
    log::debug!("clipboard write not yet supported on WASM");
}
