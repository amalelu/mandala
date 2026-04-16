//! Tests for [`RegionIndexer`] — the spatial bucket index (§T1).
//!
//! Covers initialization, insert/remove/query, reverse index,
//! boundary conditions, clone independence, and scale testing.
//! Follows the `do_*()` / `test_*()` benchmark-reuse split (§T2.2).

use crate::gfx_structs::util::regions::RegionIndexer;

// ── initialize ────────────────────────────────────────────────────

#[test]
fn test_region_indexer_initialize() {
    do_region_indexer_initialize();
}

pub fn do_region_indexer_initialize() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(500);
    assert_eq!(indexer.index_as_ref().len(), 500);
}

#[test]
fn test_region_indexer_initialize_with() {
    do_region_indexer_initialize_with();
}

/// `initialize_with(x, y)` allocates exactly `x * y` region buckets.
pub fn do_region_indexer_initialize_with() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize_with(10, 20);
    assert_eq!(indexer.index_as_ref().len(), 200);
}

#[test]
fn test_region_indexer_initialize_zero() {
    do_region_indexer_initialize_zero();
}

/// Initializing with zero regions produces an empty index.
pub fn do_region_indexer_initialize_zero() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(0);
    assert_eq!(indexer.index_as_ref().len(), 0);
}

#[test]
fn test_region_indexer_reinitialize_clears_forward_index() {
    do_region_indexer_reinitialize_clears_forward_index();
}

/// Calling `initialize` a second time drops all previously indexed
/// elements and resizes the bucket vector.
pub fn do_region_indexer_reinitialize_clears_forward_index() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(10);
    indexer.insert(42, 0);
    indexer.insert(43, 9);
    assert_eq!(indexer.elements_in_region(0).len(), 1);

    indexer.initialize(5);
    assert_eq!(indexer.index_as_ref().len(), 5);
    assert_eq!(indexer.elements_in_region(0).len(), 0);
}

// ── insert / remove / query ───────────────────────────────────────

#[test]
fn test_region_indexer_insert_and_remove() {
    do_region_indexer_insert_and_remove();
}

pub fn do_region_indexer_insert_and_remove() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(10);

    indexer.insert(100, 3);
    assert!(indexer.elements_in_region(3).contains(&100));
    assert_eq!(indexer.elements_in_region(3).len(), 1);

    indexer.remove(100, 3);
    assert!(!indexer.elements_in_region(3).contains(&100));
    assert_eq!(indexer.elements_in_region(3).len(), 0);
}

#[test]
fn test_region_indexer_multiple_elements_in_one_region() {
    do_region_indexer_multiple_elements_in_one_region();
}

pub fn do_region_indexer_multiple_elements_in_one_region() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(5);

    indexer.insert(10, 2);
    indexer.insert(20, 2);
    indexer.insert(30, 2);
    assert_eq!(indexer.elements_in_region(2).len(), 3);

    indexer.remove(20, 2);
    assert_eq!(indexer.elements_in_region(2).len(), 2);
    assert!(!indexer.elements_in_region(2).contains(&20));
    assert!(indexer.elements_in_region(2).contains(&10));
    assert!(indexer.elements_in_region(2).contains(&30));
}

#[test]
fn test_region_indexer_one_element_in_multiple_regions() {
    do_region_indexer_one_element_in_multiple_regions();
}

pub fn do_region_indexer_one_element_in_multiple_regions() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(10);

    indexer.insert(7, 0);
    indexer.insert(7, 3);
    indexer.insert(7, 9);

    assert!(indexer.elements_in_region(0).contains(&7));
    assert!(indexer.elements_in_region(3).contains(&7));
    assert!(indexer.elements_in_region(9).contains(&7));
    assert!(!indexer.elements_in_region(1).contains(&7));

    let regions = indexer.get_reverse_index_for_element(7);
    assert_eq!(regions.len(), 3);

    indexer.remove(7, 3);
    assert!(!indexer.elements_in_region(3).contains(&7));
    assert!(indexer.elements_in_region(0).contains(&7));
    let regions = indexer.get_reverse_index_for_element(7);
    assert_eq!(regions.len(), 2);
}

#[test]
fn test_region_indexer_empty_region_query() {
    do_region_indexer_empty_region_query();
}

pub fn do_region_indexer_empty_region_query() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(5);
    assert!(indexer.elements_in_region(0).is_empty());
    assert!(indexer.elements_in_region(4).is_empty());
}

#[test]
fn test_region_indexer_boundary_regions() {
    do_region_indexer_boundary_regions();
}

pub fn do_region_indexer_boundary_regions() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(100);
    indexer.insert(1, 0);
    indexer.insert(2, 99);
    assert!(indexer.elements_in_region(0).contains(&1));
    assert!(indexer.elements_in_region(99).contains(&2));
}

#[test]
fn test_region_indexer_duplicate_insert_is_idempotent() {
    do_region_indexer_duplicate_insert_is_idempotent();
}

pub fn do_region_indexer_duplicate_insert_is_idempotent() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(5);
    indexer.insert(42, 2);
    indexer.insert(42, 2);
    assert_eq!(indexer.elements_in_region(2).len(), 1);
}

#[test]
fn test_region_indexer_remove_nonexistent_is_noop() {
    do_region_indexer_remove_nonexistent_is_noop();
}

pub fn do_region_indexer_remove_nonexistent_is_noop() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(5);
    indexer.remove(999, 2);
    assert!(indexer.elements_in_region(2).is_empty());
}

// ── reverse index ─────────────────────────────────────────────────

