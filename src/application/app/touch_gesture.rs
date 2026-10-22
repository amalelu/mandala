// SPDX-License-Identifier: MPL-2.0

//! `TouchGestureRecognizer` — the touch half of the pointer
//! vocabulary, as a plain-value state machine.
//!
//! The rest of the input pipeline is mouse-first: both targets
//! consume `WindowEvent::MouseInput` / `CursorMoved` / `MouseWheel`
//! and route them through the dispatch funnel via the
//! [`crate::application::keybinds::MouseGesture`] table. A finger
//! arrives instead as `WindowEvent::Touch`, and winit's mouse
//! synthesis does not cover a hold, a second finger, or a drag that
//! never pressed a button — so without this module a phone in the
//! browser has no input path at all, on the surface
//! `run_wasm/mod.rs` calls the *primary* one this project targets.
//!
//! What the machine consumes is raw `(phase, finger_id, position,
//! now)` tuples; what it produces is a [`RecognizedGesture`]. No
//! clock is read in here and no I/O happens in here — time arrives
//! as a parameter — which is what makes every rule below provable
//! by `cargo test` on a machine with no touchscreen
//! (`TEST_CONVENTIONS §T9`).
//!
//! ## The vocabulary, and where each emission goes
//!
//! Four gestures, in two groups that reach the application by two
//! different routes (`dispatch::cross_dispatch::pointer`):
//!
//! - [`RecognizedGesture::LongPress`] is **keybind-routed**: the
//!   runtime resolves `MouseGesture::LongPress` through
//!   `action_for_gesture` and dispatches whatever `Action` the user
//!   has bound, exactly as a mouse gesture does.
//! - [`RecognizedGesture::Tap`], [`RecognizedGesture::Pan`] and
//!   [`RecognizedGesture::PinchStep`] are **direct-effect**: they
//!   take the two carve-outs `CODE_CONVENTIONS §3` already grants
//!   the mouse. A tap commits a selection, which is the
//!   "pre-funnel state-machine bookkeeping" carve-out that a
//!   single left-click takes; a pan or a pinch moves the camera by
//!   a `RenderDecree`, which is the "per-frame continuous-gesture
//!   state" carve-out that a left-drag's per-cursor-move delta
//!   takes. Neither becomes an `Action`, so neither can be dead on
//!   one target the way a `NativeOnly` binding is.
//!
//! ## State machine
//!
//! ```text
//!                   started(a)                      started(b), a still down
//!     Idle ──────────────────────► OneFinger ──────────────────────► TwoFingers
//!       ▲                          │  │  │                            │   │
//!       │  ended(a), still, quick  │  │  │ moved(a) past threshold    │   │ moved(a|b)
//!       │  ── emit Tap ────────────┘  │  │ ── emit Pan (per sample) ──┘   │ ── emit PinchStep
//!       │                             │  │                                │    (per step)
//!       │  tick at started_at +       │  │                                │
//!       │  long_press, no movement    │  │                                │
//!       │  ── emit LongPress ─────────┘  │                                │
//!       │                                │                                │
//!       │  ended(a), nothing to emit     │      ended(a|b): demote to the │
//!       └────────────────────────────────┘◄─────surviving finger ─────────┘
//! ```
//!
//! Four emit points, and the rules that keep them from overlapping:
//!
//! - **`LongPress`** fires from [`TouchGestureRecognizer::tick`] —
//!   "held for 350 ms" is a wall-clock transition, not an event —
//!   once per `OneFinger` episode, and only while the finger has
//!   stayed inside
//!   [`POINTER_DRAG_THRESHOLD_PX`](super::POINTER_DRAG_THRESHOLD_PX).
//! - **`Tap`** fires on the finger's lift, and only when the
//!   episode has emitted nothing else, the finger never left the
//!   threshold, and the hold was *shorter* than the long-press
//!   budget. The two time windows are disjoint at the boundary:
//!   `tick` fires at `>= long_press`, a tap needs `< long_press`.
//! - **`Pan`** fires on every `Moved` from the sample that crosses
//!   the threshold onward. That first emission carries the travel
//!   since the finger *landed*, not since the previous sample, so
//!   the canvas ends up displaced by exactly the finger's net
//!   displacement and the threshold slop is not silently lost.
//! - **`PinchStep`** fires while two fingers are down, whenever
//!   *either* their midpoint has travelled past the threshold or
//!   their separation has changed past it, measured from the
//!   previous emission. Both halves are reported every time, so a
//!   two-finger gesture that is really a translation reports
//!   `scale == 1.0`, and one that is really a spread reports a
//!   near-zero `pan`.
//!
//! ## Why a typed emission rather than a synthetic mouse event
//!
//! A literal "synthesize `(ElementState, MouseButton)` and call the
//! mouse handler" would make the dispatch funnel see
//! `MouseButton::Left` rather than the gesture, so a long-press
//! would run the left-click binding. Emitting the recognized
//! gesture instead leaves the recognition as the only new step; the
//! routing beyond it is the machinery each of the two groups above
//! already had.

use super::POINTER_DRAG_THRESHOLD_SQ_PX;
use std::time::Duration;
use web_time::Instant;

/// Long-press fires after this much time with no significant
/// movement. 350ms is the convention from iOS' UILongPressGesture
/// recognizer default and from Android's `ViewConfiguration`.
/// Shorter than this and accidental holds during scrolling fire;
/// longer than this and the gesture feels sluggish.
pub const LONG_PRESS_MS: u64 = 350;

/// Finger separations below this (physical pixels) carry no usable
/// scale information: two touch points reported at nearly the same
/// place make `span / previous_span` explode, and at exactly the
/// same place it divides by zero. A [`RecognizedGesture::PinchStep`]
/// measured across such a baseline reports `scale = 1.0` and
/// contributes its translation only, so a degenerate report from the
/// digitizer cannot fling the camera to `MIN_ZOOM` or to NaN.
const MIN_PINCH_SPAN_PX: f64 = 1.0;

