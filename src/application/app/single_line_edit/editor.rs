// SPDX-License-Identifier: MPL-2.0

//! The single-line editor lifecycle: open, handle one input, close.
//!
//! One buffer, one grapheme cursor, one pre-edit snapshot, bound to
//! whichever [`SingleLineEditTarget`] the caller named. Everything
//! that differs between an edge label and a portal caption lives on
//! the target; nothing in this file knows which one it is holding.
//!
//! The `*_core` half is renderer-free on purpose. It mutates the
//! model, the editor state and the preview slot, then returns an
//! [`EditRefresh`] naming the canvas work it owes; the shells below
//! are the only place that touches `Renderer` / `AppScene`. That
//! split is what lets the whole lifecycle be driven in tests —
//! TEST_CONVENTIONS §T8 keeps live wgpu out of the harness, so a
//! lifecycle that only exists inside a `&mut Renderer` signature
//! could not be tested at all.

use baumhard::mindmap::scene_cache::SceneConnectionCache;
use baumhard::mindmap::tree_builder::MindMapTree;

use crate::application::document::MindMapDocument;
use crate::application::keybinds::{Action, InputContext, ResolvedKeybinds};
use crate::application::platform::input::Key;
use crate::application::renderer::Renderer;
use crate::application::scene_host::AppScene;

use super::super::scene_rebuild::rebuild_all;
use super::super::text_edit::insert_caret;
use super::super::InteractionMode;
use super::target::SingleLineEditTarget;
use super::{route_single_line_key, seed_edit_buffer};

/// What a lifecycle step owes the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::application::app) enum EditRefresh {
    /// Nothing observable changed; skip the canvas work entirely.
    None,
    /// Re-project only the canvas role the live preview feeds. The
    /// rest of the scene cannot have changed, which is what keeps
    /// per-keystroke typing cheap.
    Preview,
    /// Full `rebuild_all`: the model changed, or the editor stopped
    /// existing, so the preview override has to leave every pass.
    All,
}

/// One editing primitive reaching the open buffer.
///
/// The two arms are the two halves of CODE_CONVENTIONS §3's modal
/// carve-out: rebindable cursor / delete verbs arrive as a resolved
/// [`Action`], while character insertion arrives as the literal
/// winit key payload because no `Action` body can carry an IME
/// cluster.
#[derive(Debug)]
pub(in crate::application::app) enum EditInput<'k> {
    /// A resolved `LabelEdit*` cursor / delete action.
    Action(Action),
    /// A literal key payload — printable characters and IME /
    /// dead-key clusters.
    Key(&'k Key),
}

/// Inline single-line editor. When `Open`, the keyboard handler
/// steals every keystroke for it (just like `ConsoleState::Open`
/// captures keys for the console input line), and a pointer release
/// away from the edited element commits it.
///
/// Mutually exclusive with the console and with the node text
/// editor by construction: a node selection opens the text editor,
/// an edge-label or portal selection opens this one, and the steal
/// ladder in `event_keyboard.rs` consumes the key for whichever is
/// open. The edge-label and portal-caption editors were separate,
/// field-for-field identical state machines until this type merged
/// them; `Action::LabelEditCommit` / `LabelEditCancel` always
/// treated them as one thing, and now there is one thing for them
/// to name.
///
/// The cursor is a grapheme-cluster index throughout, so backspace
/// over an emoji or ZWJ cluster removes the whole user-visible
/// character — same invariant as
/// `TextEditState::Open::cursor_grapheme_pos`.
#[derive(Debug, Clone)]
pub(in crate::application::app) enum SingleLineEditor {
    Closed,
    Open {
        /// Which model string the buffer will be written back to.
        target: SingleLineEditTarget,
        /// The in-progress buffer. Committed on Enter or on a
        /// click outside the edited element; discarded on Escape.
        buffer: String,
        /// Cursor position as a grapheme-cluster index into
        /// `buffer`. Valid range
        /// `[0, count_grapheme_clusters(buffer)]`.
        cursor_grapheme_pos: usize,
        /// The target's value at the moment the editor opened.
        /// Restores state on Escape, and lets an unchanged commit
        /// skip the undo push.
        original: Option<String>,
    },
}

impl SingleLineEditor {
    pub(in crate::application::app) fn is_open(&self) -> bool {
        matches!(self, SingleLineEditor::Open { .. })
    }

    /// Borrow the target under edit, if any. Callers use this for
    /// the click-outside-to-commit check and the double-click
    /// re-open guard.
    pub(in crate::application::app) fn target(&self) -> Option<&SingleLineEditTarget> {
        match self {
            SingleLineEditor::Open { target, .. } => Some(target),
            SingleLineEditor::Closed => None,
        }
    }

    /// The `(buffer, cursor)` pair, mutably, while open.
    fn buffer_mut(&mut self) -> Option<(&mut String, &mut usize)> {
        match self {
            SingleLineEditor::Open {
                buffer,
                cursor_grapheme_pos,
                ..
            } => Some((buffer, cursor_grapheme_pos)),
            SingleLineEditor::Closed => None,
        }
    }

