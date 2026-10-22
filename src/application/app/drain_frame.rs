// SPDX-License-Identifier: MPL-2.0

//! Per-frame drain helpers for the non-throttled paths in
//! `AboutToWait`. Throttled drains (drag, hover) live under
//! [`super::throttled_interaction`]; what's here are the three paths
//! that deliberately skip the throttle: rect-select, camera-driven
//! geometry rebuild, animation tick.
//!
//! The rect-select path does one thing the other two do not: it owns
//! an invariant rather than only a unit of work. See
//! [`drain_rect_select`] — running unconditionally every frame is
//! what lets it hold the rubber band's on-screen state to the drag
//! state that authorizes it, without anyone having to enumerate the
//! ways that state can end.

#![cfg(not(target_arch = "wasm32"))]

use glam::Vec2;

use super::now_ms;
use super::scene_rebuild::{rebuild_all, rebuild_camera_geometry, rebuild_selection_highlight};
use crate::application::document::{rect_select, MindMapDocument};
use crate::application::renderer::Renderer;

/// **The rubber-band gesture's whole per-frame contract, both
/// halves of it.** `DragState::SelectingRect` is the authority on
/// whether a rubber band exists; the two artifacts it leaves on
/// screen — the overlay rectangle in the renderer and the covered
/// set on [`MindMapDocument::rect_select_preview`] (the field; the
/// cross-platform read is the method of the same name) — are its
/// projection, re-derived here from that authority once per frame.
///
/// That is why this function takes the whole `DragState` rather than
/// the `SelectingRect` payload. The preview is document state every
/// node-tree rebuild in the app reads through
/// `scene_rebuild::highlight_entries_for`, so a gesture that ended
/// without it being dropped leaves the *whole canvas* painting a
/// rectangle the user let go of — and `SelectingRect` has more than
/// one way out. Enumerating those ways is what this used to depend
/// on, and the enumeration was already incomplete: a left press
/// overwrote the state with `Pending`, `Action::PanCanvas` with
/// `Panning`, and neither passed the release arm that dropped the
/// preview. Re-deriving needs no enumeration, so a way out that
/// nobody has thought of yet cannot strand it either. Nothing stale
/// reaches the screen: the two transitions above rebuild nothing
/// themselves, so this runs — in `AboutToWait`, before the redraw —
/// between the abandonment and the next frame drawn.
///
/// ## The live half
///
/// Two things used to happen on every frame of the gesture, and
/// neither has to.
///
/// The hit-test built a whole fresh arena (`doc.build_tree()`, a
/// §B7-benchmarked primitive) just to have something to hit-test
/// against. It does not need one: a rubber-band drag mutates
/// nothing, so the geometry [`rect_select`] reads is exactly the
/// geometry the installed tree already carries. The old code built
/// its own because it also needed a *clean* tree to stamp the
/// preview onto — the preview is an absolute `SetRegionColor` write
/// with no inverse, so the only way to move it was to start over.
///
/// One consequence is worth stating rather than leaving to be
/// discovered: the installed tree carries the geometry as of the
/// last rebuild, and this drain runs *before*
/// [`drain_animation_tick`] in the same frame, so a node whose
/// position an animation is advancing is hit-tested one tick behind
/// where it is drawn. The old code, building from the model each
/// frame, was not. The window is a single frame and only opens while
/// an animation and a rubber band are live at once; closing it would
/// mean either ordering the animation tick first (which would make
/// the *rectangle* lag the pointer instead — the trade
/// `MutationFrequencyThrottle`'s contract forbids) or going back to
/// a per-frame arena build.
///
/// The repaint then ran on every frame regardless. Now the covered
/// set lives on the document, where every rebuild path reads it, and
/// `set_rect_select_preview` reports whether the set actually moved.
/// A frame that only slid the rectangle without crossing a node
/// boundary — nearly all of them — calls neither `build_tree` nor
/// `rebuild_buffers_from_tree`.
///
/// **What that trade is, named as such.** The issue's other route,
/// region-color deltas, is the `lib/baumhard/CONVENTIONS.md` §B2
/// "mutation, not rebuild" shape; what landed here is not — a
/// set-change frame still allocates a fresh arena through
/// [`rebuild_selection_highlight`]. The choice made was to run the
/// non-§B2 route *less often* rather than to take the §B2-canonical
/// one, because `SetRegionColor` is an absolute write with no
/// inverse: un-previewing a node means restoring colors from a
/// snapshot, and the delta would have to reproduce by hand four
/// overlay decisions the fresh-tree path gets for free (drop the
/// pre-drag selection's highlight, keep the `NodeEdit` dim, keep
/// active toggles above the preview, stamp the preview). There is no
/// GPU test in this repo (TEST_CONVENTIONS §T8) that could catch
/// getting one of those wrong.
///
/// Putting the preview on the document also closed a hole: an
/// animation tick landing mid-gesture runs its `rebuild_all` *after*
/// this drain in the same frame, and used to wipe the preview until
/// the next drain rebuilt it. It now paints the preview like every
/// other rebuild.
pub(super) fn drain_rect_select(
    drag_state: &super::DragState,
    document: &mut Option<MindMapDocument>,
    interaction_mode: &super::InteractionMode,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    renderer: &mut Renderer,
) {
    let preview_live = document
        .as_ref()
        .is_some_and(|doc| doc.rect_select_preview().is_some());
    match rect_select_frame(drag_state, preview_live) {
        RectSelectFrame::Track {
            start_canvas: sc,
            current_canvas: cc,
        } => {
            let min = Vec2::new(sc.x.min(cc.x), sc.y.min(cc.y));
            let max = Vec2::new(sc.x.max(cc.x), sc.y.max(cc.y));
            // Unthrottled and unconditional: the rectangle is the
            // part of this gesture that tracks the pointer, and
            // `MutationFrequencyThrottle`'s contract is that
            // responsiveness is never traded for fidelity.
            renderer.rebuild_selection_rect_overlay(min, max);

            let Some(doc) = document.as_mut() else {
                return;
            };
            // No installed tree means no geometry to hit-test —
            // reachable only between a document swap and the rebuild
            // that follows it, which installs one. The preview is
            // still installed (empty) rather than skipped, because
            // `Some(_)` is what says a gesture left something on
            // screen: leaving it `None` for those frames would let
            // the `End` arm read "nothing to repaint" while the
            // overlay rectangle is drawn.
            let hits = mindmap_tree
                .as_ref()
                .map(|tree| rect_select(sc, cc, tree))
                .unwrap_or_default();
            if !doc.set_rect_select_preview(hits) {
                return;
            }
            // A preview-set change moves node text-buffer colors and
            // nothing else — no geometry, no canvas roles, no
            // mode-status line — so this is the node-tree tier rather
            // than `rebuild_all`.
            rebuild_selection_highlight(doc, interaction_mode, mindmap_tree, renderer);
        }
        RectSelectFrame::End { repaint } => {
            end_rect_select_gesture(document, renderer);
            if repaint {
                if let Some(doc) = document.as_ref() {
                    rebuild_selection_highlight(doc, interaction_mode, mindmap_tree, renderer);
                }
            }
        }
    }
}

