use crate::gfx_structs::util::regions::{RegionError, RegionIndexer, RegionParams};

// =====================================================================
// RegionIndexer — bucket index
// =====================================================================

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
fn test_region_indexer_reinitialize_clears_all() {
    do_region_indexer_reinitialize_clears_all();
}

/// Calling `initialize` a second time drops all previously indexed
/// elements and resizes the bucket vector.
pub fn do_region_indexer_reinitialize_clears_all() {
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

/// Insert element into region, verify presence, remove, verify absence.
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

/// Multiple elements can occupy the same region bucket.
pub fn do_region_indexer_multiple_elements_in_one_region() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(5);

    indexer.insert(10, 2);
    indexer.insert(20, 2);
    indexer.insert(30, 2);
    assert_eq!(indexer.elements_in_region(2).len(), 3);
    assert!(indexer.elements_in_region(2).contains(&10));
    assert!(indexer.elements_in_region(2).contains(&20));
    assert!(indexer.elements_in_region(2).contains(&30));

    // Removing one does not affect the others.
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

/// A single element can span multiple region buckets (e.g. a large
/// rectangle crossing grid lines).
pub fn do_region_indexer_one_element_in_multiple_regions() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(10);

    indexer.insert(7, 0);
    indexer.insert(7, 3);
    indexer.insert(7, 9);

    assert!(indexer.elements_in_region(0).contains(&7));
    assert!(indexer.elements_in_region(3).contains(&7));
    assert!(indexer.elements_in_region(9).contains(&7));
    // Regions it was NOT inserted into remain empty.
    assert!(!indexer.elements_in_region(1).contains(&7));

    // Reverse index should list all three regions.
    let regions = indexer.get_reverse_index_for_element(7);
    assert_eq!(regions.len(), 3);
    assert!(regions.contains(&0));
    assert!(regions.contains(&3));
    assert!(regions.contains(&9));

    // Remove from one region only — the other two survive.
    indexer.remove(7, 3);
    assert!(!indexer.elements_in_region(3).contains(&7));
    assert!(indexer.elements_in_region(0).contains(&7));
    assert!(indexer.elements_in_region(9).contains(&7));

    let regions = indexer.get_reverse_index_for_element(7);
    assert_eq!(regions.len(), 2);
}

#[test]
fn test_region_indexer_empty_region_query() {
    do_region_indexer_empty_region_query();
}

/// Querying a region that has never been inserted into returns an
/// empty set.
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

/// Insert into first and last region (boundary buckets) works
/// correctly.
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

/// Inserting the same (element, region) pair twice does not create a
/// duplicate — `BTreeSet` deduplicates.
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

/// Removing an element that was never inserted is a silent no-op.
pub fn do_region_indexer_remove_nonexistent_is_noop() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(5);

    // No panic, no error.
    indexer.remove(999, 2);
    assert!(indexer.elements_in_region(2).is_empty());
}

// ── reverse index disabled ────────────────────────────────────────

#[test]
fn test_region_indexer_no_reverse_index() {
    do_region_indexer_no_reverse_index();
}

/// When constructed with `new_without_reverse_index`, the forward
/// index works normally but the reverse index stays empty.
pub fn do_region_indexer_no_reverse_index() {
    let mut indexer = RegionIndexer::new_without_reverse_index();
    indexer.initialize(10);

    indexer.insert(5, 3);
    indexer.insert(5, 7);
    assert!(indexer.elements_in_region(3).contains(&5));
    assert!(indexer.elements_in_region(7).contains(&5));

    // Reverse index is empty — the feature is disabled.
    assert!(indexer.reverse_index_as_ref().is_empty());
    let reverse = indexer.get_reverse_index_for_element(5);
    assert!(reverse.is_empty());

    // Remove still works on the forward index.
    indexer.remove(5, 3);
    assert!(!indexer.elements_in_region(3).contains(&5));
    assert!(indexer.elements_in_region(7).contains(&5));
}

// ── reverse index for unknown element ─────────────────────────────

#[test]
fn test_region_indexer_reverse_index_unknown_element() {
    do_region_indexer_reverse_index_unknown_element();
}

/// Querying the reverse index for an element that was never inserted
/// returns an empty set (not a panic).
pub fn do_region_indexer_reverse_index_unknown_element() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(5);
    let result = indexer.get_reverse_index_for_element(9999);
    assert!(result.is_empty());
}

// ── default trait ─────────────────────────────────────────────────

#[test]
fn test_region_indexer_default() {
    do_region_indexer_default();
}

/// `Default::default()` creates the same state as `new()`.
pub fn do_region_indexer_default() {
    let indexer = RegionIndexer::default();
    assert_eq!(indexer.index_as_ref().len(), 0);
}

// ── insert at scale ───────────────────────────────────────────────

#[test]
fn test_region_indexer_insert_at_scale() {
    do_region_indexer_insert_at_scale();
}

/// Inserting 1000 elements spread across 100 regions and verifying
/// counts per region. Elements `i` go into region `i % 100`, so each
/// region should hold exactly 10 elements.
pub fn do_region_indexer_insert_at_scale() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(100);

    for i in 0..1000_usize {
        indexer.insert(i, i % 100);
    }

    for r in 0..100 {
        assert_eq!(
            indexer.elements_in_region(r).len(),
            10,
            "region {} should have 10 elements",
            r
        );
    }

    // Spot-check: element 42 should be in region 42.
    assert!(indexer.elements_in_region(42).contains(&42));
    assert!(indexer.elements_in_region(42).contains(&142));
    assert!(indexer.elements_in_region(42).contains(&942));

    // Reverse check for element 42: should be in exactly region 42.
    let regions = indexer.get_reverse_index_for_element(42);
    assert_eq!(regions.len(), 1);
    assert!(regions.contains(&42));
}

