// SPDX-License-Identifier: MPL-2.0

//! `KeyBind` parser/matcher and the two `winit::Key` ↔ binding-string
//! shims (`normalize_key_name`, `key_to_name`). Pure data — no
//! platform-specific concerns.
//!
//! Mouse gestures share this same parser. A binding string like
//! `"DoubleClick"` or `"Shift+MiddleClick"` parses into the same
//! [`KeyBind`] struct as a keyboard binding, with the gesture's
//! canonical lowercase name in the `key` field. Mouse handlers
//! synthesize the same name via [`MouseGesture::key_name`] before
//! calling `ResolvedKeybinds::action_for_context`, so the lookup
//! table is universal across input devices.

use crate::application::platform::input::Key;

/// A parsed keybinding: a logical key name plus modifier flags. Key names
/// are normalized to lowercase during parsing so comparisons are
/// case-insensitive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyBind {
    pub key: String,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// User-driven mouse gestures that participate in the keybind lookup.
///
/// Each variant carries a `#[strum(serialize = "<lowercase>")]` —
/// the canonical lowercase token `KeyBind::parse` produces and
/// that mouse handlers feed into
/// `ResolvedKeybinds::action_for_context`. `IntoStaticStr` exposes
/// it via `<&'static str>::from(self)`, surfaced as
/// [`MouseGesture::key_name`]. The PascalCase emit form (the
/// shape the user types in `keybinds.json`) is the variant name
/// itself; [`MouseGesture::pascal_form`] returns it via the same
/// `EnumIter` walk both directions share.
///
/// `LeftClick` is still absent — CODE_CONVENTIONS §5 forbids
/// reserved-but-not-dispatched variants, and no Action today
/// binds bare `LeftClick`. `RightClick` and `RightDrag` are
/// reintroduced below alongside their dispatch sites in
/// `event_mouse_click.rs` / `event_cursor_moved.rs` —
/// fast-resize gesture per `SECTIONS_BORDERS_RESIZE_PLAN.md` §6.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum_macros::EnumIter, strum_macros::IntoStaticStr)]
pub enum MouseGesture {
    /// Left-button held down + cursor movement past the drag threshold,
    /// only when the press landed on empty canvas. Continuous: the bound
    /// action's body runs for the duration of the press. Dispatched
    /// from `event_cursor_moved` through `dispatch_action` for whatever
    /// Action the gesture resolves to — there is **no** `PanCanvas`
    /// special-case in the handler, and reintroducing one would silently
    /// un-bind every other Action a user maps here. The threshold
    /// frame's pan delta is gated on the dispatch having entered
    /// `DragState::Panning`.
    #[strum(serialize = "leftdrag")]
    LeftDrag,
    /// Two left-button presses within the double-click time + distance
    /// window with matching `ClickHit`. Dispatched.
    #[strum(serialize = "doubleclick")]
    DoubleClick,
    /// Single middle-button press. Dispatched.
    #[strum(serialize = "middleclick")]
    MiddleClick,
    /// One mouse-wheel tick upward (zoom-in by convention). Dispatched
    /// when the console isn't open.
    #[strum(serialize = "wheelup")]
    WheelUp,
    /// One mouse-wheel tick downward (zoom-out by convention). Same.
    #[strum(serialize = "wheeldown")]
    WheelDown,
    /// Single right-button press (no movement past threshold).
    /// Default-bound to nothing — exists so users / future GUIs
    /// can bind context menus or other actions to it without us
    /// needing to lift it into a half-feature later. The press
    /// path is wired (release fires the bound `Action`); without
    /// a binding, the right-button press still consumes the
    /// browser context menu on WASM and otherwise is a no-op.
    #[strum(serialize = "rightclick")]
    RightClick,
    /// Right-button held + cursor movement past the drag threshold.
    /// Continuous; dispatches the bound `Action` when the press's
    /// pending state crosses the threshold. Default-bound to
    /// `Action::FastResizeStart` (Ctrl+RightDrag), the corner-
    /// anchored fast-resize gesture from anywhere on a node body.
    #[strum(serialize = "rightdrag")]
    RightDrag,
    /// Touch: a single finger held in place for ≥ 350ms with no
    /// significant movement. Emitted by `TouchGestureRecognizer`
    /// (in `app/touch_gesture.rs` — private module, intra-doc
    /// link omitted) from `WindowEvent::Touch` events.
    /// Default-bound to `Action::EnterResizeMode` — long-press is
    /// the touch peer of the keyboard's `r` for entering Resize
    /// mode on the selected target. The "no significant movement"
    /// half is the cancel rule: any drag past
    /// `POINTER_DRAG_THRESHOLD_PX` (5 physical pixels — the same
    /// budget a mouse press gets before it promotes to a drag)
    /// before the timer fires aborts the long-press candidate,
    /// because that gesture is a one-finger pan instead.
    ///
    /// **The only touch gesture that reaches this table**, and
    /// deliberately so. Tap, one-finger pan and pinch-zoom are the
    /// other three the recognizer emits; each takes a carve-out
    /// `CODE_CONVENTIONS §3` already grants the mouse (selection
    /// bookkeeping, per-frame camera deltas) rather than resolving
    /// to an `Action`, so none of them can be dead on one target.
    /// `TwoFingerDrag` used to sit beside this variant, bound to
    /// `FastResizeStart`; two fingers on a canvas is a viewport
    /// manipulation on every touch platform, so it now drives the
    /// camera directly, and a binding that fired on the same event
    /// would have been a conflict rather than a choice.
    #[strum(serialize = "longpress")]
    LongPress,
}

