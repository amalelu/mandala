//! Timing envelope, easing curves, lerp helpers, and tick logic
//! for animated `CustomMutation`s. Any custom mutation with
//! `timing: Some(AnimationTiming { ... })` is started as an
//! `AnimationInstance` instead of applied instantly; each tick
//! evaluates the easing curve and emits a blended snapshot the
//! existing `rebuild_all` path repaints.
//!
//! This module deliberately replaces the dormant
//! [`crate::core::animation`] skeleton (left untouched per the
//! roadmap note in `DEPRECATED_ROADMAP.md` — "would cost more to
//! adapt than to replace"). The dormant types are generic over
//! `T: Mutable` and don't fit Mandala's
//! `MindMap` model + `MutatorTree` shape.
//!
//! # Architecture
//!
//! - [`AnimationTiming`] is a serializable envelope on
//!   `CustomMutation` carrying `duration_ms`, `delay_ms`,
//!   [`Easing`] curve, and an optional [`Followup`].
//! - [`AnimationInstance`] is the per-active-mutation runtime
//!   record: snapshot of from/to states, current phase, elapsed
//!   time. The application owns `Vec<AnimationInstance>` on the
//!   document and ticks each instance once per frame in
//!   `AboutToWait` (with `ControlFlow::WaitUntil` while any
//!   instance is active).
//! - [`lerp_f32`], [`lerp_vec2`], [`lerp_color`] are the
//!   per-field interpolators. Position, scale, and color fields
//!   blend continuously; structural changes (text replacement,
//!   region count shifts) snap at the boundary.
//!
//! # Composition with the §B2 mutator path
//!
//! Each tick produces a `MutatorTree<GfxMutator>` (the
//! interpolated state) that lands on the live tree through the
//! same `Applicable::apply_to` discipline the rest of this
//! crate respects. Animation never touches the model — that's
//! the boundary commit. Drag, edit, and animation all share the
//! "tree-only previews, model-only commits" invariant.

use serde::{Deserialize, Serialize};

use glam::Vec2;

/// Timing envelope attached to a `CustomMutation` to convert an
/// instant mutation into an animated transition. Serde-default
/// throughout so old maps without timing fields still load.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnimationTiming {
    /// How long the animation runs after `delay_ms` elapses.
    /// `0` means "instant" — the dispatcher should bypass the
    /// animation path entirely and apply the mutation directly.
    #[serde(default)]
    pub duration_ms: u32,
    /// Delay before the animation starts. `0` is no delay.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub delay_ms: u32,
    /// Curve mapping linear time progress (`t` in `[0, 1]`) to
    /// value progress. [`Easing::Linear`] is the default and
    /// matches the unaccelerated identity.
    #[serde(default)]
    pub easing: Easing,
    /// What happens when the animation reaches `t = 1`. `None`
    /// means "stop". See [`Followup`] for the chained shapes.
    ///
    /// **Not yet wired through the tick loop.** The type exists
    /// in-memory so the eventual dispatcher lands without a
    /// wire-format migration, but `#[serde(skip)]` keeps the
    /// field out of `.mindmap.json` until the tick loop reads
    /// it. Authoring `"then": "Loop"` today would be a silent
    /// half-feature (§4) — the map loads, the animation runs
    /// once, and nothing ever loops. The gate lifts when
    /// `tick_animations` gains the followup dispatch.
    #[serde(skip)]
    pub then: Option<Followup>,
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

/// Easing curve. Maps linear time progress `t in [0, 1]` to
/// value progress, also in `[0, 1]`. Endpoints are exact —
/// every variant returns `0.0` at `t = 0` and `1.0` at `t = 1`.
///
/// `Linear` is the default and matches the identity function.
/// The cubic-feeling `EaseInOut` follows the standard
/// `t < 0.5 ? 2t² : 1 - (-2t + 2)²/2` form, which is the
/// quadratic ease-in-out from easings.net — picked over
/// `cubic` because the foundation only needs one curve per
/// shape and the visual difference at typical durations
/// (200–600 ms) is imperceptible. Add new variants when the
/// trajectory needs them.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Easing {
    #[default]
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
}

