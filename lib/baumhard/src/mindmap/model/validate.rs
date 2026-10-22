// SPDX-License-Identifier: MPL-2.0

//! Shared validation for the mindmap model's **numeric domain** —
//! `MindNode` / `MindSection` geometry, and every authored number a
//! `.mindmap.json` can carry into the scene build.
//!
//! Kept in the model crate so application setters, the loader, and
//! `maptool verify` reject the same shapes with the same messages.
//!
//! **Why the domain is checked at all.** A `.mindmap.json` is
//! untrusted input (`format/macros.md` §"Threat model"), and the
//! consumers of these numbers are not defensive: cosmic-text
//! asserts that a line height is non-zero, `f32::clamp` panics when
//! its bounds are inverted, and the border and connection builders
//! size a `String` or a `Vec` from authored geometry. A field that
//! is merely *typed* `f32` therefore reaches an `assert!` or an
//! allocator with whatever the file said — including `NaN`,
//! infinity, zero, and 1e30. None of those are recoverable at the
//! point they land: an allocation failure and a stack-exhaustion
//! abort are not panics, so CODE_CONVENTIONS §9's "degrade the
//! frame, log, keep running" has nothing to degrade.
//!
//! So the domain is enforced **once, at the trust boundary**, and
//! the model behind it carries only numbers the renderer can
//! survive. The loader rejects rather than repairs, matching the
//! posture it already takes for structure (zero sections, parent
//! cycles, the section cap): a map that would crash the editor does
//! not open, and the message names the part to fix.

use std::collections::HashMap;

use crate::mindmap::custom_mutation::CustomMutation;
use crate::util::grapheme_chad::count_grapheme_clusters;

use super::{
    Canvas, ControlPoint, EdgeLabelConfig, GlyphBorderConfig, GlyphConnectionConfig, MindEdge, MindMap,
    MindNode, MindSection, PortalEndpointState, Position, Size, MAX_NODE_AXIS, MAX_SECTIONS_PER_NODE,
};

/// The font-metric domain, defined beside the text shaper that
/// enforces it (`crate::font::fonts`) and re-exported here so the
/// document trust boundary and the runtime mutation path cannot
/// drift to two different numbers.
pub use crate::font::fonts::{MAX_FONT_SIZE_PT, MIN_FONT_SIZE_PT};

/// Ceiling on the magnitude of an authored canvas coordinate —
/// node positions, section offsets, and Bezier control points.
///
/// Coordinates are signed and nodes float freely, so this bounds
/// `|v|` rather than `v`. The number is deliberately far looser
/// than [`MAX_NODE_AXIS`]: its job is to reject `NaN`, infinity,
/// and values that make a *difference* between two coordinates
/// overflow a sample count, not to have an opinion about how large
/// a canvas may be. The derived counts carry their own caps —
/// see `connection::bezier` and `mindmap::border`.
pub const MAX_CANVAS_COORD: f64 = 1.0e9;

/// Bring a computed canvas coordinate inside [`MAX_CANVAS_COORD`].
///
/// The `f64` sibling of [`clamp_to_bound`], for `Position`, whose
/// components are `f64` rather than `f32`.
///
/// **Clamped rather than rejected, unlike the setter path.** A
/// *derived* position — a layout placing a child at
/// `parent + radius`, a cascade repositioning a subtree — has no
/// user input to decline. Both take in-bound inputs and can compute
/// an out-of-bound result, and refusing there would silently drop a
/// node out of its layout; pinning it at the boundary keeps the
/// document loadable and the node visible. `set_node_aabb`, which
/// takes a position straight from the caller, still rejects.
///
/// A non-finite input resolves to `0.0`: a computed `NaN` has no
/// nearest edge, and every downstream comparison against it is
/// false, so the node would silently never render.
///
/// Cost: O(1), no allocation.
pub fn clamp_canvas_coord(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    value.clamp(-MAX_CANVAS_COORD, MAX_CANVAS_COORD)
}

/// Validate a node size candidate, returning the first violation.
pub fn node_size(size: Size) -> Result<(), String> {
    first_or_ok(node_size_violations(size))
}

/// All node-size violations for callers that report rather than
/// reject, such as `maptool verify`.
pub fn node_size_violations(size: Size) -> Vec<String> {
    let mut out = Vec::new();
    if !size.width.is_finite() || !size.height.is_finite() {
        out.push(format!(
            "node.size has non-finite component (width={}, height={})",
            size.width, size.height
        ));
    }
    if size.width.is_finite() && size.width <= 0.0 {
        out.push(format!("node.size.width is not positive ({})", size.width));
    }
    if size.height.is_finite() && size.height <= 0.0 {
        out.push(format!("node.size.height is not positive ({})", size.height));
    }
    if size.width.is_finite() && size.width > MAX_NODE_AXIS {
        out.push(format!(
            "node.size.width ({}) exceeds the {} ceiling; likely a typo (e.g. an extra zero)",
            size.width, MAX_NODE_AXIS
        ));
    }
    if size.height.is_finite() && size.height > MAX_NODE_AXIS {
        out.push(format!(
            "node.size.height ({}) exceeds the {} ceiling; likely a typo (e.g. an extra zero)",
            size.height, MAX_NODE_AXIS
        ));
    }
    out
}

