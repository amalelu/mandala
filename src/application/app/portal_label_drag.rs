//! Portal label drag math: given a cursor position in canvas
//! space, snap it to the nearest point on the owning node's
//! border and write both the resulting `border_t` (slide along
//! the perimeter) and a signed perpendicular offset (slide away
//! from the border along its outward normal) into the edge's
//! per-endpoint state. Used by the per-frame drain loop and
//! once more at release to commit the final position.
//!
//! Mirrors the shape of [`super::edge_drag`]: one free function,
//! no per-frame allocation, mutating through
//! [`MindMapDocument`] setters so undo / dirty bookkeeping is
//! consistent with every other edge edit. The drain path does
//! **not** push undo per frame — the event-loop commit
//! (mouse-up in `event_mouse_click.rs`) pushes one
//! `UndoAction::EditEdge` with the pre-drag snapshot, matching
//! how `DraggingEdgeHandle` handles commits.

#![cfg(not(target_arch = "wasm32"))]

use glam::Vec2;

use baumhard::mindmap::portal_geometry::{
    border_outward_normal, border_point_at, nearest_border_t,
};

use crate::application::document::{EdgeRef, MindMapDocument};

/// Drag-time snap threshold for the perpendicular offset, in
/// canvas units. Cursor movements that land closer to the border
/// than this snap the stored offset back to `None`, restoring the
/// flush-to-border default. Chosen a touch above subpixel jitter
/// so the "shake it back to auto" gesture is predictable.
const PERP_SNAP_EPSILON: f32 = 0.5;

/// Project a cursor onto a node's border and return the pair
/// `(border_t, perpendicular_offset)` the drag would write for it.
/// Pure geometry — no document / model mutation — so it's callable
/// from tests without a `MindMapDocument`. Small perpendicular
/// magnitudes snap to `None` via [`PERP_SNAP_EPSILON`].
pub(in crate::application::app) fn project_cursor_to_portal_params(
    node_pos: Vec2,
    node_size: Vec2,
    cursor_canvas: Vec2,
) -> (f32, Option<f32>) {
    let t = nearest_border_t(node_pos, node_size, cursor_canvas);
    let anchor = border_point_at(node_pos, node_size, t);
    let normal = border_outward_normal(t);
    let raw_perp = (cursor_canvas - anchor).dot(normal);
    let perp = if raw_perp.abs() < PERP_SNAP_EPSILON {
        None
    } else {
        Some(raw_perp)
    };
    (t, perp)
}

/// Apply one frame of portal-label drag. Projects `cursor_canvas`
/// onto the owning node's border to compute `(border_t, perp)` —
/// perimeter parameter plus signed outward distance — and writes
/// both into the per-endpoint state on the edge. Returns `true`
/// if either field changed (so the caller knows whether the
/// frame produced a visible change); `false` on no-change, or
/// on the "edge disappeared mid-drag" case (logged and skipped).
pub(in crate::application::app) fn apply_portal_label_drag(
    doc: &mut MindMapDocument,
    edge_ref: &EdgeRef,
    endpoint_node_id: &str,
    cursor_canvas: Vec2,
) -> bool {
    // Locate the owning node to project onto. Portal labels are
    // always attached to the node named by `endpoint_node_id`
    // (one of `edge.from_id` / `edge.to_id`), and that id reached
    // us via a click hit-test, so a missing node here means the
    // node was deleted between click and drag.
    let Some(node) = doc.mindmap.nodes.get(endpoint_node_id) else {
        log::warn!(
            "apply_portal_label_drag: endpoint node {endpoint_node_id} disappeared mid-drag"
        );
        return false;
    };
    let node_pos = Vec2::new(node.position.x as f32, node.position.y as f32);
    let node_size = Vec2::new(node.size.width as f32, node.size.height as f32);
    let (t, perp) = project_cursor_to_portal_params(node_pos, node_size, cursor_canvas);

    // Direct field write — bypassing `set_portal_label_border_t`
    // because that setter pushes an `EditEdge` per call. The
    // per-frame drain would flood the undo stack; we snapshot
    // once at drag start and push a single `EditEdge` at release.
    let Some(idx) = doc
        .mindmap
        .edges
        .iter()
        .position(|e| edge_ref.matches(e))
    else {
        return false;
    };
    let edge = &mut doc.mindmap.edges[idx];
    let slot = baumhard::mindmap::model::portal_endpoint_state_mut(edge, endpoint_node_id);
    let slot = match slot {
        Some(s) => s,
        None => return false,
    };
    let prev_t = slot.as_ref().and_then(|s| s.border_t);
    let prev_perp = slot.as_ref().and_then(|s| s.perpendicular_offset);
    let t_changed = prev_t.map_or(true, |prev| (prev - t).abs() >= f32::EPSILON);
    let perp_changed = match (prev_perp, perp) {
        (None, None) => false,
        (Some(a), Some(b)) => (a - b).abs() >= f32::EPSILON,
        _ => true,
    };
    if !t_changed && !perp_changed {
        return false;
    }
    let state = slot.get_or_insert_with(baumhard::mindmap::model::PortalEndpointState::default);
    state.border_t = Some(t);
    state.perpendicular_offset = perp;
    doc.dirty = true;
    true
}

