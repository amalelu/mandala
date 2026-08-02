// SPDX-License-Identifier: MPL-2.0

//! The inline text-editor ladder.
//!
//! Two modal editors can own the keyboard and the pointer: the
//! multi-line node text editor ([`super::text_edit`]) and the
//! single-line editor ([`super::single_line_edit`]) bound to an edge
//! label or a portal caption. They are different enough to keep
//! apart — one previews by mutating the live tree and hit-tests a
//! node AABB, the other stages a string and hit-tests a glyph role
//! — but the *ladder* around them is the same twice over:
//!
//! - **Steal**: while open, resolve the key in the editor's input
//!   context; if it is that editor's commit or cancel, dispatch it
//!   through the funnel; otherwise hand the raw key to the editor's
//!   own handler. Either way the key is consumed.
//! - **Release**: a pointer release inside the edited element keeps
//!   editing and consumes the release; a release outside commits
//!   through the funnel and lets the click route normally.
//!
//! [`ModalEditor`] is that ladder, written once.

#![cfg(not(target_arch = "wasm32"))]

use crate::application::keybinds::{Action, InputContext};
use crate::application::platform::input::Key;

use super::input_context::InputHandlerContext;

/// One inline text editor, as the ladder sees it.
///
/// Ordering is load-bearing where the variants are enumerated:
/// [`Self::LADDER`] lists the node text editor first, matching the
/// order the release path has always checked them in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ModalEditor {
    /// The multi-line node text editor.
    Text,
    /// The single-line edge-label / portal-caption editor.
    SingleLine,
}

impl ModalEditor {
    /// Every modal editor, in check order.
    const LADDER: [ModalEditor; 2] = [ModalEditor::Text, ModalEditor::SingleLine];

    /// Is this editor currently open?
    fn is_open(self, ctx: &InputHandlerContext<'_>) -> bool {
        match self {
            ModalEditor::Text => ctx.text_edit_state.is_open(),
            ModalEditor::SingleLine => ctx.single_line_edit_state.is_open(),
        }
    }

    /// The keybind context this editor's keys resolve in.
    fn context(self) -> InputContext {
        match self {
            ModalEditor::Text => InputContext::TextEdit,
            ModalEditor::SingleLine => InputContext::LabelEdit,
        }
    }

    /// The `Action` a click outside the edited element dispatches.
    fn commit_action(self) -> Action {
        match self {
            ModalEditor::Text => Action::TextEditCommit,
            ModalEditor::SingleLine => Action::LabelEditCommit,
        }
    }

    /// Does this editor own `action` as its commit / cancel pair?
    /// Those two are user-named effects, so they belong in the
    /// funnel rather than in the modal handler (CODE_CONVENTIONS
    /// §3); everything else the editor claims is the literal-Key
    /// carve-out.
    fn owns_commit_or_cancel(self, action: &Action) -> bool {
        match self {
            ModalEditor::Text => matches!(action, Action::TextEditCommit | Action::TextEditCancel),
            ModalEditor::SingleLine => {
                matches!(action, Action::LabelEditCommit | Action::LabelEditCancel)
            }
        }
    }

    /// The editor currently stealing keyboard input, if any.
    pub(super) fn stealing(ctx: &InputHandlerContext<'_>) -> Option<ModalEditor> {
        ModalEditor::stealing_from(
            ModalEditor::SingleLine.is_open(ctx),
            ModalEditor::Text.is_open(ctx),
        )
    }

    /// Pure half of [`Self::stealing`], over plain open-flags so the
    /// steal *order* is pinnable without an event loop.
    ///
    /// The single-line editor is checked first, matching the order
    /// the keyboard handler has always used; the two are mutually
    /// exclusive by construction anyway (a node selection opens the
    /// text editor, an edge-label / portal selection opens the
    /// single-line one).
    fn stealing_from(single_line_open: bool, text_open: bool) -> Option<ModalEditor> {
        if single_line_open {
            Some(ModalEditor::SingleLine)
        } else if text_open {
            Some(ModalEditor::Text)
        } else {
            None
        }
    }
}

