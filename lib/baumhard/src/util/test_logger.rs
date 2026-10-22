// SPDX-License-Identifier: MPL-2.0

//! A real `log::Log` sink, so a test can assert that a degrade path
//! actually said something.
//!
//! CODE_CONVENTIONS §9 makes `warn!` the visible half of "degrade,
//! log, keep running", which means the log line is part of the
//! behavior rather than decoration — and until this existed nothing
//! could tell a `warn!` that fires from one that was deleted. The
//! loader's unknown-key report is the case that forced it: the keys
//! it preserves are observable through
//! [`MindMap::unknown_keys`](crate::mindmap::model::MindMap::unknown_keys),
//! but "and it told somebody" was untestable, and a mutation deleting
//! the `warn!` left the whole suite green.
//!
//! **This is not a mock** (TEST_CONVENTIONS §T10). It is an ordinary
//! implementation of the `log` crate's own trait, installed the way
//! `env_logger` is installed and receiving exactly what a real run
//! would send it. Nothing about the code under test changes shape to
//! accommodate it.
//!
//! **How it survives parallel tests.** `log::set_logger` is
//! process-global and `cargo test` runs tests concurrently, so the
//! buffer holds lines from whatever else happened to be logging.
//! Rather than serialize the suite behind a lock — which would be a
//! standing tax on every future test — callers search the buffer for
//! a needle they made unique, and simply do not care what else is in
//! it. [`lines_containing`](crate::util::test_logger::lines_containing)
//! is that search.
//!
//! **What it costs, and the two bounds on it.** Being process-global
//! means every record from every test in the binary is formatted and
//! retained, and the search is linear in what has accumulated. Two
//! things keep that finite: only `warn!` and above are recorded
//! (`RECORDED_LEVEL`), and the buffer stops at `CAPACITY` rather
//! than growing or forgetting. Both failure modes this could have —
//! a logger conflict at install time and a full buffer at search time
//! — panic with what actually went wrong, because either one
//! otherwise turns into "the code under test did not warn", which is
//! a wrong answer pointing at innocent code.
//!
//! Native-only: the browser build installs `console_log` and has no
//! use for a buffer.
//!
//! **Not `cfg(test)`-gated**, for the reason `test_temp` and
//! `rust_source` are not — half its callers are in `mandala`, whose
//! test build links baumhard as an ordinary dependency and so cannot
//! see anything gated behind baumhard's own `cfg(test)`. Until this
//! was widened, `rust_source`'s header named exactly that gap ("a log
//! statement whose sink the suite does not install") and mandala had
//! to pin its `warn!`s by parsing its own source instead of watching
//! them fire. The cost is that a shipped build carries the recorder;
//! it is inert unless [`install`](crate::util::test_logger::install) is
//! called, and nothing outside a
//! test calls it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once, OnceLock};

/// How many lines the buffer keeps before it stops recording.
///
/// The recorder is process-global for the whole lib-test binary, so
/// without a bound it accumulates a formatted `String` for every
/// record every test emits and holds them all under one mutex, and
/// [`lines_containing`] — which is O(lines) — gets slower for every
/// test that ran before it. Trimming the *oldest* lines instead would
/// be worse than a bound: the line a test is about to look for could
/// be the one trimmed, and the failure would read as "the loader did
/// not warn".
///
/// So the buffer stops instead of forgetting, and [`Buffer::matching`]
/// refuses to answer once it has. Sized well above what a full run
/// produces at [`RECORDED_LEVEL`] — reaching it means something
/// started logging in a loop, which is a thing to find out about
/// rather than absorb.
const CAPACITY: usize = 8192;

/// The level the recorder keeps.
///
/// `warn!` and `error!` are what CODE_CONVENTIONS §9 makes part of a
/// degrade path's behavior, and they are what every caller here
/// asserts on. Keeping only those is what makes [`CAPACITY`] a
/// generous bound rather than a tight one. A test that needs to read
/// an `info!` or a `debug!` raises this one constant — deliberately,
/// and with the volume it implies visible in the diff.
const RECORDED_LEVEL: log::LevelFilter = log::LevelFilter::Warn;

/// The recorded lines and whether recording has stopped, in one
/// value — because the bound and the refusal to answer past it are
/// one decision, and holding them apart in a `Vec` plus a separate
/// atomic is what made both untestable. Split that way, the only
/// thing that could exercise either was the process-global buffer,
/// and filling *that* to [`CAPACITY`] would make every other logging
/// test in the binary fail. Owning the pair means an ordinary local
/// value runs the same code the recorder runs.
///
/// Lines are formatted the way a reader sees them:
/// `"<LEVEL> <message>"`.
#[derive(Debug, Default)]
struct Buffer {
    lines: Vec<String>,
    /// Set once the buffer filled up and started dropping records.
    stopped: bool,
}

