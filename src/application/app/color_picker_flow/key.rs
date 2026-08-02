// SPDX-License-Identifier: MPL-2.0

//! Picker keystroke plumbing, split along the CODE_CONVENTIONS §3
//! line:
//!
//! - [`picker_op_for`] / [`PickerOp`] — the single source of truth
//!   for "the color picker owns this `Action`". Commit, cancel and
//!   the six nudges are user-named effects, so they belong in the
//!   dispatch funnel exactly like `TextEditCommit` /
//!   `LabelEditCommit`, and the funnel is what a
//!   `MacroStep::Action` reaches. The keyboard pre-filter
//!   (`event_keyboard.rs`), the click router ([`super::click`]) and
//!   the `dispatch_action` arm (`dispatch::native`) all read this
//!   one function, so an Action can never be routed toward the
//!   funnel without an arm there to receive it.
//! - [`apply_picker_nudge`] / [`nudged_hsv`] — the nudge primitive
//!   the funnel arm runs. Renderer-free, so the HSV math is
//!   directly testable.
//! - [`handle_color_picker_clipboard_key`] — what legitimately
//!   stays modal-local (CODE_CONVENTIONS §3 "modal steals"): the
//!   picker's Copy / Paste / Cut over the hex value, plus the
//!   fall-through verdict for every other key.

use crate::application::clipboard;
use crate::application::color_picker::ColorPickerState;
use crate::application::console::traits::{ClipboardContent, HandlesCopy, HandlesCut, HandlesPaste, Outcome};
use crate::application::document::MindMapDocument;
use crate::application::keybinds::Action;

use super::super::throttled_interaction::ColorPickerHoverInteraction;
use super::commit::apply_picker_preview;

/// A picker-modal effect named by an `Action`. Produced by
/// [`picker_op_for`]; consumed by the `dispatch_action` arm, which
/// owns the renderer-driving half (`cancel_color_picker`,
/// `commit_color_picker`, `commit_color_picker_to_selection`,
/// [`apply_picker_nudge`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::application::app) enum PickerOp {
    /// Close without writing the previewed color back
    /// (`Action::PickerCancel`). Contextual mode only — Standalone
    /// has no "original" to cancel back to and only closes via
    /// `color picker off` / `Action::CloseColorPicker`.
    Cancel,
    /// Write the current HSV out (`Action::PickerCommit`).
    /// Contextual commits to the bound handle and closes;
    /// Standalone fans out across the selection and stays open.
    Commit,
    /// Step one HSV channel (`Action::PickerNudge*`).
    Nudge(PickerNudge),
}

/// Which HSV channel a nudge steps, and in which direction. Hue
/// steps by 15°, saturation and value by 0.1 — see [`nudged_hsv`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::application::app) enum PickerNudge {
    HueDown,
    HueUp,
    SatDown,
    SatUp,
    ValDown,
    ValUp,
}

/// Hue step, in degrees, for one [`PickerNudge::HueUp`] /
/// [`PickerNudge::HueDown`].
const HUE_STEP_DEG: f32 = 15.0;

/// Saturation / value step for one nudge. Ten steps spans the
/// full `0.0..=1.0` range.
const SV_STEP: f32 = 0.1;

/// The picker operation `action` names, or `None` when the picker
/// doesn't own the Action.
///
/// **This is the funnel's guard.** `dispatch_action`'s picker arm
/// matches on `picker_op_for(&action).is_some()` rather than on a
/// hand-listed set of variants, so the arm's coverage is
/// definitionally this function's coverage — the keyboard
/// pre-filter and the click router read it too, and none of the
/// three can drift from the others. A new `Picker*` Action is
/// reachable from macros, keys and clicks the moment it gets an
/// arm here.
pub(in crate::application::app) fn picker_op_for(action: &Action) -> Option<PickerOp> {
    Some(match action {
        Action::PickerCancel => PickerOp::Cancel,
        Action::PickerCommit => PickerOp::Commit,
        Action::PickerNudgeHueDown => PickerOp::Nudge(PickerNudge::HueDown),
        Action::PickerNudgeHueUp => PickerOp::Nudge(PickerNudge::HueUp),
        Action::PickerNudgeSatDown => PickerOp::Nudge(PickerNudge::SatDown),
        Action::PickerNudgeSatUp => PickerOp::Nudge(PickerNudge::SatUp),
        Action::PickerNudgeValDown => PickerOp::Nudge(PickerNudge::ValDown),
        Action::PickerNudgeValUp => PickerOp::Nudge(PickerNudge::ValUp),
        _ => return None,
    })
}

