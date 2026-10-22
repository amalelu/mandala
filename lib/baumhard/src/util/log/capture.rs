// SPDX-License-Identifier: MPL-2.0

//! Scoped capture of the `warn!` / `error!` records one stretch of
//! code emits on the current thread, so a caller can hand them to
//! someone who cannot read the process's stderr.
//!
//! CODE_CONVENTIONS §9 makes `warn!` the visible half of "degrade,
//! log, keep running". That half reaches whoever reads the log, and
//! for a request that arrived over a socket, that is the wrong
//! person. The IPC reply envelope carries a `warnings` array for
//! that reason (`format/ipc.md` §Warnings). Filling it by threading a
//! sink through every dispatch arm, document setter and Baumhard
//! primitive a command can reach would be a second diagnostic
//! channel beside the first, and it would be incomplete the day it
//! was written. So the existing channel is tapped instead: every
//! record the installed logger receives also passes through
//! `observe`, and a [`capture`] window open on that thread keeps
//! it. A `warn!` added anywhere later is captured with no further
//! work. Each record is logged once and reported once, with the same
//! text.
//!
//! **A stable code travels with the record.** A log site that wants
//! a machine-branchable identity attaches it as the `code` key-value
//! (`log::warn!(code = SOME_CODE; "area: …")`, the [`CODE_KEY`]
//! key). The capture reads it from the record. Every other record
//! arrives with `code: None` and is classified by the consumer.
//!
//! **What feeds it.** Only records that reach the installed logger:
//! [`crate::util::log::init`]'s native logger in a binary, and
//! [`crate::util::test_logger`] in a test binary. With no logger
//! installed, `log`'s global max level stays `Off`, nothing reaches
//! `observe`, and a capture comes back empty. A test asserting on
//! captured records therefore calls `test_logger::install()` first.
//!
//! Native-only, like its one consumer, the IPC command executor
//! (`CLAUDE.md` "Dual-target status"). The browser's `console_log`
//! backend does not feed it, and IPC-15's browser transport owns
//! that wiring when it lands.

use std::cell::Cell;

use crate::util::grapheme_chad::truncate_to_byte_budget;

/// The key-value key a log site names its stable diagnostic code
/// under: `log::warn!(code = SOME_CODE; "…")`. The `log` macros
/// spell the key as a bare identifier, so this constant cannot be
/// substituted into the call itself. It exists so the reading side
/// and the documentation name one string.
pub const CODE_KEY: &str = "code";

/// Most records one window keeps. Beyond it records are counted in
/// [`Captured::dropped`] rather than kept, so a degrade path that
/// fires once per node on a large map cannot flood the caller. The
/// first records are kept rather than the last, because the first
/// warning is usually the cause and the rest its echoes.
///
/// Together with [`MESSAGE_BYTE_LIMIT`] this bounds what one window
/// can hold to `CAPTURE_LIMIT × MESSAGE_BYTE_LIMIT` bytes of message
/// text. A count alone would not: a message can echo arbitrary input,
/// such as a failed console line, and 64 of those could outgrow the
/// IPC reply's frame cap and cost the command its whole reply.
pub const CAPTURE_LIMIT: usize = 64;

/// Longest message one captured record keeps, in bytes. A longer one
/// is cut on a grapheme boundary and marked with how much was elided.
/// The full line still reaches the installed logger, because the cut
/// applies only to the captured copy.
pub const MESSAGE_BYTE_LIMIT: usize = 4096;

/// One `warn!` / `error!` record a capture window observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedRecord {
    /// `Warn` or `Error`. Nothing below `Warn` is captured.
    pub level: log::Level,
    /// The record's `code` key-value, when the log site attached one.
    pub code: Option<String>,
    /// The formatted message, exactly as the logger prints it after
    /// its own prefix.
    pub message: String,
}

/// What one [`capture`] window saw.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Captured {
    /// Records in emission order, at most [`CAPTURE_LIMIT`].
    pub records: Vec<CapturedRecord>,
    /// Records observed after `records` was full.
    pub dropped: usize,
}

impl Captured {
    fn push(&mut self, record: CapturedRecord) {
        if self.records.len() < CAPTURE_LIMIT {
            self.records.push(record);
        } else {
            self.dropped += 1;
        }
    }

    /// Fold an inner window's result into this one, so an outer
    /// window sees everything its nested windows saw.
    fn absorb(&mut self, inner: Captured) {
        self.dropped += inner.dropped;
        for record in inner.records {
            self.push(record);
        }
    }
}