/// What one frame owes the rubber band, given the drag state it
/// finds and whether a covered set is currently installed.
///
/// Pure so the invariant is pinnable at `DragState` level — the
/// drain takes a `&mut Renderer`, i.e. a live wgpu device, which
/// TEST_CONVENTIONS §T8 keeps out of the harness. Same shape, and
/// for the same reason, as `event_mouse_click::route_middle_button`.
#[derive(Debug, Clone, Copy, PartialEq)]
enum RectSelectFrame {
    /// A gesture is live: move the rectangle to the pointer and
    /// re-hit-test what it covers.
    Track {
        start_canvas: Vec2,
        current_canvas: Vec2,
    },
    /// No gesture is live, so neither of its artifacts may be. Drop
    /// the overlay rectangle; `repaint` is whether a covered set was
    /// still installed, i.e. whether nodes are still tinted for a
    /// gesture that is over.
    End { repaint: bool },
}

/// Decide [`RectSelectFrame`] for one frame.
///
/// There is deliberately no "nothing to do" answer. Off-gesture the
/// overlay rectangle is dropped unconditionally — `overlay_buffers`
/// is the rubber band's alone (see `renderer::selection_overlay`),
/// so "no gesture, no rectangle" needs no liveness bit of its own,
/// and a `Vec::clear` on an empty vec is what an idle frame pays for
/// not needing one.
fn rect_select_frame(drag_state: &super::DragState, preview_live: bool) -> RectSelectFrame {
    match *drag_state {
        super::DragState::SelectingRect {
            start_canvas,
            current_canvas,
        } => RectSelectFrame::Track {
            start_canvas,
            current_canvas,
        },
        _ => RectSelectFrame::End {
            repaint: preview_live,
        },
    }
}

