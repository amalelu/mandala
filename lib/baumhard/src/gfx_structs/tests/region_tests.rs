use crate::gfx_structs::util::regions::{RegionError, RegionIndexer, RegionParams};
use lazy_static::lazy_static;
use std::sync::Mutex;

#[test]
pub fn test_region_indexer_initialize() {
    do_region_indexer_initialize()
}

pub fn do_region_indexer_initialize() {
    let mut indexer = RegionIndexer::new();
    indexer.initialize(500);
    assert_eq!(indexer.index_as_ref().len(), 500);
}
lazy_static! {
    static ref TEST_INDEXER_INSERT_DELETE: Mutex<RegionIndexer> = {
        let mut indexer = RegionIndexer::new();
        indexer.initialize(500);
        Mutex::new(indexer)
    };
    static ref TEST_INDEXER: Mutex<RegionIndexer> = {
        let mut indexer = RegionIndexer::new();
        indexer.initialize(500);
        Mutex::new(indexer)
    };
}

#[test]
pub fn test_region_indexer_insert() {
    do_region_indexer_insert_delete();
}

pub fn do_region_indexer_insert_delete() {
    let mut indexer = TEST_INDEXER_INSERT_DELETE.lock().unwrap();
    for i in 0..500 {
        indexer.insert(i, i);
        assert_eq!(indexer.elements_in_region(i).len(), 1);
        assert!(indexer.elements_in_region(i).contains(&i));
        assert!(indexer.get_reverse_index_for_element(i).contains(&i));
        assert!(indexer.scan_regions_for_element(i).contains(&i));
    }

    for i in 0..500 {
        indexer.remove(i, i);
        assert_eq!(indexer.elements_in_region(i).len(), 0);
        assert!(!indexer.elements_in_region(i).contains(&i));
        assert!(!indexer.get_reverse_index_for_element(i).contains(&i));
        assert!(!indexer.scan_regions_for_element(i).contains(&i));
    }
}

#[test]
pub fn test_region_params_calculate_region_from_pixel() {
    do_region_params_calculate_region_from_pixel()
}

pub fn do_region_params_calculate_region_from_pixel() {
    let mut params = RegionParams::new(10, (1000, 1000));
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
pub fn test_region_params_calculate_regions_intersected_by_rectangle() {
    do_region_params_calculate_regions_intersected_by_rectangle()
}

pub fn do_region_params_calculate_regions_intersected_by_rectangle() {
   let mut params = RegionParams::new(10, (1000, 1000));
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
pub fn test_region_params_calculate_pixel_from_region() {
    do_region_params_calculate_pixel_from_region()
}

pub fn do_region_params_calculate_pixel_from_region() {
    let mut params = RegionParams::new(10, (1000, 1000));
    assert_eq!(params.calculate_pixel_from_region(0), Ok((0, 0)));
    assert_eq!(params.calculate_pixel_from_region(1), Ok((100, 0)));
    assert_eq!(params.calculate_pixel_from_region(2), Ok((200, 0)));
    assert_eq!(params.calculate_pixel_from_region(3), Ok((300, 0)));
    assert_eq!(params.calculate_pixel_from_region(4), Ok((400, 0)));
    assert_eq!(params.calculate_pixel_from_region(5), Ok((500, 0)));
    assert_eq!(params.calculate_pixel_from_region(6), Ok((600, 0)));
    assert_eq!(params.calculate_pixel_from_region(7), Ok((700, 0)));
    assert_eq!(params.calculate_pixel_from_region(8), Ok((800, 0)));
    assert_eq!(params.calculate_pixel_from_region(9), Ok((900, 0)));
    assert_eq!(params.calculate_pixel_from_region(10), Ok((0, 100)));
    assert_eq!(params.calculate_pixel_from_region(11), Ok((100, 100)));
    assert_eq!(params.calculate_pixel_from_region(12), Ok((200, 100)));
    assert_eq!(params.calculate_pixel_from_region(13), Ok((300, 100)));
    assert_eq!(params.calculate_pixel_from_region(14), Ok((400, 100)));
    assert_eq!(params.calculate_pixel_from_region(21), Ok((100, 200)));
    assert_eq!(params.calculate_pixel_from_region(29), Ok((900, 200)));
    assert_eq!(params.calculate_pixel_from_region(59), Ok((900, 500)));
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

#[test]
pub fn test_region_params_new_sunny_day() {
    do_region_params_new_sunny_day()
}

pub fn do_region_params_new_sunny_day() {
    let mut params = RegionParams::new(10, (1000, 1000));
    assert_eq!(params.read_region_size_y(), Ok(100));
    assert_eq!(params.read_region_size_x(), Ok(100));
    assert_eq!(params.read_current_resolution(), Ok((1000, 1000)));
    assert_eq!(params.read_region_factor_x(), Ok(10));
    assert_eq!(params.read_region_factor_y(), Ok(10));
    assert_eq!(params.read_target_region_factor(), Ok(10));
}

#[test]
pub fn test_regions_params_new_rainy_day() {
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
#[should_panic]
pub fn test_region_params_prime_1() {
    let mut params = RegionParams::new(7, (1000, 1000));
    params.adapt(13, (241, 251));
}

#[test]
#[should_panic]
pub fn test_region_params_prime_2() {
    let mut params = RegionParams::new(7, (1000, 1000));
    params.adapt(10, (241, 1000));
}

#[test]
#[should_panic]
pub fn test_region_params_prime_3() {
    let mut params = RegionParams::new(7, (1000, 1000));
    params.adapt(10, (1000, 251));
}

#[test]
#[should_panic]
pub fn test_region_params_prime_4() {
    RegionParams::new(10, (251, 1000));
}

#[test]
#[should_panic]
pub fn test_region_params_prime_5() {
    RegionParams::new(10, (1000, 251));
}
