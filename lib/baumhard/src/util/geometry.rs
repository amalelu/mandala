// SPDX-License-Identifier: MPL-2.0

//! Small-scale 2D geometry helpers: rotation around a pivot,
//! epsilon-aware float comparisons, and pixel-space ordering.

use glam::{Mat3, Vec2};

/// Rotate `a` clockwise by `degrees` around `pivot`, returning the
/// transformed point. Uses `glam::Mat3::from_rotation_z` internally;
/// O(1).
pub fn clockwise_rotation_around_pivot(a: Vec2, pivot: Vec2, degrees: f32) -> Vec2 {
    let translated = a - pivot;

    let radians = -degrees.to_radians();
    let rotation = Mat3::from_rotation_z(radians);

    rotation.transform_point2(translated) + pivot
}

const ERROR_TOLERANCE_ALMOST_EQUAL: f32 = 1e-5;

/// `|a - b| <= 1e-5`. The baumhard-wide epsilon for "close enough"
/// between two `f32`s. Cost: O(1).
pub fn almost_equal(a: f32, b: f32) -> bool {
    (a - b).abs() <= ERROR_TOLERANCE_ALMOST_EQUAL
}

/// `|a - b| <= 1e-9`. The f64 counterpart of [`almost_equal`], for
/// the model-space coordinates that are `f64` — `MindNode.position`,
/// `MindNode.size`, `MindSection.offset` — where "did this land
/// back where it started?" is the question and raw `==` answers it
/// only by luck.
///
/// The tolerance is chosen from both ends. Below it: an f64 ULP at
/// canvas magnitudes is around `1e-13`, so `1e-9` absorbs a few
/// thousand roundings of accumulated translate / rotate arithmetic
/// without ever calling two genuinely different coordinates equal.
/// Above it: `1e-9` canvas units is nine decades below one pixel,
/// so nothing this predicate treats as equal could be drawn apart.
/// Deliberately tighter than [`almost_equal`]'s `1e-5`, which is
/// sized for `f32`'s far coarser mantissa.
///
/// Cost: O(1).
pub fn almost_equal_f64(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9
}

/// `true` iff `f` is a non-NaN, non-infinite, strictly-positive
/// `f32`. The canonical predicate for "is this a valid pixel /
/// zoom / scale / font-size value coming from user input?" —
/// rejects `NaN`, `±∞`, and zero-or-negative numbers.
///
/// Used on the parse → mutation boundary (every console verb
/// that accepts a numeric pt size, every edge-style setter
/// that accepts a positive measurement). Cost: O(1).
pub fn is_positive_finite(f: f32) -> bool {
    f.is_finite() && f > 0.0
}

/// `true` iff `f` is a non-NaN, non-infinite, non-negative
/// `f64`. The canonical predicate for "is this a coordinate /
/// size safe to feed into layout math?" — rejects `NaN`,
/// `±∞`, and negative numbers, but allows zero (a zero-width
/// node or zero-coord position is a valid layout input).
///
/// Used by `document::mutations::*` layout helpers that walk
/// `MindNode.size` / `position` (both `f64`) before applying
/// derived math; a non-finite or negative coord fed into
/// trigonometry produces NaN that propagates into every
/// downstream row, so the safe-coord check stops the cascade
/// at the source. Cost: O(1).
pub fn is_non_negative_finite_f64(f: f64) -> bool {
    f.is_finite() && f >= 0.0
}

/// Logical inverse of [`almost_equal`]. Named "pretty" because it
/// treats within-epsilon pairs as equal rather than fighting with raw
/// `!=` comparisons at float boundaries. Cost: O(1).
pub fn pretty_inequal(a: f32, b: f32) -> bool {
    !almost_equal(a, b)
}

/// Pixel-reading-order `>=` on `(x, y)` pairs: y-dominant, x as
/// tie-breaker, using [`almost_equal`] for the equality test.
/// Cost: O(1).
pub fn pixel_greater_or_equal(a_greater_or: (f32, f32), equal_b: (f32, f32)) -> bool {
    pixel_greater_than(a_greater_or, equal_b)
        || (almost_equal(a_greater_or.0, equal_b.0) && almost_equal(a_greater_or.1, equal_b.1))
}

