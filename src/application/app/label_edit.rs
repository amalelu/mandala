//! Inline edge-label editor: open / handle key / close. Holds the
//! Session 6D label-editing flow — opens an in-place editor on a
//! selected edge, routes printable / navigation / commit keystrokes
//! through `route_label_edit_key`, and commits or cancels back to the
//! model's `MindEdge.label` on Enter / Esc.

use winit::keyboard::Key;

use baumhard::util::grapheme_chad;

use crate::application::document::MindMapDocument;
use crate::application::renderer::Renderer;

use super::text_edit::insert_caret;
use super::{
    rebuild_all, route_label_edit_key, update_connection_label_tree, update_portal_tree,
};

/// Session 6D: inline-edit state for a connection's label. When
/// `Open`, all keyboard input is routed to the label-edit handler
/// (just like `ConsoleState::Open` captures keys for the console
/// input line). Mutually exclusive with `ConsoleState::Open` — the
/// console check runs first, so opening the console while editing a
/// label is a no-op.
///
/// Mirrors `TextEditState` in shape (buffer + grapheme cursor),
/// per CODE_CONVENTIONS §1: every keystroke routes through
/// `grapheme_chad` so backspace over an emoji removes the whole
/// cluster, not a stray byte. The buffer is threaded into the
/// scene_builder via `MindMapDocument::label_edit_preview`; the
/// connection-label tree's §B2 mutator path (Phase 1.3) picks up
/// the new text + caret without rebuilding the arena.
#[derive(Debug, Clone)]
pub(in crate::application::app) enum LabelEditState {
    Closed,
    Open {
        edge_ref: crate::application::document::EdgeRef,
        /// The in-progress buffer. Committed to
        /// `MindEdge.label` on Enter; discarded on Escape.
        buffer: String,
        /// Cursor position as a grapheme-cluster index into
        /// `buffer`. Valid range
        /// `[0, count_grapheme_clusters(buffer)]`. Stored in
        /// graphemes (not chars or bytes) so backspace over an
        /// emoji or ZWJ cluster removes the whole user-visible
        /// character — same invariant as
        /// `TextEditState::Open::cursor_grapheme_pos`.
        cursor_grapheme_pos: usize,
        /// The edge's label value at the moment edit mode opened.
        /// Used to restore state on Escape so the cancel is clean.
        original: Option<String>,
    },
}

impl LabelEditState {
    pub(in crate::application::app) fn is_open(&self) -> bool {
        matches!(self, LabelEditState::Open { .. })
    }
}

/// Session 6D: transition into inline label edit mode for the given
/// edge. Seeds the buffer from the edge's current label (or the
/// empty string) and installs a preview override on the renderer so
/// the caret shows up immediately. Callers must ensure the edge
/// still exists in `doc.mindmap.edges` — the function silently
/// returns otherwise.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn open_label_edit(
    edge_ref: &crate::application::document::EdgeRef,
    doc: &mut MindMapDocument,
    label_edit_state: &mut LabelEditState,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    let edge = match doc.mindmap.edges.iter().find(|e| edge_ref.matches(e)) {
        Some(e) => e,
        None => return,
    };
    let original = edge.label.clone();
    let buffer = original.clone().unwrap_or_default();
    // Cursor lands at the end of the existing label, matching the
    // `TextEditState` open-on-existing-text behaviour.
    let cursor_grapheme_pos = grapheme_chad::count_grapheme_clusters(&buffer);
    *label_edit_state = LabelEditState::Open {
        edge_ref: edge_ref.clone(),
        buffer: buffer.clone(),
        cursor_grapheme_pos,
        original,
    };
    // Store the preview on the document so every subsequent
    // `doc.build_scene_*` call picks it up automatically — no renderer
    // field, no read-time override, no belt-and-suspenders branch.
    let edge_key = baumhard::mindmap::scene_cache::EdgeKey::new(
        &edge_ref.from_id,
        &edge_ref.to_id,
        &edge_ref.edge_type,
    );
    doc.label_edit_preview = Some((edge_key, insert_caret(&buffer, cursor_grapheme_pos)));
    // Rebuild labels so the caret is visible immediately. The caller
    // already ran `rebuild_all` before this, so the scene is fresh.
    let scene = doc.build_scene_with_selection(renderer.camera_zoom());
    update_connection_label_tree(&scene, app_scene, renderer);
    update_portal_tree(doc, &std::collections::HashMap::new(), app_scene, renderer);
}

