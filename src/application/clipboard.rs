// SPDX-License-Identifier: MPL-2.0

//! Platform clipboard abstraction. Native uses `arboard`; WASM is a
//! logged stub pending an async-browser integration.
//!
//! Also owns the in-process **structured section buffer**: the OS
//! clipboard carries the plain section text (so cross-app paste
//! sees something sensible); a thread-local single-slot buffer
//! carries the full `SectionPayload` so within-app section→section
//! paste round-trips per-run formatting and section chrome.
//! Read consults the buffer only when the probe text matches the
//! buffer's snapshot — guards against the user copying from
//! another app between Mandala copy and paste.
//!
//! Thread-local rather than a `static Mutex<…>` because the
//! editor's event loop is single-threaded; each parallel `cargo
//! test` worker also gets its own slot, which removes the
//! cross-thread race a global would otherwise have.

use crate::application::document::SectionPayload;
use std::cell::RefCell;

/// Read text from the system clipboard, or `None` on failure.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_clipboard() -> Option<String> {
    arboard::Clipboard::new()
        .ok()
        .and_then(|mut cb| cb.get_text().ok())
        .filter(|s| !s.is_empty())
}

/// Write text to the system clipboard; ignores failures (interactive
/// paths must not panic — §9).
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

struct SectionBufferEntry {
    text: String,
    payload: SectionPayload,
}

thread_local! {
    static SECTION_BUFFER: RefCell<Option<SectionBufferEntry>> = const { RefCell::new(None) };
}

/// Stash a structured section payload alongside the OS clipboard's
/// plain text. `text` must be byte-equal to what the caller wrote
/// to the OS clipboard for `read_section_clipboard` to ever return
/// the payload.
pub fn write_section_clipboard(text: String, payload: SectionPayload) {
    SECTION_BUFFER.with(|slot| {
        *slot.borrow_mut() = Some(SectionBufferEntry { text, payload });
    });
}

/// Drop the in-process structured section buffer. Called by
/// `prepare_copy_or_cut` before iterating (a stale single-section
/// payload from a prior copy would otherwise win the byte-equal
/// probe on the next paste — silently substituting the OS
/// clipboard's joined blob with one section's structured
/// content), and by tests between cases (parallel `cargo test`
/// workers reuse threads, so a prior test's seed would
/// otherwise leak into a later one's consistency-check probe).
pub fn clear_section_clipboard() {
    SECTION_BUFFER.with(|slot| *slot.borrow_mut() = None);
}

/// Return the buffered payload only when its text snapshot
/// matches `probe_text` exactly. Mismatch (or empty buffer)
/// returns `None`, leaving the caller to fall back to plain text.
pub fn read_section_clipboard(probe_text: &str) -> Option<SectionPayload> {
    SECTION_BUFFER.with(|slot| {
        let slot = slot.borrow();
        slot.as_ref()
            .and_then(|entry| (entry.text == probe_text).then(|| entry.payload.clone()))
    })
}
