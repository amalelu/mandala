// SPDX-License-Identifier: MPL-2.0

//! Cross-platform action dispatcher.
//!
//! Handles every Compatible-classified `Action` arm whose body
//! has been factored into a `cross_dispatch::apply_*` helper, plus
//! the cross-platform slice of the four mixed-branch Actions
//! [`MIXED_BRANCH_ACTIONS`] names (`Action::ExitMode`'s mode reset +
//! `last_click` clear, `Action::EditSelection*`-Single open, and
//! `Action::DoubleClickActivate` — everything except the edge-label
//! branch's single-line editor open). Three of the four classify
//! `NativeOnly`; `ExitMode` is `Compatible`, because its native
//! leftover is a step rather than a branch. Returns `Handled` when
//! the body ran; `Unhandled` for variants this dispatcher doesn't
//! own — the caller's fall-through (native only) runs the
//! platform-specific arm.
//!
//! Both targets reach the same body:
//! - **Native**: `dispatch::dispatch_action` splits its
//!   `InputHandlerContext` into `(InputContextCore, NativeContextExt)`
//!   via `split_borrow` and calls into here first; on `Unhandled`,
//!   falls through to the native-only arm match in `dispatch.rs`.
//! - **WASM** (post-Track C wire-up in C3): `run_wasm`'s keyboard
//!   handler and `WasmMacroDispatchTarget::dispatch_action` call
//!   this directly with `&mut InputContextCore` built from
//!   `WasmInputState::input_context_core`.
//!
//! Track C from `WASM_CONVERGENCE.md`. Sibling to
//! `dispatch_macro_core` (Track B precedent). The arm coverage
//! matches what `dispatch_compatible_action_wasm` already wires
//! today — that function gets deleted in C3 once the WASM caller
//! moves over.

use baumhard::util::geometry::is_positive_finite;

use crate::application::document::OptionEdit;
use crate::application::keybinds::{Action, WasmCompatibility, MIXED_BRANCH_ACTIONS};

use super::super::input_context_core::InputContextCore;
use super::super::InteractionMode;
use super::cross_dispatch::DispatchOutcome;

/// Parse the `runs_mode` payload on `Action::SetSectionText`.
/// `"clear"` → `Some(true)`; `"preserve"` / `""` →
/// `Some(false)`; any other string → `None` (caller logs and
/// bails). The empty string accepts as preserve so a default-
/// payload macro doesn't have to spell it out.
pub(super) fn parse_action_runs_mode(s: &str) -> Option<bool> {
    match s {
        "clear" => Some(true),
        "preserve" | "" => Some(false),
        _ => None,
    }
}

/// Parse the `at` / `at_grapheme` payload on
/// `Action::AddSection` / `Action::SplitSection`. Empty string
/// → `Some(None)`; parseable → `Some(Some(n))`; unparseable →
/// `None` (caller logs + bails). Outer Option is "did the parse
/// succeed?"; inner Option is the value semantics.
///
/// What `Some(None)` *means* is the arm's business, not the
/// parser's: `AddSection` reads it as "append", while
/// `SplitSection` rejects it — the console verb requires `at=`
/// because splitting at end-of-text silently creates an empty
/// suffix section, and the macro path holds the same line.
pub(super) fn parse_action_optional_usize(s: &str) -> Option<Option<usize>> {
    if s.is_empty() {
        return Some(None);
    }
    s.parse::<usize>().ok().map(Some)
}

