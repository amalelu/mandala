//! Native-only main-loop watchdog. A background thread wakes
//! periodically and checks whether the main thread has pinged its
//! liveness atomic recently; if the main thread has been silent
//! longer than [`FREEZE_THRESHOLD`], the watchdog prints a
//! diagnostic banner and aborts the process.
//!
//! # Why it earns its keep
//!
//! Mandala is single-threaded by design (see `CLAUDE.md`'s
//! "Architectural shape" section). That invariant covers app state
//! — the model, the tree, the scene, the renderer — but it does not
//! protect against the app's main thread becoming permanently
//! blocked: a same-thread re-entrant `std::sync::RwLock` acquire
//! hangs forever, `surface.get_current_texture()` can block on some
//! wgpu backends under driver or compositor stalls, and a future
//! loop bug with no termination guard would do the same. Without a
//! watchdog, all of these present to the user as "the app froze"
//! with no stack trace, no log line, nothing actionable.
//!
//! The watchdog is deliberately minimal. It owns a single
//! [`AtomicU64`] holding the last ping's ms-since-start timestamp;
//! the main loop writes this atomic at the top of every
//! `AboutToWait` drain. The watchdog thread only ever *reads* that
//! atomic — it never touches any app state — so the single-threaded
//! invariant for app logic is preserved.
//!
//! # Not on WASM
//!
//! WASM builds do not get an equivalent in this pass. Browser tabs
//! surface their own "page unresponsive" dialog after a similar
//! threshold, and the JS event-loop machinery differs enough that a
//! Worker-based liveness check warrants its own design. The freeze
//! class this module diagnoses is also native-heavier: `RwLock`
//! deadlocks on wasm32 are caught earlier because the runtime is
//! cooperative, and the GPU path is mediated by the browser. See
//! `CLAUDE.md`'s "Dual-target status" for the parity note.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Maximum time the main thread may go without pinging the
/// watchdog before the watchdog fires. Ten seconds is orders of
/// magnitude beyond any legitimate frame budget (60 fps = 16.6 ms;
/// even a slow scene-build is comfortably under 1 s) and
/// conservative enough to never fire on a healthy system under
/// legitimate load. Tune down cautiously — false positives here
/// kill the process.
const FREEZE_THRESHOLD: Duration = Duration::from_secs(10);

/// How often the watchdog thread wakes to check the liveness
/// atomic. One second keeps wakeup cost negligible while still
/// detecting a freeze within `FREEZE_THRESHOLD + 1 s`.
const WATCHDOG_POLL: Duration = Duration::from_secs(1);

/// Handle to the freeze watchdog. Dropping the handle does *not*
/// stop the watchdog thread — the thread is detached and lives
/// for the process lifetime. This is intentional: the watchdog's
/// whole job is to catch a permanently-stuck main thread, so a
/// mechanism that could be dropped-in-error and silently disable
/// the safety net would defeat the point.
pub struct FreezeWatchdog {
    last_activity_ms: Arc<AtomicU64>,
    /// Monotonic clock shared with the background thread so both
    /// sides measure elapsed time against the same origin. Avoids
    /// reaching into [`crate::application::app::now_ms`], which is
    /// private to its module and returns `f64`.
    epoch: Instant,
}

impl FreezeWatchdog {
    /// Spawn the watchdog thread and return a handle the main loop
    /// can ping. Safe to call exactly once per process; the thread
    /// is detached and cannot be stopped. Callers should keep the
    /// returned handle alive (store it on `InitState` or similar).
    pub fn spawn() -> Self {
        let epoch = Instant::now();
        let last_activity_ms = Arc::new(AtomicU64::new(0));
        let bg_atomic = Arc::clone(&last_activity_ms);
        let bg_epoch = epoch;
        thread::Builder::new()
            .name("mandala-freeze-watchdog".into())
            .spawn(move || watchdog_loop(bg_atomic, bg_epoch))
            .expect("failed to spawn freeze watchdog thread");
        Self { last_activity_ms, epoch }
    }

    /// Ping the watchdog. Call once per frame at the top of
    /// `AboutToWait` (or anywhere else the main loop guarantees
    /// forward progress). Writing is `Relaxed` — we don't need
    /// ordering guarantees because the watchdog only ever reads
    /// a monotonically increasing elapsed-ms value.
    #[inline]
    pub fn tick(&self) {
        let elapsed_ms = self.epoch.elapsed().as_millis() as u64;
        self.last_activity_ms.store(elapsed_ms, Ordering::Relaxed);
    }
}

fn watchdog_loop(last_activity_ms: Arc<AtomicU64>, epoch: Instant) {
    // Wait for the first tick before enforcing anything. Until the
    // main loop has pinged once, the atomic is 0 — treating that
    // as "last activity was at time 0" would fire the watchdog
    // immediately on startup if the first frame takes longer than
    // `FREEZE_THRESHOLD` to land (e.g., a large map load).
    loop {
        thread::sleep(WATCHDOG_POLL);
        let last = last_activity_ms.load(Ordering::Relaxed);
        if last == 0 {
            continue;
        }
        let now = epoch.elapsed().as_millis() as u64;
        let silence_ms = now.saturating_sub(last);
        if silence_ms > FREEZE_THRESHOLD.as_millis() as u64 {
            eprintln!();
            eprintln!("!!! MANDALA FREEZE WATCHDOG !!!");
            eprintln!(
                "main thread has not pinged for {} ms (threshold {} ms).",
                silence_ms,
                FREEZE_THRESHOLD.as_millis()
            );
            eprintln!(
                "this almost certainly means a deadlock, an infinite loop, or a \
                 blocking GPU/compositor call on the main thread."
            );
            eprintln!(
                "aborting the process so the OS can produce a core / crash report. \
                 re-run under a debugger or with `RUST_BACKTRACE=1` and \
                 `ulimit -c unlimited` to capture the stuck stack."
            );
            eprintln!();
            std::process::abort();
        }
    }
}
