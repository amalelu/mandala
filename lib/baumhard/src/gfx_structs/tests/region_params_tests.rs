//! Tests for [`RegionParams`] — grid parameter management (§T1).
//!
//! Covers construction, pixel↔region mapping, adaptation, factor
//! calculation, boundary conditions, and prime-dimension panics.
//! Rectangle intersection tests live in `region_rect_tests.rs`.
//! Follows the `do_*()` / `test_*()` benchmark-reuse split (§T2.2).

use crate::gfx_structs::util::regions::{RegionError, RegionParams};

// ── construction ──────────────────────────────────────────────────

#[test]
fn test_region_params_new_sunny_day() {
    do_region_params_new_sunny_day()
}

pub fn do_region_params_new_sunny_day() {
    let params = RegionParams::new(10, (1000, 1000));
    assert_eq!(params.read_region_size_x(), Ok(100));
    assert_eq!(params.read_region_size_y(), Ok(100));
    assert_eq!(params.read_current_resolution(), Ok((1000, 1000)));
    assert_eq!(params.read_region_factor_x(), Ok(10));
    assert_eq!(params.read_region_factor_y(), Ok(10));
    assert_eq!(params.read_target_region_factor(), Ok(10));
}

#[test]
fn test_region_params_non_divisor_target() {
    do_region_params_non_divisor_target();
}

pub fn do_region_params_non_divisor_target() {
    let mut params = RegionParams::new(7, (1000, 1000));
    assert_eq!(params.read_region_factor_x(), Ok(8));
    assert_eq!(params.read_region_size_x(), Ok(125));
    assert_eq!(params.read_target_region_factor(), Ok(7));

    params.adapt(6, (1000, 1000));
    assert_eq!(params.read_region_factor_x(), Ok(5));
    assert_eq!(params.read_region_size_x(), Ok(200));

    params.adapt(13, (1000, 1000));
    assert_eq!(params.read_region_factor_x(), Ok(10));
}

#[test]
fn test_region_params_resolution_1x1() {
    do_region_params_resolution_1x1();
}

pub fn do_region_params_resolution_1x1() {
    let params = RegionParams::new(10, (1, 1));
    assert_eq!(params.read_region_factor_x(), Ok(1));
    assert_eq!(params.calc_num_regions(), Ok(1));
    assert_eq!(params.calculate_region_from_pixel((0, 0)), Ok(0));
}

#[test]
fn test_region_params_factor_one() {
    do_region_params_factor_one();
}

pub fn do_region_params_factor_one() {
    let params = RegionParams::new(1, (1920, 1080));
    assert_eq!(params.calc_num_regions(), Ok(1));
    assert_eq!(params.calculate_region_from_pixel((0, 0)), Ok(0));
    assert_eq!(params.calculate_region_from_pixel((1919, 1079)), Ok(0));
}

#[test]
fn test_region_params_factor_equals_dimension() {
    do_region_params_factor_equals_dimension();
}

pub fn do_region_params_factor_equals_dimension() {
    let params = RegionParams::new(10, (10, 10));
    assert_eq!(params.read_region_size_x(), Ok(1));
    assert_eq!(params.calc_num_regions(), Ok(100));
    for y in 0..10_usize {
        for x in 0..10_usize {
            assert_eq!(params.calculate_region_from_pixel((x, y)), Ok(y * 10 + x));
        }
    }
}

// ── pixel ↔ region mapping ────────────────────────────────────────

#[test]
fn test_region_params_pixel_to_region() {
    do_region_params_pixel_to_region();
}

pub fn do_region_params_pixel_to_region() {
    let params = RegionParams::new(10, (1000, 1000));
    assert_eq!(params.calculate_region_from_pixel((0, 0)), Ok(0));
    assert_eq!(params.calculate_region_from_pixel((99, 99)), Ok(0));
    assert_eq!(params.calculate_region_from_pixel((100, 0)), Ok(1));
    assert_eq!(params.calculate_region_from_pixel((0, 100)), Ok(10));
    assert_eq!(params.calculate_region_from_pixel((300, 200)), Ok(23));
    assert_eq!(params.calculate_region_from_pixel((999, 999)), Ok(99));
    assert_eq!(
        params.calculate_region_from_pixel((1000, 0)),
        Err(RegionError::InvalidParameters("Pixel is out of bounds"))
    );
    assert_eq!(
        params.calculate_region_from_pixel((0, 1000)),
        Err(RegionError::InvalidParameters("Pixel is out of bounds"))
    );
}