/// Pixel-reading-order `>` on `(x, y)` pairs: if the y components are
/// almost-equal, compare x; otherwise compare y. Matches how a cursor
/// walks a page of glyphs. Cost: O(1).
pub fn pixel_greater_than(a_greater: (f32, f32), than_b: (f32, f32)) -> bool {
    if almost_equal(a_greater.1, than_b.1) {
        a_greater.0 > than_b.0
    } else {
        a_greater.1 > than_b.1
    }
}

/// Pixel-reading-order `<=` on `(x, y)` pairs. Mirror of
/// [`pixel_greater_or_equal`]. Cost: O(1).
pub fn pixel_less_or_equal(a_less_or: (f32, f32), equal_b: (f32, f32)) -> bool {
    pixel_lesser_than(a_less_or, equal_b)
        || (almost_equal(a_less_or.0, equal_b.0) && almost_equal(a_less_or.1, equal_b.1))
}

/// Pixel-reading-order `<` on `(x, y)` pairs. Mirror of
/// [`pixel_greater_than`]. Cost: O(1).
pub fn pixel_lesser_than(a_lesser: (f32, f32), than_b: (f32, f32)) -> bool {
    if almost_equal(a_lesser.1, than_b.1) {
        a_lesser.0 < than_b.0
    } else {
        a_lesser.1 < than_b.1
    }
}

/// Area of the rectangle whose width / height are the x / y
/// components of `vec`. O(1).
pub fn vec2_area(vec: Vec2) -> f32 {
    vec.x * vec.y
}

/// AABB center from a top-left position + size pair. O(1), no
/// heap. Equivalent to [`crate::mindmap::model::MindNode::center_vec2`]
/// for the case where only the geometry is in scope (anchor
/// resolution paths, scene-builder portal-pair midpoint compute);
/// where a `MindNode` is in scope, prefer the method.
///
/// **Finding the ones that got away.** Every open-coded `pos + size *
/// 0.5` in the workspace routes through this function or through
/// `center_vec2`, and the census that says so has to be re-run with a
/// pattern whose operand class admits tuple indices and casts —
/// `[a-zA-Z_.]+` matches neither `.0` (digits) nor ` as f32 `
/// (spaces), and a sweep verified with it reported "done" while six
/// sites stood:
///
/// ```text
/// grep -rnE "\+ *[A-Za-z0-9_.()]+( as f32)? *(\* *0\.5|/ *2\.0)" --include=*.rs src lib crates \
///   | grep -vE "aabb_center|center_vec2|pos \+ size|pos_vec2"
/// ```
///
/// Widening the operand class was not enough on its own: the first
/// re-run kept the operator as `* 0.5` and reported "done" while
/// three `/ 2.0` centers stood in a file the same series had already
/// edited twice. The two spellings are bit-identical for f32, so the
/// census has to admit both — which is why the pattern above is
/// written as an alternation rather than tightened again.
///
/// What that leaves is *not* AABB centers and is not this helper's:
/// the color picker's scalar layout (radii, ring sums, and the
/// `preview_pos + preview_size * 0.5` the picker computes without
/// ever naming a `Vec2`), deliberately off-center probe points, and
/// half-extent terms inside larger anchor formulas. It also leaves
/// the model-space centers in `document::mutations` (`flower_layout`,
/// `tree_cascade`), which this helper cannot express at all:
/// [`crate::mindmap::model::Position`] and
/// [`crate::mindmap::model::Size`] are `f64`, and routing them
/// through a `Vec2` would round the layout arithmetic to `f32` to
/// save a multiplication. The other
/// midpoint *form* — `(min + max) * 0.5` over two corners rather
/// than a corner and an extent — is a different signature, so it does
/// not appear here at all and this helper does not cover it
/// (the app's `touch_gesture::midpoint`, [`crate::font::fonts`]'s
/// `InkBounds::x_center` / `y_center`, and the `(r.min + r.max) * 0.5`
/// probe in the canvas-hit tests).
#[inline]
pub fn aabb_center(pos: Vec2, size: Vec2) -> Vec2 {
    pos + size * 0.5
}

