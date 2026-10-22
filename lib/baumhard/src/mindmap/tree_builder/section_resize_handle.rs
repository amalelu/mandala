// SPDX-License-Identifier: MPL-2.0

//! Section resize-handle emission. Emits 8 handles per
//! `Some`-sized selected section; `None`-sized (fill-parent)
//! sections have no AABB to stretch and produce no handles.

use std::collections::{HashMap, HashSet};
use std::fmt;

use glam::Vec2;

use crate::mindmap::model::MindMap;
use crate::mindmap::SELECTION_HIGHLIGHT_HEX;
use crate::util::geometry::aabb_center;

/// Which of the eight resize handles on a `Some`-sized section
/// the cursor is targeting. Order is the conventional clockwise-
/// from-top-left ladder used by most resize-box UIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResizeHandleSide {
    NW,
    N,
    NE,
    E,
    SE,
    S,
    SW,
    W,
}

impl ResizeHandleSide {
    /// Resolve the per-axis growth direction of this handle into a
    /// pair of `(x_axis, y_axis)` factors. `+1` grows the size,
    /// `-1` shifts the offset and shrinks the size, `0` leaves the
    /// axis untouched. Edge-midpoint handles return `0` on the axis
    /// they don't move so the drain can multiply `(dx, dy)` directly
    /// without per-side branching.
    ///
    /// **Coordinate convention.** The result is "size delta along
    /// each axis" — for an N handle (`(0, -1)`), a positive cursor
    /// `dy` shrinks the section's height and grows `offset.y` by the
    /// same amount, so the bottom edge stays put.
    pub fn axis_factors(&self) -> (i8, i8) {
        match self {
            Self::NW => (-1, -1),
            Self::N => (0, -1),
            Self::NE => (1, -1),
            Self::E => (1, 0),
            Self::SE => (1, 1),
            Self::S => (0, 1),
            Self::SW => (-1, 1),
            Self::W => (-1, 0),
        }
    }

    /// Every variant — used by the scene builder to emit one handle
    /// per side and by tests to enumerate the cardinality.
    pub fn all() -> [Self; 8] {
        [
            Self::NW,
            Self::N,
            Self::NE,
            Self::E,
            Self::SE,
            Self::S,
            Self::SW,
            Self::W,
        ]
    }

    /// Resolve a cumulative cursor delta into the new
    /// `(offset, size)` after applying this side's axis factors
    /// to a starting AABB. Pure function — both per-frame drains
    /// (node + section resize) and the release-commit arms call
    /// this so the resize math has one canonical source.
    ///
    /// Coordinate convention. W / N / NW / NE / SW shift the
    /// offset toward the cursor and shrink the size by the same
    /// amount, so the opposite edge stays put. E / S / SE only
    /// grow the size; offset stays at `start_offset`.
    pub fn resolve_aabb(
        &self,
        start_offset: crate::mindmap::model::Position,
        start_size: crate::mindmap::model::Size,
        total_delta: Vec2,
    ) -> (crate::mindmap::model::Position, crate::mindmap::model::Size) {
        let (fx, fy) = self.axis_factors();
        let dx = total_delta.x as f64;
        let dy = total_delta.y as f64;
        // `axis_factors()` is exhaustively pinned to {-1, 0, 1}; the
        // wildcard arm is mathematically dead. Per CODE_CONVENTIONS §9
        // (no panic in interactive paths) we fall through to the
        // "no axis movement" identity rather than `unreachable!()` —
        // this code runs on every resize-drag drain, and a bug
        // upstream that ever produced an out-of-range factor would
        // otherwise crash the gesture mid-drag.
        let (off_x, size_w) = match fx {
            -1 => (start_offset.x + dx, start_size.width - dx),
            1 => (start_offset.x, start_size.width + dx),
            _ => (start_offset.x, start_size.width),
        };
        let (off_y, size_h) = match fy {
            -1 => (start_offset.y + dy, start_size.height - dy),
            1 => (start_offset.y, start_size.height + dy),
            _ => (start_offset.y, start_size.height),
        };
        (
            crate::mindmap::model::Position { x: off_x, y: off_y },
            crate::mindmap::model::Size {
                width: size_w,
                height: size_h,
            },
        )
    }

    /// Stable per-side channel for the in-place mutator dispatch.
    /// Picked to avoid colliding with `edge_handle_channel_for`'s
    /// 1/2/3/100+ space — sections live in a separate canvas role
    /// so the spaces don't actually overlap, but a unique channel
    /// per side here is cheap and easy to reason about.
    pub fn channel(&self) -> usize {
        match self {
            Self::NW => 1,
            Self::N => 2,
            Self::NE => 3,
            Self::E => 4,
            Self::SE => 5,
            Self::S => 6,
            Self::SW => 7,
            Self::W => 8,
        }
    }
}

