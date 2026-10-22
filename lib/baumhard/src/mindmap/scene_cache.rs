// SPDX-License-Identifier: MPL-2.0

//! Per-edge cache of connection glyph geometry.
//!
//! Why it exists: during a drag the scene builder would otherwise
//! re-sample every visible edge every frame, even though most edges
//! have not moved. For a 20,000-unit cross-link at typical spacing
//! that is ~1,667 Bezier point evaluations + a 256-entry arc-length
//! table rebuild per frame per such edge, which is more than enough
//! to blow the drag budget and stutter the interaction.
//!
//! This module lets the scene builder stash the **pre-clip** sampled
//! positions of each edge keyed by `(from_id, to_id, edge_type)`. On the
//! next frame, if neither endpoint of the edge has moved (i.e. neither
//! appears in the drag `offsets` map) the cached samples are reused — the
//! cheap `point_inside_any_node` clip filter still runs against the current
//! frame's `node_aabbs` so a stable edge still clips correctly around a
//! *moved* third node that passes through its path.
//!
//! Invariants:
//!
//! - The cache is always safe to drop. Clearing it just forces a full
//!   re-sample on the next build.
//! - Samples are stored in canvas space. Camera *pan* does not invalidate
//!   the cache. Camera *zoom*, however, DOES change the effective
//!   canvas-space font size (and therefore the sample spacing) via
//!   `GlyphConnectionConfig::effective_font_size_pt`, so zoom changes
//!   force a full re-sample. This is enforced automatically by
//!   `ensure_zoom`, which the scene builder calls on entry — callers
//!   don't need to remember to flush the cache on zoom themselves.
//! - **An entry holds no styling the frame could read back.** What a
//!   connection *looks like* — its body glyph, its cap glyphs, its
//!   font, its size and its color — is resolved from the live model on
//!   every path, so no styling edit can be served stale out of this
//!   module: there is nothing here to serve. What an entry does hold is
//!   the geometry plus the
//!   [`SampleParams`](crate::mindmap::scene_cache::SampleParams) that
//!   geometry was produced under, and both reuse doors —
//!   [`reusable`](crate::mindmap::scene_cache::SceneConnectionCache::reusable)
//!   and
//!   [`reusable_mut`](crate::mindmap::scene_cache::SceneConnectionCache::reusable_mut)
//!   — refuse an entry whose params no longer match the frame's.
//!
//! (Those three links are spelled crate-absolute on purpose. `mod.rs`
//! carries an outer `///` summary on `pub mod scene_cache;`, rustdoc
//! merges it with this header, and a relative link in the merged block
//! resolves against no module — the failure reports no file or line, so
//! it costs more to find than to avoid.)
//! - What the caller still owes is **geometry the params cannot see**,
//!   and only that: an endpoint that moved in the model without
//!   appearing in the drag `offsets` map, an anchor or control-point
//!   edit, and a node resize. Those are `invalidate_edge` or `clear`.
//!   Nothing about *color* is on that list any more — neither a
//!   theme-variable edit nor a direct `edge.color` / `glyph_connection.color`
//!   edit, because the color is resolved from the model on every
//!   frame and never enters the cache. Selection changes need nothing
//!   either, for the same reason.

use glam::Vec2;
use std::collections::HashMap;

use crate::font::metrics::monospace_advance;
use crate::mindmap::model::{GlyphConnectionConfig, MindEdge};
use crate::util::geometry::almost_equal;

/// Stable identity of a connection. Mirrors the `(from_id, to_id, edge_type)`
/// triple that the rest of the codebase uses to identify edges
/// (`document::EdgeRef` is the same shape).
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub struct EdgeKey {
    pub from_id: String,
    pub to_id: String,
    pub edge_type: String,
}

impl EdgeKey {
    /// Construct an `EdgeKey` from its three components. Accepts
    /// anything `Into<String>` so callers can pass `&str` or `String`
    /// without `.to_string()` boilerplate. O(1) + up to 3 allocations.
    pub fn new(from_id: impl Into<String>, to_id: impl Into<String>, edge_type: impl Into<String>) -> Self {
        Self {
            from_id: from_id.into(),
            to_id: to_id.into(),
            edge_type: edge_type.into(),
        }
    }

    /// Shorthand: derive the key from a `MindEdge`'s identity fields.
    /// O(1) + 3 string clones.
    pub fn from_edge(edge: &MindEdge) -> Self {
        Self::new(&edge.from_id, &edge.to_id, &edge.edge_type)
    }
}

/// Every input to a connection's glyph sampling that comes from
/// configuration rather than from geometry — which is to say,
/// everything that decides *where along the path* the samples land
/// and how many of them there are.
///
/// The list is short because the sampler's is:
/// `sample_path(&path, effective_spacing, budget)` reads a step and a
/// cap, `build_connection_path` reads only geometry, and
/// [`monospace_advance`] scales the font size by a constant with no
/// glyph input at all. So the *body glyph*, the *cap glyphs* and the
/// *font family* are absent on purpose — none of them can move a
/// sample, and a struct that compared them would refuse reuse of
/// geometry that is in fact still correct.
///
/// The point of having the type is that the cache's two reuse doors
/// compare *this whole value* rather than a hand-listed subset of an
/// entry's fields. Before it existed the translate path compared three
/// fields and the cache-hit fast path compared none, which is the same
/// defect one level down: whichever list is shorter is the one that
/// serves stale geometry. A field added here is compared by both doors
/// or by neither, and never by one (#36 item 7).
///
/// Plain `Copy` data — two floats and a `usize`, no allocation.
#[derive(Clone, Copy, Debug)]
pub struct SampleParams {
    /// `GlyphConnectionConfig::effective_font_size_pt(camera_zoom)`:
    /// the canvas-space size after the screen-space clamp. A camera
    /// zoom change and a `font_size_pt` / `min_` / `max_` edit both
    /// land here, which is why the field is the *effective* size
    /// rather than the authored one.
    pub font_size_pt: f32,
    /// `GlyphConnectionConfig::spacing` — added to the glyph advance
    /// to get the arc-length step between consecutive body glyphs.
    pub spacing: f32,
    /// The per-path allowance
    /// [`crate::mindmap::connection::per_path_sample_budget`] handed
    /// this pass. Scene-wide in origin but per-edge in effect: it caps
    /// *this* edge's sample count, so a map whose edge count changed
    /// can produce a different sample sequence from identical per-edge
    /// config.
    pub sample_budget: usize,
}

