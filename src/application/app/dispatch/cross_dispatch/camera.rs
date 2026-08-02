// SPDX-License-Identifier: MPL-2.0

//! Camera + zoom apply_* helpers — every arm whose effect lands
//! on camera state (renderer-side) or zoom-visibility metadata
//! (document-side). Two thematic groups:
//!
//! 1. **Camera-state arms** (renderer-only, no document
//!    mutation): `apply_zoom_step`, `apply_zoom_reset`,
//!    `apply_zoom_fit`, `apply_pan_camera`,
//!    `apply_center_on_selection` (reads doc, doesn't mutate).
//! 2. **Zoom-visibility arms** (document mutations gated by the
//!    rebuild envelope): `apply_set_zoom_window`,
//!    `apply_clear_zoom`.
//!
//! `apply_jump_to_root` is the cross-bucket arm — it touches
//! both selection and camera, so it routes through
//! `rebuild_after_selection_change`.

use crate::application::common::RenderDecree;
use crate::application::document::{MindMapDocument, OptionEdit, SelectionState};
use crate::application::renderer::Renderer;
use baumhard::mindmap::tree_builder::MindMapTree;

use super::{apply_with_rebuild, RebuildContext};

/// Direction of a single keyboard / wheel zoom step. Typed so
/// callers don't have to pass `&Action` and the helper doesn't
/// have to re-match it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::application::app) enum ZoomDir {
    In,
    Out,
}

/// Step zoom toward `(screen_x, screen_y)` (typically the cursor).
/// The factor mirrors the legacy hardcoded wheel handler (1.1×) so
/// wheel-bound `ZoomIn`/`ZoomOut` behave identically across targets.
pub(in crate::application::app) fn apply_zoom_step(
    dir: ZoomDir,
    cursor_pos: (f64, f64),
    renderer: &mut Renderer,
) {
    let factor = match dir {
        ZoomDir::In => 1.1f32,
        ZoomDir::Out => 1.0f32 / 1.1f32,
    };
    renderer.process_decree(RenderDecree::CameraZoom {
        screen_x: cursor_pos.0 as f32,
        screen_y: cursor_pos.1 as f32,
        factor,
    });
}

/// Reset zoom to 1.0 anchored at the screen center (NOT the
/// cursor). A cursor-anchored zoom emits a `CameraZoom` decree
/// whose canvas-position formula shifts the camera when the focus
/// is off-center — so a Ctrl+0 with the cursor in a corner would
/// scoot the view by 200+ px instead of cleanly resetting in
/// place. Computing the factor inverse against current zoom keeps
/// the multiplicative path; using screen-center as the focus
/// cancels the position shift algebraically.
pub(in crate::application::app) fn apply_zoom_reset(renderer: &mut Renderer) {
    let zoom = renderer.camera_zoom().max(f32::EPSILON);
    renderer.process_decree(RenderDecree::CameraZoom {
        screen_x: renderer.surface_width() as f32 * 0.5,
        screen_y: renderer.surface_height() as f32 * 0.5,
        factor: 1.0f32 / zoom,
    });
}

/// Fit the viewport to the loaded tree's bounds. No-op when no
/// tree has been built yet.
pub(in crate::application::app) fn apply_zoom_fit(
    mindmap_tree: &Option<MindMapTree>,
    renderer: &mut Renderer,
) {
    if let Some(tree) = mindmap_tree.as_ref() {
        renderer.fit_camera_to_tree(&tree.tree);
    }
}

/// Direction of a single keyboard pan nudge. Typed so callers
/// don't have to pass `&Action` and the helper doesn't have to
/// re-match it. Geographic compass names mirror the
/// `Action::PanCamera*` variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::application::app) enum PanDir {
    North,
    South,
    East,
    West,
}

/// Keyboard nudge — fixed step in screen pixels, then converted
/// to a `CameraPan` decree like the LeftDrag path emits per cursor
/// move. Step size matches a coarse but perceptible nudge.
pub(in crate::application::app) fn apply_pan_camera(dir: PanDir, renderer: &mut Renderer) {
    const PAN_STEP_PX: f32 = 50.0;
    let (dx, dy) = match dir {
        PanDir::North => (0.0, -PAN_STEP_PX),
        PanDir::South => (0.0, PAN_STEP_PX),
        PanDir::East => (-PAN_STEP_PX, 0.0),
        PanDir::West => (PAN_STEP_PX, 0.0),
    };
    renderer.process_decree(RenderDecree::CameraPan(dx, dy));
}

