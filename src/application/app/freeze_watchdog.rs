// SPDX-License-Identifier: MPL-2.0

//! Native main-loop watchdog. A background thread polls a liveness
//! atomic that the main loop pings every frame; if the main thread
//! has been silent longer than [`FREEZE_THRESHOLD`], the watchdog
//! prints a banner and aborts so the OS can produce a core dump.
//!
//! The class of freeze this catches is native-heavier (RwLock
//! re-entry, GPU stalls, runaway loops); browsers surface their own
//! page-unresponsive dialog, so WASM has no equivalent yet. See
//! `CLAUDE.md` "Dual-target status".

#![cfg(not(target_arch = "wasm32"))]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Abort if the main thread is silent longer than this.
/// Conservative enough to never fire on legitimate load
/// (60 fps ≈ 16.6 ms/frame); tune down cautiously — false
/// positives here kill the process.
const FREEZE_THRESHOLD: Duration = Duration::from_secs(10);

/// Watchdog thread wake interval. Detection latency is
/// `FREEZE_THRESHOLD + WATCHDOG_POLL` in the worst case.
const WATCHDOG_POLL: Duration = Duration::from_secs(1);

/// Handle to the freeze watchdog. Dropping the handle does *not*
/// stop the watchdog thread — the thread is detached and lives
/// for the process lifetime. This is intentional: the watchdog's
/// whole job is to catch a permanently-stuck main thread, so a
/// mechanism that could be dropped-in-error and silently disable
/// the safety net would defeat the point.
pub struct FreezeWatchdog {
    last_activity_ms: Arc<AtomicU64>,
    /// `true` while the main thread is legitimately parked in
    /// `ControlFlow::Wait`, waiting for an OS event. The watchdog
    /// loop ignores silence while this is set — idle in `Wait`
    /// mode can vastly exceed `FREEZE_THRESHOLD` and must not be
    /// treated as a hang. Cleared at the top of each event handler
    /// so an event-handler that itself hangs still trips the
    /// threshold.
    parked: Arc<AtomicBool>,
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
        let parked = Arc::new(AtomicBool::new(false));
        let bg_atomic = Arc::clone(&last_activity_ms);
        let bg_parked = Arc::clone(&parked);
        let bg_epoch = epoch;
        thread::Builder::new()
            .name("mandala-freeze-watchdog".into())
            .spawn(move || watchdog_loop(bg_atomic, bg_parked, bg_epoch))
            .expect("failed to spawn freeze watchdog thread");
        Self {
            last_activity_ms,
            parked,
            epoch,
        }
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

    /// Mark the main thread as parked in `ControlFlow::Wait`. Call
    /// immediately before yielding to winit at the end of an event
    /// pump when no further work is queued. While parked, the
    /// watchdog ignores silence — legitimate idle is permitted to
    /// last indefinitely. A parked-state ping also clears any
    /// pending stale `last_activity_ms` so the unpark below resets
    /// the clock cleanly.
    #[inline]
    pub fn parked(&self) {
        self.parked.store(true, Ordering::Relaxed);
    }

    /// Mark the main thread as actively running again. Call at the
    /// top of every event handler before doing any work. Pairs
    /// with `tick()` to refresh the activity timestamp; together
    /// they tell the watchdog "an event arrived and is being
    /// processed". A handler that hangs after `unparked` still
    /// trips the threshold.
    #[inline]
    pub fn unparked(&self) {
        self.parked.store(false, Ordering::Relaxed);
        self.tick();
    }
}

/// Pure decision logic separated from the IO loop so it's
/// unit-testable. Returns `true` iff the main thread should be
/// considered hung at the time encoded by `now_ms` given the last
/// observed activity timestamp, the current `parked` flag, and the
/// configured threshold. Used by [`watchdog_loop`].
fn should_abort(now_ms: u64, last_activity_ms: u64, parked: bool, threshold_ms: u64) -> bool {
    // The first tick hasn't landed yet — treat as "still booting"
    // rather than "stuck". The watchdog only enforces after the
    // main loop has ticked once.
    if last_activity_ms == 0 {
        return false;
    }
    // Legitimate `ControlFlow::Wait` idle: the main thread is
    // sleeping waiting for an OS event, not stuck. The next event
    // will call `unparked()` and refresh the activity clock.
    if parked {
        return false;
    }
    let silence_ms = now_ms.saturating_sub(last_activity_ms);
    silence_ms > threshold_ms
}

fn watchdog_loop(last_activity_ms: Arc<AtomicU64>, parked: Arc<AtomicBool>, epoch: Instant) {
    loop {
        thread::sleep(WATCHDOG_POLL);
        let last = last_activity_ms.load(Ordering::Relaxed);
        let is_parked = parked.load(Ordering::Relaxed);
        let now = epoch.elapsed().as_millis() as u64;
        if should_abort(now, last, is_parked, FREEZE_THRESHOLD.as_millis() as u64) {
            let silence_ms = now.saturating_sub(last);
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

#[cfg(test)]
mod tests {
    use super::should_abort;

    #[test]
    fn first_tick_zero_means_still_booting() {
        // Before the main loop has ticked once, last_activity_ms
        // is the default 0. Even with `now_ms` past the threshold,
        // abort must not fire — otherwise a slow first frame
        // (large map load) would kill the process at startup.
        assert!(!should_abort(
            /*now_ms*/ 1_000_000, /*last*/ 0, /*parked*/ false, /*thr*/ 100
        ));
    }

    #[test]
    fn parked_state_suppresses_silence_check() {
        // The headline behaviour: under `ControlFlow::Wait` the
        // main thread legitimately idles. With `parked=true`, no
        // amount of silence triggers a hang verdict.
        assert!(!should_abort(
            /*now*/ 1_000_000, /*last*/ 100, /*parked*/ true, /*thr*/ 100
        ));
        // Sanity: same numbers but unparked DO trip the threshold.
        assert!(should_abort(
            /*now*/ 1_000_000, /*last*/ 100, /*parked*/ false, /*thr*/ 100
        ));
    }

    #[test]
    fn unparked_silence_within_threshold_is_alive() {
        // Activity 50ms ago with a 100ms threshold: not yet a hang.
        assert!(!should_abort(
            /*now*/ 150, /*last*/ 100, /*parked*/ false, /*thr*/ 100
        ));
    }

    #[test]
    fn unparked_silence_past_threshold_aborts() {
        // Activity 200ms ago with a 100ms threshold: hang verdict.
        assert!(should_abort(
            /*now*/ 300, /*last*/ 100, /*parked*/ false, /*thr*/ 100
        ));
    }

    #[test]
    fn saturating_sub_handles_clock_inversions() {
        // If `last_activity_ms > now_ms` (impossible in practice
        // because both come from the same monotonic epoch, but
        // worth pinning), `saturating_sub` returns 0 and the
        // result is "no abort" rather than a wraparound abort.
        assert!(!should_abort(
            /*now*/ 50, /*last*/ 100, /*parked*/ false, /*thr*/ 100
        ));
    }
}