impl SampleParams {
    /// Snapshot the sampling inputs one pass is about to read for one
    /// edge.
    ///
    /// Takes `camera_zoom` rather than a pre-computed size so the
    /// screen-space clamp is applied in exactly one place; a caller
    /// that needs the canvas-space size reads [`Self::font_size_pt`]
    /// back off the result.
    ///
    /// Cost: the arithmetic of
    /// `GlyphConnectionConfig::effective_font_size_pt`. No allocation.
    pub fn snapshot(config: &GlyphConnectionConfig, camera_zoom: f32, sample_budget: usize) -> Self {
        Self {
            font_size_pt: config.effective_font_size_pt(camera_zoom),
            spacing: config.spacing,
            sample_budget,
        }
    }

    /// Arc-length step between consecutive body glyphs: one monospace
    /// advance at [`Self::font_size_pt`] plus [`Self::spacing`].
    ///
    /// The sampler reads its step through this method so the step and
    /// the value the cache compares cannot be derived two different
    /// ways. Cost: O(1).
    pub fn sample_spacing(&self) -> f32 {
        monospace_advance(self.font_size_pt) + self.spacing
    }

    /// Whether samples taken under `self` are the ones `other` would
    /// produce — the whole freshness test for cached geometry, in one
    /// place.
    ///
    /// Deliberately **not** a `PartialEq` impl. Both float fields are
    /// compared with [`almost_equal`] rather than `==`, and
    /// almost-equality is not transitive, so it is not an equivalence
    /// relation and writing it as `==` would promise a contract this
    /// cannot keep. The tolerance is wanted on both: `font_size_pt` is
    /// an arithmetic result that drifts in its last bits under the
    /// sub-`ZOOM_EPSILON` zoom wobble `ensure_zoom` deliberately
    /// tolerates, and a `spacing` difference below the tolerance moves
    /// no sample by more than the same tolerance.
    ///
    /// Cost: O(1).
    pub fn matches(&self, other: &Self) -> bool {
        self.sample_budget == other.sample_budget
            && almost_equal(self.font_size_pt, other.font_size_pt)
            && almost_equal(self.spacing, other.spacing)
    }
}

/// The cached geometry for a single edge, plus the [`SampleParams`]
/// it was produced under — together sufficient to rebuild the edge's
/// `ConnectionElement` without recomputing the path.
///
/// `pre_clip_positions` holds the raw sampled points BEFORE the
/// `point_inside_any_node` clip filter runs. We keep them pre-clip so a
/// moved-but-unrelated node's AABB can still push glyphs out of the
/// connection on the next frame: the clip filter is cheap (arithmetic over
/// cached `Vec2`s), the sampler is not.
///
/// **The caps are not stored, because they were never separate data.**
/// A cap sits at the first or last sampled position and carries a glyph
/// named in the live `GlyphConnectionConfig`, so the emitting pass
/// reads both ends of `pre_clip_positions` and pairs them with the
/// glyphs itself. Storing them was a second copy of both halves that
/// [`Self::translate`] then had to keep in step.
///
/// **Nor is anything else the frame draws with, and there is no
/// exception.** Body glyph, font family, font size and the body color
/// were all held here and read back by the reuse paths, which is what
/// made a styling edit invisible until something flushed the cache.
/// The color was the last of them and was removed for the same reason
/// as the other four: it was handed back by both reuse doors and
/// compared by neither, so its freshness rested on a caller
/// remembering. What is left is the sampled geometry, the params it
/// was taken under, and the endpoints it was taken at.
///
/// `base_from` / `base_to` record the endpoint canvas positions that the
/// samples were taken at (i.e. `model.pos + offset_at_write`). When the next
/// frame brings a drag offset that moves both endpoints by the same delta
/// — the common subtree-drag case — the scene builder can skip the Bezier
/// sampler entirely and just translate the cached samples by that shared
/// delta. Anything that changes the edge's *shape* (endpoints moving by
/// different deltas, control-point edits, font-size / zoom clamp
/// transitions) falls through to a full resample.
#[derive(Clone, Debug)]
pub struct CachedConnection {
    pub pre_clip_positions: Vec<Vec2>,
    pub sample_params: SampleParams,
    pub base_from: Vec2,
    pub base_to: Vec2,
    /// The pass generation this entry was last handed out or written
    /// in — see [`SceneConnectionCache::begin_pass`]. Set by the cache
    /// on every route that touches the entry, and read only by
    /// [`SceneConnectionCache::evict_unseen`]; a caller constructing
    /// an entry may leave it at zero, which the
    /// [`refill`](SceneConnectionCache::refill) that stores it
    /// overwrites.
    pub last_seen: u64,
}

impl CachedConnection {
    /// Rigid-body translate of this entry's geometry in place.
    /// Shifts `pre_clip_positions` by `delta` and stamps `base_from` /
    /// `base_to` to the new reference endpoints. The caps ride along
    /// because the emitting pass reads them off the ends of those same
    /// points.
    ///
    /// Why the whole entry is mutated in place instead of being
    /// rewritten through [`SceneConnectionCache::refill`]: this runs on
    /// every internal edge of a subtree drag every drain, and a
    /// translation is one pass over points the entry already holds.
    /// Going through `refill` would re-derive them from the path
    /// instead — which is the sampling this path exists to skip.
    ///
    /// [`Self::sample_params`] is left alone by design: a rigid
    /// translation changes where the samples are, never what they were
    /// sampled under, and the caller reached this method only by
    /// presenting matching params to
    /// [`SceneConnectionCache::reusable_mut`].
    ///
    /// Cost: one pass over `pre_clip_positions`.
    pub fn translate(&mut self, delta: Vec2, new_base_from: Vec2, new_base_to: Vec2) {
        for p in &mut self.pre_clip_positions {
            *p += delta;
        }
        self.base_from = new_base_from;
        self.base_to = new_base_to;
    }
}