#[cfg(test)]
mod tests {
    //! Perpendicular drag math around a 100×50 rectangle at the
    //! origin. The drag projects the cursor onto the nearest
    //! border, and the perpendicular is the signed outward distance
    //! from that projection. Cursor on or near the border snaps
    //! back to `None` so the user can release the slide and
    //! reclaim "auto" without going through the console.
    //!
    //! The owning node's extent here is `x ∈ [0, 100]`,
    //! `y ∈ [0, 50]`.
    use super::*;
    const POS: Vec2 = Vec2::new(0.0, 0.0);
    const SIZE: Vec2 = Vec2::new(100.0, 50.0);

    #[test]
    fn test_cursor_outside_right_border_writes_positive_perpendicular() {
        // Cursor well to the right of the node. Nearest border is
        // the right edge (t in [1, 2), outward normal +x), so the
        // perpendicular is ~100 (distance from right edge at x=100
        // to cursor at x=200).
        let (_t, perp) = project_cursor_to_portal_params(
            POS,
            SIZE,
            Vec2::new(200.0, 25.0),
        );
        let perp = perp.expect("cursor > ε outside must produce Some");
        assert!(
            (perp - 100.0).abs() < 1.0,
            "expected ~100 outward, got {perp}"
        );
    }

    #[test]
    fn test_cursor_on_border_snaps_perpendicular_to_none() {
        // Cursor exactly on the right border (x = node_right).
        // Perpendicular magnitude is 0, which sits inside the
        // snap epsilon and must produce `None`.
        let (_t, perp) = project_cursor_to_portal_params(
            POS,
            SIZE,
            Vec2::new(100.0, 25.0),
        );
        assert!(perp.is_none(), "on-border must snap to None, got {perp:?}");
    }

    #[test]
    fn test_cursor_inside_node_writes_negative_perpendicular() {
        // Cursor deep inside the node. The right border is the
        // closest (cursor_x = 30 is 30 units from left, 70 from
        // right — 70 < 30? no, 30 is closer to left. Use y=0 which
        // is on top border, so snap changes. Pick x=60 so the
        // nearest border is still the right edge (distance 40)
        // and the perpendicular is ~ -40.
        let (_t, perp) = project_cursor_to_portal_params(
            POS,
            SIZE,
            Vec2::new(60.0, 45.0),
        );
        let perp = perp.expect("inside-border drag must produce Some");
        assert!(
            perp < -1.0,
            "expected negative perpendicular (cursor inside node), got {perp}"
        );
    }

    #[test]
    fn test_perpendicular_snap_epsilon_boundary() {
        // Just outside the epsilon — must store `Some`.
        let (_t, perp) = project_cursor_to_portal_params(
            POS,
            SIZE,
            Vec2::new(100.0 + PERP_SNAP_EPSILON + 0.01, 25.0),
        );
        assert!(perp.is_some(), "just past epsilon must round to Some");
        // Just inside the epsilon — must snap to None.
        let (_t, perp) = project_cursor_to_portal_params(
            POS,
            SIZE,
            Vec2::new(100.0 + PERP_SNAP_EPSILON * 0.5, 25.0),
        );
        assert!(perp.is_none(), "within epsilon must snap to None");
    }
}