/// Drop both artifacts a rubber-band gesture leaves on screen — the
/// overlay rectangle and the covered set — and report whether the
/// node highlight now needs repainting.
///
/// `true` means a preview *was* live, so the nodes it was tinting
/// are still tinted and the caller owes them a
/// [`rebuild_selection_highlight`]. `false` means there was nothing
/// to end.
///
/// [`drain_rect_select`] runs this on every frame the drag state is
/// not `SelectingRect`, which is what holds the invariant; the two
/// other callers — the left-button release arm and the target-picker
/// mode entries — call it eagerly only so that the rebuild they run
/// in the same breath does not paint the set one last time first.
pub(super) fn end_rect_select_gesture(
    document: &mut Option<MindMapDocument>,
    renderer: &mut Renderer,
) -> bool {
    renderer.clear_overlay_buffers();
    document
        .as_mut()
        .and_then(MindMapDocument::take_rect_select_preview)
        .is_some()
}

/// Camera (pan/zoom/resize) changed — rebuild
/// connection buffers against the new viewport. On
/// zoom, the document-side scene cache is also stale
/// because effective font size depends on zoom, so
/// clear it before the rebuild re-samples.
///
/// Skipped when a node drag is in progress: the
/// `MovingNode` drain rebuilds with the drag offsets
/// on its next non-zero `pending_delta` using the
/// current camera, and rebuilding here with empty
/// offsets would flicker dragged connections back to
/// their pre-drag positions for one frame. Wheel-zoom
/// during an active drag with zero `pending_delta`
/// leaves connections stale for one frame until the
/// next mouse-move flush — an acceptable tradeoff to
/// keep the two dirty sources separate. Always take
/// the flags (even when skipped) so they don't leak
/// across drag frames.
pub(super) fn drain_camera_geometry_rebuild(
    is_moving_node: bool,
    document: &Option<MindMapDocument>,
    interaction_mode: &super::InteractionMode,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    let geometry_dirty = renderer.take_connection_geometry_dirty();
    if geometry_dirty && !is_moving_node {
        if let Some(doc) = document.as_ref() {
            // Body is `scene_rebuild::rebuild_camera_geometry` —
            // WASM's wheel handler runs the same one, under the same
            // dirty flag, because the browser has no per-frame drain
            // to hang it off.
            rebuild_camera_geometry(doc, interaction_mode, app_scene, renderer, scene_cache);
        }
    }
}

