// SPDX-License-Identifier: MPL-2.0

//! The release half of a throttled drag's lifecycle: what each
//! gesture writes to the model when the button comes up, and what
//! it owes the canvas afterwards.
//!
//! Every release body does the same four things in the same order —
//! flush whatever the throttle left pending, commit the gesture to
//! the model (with its undo entry), clear the scene cache when the
//! per-frame drains left stale samples in it, and rebuild. Only the
//! first two are gesture-specific; the last two are a decree the
//! body *names* rather than performs, so the whole commit stays
//! renderer-free and therefore testable (TEST_CONVENTIONS §T8 keeps
//! live wgpu out of the harness). [`ReleaseRefresh::execute`] is the
//! shell that runs the decree.

#![cfg(not(target_arch = "wasm32"))]

use baumhard::mindmap::scene_cache::SceneConnectionCache;
use baumhard::mindmap::tree_builder::MindMapTree;

use crate::application::document::MindMapDocument;
use crate::application::platform::input::MouseButton;

use super::super::scene_rebuild::{rebuild_all, rebuild_scene_only};
use super::{DrainContext, ThrottledDrag};

/// The renderer-free half of a release-commit's context: everything
/// a commit body reads or mutates before it names the canvas work
/// it owes.
///
/// `document` is the `Option` rather than a resolved `&mut
/// MindMapDocument` on purpose: two of the seven gestures do
/// tree-side or cursor-side work *before* they check whether a
/// document is loaded, and resolving the option in the shell would
/// silently reorder that.
pub(in crate::application::app) struct ReleaseCommit<'a> {
    pub document: &'a mut Option<MindMapDocument>,
    pub mindmap_tree: &'a mut Option<MindMapTree>,
    pub scene_cache: &'a mut SceneConnectionCache,
}

/// The canvas work a release commit owes once the model write has
/// landed.
///
/// Named rather than performed so [`ReleaseCommit`] can stay free of
/// `&mut Renderer`. The variants are ordered by how much they
/// repaint; the ordering carries no meaning beyond readability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::application::app) enum ReleaseRefresh {
    /// Nothing to repaint. Reached only when no document is loaded,
    /// in which case there is nothing on screen to repaint — but a
    /// commit may still have written the tree on its way here:
    /// `MovingNode` flushes its pending delta into `mindmap_tree`
    /// *before* it checks for a document, and the oracle records
    /// that as deliberate.
    None,
    /// Scene-only rebuild. The gesture moved something that lives
    /// purely in the scene projection (an edge label), so the node
    /// trees are untouched and a full rebuild would be a wasted
    /// walk.
    SceneOnly,
    /// Full rebuild from the authoritative model. Also the
    /// snap-back path: a rejected commit leaves the model unchanged
    /// and the rebuild pulls the on-screen state back to it.
    All,
}

/// What a pointer release does with the throttled drag it found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::application::app) enum ThrottledRelease {
    /// Run the gesture's commit and let the drag state stay `None`.
    Commit,
    /// Put the drag state back untouched — this button is not the
    /// one that started the gesture, and terminating it here would
    /// end a drag the user is still performing.
    PutBack,
}

/// Resolve a release against the button that produced it.
///
/// Pure, and separated from the two dispatchers for exactly the
/// reason PR #91 in this epic needed: unit tests on the interactions
/// cannot see a caller that finalizes the wrong gestures, and both
/// dispatchers need a live renderer, which TEST_CONVENTIONS §T8
/// keeps out of the harness.
///
/// **The button that started the gesture is the one that finalizes
/// it.** A release from any other button hands the drag back
/// untouched, because ending someone else's gesture mid-flight is
/// what #37 item 5 is about: the user is still holding the button
/// that owns the drag, and the commit lands wherever the pointer
/// happened to be when the stray release arrived.
///
/// The left half of that rule is newly load-bearing. It used to read
/// "the left button finalizes unconditionally … a right-started
/// gesture is unreachable here (a left press during an in-flight
/// drag replaces the drag state with `Pending` rather than reaching
/// the release path)", and that parenthetical stopped being true the
/// moment `DragState::would_abandon_gesture` started refusing the
/// left press instead. Refusing the press is what makes the state
/// *survive* to the left release: right press → threshold →
/// `Throttled(NodeResize { started_with_right: true })` → left press
/// (refused) → left release. Committing there ends a fast-resize the
/// user is still performing with the right button down, and every
/// later right-drag sample lands on `DragState::None` and does
/// nothing. Handing it back instead leaves the gesture live, and the
/// right release — which the user has to deliver, since the button
/// is down — commits it.
///
/// `PutBack` cannot strand the gesture: both dispatchers restore the
/// drag state on that answer (`finalize_or_put_back`'s `None` arm
/// writes `DragState::Throttled(drag)` back), which is also why the
/// middle button's answer is `PutBack` rather than a third case. It
/// never reaches here — `route_middle_button` answers `Keep` — but a
/// button that could not have started the gesture has no business
/// ending it, whichever button it is.
pub(in crate::application::app) fn resolve_release(
    released: MouseButton,
    started_with_right: bool,
) -> ThrottledRelease {
    let owner = if started_with_right {
        MouseButton::Right
    } else {
        MouseButton::Left
    };
    if released == owner {
        ThrottledRelease::Commit
    } else {
        ThrottledRelease::PutBack
    }
}

