// SPDX-License-Identifier: MPL-2.0

//! Connections — the cache-coupled sampling pass and the tree it
//! feeds.
//!
//! [`build_connection_elements`] turns every visible line-mode edge
//! into a [`ConnectionElement`]: sampled glyph positions along the
//! path, clipped against the node AABBs from
//! [`super::node_clip`]. Three routes per edge:
//!
//! - **Fast path** (no endpoint in `offsets`, cache hit): reuse the
//!   cached pre-clip samples verbatim; rerun only the cheap
//!   `node_aabbs` clip filter.
//! - **Translate path** (both endpoints in `offsets` with the same
//!   delta, cache hit): shift the cached samples by the shared
//!   delta and rerun the clip filter. Skips
//!   `build_connection_path` + `sample_path` entirely — the
//!   subtree-drag hot path.
//! - **Slow path** (otherwise): build the path, resample, write
//!   the cache.
//!
//! The clip filter runs against the current frame's `node_aabbs`
//! on all three paths so a stable edge correctly clips around
//! third nodes that moved through its path this frame.
//!
//! Every styling value on the emitted element — body glyph, cap
//! glyphs, font family, font size, body color — is read from the
//! live model on all three paths, so a styling edit is visible on
//! the next frame whether or not the cache was hit. Both reuse paths
//! additionally ask the cache for an entry sampled under this
//! frame's [`SampleParams`] and get nothing when the answer is a
//! different one, so a font-size or spacing edit resamples on its
//! own. **What the caller still owes the cache** is geometry neither
//! mechanism can see, and only that: an endpoint that moved in the
//! model without appearing in `offsets`, an anchor / control-point
//! edit, and a node resize. Those are
//! `SceneConnectionCache::invalidate_edge` or `::clear`; this header
//! used to name the glyph config among them, and then the theme
//! variables.
//!
//! Cache lifecycle: `ensure_zoom` is caller-managed (the caller
//! flushes on zoom change before any pass starts). The liveness
//! generation is not: `begin_pass` opens it here and `evict_unseen`
//! closes it at the end of the loop, so eviction of deleted edges
//! stays where the touching happens. The pass keeps no record of
//! which edges it saw — reaching an entry through either reuse door,
//! or writing one through `refill`, is what marks it.
//!
//! Selected-edge handle emission rides along in the same loop:
//! single-edge selection means at most one handle batch per
//! rebuild, so there is no cost to bundling it with the connection
//! pass rather than adding a separate iteration.
//!
//! The tree half emits one `Void` per edge, with one cap /
//! body-glyph / cap `GlyphArea` per on-screen glyph. Channels are
//! layered so layout-stable changes (color, theme) take the
//! in-place mutator path via [`build_connection_mutator_tree`].

use std::collections::{HashMap, HashSet};

use glam::Vec2;

use crate::core::primitives::ColorFontRegions;
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::GfxMutator;
use crate::gfx_structs::tree::Tree;
use crate::mindmap::connection;
use crate::mindmap::model::{GlyphConnectionConfig, MindMap};
use crate::mindmap::scene_cache::{EdgeKey, SampleParams, SceneConnectionCache};
use crate::mindmap::SELECTION_HIGHLIGHT_HEX;
use crate::util::color;
use crate::util::color::resolve_var;

use super::edge_handle::{build_edge_handles, EdgeHandleElement};
use super::edge_path::{offset_node_rect, EdgePathCache};
use super::overrides::EdgeColorPreview;

/// A connection (edge) between two nodes, with pre-computed glyph positions.
pub struct ConnectionElement {
    /// Stable identity of the edge — `(from_id, to_id, edge_type)`. Used by
    /// the renderer's keyed connection buffer map so unchanged edges can
    /// reuse their shaped `cosmic_text::Buffer`s across drag frames.
    pub edge_key: EdgeKey,
    /// Sampled glyph positions along the path (canvas coordinates).
    pub glyph_positions: Vec<(f32, f32)>,
    /// The body glyph string repeated at each position.
    pub body_glyph: String,
    /// Optional start cap glyph and its position.
    pub cap_start: Option<(String, (f32, f32))>,
    /// Optional end cap glyph and its position.
    pub cap_end: Option<(String, (f32, f32))>,
    /// Font family name, if specified.
    pub font: Option<String>,
    /// Font size in points.
    pub font_size_pt: f32,
    /// Color as #RRGGBB hex string.
    pub color: String,
    /// Zoom window for the whole connection (body glyphs + caps).
    /// Resolved directly from `MindEdge.min_zoom_to_render` /
    /// `MindEdge.max_zoom_to_render` — edges are the authoring
    /// unit, no per-glyph override.
    pub zoom_visibility: crate::gfx_structs::zoom_visibility::ZoomVisibility,
}

/// Squared-length threshold below which `delta_from` and `delta_to`
/// count as "the same delta" for the translate path. In the
/// target case — subtree drag with `MovingNode`'s shared-delta
/// offsets — the two subtractions produce byte-identical f32
/// pairs, so the compare passes at zero. The epsilon only absorbs
/// any future drift from non-identical arithmetic paths; keep it
/// tight so mixed-delta edges (boundary edges at the subtree root)
/// still fall through to the slow path.
const TRANSLATE_DELTA_EPSILON_SQ: f32 = 1.0e-6;