impl Easing {
    /// Map linear time progress `t in [0, 1]` to value progress.
    /// `t` is clamped to `[0, 1]` before evaluation so a tick
    /// that overshoots due to frame-time jitter still produces a
    /// sane value.
    pub fn evaluate(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            Easing::EaseIn => t * t,
            Easing::EaseOut => {
                let inv = 1.0 - t;
                1.0 - inv * inv
            }
            Easing::EaseInOut => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    let inv = -2.0 * t + 2.0;
                    1.0 - inv * inv / 2.0
                }
            }
        }
    }
}

/// What happens after an animation completes its forward pass.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Followup {
    /// Hold the `to` state for `hold_ms`, then run the animation
    /// in reverse so the net persistent effect is zero.
    /// **Forces `MutationBehavior::Toggle` semantics** — a
    /// reversed animation has no persistent effect, so a
    /// `Persistent`-declared mutation paired with `Reverse` is
    /// inconsistent. The registry should warn at build time.
    Reverse { hold_ms: u32 },
    /// On completion, fire the `CustomMutation` with the named
    /// id (which may itself carry a `timing`, chaining further).
    Chain { id: String },
    /// Loop the animation forever — restart at `t = 0`
    /// immediately when `t = 1` is reached. Reset clears it.
    Loop,
}

/// Linear interpolation between `from` and `to` at progress `t`
/// (already eased). `t` should be in `[0, 1]`; values outside
/// the range extrapolate, which is the right behaviour for
/// `EaseOut` curves that reach `t = 1` early but get evaluated
/// once more at `t > 1` due to frame-time jitter — clamp at the
/// caller if extrapolation isn't desired.
#[inline]
pub fn lerp_f32(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}

/// Component-wise lerp on `Vec2`. Shared by position and bounds
/// interpolation in the per-tick mutator builder.
#[inline]
pub fn lerp_vec2(from: Vec2, to: Vec2, t: f32) -> Vec2 {
    Vec2::new(lerp_f32(from.x, to.x, t), lerp_f32(from.y, to.y, t))
}