/// What the recognizer's `ingest` / `tick` returns when a gesture is
/// identified. Every position is in whatever space went in —
/// physical pixels, the space `WindowEvent::Touch` reports and the
/// space `cursor_pos` holds — so the runtime can move the cursor to
/// one and hand another straight to a `RenderDecree`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RecognizedGesture {
    /// One finger down and back up inside the long-press budget,
    /// having stayed inside
    /// [`POINTER_DRAG_THRESHOLD_PX`](super::POINTER_DRAG_THRESHOLD_PX).
    /// The touch peer of a left click: the runtime commits the
    /// selection under `pos`.
    Tap { pos: (f64, f64) },
    /// One finger held in place for [`LONG_PRESS_MS`] ms with
    /// movement under
    /// [`POINTER_DRAG_THRESHOLD_PX`](super::POINTER_DRAG_THRESHOLD_PX).
    /// The runtime dispatches the binding for
    /// [`MouseGesture::LongPress`](crate::application::keybinds::MouseGesture::LongPress)
    /// at `pos`.
    LongPress { pos: (f64, f64) },
    /// One finger dragging. Emitted once per `Moved` sample from the
    /// threshold crossing onward; the runtime translates the camera
    /// by `delta`.
    ///
    /// `delta` is the finger's movement since the previous emission,
    /// except on the first one, where it is the movement since the
    /// finger landed — so summing every `delta` of an episode gives
    /// the finger's net displacement exactly.
    Pan {
        /// Where the finger is now.
        pos: (f64, f64),
        /// Screen-space translation this step asks the camera for.
        delta: (f64, f64),
    },
    /// Two fingers moving. Emitted once per "step" — whenever the
    /// midpoint or the separation has changed past
    /// [`POINTER_DRAG_THRESHOLD_PX`](super::POINTER_DRAG_THRESHOLD_PX)
    /// since the previous emission — carrying the whole similarity
    /// transform the two fingers describe over that step.
    PinchStep {
        /// Midpoint of the two fingers now. The anchor the scale is
        /// applied about, so the canvas point between the fingers
        /// stays under them.
        center: (f64, f64),
        /// Midpoint translation since the previous emission.
        pan: (f64, f64),
        /// Ratio of the current finger separation to the separation
        /// at the previous emission. `> 1.0` is a spread (zoom in),
        /// `< 1.0` a pinch (zoom out), exactly `1.0` when the
        /// fingers held their separation — or when the baseline
        /// separation was below [`MIN_PINCH_SPAN_PX`] and no ratio
        /// could be formed.
        ///
        /// A ratio rather than a difference because zoom composes
        /// multiplicatively: `RenderDecree::CameraZoom` multiplies,
        /// so consecutive steps of a pinch compose into the total
        /// scale without the recognizer tracking an origin.
        scale: f64,
    },
}

/// What the runtime feeds the recognizer. winit's
/// `event::TouchPhase` doesn't impl `Hash`/`Copy` reliably across
/// versions, and we don't want the recognizer to depend on a
/// specific winit version, so the runtime translates phases into
/// this stable enum at the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Finger just landed. Equivalent to `TouchPhase::Started`.
    Started,
    /// Finger is moving. Equivalent to `TouchPhase::Moved`.
    Moved,
    /// Finger lifted deliberately. Equivalent to
    /// `TouchPhase::Ended` — the only lift that can be a
    /// [`RecognizedGesture::Tap`].
    Ended,
    /// The system took the touch away: a notification shade pulled
    /// down over the canvas, palm rejection, the browser handing
    /// the sequence to its own scroll. Equivalent to
    /// `TouchPhase::Cancelled`.
    ///
    /// It clears the same slot `Ended` does and emits nothing.
    /// **The distinction is load-bearing** and was not, before the
    /// vocabulary grew a tap: folding `Cancelled` onto `Ended`
    /// would let the operating system commit a selection the user
    /// never asked for, every time it interrupted a finger resting
    /// on a node.
    Cancelled,
}

/// One tracked finger. Held by both `OneFinger` and `TwoFingers`
/// state branches.
#[derive(Debug, Clone, Copy, PartialEq)]
struct FingerTrack {
    id: u64,
    started_at: Instant,
    started_pos: (f64, f64),
    current_pos: (f64, f64),
    /// Set true on the first `Moved` event whose distance from
    /// `started_pos` exceeds
    /// [`POINTER_DRAG_THRESHOLD_PX`](super::POINTER_DRAG_THRESHOLD_PX).
    /// Sticky — once true it stays true for the finger's lifetime.
    ///
    /// It is the discriminator between the two one-finger gestures:
    /// false keeps the tap and the long-press alive, true starts a
    /// pan and kills both.
    has_moved: bool,
}

impl FingerTrack {
    fn new(id: u64, pos: (f64, f64), now: Instant) -> Self {
        Self {
            id,
            started_at: now,
            started_pos: pos,
            current_pos: pos,
            has_moved: false,
        }
    }

    /// `move_threshold_sq` is the *squared* travel budget — the
    /// caller's, not the module constant's, so the recognizer's
    /// test constructor reaches this latch as well as the centroid
    /// check. It used to read the constant directly, which left
    /// `TouchGestureRecognizer::with_thresholds` half-honored:
    /// a test could loosen the two-finger step and not the
    /// long-press cancel.
    fn update_pos(&mut self, pos: (f64, f64), move_threshold_sq: f64) {
        self.current_pos = pos;
        let (dx, dy) = (pos.0 - self.started_pos.0, pos.1 - self.started_pos.1);
        if dx * dx + dy * dy > move_threshold_sq {
            self.has_moved = true;
        }
    }
}

/// Internal state. Tested through the public `ingest` / `tick`
/// API rather than directly — the variants are
/// implementation-detail.
#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Idle,
    /// Single finger tracked: a tap, a long-press and a pan are all
    /// still possible from here.
    OneFinger {
        track: FingerTrack,
        /// The episode has already produced a discrete gesture, or
        /// inherited a history that forbids one. Set when
        /// [`RecognizedGesture::LongPress`] fires — so a continued
        /// hold cannot re-fire it, and the eventual lift cannot
        /// also read as a tap — and set on demotion from
        /// `TwoFingers`, so the finger left over from a pinch is
        /// not a fresh tap candidate.
        ///
        /// **This latch is the whole mechanism.** The demotion arm
        /// used to additionally back-date the survivor's
        /// `started_at` under a comment claiming *that* was what
        /// stopped the long-press; it was not, it was this flag,
        /// and the two together were one mechanism plus a decoy.
        discrete_emitted: bool,
    },
    /// Two fingers tracked. The only gesture reachable from here is
    /// [`RecognizedGesture::PinchStep`], which reports the midpoint
    /// translation and the separation ratio together.
    ///
    /// Both baselines are captured when the second finger lands, so
    /// the first step measures from the moment the gesture began
    /// rather than from the sample before it.
    TwoFingers {
        primary: FingerTrack,
        secondary: FingerTrack,
        last_emit_centroid: (f64, f64),
        last_emit_span: f64,
    },
}

/// Touch gesture recognizer. One per app instance. Fed by the
/// runtime's `WindowEvent::Touch` handler; consulted at frame
/// boundaries via [`Self::tick`] for time-based gestures
/// (long-press) that don't fire on the touch event itself.
#[derive(Debug, Clone)]
pub struct TouchGestureRecognizer {
    state: State,
    /// Long-press threshold, configurable for tests. Default
    /// [`LONG_PRESS_MS`]. Doubles as the tap's *upper* bound: a
    /// hold that reaches this is a long-press, so it can no longer
    /// be a tap.
    ///
    /// Both readers measure with `saturating_duration_since`, not
    /// `duration_since`. `web_time::Instant` can hand back a
    /// regressed timestamp — a Firefox bfcache restore is the
    /// reported case — and a saturated zero is the right reading of
    /// one: the hold is not long enough to be a long-press, and is
    /// short enough to still be a tap.
    long_press: Duration,
    /// Squared movement threshold for every emit point: the
    /// long-press / tap cancellation, the one-finger pan promotion,
    /// and the two-finger step. Configurable for tests. Squared so
    /// no emit point pays a `sqrt` per motion event, matching the
    /// mouse arms in `event_cursor_moved.rs`.
    move_threshold_sq: f64,
}

impl Default for TouchGestureRecognizer {
    fn default() -> Self {
        Self::new()
    }
}

impl TouchGestureRecognizer {
    /// New recognizer at production thresholds.
    pub fn new() -> Self {
        Self {
            state: State::Idle,
            long_press: Duration::from_millis(LONG_PRESS_MS),
            move_threshold_sq: POINTER_DRAG_THRESHOLD_SQ_PX,
        }
    }

