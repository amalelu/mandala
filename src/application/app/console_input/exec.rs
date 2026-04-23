//! Console line execution and Ctrl+S save. Split from the dispatcher
//! so the command-runner concern (parse → execute → drain effects)
//! lives independently from the per-keystroke edit logic.

use crate::application::color_picker::ColorPickerState;
use crate::application::console::commands::Command;
use crate::application::console::parser::{parse, Args, ParseResult};
use crate::application::console::{ConsoleEffects, ConsoleState, ExecResult};
use crate::application::document::MindMapDocument;
use crate::application::renderer::Renderer;

use super::super::color_picker_flow::{
    close_color_picker_standalone, open_color_picker_contextual,
    open_color_picker_standalone,
};
use super::super::{
    open_label_edit, open_portal_text_edit, rebuild_all, LabelEditState, PortalTextEditState,
};
use super::{push_scrollback_error, push_scrollback_output};

/// Parse and execute a console line. Drains deferred modal handoffs
/// (`open_label_edit`, `open_color_picker`), custom mutation apply
/// requests (`run_mutation`, needs tree access), binding overlay
/// updates (`bind_mutation` / `unbind_mutation`, need
/// `ResolvedKeybinds` access), and alias writes (`set_alias`).
/// Appends the result to the scrollback; rebuilds the scene on any
/// document mutation.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn execute_console_line(
    line: &str,
    console_state: &mut ConsoleState,
    label_edit_state: &mut LabelEditState,
    portal_text_edit_state: &mut PortalTextEditState,
    color_picker_state: &mut ColorPickerState,
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    if line.trim().is_empty() {
        return;
    }
    let (cmd, args) = match parse(line) {
        ParseResult::Ok { cmd, args } => (cmd, args),
        ParseResult::Empty => return,
        ParseResult::Unknown(ref head) => {
            push_scrollback_error(
                console_state,
                format!("unknown command: {}", head),
            );
            return;
        }
    };
    let cmd: &'static Command = cmd;
    let mut effects = ConsoleEffects::new(doc);
    let result = (cmd.execute)(&Args::new(&args), &mut effects);
    let label_edit_req = effects.open_label_edit.take();
    let portal_text_edit_req = effects.open_portal_text_edit.take();
    let color_picker_req = effects.open_color_picker.take();
    let color_picker_standalone_req = effects.open_color_picker_standalone;
    let color_picker_close_req = effects.close_color_picker;
    let close_after = effects.close_console;
    let fps_display_req = effects.set_fps_display.take();
    let replace_doc = effects.replace_document.take();

    // Emit the command's result lines into the scrollback.
    match result {
        ExecResult::Ok(s) => {
            if !s.is_empty() {
                push_scrollback_output(console_state, s);
            }
        }
        ExecResult::Err(s) => push_scrollback_error(console_state, s),
        ExecResult::Lines(lines) => {
            for l in lines {
                push_scrollback_output(console_state, l);
            }
        }
    }

    // Wholesale document swap from `open` / `new`. Drop the cached
    // tree so `rebuild_all` rebuilds it fresh against the new map,
    // and clear any open modal-editor state so stale references
    // into the old document can't outlive the swap.
    if let Some(new_doc) = replace_doc {
        *doc = new_doc;
        *mindmap_tree = None;
        *label_edit_state = LabelEditState::Closed;
        *portal_text_edit_state = PortalTextEditState::Closed;
        *color_picker_state = ColorPickerState::Closed;
    }

    // `fps on` / `fps off` — forward to the renderer. The decree bus
    // clears the overlay buffers when toggled off; the rebuild helper
    // in `Renderer::process()` re-shapes them on the next frame when
    // toggled on.
    if let Some(mode) = fps_display_req {
        renderer.set_fps_display(mode);
    }

    // Any successful command may have mutated the doc; rebuild.
    scene_cache.clear();
    rebuild_all(doc, mindmap_tree, app_scene, renderer);

    if let Some(er) = label_edit_req {
        open_label_edit(&er, doc, label_edit_state, app_scene, renderer);
        *console_state = ConsoleState::Closed;
        renderer.rebuild_console_overlay_buffers(app_scene, None);
    } else if let Some((er, endpoint)) = portal_text_edit_req {
        open_portal_text_edit(
            &er,
            &endpoint,
            doc,
            portal_text_edit_state,
            app_scene,
            renderer,
        );
        *console_state = ConsoleState::Closed;
        renderer.rebuild_console_overlay_buffers(app_scene, None);
    } else if let Some(target) = color_picker_req {
        open_color_picker_contextual(target, doc, color_picker_state, app_scene, renderer);
        *console_state = ConsoleState::Closed;
        renderer.rebuild_console_overlay_buffers(app_scene, None);
    } else if color_picker_standalone_req {
        open_color_picker_standalone(doc, color_picker_state, app_scene, renderer);
        *console_state = ConsoleState::Closed;
        renderer.rebuild_console_overlay_buffers(app_scene, None);
    } else if color_picker_close_req {
        close_color_picker_standalone(
            color_picker_state,
            doc,
            mindmap_tree,
            app_scene,
            renderer,
        );
        *console_state = ConsoleState::Closed;
        renderer.rebuild_console_overlay_buffers(app_scene, None);
    } else if close_after {
        *console_state = ConsoleState::Closed;
        renderer.rebuild_console_overlay_buffers(app_scene, None);
    }
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
    match baumhard::mindmap::loader::save_to_file(
        std::path::Path::new(&path),
        &doc.mindmap,
    ) {
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
