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
//! - Structural edge changes (add/remove, endpoint change, control points,
//!   glyph config) are handled by the caller clearing the relevant entries
//!   (`invalidate_edge`) or dropping the whole cache (`clear`). Selection
//!   changes do NOT require invalidation — the color override is applied
//!   at scene-build time from the cached entry.

use glam::Vec2;
use std::collections::HashMap;

use crate::mindmap::model::MindEdge;

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

/// The cached geometry + styling for a single edge, sufficient to rebuild
/// its `ConnectionElement` without recomputing the path.
///
/// `pre_clip_positions` / `cap_*` are the raw sampled points BEFORE the
/// `point_inside_any_node` clip filter runs. We keep them pre-clip so a
/// moved-but-unrelated node's AABB can still push glyphs out of the
/// connection on the next frame: the clip filter is cheap (arithmetic over
/// cached `Vec2`s), the sampler is not.
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
    pub cap_start: Option<(String, Vec2)>,
    pub cap_end: Option<(String, Vec2)>,
    pub body_glyph: String,
    pub font: Option<String>,
    pub font_size_pt: f32,
    pub color: String,
    pub base_from: Vec2,
    pub base_to: Vec2,
}

/// Per-edge cache of sampled connection geometry, plus a reverse
/// index from node ID → edges that touch it so a drag of node N
/// dirties the right edges in `O(k_N)` instead of walking the whole
/// edge list. Owned by the app's document / renderer glue and passed
/// into [`crate::mindmap::scene_builder::build_scene_with_cache`] on
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
    /// `build_scene_with_cache` on entry so the invariant is enforced
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

    /// Look up a cached entry. Scene-builder reads go through this.
    pub fn get(&self, key: &EdgeKey) -> Option<&CachedConnection> {
        self.entries.get(key)
    }

    /// Insert or replace an entry, keeping the `by_node` reverse index in
    /// sync. Scene-builder writes (both "fresh sample" and "resample because
    /// endpoint moved") go through this.
    pub fn insert(&mut self, key: EdgeKey, entry: CachedConnection) {
        // Remove the key from any stale `by_node` bucket first — we can't
        // know the old endpoints without looking at the previous entry, so
        // the simplest correct thing is to strip the key from both new
        // endpoints' buckets and re-add it. In practice insertions come
        // paired with the current endpoints in the live map, so the
        // `from_id` / `to_id` on `key` are already the up-to-date ones.
        self.by_node
            .entry(key.from_id.clone())
            .or_default()
            .retain(|k| k != &key);
        self.by_node
            .entry(key.to_id.clone())
            .or_default()
            .retain(|k| k != &key);

        self.entries.insert(key.clone(), entry);

        self.by_node
            .entry(key.from_id.clone())
            .or_default()
            .push(key.clone());
        self.by_node
            .entry(key.to_id.clone())
            .or_default()
            .push(key);
    }

    /// Which edges touch the given node? Used by the drag drain to mark
    /// dirty edges.
    pub fn edges_touching(&self, node_id: &str) -> &[EdgeKey] {
        self.by_node
            .get(node_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
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

    /// Rigid-body translate of a cached entry's geometry in place.
    /// Shifts `pre_clip_positions` and both caps by `delta`, stamps
    /// `base_from` / `base_to` to the new reference endpoints.
    /// Returns a borrow of the mutated entry so the scene builder's
    /// translate path can emit the `ConnectionElement` without a
    /// follow-up `get`.
    ///
    /// Why a dedicated method instead of `insert`: this runs on
    /// every internal edge of a subtree drag every drain. Routing
    /// through `insert` would reindex both `by_node` buckets (two
    /// `retain` scans + two `push` calls per edge) and clone
    /// `body_glyph` / `font` / `color` — none of which change under
    /// a pure translation. On a 500-edge drag that's ~1000 bucket
    /// scans and ~1500 string clones per drain the translate path
    /// is specifically trying to avoid.
    ///
    /// Returns `None` if the key isn't cached. Callers should fall
    /// back to the slow path in that case.
    pub fn translate_in_place(
        &mut self,
        key: &EdgeKey,
        delta: Vec2,
        new_base_from: Vec2,
        new_base_to: Vec2,
    ) -> Option<&CachedConnection> {
        let entry = self.entries.get_mut(key)?;
        for p in &mut entry.pre_clip_positions {
            *p += delta;
        }
        if let Some((_, p)) = entry.cap_start.as_mut() {
            *p += delta;
        }
        if let Some((_, p)) = entry.cap_end.as_mut() {
            *p += delta;
        }
        entry.base_from = new_base_from;
        entry.base_to = new_base_to;
        Some(&*entry)
    }

    /// After a scene build, evict any cache entries whose keys are not in
    /// the "seen this frame" set. Handles edges that were deleted from the
    /// model between builds.
    pub fn retain_keys(&mut self, seen: &std::collections::HashSet<EdgeKey>) {
        let to_evict: Vec<EdgeKey> = self
            .entries
            .keys()
            .filter(|k| !seen.contains(*k))
            .cloned()
            .collect();
        for key in to_evict {
            self.invalidate_edge(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_entry(color: &str) -> CachedConnection {
        CachedConnection {
            pre_clip_positions: vec![Vec2::new(1.0, 2.0), Vec2::new(3.0, 4.0)],
            cap_start: None,
            cap_end: None,
            body_glyph: "·".into(),
            font: None,
            font_size_pt: 12.0,
            color: color.into(),
            base_from: Vec2::ZERO,
            base_to: Vec2::ZERO,
        }
    }

    #[test]
    fn insert_and_get_round_trips() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        cache.insert(key.clone(), mk_entry("#fff"));
        assert_eq!(cache.get(&key).unwrap().color, "#fff");
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn edges_touching_indexes_both_endpoints() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        cache.insert(key.clone(), mk_entry("#fff"));
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
        cache.insert(k1.clone(), mk_entry("#111"));
        cache.insert(k2.clone(), mk_entry("#222"));
        cache.insert(k3.clone(), mk_entry("#333"));

        let touching: std::collections::HashSet<&EdgeKey> =
            cache.edges_touching("hub").iter().collect();
        assert_eq!(touching.len(), 3);
        assert!(touching.contains(&k1));
        assert!(touching.contains(&k2));
        assert!(touching.contains(&k3));
    }

    #[test]
    fn invalidate_edge_removes_from_entries_and_index() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        cache.insert(key.clone(), mk_entry("#fff"));
        cache.invalidate_edge(&key);
        assert!(cache.get(&key).is_none());
        assert!(cache.edges_touching("a").is_empty());
        assert!(cache.edges_touching("b").is_empty());
    }

    #[test]
    fn clear_empties_everything() {
        let mut cache = SceneConnectionCache::new();
        cache.insert(EdgeKey::new("a", "b", "cross_link"), mk_entry("#fff"));
        cache.insert(EdgeKey::new("b", "c", "cross_link"), mk_entry("#000"));
        cache.clear();
        assert!(cache.is_empty());
        assert!(cache.edges_touching("a").is_empty());
        assert!(cache.edges_touching("b").is_empty());
    }

    #[test]
    fn retain_keys_evicts_unseen() {
        use std::collections::HashSet;
        let mut cache = SceneConnectionCache::new();
        let kept = EdgeKey::new("a", "b", "cross_link");
        let evicted = EdgeKey::new("c", "d", "cross_link");
        cache.insert(kept.clone(), mk_entry("#111"));
        cache.insert(evicted.clone(), mk_entry("#222"));

        let mut seen = HashSet::new();
        seen.insert(kept.clone());
        cache.retain_keys(&seen);

        assert!(cache.get(&kept).is_some());
        assert!(cache.get(&evicted).is_none());
        assert!(cache.edges_touching("c").is_empty());
        assert!(cache.edges_touching("d").is_empty());
    }

    #[test]
    fn reinsert_same_key_does_not_duplicate_index_entries() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        cache.insert(key.clone(), mk_entry("#111"));
        cache.insert(key.clone(), mk_entry("#222"));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.edges_touching("a").len(), 1);
        assert_eq!(cache.edges_touching("b").len(), 1);
        assert_eq!(cache.get(&key).unwrap().color, "#222");
    }

    #[test]
    fn ensure_zoom_preserves_cache_on_matching_zoom() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        cache.insert(key.clone(), mk_entry("#fff"));
        cache.ensure_zoom(1.0);
        // Same zoom again — nothing should change.
        cache.ensure_zoom(1.0);
        assert!(cache.get(&key).is_some());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn ensure_zoom_invalidates_on_zoom_change() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        cache.insert(key.clone(), mk_entry("#fff"));
        cache.ensure_zoom(1.0);
        // Wheel-tick to 1.1 — entries must be dropped.
        cache.ensure_zoom(1.1);
        assert!(cache.get(&key).is_none());
        assert!(cache.edges_touching("a").is_empty());
    }

    #[test]
    fn ensure_zoom_tolerates_sub_epsilon_drift() {
        let mut cache = SceneConnectionCache::new();
        let key = EdgeKey::new("a", "b", "cross_link");
        cache.insert(key.clone(), mk_entry("#fff"));
        cache.ensure_zoom(1.0);
        // Tiny floating-point drift well below ZOOM_EPSILON (1e-3).
        cache.ensure_zoom(1.0 + 1.0e-6);
        assert!(cache.get(&key).is_some(), "sub-epsilon drift should not flush");
    }

    #[test]
    fn ensure_zoom_after_clear_just_stamps() {
        let mut cache = SceneConnectionCache::new();
        // Empty cache + ensure_zoom should just stamp, not panic or
        // touch anything.
        cache.ensure_zoom(0.5);
        cache.insert(EdgeKey::new("a", "b", "cross_link"), mk_entry("#111"));
        // Same zoom — preserved.
        cache.ensure_zoom(0.5);
        assert_eq!(cache.len(), 1);
    }
}
