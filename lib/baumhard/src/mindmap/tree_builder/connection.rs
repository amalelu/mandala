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
//!   delta, cache hit at matching font size): shift the cached
//!   samples by the shared delta and rerun the clip filter. Skips
//!   `build_connection_path` + `sample_path` entirely — the
//!   subtree-drag hot path.
//! - **Slow path** (otherwise): build the path, resample, write
//!   the cache.
//!
//! The clip filter runs against the current frame's `node_aabbs`
//! on all three paths so a stable edge correctly clips around
//! third nodes that moved through its path this frame.
//!
//! Cache lifecycle: `ensure_zoom` is caller-managed (the caller
//! flushes on zoom change before any pass starts); `retain_keys`
//! runs here at the end of the loop so eviction of deleted edges
//! stays colocated with the keys-seen bookkeeping.
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
use crate::font::metrics::monospace_advance;
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::GfxMutator;
use crate::gfx_structs::tree::Tree;
use crate::mindmap::connection;
use crate::mindmap::model::{GlyphConnectionConfig, MindMap};
use crate::mindmap::scene_cache::{CachedConnection, EdgeKey, SceneConnectionCache};
use crate::mindmap::SELECTION_HIGHLIGHT_HEX;
use crate::util::color;
use crate::util::color::resolve_var;

use super::edge_handle::{build_edge_handles, EdgeHandleElement};
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
/// for the three previous emit sites (cache-hit fast path,
/// translate path, slow rebuild) — they each filtered cap_start /
/// cap_end / glyph_positions against `node_aabbs`, applied the
/// "all-clipped → skip" rule, and pushed an identical struct
/// literal. Returns `true` iff an element was actually pushed
/// (callers don't currently use it; a future "did this edge clip
/// out?" check would).
#[allow(clippy::too_many_arguments)]
fn emit_connection_element(
    out: &mut Vec<ConnectionElement>,
    edge_key: EdgeKey,
    body_glyph: String,
    font: Option<String>,
    font_size_pt: f32,
    color: String,
    zoom_visibility: crate::gfx_structs::zoom_visibility::ZoomVisibility,
    cap_start_raw: Option<(String, Vec2)>,
    cap_end_raw: Option<(String, Vec2)>,
    pre_clip_positions: &[Vec2],
    node_aabbs: &[(Vec2, Vec2)],
) -> bool {
    let cap_start = cap_start_raw
        .filter(|(_, p)| !point_inside_any_node(*p, node_aabbs))
        .map(|(g, p)| (g, (p.x, p.y)));
    let cap_end = cap_end_raw
        .filter(|(_, p)| !point_inside_any_node(*p, node_aabbs))
        .map(|(g, p)| (g, (p.x, p.y)));
    let glyph_positions: Vec<(f32, f32)> = pre_clip_positions
        .iter()
        .filter(|p| !point_inside_any_node(**p, node_aabbs))
        .map(|p| (p.x, p.y))
        .collect();
    if glyph_positions.is_empty() && cap_start.is_none() && cap_end.is_none() {
        return false;
    }
    out.push(ConnectionElement {
        edge_key,
        glyph_positions,
        body_glyph,
        cap_start,
        cap_end,
        font,
        font_size_pt,
        color,
        zoom_visibility,
    });
    true
}