    /// Test-only constructor. Lets the state-machine tests pin
    /// timing without sleeping for 350ms per case.
    /// `move_threshold_px` is linear (a test says "four pixels",
    /// not "sixteen"); it is squared here, once, at construction.
    #[cfg(test)]
    pub(crate) fn with_thresholds(long_press: Duration, move_threshold_px: f64) -> Self {
        Self {
            state: State::Idle,
            long_press,
            move_threshold_sq: move_threshold_px * move_threshold_px,
        }
    }

    /// Drive a touch event through the state machine.
    /// `id` is the finger id (winit's `Touch.id`); `pos` is in
    /// **physical** pixels — `dispatch_touch_event` passes
    /// `touch.location`, a `PhysicalPosition<f64>`, straight
    /// through. (This used to claim logical, "the runtime converts
    /// via the scale factor before calling here"; no such
    /// conversion exists, on either target. See
    /// [`POINTER_DRAG_THRESHOLD_PX`](super::POINTER_DRAG_THRESHOLD_PX)
    /// for what that costs and what closing it would take.)
    /// Returns `Some(gesture)` when the event triggered a
    /// recognition. The runtime is responsible for acting on that
    /// gesture; tests frequently discard the return when they're
    /// staging the state machine for a later assertion (no
    /// `#[must_use]` for that reason — the caller layer is the
    /// right place for the lint, and the runtime is a single call
    /// site that can't accidentally drop the result).
    pub fn ingest(
        &mut self,
        phase: Phase,
        id: u64,
        pos: (f64, f64),
        now: Instant,
    ) -> Option<RecognizedGesture> {
        match phase {
            Phase::Started => self.on_started(id, pos, now),
            Phase::Moved => self.on_moved(id, pos),
            // Both lifts clear the same slot; only a deliberate one
            // can be a tap. See [`Phase::Cancelled`].
            Phase::Ended => self.on_lifted(id, now, true),
            Phase::Cancelled => self.on_lifted(id, now, false),
        }
    }

    /// Frame-boundary tick. The runtime calls this once per frame
    /// (cheap when the state is `Idle` — one branch). Long-press
    /// fires here, not on the `Moved` / `Started` events, because
    /// "the user has been holding for 350ms" is a wall-clock
    /// transition, not a touch-event-driven one. Symmetric with
    /// [`Self::ingest`] — no `#[must_use]` for the same
    /// reasoning.
    pub fn tick(&mut self, now: Instant) -> Option<RecognizedGesture> {
        if let State::OneFinger {
            track,
            discrete_emitted,
        } = &mut self.state
        {
            if !*discrete_emitted
                && !track.has_moved
                && now.saturating_duration_since(track.started_at) >= self.long_press
            {
                *discrete_emitted = true;
                return Some(RecognizedGesture::LongPress {
                    pos: track.current_pos,
                });
            }
        }
        None
    }

    /// Reset to `Idle` — cancels any in-flight gesture without
    /// emitting.
    ///
    /// Test-gated. The doc used to say "called on context loss /
    /// window minimize"; neither run loop has ever called it, on
    /// either target. Its contract stays pinned by the recognizer
    /// tests so the day a suspend / visibility-change handler lands,
    /// the behavior it needs is already specified.
    #[cfg(test)]
    pub fn reset(&mut self) {
        self.state = State::Idle;
    }

    fn on_started(&mut self, id: u64, pos: (f64, f64), now: Instant) -> Option<RecognizedGesture> {
        match self.state {
            State::Idle => {
                self.state = State::OneFinger {
                    track: FingerTrack::new(id, pos, now),
                    discrete_emitted: false,
                };
                None
            }
            State::OneFinger { track, .. } => {
                let secondary = FingerTrack::new(id, pos, now);
                self.state = State::TwoFingers {
                    primary: track,
                    secondary,
                    last_emit_centroid: midpoint(track.current_pos, secondary.current_pos),
                    last_emit_span: separation(track.current_pos, secondary.current_pos),
                };
                None
            }
            State::TwoFingers { .. } => {
                // Third finger landed — outside the supported
                // gesture vocabulary. Stay in TwoFingers; ignore
                // the third finger's events. (winit will route
                // its `Moved`/`Ended` to us; we'll filter by id.)
                None
            }
        }
    }

    fn on_moved(&mut self, id: u64, pos: (f64, f64)) -> Option<RecognizedGesture> {
        let move_threshold_sq = self.move_threshold_sq;
        match &mut self.state {
            State::Idle => None,
            State::OneFinger { track, .. } => {
                if track.id != id {
                    return None;
                }
                // The sample that crosses the threshold measures
                // from where the finger *landed*, every later one
                // from the previous sample. Summing the deltas of
                // an episode therefore lands on the finger's net
                // displacement — measuring the first step from the
                // previous sample instead would silently keep the
                // threshold's worth of slop, and measuring every
                // step from `started_pos` would re-apply the whole
                // travel on every sample.
                let from = if track.has_moved {
                    track.current_pos
                } else {
                    track.started_pos
                };
                track.update_pos(pos, move_threshold_sq);
                track.has_moved.then_some(RecognizedGesture::Pan {
                    pos,
                    delta: (pos.0 - from.0, pos.1 - from.1),
                })
            }
            State::TwoFingers {
                primary,
                secondary,
                last_emit_centroid,
                last_emit_span,
            } => {
                if primary.id == id {
                    primary.update_pos(pos, move_threshold_sq);
                } else if secondary.id == id {
                    secondary.update_pos(pos, move_threshold_sq);
                } else {
                    return None;
                }
                let centroid = midpoint(primary.current_pos, secondary.current_pos);
                let span = separation(primary.current_pos, secondary.current_pos);
                let pan = (
                    centroid.0 - last_emit_centroid.0,
                    centroid.1 - last_emit_centroid.1,
                );
                let span_step = span - *last_emit_span;
                // Either half of the transform can carry the step
                // on its own: fingers spreading around a fixed
                // midpoint move the centroid not at all, and
                // fingers translating in parallel change the
                // separation not at all. Gating on the midpoint
                // alone — which is what the pre-#35 two-finger
                // check did — makes a pure pinch unrecognizable.
                if pan.0 * pan.0 + pan.1 * pan.1 <= move_threshold_sq
                    && span_step * span_step <= move_threshold_sq
                {
                    return None;
                }
                let scale = if *last_emit_span >= MIN_PINCH_SPAN_PX && span >= MIN_PINCH_SPAN_PX {
                    span / *last_emit_span
                } else {
                    1.0
                };
                *last_emit_centroid = centroid;
                *last_emit_span = span;
                Some(RecognizedGesture::PinchStep {
                    center: centroid,
                    pan,
                    scale,
                })
            }
        }
    }