/// Validate the section-count cap for one node.
pub fn section_count(node: &MindNode) -> Result<(), String> {
    if node.sections.len() > MAX_SECTIONS_PER_NODE {
        Err(format!(
            "node.sections.len()={} exceeds cap {}",
            node.sections.len(),
            MAX_SECTIONS_PER_NODE
        ))
    } else {
        Ok(())
    }
}

/// Validate one existing section's AABB against its parent node.
pub fn section_aabb(node: &MindNode, section_idx: usize) -> Result<(), String> {
    let Some(section) = node.sections.get(section_idx) else {
        return Ok(());
    };
    node_size(node.size)?;
    first_or_ok(section_aabb_violations(
        node.size,
        section_idx,
        section.offset,
        section.size,
    ))
}

/// Validate a candidate section AABB against a parent node size.
pub fn section_candidate_aabb(
    parent_size: Size,
    section_idx: usize,
    offset: Position,
    size: Option<Size>,
) -> Result<(), String> {
    node_size(parent_size)?;
    first_or_ok(section_aabb_violations(parent_size, section_idx, offset, size))
}

/// All section-AABB violations for callers that report rather than
/// reject, such as `maptool verify`.
pub fn section_aabb_violations(
    parent_size: Size,
    section_idx: usize,
    offset: Position,
    size: Option<Size>,
) -> Vec<String> {
    let mut out = section_extent_violations(parent_size, section_idx, offset, size);
    out.extend(section_containment_violations(
        parent_size,
        section_idx,
        offset,
        size,
    ));
    out
}

/// The half of [`section_aabb_violations`] about the section's own
/// shape: finite, positive, not an obvious typo. Split out because
/// two callers want different halves — the loader's numeric sweep
/// needs this one and not containment (a section hanging past its
/// node renders fine and is `maptool verify`'s call), while `verify`
/// needs containment and gets this half through the numeric sweep.
/// Sharing the body is what keeps them from reporting the same
/// defect in two sentences.
pub fn section_extent_violations(
    parent_size: Size,
    section_idx: usize,
    offset: Position,
    size: Option<Size>,
) -> Vec<String> {
    let mut out = Vec::new();
    if !offset.x.is_finite() || !offset.y.is_finite() {
        out.push(format!(
            "section[{}].offset has non-finite component (x={}, y={})",
            section_idx, offset.x, offset.y
        ));
    }
    if offset.x.is_finite() && offset.x < 0.0 {
        out.push(format!(
            "section[{}].offset.x is negative ({})",
            section_idx, offset.x
        ));
    }
    if offset.y.is_finite() && offset.y < 0.0 {
        out.push(format!(
            "section[{}].offset.y is negative ({})",
            section_idx, offset.y
        ));
    }
    if let Some(size) = size {
        if !size.width.is_finite() || !size.height.is_finite() {
            out.push(format!(
                "section[{}].size has non-finite component (width={}, height={})",
                section_idx, size.width, size.height
            ));
        }
        if size.width.is_finite() && size.width <= 0.0 {
            out.push(format!(
                "section[{}].size.width is not positive ({})",
                section_idx, size.width
            ));
        }
        if size.height.is_finite() && size.height <= 0.0 {
            out.push(format!(
                "section[{}].size.height is not positive ({})",
                section_idx, size.height
            ));
        }
        if parent_size.width.is_finite() && size.width.is_finite() && size.width > parent_size.width * 100.0 {
            out.push(format!(
                "section[{}].size.width ({}) is over 100× the node's width ({}); \
                 likely a typo (e.g. an extra zero)",
                section_idx, size.width, parent_size.width
            ));
        }
        if parent_size.height.is_finite()
            && size.height.is_finite()
            && size.height > parent_size.height * 100.0
        {
            out.push(format!(
                "section[{}].size.height ({}) is over 100× the node's height ({}); \
                 likely a typo (e.g. an extra zero)",
                section_idx, size.height, parent_size.height
            ));
        }
    }

    out
}

/// The half of [`section_aabb_violations`] about fitting inside the
/// parent node. Not load-blocking: a section that hangs past its
/// node's edge still renders, so `format/schema.md` assigns it to
/// `maptool verify`.
pub fn section_containment_violations(
    parent_size: Size,
    section_idx: usize,
    offset: Position,
    size: Option<Size>,
) -> Vec<String> {
    let mut out = Vec::new();
    let effective = size.unwrap_or(parent_size);
    if offset.x.is_finite()
        && offset.y.is_finite()
        && effective.width.is_finite()
        && effective.height.is_finite()
    {
        let right = offset.x + effective.width;
        let bottom = offset.y + effective.height;
        if right > parent_size.width {
            out.push(format!(
                "section[{}] extends past node right edge ({} > {})",
                section_idx, right, parent_size.width
            ));
        }
        if bottom > parent_size.height {
            out.push(format!(
                "section[{}] extends past node bottom edge ({} > {})",
                section_idx, bottom, parent_size.height
            ));
        }
    }
    out
}

/// Section-channel collision warnings for one node.
pub fn section_channel_collisions(node: &MindNode) -> Vec<String> {
    let mut by_channel: HashMap<usize, Vec<usize>> = HashMap::new();
    for (idx, section) in node.sections.iter().enumerate() {
        by_channel
            .entry(section.effective_channel(idx))
            .or_default()
            .push(idx);
    }

    let mut out = Vec::new();
    for (channel, indices) in by_channel {
        if indices.len() > 1 {
            out.push(format!(
                "channel {} shared by sections {:?}; mutations targeting that channel \
                 broadcast to all listed sections — usually unintentional",
                channel, indices
            ));
        }
    }
    out
}