/// Run `released` against an in-flight throttled drag, renderer-free:
/// commit the gesture iff `released` is the button that owns it.
///
/// `Some(refresh)` is "committed, and the caller owes the canvas this
/// decree"; `None` is "put the drag state back — this button did not
/// start the gesture".
///
/// This is the whole of `event_mouse_click::finalize_or_put_back`
/// except the two things that need the input layer: running the
/// decree (`&mut Renderer`) and restoring `DragState`. The split is
/// not cosmetic — the two branches are disjoint in what they need,
/// and pulling the commit branch out here is what makes "does a
/// right release on a left-started drag write anything?" a question
/// the harness can ask. The full shell takes `&mut
/// InputHandlerContext`, i.e. a live wgpu device, which
/// TEST_CONVENTIONS §T8 keeps out of the tests.
pub(in crate::application::app) fn commit_if_resolved(
    released: MouseButton,
    drag: &mut ThrottledDrag,
    commit: ReleaseCommit<'_>,
) -> Option<ReleaseRefresh> {
    match resolve_release(released, drag.as_dyn().started_with_right()) {
        ThrottledRelease::Commit => Some(drag.as_dyn_mut().commit_on_release_core(commit)),
        ThrottledRelease::PutBack => None,
    }
}

impl ReleaseRefresh {
    /// Run the decree against the live renderer.
    pub(in crate::application::app) fn execute(self, ctx: DrainContext<'_>) {
        let DrainContext {
            document,
            mindmap_tree,
            app_scene,
            renderer,
            scene_cache,
            interaction_mode,
            ..
        } = ctx;
        let Some(doc) = document.as_ref() else {
            return;
        };
        match self {
            Self::None => {}
            Self::SceneOnly => {
                rebuild_scene_only(doc, interaction_mode, app_scene, renderer, scene_cache);
            }
            Self::All => {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All four cases of the release resolver. The diagonal —
    /// commit on the owning button, put back on the other — is the
    /// whole gate: widen it and a stray click ends a drag the user
    /// is still performing; narrow it and a gesture never commits at
    /// all.
    ///
    /// Fails on the `(Left, true)` row for the shipped-before-#37
    /// spelling, `match released { Right if !started_with_right =>
    /// PutBack, _ => Commit }`, which answered `Commit` there.
    #[test]
    fn test_resolve_release_covers_both_buttons_and_both_origins() {
        assert_eq!(
            resolve_release(MouseButton::Left, false),
            ThrottledRelease::Commit,
            "a left release finalizes a left-started drag"
        );
        assert_eq!(
            resolve_release(MouseButton::Left, true),
            ThrottledRelease::PutBack,
            "a left release must not end a right-started fast-resize: the \
             right button is still down and its release will commit"
        );
        assert_eq!(
            resolve_release(MouseButton::Right, true),
            ThrottledRelease::Commit,
            "a right release finalizes the fast-resize it started"
        );
        assert_eq!(
            resolve_release(MouseButton::Right, false),
            ThrottledRelease::PutBack,
            "a stray right-click must not end a left-button drag"
        );
    }

    /// The middle button never reaches the throttled release path —
    /// its own arm keeps a `Throttled` state rather than routing it
    /// (`route_middle_button` answers `Keep`) — but it must not be a
    /// finalizer if that ever changes: it cannot have started either
    /// gesture, so the answer is the same `PutBack` a wrong-button
    /// release gets, and `finalize_or_put_back` restores the state on
    /// it rather than stranding the drag.
    #[test]
    fn test_resolve_release_finalizes_only_the_button_that_started_the_gesture() {
        assert_eq!(
            resolve_release(MouseButton::Middle, false),
            ThrottledRelease::PutBack
        );
        assert_eq!(
            resolve_release(MouseButton::Middle, true),
            ThrottledRelease::PutBack
        );
    }
}