// ── reinitialize leaves reverse index stale ───────────────────────
// NOTE: This test documents current behaviour. The reverse index is
// NOT cleared by `initialize`, so after reinitializing the forward
// index is empty but the reverse index still has stale entries from
// before. Callers that reinitialize must be aware of this.

#[test]
fn test_region_indexer_reinitialize_stale_reverse_index() {
    do_region_indexer_reinitialize_stale_reverse_index();
}

/// After reinitialize, the forward index is empty but the reverse
/// index retains stale entries from the prior generation.
pub fn do_region_indexer_reinitialize_stale_reverse_index() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(10);
    indexer.insert(5, 3);
    indexer.insert(5, 7);

    // Forward index is populated.
    assert!(indexer.elements_in_region(3).contains(&5));

    // Reinitialize — forward index is wiped.
    indexer.initialize(10);
    assert!(indexer.elements_in_region(3).is_empty());

    // Reverse index is STALE — it still reports the old regions.
    // This is the current behaviour; document it so we catch any
    // future change (intentional or not).
    let stale = indexer.get_reverse_index_for_element(5);
    assert!(
        stale.contains(&3) && stale.contains(&7),
        "reverse index should retain stale entries after reinitialize \
         (this is a known limitation, not a bug — callers should be aware)"
    );
}

// ── element id = 0 ────────────────────────────────────────────────

#[test]
fn test_region_indexer_element_id_zero() {
    do_region_indexer_element_id_zero();
}

/// Element id 0 is a valid usize and must be indexable.
pub fn do_region_indexer_element_id_zero() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(3);

    indexer.insert(0, 0);
    indexer.insert(0, 2);
    assert!(indexer.elements_in_region(0).contains(&0));
    assert!(indexer.elements_in_region(2).contains(&0));

    let regions = indexer.get_reverse_index_for_element(0);
    assert_eq!(regions.len(), 2);

    indexer.remove(0, 0);
    assert!(!indexer.elements_in_region(0).contains(&0));
    assert!(indexer.elements_in_region(2).contains(&0));
}

// ── element id = usize::MAX ───────────────────────────────────────

#[test]
fn test_region_indexer_element_id_max() {
    do_region_indexer_element_id_max();
}

/// Extreme element id (usize::MAX) must be indexable without overflow.
pub fn do_region_indexer_element_id_max() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(2);

    indexer.insert(usize::MAX, 0);
    indexer.insert(usize::MAX, 1);
    assert!(indexer.elements_in_region(0).contains(&usize::MAX));
    assert!(indexer.elements_in_region(1).contains(&usize::MAX));

    indexer.remove(usize::MAX, 0);
    assert!(!indexer.elements_in_region(0).contains(&usize::MAX));
    assert!(indexer.elements_in_region(1).contains(&usize::MAX));
}

// ── out-of-bounds region panics ───────────────────────────────────

#[test]
#[should_panic]
fn test_region_indexer_insert_oob_panics() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(5);
    indexer.insert(1, 5); // region 5 doesn't exist (0..4)
}

#[test]
#[should_panic]
fn test_region_indexer_query_oob_panics() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(5);
    let _ = indexer.elements_in_region(5);
}

// ── clone independence ────────────────────────────────────────────

#[test]
fn test_region_indexer_clone_is_independent() {
    do_region_indexer_clone_is_independent();
}

/// Cloning an indexer produces a snapshot; mutations to the clone
/// do not affect the original.
pub fn do_region_indexer_clone_is_independent() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(5);
    indexer.insert(10, 2);

    let mut cloned = indexer.clone();
    cloned.insert(20, 2);
    cloned.remove(10, 2);

    // Original is unchanged.
    assert!(indexer.elements_in_region(2).contains(&10));
    assert!(!indexer.elements_in_region(2).contains(&20));

    // Clone has its own state.
    assert!(!cloned.elements_in_region(2).contains(&10));
    assert!(cloned.elements_in_region(2).contains(&20));
}

// ── initialize_with one axis zero ─────────────────────────────────

#[test]
fn test_region_indexer_initialize_with_zero_axis() {
    do_region_indexer_initialize_with_zero_axis();
}

/// `initialize_with(0, N)` and `initialize_with(N, 0)` both produce
/// zero regions (0 * N = 0).
pub fn do_region_indexer_initialize_with_zero_axis() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize_with(0, 100);
    assert_eq!(indexer.index_as_ref().len(), 0);

    indexer.initialize_with(100, 0);
    assert_eq!(indexer.index_as_ref().len(), 0);
}

// ── remove element from wrong region ──────────────────────────────

#[test]
fn test_region_indexer_remove_wrong_region_no_damage() {
    do_region_indexer_remove_wrong_region_no_damage();
}

/// Removing an element from a region it was never inserted into does
/// not corrupt the reverse index or other regions.
pub fn do_region_indexer_remove_wrong_region_no_damage() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(5);

    indexer.insert(7, 1);
    indexer.insert(7, 3);

    // Remove from region 2 where element 7 was never inserted.
    indexer.remove(7, 2);

    // Existing entries are untouched.
    assert!(indexer.elements_in_region(1).contains(&7));
    assert!(indexer.elements_in_region(3).contains(&7));

    let regions = indexer.get_reverse_index_for_element(7);
    assert_eq!(regions.len(), 2);
    assert!(regions.contains(&1));
    assert!(regions.contains(&3));
}

// ── single-region indexer ─────────────────────────────────────────

#[test]
fn test_region_indexer_single_region() {
    do_region_indexer_single_region();
}