/// Build + push one `ConnectionElement` to `out`. Single source
/// for the three emit sites (cache-hit fast path, translate path,
/// slow rebuild) — they each filtered cap_start / cap_end /
/// glyph_positions against `node_aabbs`, applied the "all-clipped →
/// skip" rule, and pushed an identical struct literal. Returns
/// `true` iff an element was actually pushed (callers don't
/// currently use it; a future "did this edge clip out?" check
/// would).
///
/// Every styling value comes from the live `config` and `params`
/// rather than from whatever the cache happens to hold, which is
/// what makes a glyph edit visible on the next frame from all three
/// sites at once instead of from one of them (#36 item 7).
// `clippy::too_many_arguments`: eight, down from eleven. What is
// left is the frame's inputs (`config`, `params`, `color`,
// `zoom_visibility`, `node_clip`), the geometry, and the two
// out-parameters; bundling any of them would hide from the call
// site which values are per-edge and which are per-frame, which is
// the distinction the three paths differ on.
#[allow(clippy::too_many_arguments)]
fn emit_connection_element(
    out: &mut Vec<ConnectionElement>,
    edge_key: EdgeKey,
    config: &GlyphConnectionConfig,
    params: &SampleParams,
    color: String,
    zoom_visibility: crate::gfx_structs::zoom_visibility::ZoomVisibility,
    pre_clip_positions: &[Vec2],
    node_clip: &NodeClipIndex,
) -> bool {
    let cap_at = |glyph: Option<&String>, point: Option<&Vec2>| {
        glyph
            .zip(point)
            .filter(|(_, p)| !node_clip.contains(**p))
            .map(|(g, p)| (g.clone(), (p.x, p.y)))
    };
    // `sample_path` walks the path from the source anchor to the
    // target anchor, so the first sample is where the start cap
    // belongs and the last is where the end cap does. Reading the
    // ends here rather than storing two more points is what lets
    // `CachedConnection::translate` move the caps by moving nothing.
    let cap_start = cap_at(config.cap_start.as_ref(), pre_clip_positions.first());
    let cap_end = cap_at(config.cap_end.as_ref(), pre_clip_positions.last());
    // Sized for the unclipped case, which is the common one and the
    // largest: `collect` over a `Filter` has no exact size hint, so it
    // starts empty and grows. One over-sized allocation beats a growth
    // sequence here — the surviving count is a filter away from the
    // input count, never above it.
    let mut glyph_positions: Vec<(f32, f32)> = Vec::with_capacity(pre_clip_positions.len());
    glyph_positions.extend(
        pre_clip_positions
            .iter()
            .filter(|p| !node_clip.contains(**p))
            .map(|p| (p.x, p.y)),
    );
    if glyph_positions.is_empty() && cap_start.is_none() && cap_end.is_none() {
        return false;
    }
    out.push(ConnectionElement {
        edge_key,
        glyph_positions,
        body_glyph: config.body.clone(),
        cap_start,
        cap_end,
        font: config.font.clone(),
        font_size_pt: params.font_size_pt,
        color,
        zoom_visibility,
    });
    true
}

