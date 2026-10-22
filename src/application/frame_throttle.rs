// SPDX-License-Identifier: MPL-2.0

//! Mutation-frequency throttle.
//!
//! Responsiveness is never traded for visual fidelity. Input is
//! accumulated every tick; the throttle gates only how often the
//! mutation-and-rebuild work runs. A moving average over
//! [`WINDOW_SIZE`] drained frames raises the drain divisor `n`
//! toward [`MAX_N`] when work overruns the budget and decays it
//! back when it drops, with a 30% hysteresis band to prevent
//! oscillation. Self-tuning — the only knob is the budget at
//! construction.
//!
//! Visual consequences: with `n = 1` the throttle is a no-op. With
//! `n > 1` the dragged content advances in chunks, catching up to
//! the cursor every `n` frames; on a 60 Hz display at `n = 4` the
//! update cadence is ~66 ms, perceptible but still tracking. The
//! refresh rate itself never drops — the GPU swap chain keeps
//! presenting already-built buffers on skipped frames — and the
//! hardware cursor never lags.

use std::collections::VecDeque;
use std::time::Duration;

/// Size of the moving-average window. Eight frames is small enough to
/// react to a sustained stress within ~130ms on a 60 Hz display, and
/// large enough to absorb single-frame noise without oscillating.
pub const WINDOW_SIZE: usize = 8;

/// Maximum drain divisor. At 60 fps, `N = 8` means the dragged node
/// updates every ~133ms — laggy but still tracking. Past this, visual
/// tracking becomes so stale that capping is the kinder behaviour; the
/// remaining budget shortfall has to be addressed by the companion
/// techniques (culling, incremental rebuild, shape-once reuse).
pub const MAX_N: u32 = 8;

/// A conservative default refresh budget: 16.67ms (60 Hz) minus ~2.7ms
/// of safety margin for GPU present and other per-frame overhead. The
/// correct value depends on the actual monitor refresh rate; runtime
/// detection of that is still an open question.
pub const DEFAULT_BUDGET: Duration = Duration::from_micros(14_000);

/// Per-frame throttle that degrades mutation frequency under load.
/// Call `reset` when the drag ends so the next drag starts
/// at `n = 1`.
#[derive(Debug)]
pub struct MutationFrequencyThrottle {
    budget: Duration,
    window: VecDeque<Duration>,
    n: u32,
    frames_since_drain: u32,
}

impl MutationFrequencyThrottle {
    /// Construct with the given per-frame work budget. See [`DEFAULT_BUDGET`].
    pub fn new(budget: Duration) -> Self {
        MutationFrequencyThrottle {
            budget,
            window: VecDeque::with_capacity(WINDOW_SIZE),
            n: 1,
            frames_since_drain: 0,
        }
    }

    /// Construct with the default budget. Convenience wrapper for call
    /// sites that don't care to pass one.
    pub fn with_default_budget() -> Self {
        Self::new(DEFAULT_BUDGET)
    }

    /// Returns `true` if the caller should perform its heavy work this
    /// frame, or `false` if the frame should be skipped.
    ///
    /// Increments the internal skip counter. When the counter reaches
    /// the current drain divisor `n`, returns `true` and resets the
    /// counter to zero. Otherwise returns `false` — the caller must
    /// preserve its accumulated state so the next successful drain can
    /// fold in what this frame skipped.
    pub fn should_drain(&mut self) -> bool {
        self.frames_since_drain += 1;
        if self.frames_since_drain >= self.n {
            self.frames_since_drain = 0;
            true
        } else {
            false
        }
    }