/// An indexer with exactly one region puts everything in bucket 0.
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

// =====================================================================
// RegionParams — grid parameter management
// =====================================================================

#[test]
fn test_region_params_new_sunny_day() {
    do_region_params_new_sunny_day()
}

pub fn do_region_params_new_sunny_day() {
    let params = RegionParams::new(10, (1000, 1000));
    assert_eq!(params.read_region_size_y(), Ok(100));
    assert_eq!(params.read_region_size_x(), Ok(100));
    assert_eq!(params.read_current_resolution(), Ok((1000, 1000)));
    assert_eq!(params.read_region_factor_x(), Ok(10));
    assert_eq!(params.read_region_factor_y(), Ok(10));
    assert_eq!(params.read_target_region_factor(), Ok(10));
}

#[test]
fn test_regions_params_new_rainy_day() {
    do_regions_params_new_rainy_day();
}

pub fn do_regions_params_new_rainy_day() {
    let mut params = RegionParams::new(7, (1000, 1000));
    assert_eq!(params.read_region_size_y(), Ok(125));
    assert_eq!(params.read_region_size_x(), Ok(125));
    assert_eq!(params.read_current_resolution(), Ok((1000, 1000)));
    assert_eq!(params.read_region_factor_x(), Ok(8));
    assert_eq!(params.read_region_factor_y(), Ok(8));
    assert_eq!(params.read_target_region_factor(), Ok(7));

    params.adapt(6, (1000, 1000));
    assert_eq!(params.read_current_resolution(), Ok((1000, 1000)));
    assert_eq!(params.read_region_factor_x(), Ok(5));
    assert_eq!(params.read_region_factor_y(), Ok(5));
    assert_eq!(params.read_region_size_y(), Ok(200));
    assert_eq!(params.read_region_size_x(), Ok(200));
    assert_eq!(params.read_target_region_factor(), Ok(6));

    params.adapt(13, (1000, 1000));
    assert_eq!(params.read_current_resolution(), Ok((1000, 1000)));
    assert_eq!(params.read_region_factor_x(), Ok(10));
    assert_eq!(params.read_region_factor_y(), Ok(10));
    assert_eq!(params.read_region_size_y(), Ok(100));
    assert_eq!(params.read_region_size_x(), Ok(100));
    assert_eq!(params.read_target_region_factor(), Ok(13));
}

#[test]
fn test_region_params_calculate_region_from_pixel() {
    do_region_params_calculate_region_from_pixel()
}

pub fn do_region_params_calculate_region_from_pixel() {
    let params = RegionParams::new(10, (1000, 1000));
    assert_eq!(params.calculate_region_from_pixel((0, 0)), Ok(0));
    assert_eq!(params.calculate_region_from_pixel((1, 1)), Ok(0));
    assert_eq!(params.calculate_region_from_pixel((10, 10)), Ok(0));
    assert_eq!(params.calculate_region_from_pixel((1, 99)), Ok(0));
    assert_eq!(params.calculate_region_from_pixel((1, 100)), Ok(10));
    assert_eq!(params.calculate_region_from_pixel((99, 99)), Ok(0));
    assert_eq!(params.calculate_region_from_pixel((100, 99)), Ok(1));
    assert_eq!(params.calculate_region_from_pixel((200, 99)), Ok(2));
    assert_eq!(params.calculate_region_from_pixel((300, 99)), Ok(3));
    assert_eq!(params.calculate_region_from_pixel((300, 100)), Ok(13));
    assert_eq!(params.calculate_region_from_pixel((300, 200)), Ok(23));
    assert_eq!(
        params.calculate_region_from_pixel((1200, 200)),
        Err(RegionError::InvalidParameters("Pixel is out of bounds"))
    );
    assert_eq!(
        params.calculate_region_from_pixel((1, 1000)),
        Err(RegionError::InvalidParameters("Pixel is out of bounds"))
    );
    assert_eq!(params.calculate_region_from_pixel((999, 999)), Ok(99));
}

#[test]
fn test_region_params_calculate_regions_intersected_by_rectangle() {
    do_region_params_calculate_regions_intersected_by_rectangle()
}

pub fn do_region_params_calculate_regions_intersected_by_rectangle() {
   let params = RegionParams::new(10, (1000, 1000));
   assert_eq!(params.calculate_regions_intersected_by_rectangle((0,0),(399, 399)),
              Ok(vec![0, 1, 2, 3, 10, 11, 12, 13, 20, 21, 22, 23, 30, 31, 32, 33]));

   assert_eq!(params.calculate_regions_intersected_by_rectangle((0,0),(399, 400)),
              Ok(vec![0, 1, 2, 3, 10, 11, 12, 13, 20, 21, 22, 23, 30, 31, 32, 33, 40, 41, 42, 43]));

   assert_eq!(params.calculate_regions_intersected_by_rectangle((0,0),(400, 399)),
              Ok(vec![0, 1, 2, 3, 4, 10, 11, 12, 13, 14, 20, 21, 22, 23, 24, 30, 31, 32, 33, 34]));

   assert_eq!(params.calculate_regions_intersected_by_rectangle((0,0),(400, 400)),
              Ok(vec![0, 1, 2, 3, 4, 10, 11, 12, 13, 14, 20, 21, 22, 23, 24, 30, 31, 32, 33, 34, 40, 41, 42, 43, 44]));

   assert_eq!(params.calculate_regions_intersected_by_rectangle((0,0),(99, 99)), Ok(vec![0]));

   assert_eq!(params.calculate_regions_intersected_by_rectangle((99, 99),(99, 99)), Ok(vec![0]));

   assert_eq!(params.calculate_regions_intersected_by_rectangle((100, 99),(200, 99)), Ok(vec![1, 2]));

   assert_eq!(params.calculate_regions_intersected_by_rectangle((100, 100),(200, 100)), Ok(vec![11, 12]));

   assert_eq!(params.calculate_regions_intersected_by_rectangle((100, 100),(99, 99)),
              Err(RegionError::InvalidParameters("Start position is higher than end position")));

   assert_eq!(params.calculate_regions_intersected_by_rectangle((1000, 1000),(2000, 2000)),
              Err(RegionError::InvalidParameters("Start position is out of resolution bounds")));

   assert_eq!(params.calculate_regions_intersected_by_rectangle((999, 999),(2000, 2000)),
              Err(RegionError::InvalidParameters("End position is out of resolution bounds")));
}