/// Hand a keystroke to the modal editor that is stealing input.
///
/// The commit / cancel pre-filter runs *before* the modal handler:
/// `dispatch_action`'s arm body owns the close-and-rebuild path, and
/// routing through it is what makes those two verbs reachable from
/// macros, the console and IPC as well as from the keyboard. The
/// modal handler keeps only what `Action` cannot carry — character
/// insertion and IME sequences — plus the rebindable cursor / delete
/// primitives, which it re-resolves in the same context.
pub(super) fn steal_key_for_modal(
    modal: ModalEditor,
    key_name: &Option<String>,
    logical_key: &Key,
    ctx: &mut InputHandlerContext<'_>,
) {
    let action = key_name.as_deref().and_then(|n| {
        ctx.keybinds.action_for_context(
            modal.context(),
            n,
            ctx.modifiers.control_key(),
            ctx.modifiers.shift_key(),
            ctx.modifiers.alt_key(),
        )
    });
    if let Some(modal_action) = action.filter(|a| modal.owns_commit_or_cancel(a)) {
        let _ = super::dispatch::dispatch_action(modal_action, ctx, None);
        return;
    }
    let (ctrl, shift, alt) = (
        ctx.modifiers.control_key(),
        ctx.modifiers.shift_key(),
        ctx.modifiers.alt_key(),
    );
    let Some(doc) = ctx.document.as_mut() else {
        return;
    };
    match modal {
        ModalEditor::Text => super::text_edit::handle_text_edit_key(
            key_name,
            logical_key,
            ctrl,
            shift,
            alt,
            ctx.keybinds,
            ctx.text_edit_state,
            doc,
            ctx.mindmap_tree,
            ctx.app_scene,
            ctx.renderer,
            ctx.scene_cache,
        ),
        ModalEditor::SingleLine => super::single_line_edit::handle_single_line_edit_key(
            key_name,
            logical_key,
            ctrl,
            shift,
            alt,
            ctx.keybinds,
            ctx.single_line_edit_state,
            doc,
            ctx.interaction_mode,
            ctx.mindmap_tree,
            ctx.app_scene,
            ctx.renderer,
            ctx.scene_cache,
        ),
    }
}

/// Resolve a pointer release against every open modal editor.
///
/// Returns `true` when the release landed *inside* an edited
/// element: the editor keeps its buffer and the caller must return
/// without routing the click, because routing it would change the
/// selection out from under a live edit.
///
/// Otherwise every open editor commits through the funnel and the
/// caller falls through to the normal click path, so the new
/// selection lands on whatever was clicked. Walking
/// [`ModalEditor::LADDER`] rather than resolving a single active
/// editor keeps the pathological both-open case behaving exactly as
/// the three hand-written blocks this replaced did.
pub(super) fn commit_modal_editors_on_release(ctx: &mut InputHandlerContext<'_>) -> bool {
    for modal in ModalEditor::LADDER {
        if !modal.is_open(ctx) {
            continue;
        }
        if release_stays_inside(modal, ctx) {
            return true;
        }
        let _ = super::dispatch::dispatch_action(modal.commit_action(), ctx, None);
    }
    false
}

/// Did the release land on the element `modal` is editing?
fn release_stays_inside(modal: ModalEditor, ctx: &mut InputHandlerContext<'_>) -> bool {
    let release_canvas = ctx
        .renderer
        .screen_to_canvas(ctx.cursor_pos.0 as f32, ctx.cursor_pos.1 as f32);
    match modal {
        // Cross-platform: the WASM release path runs the same
        // predicate, so the AABB-refresh-then-contain dance exists
        // once.
        ModalEditor::Text => super::text_edit::release_stays_inside_edited_node(
            ctx.text_edit_state,
            ctx.mindmap_tree,
            release_canvas,
        ),
        ModalEditor::SingleLine => {
            // Cloned so the `&mut AppScene` hit-test can borrow
            // `ctx` freely; the target is two or three short
            // strings.
            let target = ctx.single_line_edit_state.target().cloned();
            target
                .map(|t| t.release_stays_on_target(ctx.app_scene, release_canvas))
                .unwrap_or(false)
        }
    }
}