/// Run `f` against a `RebuildContext` built from `core`, IF the
/// document is loaded. Skips silently otherwise. Captures the
/// `if let Some(doc) = core.document.as_deref_mut() { let mut rc =
/// rebuild_ctx!(core, doc); f(&mut rc); }` pattern that 20+
/// document-mutating arms in [`dispatch_compatible`] previously
/// repeated inline.
///
/// CODE_CONVENTIONS §5: "If a function is needed in two or more
/// places, the answer is never to copy it." This helper closes
/// that gap inside the dispatcher.
pub(in crate::application::app) fn with_doc_rebuild<F>(core: &mut InputContextCore<'_>, f: F)
where
    F: FnOnce(&mut super::cross_dispatch::RebuildContext<'_>),
{
    if let Some(doc) = core.document.as_deref_mut() {
        let mut rc = super::cross_dispatch::rebuild_ctx!(core, doc);
        f(&mut rc);
    }
}

/// Lift `Unhandled → Handled` for the mixed-branch arms whose
/// cross-platform slice IS the totality of what WASM can do
/// (`ExitMode`, `EditSelection*`, and `DoubleClickActivate`'s
/// edge-label branch). On native, `Unhandled` flows to the
/// dispatcher's existing match for the target-picker
/// (Reparent / Connect) overlay clear or EdgeLabel/Portal editor
/// open. WASM has no such fall-through —
/// `WasmMacroDispatchTarget::dispatch_action` calls
/// [`dispatch_compatible`] directly, so the macro loop's
/// `any_ran` flag would stop bumping for these arms without this
/// lift.
///
/// `double_click_residual` is what makes the `DoubleClickActivate`
/// entry safe to list. That arm produces **two different**
/// `Unhandled`s, and the outcome alone cannot tell them apart —
/// neither can "was there a hit", which is the *wrong* discriminator
/// and was the one this function used before:
///
/// - **No residual at all** (`None`): the arm never reached the apply
///   half because there was no `DispatchHit`. A soft-skip — it logged
///   and touched no document state, which
///   `MacroDispatchTarget::dispatch_action`'s contract defines as
///   "did not run". Lifting it would report `any_ran=true` for a step
///   that did nothing, and a User-tier macro whose step list is
///   `[Action(DoubleClickActivate)]` would stop falling through to
///   the custom-mutation tier.
/// - **`OpenEdgeLabelEditor`**: a hit *was* present, but the shared
///   half may legitimately have done nothing — it skips the commit
///   and the rebuild when the label is already the current selection
///   ([`edge_label_selection_is_current`](super::cross_dispatch::edge_label_selection_is_current)),
///   which is the normal case because the double-click's first click
///   already committed it. "There was a hit" therefore does **not**
///   mean "work ran"; treating it that way reproduced the very
///   misreport this lift exists to prevent, one condition deeper.
///
/// So the verdict is read from
/// [`double_click_outcome`](super::cross_dispatch::double_click_outcome)
/// — the single place the residual → outcome mapping is written, and
/// the same one the dispatcher arm returns — rather than inferred
/// here. Today a macro carries no hit, so the residual is always
/// `None` and the entry always declines; that is the correct answer
/// by construction, not an accident of reachability, and it stays
/// correct if a future macro step gains a hit payload.
///
/// Membership is [`MIXED_BRANCH_ACTIONS`], shared with the keybind
/// test that pins each member's classification, so the two cannot
/// name different sets.
///
/// Public-in-app so the WASM macro target's impl can call it,
/// and so unit tests under `#[cfg(test)]` can pin the contract
/// without spinning up a `WasmInputState`.
pub(in crate::application::app) fn lift_mixed_branch_for_wasm_macro(
    action: &Action,
    double_click_residual: Option<super::cross_dispatch::DoubleClickResidual>,
    outcome: DispatchOutcome,
) -> DispatchOutcome {
    if !matches!(outcome, DispatchOutcome::Unhandled) {
        return outcome;
    }
    if !MIXED_BRANCH_ACTIONS.iter().any(|(a, _)| a == action) {
        return outcome;
    }
    let work_ran = match action {
        Action::ExitMode | Action::EditSelection | Action::EditSelectionClean => true,
        Action::DoubleClickActivate => matches!(
            super::cross_dispatch::double_click_outcome(double_click_residual),
            DispatchOutcome::Handled
        ),
        other => {
            // Reachable only by adding a member to
            // `MIXED_BRANCH_ACTIONS` without giving it a verdict —
            // the guard above already excluded everything else. A
            // silent `false` there is the misreport this function
            // exists to prevent, so it fails any test build.
            debug_assert!(
                false,
                "mixed-branch Action {other:?} has no lift verdict — add an arm",
            );
            false
        }
    };
    if work_ran {
        DispatchOutcome::Handled
    } else {
        outcome
    }
}

/// Cross-platform action dispatcher. Returns `Handled` when the
/// arm body ran; `Unhandled` for variants this dispatcher doesn't
/// own (NativeOnly Actions without a cross-platform slice, or
/// mixed-branch Actions whose cross-platform slice didn't apply
/// — caller's fall-through runs the native arm).
///
/// `hit` carries the pointer-event-only payload (what the press
/// landed on, where it landed in canvas space). Keyboard, macro and
/// touch callers pass `None`; the mouse handlers on **both** targets
/// populate it, which is what lets `DoubleClickActivate` resolve
/// through the keybind table in the browser instead of being
/// hardcoded there.
pub(in crate::application::app) fn dispatch_compatible(
    action: &Action,
    core: &mut InputContextCore<'_>,
    hit: Option<&super::cross_dispatch::DispatchHit>,
) -> DispatchOutcome {
    // Mixed-branch arms — handle the cross-platform slice here.
    // Caller's fall-through (native only) handles the residual
    // NativeOnly branches (EdgeLabel / Portal editors,
    // InteractionMode clearing). Returning `Unhandled` from a mixed
    // arm means "the cross-platform slice ran (or wasn't applicable);
    // native may have more to do".
    match action {
        Action::ExitMode => {
            // Cross-platform slice runs first:
            // 0. Cancel any active border preview. The plan's
            //    intended Esc shape was "if a preview is up, Esc
            //    cancels it; otherwise Esc falls through to the
            //    rest of ExitMode". Since `cancel_border_preview`
            //    can't share Esc with `exit_mode` through the
            //    keybind resolver (first match wins, no chaining),
            //    we collapse the chain into ExitMode's body — Esc
            //    on a preview cancels the preview AND skips the
            //    mode-clear, so a user previewing while in Resize
            //    doesn't lose their resize mode just because they
            //    typed Esc to drop a preview. C7 fix.
            if let Some(doc) = core.document.as_deref_mut() {
                if doc.cancel_border_preview() {
                    let mut rc = super::cross_dispatch::RebuildContext {
                        document: doc,
                        mindmap_tree: core.mindmap_tree,
                        app_scene: core.app_scene,
                        renderer: core.renderer,
                        scene_cache: core.scene_cache,
                        interaction_mode: core.interaction_mode,
                    };
                    rc.rebuild_after_geometry_change();
                    return DispatchOutcome::Handled;
                }
            }
            // 1. Clear `last_click` so a post-Esc click isn't paired
            //    with a pre-Esc one (was already in the pre-Batch-2
            //    `ExitMode` cross-platform body).
            // 2. Clear `interaction_mode` to `Default` and rebuild
            //    when the active mode is `Resize { .. }` or
            //    `NodeEdit { .. }`. Both modes need a way out: Resize
            //    used to be NativeOnly (review-fix in Batch 2), and
            //    NodeEdit was added by Batch 3 with no Esc-handler at
            //    all — multi-section users got trapped (no outside-
            //    click on WASM, no NodeEdit-context Esc binding). On
            //    NodeEdit exit also lift selection back to
            //    `Single(node_id)` so per-node verbs stay usable
            //    after exit; a stale Section selection would point
            //    at a dimming-cleared node and surprise the user.
            //
            // The native fallthrough still handles the
            // Reparent / Connect target-picker overlay clear (which
            // depends on `hovered_node` from `NativeContextExt`).
            *core.last_click = None;
            let exit_target_node = match &*core.interaction_mode {
                InteractionMode::Resize { .. } => Some(None),
                InteractionMode::NodeEdit { node_id } => Some(Some(node_id.clone())),
                _ => None,
            };
            if let Some(node_id_to_lift) = exit_target_node {
                *core.interaction_mode = InteractionMode::Default;
                if let Some(doc) = core.document.as_deref_mut() {
                    if let Some(node_id) = node_id_to_lift {
                        // Lift Section / SectionRange / Single
                        // selections that point at the exited
                        // NodeEdit node back to whole-node Single.
                        // Other selection states (Multi, edge,
                        // portal) stay untouched — the user steered
                        // away from the active node deliberately.
                        if doc.selection.primary_node_id().is_some_and(|id| id == node_id) {
                            doc.selection = crate::application::document::SelectionState::Single(node_id);
                        }
                    }
                    let mut rc = super::cross_dispatch::RebuildContext {
                        document: doc,
                        mindmap_tree: core.mindmap_tree,
                        app_scene: core.app_scene,
                        renderer: core.renderer,
                        scene_cache: core.scene_cache,
                        interaction_mode: core.interaction_mode,
                    };
                    rc.rebuild_after_selection_change();
                }
                return DispatchOutcome::Handled;
            }
            return DispatchOutcome::Unhandled;
        }
        Action::EditSelection | Action::EditSelectionClean => {
            // Cross-platform slice: Single / Section / SectionRange
            // selections route through `apply_enter_node_edit` —
            // flips `InteractionMode::NodeEdit { node_id }` and (for
            // single-section nodes) opens the inline editor in the
            // same call. Returns true iff a node-scoped selection
            // was found. EdgeLabel + Portal branches are native-only;
            // on false return, caller's fall-through tries them.
            let clean = matches!(action, Action::EditSelectionClean);
            let opened = if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::RebuildContext {
                    document: doc,
                    mindmap_tree: core.mindmap_tree,
                    app_scene: core.app_scene,
                    renderer: core.renderer,
                    scene_cache: core.scene_cache,
                    interaction_mode: core.interaction_mode,
                };
                super::cross_dispatch::apply_enter_node_edit(clean, &mut rc, core.text_edit_state)
            } else {
                false
            };
            return if opened {
                DispatchOutcome::Handled
            } else {
                DispatchOutcome::Unhandled
            };
        }
        Action::DoubleClickActivate => {
            // Routes by what the press hit. The mouse handlers on
            // both targets populate `hit` before dispatching; without
            // it there is no target and the gesture silently no-ops
            // (it was bound but fired from a non-pointer source, e.g.
            // a macro carrying no hit context). That is a soft-skip:
            // the step logs and changes no document state, which is
            // exactly the case `MacroDispatchTarget::dispatch_action`'s
            // contract (`macro_core.rs`) pins as "did not run".
            //
            // `None` residual is that fact, and
            // `double_click_outcome` — not this arm — turns it into
            // the reported outcome. The mapping lives there because
            // `lift_mixed_branch_for_wasm_macro` has to reach the
            // same verdict, and because it is pure data the suite can
            // pin without a `Renderer` (`TEST_CONVENTIONS §T8`/§T9).
            let residual = match hit {
                Some(h) => Some(super::cross_dispatch::apply_double_click_activate(h, core)),
                None => {
                    log::debug!("DoubleClickActivate: no DispatchHit; skipping");
                    None
                }
            };
            return super::cross_dispatch::double_click_outcome(residual);
        }
        _ => {}
    }

    // Compatible-classified Actions only beyond this point.
    // NativeOnly returns `Unhandled` so the caller's fall-through
    // runs them (on native) or they're silently skipped (on WASM,
    // where the keystroke wasn't bound anyway in well-formed
    // configs).
    if action.wasm_compatibility() == WasmCompatibility::NativeOnly {
        return DispatchOutcome::Unhandled;
    }

    match action {
        // ── Modal text-edit commit / cancel ───────────────────
        // Cross-platform — `text_edit::close_text_edit` is reachable
        // on both targets. WASM keyboard handler + click-outside
        // path AND native modal handler all funnel through these
        // arms; the close helper owns the `mem::replace(.., Closed)`
        // mode-exit + tree-revert / scene-rebuild lifecycle. Inlined
        // (not via `with_doc_rebuild`) because the helper signature
        // takes `text_edit_state` separately from the rebuild
        // bundle, and the closure shape can't split-borrow `core`.
        Action::TextEditCancel | Action::TextEditCommit => {
            let commit = matches!(action, Action::TextEditCommit);
            if let Some(doc) = core.document.as_deref_mut() {
                super::super::text_edit::close_text_edit(
                    commit,
                    doc,
                    core.interaction_mode,
                    core.text_edit_state,
                    core.mindmap_tree,
                    core.app_scene,
                    core.renderer,
                    core.scene_cache,
                );
            }
        }
        // ── Document-lifecycle ─────────────────────────────────
        Action::Undo => with_doc_rebuild(core, super::cross_dispatch::apply_undo),
        Action::DeleteSelection => with_doc_rebuild(core, super::cross_dispatch::apply_delete_selection),
        Action::OrphanSelection => with_doc_rebuild(core, super::cross_dispatch::apply_orphan_selection),
        Action::CreateOrphanNode => {
            let canvas_pos = core
                .renderer
                .screen_to_canvas(core.cursor_pos.0 as f32, core.cursor_pos.1 as f32);
            with_doc_rebuild(core, |rc| {
                super::cross_dispatch::apply_create_orphan_node(canvas_pos, rc)
            });
        }
        // ── Camera / zoom ──────────────────────────────────────
        Action::ZoomIn => super::cross_dispatch::apply_zoom_step(
            super::cross_dispatch::ZoomDir::In,
            *core.cursor_pos,
            core.renderer,
        ),
        Action::ZoomOut => super::cross_dispatch::apply_zoom_step(
            super::cross_dispatch::ZoomDir::Out,
            *core.cursor_pos,
            core.renderer,
        ),
        Action::ZoomReset => super::cross_dispatch::apply_zoom_reset(core.renderer),
        Action::ZoomFit => super::cross_dispatch::apply_zoom_fit(core.mindmap_tree, core.renderer),
        Action::PanCameraNorth => {
            super::cross_dispatch::apply_pan_camera(super::cross_dispatch::PanDir::North, core.renderer)
        }
        Action::PanCameraSouth => {
            super::cross_dispatch::apply_pan_camera(super::cross_dispatch::PanDir::South, core.renderer)
        }
        Action::PanCameraEast => {
            super::cross_dispatch::apply_pan_camera(super::cross_dispatch::PanDir::East, core.renderer)
        }
        Action::PanCameraWest => {
            super::cross_dispatch::apply_pan_camera(super::cross_dispatch::PanDir::West, core.renderer)
        }
        Action::CenterOnSelection => {
            // Read-only on document; doesn't fit `with_doc_rebuild`'s
            // `&mut RebuildContext` shape.
            if let Some(doc) = core.document.as_deref() {
                super::cross_dispatch::apply_center_on_selection(doc, core.renderer);
            }
        }
        Action::JumpToRoot => with_doc_rebuild(core, super::cross_dispatch::apply_jump_to_root),
        // ── FPS overlay ────────────────────────────────────────
        Action::ToggleFps => super::cross_dispatch::apply_toggle_fps(core.renderer),
        Action::ToggleFpsDebug => super::cross_dispatch::apply_toggle_fps_debug(core.renderer),
        // ── Selection navigation ───────────────────────────────
        Action::SelectAll
        | Action::DeselectAll
        | Action::InvertSelection
        | Action::SelectParent
        | Action::SelectChild
        | Action::SelectNextSibling
        | Action::SelectPrevSibling => with_doc_rebuild(core, |rc| match action {
            Action::SelectAll => super::cross_dispatch::apply_select_all(rc),
            Action::DeselectAll => super::cross_dispatch::apply_deselect_all(rc),
            Action::InvertSelection => super::cross_dispatch::apply_invert_selection(rc),
            Action::SelectParent => super::cross_dispatch::apply_select_parent(rc),
            Action::SelectChild => super::cross_dispatch::apply_select_child(rc),
            Action::SelectNextSibling => super::cross_dispatch::apply_select_sibling(true, rc),
            Action::SelectPrevSibling => super::cross_dispatch::apply_select_sibling(false, rc),
            // Safe fallback per CODE_CONVENTIONS §9 (interactive
            // paths fail-safe). Reachable only if a future Action
            // variant joins the outer cluster without an inner arm.
            _ => log::error!(
                "dispatch_compatible: selection fan-out missed inner-match: {:?}",
                action,
            ),
        }),
        // ── Parametric mutators ────────────────────────────────
        Action::SetEdgeAnchor { from, to } => with_doc_rebuild(core, |rc| {
            super::cross_dispatch::apply_set_edge_anchor(from, to, rc)
        }),
        Action::SetEdgeBodyGlyph(preset) => with_doc_rebuild(core, |rc| {
            super::cross_dispatch::apply_set_edge_body_glyph(preset, rc)
        }),
        Action::SetBorderField { field, value } => with_doc_rebuild(core, |rc| {
            super::cross_dispatch::apply_set_border_field(field, value, rc)
        }),
        Action::CycleBorderPreset => with_doc_rebuild(core, super::cross_dispatch::apply_cycle_border_preset),
        Action::ToggleBorderVisible => {
            with_doc_rebuild(core, super::cross_dispatch::apply_toggle_border_visible)
        }
        Action::SetBorderPreview {
            target_kind,
            field,
            value,
        } => with_doc_rebuild(core, |rc| {
            super::cross_dispatch::apply_set_border_preview(*target_kind, field, value, rc)
        }),
        Action::CommitBorderPreview => {
            with_doc_rebuild(core, super::cross_dispatch::apply_commit_border_preview)
        }
        Action::CancelBorderPreview => {
            with_doc_rebuild(core, super::cross_dispatch::apply_cancel_border_preview)
        }
        Action::SetEdgeCap { from, to } => {
            with_doc_rebuild(core, |rc| super::cross_dispatch::apply_set_edge_cap(from, to, rc))
        }
        Action::SetColor { axis, value } => with_doc_rebuild(core, |rc| {
            super::cross_dispatch::apply_set_color_axis(*axis, value, rc)
        }),
        Action::SetEdgeType(value) => {
            with_doc_rebuild(core, |rc| super::cross_dispatch::apply_set_edge_type(value, rc))
        }
        Action::SetEdgeDisplayMode(value) => with_doc_rebuild(core, |rc| {
            super::cross_dispatch::apply_set_edge_display_mode(value, rc)
        }),
        Action::ResetEdge(kind) => {
            with_doc_rebuild(core, |rc| super::cross_dispatch::apply_reset_edge(kind, rc))
        }
        Action::SetFontFamily(family) => with_doc_rebuild(core, |rc| {
            super::cross_dispatch::apply_set_font_family(family, rc)
        }),
        Action::SetFont { slot, value } => {
            let parsed = match value.parse::<f32>() {
                Ok(v) if is_positive_finite(v) => v,
                _ => {
                    log::warn!("SetFont{{slot={:?}}}: invalid '{}'", slot, value);
                    return DispatchOutcome::Handled;
                }
            };
            with_doc_rebuild(core, |rc| {
                super::cross_dispatch::apply_set_font_kv(*slot, parsed, rc)
            });
        }
        Action::SetEdgeLabelText(text) => with_doc_rebuild(core, |rc| {
            super::cross_dispatch::apply_set_edge_label_text(text, rc)
        }),
        Action::SetEdgeLabelPosition(pos) => with_doc_rebuild(core, |rc| {
            super::cross_dispatch::apply_set_edge_label_position(pos, rc)
        }),
        Action::SetSpacing(i) => with_doc_rebuild(core, |rc| super::cross_dispatch::apply_set_spacing(i, rc)),
        Action::SetZoom { bound, value } => {
            let parsed = match crate::application::console::commands::zoom::parse_zoom_payload(value) {
                Some(e) => e,
                None => {
                    log::warn!("SetZoom{{bound={:?}}}: invalid '{}'", bound, value);
                    return DispatchOutcome::Handled;
                }
            };
            let (min, max) = match bound {
                crate::application::keybinds::ZoomBound::Min => (parsed, OptionEdit::Keep),
                crate::application::keybinds::ZoomBound::Max => (OptionEdit::Keep, parsed),
            };
            with_doc_rebuild(core, |rc| {
                super::cross_dispatch::apply_set_zoom_window(min, max, rc)
            });
        }
        Action::ClearZoom => with_doc_rebuild(core, super::cross_dispatch::apply_clear_zoom),
        Action::SetSectionOffsetDelta { dx, dy } => {
            let (Some(dx_v), Some(dy_v)) = (dx.parse::<f64>().ok(), dy.parse::<f64>().ok()) else {
                log::warn!("SetSectionOffsetDelta: invalid dx='{}' or dy='{}'", dx, dy);
                return DispatchOutcome::Handled;
            };
            with_doc_rebuild(core, |rc| {
                super::cross_dispatch::apply_set_section_offset_delta(dx_v, dy_v, rc)
            });
        }
        Action::SetSectionSizeAbs { w, h } => {
            let (Some(w_v), Some(h_v)) = (w.parse::<f64>().ok(), h.parse::<f64>().ok()) else {
                log::warn!("SetSectionSizeAbs: invalid w='{}' or h='{}'", w, h);
                return DispatchOutcome::Handled;
            };
            with_doc_rebuild(core, |rc| {
                super::cross_dispatch::apply_set_section_size(
                    Some(baumhard::mindmap::model::Size {
                        width: w_v,
                        height: h_v,
                    }),
                    rc,
                )
            });
        }
        Action::SetSectionSizeFillParent => {
            with_doc_rebuild(core, |rc| super::cross_dispatch::apply_set_section_size(None, rc))
        }
        Action::SetSectionOffsetAbs { x, y } => {
            let (Some(x_v), Some(y_v)) = (x.parse::<f64>().ok(), y.parse::<f64>().ok()) else {
                log::warn!("SetSectionOffsetAbs: invalid x='{}' or y='{}'", x, y);
                return DispatchOutcome::Handled;
            };
            with_doc_rebuild(core, |rc| {
                super::cross_dispatch::apply_set_section_offset_abs(x_v, y_v, rc)
            });
        }
        Action::SetSectionText { text, runs_mode } => {
            let Some(clear_runs) = parse_action_runs_mode(runs_mode) else {
                log::warn!(
                    "SetSectionText: runs_mode='{}' not recognized; use 'preserve' or 'clear'",
                    runs_mode
                );
                return DispatchOutcome::Handled;
            };
            let text_owned = text.clone();
            with_doc_rebuild(core, |rc| {
                super::cross_dispatch::apply_set_section_text(text_owned, clear_runs, rc)
            });
        }
        Action::AddSection { at, text } => {
            let Some(at_opt) = parse_action_optional_usize(at) else {
                log::warn!("AddSection: at='{}' is not a non-negative integer", at);
                return DispatchOutcome::Handled;
            };
            let text_owned = text.clone();
            with_doc_rebuild(core, |rc| {
                super::cross_dispatch::apply_add_section(at_opt, text_owned, rc)
            });
        }
        Action::DeleteSection => with_doc_rebuild(core, super::cross_dispatch::apply_delete_section),
        Action::SplitSection { at_grapheme } => {
            let Some(at_opt) = parse_action_optional_usize(at_grapheme) else {
                log::warn!(
                    "SplitSection: at_grapheme='{}' is not a non-negative integer",
                    at_grapheme
                );
                return DispatchOutcome::Handled;
            };
            with_doc_rebuild(core, |rc| super::cross_dispatch::apply_split_section(at_opt, rc));
        }
        // ── Clipboard ─────────────────────────────────────────
        // Compatible because `clipboard::{read,write}_clipboard`
        // are logged stubs on WASM (pending async-clipboard) and
        // the trait-driven walk over `selection_targets` compiles
        // on both targets.
        Action::Copy => {
            if let Some(doc) = core.document.as_deref_mut() {
                let _ = super::cross_dispatch::apply_copy_or_cut(false, doc);
            }
        }
        // Cut mutates the source component's text — gate the
        // rebuild on at least one target accepting the cut,
        // mirroring the `apply_paste` shape.
        Action::Cut => with_doc_rebuild(core, |rc| {
            if super::cross_dispatch::apply_copy_or_cut(true, rc.document) {
                rc.rebuild_after_geometry_change();
            }
        }),
        Action::Paste => with_doc_rebuild(core, super::cross_dispatch::apply_paste),
        // ── Create-orphan-and-edit (keyboard shape) ───────────
        // The mouse-driven empty-canvas double-click reaches the
        // same `apply_create_orphan_node_and_edit` body through
        // `DoubleClickRoute::CreateOrphanAndEdit`, which uses
        // `DispatchHit::canvas_pos`. This arm is the keyboard shape
        // and uses `cursor_pos`; the two differ only in where the
        // position comes from.
        Action::CreateOrphanNodeAndEdit => {
            let canvas_pos = core
                .renderer
                .screen_to_canvas(core.cursor_pos.0 as f32, core.cursor_pos.1 as f32);
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::rebuild_ctx!(core, doc);
                super::cross_dispatch::apply_create_orphan_node_and_edit(
                    canvas_pos,
                    &mut rc,
                    core.text_edit_state,
                );
            }
        }
        // ── TextEdit cursor primitives ────────────────────────
        // Pure state mutations on `text_edit_state`. The modal
        // handler `handle_text_edit_key` calls `dispatch_action`
        // and refreshes the preview tree afterwards, so arms
        // here only touch state — no rebuild.
        Action::TextEditCursorLeft
        | Action::TextEditCursorRight
        | Action::TextEditCursorUp
        | Action::TextEditCursorDown
        | Action::TextEditCursorHome
        | Action::TextEditCursorEnd
        | Action::TextEditCursorLeftSelect
        | Action::TextEditCursorRightSelect
        | Action::TextEditCursorUpSelect
        | Action::TextEditCursorDownSelect
        | Action::TextEditCursorHomeSelect
        | Action::TextEditCursorEndSelect
        | Action::TextEditDeleteBack
        | Action::TextEditDeleteForward
        | Action::TextEditWordLeft
        | Action::TextEditWordRight
        | Action::TextEditDeleteWordBack
        | Action::TextEditDeleteWordForward => {
            super::super::text_edit::apply_text_edit_action(action.clone(), core.text_edit_state);
        }
        // Catch-all for variants `dispatch_compatible` doesn't
        // own. Two cohorts reach here: NativeOnly arms (caller's
        // fall-through runs them on native; on WASM they're
        // silently skipped, which is fine — well-formed configs
        // don't bind NativeOnly Actions to WASM key combos), and
        // mixed-branch arms whose cross-platform slice fell
        // through (`EditSelection*` on a non-Single selection).
        _ => return DispatchOutcome::Unhandled,
    }
    DispatchOutcome::Handled
}