    /// Re-stage the preview from the current buffer + cursor.
    fn stage_preview(&self, doc: &mut MindMapDocument) {
        if let SingleLineEditor::Open {
            target,
            buffer,
            cursor_grapheme_pos,
            ..
        } = self
        {
            target.stage_preview(doc, insert_caret(buffer, *cursor_grapheme_pos));
        }
    }

    /// Renderer-free half of [`open_single_line_edit`]. Returns
    /// [`EditRefresh::None`] and leaves the editor closed when the
    /// target no longer resolves (`target.find` logs the reason).
    fn open_core(
        &mut self,
        target: SingleLineEditTarget,
        clean: bool,
        doc: &mut MindMapDocument,
    ) -> EditRefresh {
        let Some(original) = target.find(doc) else {
            return EditRefresh::None;
        };
        // Re-opening over a live editor on a *different* target
        // would otherwise strand that target's preview slot, since
        // each target only ever clears its own. Unreachable today
        // (the steal ladder and the double-click guard both stop
        // short of it) and cheap to keep correct.
        if let Some(previous) = self.target() {
            if previous != &target {
                previous.clear_preview(doc);
            }
        }
        let (buffer, cursor_grapheme_pos) = seed_edit_buffer(clean, original.as_deref());
        *self = SingleLineEditor::Open {
            target,
            buffer,
            cursor_grapheme_pos,
            original,
        };
        self.stage_preview(doc);
        EditRefresh::Preview
    }

    /// Renderer-free half of [`handle_single_line_edit_key`],
    /// including the mid-edit validity guard.
    fn handle_input_core(&mut self, input: EditInput<'_>, doc: &mut MindMapDocument) -> EditRefresh {
        let Some(target) = self.target() else {
            return EditRefresh::None;
        };
        // A target that stopped being editable under the open
        // editor closes it without committing — see
        // `SingleLineEditTarget::still_editable` for why the two
        // kinds answer this differently.
        if !target.still_editable(doc) {
            return self.close_core(false, doc);
        }

        let Some((buffer, cursor)) = self.buffer_mut() else {
            return EditRefresh::None;
        };
        let changed = match input {
            EditInput::Action(a) => {
                crate::application::app::dispatch::apply_label_edit_action_to_buffer(a, buffer, cursor)
            }
            EditInput::Key(logical_key) => route_single_line_key(logical_key, buffer, cursor),
        };
        if !changed {
            return EditRefresh::None;
        }
        // The target's tree keeps its per-element identity sequence
        // during an edit, so the in-place mutator path picks up the
        // new text + caret without rebuilding the arena.
        self.stage_preview(doc);
        EditRefresh::Preview
    }

    /// Renderer-free half of [`close_single_line_edit`].
    fn close_core(&mut self, commit: bool, doc: &mut MindMapDocument) -> EditRefresh {
        let SingleLineEditor::Open {
            target,
            buffer,
            original,
            ..
        } = std::mem::replace(self, SingleLineEditor::Closed)
        else {
            return EditRefresh::None;
        };
        target.clear_preview(doc);
        if commit {
            let new_val = if buffer.is_empty() { None } else { Some(buffer) };
            // Only push undo if the committed value actually differs
            // from the pre-edit original — avoids a dead undo entry
            // on unchanged text.
            if new_val != original {
                target.commit(doc, new_val);
            }
        }
        EditRefresh::All
    }
}

/// Run the canvas work a lifecycle step asked for.
#[allow(clippy::too_many_arguments)]
fn apply_refresh(
    refresh: EditRefresh,
    state: &SingleLineEditor,
    doc: &mut MindMapDocument,
    interaction_mode: &InteractionMode,
    mindmap_tree: &mut Option<MindMapTree>,
    app_scene: &mut AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut SceneConnectionCache,
) {
    match refresh {
        EditRefresh::None => {}
        EditRefresh::Preview => {
            if let Some(target) = state.target() {
                target.refresh(doc, app_scene, renderer);
            }
        }
        EditRefresh::All => {
            rebuild_all(
                doc,
                interaction_mode,
                mindmap_tree,
                app_scene,
                renderer,
                scene_cache,
            );
        }
    }
}

/// Open the single-line editor on `target`, seeding the buffer from
/// the target's current text and installing a preview override so
/// the caret shows up immediately.
///
/// If the target isn't in the model any more (e.g. an undo popped
/// the edge between selection and the open gesture), logs and
/// returns without mutating state — callers read `state.is_open()`
/// to detect that case; they don't need a return value to
/// disambiguate.
///
/// `clean` opens on an empty buffer instead of the existing text —
/// `Action::EditSelectionClean`'s documented contract, and the
/// single-line counterpart of `open_text_edit`'s `from_creation`.
/// `original` still holds the pre-edit text either way, so Esc
/// restores it and an unchanged commit still skips the undo push.
///
/// Only the preview canvas is re-projected here; the caller has
/// already run `rebuild_all`, so the rest of the scene is fresh.
pub(in crate::application::app) fn open_single_line_edit(
    target: SingleLineEditTarget,
    clean: bool,
    doc: &mut MindMapDocument,
    state: &mut SingleLineEditor,
    app_scene: &mut AppScene,
    renderer: &mut Renderer,
) {
    if state.open_core(target, clean, doc) == EditRefresh::Preview {
        if let Some(target) = state.target() {
            target.refresh(doc, app_scene, renderer);
        }
    }
}

