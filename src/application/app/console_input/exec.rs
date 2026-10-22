// SPDX-License-Identifier: MPL-2.0

//! Console line execution and Ctrl+S save. Split from the dispatcher
//! so the command-runner concern (parse → execute → drain effects)
//! lives independently from the per-keystroke edit logic.

use crate::application::color_picker::ColorPickerState;
use crate::application::console::commands::Command;
use crate::application::console::parser::{parse, Args, ParseResult};
use crate::application::console::{ConsoleEffects, ConsoleSideEffect, ConsoleState, ExecResult, LineOutcome};
use crate::application::document::MindMapDocument;
use crate::application::renderer::Renderer;
use baumhard::mindmap::scene_cache::SceneConnectionCache;
use baumhard::mindmap::tree_builder::MindMapTree;

use super::super::color_picker_flow::{
    close_color_picker_standalone, open_color_picker_contextual, open_color_picker_standalone,
};
use super::super::input_context::InputHandlerContext;
use super::super::scene_rebuild::rebuild_all;
use super::super::single_line_edit::{open_single_line_edit, SingleLineEditTarget, SingleLineEditor};
use super::super::text_edit::TextEditState;
use super::super::{DragState, LastClick};
use super::{push_scrollback_error, push_scrollback_output, push_scrollback_output_in_font};

/// What one [`execute_console_line`] did.
#[must_use = "a console line's outcome is its only report: print it, log it, or return it"]
pub(in crate::application::app) struct LineRun {
    /// What the verb produced.
    pub(in crate::application::app) outcome: LineOutcome,
    /// Whether anything the window draws may have changed: a scene
    /// rebuild, any side effect (the FPS overlay and the modal handoffs
    /// included), or the console closing. A caller deciding whether to
    /// request a redraw reads this rather than assuming one.
    pub(in crate::application::app) changed_view: bool,
}

