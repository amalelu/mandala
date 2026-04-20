//! Spatial bucket index for efficient element lookup by screen region.
//!
//! A `RegionIndexer` partitions a pixel grid into a flat vector of
//! `BTreeSet<usize>` buckets. Each element id is inserted into one or
//! more region buckets; queries then check a single bucket to find
//! "which elements occupy this part of the screen" — O(1) per bucket
//! lookup instead of O(elements). The reverse element→regions map is
//! optional (enabled by default, disabled via
//! `RegionIndexer::new_without_reverse_index` when no caller needs
//! it) and trades memory for fast "which regions does element X sit
//! in?" queries.
//!
//! `RegionIndexer` owns only the index structure — it does not know
//! about pixel dimensions, grid factors, or prime-avoidance. Those
//! concerns live in the companion `regions::RegionParams`, which
//! computes region IDs and passes them here.

use rustc_hash::FxHashMap;
use std::collections::BTreeSet;

/// Forward region→elements (and optional reverse element→regions)
/// index. Owned per-tree and per-scene; does not cross thread
/// boundaries. See the module header for the partitioning scheme.
#[derive(Debug, Clone)]
pub struct RegionIndexer {
    /// Forward index: position `r` holds the element ids sitting in
    /// region `r`. `BTreeSet` keeps iteration order deterministic and
    /// supports O(log n) insert/remove.
    index: Vec<BTreeSet<usize>>,
    /// Reverse index: `element_id → regions it occupies`. Only
    /// populated when `use_reverse_index` is true.
    reverse_index: FxHashMap<usize, BTreeSet<usize>>,
    /// Whether to maintain the reverse index on `insert` / `remove`.
    use_reverse_index: bool,
}

impl RegionIndexer {
    /// Construct a fresh indexer with the reverse element→regions index
    /// enabled. Cheap — no region slots allocated until
    /// [`RegionIndexer::initialize`] is called.
    pub fn new() -> Self {
        RegionIndexer {
            index: Vec::new(),
            reverse_index: Default::default(),
            use_reverse_index: true,
        }
    }

    /// Construct a fresh indexer without the reverse element→regions
    /// index. Use when the consumer never calls
    /// [`RegionIndexer::get_reverse_index_for_element`] — saves memory
    /// proportional to the number of indexed elements.
    pub fn new_without_reverse_index() -> Self {
        RegionIndexer {
            index: Vec::new(),
            reverse_index: Default::default(),
            use_reverse_index: false,
        }
    }

    /// Allocate `x * y` region buckets in one call. Convenience for the
    /// common 2-D grid case; equivalent to
    /// [`RegionIndexer::initialize`]`(x * y)`.
    pub fn initialize_with(&mut self, x: usize, y: usize) {
        self.initialize(x * y)
    }

    /// Allocate `num_regions` empty region buckets. Drops any previously
    /// allocated buckets — O(num_regions) and clears all indexed
    /// elements.
    pub fn initialize(&mut self, num_regions: usize) {
        if self.index.len() > 0 {
            self.index = Vec::new();
        }
        for _ in 0..num_regions {
            self.index.push(BTreeSet::new());
        }
    }

    /// Record that `element_id` currently occupies `region`. Also
    /// updates the reverse (element→regions) map when that map is
    /// enabled. O(log n) into the `BTreeSet`; one `BTreeSet`
    /// allocation on first insert of a given element when the
    /// reverse index is live.
    pub fn insert(&mut self, element_id: usize, region: usize) {
        self.index[region].insert(element_id);
        if self.use_reverse_index {
            if !self.reverse_index.contains_key(&element_id) {
                self.reverse_index.insert(element_id, BTreeSet::new());
            }
            self.reverse_index
                .get_mut(&element_id)
                .unwrap()
                .insert(region);
        }
    }

    /// Undo a prior [`insert`](Self::insert) — drop the element from
    /// `region` in both the forward and (when enabled) reverse index.
    /// O(log n); missing entries are silent no-ops.
    pub fn remove(&mut self, element_id: usize, region: usize) {
        self.index[region].remove(&element_id);
        if self.use_reverse_index {
            if self.reverse_index.contains_key(&element_id) {
                self.reverse_index
                    .get_mut(&element_id)
                    .unwrap()
                    .remove(&region);
            }
        }
    }

    /// Borrow the set of element ids currently sitting in `region`.
    /// Zero-copy by design: callers that want an owned set can call
    /// `.clone()`, and the common "count" / "contains" queries stay
    /// allocation-free. O(1). Panics if `region` is out of bounds —
    /// caller is responsible for validating against
    /// [`RegionParams::calc_num_regions`](crate::gfx_structs::util::regions::RegionParams::calc_num_regions).
    pub fn elements_in_region(&self, region: usize) -> &BTreeSet<usize> {
        &self.index[region]
    }

    /// Borrow the per-region index slot vector. Index `r` is the set of
    /// element ids currently sitting in region `r`. O(1).
    pub fn index_as_ref(&self) -> &Vec<BTreeSet<usize>> {
        &self.index
    }

    /// Borrow the reverse element→regions map. Empty when the indexer
    /// was constructed with [`RegionIndexer::new_without_reverse_index`].
    /// O(1).
    pub fn reverse_index_as_ref(&self) -> &FxHashMap<usize, BTreeSet<usize>> {
        &self.reverse_index
    }

    /// Clone the set of regions that `element_id` currently occupies,
    /// or an empty set when the element has no entries (or the
    /// reverse index was disabled at construction). O(n) in the
    /// clone size; the indexer itself is not mutated despite the
    /// `&mut self` signature (kept for API compatibility with older
    /// callers).
    pub fn get_reverse_index_for_element(&mut self, element_id: usize) -> BTreeSet<usize> {
        self.reverse_index
            .get(&element_id)
            .cloned()
            .unwrap_or_default()
    }
}

impl Default for RegionIndexer {
    fn default() -> Self {
        Self::new()
    }
}
