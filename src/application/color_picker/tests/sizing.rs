//! Picker sizing: screen-short-side target, user size_scale
//! monotonicity, small-window font shrink, dimensionless advance
//! ratios, and the safety clamp that dominates user scale on
//! tiny screens.

use super::fixtures::sample_geometry;
use crate::application::color_picker::compute_color_picker_layout;
use crate::application::widgets::color_picker_widget::load_spec;

/// Canonical sizing formula: at `size_scale = 1.0` the picker
/// occupies roughly `target_frac` of the screen's shorter side.
#[test]
fn layout_targets_screen_short_side_fraction() {
    let g = sample_geometry();
    let spec = load_spec();
    let target_frac = spec.geometry.target_frac;
    for &(w, h) in &[(1920.0_f32, 1080.0_f32), (1280.0, 720.0), (800.0, 600.0)] {
        let layout = compute_color_picker_layout(&g, w, h);
        let short = w.min(h);
        let (_, _, bw, bh) = layout.backdrop;
        let max_extent = bw.max(bh);
        let upper = short * target_frac * 2.5;
        let lower = short * target_frac * 0.3;
        assert!(
            max_extent >= lower && max_extent <= upper,
            "{w}x{h}: backdrop extent {max_extent} not in [{lower}, {upper}] \
            — formula doesn't drive size from short axis"
        );
    }
}

/// User-controlled `size_scale` must scale the picker
/// proportionally.
#[test]
fn layout_scales_with_user_size_scale() {
    let g_baseline = sample_geometry();
    let mut g_grown = sample_geometry();
    g_grown.size_scale = 1.5;
    let mut g_shrunk = sample_geometry();
    g_shrunk.size_scale = 0.7;
    let baseline = compute_color_picker_layout(&g_baseline, 1920.0, 1080.0);
    let grown = compute_color_picker_layout(&g_grown, 1920.0, 1080.0);
    let shrunk = compute_color_picker_layout(&g_shrunk, 1920.0, 1080.0);
    let (_, _, bw_b, _) = baseline.backdrop;
    let (_, _, bw_g, _) = grown.backdrop;
    let (_, _, bw_s, _) = shrunk.backdrop;
    assert!(bw_g > bw_b, "size_scale=1.5 backdrop {bw_g} not larger than baseline {bw_b}");
    assert!(bw_s < bw_b, "size_scale=0.7 backdrop {bw_s} not smaller than baseline {bw_b}");
    assert!(grown.font_size > baseline.font_size);
    assert!(shrunk.font_size < baseline.font_size);
}

/// Small-window robustness: even on a tiny viewport the layout
/// must never produce negative geometry or a backdrop that
/// overflows the screen.
#[test]
fn layout_font_shrinks_on_small_windows() {
    let g = sample_geometry();
    let big = compute_color_picker_layout(&g, 1920.0, 1080.0);
    let small = compute_color_picker_layout(&g, 400.0, 300.0);
    assert!(
        small.font_size < big.font_size,
        "small-window font_size {} should shrink below big-window {}",
        small.font_size,
        big.font_size
    );
    let (left, top, bw, bh) = small.backdrop;
    assert!(left >= 0.0 && top >= 0.0);
    assert!(left + bw <= 400.5);
    assert!(top + bh <= 300.5);
}

/// `measurement_font_size` factors out: a stub measured at
/// font_size = 16 with cell_advance = 16 (ratio 1.0) should
/// produce the same layout as a stub measured at font_size = 8
/// with cell_advance = 8 (also ratio 1.0).
#[test]
fn layout_uses_dimensionless_advance_ratios() {
    let g_a = sample_geometry();
    let mut g_b = sample_geometry();
    g_b.measurement_font_size = 8.0;
    g_b.max_cell_advance = 8.0;
    g_b.max_ring_advance = 12.0;
    let layout_a = compute_color_picker_layout(&g_a, 1280.0, 720.0);
    let layout_b = compute_color_picker_layout(&g_b, 1280.0, 720.0);
    assert!((layout_a.font_size - layout_b.font_size).abs() < 1e-3);
    assert!((layout_a.outer_radius - layout_b.outer_radius).abs() < 1e-3);
}

/// A picker opened on a window-resize-shrunk viewport at
/// `size_scale = 0.5` keeps the backdrop fully on-screen.
#[test]
fn layout_safety_clamp_dominates_user_scale_on_tiny_screens() {
    let mut g = sample_geometry();
    g.size_scale = 1.5;
    let layout = compute_color_picker_layout(&g, 250.0, 200.0);
    let (left, top, bw, bh) = layout.backdrop;
    assert!(left >= 0.0 && top >= 0.0);
    assert!(left + bw <= 250.5);
    assert!(top + bh <= 200.5);
}