/// Tick any active animations. Each tick lerps the from / to
/// snapshots into the model and (on completion) routes the final
/// state through `apply_custom_mutation` so the standard
/// model-sync + undo-push runs once. Drives `rebuild_all` only
/// when something actually advanced. The event loop's
/// `ControlFlow::Wait` / `ControlFlow::Poll` choice is decided in
/// `NativeApp::about_to_wait` from `InitState::needs_continuation`,
/// which factors in `has_active_animations` — so when no animations
/// are active and no other source needs continuation, the loop
/// parks until the next OS event.
pub(super) fn drain_animation_tick(
    document: &mut Option<MindMapDocument>,
    interaction_mode: &super::InteractionMode,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    let animation_advanced = match document.as_mut() {
        Some(doc) if doc.has_active_animations() => {
            doc.tick_animations(now_ms() as u64, mindmap_tree.as_mut())
        }
        _ => false,
    };
    if animation_advanced {
        if let Some(doc) = document.as_ref() {
            // Animation ticks lerp positions (and on completion
            // route through `apply_custom_mutation`) in place; the
            // cache's `pre_clip_positions` go stale under both
            // paths. Clear before re-sampling.
            scene_cache.clear();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::app::scene_rebuild::highlight_entries_for;
    use crate::application::document::SelectionState;

    fn pending() -> super::super::DragState {
        super::super::DragState::Pending(Box::new(super::super::PendingPress {
            start_pos: (0.0, 0.0),
            hit_node: None,
            hit_section_idx: None,
            hit_edge_handle: None,
            hit_portal_label: None,
            hit_edge_label: None,
            hit_section_resize_handle: None,
            hit_node_resize_handle: None,
        }))
    }

    fn selecting_rect() -> super::super::DragState {
        super::super::DragState::SelectingRect {
            start_canvas: Vec2::new(1.0, 2.0),
            current_canvas: Vec2::new(30.0, 40.0),
        }
    }

    /// A live gesture keeps its preview and gets its rectangle
    /// tracked, whether or not a covered set is installed yet — the
    /// first frame of every gesture has none.
    #[test]
    fn test_a_live_rubber_band_tracks_the_pointer_from_its_first_frame() {
        for preview_live in [false, true] {
            assert_eq!(
                rect_select_frame(&selecting_rect(), preview_live),
                RectSelectFrame::Track {
                    start_canvas: Vec2::new(1.0, 2.0),
                    current_canvas: Vec2::new(30.0, 40.0),
                },
                "preview_live={preview_live} must not change what a live gesture owes"
            );
        }
    }

    /// **Every way out of `SelectingRect`, not the one the release
    /// arm covers.** The state is authority; a frame that does not
    /// find it ends the gesture regardless of *how* it ended. These
    /// four rows are the transitions that actually strand it: a left
    /// press (`Pending`), `Action::PanCanvas` (`Panning`), a
    /// threshold-cross into a drag, and a plain reset.
    ///
    /// Fails on the pre-fix shape — nothing ran off-gesture at all,
    /// i.e. `_ => RectSelectFrame::End { repaint: false }`.
    #[test]
    fn test_every_exit_from_a_rubber_band_ends_the_preview_it_left_behind() {
        for drag in [
            pending(),
            super::super::DragState::Panning,
            super::super::DragState::None,
            super::super::DragState::PendingRight {
                start_pos: (0.0, 0.0),
                start_canvas: Vec2::ZERO,
                hit_node: None,
                hit_section_idx: None,
            },
        ] {
            assert_eq!(
                rect_select_frame(&drag, true),
                RectSelectFrame::End { repaint: true },
                "an abandoned preview must be ended and repainted from {:?}",
                std::mem::discriminant(&drag)
            );
        }
    }

    /// The complement: off-gesture with nothing installed still ends
    /// (the overlay rectangle is dropped unconditionally) but owes no
    /// repaint. Without this row the fix could be "repaint always",
    /// which would run a node-tree rebuild on every idle frame of the
    /// session.
    #[test]
    fn test_an_idle_frame_owes_no_highlight_repaint() {
        assert_eq!(
            rect_select_frame(&super::super::DragState::None, false),
            RectSelectFrame::End { repaint: false }
        );
    }

    /// **The leak, end to end.** `highlight_entries_for` is what
    /// *every* node-tree rebuild in the app reads, so a preview left
    /// installed after its gesture ended is not a stale overlay — it
    /// is the whole canvas painting a set the user let go of, on
    /// every rebuild from any source. The fixture makes the two
    /// answers impossible to confuse.
    ///
    /// Fails on the pre-fix shape (`_ => End { repaint: false }` plus
    /// no `take` off-gesture) with exactly the reviewer's Probe B
    /// output: left `["stale-preview"]`, right `["the-real-selection"]`.
    #[test]
    fn test_an_abandoned_rubber_band_stops_being_painted_by_every_rebuild() {
        let mut doc = crate::application::document::tests_common::load_test_doc();
        doc.selection = SelectionState::Single("the-real-selection".to_string());
        doc.set_rect_select_preview(vec!["stale-preview".to_string()]);

        // A left press mid-gesture: the drag state is replaced with
        // `Pending` and the release arm that ends a rubber band is
        // never reached.
        let mut document = Some(doc);
        let frame = rect_select_frame(
            &pending(),
            document
                .as_ref()
                .is_some_and(|d| d.rect_select_preview().is_some()),
        );
        // The drain's own body, minus the half that needs a live wgpu
        // device (TEST_CONVENTIONS §T8): the frame's answer is what
        // decides whether the covered set is handed back, so a wrong
        // answer strands it here exactly as it would on screen.
        if matches!(frame, RectSelectFrame::End { repaint: true }) {
            document.as_mut().and_then(|d| d.take_rect_select_preview());
        }

        let doc = document.as_ref().expect("fixture document");
        let ids: Vec<&str> = highlight_entries_for(doc)
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        assert_eq!(
            ids,
            vec!["the-real-selection"],
            "LEAK: every rebuild in the app now paints the abandoned rubber-band set"
        );
    }
}