/// Emit connection elements + edge-handle elements. Consumes
/// `node_aabbs` from [`super::node_clip_aabbs`] for the clip filter;
/// mutates `cache` on every edge it reaches — reuse marks liveness —
/// and after the loop, where `evict_unseen` drops the edges the model
/// lost since the previous build.
///
/// `paths` is this frame's shared [`EdgePathCache`]; both the
/// selected edge's handles and the slow path read the same entry
/// out of it, and the label pass reads it again afterwards
/// (#36 item 6).
///
/// # Costs
///
/// O(visible edges). Each edge takes the fast, translate, or slow
/// path (see the module header); only the slow path asks `paths` for
/// a path and runs `sample_path`. The clip filter is
/// O(samples x visible nodes) on every path.
// `clippy::too_many_arguments`: each argument has a different
// caller-vs-pass lifetime relationship — `map` / `offsets` borrow
// the persisted document, `node_aabbs` is this frame's clip input,
// the selection and preview are per-frame snapshots, `cache` is
// mut-borrowed across frames for memoization, `paths` is
// mut-borrowed for this frame only, and `camera_zoom` is by-value.
// Bundling them would either duplicate every field's lifetime
// annotation or hide the borrow shape from the caller.
#[allow(clippy::too_many_arguments)]
pub fn build_connection_elements(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    node_aabbs: &[(Vec2, Vec2)],
    selected_edge: Option<(&str, &str, &str)>,
    edge_color_preview: Option<EdgeColorPreview<'_>>,
    cache: &mut SceneConnectionCache,
    paths: &mut EdgePathCache<'_>,
    camera_zoom: f32,
    hidden_set: &HashSet<&str>,
) -> (Vec<ConnectionElement>, Vec<EdgeHandleElement>) {
    let vars = &map.canvas.theme_variables;
    let default_config = GlyphConnectionConfig::default();
    // Indexed once per pass. The clip is queried once per sampled
    // glyph on every edge, and `MAX_PATH_SAMPLES` caps a single path
    // at 10 000 points — scanning every node for each of those is what
    // made this pass quadratic in the map.
    let node_clip = NodeClipIndex::build(node_aabbs);
    // Every edge gets the same share of the scene-wide glyph budget, so
    // the result cannot depend on iteration order and no edge renders
    // while a later one vanishes. See `MAX_TOTAL_PATH_SAMPLES`.
    let per_path_samples = connection::per_path_sample_budget(map.edges.len());
    // One slot per edge is the ceiling — portal, hidden and dangling
    // edges emit nothing — so this over-reserves on a map that is
    // mostly portals and never grows on any other.
    let mut connection_elements = Vec::with_capacity(map.edges.len());
    // Grab-handles for the currently selected edge. Populated at most
    // once per scene build (selection is single-edge); empty
    // otherwise, which is why this one is *not* pre-sized: reserving
    // for a selection that is usually absent would allocate on every
    // frame to hold nothing.
    let mut edge_handles: Vec<EdgeHandleElement> = Vec::new();
    // Open the liveness generation. Every entry this pass hands out or
    // writes is stamped with it, and `evict_unseen` below drops
    // whatever still carries an older one — the edges the model lost
    // between builds. Nothing here has to keep a set of what it saw.
    cache.begin_pass();

    for (edge_index, edge) in map.edges.iter().enumerate() {
        if !edge.visible {
            continue;
        }
        // Portal-mode edges render as markers in the portal pass,
        // not as a path. Skip them here so the connection pipeline
        // (sampling, clipping, edge handles, labels) never touches
        // an edge that has no line form.
        if crate::mindmap::model::is_portal_edge(edge) {
            continue;
        }
        let from_node = match map.nodes.get(&edge.from_id) {
            Some(n) => n,
            None => continue,
        };
        let to_node = match map.nodes.get(&edge.to_id) {
            Some(n) => n,
            None => continue,
        };
        if hidden_set.contains(from_node.id.as_str()) || hidden_set.contains(to_node.id.as_str()) {
            continue;
        }

        let edge_key = EdgeKey::from_edge(edge);

        // Resolve glyph config: edge override > canvas default > hardcoded default
        let config = edge
            .glyph_connection
            .as_ref()
            .or(map.canvas.default_connection.as_ref())
            .unwrap_or(&default_config);

        let is_selected = selected_edge
            .is_some_and(|(f, t, ty)| f == edge.from_id && t == edge.to_id && ty == edge.edge_type);

        // Emit grab-handles for the selected edge. Done once, from
        // the LIVE edge + current (offset-applied) endpoint positions —
        // the caller may be in the middle of a drag and the handle
        // positions have to track that live state. Cost is bounded
        // (one edge per build) so no cache.
        if is_selected {
            let (from_pos, from_size) = offset_node_rect(from_node, offsets);
            let (to_pos, to_size) = offset_node_rect(to_node, offsets);
            if let Some(path) = paths.path(edge_index) {
                edge_handles.extend(build_edge_handles(
                    edge, &edge_key, path, from_pos, from_size, to_pos, to_size,
                ));
            }
        }

        // Did either endpoint of THIS edge move this frame?
        let endpoint_moved = offsets.contains_key(&from_node.id) || offsets.contains_key(&to_node.id);

        // The sampling snapshot this frame wants for this edge.
        // `SampleParams::font_size_pt` is the canvas-space size clamped
        // to keep the on-screen glyph size inside
        // [min_font_size_pt, max_font_size_pt]. At extreme zoom-out
        // that inflates the canvas-space size so sample spacing grows
        // and the per-edge glyph count falls — the LOD mechanism that
        // keeps zoomed-out connections from becoming a dust cloud.
        // Inversely, at extreme zoom-in the effective font size
        // shrinks, spacing shrinks, and per-edge sample count rises
        // linearly with zoom — which is what blows the drag drain
        // budget at high zoom. The translate path below is what keeps
        // subtree-drag cost bounded there.
        //
        // Both reuse paths hand this to the cache, which refuses any
        // entry sampled under different params. That is what keeps a
        // font-size or spacing edit from depending on the mutating site
        // to remember `SceneConnectionCache::clear` (#36 item 7).
        let want = SampleParams::snapshot(config, camera_zoom, per_path_samples);

        // Body color, resolved once for all three paths, highest
        // priority first: the color-picker preview (so the user's live
        // feedback is visible on the connection body rather than
        // masked by the cyan selection highlight), then the selection
        // highlight, then the committed cascade.
        //
        // Resolved per frame rather than cached. It used to be the one
        // styling value the cache still held, handed back by both
        // reuse doors without either comparing it — which is the shape
        // #36 item 7 removed for the body glyph, the caps, the font
        // family and the font size, left standing on the fifth field.
        // The cascade underneath is a node lookup the pass has already
        // done, an `Option` test, at most one palette lookup and a
        // `Vec::get`; the `to_string` at the end is the same single
        // allocation `cached.color.clone()` used to cost.
        let color = if let Some(preview) =
            edge_color_preview.and_then(|p| (*p.edge_key == edge_key).then_some(p.color))
        {
            resolve_var(preview, vars).to_string()
        } else if is_selected {
            SELECTION_HIGHLIGHT_HEX.to_string()
        } else {
            resolve_var(map.edge_body_color(edge), vars).to_string()
        };

        // --- Fast path: cached geometry is still valid ---
        //
        // If the endpoints haven't moved and we have a cached entry for
        // this edge sampled under `want`, reuse the cached pre-clip
        // samples and skip `build_connection_path` / `sample_path`
        // entirely. The cheap clip filter still runs against THIS
        // frame's `node_aabbs` so a stable edge correctly clips around
        // a third node that moved through its path.
        if !endpoint_moved {
            if let Some(cached) = cache.reusable(&edge_key, &want) {
                emit_connection_element(
                    &mut connection_elements,
                    edge_key,
                    config,
                    &want,
                    color,
                    edge.zoom_window(),
                    &cached.pre_clip_positions,
                    &node_clip,
                );
                continue;
            }
        }

        // Resolved here rather than above the fast path on purpose: a
        // cache-hit edge needs none of this, and hoisting it would put
        // four hash lookups on every edge of every static frame. Only
        // the positions are wanted — the sizes reach the path through
        // `paths`, which resolves them the same way.
        let (from_pos, _) = offset_node_rect(from_node, offsets);
        let (to_pos, _) = offset_node_rect(to_node, offsets);

        // --- Translate path: rigid-body subtree-drag optimization ---
        //
        // The common case inside a subtree drag: both endpoints of an
        // internal edge are in `offsets` with the SAME delta, so the
        // edge is a pure translation of its last-sampled geometry.
        // When the cache has an entry sampled under `want`, we can skip
        // `build_connection_path` + `sample_path` entirely and shift
        // the cached samples by the shared delta.
        //
        // Fall-throughs to the slow path:
        // - Boundary edges (one endpoint moved, one not; or both moved
        //   by different deltas — a rotating / stretching edge).
        // - Zoom-change frames and mid-drag `font_size_pt` / `spacing`
        //   edits, both of which move `want` away from the entry's
        //   stored params. A body / cap / font-family edit does *not*
        //   land here, and does not need to: none of them can move a
        //   sample, and the element is emitted from the live config
        //   either way.
        //
        // Probe and mutation are one phase, on one borrow:
        // `reusable_mut` hands out the cached entry only if its params
        // match, the delta is read off it, `translate` shifts it in
        // place (no `by_node` reindex, no string clones on the hot
        // path), and the same borrow is reborrowed as shared to emit
        // the element. One hash lookup covers the whole path, and
        // there is no re-lookup whose success has to be asserted after
        // the guard that already proved it.
        let translated = cache.reusable_mut(&edge_key, &want).and_then(|cached| {
            let delta_from = from_pos - cached.base_from;
            let delta_to = to_pos - cached.base_to;
            if (delta_from - delta_to).length_squared() >= TRANSLATE_DELTA_EPSILON_SQ {
                return None;
            }
            cached.translate(delta_from, from_pos, to_pos);
            Some(&*cached)
        });

        if let Some(entry) = translated {
            // Clip filter runs every frame against this frame's
            // node_aabbs — an unrelated moved node passing through
            // a translated edge must still clip out the glyphs
            // inside it.
            emit_connection_element(
                &mut connection_elements,
                edge_key,
                config,
                &want,
                color,
                edge.zoom_window(),
                &entry.pre_clip_positions,
                &node_clip,
            );
            continue;
        }

        // --- Slow path: sample fresh and update the cache ---
        // Shared with the handle emission above and with the label
        // pass afterwards: whichever of the three asks first pays for
        // the anchor resolution, and the other two do not.
        let Some(path) = paths.path(edge_index) else {
            continue;
        };

        // Sample straight into the entry's own buffer. The whole write
        // is one call so the entry and both its `by_node` memberships
        // stay in place across it — an edge that resamples while
        // already cached does not allocate. `None` means the edge
        // sampled to nothing, in which case `refill` has already
        // evicted it and the next build will retry.
        //
        // Caps live at the first and last positions this fills (the
        // anchor points resolved from the source/target node bounds),
        // which is why nothing here computes them:
        // `emit_connection_element` reads both ends of the array.
        // Those points sit on the raw node edge — which is ON the clip
        // AABB boundary for an unframed node (so they survive
        // clipping) but INSIDE the expanded clip AABB for a framed one
        // (so they get dropped along with the body glyphs that would
        // also render inside the frame area).
        //
        // The emitter's slice is read back out of the entry, so the
        // samples are stored once and filtered from where they are
        // stored rather than cloned so one copy can be each.
        let Some(entry) = cache.refill(&edge_key, want, from_pos, to_pos, |out| {
            connection::sample_path_into(path, want.sample_spacing(), want.sample_budget, out)
        }) else {
            continue;
        };

        // Now produce the post-clip element for THIS frame.
        emit_connection_element(
            &mut connection_elements,
            edge_key,
            config,
            &want,
            color,
            edge.zoom_window(),
            &entry.pre_clip_positions,
            &node_clip,
        );
    }

    // Evict any cache entries for edges that were in the cache but NOT in
    // the map this frame — handles edges that were deleted between builds.
    cache.evict_unseen();

    (connection_elements, edge_handles)
}