fn first_or_ok(messages: Vec<String>) -> Result<(), String> {
    match messages.into_iter().next() {
        Some(message) => Err(message),
        None => Ok(()),
    }
}

// ---- Numeric-domain primitives ------------------------------------

/// What an inverted font-size window costs: the cascades hand the
/// pair to a clamp, and `f32::clamp` panics rather than saturating
/// when its bounds cross. The clamp sites order the pair defensively
/// (`font::fonts::clamp_to_font_window`), so this is the author-facing
/// half of a defense that no longer depends on it.
const INVERTED_SIZE_WINDOW: &str =
    "the size cascades clamp with this pair, and a crossed pair is not a window";

/// What an inverted zoom window costs: nothing crashes — the bounds
/// are two independent comparisons — but no zoom satisfies both, so
/// the element is invisible at every zoom level, which is rarely what
/// an author meant to write.
const INVERTED_ZOOM_WINDOW: &str = "no zoom satisfies both bounds, so this never renders at any zoom level";

/// Join a label prefix and a field name, tolerating an empty prefix
/// so a top-level field reads `min_zoom_to_render` rather than
/// `.min_zoom_to_render`. The prefix is the *document part* the
/// caller is inside; the loader stamps the node id or edge index
/// on top of whatever comes back.
fn field(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

/// `Some(message)` unless `value` is a font size the text shaper can
/// survive: finite and inside
/// [`MIN_FONT_SIZE_PT`]..=[`MAX_FONT_SIZE_PT`].
fn font_size(prefix: &str, name: &str, value: f32) -> Option<String> {
    if !value.is_finite() {
        return Some(format!("{} is not finite ({value})", field(prefix, name)));
    }
    if value < MIN_FONT_SIZE_PT {
        return Some(format!(
            "{} ({value}) is under the {MIN_FONT_SIZE_PT}pt floor — zero aborts the \
             text shaper, and a sub-point size asks the border fitter for millions of glyphs",
            field(prefix, name)
        ));
    }
    if value > MAX_FONT_SIZE_PT {
        return Some(format!(
            "{} ({value}) is over the {MAX_FONT_SIZE_PT}pt ceiling",
            field(prefix, name)
        ));
    }
    None
}

/// [`font_size`] for an optional field — absent means "inherit",
/// which is always in domain.
fn opt_font_size(prefix: &str, name: &str, value: Option<f32>) -> Option<String> {
    font_size(prefix, name, value?)
}

/// Bring `value` inside the `±limit` magnitude bound that the
/// private `bounded` check enforces, so a setter cannot author a
/// value the loader would then refuse.
///
/// The loader rejects rather than repairs, which makes every writer
/// that can exceed the domain a lockout candidate: the user edits,
/// saves, and their own map stops opening — a file destroyed by the
/// hardening meant to protect it. Screening at the door is not
/// enough on its own, because console verbs, macros and IPC write
/// these fields without passing the loader at all; the answer is the
/// same one the font metric takes in
/// [`clamp_font_metric`](crate::font::fonts::clamp_font_metric),
/// which is to clamp where the value lands.
///
/// A non-finite `value` falls back to `fallback` rather than to a
/// bound: `NaN` has no nearest edge, and the caller's previous value
/// is a better answer than an arbitrary one.
///
/// Cost: O(1), no allocation.
pub fn clamp_to_bound(value: f32, limit: f64, fallback: f32) -> f32 {
    if !value.is_finite() {
        return fallback;
    }
    // The limit is normalized before it reaches `f32::clamp`, which
    // panics on `min > max` or a `NaN` bound — the exact hazard this
    // module exists to name, and one this function could otherwise
    // reintroduce. A negative limit spells the pair backwards
    // (`clamp(5.0, -5.0)`); a `NaN` limit makes both bounds `NaN`;
    // a `limit` past `f32::MAX` casts to infinity, which clamps to
    // nothing but is at least total. Taking the magnitude and
    // screening non-finite covers all three.
    let limit = limit as f32;
    if limit.is_nan() {
        return fallback;
    }
    let limit = limit.abs();
    value.clamp(-limit, limit)
}

/// `Some(message)` unless `value` is finite and within `±limit`.
fn bounded(prefix: &str, name: &str, value: f64, limit: f64) -> Option<String> {
    if !value.is_finite() {
        return Some(format!("{} is not finite ({value})", field(prefix, name)));
    }
    if value.abs() > limit {
        return Some(format!(
            "{} ({value}) is outside the ±{limit} bound",
            field(prefix, name)
        ));
    }
    None
}

/// `Some(message)` unless `value` is finite. For fields whose
/// magnitude carries no risk on its own but whose `NaN` poisons a
/// comparison — a zoom bound, a normalized position along a path.
fn finite(prefix: &str, name: &str, value: Option<f32>) -> Option<String> {
    let value = value?;
    if value.is_finite() {
        None
    } else {
        Some(format!("{} is not finite ({value})", field(prefix, name)))
    }
}

/// `Some(message)` when a min / max pair is inverted, explaining the
/// consequence the caller actually suffers.
///
/// `consequence` is a parameter because the two callers do not share
/// one: a font-size window is handed to a clamp, while a zoom window
/// is two independent comparisons that are total for any ordering.
/// Reporting the wrong one would tell an author to fear a crash that
/// cannot happen, or shrug at an element that silently never renders.
fn ordered_pair(
    prefix: &str,
    name: &str,
    min: Option<f32>,
    max: Option<f32>,
    consequence: &str,
) -> Option<String> {
    let (min, max) = (min?, max?);
    if min > max {
        return Some(format!(
            "{}: min ({min}) is above max ({max}) — {consequence}",
            field(prefix, name)
        ));
    }
    None
}

// ---- Per-struct numeric domain ------------------------------------

/// The zero-section rule, for one node: every renderable node needs
/// at least one section.
///
/// `pub` and per-node because it has two consumers that want different
/// shapes of the same answer. The loader refuses the whole document at
/// the first offender — it only needs to know the map cannot open.
/// `maptool verify` reports every one, because a user fixing a map
/// wants the list rather than one node per run.
///
/// Sharing it is what keeps the two ends in step. They were not: the
/// loader has refused on this since before the invariant was written
/// down, and `verify` had no counterpart at all, so a map with an empty
/// `sections` array was reported **valid** by the tool whose contract
/// is "diagnose exactly the maps the editor refuses". That went
/// unnoticed while `verify` loaded through the strict door and would
/// have failed earlier; it became visible the moment `verify` moved to
/// `parse_for_inspection`, which deliberately skips the loader's
/// invariants so a broken map can still be inspected.
pub fn zero_section_node(node: &MindNode) -> Option<String> {
    node.sections.is_empty().then(|| {
        format!(
            "node {:?} ships zero sections — every renderable node \
             needs at least one. Run `maptool convert --sections <file>` \
             to migrate, or add an explicit `sections` array.",
            node.id
        )
    })
}

/// Every numeric-domain violation on a [`GlyphBorderConfig`],
/// stamped with `label` (the document part that carries it).
pub fn border_config_violations(label: &str, border: &GlyphBorderConfig) -> Vec<String> {
    let mut out = Vec::new();
    out.extend(font_size(label, "font_size_pt", border.font_size_pt));
    // Padding is a signed inset; only its magnitude and finiteness
    // matter, since the border geometry adds it to an axis already
    // bounded by MAX_NODE_AXIS.
    out.extend(bounded(label, "padding", border.padding as f64, MAX_NODE_AXIS));
    if let Some(glyphs) = border.glyphs.as_ref() {
        for (field, glyph) in [
            ("top", &glyphs.top),
            ("bottom", &glyphs.bottom),
            ("left", &glyphs.left),
            ("right", &glyphs.right),
            ("top_left", &glyphs.top_left),
            ("top_right", &glyphs.top_right),
            ("bottom_left", &glyphs.bottom_left),
            ("bottom_right", &glyphs.bottom_right),
        ] {
            out.extend(border_glyph_violations(label, field, glyph));
        }
    }
    out
}

/// Ceiling on the grapheme length of one authored border glyph.
///
/// A border glyph is a motif: the four sides are patterns repeated
/// to fill an edge, and the four corners are single marks. The
/// longest one authored anywhere in this repository is twelve
/// clusters (`+=#(\(\))#=+`), so this is generous by several times
/// over and still bounded.
pub const MAX_BORDER_GLYPH_CLUSTERS: usize = 64;

/// Ceiling on the byte length of one authored border glyph.
///
/// Paired with [`MAX_BORDER_GLYPH_CLUSTERS`] for the reason
/// [`MAX_CONNECTION_GLYPH_BYTES`] is paired with its cluster count:
/// a single extended grapheme cluster can carry any number of
/// combining marks, so a cluster ceiling alone bounds the shape of
/// the motif and not its cost.
///
/// The cost here is not only the repetition — `MAX_BORDER_SIDE_BYTES`
/// bounds what a side rail *emits*. It is the corners, which are
/// copied verbatim into their run specs and handed to
/// `glyph_ink_with` four times per node per frame.
///
/// That is **not** an unbounded shape call per frame: `glyph_ink_with`
/// memoizes through `INK_EXTENT_CACHE`, so a given corner is
/// rasterized once. The per-frame cost is the *lookup*, whose key is
/// `(face, size, grapheme.to_string())` — an owned copy of the glyph,
/// allocated unconditionally before the cache is even read. So a
/// megabyte corner is a megabyte cloned four times per node per
/// frame, and the cache then retains a copy per distinct glyph.
/// Bounded either way, and worth stating correctly: an earlier
/// version of this comment blamed the shaper, which is the one cost
/// the cache already removes.
pub const MAX_BORDER_GLYPH_BYTES: usize = 1024;

/// Both ceilings on one authored border glyph.
///
/// `pub` because the document setter screens the same eight fields
/// with it before writing. That is not defense in depth for its own
/// sake: the loader *rejects* an over-long glyph, so a writer that
/// did not screen would let the editor author a map it then refused
/// to reopen — and sharing this function is what keeps the two ends
/// from drifting to different ceilings.
pub fn border_glyph_violations(label: &str, name: &str, glyph: &str) -> Vec<String> {
    let mut out = Vec::new();
    let where_ = field(label, &format!("glyphs.{name}"));
    let clusters = count_grapheme_clusters(glyph);
    if clusters > MAX_BORDER_GLYPH_CLUSTERS {
        out.push(format!(
            "{where_} is {clusters} grapheme clusters, over the {MAX_BORDER_GLYPH_CLUSTERS} \
             ceiling — a border glyph is a motif repeated to fill an edge, not a paragraph"
        ));
    }
    let bytes = glyph.len();
    if bytes > MAX_BORDER_GLYPH_BYTES {
        out.push(format!(
            "{where_} is {bytes} bytes, over the {MAX_BORDER_GLYPH_BYTES} byte ceiling — a \
             single grapheme cluster can carry any number of combining marks, and a corner \
             glyph is cloned into a metric-cache key four times per node per frame"
        ));
    }
    out
}

/// Ceiling on the grapheme length of a connection's body or cap
/// glyph.
///
/// The field is documented as "the glyph(s) used for the body,
/// repeated to fill length" — a motif, not a paragraph. The cap
/// matters because the body is **emitted once per sampled point**:
/// each sample clones this string, segments it, and turns it into
/// its own `GlyphArea` and shaped buffer. The sample count already
/// has a ceiling, but the product of the two is what actually gets
/// allocated, so an unbounded string multiplies a bounded count
/// into an unbounded cost.
///
/// This ceiling alone does not bound that product — see
/// [`MAX_CONNECTION_GLYPH_BYTES`], which does.
pub const MAX_CONNECTION_GLYPH_GRAPHEMES: usize = 16;

/// Ceiling on the **byte** length of a connection's body or cap
/// glyph.
///
/// [`MAX_CONNECTION_GLYPH_GRAPHEMES`] counts clusters, and a UAX #29
/// extended grapheme cluster has no length bound: a base character
/// followed by any number of combining marks is exactly one cluster.
/// So a megabyte-long `body` passes a cluster cap with `length == 1`
/// while costing a megabyte per sampled point — the cluster ceiling
/// measures the motif, but the allocation is charged in bytes.
///
/// Both ceilings are enforced, because they bound different things:
/// the cluster count is what makes the field a motif rather than a
/// paragraph, and this is what makes the product finite. At
/// [`MAX_PATH_SAMPLES`](crate::mindmap::connection::MAX_PATH_SAMPLES)
/// (10,000) one path costs at most 10,000 × 512 B ≈ 5 MB of text.
/// That is the ceiling for a *single* connection, not for the
/// document: element count is not capped, so N edges cost N times
/// it. Bounding the total is a frame budget, which
/// `format/macros.md` names as still open.
///
/// 512 B across 16 clusters leaves 32 B per cluster. The longest
/// RGI emoji sequences land just inside that — a subdivision flag
/// is 28 bytes, a four-person family 25, a kiss sequence 27 — so
/// the margin is a few bytes on the worst realistic cluster rather
/// than the comfortable headroom an earlier version of this comment
/// claimed. A motif of ordinary glyphs is nowhere near either
/// ceiling.
pub const MAX_CONNECTION_GLYPH_BYTES: usize = 512;

/// Every numeric-domain violation on a [`GlyphConnectionConfig`].
pub fn connection_config_violations(label: &str, conn: &GlyphConnectionConfig) -> Vec<String> {
    let mut out = Vec::new();
    for (field, glyph) in [
        ("body", Some(&conn.body)),
        ("cap_start", conn.cap_start.as_ref()),
        ("cap_end", conn.cap_end.as_ref()),
    ] {
        let Some(glyph) = glyph else { continue };
        let length = count_grapheme_clusters(glyph);
        if length > MAX_CONNECTION_GLYPH_GRAPHEMES {
            out.push(format!(
                "{}.{field} is {length} grapheme clusters, over the \
                 {MAX_CONNECTION_GLYPH_GRAPHEMES} ceiling — this glyph is emitted once per \
                 sampled point along the path, so its length multiplies the sample count",
                label
            ));
        }
        // Checked separately from the cluster count, and not as a
        // formality: one cluster can carry an unbounded number of
        // combining marks, so the cluster ceiling above can pass a
        // megabyte. This is the ceiling that makes
        // `bytes × MAX_PATH_SAMPLES` finite.
        let bytes = glyph.len();
        if bytes > MAX_CONNECTION_GLYPH_BYTES {
            out.push(format!(
                "{}.{field} is {bytes} bytes, over the {MAX_CONNECTION_GLYPH_BYTES} byte \
                 ceiling — a single grapheme cluster can carry any number of combining \
                 marks, and this glyph is cloned once per sampled point along the path",
                label
            ));
        }
    }
    out.extend(font_size(label, "font_size_pt", conn.font_size_pt));
    out.extend(font_size(label, "min_font_size_pt", conn.min_font_size_pt));
    out.extend(font_size(label, "max_font_size_pt", conn.max_font_size_pt));
    out.extend(ordered_pair(
        label,
        "font_size",
        Some(conn.min_font_size_pt),
        Some(conn.max_font_size_pt),
        INVERTED_SIZE_WINDOW,
    ));
    // Spacing is added to the glyph advance to derive the sample
    // step, so only its finiteness and magnitude matter here.
    //
    // A *negative* spacing is deliberately allowed. It tightens the
    // rail below the `tight` preset into overlapping glyphs, which
    // is a real authoring choice the `spacing` console verb already
    // accepts — and rejecting it here would mean the editor could
    // write a map it then refused to reopen. The hazard it looked
    // like (a step that never advances) is already total at the
    // sampler: `sample_path` refuses a non-positive spacing up
    // front and emits no samples at all, so the loop is never
    // entered. (`sample_count` carries its own non-positive guard
    // returning a single point, but `sample_path` short-circuits
    // before reaching it — that arm covers the non-finite cases,
    // which do get through, because `NaN <= 0.0` is false.)
    out.extend(bounded(label, "spacing", conn.spacing as f64, MAX_NODE_AXIS));
    out
}

/// Every numeric-domain violation on an [`EdgeLabelConfig`].
pub fn label_config_violations(label: &str, cfg: &EdgeLabelConfig) -> Vec<String> {
    let mut out = Vec::new();
    out.extend(opt_font_size(label, "font_size_pt", cfg.font_size_pt));
    out.extend(opt_font_size(label, "min_font_size_pt", cfg.min_font_size_pt));
    out.extend(opt_font_size(label, "max_font_size_pt", cfg.max_font_size_pt));
    out.extend(ordered_pair(
        label,
        "font_size",
        cfg.min_font_size_pt,
        cfg.max_font_size_pt,
        INVERTED_SIZE_WINDOW,
    ));
    out.extend(finite(label, "position_t", cfg.position_t));
    out.extend(finite(label, "perpendicular_offset", cfg.perpendicular_offset));
    out.extend(zoom_window_violations(
        label,
        cfg.min_zoom_to_render,
        cfg.max_zoom_to_render,
    ));
    out
}

/// Every numeric-domain violation on a [`PortalEndpointState`].
pub fn portal_endpoint_violations(label: &str, portal: &PortalEndpointState) -> Vec<String> {
    let mut out = Vec::new();
    out.extend(opt_font_size(
        label,
        "text_font_size_pt",
        portal.text_font_size_pt,
    ));
    out.extend(opt_font_size(
        label,
        "text_min_font_size_pt",
        portal.text_min_font_size_pt,
    ));
    out.extend(opt_font_size(
        label,
        "text_max_font_size_pt",
        portal.text_max_font_size_pt,
    ));
    out.extend(ordered_pair(
        label,
        "text",
        portal.text_min_font_size_pt,
        portal.text_max_font_size_pt,
        INVERTED_SIZE_WINDOW,
    ));
    out.extend(finite(label, "border_t", portal.border_t));
    out.extend(finite(label, "perpendicular_offset", portal.perpendicular_offset));
    out.extend(zoom_window_violations(
        label,
        portal.min_zoom_to_render,
        portal.max_zoom_to_render,
    ));
    out
}

/// Violations on a zoom window — the `min` / `max` pair every
/// zoom-gated part of the format carries.
///
/// A `NaN` bound is the interesting case: every comparison against
/// it is false, so the element silently never renders and the
/// author has no way to see why. Both bounds are inclusive and a
/// zoom is a positive scale, so a negative bound is authored
/// nonsense rather than a wider window.
pub fn zoom_window_violations(label: &str, min: Option<f32>, max: Option<f32>) -> Vec<String> {
    let mut out = Vec::new();
    out.extend(finite(label, "min_zoom_to_render", min));
    out.extend(finite(label, "max_zoom_to_render", max));
    // Through `field()` rather than `format!("{label}.{name}")`:
    // node- and edge-level windows pass an empty label, and the
    // direct form spells that ".min_zoom_to_render is negative",
    // with a leading dot and no subject. Avoiding exactly that is
    // what `field()` is for.
    for (name, value) in [("min_zoom_to_render", min), ("max_zoom_to_render", max)] {
        if let Some(value) = value {
            if value.is_finite() && value < 0.0 {
                out.push(format!("{} is negative ({value})", field(label, name)));
            }
        }
    }
    out.extend(ordered_pair(label, "zoom", min, max, INVERTED_ZOOM_WINDOW));
    out
}

/// Every numeric-domain violation across one section's `text_runs`.
///
/// Two separate concerns. Each run's `size_pt` is the font size the
/// shaper receives, so it lives under the same floor and ceiling as
/// every other size. The run *ranges* are the second: the model
/// documents them as sorted, half-open `[start, end)`, non-
/// overlapping grapheme ranges, and `text_run_ops` relies on that
/// — but nothing enforced it on the way in. Overlapping runs make
/// the renderer's span builder emit the same graphemes once per
/// covering run, so N nested runs over one string is a quadratic
/// blow-up in shaped text from a linear file.
fn text_run_violations(section_idx: usize, section: &MindSection) -> Vec<String> {
    let mut out = Vec::new();
    if section.text_runs.is_empty() {
        return out;
    }
    // Grapheme clusters, not bytes — `start` / `end` index what the
    // user sees (`format/text-runs.md`), which is also the unit
    // `maptool verify` counts in.
    let total = count_grapheme_clusters(&section.text);
    let mut previous_end: Option<usize> = None;

    for (run_idx, run) in section.text_runs.iter().enumerate() {
        let label = format!("section[{section_idx}].text_runs[{run_idx}]");
        // Same domain as every other size the shaper receives —
        // finite, over the sub-point floor, under the atlas
        // ceiling. `size_pt` went through this check with its own
        // integer wording while it was a `u32`; as an `f32` it
        // shares the helper so the floor (0.5pt) and the message
        // cannot drift from the border / connection sizes.
        out.extend(font_size(&label, "size_pt", run.size_pt));
        // `>=`, not `>`: a zero-length run is as malformed as an
        // inverted one. `text_run_ops` calls the pair "zero-length
        // or inverted" and the document layer carries a
        // `debug_assert` that a run is non-empty, so accepting
        // `start == end` here leaves an abort reachable in every
        // debug build — including `cargo run`, the documented
        // one-off iteration command.
        if run.start >= run.end {
            out.push(format!(
                "{label} has start {} not less than end {} — a zero-length or inverted run",
                run.start, run.end
            ));
            // Its range is meaningless, so it neither bounds the
            // next run nor gets a length check. Mirrors verify.
            continue;
        }
        if run.end > total {
            out.push(format!(
                "{label} end {} exceeds text length {total} (grapheme clusters)",
                run.end
            ));
        }
        if let Some(previous_end) = previous_end {
            if run.start < previous_end {
                out.push(format!(
                    "{label} overlaps previous run (start {} < previous end {previous_end}) — \
                     overlapping runs re-emit the same graphemes once per covering run",
                    run.start
                ));
            }
        }
        previous_end = Some(run.end);
    }
    out
}

/// Every numeric-domain violation carried by one section.
///
/// Deliberately **not** the AABB containment rules: a section that
/// hangs past its node's edge renders fine and is
/// `maptool verify`'s call (`format/schema.md`). What belongs here
/// is only what the renderer cannot survive — a non-finite or
/// degenerate extent, a hostile border config, a bad run.
pub fn section_numeric_violations(idx: usize, section: &MindSection, parent_size: Size) -> Vec<String> {
    // The section's own shape comes from the shared extent rules —
    // restating them here is what made the loader and `verify`
    // describe one defect in two sentences.
    let mut out = section_extent_violations(parent_size, idx, section.offset, section.size);
    let label = format!("section[{idx}]");
    // The one bound extent does not express: an offset's magnitude.
    // Its finiteness is already covered above, so only the
    // out-of-range case is added here.
    for (name, value) in [("offset.x", section.offset.x), ("offset.y", section.offset.y)] {
        if value.is_finite() && value.abs() > MAX_CANVAS_COORD {
            out.push(format!(
                "{} ({value}) is outside the ±{MAX_CANVAS_COORD} bound",
                field(&label, name)
            ));
        }
    }
    if let Some(border) = section.frame_border.as_ref() {
        out.extend(border_config_violations(&field(&label, "frame_border"), border));
    }
    out.extend(text_run_violations(idx, section));
    out
}

/// Every numeric-domain violation carried by one [`MindNode`] —
/// its own geometry, its style and layout numbers, its border
/// override, and each of its sections.
///
/// Cost: O(sections + text runs) on the node.
///
/// *Field* labels are built only on the failing arm — `field()`
/// formats `prefix.name` lazily — but the *part* prefixes are not:
/// `section_numeric_violations` and `text_run_violations` each build
/// theirs up front, so a well-formed node still allocates one
/// `String` per section and one per text run. The empty `Vec`s
/// threaded through cost nothing (`Vec::new` does not allocate).
///
/// That is a load-time cost on a path that has just parsed the whole
/// document, and `maptool verify` runs it again on the same terms.
/// Making the prefixes lazy too would mean passing the index down to
/// every helper instead of a `&str`; it has not been worth that.
pub fn node_numeric_violations(node: &MindNode) -> Vec<String> {
    let mut out = Vec::new();
    out.extend(bounded("position", "x", node.position.x, MAX_CANVAS_COORD));
    out.extend(bounded("position", "y", node.position.y, MAX_CANVAS_COORD));
    out.extend(node_size_violations(node.size));
    out.extend(bounded(
        "style",
        "corner_radius_percent",
        node.style.corner_radius_percent,
        MAX_NODE_AXIS,
    ));
    out.extend(bounded(
        "style",
        "frame_thickness",
        node.style.frame_thickness,
        MAX_NODE_AXIS,
    ));
    out.extend(bounded(
        "layout",
        "spacing",
        node.layout.spacing,
        MAX_CANVAS_COORD,
    ));
    if let Some(border) = node.style.border.as_ref() {
        out.extend(border_config_violations("style.border", border));
    }
    out.extend(zoom_window_violations(
        "",
        node.min_zoom_to_render,
        node.max_zoom_to_render,
    ));
    for (idx, section) in node.sections.iter().enumerate() {
        out.extend(section_numeric_violations(idx, section, node.size));
    }
    out
}

/// Every numeric-domain violation carried by one [`MindEdge`] —
/// its control points, its glyph-connection override, its label
/// config, and both portal endpoints.
pub fn edge_numeric_violations(edge: &MindEdge) -> Vec<String> {
    let mut out = Vec::new();
    for (idx, point) in edge.control_points.iter().enumerate() {
        out.extend(control_point_violations(idx, point));
    }
    if let Some(conn) = edge.glyph_connection.as_ref() {
        out.extend(connection_config_violations("glyph_connection", conn));
    }
    if let Some(cfg) = edge.label_config.as_ref() {
        out.extend(label_config_violations("label_config", cfg));
    }
    if let Some(portal) = edge.portal_from.as_ref() {
        out.extend(portal_endpoint_violations("portal_from", portal));
    }
    if let Some(portal) = edge.portal_to.as_ref() {
        out.extend(portal_endpoint_violations("portal_to", portal));
    }
    out.extend(zoom_window_violations(
        "",
        edge.min_zoom_to_render,
        edge.max_zoom_to_render,
    ));
    out
}

/// Violations on one Bezier control point. Control-point offsets
/// feed the path sampler, whose sample count is the path length
/// over the glyph step — so a non-finite or astronomical offset is
/// what turns one edge into an unbounded allocation.
fn control_point_violations(idx: usize, point: &ControlPoint) -> Vec<String> {
    let label = format!("control_points[{idx}]");
    let mut out = Vec::new();
    out.extend(bounded(&label, "x", point.x, MAX_CANVAS_COORD));
    out.extend(bounded(&label, "y", point.y, MAX_CANVAS_COORD));
    out
}

/// Every numeric-domain violation carried by the [`Canvas`]'s
/// default border and connection cascades.
///
/// These matter out of proportion to their size: a canvas default
/// is the fallback for *every* node or edge that does not override
/// it, so one hostile number here poisons the whole document at
/// once rather than a single element.
pub fn canvas_numeric_violations(canvas: &Canvas) -> Vec<String> {
    let mut out = Vec::new();
    for (label, border) in [
        ("default_border", canvas.default_border.as_ref()),
        (
            "default_section_frame_border",
            canvas.default_section_frame_border.as_ref(),
        ),
        (
            "default_focused_section_frame_border",
            canvas.default_focused_section_frame_border.as_ref(),
        ),
    ] {
        if let Some(border) = border {
            out.extend(border_config_violations(label, border));
        }
    }
    if let Some(conn) = canvas.default_connection.as_ref() {
        out.extend(connection_config_violations("default_connection", conn));
    }
    out
}

/// Ceiling on an authored animation envelope, in milliseconds.
///
/// While any animation is live the event loop holds
/// `ControlFlow::Poll` — it must, to produce frames — so the
/// envelope is also how long a map can keep the process off its
/// idle path. `u32` milliseconds reaches roughly 49 days per field,
/// and a maxed *delay* renders nothing at all, so the app looks
/// idle while spinning a core and submitting frames. A minute is
/// far beyond any authorable UI transition and bounds the worst
/// case at something a user would sit through rather than a season.
pub const MAX_ANIMATION_MS: u32 = 60_000;

/// Numeric-domain violations on one [`CustomMutation`].
///
/// **Mutations are the second way numbers enter the model**, and
/// they arrive after the loader has finished: a map carries its own
/// `custom_mutations`, a node carries `inline_mutations`, and a
/// trigger binding fires one on a click.
///
/// What this screens is the **animation envelope**, because nothing
/// downstream bounds it — a duration is a promise to hold the event
/// loop off its idle path for that long, and only the loader is in a
/// position to refuse.
///
/// What it deliberately does *not* screen is the mutator payload
/// itself. Those leaves set scene metrics, and scene metrics are
/// clamped where they land (`GlyphArea`'s scale setters and its
/// delta path, via `font::fonts::clamp_font_metric`) — which has to
/// hold anyway, since console verbs, macros and IPC write the same
/// fields and never reach this function.
pub fn mutation_numeric_violations(mutation: &CustomMutation) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(timing) = mutation.timing.as_ref() {
        for (field, value) in [("duration_ms", timing.duration_ms), ("delay_ms", timing.delay_ms)] {
            if value > MAX_ANIMATION_MS {
                out.push(format!(
                    "{:?}: timing.{field} ({value}ms) is over the {MAX_ANIMATION_MS}ms ceiling — \
                     an animation holds the event loop off its idle path for its whole envelope",
                    mutation.id
                ));
            }
        }
    }
    out
}