#[test]
fn test_region_params_region_to_pixel() {
    do_region_params_region_to_pixel();
}

pub fn do_region_params_region_to_pixel() {
    let params = RegionParams::new(10, (1000, 1000));
    assert_eq!(params.calculate_pixel_from_region(0), Ok((0, 0)));
    assert_eq!(params.calculate_pixel_from_region(1), Ok((100, 0)));
    assert_eq!(params.calculate_pixel_from_region(10), Ok((0, 100)));
    assert_eq!(params.calculate_pixel_from_region(99), Ok((900, 900)));
    assert_eq!(
        params.calculate_pixel_from_region(100),
        Err(RegionError::InvalidParameters("Region is out of bounds"))
    );
}

#[test]
fn test_region_params_pixel_region_roundtrip() {
    do_region_params_pixel_region_roundtrip();
}

pub fn do_region_params_pixel_region_roundtrip() {
    let params = RegionParams::new(10, (1000, 1000));
    for r in 0..100 {
        let pixel = params.calculate_pixel_from_region(r).unwrap();
        assert_eq!(params.calculate_region_from_pixel(pixel).unwrap(), r);
    }
}

#[test]
fn test_region_params_roundtrip_asymmetric() {
    do_region_params_roundtrip_asymmetric();
}

pub fn do_region_params_roundtrip_asymmetric() {
    let params = RegionParams::new(10, (1920, 1080));
    for r in 0..params.calc_num_regions().unwrap() {
        let pixel = params.calculate_pixel_from_region(r).unwrap();
        assert_eq!(params.calculate_region_from_pixel(pixel).unwrap(), r);
    }
}

// ── boundary pixels ───────────────────────────────────────────────

#[test]
fn test_region_params_boundary_pixels() {
    do_region_params_boundary_pixels();
}

pub fn do_region_params_boundary_pixels() {
    let params = RegionParams::new(10, (1000, 1000));
    assert_eq!(params.calculate_region_from_pixel((99, 99)), Ok(0));
    assert_eq!(params.calculate_region_from_pixel((100, 0)), Ok(1));
    assert_eq!(params.calculate_region_from_pixel((0, 100)), Ok(10));
    assert_eq!(params.calculate_region_from_pixel((999, 999)), Ok(99));
    assert_eq!(params.calculate_region_from_pixel((0, 999)), Ok(90));
    assert_eq!(params.calculate_region_from_pixel((999, 0)), Ok(9));
}

// ── asymmetric / wide resolutions ─────────────────────────────────

#[test]
fn test_region_params_asymmetric() {
    do_region_params_asymmetric();
}

pub fn do_region_params_asymmetric() {
    let params = RegionParams::new(10, (1920, 1080));
    assert_eq!(params.read_region_size_x(), Ok(192));
    assert_eq!(params.read_region_size_y(), Ok(108));
    assert_eq!(params.calculate_region_from_pixel((192, 108)), Ok(11));
}

#[test]
fn test_region_params_very_wide() {
    do_region_params_very_wide();
}

pub fn do_region_params_very_wide() {
    let params = RegionParams::new(10, (10000, 4));
    assert_eq!(params.read_region_factor_y(), Ok(4));
    assert_eq!(params.read_region_size_y(), Ok(1));
    assert_eq!(params.calc_num_regions(), Ok(40));
    assert_eq!(params.calculate_region_from_pixel((9999, 3)), Ok(39));
}

// ── exhaustive small grid ─────────────────────────────────────────

#[test]
fn test_region_params_exhaustive_4x4() {
    do_region_params_exhaustive_4x4();
}

pub fn do_region_params_exhaustive_4x4() {
    let params = RegionParams::new(2, (4, 4));
    let expected = [[0,0,1,1],[0,0,1,1],[2,2,3,3],[2,2,3,3]];
    for y in 0..4_usize {
        for x in 0..4_usize {
            assert_eq!(params.calculate_region_from_pixel((x, y)), Ok(expected[y][x]));
        }
    }
}

#[test]
fn test_region_params_exhaustive_12x12() {
    do_region_params_exhaustive_12x12();
}

pub fn do_region_params_exhaustive_12x12() {
    let params = RegionParams::new(3, (12, 12));
    let mut hits = vec![0_usize; 9];
    for y in 0..12_usize {
        for x in 0..12_usize {
            let r = params.calculate_region_from_pixel((x, y)).unwrap();
            assert!(r < 9);
            hits[r] += 1;
        }
    }
    for (r, c) in hits.iter().enumerate() {
        assert_eq!(*c, 16, "region {} should have 16 pixels", r);
    }
}