/// Per-edge cache of sampled connection geometry, plus a reverse
/// index from node ID → edges that touch it so a drag of node N
/// dirties the right edges in `O(k_N)` instead of walking the whole
/// edge list. Owned by the app's document / renderer glue and passed
/// into [`crate::mindmap::tree_builder::build_connection_elements`] on
/// each frame.
#[derive(Default, Debug)]
pub struct SceneConnectionCache {
    entries: HashMap<EdgeKey, CachedConnection>,
    by_node: HashMap<String, Vec<EdgeKey>>,
    /// Camera zoom level at which the cached samples were taken. `None`
    /// means "cache is empty / zoom unknown". When the scene builder is
    /// asked to build at a zoom that differs from this (beyond
    /// `ZOOM_EPSILON`), `ensure_zoom` flushes the cache so stale sample
    /// spacings don't leak into the new frame. Kept out of
    /// `CachedConnection` because it's a whole-cache property, not a
    /// per-edge one.
    scene_zoom: Option<f32>,
    /// Monotonic pass counter. [`SceneConnectionCache::begin_pass`]
    /// bumps it; every route that hands out or writes an entry stamps
    /// that entry with it; [`SceneConnectionCache::evict_unseen`]
    /// drops whatever still carries an older one.
    ///
    /// This replaces the `HashSet<EdgeKey>` the pass used to build
    /// from scratch every frame — one allocation plus an owned
    /// three-`String` key per visible edge, to answer a question the
    /// cache can answer about itself. It also makes the bookkeeping
    /// **structural rather than remembered**: a caller cannot obtain
    /// an entry to reuse without the cache marking it seen, because
    /// marking it is what the reuse doors do.
    generation: u64,
}

/// Threshold for "zoom changed enough to invalidate cached samples".
/// The sample spacing is `effective_font * 0.6 + spacing`, so sub-0.1%
/// zoom deltas shift spacing by a fraction of a pixel — cheaper to
/// ignore than to rebuild. 0.1% matches the lower bound of what a user
/// can deliberately produce with the wheel-zoom step (10%).
const ZOOM_EPSILON: f32 = 1.0e-3;