/// Parse and execute a console line against `ctx`, and return what the
/// verb produced. `None` when no document is loaded, because every verb
/// runs against one. Drains deferred modal handoffs
/// (`open_single_line_edit`, `open_color_picker`), custom mutation
/// apply requests (`run_mutation`, needs tree access), binding overlay
/// updates (`bind_mutation` / `unbind_mutation`, need
/// `ResolvedKeybinds` access), and alias writes (`set_alias`). Rebuilds
/// the scene on any document mutation.
///
/// The one entry point for every caller: the console prompt, the
/// file-path Actions, a macro `ConsoleLine` step, and the IPC
/// `act.console` command. Each takes the same `InputHandlerContext`,
/// so a field the drain needs is threaded in one place, not four.
///
/// **Writes nothing to the scrollback.** Rendering the outcome is the
/// caller's decision, because the callers do not agree on where it
/// goes. Only the prompt prints the outcome into the console, through
/// [`print_console_line`]. The others log a failure through
/// [`log_console_line`], and IPC also returns the outcome in its
/// reply. No caller but the prompt touches the scrollback
/// (`format/ipc.md` §act).
///
/// `#[must_use]` here as well as on [`LineRun`]: the lint does not
/// reach through the `Option` this returns, so without it a bare call
/// that drops the line's only report would compile clean.
#[must_use = "a console line's outcome is its only report: print it, log it, or return it"]
pub(in crate::application::app) fn execute_console_line(
    ctx: &mut InputHandlerContext<'_>,
    line: &str,
) -> Option<LineRun> {
    let doc = ctx.document.as_mut()?;
    if line.trim().is_empty() {
        return Some(LineRun {
            outcome: LineOutcome::Blank,
            changed_view: false,
        });
    }
    let (cmd, args) = match parse(line) {
        ParseResult::Ok { cmd, args } => (cmd, args),
        ParseResult::Empty => {
            return Some(LineRun {
                outcome: LineOutcome::Blank,
                changed_view: false,
            })
        }
        ParseResult::Unknown(head) => {
            return Some(LineRun {
                outcome: LineOutcome::Unknown(head),
                changed_view: false,
            })
        }
    };
    let cmd: &'static Command = cmd;
    let mut effects = ConsoleEffects::new(doc);
    let result = (cmd.execute)(&Args::new(&args), &mut effects);
    let document_mutated = effects.document_mutated();
    let side_effect = effects.side_effect.take();
    let close_after = effects.close_console;
    let had_side_effect = side_effect.is_some();

    // Decided before the pre-rebuild handler consumes the effect,
    // because two of its arms write state the rebuild has to see —
    // the document swap and the interaction-mode flip both land
    // through the raw `&mut` this function holds rather than through
    // `ConsoleEffects`, so `document_mutated` cannot know about
    // them.
    let needs_rebuild = console_line_needs_rebuild(document_mutated, side_effect.as_ref());

    // Document swap from `open` / `new` happens before
    // `rebuild_all` so the rebuild sees the new doc; the others
    // happen after rebuild_all because they transition modal
    // state on top of the rebuilt scene. `set_fps_display` is
    // also pre-rebuild because the FPS overlay is screen-space
    // and doesn't share state with the rest of `rebuild_all`.
    let post_rebuild = handle_pre_rebuild_side_effect(
        side_effect,
        doc,
        SessionState {
            interaction_mode: ctx.interaction_mode,
            mindmap_tree: ctx.mindmap_tree,
            single_line_edit_state: ctx.single_line_edit_state,
            color_picker_state: ctx.color_picker_state,
            text_edit_state: ctx.text_edit_state,
            drag_state: ctx.drag_state,
            hovered_node: ctx.hovered_node,
            last_click: ctx.last_click,
        },
        ctx.macros,
        ctx.renderer,
    );

    // Rebuild only when something the canvas draws could have
    // moved. Pre-#37 this ran unconditionally, so `help`, `fps`,
    // `mutation list` and every command that failed after reading
    // the document each dropped the whole connection cache and ran
    // a full `doc.build_tree()` + cosmic-text buffer rebuild for
    // output that never left the scrollback.
    if needs_rebuild {
        ctx.scene_cache.clear();
        rebuild_all(
            doc,
            ctx.interaction_mode,
            ctx.mindmap_tree,
            ctx.app_scene,
            ctx.renderer,
            ctx.scene_cache,
        );
    }

    let opened_modal = handle_post_rebuild_side_effect(
        post_rebuild,
        doc,
        ctx.interaction_mode,
        ctx.mindmap_tree,
        ctx.single_line_edit_state,
        ctx.color_picker_state,
        ctx.text_edit_state,
        ctx.app_scene,
        ctx.renderer,
        ctx.scene_cache,
    );
    if opened_modal || close_after {
        *ctx.console_state = ConsoleState::Closed;
        ctx.renderer.rebuild_console_overlay_buffers(ctx.app_scene, None);
    }
    Some(LineRun {
        outcome: LineOutcome::Ran(result),
        changed_view: needs_rebuild || had_side_effect || opened_modal || close_after,
    })
}

/// Print what [`execute_console_line`] produced to the console
/// prompt that submitted the line: output lines in their pinned
/// faces, a failure in the error color.
///
/// **The scrollback is the prompt's own transcript.** This is its only
/// writer. Every other caller of [`execute_console_line`] reports
/// through [`log_console_line`] instead: a key-bound file Action, a
/// macro step, and the IPC `act.console` / `act.action` /
/// `act.macro` commands. Those callers do not own the console. They
/// can fire while it is open (an IPC command, or a mouse binding),
/// and a line pushed into the scrollback from outside would show
/// output the user never typed for, in an overlay nothing rebuilds
/// until their next keystroke.
pub(in crate::application::app) fn print_console_line(
    outcome: LineOutcome,
    console_state: &mut ConsoleState,
) {
    if let Some(failure) = outcome.failure() {
        push_scrollback_error(console_state, failure);
        return;
    }
    match outcome {
        LineOutcome::Ran(ExecResult::Ok(s)) if !s.is_empty() => push_scrollback_output(console_state, s),
        LineOutcome::Ran(ExecResult::Lines(lines)) => {
            for line in lines {
                push_scrollback_output_in_font(console_state, line.text, line.font_family);
            }
        }
        // Blank lines, empty successes, and the failures handled above.
        _ => {}
    }
}