/// Emit connection elements + edge-handle elements. Consumes
/// `node_aabbs` from [`super::node_clip_aabbs`] for the clip filter;
/// mutates `cache` on slow-path edges and after the loop
/// (`retain_keys` evicts deleted edges).
///
/// # Costs
///
/// O(visible edges). Each edge takes the fast, translate, or slow
/// path (see the module header); only the slow path runs
/// `build_connection_path` + `sample_path`. The clip filter is
/// O(samples x visible nodes) on every path.
// `clippy::too_many_arguments`: each argument has a different
// caller-vs-pass lifetime relationship — `map` / `offsets` borrow
// the persisted document, `node_aabbs` is this frame's clip input,
// the selection and preview are per-frame snapshots, `cache` is
// mut-borrowed across frames for memoization, and `camera_zoom` is
// by-value. Bundling them would either duplicate every field's
// lifetime annotation or hide the borrow shape from the caller.
#[allow(clippy::too_many_arguments)]
pub fn build_connection_elements(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    node_aabbs: &[(Vec2, Vec2)],
    selected_edge: Option<(&str, &str, &str)>,
    edge_color_preview: Option<EdgeColorPreview<'_>>,
    cache: &mut SceneConnectionCache,
    camera_zoom: f32,
    hidden_set: &HashSet<&str>,
) -> (Vec<ConnectionElement>, Vec<EdgeHandleElement>) {
    let vars = &map.canvas.theme_variables;
    let default_config = GlyphConnectionConfig::default();
    let mut connection_elements = Vec::new();
    // Grab-handles for the currently selected edge. Populated at most
    // once per scene build (selection is single-edge); empty otherwise.
    let mut edge_handles: Vec<EdgeHandleElement> = Vec::new();
    // Keys seen this frame — used after the loop to evict stale cache
    // entries for edges that were removed from the model between builds.
    let mut seen_keys: HashSet<EdgeKey> = HashSet::with_capacity(map.edges.len());

    for edge in &map.edges {
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
        seen_keys.insert(edge_key.clone());

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
            let (fox, foy) = offsets.get(&from_node.id).copied().unwrap_or((0.0, 0.0));
            let (tox, toy) = offsets.get(&to_node.id).copied().unwrap_or((0.0, 0.0));
            let from_pos = from_node.pos_vec2() + Vec2::new(fox, foy);
            let from_size = from_node.size_vec2();
            let to_pos = to_node.pos_vec2() + Vec2::new(tox, toy);
            let to_size = to_node.size_vec2();
            edge_handles.extend(build_edge_handles(
                edge, &edge_key, from_pos, from_size, to_pos, to_size,
            ));
        }

        // Did either endpoint of THIS edge move this frame?
        let endpoint_moved = offsets.contains_key(&from_node.id) || offsets.contains_key(&to_node.id);

        // --- Fast path: cached geometry is still valid ---
        //
        // If the endpoints haven't moved and we have a cached entry for
        // this edge, reuse the cached pre-clip samples and skip
        // `build_connection_path` / `sample_path` entirely. The cheap
        // clip filter still runs against THIS frame's `node_aabbs` so a
        // stable edge correctly clips around a third node that moved
        // through its path.
        // Color picker preview: resolve once here so both the cached
        // and slow paths pick it up. Preview beats selection on the
        // previewed edge so the user's live feedback is visible on the
        // connection body, not masked by the cyan selection highlight.
        let preview_for_this_edge: Option<&str> = edge_color_preview.and_then(|p| {
            if *p.edge_key == edge_key {
                Some(p.color)
            } else {
                None
            }
        });

        if !endpoint_moved {
            if let Some(cached) = cache.get(&edge_key) {
                let color = if let Some(p) = preview_for_this_edge {
                    resolve_var(p, vars).to_string()
                } else if is_selected {
                    SELECTION_HIGHLIGHT_HEX.to_string()
                } else {
                    cached.color.clone()
                };
                emit_connection_element(
                    &mut connection_elements,
                    edge_key,
                    cached.body_glyph.clone(),
                    cached.font.clone(),
                    cached.font_size_pt,
                    color,
                    edge.zoom_window(),
                    cached.cap_start.clone(),
                    cached.cap_end.clone(),
                    &cached.pre_clip_positions,
                    node_aabbs,
                );
                continue;
            }
        }

        // Canvas-space font size clamped to keep the on-screen glyph
        // size inside [min_font_size_pt, max_font_size_pt]. At extreme
        // zoom-out this inflates the canvas-space size so sample
        // spacing grows and the per-edge glyph count falls — the LOD
        // mechanism that keeps zoomed-out connections from becoming a
        // dust cloud. Inversely, at extreme zoom-in the effective font
        // size shrinks, spacing shrinks, and per-edge sample count
        // rises linearly with zoom — which is what blows the drag
        // drain budget at high zoom. The translate path below is what
        // keeps subtree-drag cost bounded there.
        let font_size = config.effective_font_size_pt(camera_zoom);
        let approx_glyph_width = monospace_advance(font_size);
        let effective_spacing = approx_glyph_width + config.spacing;

        let (fox, foy) = offsets.get(&from_node.id).copied().unwrap_or((0.0, 0.0));
        let (tox, toy) = offsets.get(&to_node.id).copied().unwrap_or((0.0, 0.0));

        let from_pos = from_node.pos_vec2() + Vec2::new(fox, foy);
        let from_size = from_node.size_vec2();
        let to_pos = to_node.pos_vec2() + Vec2::new(tox, toy);
        let to_size = to_node.size_vec2();

        // --- Translate path: rigid-body subtree-drag optimization ---
        //
        // The common case inside a subtree drag: both endpoints of an
        // internal edge are in `offsets` with the SAME delta, so the
        // edge is a pure translation of its last-sampled geometry.
        // When the cache has a fresh entry at the same font size AND
        // the same glyph-config snapshot (body / font), we can skip
        // `build_connection_path` + `sample_path` entirely and shift
        // the cached samples by the shared delta.
        //
        // Fall-throughs to the slow path:
        // - Boundary edges (one endpoint moved, one not; or both moved
        //   by different deltas — a rotating / stretching edge).
        // - Zoom-change frames (`font_size != cached.font_size_pt`).
        // - Glyph-config mutation mid-drag (console edits to
        //   `edge.glyph_connection.body` / `font`) — cached values are
        //   frozen at sample time, so the slow path resamples and
        //   re-caches with the new config.
        //
        // The probe is split in two phases so the borrow checker lets
        // us mutate the cache after reading it: first a read-only
        // `cache.get` computes the delta (returned as a value), then
        // `cache.translate_in_place` mutates the entry's positions
        // without re-indexing `by_node` or cloning the string fields.
        let translate_delta = cache.get(&edge_key).and_then(|cached| {
            if crate::util::geometry::pretty_inequal(cached.font_size_pt, font_size) {
                return None;
            }
            if cached.body_glyph != config.body {
                return None;
            }
            if cached.font != config.font {
                return None;
            }
            let delta_from = from_pos - cached.base_from;
            let delta_to = to_pos - cached.base_to;
            if (delta_from - delta_to).length_squared() >= TRANSLATE_DELTA_EPSILON_SQ {
                return None;
            }
            Some(delta_from)
        });

        if let Some(delta) = translate_delta {
            // Mutate the cached entry in place. No by_node reindex,
            // no string clones on the hot path. `translate_in_place`
            // returns a borrow of the mutated entry so we can emit
            // the element without a follow-up `get`.
            let entry = cache
                .translate_in_place(&edge_key, delta, from_pos, to_pos)
                .expect("entry was present in the guard above");

            let color = if let Some(p) = preview_for_this_edge {
                resolve_var(p, vars).to_string()
            } else if is_selected {
                SELECTION_HIGHLIGHT_HEX.to_string()
            } else {
                entry.color.clone()
            };

            // Clip filter runs every frame against this frame's
            // node_aabbs — an unrelated moved node passing through
            // a translated edge must still clip out the glyphs
            // inside it.
            emit_connection_element(
                &mut connection_elements,
                edge_key,
                entry.body_glyph.clone(),
                entry.font.clone(),
                font_size,
                color,
                edge.zoom_window(),
                entry.cap_start.clone(),
                entry.cap_end.clone(),
                &entry.pre_clip_positions,
                node_aabbs,
            );
            continue;
        }

        // --- Slow path: sample fresh and update the cache ---
        let stored_color = {
            // The color we STORE in the cache is the resolved-but-unselected
            // color. Selection overrides are applied at read time above so
            // selection changes don't invalidate the cache.
            resolve_var(edge.body_color(&map.canvas), vars).to_string()
        };
        let color = if let Some(p) = preview_for_this_edge {
            resolve_var(p, vars).to_string()
        } else if is_selected {
            SELECTION_HIGHLIGHT_HEX.to_string()
        } else {
            stored_color.clone()
        };

        let path = connection::build_connection_path(
            from_pos,
            from_size,
            &edge.anchor_from,
            to_pos,
            to_size,
            &edge.anchor_to,
            &edge.control_points,
        );
        let samples = connection::sample_path(&path, effective_spacing);
        if samples.is_empty() {
            // Edge produces no samples; make sure any stale cache entry is
            // dropped so we re-try next frame.
            cache.invalidate_edge(&edge_key);
            continue;
        }

        // Caps live at the ORIGINAL first and last sample positions (the
        // anchor points resolved from the source/target node bounds).
        // Those points sit on the raw node edge — which is ON the clip
        // AABB boundary for an unframed node (so they survive clipping)
        // but INSIDE the expanded clip AABB for a framed node (so they
        // get dropped along with the body glyphs that would also render
        // inside the frame area).
        let first_pos = samples[0].position;
        let last_pos = samples.last().unwrap().position;
        let cached_cap_start = config.cap_start.as_ref().map(|g| (g.clone(), first_pos));
        let cached_cap_end = config.cap_end.as_ref().map(|g| (g.clone(), last_pos));

        let pre_clip_positions: Vec<Vec2> = samples.iter().map(|s| s.position).collect();

        // Write fresh geometry back into the cache BEFORE applying the
        // frame-specific clip filter so next frame can reuse it.
        cache.insert(
            edge_key.clone(),
            CachedConnection {
                pre_clip_positions: pre_clip_positions.clone(),
                cap_start: cached_cap_start.clone(),
                cap_end: cached_cap_end.clone(),
                body_glyph: config.body.clone(),
                font: config.font.clone(),
                font_size_pt: font_size,
                color: stored_color,
                base_from: from_pos,
                base_to: to_pos,
            },
        );

        // Now produce the post-clip element for THIS frame.
        emit_connection_element(
            &mut connection_elements,
            edge_key,
            config.body.clone(),
            config.font.clone(),
            font_size,
            color,
            edge.zoom_window(),
            cached_cap_start,
            cached_cap_end,
            &pre_clip_positions,
            node_aabbs,
        );
    }

    // Evict any cache entries for edges that were in the cache but NOT in
    // the map this frame — handles edges that were deleted between builds.
    cache.retain_keys(&seen_keys);

    (connection_elements, edge_handles)
}

/// Returns true if `point` is strictly inside any of the given AABBs. Uses a
/// small epsilon so points that sit exactly on a border (e.g. connection
/// anchor points, which are placed at node-edge midpoints) are NOT
/// considered inside — that would accidentally clip the endpoints.
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
        let edge_root = tree
            .arena
            .new_node(GfxElement::new_void_with_id(edge_channel, unique_id));
        unique_id += 1;
        tree.root.append(edge_root, &mut tree.arena);

        for (channel, area) in children {
            let element = GfxElement::new_area_non_indexed_with_id(area, channel, unique_id);
            unique_id += 1;
            let leaf = tree.arena.new_node(element);
            edge_root.append(leaf, &mut tree.arena);
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
        let edge_node = mt.arena.new_node(GfxMutator::new_void(edge_channel));
        mt.root.append(edge_node, &mut mt.arena);

        for (channel, area) in children {
            let delta = DeltaGlyphArea::full_assign_from(&area);
            let leaf = mt
                .arena
                .new_node(GfxMutator::new(Mutation::AreaDelta(Box::new(delta)), channel));
            edge_node.append(leaf, &mut mt.arena);
        }
    }
    mt
}