/// Component-wise lerp on an RGBA float color tuple. Used for
/// `ColorFontRegions` and any other color field carried as
/// `[f32; 4]`. Alpha lerps the same as RGB — that matches the
/// semantics of "fade in / fade out" cleanly.
#[inline]
pub fn lerp_color(from: [f32; 4], to: [f32; 4], t: f32) -> [f32; 4] {
    [
        lerp_f32(from[0], to[0], t),
        lerp_f32(from[1], to[1], t),
        lerp_f32(from[2], to[2], t),
        lerp_f32(from[3], to[3], t),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every easing curve must return exactly `0.0` at `t = 0`
    /// and `1.0` at `t = 1`. Without this, animations would
    /// snap or undershoot at the endpoints — visible as a "pop"
    /// at the start or end of a transition.
    #[test]
    fn test_easing_endpoints_are_exact() {
        for easing in [
            Easing::Linear,
            Easing::EaseIn,
            Easing::EaseOut,
            Easing::EaseInOut,
        ] {
            assert_eq!(easing.evaluate(0.0), 0.0, "{easing:?} at 0");
            assert_eq!(easing.evaluate(1.0), 1.0, "{easing:?} at 1");
        }
    }

    /// `Easing::Linear` is the identity at the midpoint
    /// (`t = 0.5` → `0.5`). `EaseIn` and `EaseOut` are mirror
    /// images: `EaseIn(0.5) + EaseOut(0.5) = 1.0` (within FP
    /// tolerance). `EaseInOut` reaches the midpoint exactly at
    /// `t = 0.5` by construction (the join point of the two
    /// halves).
    #[test]
    fn test_easing_midpoint_relationships() {
        assert_eq!(Easing::Linear.evaluate(0.5), 0.5);
        let in_mid = Easing::EaseIn.evaluate(0.5);
        let out_mid = Easing::EaseOut.evaluate(0.5);
        assert!((in_mid + out_mid - 1.0).abs() < 1e-6);
        let inout_mid = Easing::EaseInOut.evaluate(0.5);
        assert!((inout_mid - 0.5).abs() < 1e-6);
    }

    /// `evaluate` clamps `t` to `[0, 1]` so frame-time jitter
    /// that pushes the elapsed fraction slightly over `1.0`
    /// still produces a value at the `to` state — not an
    /// extrapolated overshoot.
    #[test]
    fn test_easing_evaluate_clamps_overshoot() {
        for easing in [
            Easing::Linear,
            Easing::EaseIn,
            Easing::EaseOut,
            Easing::EaseInOut,
        ] {
            assert_eq!(easing.evaluate(1.5), 1.0);
            assert_eq!(easing.evaluate(-0.3), 0.0);
        }
    }

    /// Linear midpoint blend: at `t = 0.5` the lerp sits at the
    /// arithmetic mean of `from` and `to`. Pins the per-tick
    /// math the tween path produces.
    #[test]
    fn test_lerp_midpoint_is_arithmetic_mean() {
        assert_eq!(lerp_f32(0.0, 10.0, 0.5), 5.0);
        assert_eq!(lerp_f32(-4.0, 4.0, 0.5), 0.0);
        assert_eq!(lerp_vec2(Vec2::ZERO, Vec2::new(8.0, 6.0), 0.5), Vec2::new(4.0, 3.0));
        let mid_color = lerp_color([0.0, 0.0, 0.0, 0.0], [1.0, 0.5, 0.25, 1.0], 0.5);
        assert_eq!(mid_color, [0.5, 0.25, 0.125, 0.5]);
    }

    /// Lerp endpoints are exact: `t = 0` gives `from`,
    /// `t = 1` gives `to`. Combined with the easing endpoint
    /// invariant, this is what guarantees "no pop" at animation
    /// boundaries.
    #[test]
    fn test_lerp_endpoints_are_exact() {
        let from = 3.5;
        let to = -7.25;
        assert_eq!(lerp_f32(from, to, 0.0), from);
        assert_eq!(lerp_f32(from, to, 1.0), to);
        let from_v = Vec2::new(1.0, 2.0);
        let to_v = Vec2::new(-1.0, 4.0);
        assert_eq!(lerp_vec2(from_v, to_v, 0.0), from_v);
        assert_eq!(lerp_vec2(from_v, to_v, 1.0), to_v);
    }

    /// Round-trip through serde keeps the on-the-wire timing
    /// envelope intact. Pins the wire format so a
    /// `.mindmap.json` saved with timing fields loads back
    /// correctly.
    #[test]
    fn test_animation_timing_serde_round_trip() {
        let timing = AnimationTiming {
            duration_ms: 350,
            delay_ms: 100,
            easing: Easing::EaseInOut,
            // `then` is intentionally `None` — the wire format
            // skips `Followup` until the tick loop dispatches
            // it (see the field's doc-comment).
            then: None,
        };
        let json = serde_json::to_string(&timing).unwrap();
        let back: AnimationTiming = serde_json::from_str(&json).unwrap();
        assert_eq!(back, timing);
    }

    /// Default `AnimationTiming` round-trips through an empty
    /// JSON object — the serde defaults make every field
    /// optional, so an old map without a `timing` key
    /// deserializes into the same value as no envelope at all.
    #[test]
    fn test_animation_timing_default_serde_is_empty_object() {
        let default_timing = AnimationTiming::default();
        let from_empty: AnimationTiming = serde_json::from_str("{}").unwrap();
        assert_eq!(from_empty, default_timing);
        // And the default serializes without the optional fields
        // (zero delay_ms is skipped, `then` is always skipped).
        let json = serde_json::to_string(&default_timing).unwrap();
        assert!(!json.contains("delay_ms"));
        assert!(!json.contains("then"));
    }

    /// `Followup` variants are not reachable via the wire
    /// format. Authoring `"then": "Loop"` in a `.mindmap.json`
    /// today would be a silent half-feature (§4): the map would
    /// load, the animation would run once, and nothing would
    /// loop. The gate lifts when `tick_animations` gains the
    /// followup dispatch.
    #[test]
    fn test_followup_is_never_deserialized() {
        let with_followup = r#"{
            "duration_ms": 200,
            "then": {"Reverse": {"hold_ms": 50}}
        }"#;
        let parsed: AnimationTiming = serde_json::from_str(with_followup).unwrap();
        assert!(parsed.then.is_none());
        let loop_followup = r#"{"duration_ms": 200, "then": "Loop"}"#;
        let parsed: AnimationTiming = serde_json::from_str(loop_followup).unwrap();
        assert!(parsed.then.is_none());
    }
}