/// Returns true if `point` is strictly inside any of the given AABBs. Uses a
/// small epsilon so points that sit exactly on a border (e.g. connection
/// anchor points, which are placed at node-edge midpoints) are NOT
/// considered inside — that would accidentally clip the endpoints.
/// A spatial index over the node AABBs a connection pass clips
/// against.
///
/// **The linear scan it replaces made the pass O(edges x samples x
/// nodes).** `MAX_PATH_SAMPLES` caps one path at 10 000 points and
/// nothing caps the node count, so every sampled glyph on every edge
/// was tested against every node. Measured on
/// `build_connection_elements` with glyph-rendered edges: 100 nodes
/// took 77 ms, 200 took 294 ms, 400 took 1.23 s — roughly four times
/// the cost for twice the map, from a 293 KB file. Past ten seconds
/// `FreezeWatchdog` aborts the process, so this ended in unsaved work
/// destroyed by opening a file.
///
/// A uniform hash grid, because the query is "does any AABB contain
/// this point" rather than a range or a nearest-neighbor search: the
/// point falls in exactly one cell, and only that cell's occupants can
/// contain it. Occupied cells only, so an empty region of a
/// `MAX_CANVAS_COORD`-wide canvas costs nothing.
///
/// An AABB spanning more cells than [`Self::MAX_CELLS_PER_AABB`] goes
/// in `oversized` and is checked on every query. That keeps a single
/// node the size of the canvas from inserting itself into millions of
/// buckets — the failure this structure would otherwise introduce
/// while fixing another.
///
/// **Why there is no path-level pre-bound**, since
/// [`connection::path_bounds`] is sitting right there and the idea
/// recurs: skip the per-sample queries for an edge whose whole path
/// misses every node. Four reasons, none of which is the f32 hull
/// escape:
///
/// 1. [`Self::contains`] is **not** O(1) — it linear-scans
///    `oversized` before it touches the grid. A pre-bound does not
///    remove that floor, because it would have to test the path box
///    against every oversized AABB too. What it removes is one bucket
///    hash per sample.
/// 2. `path_bounds` is the **control-polygon** box, not the curve's
///    extent, and for an S-curve those differ a lot. So the early-out
///    would fire least often on exactly the curved edges whose
///    sampling costs most.
/// 3. This index has no box query, and adding one is not obviously
///    cheap: a long cross-link's box can span the canvas, which is
///    the same unbounded cell coverage [`Self::MAX_CELLS_PER_AABB`]
///    exists to bound on the insertion side.
/// 4. The hull escape is a real term but no longer an argument by
///    itself — `distance_to_path_within` ships `HULL_ESCAPE_SLACK`
///    for it. Its size is
///    `|coordinate| × MEASURED_WORST_ESCAPE_ULPS × f32::EPSILON`;
///    that constant lives beside
///    `test_path_bounds_slack_covers_the_sampler_escape` in
///    `connection/tests.rs`, and it is a measured maximum over a
///    finite sweep rather than a bound. What matters here is the
///    *shape* of the expression: the escape scales with the
///    coordinate and this filter's own `EDGE_EPSILON` does not, so
///    which of the two is larger depends on where on the canvas the
///    edge sits — below it at ordinary magnitudes, above it at the
///    coordinates `MAX_CANVAS_COORD` admits. A pre-bound would
///    therefore owe the same coordinate-scaled slack rather than get
///    to ignore the question. A cost and an obligation, not a
///    blocker.
///
///    (Deliberately no digits. That constant's own doc asks prose to
///    name it instead of restating it, because a figure copied into a
///    comment goes stale silently — and the first version of this
///    paragraph did exactly that, with a one-ulp premise that made
///    all three of its numbers wrong.)
pub struct NodeClipIndex {
    cell: f32,
    buckets: std::collections::HashMap<(i32, i32), Vec<(Vec2, Vec2)>>,
    oversized: Vec<(Vec2, Vec2)>,
}