#[cfg(test)]
mod parsing_tests {
    //! Pin the §4.6 Action payload-parsing helpers. The full
    //! dispatch arms need a `Renderer` per `TEST_CONVENTIONS.md
    //! §T8` so they aren't testable headless; the parsing
    //! helpers are pure data and the load-bearing piece for
    //! "did the macro author's payload reach the apply_*?".
    //! Test Quality #1 flagged 0 tests for the §4.6 dispatch
    //! arms; this mod is the parser-side leg.

    use super::*;

    #[test]
    fn parse_action_runs_mode_clear() {
        assert_eq!(parse_action_runs_mode("clear"), Some(true));
    }

    #[test]
    fn parse_action_runs_mode_preserve() {
        assert_eq!(parse_action_runs_mode("preserve"), Some(false));
    }

    #[test]
    fn parse_action_runs_mode_empty_defaults_to_preserve() {
        assert_eq!(parse_action_runs_mode(""), Some(false));
    }

    #[test]
    fn parse_action_runs_mode_unknown_rejects() {
        assert_eq!(parse_action_runs_mode("invalid"), None);
        assert_eq!(parse_action_runs_mode("PRESERVE"), None);
        assert_eq!(parse_action_runs_mode(" preserve"), None);
    }

    #[test]
    fn parse_action_optional_usize_empty_is_none() {
        assert_eq!(parse_action_optional_usize(""), Some(None));
    }