/// Report what [`execute_console_line`] produced for a caller that is
/// not the console prompt (see [`print_console_line`] for why such a
/// caller never writes the scrollback).
///
/// A failure is `warn!`ed, so a bound `open` of a missing file still
/// says why it did nothing (§9). Under `--ipc` the same line is the
/// command's reply warning. Successful output has no reader and is
/// dropped.
pub(in crate::application::app) fn log_console_line(line: &str, outcome: &LineOutcome) {
    if let Some(failure) = outcome.failure() {
        log::warn!(
            code = crate::application::console::LINE_FAILED_CODE;
            "console: `{line}` failed: {failure}"
        );
    }
}

/// Whether a console line's execution owes the canvas a
/// `scene_cache.clear()` + `rebuild_all`.
///
/// Two independent sources, because the command surface has two
/// ways to change what is drawn: writing the document (reported by
/// `ConsoleEffects::document_mutated`, which is raised by the
/// `document_mut` borrow itself so no command can forget it) and
/// requesting a [`ConsoleSideEffect`] the dispatcher applies on the
/// command's behalf.
///
/// Pure and separate from [`execute_console_line`] so the answer is
/// pinnable per console line — the full path takes a `&mut
/// Renderer`, which TEST_CONVENTIONS §T8 keeps out of the harness,
/// and nothing after this decision feeds back into it.
fn console_line_needs_rebuild(document_mutated: bool, side_effect: Option<&ConsoleSideEffect>) -> bool {
    document_mutated || side_effect_changes_the_canvas(side_effect)
}

/// Whether a command's [`ConsoleSideEffect`] changes something the
/// canvas draws, and so needs the rebuild even when the command
/// never took `ConsoleEffects::document_mut`.
///
/// Every variant does except one. `SetFpsDisplay` is the exception
/// for the reason [`handle_pre_rebuild_side_effect`] already gives
/// for running it early: the FPS overlay is screen-space and shares
/// no state with the scene tree, so `fps on` / `fps debug` /
/// `fps off` are the one modal-ish transition the scene does not
/// have to be re-projected for.
///
/// Written as an exhaustive match rather than a `matches!` so a new
/// variant cannot inherit an answer by omission — the default for a
/// transition nobody has thought about is "rebuild", and the
/// compiler asks.
fn side_effect_changes_the_canvas(side_effect: Option<&ConsoleSideEffect>) -> bool {
    match side_effect {
        None => false,
        Some(ConsoleSideEffect::SetFpsDisplay(_)) => false,
        Some(
            ConsoleSideEffect::ReplaceDocument(_)
            | ConsoleSideEffect::SetInteractionMode(_)
            | ConsoleSideEffect::OpenSectionEdit { .. }
            | ConsoleSideEffect::OpenLabelEdit(_)
            | ConsoleSideEffect::OpenPortalTextEdit(..)
            | ConsoleSideEffect::OpenColorPicker(_)
            | ConsoleSideEffect::OpenColorPickerStandalone
            | ConsoleSideEffect::CloseColorPicker,
        ) => true,
    }
}

/// The interaction state a document swap must not leave pointing at
/// the document it replaced. Grouped so [`handle_pre_rebuild_side_effect`]
/// takes one value for it rather than eight loose borrows.
struct SessionState<'s> {
    interaction_mode: &'s mut super::super::InteractionMode,
    mindmap_tree: &'s mut Option<MindMapTree>,
    single_line_edit_state: &'s mut SingleLineEditor,
    color_picker_state: &'s mut ColorPickerState,
    text_edit_state: &'s mut TextEditState,
    drag_state: &'s mut DragState,
    hovered_node: &'s mut Option<String>,
    last_click: &'s mut Option<LastClick>,
}

