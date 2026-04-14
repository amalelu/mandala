//! Capability traits a console-target component can implement. The
//! `TargetView` enum implements all six and dispatches on its
//! variant; commands reach for the trait method matching their key
//! and let `NotApplicable` fall out naturally for variants that
//! don't support that channel.

use super::color_value::ColorValue;
use super::outcome::Outcome;

/// Target supports setting its background / fill color.
///
/// For nodes this is the frame fill. For portals it's the glyph
/// color itself (portals have no separate fill). For edges it is
/// unsupported — edges don't have a fill concept.
pub trait HasBgColor {
    fn set_bg_color(&mut self, c: ColorValue) -> Outcome;
}

/// Target supports setting its foreground / text color.
///
/// For nodes this rewrites `style.text_color` and any `TextRun`
/// whose color matched the pre-edit default (per-run overrides are
/// preserved). For edges this is the label / line color. For
/// portals it is unsupported.
pub trait HasTextColor {
    fn set_text_color(&mut self, c: ColorValue) -> Outcome;
}

/// Target supports setting its border / outline color.
///
/// For nodes this is `style.frame_color`. For edges this is the
/// connection body (glyph) color. For portals it is unsupported.
pub trait HasBorderColor {
    fn set_border_color(&mut self, c: ColorValue) -> Outcome;
}

/// Target supports setting its font size in points.
pub trait HasFontSize {
    fn set_font_size(&mut self, pt: f32) -> Outcome;
}

/// Target supports setting or clearing a label.
///
/// `None` clears the label; `Some(s)` sets it. For edges this is
/// `MindEdge.label`; nodes and portals do not implement it.
pub trait HasLabel {
    fn set_label(&mut self, s: Option<String>) -> Outcome;
}

/// Target supports receiving a color from the **standalone color
/// wheel** (the `color picker on` persistent palette). The wheel
/// doesn't pick an axis — it pushes one color at the selection and
/// asks each component type to decide which channel that color
/// belongs on. Nodes take it on `Bg`; edges take it on their single
/// color field (routed through `set_border_color`, which is the same
/// sink as `set_text_color` for edges). Portals haven't been ported
/// to Baumhard yet — they return `NotApplicable` today and will
/// switch to their fill channel once the port lands.
///
/// Separate trait from `HasBgColor` / `HasTextColor` / `HasBorderColor`
/// by design: the `Has*` axis traits answer "can you accept a color
/// on channel X?"; `AcceptsWheelColor` answers the narrower question
/// "if someone hands you one color without specifying a channel,
/// where does it go?". The default-channel choice belongs with the
/// component implementation, not with every caller.
pub trait AcceptsWheelColor {
    fn apply_wheel_color(&mut self, c: ColorValue) -> Outcome;
}
