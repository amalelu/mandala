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

/// Target supports producing a text representation when the user
/// copies (Ctrl+C). The trait method is a pure data transformation —
/// it reads the component's state and returns what should go on the
/// clipboard. The caller (event loop) handles system clipboard I/O.
///
/// Components that don't support copy return
/// `ClipboardContent::NotApplicable`. Components that support it but
/// have nothing to give right now return `ClipboardContent::Empty`.
pub trait HandlesCopy {
    fn clipboard_copy(&self) -> ClipboardContent;
}

/// Target supports accepting text from the clipboard when the user
/// pastes (Ctrl+V). Returns `Outcome` — the same result type the
/// existing capability traits use — so the dispatcher can aggregate
/// paste results the same way it aggregates color or font results.
///
/// The `content` parameter is the string read from the system
/// clipboard by the event loop before the trait call. The trait
/// method decides how to integrate it (parse a hex color, set node
/// text, etc.) and reports the result.
pub trait HandlesPaste {
    fn clipboard_paste(&mut self, content: &str) -> Outcome;
}

/// Target supports producing a text representation *and* clearing or
/// resetting its source state when the user cuts (Ctrl+X). For
/// components where "clearing" doesn't apply (e.g. a color picker
/// always shows a color), cut may behave identically to copy.
pub trait HandlesCut {
    fn clipboard_cut(&mut self) -> ClipboardContent;
}