/// Pick a corner anchor for a fast-resize gesture from the cursor's
/// position within an AABB. Returns one of the four corner sides
/// (`NW` / `NE` / `SW` / `SE`). Edge handles (`N` / `E` / `S` / `W`)
/// are deliberately not picked here — single-axis resize is a
/// finer-grained operation that needs explicit Resize-mode handle
/// targeting; the fast-resize gesture is "grab a corner from
/// anywhere in this body".
///
/// Quadrant boundaries are split at the AABB center. A cursor
/// exactly on the centerline rounds south and east (`>=` on both
/// axes). Tested as a pure function — no GPU, no scene state.
pub fn infer_resize_anchor(cursor_canvas: Vec2, aabb_pos: Vec2, aabb_size: Vec2) -> ResizeHandleSide {
    let center = aabb_center(aabb_pos, aabb_size);
    let east = cursor_canvas.x >= center.x;
    let south = cursor_canvas.y >= center.y;
    match (east, south) {
        (false, false) => ResizeHandleSide::NW,
        (true, false) => ResizeHandleSide::NE,
        (false, true) => ResizeHandleSide::SW,
        (true, true) => ResizeHandleSide::SE,
    }
}

impl fmt::Display for ResizeHandleSide {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::NW => "nw",
            Self::N => "n",
            Self::NE => "ne",
            Self::E => "e",
            Self::SE => "se",
            Self::S => "s",
            Self::SW => "sw",
            Self::W => "w",
        };
        f.write_str(s)
    }
}

/// One resize-handle glyph emitted on top of a selected section.
/// Rendered as a small cosmic-text buffer in canvas space — the
/// renderer treats `section_resize_handles` as its own buffer
/// family since the handle set is small (8) and only exists for
/// the currently-selected `Some`-sized section.
pub struct SectionResizeHandleElement {
    /// Owning MindNode id — same id every per-node element keys on.
    pub node_id: String,
    /// Index into [`MindNode.sections`](crate::mindmap::model::MindNode::sections).
    pub section_idx: usize,
    /// Which of the 8 handles this element represents.
    pub side: ResizeHandleSide,
    /// Canvas-space center of the handle.
    pub position: (f32, f32),
    /// Glyph string (single char).
    pub glyph: String,
    /// Color as `#RRGGBB` hex.
    pub color: String,
    /// Font size in points.
    pub font_size_pt: f32,
}

impl crate::mindmap::tree_builder::HandleVisual for SectionResizeHandleElement {
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
        self.side.channel()
    }
}

/// Glyph used for section resize handles. A small open square reads
/// distinctly from the edge-handle diamond / midpoint hook so the
/// two families are visually disambiguated when they coexist on a
/// crowded screen.
pub const SECTION_RESIZE_HANDLE_GLYPH: &str = "\u{25A1}"; // □

/// Font size (in points) for the section resize-handle glyphs.
/// Same size as the edge handles — the two families share visual
/// weight so the eye reads them as a unified "grab here" idiom.
pub const SECTION_RESIZE_HANDLE_FONT_SIZE_PT: f32 = 14.0;

/// The 8 resize-handle positions around an AABB. Returns `None`
/// when the size is non-finite or non-positive — no meaningful
/// handles can be drawn. Single source of truth for the position
/// layout shared by node and section resize-handle builders.
pub fn resize_handle_positions(pos: Vec2, size: Vec2) -> Option<[(ResizeHandleSide, (f32, f32)); 8]> {
    if !size.x.is_finite() || !size.y.is_finite() || size.x <= 0.0 || size.y <= 0.0 {
        return None;
    }
    let (x, y) = (pos.x, pos.y);
    let (w, h) = (size.x, size.y);
    let center = aabb_center(pos, size);
    let (cx, cy) = (center.x, center.y);
    let right = x + w;
    let bottom = y + h;
    Some([
        (ResizeHandleSide::NW, (x, y)),
        (ResizeHandleSide::N, (cx, y)),
        (ResizeHandleSide::NE, (right, y)),
        (ResizeHandleSide::E, (right, cy)),
        (ResizeHandleSide::SE, (right, bottom)),
        (ResizeHandleSide::S, (cx, bottom)),
        (ResizeHandleSide::SW, (x, bottom)),
        (ResizeHandleSide::W, (x, cy)),
    ])
}

