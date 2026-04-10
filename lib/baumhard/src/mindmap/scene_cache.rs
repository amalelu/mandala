//! Per-edge cache of connection glyph geometry.
//!
//! Phase B of the "Connection & border render cost" work: during a drag the
//! scene builder would re-sample every visible edge every frame, even though
//! most edges had not moved. For a 20,000-unit cross-link at typical spacing
//! that is ~1,667 Bezier point evaluations + a 256-entry arc-length table
//! rebuild per frame per such edge, which is more than enough to blow the
//! drag budget and stutter the interaction.
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
//! - Samples are stored in canvas space and are independent of the camera.
//!   Camera pan/zoom does not invalidate the cache; only the *downstream*
//!   renderer-side glyph buffers need to be rebuilt when the viewport
//!   changes.
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
    pub fn new(from_id: impl Into<String>, to_id: impl Into<String>, edge_type: impl Into<String>) -> Self {
        Self {
            from_id: from_id.into(),
            to_id: to_id.into(),
            edge_type: edge_type.into(),
        }
    }

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
#[derive(Clone, Debug)]
pub struct CachedConnection {
    pub pre_clip_positions: Vec<Vec2>,
    pub cap_start: Option<(String, Vec2)>,
    pub cap_end: Option<(String, Vec2)>,
    pub body_glyph: String,
    pub font: Option<String>,
    pub font_size_pt: f32,
    pub color: String,
}

/// Per-edge cache, plus a reverse index from node ID → edges that touch it
/// so a drag of node N dirties the right edges in O(k_N) instead of walking
/// the whole edge list.
#[derive(Default, Debug)]
pub struct SceneConnectionCache {
    entries: HashMap<EdgeKey, CachedConnection>,
    by_node: HashMap<String, Vec<EdgeKey>>,
}

impl SceneConnectionCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop everything. Used at drag-drop, undo, reparent, edge CRUD, fold
    /// toggle, theme-variable change — the cheap "when in doubt, flush"
    /// path. The next scene build re-populates the cache.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.by_node.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

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
}