    /// `may_tap` is false when the system cancelled the touch: the
    /// slot clears exactly as it does for a deliberate lift, and no
    /// gesture is emitted.
    fn on_lifted(&mut self, id: u64, now: Instant, may_tap: bool) -> Option<RecognizedGesture> {
        match self.state {
            State::Idle => None,
            State::OneFinger {
                track,
                discrete_emitted,
            } => {
                if track.id != id {
                    return None;
                }
                self.state = State::Idle;
                // A tap is the episode that produced nothing else:
                // no long-press (which `discrete_emitted` records),
                // no pan (which `has_moved` records), and short
                // enough that the long-press timer had not come due.
                // The last clause is what makes a hold whose `tick`
                // never ran — the wake-up gap
                // `dispatch_touch_event` documents — read as the
                // long hold it was rather than as a tap.
                let quick = now.saturating_duration_since(track.started_at) < self.long_press;
                (may_tap && !discrete_emitted && !track.has_moved && quick).then_some(
                    RecognizedGesture::Tap {
                        pos: track.current_pos,
                    },
                )
            }
            State::TwoFingers {
                primary, secondary, ..
            } => {
                // Demote to the surviving finger, keeping its own
                // history: a finger already past the drag threshold
                // goes on panning, so lifting one finger out of a
                // pinch continues the gesture instead of dropping
                // it. `discrete_emitted` is the one thing set — the
                // survivor of a two-finger gesture is not a tap
                // candidate and not a long-press candidate, whatever
                // its own timings say.
                let survivor = if primary.id == id {
                    secondary
                } else if secondary.id == id {
                    primary
                } else {
                    return None;
                };
                self.state = State::OneFinger {
                    track: survivor,
                    discrete_emitted: true,
                };
                None
            }
        }
    }
}

fn midpoint(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5)
}

