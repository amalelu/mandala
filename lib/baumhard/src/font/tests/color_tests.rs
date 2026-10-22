// SPDX-License-Identifier: MPL-2.0

//! Tests for [`crate::font::color`] — the byte / float ↔
//! [`cosmic_text::Color`] wall. The hex side of the same wall is
//! covered by `font/tests/hex_tests.rs`; what these pin is the
//! byte-side entry point, which is what a compile-time color
//! constant crosses on its way to a shaped buffer.

use crate::font::color::{cosmic_color_from_color, cosmic_color_from_rgba};
use crate::util::color::Color;

#[test]
fn test_cosmic_color_from_color_carries_every_channel() {
    do_cosmic_color_from_color_carries_every_channel();
}

/// [`cosmic_color_from_color`] hands cosmic-text a baumhard
/// [`Color`]'s bytes with no float round trip, and does it in
/// `const` position — `CONST_BRIDGED` is the half of the claim a
/// runtime assertion cannot make, since a regression there is a
/// compile error rather than a failure.
///
/// The four channels are mutually distinct on purpose: the input
/// that makes this fail is a bridge that transposes two of them,
/// which a `[0, 229, 255, 255]`-shaped fixture would hide across
/// the blue / alpha pair.
pub fn do_cosmic_color_from_color_carries_every_channel() {
    const SOURCE: Color = Color {
        rgba: [5, 99, 143, 200],
    };
    const CONST_BRIDGED: cosmic_text::Color = cosmic_color_from_color(SOURCE);
    assert_eq!(CONST_BRIDGED.r(), 5);
    assert_eq!(CONST_BRIDGED.g(), 99);
    assert_eq!(CONST_BRIDGED.b(), 143);
    assert_eq!(CONST_BRIDGED.a(), 200);
}

#[test]
fn test_cosmic_color_from_color_agrees_with_the_float_bridge() {
    do_cosmic_color_from_color_agrees_with_the_float_bridge();
}

/// The byte-side bridge is a shortcut past the float side, not a
/// second answer: for every alpha a color constant can carry, the
/// two doors land on the same [`cosmic_text::Color`]. Sweeping all
/// 256 is what makes this a claim about the pair rather than about
/// one lucky value.
///
/// The `assert_ne!` at the end is the control: the equality above
/// is only evidence because the two sides *can* disagree. An
/// alpha-dropping bridge — the plausible mistake, since the three
/// visible channels would still look right — differs at every alpha
/// but 255, and the control pins that the comparison sees it.
pub fn do_cosmic_color_from_color_agrees_with_the_float_bridge() {
    for alpha in 0..=u8::MAX {
        let source = Color {
            rgba: [5, 99, 143, alpha],
        };
        assert_eq!(
            cosmic_color_from_color(source),
            cosmic_color_from_rgba(source.to_float()),
            "the two bridges disagree at alpha {alpha}"
        );
    }

    assert_ne!(
        cosmic_color_from_color(Color {
            rgba: [5, 99, 143, 200]
        }),
        cosmic_color_from_rgba([5.0 / 255.0, 99.0 / 255.0, 143.0 / 255.0, 1.0]),
        "an alpha-dropping bridge must be distinguishable, or the sweep proves nothing"
    );
}