impl Buffer {
    /// Keep `line`, or stop recording if the buffer is already full.
    ///
    /// Stops rather than forgetting: trimming the oldest line
    /// instead could trim the very line a test is about to search
    /// for, and that failure would read as "the code under test did
    /// not warn".
    fn record(&mut self, line: String) {
        if self.lines.len() >= CAPACITY {
            self.stopped = true;
            return;
        }
        self.lines.push(line);
    }

    /// Every recorded line containing `needle`, oldest first.
    ///
    /// **Panics once recording has stopped.** Past [`CAPACITY`] a
    /// "not found" no longer means "not logged", and letting an
    /// unrelated test fail with that wrong reason is worse than
    /// refusing to answer.
    ///
    /// Cost: O(lines recorded so far).
    fn matching(&self, needle: &str) -> Vec<String> {
        assert!(
            !self.stopped,
            "util::test_logger recorded {CAPACITY} lines and stopped; a search of the \
             buffer can no longer tell 'never logged' from 'logged and dropped'. \
             Something in this run is logging in a loop."
        );
        self.lines
            .iter()
            .filter(|line| line.contains(needle))
            .cloned()
            .collect()
    }
}

/// The process-global buffer every [`Recorder`] record lands in.
fn buffer() -> &'static Mutex<Buffer> {
    static BUFFER: OnceLock<Mutex<Buffer>> = OnceLock::new();
    BUFFER.get_or_init(|| {
        Mutex::new(Buffer {
            lines: Vec::with_capacity(64),
            stopped: false,
        })
    })
}

/// Set once [`install`] has actually put the recorder in place.
/// Distinct from "install was called": the difference is exactly the
/// failure this flag exists to make loud.
static RECORDING: AtomicBool = AtomicBool::new(false);

/// The sink itself. Records and prints nothing — a test run should
/// not gain a stderr flood because one test wanted to read a warning.
struct Recorder;

impl log::Log for Recorder {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= RECORDED_LEVEL
    }

    fn log(&self, record: &log::Record) {
        // The same tap `util::log::init`'s native logger carries, so a
        // test can open a `capture` window and see what a shipped
        // build's window would.
        crate::util::log::capture::observe(record);
        if !self.enabled(record.metadata()) {
            return;
        }
        // A poisoned buffer means some other test panicked mid-push.
        // Recording is not what that test failed at, so recover the
        // lock rather than turning one failure into many.
        buffer().lock().unwrap_or_else(|e| e.into_inner()).record(format!(
            "{} {}",
            record.level(),
            record.args()
        ));
    }

    fn flush(&self) {}
}

/// Install the recorder, once per process.
///
/// Idempotent and safe to call from every test that wants to read a
/// log line.
///
/// **Panics if the recorder cannot be installed.** `log::set_logger`
/// fails when something else got there first, and the `Once` means it
/// is never retried — so a silent failure would leave every
/// `lines_containing` returning nothing forever, and every logging
/// test would fail as "the code under test did not warn". That is a
/// wrong diagnosis pointing at innocent code, which is worse than no
/// test. Nothing installs a competing logger in a test target today;
/// if something ever does, this says so by name.
///
/// Cost: one atomic check after the first call.
pub fn install() {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        if log::set_logger(&Recorder).is_ok() {
            log::set_max_level(RECORDED_LEVEL);
            RECORDING.store(true, Ordering::Relaxed);
        }
    });
    assert_recording(RECORDING.load(Ordering::Relaxed));
}

/// Turn "the recorder is not installed" into a panic that says so.
///
/// A free function rather than an inline `assert!` because it is the
/// only part of the failure path a test can reach: installing a
/// competing logger to provoke the real thing would break every other
/// logging test in the binary, and the `Once` means the situation
/// cannot be un-done once it happens. So the *diagnosis* is tested
/// here directly and the trigger is left to the real world.
fn assert_recording(recording: bool) {
    assert!(
        recording,
        "util::test_logger could not install itself — `log::set_logger` failed, which \
         means another logger is already installed in this test binary. Every \
         assertion about a log line will now fail as though the code under test said \
         nothing. Find the competing `set_logger` rather than the test that reported \
         this."
    );
}