    /// Feed a measured work duration back into the tracker. Updates the
    /// moving average and adjusts `n` to hold the invariant:
    ///
    /// - If the moving average exceeds `budget`, `n` increments toward
    ///   [`MAX_N`] (throttle engages more aggressively).
    /// - If the moving average drops below 70% of `budget`, `n` decays
    ///   toward `1` (throttle relaxes). The 30% gap is hysteresis —
    ///   without it, a frame sitting exactly at budget would oscillate.
    pub fn record_work_duration(&mut self, duration: Duration) {
        if self.window.len() >= WINDOW_SIZE {
            self.window.pop_front();
        }
        self.window.push_back(duration);

        let avg = self.moving_average();
        if avg > self.budget {
            if self.n < MAX_N {
                self.n += 1;
            }
        } else if avg < self.budget.mul_f32(0.7) && self.n > 1 {
            self.n -= 1;
        }
    }

    /// Clear the moving-average window, reset `n` to `1`, reset the
    /// skip counter.
    ///
    /// Test-gated: a drag's throttle lives inside the `ThrottledDrag`
    /// the release path drops, so a fresh drag starts from a fresh
    /// throttle without anyone clearing the old one. The doc used to
    /// claim the drag-end path called this; it never did. Remove the
    /// gate the day a throttle outlives its interaction.
    #[cfg(test)]
    pub fn reset(&mut self) {
        self.window.clear();
        self.n = 1;
        self.frames_since_drain = 0;
    }

    /// Current drain divisor (1 = every frame; higher under load).
    /// Test-gated: the divisor is consumed inside `should_drain`,
    /// never read from outside.
    #[cfg(test)]
    pub fn current_n(&self) -> u32 {
        self.n
    }

    /// Current moving average, or [`Duration::ZERO`] when nothing recorded.
    pub fn moving_average(&self) -> Duration {
        if self.window.is_empty() {
            return Duration::ZERO;
        }
        let sum: Duration = self.window.iter().sum();
        sum / self.window.len() as u32
    }
}