impl MouseGesture {
    /// Canonical lowercase binding-string token for this gesture.
    /// The same token `KeyBind::parse` produces from `"DoubleClick"`,
    /// `"MiddleClick"`, etc. Mouse handlers feed this directly into
    /// `ResolvedKeybinds::action_for_context`. Backed by strum's
    /// `IntoStaticStr` derive — the per-variant `#[strum(serialize)]`
    /// attribute is the source of truth.
    pub fn key_name(self) -> &'static str {
        self.into()
    }

    /// PascalCase emit form for this gesture — the variant name
    /// itself, which is the shape the user types in `keybinds.json`.
    /// Used by `KeyBind::to_binding_string` so a parsed-then-
    /// emitted gesture round-trips to its canonical capitalisation
    /// rather than the lowercased internal form.
    pub fn pascal_form(self) -> &'static str {
        match self {
            MouseGesture::LeftDrag => "LeftDrag",
            MouseGesture::DoubleClick => "DoubleClick",
            MouseGesture::MiddleClick => "MiddleClick",
            MouseGesture::WheelUp => "WheelUp",
            MouseGesture::WheelDown => "WheelDown",
            MouseGesture::RightClick => "RightClick",
            MouseGesture::RightDrag => "RightDrag",
            MouseGesture::LongPress => "LongPress",
        }
    }
}

/// Look up the [`MouseGesture::pascal_form`] for a known
/// `lower`-case token, or `None` for keyboard names. Walks
/// `MouseGesture::iter()` so adding a new gesture variant
/// auto-extends the round-trip without touching this fn.
/// Test-gated with its only caller,
/// [`KeyBind::to_binding_string`].
#[cfg(test)]
fn gesture_emit_form(lower: &str) -> Option<&'static str> {
    use strum::IntoEnumIterator;
    MouseGesture::iter()
        .find(|g| g.key_name() == lower)
        .map(MouseGesture::pascal_form)
}

impl KeyBind {
    /// Parse a binding string like `"Ctrl+Z"`, `"Shift+Alt+Delete"`, or
    /// just `"Escape"`. Modifier order doesn't matter; whitespace is
    /// tolerated; key names are matched case-insensitively.
    pub fn parse(input: &str) -> Result<Self, String> {
        let mut ctrl = false;
        let mut shift = false;
        let mut alt = false;
        let mut key: Option<String> = None;

        for raw in input.split('+') {
            let part = raw.trim().to_ascii_lowercase();
            if part.is_empty() {
                continue;
            }
            match part.as_str() {
                "ctrl" | "control" | "cmd" | "command" | "meta" | "super" => ctrl = true,
                "shift" => shift = true,
                "alt" | "option" => alt = true,
                _ => {
                    if key.is_some() {
                        return Err(format!("keybind '{}' has multiple non-modifier keys", input));
                    }
                    key = Some(part);
                }
            }
        }

        match key {
            Some(key) => Ok(KeyBind {
                key,
                ctrl,
                shift,
                alt,
            }),
            None => Err(format!("keybind '{}' has no key", input)),
        }
    }

    /// Returns true if this binding matches the given logical key name and
    /// modifier state. The caller is expected to have normalized `key_name`
    /// to lowercase via `normalize_key_name`.
    pub fn matches(&self, key_name: &str, ctrl: bool, shift: bool, alt: bool) -> bool {
        self.key == key_name && self.ctrl == ctrl && self.shift == shift && self.alt == alt
    }

    /// Render the binding back to a `Ctrl+Shift+Alt+Key` string form.
    /// Inverse of `parse` up to modifier-order normalization — parsing
    /// this output must produce an equal `KeyBind`, which is locked in
    /// by `test_keybind_string_round_trip_through_parse`.
    ///
    /// Test-gated: nothing writes a keybind file back out yet, so the
    /// round trip has only the test end. The day a "save my
    /// keybinds" path lands, this is the emitter it needs.
    #[cfg(test)]
    pub fn to_binding_string(&self) -> String {
        let mut parts: Vec<&str> = Vec::with_capacity(4);
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.alt {
            parts.push("Alt");
        }
        // Recognized mouse gestures emit in PascalCase so a parsed-
        // then-emitted binding string round-trips to its canonical
        // form. Other keys emit lowercase as stored.
        let key_display: String = gesture_emit_form(&self.key)
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.key.clone());
        let joined = parts.join("+");
        if joined.is_empty() {
            key_display
        } else {
            format!("{}+{}", joined, key_display)
        }
    }
}

/// Normalize a winit logical-key representation to the same lowercase form
/// `KeyBind::parse` uses. The caller passes the string form it extracted
/// from its key event (character or named-key debug name) and this function
/// lowercases and trims it.
pub fn normalize_key_name(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

/// Convert a winit `Key` into the lowercase string form that
/// `KeyBind::parse` produces, so keybind comparison is symmetric.
/// Pairs with `normalize_key_name`; the two together produce comparable
/// strings from either the stored-config side or the live-event side.
pub fn key_to_name(key: &Key) -> Option<String> {
    match key {
        Key::Character(c) => Some(normalize_key_name(c.as_ref())),
        Key::Named(named) => Some(normalize_key_name(&format!("{:?}", named))),
        _ => None,
    }
}