/// Every recorded line containing `needle`, oldest first.
///
/// Installs the recorder if it is not already installed, so a caller
/// is one function call away from an assertion.
///
/// **`needle` has to be something only this test could have
/// produced** — a node id, a key name, a path it built. The buffer is
/// shared with every other test running at the same time, and this
/// deliberately does not clear it: clearing would race with those
/// tests instead of merely coexisting with them.
///
/// Only `warn!` and `error!` are recorded — see `RECORDED_LEVEL`.
///
/// **Panics if recording stopped**; see `Buffer::matching`.
///
/// Cost: O(lines recorded so far) per call, under one mutex
/// acquisition.
pub fn lines_containing(needle: &str) -> Vec<String> {
    install();
    buffer()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .matching(needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The recorder records, and the search finds what this test put
    /// there rather than what a neighbor did.
    #[test]
    fn test_a_warning_reaches_the_buffer() {
        install();
        log::warn!("test_logger: unmistakable-needle-4a91 happened");
        let found = lines_containing("unmistakable-needle-4a91");
        assert_eq!(
            found.len(),
            1,
            "expected exactly one recorded line, got {found:?}"
        );
        assert!(
            found[0].starts_with("WARN "),
            "the level must survive: {}",
            found[0]
        );
    }

    /// A needle nobody logged finds nothing — the control that keeps
    /// the assertion above from passing on an always-true search.
    #[test]
    fn test_an_unlogged_needle_finds_nothing() {
        install();
        assert!(lines_containing("never-logged-needle-0c37").is_empty());
    }

    /// **A failed install must be loud.** If `log::set_logger` ever
    /// loses a race, the `Once` never retries and every search comes
    /// back empty forever — so every logging test in the binary would
    /// fail as "the code under test did not warn", which points the
    /// reader at innocent code. The condition cannot be provoked
    /// without wrecking the binary for every other test, so the
    /// diagnosis is exercised on its own.
    #[test]
    #[should_panic(expected = "another logger is already installed")]
    fn test_a_failed_install_says_what_actually_went_wrong() {
        assert_recording(false);
    }

    /// The recorder keeps warnings and errors and drops the rest, so
    /// the whole suite's `debug!` traffic does not accumulate in a
    /// process-global buffer behind one mutex. A test that needs a
    /// lower level raises [`RECORDED_LEVEL`] on purpose; until then
    /// this pins what "recorded" means.
    #[test]
    fn test_levels_below_the_recorded_one_are_not_kept() {
        install();
        log::info!("test_logger: below-threshold-needle-77b2 happened");
        log::error!("test_logger: above-threshold-needle-77b2 happened");
        assert!(
            lines_containing("below-threshold-needle-77b2").is_empty(),
            "an info! must not reach the buffer"
        );
        assert_eq!(lines_containing("above-threshold-needle-77b2").len(), 1);
    }

    /// **The bound is behavior, not a comment.** Without it the
    /// buffer grows for every record every test in the binary emits
    /// and [`Buffer::matching`] gets slower for each one — and the
    /// bound that replaced it must *stop* rather than forget, because
    /// a ring buffer would drop the line a test is about to look for
    /// and report it as "the code under test did not warn".
    ///
    /// Driven over a local [`Buffer`] rather than the process-global
    /// one: filling that to [`CAPACITY`] would set its stopped flag
    /// and make every other logging test in this binary panic. Same
    /// code, no blast radius.
    #[test]
    fn test_the_buffer_stops_recording_rather_than_forgetting() {
        let mut buffer = Buffer::default();
        for i in 0..CAPACITY {
            buffer.record(format!("WARN line {i}"));
        }
        assert!(!buffer.stopped, "the buffer holds exactly CAPACITY lines");
        assert_eq!(buffer.matching("line 0"), vec!["WARN line 0".to_string()]);

        buffer.record("WARN one line too many".to_string());
        assert_eq!(
            buffer.lines.len(),
            CAPACITY,
            "past CAPACITY the recorder stops; it must not keep growing"
        );
        assert!(
            buffer.stopped,
            "dropping a record without marking the buffer stopped would leave a search \
             answering 'not logged' about a line that was logged"
        );
        assert_eq!(
            buffer.lines[0], "WARN line 0",
            "the oldest line must survive — trimming it to make room could trim the \
             very line a test is about to search for"
        );
    }

    /// And once it has stopped, a search must refuse rather than
    /// answer: "not found" no longer distinguishes "never logged"
    /// from "logged and dropped", so an innocent test would fail with
    /// a wrong reason. The same shape as
    /// `test_a_failed_install_says_what_actually_went_wrong` — the
    /// diagnosis is what is exercised.
    #[test]
    #[should_panic(expected = "logging in a loop")]
    fn test_a_search_of_a_stopped_buffer_refuses_to_answer() {
        let mut buffer = Buffer::default();
        for i in 0..=CAPACITY {
            buffer.record(format!("WARN line {i}"));
        }
        let _ = buffer.matching("line 0");
    }
}
