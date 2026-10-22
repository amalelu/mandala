// SPDX-License-Identifier: MPL-2.0

//! The wasm32 side of `tests/wasm_font_lock.rs`: a `cdylib` that
//! exports one function per scenario, each calling baumhard's real
//! font-lock helpers, so a node runner can execute them in a real
//! WebAssembly engine.
//!
//! This file is not part of any cargo target in the repository. The
//! test copies it into a staging crate under `target/` and builds it
//! there, against the workspace's own `Cargo.lock`, so it compiles the
//! dependency graph the app ships rather than a second one that could
//! drift.
//!
//! Every export returns `1` on success. The `reentrant_*` exports are
//! *meant* to panic: on `wasm32-unknown-unknown` a panic aborts as a
//! trap, which says nothing about its cause, so the hook below leaves
//! the message where the host can read it. The runner decides pass or
//! fail from that message, not from the trap: a trap on its own is
//! exactly what `Instant::now()` produces too, so only the message
//! tells a deliberate panic from a bug.
//!
//! **Plain safe Rust, by design.** Baumhard keeps its source free of
//! the keyword this file would otherwise need for a host callback
//! (`lib/baumhard/CONVENTIONS.md`), and a probe that lives under
//! `lib/baumhard/tests/` is held to the same rule. So there is no host
//! import to call. The hook writes the message into a static buffer,
//! the module exports the *addresses* of that buffer and its length,
//! and the host reads linear memory directly. It fetches the two
//! addresses before it runs a scenario, so it never has to call into
//! an instance that has already trapped.

use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::Once;

use baumhard::font::fonts::{acquire_font_system_read, acquire_font_system_write, try_acquire_font_system_write};

/// Longest panic message kept. Longer ones are truncated; every message
/// the runner looks for is in the first line.
const MESSAGE_CAP: usize = 1024;

/// The last panic's message. `AtomicU8` has the same in-memory
/// representation as `u8`, which is what lets the host read these as
/// plain bytes.
static MESSAGE: [AtomicU8; MESSAGE_CAP] = [const { AtomicU8::new(0) }; MESSAGE_CAP];
static MESSAGE_LEN: AtomicUsize = AtomicUsize::new(0);

fn record_panic(message: &str) {
    let bytes = message.as_bytes();
    let kept = bytes.len().min(MESSAGE_CAP);
    for (slot, byte) in MESSAGE.iter().zip(bytes) {
        slot.store(*byte, Ordering::Relaxed);
    }
    MESSAGE_LEN.store(kept, Ordering::Relaxed);
}

/// Where the host finds the panic message bytes in linear memory.
#[no_mangle]
pub extern "C" fn panic_message_addr() -> usize {
    MESSAGE.as_ptr() as usize
}

/// Where the host finds the message's length (a `usize`, 4 bytes here).
#[no_mangle]
pub extern "C" fn panic_message_len_addr() -> usize {
    std::ptr::from_ref(&MESSAGE_LEN) as usize
}

fn install_hook() {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|info| record_panic(&info.to_string())));
    });
}

/// Take the write guard, use it, release it, take it again. The second
/// acquire proves the release cleared the thread's holding record, and
/// the deref proves the guard type reaches the `FontSystem`.
#[no_mangle]
pub extern "C" fn uncontended_write() -> u32 {
    install_hook();
    let first = acquire_font_system_write("probe::first");
    let _faces = first.db().len();
    drop(first);
    let second = acquire_font_system_write("probe::second");
    drop(second);
    1
}

#[no_mangle]
pub extern "C" fn uncontended_try_write() -> u32 {
    install_hook();
    u32::from(try_acquire_font_system_write("probe::try").is_some())
}

/// Two nested reads on one thread are legal, and are the case a plain
/// (non-recursive) read could deadlock behind a queued writer.
#[no_mangle]
pub extern "C" fn uncontended_read() -> u32 {
    install_hook();
    let outer = acquire_font_system_read("probe::outer");
    let inner = acquire_font_system_read("probe::inner");
    let _faces = inner.db().len();
    drop(inner);
    drop(outer);
    1
}

#[no_mangle]
pub extern "C" fn reentrant_write_write() -> u32 {
    install_hook();
    let _outer = acquire_font_system_write("probe::outer");
    let _inner = acquire_font_system_write("probe::inner");
    0
}

#[no_mangle]
pub extern "C" fn reentrant_write_try_write() -> u32 {
    install_hook();
    let _outer = acquire_font_system_write("probe::outer");
    let _inner = try_acquire_font_system_write("probe::inner");
    0
}

#[no_mangle]
pub extern "C" fn reentrant_write_read() -> u32 {
    install_hook();
    let _outer = acquire_font_system_write("probe::outer");
    let _inner = acquire_font_system_read("probe::inner");
    0
}

#[no_mangle]
pub extern "C" fn reentrant_read_write() -> u32 {
    install_hook();
    let _outer = acquire_font_system_read("probe::outer");
    let _inner = acquire_font_system_write("probe::inner");
    0
}