#[test]
fn test_region_params_calculate_pixel_from_region() {
    do_region_params_calculate_pixel_from_region()
}

pub fn do_region_params_calculate_pixel_from_region() {
    let params = RegionParams::new(10, (1000, 1000));
    assert_eq!(params.calculate_pixel_from_region(0), Ok((0, 0)));
    assert_eq!(params.calculate_pixel_from_region(1), Ok((100, 0)));
    assert_eq!(params.calculate_pixel_from_region(9), Ok((900, 0)));
    assert_eq!(params.calculate_pixel_from_region(10), Ok((0, 100)));
    assert_eq!(params.calculate_pixel_from_region(11), Ok((100, 100)));
    assert_eq!(params.calculate_pixel_from_region(99), Ok((900, 900)));
    assert_eq!(
        params.calculate_pixel_from_region(100),
        Err(RegionError::InvalidParameters("Region is out of bounds"))
    );
    assert_eq!(
        params.calculate_pixel_from_region(9000),
        Err(RegionError::InvalidParameters("Region is out of bounds"))
    );
}

// ── pixel → region → pixel roundtrip ──────────────────────────────

#[test]
fn test_region_params_pixel_region_roundtrip() {
    do_region_params_pixel_region_roundtrip();
}

/// For every pixel at the top-left corner of each region cell, the
/// roundtrip pixel→region→pixel returns the same pixel.
pub fn do_region_params_pixel_region_roundtrip() {
    let params = RegionParams::new(10, (1000, 1000));
    for r in 0..100 {
        let pixel = params.calculate_pixel_from_region(r).unwrap();
        let back = params.calculate_region_from_pixel(pixel).unwrap();
        assert_eq!(back, r, "roundtrip failed for region {}", r);
    }
}

// ── asymmetric resolution ─────────────────────────────────────────

#[test]
fn test_region_params_asymmetric_resolution() {
    do_region_params_asymmetric_resolution();
}

/// When width != height, region factors and sizes may differ per axis.
pub fn do_region_params_asymmetric_resolution() {
    // 1920x1080 — target 10.  1920/10=192 exact; 1080 closest divisor
    // to 10 is 10 (1080/10=108). Both divide evenly.
    let params = RegionParams::new(10, (1920, 1080));
    assert_eq!(params.read_region_factor_x(), Ok(10));
    assert_eq!(params.read_region_factor_y(), Ok(10));
    assert_eq!(params.read_region_size_x(), Ok(192));
    assert_eq!(params.read_region_size_y(), Ok(108));

    // Pixel at (192, 108) should be in region 11 (second column,
    // second row).
    assert_eq!(params.calculate_region_from_pixel((192, 108)), Ok(11));
}

// ── adapt changes resolution ──────────────────────────────────────

#[test]
fn test_region_params_adapt_changes_resolution() {
    do_region_params_adapt_changes_resolution();
}

/// `adapt` with a new resolution recalculates factors and sizes.
pub fn do_region_params_adapt_changes_resolution() {
    let mut params = RegionParams::new(10, (1000, 1000));
    assert_eq!(params.calc_num_regions(), Ok(100));

    params.adapt(5, (500, 500));
    assert_eq!(params.read_current_resolution(), Ok((500, 500)));
    assert_eq!(params.read_region_factor_x(), Ok(5));
    assert_eq!(params.read_region_factor_y(), Ok(5));
    assert_eq!(params.read_region_size_x(), Ok(100));
    assert_eq!(params.read_region_size_y(), Ok(100));
    assert_eq!(params.calc_num_regions(), Ok(25));
}

// ── calculate_actual_region_factor edge cases ─────────────────────

#[test]
fn test_region_factor_zero_target() {
    do_region_factor_zero_target();
}

/// Target factor of 0 falls back to 1.
pub fn do_region_factor_zero_target() {
    assert_eq!(RegionParams::calculate_actual_region_factor(0, 1000), 1);
}

#[test]
fn test_region_factor_zero_dimension() {
    do_region_factor_zero_dimension();
}

/// Zero-pixel dimension falls back to factor 1.
pub fn do_region_factor_zero_dimension() {
    assert_eq!(RegionParams::calculate_actual_region_factor(10, 0), 1);
}

#[test]
fn test_region_factor_exact_divisor() {
    do_region_factor_exact_divisor();
}

/// When the target is an exact divisor of the dimension, it is
/// returned unchanged.
pub fn do_region_factor_exact_divisor() {
    assert_eq!(RegionParams::calculate_actual_region_factor(10, 1000), 10);
    assert_eq!(RegionParams::calculate_actual_region_factor(5, 1000), 5);
    assert_eq!(RegionParams::calculate_actual_region_factor(25, 1000), 25);
}

#[test]
fn test_region_factor_target_exceeds_dimension() {
    do_region_factor_target_exceeds_dimension();
}