/// Apply the side effects that need to land **before** the
/// `rebuild_all` pass: wholesale document swap (so the rebuild
/// sees the new doc) and FPS overlay toggle (orthogonal to the
/// scene tree). Returns the side effect untouched if it's a
/// post-rebuild modal transition; returns `None` if the effect
/// was consumed here.
fn handle_pre_rebuild_side_effect(
    side_effect: Option<ConsoleSideEffect>,
    doc: &mut MindMapDocument,
    session: SessionState<'_>,
    macros: &mut crate::application::macros::MacroRegistry,
    renderer: &mut Renderer,
) -> Option<ConsoleSideEffect> {
    let SessionState {
        interaction_mode,
        mindmap_tree,
        single_line_edit_state,
        color_picker_state,
        text_edit_state,
        drag_state,
        hovered_node,
        last_click,
    } = session;
    match side_effect? {
        ConsoleSideEffect::ReplaceDocument(new_doc) => {
            *doc = *new_doc;
            *mindmap_tree = None;
            *single_line_edit_state = SingleLineEditor::Closed;
            *color_picker_state = ColorPickerState::Closed;
            // Every interaction in flight names ids, a tree or a hit
            // of the document just replaced. The node text editor
            // would commit its buffer into whatever node carries the
            // old id in the new map. A throttled drag would drive the
            // old gesture against the new tree on the next drain. A
            // remembered click or hover would pair a stale hit with
            // the next real one. None of them survives the swap. The
            // editor's own revert targets the old tree, which is
            // already gone, so closing the state is the whole cancel.
            // Reachable whenever the swap does not come from the
            // prompt, which the key-stealing modals otherwise guard:
            // an IPC `act.console "open …"`, or a file Action bound
            // to a mouse gesture.
            *text_edit_state = TextEditState::Closed;
            *drag_state = DragState::None;
            *hovered_node = None;
            *last_click = None;
            // The window's cursor icon is deliberately left alone: it
            // can still show a resize arrow or a button hand from the
            // old document until the pointer next moves, when
            // `event_cursor_moved` recomputes it. Resetting it here would
            // need the `Window`, which this drain does not hold, to fix a
            // glyph that heals on the user's next motion.
            // Reset interaction mode: a stale `NodeEdit { node_id }`
            // or `Resize { target }` from the prior document points
            // at ids that don't exist in the new one — the next
            // rebuild would render `editing: <stale-id>` overlay and
            // dim the entire new map (no node matches the stale id).
            *interaction_mode = super::super::InteractionMode::Default;
            // Clear the renderer's status overlay too, in case the
            // mode-status setter was last called for an
            // already-stale mode value before this swap landed.
            renderer.set_mode_status_text(None);
            // Rebuild the document-derived tiers (Map + Inline).
            // App and User tiers loaded at startup are untouched.
            // The single-entry helper enforces Map-then-Inline
            // ordering (Inline is highest precedence) so the
            // two-call ordering can't drift between this site
            // and `run_native_init::build`.
            crate::application::macros::loader::rebuild_document_macros(macros, doc);
            None
        }
        ConsoleSideEffect::SetFpsDisplay(mode) => {
            // The decree bus clears the overlay buffers when
            // toggled off; the rebuild helper in
            // `Renderer::process()` re-shapes them on the next
            // frame when toggled on.
            renderer.set_fps_display(mode);
            None
        }
        ConsoleSideEffect::SetInteractionMode(mode) => {
            // Flip the mode in place so the rebuild that runs
            // after this helper sees the new value when reading
            // `interaction_mode.resize_handle_overrides()`. No
            // separate rebuild here — `execute_console_line`'s
            // post-handler `rebuild_all` covers it.
            *interaction_mode = mode;
            None
        }
        ConsoleSideEffect::OpenSectionEdit { node_id, section_idx } => {
            // Set the document selection + interaction mode before
            // the rebuild so the rebuild sees the section-frame
            // chrome on the right node. The actual text-editor
            // open happens in `handle_post_rebuild_side_effect`
            // (text_edit_state isn't in this handler's signature).
            // Re-emit the side effect so the post-handler can
            // pick it up.
            doc.selection = crate::application::document::SelectionState::Section(
                crate::application::document::SectionSel {
                    node_id: node_id.clone(),
                    section_idx,
                },
            );
            *interaction_mode = super::super::InteractionMode::NodeEdit {
                node_id: node_id.clone(),
            };
            Some(ConsoleSideEffect::OpenSectionEdit { node_id, section_idx })
        }
        other => Some(other),
    }
}