// ── adapt ─────────────────────────────────────────────────────────

#[test]
fn test_region_params_adapt_changes_resolution() {
    do_region_params_adapt_changes_resolution();
}

pub fn do_region_params_adapt_changes_resolution() {
    let mut params = RegionParams::new(10, (1000, 1000));
    params.adapt(5, (500, 500));
    assert_eq!(params.read_current_resolution(), Ok((500, 500)));
    assert_eq!(params.calc_num_regions(), Ok(25));
}

#[test]
fn test_region_params_adapt_chained() {
    do_region_params_adapt_chained();
}

pub fn do_region_params_adapt_chained() {
    let mut params = RegionParams::new(10, (1000, 1000));
    params.adapt(5, (500, 500));
    assert_eq!(params.calc_num_regions(), Ok(25));

    params.adapt(8, (800, 600));
    assert_eq!(params.read_region_factor_x(), Ok(8));

    params.adapt(3, (120, 90));
    assert_eq!(params.read_region_size_x(), Ok(40));
    assert_eq!(params.read_region_size_y(), Ok(30));
    assert_eq!(params.calculate_region_from_pixel((119, 89)), Ok(8));
    assert_eq!(
        params.calculate_region_from_pixel((120, 0)),
        Err(RegionError::InvalidParameters("Pixel is out of bounds"))
    );
}

// ── calculate_actual_region_factor ────────────────────────────────

#[test]
fn test_factor_zero_inputs() {
    assert_eq!(RegionParams::calculate_actual_region_factor(0, 1000), 1);
    assert_eq!(RegionParams::calculate_actual_region_factor(10, 0), 1);
}

#[test]
fn test_factor_exact_divisor() {
    assert_eq!(RegionParams::calculate_actual_region_factor(10, 1000), 10);
    assert_eq!(RegionParams::calculate_actual_region_factor(25, 1000), 25);
}

#[test]
fn test_factor_target_exceeds_dimension() {
    assert_eq!(RegionParams::calculate_actual_region_factor(100, 10), 10);
    assert_eq!(RegionParams::calculate_actual_region_factor(500, 4), 4);
}

#[test]
fn test_factor_closest_divisor() {
    assert_eq!(RegionParams::calculate_actual_region_factor(7, 1000), 8);
    assert_eq!(RegionParams::calculate_actual_region_factor(6, 1000), 5);
    assert_eq!(RegionParams::calculate_actual_region_factor(3, 1000), 2);
}

#[test]
fn test_factor_power_of_two() {
    assert_eq!(RegionParams::calculate_actual_region_factor(10, 256), 8);
    assert_eq!(RegionParams::calculate_actual_region_factor(24, 256), 16);
    assert_eq!(RegionParams::calculate_actual_region_factor(33, 256), 32);
}

#[test]
fn test_factor_dimension_1_and_2() {
    assert_eq!(RegionParams::calculate_actual_region_factor(100, 1), 1);
    assert_eq!(RegionParams::calculate_actual_region_factor(1, 2), 1);
    assert_eq!(RegionParams::calculate_actual_region_factor(2, 2), 2);
    assert_eq!(RegionParams::calculate_actual_region_factor(10, 2), 2);
}

#[test]
fn test_factor_square_of_prime() {
    // 49 = 7². Divisors: {1, 7, 49}.
    assert_eq!(RegionParams::calculate_actual_region_factor(7, 49), 7);
    assert_eq!(RegionParams::calculate_actual_region_factor(8, 49), 7);
    assert_eq!(RegionParams::calculate_actual_region_factor(28, 49), 7);
    assert_eq!(RegionParams::calculate_actual_region_factor(29, 49), 49);
}

// ── prime panics ──────────────────────────────────────────────────

#[test]
#[should_panic]
fn test_prime_x() { RegionParams::new(10, (251, 1000)); }

#[test]
#[should_panic]
fn test_prime_y() { RegionParams::new(10, (1000, 251)); }

#[test]
#[should_panic]
fn test_adapt_prime_x() {
    let mut p = RegionParams::new(7, (1000, 1000));
    p.adapt(10, (241, 1000));
}

#[test]
#[should_panic]
fn test_adapt_prime_y() {
    let mut p = RegionParams::new(7, (1000, 1000));
    p.adapt(10, (1000, 251));
}

#[test]
#[should_panic]
fn test_adapt_prime_both() {
    let mut p = RegionParams::new(7, (1000, 1000));
    p.adapt(13, (241, 251));
}
