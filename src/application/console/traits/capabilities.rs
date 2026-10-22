// SPDX-License-Identifier: MPL-2.0

//! Capability traits a console-target component can implement. The
//! `TargetView` enum implements each trait and dispatches on its
//! variant; commands reach for the trait method matching their key
//! and let `NotApplicable` fall out naturally for variants that
//! don't support that channel.

use super::color_value::ColorValue;
use super::outcome::{ClipboardContent, Outcome};

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

/// Target supports setting or clearing a label.
///
/// `None` clears the label; `Some(s)` sets it. For edges this is
/// `MindEdge.label`; nodes and portals do not implement it.
pub trait HasLabel {
    fn set_label(&mut self, s: Option<String>) -> Outcome;
}

/// Target accepts a single channel-less color from the standalone
/// wheel. Each variant decides which channel the color lands on
/// (nodes → `Bg`; edges → their single color field). Distinct from
/// the `Has*` axis traits, which answer "accept a color on channel
/// X?" — this answers "where does an unspecified-channel color go?".
pub trait AcceptsWheelColor {
    fn apply_wheel_color(&mut self, c: ColorValue) -> Outcome;
}

/// Target accepts a channel-less font-family choice — companion of
/// [`AcceptsWheelColor`]. Per-variant routing: Node writes every
/// `TextRun.font`; Edge / PortalLabel route through
/// `glyph_connection.font`; EdgeLabel and PortalText return
/// `NotApplicable` (they inherit the edge body's font today).
///
/// `family = Some(name)` pins; `None` clears. The console verb rejects
/// unknown families upstream; programmatic callers that skip that
/// check will land an unknown family that the renderer degrades to
/// monospace with a `warn!`.
pub trait AcceptsFontFamily {
    fn set_font_family(&mut self, family: Option<&str>) -> Outcome;
}

/// Target produces clipboard text on Ctrl+C. Pure read — the event
/// loop handles system I/O. Variants without copy return
/// `NotApplicable`; variants with copy but nothing to give return
/// `Empty`.
pub trait HandlesCopy {
    fn clipboard_copy(&self) -> ClipboardContent;
}

/// Target accepts clipboard text on Ctrl+V. `content` is the string
/// read from the system clipboard.
pub trait HandlesPaste {
    fn clipboard_paste(&mut self, content: &str) -> Outcome;
}

/// Target produces clipboard text on Ctrl+X and clears its source.
/// For components where "clearing" doesn't apply, cut may behave
/// identically to copy.
pub trait HandlesCut {
    fn clipboard_cut(&mut self) -> ClipboardContent;
}