impl NodeClipIndex {
    /// Past this many cells an AABB is held aside and checked every
    /// query, rather than inserted into each cell it covers.
    const MAX_CELLS_PER_AABB: i64 = 64;

    /// Smallest cell edge, so a map of degenerate zero-size nodes
    /// cannot drive the cell coordinate past `i32`.
    const MIN_CELL: f32 = 1.0;

    /// Index `aabbs`. O(total cells covered), which
    /// `MAX_CELLS_PER_AABB` bounds at 64 per AABB.
    pub fn build(aabbs: &[(Vec2, Vec2)]) -> Self {
        // Cell size from the mean node extent: big enough that an
        // ordinary node touches a handful of cells, small enough that a
        // cell holds a handful of nodes.
        let mut total = 0.0f64;
        let mut counted = 0u32;
        for (_, size) in aabbs {
            if size.x.is_finite() && size.y.is_finite() {
                total += size.x.max(size.y).abs() as f64;
                counted += 1;
            }
        }
        let mean = if counted == 0 {
            Self::MIN_CELL
        } else {
            (total / counted as f64) as f32
        };
        let cell = if mean.is_finite() && mean > Self::MIN_CELL {
            mean
        } else {
            Self::MIN_CELL
        };

        let mut buckets: std::collections::HashMap<(i32, i32), Vec<(Vec2, Vec2)>> =
            std::collections::HashMap::new();
        let mut oversized = Vec::new();
        for aabb in aabbs {
            let (pos, size) = *aabb;
            if !pos.x.is_finite() || !pos.y.is_finite() || !size.x.is_finite() || !size.y.is_finite() {
                // Nothing can be strictly inside a non-finite box, but
                // keeping it out of the grid rather than reasoning
                // about it is cheaper than being clever.
                oversized.push(*aabb);
                continue;
            }
            let (x0, y0) = (Self::cell_of(pos.x, cell), Self::cell_of(pos.y, cell));
            let (x1, y1) = (
                Self::cell_of(pos.x + size.x, cell),
                Self::cell_of(pos.y + size.y, cell),
            );
            let (x0, x1) = (x0.min(x1), x0.max(x1));
            let (y0, y1) = (y0.min(y1), y0.max(y1));
            let spans = (x1 as i64 - x0 as i64 + 1) * (y1 as i64 - y0 as i64 + 1);
            if spans > Self::MAX_CELLS_PER_AABB {
                oversized.push(*aabb);
                continue;
            }
            for cx in x0..=x1 {
                for cy in y0..=y1 {
                    buckets.entry((cx, cy)).or_default().push(*aabb);
                }
            }
        }
        Self {
            cell,
            buckets,
            oversized,
        }
    }

