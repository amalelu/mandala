// SPDX-License-Identifier: MPL-2.0

//! The `(size, min, max)` font triple shared by the three edge
//! font channels — the edge body's `glyph_connection`, a line-mode
//! edge's `label_config`, and a portal endpoint's text state.
//!
//! All three want the same thing: apply whichever of the three
//! values the caller supplied, atomically, with `min` / `max`
//! landing *before* the clamped `size` so `font size=14 max=10`
//! resolves to `size=10, max=10` rather than the wrong
//! `size=14, max=10` a one-at-a-time dispatch would produce. All
//! three must also reject an inverted `(min, max)` pair before
//! touching anything: `f32::clamp` panics on `min > max`, and an
//! inverted pair that reached the model would panic the next
//! renderer frame through
//! [`GlyphConnectionConfig::effective_font_size_pt`](baumhard::mindmap::model::GlyphConnectionConfig::effective_font_size_pt)
//! — forbidden in an interactive path (`CODE_CONVENTIONS.md` §9).
//!
//! Only the *storage* differs: the body channel keeps three plain
//! `f32`s (it is the bottom of the cascade, so it always has
//! concrete values), while the label and portal-text channels keep
//! `Option<f32>`s that fall back to the body's clamps. [`FontTriple`]
//! is the common currency — `Option<f32>` throughout, with the
//! body channel's `None` simply never occurring — and
//! [`resolve_font_triple`] is the one implementation of the
//! ordering, the inversion guard, and the clamp.

use baumhard::util::geometry::{is_positive_finite, option_almost_equal};

/// One channel's authored `(size, min, max)` in points. `None`
/// means "this channel authors nothing here and inherits".
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct FontTriple {
    pub size: Option<f32>,
    pub min: Option<f32>,
    pub max: Option<f32>,
}

impl FontTriple {
    /// `true` when any of the three values differs from `other`'s
    /// counterpart, comparing present values with
    /// [`option_almost_equal`] (1e-5).
    ///
    /// The tolerance is what makes the setters idempotent against
    /// the f32 round-trip: a `size` that re-clamps to a value
    /// 1e-7 away from the stored one is the same size, and
    /// treating it as a change would mint an undo entry for a
    /// mutation the user cannot see.
    pub(super) fn differs_from(&self, other: &FontTriple) -> bool {
        !option_almost_equal(self.size, other.size)
            || !option_almost_equal(self.min, other.min)
            || !option_almost_equal(self.max, other.max)
    }
}

