// SPDX-License-Identifier: MPL-2.0

//! Edge grab-handles for the currently-selected edge — the
//! connection reshape surface. Owns the [`EdgeHandleKind`] /
//! [`EdgeHandleElement`] pair, the glyph + size constants, the
//! per-kind channel mapping, and the emission pass that turns one
//! selected edge into its handle set.
//!
//! Emission runs at most once per rebuild (selection is
//! single-edge), so the cost is trivial and there is no cache.
//! [`super::handle`] projects the emitted elements into the
//! `EdgeHandles` canvas tree.

use glam::Vec2;

use crate::mindmap::connection;
use crate::mindmap::scene_cache::EdgeKey;
use crate::mindmap::SELECTION_HIGHLIGHT_HEX;

use super::HandleVisual;

/// Which part of a selected edge a grab-handle targets. The
/// connection reshape surface: anchor endpoints can be dragged to
/// change which side of a node an edge attaches to, control points
/// can be dragged to reshape a curve, and the `Midpoint` handle on a
/// straight edge inserts a control point on first drag to convert
/// the straight line into a quadratic Bezier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeHandleKind {
    /// Endpoint anchor on the `from_id` side.
    AnchorFrom,
    /// Endpoint anchor on the `to_id` side.
    AnchorTo,
    /// Existing control point at `edge.control_points[index]`.
    ControlPoint(usize),
    /// Only emitted for straight edges (empty `control_points`).
    /// Dragging this handle inserts a new control point to curve
    /// the edge. After insertion, subsequent frames treat the drag
    /// as a `ControlPoint(0)` drag.
    Midpoint,
}

/// One grab-handle glyph emitted on top of a selected edge. Rendered
/// as a small cosmic-text buffer in canvas space — the Renderer
/// treats `edge_handles` as its own buffer family since the handle
/// set is small, bounded, and only exists for the currently-selected
/// edge.
pub struct EdgeHandleElement {
    pub edge_key: EdgeKey,
    pub kind: EdgeHandleKind,
    /// Canvas-space position of the handle, already resolved from
    /// the edge's current `control_points` and anchors.
    pub position: (f32, f32),
    /// Glyph string (usually a single char like ◆).
    pub glyph: String,
    /// Color as `#RRGGBB` hex.
    pub color: String,
    /// Font size in points.
    pub font_size_pt: f32,
}

/// Glyph used for anchor and control-point edge grab-handles. A
/// solid black diamond reads as a clickable control point across
/// most fonts.
const EDGE_HANDLE_GLYPH: &str = "\u{25C6}"; // ◆

/// Distinct glyph for the `Midpoint` handle that appears only on
/// straight edges and bootstraps the "curve this line" gesture on
/// drag. A curved arrow reads as "bend me" — specifically an
/// counterclockwise hook (`↺`) so nothing about the handle looks like
/// a plain re-selection target. Without this second glyph the
/// midpoint handle is visually identical to the anchor handles and
/// the gesture is undiscoverable (see `commands/edge.rs` for the
/// console-side counterpart, `edge reset=curve`).
const EDGE_MIDPOINT_HANDLE_GLYPH: &str = "\u{21BA}"; // ↺

/// Font size (in points) for the edge handle glyphs. Slightly larger
/// than the default connection glyph size so handles stand out on top
/// of the selected edge.
const EDGE_HANDLE_FONT_SIZE_PT: f32 = 14.0;

/// Map a handle kind to its stable tree channel. Anchors take 1/2,
/// the midpoint takes 3, and control points stride from 100 by
/// index so adding a CP never collides with the fixed anchor
/// channels. The in-place mutator path compares channels from
/// this function on both sides of the rebuild so the same handle
/// kind always lands on the same arena slot. O(1).
pub fn edge_handle_channel_for(kind: EdgeHandleKind) -> usize {
    match kind {
        EdgeHandleKind::AnchorFrom => 1,
        EdgeHandleKind::AnchorTo => 2,
        EdgeHandleKind::Midpoint => 3,
        EdgeHandleKind::ControlPoint(n) => 100 + n,
    }
}

