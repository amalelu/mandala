// SPDX-License-Identifier: MPL-2.0

//! Console line execution and Ctrl+S save. Split from the dispatcher
//! so the command-runner concern (parse → execute → drain effects)
//! lives independently from the per-keystroke edit logic.

use crate::application::color_picker::ColorPickerState;
use crate::application::console::commands::Command;
use crate::application::console::parser::{parse, Args, ParseResult};
use crate::application::console::{ConsoleEffects, ConsoleSideEffect, ConsoleState, ExecResult};
use crate::application::document::MindMapDocument;
use crate::application::renderer::Renderer;
use baumhard::mindmap::scene_cache::SceneConnectionCache;
use baumhard::mindmap::tree_builder::MindMapTree;

use super::super::color_picker_flow::{
    close_color_picker_standalone, open_color_picker_contextual, open_color_picker_standalone,
};
use super::super::scene_rebuild::rebuild_all;
use super::super::single_line_edit::{open_single_line_edit, SingleLineEditTarget, SingleLineEditor};
use super::{push_scrollback_error, push_scrollback_output, push_scrollback_output_in_font};

/// Parse and execute a console line. Drains deferred modal handoffs
/// (`open_single_line_edit`, `open_color_picker`), custom mutation apply
/// requests (`run_mutation`, needs tree access), binding overlay
/// updates (`bind_mutation` / `unbind_mutation`, need
/// `ResolvedKeybinds` access), and alias writes (`set_alias`).
/// Appends the result to the scrollback; rebuilds the scene on any
/// document mutation.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
pub(in crate::application::app) fn execute_console_line(
    line: &str,
    console_state: &mut ConsoleState,
    single_line_edit_state: &mut SingleLineEditor,
    color_picker_state: &mut ColorPickerState,
    text_edit_state: &mut super::super::text_edit::TextEditState,
    doc: &mut MindMapDocument,
    interaction_mode: &mut super::super::InteractionMode,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
    macros: &mut crate::application::macros::MacroRegistry,
) {
    if line.trim().is_empty() {
        return;
    }
    let (cmd, args) = match parse(line) {
        ParseResult::Ok { cmd, args } => (cmd, args),
        ParseResult::Empty => return,
        ParseResult::Unknown(ref head) => {
            push_scrollback_error(console_state, format!("unknown command: {}", head));
            return;
        }
    };
    let cmd: &'static Command = cmd;
    let mut effects = ConsoleEffects::new(doc);
    let result = (cmd.execute)(&Args::new(&args), &mut effects);
    let side_effect = effects.side_effect.take();
    let close_after = effects.close_console;

    // Emit the command's result lines into the scrollback.
    match result {
        ExecResult::Ok(s) => {
            if !s.is_empty() {
                push_scrollback_output(console_state, s);
            }
        }
        ExecResult::Err(s) => push_scrollback_error(console_state, s),
        ExecResult::Lines(lines) => {
            for line in lines {
                push_scrollback_output_in_font(console_state, line.text, line.font_family);
            }
        }
    }

    // Document swap from `open` / `new` happens before
    // `rebuild_all` so the rebuild sees the new doc; the others
    // happen after rebuild_all because they transition modal
    // state on top of the rebuilt scene. `set_fps_display` is
    // also pre-rebuild because the FPS overlay is screen-space
    // and doesn't share state with the rest of `rebuild_all`.
    let post_rebuild = handle_pre_rebuild_side_effect(
        side_effect,
        doc,
        interaction_mode,
        mindmap_tree,
        single_line_edit_state,
        color_picker_state,
        macros,
        renderer,
    );

    // Any successful command may have mutated the doc; rebuild.
    scene_cache.clear();
    rebuild_all(
        doc,
        interaction_mode,
        mindmap_tree,
        app_scene,
        renderer,
        scene_cache,
    );

    let opened_modal = handle_post_rebuild_side_effect(
        post_rebuild,
        doc,
        interaction_mode,
        mindmap_tree,
        single_line_edit_state,
        color_picker_state,
        text_edit_state,
        app_scene,
        renderer,
        scene_cache,
    );
    if opened_modal || close_after {
        *console_state = ConsoleState::Closed;
        renderer.rebuild_console_overlay_buffers(app_scene, None);
    }
}

/// Apply the side effects that need to land **before** the
/// `rebuild_all` pass: wholesale document swap (so the rebuild
/// sees the new doc) and FPS overlay toggle (orthogonal to the
/// scene tree). Returns the side effect untouched if it's a
/// post-rebuild modal transition; returns `None` if the effect
/// was consumed here.
#[allow(clippy::too_many_arguments)]
fn handle_pre_rebuild_side_effect(
    side_effect: Option<ConsoleSideEffect>,
    doc: &mut MindMapDocument,
    interaction_mode: &mut super::super::InteractionMode,
    mindmap_tree: &mut Option<MindMapTree>,
    single_line_edit_state: &mut SingleLineEditor,
    color_picker_state: &mut ColorPickerState,
    macros: &mut crate::application::macros::MacroRegistry,
    renderer: &mut Renderer,
) -> Option<ConsoleSideEffect> {
    match side_effect? {
        ConsoleSideEffect::ReplaceDocument(new_doc) => {
            *doc = new_doc;
            *mindmap_tree = None;
            *single_line_edit_state = SingleLineEditor::Closed;
            *color_picker_state = ColorPickerState::Closed;
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
                "{:?} reached post-rebuild handler; should have been consumed by \
                 handle_pre_rebuild_side_effect — ignoring to avoid crash",
                eff
            );
            return false;
        }
    }
    true
}

/// Persist the document to its bound `file_path`, clear the dirty
/// flag, and surface the outcome — to the console scrollback when
/// open, and always to the log. Used by the `Ctrl+S` keybind. When
/// no path is bound, surfaces a hint pointing the user at `save
/// <path>` from the console; the dirty flag is left untouched.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn save_document_to_bound_path(
    doc: &mut MindMapDocument,
    console_state: &mut ConsoleState,
) {
    let path = match doc.file_path.clone() {
        Some(p) => p,
        None => {
            let msg = "no file path bound; use `save <path>` to choose one".to_string();
            log::warn!("{}", msg);
            push_scrollback_error(console_state, msg);
            return;
        }
    };
    match baumhard::mindmap::loader::save_to_file(std::path::Path::new(&path), &doc.mindmap) {
        Ok(()) => {
            doc.dirty = false;
            let msg = format!("saved to {}", path);
            log::info!("{}", msg);
            push_scrollback_output(console_state, msg);
        }
        Err(e) => {
            log::error!("{}", e);
            push_scrollback_error(console_state, e);
        }
    }
}