    #[test]
    fn parse_action_optional_usize_zero_is_some_zero() {
        assert_eq!(parse_action_optional_usize("0"), Some(Some(0)));
    }

    #[test]
    fn parse_action_optional_usize_positive_is_some() {
        assert_eq!(parse_action_optional_usize("42"), Some(Some(42)));
    }

    #[test]
    fn parse_action_optional_usize_negative_rejects() {
        // `usize::parse` rejects negative; `at=-1` should not silently round to 0.
        assert_eq!(parse_action_optional_usize("-1"), None);
    }

    #[test]
    fn parse_action_optional_usize_garbage_rejects() {
        assert_eq!(parse_action_optional_usize("abc"), None);
        assert_eq!(parse_action_optional_usize(" 5"), None);
        assert_eq!(parse_action_optional_usize("5.0"), None);
    }
}

#[cfg(test)]
mod tests {
    //! Unit coverage for the mixed-branch outcome lift. The full
    //! `dispatch_compatible` body needs a `Renderer` per
    //! `TEST_CONVENTIONS.md §T8` so it isn't testable headless;
    //! the lift helper, however, is pure data — pin its contract
    //! here so the WASM-macro `any_ran` regression flagged by the
    //! Track-C parity reviewer can't recur silently.
    use super::*;
    use crate::application::app::dispatch::cross_dispatch::DoubleClickResidual;
    use crate::application::keybinds::Action;