    /// Which cell a coordinate falls in, saturating rather than
    /// wrapping — `MAX_CANVAS_COORD` divided by [`Self::MIN_CELL`] is
    /// inside `i32`, but the cast is written to survive input that is
    /// not.
    fn cell_of(v: f32, cell: f32) -> i32 {
        let scaled = (v / cell).floor();
        if scaled >= i32::MAX as f32 {
            i32::MAX
        } else if scaled <= i32::MIN as f32 {
            i32::MIN
        } else {
            scaled as i32
        }
    }

    /// Whether any indexed AABB strictly contains `point`, by the same
    /// rule as [`point_inside_any_node`] — which is the reference this
    /// is property-tested against.
    pub fn contains(&self, point: Vec2) -> bool {
        if point_inside_any_node(point, &self.oversized) {
            return true;
        }
        let key = (
            Self::cell_of(point.x, self.cell),
            Self::cell_of(point.y, self.cell),
        );
        self.buckets
            .get(&key)
            .is_some_and(|bucket| point_inside_any_node(point, bucket))
    }
}

pub fn point_inside_any_node(point: Vec2, aabbs: &[(Vec2, Vec2)]) -> bool {
    const EDGE_EPSILON: f32 = 0.5;
    for (pos, size) in aabbs {
        if point.x > pos.x + EDGE_EPSILON
            && point.x < pos.x + size.x - EDGE_EPSILON
            && point.y > pos.y + EDGE_EPSILON
            && point.y < pos.y + size.y - EDGE_EPSILON
        {
            return true;
        }
    }
    false
}

/// Channel layout inside each per-edge Void parent. Caps live
/// at the ends (1 for start, 1_000_001 for end so it sorts after
/// any plausible body count), body glyphs stride from 100.
const CONN_CAP_START_CHANNEL: usize = 1;
const CONN_BODY_BASE_CHANNEL: usize = 100;
const CONN_CAP_END_CHANNEL: usize = 1_000_001;

/// Per-edge structural identity the in-place mutator path compares
/// against. Matches iff both the edge (`EdgeKey`), its cap presence,
/// and its sampled body-glyph count are unchanged — which is what
/// `align_child_walks` needs to step through each child channel by
/// position without shape drift. Any field changing forces a full
/// rebuild. Plain data; no runtime cost.
pub type ConnectionEdgeIdentity = (
    crate::mindmap::scene_cache::EdgeKey,
    /* has_cap_start */ bool,
    /* body_count    */ usize,
    /* has_cap_end   */ bool,
);

/// Identity sequence for a slice of `ConnectionElement`s. Used by
/// the dispatcher in `update_connection_tree` to choose between
/// full rebuild and the in-place mutator path. During endpoint
/// drag the body-glyph count typically shifts every few pixels and
/// the dispatcher takes the rebuild path; for selection / color
/// preview / theme switches it stays stable and the mutator path
/// runs.
pub fn connection_identity_sequence(elements: &[ConnectionElement]) -> Vec<ConnectionEdgeIdentity> {
    elements
        .iter()
        .map(|e| {
            (
                e.edge_key.clone(),
                e.cap_start.is_some(),
                e.glyph_positions.len(),
                e.cap_end.is_some(),
            )
        })
        .collect()
}

/// Lay out the per-glyph shape data both connection build paths
/// emit. Returns `(edge_channel, Vec<(child_channel, GlyphArea)>)`
/// for one edge. Single source of truth — the initial-build path
/// ([`build_connection_tree`]) and the in-place mutator path
/// ([`build_connection_mutator_tree`]) cannot drift.
fn connection_edge_layout(edge_index: usize, elem: &ConnectionElement) -> (usize, Vec<(usize, GlyphArea)>) {
    let font_size = elem.font_size_pt;
    let half_glyph = font_size * 0.3;
    let half_height = font_size * 0.5;
    let glyph_bounds = Vec2::new(font_size, font_size);
    let color_rgba = color::hex_to_rgba_safe(&elem.color, [0.78, 0.78, 0.78, 1.0]);

    let edge_zoom_window = elem.zoom_visibility;
    let mk_area = |text: &str, pos: Vec2| -> GlyphArea {
        let mut area = GlyphArea::new_with_str(text, font_size, font_size, pos, glyph_bounds);
        area.zoom_visibility = edge_zoom_window;
        let cluster_count = crate::util::grapheme_chad::count_grapheme_clusters(text);
        area.regions = ColorFontRegions::single_span(cluster_count, Some(color_rgba), None);
        area
    };

    let mut children: Vec<(usize, GlyphArea)> = Vec::new();
    if let Some((glyph_text, (cx, cy))) = elem.cap_start.as_ref() {
        let pos = Vec2::new(cx - half_glyph, cy - half_height);
        children.push((CONN_CAP_START_CHANNEL, mk_area(glyph_text, pos)));
    }
    for (i, &(gx, gy)) in elem.glyph_positions.iter().enumerate() {
        let pos = Vec2::new(gx - half_glyph, gy - half_height);
        children.push((CONN_BODY_BASE_CHANNEL + i, mk_area(&elem.body_glyph, pos)));
    }
    if let Some((glyph_text, (cx, cy))) = elem.cap_end.as_ref() {
        let pos = Vec2::new(cx - half_glyph, cy - half_height);
        children.push((CONN_CAP_END_CHANNEL, mk_area(glyph_text, pos)));
    }

    // Per-edge channel: 1-based edge index. Stable across rebuilds
    // for the same identity sequence.
    (edge_index + 1, children)
}

