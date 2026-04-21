//! Hit-testing for the picker — classify a screen-space `(x, y)`
//! cursor position against a cached `ColorPickerLayout`. Used by
//! the mouse-move and click handlers in `app.rs`; pure function,
//! no state.

use baumhard::gfx_structs::shape::NodeShape;
use glam::Vec2;

use super::glyph_tables::CROSSHAIR_CENTER_CELL;
use super::layout::ColorPickerLayout;

/// Result of a hit test against a `ColorPickerLayout`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerHit {
    /// Index into `hue_slot_positions`.
    Hue(usize),
    /// Index into `sat_cell_positions`.
    SatCell(usize),
    /// Index into `val_cell_positions`.
    ValCell(usize),
    /// The center preview glyph — clicking commits the current HSV
    /// (Contextual: to the bound handle; Standalone: to the
    /// current document selection).
    Commit,
    /// Inside the wheel's circle but not on any interactive element.
    /// Mouse-down here starts a wheel drag; drag ends on mouse-up.
    /// Replaces the older `Inside` fallback — every inside-but-
    /// not-glyph region is now a drag anchor by design.
    DragAnchor,
    /// Outside the wheel's circular outer bound. The wheel visually
    /// reads as a disk of radius `outer_radius`, so the outer gate
    /// matches that disk exactly — corner regions of the (still
    /// rectangular) backdrop chrome are treated as canvas, not
    /// picker.
    Outside,
}

/// Hit-test a screen position against the cached picker layout. The
/// outer gate is the wheel's circle (center, `outer_radius`), shared
/// with the rest of the codebase via
/// [`NodeShape::Ellipse::contains_local`] — the picker fits the same
/// shape abstraction as any ellipse-shaped node. Inside that circle
/// the search order matches the visual layering: val bar → sat bar →
/// hue ring → center preview glyph (commit). Inside-the-circle-
/// but-not-on-any-glyph is the drag anchor for the wheel. Returns
/// `Outside` when the cursor is past the circle.
pub fn hit_test_picker(layout: &ColorPickerLayout, x: f32, y: f32) -> PickerHit {
    // Outer gate: the wheel is a circle centered on `layout.center`
    // with radius `layout.outer_radius` — the same disk the hue ring
    // glyphs sit inside of (their outer edge is exactly
    // `ring_r + ring_font_size * 0.5 == outer_radius`). Route through
    // `NodeShape::Ellipse` so every circular hit-test in the app
    // shares one implementation; a future tweak to the ellipse math
    // propagates here for free. `outer_radius <= 0.0` (degenerate
    // layout in a 0-px viewport) falls through the `<= 0.0` guard
    // inside `contains_local` and reports `Outside`.
    let r = layout.outer_radius;
    let bounds = Vec2::new(2.0 * r, 2.0 * r);
    let local = Vec2::new(
        x - (layout.center.0 - r),
        y - (layout.center.1 - r),
    );
    if !NodeShape::Ellipse.contains_local(local, bounds) {
        return PickerHit::Outside;
    }

    // Sat/val bars: pick the closer of the two if the cursor is inside
    // the inner cross region. Cell tolerance scales with the actual
    // per-cell advance so denser bars (smaller cell_advance on small
    // windows) have proportionally smaller hit boxes.
    let cell_half = (layout.cell_advance * 0.5).max(layout.font_size * 0.4);

    // Sat (horizontal) bar — only consider when cursor is vertically
    // close to the bar line. Skip the center cell so a click at the
    // wheel center resolves to `Commit` (the ࿕ button) below.
    if (y - layout.center.1).abs() <= cell_half {
        for (i, (cx, _)) in layout.sat_cell_positions.iter().enumerate() {
            if i == CROSSHAIR_CENTER_CELL {
                continue;
            }
            if (x - cx).abs() <= cell_half {
                return PickerHit::SatCell(i);
            }
        }
    }
    // Val (vertical) bar — same in the other axis.
    if (x - layout.center.0).abs() <= cell_half {
        for (i, (_, cy)) in layout.val_cell_positions.iter().enumerate() {
            if i == CROSSHAIR_CENTER_CELL {
                continue;
            }
            if (y - cy).abs() <= cell_half {
                return PickerHit::ValCell(i);
            }
        }
    }

    // Hue ring — annular hit. Only the slot whose glyph contains the
    // cursor counts (ring-scaled tolerance), so the empty space inside
    // the ring stays inert.
    let ring_half = layout.ring_font_size * 0.5;
    for (i, (px, py)) in layout.hue_slot_positions.iter().enumerate() {
        let dx = x - px;
        let dy = y - py;
        if dx * dx + dy * dy <= ring_half * ring_half {
            return PickerHit::Hue(i);
        }
    }

    // Center ࿕ — the commit button. Circular hit of radius
    // `preview_size * 0.45` (slightly smaller than the glyph box so
    // users who click in the padding between the ࿕ and the crosshair
    // arm glyphs don't accidentally commit).
    let commit_radius = layout.preview_size * 0.45;
    let dx = x - layout.center.0;
    let dy = y - layout.center.1;
    if dx * dx + dy * dy <= commit_radius * commit_radius {
        return PickerHit::Commit;
    }

    // Anywhere else inside the wheel disk is the drag anchor — the
    // hex readout band falls naturally outside the circle now, so a
    // click on the hex text lands in `Outside` rather than spinning
    // the wheel. That matches the visual affordance (the hex is a
    // display element, not a drag surface).
    PickerHit::DragAnchor
}