    #[test]
    fn exit_mode_unhandled_lifts_to_handled() {
        let out = lift_mixed_branch_for_wasm_macro(&Action::ExitMode, None, DispatchOutcome::Unhandled);
        assert_eq!(out, DispatchOutcome::Handled);
    }

    #[test]
    fn edit_selection_unhandled_lifts_to_handled() {
        let out = lift_mixed_branch_for_wasm_macro(&Action::EditSelection, None, DispatchOutcome::Unhandled);
        assert_eq!(out, DispatchOutcome::Handled);
    }

    #[test]
    fn edit_selection_clean_unhandled_lifts_to_handled() {
        let out =
            lift_mixed_branch_for_wasm_macro(&Action::EditSelectionClean, None, DispatchOutcome::Unhandled);
        assert_eq!(out, DispatchOutcome::Handled);
    }

    /// No residual means the arm never reached the apply half: there
    /// was no `DispatchHit`, so it logged and touched no document
    /// state. `MacroDispatchTarget::dispatch_action`'s contract
    /// defines that as "did not run", so the lift must decline and
    /// the macro loop's `any_ran` must stay `false`.
    ///
    /// This is the shape a macro actually reaches today — macros
    /// carry no hit — so it is the case that decides whether a
    /// User-tier `[Action(DoubleClickActivate)]` macro falls through
    /// to the custom-mutation tier on WASM. It must.
    #[test]
    fn test_double_click_activate_unhandled_without_a_residual_stays_unhandled() {
        let out =
            lift_mixed_branch_for_wasm_macro(&Action::DoubleClickActivate, None, DispatchOutcome::Unhandled);
        assert_eq!(out, DispatchOutcome::Unhandled);
    }