/// Centre the camera on the centroid of the currently-selected
/// nodes. No-op when nothing is selected (or only an edge /
/// portal-marker selection, which carries no point centroid).
pub(in crate::application::app) fn apply_center_on_selection(
    document: &MindMapDocument,
    renderer: &mut Renderer,
) {
    let ids: Vec<&str> = document.selection.selected_ids();
    if ids.is_empty() {
        return;
    }
    let mut sum = glam::Vec2::ZERO;
    let mut count = 0u32;
    for id in &ids {
        if let Some(node) = document.mindmap.nodes.get(*id) {
            sum += node.center_vec2();
            count += 1;
        }
    }
    if count > 0 {
        renderer.set_camera_center(sum / count as f32);
    }
}

/// Set the document's selection to its first root node (id-sorted)
/// and return the canvas-space center the camera should jump to.
/// Returns `None` when the document is empty (and selection is
/// untouched).
///
/// Lives in camera.rs alongside [`apply_jump_to_root`] (its sole
/// consumer) — the function pair is the cross-bucket arm
/// described in this file's `//!` header.
#[must_use = "Some(center) is the camera target — drop with `let _ = …` to skip the camera move"]
pub(in crate::application::app) fn jump_to_root_in(doc: &mut MindMapDocument) -> Option<glam::Vec2> {
    let (id, center) = doc
        .mindmap
        .root_nodes()
        .first()
        .map(|n| (n.id.clone(), n.center_vec2()))?;
    doc.selection = SelectionState::Single(id);
    Some(center)
}

/// Select the document's first root node and center the camera on
/// it. No-op when the document is empty.
pub(in crate::application::app) fn apply_jump_to_root(rc: &mut RebuildContext<'_>) {
    if let Some(center) = jump_to_root_in(rc.document) {
        rc.renderer.set_camera_center(center);
        rc.rebuild_after_selection_change();
    }
}

/// Set the per-element zoom-visibility window (`min_zoom_to_render`,
/// `max_zoom_to_render`) on every node / edge / portal in the
/// current selection. Either bound may be `OptionEdit::Set` (write
/// the value), `OptionEdit::Clear` (drop the bound entirely), or
/// `OptionEdit::Keep` (no-op). Document mutation → full rebuild
/// via `apply_with_rebuild`.
///
/// This is a *document* zoom edit (per-element visibility window
/// stored on the model), not a *camera* zoom (`apply_zoom_step` /
/// `_reset` / `_fit`). Both share this file because the user-
/// facing concept "zoom" covers both — but they touch different
/// state and only this one runs through the rebuild envelope.
pub(in crate::application::app) fn apply_set_zoom_window(
    min: OptionEdit<f32>,
    max: OptionEdit<f32>,
    rc: &mut RebuildContext<'_>,
) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::zoom::apply_zoom_to_selection(doc, min, max)
    });
}

/// Clear both zoom-visibility bounds on every element in the
/// current selection (equivalent to `apply_set_zoom_window` with
/// `OptionEdit::Clear` on both axes). Document mutation → full
/// rebuild via `apply_with_rebuild`.
pub(in crate::application::app) fn apply_clear_zoom(rc: &mut RebuildContext<'_>) {
    apply_with_rebuild(rc, |doc| {
        crate::application::console::commands::zoom::apply_zoom_to_selection(
            doc,
            OptionEdit::Clear,
            OptionEdit::Clear,
        )
    });
}

#[cfg(test)]
mod tests {
    //! Pure-doc-mutation tests for the camera-bucket helpers
    //! that DO mutate document state — `jump_to_root_in`
    //! today; future zoom-window helpers' inner functions
    //! would land here too. The renderer-touching `apply_*`
    //! wrappers are out of scope per `TEST_CONVENTIONS.md §T8`.

    use super::*;
    use crate::application::document::tests_common::load_test_doc;

    fn first_root_id(doc: &MindMapDocument) -> String {
        doc.mindmap
            .root_nodes()
            .first()
            .expect("test fixture has at least one root")
            .id
            .clone()
    }

    #[test]
    fn jump_to_root_in_returns_first_root_center_and_selects_it() {
        let mut doc = load_test_doc();
        let expected_root = first_root_id(&doc);
        let center = jump_to_root_in(&mut doc).expect("non-empty fixture");
        assert!(center.x.is_finite() && center.y.is_finite());
        assert!(matches!(
            doc.selection,
            SelectionState::Single(ref s) if s == &expected_root
        ));
    }

    #[test]
    fn jump_to_root_in_returns_none_for_empty_doc() {
        let mut doc = MindMapDocument::new_blank(None);
        assert!(jump_to_root_in(&mut doc).is_none());
        assert!(matches!(doc.selection, SelectionState::None));
    }
}