/// Build the 8-handle set for a single selected section, given the
/// section's already-resolved (offset-applied) canvas-space AABB.
/// Returns an empty vector for `None` (fill-parent) or non-finite /
/// non-positive sizes.
pub fn build_section_resize_handles(
    node_id: &str,
    section_idx: usize,
    section_pos: Vec2,
    section_size: Option<Vec2>,
) -> Vec<SectionResizeHandleElement> {
    let Some(size) = section_size else { return Vec::new() };
    let Some(positions) = resize_handle_positions(section_pos, size) else {
        return Vec::new();
    };
    positions
        .into_iter()
        .map(|(side, position)| SectionResizeHandleElement {
            node_id: node_id.to_string(),
            section_idx,
            side,
            position,
            glyph: SECTION_RESIZE_HANDLE_GLYPH.to_string(),
            color: SELECTION_HIGHLIGHT_HEX.to_string(),
            font_size_pt: SECTION_RESIZE_HANDLE_FONT_SIZE_PT,
        })
        .collect()
}

/// Resolve `selected_section` into the section's canvas-space
/// `(position, size)` and dispatch to
/// [`build_section_resize_handles`]. Returns `Vec::new()` when no
/// section is selected, the named node / section is missing or
/// hidden by fold, or the section is `None`-sized (fill-parent —
/// there is no per-section AABB to stretch, so the builder itself
/// also returns empty).
///
/// The selection gate lives here rather than at the call site so
/// every rebuild path applies the same rule.
///
/// # Costs
///
/// O(1) — one hash lookup plus, at most, the eight-element
/// allocation [`build_section_resize_handles`] returns.
pub fn build_selected_section_handles(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    selected_section: Option<(&str, usize)>,
    hidden_set: &HashSet<&str>,
) -> Vec<SectionResizeHandleElement> {
    let Some((node_id, section_idx)) = selected_section else {
        return Vec::new();
    };
    let Some(node) = map.nodes.get(node_id) else {
        return Vec::new();
    };
    if hidden_set.contains(node.id.as_str()) {
        return Vec::new();
    }
    let Some(section) = node.sections.get(section_idx) else {
        return Vec::new();
    };
    let (ox, oy) = offsets.get(&node.id).copied().unwrap_or((0.0, 0.0));
    let node_pos = node.pos_vec2();
    let section_pos = Vec2::new(
        node_pos.x + ox + section.offset.x as f32,
        node_pos.y + oy + section.offset.y as f32,
    );
    let section_size = section
        .size
        .as_ref()
        .map(|s| Vec2::new(s.width as f32, s.height as f32));
    build_section_resize_handles(node_id, section_idx, section_pos, section_size)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pin every variant's axis factors. Drift on any one of these
    /// produces a silent off-by-sign bug in the resize drain — the
    /// section grows when it should shrink, or vice versa.
    #[test]
    fn axis_factors_pin_per_side() {
        assert_eq!(ResizeHandleSide::NW.axis_factors(), (-1, -1));
        assert_eq!(ResizeHandleSide::N.axis_factors(), (0, -1));
        assert_eq!(ResizeHandleSide::NE.axis_factors(), (1, -1));
        assert_eq!(ResizeHandleSide::E.axis_factors(), (1, 0));
        assert_eq!(ResizeHandleSide::SE.axis_factors(), (1, 1));
        assert_eq!(ResizeHandleSide::S.axis_factors(), (0, 1));
        assert_eq!(ResizeHandleSide::SW.axis_factors(), (-1, 1));
        assert_eq!(ResizeHandleSide::W.axis_factors(), (-1, 0));
    }

    #[test]
    fn all_returns_eight_unique_variants() {
        let all = ResizeHandleSide::all();
        assert_eq!(all.len(), 8);
        for i in 0..all.len() {
            for j in (i + 1)..all.len() {
                assert_ne!(all[i], all[j], "duplicate variant in all()");
            }
        }
    }

    #[test]
    fn channel_is_unique_per_side() {
        let mut seen = std::collections::HashSet::new();
        for s in ResizeHandleSide::all() {
            assert!(seen.insert(s.channel()), "duplicate channel for {:?}", s);
        }
    }

    /// `None`-sized sections (fill-parent) get no handles.
    #[test]
    fn build_returns_empty_for_none_sized_section() {
        let handles = build_section_resize_handles("0", 0, Vec2::ZERO, None);
        assert!(handles.is_empty());
    }

    /// `Some`-sized sections get exactly 8 handles, one per side.
    #[test]
    fn build_emits_eight_handles_for_some_sized_section() {
        let handles =
            build_section_resize_handles("0", 1, Vec2::new(10.0, 20.0), Some(Vec2::new(100.0, 50.0)));
        assert_eq!(handles.len(), 8);
        let mut sides: Vec<ResizeHandleSide> = handles.iter().map(|h| h.side).collect();
        sides.sort_by_key(|s| s.channel());
        let mut expected: Vec<ResizeHandleSide> = ResizeHandleSide::all().to_vec();
        expected.sort_by_key(|s| s.channel());
        assert_eq!(sides, expected);
    }

    /// Pin the four corner positions and the four edge-mid
    /// positions for a fixed section AABB. Drift here would shift
    /// every handle visually by the same amount — easy to catch.
    #[test]
    fn build_handle_positions_match_section_aabb() {
        let handles =
            build_section_resize_handles("0", 0, Vec2::new(10.0, 20.0), Some(Vec2::new(100.0, 40.0)));
        let by_side: std::collections::HashMap<ResizeHandleSide, (f32, f32)> =
            handles.iter().map(|h| (h.side, h.position)).collect();
        // Corners.
        assert_eq!(by_side[&ResizeHandleSide::NW], (10.0, 20.0));
        assert_eq!(by_side[&ResizeHandleSide::NE], (110.0, 20.0));
        assert_eq!(by_side[&ResizeHandleSide::SW], (10.0, 60.0));
        assert_eq!(by_side[&ResizeHandleSide::SE], (110.0, 60.0));
        // Edge mids.
        assert_eq!(by_side[&ResizeHandleSide::N], (60.0, 20.0));
        assert_eq!(by_side[&ResizeHandleSide::S], (60.0, 60.0));
        assert_eq!(by_side[&ResizeHandleSide::W], (10.0, 40.0));
        assert_eq!(by_side[&ResizeHandleSide::E], (110.0, 40.0));
    }

    #[test]
    fn display_uses_lowercase_compass_strings() {
        assert_eq!(format!("{}", ResizeHandleSide::NW), "nw");
        assert_eq!(format!("{}", ResizeHandleSide::E), "e");
        assert_eq!(format!("{}", ResizeHandleSide::SE), "se");
    }

    #[test]
    fn infer_resize_anchor_picks_quadrant_corner() {
        // 100×80 AABB at (10, 20) → center (60, 60).
        let pos = Vec2::new(10.0, 20.0);
        let size = Vec2::new(100.0, 80.0);
        // Strictly NW of center.
        assert_eq!(
            infer_resize_anchor(Vec2::new(20.0, 30.0), pos, size),
            ResizeHandleSide::NW
        );
        // Strictly NE.
        assert_eq!(
            infer_resize_anchor(Vec2::new(100.0, 30.0), pos, size),
            ResizeHandleSide::NE
        );
        // Strictly SW.
        assert_eq!(
            infer_resize_anchor(Vec2::new(20.0, 90.0), pos, size),
            ResizeHandleSide::SW
        );
        // Strictly SE.
        assert_eq!(
            infer_resize_anchor(Vec2::new(100.0, 90.0), pos, size),
            ResizeHandleSide::SE
        );
    }

    #[test]
    fn infer_resize_anchor_center_rounds_south_east() {
        // Cursor exactly on the AABB center — tie-breaker rounds
        // SE (the `>=` comparison on both axes). The convention
        // matters only for cursor-on-center; in practice the press
        // is almost never exactly at center, but pinning the
        // tie-break behavior locks the code path.
        let pos = Vec2::new(0.0, 0.0);
        let size = Vec2::new(100.0, 100.0);
        assert_eq!(
            infer_resize_anchor(Vec2::new(50.0, 50.0), pos, size),
            ResizeHandleSide::SE
        );
    }

    #[test]
    fn infer_resize_anchor_off_aabb_still_resolves_a_quadrant() {
        // The function doesn't bounds-check; a cursor outside the
        // AABB still resolves to whichever quadrant the center-
        // relative coords land in. Keeps the function pure and
        // composable with off-AABB hit-tests downstream.
        let pos = Vec2::new(0.0, 0.0);
        let size = Vec2::new(10.0, 10.0);
        assert_eq!(
            infer_resize_anchor(Vec2::new(-100.0, -100.0), pos, size),
            ResizeHandleSide::NW
        );
        assert_eq!(
            infer_resize_anchor(Vec2::new(1000.0, 1000.0), pos, size),
            ResizeHandleSide::SE
        );
    }
}