    /// **The regression this arm keeps producing, at the level where
    /// it is now decidable.** `OpenEdgeLabelEditor` says native has
    /// an editor left to open; it does *not* say the shared half did
    /// anything. It is returned unconditionally, including when the
    /// label was already the current selection and both the commit
    /// and the rebuild were skipped — the normal case, since the
    /// double-click's first click commits the selection.
    ///
    /// The previous discriminator was `hit.is_some()`, which is
    /// `true` here, so this exact input lifted to `Handled` and
    /// reported `any_ran=true` for a step that touched nothing.
    #[test]
    fn test_double_click_activate_edge_label_residual_stays_unhandled() {
        let out = lift_mixed_branch_for_wasm_macro(
            &Action::DoubleClickActivate,
            Some(DoubleClickResidual::OpenEdgeLabelEditor),
            DispatchOutcome::Unhandled,
        );
        assert_eq!(out, DispatchOutcome::Unhandled);
    }

    /// The other direction of the same discriminator: the verdict is
    /// *read* from `double_click_outcome`, not hardwired to "never
    /// lift". A `Done` residual is the arm having run the whole
    /// behavior, and that is `Handled`.
    ///
    /// The pair (`Done`, `Unhandled`) cannot come out of the
    /// dispatcher — `Done` makes the arm return `Handled` and this
    /// function returns early on anything but `Unhandled` — so this
    /// pins the lift's *decision rule* rather than a reachable
    /// dispatch state. Without it, replacing the rule with a constant
    /// `false` would pass the suite while making the entry a lie.
    #[test]
    fn test_double_click_activate_verdict_follows_the_residual_not_a_constant() {
        let out = lift_mixed_branch_for_wasm_macro(
            &Action::DoubleClickActivate,
            Some(DoubleClickResidual::Done),
            DispatchOutcome::Unhandled,
        );
        assert_eq!(out, DispatchOutcome::Handled);
    }

