//! Inline edge-label editor: open / handle key / close. Holds the
//! Session 6D label-editing flow — opens an in-place editor on a
//! selected edge, routes printable / navigation / commit keystrokes
//! through `route_label_edit_key`, and commits or cancels back to the
//! model's `MindEdge.label` on Enter / Esc.

use winit::keyboard::Key;

use baumhard::util::grapheme_chad;

use crate::application::document::MindMapDocument;
use crate::application::keybinds::{Action, InputContext, ResolvedKeybinds};
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

/// Route a keystroke to the inline label editor. Cancel and commit
/// are resolved through `action_for_context(InputContext::LabelEdit)`;
/// navigation and character input stay as direct key checks (they
/// are structural text-editing primitives, not rebindable actions).
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn handle_label_edit_key(
    key_name: &Option<String>,
    logical_key: &Key,
    ctrl: bool,
    shift: bool,
    alt: bool,
    keybinds: &ResolvedKeybinds,
    label_edit_state: &mut LabelEditState,
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    let name = key_name.as_deref();
    let action = name.and_then(|n| {
        keybinds.action_for_context(InputContext::LabelEdit, n, ctrl, shift, alt)
    });
    if action == Some(Action::LabelEditCancel) {
        close_label_edit(false, doc, label_edit_state, mindmap_tree, app_scene, renderer);
        return;
    }
    if action == Some(Action::LabelEditCommit) {
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

/// Inline-edit state for a portal label's text. Parallel to
/// [`LabelEditState`] but keyed to `(edge_ref, endpoint_node_id)`
/// — portal labels are per-endpoint, so the editor needs both
/// parts of the identity. Routes keystrokes through
/// `route_label_edit_key` just like the edge-label editor so
/// grapheme semantics (emoji / ZWJ backspace, arrow-key walking)
/// behave identically.
///
/// Mutually exclusive with `LabelEditState`: the event loop's
/// keystroke routing checks the portal editor first (rarer form
/// so the short-circuit is usually cheap), then falls through
/// to the edge-label editor.
#[derive(Debug, Clone)]
pub(in crate::application::app) enum PortalTextEditState {
    Closed,
    Open {
        edge_ref: crate::application::document::EdgeRef,
        endpoint_node_id: String,
        /// The in-progress buffer. Committed to
        /// `PortalEndpointState.text` on Enter; discarded on
        /// Escape.
        buffer: String,
        /// Cursor position as a grapheme-cluster index into
        /// `buffer`. Same invariant as `LabelEditState::Open`.
        cursor_grapheme_pos: usize,
        /// The endpoint's text value at the moment edit mode
        /// opened. Used to restore state on Escape and to skip
        /// undo entries on unchanged commit.
        original: Option<String>,
    },
}

impl PortalTextEditState {
    pub(in crate::application::app) fn is_open(&self) -> bool {
        matches!(self, PortalTextEditState::Open { .. })
    }
}

/// Transition into inline portal-text edit mode for the given
/// endpoint. Seeds the buffer from the endpoint's current text,
/// installs a preview override on the document so the caret
/// shows up immediately, and runs a portal-tree update so the
/// caret renders on the next frame.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn open_portal_text_edit(
    edge_ref: &crate::application::document::EdgeRef,
    endpoint_node_id: &str,
    doc: &mut MindMapDocument,
    state: &mut PortalTextEditState,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    // Verify the edge + endpoint still exist before entering
    // edit mode. If either vanished between the selection and
    // the open gesture (e.g. an undo raced with EditSelection),
    // silently return rather than install a stale editor.
    let edge = match doc.mindmap.edges.iter().find(|e| edge_ref.matches(e)) {
        Some(e) => e,
        None => return,
    };
    if endpoint_node_id != edge.from_id && endpoint_node_id != edge.to_id {
        return;
    }
    let original =
        baumhard::mindmap::model::portal_endpoint_state(edge, endpoint_node_id)
            .and_then(|s| s.text.clone());
    let buffer = original.clone().unwrap_or_default();
    let cursor_grapheme_pos = grapheme_chad::count_grapheme_clusters(&buffer);
    *state = PortalTextEditState::Open {
        edge_ref: edge_ref.clone(),
        endpoint_node_id: endpoint_node_id.to_string(),
        buffer: buffer.clone(),
        cursor_grapheme_pos,
        original,
    };
    let edge_key = baumhard::mindmap::scene_cache::EdgeKey::new(
        &edge_ref.from_id,
        &edge_ref.to_id,
        &edge_ref.edge_type,
    );
    doc.portal_text_edit_preview = Some((
        edge_key,
        endpoint_node_id.to_string(),
        insert_caret(&buffer, cursor_grapheme_pos),
    ));
    update_portal_tree(doc, &std::collections::HashMap::new(), app_scene, renderer);
}

/// Route a keystroke to the inline portal-text editor. Mirrors
/// `handle_label_edit_key` — commit / cancel resolve through
/// `InputContext::LabelEdit` (shared with the edge-label editor
/// since the two editors use the same key semantics) and
/// navigation / character input go through the shared
/// `route_label_edit_key` router.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn handle_portal_text_edit_key(
    key_name: &Option<String>,
    logical_key: &Key,
    ctrl: bool,
    shift: bool,
    alt: bool,
    keybinds: &ResolvedKeybinds,
    state: &mut PortalTextEditState,
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    let name = key_name.as_deref();
    let action = name.and_then(|n| {
        keybinds.action_for_context(InputContext::LabelEdit, n, ctrl, shift, alt)
    });
    if action == Some(Action::LabelEditCancel) {
        close_portal_text_edit(false, doc, state, mindmap_tree, app_scene, renderer);
        return;
    }
    if action == Some(Action::LabelEditCommit) {
        close_portal_text_edit(true, doc, state, mindmap_tree, app_scene, renderer);
        return;
    }

    let Some((buffer, cursor)) = (match state {
        PortalTextEditState::Open {
            buffer,
            cursor_grapheme_pos,
            ..
        } => Some((buffer, cursor_grapheme_pos)),
        PortalTextEditState::Closed => None,
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

    if let PortalTextEditState::Open {
        edge_ref,
        endpoint_node_id,
        buffer,
        cursor_grapheme_pos,
        ..
    } = state
    {
        let edge_key = baumhard::mindmap::scene_cache::EdgeKey::new(
            &edge_ref.from_id,
            &edge_ref.to_id,
            &edge_ref.edge_type,
        );
        doc.portal_text_edit_preview = Some((
            edge_key,
            endpoint_node_id.clone(),
            insert_caret(buffer, *cursor_grapheme_pos),
        ));
        update_portal_tree(doc, &std::collections::HashMap::new(), app_scene, renderer);
    }
}

/// Close the inline portal-text editor. If `commit` is true,
/// writes the buffer to `PortalEndpointState.text` (with undo
/// entry) when the value actually differs from the pre-edit
/// original. Restores the pre-edit state on cancel.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) fn close_portal_text_edit(
    commit: bool,
    doc: &mut MindMapDocument,
    state: &mut PortalTextEditState,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
) {
    let (edge_ref, endpoint_node_id, buffer, original) =
        match std::mem::replace(state, PortalTextEditState::Closed) {
            PortalTextEditState::Open {
                edge_ref,
                endpoint_node_id,
                buffer,
                original,
                ..
            } => (edge_ref, endpoint_node_id, buffer, original),
            PortalTextEditState::Closed => return,
        };
    doc.portal_text_edit_preview = None;
    if commit {
        let new_val = if buffer.is_empty() { None } else { Some(buffer) };
        if new_val != original {
            doc.set_portal_label_text(&edge_ref, &endpoint_node_id, new_val);
        }
    }
    rebuild_all(doc, mindmap_tree, app_scene, renderer);
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    //! Label-edit key-routing tests — backspace / delete / arrow /
    //! home / end / printable-char behaviour for
    //! [`super::route_label_edit_key`], the pure keyboard router
    //! both the label editor and its text-edit sibling consume.
    //! No winit event loop needed; the router is a pure function.

    use super::*;

    #[test]
    fn test_route_label_edit_backspace_deletes_grapheme_before_cursor() {
        let mut buf = String::from("café");
        // 4 graphemes: c a f é. Cursor at end; backspace removes é.
        let mut cursor = 4;
        let changed = route_label_edit_key(Some("backspace"), None, &mut buf, &mut cursor);
        assert!(changed);
        assert_eq!(buf, "caf");
        assert_eq!(cursor, 3);
    }

    #[test]
    fn test_route_label_edit_backspace_at_zero_is_noop() {
        let mut buf = String::from("abc");
        let mut cursor = 0;
        let changed = route_label_edit_key(Some("backspace"), None, &mut buf, &mut cursor);
        assert!(!changed);
        assert_eq!(buf, "abc");
        assert_eq!(cursor, 0);
    }

    #[test]
    fn test_route_label_edit_delete_at_end_is_noop() {
        let mut buf = String::from("abc");
        let mut cursor = 3;
        let changed = route_label_edit_key(Some("delete"), None, &mut buf, &mut cursor);
        assert!(!changed);
        assert_eq!(buf, "abc");
        assert_eq!(cursor, 3);
    }

    #[test]
    fn test_route_label_edit_delete_removes_grapheme_at_cursor() {
        let mut buf = String::from("abc");
        let mut cursor = 1;
        let changed = route_label_edit_key(Some("delete"), None, &mut buf, &mut cursor);
        assert!(changed);
        assert_eq!(buf, "ac");
        assert_eq!(cursor, 1);
    }

    #[test]
    fn test_route_label_edit_arrow_left_right_walks_graphemes() {
        let mut buf = String::from("café");
        let mut cursor = 4;
        // Left past é, f, a — landing on the c boundary.
        assert!(route_label_edit_key(Some("arrowleft"), None, &mut buf, &mut cursor));
        assert_eq!(cursor, 3);
        assert!(route_label_edit_key(Some("arrowleft"), None, &mut buf, &mut cursor));
        assert_eq!(cursor, 2);
        // Right brings us back.
        assert!(route_label_edit_key(Some("arrowright"), None, &mut buf, &mut cursor));
        assert_eq!(cursor, 3);
    }

    #[test]
    fn test_route_label_edit_arrow_left_at_zero_is_noop() {
        let mut buf = String::from("abc");
        let mut cursor = 0;
        assert!(!route_label_edit_key(Some("arrowleft"), None, &mut buf, &mut cursor));
        assert_eq!(cursor, 0);
    }

    #[test]
    fn test_route_label_edit_home_end_jump_to_ends() {
        let mut buf = String::from("café");
        let mut cursor = 2;
        assert!(route_label_edit_key(Some("home"), None, &mut buf, &mut cursor));
        assert_eq!(cursor, 0);
        // Home again is a no-op.
        assert!(!route_label_edit_key(Some("home"), None, &mut buf, &mut cursor));
        assert_eq!(cursor, 0);
        assert!(route_label_edit_key(Some("end"), None, &mut buf, &mut cursor));
        assert_eq!(cursor, 4);
        // End again is a no-op.
        assert!(!route_label_edit_key(Some("end"), None, &mut buf, &mut cursor));
        assert_eq!(cursor, 4);
    }

    #[test]
    fn test_route_label_edit_printable_inserts_and_advances() {
        let mut buf = String::from("ab");
        let mut cursor = 1;
        let changed = route_label_edit_key(None, Some("X"), &mut buf, &mut cursor);
        assert!(changed);
        assert_eq!(buf, "aXb");
        assert_eq!(cursor, 2);
    }

    /// IME / dead-key sequences can arrive as multi-char strings.
    /// Each non-control char inserts in order and the cursor
    /// advances past them.
    #[test]
    fn test_route_label_edit_multichar_typed_payload() {
        let mut buf = String::from("");
        let mut cursor = 0;
        let changed = route_label_edit_key(None, Some("né"), &mut buf, &mut cursor);
        assert!(changed);
        assert_eq!(buf, "né");
        assert_eq!(cursor, 2);
    }

    /// Control characters in a typed payload are filtered out.
    /// Pins the regression where an IME sequence like `"a\t"`
    /// would otherwise insert a literal tab.
    #[test]
    fn test_route_label_edit_typed_control_chars_are_skipped() {
        let mut buf = String::from("");
        let mut cursor = 0;
        let changed = route_label_edit_key(None, Some("a\tb"), &mut buf, &mut cursor);
        assert!(changed);
        assert_eq!(buf, "ab");
        assert_eq!(cursor, 2);
    }
}