/// Why the funnel arm declines a [`PickerOp`] without running it.
/// Pure classification of `(op, is_open, is_standalone,
/// has_document)`, so the decline branches are testable without a
/// renderer — they read three booleans.
///
/// Getting these wrong is quiet: collapse
/// [`Self::StandaloneCancel`] and Esc starts being swallowed by the
/// persistent palette; collapse [`Self::PickerClosed`] and macros
/// get a false "it ran".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::application::app) enum PickerDecline {
    /// The picker isn't open, so there is nothing to commit,
    /// cancel or nudge. Unreachable from the keyboard and mouse
    /// paths (both gate on `is_open()`) but **reachable from a
    /// macro / IPC step**, which is exactly what routing the picker
    /// Actions through the funnel opened up. Declining keeps
    /// `dispatch_macro`'s `any_ran` false so the keyboard handler's
    /// custom-mutation tier still gets its turn on that key, and
    /// keeps the scripting surface honest about what ran.
    PickerClosed,
    /// Cancel in Standalone mode. The persistent palette has no
    /// "original" to cancel back to and only closes via `color
    /// picker off` / `Action::CloseColorPicker`, so Esc must stay
    /// available to the Document context. The keyboard pre-filter
    /// reads the decline and falls through — byte-for-byte the
    /// pre-funnel behavior, which returned `false` from the
    /// picker's key handler here.
    StandaloneCancel,
    /// No document loaded — nothing to preview or commit onto.
    NoDocument,
}

/// Decide whether the funnel arm can run `op`, or why it can't.
/// `None` means "run it".
///
/// `PickerClosed` is checked first so the reported reason is the
/// right one for any input, including the `(is_open: false,
/// is_standalone: true)` pair. That pair cannot occur at the sole
/// call site — `ColorPickerState::is_standalone()` is
/// `matches!(self, Open { mode: Standalone, .. })`, so it implies
/// open — which means swapping the two checks would be
/// behaviorally identical in production today. The order is a
/// contract of this pure function, not a property of any state the
/// program can reach; it is pinned so a future caller that derives
/// the two flags separately still gets a truthful reason.
///
/// Taking `&ColorPickerState` instead of two dependent booleans
/// would make the impossible pair unrepresentable and retire the
/// question. Left as-is here to keep the fix scoped.
pub(in crate::application::app) fn picker_decline_reason(
    op: PickerOp,
    is_open: bool,
    is_standalone: bool,
    has_document: bool,
) -> Option<PickerDecline> {
    if !is_open {
        return Some(PickerDecline::PickerClosed);
    }
    if matches!(op, PickerOp::Cancel) && is_standalone {
        return Some(PickerDecline::StandaloneCancel);
    }
    if !has_document {
        return Some(PickerDecline::NoDocument);
    }
    None
}

/// Step one channel of an `(hue_deg, sat, val)` triple. Hue wraps
/// through `rem_euclid` so 0° − 15° lands on 345° rather than a
/// negative angle the wheel can't render; saturation and value
/// clamp at the ends. Pure, so the wrap / clamp behavior is
/// testable without a picker or a document.
pub(in crate::application::app) fn nudged_hsv(nudge: PickerNudge, hsv: (f32, f32, f32)) -> (f32, f32, f32) {
    let (mut hue_deg, mut sat, mut val) = hsv;
    match nudge {
        PickerNudge::HueDown => hue_deg = (hue_deg - HUE_STEP_DEG).rem_euclid(360.0),
        PickerNudge::HueUp => hue_deg = (hue_deg + HUE_STEP_DEG).rem_euclid(360.0),
        PickerNudge::SatDown => sat = (sat - SV_STEP).clamp(0.0, 1.0),
        PickerNudge::SatUp => sat = (sat + SV_STEP).clamp(0.0, 1.0),
        PickerNudge::ValDown => val = (val - SV_STEP).clamp(0.0, 1.0),
        PickerNudge::ValUp => val = (val + SV_STEP).clamp(0.0, 1.0),
    }
    (hue_deg, sat, val)
}