/// Whether `point` lies inside the axis-aligned box `[min, max]`,
/// **boundary included**. The one closed-interval point-in-AABB
/// test in the project: the BVH descent prunes with it, the scene
/// picks entries with it, the rectangle shape answers
/// `contains_local` with it, and the app's node hit-test asks it
/// twice per node.
///
/// Closed rather than half-open because every one of those callers
/// is answering "did the user click this?", and a click landing
/// exactly on a node's right or bottom edge has to hit the node
/// rather than fall through to whatever is beneath it. The
/// deliberately *strict* epsilon-inset variants — the connection
/// path's `< f32::EPSILON` degenerate-length guards — are asking a
/// different question and stay as they are.
///
/// `max` below `min` on either axis contains nothing, which is the
/// answer a degenerate box should give and needs no separate guard.
///
/// **`NaN` anywhere means "contains nothing"**, for the same reason
/// and with no guard either: every comparison is `>=` / `<=`, and
/// both are false against `NaN`. That is a deliberate difference
/// from the four-way *disjunction* this replaced at
/// [`crate::gfx_structs::tree_walker`]'s BVH prune, which spelled
/// the same question as `x < min.x || x > max.x || …` and so
/// answered "outside is false, descend" for a `NaN` box — walking a
/// subtree whose bounds are not a box. No reachable `NaN` was found
/// on the way in (`serde_json` rejects the literals,
/// `renderable_section` screens non-finite section geometry,
/// mutation payloads are finiteness-screened, and
/// `clamp_canvas_coord` maps `NaN` to 0.0), so this is not a bug
/// fixed but an answer chosen: a box with a `NaN` bound contains
/// nothing, and the prune drops it.
///
/// Cost: four comparisons, O(1), no heap. Carries no `#[inline]`,
/// and the honest reason is §B7's rather than an absence of effect:
/// a *new* `#[inline]` needs a benchmark that resolves it, and
/// `AGENTS.md` forbids an agent from running one, so omission is the
/// only compliant outcome. There would be something to resolve —
/// this is `pub` with a cross-crate caller (`mandala`'s
/// `document::hit_test::point_in_node_aabb`) and the workspace
/// release profile keeps `codegen-units = 16` with thin-local LTO,
/// which by the root `Cargo.toml`'s own words leaves cross-crate
/// inlining on the table. The stakes are small — the genuinely hot
/// caller, `bvh_find`, is in-crate, and the cross-crate one runs per
/// pointer event rather than per glyph — but "small" is a guess and
/// the rule is not.
pub fn aabb_contains(point: Vec2, min: Vec2, max: Vec2) -> bool {
    point.x >= min.x && point.x <= max.x && point.y >= min.y && point.y <= max.y
}

/// Component-wise [`pretty_inequal`] on two vectors: true if either
/// component pair is outside the `almost_equal` tolerance.
/// Cost: O(1).
pub fn pretty_inequal_vec2(vec1: Vec2, vec2: Vec2) -> bool {
    pretty_inequal(vec1.x, vec2.x) || pretty_inequal(vec1.y, vec2.y)
}

/// Component-wise [`almost_equal`] on two vectors: true iff both
/// component pairs are within tolerance. Cost: O(1).
pub fn almost_equal_vec2(vec1: Vec2, vec2: Vec2) -> bool {
    almost_equal(vec1.x, vec2.x) && almost_equal(vec1.y, vec2.y)
}

/// Option-aware [`almost_equal`]: `(None, None) -> true`,
/// `(Some(a), Some(b)) -> almost_equal(a, b)`, mismatched-tag
/// pairs (`Some` vs `None`) -> false.
///
/// Used by per-axis edge / portal-label setters to short-circuit
/// "no-op writes" when the user re-types a value that's
/// within-epsilon of the stored one. Centralised here so the
/// setter no-op contract is single-sourced across edges/style.rs,
/// edge_label_drag, portal_label_drag, and the connection cache's
/// font-size diff. Cost: O(1).
pub fn option_almost_equal(a: Option<f32>, b: Option<f32>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => almost_equal(x, y),
        _ => false,
    }
}