impl SceneConnectionCache {
    /// Construct an empty cache. Same as `Self::default()` — the
    /// explicit constructor exists so callers don't have to know the
    /// `Default` trait is derived. Allocation-free.
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop everything. Used at drag-drop, undo, reparent, edge CRUD, fold
    /// toggle, theme-variable change — the cheap "when in doubt, flush"
    /// path. The next scene build re-populates the cache.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.by_node.clear();
        self.scene_zoom = None;
    }

    /// Ensure the cache is consistent with `camera_zoom`. If the stored
    /// zoom differs from the incoming one beyond `ZOOM_EPSILON`, drop
    /// all cached samples; either way, stamp the new zoom. Called by
    /// the connection pass on entry so the invariant is enforced
    /// locally instead of requiring every caller to remember to flush
    /// on zoom changes.
    ///
    /// When `scene_zoom` is `None` (fresh cache, post-`clear`, or
    /// pre-stamp) we just stamp without invalidating — any existing
    /// entries are assumed to be correct for `camera_zoom` (in
    /// production the scene builder stamps before inserting).
    pub fn ensure_zoom(&mut self, camera_zoom: f32) {
        let z = camera_zoom.max(f32::EPSILON);
        if let Some(prev) = self.scene_zoom {
            if (prev - z).abs() > ZOOM_EPSILON {
                self.entries.clear();
                self.by_node.clear();
            }
        }
        self.scene_zoom = Some(z);
    }

    /// `true` iff no cached entries are currently held. O(1).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of cached edge entries. O(1).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Look up a cached entry **whatever it was sampled under** — for
    /// diagnostics, tests, and any consumer that wants to see what is
    /// held rather than to reuse it.
    ///
    /// Reuse goes through [`Self::reusable`] / [`Self::reusable_mut`],
    /// which is why this is named for *looking* rather than for
    /// getting: a call site that reads geometry out of this method and
    /// draws with it is skipping the freshness check, and the name
    /// should say so where it is written. Cost: one hash lookup.
    pub fn inspect(&self, key: &EdgeKey) -> Option<&CachedConnection> {
        self.entries.get(key)
    }

    /// The read-only reuse door: the entry for `key`, but only if its
    /// samples were taken under `want`.
    ///
    /// This is what the scene builder's cache-hit fast path holds. A
    /// `None` here means "resample", never "no such edge" — the two
    /// are indistinguishable to the caller on purpose, because the
    /// correct response to both is the same and a caller that could
    /// tell them apart could act on the difference.
    ///
    /// Takes `&mut self` because handing an entry out **is** the
    /// liveness bookkeeping: the entry is stamped with the current
    /// pass generation, which is what keeps
    /// [`Self::evict_unseen`] from dropping it. A shared-borrow door
    /// plus a separate "mark it seen" call would be one more thing
    /// every future call site has to remember, which is the shape
    /// this module has already been bitten by once.
    ///
    /// Cost: one hash lookup plus [`SampleParams::matches`].
    pub fn reusable(&mut self, key: &EdgeKey, want: &SampleParams) -> Option<&CachedConnection> {
        let generation = self.generation;
        let entry = self.entries.get_mut(key)?;
        if !entry.sample_params.matches(want) {
            return None;
        }
        entry.last_seen = generation;
        Some(&*entry)
    }

    /// The mutating reuse door: as [`Self::reusable`], but handing out
    /// the `&mut` that [`CachedConnection::translate`] needs.
    ///
    /// This is what the scene builder's translate path holds: the same
    /// borrow answers "is this edge cached and sampled the way this
    /// frame wants?", performs the translate, and is reborrowed as
    /// shared to emit the element — one hash lookup for the whole
    /// path, and no re-lookup whose success has to be asserted after
    /// the fact.
    ///
    /// Leaving `by_node` alone is sound rather than a shortcut: that
    /// index maps a node id to the edges that touch it, and the node
    /// ids live in the [`EdgeKey`], not in the entry this borrow
    /// reaches. Changing *which* nodes an edge connects is therefore
    /// changing its key, which is [`Self::invalidate_edge`] followed
    /// by a fresh [`Self::refill`] — the two paths that do re-index.
    ///
    /// Stamps the entry with the current pass generation, as
    /// [`Self::reusable`] does and for the same reason.
    ///
    /// Cost: one hash lookup plus [`SampleParams::matches`].
    pub fn reusable_mut(&mut self, key: &EdgeKey, want: &SampleParams) -> Option<&mut CachedConnection> {
        let generation = self.generation;
        let entry = self.entries.get_mut(key)?;
        if !entry.sample_params.matches(want) {
            return None;
        }
        entry.last_seen = generation;
        Some(entry)
    }

    /// Re-sample the entry for `key` in place: hand `fill` the buffer
    /// this edge filled last time — emptied, capacity kept — then
    /// stamp the result with `params`, the new base endpoints and the
    /// current pass generation.
    ///
    /// Returns the stored entry, so the emitting pass can read the
    /// geometry it just wrote without keeping a second copy to hand to
    /// the emitter. Returns `None` — and evicts — when `fill` leaves
    /// the buffer empty, which is an edge that samples to nothing.
    ///
    /// **The whole write is one operation on purpose.** The earlier
    /// shape was a `reclaim_sample_buffer` that evicted the entry to
    /// hand its vector back, followed by an `insert` that put a new one
    /// in; the eviction made the "already cached" branch of `insert`
    /// unreachable from the pass, so the allocation the reuse existed
    /// to avoid was made on every resample after all. (Both of those
    /// are gone — named here as the history this shape exists to
    /// close, not as functions a reader will find.) Here the entry
    /// and both its `by_node` memberships stay put, and **an edge that
    /// resamples while already cached allocates nothing at all**: no
    /// `EdgeKey` clone, no `String` for a bucket that exists, and no
    /// sample vector for a buffer with the room.
    ///
    /// Cost: four hash lookups on an edge that is already cached —
    /// the presence test, the buffer borrow, the emptiness check and
    /// the stamp — plus whatever `fill` does. On the first sample of
    /// an edge, additionally the insert itself, one `EdgeKey` clone,
    /// and up to two `String`s for buckets that do not exist yet.
    pub fn refill(
        &mut self,
        key: &EdgeKey,
        params: SampleParams,
        base_from: Vec2,
        base_to: Vec2,
        fill: impl FnOnce(&mut Vec<Vec2>),
    ) -> Option<&CachedConnection> {
        let generation = self.generation;
        if !self.entries.contains_key(key) {
            // The reverse index maps a node id to the edges touching
            // it, and `key` names both of this edge's endpoints, so the
            // only question per bucket is whether this key is in it
            // already.
            for node in [&key.from_id, &key.to_id] {
                let bucket = Self::bucket_mut(&mut self.by_node, node);
                if !bucket.iter().any(|k| k == key) {
                    bucket.push(key.clone());
                }
            }
            self.entries.insert(
                key.clone(),
                CachedConnection {
                    pre_clip_positions: Vec::new(),
                    sample_params: params,
                    base_from,
                    base_to,
                    // Placeholder. The single stamp is at the tail of
                    // this function, which every entry that survives
                    // reaches; writing `generation` here as well was a
                    // second write nothing could observe, so removing
                    // it is what makes "one stamp site per route" true
                    // of the code and not only of the sentence.
                    last_seen: 0,
                },
            );
        }

        {
            let entry = self
                .entries
                .get_mut(key)
                .expect("created immediately above if absent");
            entry.pre_clip_positions.clear();
            fill(&mut entry.pre_clip_positions);
        }

        // An edge that samples to nothing leaves no entry behind, so
        // the next build retries it rather than serving an entry with
        // no points — which the reuse doors would happily hand out,
        // since its params still match.
        if self.entries[key].pre_clip_positions.is_empty() {
            self.invalidate_edge(key);
            return None;
        }

        let entry = self.entries.get_mut(key).expect("present and non-empty");
        entry.sample_params = params;
        entry.base_from = base_from;
        entry.base_to = base_to;
        entry.last_seen = generation;
        Some(&*entry)
    }

    /// The bucket for `node`, created empty if absent.
    ///
    /// Written as contains-then-branch rather than `entry(..)` because
    /// `HashMap::entry` needs an owned key whether or not it ends up
    /// storing one, and it is called twice for every edge that reaches
    /// the cache for the first time. Cost: two hash lookups when the
    /// bucket exists — the presence test and the borrow — and on the
    /// first edge to touch a node, a third plus one `String`
    /// allocation.
    fn bucket_mut<'m>(by_node: &'m mut HashMap<String, Vec<EdgeKey>>, node: &str) -> &'m mut Vec<EdgeKey> {
        if by_node.contains_key(node) {
            return by_node.get_mut(node).expect("checked immediately above");
        }
        by_node.entry(node.to_string()).or_default()
    }

    /// Open a new projection pass, so the entries this one touches can
    /// be told from the entries it does not.
    ///
    /// Paired with [`Self::evict_unseen`] at the end of the same pass.
    /// Calling this without the eviction leaves stale entries in place
    /// until some later pass evicts them, which costs memory and never
    /// correctness — a stale entry cannot be handed out, because the
    /// only routes to one are keyed on an edge the model still has.
    ///
    /// Cost: one increment. Wrapping, because a `u64` bumped once per
    /// rendered frame does not run out inside any span this program
    /// will be running for.
    pub fn begin_pass(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    /// Drop every entry not touched since the last [`Self::begin_pass`]
    /// — the edges that were removed from the model between builds.
    ///
    /// Replaces a since-deleted `retain_keys(&HashSet<EdgeKey>)` that
    /// the pass filled by cloning one owned key per visible edge,
    /// every frame, to hand the cache a set it could derive itself.
    ///
    /// Cost: one pass over the entries, plus the bucket maintenance of
    /// whatever it drops. Allocation-free when nothing is evicted,
    /// which is every frame that did not delete an edge.
    pub fn evict_unseen(&mut self) {
        let generation = self.generation;
        if self.entries.values().all(|e| e.last_seen == generation) {
            return;
        }
        let stale: Vec<EdgeKey> = self
            .entries
            .iter()
            .filter(|(_, e)| e.last_seen != generation)
            .map(|(k, _)| k.clone())
            .collect();
        for key in stale {
            self.invalidate_edge(&key);
        }
    }

    /// Which edges touch the given node? Used by the drag drain to mark
    /// dirty edges.
    pub fn edges_touching(&self, node_id: &str) -> &[EdgeKey] {
        self.by_node.get(node_id).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Drop a single edge (key-direct invalidation). Keeps the reverse
    /// index in sync.
    pub fn invalidate_edge(&mut self, key: &EdgeKey) {
        if self.entries.remove(key).is_none() {
            return;
        }
        if let Some(bucket) = self.by_node.get_mut(&key.from_id) {
            bucket.retain(|k| k != key);
        }
        if let Some(bucket) = self.by_node.get_mut(&key.to_id) {
            bucket.retain(|k| k != key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The params every fixture entry in this module is sampled
    /// under, so a test that wants a reuse *hit* asks with this and a
    /// test that wants a *miss* asks with something else.
    fn mk_params() -> SampleParams {
        SampleParams {
            font_size_pt: 12.0,
            spacing: 0.0,
            sample_budget: 1000,
        }
    }

    /// Plant an entry for `key` whose only distinguishing feature is
    /// its geometry — there is no styling field left to tell two
    /// entries apart by, which is the point of the type.
    ///
    /// Goes through [`SceneConnectionCache::refill`] because that is
    /// the cache's one writer; a fixture that reached past it would be
    /// exercising a shape production cannot produce.
    fn plant(cache: &mut SceneConnectionCache, key: &EdgeKey, mark: f32) {
        plant_points(cache, key, &[Vec2::new(mark, 2.0), Vec2::new(3.0, 4.0)]);
    }

    /// [`plant`] with the geometry spelled out.
    fn plant_points(cache: &mut SceneConnectionCache, key: &EdgeKey, points: &[Vec2]) {
        cache.refill(key, mk_params(), Vec2::ZERO, Vec2::ZERO, |out| {
            out.extend_from_slice(points);
        });
    }

    #[test]
    fn refill_and_inspect_round_trip() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        plant(&mut cache, &key, 1.0);
        assert_eq!(cache.inspect(&key).unwrap().pre_clip_positions[0].x, 1.0);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn edges_touching_indexes_both_endpoints() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        plant(&mut cache, &key, 1.0);
        assert_eq!(cache.edges_touching("a"), std::slice::from_ref(&key));
        assert_eq!(cache.edges_touching("b"), std::slice::from_ref(&key));
        assert!(cache.edges_touching("c").is_empty());
    }

    #[test]
    fn edges_touching_handles_multiple_edges_per_node() {
        let mut cache = SceneConnectionCache::new();
        let k1 = EdgeKey::new("hub", "a", "cross_link");
        let k2 = EdgeKey::new("hub", "b", "cross_link");
        let k3 = EdgeKey::new("c", "hub", "parent_child");
        plant(&mut cache, &k1, 2.0);
        plant(&mut cache, &k2, 3.0);
        plant(&mut cache, &k3, 4.0);

        let touching: std::collections::HashSet<&EdgeKey> = cache.edges_touching("hub").iter().collect();
        assert_eq!(touching.len(), 3);
        assert!(touching.contains(&k1));
        assert!(touching.contains(&k2));
        assert!(touching.contains(&k3));
    }

    #[test]
    fn invalidate_edge_removes_from_entries_and_index() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        plant(&mut cache, &key, 1.0);
        cache.invalidate_edge(&key);
        assert!(cache.inspect(&key).is_none());
        assert!(cache.edges_touching("a").is_empty());
        assert!(cache.edges_touching("b").is_empty());
    }

    #[test]
    fn clear_empties_everything() {
        let mut cache = SceneConnectionCache::new();
        plant(&mut cache, &EdgeKey::new("a", "b", "cross_link"), 1.0);
        plant(&mut cache, &EdgeKey::new("b", "c", "cross_link"), 5.0);
        cache.clear();
        assert!(cache.is_empty());
        assert!(cache.edges_touching("a").is_empty());
        assert!(cache.edges_touching("b").is_empty());
    }

    #[test]
    fn evict_unseen_drops_what_this_pass_did_not_touch() {
        let mut cache = SceneConnectionCache::new();
        let kept = EdgeKey::new("a", "b", "cross_link");
        let evicted = EdgeKey::new("c", "d", "cross_link");
        cache.begin_pass();
        plant(&mut cache, &kept, 2.0);
        plant(&mut cache, &evicted, 3.0);

        // A second pass touches only one of them.
        cache.begin_pass();
        assert!(
            cache.reusable(&kept, &mk_params()).is_some(),
            "precondition: reuse is what marks an entry seen"
        );
        cache.evict_unseen();

        assert!(cache.inspect(&kept).is_some());
        assert!(cache.inspect(&evicted).is_none());
        assert!(cache.edges_touching("c").is_empty());
        assert!(cache.edges_touching("d").is_empty());
    }

    #[test]
    fn reinsert_same_key_does_not_duplicate_index_entries() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        plant(&mut cache, &key, 2.0);
        plant(&mut cache, &key, 3.0);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.edges_touching("a").len(), 1);
        assert_eq!(cache.edges_touching("b").len(), 1);
        assert_eq!(
            cache.inspect(&key).unwrap().pre_clip_positions[0].x,
            3.0,
            "the later insert is the one that survives"
        );
    }

    #[test]
    fn ensure_zoom_preserves_cache_on_matching_zoom() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        plant(&mut cache, &key, 1.0);
        cache.ensure_zoom(1.0);
        // Same zoom again — nothing should change.
        cache.ensure_zoom(1.0);
        assert!(cache.inspect(&key).is_some());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn ensure_zoom_invalidates_on_zoom_change() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        plant(&mut cache, &key, 1.0);
        cache.ensure_zoom(1.0);
        // Wheel-tick to 1.1 — entries must be dropped.
        cache.ensure_zoom(1.1);
        assert!(cache.inspect(&key).is_none());
        assert!(cache.edges_touching("a").is_empty());
    }

    #[test]
    fn ensure_zoom_tolerates_sub_epsilon_drift() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        plant(&mut cache, &key, 1.0);
        cache.ensure_zoom(1.0);
        // Tiny floating-point drift well below ZOOM_EPSILON (1e-3).
        cache.ensure_zoom(1.0 + 1.0e-6);
        assert!(
            cache.inspect(&key).is_some(),
            "sub-epsilon drift should not flush"
        );
    }

    #[test]
    fn ensure_zoom_after_clear_just_stamps() {
        let mut cache = SceneConnectionCache::new();
        // Empty cache + ensure_zoom should just stamp, not panic or
        // touch anything.
        cache.ensure_zoom(0.5);
        plant(&mut cache, &EdgeKey::new("a", "b", "cross_link"), 2.0);
        // Same zoom — preserved.
        cache.ensure_zoom(0.5);
        assert_eq!(cache.len(), 1);
    }

    /// `EdgeKey::new` accepts mixed `&str` and `String` payloads via
    /// `Into<String>`. Pins the constructor surface — callers
    /// shouldn't have to know which side wants what.
    #[test]
    fn edge_key_new_accepts_mixed_str_and_string_inputs() {
        let from = String::from("a");
        let k1 = EdgeKey::new(&from, "b", "cross_link");
        let k2 = EdgeKey::new("a", String::from("b"), "cross_link");
        let k3 = EdgeKey::new(from.clone(), "b".to_string(), "cross_link");
        assert_eq!(k1, k2);
        assert_eq!(k1, k3);
        assert_eq!(k1.from_id, "a");
        assert_eq!(k1.to_id, "b");
        assert_eq!(k1.edge_type, "cross_link");
    }

    /// `EdgeKey::from_edge` produces the same key as the
    /// `EdgeKey::new(from, to, type)` triple constructed from the
    /// edge's identity fields. Pins the round-trip — a regression
    /// where `from_edge` looked at `to_id` first or used a different
    /// edge_type would show up here.
    #[test]
    fn edge_key_from_edge_matches_explicit_new() {
        let edge = crate::mindmap::test_helpers::synthetic_edge("alpha", "beta", "auto", "auto");
        let k_from = EdgeKey::from_edge(&edge);
        // `synthetic_edge` defaults edge_type to `cross_link`.
        let k_new = EdgeKey::new("alpha", "beta", "cross_link");
        assert_eq!(k_from, k_new);
    }

    /// Edge-deletion eviction: when a caller drops an edge from
    /// the model, `invalidate_edge` must remove BOTH the cache
    /// entry AND the reverse-index buckets for both endpoints.
    /// A regression that forgot to clear `by_node` would leak
    /// stale `EdgeKey`s in `edges_touching` results, which the
    /// drag-drain consumes — leading to "ghost edges" that
    /// repaint with no model backing.
    #[test]
    fn invalidate_edge_removes_from_both_endpoint_buckets_on_deletion() {
        let mut cache = SceneConnectionCache::new();
        let kept = EdgeKey::new("hub", "a", "cross_link");
        let evicted = EdgeKey::new("hub", "b", "cross_link");
        plant(&mut cache, &kept, 2.0);
        plant(&mut cache, &evicted, 3.0);

        cache.invalidate_edge(&evicted);

        // Hub side: only the kept edge survives in the bucket.
        let hub_bucket: std::collections::HashSet<&EdgeKey> = cache.edges_touching("hub").iter().collect();
        assert_eq!(hub_bucket.len(), 1);
        assert!(hub_bucket.contains(&kept));
        // Other endpoint's bucket is now empty.
        assert!(cache.edges_touching("b").is_empty());
        // Forward lookup is gone too.
        assert!(cache.inspect(&evicted).is_none());
        // Sibling edge is untouched.
        assert!(cache.inspect(&kept).is_some());
    }

    /// Cache-miss semantics: [`SceneConnectionCache::inspect`] on an
    /// unknown key returns `None` cleanly without mutating cache
    /// state. The drag-tick
    /// hot path asks the cache "do you know this edge?" hundreds
    /// of times per frame; a miss must be cheap and side-effect
    /// free.
    #[test]
    fn inspect_on_missing_key_returns_none_without_side_effects() {
        let mut cache = SceneConnectionCache::new();
        let known = EdgeKey::new("a", "b", "cross_link");
        plant(&mut cache, &known, 1.0);
        let len_before = cache.len();

        let missing = EdgeKey::new("x", "y", "cross_link");
        assert!(cache.inspect(&missing).is_none());

        // No phantom insertion on miss; reverse index untouched.
        assert_eq!(cache.len(), len_before);
        assert!(cache.edges_touching("x").is_empty());
        assert!(cache.edges_touching("y").is_empty());
        assert!(cache.inspect(&known).is_some());
    }

    /// The other half of the miss-semantics test above: `inspect`
    /// answers "what is stored", and deliberately not "what may be
    /// reused" — so a present entry and a *reusable* one are two
    /// different questions.
    #[test]
    fn inspect_returns_an_entry_the_reuse_doors_would_refuse() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        plant(&mut cache, &key, 1.0);

        let mut other = mk_params();
        other.font_size_pt += 10.0;

        assert!(
            cache.inspect(&key).is_some(),
            "inspect answers regardless of what the entry was sampled under"
        );
        assert!(
            cache.reusable(&key, &other).is_none(),
            "the reuse door must refuse the same entry"
        );
    }

    /// The whole of item 7's guard in one assertion per field: an
    /// entry sampled under one set of params is not reusable under
    /// any other.
    ///
    /// Each case names the edit that produces it — a zoom step or an
    /// `edge font size` verb for the first, an `edge spacing` verb
    /// for the second, adding or deleting an edge for the third.
    #[test]
    fn reusable_refuses_an_entry_sampled_under_different_params() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        plant(&mut cache, &key, 1.0);
        let same = mk_params();
        assert!(
            cache.reusable(&key, &same).is_some(),
            "precondition: matching params must be reusable, or every case below is vacuous"
        );

        for (label, mutate) in [
            (
                "font_size_pt",
                (|p: &mut SampleParams| p.font_size_pt += 1.0) as fn(&mut SampleParams),
            ),
            ("spacing", |p: &mut SampleParams| p.spacing += 1.0),
            ("sample_budget", |p: &mut SampleParams| p.sample_budget += 1),
        ] {
            let mut want = mk_params();
            mutate(&mut want);
            assert!(
                cache.reusable(&key, &want).is_none(),
                "a changed `{label}` must make the entry non-reusable"
            );
            assert!(
                cache.reusable_mut(&key, &want).is_none(),
                "the mutating door must refuse a changed `{label}` too"
            );
        }
    }

    /// Sub-tolerance drift is not a config change. `ensure_zoom`
    /// deliberately keeps the cache across a zoom wobble below
    /// `ZOOM_EPSILON`; a params check that used `==` would then
    /// resample every one of those frames anyway, undoing the
    /// tolerance one level up.
    #[test]
    fn reusable_tolerates_sub_epsilon_params_drift() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        plant(&mut cache, &key, 1.0);

        let mut wobbled = mk_params();
        wobbled.font_size_pt += 1.0e-7;
        wobbled.spacing += 1.0e-7;
        assert!(
            cache.reusable(&key, &wobbled).is_some(),
            "drift far below the almost-equal tolerance must not force a resample"
        );
    }

    /// `translate` moves the ends of the sample array — which is
    /// where the emitting pass reads the caps from — and leaves the
    /// sampling params alone, because a rigid shift changes where
    /// samples are, not what they were taken under.
    #[test]
    fn translate_carries_the_sample_ends_and_leaves_the_params_alone() {
        let mut entry = CachedConnection {
            pre_clip_positions: vec![Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0)],
            sample_params: mk_params(),
            base_from: Vec2::new(0.0, 0.0),
            base_to: Vec2::new(10.0, 0.0),
            last_seen: 0,
        };
        let before = entry.sample_params;

        entry.translate(Vec2::new(3.0, -4.0), Vec2::new(3.0, -4.0), Vec2::new(13.0, -4.0));

        assert_eq!(
            (
                entry.pre_clip_positions.first().copied(),
                entry.pre_clip_positions.last().copied()
            ),
            (Some(Vec2::new(3.0, -4.0)), Some(Vec2::new(13.0, -4.0))),
            "both ends — and so both caps — must ride the translation"
        );
        assert_eq!(entry.base_from, Vec2::new(3.0, -4.0));
        assert_eq!(entry.base_to, Vec2::new(13.0, -4.0));
        assert!(
            before.matches(&entry.sample_params),
            "a translation must not disturb the params the entry is keyed on for reuse"
        );
    }

    /// `sample_spacing` is the arc-length step the sampler walks and
    /// the value the cache compares, derived once. Computed here
    /// from the documented ratio rather than by calling
    /// `monospace_advance` again, so a change to that ratio has to
    /// be a decision rather than a silent agreement.
    #[test]
    fn sample_spacing_is_the_glyph_advance_plus_the_configured_gap() {
        let params = SampleParams {
            font_size_pt: 10.0,
            spacing: 2.5,
            sample_budget: 100,
        };
        // MONOSPACE_ADVANCE_RATIO is 0.6, so 10pt advances 6.0.
        assert!(
            almost_equal(params.sample_spacing(), 8.5),
            "expected 6.0 + 2.5, got {}",
            params.sample_spacing()
        );
    }

    /// `refill` hands the closure the buffer that was already there,
    /// emptied — the two halves of "refill this instead of allocating
    /// a new one".
    ///
    /// Input that makes the first half fail: dropping the `clear()`,
    /// which would leave last frame's points in front of this frame's
    /// and draw the edge twice. Input for the second: any
    /// implementation that hands over a fresh `Vec`, which would make
    /// the reuse a no-op.
    #[test]
    fn refill_hands_over_the_old_allocation_emptied() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        let wide: Vec<Vec2> = (0..64).map(|i| Vec2::new(i as f32, 0.0)).collect();
        plant_points(&mut cache, &key, &wide);
        let held = cache
            .inspect(&key)
            .expect("planted")
            .pre_clip_positions
            .capacity();
        assert!(held >= 64, "precondition: the fixture entry holds a real buffer");

        let mut seen_len = usize::MAX;
        let mut seen_capacity = 0usize;
        cache.refill(&key, mk_params(), Vec2::ZERO, Vec2::ZERO, |out| {
            seen_len = out.len();
            seen_capacity = out.capacity();
            out.push(Vec2::ZERO);
        });
        assert_eq!(seen_len, 0, "a refillable buffer must arrive empty");
        assert_eq!(
            seen_capacity, held,
            "the point is to hand over the allocation, not to make a new one"
        );
    }

    /// A refill that fills nothing evicts, including from the reverse
    /// index, so an edge that samples to nothing does not leave a key
    /// pointing at an entry that is no longer there — or, worse, an
    /// entry with no points that the reuse doors would happily serve.
    #[test]
    fn refill_that_fills_nothing_evicts_the_entry_and_its_index() {
        let mut cache = SceneConnectionCache::new();
        let kept = EdgeKey::new("hub", "a", "cross_link");
        let taken = EdgeKey::new("hub", "b", "cross_link");
        plant(&mut cache, &kept, 2.0);
        plant(&mut cache, &taken, 3.0);

        assert!(
            cache
                .refill(&taken, mk_params(), Vec2::ZERO, Vec2::ZERO, |_| {})
                .is_none(),
            "a refill that appends nothing reports nothing"
        );

        assert!(cache.inspect(&taken).is_none());
        assert!(cache.edges_touching("b").is_empty());
        assert_eq!(cache.edges_touching("hub"), std::slice::from_ref(&kept));
        assert!(cache.inspect(&kept).is_some(), "the sibling edge is untouched");
    }

    /// Refilling an edge that was never cached creates it — the
    /// connection pass's first frame takes this branch for every edge.
    /// The closure gets an empty buffer, and the entry lands indexed.
    #[test]
    fn refill_of_an_unknown_key_creates_and_indexes_the_entry() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("x", "y", "cross_link");
        let mut arrived_empty = false;
        let stored = cache
            .refill(&key, mk_params(), Vec2::ZERO, Vec2::ZERO, |out| {
                arrived_empty = out.is_empty();
                out.push(Vec2::new(4.0, 5.0));
            })
            .expect("a filled refill reports its entry");
        assert_eq!(stored.pre_clip_positions, vec![Vec2::new(4.0, 5.0)]);
        assert!(arrived_empty, "a first fill starts from nothing");
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.edges_touching("x"), std::slice::from_ref(&key));
        assert_eq!(cache.edges_touching("y"), std::slice::from_ref(&key));
    }

    /// [`SceneConnectionCache::refill`] marks the entry seen, so an
    /// edge sampled fresh in a pass survives that same pass's
    /// eviction.
    ///
    /// Input that makes it fail: `refill` not stamping. Every edge on
    /// the slow path would then be written and immediately dropped,
    /// and the cache would never hold anything for longer than the
    /// build that filled it.
    #[test]
    fn refill_marks_the_entry_seen_in_the_current_pass() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        cache.begin_pass();
        plant(&mut cache, &key, 1.0);
        cache.evict_unseen();
        assert!(
            cache.inspect(&key).is_some(),
            "an entry written this pass has been seen this pass"
        );
    }

    /// A door that refuses does not mark. The entry is about to be
    /// rewritten by the slow path either way, so the stamp would be
    /// redundant — and marking on refusal would keep an entry alive
    /// through a pass that ended up not using it.
    #[test]
    fn a_refused_reuse_does_not_mark_the_entry_seen() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        cache.begin_pass();
        plant(&mut cache, &key, 1.0);

        cache.begin_pass();
        let mut other = mk_params();
        other.font_size_pt += 10.0;
        assert!(cache.reusable(&key, &other).is_none());
        assert!(cache.reusable_mut(&key, &other).is_none());
        cache.evict_unseen();
        assert!(
            cache.inspect(&key).is_none(),
            "a pass that asked and was refused did not use this entry"
        );
    }

    /// `evict_unseen` with no pass ever opened drops nothing: a fresh
    /// entry and a fresh cache are both at generation zero. The pass
    /// always calls `begin_pass` first, so this is a shape a consumer
    /// could reach rather than one the app does — it must not silently
    /// empty the cache.
    #[test]
    fn evict_unseen_before_any_pass_drops_nothing() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        plant(&mut cache, &key, 1.0);
        cache.evict_unseen();
        assert_eq!(cache.len(), 1);
    }

    /// `refill` hands back the entry it just wrote, which is what lets
    /// the slow path filter the samples out of the cache rather than
    /// out of a second copy it kept.
    #[test]
    fn refill_returns_a_borrow_of_the_entry_it_stored() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");

        let stored = cache
            .refill(&key, mk_params(), Vec2::ZERO, Vec2::ZERO, |out| {
                out.push(Vec2::new(7.0, 8.0));
            })
            .expect("a refill that filled something reports the entry");
        assert_eq!(stored.pre_clip_positions, vec![Vec2::new(7.0, 8.0)]);
        // And it really is the stored one, not a temporary.
        assert_eq!(
            cache.inspect(&key).unwrap().pre_clip_positions,
            vec![Vec2::new(7.0, 8.0)]
        );
    }

    /// The claim `refill` exists to make true: an edge that resamples
    /// while already cached does not allocate.
    ///
    /// The observable is the buffer's identity across the two calls —
    /// its capacity, from a first fill wide enough that a second,
    /// narrower one cannot need to grow. A `refill` that evicted and
    /// re-created (which is what the earlier reclaim-then-insert pair
    /// did, making this branch unreachable from the pass) would hand
    /// the second fill a fresh `Vec` and land on its exact reserve.
    #[test]
    fn refill_of_a_cached_edge_reuses_the_buffer_it_already_had() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        let wide: Vec<Vec2> = (0..96).map(|i| Vec2::new(i as f32, 0.0)).collect();
        plant_points(&mut cache, &key, &wide);
        let held = cache
            .inspect(&key)
            .expect("planted")
            .pre_clip_positions
            .capacity();

        cache.refill(&key, mk_params(), Vec2::ZERO, Vec2::ZERO, |out| {
            out.reserve(8);
            out.extend((0..8).map(|i| Vec2::new(i as f32, 1.0)));
        });

        let entry = cache.inspect(&key).expect("still cached");
        assert_eq!(
            entry.pre_clip_positions.len(),
            8,
            "precondition: the refill is narrower"
        );
        assert_eq!(
            entry.pre_clip_positions.capacity(),
            held,
            "a cached edge's resample must reuse the buffer it already had"
        );
    }

    /// `snapshot` reads the *effective* font size, so the
    /// screen-space clamp is inside the value the cache compares
    /// rather than outside it. At zoom 4 a 12pt authored size wants
    /// 48pt on screen; a 20pt ceiling pulls it back to 20 on screen,
    /// which is 5pt in canvas space.
    #[test]
    fn snapshot_records_the_clamped_effective_font_size() {
        let config = GlyphConnectionConfig {
            font_size_pt: 12.0,
            min_font_size_pt: 8.0,
            max_font_size_pt: 20.0,
            spacing: 1.5,
            ..GlyphConnectionConfig::default()
        };
        let params = SampleParams::snapshot(&config, 4.0, 77);
        assert!(
            almost_equal(params.font_size_pt, 5.0),
            "expected the 20pt screen ceiling divided back through zoom 4, got {}",
            params.font_size_pt
        );
        assert!(almost_equal(params.spacing, 1.5));
        assert_eq!(params.sample_budget, 77);
    }
}