/// When the target factor exceeds the dimension span, the factor is
/// clamped to the dimension (one region per pixel).
pub fn do_region_factor_target_exceeds_dimension() {
    assert_eq!(RegionParams::calculate_actual_region_factor(100, 10), 10);
    assert_eq!(RegionParams::calculate_actual_region_factor(500, 4), 4);
}

#[test]
fn test_region_factor_closest_divisor() {
    do_region_factor_closest_divisor();
}

/// Non-divisor target finds the closest divisor.
pub fn do_region_factor_closest_divisor() {
    // 1000 divisors near 7: 5 and 8. 8 is closer → 8.
    assert_eq!(RegionParams::calculate_actual_region_factor(7, 1000), 8);
    // 1000 divisors near 6: 5 and 8. 5 is closer → 5.
    assert_eq!(RegionParams::calculate_actual_region_factor(6, 1000), 5);
    // 1000 divisors near 3: 2 and 4. Equidistant → smaller wins → 2.
    assert_eq!(RegionParams::calculate_actual_region_factor(3, 1000), 2);
}

// ── calc_num_regions ──────────────────────────────────────────────

#[test]
fn test_region_params_calc_num_regions() {
    do_region_params_calc_num_regions();
}

/// `calc_num_regions` returns `factor_x * factor_y`.
pub fn do_region_params_calc_num_regions() {
    let params = RegionParams::new(10, (1000, 1000));
    assert_eq!(params.calc_num_regions(), Ok(100));

    let params2 = RegionParams::new(10, (1920, 1080));
    assert_eq!(params2.calc_num_regions(), Ok(100)); // 10*10
}

// ── prime panics ──────────────────────────────────────────────────

#[test]
#[should_panic]
fn test_region_params_prime_x() {
    RegionParams::new(10, (251, 1000));
}

#[test]
#[should_panic]
fn test_region_params_prime_y() {
    RegionParams::new(10, (1000, 251));
}

#[test]
#[should_panic]
fn test_region_params_prime_both() {
    let mut params = RegionParams::new(7, (1000, 1000));
    params.adapt(13, (241, 251));
}

#[test]
#[should_panic]
fn test_region_params_adapt_prime_x() {
    let mut params = RegionParams::new(7, (1000, 1000));
    params.adapt(10, (241, 1000));
}

#[test]
#[should_panic]
fn test_region_params_adapt_prime_y() {
    let mut params = RegionParams::new(7, (1000, 1000));
    params.adapt(10, (1000, 251));
}

// =====================================================================
// RegionParams — extreme edge cases
// =====================================================================

// ── smallest valid grids ──────────────────────────────────────────

#[test]
fn test_region_params_resolution_1x1() {
    do_region_params_resolution_1x1();
}

/// Resolution (1, 1) — the smallest non-prime non-zero resolution.
/// Factor must be 1 (only divisor of 1), so a single region covers
/// the single pixel.
pub fn do_region_params_resolution_1x1() {
    let params = RegionParams::new(10, (1, 1));
    assert_eq!(params.read_region_factor_x(), Ok(1));
    assert_eq!(params.read_region_factor_y(), Ok(1));
    assert_eq!(params.read_region_size_x(), Ok(1));
    assert_eq!(params.read_region_size_y(), Ok(1));
    assert_eq!(params.calc_num_regions(), Ok(1));
    assert_eq!(params.calculate_region_from_pixel((0, 0)), Ok(0));
}

#[test]
fn test_region_params_resolution_4x4() {
    do_region_params_resolution_4x4();
}

/// Resolution (4, 4) with target 2 → exact 2x2 grid, each cell is
/// 2x2 pixels. Verify every pixel maps to the correct region.
pub fn do_region_params_resolution_4x4() {
    let params = RegionParams::new(2, (4, 4));
    assert_eq!(params.read_region_factor_x(), Ok(2));
    assert_eq!(params.read_region_factor_y(), Ok(2));
    assert_eq!(params.read_region_size_x(), Ok(2));
    assert_eq!(params.read_region_size_y(), Ok(2));
    assert_eq!(params.calc_num_regions(), Ok(4));

    // Exhaustive pixel → region mapping for the entire 4x4 grid:
    //   (0,0)=0  (1,0)=0  (2,0)=1  (3,0)=1
    //   (0,1)=0  (1,1)=0  (2,1)=1  (3,1)=1
    //   (0,2)=2  (1,2)=2  (2,2)=3  (3,2)=3
    //   (0,3)=2  (1,3)=2  (2,3)=3  (3,3)=3
    let expected = [
        [0, 0, 1, 1],
        [0, 0, 1, 1],
        [2, 2, 3, 3],
        [2, 2, 3, 3],
    ];
    for y in 0..4_usize {
        for x in 0..4_usize {
            assert_eq!(
                params.calculate_region_from_pixel((x, y)),
                Ok(expected[y][x]),
                "pixel ({}, {}) should be region {}",
                x, y, expected[y][x],
            );
        }
    }
}

// ── factor = 1 (single region covers everything) ──────────────────

#[test]
fn test_region_params_factor_one() {
    do_region_params_factor_one();
}

/// Target factor 1 always produces a single region regardless of
/// resolution.
pub fn do_region_params_factor_one() {
    let params = RegionParams::new(1, (1920, 1080));
    assert_eq!(params.read_region_factor_x(), Ok(1));
    assert_eq!(params.read_region_factor_y(), Ok(1));
    assert_eq!(params.calc_num_regions(), Ok(1));

    // Every pixel maps to region 0.
    assert_eq!(params.calculate_region_from_pixel((0, 0)), Ok(0));
    assert_eq!(params.calculate_region_from_pixel((1919, 1079)), Ok(0));
    assert_eq!(params.calculate_region_from_pixel((960, 540)), Ok(0));
}