/// Apply a nudge to the open picker: step the selected HSV, drop
/// any mouse `hover_preview` (the keyboard just took over from the
/// pointer), and restamp the document preview so the targeted edge
/// repaints on the next drain. Returns `true` when the picker was
/// open and the nudge landed.
///
/// Renderer-free by construction — [`apply_picker_preview`] marks
/// `picker_hover.dirty` / `.canvas_dirty` and the per-frame drain
/// does the rebuilding.
pub(in crate::application::app) fn apply_picker_nudge(
    nudge: PickerNudge,
    state: &mut ColorPickerState,
    doc: &mut MindMapDocument,
    picker_hover: &mut ColorPickerHoverInteraction,
) -> bool {
    let ColorPickerState::Open {
        hue_deg,
        sat,
        val,
        hover_preview,
        ..
    } = state
    else {
        return false;
    };
    let (nh, ns, nv) = nudged_hsv(nudge, (*hue_deg, *sat, *val));
    *hue_deg = nh;
    *sat = ns;
    *val = nv;
    *hover_preview = None;
    apply_picker_preview(state, doc, picker_hover);
    true
}

/// Run the picker's modal-local half of a keystroke, given the
/// `Action` the caller already resolved through
/// [`crate::application::keybinds::InputContext::ColorPicker`].
/// Returns `true` if the key was consumed, `false` to let it fall
/// through to the event loop's normal dispatch.
///
/// **Scope is deliberately narrow.** Only the clipboard verbs live
/// here — Copy / Paste / Cut carry a hex payload the picker owns
/// and no `Action` body could express without reaching into
/// `ColorPickerState`. Everything the picker does that a user would
/// *name* (commit, cancel, nudge) is pre-filtered into
/// `dispatch_action` by the caller in `event_keyboard.rs`; see
/// [`picker_op_for`].
pub(in crate::application::app) fn handle_color_picker_clipboard_key(
    action: Option<&Action>,
    state: &mut ColorPickerState,
    doc: &mut MindMapDocument,
    picker_hover: &mut ColorPickerHoverInteraction,
) -> bool {
    match action {
        Some(Action::Copy) => {
            if let ClipboardContent::Text(hex) = state.clipboard_copy() {
                clipboard::write_clipboard(&hex);
            }
            true
        }
        Some(Action::Paste) => {
            if let Some(text) = clipboard::read_clipboard() {
                if let Outcome::Applied = state.clipboard_paste(&text) {
                    apply_picker_preview(state, doc, picker_hover);
                }
            }
            true
        }
        Some(Action::Cut) => {
            if let ClipboardContent::Text(hex) = state.clipboard_cut() {
                clipboard::write_clipboard(&hex);
            }
            true
        }
        Some(a) if picker_op_for(a).is_some() => {
            // Unreachable through `event_keyboard.rs`, which
            // pre-filters every funnel-owned Action away before
            // calling here. Reaching this arm means a caller
            // skipped the pre-filter, which would silently drop
            // the effect — the exact failure this issue closes.
            // Fail safe (CODE_CONVENTIONS §9): report unconsumed
            // and log loudly rather than run a second copy of the
            // arm body.
            log::error!(
                "handle_color_picker_clipboard_key reached with funnel-owned Action {:?}; \
                 caller skipped the dispatch pre-filter",
                a
            );
            false
        }
        Some(_) => {
            // A Document-level action fell through — let the event
            // loop handle it via normal dispatch.
            false
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    //! Nudge math + the funnel-ownership table. The ownership test
    //! walks every `Action` variant, so a new ColorPicker-context
    //! Action can't ship without an entry in [`picker_op_for`] —
    //! which is the same predicate the `dispatch_action` arm guards
    //! on and the same one the keyboard / click routers consult.

    use super::*;
    use crate::application::keybinds::{ActionKind, InputContext};
    use std::collections::HashSet;
    use strum::IntoEnumIterator;

    /// The eight picker-owned Actions, listed value-explicitly.
    /// `ActionKind` is payload-free and can't be turned back into
    /// an `Action`, so the walk below compares kinds instead.
    fn picker_actions() -> Vec<Action> {
        vec![
            Action::PickerCancel,
            Action::PickerCommit,
            Action::PickerNudgeHueDown,
            Action::PickerNudgeHueUp,
            Action::PickerNudgeSatDown,
            Action::PickerNudgeSatUp,
            Action::PickerNudgeValDown,
            Action::PickerNudgeValUp,
        ]
    }

    /// Every Action classified into `InputContext::ColorPicker`
    /// must be claimed by `picker_op_for`. Because the dispatch
    /// arm's guard *is* `picker_op_for(..).is_some()`, this pins
    /// the property the issue is about: an Action the picker
    /// resolves a keystroke to cannot fail to reach a funnel arm.
    /// `ActionKind::iter()` supplies the structural completeness —
    /// a ninth **ColorPicker-context** Action fails this test until
    /// it is wired. Note the scope: `OpenColorPicker` /
    /// `CloseColorPicker` are `Picker`-named but sit in
    /// `context = Document`, so the walk does not (and should not)
    /// see them — they are console-verb mirrors with their own
    /// funnel arms, not modal keystrokes.
    #[test]
    fn test_picker_op_for_claims_every_color_picker_context_action() {
        let listed: HashSet<ActionKind> = picker_actions().iter().map(ActionKind::from).collect();
        let by_context: HashSet<ActionKind> = ActionKind::iter()
            .filter(|k| k.context() == InputContext::ColorPicker)
            .collect();
        assert_eq!(
            listed, by_context,
            "the ColorPicker-context Action set changed; wire the new variant \
             into picker_op_for (and this list) or the funnel will drop it"
        );
        for a in picker_actions() {
            assert!(
                picker_op_for(&a).is_some(),
                "{:?} resolves in InputContext::ColorPicker but picker_op_for \
                 doesn't claim it — the dispatch arm would drop it",
                a
            );
        }
    }

    /// The mapping itself, value-explicitly. A silent re-point of
    /// (say) `PickerNudgeSatUp` onto the hue channel would still
    /// satisfy the coverage test above.
    #[test]
    fn test_picker_op_for_maps_each_action_to_its_effect() {
        assert_eq!(picker_op_for(&Action::PickerCancel), Some(PickerOp::Cancel));
        assert_eq!(picker_op_for(&Action::PickerCommit), Some(PickerOp::Commit));
        assert_eq!(
            picker_op_for(&Action::PickerNudgeHueDown),
            Some(PickerOp::Nudge(PickerNudge::HueDown))
        );
        assert_eq!(
            picker_op_for(&Action::PickerNudgeHueUp),
            Some(PickerOp::Nudge(PickerNudge::HueUp))
        );
        assert_eq!(
            picker_op_for(&Action::PickerNudgeSatDown),
            Some(PickerOp::Nudge(PickerNudge::SatDown))
        );
        assert_eq!(
            picker_op_for(&Action::PickerNudgeSatUp),
            Some(PickerOp::Nudge(PickerNudge::SatUp))
        );
        assert_eq!(
            picker_op_for(&Action::PickerNudgeValDown),
            Some(PickerOp::Nudge(PickerNudge::ValDown))
        );
        assert_eq!(
            picker_op_for(&Action::PickerNudgeValUp),
            Some(PickerOp::Nudge(PickerNudge::ValUp))
        );
    }

    /// The picker must not claim the clipboard verbs — those stay
    /// modal-local (they carry a hex payload no Action body can
    /// express) and are the reason
    /// `handle_color_picker_clipboard_key` still exists.
    #[test]
    fn test_picker_op_for_leaves_clipboard_actions_to_the_modal() {
        assert_eq!(picker_op_for(&Action::Copy), None);
        assert_eq!(picker_op_for(&Action::Paste), None);
        assert_eq!(picker_op_for(&Action::Cut), None);
        assert_eq!(picker_op_for(&Action::ExitMode), None);
    }

    /// A picker open in Contextual mode bound to edge 0 of the
    /// testament map, seeded at a known HSV. Enough state for the
    /// nudge path — layout / ink offsets are only read by the
    /// renderer, which this path never touches.
    fn open_contextual_picker(hsv: (f32, f32, f32)) -> ColorPickerState {
        use crate::application::color_picker::{PickerHandle, PickerMode, CROSSHAIR_CENTER_CELL};
        ColorPickerState::Open {
            mode: PickerMode::Contextual {
                handle: PickerHandle::Edge(0),
                seed_var_ref: None,
                seed_hsv: hsv,
            },
            hue_deg: hsv.0,
            sat: hsv.1,
            val: hsv.2,
            last_cursor_pos: None,
            max_cell_advance: 1.0,
            max_ring_advance: 1.0,
            measurement_font_size: 16.0,
            arm_top_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
            arm_bottom_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
            arm_left_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
            arm_right_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
            preview_ink_offset: (0.0, 0.0),
            layout: None,
            center_override: None,
            size_scale: 1.0,
            gesture: None,
            hovered_hit: None,
            hover_preview: None,
            pending_error_flash: false,
            last_dynamic_apply: None,
        }
    }

    /// The nudge actually lands: HSV moves on the picker, the
    /// document's transient preview is restamped with the new hex,
    /// and the throttled drain is flagged. This is the effect half
    /// of `PickerOp::Nudge` — `dispatch_action`'s picker arm calls
    /// exactly this function.
    #[test]
    fn test_apply_picker_nudge_moves_hsv_and_restamps_the_document_preview() {
        let mut doc = crate::application::document::tests_common::load_test_doc();
        assert!(
            !doc.mindmap.edges.is_empty(),
            "testament map is expected to carry edges for the Edge(0) handle"
        );
        let mut state = open_contextual_picker((10.0, 0.5, 0.5));
        let mut hover = ColorPickerHoverInteraction::default();
        doc.color_picker_preview = None;

        assert!(apply_picker_nudge(
            PickerNudge::HueUp,
            &mut state,
            &mut doc,
            &mut hover
        ));

        let ColorPickerState::Open { hue_deg, .. } = state else {
            panic!("nudge must not close the picker");
        };
        assert_eq!(hue_deg, 25.0);
        let preview = doc
            .color_picker_preview
            .expect("nudge restamps the document's transient preview");
        assert_eq!(preview.color, baumhard::util::color::hsv_to_hex(25.0, 0.5, 0.5));
        use crate::application::app::throttled_interaction::ThrottledInteraction;
        assert!(hover.has_pending(), "the per-frame drain must be flagged");
        assert!(
            hover.canvas_needs_rebuild(),
            "the canvas must repaint the previewed edge"
        );
    }

    /// A keyboard nudge takes over from the pointer: the stale
    /// mouse `hover_preview` is dropped so the wheel shows the
    /// value the keyboard just chose.
    #[test]
    fn test_apply_picker_nudge_clears_a_stale_hover_preview() {
        let mut doc = crate::application::document::tests_common::load_test_doc();
        let mut state = open_contextual_picker((10.0, 0.5, 0.5));
        if let ColorPickerState::Open { hover_preview, .. } = &mut state {
            *hover_preview = Some((200.0, 0.1, 0.9));
        }
        let mut hover = ColorPickerHoverInteraction::default();

        assert!(apply_picker_nudge(
            PickerNudge::SatUp,
            &mut state,
            &mut doc,
            &mut hover
        ));

        let ColorPickerState::Open {
            hover_preview, sat, ..
        } = state
        else {
            panic!("nudge must not close the picker");
        };
        assert_eq!(hover_preview, None);
        assert_eq!(sat, 0.6);
    }

    /// A nudge against a closed picker is a no-op, not a panic —
    /// a macro can fire `PickerNudgeHueUp` at any time.
    #[test]
    fn test_apply_picker_nudge_on_a_closed_picker_is_a_noop() {
        let mut doc = crate::application::document::tests_common::load_test_doc();
        let mut state = ColorPickerState::Closed;
        let mut hover = ColorPickerHoverInteraction::default();
        assert!(!apply_picker_nudge(
            PickerNudge::HueUp,
            &mut state,
            &mut doc,
            &mut hover
        ));
        use crate::application::app::throttled_interaction::ThrottledInteraction;
        assert!(!hover.has_pending());
    }

    // ── Picker-arm decline branches ─────────────────────────────
    //
    // The three cases where the picker arm must report `Unhandled`.
    // They read only `(op, is_open, is_standalone, has_document)`,
    // so §T8's no-wgpu rule doesn't reach them — and each one is a
    // silent failure if it regresses.

    /// An open contextual picker with a document runs every op.
    #[test]
    fn test_picker_decline_reason_allows_every_op_when_open_and_loaded() {
        for op in [
            PickerOp::Cancel,
            PickerOp::Commit,
            PickerOp::Nudge(PickerNudge::HueUp),
        ] {
            assert_eq!(
                picker_decline_reason(op, true, false, true),
                None,
                "{:?} should run against an open contextual picker",
                op
            );
        }
    }

    /// **Closed picker.** Unreachable from the keyboard and mouse
    /// paths (both gate on `is_open()`), but reachable from a macro
    /// step — which is precisely what this PR wired up. Reporting
    /// `Handled` here would set `dispatch_macro`'s `any_ran`, and
    /// the keyboard handler skips the custom-mutation tier when a
    /// macro "ran", so a `CustomMutation` bound to the same key
    /// would silently stop firing.
    #[test]
    fn test_picker_decline_reason_declines_every_op_when_the_picker_is_closed() {
        for op in [
            PickerOp::Cancel,
            PickerOp::Commit,
            PickerOp::Nudge(PickerNudge::SatDown),
        ] {
            assert_eq!(
                picker_decline_reason(op, false, false, true),
                Some(PickerDecline::PickerClosed),
                "{:?} must not claim to have run against a closed picker",
                op
            );
        }
    }

    /// **Standalone cancel.** The persistent palette has no
    /// "original" to cancel back to, so Esc must fall through to
    /// the Document context. Collapse this branch and Esc starts
    /// being swallowed while the palette is up.
    #[test]
    fn test_picker_decline_reason_declines_cancel_in_standalone_mode() {
        assert_eq!(
            picker_decline_reason(PickerOp::Cancel, true, true, true),
            Some(PickerDecline::StandaloneCancel),
        );
    }

    /// Standalone declines *only* cancel — commit fans the wheel
    /// color across the selection and nudges still move the HSV.
    #[test]
    fn test_picker_decline_reason_allows_commit_and_nudge_in_standalone_mode() {
        assert_eq!(picker_decline_reason(PickerOp::Commit, true, true, true), None);
        assert_eq!(
            picker_decline_reason(PickerOp::Nudge(PickerNudge::ValUp), true, true, true),
            None
        );
    }

    /// **No document.** Nothing to preview or commit onto.
    #[test]
    fn test_picker_decline_reason_declines_every_op_without_a_document() {
        for op in [
            PickerOp::Cancel,
            PickerOp::Commit,
            PickerOp::Nudge(PickerNudge::HueDown),
        ] {
            assert_eq!(
                picker_decline_reason(op, true, false, false),
                Some(PickerDecline::NoDocument),
                "{:?} needs a document",
                op
            );
        }
    }

    /// A closed picker is neither standalone nor contextual, so
    /// the closed check must win over the standalone-cancel check
    /// — otherwise the log line names the wrong reason and the
    /// two branches can't be told apart in a bug report.
    #[test]
    fn test_picker_decline_reason_reports_closed_before_standalone_cancel() {
        assert_eq!(
            picker_decline_reason(PickerOp::Cancel, false, true, true),
            Some(PickerDecline::PickerClosed),
        );
    }

    #[test]
    fn test_nudged_hsv_steps_hue_by_fifteen_degrees() {
        assert_eq!(nudged_hsv(PickerNudge::HueUp, (10.0, 0.5, 0.5)).0, 25.0);
        assert_eq!(nudged_hsv(PickerNudge::HueDown, (30.0, 0.5, 0.5)).0, 15.0);
    }

    /// Hue is an angle: stepping below 0° or past 360° wraps
    /// rather than clamping, so the wheel keeps turning in one
    /// direction.
    #[test]
    fn test_nudged_hsv_wraps_hue_at_both_ends() {
        assert_eq!(nudged_hsv(PickerNudge::HueDown, (0.0, 0.5, 0.5)).0, 345.0);
        assert_eq!(nudged_hsv(PickerNudge::HueUp, (355.0, 0.5, 0.5)).0, 10.0);
    }

    /// Saturation and value are bounded, so they clamp instead of
    /// wrapping — a nudge past the end is a no-op, not a jump to
    /// the opposite extreme.
    #[test]
    fn test_nudged_hsv_clamps_saturation_and_value() {
        assert_eq!(nudged_hsv(PickerNudge::SatDown, (0.0, 0.05, 0.5)).1, 0.0);
        assert_eq!(nudged_hsv(PickerNudge::SatUp, (0.0, 0.95, 0.5)).1, 1.0);
        assert_eq!(nudged_hsv(PickerNudge::ValDown, (0.0, 0.5, 0.05)).2, 0.0);
        assert_eq!(nudged_hsv(PickerNudge::ValUp, (0.0, 0.5, 0.95)).2, 1.0);
    }

    /// Each nudge touches exactly one channel.
    #[test]
    fn test_nudged_hsv_leaves_the_other_channels_alone() {
        let base = (100.0_f32, 0.4_f32, 0.6_f32);
        let hue = nudged_hsv(PickerNudge::HueUp, base);
        assert_eq!((hue.1, hue.2), (base.1, base.2));
        let sat = nudged_hsv(PickerNudge::SatUp, base);
        assert_eq!((sat.0, sat.2), (base.0, base.2));
        let val = nudged_hsv(PickerNudge::ValUp, base);
        assert_eq!((val.0, val.1), (base.0, base.1));
    }
}