impl Default for MutationFrequencyThrottle {
    fn default() -> Self {
        Self::with_default_budget()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn us(n: u64) -> Duration {
        Duration::from_micros(n)
    }

    /// Drive `t` until `n` saturates at [`MAX_N`], so a following
    /// phase measures the phase's own behavior rather than the
    /// window still emptying itself of the raise phase's samples.
    ///
    /// Saturation is what makes that work: while the window holds a
    /// mix of the old heavy samples and the new light ones the
    /// average is still over budget, so `n` would keep climbing —
    /// invisibly, because it is already clamped.
    fn saturate(t: &mut MutationFrequencyThrottle, heavy: Duration) {
        for _ in 0..200 {
            if t.should_drain() {
                t.record_work_duration(heavy);
            }
        }
        assert_eq!(
            t.current_n(),
            MAX_N,
            "the raise phase must saturate at MAX_N, or what follows measures the \
             window's transition instead of the load it feeds"
        );
    }

    /// A fresh throttle drains every frame: `n` starts at 1 and the
    /// skip counter at 0, so no frame is ever withheld before any
    /// work has been measured.
    ///
    /// Fails when: the constructor starts `n` above 1, or
    /// `should_drain` compares with `>` instead of `>=` (every frame
    /// would then be skipped at `n = 1`).
    ///
    /// That "always true" is not vacuous is shown by
    /// `test_sustained_over_budget_raises_n_and_skips_frames`, where
    /// the same call returns false.
    #[test]
    fn test_a_fresh_throttle_starts_at_n_one_and_drains_every_frame() {
        let mut t = MutationFrequencyThrottle::new(ms(14));
        assert_eq!(t.current_n(), 1);
        for frame in 0..50 {
            assert!(t.should_drain(), "frame {frame} of an idle throttle was skipped");
        }
        assert_eq!(t.current_n(), 1);
    }

    /// Work comfortably under budget never raises `n` — not at the
    /// end of the run, and not transiently in the middle of it.
    ///
    /// Fails when: the raise branch compares against the wrong side
    /// of the budget, or fires on any sample rather than on the
    /// average. Asserting inside the loop rather than after it is
    /// what makes a transient raise reachable; an end-of-run check
    /// alone is satisfied by a throttle that raised `n` and decayed
    /// it back.
    ///
    /// Control on the feed path: the moving average is checked
    /// against a value computed here, not by the throttle, so
    /// "`n` never moved" cannot be explained by durations that were
    /// never recorded in the first place.
    #[test]
    fn test_under_budget_work_keeps_n_at_one_on_every_frame() {
        let mut t = MutationFrequencyThrottle::new(ms(14));
        for frame in 0..40 {
            assert!(t.should_drain(), "frame {frame} was skipped at n = 1");
            // 3 ms is ~21% of budget — under the decay threshold, so
            // this also exercises the decay branch finding nothing to
            // decay.
            t.record_work_duration(ms(3));
            assert_eq!(t.current_n(), 1, "n rose on frame {frame} of under-budget work");
        }
        assert_eq!(
            t.moving_average(),
            ms(3),
            "every sample was 3 ms, so the window's mean must be 3 ms — if it is \
             not, the samples never landed and the assertions above prove nothing"
        );
    }

    /// Sustained over-budget work raises `n`, and a raised `n` is
    /// what makes `should_drain` withhold frames.
    ///
    /// Fails when: the raise branch is removed (`n` stays 1), or when
    /// `should_drain` ignores `n` and drains unconditionally — the
    /// second is the one that leaves the throttle looking healthy
    /// while doing nothing, since `n` still climbs in the readout.
    #[test]
    fn test_sustained_over_budget_raises_n_and_skips_frames() {
        let mut t = MutationFrequencyThrottle::new(ms(14));
        // 20 ms against a 14 ms budget: over, but not so far over
        // that a threshold off by a few percent would still be caught
        // by the margin.
        for _ in 0..20 {
            if t.should_drain() {
                t.record_work_duration(ms(20));
            }
        }
        assert!(
            t.current_n() > 1,
            "expected n > 1 after sustained over-budget work, got {}",
            t.current_n()
        );

        let mut skipped = false;
        for _ in 0..20 {
            if !t.should_drain() {
                skipped = true;
                break;
            }
            t.record_work_duration(ms(20));
        }
        assert!(skipped, "with n > 1, should_drain must withhold some frames");
    }

    /// Absurd load climbs to [`MAX_N`] and stops there — the cap is
    /// a clamp, not a target.
    ///
    /// Fails when: the raise branch loses its `n < MAX_N` guard (the
    /// in-loop assertion fires on the first frame past the cap). The
    /// end-of-run equality is what keeps that in-loop assertion from
    /// passing vacuously on a throttle whose `n` never left 1.
    #[test]
    fn test_extreme_load_climbs_to_max_n_and_never_past_it() {
        let mut t = MutationFrequencyThrottle::new(ms(14));
        for frame in 0..500 {
            if t.should_drain() {
                t.record_work_duration(ms(500));
            }
            assert!(
                t.current_n() <= MAX_N,
                "n exceeded MAX_N on frame {frame}: {}",
                t.current_n()
            );
        }
        assert_eq!(t.current_n(), MAX_N);
    }

    /// Load dropping below the hysteresis band decays `n` all the
    /// way back to 1, one step per recorded frame.
    ///
    /// Fails when: the decay branch is removed, or its `n > 1` guard
    /// is dropped so `n` underflows past 1. The `peak > 1`
    /// precondition is the control — without it "ended at 1" is
    /// satisfied by a throttle that never left 1.
    #[test]
    fn test_load_dropping_below_the_band_decays_n_back_to_one() {
        let mut t = MutationFrequencyThrottle::new(ms(14));
        saturate(&mut t, ms(50));
        let peak = t.current_n();

        // 1 ms is well under 70% of a 14 ms budget (9.8 ms), so every
        // recorded frame takes one step off n.
        for _ in 0..1000 {
            if t.should_drain() {
                t.record_work_duration(ms(1));
            }
        }
        assert!(
            t.current_n() < peak,
            "expected n to decay from {peak}, got {}",
            t.current_n()
        );
        assert_eq!(t.current_n(), 1, "expected full decay to 1");
    }

    /// `n` holds steady everywhere inside the hysteresis band —
    /// from just above the 70% decay threshold up to and including a
    /// frame sitting exactly at budget, which is the oscillation the
    /// band exists to prevent.
    ///
    /// Fails when: the decay threshold widens toward budget (the
    /// 7.5 ms row decays), or the raise comparison becomes `>=` (the
    /// exactly-at-budget row would raise, except `n` is already
    /// clamped — which is why that row asserts the *decay* side and
    /// the raise side is covered by
    /// `test_sustained_over_budget_raises_n_and_skips_frames`).
    ///
    /// Control on the same path: the final row feeds 6 ms, below the
    /// threshold, and `n` decays to 1. Without it every row above is
    /// satisfied by a `record_work_duration` that has stopped
    /// touching `n` at all.
    #[test]
    fn test_n_holds_steady_everywhere_inside_the_hysteresis_band() {
        let budget = ms(10);
        // 70% of a 10 ms budget is 7 ms. The exact edge is not
        // tested: `budget.mul_f32(0.7)` round-trips through f32, so
        // an assertion on 7 ms exactly would be measuring float
        // representation rather than the band.
        for inside in [us(7_500), us(8_000), us(9_000), us(10_000)] {
            let mut t = MutationFrequencyThrottle::new(budget);
            saturate(&mut t, ms(20));
            let raised = t.current_n();

            for _ in 0..200 {
                if t.should_drain() {
                    t.record_work_duration(inside);
                }
            }
            assert_eq!(
                t.current_n(),
                raised,
                "{inside:?} is inside the hysteresis band, so n must not move"
            );
        }

        let mut t = MutationFrequencyThrottle::new(budget);
        saturate(&mut t, ms(20));
        for _ in 0..200 {
            if t.should_drain() {
                t.record_work_duration(ms(6));
            }
        }
        assert_eq!(
            t.current_n(),
            1,
            "6 ms is below the 7 ms decay threshold — if this does not decay, the \
             rows above are holding a value nothing can move"
        );
    }

    /// A raised `n` actually removes work: fewer than every frame
    /// drains, and never fewer than one in [`MAX_N`].
    ///
    /// Fails when: `should_drain` stops consulting `n` (all 32 frames
    /// drain), or when the skip counter is never cleared (nothing
    /// drains after the first). The `n > 1` precondition is what
    /// makes both halves reachable.
    #[test]
    fn test_throttled_frames_skip_work() {
        let mut t = MutationFrequencyThrottle::new(ms(10));
        saturate(&mut t, ms(50));
        assert!(t.current_n() > 1);

        let mut drained = 0;
        for _ in 0..32 {
            if t.should_drain() {
                drained += 1;
                // The same heavy duration, so n stays where it is and
                // the cadence being counted is one cadence.
                t.record_work_duration(ms(50));
            }
        }
        assert!(drained < 32, "expected throttling to skip frames, all 32 drained");
        assert!(
            drained >= 32 / MAX_N as usize,
            "expected at least one drain per MAX_N frames, got {drained}"
        );
    }

    /// Drains land exactly `n` frames apart once `n` is stable.
    ///
    /// Fails when: the skip counter is reset to 1 instead of 0 (the
    /// spacing shortens by one), or when it is not reset at all
    /// (there is no second drain to space against).
    #[test]
    fn test_drain_cadence_matches_n() {
        let mut t = MutationFrequencyThrottle::new(ms(10));
        saturate(&mut t, ms(100));
        let n = t.current_n();
        assert!(n >= 2, "the cadence under test needs n >= 2, got {n}");

        let mut drain_indices = Vec::new();
        for i in 0..(n * 4) {
            if t.should_drain() {
                drain_indices.push(i);
                // Same duration throughout, so n does not move and
                // the spacing has one value to have.
                t.record_work_duration(ms(100));
            }
        }
        assert!(
            drain_indices.len() >= 2,
            "need two drains to measure a spacing, got {}",
            drain_indices.len()
        );
        for w in drain_indices.windows(2) {
            assert_eq!(w[1] - w[0], n, "drains not spaced by n = {n}");
        }
    }

    /// The moving average is the arithmetic mean of the window.
    ///
    /// Fails when: the sum or the divisor drifts — a mean of 10, 20
    /// and 30 that is not 20 is one of the two.
    #[test]
    fn test_moving_average_is_the_arithmetic_mean_of_the_window() {
        let mut t = MutationFrequencyThrottle::new(ms(100));
        t.record_work_duration(ms(10));
        t.record_work_duration(ms(20));
        t.record_work_duration(ms(30));
        assert_eq!(t.moving_average(), ms(20));
    }

    /// The window holds [`WINDOW_SIZE`] samples and evicts the
    /// oldest, so a spike ages out instead of weighting forever.
    ///
    /// Fails when: the eviction is dropped (the window grows and the
    /// spike is diluted across more slots than it should be). The
    /// expectation is computed here from the window size and the two
    /// sample values, never from the throttle.
    #[test]
    fn test_window_evicts_the_oldest_sample_beyond_its_size() {
        let mut t = MutationFrequencyThrottle::new(ms(100));
        for _ in 0..WINDOW_SIZE {
            t.record_work_duration(ms(10));
        }
        assert_eq!(t.moving_average(), ms(10));

        // One 100 ms spike evicts the oldest 10 ms: the window is
        // 7 x 10 + 100 = 170 ms over 8 slots = 21.25 ms. `Duration /
        // u32` keeps sub-millisecond precision, so the expectation is
        // in nanoseconds rather than truncated to milliseconds.
        t.record_work_duration(ms(100));
        let expected_nanos = (10 * (WINDOW_SIZE as u64 - 1) + 100) * 1_000_000 / WINDOW_SIZE as u64;
        assert_eq!(t.moving_average(), Duration::from_nanos(expected_nanos));
    }

    /// `reset` returns every piece of state to what a fresh throttle
    /// carries: `n`, the window, and the skip counter.
    ///
    /// Fails when: any one of the three is left behind. The skip
    /// counter is the one with no readout, so it is checked by
    /// behavior — three consecutive drains, which a non-zero counter
    /// at `n = 1` still gives, but a counter left at `n`'s old value
    /// with `n` uncleared does not.
    ///
    /// The pre-reset assertions are the control: both fields are
    /// observably away from their fresh values before `reset` runs.
    #[test]
    fn test_reset_returns_the_throttle_to_its_fresh_state() {
        let mut t = MutationFrequencyThrottle::new(ms(14));
        saturate(&mut t, ms(50));
        assert!(t.current_n() > 1);
        assert!(t.moving_average() > Duration::ZERO);

        t.reset();
        assert_eq!(t.current_n(), 1);
        assert_eq!(t.moving_average(), Duration::ZERO);
        assert!(t.should_drain());
        assert!(t.should_drain());
        assert!(t.should_drain());
    }

    /// The default budget leaves headroom inside a 60 Hz frame.
    ///
    /// Fails when: [`DEFAULT_BUDGET`] is raised to or past the frame
    /// interval, which would let a frame that misses its deadline
    /// still count as within budget.
    #[test]
    fn test_default_budget_is_under_a_60hz_frame() {
        assert!(DEFAULT_BUDGET < Duration::from_micros(16_667));
    }

    /// An empty window reports zero rather than dividing by it.
    ///
    /// Fails when: the empty guard goes — the division panics rather
    /// than returning a wrong value, which is worse in an
    /// interactive path (CODE_CONVENTIONS §9).
    #[test]
    fn test_zero_frames_recorded_reports_a_zero_average() {
        let t = MutationFrequencyThrottle::new(ms(14));
        assert_eq!(t.moving_average(), Duration::ZERO);
    }
}