// ── factor = dimension (one region per pixel row/col) ─────────────

#[test]
fn test_region_params_factor_equals_dimension() {
    do_region_params_factor_equals_dimension();
}

/// When factor equals the dimension, each cell is 1 pixel wide/tall.
pub fn do_region_params_factor_equals_dimension() {
    let params = RegionParams::new(10, (10, 10));
    assert_eq!(params.read_region_factor_x(), Ok(10));
    assert_eq!(params.read_region_factor_y(), Ok(10));
    assert_eq!(params.read_region_size_x(), Ok(1));
    assert_eq!(params.read_region_size_y(), Ok(1));
    assert_eq!(params.calc_num_regions(), Ok(100));

    // Each pixel is its own region.
    for y in 0..10_usize {
        for x in 0..10_usize {
            assert_eq!(
                params.calculate_region_from_pixel((x, y)),
                Ok(y * 10 + x),
            );
        }
    }
}

// ── region boundary pixels ────────────────────────────────────────

#[test]
fn test_region_params_boundary_pixels() {
    do_region_params_boundary_pixels();
}

/// Pixels at region boundaries: last pixel of one region vs first
/// pixel of the next.
pub fn do_region_params_boundary_pixels() {
    let params = RegionParams::new(10, (1000, 1000));
    // region_size = 100 pixels per cell.

    // Last pixel of region 0: (99, 99).
    assert_eq!(params.calculate_region_from_pixel((99, 99)), Ok(0));
    // First pixel of region 1: (100, 0).
    assert_eq!(params.calculate_region_from_pixel((100, 0)), Ok(1));
    // First pixel of region 10: (0, 100).
    assert_eq!(params.calculate_region_from_pixel((0, 100)), Ok(10));
    // Last pixel of last region: (999, 999).
    assert_eq!(params.calculate_region_from_pixel((999, 999)), Ok(99));
    // Pixel (0, 999): first column, last row.
    assert_eq!(params.calculate_region_from_pixel((0, 999)), Ok(90));
    // Pixel (999, 0): last column, first row.
    assert_eq!(params.calculate_region_from_pixel((999, 0)), Ok(9));
}

// ── highly asymmetric resolution ──────────────────────────────────

#[test]
fn test_region_params_very_wide() {
    do_region_params_very_wide();
}

/// A very wide, short resolution (10000x4) to stress the asymmetry
/// path. Different factors and sizes per axis.
pub fn do_region_params_very_wide() {
    let params = RegionParams::new(10, (10000, 4));
    assert_eq!(params.read_region_factor_x(), Ok(10));
    // 4 has divisors: 1, 2, 4. Closest to 10 is 4.
    assert_eq!(params.read_region_factor_y(), Ok(4));
    assert_eq!(params.read_region_size_x(), Ok(1000));
    assert_eq!(params.read_region_size_y(), Ok(1));
    assert_eq!(params.calc_num_regions(), Ok(40));

    // Bottom-right pixel.
    assert_eq!(params.calculate_region_from_pixel((9999, 3)), Ok(39));
    // Top-left pixel.
    assert_eq!(params.calculate_region_from_pixel((0, 0)), Ok(0));
}

// ── rectangle: entire grid, single pixel, corners ─────────────────

#[test]
fn test_region_params_rect_entire_grid() {
    do_region_params_rect_entire_grid();
}

/// A rectangle covering the full grid returns every region.
pub fn do_region_params_rect_entire_grid() {
    let params = RegionParams::new(4, (100, 100));
    // factor=4 exact (100/4=25), so 16 regions total.
    let all = params
        .calculate_regions_intersected_by_rectangle((0, 0), (99, 99))
        .unwrap();
    assert_eq!(all.len(), 16);
    for r in 0..16 {
        assert!(all.contains(&r), "region {} should be in full-grid rect", r);
    }
}

#[test]
fn test_region_params_rect_single_pixel() {
    do_region_params_rect_single_pixel();
}

/// A zero-area rectangle (single point) maps to exactly one region.
pub fn do_region_params_rect_single_pixel() {
    let params = RegionParams::new(10, (1000, 1000));
    let one = params
        .calculate_regions_intersected_by_rectangle((500, 500), (500, 500))
        .unwrap();
    assert_eq!(one.len(), 1);
    assert_eq!(one[0], 55); // column 5, row 5 → 5*10+5=55
}

#[test]
fn test_region_params_rect_bottom_right_corner() {
    do_region_params_rect_bottom_right_corner();
}

/// Rectangle at the bottom-right corner of the grid.
pub fn do_region_params_rect_bottom_right_corner() {
    let params = RegionParams::new(10, (1000, 1000));
    let corner = params
        .calculate_regions_intersected_by_rectangle((900, 900), (999, 999))
        .unwrap();
    assert_eq!(corner, vec![99]);
}

// =====================================================================
// calculate_regions_intersected_by_rectangle — known limitations
// =====================================================================
//
// The algorithm in `calculate_regions_intersected_by_rectangle` scans
// by advancing one region-id at a time (`region + 1`) and terminates
// when the scan head exceeds *both* end.x and end.y simultaneously.
// This design has two known failure modes:
//
// BUG 1 — Termination failure on full-width rectangles:
// When end.x is the last pixel column (e.g. 999 on a 1000-wide
// grid), the scan head wraps to x=0 on the next row and can never
// exceed end.x again. The loop exhausts all regions and returns
// `Err(InvalidParameters("Region is out of bounds"))`.
//
// BUG 2 — Left-overshoot on offset rectangles:
// When start.x > 0, the scan head wraps to x=0 on subsequent rows.
// Since the algorithm only checks end bounds (not start bounds),
// it incorrectly includes regions to the LEFT of start.x on rows
// after the first.
//
// Both bugs stem from the same root cause: the linear region-id
// scan (`region + 1`) has no concept of "skip to the start column
// of the next row". The existing passing tests all use origin-
// anchored rectangles (start.x = 0) with end.x < last pixel,
// which avoids both bugs.
//
// The tests below isolate each failure mode.