/// Session 6D + Phase 2.1: route a keystroke to the inline label
/// editor. Escape discards, Enter commits, navigation keys move the
/// grapheme cursor, Backspace/Delete remove a grapheme cluster
/// (never a stray byte), printable characters insert at the cursor.
///
/// Mirrors [`handle_text_edit_key`] in shape: every text mutation
/// goes through `grapheme_chad` so emoji and ZWJ clusters survive
/// edits intact (CODE_CONVENTIONS §1). Multi-line is intentionally
/// out of scope — labels are short, single-line; Enter commits, not
/// inserts. Cursor navigation is constrained to the one row.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn handle_label_edit_key(
    key_name: &Option<String>,
    logical_key: &Key,
    label_edit_state: &mut LabelEditState,
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    let name = key_name.as_deref();
    if name == Some("escape") {
        close_label_edit(false, doc, label_edit_state, mindmap_tree, app_scene, renderer);
        return;
    }
    if name == Some("enter") {
        close_label_edit(true, doc, label_edit_state, mindmap_tree, app_scene, renderer);
        return;
    }

    let Some((buffer, cursor)) = (match label_edit_state {
        LabelEditState::Open {
            buffer,
            cursor_grapheme_pos,
            ..
        } => Some((buffer, cursor_grapheme_pos)),
        LabelEditState::Closed => None,
    }) else {
        return;
    };

    let typed = match logical_key {
        Key::Character(c) => Some(c.as_str()),
        _ => None,
    };
    if !route_label_edit_key(name, typed, buffer, cursor) {
        return;
    }

    // Refresh the preview on the document so the caret + edited text
    // render on the next frame. The connection-label tree's §B2
    // mutator path (Phase 1.3) picks up the new text without
    // rebuilding the arena because the per-edge identity sequence
    // stays constant during a label edit.
    if let LabelEditState::Open {
        edge_ref,
        buffer,
        cursor_grapheme_pos,
        ..
    } = label_edit_state
    {
        let edge_key = baumhard::mindmap::scene_cache::EdgeKey::new(
            &edge_ref.from_id,
            &edge_ref.to_id,
            &edge_ref.edge_type,
        );
        doc.label_edit_preview = Some((edge_key, insert_caret(buffer, *cursor_grapheme_pos)));
        let scene = doc.build_scene_with_selection(renderer.camera_zoom());
        update_connection_label_tree(&scene, app_scene, renderer);
        update_portal_tree(doc, &std::collections::HashMap::new(), app_scene, renderer);
    }
}

/// Session 6D: close the inline label editor. If `commit` is true,
/// writes the current buffer into the edge's label (via
/// `document.set_edge_label`) and pushes an undo entry. If false,
/// restores the pre-edit label (equivalent to discarding the buffer)
/// — no undo push because we never mutated the model during typing.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn close_label_edit(
    commit: bool,
    doc: &mut MindMapDocument,
    label_edit_state: &mut LabelEditState,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    let (edge_ref, buffer, original) = match std::mem::replace(label_edit_state, LabelEditState::Closed) {
        LabelEditState::Open { edge_ref, buffer, original, .. } => (edge_ref, buffer, original),
        LabelEditState::Closed => return,
    };
    doc.label_edit_preview = None;
    if commit {
        let new_val = if buffer.is_empty() { None } else { Some(buffer) };
        // Only push undo if the committed value actually differs from the
        // pre-edit original — avoids a dead undo entry on unchanged text.
        if new_val != original {
            doc.set_edge_label(&edge_ref, new_val);
        }
    }
    // Rebuild so the label reflects the model state (or vanishes if
    // the buffer was empty + original was None).
    rebuild_all(doc, mindmap_tree, app_scene, renderer);
}