thread_local! {
    /// The open window on this thread, if any. A `Cell` rather than
    /// a `RefCell`: every access is a `take` followed by a `set`, so
    /// a record emitted while the window is out of its slot (a
    /// `Display` impl that itself logs, say) finds the slot empty
    /// and is skipped. A `RefCell` would turn that into a
    /// double-borrow panic on an interactive path (§9).
    static WINDOW: Cell<Option<Captured>> = const { Cell::new(None) };
}

/// Run `f`, and return what it produced together with every `warn!`
/// / `error!` record emitted on this thread while it ran.
///
/// Windows nest. An inner window returns only its own records and
/// also folds them into the window around it.
///
/// Panic-safe in the only sense that matters here: if `f` unwinds,
/// the outer window (or the absence of one) is restored on the way
/// out, so a caught panic does not leave a stale window capturing
/// the thread forever.
///
/// Cost: two thread-local swaps per call, plus one `String`
/// allocation per captured record. Nothing is allocated for a window
/// that observes no records.
pub fn capture<R>(f: impl FnOnce() -> R) -> (R, Captured) {
    let outer = WINDOW.with(|w| w.replace(Some(Captured::default())));
    let mut restore = Restore { outer: Some(outer) };
    let result = f();
    let ours = restore.finish();
    (result, ours)
}

/// Puts the enclosing window back when a [`capture`] ends, whether it
/// returned or unwound.
struct Restore {
    /// `Some` until the window has been closed. The inner `Option` is
    /// the enclosing window, absent at the outermost level.
    outer: Option<Option<Captured>>,
}

impl Restore {
    /// Close the window: take this level's records, then reinstate
    /// the enclosing window with them folded in.
    fn finish(&mut self) -> Captured {
        let ours = WINDOW.with(Cell::take).unwrap_or_default();
        if let Some(outer) = self.outer.take() {
            WINDOW.with(|w| {
                w.set(outer.map(|mut outer| {
                    outer.absorb(ours.clone());
                    outer
                }))
            });
        }
        ours
    }
}

impl Drop for Restore {
    fn drop(&mut self) {
        if self.outer.is_some() {
            self.finish();
        }
    }
}

/// Whether a window is open on this thread. A logger asks this from
/// `enabled`, so a record its own filter would discard still reaches
/// [`observe`] while someone is listening.
pub(crate) fn active() -> bool {
    WINDOW.with(|w| {
        let window = w.take();
        let open = window.is_some();
        w.set(window);
        open
    })
}

/// Offer `record` to the window open on this thread, if any. Called
/// by every logger this crate installs, before that logger's own
/// filter runs, so the capture does not depend on `RUST_LOG`.
///
/// Records below `Warn` are ignored without being formatted.
pub(crate) fn observe(record: &log::Record) {
    if record.level() > log::Level::Warn {
        return;
    }
    let Some(mut window) = WINDOW.with(Cell::take) else {
        return;
    };
    window.push(CapturedRecord {
        level: record.level(),
        code: record_code(record),
        message: bounded_message(record.args().to_string()),
    });
    WINDOW.with(|w| w.set(Some(window)));
}

/// `message`, held to [`MESSAGE_BYTE_LIMIT`]. Over the limit, the kept
/// prefix and its elision marker together stay within it.
fn bounded_message(message: String) -> String {
    if message.len() <= MESSAGE_BYTE_LIMIT {
        return message;
    }
    // The marker's length depends on the count it reports. Sizing it
    // with the largest count it could ever print keeps the arithmetic
    // one pass and the result under the limit.
    let marker_room = format!(" … [{} bytes elided]", message.len()).len();
    let (kept, _) = truncate_to_byte_budget(&message, MESSAGE_BYTE_LIMIT - marker_room);
    format!("{kept} … [{} bytes elided]", message.len() - kept.len())
}