// ── cases that work correctly ─────────────────────────────────────

#[test]
fn test_region_params_rect_origin_anchored_block() {
    do_region_params_rect_origin_anchored_block();
}

/// Origin-anchored multi-cell block — the happy path the algorithm
/// was designed for.
pub fn do_region_params_rect_origin_anchored_block() {
    let params = RegionParams::new(10, (1000, 1000));
    let result = params
        .calculate_regions_intersected_by_rectangle((0, 0), (299, 199))
        .unwrap();
    assert_eq!(result, vec![0, 1, 2, 10, 11, 12]);
}

#[test]
fn test_region_params_rect_single_row_offset() {
    do_region_params_rect_single_row_offset();
}

/// Single-row offset rectangle works because only one row is scanned
/// (no wrap → no left-overshoot).
pub fn do_region_params_rect_single_row_offset() {
    let params = RegionParams::new(10, (1000, 1000));
    let result = params
        .calculate_regions_intersected_by_rectangle((300, 200), (599, 299))
        .unwrap();
    assert_eq!(result, vec![23, 24, 25]);
}

// ── BUG 1: full-width rectangle fails to terminate ────────────────

#[test]
fn test_region_params_rect_full_width_fails() {
    do_region_params_rect_full_width_fails();
}

/// A rectangle spanning the full pixel width (end.x = 999) cannot
/// terminate — the scan head wraps to x=0 and never exceeds end.x.
/// The function returns an error when it overshoots the last region.
pub fn do_region_params_rect_full_width_fails() {
    let params = RegionParams::new(10, (1000, 1000));
    let result = params
        .calculate_regions_intersected_by_rectangle((0, 0), (999, 99));
    assert!(
        result.is_err(),
        "full-width rectangle should fail (BUG 1: termination failure)"
    );
}

#[test]
fn test_region_params_rect_last_column_fails() {
    do_region_params_rect_last_column_fails();
}

/// Even a multi-row rectangle whose end.x lands in the last region
/// column fails, because the head wraps to x=0 on every row after
/// the rectangle's vertical extent.
pub fn do_region_params_rect_last_column_fails() {
    let params = RegionParams::new(10, (1000, 1000));
    let result = params
        .calculate_regions_intersected_by_rectangle((800, 0), (999, 199));
    assert!(
        result.is_err(),
        "rectangle reaching last column should fail (BUG 1)"
    );
}

// ── BUG 2: left-overshoot on offset multi-row rectangle ───────────

#[test]
fn test_region_params_rect_offset_multi_row_includes_wrong_regions() {
    do_region_params_rect_offset_multi_row_includes_wrong_regions();
}

/// An offset rectangle spanning multiple rows includes regions to
/// the LEFT of start.x on the second row (and beyond). The algorithm
/// scans linearly and has no "skip to start column" logic.
///
/// Rectangle (300,400)-(499,599) should hit regions {43,44,53,54}
/// but the algorithm wraps to x=0 on row 5 and also includes
/// regions 50, 51, 52 (which are left of the rectangle).
pub fn do_region_params_rect_offset_multi_row_includes_wrong_regions() {
    let params = RegionParams::new(10, (1000, 1000));
    let result = params
        .calculate_regions_intersected_by_rectangle((300, 400), (499, 599));

    // The function either errors out (termination failure because
    // end.x=499 < last-column pixel) or returns a result with
    // spurious left-overshoot regions. In the current implementation
    // it errors because after row 5, wrap to x=0 and head.x never
    // exceeds 499 simultaneously with head.y exceeding 599.
    //
    // This documents the current broken behaviour — a correct
    // implementation would return Ok(vec![43, 44, 53, 54]).
    assert!(
        result.is_err() || result.as_ref().unwrap() != &vec![43, 44, 53, 54],
        "offset multi-row rectangle is incorrect (BUG 2)"
    );
}

// ── working cases near the bugs (regression guards) ───────────────

#[test]
fn test_region_params_rect_not_quite_full_width() {
    do_region_params_rect_not_quite_full_width();
}

/// A rectangle that spans all but the last column works — the scan
/// head can exceed end.x when it reaches the last column's start
/// pixel, allowing termination.
pub fn do_region_params_rect_not_quite_full_width() {
    let params = RegionParams::new(10, (1000, 1000));
    let result = params
        .calculate_regions_intersected_by_rectangle((0, 0), (899, 199))
        .unwrap();
    // Columns 0..8, rows 0..1 → 18 regions.
    assert_eq!(result.len(), 18);
    assert_eq!(result[0], 0);
    assert_eq!(result[8], 8);
    assert_eq!(result[9], 10);
    assert_eq!(result[17], 18);
}

// ── calculate_actual_region_factor: power-of-two dimensions ───────

#[test]
fn test_region_factor_power_of_two() {
    do_region_factor_power_of_two();
}

/// Power-of-two dimensions have well-known divisor sets. Verify
/// the closest-divisor logic against known values.
pub fn do_region_factor_power_of_two() {
    // 256: divisors include 1,2,4,8,16,32,64,128,256
    assert_eq!(RegionParams::calculate_actual_region_factor(10, 256), 8);
    assert_eq!(RegionParams::calculate_actual_region_factor(17, 256), 16);
    assert_eq!(RegionParams::calculate_actual_region_factor(20, 256), 16);
    // Equidistant from 16 and 32 → smaller wins.
    assert_eq!(RegionParams::calculate_actual_region_factor(24, 256), 16);
    assert_eq!(RegionParams::calculate_actual_region_factor(33, 256), 32);
}