/// Distance between two touch points — the "span" a pinch scales.
/// The one `sqrt` in the module, and it is unavoidable: a ratio of
/// squared distances is the square of the ratio the camera wants.
fn separation(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (a.0 - b.0, a.1 - b.1);
    (dx * dx + dy * dy).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::app::POINTER_DRAG_THRESHOLD_PX;
    use baumhard::util::geometry::almost_equal_f64;

    /// Test thresholds. Tight long-press (10ms) so tests don't
    /// sleep; same move threshold as production so distance math
    /// matches the real recognizer.
    const TEST_LONG_PRESS: Duration = Duration::from_millis(10);

    fn r() -> TouchGestureRecognizer {
        TouchGestureRecognizer::with_thresholds(TEST_LONG_PRESS, POINTER_DRAG_THRESHOLD_PX)
    }

    fn t0() -> Instant {
        Instant::now()
    }

    /// Two fingers down at `a` and `b`, nothing moved yet.
    fn two_fingers_down(a: (f64, f64), b: (f64, f64)) -> (TouchGestureRecognizer, Instant) {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, a, t);
        rec.ingest(Phase::Started, 2, b, t);
        (rec, t)
    }

    /// The `(pan, scale)` a `PinchStep` carries, or a panic naming
    /// what arrived instead. Keeps every pinch assertion below to
    /// the two numbers it is about.
    fn pinch_parts(g: Option<RecognizedGesture>) -> ((f64, f64), f64) {
        match g {
            Some(RecognizedGesture::PinchStep { pan, scale, .. }) => (pan, scale),
            other => panic!("expected a PinchStep, got {other:?}"),
        }
    }

    // ── Long press ──────────────────────────────────────────────

    /// `LongPress` fires when one finger is held in place past
    /// the threshold with no movement, and the latch stops a
    /// continued hold from re-firing it.
    ///
    /// Fails if `tick` stops latching `discrete_emitted` (the third
    /// assertion fires) or stops comparing against `long_press`
    /// (the first).
    #[test]
    fn test_long_press_fires_after_threshold_with_no_movement() {
        let mut rec = r();
        let t = t0();
        assert!(rec.ingest(Phase::Started, 1, (100.0, 200.0), t).is_none());
        assert!(rec.tick(t + Duration::from_millis(5)).is_none());
        let g = rec.tick(t + Duration::from_millis(15)).expect("LongPress");
        assert_eq!(g, RecognizedGesture::LongPress { pos: (100.0, 200.0) });
        assert!(rec.tick(t + Duration::from_millis(20)).is_none());
    }

    /// Movement past the threshold turns the episode into a pan and
    /// so cancels the long-press.
    ///
    /// Fails if `tick` stops reading `has_moved` — the finger has
    /// travelled 10px, four times the budget, and would still be a
    /// long-press candidate.
    #[test]
    fn test_long_press_cancelled_by_movement_past_threshold() {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, (100.0, 200.0), t);
        assert_eq!(
            rec.ingest(Phase::Moved, 1, (110.0, 200.0), t + Duration::from_millis(2)),
            Some(RecognizedGesture::Pan {
                pos: (110.0, 200.0),
                delta: (10.0, 0.0),
            }),
        );
        assert!(rec.tick(t + Duration::from_millis(15)).is_none());
    }

    /// Movement under the threshold doesn't cancel — the user's
    /// finger jitter shouldn't kill the long-press, and it emits no
    /// pan either.
    ///
    /// Fails if the `has_moved` latch loses its `>` (a 2px drift in
    /// a 5px budget would latch), or if `on_moved` emits a `Pan`
    /// before the latch is set.
    #[test]
    fn test_long_press_survives_sub_threshold_jitter() {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, (100.0, 200.0), t);
        assert!(rec
            .ingest(Phase::Moved, 1, (102.0, 200.0), t + Duration::from_millis(2))
            .is_none());
        let g = rec
            .tick(t + Duration::from_millis(15))
            .expect("LongPress despite jitter");
        // Long-press emits the *current* position, not the
        // started position — surfaces the jittered location.
        assert_eq!(g, RecognizedGesture::LongPress { pos: (102.0, 200.0) });
    }

    /// A second finger landing transitions OneFinger →
    /// TwoFingers; long-press never fires from that point.
    #[test]
    fn test_second_finger_cancels_long_press_path() {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, (100.0, 200.0), t);
        rec.ingest(Phase::Started, 2, (200.0, 200.0), t + Duration::from_millis(2));
        assert!(rec.tick(t + Duration::from_millis(15)).is_none());
    }

    /// The long-press cancel latch reads the recognizer's own
    /// threshold, not the module constant. Both directions are
    /// exercised on the one path: a loosened budget must let a
    /// 10px drift keep the candidate alive, and a tightened one
    /// must kill it at 2px.
    ///
    /// The input that makes it fail is the shape this replaced,
    /// where `FingerTrack::update_pos` compared against the
    /// module constant directly: the loose case then cancels at
    /// 10px and the first assertion fires, leaving
    /// `TouchGestureRecognizer::with_thresholds` half-honored —
    /// it configured the two-finger step and not this one.
    #[test]
    fn test_long_press_cancel_honors_the_configured_move_threshold() {
        let t = t0();

        let mut loose = TouchGestureRecognizer::with_thresholds(TEST_LONG_PRESS, 40.0);
        loose.ingest(Phase::Started, 1, (100.0, 200.0), t);
        loose.ingest(Phase::Moved, 1, (110.0, 200.0), t + Duration::from_millis(2));
        assert_eq!(
            loose.tick(t + Duration::from_millis(15)),
            Some(RecognizedGesture::LongPress { pos: (110.0, 200.0) }),
            "a 10px drift is inside a 40px move budget; the candidate must survive it"
        );

        let mut tight = TouchGestureRecognizer::with_thresholds(TEST_LONG_PRESS, 1.0);
        tight.ingest(Phase::Started, 1, (100.0, 200.0), t);
        tight.ingest(Phase::Moved, 1, (102.0, 200.0), t + Duration::from_millis(2));
        assert!(
            tight.tick(t + Duration::from_millis(15)).is_none(),
            "a 2px drift is outside a 1px move budget; the candidate must be cancelled"
        );
    }

    // ── Tap ─────────────────────────────────────────────────────

    /// A finger down and back up inside the budget, having not
    /// moved, is a tap — and the episode is over, so a later tick
    /// produces nothing.
    ///
    /// Fails if `on_lifted` stops emitting (assertion 1) or stops
    /// returning to `Idle` (assertion 2 — a `OneFinger` left
    /// standing would fire a stale `LongPress`).
    #[test]
    fn test_tap_fires_on_a_quick_lift_that_did_not_move() {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, (100.0, 200.0), t);
        assert_eq!(
            rec.ingest(Phase::Ended, 1, (100.0, 200.0), t + Duration::from_millis(5)),
            Some(RecognizedGesture::Tap { pos: (100.0, 200.0) }),
        );
        assert!(rec.tick(t + Duration::from_millis(50)).is_none());
    }

    /// The tap reports where the finger came to rest, not where it
    /// landed — a 2px drift inside the budget still taps, at the
    /// drifted point.
    ///
    /// Fails if `on_lifted` reports `started_pos`: the position in
    /// the assertion then comes back as `(100.0, 200.0)`.
    #[test]
    fn test_tap_reports_the_position_the_finger_rested_at() {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, (100.0, 200.0), t);
        rec.ingest(Phase::Moved, 1, (102.0, 200.0), t + Duration::from_millis(2));
        assert_eq!(
            rec.ingest(Phase::Ended, 1, (102.0, 200.0), t + Duration::from_millis(5)),
            Some(RecognizedGesture::Tap { pos: (102.0, 200.0) }),
        );
    }

    /// **The tap that moved too far.** Past the drag threshold the
    /// episode is a pan, and a pan does not end in a selection
    /// change — otherwise every one-finger canvas drag would also
    /// reselect whatever it finished over.
    ///
    /// Fails if `on_lifted` stops consulting `has_moved`.
    #[test]
    fn test_tap_is_refused_when_the_finger_moved_past_the_threshold() {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, (100.0, 200.0), t);
        rec.ingest(Phase::Moved, 1, (140.0, 200.0), t + Duration::from_millis(2));
        assert!(rec
            .ingest(Phase::Ended, 1, (140.0, 200.0), t + Duration::from_millis(5))
            .is_none());
    }

    /// **The tap that was held too long, with the timer running.**
    /// The hold already emitted a `LongPress`; the lift must not
    /// also select.
    ///
    /// Fails if `on_lifted` stops consulting `discrete_emitted`.
    #[test]
    fn test_tap_is_refused_after_a_long_press_fired() {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, (100.0, 200.0), t);
        assert!(rec.tick(t + Duration::from_millis(15)).is_some());
        assert!(rec
            .ingest(Phase::Ended, 1, (100.0, 200.0), t + Duration::from_millis(20))
            .is_none());
    }

    /// **The tap that was held too long, with the timer never
    /// run.** No `tick` happens at all here, which is the wake-up
    /// gap `dispatch_touch_event` documents: nothing latched
    /// `discrete_emitted`, so only the duration clause is left to
    /// refuse this, and it must.
    ///
    /// Fails if `on_lifted` drops the `< self.long_press` clause —
    /// a finger parked for a full second then lifted would select.
    #[test]
    fn test_tap_is_refused_for_a_long_hold_whose_timer_never_ran() {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, (100.0, 200.0), t);
        assert!(rec
            .ingest(Phase::Ended, 1, (100.0, 200.0), t + Duration::from_millis(1000))
            .is_none());
    }

    /// The tap and the long-press windows are disjoint at the
    /// boundary: `tick` claims `>= long_press`, so the tap must
    /// claim strictly less. Exactly at the budget, the lift is not
    /// a tap.
    ///
    /// Fails if `on_lifted` uses `<=`.
    #[test]
    fn test_tap_and_long_press_windows_do_not_overlap_at_the_boundary() {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, (100.0, 200.0), t);
        assert!(rec
            .ingest(Phase::Ended, 1, (100.0, 200.0), t + TEST_LONG_PRESS)
            .is_none());
    }

    /// **A cancelled touch is not a tap.** The system taking the
    /// finger away — a notification shade, palm rejection — clears
    /// the slot exactly as a lift does, and commits nothing.
    ///
    /// Fails if `Phase::Cancelled` is folded back onto
    /// `Phase::Ended`, which is what `touch_phase` did before the
    /// vocabulary grew a tap: the first assertion then returns a
    /// `Tap`, and every interrupted finger reselects the canvas.
    #[test]
    fn test_a_cancelled_touch_neither_taps_nor_leaves_state_behind() {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, (100.0, 200.0), t);
        assert!(rec
            .ingest(Phase::Cancelled, 1, (100.0, 200.0), t + Duration::from_millis(5))
            .is_none());
        // The slot really is clear: a stale `OneFinger` would fire a
        // long-press from a finger that is no longer on the glass.
        assert!(rec.tick(t + Duration::from_millis(50)).is_none());
    }

    /// A cancelled finger out of a pair leaves the survivor tracked,
    /// exactly as a lift does — the system took one finger, not the
    /// gesture.
    #[test]
    fn test_a_cancelled_finger_out_of_a_pair_leaves_the_survivor_panning() {
        let (mut rec, t) = two_fingers_down((0.0, 0.0), (100.0, 0.0));
        rec.ingest(Phase::Moved, 2, (100.0, 30.0), t);
        assert!(rec.ingest(Phase::Cancelled, 1, (0.0, 0.0), t).is_none());
        assert_eq!(
            rec.ingest(Phase::Moved, 2, (100.0, 40.0), t),
            Some(RecognizedGesture::Pan {
                pos: (100.0, 40.0),
                delta: (0.0, 10.0),
            }),
        );
    }

    /// A lift before the budget with no movement is a tap, and the
    /// long-press that would have come due never arrives.
    #[test]
    fn test_a_lift_before_the_budget_taps_and_cancels_the_long_press() {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, (100.0, 200.0), t);
        assert!(rec
            .ingest(Phase::Ended, 1, (100.0, 200.0), t + Duration::from_millis(5))
            .is_some());
        assert!(rec.tick(t + Duration::from_millis(15)).is_none());
    }

    // ── One-finger pan ──────────────────────────────────────────

    /// The sample that crosses the threshold carries the travel
    /// since the finger *landed*, so no part of the gesture is
    /// swallowed by the threshold.
    ///
    /// Fails if `on_moved` measures the first step from the
    /// previous sample: the delta then comes back as `(7.0, 0.0)`
    /// — the 3px of sub-threshold drift before it would be lost.
    #[test]
    fn test_pan_first_step_carries_the_travel_since_the_finger_landed() {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, (100.0, 100.0), t);
        assert!(rec.ingest(Phase::Moved, 1, (103.0, 100.0), t).is_none());
        assert_eq!(
            rec.ingest(Phase::Moved, 1, (110.0, 100.0), t),
            Some(RecognizedGesture::Pan {
                pos: (110.0, 100.0),
                delta: (10.0, 0.0),
            }),
        );
    }

    /// Every later sample carries only its own movement.
    ///
    /// Fails if `on_moved` keeps measuring from `started_pos` after
    /// the crossing: the second delta comes back as `(15.0, 0.0)`
    /// and the canvas travels twice as far as the finger.
    #[test]
    fn test_pan_steps_after_the_first_carry_the_per_sample_delta() {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, (100.0, 100.0), t);
        rec.ingest(Phase::Moved, 1, (110.0, 100.0), t);
        assert_eq!(
            rec.ingest(Phase::Moved, 1, (115.0, 100.0), t),
            Some(RecognizedGesture::Pan {
                pos: (115.0, 100.0),
                delta: (5.0, 0.0),
            }),
        );
    }

    /// The property the two rules above exist for: the deltas of a
    /// whole episode sum to the finger's net displacement. This is
    /// the assertion that fails for *either* mis-measurement, and
    /// it is computed from the input coordinates rather than from
    /// the recognizer.
    #[test]
    fn test_pan_deltas_sum_to_the_net_finger_displacement() {
        let mut rec = r();
        let t = t0();
        let path = [
            (100.0, 100.0),
            (103.0, 100.0),
            (110.0, 100.0),
            (115.0, 104.0),
            (120.0, 111.0),
        ];
        rec.ingest(Phase::Started, 1, path[0], t);
        let mut sum = (0.0f64, 0.0f64);
        for step in &path[1..] {
            if let Some(RecognizedGesture::Pan { delta, .. }) = rec.ingest(Phase::Moved, 1, *step, t) {
                sum = (sum.0 + delta.0, sum.1 + delta.1);
            }
        }
        let last = path[path.len() - 1];
        let net = (last.0 - path[0].0, last.1 - path[0].1);
        assert!(
            almost_equal_f64(sum.0, net.0) && almost_equal_f64(sum.1, net.1),
            "pan deltas summed to {sum:?}, finger moved {net:?}"
        );
    }

    /// A `Moved` for a finger the machine never saw start is
    /// ignored: it neither pans nor disturbs the tracked finger.
    ///
    /// Fails if `on_moved`'s `OneFinger` arm drops its id check —
    /// the bogus sample would latch `has_moved` on the real finger
    /// and the long-press would never come.
    #[test]
    fn test_pan_ignores_a_moved_for_an_untracked_finger() {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, (100.0, 100.0), t);
        assert!(rec.ingest(Phase::Moved, 999, (500.0, 500.0), t).is_none());
        let g = rec
            .tick(t + Duration::from_millis(15))
            .expect("LongPress despite untracked Moved");
        assert_eq!(g, RecognizedGesture::LongPress { pos: (100.0, 100.0) });
    }

    /// An `Ended` for a finger the machine never saw start is
    /// ignored — the peer of the `Moved` case above, on the lift
    /// path, and the one that was missing.
    ///
    /// Two things go wrong without the guard, and both are asserted:
    /// the stale lift **emits a phantom `Tap`** at the tracked
    /// finger's position, committing a selection the user never
    /// asked for; and it drops the machine to `Idle` while a finger
    /// is still on the glass, so the very next real sample from that
    /// finger produces nothing and an in-flight pan dies mid-gesture.
    ///
    /// Fails if `on_lifted`'s `OneFinger` arm drops `if track.id !=
    /// id`. The whole suite stayed green without it until this test
    /// existed — `on_moved`'s identical guard was covered and this
    /// one was not.
    #[test]
    fn test_a_lift_for_an_untracked_finger_is_ignored() {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, (100.0, 100.0), t);
        assert!(
            rec.ingest(Phase::Ended, 999, (500.0, 500.0), t + Duration::from_millis(1))
                .is_none(),
            "a lift for a finger that never landed must not emit a Tap"
        );
        // The tracked finger is still down: it can still pan, which
        // it could not if the stale lift had cleared the slot.
        assert_eq!(
            rec.ingest(Phase::Moved, 1, (120.0, 100.0), t + Duration::from_millis(2)),
            Some(RecognizedGesture::Pan {
                pos: (120.0, 100.0),
                delta: (20.0, 0.0),
            }),
            "the tracked finger must still be tracked after a stranger's lift"
        );
    }

    /// The reachable sequence the guard exists for, end to end: a
    /// third finger lands during a two-finger gesture and is
    /// deliberately ignored, one of the pair lifts, and then the
    /// **third** finger's own lift arrives. It must change nothing —
    /// the survivor of the pinch is still panning.
    ///
    /// Fails the same way as the test above, and this is the shape a
    /// real hand produces: a stray finger resting on the glass
    /// during a pinch outlives the gesture it was never part of.
    #[test]
    fn test_a_stray_third_finger_lifting_does_not_end_the_survivors_pan() {
        let (mut rec, t) = two_fingers_down((0.0, 0.0), (100.0, 0.0));
        rec.ingest(Phase::Started, 3, (500.0, 500.0), t);
        rec.ingest(Phase::Moved, 1, (0.0, 30.0), t);
        rec.ingest(Phase::Ended, 2, (100.0, 0.0), t);
        assert!(
            rec.ingest(Phase::Ended, 3, (500.0, 500.0), t).is_none(),
            "the stray finger's lift must emit nothing"
        );
        assert_eq!(
            rec.ingest(Phase::Moved, 1, (0.0, 40.0), t),
            Some(RecognizedGesture::Pan {
                pos: (0.0, 40.0),
                delta: (0.0, 10.0),
            }),
            "the surviving finger must go on panning across the stray lift"
        );
    }

    // ── Two-finger pinch / pan ──────────────────────────────────

    /// **A spread is zoom without net translation.** Two fingers
    /// moving symmetrically apart arrive as two samples, so the
    /// midpoint wobbles by half a finger-step on each; what the
    /// gesture means is the pair, and the pair sums to no
    /// translation and multiplies to the separation ratio.
    ///
    /// Fails if `scale` is computed as a difference rather than a
    /// ratio (the product assertion), or if the emitted `pan` stops
    /// being measured from the previous emission (the sum
    /// assertion).
    #[test]
    fn test_pinch_reports_a_spread_as_scale_with_no_net_pan() {
        let (mut rec, t) = two_fingers_down((0.0, 0.0), (100.0, 0.0));
        let (pan_a, scale_a) = pinch_parts(rec.ingest(Phase::Moved, 1, (-10.0, 0.0), t));
        let (pan_b, scale_b) = pinch_parts(rec.ingest(Phase::Moved, 2, (110.0, 0.0), t));
        assert!(
            almost_equal_f64(pan_a.0 + pan_b.0, 0.0) && almost_equal_f64(pan_a.1 + pan_b.1, 0.0),
            "a symmetric spread must not translate the camera; got {pan_a:?} then {pan_b:?}"
        );
        // 100px apart to 120px apart, independently of the machine.
        assert!(
            almost_equal_f64(scale_a * scale_b, 120.0 / 100.0),
            "scales {scale_a} and {scale_b} must compose to the separation ratio"
        );
    }

    /// **The pinch that is really a pan.** Two fingers translating
    /// in parallel change their separation not at all, so the
    /// scales of the step pair must cancel to exactly 1.0 while
    /// the pans sum to the translation.
    ///
    /// Fails if the emission gates on the midpoint alone and
    /// reports no scale, or if `scale` ever becomes additive: the
    /// product then lands on 2.0 rather than 1.0.
    #[test]
    fn test_pinch_reports_a_parallel_two_finger_move_as_pan_with_unit_scale() {
        let (mut rec, t) = two_fingers_down((0.0, 0.0), (100.0, 0.0));
        let (pan_a, scale_a) = pinch_parts(rec.ingest(Phase::Moved, 1, (30.0, 0.0), t));
        let (pan_b, scale_b) = pinch_parts(rec.ingest(Phase::Moved, 2, (130.0, 0.0), t));
        assert!(
            almost_equal_f64(pan_a.0 + pan_b.0, 30.0) && almost_equal_f64(pan_a.1 + pan_b.1, 0.0),
            "the pair must translate the camera by the fingers' 30px; got {pan_a:?} then {pan_b:?}"
        );
        assert!(
            almost_equal_f64(scale_a * scale_b, 1.0),
            "parallel fingers hold their separation, so {scale_a} * {scale_b} must be 1.0"
        );
    }

    /// **Two-finger jitter.** A 2px twitch on one finger moves the
    /// midpoint 1px and the separation 2px — both inside the
    /// budget — so the camera must not move at all.
    ///
    /// Fails if either gate loses its `<=`, or if the emission is
    /// made unconditional.
    #[test]
    fn test_pinch_does_not_fire_on_two_finger_jitter() {
        let (mut rec, t) = two_fingers_down((0.0, 0.0), (100.0, 0.0));
        assert!(rec.ingest(Phase::Moved, 1, (2.0, 0.0), t).is_none());
        assert!(rec.ingest(Phase::Moved, 2, (98.0, 0.0), t).is_none());
    }

    /// Each step past the budget fires once, so a continuous
    /// two-finger drag produces a stream rather than one emission.
    ///
    /// Fails if the baselines stop being re-stamped on emission —
    /// the second step's `pan` then measures from the gesture's
    /// start and double-counts the first.
    #[test]
    fn test_pinch_emits_once_per_step() {
        let (mut rec, t) = two_fingers_down((0.0, 0.0), (100.0, 0.0));
        let (pan_a, _) = pinch_parts(rec.ingest(Phase::Moved, 1, (0.0, 20.0), t));
        let (pan_b, _) = pinch_parts(rec.ingest(Phase::Moved, 1, (0.0, 40.0), t));
        assert!(
            almost_equal_f64(pan_a.1, 10.0) && almost_equal_f64(pan_b.1, 10.0),
            "each 20px finger step moves the midpoint 10px; got {pan_a:?} then {pan_b:?}"
        );
    }

    /// Two fingers reported at the same point give no baseline to
    /// form a ratio against. The step still translates, and its
    /// scale is exactly 1.0 rather than an infinity that would
    /// slam the camera into its zoom clamp.
    ///
    /// Fails if [`MIN_PINCH_SPAN_PX`] stops guarding the division:
    /// `50.0 / 0.0` is `inf`, and `assert_eq!(scale, 1.0)` catches
    /// it.
    #[test]
    fn test_pinch_scale_is_unit_when_the_baseline_separation_is_degenerate() {
        let (mut rec, t) = two_fingers_down((50.0, 50.0), (50.0, 50.0));
        let (_, scale) = pinch_parts(rec.ingest(Phase::Moved, 2, (100.0, 50.0), t));
        assert_eq!(scale, 1.0);
    }

    /// A third finger landing is ignored, and so are its moves —
    /// the midpoint is the first two fingers' and nothing else's.
    ///
    /// Fails if `on_started`'s `TwoFingers` arm starts tracking the
    /// third finger: the first assertion then emits, because a jump
    /// from (1000, 1000) to (2000, 2000) is very far past the
    /// budget.
    #[test]
    fn test_third_finger_does_not_disrupt_the_pinch() {
        let (mut rec, t) = two_fingers_down((0.0, 0.0), (100.0, 0.0));
        rec.ingest(Phase::Started, 3, (1000.0, 1000.0), t);
        assert!(rec.ingest(Phase::Moved, 3, (2000.0, 2000.0), t).is_none());
        let (pan, _) = pinch_parts(rec.ingest(Phase::Moved, 1, (0.0, 20.0), t));
        assert!(
            almost_equal_f64(pan.0, 0.0) && almost_equal_f64(pan.1, 10.0),
            "only fingers 1 and 2 may drive the midpoint; got {pan:?}"
        );
    }

    // ── Finger-lift ordering ────────────────────────────────────

    /// Lifting the finger that landed first hands the gesture to
    /// the second, which goes on panning with its own history —
    /// the user who lifts one finger mid-pinch keeps dragging.
    ///
    /// Fails if the demotion rebuilds the survivor's track rather
    /// than carrying it: a reset `has_moved` makes the next 10px
    /// sample emit nothing, and a reset `current_pos` makes its
    /// delta wrong.
    #[test]
    fn test_lifting_the_first_finger_leaves_the_second_panning() {
        let (mut rec, t) = two_fingers_down((0.0, 0.0), (100.0, 0.0));
        rec.ingest(Phase::Moved, 2, (100.0, 30.0), t);
        assert!(rec.ingest(Phase::Ended, 1, (0.0, 0.0), t).is_none());
        assert_eq!(
            rec.ingest(Phase::Moved, 2, (100.0, 40.0), t),
            Some(RecognizedGesture::Pan {
                pos: (100.0, 40.0),
                delta: (0.0, 10.0),
            }),
        );
    }

    /// The mirror: lifting the finger that landed second hands the
    /// gesture to the first. Which finger leaves changes which
    /// `FingerTrack` survives, and both must.
    ///
    /// Fails if `on_lifted`'s `TwoFingers` arm handles only one of
    /// the two ids.
    #[test]
    fn test_lifting_the_second_finger_leaves_the_first_panning() {
        let (mut rec, t) = two_fingers_down((0.0, 0.0), (100.0, 0.0));
        rec.ingest(Phase::Moved, 1, (0.0, 30.0), t);
        assert!(rec.ingest(Phase::Ended, 2, (100.0, 0.0), t).is_none());
        assert_eq!(
            rec.ingest(Phase::Moved, 1, (0.0, 40.0), t),
            Some(RecognizedGesture::Pan {
                pos: (0.0, 40.0),
                delta: (0.0, 10.0),
            }),
        );
    }

    /// The finger left over from a two-finger gesture is not a tap
    /// candidate, whichever finger left first — a two-finger
    /// gesture must never end in a selection change.
    ///
    /// Fails if the demotion stops setting `discrete_emitted`: the
    /// second lift is then quick, unmoved and unspent, and taps.
    #[test]
    fn test_tap_is_refused_for_the_finger_left_over_from_a_pinch() {
        for lift_first in [1u64, 2u64] {
            let (mut rec, t) = two_fingers_down((0.0, 0.0), (100.0, 0.0));
            let lift_second = if lift_first == 1 { 2 } else { 1 };
            assert!(rec.ingest(Phase::Ended, lift_first, (0.0, 0.0), t).is_none());
            assert!(
                rec.ingest(Phase::Ended, lift_second, (100.0, 0.0), t).is_none(),
                "lifting {lift_first} then {lift_second} must not tap"
            );
        }
    }

    /// The survivor of a two-finger gesture is not a long-press
    /// candidate either, even held well past the budget.
    #[test]
    fn test_lifting_one_of_two_fingers_does_not_trigger_long_press() {
        let (mut rec, t) = two_fingers_down((100.0, 100.0), (200.0, 100.0));
        rec.ingest(Phase::Ended, 1, (100.0, 100.0), t + Duration::from_millis(5));
        assert!(rec.tick(t + Duration::from_millis(50)).is_none());
    }

    /// Lifting both fingers returns the recognizer to Idle, and a
    /// fresh finger after that is an ordinary long-press candidate.
    ///
    /// Fails if `on_lifted` leaves `discrete_emitted` set across the
    /// return to `Idle` — the new episode would inherit a spent
    /// latch and never fire.
    #[test]
    fn test_back_to_idle_after_both_fingers_lift() {
        let (mut rec, t) = two_fingers_down((100.0, 100.0), (200.0, 100.0));
        rec.ingest(Phase::Ended, 1, (100.0, 100.0), t);
        rec.ingest(Phase::Ended, 2, (200.0, 100.0), t);
        let t1 = t + Duration::from_millis(100);
        rec.ingest(Phase::Started, 3, (50.0, 50.0), t1);
        assert_eq!(
            rec.tick(t1 + Duration::from_millis(15)),
            Some(RecognizedGesture::LongPress { pos: (50.0, 50.0) }),
        );
    }

    /// `reset()` clears state regardless of variant.
    ///
    /// It covers **no runtime path** — this doc used to claim it
    /// covered "the runtime path that responds to context loss /
    /// window minimization", the same claim the method's own doc
    /// carried and that was corrected there while this copy was
    /// left standing. Neither run loop has ever called `reset`, on
    /// either target. What the test is for is stated on the method:
    /// the contract stays pinned so the day a suspend /
    /// visibility-change handler lands, the behavior it needs is
    /// already specified.
    #[test]
    fn test_reset_clears_state_from_any_variant() {
        let mut rec = r();
        let t = t0();
        rec.ingest(Phase::Started, 1, (100.0, 100.0), t);
        rec.reset();
        assert!(rec.tick(t + Duration::from_millis(50)).is_none());
        rec.ingest(Phase::Started, 1, (100.0, 100.0), t);
        rec.ingest(Phase::Started, 2, (200.0, 100.0), t);
        rec.reset();
        let t1 = t + Duration::from_millis(100);
        rec.ingest(Phase::Started, 3, (50.0, 50.0), t1);
        assert!(rec.tick(t1 + Duration::from_millis(15)).is_some());
    }

    // ── Threshold parity ────────────────────────────────────────

    /// The **touch** side of the one-threshold-for-every-pointer
    /// claim, on the path that is the mouse's exact peer: one
    /// pointer travels, and promotes to a drag after exactly the
    /// travel [`POINTER_DRAG_THRESHOLD_SQ_PX`] allows. The
    /// reference distance is read back out of that constant and
    /// driven through a *production* [`TouchGestureRecognizer`], so
    /// the assertion spans the recognizer rather than restating the
    /// number.
    ///
    /// The input that makes it fail is the shape this replaced: a
    /// touch-local `MOVE_THRESHOLD_PX = 4.0`. A 4.99px step is
    /// inside the mouse's budget and outside that one, so the first
    /// assertion fires.
    ///
    /// **What it does not catch**, stated because the claim it used to
    /// carry was false: re-inlining `25.0` in `event_cursor_moved.rs`
    /// leaves this test green. Its "mouse side" is the same constant
    /// the recognizer reads, so the two ends of the comparison are two
    /// derivations from one source — planting that literal leaves the
    /// whole `mandala` suite green but for the one test written for it,
    /// `event_cursor_moved::tests::test_the_mouse_drag_arms_name_the_shared_pointer_threshold`,
    /// which reads the mouse arms out of the source. That pin and this
    /// test are the parity claim together.
    #[test]
    fn test_pointer_drag_threshold_is_shared_by_touch_and_mouse() {
        let budget_px = POINTER_DRAG_THRESHOLD_SQ_PX.sqrt();
        let t = t0();

        let mut under = TouchGestureRecognizer::new();
        under.ingest(Phase::Started, 1, (0.0, 0.0), t);
        assert!(
            under
                .ingest(Phase::Moved, 1, (budget_px - 0.01, 0.0), t)
                .is_none(),
            "a step of {} px is inside the {budget_px}px pointer budget and must not pan",
            budget_px - 0.01
        );

        let mut over = TouchGestureRecognizer::new();
        over.ingest(Phase::Started, 1, (0.0, 0.0), t);
        assert_eq!(
            over.ingest(Phase::Moved, 1, (budget_px + 0.01, 0.0), t),
            Some(RecognizedGesture::Pan {
                pos: (budget_px + 0.01, 0.0),
                delta: (budget_px + 0.01, 0.0),
            }),
            "a step of {} px is outside the {budget_px}px pointer budget and must pan",
            budget_px + 0.01
        );
    }

    /// The two-finger **midpoint** gate reads the same budget. A
    /// finger moving perpendicular to the finger axis barely changes
    /// the separation, so this isolates the midpoint half: a step
    /// that moves the midpoint just inside the budget is silent and
    /// one just outside it fires.
    ///
    /// Fails if the midpoint gate is given a threshold of its own.
    #[test]
    fn test_pinch_midpoint_gate_uses_the_shared_pointer_threshold() {
        let budget_px = POINTER_DRAG_THRESHOLD_SQ_PX.sqrt();
        let t = t0();
        // The midpoint moves half as far as a single finger.
        let mut under = TouchGestureRecognizer::new();
        under.ingest(Phase::Started, 1, (0.0, 0.0), t);
        under.ingest(Phase::Started, 2, (100.0, 0.0), t);
        assert!(under
            .ingest(Phase::Moved, 1, (0.0, 2.0 * (budget_px - 0.01)), t)
            .is_none());

        let mut over = TouchGestureRecognizer::new();
        over.ingest(Phase::Started, 1, (0.0, 0.0), t);
        over.ingest(Phase::Started, 2, (100.0, 0.0), t);
        assert!(over
            .ingest(Phase::Moved, 1, (0.0, 2.0 * (budget_px + 0.01)), t)
            .is_some());
    }

    /// The two-finger **separation** gate reads the same budget. A
    /// finger moving along the finger axis changes the separation
    /// twice as fast as the midpoint, so a separation step just
    /// outside the budget fires while its midpoint step — half as
    /// large — is still well inside.
    ///
    /// Fails if the separation half of the gate is dropped: the
    /// second assertion then reports nothing, because a 2.5px
    /// midpoint step cannot carry it alone.
    #[test]
    fn test_pinch_separation_gate_uses_the_shared_pointer_threshold() {
        let budget_px = POINTER_DRAG_THRESHOLD_SQ_PX.sqrt();
        let t = t0();
        let mut under = TouchGestureRecognizer::new();
        under.ingest(Phase::Started, 1, (0.0, 0.0), t);
        under.ingest(Phase::Started, 2, (100.0, 0.0), t);
        assert!(under
            .ingest(Phase::Moved, 2, (100.0 + budget_px - 0.01, 0.0), t)
            .is_none());

        let mut over = TouchGestureRecognizer::new();
        over.ingest(Phase::Started, 1, (0.0, 0.0), t);
        over.ingest(Phase::Started, 2, (100.0, 0.0), t);
        assert!(over
            .ingest(Phase::Moved, 2, (100.0 + budget_px + 0.01, 0.0), t)
            .is_some());
    }
}