/// Apply post-rebuild modal transitions. Returns `true` if a
/// modal opened (so the dispatcher closes the console too).
#[allow(clippy::too_many_arguments)]
fn handle_post_rebuild_side_effect(
    side_effect: Option<ConsoleSideEffect>,
    doc: &mut MindMapDocument,
    interaction_mode: &mut super::super::InteractionMode,
    mindmap_tree: &mut Option<MindMapTree>,
    single_line_edit_state: &mut SingleLineEditor,
    color_picker_state: &mut ColorPickerState,
    text_edit_state: &mut super::super::text_edit::TextEditState,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut SceneConnectionCache,
) -> bool {
    let Some(eff) = side_effect else { return false };
    match eff {
        // `label edit` seeds the editor with the existing text —
        // the verb has no "clean" spelling, so `clean = false`.
        ConsoleSideEffect::OpenLabelEdit(er) => {
            open_single_line_edit(
                SingleLineEditTarget::EdgeLabel { edge_ref: er },
                false,
                doc,
                single_line_edit_state,
                app_scene,
                renderer,
            );
        }
        ConsoleSideEffect::OpenPortalTextEdit(er, endpoint) => {
            open_single_line_edit(
                SingleLineEditTarget::PortalText {
                    edge_ref: er,
                    endpoint_node_id: endpoint,
                },
                false,
                doc,
                single_line_edit_state,
                app_scene,
                renderer,
            );
        }
        ConsoleSideEffect::OpenColorPicker(target) => {
            open_color_picker_contextual(
                target,
                doc,
                color_picker_state,
                interaction_mode,
                app_scene,
                renderer,
                scene_cache,
            );
        }
        ConsoleSideEffect::OpenColorPickerStandalone => {
            open_color_picker_standalone(
                doc,
                color_picker_state,
                interaction_mode,
                app_scene,
                renderer,
                scene_cache,
            );
        }
        ConsoleSideEffect::CloseColorPicker => {
            close_color_picker_standalone(
                color_picker_state,
                doc,
                interaction_mode,
                mindmap_tree,
                app_scene,
                renderer,
                scene_cache,
            );
        }
        ConsoleSideEffect::OpenSectionEdit { .. } => {
            // Pre-rebuild handler already wrote `doc.selection =
            // Section { node_id, section_idx }` + flipped
            // `interaction_mode = NodeEdit { node_id }`. Delegate
            // the actual editor open to `apply_enter_section_edit`
            // — the canonical Action-side path — for the
            // `OwnerMismatch` validation and consistent posture
            // with `Action::EnterSectionEdit`. Pre-fix this
            // re-implemented `open_text_edit` directly,
            // bypassing the validation (Architecture #4).
            let mut rc = super::super::dispatch::cross_dispatch::RebuildContext {
                document: doc,
                mindmap_tree,
                app_scene,
                renderer,
                scene_cache,
                interaction_mode,
            };
            super::super::dispatch::cross_dispatch::apply_enter_section_edit(
                /* clean */ false,
                &mut rc,
                text_edit_state,
            );
        }
        // Pre-rebuild variants — already consumed. Per
        // CODE_CONVENTIONS §9 (interactive paths must not panic),
        // log + soft-skip instead of `unreachable!`. A future
        // contributor adding a variant that forgets the pre-
        // rebuild arm will see a loud log line, not a crash.
        ConsoleSideEffect::ReplaceDocument(_)
        | ConsoleSideEffect::SetFpsDisplay(_)
        | ConsoleSideEffect::SetInteractionMode(_) => {
            log::error!(
                "console: {:?} reached post-rebuild handler; should have been consumed by \
                 handle_pre_rebuild_side_effect — ignoring to avoid crash",
                eff
            );
            return false;
        }
    }
    true
}