    /// Every member of the one canonical mixed-branch set reaches a
    /// verdict arm in the lift. A member added to
    /// `MIXED_BRANCH_ACTIONS` without one hits the `debug_assert!`
    /// in the catch-all and panics here, which is the enforcement
    /// that keeps the list and the lift from drifting apart again.
    #[test]
    fn test_lift_decides_every_mixed_branch_member() {
        for (action, _) in crate::application::keybinds::MIXED_BRANCH_ACTIONS {
            let out = lift_mixed_branch_for_wasm_macro(&action, None, DispatchOutcome::Unhandled);
            assert!(
                matches!(out, DispatchOutcome::Handled | DispatchOutcome::Unhandled),
                "{:?} produced no verdict",
                action,
            );
        }
    }

    #[test]
    fn handled_passes_through_for_mixed_branch_arms() {
        // The lift only flips Unhandled→Handled; an already-Handled
        // outcome is passed through untouched. (EditSelection on a
        // Single selection returns Handled from the cross-platform
        // dispatcher; we don't want to alter that.)
        for action in [
            Action::ExitMode,
            Action::EditSelection,
            Action::EditSelectionClean,
            Action::DoubleClickActivate,
        ] {
            let out = lift_mixed_branch_for_wasm_macro(&action, None, DispatchOutcome::Handled);
            assert_eq!(out, DispatchOutcome::Handled, "action={:?}", action);
        }
    }