/// The record's [`CODE_KEY`] key-value, as a string.
fn record_code(record: &log::Record) -> Option<String> {
    let value = record.key_values().get(log::kv::Key::from_str(CODE_KEY))?;
    Some(
        value
            .to_borrowed_str()
            .map_or_else(|| value.to_string(), str::to_owned),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed() {
        crate::util::test_logger::install();
    }

    #[test]
    fn test_capture_keeps_warn_and_error_in_order_and_ignores_lower_levels() {
        installed();
        let (value, captured) = capture(|| {
            log::info!("capture-test: info-9a1c is below the floor");
            log::warn!("capture-test: first-9a1c");
            log::debug!("capture-test: debug-9a1c is below the floor");
            log::error!("capture-test: second-9a1c");
            7
        });
        assert_eq!(value, 7, "the closure's result is handed back");
        assert_eq!(
            captured.records,
            vec![
                CapturedRecord {
                    level: log::Level::Warn,
                    code: None,
                    message: "capture-test: first-9a1c".into(),
                },
                CapturedRecord {
                    level: log::Level::Error,
                    code: None,
                    message: "capture-test: second-9a1c".into(),
                },
            ]
        );
        assert_eq!(captured.dropped, 0);
    }

    #[test]
    fn test_capture_reads_the_code_key_value_from_the_record() {
        installed();
        const SOME_CODE: &str = "some_code";
        let ((), captured) = capture(|| {
            log::warn!(code = SOME_CODE; "capture-test: coded-4be0");
        });
        assert_eq!(captured.records.len(), 1);
        assert_eq!(captured.records[0].code.as_deref(), Some(SOME_CODE));
        assert_eq!(
            captured.records[0].message, "capture-test: coded-4be0",
            "the code travels beside the message, not inside it"
        );
    }

    #[test]
    fn test_records_outside_any_window_are_not_captured() {
        installed();
        log::warn!("capture-test: before-the-window-77d2");
        let ((), captured) = capture(|| ());
        assert!(captured.records.is_empty(), "{captured:?}");
        assert!(!active(), "no window is left open after a capture ends");
    }

    #[test]
    fn test_a_window_keeps_the_first_records_and_counts_the_rest() {
        installed();
        let ((), captured) = capture(|| {
            for i in 0..CAPTURE_LIMIT + 5 {
                log::warn!("capture-test: flood-{i}-c3e8");
            }
        });
        assert_eq!(captured.records.len(), CAPTURE_LIMIT);
        assert_eq!(captured.dropped, 5);
        assert_eq!(captured.records[0].message, "capture-test: flood-0-c3e8");
    }

    /// A message that echoes large input is cut, not the window: the
    /// bound on a window is records × bytes, not records alone.
    #[test]
    fn test_an_oversized_message_is_cut_within_the_byte_limit_and_says_so() {
        installed();
        let huge = "é".repeat(MESSAGE_BYTE_LIMIT);
        let ((), captured) = capture(|| log::warn!("capture-test: huge-8b07 {huge}"));
        let message = &captured.records[0].message;
        assert!(message.len() <= MESSAGE_BYTE_LIMIT, "{} bytes", message.len());
        assert!(message.starts_with("capture-test: huge-8b07 éé"));
        assert!(message.ends_with(" bytes elided]"), "the cut is marked");
        let short = "capture-test: short-8b07";
        assert_eq!(
            bounded_message(short.to_string()),
            short,
            "a short message is untouched"
        );
    }

    #[test]
    fn test_nested_windows_report_their_own_records_and_feed_the_outer_one() {
        installed();
        let (inner, outer) = capture(|| {
            log::warn!("capture-test: outer-only-51fa");
            let ((), inner) = capture(|| log::warn!("capture-test: inner-51fa"));
            inner
        });
        let messages = |c: &Captured| c.records.iter().map(|r| r.message.clone()).collect::<Vec<_>>();
        assert_eq!(messages(&inner), ["capture-test: inner-51fa"]);
        assert_eq!(
            messages(&outer),
            ["capture-test: outer-only-51fa", "capture-test: inner-51fa"]
        );
    }

    #[test]
    fn test_an_unwinding_closure_does_not_leave_its_window_open() {
        installed();
        let unwound = std::panic::catch_unwind(|| {
            capture(|| {
                log::warn!("capture-test: before-the-panic-0d6b");
                panic!("capture-test: deliberate");
            })
        });
        assert!(unwound.is_err());
        assert!(!active(), "the window must close on unwind");
        let ((), captured) = capture(|| ());
        assert!(
            captured.records.is_empty(),
            "a fresh window must not inherit the unwound one's records"
        );
    }

    #[test]
    fn test_windows_are_per_thread() {
        installed();
        let ((), captured) = capture(|| {
            std::thread::spawn(|| log::warn!("capture-test: other-thread-e21f"))
                .join()
                .expect("the logging thread does not panic");
        });
        assert!(
            captured.records.is_empty(),
            "a record from another thread must not land in this thread's window: {captured:?}"
        );
    }
}