#[test]
fn test_region_factor_dimension_is_1() {
    do_region_factor_dimension_is_1();
}

/// Dimension 1 has only divisor 1, regardless of target.
pub fn do_region_factor_dimension_is_1() {
    assert_eq!(RegionParams::calculate_actual_region_factor(1, 1), 1);
    assert_eq!(RegionParams::calculate_actual_region_factor(100, 1), 1);
}

#[test]
fn test_region_factor_dimension_is_2() {
    do_region_factor_dimension_is_2();
}

/// Dimension 2 has divisors {1, 2}.
pub fn do_region_factor_dimension_is_2() {
    assert_eq!(RegionParams::calculate_actual_region_factor(1, 2), 1);
    assert_eq!(RegionParams::calculate_actual_region_factor(2, 2), 2);
    // Target 10 > dimension 2 → clamped to 2.
    assert_eq!(RegionParams::calculate_actual_region_factor(10, 2), 2);
}

#[test]
fn test_region_factor_square_of_prime() {
    do_region_factor_square_of_prime();
}

/// Dimension 49 = 7². Divisors: {1, 7, 49}. The gap between 7 and
/// 49 is large — targets in the middle should snap to one or the
/// other.
pub fn do_region_factor_square_of_prime() {
    assert_eq!(RegionParams::calculate_actual_region_factor(7, 49), 7);
    assert_eq!(RegionParams::calculate_actual_region_factor(8, 49), 7);
    assert_eq!(RegionParams::calculate_actual_region_factor(27, 49), 7);
    // 28 is equidistant from 7 and 49 → smaller wins → 7.
    assert_eq!(RegionParams::calculate_actual_region_factor(28, 49), 7);
    assert_eq!(RegionParams::calculate_actual_region_factor(29, 49), 49);
    assert_eq!(RegionParams::calculate_actual_region_factor(49, 49), 49);
}

// ── adapt multiple times in sequence ──────────────────────────────

#[test]
fn test_region_params_adapt_chained() {
    do_region_params_adapt_chained();
}

/// Calling `adapt` multiple times, each time with a different
/// resolution, leaves the params in a consistent state matching
/// only the last adaptation.
pub fn do_region_params_adapt_chained() {
    let mut params = RegionParams::new(10, (1000, 1000));

    params.adapt(5, (500, 500));
    assert_eq!(params.calc_num_regions(), Ok(25));

    params.adapt(8, (800, 600));
    assert_eq!(params.read_current_resolution(), Ok((800, 600)));
    assert_eq!(params.read_region_factor_x(), Ok(8));
    // 600: divisors near 8 → 8 divides? 600/8=75, yes.
    assert_eq!(params.read_region_factor_y(), Ok(8));

    params.adapt(3, (120, 90));
    assert_eq!(params.read_current_resolution(), Ok((120, 90)));
    // 120: divisors near 3 → 3 divides? 120/3=40, yes.
    assert_eq!(params.read_region_factor_x(), Ok(3));
    // 90: divisors near 3 → 3 divides? 90/3=30, yes.
    assert_eq!(params.read_region_factor_y(), Ok(3));
    assert_eq!(params.read_region_size_x(), Ok(40));
    assert_eq!(params.read_region_size_y(), Ok(30));

    // Pixel mapping should use the latest resolution.
    assert_eq!(params.calculate_region_from_pixel((0, 0)), Ok(0));
    assert_eq!(params.calculate_region_from_pixel((119, 89)), Ok(8));
    assert_eq!(
        params.calculate_region_from_pixel((120, 0)),
        Err(RegionError::InvalidParameters("Pixel is out of bounds"))
    );
}

// ── roundtrip on asymmetric grid ──────────────────────────────────

#[test]
fn test_region_params_pixel_region_roundtrip_asymmetric() {
    do_region_params_pixel_region_roundtrip_asymmetric();
}

/// Pixel→region→pixel roundtrip on an asymmetric grid (1920x1080).
pub fn do_region_params_pixel_region_roundtrip_asymmetric() {
    let params = RegionParams::new(10, (1920, 1080));
    let num = params.calc_num_regions().unwrap();
    for r in 0..num {
        let pixel = params.calculate_pixel_from_region(r).unwrap();
        let back = params.calculate_region_from_pixel(pixel).unwrap();
        assert_eq!(back, r, "roundtrip failed for region {} on 1920x1080", r);
    }
}

// ── every pixel in a small grid maps to exactly one region ────────

#[test]
fn test_region_params_exhaustive_small_grid() {
    do_region_params_exhaustive_small_grid();
}

/// For a 12x12 grid with factor 3, exhaustively verify that every
/// pixel maps to a valid region, every region is hit by at least one
/// pixel, and the region count is exactly 9.
pub fn do_region_params_exhaustive_small_grid() {
    let params = RegionParams::new(3, (12, 12));
    assert_eq!(params.calc_num_regions(), Ok(9));

    let mut region_hits = vec![0_usize; 9];
    for y in 0..12_usize {
        for x in 0..12_usize {
            let r = params.calculate_region_from_pixel((x, y)).unwrap();
            assert!(r < 9, "pixel ({}, {}) mapped to out-of-range region {}", x, y, r);
            region_hits[r] += 1;
        }
    }
    // Every region should have been hit (12/3=4 pixels per axis per cell → 16 pixels each).
    for (r, count) in region_hits.iter().enumerate() {
        assert_eq!(*count, 16, "region {} should have 16 pixels, got {}", r, count);
    }
}