/// Build a baumhard tree of connection glyphs from a slice of
/// pre-computed `ConnectionElement`s.
///
/// Tree shape per visible edge:
///
/// ```text
/// Void (per edge — channel = edge index, 1-based)
/// ├── GlyphArea (cap_start, channel = 1)              // optional
/// ├── GlyphArea (body glyph @ position 0, ch=100+0)
/// ├── GlyphArea (body glyph @ position 1, ch=100+1)
/// │   ...
/// └── GlyphArea (cap_end, channel = 1_000_001)        // optional
/// ```
///
/// Each GlyphArea is sized `(font_size, font_size)` and centered on
/// the connection-element's reported glyph position via the same
/// `(half_glyph, half_height)` offset the legacy
/// `Renderer::rebuild_connection_buffers_keyed` applied. Color is
/// baked into a single `ColorFontRegion` covering the body glyph.
///
/// Channels come from `connection_edge_layout` so the in-place
/// [`build_connection_mutator_tree`] path can target each leaf by
/// the same channel across calls when the structure (body glyph
/// count, cap presence) hasn't changed.
///
/// # Costs
///
/// O(sum of glyph_positions across all elements). Allocates the
/// tree arena. No font shaping happens here — that's the
/// renderer's `walk_tree_into_buffers` step.
pub fn build_connection_tree(elements: &[ConnectionElement]) -> Tree<GfxElement, GfxMutator> {
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    let mut unique_id: usize = 1;

    for (idx, elem) in elements.iter().enumerate() {
        let (edge_channel, children) = connection_edge_layout(idx, elem);
        let edge_root = tree.root.append_value(
            GfxElement::new_void_with_id(edge_channel, unique_id),
            &mut tree.arena,
        );
        unique_id += 1;

        for (channel, area) in children {
            let element = GfxElement::new_area_non_indexed_with_id(area, channel, unique_id);
            unique_id += 1;
            edge_root.append_value(element, &mut tree.arena);
        }
    }

    tree
}

/// Build a [`MutatorTree`](crate::gfx_structs::tree::MutatorTree)
/// that updates an already-registered connection tree to the
/// current `elements` state without rebuilding the arena. Pairs
/// with [`build_connection_tree`] — both consume
/// `connection_edge_layout`, so applying this mutator to a tree
/// built from an element slice with the same
/// [`connection_identity_sequence`] updates each glyph's variable
/// fields in place.
pub fn build_connection_mutator_tree(
    elements: &[ConnectionElement],
) -> crate::gfx_structs::tree::MutatorTree<GfxMutator> {
    use crate::gfx_structs::area::DeltaGlyphArea;
    use crate::gfx_structs::mutator::Mutation;
    use crate::gfx_structs::tree::MutatorTree;

    let mut mt: MutatorTree<GfxMutator> = MutatorTree::new_with(GfxMutator::new_void(0));
    for (idx, elem) in elements.iter().enumerate() {
        let (edge_channel, children) = connection_edge_layout(idx, elem);
        let edge_node = mt
            .root
            .append_value(GfxMutator::new_void(edge_channel), &mut mt.arena);

        for (channel, area) in children {
            let delta = DeltaGlyphArea::full_assign_from(&area);
            edge_node.append_value(
                GfxMutator::new(Mutation::AreaDelta(Box::new(delta)), channel),
                &mut mt.arena,
            );
        }
    }
    mt
}

#[cfg(test)]
mod clip_index_tests {
    use super::{point_inside_any_node, NodeClipIndex};
    use crate::util::geometry::aabb_center;
    use glam::Vec2;