#[test]
fn test_region_indexer_no_reverse_index() {
    do_region_indexer_no_reverse_index();
}

pub fn do_region_indexer_no_reverse_index() {
    let mut indexer = RegionIndexer::new_without_reverse_index();
    indexer.initialize(10);
    indexer.insert(5, 3);
    indexer.insert(5, 7);
    assert!(indexer.elements_in_region(3).contains(&5));
    assert!(indexer.reverse_index_as_ref().is_empty());
    indexer.remove(5, 3);
    assert!(!indexer.elements_in_region(3).contains(&5));
    assert!(indexer.elements_in_region(7).contains(&5));
}

#[test]
fn test_region_indexer_reverse_index_unknown_element() {
    do_region_indexer_reverse_index_unknown_element();
}

pub fn do_region_indexer_reverse_index_unknown_element() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(5);
    assert!(indexer.get_reverse_index_for_element(9999).is_empty());
}

#[test]
fn test_region_indexer_reinitialize_stale_reverse_index() {
    do_region_indexer_reinitialize_stale_reverse_index();
}

/// After reinitialize, the reverse index retains stale entries.
/// Known limitation — callers must be aware.
pub fn do_region_indexer_reinitialize_stale_reverse_index() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(10);
    indexer.insert(5, 3);
    indexer.insert(5, 7);
    indexer.initialize(10);
    assert!(indexer.elements_in_region(3).is_empty());
    let stale = indexer.get_reverse_index_for_element(5);
    assert!(stale.contains(&3) && stale.contains(&7));
}

// ── edge cases ────────────────────────────────────────────────────

#[test]
fn test_region_indexer_default() {
    do_region_indexer_default();
}

pub fn do_region_indexer_default() {
    let indexer = RegionIndexer::default();
    assert_eq!(indexer.index_as_ref().len(), 0);
}

#[test]
fn test_region_indexer_element_id_zero() {
    do_region_indexer_element_id_zero();
}

pub fn do_region_indexer_element_id_zero() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(3);
    indexer.insert(0, 0);
    indexer.insert(0, 2);
    assert!(indexer.elements_in_region(0).contains(&0));
    assert_eq!(indexer.get_reverse_index_for_element(0).len(), 2);
    indexer.remove(0, 0);
    assert!(!indexer.elements_in_region(0).contains(&0));
    assert!(indexer.elements_in_region(2).contains(&0));
}

#[test]
fn test_region_indexer_element_id_max() {
    do_region_indexer_element_id_max();
}

pub fn do_region_indexer_element_id_max() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(2);
    indexer.insert(usize::MAX, 0);
    indexer.insert(usize::MAX, 1);
    assert!(indexer.elements_in_region(0).contains(&usize::MAX));
    indexer.remove(usize::MAX, 0);
    assert!(!indexer.elements_in_region(0).contains(&usize::MAX));
    assert!(indexer.elements_in_region(1).contains(&usize::MAX));
}

#[test]
#[should_panic]
fn test_region_indexer_insert_oob_panics() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(5);
    indexer.insert(1, 5);
}

#[test]
#[should_panic]
fn test_region_indexer_query_oob_panics() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(5);
    let _ = indexer.elements_in_region(5);
}

#[test]
fn test_region_indexer_clone_is_independent() {
    do_region_indexer_clone_is_independent();
}

pub fn do_region_indexer_clone_is_independent() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(5);
    indexer.insert(10, 2);
    let mut cloned = indexer.clone();
    cloned.insert(20, 2);
    cloned.remove(10, 2);
    assert!(indexer.elements_in_region(2).contains(&10));
    assert!(!indexer.elements_in_region(2).contains(&20));
}

#[test]
fn test_region_indexer_initialize_with_zero_axis() {
    do_region_indexer_initialize_with_zero_axis();
}

pub fn do_region_indexer_initialize_with_zero_axis() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize_with(0, 100);
    assert_eq!(indexer.index_as_ref().len(), 0);
    indexer.initialize_with(100, 0);
    assert_eq!(indexer.index_as_ref().len(), 0);
}

#[test]
fn test_region_indexer_remove_wrong_region_no_damage() {
    do_region_indexer_remove_wrong_region_no_damage();
}

pub fn do_region_indexer_remove_wrong_region_no_damage() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(5);
    indexer.insert(7, 1);
    indexer.insert(7, 3);
    indexer.remove(7, 2);
    assert!(indexer.elements_in_region(1).contains(&7));
    assert!(indexer.elements_in_region(3).contains(&7));
    assert_eq!(indexer.get_reverse_index_for_element(7).len(), 2);
}

#[test]
fn test_region_indexer_single_region() {
    do_region_indexer_single_region();
}

pub fn do_region_indexer_single_region() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(1);
    indexer.insert(1, 0);
    indexer.insert(2, 0);
    indexer.insert(3, 0);
    assert_eq!(indexer.elements_in_region(0).len(), 3);
    indexer.remove(2, 0);
    assert_eq!(indexer.elements_in_region(0).len(), 2);
}

// ── scale ─────────────────────────────────────────────────────────

#[test]
fn test_region_indexer_insert_at_scale() {
    do_region_indexer_insert_at_scale();
}

pub fn do_region_indexer_insert_at_scale() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(100);
    for i in 0..1000_usize {
        indexer.insert(i, i % 100);
    }
    for r in 0..100 {
        assert_eq!(indexer.elements_in_region(r).len(), 10);
    }
    assert!(indexer.elements_in_region(42).contains(&42));
    assert!(indexer.elements_in_region(42).contains(&942));
    assert_eq!(indexer.get_reverse_index_for_element(42).len(), 1);
}