    #[test]
    fn non_mixed_branch_actions_pass_through_both_outcomes() {
        // PURE Compatible arms: their dispatch_compatible behavior
        // is authoritative on both targets. The lift must NOT alter
        // outcomes for them — if cross-platform returned Unhandled
        // for `Undo` (e.g. no document loaded), it stays Unhandled.
        let cases = [
            (Action::Undo, DispatchOutcome::Unhandled),
            (Action::Undo, DispatchOutcome::Handled),
            (Action::ZoomIn, DispatchOutcome::Unhandled),
            (Action::ZoomReset, DispatchOutcome::Handled),
            (Action::SelectAll, DispatchOutcome::Unhandled),
        ];
        for (action, outcome) in cases {
            let out = lift_mixed_branch_for_wasm_macro(&action, None, outcome);
            assert_eq!(out, outcome, "action={:?}", action);
        }
    }

    #[test]
    fn pure_native_only_unhandled_stays_unhandled() {
        // PURE NativeOnly arms (not in the mixed-branch set): the
        // lift must NOT promote these. A WASM macro firing
        // `Action::OpenConsole` should report `any_ran=false` because
        // the dispatcher really did nothing.
        let cases = [
            Action::OpenConsole,
            Action::EnterReparentMode,
            Action::EnterConnectMode,
            Action::SaveDocument,
        ];
        for action in cases {
            let out = lift_mixed_branch_for_wasm_macro(&action, None, DispatchOutcome::Unhandled);
            assert_eq!(
                out,
                DispatchOutcome::Unhandled,
                "lift must not promote PURE NativeOnly: {:?}",
                action,
            );
        }
    }
}
