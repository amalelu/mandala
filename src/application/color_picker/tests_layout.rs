//! Picker unit tests — part 1 of 2. Layout / crosshair / ink-offset
//! coverage. Split from the pre-consolidation single tests module
//! for the 600-line-per-file target.

use super::*;
use crate::application::widgets::color_picker_widget::load_spec;


    fn sample_geometry() -> ColorPickerOverlayGeometry {
        // Plausible stub advances measured at a notional 16 pt
        // baseline. cell ratio = 1.0 (worst-case sacred-script-ish),
        // ring ratio = 0.7 (typical at ring_scale = 1.7). The
        // pure-function layout only cares that the numbers are
        // non-zero and self-consistent. Ink offsets default to zero
        // so the layout tests see the classic em-box centering
        // unless a test explicitly overrides them.
        ColorPickerOverlayGeometry {
            target_label: "edge",
            hue_deg: 0.0,
            sat: 1.0,
            val: 1.0,
            preview_hex: "#ff0000".to_string(),
            hex_visible: false,
            max_cell_advance: 16.0,
            max_ring_advance: 24.0,
            measurement_font_size: 16.0,
            size_scale: 1.0,
            center_override: None,
            hovered_hit: None,
            arm_top_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
            arm_bottom_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
            arm_left_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
            arm_right_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
            preview_ink_offset: (0.0, 0.0),
        }
    }

    fn sample_geometry_with_hex() -> ColorPickerOverlayGeometry {
        let mut g = sample_geometry();
        g.hex_visible = true;
        g
    }

    /// The per-glyph arm and preview ink offsets carried on geometry
    /// are subtracted from the corresponding cell / preview-anchor
    /// positions at layout time. The previous revision tested a
    /// uniform per-arm offset; we now test that each cell consumes
    /// its own array entry — varying the offset per index and
    /// checking each cell shifted by exactly its own (dx, dy) scaled
    /// to layout pixels.
    #[test]
    fn layout_subtracts_ink_offsets_for_arms_and_preview() {
        let baseline = compute_color_picker_layout(&sample_geometry(), 1280.0, 720.0);
        let mut g = sample_geometry();
        // Distinct per-cell offsets so a "single offset for the whole
        // arm" regression would visibly fail. Index `i` (arm-local)
        // carries (0.01*(i+1), -0.02*(i+1)) on the top arm, etc.
        for i in 0..CROSSHAIR_CENTER_CELL {
            let f = (i + 1) as f32;
            g.arm_top_ink_offsets[i] = (0.01 * f, -0.02 * f);
            g.arm_bottom_ink_offsets[i] = (-0.01 * f, 0.02 * f);
            g.arm_left_ink_offsets[i] = (0.015 * f, 0.005 * f);
            g.arm_right_ink_offsets[i] = (-0.015 * f, -0.005 * f);
        }
        g.preview_ink_offset = (0.25, -0.3);
        let shifted = compute_color_picker_layout(&g, 1280.0, 720.0);

        let cell_fs = baseline.cell_font_size;
        let preview_size = baseline.preview_size;

        // Val bar: each arm cell shifts by its own (dx, dy)*cell_fs.
        // Centre cell at CROSSHAIR_CENTER_CELL is untouched.
        for i in 0..VAL_CELL_COUNT {
            let (expect_dx, expect_dy) = if i == CROSSHAIR_CENTER_CELL {
                (0.0, 0.0)
            } else if i < CROSSHAIR_CENTER_CELL {
                let (rx, ry) = g.arm_top_ink_offsets[i];
                (-rx * cell_fs, -ry * cell_fs)
            } else {
                let (rx, ry) = g.arm_bottom_ink_offsets[i - CROSSHAIR_CENTER_CELL - 1];
                (-rx * cell_fs, -ry * cell_fs)
            };
            let dx = shifted.val_cell_positions[i].0 - baseline.val_cell_positions[i].0;
            let dy = shifted.val_cell_positions[i].1 - baseline.val_cell_positions[i].1;
            assert!((dx - expect_dx).abs() < 0.001, "val[{i}].dx {dx} vs {expect_dx}");
            assert!((dy - expect_dy).abs() < 0.001, "val[{i}].dy {dy} vs {expect_dy}");
        }

        // Sat bar: same per-cell pattern using the left/right arrays.
        for i in 0..SAT_CELL_COUNT {
            let (expect_dx, expect_dy) = if i == CROSSHAIR_CENTER_CELL {
                (0.0, 0.0)
            } else if i < CROSSHAIR_CENTER_CELL {
                let (rx, ry) = g.arm_left_ink_offsets[i];
                (-rx * cell_fs, -ry * cell_fs)
            } else {
                let (rx, ry) = g.arm_right_ink_offsets[i - CROSSHAIR_CENTER_CELL - 1];
                (-rx * cell_fs, -ry * cell_fs)
            };
            let dx = shifted.sat_cell_positions[i].0 - baseline.sat_cell_positions[i].0;
            let dy = shifted.sat_cell_positions[i].1 - baseline.sat_cell_positions[i].1;
            assert!((dx - expect_dx).abs() < 0.001, "sat[{i}].dx {dx} vs {expect_dx}");
            assert!((dy - expect_dy).abs() < 0.001, "sat[{i}].dy {dy} vs {expect_dy}");
        }

        // Preview glyph anchor shifts by (-0.25*preview_size, +0.3*preview_size).
        let dx = shifted.preview_pos.0 - baseline.preview_pos.0;
        let dy = shifted.preview_pos.1 - baseline.preview_pos.1;
        assert!((dx - (-0.25 * preview_size)).abs() < 0.001);
        assert!((dy - (0.3 * preview_size)).abs() < 0.001);
    }

    #[test]
    fn layout_emits_24_hue_slots_on_circle() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        assert_eq!(layout.hue_slot_positions.len(), 24);
        // Ring radius is derived from the actual ring font size, not
        // the base font_size, since HUE_RING_FONT_SCALE > 1 makes
        // the ring glyphs larger than the base.
        let r_target = layout.outer_radius - layout.ring_font_size * 0.5;
        for (i, (px, py)) in layout.hue_slot_positions.iter().enumerate() {
            let dx = px - layout.center.0;
            let dy = py - layout.center.1;
            let r = (dx * dx + dy * dy).sqrt();
            assert!(
                (r - r_target).abs() < 0.5,
                "slot {i} radius {r} differs from {r_target}",
            );
        }
    }

    #[test]
    fn layout_first_hue_slot_is_at_top() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        let (px, py) = layout.hue_slot_positions[0];
        // Slot 0 sits at 12 o'clock — same x as center, smaller y.
        assert!((px - layout.center.0).abs() < 0.5);
        assert!(py < layout.center.1);
    }

    #[test]
    fn layout_sat_bar_monotonic_x_constant_y() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        assert_eq!(layout.sat_cell_positions.len(), SAT_CELL_COUNT);
        for w in layout.sat_cell_positions.windows(2) {
            assert!(w[1].0 > w[0].0, "sat cells must increase in x");
            assert!((w[0].1 - w[1].1).abs() < 0.1, "sat cells share y");
        }
    }

    #[test]
    fn layout_val_bar_monotonic_y_constant_x() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        assert_eq!(layout.val_cell_positions.len(), VAL_CELL_COUNT);
        for w in layout.val_cell_positions.windows(2) {
            assert!(w[1].1 > w[0].1, "val cells must increase in y");
            assert!((w[0].0 - w[1].0).abs() < 0.1, "val cells share x");
        }
    }

    #[test]
    fn hit_test_outside_backdrop_returns_outside() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        assert_eq!(hit_test_picker(&layout, -10.0, -10.0), PickerHit::Outside);
        assert_eq!(hit_test_picker(&layout, 5000.0, 5000.0), PickerHit::Outside);
    }

    /// A click at the exact wheel center — where the central ࿕ glyph
    /// lives — must resolve to `Commit`. This is the gesture that
    /// commits the current HSV (Contextual) or applies it to the
    /// document selection (Standalone). The center used to be
    /// inert (`Inside`); the new picker makes it the commit button.
    #[test]
    fn hit_test_hits_commit_on_center() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        let (cx, cy) = layout.center;
        assert_eq!(hit_test_picker(&layout, cx, cy), PickerHit::Commit);
    }

    /// A click just inside the backdrop but outside every
    /// interactive glyph must resolve to `DragAnchor` — the
    /// anywhere-you-can-grab-the-wheel zone. Picks a point far from
    /// the center (outside the commit radius), well outside any
    /// bar cell, and not inside the hue ring annulus. The backdrop
    /// corner is a reliable "nothing here but drag anchor" pick.
    #[test]
    fn hit_test_drag_anchor_when_inside_backdrop_not_on_glyph() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        let (bl, bt, _bw, _bh) = layout.backdrop;
        // 4 px inside the backdrop's top-left corner — far from the
        // ring (which is centered on the wheel), the chips (bottom),
        // the ࿕ (center), and the crosshair arms (central cross).
        let x = bl + 4.0;
        let y = bt + 4.0;
        assert_eq!(hit_test_picker(&layout, x, y), PickerHit::DragAnchor);
    }

    /// With `center_override` set, every position in the layout
    /// (hue slots, bar cells, chips, preview, backdrop) must
    /// translate by the offset between the override and the default
    /// window center. Regression guard for drag repositioning the
    /// wheel — if any one component forgot to read the override, the
    /// test catches it.
    #[test]
    fn center_override_translates_all_positions() {
        let screen_w = 1280.0;
        let screen_h = 720.0;
        let mut g = sample_geometry();
        let baseline = compute_color_picker_layout(&g, screen_w, screen_h);
        let offset = (200.0_f32, -80.0_f32);
        g.center_override = Some((
            screen_w * 0.5 + offset.0,
            screen_h * 0.5 + offset.1,
        ));
        let shifted = compute_color_picker_layout(&g, screen_w, screen_h);
        // Center itself.
        assert!((shifted.center.0 - baseline.center.0 - offset.0).abs() < 1e-3);
        assert!((shifted.center.1 - baseline.center.1 - offset.1).abs() < 1e-3);
        // Hue slot 0.
        assert!((shifted.hue_slot_positions[0].0 - baseline.hue_slot_positions[0].0 - offset.0).abs() < 1e-3);
        assert!((shifted.hue_slot_positions[0].1 - baseline.hue_slot_positions[0].1 - offset.1).abs() < 1e-3);
        // First sat cell.
        assert!((shifted.sat_cell_positions[0].0 - baseline.sat_cell_positions[0].0 - offset.0).abs() < 1e-3);
        // Backdrop top-left.
        let (bl0, bt0, _, _) = baseline.backdrop;
        let (bl1, bt1, _, _) = shifted.backdrop;
        assert!((bl1 - bl0 - offset.0).abs() < 1e-3);
        assert!((bt1 - bt0 - offset.1).abs() < 1e-3);
    }

    /// Hover-hit diffing: a layout computed with no hover should
    /// not differ structurally from one computed with a Hue hover
    /// — the builder applies the scale bump, but the pure layout
    /// positions don't shift. This locks in that `hovered_hit`
    /// stays out of `compute_color_picker_layout`'s output.
    #[test]
    fn hovered_hit_does_not_alter_layout_positions() {
        let mut g = sample_geometry();
        let baseline = compute_color_picker_layout(&g, 1280.0, 720.0);
        g.hovered_hit = Some(PickerHit::Hue(0));
        let hovered = compute_color_picker_layout(&g, 1280.0, 720.0);
        assert_eq!(
            baseline.hue_slot_positions[0], hovered.hue_slot_positions[0],
            "hovered_hit must not alter hue slot positions"
        );
        assert_eq!(baseline.backdrop, hovered.backdrop);
    }

    #[test]
    fn hit_test_hits_first_hue_slot() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        let (px, py) = layout.hue_slot_positions[0];
        assert_eq!(hit_test_picker(&layout, px, py), PickerHit::Hue(0));
    }

    #[test]
    fn hit_test_hits_off_center_sat_cell() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        // Pick a sat cell offset from the center so the val-bar branch
        // is not a candidate (the val bar shares the center column).
        let off = SAT_CELL_COUNT / 2 + 2;
        let (cx, cy) = layout.sat_cell_positions[off];
        assert_eq!(hit_test_picker(&layout, cx, cy), PickerHit::SatCell(off));
    }

    #[test]
    fn hue_slot_to_degrees_round_trip() {
        for slot in 0..HUE_SLOT_COUNT {
            let deg = hue_slot_to_degrees(slot);
            assert_eq!(degrees_to_hue_slot(deg), slot);
        }
    }

    #[test]
    fn sat_and_val_cell_value_endpoints() {
        assert!((sat_cell_to_value(0) - 0.0).abs() < 1e-6);
        assert!((sat_cell_to_value(SAT_CELL_COUNT - 1) - 1.0).abs() < 1e-6);
        // Val bar inverted: top cell = brightest.
        assert!((val_cell_to_value(0) - 1.0).abs() < 1e-6);
        assert!((val_cell_to_value(VAL_CELL_COUNT - 1) - 0.0).abs() < 1e-6);
    }

    /// Layout must fit inside the window even on small windows. The
    /// backdrop's vertical extent (top + height) must not exceed the
    /// screen height; same for horizontal. Regression guard for the
    /// "side defaulted to 280 even on a 200×200 window" overflow.
    #[test]
    fn layout_backdrop_fits_inside_small_window() {
        let g = sample_geometry();
        for &(w, h) in &[(320.0_f32, 240.0_f32), (400.0, 300.0), (200.0, 200.0)] {
            let layout = compute_color_picker_layout(&g, w, h);
            let (left, top, bw, bh) = layout.backdrop;
            assert!(left >= 0.0, "backdrop left underflows on {}x{}", w, h);
            assert!(top >= 0.0, "backdrop top underflows on {}x{}", w, h);
            assert!(left + bw <= w + 0.5,
                "backdrop right overflows on {}x{}: left={} bw={} w={}",
                w, h, left, bw, w);
            assert!(top + bh <= h + 0.5,
                "backdrop bottom overflows on {}x{}: top={} bh={} h={}",
                w, h, top, bh, h);
        }
    }

    /// Preview glyph must center on the geometric wheel center given
    /// the layout-emitted `preview_size`. The ࿕ svasti is a Tibetan
    /// ideograph whose ink sits centered in the em box, so the
    /// canonical half-size offset `(0.5, 0.5)` is the right anchor
    /// on both axes — any future preview glyph with a skewed visible
    /// center needs a commensurate tweak here. Regression guard for
    /// the "preview was anchored off-center" bug.
    #[test]
    fn layout_preview_centered_on_wheel_center() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        let (px, py) = layout.preview_pos;
        let cx = px + layout.preview_size * 0.5;
        let cy = py + layout.preview_size * 0.5;
        // The preview's visible center should be within ~1 px of the
        // wheel center on each axis.
        assert!((cx - layout.center.0).abs() < 1.0,
            "preview x center {} differs from wheel center {}", cx, layout.center.0);
        assert!((cy - layout.center.1).abs() < 1.0,
            "preview y center {} differs from wheel center {}", cy, layout.center.1);
    }

    /// Regression guard for the "࿕ overlaps the first arm letter"
    /// bug. With `preview_size_scale = 3.0` and the sacred-script
    /// `cell_factor ≈ 1.0`, the nearest arm cell (index
    /// `CROSSHAIR_CENTER_CELL ± 1`) would sit at `cell_advance` from
    /// centre while the preview reaches `preview_size / 2 =
    /// 1.5 × font_size` — the preview ink ended up covering that
    /// cell. `compute_color_picker_layout` now floors `cell_factor`
    /// at `preview_size_scale * 0.5 + bar_to_preview_padding_scale`,
    /// so cell[9] / cell[11] (and their sat-bar twins) always sit at
    /// least `preview_size / 2 + padding_px` from centre.
    #[test]
    fn layout_keeps_preview_clear_of_adjacent_arm_cells() {
        let padding_scale = load_spec().geometry.bar_to_preview_padding_scale;
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        let preview_radius = layout.preview_size * 0.5;
        let min_clearance = preview_radius + layout.font_size * padding_scale;

        let center = layout.center;
        let neighbours = [
            ("val[CENTER - 1]", layout.val_cell_positions[CROSSHAIR_CENTER_CELL - 1]),
            ("val[CENTER + 1]", layout.val_cell_positions[CROSSHAIR_CENTER_CELL + 1]),
            ("sat[CENTER - 1]", layout.sat_cell_positions[CROSSHAIR_CENTER_CELL - 1]),
            ("sat[CENTER + 1]", layout.sat_cell_positions[CROSSHAIR_CENTER_CELL + 1]),
        ];
        // 0.5 px slack for rounding / ink-offset drift.
        let slack = 0.5;
        for (label, (px, py)) in neighbours {
            let dx = px - center.0;
            let dy = py - center.1;
            let dist = (dx * dx + dy * dy).sqrt();
            assert!(
                dist + slack >= min_clearance,
                "{label} at ({px:.1}, {py:.1}) is {dist:.1} px from centre — below preview clearance {min_clearance:.1}",
            );
        }
    }

    /// Hue wrap: degrees_to_hue_slot must wrap correctly across the
    /// 0/360 boundary in both directions, and slots near the
    /// boundary must round to slot 0 (not slot 24).
    #[test]
    fn degrees_to_hue_slot_wraps_at_boundary() {
        assert_eq!(degrees_to_hue_slot(0.0), 0);
        assert_eq!(degrees_to_hue_slot(360.0), 0);
        assert_eq!(degrees_to_hue_slot(720.0), 0);
        assert_eq!(degrees_to_hue_slot(-15.0), 23);
        assert_eq!(degrees_to_hue_slot(-360.0), 0);
        // 357° rounds to slot 0 (since 24 % 24 = 0).
        assert_eq!(degrees_to_hue_slot(357.0), 0);
        // 352° rounds to slot 23.
        assert_eq!(degrees_to_hue_slot(352.0), 23);
    }

    /// Quantization: every input degree in `[0, 360)` must fall into
    /// the slot whose center is closest. Walk the range in 1° steps
    /// and check that no input is more than 7.5° (half a slot) from
    /// its resolved slot's canonical degree. Guards against a future
    /// refactor that silently shifts the quantization phase or
    /// introduces floor-vs-round inconsistencies.
    #[test]
    fn degrees_to_hue_slot_quantizes_to_nearest() {
        for d in 0..360 {
            let deg = d as f32;
            let slot = degrees_to_hue_slot(deg);
            let canonical = hue_slot_to_degrees(slot);
            // Circular distance from `deg` to `canonical`, taking the
            // shorter arc of the two directions.
            let diff = ((deg - canonical).rem_euclid(360.0)).min(
                (canonical - deg).rem_euclid(360.0),
            );
            assert!(diff <= 7.5 + 1e-4,
                "deg {} → slot {} (canonical {}°) distance {} > 7.5",
                deg, slot, canonical, diff);
        }
    }

    /// Boundary rounding: 7.4° rounds to slot 0 (closer), 7.6°
    /// rounds to slot 1 (closer). Explicit test guarding the
    /// round-half-to-even-or-away edge case.
    #[test]
    fn degrees_to_hue_slot_mid_slot_rounding() {
        assert_eq!(degrees_to_hue_slot(7.4), 0);
        assert_eq!(degrees_to_hue_slot(7.6), 1);
        assert_eq!(degrees_to_hue_slot(22.4), 1);
        assert_eq!(degrees_to_hue_slot(22.6), 2);
    }

    /// Hue ring slots must not overlap at the new 1.5× font scale.
    /// On a full-size window, consecutive slot centers should be at
    /// least `0.9 * max_ring_advance` apart by straight-line (chord)
    /// distance — anything less means the ring radius got clamped too
    /// tight and glyphs will collide visually. Chord distance (not
    /// arc) because that's what matters for glyph collision: the
    /// glyphs sit at the slot centers, and two glyphs collide when
    /// their chord distance falls below their shaped widths.
    #[test]
    fn hue_ring_slots_do_not_overlap_at_new_font_scale() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        for i in 0..HUE_SLOT_COUNT {
            let j = (i + 1) % HUE_SLOT_COUNT;
            let (px, py) = layout.hue_slot_positions[i];
            let (qx, qy) = layout.hue_slot_positions[j];
            let dx = qx - px;
            let dy = qy - py;
            let dist = (dx * dx + dy * dy).sqrt();
            assert!(
                dist >= g.max_ring_advance * 0.9,
                "adjacent hue slots {i} and {j} only {dist} apart, \
                expected >= {}",
                g.max_ring_advance * 0.9,
            );
        }
    }
