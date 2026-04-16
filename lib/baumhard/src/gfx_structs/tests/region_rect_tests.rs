//! Tests for [`RegionParams::calculate_regions_intersected_by_rectangle`].
//!
//! Separated from `region_tests.rs` to keep files under 1000 lines.
//! Follows the `do_*()` / `test_*()` benchmark-reuse split (§T2.2).

use crate::gfx_structs::util::regions::{RegionError, RegionParams};

// ── origin-anchored rectangles ────────────────────────────────────

#[test]
fn test_rect_origin_4x4_block() {
    do_rect_origin_4x4_block();
}

/// Standard 4x4 cell block from origin.
pub fn do_rect_origin_4x4_block() {
    let params = RegionParams::new(10, (1000, 1000));
    assert_eq!(
        params.calculate_regions_intersected_by_rectangle((0, 0), (399, 399)).unwrap(),
        vec![0, 1, 2, 3, 10, 11, 12, 13, 20, 21, 22, 23, 30, 31, 32, 33]
    );
}

#[test]
fn test_rect_origin_4x5_block() {
    do_rect_origin_4x5_block();
}

pub fn do_rect_origin_4x5_block() {
    let params = RegionParams::new(10, (1000, 1000));
    assert_eq!(
        params.calculate_regions_intersected_by_rectangle((0, 0), (399, 400)).unwrap(),
        vec![0, 1, 2, 3, 10, 11, 12, 13, 20, 21, 22, 23, 30, 31, 32, 33, 40, 41, 42, 43]
    );
}

#[test]
fn test_rect_origin_5x4_block() {
    do_rect_origin_5x4_block();
}

pub fn do_rect_origin_5x4_block() {
    let params = RegionParams::new(10, (1000, 1000));
    assert_eq!(
        params.calculate_regions_intersected_by_rectangle((0, 0), (400, 399)).unwrap(),
        vec![0, 1, 2, 3, 4, 10, 11, 12, 13, 14, 20, 21, 22, 23, 24, 30, 31, 32, 33, 34]
    );
}

#[test]
fn test_rect_origin_3x2_block() {
    do_rect_origin_3x2_block();
}

pub fn do_rect_origin_3x2_block() {
    let params = RegionParams::new(10, (1000, 1000));
    assert_eq!(
        params.calculate_regions_intersected_by_rectangle((0, 0), (299, 199)).unwrap(),
        vec![0, 1, 2, 10, 11, 12]
    );
}

// ── single-cell and single-pixel rectangles ───────────────────────

#[test]
fn test_rect_single_cell() {
    do_rect_single_cell();
}

pub fn do_rect_single_cell() {
    let params = RegionParams::new(10, (1000, 1000));
    assert_eq!(
        params.calculate_regions_intersected_by_rectangle((0, 0), (99, 99)).unwrap(),
        vec![0]
    );
    assert_eq!(
        params.calculate_regions_intersected_by_rectangle((99, 99), (99, 99)).unwrap(),
        vec![0]
    );
}

#[test]
fn test_rect_single_pixel_each_corner() {
    do_rect_single_pixel_each_corner();
}

/// Single-pixel rectangle at each corner of the grid.
pub fn do_rect_single_pixel_each_corner() {
    let params = RegionParams::new(10, (1000, 1000));
    assert_eq!(
        params.calculate_regions_intersected_by_rectangle((0, 0), (0, 0)).unwrap(),
        vec![0]
    );
    assert_eq!(
        params.calculate_regions_intersected_by_rectangle((999, 0), (999, 0)).unwrap(),
        vec![9]
    );
    assert_eq!(
        params.calculate_regions_intersected_by_rectangle((0, 999), (0, 999)).unwrap(),
        vec![90]
    );
    assert_eq!(
        params.calculate_regions_intersected_by_rectangle((999, 999), (999, 999)).unwrap(),
        vec![99]
    );
}

#[test]
fn test_rect_single_pixel_center() {
    do_rect_single_pixel_center();
}

pub fn do_rect_single_pixel_center() {
    let params = RegionParams::new(10, (1000, 1000));
    assert_eq!(
        params.calculate_regions_intersected_by_rectangle((500, 500), (500, 500)).unwrap(),
        vec![55]
    );
}

// ── full-width and full-height rectangles ─────────────────────────

#[test]
fn test_rect_full_width_single_row() {
    do_rect_full_width_single_row();
}

pub fn do_rect_full_width_single_row() {
    let params = RegionParams::new(10, (1000, 1000));
    assert_eq!(
        params.calculate_regions_intersected_by_rectangle((0, 0), (999, 99)).unwrap(),
        vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]
    );
}

#[test]
fn test_rect_full_height_single_column() {
    do_rect_full_height_single_column();
}