impl HandleVisual for EdgeHandleElement {
    fn position(&self) -> (f32, f32) {
        self.position
    }
    fn glyph(&self) -> &str {
        &self.glyph
    }
    fn color(&self) -> &str {
        &self.color
    }
    fn font_size_pt(&self) -> f32 {
        self.font_size_pt
    }
    fn channel(&self) -> usize {
        edge_handle_channel_for(self.kind)
    }
}

/// Build the grab-handle set for a single selected edge, given the
/// current (offset-applied) positions and sizes of its endpoint
/// nodes. Called once per scene build (for the selected edge only),
/// so the cost is trivial and needs no cache.
///
/// Always emits AnchorFrom + AnchorTo. On top of that:
/// - an edge with 0 control points gets a `Midpoint` handle
///   (dragging it curves the straight line);
/// - an edge with ≥ 1 control points gets `ControlPoint(i)` handles
///   at each stored offset-from-center.
pub fn build_edge_handles(
    edge: &crate::mindmap::model::MindEdge,
    edge_key: &EdgeKey,
    from_pos: Vec2,
    from_size: Vec2,
    to_pos: Vec2,
    to_size: Vec2,
) -> Vec<EdgeHandleElement> {
    let path = connection::build_connection_path(
        from_pos,
        from_size,
        &edge.anchor_from,
        to_pos,
        to_size,
        &edge.anchor_to,
        &edge.control_points,
    );
    let (start, end) = match &path {
        connection::ConnectionPath::Straight { start, end } => (*start, *end),
        connection::ConnectionPath::CubicBezier { start, end, .. } => (*start, *end),
    };

    let from_center = Vec2::new(from_pos.x + from_size.x * 0.5, from_pos.y + from_size.y * 0.5);
    let to_center = Vec2::new(to_pos.x + to_size.x * 0.5, to_pos.y + to_size.y * 0.5);

    let make = |kind: EdgeHandleKind, position: Vec2| {
        // The midpoint handle carries a distinct glyph so the
        // "drag-to-curve" gesture is visible on a straight edge —
        // without it, users can't tell the midpoint marker apart
        // from the two anchor handles and the gesture is
        // undiscoverable.
        let glyph = if kind == EdgeHandleKind::Midpoint {
            EDGE_MIDPOINT_HANDLE_GLYPH
        } else {
            EDGE_HANDLE_GLYPH
        };
        EdgeHandleElement {
            edge_key: edge_key.clone(),
            kind,
            position: (position.x, position.y),
            glyph: glyph.to_string(),
            color: SELECTION_HIGHLIGHT_HEX.to_string(),
            font_size_pt: EDGE_HANDLE_FONT_SIZE_PT,
        }
    };

    let mut handles = Vec::with_capacity(5);
    handles.push(make(EdgeHandleKind::AnchorFrom, start));
    handles.push(make(EdgeHandleKind::AnchorTo, end));

    match edge.control_points.len() {
        0 => {
            // Straight edge: offer a midpoint handle that starts a
            // "curve this line" gesture on drag.
            let mid = start.lerp(end, 0.5);
            handles.push(make(EdgeHandleKind::Midpoint, mid));
        }
        1 => {
            // Quadratic Bezier (stored as 1 CP offset from from_center).
            let cp0 =
                from_center + Vec2::new(edge.control_points[0].x as f32, edge.control_points[0].y as f32);
            handles.push(make(EdgeHandleKind::ControlPoint(0), cp0));
        }
        _ => {
            // Cubic Bezier (stored as 2 CPs: cp[0] from from_center,
            // cp[1] from to_center).
            let cp0 =
                from_center + Vec2::new(edge.control_points[0].x as f32, edge.control_points[0].y as f32);
            let cp1 = to_center + Vec2::new(edge.control_points[1].x as f32, edge.control_points[1].y as f32);
            handles.push(make(EdgeHandleKind::ControlPoint(0), cp0));
            handles.push(make(EdgeHandleKind::ControlPoint(1), cp1));
        }
    }

    handles
}
