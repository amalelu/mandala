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