    /// Deterministic pseudo-random source. A fixed LCG rather than
    /// `rand`, so a failure reproduces exactly from the seed printed in
    /// the assertion.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 11
        }
        fn f32_in(&mut self, lo: f32, hi: f32) -> f32 {
            lo + (self.next() % 100_000) as f32 / 100_000.0 * (hi - lo)
        }
    }

    /// **The oversized path, which the random generator does not
    /// reach.**
    ///
    /// A box goes in `oversized` only when it spans more than
    /// `MAX_CELLS_PER_AABB`, and the cell size is the mean node extent
    /// — so a map whose boxes are all roughly one size never produces
    /// one, however large they are. It takes many small boxes (which
    /// pull the cell small) plus one enormous one. Verified by deleting
    /// the `oversized` check: without this case the suite stayed green,
    /// which is the whole reason it is written separately.
    #[test]
    fn test_a_box_far_larger_than_the_cell_is_still_checked() {
        let mut aabbs: Vec<(Vec2, Vec2)> = (0..40)
            .map(|i| (Vec2::new(i as f32 * 12.0, 0.0), Vec2::new(10.0, 10.0)))
            .collect();
        // Mean extent is ~10, so the cell is ~10 and this spans
        // 500 x 500 cells — far past the 64-cell limit.
        aabbs.push((Vec2::new(-2500.0, -2500.0), Vec2::new(5000.0, 5000.0)));
        let index = NodeClipIndex::build(&aabbs);

        let deep_inside = Vec2::new(-1000.0, -1000.0);
        assert!(
            point_inside_any_node(deep_inside, &aabbs),
            "fixture check: the scan must see this point inside the big box"
        );
        assert!(
            index.contains(deep_inside),
            "a box too large to bucket must still be checked — otherwise the index \
             silently stops clipping against the biggest nodes on the canvas"
        );

        // And it must not over-report: a point outside everything.
        let outside = Vec2::new(-9000.0, -9000.0);
        assert_eq!(index.contains(outside), point_inside_any_node(outside, &aabbs));
    }

    /// **The index must answer exactly what the scan answered.**
    ///
    /// It replaces `point_inside_any_node` on the hot path, so a
    /// disagreement is a connection glyph drawn inside a node or
    /// clipped when it should not be — a rendering defect that no
    /// timing improvement would justify. This holds the two to
    /// bit-identical verdicts over random geometry, including the cases
    /// the structure is most likely to get wrong: points exactly on a
    /// border (the `EDGE_EPSILON` rule), boxes that straddle cell
    /// boundaries, boxes far larger than a cell, negative coordinates,
    /// and degenerate zero-size boxes.
    #[test]
    fn test_the_clip_index_agrees_with_the_linear_scan() {
        let mut rng = Lcg(0x5EED_1234_ABCD_0001);
        let mut checked = 0usize;
        let mut inside_seen = 0usize;

        for round in 0..40 {
            // Mix of ordinary, huge, and degenerate boxes.
            let count = 1 + (rng.next() % 40) as usize;
            let aabbs: Vec<(Vec2, Vec2)> = (0..count)
                .map(|i| {
                    let pos = Vec2::new(rng.f32_in(-4000.0, 4000.0), rng.f32_in(-4000.0, 4000.0));
                    let size = match i % 5 {
                        0 => Vec2::new(0.0, 0.0),
                        1 => Vec2::new(rng.f32_in(2000.0, 9000.0), rng.f32_in(2000.0, 9000.0)),
                        _ => Vec2::new(rng.f32_in(1.0, 300.0), rng.f32_in(1.0, 300.0)),
                    };
                    (pos, size)
                })
                .collect();
            let index = NodeClipIndex::build(&aabbs);

            for _ in 0..200 {
                // Half free points, half points aimed at a box's
                // corners and edges, where the epsilon rule decides.
                let point = if rng.next().is_multiple_of(2) {
                    Vec2::new(rng.f32_in(-5000.0, 5000.0), rng.f32_in(-5000.0, 5000.0))
                } else {
                    let (pos, size) = aabbs[(rng.next() as usize) % aabbs.len()];
                    let pick = rng.next() % 6;
                    match pick {
                        0 => pos,
                        1 => pos + size,
                        2 => aabb_center(pos, size),
                        3 => Vec2::new(pos.x + size.x, pos.y),
                        4 => Vec2::new(pos.x + 0.5, pos.y + 0.5),
                        _ => Vec2::new(pos.x + size.x - 0.5, pos.y + size.y - 0.5),
                    }
                };
                let want = point_inside_any_node(point, &aabbs);
                let got = index.contains(point);
                assert_eq!(
                    got, want,
                    "round {round}: index and scan disagree at {point:?} \
                     (scan says {want}, index says {got}) over {count} boxes"
                );
                checked += 1;
                inside_seen += usize::from(want);
            }
        }

        // Guard against a vacuous pass: the run must have produced both
        // verdicts, or agreement proves nothing.
        assert!(checked > 5000, "too few samples to mean anything: {checked}");
        assert!(
            inside_seen > 100 && inside_seen < checked - 100,
            "the sample must contain both inside and outside points, got {inside_seen} \
             inside of {checked}"
        );
    }

    /// An empty clip list contains nothing, and a non-finite box is not
    /// a trap — both are reachable from an authored map.
    #[test]
    fn test_the_clip_index_handles_empty_and_non_finite_inputs() {
        let empty = NodeClipIndex::build(&[]);
        assert!(!empty.contains(Vec2::new(0.0, 0.0)));

        let odd = [
            (Vec2::new(f32::NAN, 0.0), Vec2::new(10.0, 10.0)),
            (Vec2::new(0.0, 0.0), Vec2::new(f32::INFINITY, 10.0)),
            (Vec2::new(10.0, 10.0), Vec2::new(20.0, 20.0)),
        ];
        let index = NodeClipIndex::build(&odd);
        for p in [
            Vec2::new(0.0, 0.0),
            Vec2::new(15.0, 15.0),
            Vec2::new(-1.0, -1.0),
            Vec2::new(f32::NAN, 1.0),
        ] {
            assert_eq!(
                index.contains(p),
                point_inside_any_node(p, &odd),
                "index and scan must agree at {p:?} even with non-finite geometry"
            );
        }
    }
}