/// Apply a `(size, min, max)` request to `current` and return the
/// triple that should land, or `None` when the request must be
/// rejected outright.
///
/// - Each requested value is ignored unless it is
///   [`is_positive_finite`]; a rejected value leaves that field
///   as it was, exactly as if the caller had passed `None`.
/// - `fallback` is the `(min, max)` this channel inherits when it
///   authors no clamp of its own — the body channel's resolved
///   clamps for the label / portal-text channels, and the
///   channel's own current values for the body itself.
/// - The resolved `(min, max)` after the request is checked for
///   inversion **before** anything is written. An inverted pair
///   returns `None` and the caller must not mutate.
/// - `size` is then clamped into the *post-request* bounds, which
///   is what makes the triple atomic.
///
/// Returns `Some(triple)` on success — the caller compares it
/// against `current` with [`FontTriple::differs_from`] to decide
/// whether the edit is a no-op. O(1).
pub(super) fn resolve_font_triple(
    current: FontTriple,
    fallback: (f32, f32),
    size: Option<f32>,
    min: Option<f32>,
    max: Option<f32>,
) -> Option<FontTriple> {
    let requested_min = min.filter(|v| is_positive_finite(*v));
    let requested_max = max.filter(|v| is_positive_finite(*v));
    let requested_size = size.filter(|v| is_positive_finite(*v));

    let new_min = requested_min.or(current.min);
    let new_max = requested_max.or(current.max);
    // Resolve against the inherited clamps for the inversion
    // check: a label that authors only `min` still has to be
    // ordered against the body's `max`, or the renderer clamps
    // with an inverted pair.
    let effective_min = new_min.unwrap_or(fallback.0);
    let effective_max = new_max.unwrap_or(fallback.1);
    if effective_min > effective_max {
        return None;
    }

    let new_size = match requested_size {
        // Bounds resolved and known-ordered above, so `clamp` is
        // panic-safe here.
        Some(s) => Some(s.clamp(effective_min, effective_max)),
        None => current.size,
    };

    Some(FontTriple {
        size: new_size,
        min: new_min,
        max: new_max,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ordering contract: `max` lands before the clamped
    /// `size`, so `size=14 max=10` resolves to `size=10, max=10`.
    /// The whole reason the triple is one atomic call.
    #[test]
    fn test_font_triple_clamps_size_against_the_new_max() {
        let current = FontTriple {
            size: Some(12.0),
            min: Some(8.0),
            max: Some(128.0),
        };
        let out = resolve_font_triple(current, (8.0, 128.0), Some(14.0), None, Some(10.0))
            .expect("ordered pair accepted");
        assert_eq!(out.size, Some(10.0));
        assert_eq!(out.max, Some(10.0));
        assert_eq!(out.min, Some(8.0), "untouched side survives");
    }

    /// Same contract on the lower side: a `min` raised above the
    /// requested `size` pulls the size up with it.
    #[test]
    fn test_font_triple_clamps_size_against_the_new_min() {
        let current = FontTriple {
            size: Some(12.0),
            min: Some(8.0),
            max: Some(128.0),
        };
        let out = resolve_font_triple(current, (8.0, 128.0), Some(6.0), Some(20.0), None)
            .expect("ordered pair accepted");
        assert_eq!(out.size, Some(20.0));
        assert_eq!(out.min, Some(20.0));
    }

    /// An inverted resolved pair is rejected before any value is
    /// produced — the guard that keeps `f32::clamp` from panicking
    /// here and in the renderer.
    #[test]
    fn test_font_triple_rejects_inverted_pair() {
        let current = FontTriple {
            size: Some(12.0),
            min: Some(8.0),
            max: Some(128.0),
        };
        assert!(resolve_font_triple(current, (8.0, 128.0), None, Some(50.0), Some(10.0)).is_none());
    }

    /// Inversion is checked against the *inherited* clamps too:
    /// a channel that authors only `min` must still be ordered
    /// against the fallback `max` it inherits.
    #[test]
    fn test_font_triple_rejects_inversion_against_inherited_max() {
        let current = FontTriple::default();
        assert!(
            resolve_font_triple(current, (8.0, 20.0), None, Some(50.0), None).is_none(),
            "min=50 against an inherited max=20 must be rejected"
        );
    }

    /// Non-finite and non-positive values are ignored rather than
    /// rejected: the field keeps its prior value and the rest of
    /// the triple still applies.
    #[test]
    fn test_font_triple_ignores_non_positive_finite_values() {
        let current = FontTriple {
            size: Some(12.0),
            min: Some(8.0),
            max: Some(128.0),
        };
        let out = resolve_font_triple(
            current,
            (8.0, 128.0),
            Some(f32::NAN),
            Some(-1.0),
            Some(f32::INFINITY),
        )
        .expect("all three ignored, nothing inverts");
        assert_eq!(out, current, "every rejected value leaves its field alone");
    }

    /// An all-`None` request resolves to the current triple, so
    /// the caller's `differs_from` sees a no-op.
    #[test]
    fn test_font_triple_all_none_is_a_no_op() {
        let current = FontTriple {
            size: Some(12.0),
            min: Some(8.0),
            max: Some(128.0),
        };
        let out = resolve_font_triple(current, (8.0, 128.0), None, None, None).expect("accepted");
        assert!(!out.differs_from(&current));
    }

    /// `differs_from` uses the almost-equal tolerance, so a
    /// sub-1e-5 drift is not a change. Guards against an undo
    /// entry the user cannot see.
    #[test]
    fn test_font_triple_differs_from_uses_tolerance() {
        let a = FontTriple {
            size: Some(12.0),
            min: None,
            max: None,
        };
        let b = FontTriple {
            size: Some(12.000001),
            min: None,
            max: None,
        };
        assert!(!a.differs_from(&b));
        let c = FontTriple {
            size: Some(12.5),
            min: None,
            max: None,
        };
        assert!(a.differs_from(&c));
    }

    /// A `Some` / `None` mismatch is always a change — clearing
    /// or authoring a clamp is a real edit even when the
    /// inherited value happens to equal it.
    #[test]
    fn test_font_triple_differs_from_on_presence_mismatch() {
        let authored = FontTriple {
            size: None,
            min: Some(8.0),
            max: None,
        };
        let inherited = FontTriple::default();
        assert!(authored.differs_from(&inherited));
    }
}