#[cfg(test)]
mod tests {
    //! Entry-point contracts for the modal ladder.
    //!
    //! Unit tests on the editors themselves cannot catch a caller
    //! that resolves the modals in a different order or dispatches
    //! one editor's commit while stealing in the other's context, so
    //! the ordering and the per-modal wiring are pinned here rather
    //! than only inside the editors.

    use super::*;

    /// Steal order: the single-line editor wins. The two are
    /// mutually exclusive in practice, but the resolution order is
    /// what decides which handler a keystroke reaches if that ever
    /// stops holding, and it must stay the order the three
    /// hand-written blocks used.
    #[test]
    fn test_steal_order_prefers_the_single_line_editor() {
        assert_eq!(
            ModalEditor::stealing_from(true, true),
            Some(ModalEditor::SingleLine)
        );
        assert_eq!(
            ModalEditor::stealing_from(true, false),
            Some(ModalEditor::SingleLine)
        );
        assert_eq!(ModalEditor::stealing_from(false, true), Some(ModalEditor::Text));
        assert_eq!(ModalEditor::stealing_from(false, false), None);
    }

    /// Release order: the node text editor is resolved first, which
    /// is the order the release path used before the ladder existed.
    /// The exhaustive `tag` match makes a new `ModalEditor` variant a
    /// build error here, and this assertion then fails until the
    /// variant is added to `LADDER` — so a new modal editor cannot
    /// silently skip the click-outside commit.
    #[test]
    fn test_release_ladder_lists_every_variant_text_first() {
        fn tag(m: ModalEditor) -> u8 {
            match m {
                ModalEditor::Text => 0,
                ModalEditor::SingleLine => 1,
            }
        }
        let tags: Vec<u8> = ModalEditor::LADDER.iter().copied().map(tag).collect();
        assert_eq!(tags, vec![0, 1]);
    }

    /// Each modal resolves keys in its own context. Sharing one
    /// would make the other editor's bindings fire while this one is
    /// open.
    #[test]
    fn test_each_modal_steals_in_its_own_context() {
        assert_eq!(ModalEditor::Text.context(), InputContext::TextEdit);
        assert_eq!(ModalEditor::SingleLine.context(), InputContext::LabelEdit);
    }

    /// The action the release path dispatches must be one the steal
    /// path recognizes as that same modal's commit. If the two ever
    /// disagree, a click-outside would commit through an arm the
    /// keyboard would not route to — the cross-wiring this pins.
    #[test]
    fn test_commit_action_is_owned_by_its_own_modal() {
        for modal in ModalEditor::LADDER {
            let commit = modal.commit_action();
            assert!(
                modal.owns_commit_or_cancel(&commit),
                "{:?} dispatches {:?} on click-outside but does not claim it",
                modal,
                commit
            );
        }
    }

    /// Neither modal claims the other's commit / cancel pair, and
    /// neither claims an unrelated action. Without this, a keystroke
    /// bound in one editor's context could funnel the other editor's
    /// close.
    #[test]
    fn test_modals_do_not_claim_each_others_commit_or_cancel() {
        let text_pair = [Action::TextEditCommit, Action::TextEditCancel];
        let single_pair = [Action::LabelEditCommit, Action::LabelEditCancel];
        for a in &text_pair {
            assert!(ModalEditor::Text.owns_commit_or_cancel(a));
            assert!(!ModalEditor::SingleLine.owns_commit_or_cancel(a));
        }
        for a in &single_pair {
            assert!(ModalEditor::SingleLine.owns_commit_or_cancel(a));
            assert!(!ModalEditor::Text.owns_commit_or_cancel(a));
        }
        // A cursor primitive stays with the modal handler — it is
        // rebindable and must not be swallowed by the funnel
        // pre-filter.
        for modal in ModalEditor::LADDER {
            assert!(!modal.owns_commit_or_cancel(&Action::LabelEditCursorLeft));
            assert!(!modal.owns_commit_or_cancel(&Action::Undo));
        }
    }
}