/// Route one keystroke to the open single-line editor.
///
/// Commit and cancel never reach here — the keyboard handler
/// pre-filters `Action::LabelEditCommit` / `LabelEditCancel`
/// through the dispatch funnel first. What is left is the modal
/// carve-out: rebindable cursor / delete primitives resolved
/// through `InputContext::LabelEdit`, and literal character
/// payloads the `Action` vocabulary cannot express.
///
/// `InputContext::LabelEdit` is shared by both target kinds — the
/// two editors always had identical key semantics, and the
/// `LabelEdit*` spelling is the stable `keybinds.json` name.
#[allow(clippy::too_many_arguments)]
pub(in crate::application::app) fn handle_single_line_edit_key(
    key_name: &Option<String>,
    logical_key: &Key,
    ctrl: bool,
    shift: bool,
    alt: bool,
    keybinds: &ResolvedKeybinds,
    state: &mut SingleLineEditor,
    doc: &mut MindMapDocument,
    interaction_mode: &InteractionMode,
    mindmap_tree: &mut Option<MindMapTree>,
    app_scene: &mut AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut SceneConnectionCache,
) {
    let action = key_name
        .as_deref()
        .and_then(|n| keybinds.action_for_context(InputContext::LabelEdit, n, ctrl, shift, alt));
    let input = match action {
        Some(a) => EditInput::Action(a),
        None => EditInput::Key(logical_key),
    };
    let refresh = state.handle_input_core(input, doc);
    apply_refresh(
        refresh,
        state,
        doc,
        interaction_mode,
        mindmap_tree,
        app_scene,
        renderer,
        scene_cache,
    );
}

/// Apply a `LabelEdit*` cursor / delete primitive that arrived
/// through the dispatch funnel rather than through a keystroke —
/// a macro step, a console-driven `Action`, or IPC.
///
/// Same body as the keyboard path, refresh included. The funnel arm
/// used to call a buffer mutator with no refresh at all, so a
/// macro-driven cursor move left the caret painted at its old
/// position until the next unrelated redraw.
#[allow(clippy::too_many_arguments)]
pub(in crate::application::app) fn apply_single_line_edit_action(
    action: Action,
    state: &mut SingleLineEditor,
    doc: &mut MindMapDocument,
    interaction_mode: &InteractionMode,
    mindmap_tree: &mut Option<MindMapTree>,
    app_scene: &mut AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut SceneConnectionCache,
) {
    let refresh = state.handle_input_core(EditInput::Action(action), doc);
    apply_refresh(
        refresh,
        state,
        doc,
        interaction_mode,
        mindmap_tree,
        app_scene,
        renderer,
        scene_cache,
    );
}

/// Close the single-line editor. On commit, writes the buffer back
/// through the target's setter (pushing one undo entry) when the
/// value actually differs from the pre-edit original. On cancel,
/// discards the buffer — no undo push, because typing never touched
/// the model.
///
/// Rebuilds so the element reflects the model state (or vanishes,
/// if the buffer was empty and there was nothing there before).
#[allow(clippy::too_many_arguments)]
pub(in crate::application::app) fn close_single_line_edit(
    commit: bool,
    doc: &mut MindMapDocument,
    interaction_mode: &InteractionMode,
    state: &mut SingleLineEditor,
    mindmap_tree: &mut Option<MindMapTree>,
    app_scene: &mut AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut SceneConnectionCache,
) {
    let refresh = state.close_core(commit, doc);
    apply_refresh(
        refresh,
        state,
        doc,
        interaction_mode,
        mindmap_tree,
        app_scene,
        renderer,
        scene_cache,
    );
}

#[cfg(test)]
impl SingleLineEditor {
    /// Test-only view of the open buffer + cursor.
    pub(super) fn buffer_and_cursor(&self) -> Option<(&str, usize)> {
        match self {
            SingleLineEditor::Open {
                buffer,
                cursor_grapheme_pos,
                ..
            } => Some((buffer.as_str(), *cursor_grapheme_pos)),
            SingleLineEditor::Closed => None,
        }
    }

    /// Test-only driver for the renderer-free open step.
    pub(super) fn open_for_test(
        &mut self,
        target: SingleLineEditTarget,
        clean: bool,
        doc: &mut MindMapDocument,
    ) -> EditRefresh {
        self.open_core(target, clean, doc)
    }

    /// Test-only driver for the renderer-free input step.
    pub(super) fn handle_input_for_test(
        &mut self,
        input: EditInput<'_>,
        doc: &mut MindMapDocument,
    ) -> EditRefresh {
        self.handle_input_core(input, doc)
    }

    /// Test-only driver for the renderer-free close step.
    pub(super) fn close_for_test(&mut self, commit: bool, doc: &mut MindMapDocument) -> EditRefresh {
        self.close_core(commit, doc)
    }
}