/// Persist the document to its bound `file_path`, clear the dirty
/// flag, and report the outcome to the log. This is the body of
/// `Action::SaveDocument` (`Ctrl+S` by default). When no path is
/// bound it `warn!`s a hint pointing at `save <path>` and leaves the
/// dirty flag untouched.
///
/// It reports through the log only, never the console scrollback,
/// for the reason [`print_console_line`] gives: the Action is not the
/// console prompt, and under `--ipc` it can fire while the console is
/// open. The `save` verb typed at the prompt reports through its own
/// `ExecResult`. Each failure carries a code, so a caller collecting
/// diagnostics can tell "nothing is bound" (fixable with a save-as)
/// from a write that failed.
///
/// Returns whether the document was written.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn save_document_to_bound_path(doc: &mut MindMapDocument) -> bool {
    use crate::application::console::{SAVE_FAILED_CODE, SAVE_UNBOUND_CODE};
    let Some(path) = doc.file_path.clone() else {
        log::warn!(code = SAVE_UNBOUND_CODE; "save: no file path bound; use `save <path>` to choose one");
        return false;
    };
    match baumhard::mindmap::loader::save_to_file(std::path::Path::new(&path), &doc.mindmap) {
        Ok(()) => {
            doc.dirty = false;
            log::info!("save: saved to {}", path);
            true
        }
        Err(e) => {
            log::error!(code = SAVE_FAILED_CODE; "save: {}", e);
            false
        }
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::application::document::tests_common::load_test_doc;
    use crate::application::document::SelectionState;

    /// What the caller wants selected before the line runs. Several
    /// verbs refuse to touch the document without one, so "no
    /// selection" is not a neutral fixture — it is its own case.
    enum Sel {
        None,
        SingleNode,
    }

    /// Run one console line against the testament fixture and report
    /// the rebuild gate's answer, computed exactly as
    /// [`execute_console_line`] computes it.
    ///
    /// Renderer-free: everything downstream of the gate needs a
    /// device, and none of it feeds back into the decision.
    fn rebuild_gate_for(line: &str, sel: Sel) -> bool {
        let mut doc = load_test_doc();
        if let Sel::SingleNode = sel {
            let id = doc
                .mindmap
                .nodes
                .keys()
                .min()
                .cloned()
                .expect("the testament fixture has nodes");
            doc.selection = SelectionState::Single(id);
        }
        let (cmd, tokens) = match parse(line) {
            ParseResult::Ok { cmd, args } => (cmd, args),
            ParseResult::Empty => panic!("fixture line {line:?} must parse; got Empty"),
            ParseResult::Unknown(head) => {
                panic!("fixture line {line:?} must parse; got Unknown({head})")
            }
        };
        let mut eff = ConsoleEffects::new(&mut doc);
        let _ = (cmd.execute)(&Args::new(&tokens), &mut eff);
        console_line_needs_rebuild(eff.document_mutated(), eff.side_effect.as_ref())
    }

    /// #37 item 3's acceptance criterion, plus the three neighbors
    /// the issue names beside it. None of these four lines writes
    /// anything the canvas draws: `help` and `mutation list` render
    /// scrollback text out of a document they only read, `fps`
    /// toggles a screen-space overlay that shares no state with the
    /// scene tree, and `border show` with no selection fails before
    /// it reaches a setter.
    ///
    /// Pre-#37 every one of them dropped the whole
    /// `SceneConnectionCache` and ran a full `doc.build_tree()` plus
    /// a cosmic-text buffer rebuild, because the clear + rebuild sat
    /// unconditionally after the command.
    ///
    /// Fails on any of these lines the moment the gate stops reading
    /// `document_mutated` — the pre-fix shape is `true` for every
    /// input.
    #[test]
    fn test_a_console_line_that_only_reads_the_document_owes_no_rebuild() {
        for line in ["help", "mutation list", "fps on", "border show"] {
            assert!(
                !rebuild_gate_for(line, Sel::None),
                "`{line}` writes nothing the canvas draws, so it must not clear the scene cache"
            );
        }
    }

    /// The control that keeps the row above from being "answer
    /// `false` always" — one line per source the gate reads.
    ///
    /// `node fit` writes through `ConsoleEffects::document_mut` and
    /// nothing else; `mode default` never touches the document at
    /// all and instead hands back
    /// `ConsoleSideEffect::SetInteractionMode`, which the dispatcher
    /// applies through its own `&mut` and which the mode-status
    /// overlay and the resize-handle trees both read. Drop either
    /// source from the gate and exactly one of these two rows goes
    /// red.
    #[test]
    fn test_a_console_line_that_changes_the_canvas_still_owes_a_rebuild() {
        assert!(
            rebuild_gate_for("node fit", Sel::SingleNode),
            "a document write must still clear the cache and rebuild"
        );
        assert!(
            rebuild_gate_for("mode default", Sel::None),
            "an interaction-mode flip changes the mode overlay and the handle trees"
        );
    }

    /// `fps` is the one side effect the gate lets through without a
    /// rebuild, so it is the one worth stating on its own: the
    /// overlay is screen-space, and `handle_pre_rebuild_side_effect`
    /// already runs it before the rebuild for that reason.
    ///
    /// Fails if `side_effect_changes_the_canvas` is widened to "any
    /// side effect", which is the obvious over-cautious version of
    /// this gate.
    #[test]
    fn test_the_fps_overlay_toggle_is_the_one_side_effect_that_needs_no_rebuild() {
        assert!(!side_effect_changes_the_canvas(Some(
            &ConsoleSideEffect::SetFpsDisplay(crate::application::common::FpsDisplayMode::Snapshot)
        )));
        assert!(side_effect_changes_the_canvas(Some(
            &ConsoleSideEffect::CloseColorPicker
        )));
        assert!(!side_effect_changes_the_canvas(None));
    }

    /// A failed line run by anything but the prompt is logged under
    /// `LINE_FAILED_CODE`, the code an IPC reply carries it as. A line
    /// that worked logs nothing.
    #[test]
    fn test_log_console_line_warns_a_failure_under_its_code_and_nothing_else() {
        use baumhard::util::log::capture::capture;

        baumhard::util::test_logger::install();
        let ((), captured) =
            capture(|| log_console_line("frobnicate-5d1e", &LineOutcome::Unknown("frobnicate-5d1e".into())));
        assert_eq!(captured.records.len(), 1, "{captured:?}");
        assert_eq!(
            captured.records[0].code.as_deref(),
            Some(crate::application::console::LINE_FAILED_CODE)
        );
        assert!(captured.records[0]
            .message
            .contains("unknown command: frobnicate-5d1e"));

        let ((), captured) = capture(|| {
            log_console_line("help", &LineOutcome::Ran(ExecResult::Ok("fine".into())));
            log_console_line("", &LineOutcome::Blank);
        });
        assert!(
            captured.records.is_empty(),
            "success and blank lines report nothing: {captured:?}"
        );
    }

    /// `SaveDocument`'s three outcomes, each with the code a caller
    /// can branch on: no bound path, a write that fails, and a write
    /// that succeeds and clears the dirty flag.
    #[test]
    fn test_save_to_bound_path_reports_each_outcome_under_its_code() {
        use crate::application::console::{SAVE_FAILED_CODE, SAVE_UNBOUND_CODE};
        use baumhard::util::log::capture::capture;

        baumhard::util::test_logger::install();
        let mut doc = load_test_doc();
        doc.file_path = None;
        let (saved, captured) = capture(|| save_document_to_bound_path(&mut doc));
        assert!(!saved);
        assert_eq!(captured.records[0].code.as_deref(), Some(SAVE_UNBOUND_CODE));

        let dir = baumhard::util::test_temp::TempDir::new("save-codes");
        doc.file_path = Some(
            dir.path()
                .join("missing-dir/m.mindmap.json")
                .display()
                .to_string(),
        );
        let (saved, captured) = capture(|| save_document_to_bound_path(&mut doc));
        assert!(!saved, "a parent that does not exist cannot be written into");
        assert_eq!(captured.records[0].code.as_deref(), Some(SAVE_FAILED_CODE));

        doc.file_path = Some(dir.path().join("m.mindmap.json").display().to_string());
        doc.dirty = true;
        let (saved, captured) = capture(|| save_document_to_bound_path(&mut doc));
        assert!(saved && !doc.dirty, "a written save clears the dirty flag");
        assert!(captured.records.is_empty(), "{captured:?}");
    }
}