/// The whole-map numeric-domain sweep the loader runs, returning
/// the first violation stamped with the document part that carries
/// it — `canvas`, `node "1.2"`, `edge[3]` — matching how the
/// loader's other rejections address the file.
///
/// Nodes are visited in sorted-id order so the reported part is
/// deterministic across `HashMap` iteration order, the same reason
/// the zero-section and section-cap checks sort.
///
/// Cost: O(n log n) in the node count for the sort, plus
/// O(sections + runs + edges + control points).
///
/// It allocates on a clean map, not only on the failure it reports:
/// the sort needs a `Vec` of borrows, and each section and text run
/// costs one `String` for its label prefix — see
/// [`node_numeric_violations`]. Per-*field* labels are lazy, built
/// only on the arm that fails.
pub fn map_numeric_domain(map: &MindMap) -> Option<String> {
    if let Some(message) = canvas_numeric_violations(&map.canvas).into_iter().next() {
        return Some(format!("canvas: {message}"));
    }
    for (idx, mutation) in map.custom_mutations.iter().enumerate() {
        if let Some(message) = mutation_numeric_violations(mutation).into_iter().next() {
            return Some(format!("custom_mutations[{idx}]: {message}"));
        }
    }
    let mut nodes: Vec<&MindNode> = map.nodes.values().collect();
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    for node in nodes {
        if let Some(message) = node_numeric_violations(node).into_iter().next() {
            return Some(format!("node {:?}: {message}", node.id));
        }
        for (idx, mutation) in node.inline_mutations.iter().enumerate() {
            if let Some(message) = mutation_numeric_violations(mutation).into_iter().next() {
                return Some(format!("node {:?}: inline_mutations[{idx}]: {message}", node.id));
            }
        }
    }
    for (idx, edge) in map.edges.iter().enumerate() {
        if let Some(message) = edge_numeric_violations(edge).into_iter().next() {
            return Some(format!("edge[{idx}]: {message}"));
        }
    }
    None
}
