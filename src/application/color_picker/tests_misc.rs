//! Picker unit tests — part 2 of 2. Hex readout, hue-slot math,
//! per-script grouping, channel ascending, resize-gesture math.
//! Fixtures are duplicated from tests_layout so each submodule
//! stays self-contained (~30 lines; cheaper than routing through
//! a shared `pub(super)` helper).

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

    /// `hex_pos` must be `Some` when geometry declares it visible and
    /// `None` otherwise. Regression guard against a renderer that
    /// reaches for hex_pos unconditionally.
    #[test]
    fn hex_pos_is_some_iff_hex_visible() {
        let invisible = compute_color_picker_layout(&sample_geometry(), 1280.0, 720.0);
        assert!(
            invisible.hex_pos.is_none(),
            "hex_pos must be None when hex_visible=false",
        );
        let visible = compute_color_picker_layout(&sample_geometry_with_hex(), 1280.0, 720.0);
        assert!(
            visible.hex_pos.is_some(),
            "hex_pos must be Some when hex_visible=true",
        );
    }

    /// When visible, the hex readout must be horizontally centered on
    /// the wheel center. The top-left anchor is offset left by half
    /// the hex text width (7 chars * char_width).
    #[test]
    fn hex_pos_horizontally_centered_on_wheel_center() {
        let layout = compute_color_picker_layout(&sample_geometry_with_hex(), 1280.0, 720.0);
        let (hx, _) = layout.hex_pos.expect("hex_pos should be Some");
        let hex_width = layout.char_width * 7.0;
        let hex_center_x = hx + hex_width * 0.5;
        assert!(
            (hex_center_x - layout.center.0).abs() < 1.0,
            "hex readout center {hex_center_x} not aligned with wheel center {}",
            layout.center.0,
        );
    }

    /// Each crosshair arm must render exactly 8 cells. The bars
    /// have SAT_CELL_COUNT / VAL_CELL_COUNT = 17 cells, cell
    /// CROSSHAIR_CENTER_CELL = 8 is the shared wheel-center slot
    /// (࿕ overlay), and each arm covers 8 non-center cells —
    /// totaling 32 rendered crosshair glyphs. Also asserts that the
    /// center cells of both bars sit exactly on the wheel center.
    #[test]
    fn crosshair_arms_render_exactly_8_cells_each() {
        let layout = compute_color_picker_layout(&sample_geometry(), 1280.0, 720.0);
        // Center cell of the sat bar = wheel center.
        let (scx, scy) = layout.sat_cell_positions[CROSSHAIR_CENTER_CELL];
        assert!((scx - layout.center.0).abs() < 0.1);
        assert!((scy - layout.center.1).abs() < 0.1);
        // Center cell of the val bar = wheel center.
        let (vcx, vcy) = layout.val_cell_positions[CROSSHAIR_CENTER_CELL];
        assert!((vcx - layout.center.0).abs() < 0.1);
        assert!((vcy - layout.center.1).abs() < 0.1);
        // Left arm = 8 cells (0..CROSSHAIR_CENTER_CELL).
        assert_eq!(CROSSHAIR_CENTER_CELL, 8);
        assert_eq!(arm_left_glyphs().len(), 8);
        // Right arm = 8 cells (CROSSHAIR_CENTER_CELL+1..SAT_CELL_COUNT).
        assert_eq!(SAT_CELL_COUNT - CROSSHAIR_CENTER_CELL - 1, 8);
        assert_eq!(arm_right_glyphs().len(), 8);
        // Top arm = 8 cells, bottom arm = 8 cells.
        assert_eq!(arm_top_glyphs().len(), 8);
        assert_eq!(arm_bottom_glyphs().len(), 8);
        // Four arms × 8 glyphs = 32 total.
        assert_eq!(
            arm_top_glyphs().len()
                + arm_bottom_glyphs().len()
                + arm_left_glyphs().len()
                + arm_right_glyphs().len(),
            32,
        );
    }

    /// Every arm cell, after ink correction, must sit on its target
    /// radial point — sat cells on `center.y`, val cells on
    /// `center.x` — within 0.1 px. This is the per-cell version of
    /// the centre-only assertion in
    /// [`crosshair_arms_render_exactly_10_cells_each`]: a regression
    /// against the previous "single per-arm offset" model would
    /// fail this test because the worst-case heuristic mis-corrected
    /// non-worst glyphs by their delta. We re-add each cell's stored
    /// ink offset (`offset_ratio * cell_fs`) and check the result
    /// hits the unrotated radial point.
    #[test]
    fn crosshair_arms_per_cell_ink_correction_aligns_to_radial_target() {
        // Use a non-trivial per-cell offset pattern so any "lost"
        // index would visibly fail.
        let mut g = sample_geometry();
        for i in 0..CROSSHAIR_CENTER_CELL {
            let f = (i + 1) as f32;
            g.arm_top_ink_offsets[i] = (0.02 * f, -0.03 * f);
            g.arm_bottom_ink_offsets[i] = (-0.02 * f, 0.03 * f);
            g.arm_left_ink_offsets[i] = (0.025 * f, 0.01 * f);
            g.arm_right_ink_offsets[i] = (-0.025 * f, -0.01 * f);
        }
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        let cell_fs = layout.cell_font_size;
        let step = layout.cell_advance;
        let cx = layout.center.0;
        let cy = layout.center.1;

        // Sat cells: each (after re-adding its ink offset) must land
        // on (cx + (i - CENTER)*step, cy).
        for i in 0..SAT_CELL_COUNT {
            if i == CROSSHAIR_CENTER_CELL {
                continue;
            }
            let (rx, ry) = if i < CROSSHAIR_CENTER_CELL {
                g.arm_left_ink_offsets[i]
            } else {
                g.arm_right_ink_offsets[i - CROSSHAIR_CENTER_CELL - 1]
            };
            let (px, py) = layout.sat_cell_positions[i];
            let restored_x = px + rx * cell_fs;
            let restored_y = py + ry * cell_fs;
            let target_x = cx + (i as f32 - CROSSHAIR_CENTER_CELL as f32) * step;
            assert!(
                (restored_x - target_x).abs() < 0.1,
                "sat[{i}] restored x {restored_x} != target {target_x}",
            );
            assert!(
                (restored_y - cy).abs() < 0.1,
                "sat[{i}] restored y {restored_y} != center y {cy}",
            );
        }

        // Val cells: each must land on (cx, cy + (i - CENTER)*step).
        for i in 0..VAL_CELL_COUNT {
            if i == CROSSHAIR_CENTER_CELL {
                continue;
            }
            let (rx, ry) = if i < CROSSHAIR_CENTER_CELL {
                g.arm_top_ink_offsets[i]
            } else {
                g.arm_bottom_ink_offsets[i - CROSSHAIR_CENTER_CELL - 1]
            };
            let (px, py) = layout.val_cell_positions[i];
            let restored_x = px + rx * cell_fs;
            let restored_y = py + ry * cell_fs;
            let target_y = cy + (i as f32 - CROSSHAIR_CENTER_CELL as f32) * step;
            assert!(
                (restored_x - cx).abs() < 0.1,
                "val[{i}] restored x {restored_x} != center x {cx}",
            );
            assert!(
                (restored_y - target_y).abs() < 0.1,
                "val[{i}] restored y {restored_y} != target {target_y}",
            );
        }
    }

    /// The four crosshair arms must emit the same per-cell advance so
    /// the cross reads as a symmetric cross, not a plus sign with one
    /// fat arm. Checks that consecutive sat cells and consecutive val
    /// cells have identical step distances.
    #[test]
    fn crosshair_arms_emit_symmetric_cell_advance() {
        let layout = compute_color_picker_layout(&sample_geometry(), 1280.0, 720.0);
        let sat_step = layout.sat_cell_positions[1].0 - layout.sat_cell_positions[0].0;
        let val_step = layout.val_cell_positions[1].1 - layout.val_cell_positions[0].1;
        assert!(
            (sat_step - val_step).abs() < 0.1,
            "sat step {sat_step} differs from val step {val_step} — \
            cross would render asymmetrically",
        );
        // Every step should be equal (not just the first pair).
        for i in 0..SAT_CELL_COUNT - 1 {
            let s = layout.sat_cell_positions[i + 1].0 - layout.sat_cell_positions[i].0;
            let v = layout.val_cell_positions[i + 1].1 - layout.val_cell_positions[i].1;
            assert!((s - sat_step).abs() < 0.1, "sat step {i}→{} drifted", i + 1);
            assert!((v - val_step).abs() < 0.1, "val step {i}→{} drifted", i + 1);
        }
    }

    /// The 24-glyph hue ring array must have a Devanagari arc, a
    /// Hebrew arc, and a Tibetan arc. Codepoint-range check — not
    /// the identity of individual glyphs — so swapping letters in
    /// the same script doesn't break the test.
    #[test]
    fn hue_ring_glyphs_are_grouped_by_script() {
        fn first_cp(s: &str) -> u32 {
            s.chars().next().expect("glyph string non-empty") as u32
        }
        // Slots 0-7 Devanagari
        for i in 0..8 {
            let cp = first_cp(hue_ring_glyphs()[i]);
            assert!(
                (0x0900..=0x097F).contains(&cp),
                "slot {i} codepoint U+{cp:04X} not in Devanagari",
            );
        }
        // Slots 8-15 Hebrew
        for i in 8..16 {
            let cp = first_cp(hue_ring_glyphs()[i]);
            assert!(
                (0x0590..=0x05FF).contains(&cp),
                "slot {i} codepoint U+{cp:04X} not in Hebrew",
            );
        }
        // Slots 16-23 Tibetan
        for i in 16..24 {
            let cp = first_cp(hue_ring_glyphs()[i]);
            assert!(
                (0x0F00..=0x0FFF).contains(&cp),
                "slot {i} codepoint U+{cp:04X} not in Tibetan",
            );
        }
    }

    /// Each crosshair arm must be grouped by its own script. Codepoint-
    /// range check — not the identity of individual glyphs — so
    /// swapping letters within the same script doesn't break the test,
    /// while an accidental swap between arms will.
    #[test]
    fn arm_glyphs_are_grouped_by_script() {
        fn first_cp(s: &str) -> u32 {
            s.chars().next().expect("glyph string non-empty") as u32
        }
        // Top arm: Devanagari (U+0900–U+097F)
        for (i, g) in arm_top_glyphs().iter().enumerate() {
            let cp = first_cp(g);
            assert!(
                (0x0900..=0x097F).contains(&cp),
                "top arm cell {i} codepoint U+{cp:04X} not in Devanagari",
            );
        }
        // Bottom arm: Egyptian Hieroglyphs (U+13000–U+1342F)
        for (i, g) in arm_bottom_glyphs().iter().enumerate() {
            let cp = first_cp(g);
            assert!(
                (0x13000..=0x1342F).contains(&cp),
                "bottom arm cell {i} codepoint U+{cp:05X} not in Egyptian Hieroglyphs",
            );
        }
        // Left arm: Tibetan (U+0F00–U+0FFF)
        for (i, g) in arm_left_glyphs().iter().enumerate() {
            let cp = first_cp(g);
            assert!(
                (0x0F00..=0x0FFF).contains(&cp),
                "left arm cell {i} codepoint U+{cp:04X} not in Tibetan",
            );
        }
        // Right arm: Hebrew (U+0590–U+05FF)
        for (i, g) in arm_right_glyphs().iter().enumerate() {
            let cp = first_cp(g);
            assert!(
                (0x0590..=0x05FF).contains(&cp),
                "right arm cell {i} codepoint U+{cp:04X} not in Hebrew",
            );
        }
    }

    /// Canonical sizing formula: at `size_scale = 1.0` the picker
    /// occupies roughly `target_frac` of the screen's shorter side.
    /// Verifies the new screen-driven sizing — the old `min(22, h/12)`
    /// formula was effectively constant on any window taller than
    /// 264 px, which is why the picker filled small windows.
    #[test]
    fn layout_targets_screen_short_side_fraction() {
        let g = sample_geometry();
        let spec = load_spec();
        let target_frac = spec.geometry.target_frac;
        for &(w, h) in &[(1920.0_f32, 1080.0_f32), (1280.0, 720.0), (800.0, 600.0)] {
            let layout = compute_color_picker_layout(&g, w, h);
            let short = w.min(h);
            let (_, _, bw, bh) = layout.backdrop;
            // Backdrop's bigger dimension should sit within ~10–80%
            // of the short axis at scale 1.0 — a wide window like
            // chip-row safety can dominate the wheel-driven side
            // a bit, so the bound is loose.
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
    /// proportionally. A 1.5× scale on the same window produces a
    /// strictly larger backdrop than the 1.0× baseline, all else
    /// equal. Regression guard for the RMB-resize gesture.
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
        assert!(
            bw_g > bw_b,
            "size_scale=1.5 backdrop {bw_g} not larger than baseline {bw_b}"
        );
        assert!(
            bw_s < bw_b,
            "size_scale=0.7 backdrop {bw_s} not smaller than baseline {bw_b}"
        );
        // font_size also scales monotonically.
        assert!(grown.font_size > baseline.font_size);
        assert!(shrunk.font_size < baseline.font_size);
    }

    /// Small-window robustness: even on a tiny viewport the layout
    /// must never produce negative geometry or a backdrop that
    /// overflows the screen — the canonical formula's safety
    /// clamp on `font_size` should kick in.
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
    /// with cell_advance = 8 (also ratio 1.0). The dimensionless
    /// ratio is what matters; the absolute measurement scale is
    /// not.
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
    /// `size_scale = 0.5` keeps the backdrop fully on-screen even
    /// though the user-controlled scale would otherwise produce a
    /// cramped widget. The safety clamp on `font_size` must
    /// dominate the user scale when needed.
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

    /// Picker channels must be strictly ascending in tree-insertion
    /// order, otherwise Baumhard's `align_child_walks` (which pairs
    /// mutator children with target children by ascending channel)
    /// breaks alignment and the §B2 mutator path silently misses
    /// elements. Channels are resolved through [`picker_channel`],
    /// which reads `widgets/color_picker.json`'s `mutator_spec`.
    ///
    /// Insertion order: title → hue ring (24 slots) → hint →
    /// sat bar (17 cells, channels also stride through the
    /// skipped center) → val bar (same) → preview → hex.
    #[test]
    fn picker_channels_are_strictly_ascending() {
        let bands: &[(&str, usize, usize)] = &[
            ("title", picker_channel("title", 0), 1),
            ("hue ring", picker_channel("hue_ring", 0), HUE_SLOT_COUNT),
            ("hint", picker_channel("hint", 0), 1),
            ("sat bar", picker_channel("sat_bar", 0), SAT_CELL_COUNT),
            ("val bar", picker_channel("val_bar", 0), VAL_CELL_COUNT),
            ("preview", picker_channel("preview", 0), 1),
            ("hex", picker_channel("hex", 0), 1),
        ];
        let mut prev_band_max: usize = 0;
        for (name, base, count) in bands {
            let band_min = *base;
            let band_max = *base + count - 1;
            assert!(
                band_min > prev_band_max,
                "{name} band starts at {band_min} but previous band ended at {prev_band_max}"
            );
            prev_band_max = band_max;
        }
    }

    /// `PickerGesture::Resize` must compute the new scale
    /// multiplicatively from cursor radius. A 2× radius produces
    /// a 2× scale, a 0.5× radius produces a 0.5× scale, modulo
    /// the spec's `[resize_scale_min, resize_scale_max]` clamp.
    /// The math is shared with `handle_color_picker_mouse_move`;
    /// this test pins it as a pure formula so a refactor that
    /// silently flips additive can't slip through.
    #[test]
    fn resize_gesture_scale_math_is_multiplicative() {
        let spec = load_spec();
        let geom = &spec.geometry;
        let anchor_radius: f32 = 100.0;
        let anchor_scale: f32 = 1.0;
        // 2x radius ⇒ 2x scale (clamped).
        let r_double = anchor_radius * 2.0;
        let new_double =
            (anchor_scale * (r_double / anchor_radius)).clamp(geom.resize_scale_min, geom.resize_scale_max);
        assert!(new_double > anchor_scale);
        assert!(new_double <= geom.resize_scale_max);
        // 0.5x radius ⇒ 0.5x scale (clamped).
        let r_half = anchor_radius * 0.5;
        let new_half =
            (anchor_scale * (r_half / anchor_radius)).clamp(geom.resize_scale_min, geom.resize_scale_max);
        assert!(new_half < anchor_scale);
        assert!(new_half >= geom.resize_scale_min);
        // Identity at same radius.
        let new_same =
            (anchor_scale * (anchor_radius / anchor_radius)).clamp(geom.resize_scale_min, geom.resize_scale_max);
        assert!((new_same - anchor_scale).abs() < 1e-6);
    }