pub fn do_rect_full_height_single_column() {
    let params = RegionParams::new(10, (1000, 1000));
    assert_eq!(
        params.calculate_regions_intersected_by_rectangle((0, 0), (99, 999)).unwrap(),
        vec![0, 10, 20, 30, 40, 50, 60, 70, 80, 90]
    );
}

#[test]
fn test_rect_full_grid() {
    do_rect_full_grid();
}

/// Rectangle covering the entire pixel grid returns every region.
pub fn do_rect_full_grid() {
    let params = RegionParams::new(10, (1000, 1000));
    let result = params
        .calculate_regions_intersected_by_rectangle((0, 0), (999, 999))
        .unwrap();
    let expected: Vec<usize> = (0..100).collect();
    assert_eq!(result, expected);
}

// ── thin strips ───────────────────────────────────────────────────

#[test]
fn test_rect_thin_vertical_strip() {
    do_rect_thin_vertical_strip();
}

/// A 1-pixel-wide vertical strip spanning the full height.
pub fn do_rect_thin_vertical_strip() {
    let params = RegionParams::new(10, (1000, 1000));
    let strip = params
        .calculate_regions_intersected_by_rectangle((50, 0), (50, 999))
        .unwrap();
    assert_eq!(strip, vec![0, 10, 20, 30, 40, 50, 60, 70, 80, 90]);
}

#[test]
fn test_rect_thin_horizontal_strip() {
    do_rect_thin_horizontal_strip();
}

/// A 1-pixel-tall horizontal strip spanning the full width.
pub fn do_rect_thin_horizontal_strip() {
    let params = RegionParams::new(10, (1000, 1000));
    let strip = params
        .calculate_regions_intersected_by_rectangle((0, 50), (999, 50))
        .unwrap();
    assert_eq!(strip, vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
}

// ── offset rectangles ─────────────────────────────────────────────

#[test]
fn test_rect_offset_single_row() {
    do_rect_offset_single_row();
}

pub fn do_rect_offset_single_row() {
    let params = RegionParams::new(10, (1000, 1000));
    assert_eq!(
        params.calculate_regions_intersected_by_rectangle((100, 99), (200, 99)).unwrap(),
        vec![1, 2]
    );
    assert_eq!(
        params.calculate_regions_intersected_by_rectangle((100, 100), (200, 100)).unwrap(),
        vec![11, 12]
    );
    assert_eq!(
        params.calculate_regions_intersected_by_rectangle((300, 200), (599, 299)).unwrap(),
        vec![23, 24, 25]
    );
}

#[test]
fn test_rect_offset_multi_row() {
    do_rect_offset_multi_row();
}

/// Offset multi-row rectangle returns only the correct regions.
pub fn do_rect_offset_multi_row() {
    let params = RegionParams::new(10, (1000, 1000));
    assert_eq!(
        params.calculate_regions_intersected_by_rectangle((300, 400), (499, 599)).unwrap(),
        vec![43, 44, 53, 54]
    );
}

#[test]
fn test_rect_offset_last_column_multi_row() {
    do_rect_offset_last_column_multi_row();
}

pub fn do_rect_offset_last_column_multi_row() {
    let params = RegionParams::new(10, (1000, 1000));
    assert_eq!(
        params.calculate_regions_intersected_by_rectangle((800, 0), (999, 199)).unwrap(),
        vec![8, 9, 18, 19]
    );
}

#[test]
fn test_rect_center_3x3() {
    do_rect_center_3x3();
}

pub fn do_rect_center_3x3() {
    let params = RegionParams::new(10, (1000, 1000));
    // Columns 3-5, rows 3-5 = 9 regions.
    assert_eq!(
        params.calculate_regions_intersected_by_rectangle((300, 300), (599, 599)).unwrap(),
        vec![33, 34, 35, 43, 44, 45, 53, 54, 55]
    );
}

#[test]
fn test_rect_bottom_right_2x2() {
    do_rect_bottom_right_2x2();
}

pub fn do_rect_bottom_right_2x2() {
    let params = RegionParams::new(10, (1000, 1000));
    assert_eq!(
        params.calculate_regions_intersected_by_rectangle((800, 800), (999, 999)).unwrap(),
        vec![88, 89, 98, 99]
    );
}

// ── asymmetric grid ───────────────────────────────────────────────

#[test]
fn test_rect_asymmetric_grid() {
    do_rect_asymmetric_grid();
}

/// Rectangle on an asymmetric grid (1920x1080, factor 10).
pub fn do_rect_asymmetric_grid() {
    let params = RegionParams::new(10, (1920, 1080));
    // cell_x=192, cell_y=108.

    // Single cell at col 2, row 3.
    assert_eq!(
        params.calculate_regions_intersected_by_rectangle((384, 324), (575, 431)).unwrap(),
        vec![32]
    );

    // 2x2 block crossing cell boundaries: cols 2-3, rows 3-4.
    assert_eq!(
        params.calculate_regions_intersected_by_rectangle((384, 324), (576, 432)).unwrap(),
        vec![32, 33, 42, 43]
    );

    // Full width, one row.
    assert_eq!(
        params.calculate_regions_intersected_by_rectangle((0, 0), (1919, 107)).unwrap(),
        vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]
    );

    // Full grid.
    let all = params
        .calculate_regions_intersected_by_rectangle((0, 0), (1919, 1079))
        .unwrap();
    assert_eq!(all.len(), 100);
}

// ── error cases ───────────────────────────────────────────────────

#[test]
fn test_rect_start_after_end() {
    do_rect_start_after_end();
}

pub fn do_rect_start_after_end() {
    let params = RegionParams::new(10, (1000, 1000));
    assert_eq!(
        params.calculate_regions_intersected_by_rectangle((100, 100), (99, 99)),
        Err(RegionError::InvalidParameters("Start position is higher than end position"))
    );
}

#[test]
fn test_rect_start_out_of_bounds() {
    do_rect_start_out_of_bounds();
}

pub fn do_rect_start_out_of_bounds() {
    let params = RegionParams::new(10, (1000, 1000));
    assert_eq!(
        params.calculate_regions_intersected_by_rectangle((1000, 1000), (2000, 2000)),
        Err(RegionError::InvalidParameters("Start position is out of resolution bounds"))
    );
}

#[test]
fn test_rect_end_out_of_bounds() {
    do_rect_end_out_of_bounds();
}

pub fn do_rect_end_out_of_bounds() {
    let params = RegionParams::new(10, (1000, 1000));
    assert_eq!(
        params.calculate_regions_intersected_by_rectangle((999, 999), (2000, 2000)),
        Err(RegionError::InvalidParameters("End position is out of resolution bounds"))
    );
}

// ── exhaustive brute-force verification ───────────────────────────

#[test]
fn test_rect_exhaustive_4x4_grid() {
    do_rect_exhaustive_4x4_grid();
}

/// On a 4x4 pixel grid with factor 2 (4 regions), test every
/// possible valid rectangle and verify against brute-force.
pub fn do_rect_exhaustive_4x4_grid() {
    let params = RegionParams::new(2, (4, 4));

    for sy in 0..4_usize {
        for sx in 0..4_usize {
            for ey in sy..4_usize {
                for ex in sx..4_usize {
                    let result = params
                        .calculate_regions_intersected_by_rectangle((sx, sy), (ex, ey))
                        .unwrap();

                    // Brute-force: collect regions that contain at
                    // least one pixel in the rectangle.
                    let mut expected = std::collections::BTreeSet::new();
                    for y in sy..=ey {
                        for x in sx..=ex {
                            expected.insert(
                                params.calculate_region_from_pixel((x, y)).unwrap()
                            );
                        }
                    }
                    let expected_vec: Vec<usize> = expected.into_iter().collect();

                    assert_eq!(
                        result, expected_vec,
                        "rect ({},{})-({},{}) mismatch", sx, sy, ex, ey
                    );
                }
            }
        }
    }
}

#[test]
fn test_rect_exhaustive_12x12_grid() {
    do_rect_exhaustive_12x12_grid();
}

/// On a 12x12 grid with factor 3 (9 regions), exhaustively test
/// a selection of rectangles at cell boundaries and mid-cell points.
pub fn do_rect_exhaustive_12x12_grid() {
    let params = RegionParams::new(3, (12, 12));
    // cell_size = 4x4, 3x3 grid.

    // Test a representative sample of rectangles.
    let cases: Vec<((usize, usize), (usize, usize), Vec<usize>)> = vec![
        // Single cell.
        ((0, 0), (3, 3), vec![0]),
        ((4, 0), (7, 3), vec![1]),
        ((8, 8), (11, 11), vec![8]),
        // 2x1 row.
        ((0, 0), (7, 3), vec![0, 1]),
        // 1x2 column.
        ((0, 0), (3, 7), vec![0, 3]),
        // Full row.
        ((0, 0), (11, 3), vec![0, 1, 2]),
        // Full column.
        ((0, 0), (3, 11), vec![0, 3, 6]),
        // Full grid.
        ((0, 0), (11, 11), vec![0, 1, 2, 3, 4, 5, 6, 7, 8]),
        // Offset 2x2 block.
        ((4, 4), (11, 11), vec![4, 5, 7, 8]),
        // Sub-cell single pixel.
        ((5, 5), (5, 5), vec![4]),
        // Cross-cell single pixel row.
        ((3, 0), (4, 0), vec![0, 1]),
    ];

    for (start, end, expected) in &cases {
        let result = params
            .calculate_regions_intersected_by_rectangle(*start, *end)
            .unwrap();
        assert_eq!(
            &result, expected,
            "rect {:?}-{:?} expected {:?}, got {:?}",
            start, end, expected, result
        );
    }
}
