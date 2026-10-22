// SPDX-License-Identifier: MPL-2.0

use super::*;

// ====================================================================
// Console overlay layout
// ====================================================================

fn empty_console_geometry() -> ConsoleOverlayGeometry {
    ConsoleOverlayGeometry {
        input: String::new(),
        cursor_grapheme: 0,
        scrollback: Vec::new(),
        completions: Vec::new(),
        selected_completion: None,
        font_family: String::new(),
        font_size: 16.0,
    }
}

fn sample_console_geometry() -> ConsoleOverlayGeometry {
    ConsoleOverlayGeometry {
        input: "anchor set from t".to_string(),
        cursor_grapheme: 17,
        scrollback: vec![
            ConsoleOverlayLine {
                text: "> help".to_string(),
                kind: ConsoleOverlayLineKind::Input,
                font_family: None,
            },
            ConsoleOverlayLine {
                text: "commands:".to_string(),
                kind: ConsoleOverlayLineKind::Output,
                font_family: None,
            },
        ],
        completions: vec![ConsoleOverlayCompletion {
            text: "top".to_string(),
            hint: None,
            font_family: None,
        }],
        selected_completion: Some(0),
        font_family: String::new(),
        font_size: 16.0,
    }
}

#[test]
fn test_console_backdrop_matches_border_bounds_exactly() {
    let geometry = sample_console_geometry();
    let layout = compute_console_frame_layout(&geometry, 1920.0, 1080.0);
    let (bd_left, bd_top, bd_w, bd_h) = layout.backdrop_rect();
    assert_eq!(bd_left, layout.left);
    assert_eq!(bd_top, layout.top);
    assert_eq!(bd_w, layout.frame_width);
    assert_eq!(bd_h, layout.frame_height + layout.font_size);
}

#[test]
fn test_console_backdrop_has_no_horizontal_overhang() {
    let geometry = sample_console_geometry();
    let layout = compute_console_frame_layout(&geometry, 1920.0, 1080.0);
    let (bd_left, _, bd_w, _) = layout.backdrop_rect();
    let bd_right = bd_left + bd_w;
    let border_right = layout.left + layout.frame_width;
    assert!(bd_right <= border_right + 0.001);
    assert!(bd_left >= layout.left - 0.001);
}

#[test]
fn test_console_frame_is_bottom_anchored() {
    let geometry = sample_console_geometry();
    let layout = compute_console_frame_layout(&geometry, 1920.0, 1080.0);
    // Bottom border glyph row extends `font_size` below frame_height.
    // Its bottom edge should sit within `inner_padding` of the
    // screen bottom.
    let frame_bottom = layout.top + layout.frame_height + layout.font_size;
    let gap = 1080.0 - frame_bottom;
    assert!(
        gap <= layout.inner_padding + 0.5 && gap >= 0.0,
        "frame not bottom-anchored: gap={gap}"
    );
}

#[test]
fn test_console_frame_height_linear_in_scrollback_rows() {
    let g_empty = empty_console_geometry();
    let mut g_one = empty_console_geometry();
    g_one.scrollback.push(ConsoleOverlayLine {
        text: "one".into(),
        kind: ConsoleOverlayLineKind::Output,
        font_family: None,
    });
    let mut g_two = g_one.clone();
    g_two.scrollback.push(ConsoleOverlayLine {
        text: "two".into(),
        kind: ConsoleOverlayLineKind::Output,
        font_family: None,
    });
    let h0 = compute_console_frame_layout(&g_empty, 1920.0, 1080.0).frame_height;
    let h1 = compute_console_frame_layout(&g_one, 1920.0, 1080.0).frame_height;
    let h2 = compute_console_frame_layout(&g_two, 1920.0, 1080.0).frame_height;
    let delta1 = h1 - h0;
    let delta2 = h2 - h1;
    assert!((delta1 - delta2).abs() < 0.01);
}

#[test]
fn test_console_scrollback_clamped_to_max_rows() {
    let mut geometry = empty_console_geometry();
    for i in 0..100 {
        geometry.scrollback.push(ConsoleOverlayLine {
            text: format!("line {i}"),
            kind: ConsoleOverlayLineKind::Output,
            font_family: None,
        });
    }
    let layout = compute_console_frame_layout(&geometry, 1920.0, 1080.0);
    assert_eq!(layout.scrollback_rows, MAX_CONSOLE_SCROLLBACK_ROWS);
}

#[test]
fn test_console_completions_clamped_to_max_rows() {
    let mut geometry = empty_console_geometry();
    for i in 0..100 {
        geometry.completions.push(ConsoleOverlayCompletion {
            text: format!("cmd_{i}"),
            hint: None,
            font_family: None,
        });
    }
    let layout = compute_console_frame_layout(&geometry, 1920.0, 1080.0);
    assert_eq!(layout.completion_rows, MAX_CONSOLE_COMPLETION_ROWS);
}

#[test]
fn test_console_frame_is_full_window_width() {
    // The console is a bottom-anchored full-width strip with a
    // small horizontal margin on each side. Frame width + 2 ×
    // margin should sum to roughly the screen width.
    let layout = compute_console_frame_layout(&empty_console_geometry(), 1920.0, 1080.0);
    let total = layout.left * 2.0 + layout.frame_width;
    assert!((total - 1920.0).abs() < 1.0, "frame doesn't span full width");
}

#[test]
fn test_console_frame_width_independent_of_scrollback_len() {
    // With the full-width layout, a long scrollback line cannot
    // push the frame wider — it's clipped by the content area.
    let short = compute_console_frame_layout(&empty_console_geometry(), 1920.0, 1080.0).frame_width;
    let mut huge = empty_console_geometry();
    huge.scrollback.push(ConsoleOverlayLine {
        text: "x".repeat(500),
        kind: ConsoleOverlayLineKind::Output,
        font_family: None,
    });
    let long = compute_console_frame_layout(&huge, 1920.0, 1080.0).frame_width;
    assert_eq!(short, long);
}

#[test]
fn test_console_frame_width_stable_for_wide_char_scrollback() {
    // Backdrop-vs-border alignment with a wide-char line — the
    // content is truncated by baumhard's `truncate_to_display_width`
    // so it can't blow past the right border, and the frame
    // itself is still the full window width.
    let mut g = empty_console_geometry();
    g.scrollback.push(ConsoleOverlayLine {
        text: "日本語".repeat(200),
        kind: ConsoleOverlayLineKind::Output,
        font_family: None,
    });
    let layout = compute_console_frame_layout(&g, 1920.0, 1080.0);
    let (bd_left, _, bd_w, _) = layout.backdrop_rect();
    assert_eq!(bd_left, layout.left);
    assert_eq!(bd_w, layout.frame_width);
}

// -----------------------------------------------------------------
// Console border source-string tests
//
// The border draw uses baumhard's `BorderGlyphSet::box_drawing_rounded`
// via `build_console_border_strings(cols, rows)`.
// -----------------------------------------------------------------

#[test]
fn test_console_border_uses_rounded_corners() {
    let (top, bottom, _, _) = build_console_border_strings(10, 4);
    let top_chars: Vec<char> = top.chars().collect();
    let bot_chars: Vec<char> = bottom.chars().collect();
    assert_eq!(top_chars[0], '\u{256D}'); // ╭
    assert_eq!(*top_chars.last().unwrap(), '\u{256E}'); // ╮
    assert_eq!(bot_chars[0], '\u{2570}'); // ╰
    assert_eq!(*bot_chars.last().unwrap(), '\u{256F}'); // ╯
                                                        // Middle chars of the top border are `─`.
    for c in &top_chars[1..top_chars.len() - 1] {
        assert_eq!(*c, '\u{2500}');
    }
}

#[test]
fn test_console_border_top_row_length_matches_cols() {
    // `cols` = total border length including both corners.
    let (top, bottom, _, _) = build_console_border_strings(20, 4);
    assert_eq!(top.chars().count(), 20);
    assert_eq!(bottom.chars().count(), 20);
}

#[test]
fn test_console_border_sides_one_char_per_line() {
    let (_, _, left, right) = build_console_border_strings(10, 5);
    // One `│` per line, newline-separated; 5 lines total.
    assert_eq!(left.lines().count(), 5);
    assert_eq!(right.lines().count(), 5);
    for line in left.lines() {
        assert_eq!(line.chars().count(), 1);
        assert_eq!(line.chars().next().unwrap(), '\u{2502}');
    }
}

#[test]
fn test_console_border_scales_with_cols_and_rows() {
    let (top_narrow, _, left_short, _) = build_console_border_strings(10, 3);
    let (top_wide, _, left_tall, _) = build_console_border_strings(40, 10);
    assert!(top_wide.chars().count() > top_narrow.chars().count());
    assert!(left_tall.lines().count() > left_short.lines().count());
}

#[test]
fn test_console_prompt_y_sits_below_scrollback_and_completions() {
    // Regression guard for the overlap bug where `prompt_y`
    // floated at `frame_height - inner_padding - font_size`,
    // landing ~0.6 · font_size *above* the last completion row
    // instead of below it.
    let mut g = empty_console_geometry();
    g.scrollback = vec![
        ConsoleOverlayLine {
            text: "one".into(),
            kind: ConsoleOverlayLineKind::Output,
            font_family: None,
        },
        ConsoleOverlayLine {
            text: "two".into(),
            kind: ConsoleOverlayLineKind::Output,
            font_family: None,
        },
    ];
    g.completions = vec![ConsoleOverlayCompletion {
        text: "help".into(),
        hint: None,
        font_family: None,
    }];
    g.selected_completion = Some(0);
    let layout = compute_console_frame_layout(&g, 1920.0, 1080.0);

    let content_top = layout.top + layout.font_size + layout.inner_padding;
    let last_completion_end =
        content_top + layout.row_height * (layout.scrollback_rows + layout.completion_rows) as f32;
    assert!(
        layout.prompt_y() >= last_completion_end - 0.01,
        "prompt_y {} overlaps last completion row ending at {}",
        layout.prompt_y(),
        last_completion_end
    );
}

#[test]
fn test_console_prompt_y_fits_inside_frame() {
    // The prompt row plus its padded budget must stay inside
    // `frame_height`; otherwise it renders outside the border.
    let geometry = sample_console_geometry();
    let layout = compute_console_frame_layout(&geometry, 1920.0, 1080.0);
    let prompt_bottom = layout.prompt_y() + layout.font_size * 1.4;
    let frame_bottom = layout.top + layout.frame_height;
    assert!(
        prompt_bottom <= frame_bottom + 0.01,
        "prompt bottom {} overruns frame bottom {}",
        prompt_bottom,
        frame_bottom
    );
}

#[test]
fn test_console_border_fills_full_frame_cols() {
    // The renderer picks `cols = floor(frame_width / char_width)`
    // and calls `build_console_border_strings(cols, rows)`, so
    // the top string always has exactly `cols` glyphs — one per
    // char-width cell.
    let geometry = sample_console_geometry();
    let layout = compute_console_frame_layout(&geometry, 1920.0, 1080.0);
    let cols = (layout.frame_width / layout.char_width).floor() as usize;
    let (top, _, _, _) = build_console_border_strings(cols, 4);
    assert_eq!(top.chars().count(), cols);
}

#[test]
fn test_console_frame_layout_scales_with_font_size() {
    let mut g = empty_console_geometry();
    g.font_size = 8.0;
    let small = compute_console_frame_layout(&g, 1920.0, 1080.0);
    g.font_size = 32.0;
    let large = compute_console_frame_layout(&g, 1920.0, 1080.0);
    assert!(large.font_size > small.font_size);
    assert!(large.row_height > small.row_height);
    assert!(large.frame_height > small.frame_height);
}

/// Console round-trip: applying the mutator to a tree built
/// at state A leaves it byte-identical (per variable field) to
/// a fresh `build_console_overlay_tree(B)`. Pins the §B2
/// in-place update path for the keystroke hot path: the
/// dispatcher in `rebuild_console_overlay_buffers` takes this
/// branch on every input change frame.
#[test]
fn console_mutator_round_trips_to_fresh_build() {
    use baumhard::core::primitives::Applicable;
    use baumhard::gfx_structs::tree::BranchChannel;
    baumhard::font::fonts::init();

    let mut g_a = sample_console_geometry();
    g_a.input = "anchor".into();
    g_a.cursor_grapheme = 6;
    let layout_a = compute_console_frame_layout(&g_a, 1280.0, 720.0);

    let mut g_b = sample_console_geometry();
    g_b.input = "anchor set".into();
    g_b.cursor_grapheme = 10;
    let layout_b = compute_console_frame_layout(&g_b, 1280.0, 720.0);

    // Same scrollback_rows / completion_rows means the
    // structural signature matches and the mutator is sound.
    assert_eq!(layout_a.scrollback_rows, layout_b.scrollback_rows);
    assert_eq!(layout_a.completion_rows, layout_b.completion_rows);

    let mut tree = {
        let mut fs = baumhard::font::fonts::acquire_font_system_write("renderer::tests (overlay tree)");
        build_console_overlay_tree(&g_a, &layout_a, &mut fs)
    };
    let mutator = {
        let mut fs = baumhard::font::fonts::acquire_font_system_write("renderer::tests (overlay mutator)");
        build_console_overlay_mutator(&g_b, &layout_b, &mut fs)
    };
    mutator.apply_to(&mut tree);

    let expected = {
        let mut fs =
            baumhard::font::fonts::acquire_font_system_write("renderer::tests (overlay areas expected)");
        console_overlay_areas(&g_b, &layout_b, &mut fs)
    };

    let mut got: Vec<(usize, GlyphArea)> = Vec::new();
    for descendant_id in tree.root().descendants(&tree.arena) {
        let node = tree.arena.get(descendant_id).expect("arena node");
        let element = node.get();
        if let Some(area) = element.glyph_area() {
            got.push((element.channel(), area.clone()));
        }
    }

    assert_eq!(got.len(), expected.len(), "post-mutation element count");
    for ((c_got, a_got), (c_exp, a_exp)) in got.iter().zip(expected.iter()) {
        assert_eq!(c_got, c_exp, "channel mismatch");
        assert_eq!(a_got.text, a_exp.text, "text on ch {c_got}");
        assert_eq!(a_got.position, a_exp.position, "position on ch {c_got}");
        assert_eq!(a_got.regions, a_exp.regions, "regions on ch {c_got}");
    }

    // The signature itself must agree across the two layouts
    // (otherwise the dispatcher wouldn't take the mutator
    // branch in the first place).
    assert_eq!(
        console_overlay_signature(&layout_a),
        console_overlay_signature(&layout_b)
    );
}

/// Scrollback-grow shifts the structural signature — the
/// dispatcher must take the full-rebuild path, not the
/// in-place mutator path. Without this, a mutator computed
/// from N+1 scrollback entries applied to a tree built from
/// N would walk off the end and silently drop content. Pins
/// the structural-signature contract the dispatcher relies
/// on in `rebuild_console_overlay_buffers`.
#[test]
fn console_signature_shifts_on_scrollback_grow() {
    baumhard::font::fonts::init();

    let mut g_one = sample_console_geometry();
    g_one.scrollback = vec![ConsoleOverlayLine {
        text: "> help".into(),
        kind: ConsoleOverlayLineKind::Input,
        font_family: None,
    }];
    let layout_one = compute_console_frame_layout(&g_one, 1280.0, 720.0);

    let mut g_two = sample_console_geometry();
    g_two.scrollback = vec![
        ConsoleOverlayLine {
            text: "> help".into(),
            kind: ConsoleOverlayLineKind::Input,
            font_family: None,
        },
        ConsoleOverlayLine {
            text: "new output line".into(),
            kind: ConsoleOverlayLineKind::Output,
            font_family: None,
        },
    ];
    let layout_two = compute_console_frame_layout(&g_two, 1280.0, 720.0);

    assert_ne!(layout_one.scrollback_rows, layout_two.scrollback_rows);
    assert_ne!(
        console_overlay_signature(&layout_one),
        console_overlay_signature(&layout_two)
    );
}

/// `console_overlay_areas` degrades (logs + skips the slot) rather
/// than panicking when a caller violates the
/// `scrollback_rows = min(scrollback.len(), MAX)` (or
/// `completion_rows` mirror) invariant — interactive paths never
/// abort (§7). Pin the degraded behavior: artificially shorten the
/// geometry's scrollback vec AFTER computing the layout so
/// `scrollback_rows` (baked into the layout) exceeds
/// `geometry.scrollback.len()`, then call `console_overlay_areas`
/// and assert we return without panic.
///
/// A regression to `.expect()` would poison the test thread; the
/// surviving return proves the defensive path still fires.
#[test]
fn console_overlay_areas_degrades_when_scrollback_shorter_than_layout_rows() {
    baumhard::font::fonts::init();

    let mut g = sample_console_geometry();
    // Populate enough scrollback entries for the layout to reserve
    // several rows, then truncate AFTER layout so the
    // `scrollback_rows` count in the layout outruns the vec's length.
    g.scrollback = (0..5)
        .map(|i| ConsoleOverlayLine {
            text: format!("line {i}"),
            kind: ConsoleOverlayLineKind::Output,
            font_family: None,
        })
        .collect();
    g.completions = Vec::new();
    let layout = compute_console_frame_layout(&g, 1280.0, 720.0);
    assert!(layout.scrollback_rows >= 1, "layout must reserve rows");

    // Evict scrollback so layout.scrollback_rows > geometry.scrollback.len().
    g.scrollback.clear();

    let areas = {
        let mut fs = baumhard::font::fonts::acquire_font_system_write("renderer::tests (scrollback degrade)");
        console_overlay_areas(&g, &layout, &mut fs)
    };
    // Survival check: we got here without aborting. Every slot the
    // degraded path skipped dropped out of the output, but the
    // prompt / border / empty-completion slots still emit.
    assert!(!areas.is_empty(), "non-scrollback slots still render");
}

/// Mirror guard for the completion-popup slot. Populate completions
/// enough for the layout to reserve rows, clear the vec AFTER
/// layout, then call `console_overlay_areas` and assert no panic.
#[test]
fn console_overlay_areas_degrades_when_completions_shorter_than_layout_rows() {
    baumhard::font::fonts::init();

    let mut g = sample_console_geometry();
    g.scrollback = Vec::new();
    g.completions = (0..3)
        .map(|i| ConsoleOverlayCompletion {
            text: format!("cand{i}"),
            hint: None,
            font_family: None,
        })
        .collect();
    g.selected_completion = Some(0);
    let layout = compute_console_frame_layout(&g, 1280.0, 720.0);
    assert!(layout.completion_rows >= 1, "layout must reserve rows");

    g.completions.clear();
    g.selected_completion = None;

    let areas = {
        let mut fs = baumhard::font::fonts::acquire_font_system_write("renderer::tests (completion degrade)");
        console_overlay_areas(&g, &layout, &mut fs)
    };
    assert!(!areas.is_empty(), "non-completion slots still render");
}

/// Freeze-hardening regression: the surface-size clamp must leave
/// dimensions untouched when both axes are within the GPU's
/// `max_texture_dimension_2d` budget. Picking up an oversize
/// request silently (not clamping at all) would defeat the guard;
/// clamping when we didn't need to would spuriously letterbox.
#[test]
fn clamp_surface_size_is_identity_below_limit() {
    // A typical 4K panel in landscape — well under any modern
    // GPU's 2D texture limit (typically 8192 or 16384).
    let (w, h) = clamp_surface_size_to_gpu_limit(3840, 2160, 8192);
    assert_eq!((w, h), (3840, 2160));
}

/// The clamp must pin each axis that exceeds the GPU limit and
/// leave the other axis alone. Ultrawide-at-max on a modest GPU
/// is the realistic freeze-triggering scenario.
#[test]
fn clamp_surface_size_caps_only_the_oversized_axis() {
    // Width over, height fine.
    assert_eq!(clamp_surface_size_to_gpu_limit(10_000, 4096, 8192), (8192, 4096));
    // Height over, width fine.
    assert_eq!(clamp_surface_size_to_gpu_limit(4096, 10_000, 8192), (4096, 8192));
    // Both over — both pinned.
    assert_eq!(
        clamp_surface_size_to_gpu_limit(10_000, 12_000, 8192),
        (8192, 8192)
    );
}

/// Boundary: exactly at the limit is not clamped. The wgpu
/// contract is that dimensions **up to and including**
/// `max_texture_dimension_2d` are valid.
#[test]
fn clamp_surface_size_passes_exact_limit() {
    let (w, h) = clamp_surface_size_to_gpu_limit(8192, 8192, 8192);
    assert_eq!((w, h), (8192, 8192));
}

/// Integration-level cull: a `NodeBackgroundRect` whose
/// spatial AABB is fully inside the viewport must still be
/// dropped when `camera.zoom` falls outside its
/// `zoom_visibility` window. Exercises the combined predicate
/// that `render::render` runs on every background rect each
/// frame; a regression that short-circuited the zoom check
/// (e.g. `||` instead of `&&`) would leave the rect visible
/// at every zoom and trip this test.
#[test]
fn background_rect_culled_when_zoom_outside_window() {
    use baumhard::gfx_structs::camera::Camera2D;
    use baumhard::gfx_structs::shape::NodeShape;
    use baumhard::gfx_structs::zoom_visibility::ZoomVisibility;

    let mut camera = Camera2D::new(800, 600);
    // Rect centered at canvas origin (the camera's default
    // position) so the spatial check is satisfied at every
    // zoom in the camera's clamped range — we want the zoom
    // window to be the sole rejection reason.
    let rect = NodeBackgroundRect {
        position: Vec2::new(-50.0, -50.0),
        size: Vec2::new(100.0, 100.0),
        color: [64, 64, 64, 255],
        shape_id: NodeShape::Rectangle.shader_id(),
        zoom_visibility: ZoomVisibility {
            min: Some(1.0),
            max: Some(2.0),
        },
        unique_id: 0,
    };

    // Inside the window: visible.
    camera.zoom = 1.0;
    assert!(rect.visible_at(&camera), "zoom at min bound should render");
    camera.zoom = 1.5;
    assert!(rect.visible_at(&camera));
    camera.zoom = 2.0;
    assert!(rect.visible_at(&camera), "zoom at max bound should render");

    // Outside the window: culled.
    camera.zoom = 0.5;
    assert!(!rect.visible_at(&camera), "zoom below min should cull");
    camera.zoom = 3.0;
    assert!(!rect.visible_at(&camera), "zoom above max should cull");
}

/// Integration-level cull: an unbounded rect (the historical
/// default — both bounds `None`) renders regardless of
/// `camera.zoom`. Pins the "existing maps pay nothing" contract.
#[test]
fn background_rect_with_unbounded_window_renders_at_every_zoom() {
    use baumhard::gfx_structs::camera::Camera2D;
    use baumhard::gfx_structs::shape::NodeShape;
    use baumhard::gfx_structs::zoom_visibility::ZoomVisibility;

    let mut camera = Camera2D::new(800, 600);
    let rect = NodeBackgroundRect {
        position: Vec2::new(-50.0, -50.0),
        size: Vec2::new(100.0, 100.0),
        color: [64, 64, 64, 255],
        shape_id: NodeShape::Rectangle.shader_id(),
        zoom_visibility: ZoomVisibility::unbounded(),
        unique_id: 0,
    };

    for z in [0.05_f32, 0.5, 1.0, 2.5, 5.0] {
        camera.zoom = z;
        assert!(
            rect.visible_at(&camera),
            "unbounded window must render at zoom {z}"
        );
    }
}

/// Spatial and zoom culls compose as AND: a rect outside the
/// viewport is dropped even if its zoom window is satisfied.
/// Mirrors the "spatial cull short-circuits" invariant so a
/// future refactor that reverses the two checks still sees
/// this test stay green.
#[test]
fn background_rect_off_viewport_still_culled_with_matching_zoom() {
    use baumhard::gfx_structs::camera::Camera2D;
    use baumhard::gfx_structs::shape::NodeShape;
    use baumhard::gfx_structs::zoom_visibility::ZoomVisibility;

    let mut camera = Camera2D::new(800, 600);
    camera.zoom = 1.0;
    // Far off to the right of the viewport at canvas x = 10_000.
    let rect = NodeBackgroundRect {
        position: Vec2::new(10_000.0, 200.0),
        size: Vec2::new(100.0, 100.0),
        color: [64, 64, 64, 255],
        shape_id: NodeShape::Rectangle.shader_id(),
        zoom_visibility: ZoomVisibility {
            min: Some(1.0),
            max: Some(2.0),
        },
        unique_id: 0,
    };
    assert!(
        !rect.visible_at(&camera),
        "off-viewport rect must be culled regardless of zoom window"
    );
}

// --- FrameIntervalRing --------------------------------------------------
// Fundamentals coverage for the sum invariant backing
// `FpsDisplayMode::Debug`'s rolling average. Pure arithmetic, no clock.

#[test]
fn frame_interval_ring_new_is_empty() {
    let ring = FrameIntervalRing::new();
    assert_eq!(ring.avg_micros(), None, "empty ring has no average");
}

#[test]
fn frame_interval_ring_single_push_is_that_value() {
    let mut ring = FrameIntervalRing::new();
    ring.push(16_666);
    assert_eq!(ring.avg_micros(), Some(16_666));
}

#[test]
fn frame_interval_ring_partial_fill_averages_visible_samples() {
    let mut ring = FrameIntervalRing::new();
    ring.push(10);
    ring.push(20);
    ring.push(30);
    // Divisor is `filled` (3), not `FPS_WINDOW` — zero-padding the array
    // on cold start must not pull the reported average toward zero.
    assert_eq!(ring.avg_micros(), Some(20));
}

#[test]
fn frame_interval_ring_exact_fill_reports_uniform_value() {
    let mut ring = FrameIntervalRing::new();
    for _ in 0..FPS_WINDOW {
        ring.push(1_000);
    }
    assert_eq!(ring.avg_micros(), Some(1_000));
}

#[test]
fn frame_interval_ring_wrap_drops_oldest_sample() {
    let mut ring = FrameIntervalRing::new();
    // Seed with a distinctive sentinel so we can confirm it leaves the
    // window on wraparound.
    let sentinel = 999_999u128;
    ring.push(sentinel);
    for _ in 0..(FPS_WINDOW - 1) {
        ring.push(1_000);
    }
    // Ring is exactly full; sentinel + (FPS_WINDOW - 1) * 1000 in the
    // window. Average:
    //   (999_999 + 199 * 1000) / 200 = (999_999 + 199_000) / 200 = 5994
    let expected_with_sentinel = (sentinel + 1_000u128 * (FPS_WINDOW as u128 - 1)) / FPS_WINDOW as u128;
    assert_eq!(ring.avg_micros(), Some(expected_with_sentinel));

    // Push one more — the sentinel falls out of the window, and the
    // running sum must update accordingly. After this, the ring holds
    // FPS_WINDOW copies of 1_000.
    ring.push(1_000);
    assert_eq!(
        ring.avg_micros(),
        Some(1_000),
        "oldest sample must drop out of the rolling sum on wraparound"
    );
}

#[test]
fn frame_interval_ring_zero_value_still_occupies_slot() {
    let mut ring = FrameIntervalRing::new();
    ring.push(0);
    ring.push(200);
    // Two samples, sum 200 → avg 100. The zero push did NOT refuse the
    // slot; it contributed zero to the sum but advanced `filled`.
    assert_eq!(ring.avg_micros(), Some(100));
}

#[test]
fn frame_interval_ring_clear_restores_empty_state() {
    let mut ring = FrameIntervalRing::new();
    for i in 0..50 {
        ring.push((i + 1) as u128 * 100);
    }
    assert!(ring.avg_micros().is_some());
    ring.clear();
    assert_eq!(ring.avg_micros(), None);
    // And a fresh push lands cleanly on top — prior state did not
    // leak through clear().
    ring.push(42);
    assert_eq!(ring.avg_micros(), Some(42));
}

// ====================================================================
// Overlay re-shape granularity
// ====================================================================
//
// `rebuild_overlay_scene_buffers` keeps an element's shaped buffers
// when nothing the shaper reads has changed. These tests pin the
// reuse rule in `scene_shape_cache` — what invalidates a cached
// element, and what deliberately does not. The pass itself is not
// exercised: it lives on `Renderer`, and standing up a wgpu device
// for a test is what TEST_CONVENTIONS §T8 forbids. What is
// exercised is the whole of the decision that pass makes.

use baumhard::core::primitives::{Applicable, ApplyOperation, ColorFontRegions};
use baumhard::gfx_structs::area::DeltaGlyphArea;
use baumhard::gfx_structs::area_fields::{GlyphAreaField, GlyphAreaFieldType, OutlineStyle};
use baumhard::gfx_structs::delta::DeltaField;
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::mutator::GfxMutator;
use baumhard::gfx_structs::scene::{Scene, SceneTreeId};
use baumhard::gfx_structs::shape::NodeShape;
use baumhard::gfx_structs::tree::Tree;
use baumhard::gfx_structs::zoom_visibility::ZoomVisibility;
use strum::IntoEnumIterator;

use super::scene_shape_cache::{self, ScenePassKind, ShapedSceneElement};

/// A styled, outlined, background-filled area — every field the
/// shaper reads set to something other than its default, so a
/// mutation to any one of them is a real difference rather than a
/// coincidence with `GlyphArea::new`'s zeros.
fn overlay_cache_fixture_area() -> GlyphArea {
    let mut area = GlyphArea::new_with_str("cell", 16.0, 18.0, Vec2::new(3.0, 5.0), Vec2::new(40.0, 20.0));
    area.regions = ColorFontRegions::single_span(4, Some([0.1, 0.2, 0.3, 1.0]), None);
    area.background_color = Some([9, 8, 7, 255]);
    area.align_center = true;
    area.outline = Some(OutlineStyle {
        color: [1, 2, 3, 4],
        px: 2.0,
    });
    area.shape = NodeShape::Ellipse;
    area
}

/// The fixture area wrapped in an element, registered in a scene so
/// the test has a real [`SceneTreeId`] to key against.
fn overlay_cache_fixture() -> (Scene, SceneTreeId, GfxElement) {
    let element = GfxElement::new_area_non_indexed_with_id(overlay_cache_fixture_area(), 7, 7);
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    let leaf = tree.arena.new_node(element.clone());
    tree.root.append(leaf, &mut tree.arena);
    let mut scene = Scene::new();
    let id = scene.insert(tree, 0, Vec2::ZERO);
    (scene, id, element)
}

/// One representative payload per area-field tag, differing from
/// [`overlay_cache_fixture_area`] on every one. The `match` is over
/// the strum-derived tag enum, so it is exhaustive and a new
/// `GlyphAreaField` variant will not compile until it is classified
/// here — the same forcing `gfx_structs::tests::delta_tests`'s
/// `area_field_for` relies on.
fn overlay_cache_field_for(tag: GlyphAreaFieldType) -> GlyphAreaField {
    match tag {
        GlyphAreaFieldType::Text => GlyphAreaField::Text("other".to_string()),
        GlyphAreaFieldType::Scale => GlyphAreaField::scale(11.0),
        GlyphAreaFieldType::LineHeight => GlyphAreaField::line_height(13.0),
        GlyphAreaFieldType::Position => GlyphAreaField::position(101.0, 202.0),
        GlyphAreaFieldType::Bounds => GlyphAreaField::bounds(303.0, 404.0),
        GlyphAreaFieldType::ColorFontRegions => GlyphAreaField::ColorFontRegions(
            ColorFontRegions::single_span(4, Some([0.9, 0.8, 0.7, 1.0]), None),
        ),
        GlyphAreaFieldType::Outline => GlyphAreaField::Outline(Some(OutlineStyle {
            color: [200, 201, 202, 203],
            px: 5.0,
        })),
        GlyphAreaFieldType::Shape => GlyphAreaField::Shape(NodeShape::Rectangle),
        GlyphAreaFieldType::ZoomVisibility => {
            GlyphAreaField::ZoomVisibility(ZoomVisibility::try_new(Some(0.5), Some(2.0)).unwrap())
        }
        // Control variant: it names the arithmetic the sibling
        // deltas apply with and writes no field of its own.
        GlyphAreaFieldType::Operation => GlyphAreaField::Operation(ApplyOperation::Assign),
    }
}

/// Every `GlyphAreaField` a mutator can write invalidates the cached
/// shaping — and the one control variant that writes nothing does
/// not.
///
/// This is the guard the narrowing rests on.
/// `rebuild_overlay_scene_buffers` skips re-shaping an element whose
/// inputs still compare equal, so a field a mutator can change but
/// the comparison cannot see would leave a stale glyph on screen —
/// with no test in this repo able to see it (§T8 rules out pixels).
/// Adding a variant to `GlyphAreaField` fails to compile here until
/// it is classified, and classifying it as a real field fails the
/// assertion below unless the comparison actually covers it.
#[test]
fn test_scene_shape_cache_invalidates_on_every_writable_area_field() {
    for tag in GlyphAreaFieldType::iter() {
        let (_scene, id, element) = overlay_cache_fixture();
        let cached = ShapedSceneElement::new(id, &element, Vec2::ZERO, Vec::new(), None);
        assert!(
            cached.still_matches(id, &element, Vec2::ZERO),
            "an untouched element must match what it was shaped from"
        );

        let mut mutated = element.clone();
        let area = mutated.glyph_area_mut().expect("fixture is an area element");
        DeltaGlyphArea::new(vec![
            GlyphAreaField::Operation(ApplyOperation::Assign),
            overlay_cache_field_for(tag),
        ])
        .apply_to(area);

        // "Did the area actually change?" is asked through derived
        // `Debug`, not through `PartialEq`. `PartialEq` is half of
        // what `still_matches` consults, so using it here would make
        // the guard below circular — and it is specifically blind to
        // a region recolor, the exact change a picker hover makes.
        // `Debug` is derived over every field, so it sees whatever
        // moved.
        let described = |e: &GfxElement| format!("{:?}", e.glyph_area());
        if tag == GlyphAreaField::OPERATION_KEY {
            assert_eq!(
                described(&mutated),
                described(&element),
                "GlyphAreaField::{tag} is documented as writing no field, but it changed the area"
            );
            assert!(
                cached.still_matches(id, &mutated, Vec2::ZERO),
                "GlyphAreaField::{tag} changes nothing the shaper reads, so it must not force a re-shape"
            );
            continue;
        }

        // Without this the next assertion would pass for a payload
        // that happens to equal the fixture's value — the field
        // would look covered while nothing had actually changed.
        assert_ne!(
            described(&mutated),
            described(&element),
            "the representative payload for GlyphAreaField::{tag} does not differ from the \
             fixture, so this iteration cannot prove anything"
        );
        assert!(
            !cached.still_matches(id, &mutated, Vec2::ZERO),
            "GlyphAreaField::{tag} changed the area but the overlay shape cache would have \
             reused the old buffers — the element would render stale"
        );
    }
}

/// The three shaping inputs that are not area fields. Read straight
/// off `shape_one_element_into_buffers`, whose entire input is
/// `element.glyph_area()`, `element.unique_id()`, and its `offset`
/// argument; the tree id is carried alongside because a scene slab
/// index is reused after an overlay unregisters.
#[test]
fn test_scene_shape_cache_invalidates_on_identity_and_offset() {
    let (mut scene, id, element) = overlay_cache_fixture();
    let cached = ShapedSceneElement::new(id, &element, Vec2::ZERO, Vec::new(), None);

    // `unique_id` reaches the emitted buffer through the walker's
    // yield, and is how a caller finds the buffer again.
    let renamed = GfxElement::new_area_non_indexed_with_id(overlay_cache_fixture_area(), 7, 8);
    assert!(
        !cached.still_matches(id, &renamed, Vec2::ZERO),
        "a different element at this walk position must not inherit the cached buffers"
    );

    // The registered tree offset is added to every emitted buffer's
    // `pos`, so moving the overlay must re-shape rather than leave
    // buffers at the old screen slot.
    assert!(
        !cached.still_matches(id, &element, Vec2::new(0.0, 30.0)),
        "a shifted tree offset must not reuse buffers positioned for the old one"
    );

    // A second tree in the same scene is a different overlay.
    let other_tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    let other_id = scene.insert(other_tree, 1, Vec2::ZERO);
    assert!(
        !cached.still_matches(other_id, &element, Vec2::ZERO),
        "cached buffers belong to the tree they were walked from"
    );
}

/// `hitbox` is the one `GlyphArea` field the equality check skips,
/// and it is skipped because the shaper never reads it. Asserted so
/// the check is known to be a real discriminator rather than a
/// constant `false` that would make every other assertion above
/// vacuous.
#[test]
fn test_scene_shape_cache_reuses_when_only_the_hitbox_moved() {
    let (_scene, id, element) = overlay_cache_fixture();
    let cached = ShapedSceneElement::new(id, &element, Vec2::ZERO, Vec::new(), None);

    let mut moved_hitbox = element.clone();
    let area = moved_hitbox.glyph_area_mut().expect("fixture is an area element");
    area.hitbox_as_mut()
        .add(baumhard::gfx_structs::util::hitbox::BoundingRectangle::at_origin(
            99.0, 99.0,
        ));
    assert_ne!(
        moved_hitbox.glyph_area().map(|a| a.hitbox().rectangles.len()),
        element.glyph_area().map(|a| a.hitbox().rectangles.len()),
        "the hit box has to actually differ, or this test proves nothing"
    );
    assert!(
        cached.still_matches(id, &moved_hitbox, Vec2::ZERO),
        "the hit box does not reach cosmic-text, so changing it must not cost a re-shape"
    );
}

/// One named single-field mutation of a `GlyphArea`, for
/// [`test_glyph_area_equality_ignores_only_the_hitbox`]'s table. A
/// `fn` pointer rather than a closure type so every row of the table
/// shares one type and the whole thing fits in an array.
type NamedAreaMutation = (&'static str, fn(&mut GlyphArea));

/// `GlyphArea`'s `PartialEq` ignore-list is exactly `{hitbox}`.
///
/// The overlay cache's module header rests on "a field added to the
/// area joins the comparison without anyone remembering to wire it
/// in". That is only true for a field *not* marked
/// `#[derivative(PartialEq = "ignore")]`, and the struct already
/// carries one such marking — so the sentence needs a guard rather
/// than trust. This is it, in two halves:
///
/// - the exhaustive destructuring has no `..` rest pattern, so a
///   field added to `GlyphArea` fails to compile here until someone
///   names it and decides which list it belongs on;
/// - every field named on the equality-visible side is then mutated
///   in isolation and asserted to break `==`, so an `ignore` added
///   to one of today's fields fails rather than quietly widening the
///   cache's blind spot.
///
/// Three of those fields — `background_color`, `background_padding`
/// and `align_center` — are read by the shaper (or by the fill the
/// shaper's walker emits) and reachable by no `GlyphAreaField`
/// mutator variant, so the sibling
/// `test_scene_shape_cache_invalidates_on_every_writable_area_field`
/// cannot see them. **This test is not where they are covered**, and
/// the sentence that used to say so was wrong in a way worth naming:
/// `GlyphArea::==` and the reuse key are two different subjects, and
/// a field can be visible to the first while no test holds the
/// second against it. All three are covered against the key by name
/// — the two fill inputs in
/// [`test_canvas_fill_inputs_are_all_visible_to_the_reuse_key`], the
/// one text input in
/// [`test_canvas_text_inputs_are_all_visible_to_the_reuse_key`].
/// What *this* test covers is the property those two rest on: that
/// `==` is a real discriminator over every field but `hitbox`, so
/// the key does not have to enumerate them.
#[test]
fn test_glyph_area_equality_ignores_only_the_hitbox() {
    use baumhard::gfx_structs::area::EdgePadding;
    use baumhard::gfx_structs::util::hitbox::BoundingRectangle;

    let base = overlay_cache_fixture_area();

    // Exhaustive on purpose — no `..`. A new `GlyphArea` field stops
    // the build right here.
    let GlyphArea {
        text: _,
        scale: _,
        line_height: _,
        position: _,
        render_bounds: _,
        regions: _,
        background_color: _,
        background_padding: _,
        align_center: _,
        outline: _,
        shape: _,
        zoom_visibility: _,
        hitbox: _,
    } = &base;

    // One mutation per equality-visible field, each applied to a
    // fresh clone of the fixture so the fields stay independent.
    // `regions` is perturbed by its *span set* rather than by a
    // color: derived equality bottoms out in `BTreeSet` identity by
    // range and is deliberately blind to a recolor — the module
    // header's "The one place `==` is not enough", which
    // `ColorFontRegions::same_content` answers separately.
    let visible: [NamedAreaMutation; 12] = [
        ("text", |a| a.text.push('!')),
        ("scale", |a| a.scale = (a.scale.0 + 1.0).into()),
        ("line_height", |a| a.line_height = (a.line_height.0 + 1.0).into()),
        ("position", |a| a.set_position((123.0, 456.0))),
        ("render_bounds", |a| a.set_bounds((321.0, 654.0))),
        ("regions", |a| {
            a.regions = ColorFontRegions::single_span(2, Some([0.1, 0.2, 0.3, 1.0]), None)
        }),
        ("background_color", |a| a.background_color = Some([1, 2, 3, 4])),
        ("background_padding", |a| {
            a.background_padding = EdgePadding::new(1.0, 2.0, 3.0, 4.0)
        }),
        ("align_center", |a| a.align_center = !a.align_center),
        ("outline", |a| a.outline = None),
        ("shape", |a| a.shape = NodeShape::Rectangle),
        ("zoom_visibility", |a| {
            a.zoom_visibility = ZoomVisibility::try_new(Some(0.25), Some(4.0)).unwrap()
        }),
    ];

    for (name, mutate) in visible {
        let mut changed = base.clone();
        mutate(&mut changed);
        // Derived `Debug`, not `PartialEq` — asking `PartialEq`
        // whether the field moved is the circularity this test
        // exists to avoid.
        assert_ne!(
            format!("{:?}", changed),
            format!("{:?}", base),
            "the mutation for `{name}` does not change the area, so this iteration proves nothing"
        );
        assert_ne!(
            changed, base,
            "`GlyphArea::{name}` is read when shaping but `PartialEq` cannot see it — the overlay \
             shape cache would reuse a stale buffer. Either drop its `PartialEq = \"ignore\"` or \
             move it to the ignored side of this test and say why the shaper never reads it."
        );
    }

    // ...and the one field on the ignored side.
    let mut moved_hitbox = base.clone();
    moved_hitbox
        .hitbox_as_mut()
        .add(BoundingRectangle::at_origin(99.0, 99.0));
    assert_ne!(
        format!("{:?}", moved_hitbox),
        format!("{:?}", base),
        "the hit box has to actually differ, or the assertion below is vacuous"
    );
    assert_eq!(
        moved_hitbox, base,
        "`hitbox` is the only field `PartialEq` is allowed to skip"
    );
}

/// Snapshot a registered overlay tree exactly the way
/// `rebuild_overlay_scene_buffers` does — one cache entry per walked
/// element, in walk order. Buffers are left empty: `still_matches`
/// never inspects them, and shaping real ones would buy the test
/// nothing it could assert on (§T8).
fn snapshot_overlay(scene: &Scene, id: SceneTreeId) -> Vec<ShapedSceneElement> {
    let tree = scene.tree(id).expect("tree registered");
    tree.root()
        .descendants(&tree.arena)
        .filter_map(|d| tree.arena.get(d).map(|n| n.get()))
        .map(|element| ShapedSceneElement::new(id, element, Vec2::ZERO, Vec::new(), None))
        .collect()
}

/// Which walk positions of a registered overlay tree no longer match
/// `cached` — i.e. exactly the elements
/// `rebuild_overlay_scene_buffers` would re-shape.
fn overlay_elements_needing_reshape<'a>(
    scene: &'a Scene,
    id: SceneTreeId,
    cached: &[ShapedSceneElement],
) -> Vec<&'a GfxElement> {
    let tree = scene.tree(id).expect("tree registered");
    tree.root()
        .descendants(&tree.arena)
        .filter_map(|d| tree.arena.get(d).map(|n| n.get()))
        .enumerate()
        .filter(|(i, element)| {
            !cached
                .get(*i)
                .is_some_and(|c| c.still_matches(id, element, Vec2::ZERO))
        })
        .map(|(_, element)| element)
        .collect()
}

/// A console keystroke re-shapes the prompt line and nothing else.
///
/// This is the granularity claim for the console half of the issue,
/// asserted on the real console tree and the real in-place mutator.
/// `build_console_overlay_mutator` writes a `full_assign_from` delta
/// to *every* slot on every keystroke, so before the reuse check the
/// pass re-shaped every border, gutter, scrollback row, completion
/// row and the prompt; the count below is what the same event costs
/// now.
///
/// The identical-state case is asserted alongside it because it is
/// what makes the count meaningful: a mutator that overwrote each
/// slot with a *different-looking* value would also produce "1
/// changed" here for the wrong reason.
#[test]
fn test_console_keystroke_reshapes_only_the_prompt_line() {
    baumhard::font::fonts::init();

    let mut before = sample_console_geometry();
    before.input = "anchor".into();
    before.cursor_grapheme = 6;
    let layout_before = compute_console_frame_layout(&before, 1280.0, 720.0);

    let mut after = sample_console_geometry();
    after.input = "anchors".into();
    after.cursor_grapheme = 7;
    let layout_after = compute_console_frame_layout(&after, 1280.0, 720.0);

    // The in-place mutator arm only runs while the structural
    // signature holds; a keystroke that changed it would take the
    // full-rebuild arm and this test would be describing a path the
    // console never takes.
    assert_eq!(
        console_overlay_signature(&layout_before),
        console_overlay_signature(&layout_after)
    );

    let tree = {
        let mut fs = baumhard::font::fonts::acquire_font_system_write("renderer::tests (keystroke tree)");
        build_console_overlay_tree(&before, &layout_before, &mut fs)
    };
    let mut scene = Scene::new();
    let id = scene.insert(tree, 0, Vec2::ZERO);
    let cached = snapshot_overlay(&scene, id);
    assert!(
        cached.len() > 10,
        "the console fixture must have many slots, or \"only one changed\" says nothing: {} slots",
        cached.len()
    );

    // Re-applying the state it already holds must invalidate
    // nothing, even though the mutator assigns every field of every
    // slot.
    let idempotent = {
        let mut fs = baumhard::font::fonts::acquire_font_system_write("renderer::tests (keystroke same)");
        build_console_overlay_mutator(&before, &layout_before, &mut fs)
    };
    scene.apply_mutator(id, &idempotent);
    assert!(
        overlay_elements_needing_reshape(&scene, id, &cached).is_empty(),
        "a mutator that writes the state already on screen must not cost a single re-shape"
    );

    let keystroke = {
        let mut fs = baumhard::font::fonts::acquire_font_system_write("renderer::tests (keystroke next)");
        build_console_overlay_mutator(&after, &layout_after, &mut fs)
    };
    scene.apply_mutator(id, &keystroke);

    let changed = overlay_elements_needing_reshape(&scene, id, &cached);
    let texts: Vec<&str> = changed
        .iter()
        .filter_map(|e| e.glyph_area())
        .map(|a| a.text.as_str())
        .collect();
    assert_eq!(
        changed.len(),
        1,
        "one keystroke should dirty one console line, got {texts:?}"
    );
    assert!(
        texts[0].contains("anchors"),
        "the dirtied line should be the prompt carrying the new input, got {texts:?}"
    );
}

/// A color-picker hover re-shapes the one cell it recolors, not the
/// whole wheel.
///
/// Sibling of the console test above, on the other overlay the issue
/// names, and pinned to an exact count the same way that one is.
/// `build_dynamic_mutator` writes color regions, hover scale and hex
/// text into *every* picker slot on every mouse-move frame, and the
/// wheel's cells carry an `outline`, so each one costs nine
/// cosmic-text shapings; before the reuse check a hover paid that
/// for all of them.
///
/// Nothing here is environment-dependent — the fixture and the
/// layout are deterministic, and `still_matches` compares
/// `GlyphArea`s without shaping anything — so there is no reason to
/// spend the assertion on a fraction. A `changed * 4 < len` bound
/// would have tolerated fourteen dirty cells out of fifty-nine and
/// stayed green through a tenfold regression. If a future picker
/// legitimately recolors more than one cell on a hue hover, this
/// number is meant to be re-read and updated, not widened into a
/// threshold.
#[test]
fn test_picker_hover_reshapes_a_small_part_of_the_wheel() {
    use crate::application::color_picker::tests::fixtures::sample_geometry;
    use crate::application::color_picker::{compute_color_picker_layout, PickerHit};
    use crate::application::color_picker_overlay;

    baumhard::font::fonts::init();

    let geometry = sample_geometry();
    let layout = compute_color_picker_layout(&geometry, 1280.0, 720.0);
    let built = color_picker_overlay::build(&geometry, &layout);
    let mut scene = Scene::new();
    let id = scene.insert(built.tree, 0, Vec2::ZERO);
    let cached = snapshot_overlay(&scene, id);
    assert!(
        cached.len() > 30,
        "the picker fixture must have many cells, or a fraction says nothing: {} cells",
        cached.len()
    );

    // Re-asserting the state already on screen must cost nothing,
    // even though the dynamic mutator assigns every slot.
    let idempotent = color_picker_overlay::build_dynamic_mutator(&geometry, &layout);
    scene.apply_mutator(id, &idempotent);
    assert!(
        overlay_elements_needing_reshape(&scene, id, &cached).is_empty(),
        "a hover frame that changed nothing must not re-shape a single cell"
    );

    let mut hovered = sample_geometry();
    hovered.hovered_hit = Some(PickerHit::Hue(3));
    let hover = color_picker_overlay::build_dynamic_mutator(&hovered, &layout);
    scene.apply_mutator(id, &hover);

    let changed = overlay_elements_needing_reshape(&scene, id, &cached).len();
    assert_eq!(
        changed,
        1,
        "hovering one hue slot recolors exactly that cell, so exactly one of the wheel's {} \
         elements may be re-shaped",
        cached.len()
    );
}

// ====================================================================
// Canvas re-shape granularity — the background-fill half
// ====================================================================
//
// The canvas pass differs from the overlay pass in one thing: it
// keeps the background fills the walker emits, and those fills are
// wired to a draw pass that paints them in index order. These tests
// cover that half. The reuse rule itself is shared and is covered by
// the overlay tests above, which now name `scene_shape_cache`
// because the rule has one owner for both sub-scenes.
//
// No `tree_builder` pass sets `background_color` on a canvas-role
// element today, so the fills below are authored here rather than
// projected from a role. The invariant is pinned anyway, for the
// same reason `test_walker_records_the_stamp_offset_of_every_emitted_buffer`
// pins halo bookkeeping for mindmap elements that carry no outline:
// the day a canvas role gains a fill, the draw-order rule it rests
// on has to already hold.

/// A `GlyphArea` with every fill input set to something other than
/// its default, so a mutation to any one of them is a real
/// difference rather than a coincidence with `GlyphArea::new`'s
/// zeros.
fn canvas_fill_fixture_area(text: &str, x: f32) -> GlyphArea {
    use baumhard::gfx_structs::area::EdgePadding;
    let mut area = GlyphArea::new_with_str(text, 16.0, 18.0, Vec2::new(x, 4.0), Vec2::new(40.0, 20.0));
    area.background_color = Some([10, 20, 30, 255]);
    area.background_padding = EdgePadding::new(1.0, 2.0, 3.0, 4.0);
    area.shape = NodeShape::Ellipse;
    area.zoom_visibility = ZoomVisibility::try_new(Some(0.5), Some(8.0)).unwrap();
    area
}

/// Two canvas trees of two filled elements each, registered in one
/// scene at ascending layers — the smallest shape in which "an early
/// element's fill was lifted above a later one's" is a visible
/// difference.
fn canvas_fill_fixture_scene() -> (Scene, Vec<SceneTreeId>) {
    let mut scene = Scene::new();
    let mut ids = Vec::new();
    for (tree_idx, base_x) in [(0usize, 0.0f32), (1usize, 500.0f32)] {
        let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
        for leaf_idx in 0..2usize {
            let unique_id = tree_idx * 10 + leaf_idx + 1;
            let area = canvas_fill_fixture_area(&format!("r{unique_id}"), base_x + 60.0 * leaf_idx as f32);
            let element = GfxElement::new_area_non_indexed_with_id(area, 1, unique_id);
            let leaf = tree.arena.new_node(element);
            tree.root.append(leaf, &mut tree.arena);
        }
        ids.push(scene.insert(tree, tree_idx as i32, Vec2::ZERO));
    }
    (scene, ids)
}

/// The pass's fill stream in draw order, described through derived
/// `Debug` so a difference in any field shows up.
fn canvas_fill_stream(shaped: &[ShapedSceneElement]) -> Vec<String> {
    scene_shape_cache::background_fills(shaped)
        .map(|rect| format!("{rect:?}"))
        .collect()
}

/// The pass's text stream in draw order, described by the one thing
/// about a shaped `cosmic_text::Buffer` that is comparable — where it
/// was placed.
fn canvas_text_stream(shaped: &[ShapedSceneElement]) -> Vec<(f32, f32)> {
    scene_shape_cache::text_buffers(shaped)
        .map(|buffer| buffer.pos)
        .collect()
}

/// A partial re-shape leaves both order-sensitive draw streams
/// exactly where a full one would put them.
///
/// This is the hazard the issue names for the canvas half, and the
/// mindmap path shipped it once: `reshape_buffer_for` dropped an
/// element's fill and re-pushed the fresh one at the end of a flat
/// list `render.rs` paints in index order, so the element being
/// edited floated above every other node's fill until the next full
/// rebuild. The canvas pass cannot reach that state — each element's
/// fill lives inside that element's own entry, and an entry is
/// rewritten at the walk position it already occupied — and this is
/// what says so.
///
/// The reference is not a hand-written expectation: it is what the
/// same scene produces through an *empty* cache, i.e. the whole-scene
/// re-shape this pass used to do unconditionally. Equality between
/// the two is the claim.
#[test]
fn test_canvas_partial_reshape_matches_a_full_one_stream_for_stream() {
    baumhard::font::fonts::init();

    let (mut scene, ids) = canvas_fill_fixture_scene();
    let mut warm: Vec<ShapedSceneElement> = Vec::new();
    let cold = scene_shape_cache::refresh(&scene, &ids, &mut warm, ScenePassKind::Canvas);
    assert_eq!(
        (cold.walked, cold.reshaped),
        (6, 6),
        "two roots plus four leaves, none of them cached yet"
    );
    assert_eq!(
        canvas_fill_stream(&warm).len(),
        4,
        "each leaf authors one fill; the void roots author none"
    );

    // Move the *first* element in draw order — the one a re-append
    // would lift above all three others, which is the direction the
    // mindmap path's bug went.
    let first_leaf = {
        let tree = scene.tree(ids[0]).expect("tree registered");
        tree.root()
            .children(&tree.arena)
            .next()
            .expect("fixture tree has leaves")
    };
    {
        let tree = scene.tree_mut(ids[0]).expect("tree registered");
        let area = tree
            .arena
            .get_mut(first_leaf)
            .expect("leaf resolves")
            .get_mut()
            .glyph_area_mut()
            .expect("leaf is an area element");
        area.set_position((7.0, 11.0));
    }

    let partial = scene_shape_cache::refresh(&scene, &ids, &mut warm, ScenePassKind::Canvas);
    assert_eq!(
        (partial.walked, partial.reshaped),
        (6, 1),
        "one element moved, so one walk position may re-shape"
    );

    let mut full: Vec<ShapedSceneElement> = Vec::new();
    let reference = scene_shape_cache::refresh(&scene, &ids, &mut full, ScenePassKind::Canvas);
    assert_eq!(
        (reference.walked, reference.reshaped),
        (6, 6),
        "the reference pass must actually re-shape everything, or it is not a reference"
    );

    assert_eq!(
        canvas_fill_stream(&warm),
        canvas_fill_stream(&full),
        "a partially re-shaped pass must hand the rect pipeline the same fills, in the same \
         order, as a fully re-shaped one"
    );
    assert_eq!(
        canvas_text_stream(&warm),
        canvas_text_stream(&full),
        "and the same text areas, in the same order"
    );
}

/// One named single-field mutation of a `GlyphArea`, for the fill-
/// input table below. A `fn` pointer rather than a closure type so
/// every row shares one type.
type NamedFillMutation = (&'static str, fn(&mut GlyphArea));

/// Every input the background fill is derived from is an input the
/// reuse key can see.
///
/// The completeness half needs no test: `extract_background_rect`'s
/// whole signature is `(element, area, offset)`, and the key carries
/// the element's `unique_id`, the area, and the offset. What a test
/// is needed for is the other half — that each of the area fields it
/// *does* read is one the key is not blind to. `GlyphArea`'s derived
/// equality has a documented blind spot (`regions`, whose `BTreeSet`
/// identity ignores color) and a documented opt-out marker, so
/// "the key sees every area field" is a property that has already
/// been false once and is not assumed here.
///
/// Each row asserts in two steps, and the first is what makes the
/// second non-vacuous: the mutation must actually move the emitted
/// fill — asked of `extract_background_rect` itself, so a field that
/// turns out not to reach the fill fails here rather than passing
/// quietly — and only then must the key refuse to reuse. A row that
/// passed step one and failed step two is a fill left stale on
/// screen, which no test in this repo could otherwise see (§T8).
#[test]
fn test_canvas_fill_inputs_are_all_visible_to_the_reuse_key() {
    use baumhard::gfx_structs::area::EdgePadding;

    let base = canvas_fill_fixture_area("fill", 3.0);
    let mut scene = Scene::new();
    let id = scene.insert(Tree::new_non_indexed(), 0, Vec2::ZERO);

    let inputs: [NamedFillMutation; 6] = [
        ("background_color", |a| a.background_color = Some([1, 2, 3, 4])),
        ("background_padding", |a| {
            a.background_padding = EdgePadding::new(9.0, 8.0, 7.0, 6.0)
        }),
        ("position", |a| a.set_position((123.0, 456.0))),
        ("render_bounds", |a| a.set_bounds((321.0, 654.0))),
        ("shape", |a| a.shape = NodeShape::Rectangle),
        ("zoom_visibility", |a| {
            a.zoom_visibility = ZoomVisibility::try_new(Some(0.25), Some(4.0)).unwrap()
        }),
    ];

    let described_fill = |element: &GfxElement| {
        let area = element.glyph_area().expect("fixture is an area element");
        format!(
            "{:?}",
            super::tree_walker::extract_background_rect(element, area, Vec2::ZERO)
        )
    };

    for (name, mutate) in inputs {
        let element = GfxElement::new_area_non_indexed_with_id(base.clone(), 1, 1);
        let mut changed = element.clone();
        mutate(changed.glyph_area_mut().expect("fixture is an area element"));

        assert_ne!(
            described_fill(&changed),
            described_fill(&element),
            "`GlyphArea::{name}` is listed as an input of the background fill but changing it \
             leaves the emitted rect identical — the row proves nothing as written"
        );

        let cached = ShapedSceneElement::new(id, &element, Vec2::ZERO, Vec::new(), None);
        assert!(
            !cached.still_matches(id, &changed, Vec2::ZERO),
            "`GlyphArea::{name}` changed the background fill but the scene shape cache would \
             have kept the old one — the fill would render stale"
        );
    }
}

/// The overlay pass keeps no fills, and the canvas pass keeps them
/// all.
///
/// [`ScenePassKind`] is the only thing separating the two passes, so
/// it is worth one assertion that it separates them in the direction
/// each needs: an overlay entry carrying a fill would be memory
/// nothing draws, and a canvas entry missing one is a fill that
/// vanishes from the rect pipeline the moment its element re-shapes.
#[test]
fn test_only_the_canvas_pass_keeps_the_walker_s_background_fills() {
    baumhard::font::fonts::init();

    let (scene, ids) = canvas_fill_fixture_scene();

    let mut as_canvas: Vec<ShapedSceneElement> = Vec::new();
    scene_shape_cache::refresh(&scene, &ids, &mut as_canvas, ScenePassKind::Canvas);
    assert_eq!(
        canvas_fill_stream(&as_canvas).len(),
        4,
        "the canvas pass draws its fills, so it has to keep them"
    );

    let mut as_overlay: Vec<ShapedSceneElement> = Vec::new();
    scene_shape_cache::refresh(&scene, &ids, &mut as_overlay, ScenePassKind::Overlay);
    assert!(
        canvas_fill_stream(&as_overlay).is_empty(),
        "there is no screen-space rect pipeline, so an overlay fill is storage nothing reads"
    );
    assert_eq!(
        canvas_text_stream(&as_overlay),
        canvas_text_stream(&as_canvas),
        "the two passes differ in the fills alone — the text they shape is the same"
    );
}

/// A tree that shortens gives its walk positions back.
///
/// The pass indexes its cache by walk position, so an unregistered
/// or shortened tree has to leave nothing of itself behind: a stale
/// entry past the end of the walk would keep handing the render pass
/// buffers and a fill for an element that no longer exists. The
/// truncation that prevents it runs on every pass, including one
/// where nothing else changed at all.
#[test]
fn test_canvas_pass_drops_entries_a_shortened_tree_no_longer_walks() {
    baumhard::font::fonts::init();

    let (scene, ids) = canvas_fill_fixture_scene();
    let mut shaped: Vec<ShapedSceneElement> = Vec::new();
    scene_shape_cache::refresh(&scene, &ids, &mut shaped, ScenePassKind::Canvas);
    assert_eq!(shaped.len(), 6);

    // Drop the trailing tree entirely — the shape an unregistered
    // canvas role takes at the walk.
    let kept = &ids[..1];
    let after = scene_shape_cache::refresh(&scene, kept, &mut shaped, ScenePassKind::Canvas);
    assert_eq!(
        (after.walked, after.reshaped),
        (3, 0),
        "the surviving tree's elements are unchanged, so none of them re-shapes"
    );
    assert_eq!(
        shaped.len(),
        3,
        "the departed tree's entries must not outlive the walk that produced them"
    );
    assert_eq!(
        canvas_fill_stream(&shaped).len(),
        2,
        "and neither must their fills"
    );

    // Registering it again brings the positions back, freshly shaped.
    let restored = scene_shape_cache::refresh(&scene, &ids, &mut shaped, ScenePassKind::Canvas);
    assert_eq!(
        (restored.walked, restored.reshaped),
        (6, 3),
        "the three positions that came back are the three that re-shape"
    );
    let mut full: Vec<ShapedSceneElement> = Vec::new();
    scene_shape_cache::refresh(&scene, &ids, &mut full, ScenePassKind::Canvas);
    assert_eq!(canvas_fill_stream(&shaped), canvas_fill_stream(&full));
}

/// Every glyph offset cosmic-text lays out for `element`, across
/// every buffer the walker emits for it, in emission order.
///
/// The shaped output read straight off the shaper, without going
/// anywhere near the reuse key — which is what lets it serve as the
/// independent first step of
/// [`test_canvas_text_inputs_are_all_visible_to_the_reuse_key`].
fn shaped_glyph_offsets(element: &GfxElement) -> Vec<f32> {
    let mut font_system = baumhard::font::fonts::acquire_font_system_write("renderer::tests (glyph offsets)");
    let mut offsets = Vec::new();
    super::tree_walker::shape_one_element_into_buffers(
        element,
        Vec2::ZERO,
        &mut font_system,
        &mut |_unique_id, buffer| {
            offsets.extend(
                buffer
                    .buffer
                    .layout_runs()
                    .flat_map(|run| run.glyphs.iter().map(|glyph| glyph.x)),
            );
        },
        &mut |_rect| {},
    );
    offsets
}

/// One named single-field mutation of a `GlyphArea`, for the
/// text-input table below. Same shape as [`NamedFillMutation`], for
/// the other half of what the shaper emits.
type NamedTextMutation = (&'static str, fn(&mut GlyphArea));

/// Every input the *laid-out glyphs* depend on that no
/// `GlyphAreaField` mutator can reach is an input the reuse key can
/// see.
///
/// The area fields a mutator *can* write are covered by
/// [`test_scene_shape_cache_invalidates_on_every_writable_area_field`],
/// which iterates the strum-derived tag enum and is therefore
/// exhaustive over them by construction. Three fields sit outside
/// that loop because no variant writes them, and they split by which
/// of the walker's two outputs they feed: `background_color` and
/// `background_padding` feed the fill and are
/// [`test_canvas_fill_inputs_are_all_visible_to_the_reuse_key`]'s;
/// `align_center` feeds the *text*, and is this table's — the row
/// that had no home, and the only component of the whole reuse key
/// that could be removed with the suite still green.
///
/// It is a table with one row on purpose rather than a test named
/// after the field: the classification it encodes — mutator-reachable,
/// or fill input, or text input — is what the next unreachable field
/// has to be sorted into, and a table is where that sorting is
/// visible.
///
/// Two steps, and the first is what stops the second being vacuous:
/// the mutation must actually move the glyphs cosmic-text lays out,
/// asked of `shape_one_element_into_buffers` rather than of anything
/// in the cache, so a field that turns out not to reach the shaper
/// fails here instead of passing quietly. Only then must the key
/// refuse to reuse. A row that passes step one and fails step two is
/// text left stale on screen at the wrong alignment, which no test
/// in this repo could otherwise see (§T8).
#[test]
fn test_canvas_text_inputs_are_all_visible_to_the_reuse_key() {
    baumhard::font::fonts::init();

    // Deliberately *not* centered, and deliberately narrower than
    // its bounds: centering can only move a glyph that has room to
    // move, so a fixture whose text filled its box would make the
    // step-one assertion unprovable.
    let mut base = canvas_fill_fixture_area("fill", 3.0);
    base.align_center = false;

    let mut scene = Scene::new();
    let id = scene.insert(Tree::new_non_indexed(), 0, Vec2::ZERO);

    let inputs: [NamedTextMutation; 1] = [("align_center", |a| a.align_center = !a.align_center)];

    for (name, mutate) in inputs {
        let element = GfxElement::new_area_non_indexed_with_id(base.clone(), 1, 1);
        let mut changed = element.clone();
        mutate(changed.glyph_area_mut().expect("fixture is an area element"));

        let before = shaped_glyph_offsets(&element);
        let after = shaped_glyph_offsets(&changed);
        assert!(
            !before.is_empty(),
            "the fixture must lay out glyphs, or `{name}` has nothing to move"
        );
        assert_eq!(
            before.len(),
            after.len(),
            "`GlyphArea::{name}` changed how many glyphs are laid out, which this row does not \
             expect — re-read it before trusting the shift below"
        );
        let shift = before
            .iter()
            .zip(&after)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(
            shift > 1.0,
            "`GlyphArea::{name}` is listed as a text-shaping input but changing it moved no \
             glyph by more than {shift} — the row proves nothing as written"
        );

        let cached = ShapedSceneElement::new(id, &element, Vec2::ZERO, Vec::new(), None);
        assert!(
            !cached.still_matches(id, &changed, Vec2::ZERO),
            "`GlyphArea::{name}` re-laid-out the glyphs but the scene shape cache would have \
             kept the old buffers — the text would render at the old alignment"
        );
    }
}

// ====================================================================
// Draw order out of the shared flatteners
// ====================================================================
//
// `render.rs` reaches all four order-sensitive streams through
// exactly two functions — `background_fills(&canvas_scene_elements)`
// for the rect pipeline, and `text_buffers` over each of the two
// sub-scenes for the two glyphon passes. That consolidation is new
// with this pass (the canvas half used to be a flat `Vec` nobody had
// to flatten), and it makes these two functions the single place
// draw order is decided for both sub-scenes at once.
//
// Which is why the order is asserted here *absolutely*, against
// hand-written expectations, and not by comparing two passes with
// each other. `test_canvas_partial_reshape_matches_a_full_one_stream_for_stream`
// reads both of its sides through these same functions, so it is
// blind to any reordering they apply uniformly — reversing either
// one leaves that test, and the whole suite before these two, green.
// That is a real boundary of a comparison-shaped assertion rather
// than a flaw in it, and the answer is an assertion of a different
// shape, which is what these are.

/// The two flatteners hand the render pass their streams in walk
/// order, element for element.
///
/// The expectations are literals read off the fixture's own
/// construction — `unique_id` 1 and 2 in the first tree, 11 and 12
/// in the second, at four known anchors — not a second call to the
/// thing under test.
#[test]
fn test_the_draw_streams_come_out_in_walk_order() {
    baumhard::font::fonts::init();

    let (scene, ids) = canvas_fill_fixture_scene();
    let mut shaped: Vec<ShapedSceneElement> = Vec::new();
    scene_shape_cache::refresh(&scene, &ids, &mut shaped, ScenePassKind::Canvas);

    let fill_ids: Vec<usize> = scene_shape_cache::background_fills(&shaped)
        .map(|rect| rect.unique_id)
        .collect();
    assert_eq!(
        fill_ids,
        vec![1, 2, 11, 12],
        "the rect pipeline paints in index order, so the fills must arrive in the order the \
         scene walks them: both of the first tree's, then both of the second's"
    );

    assert_eq!(
        canvas_text_stream(&shaped),
        vec![(0.0, 4.0), (60.0, 4.0), (500.0, 4.0), (560.0, 4.0)],
        "and the glyphon pass is handed its text areas in the same order, at the anchors the \
         fixture put them at"
    );
}

/// Inside one element, every halo stamp is yielded before the glyph
/// it rings.
///
/// The walker emits them that way on purpose — glyphon draws later
/// text areas on top, so halos first is what puts them *behind*
/// (`tree_walker.rs`'s "Halos first" note) — and the flattener is
/// what carries that ordering to the render pass now. Reversing the
/// inner half of `text_buffers` inverts the halo of every outlined
/// element on both sub-scenes; nothing noticed before this test.
///
/// Read through emission offsets rather than glyph positions: the
/// main stamp is the one at `(0, 0)`, by the walker's own
/// bookkeeping, which
/// `test_walker_records_the_stamp_offset_of_every_emitted_buffer`
/// pins independently.
#[test]
fn test_the_text_stream_puts_every_halo_stamp_before_the_glyph_it_rings() {
    baumhard::font::fonts::init();

    let outline = OutlineStyle {
        color: [255, 0, 0, 255],
        px: 3.0,
    };
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    for (unique_id, ring) in [(1usize, true), (2usize, false)] {
        let mut area = canvas_fill_fixture_area(&format!("h{unique_id}"), 40.0 * unique_id as f32);
        area.outline = ring.then_some(outline);
        let leaf = tree
            .arena
            .new_node(GfxElement::new_area_non_indexed_with_id(area, 1, unique_id));
        tree.root.append(leaf, &mut tree.arena);
    }
    let mut scene = Scene::new();
    let id = scene.insert(tree, 0, Vec2::ZERO);

    let mut shaped: Vec<ShapedSceneElement> = Vec::new();
    scene_shape_cache::refresh(&scene, &[id], &mut shaped, ScenePassKind::Canvas);

    let stamps: Vec<(f32, f32)> = scene_shape_cache::text_buffers(&shaped)
        .map(|buffer| buffer.emission_offset)
        .collect();
    let halo_count = outline.offsets().count();
    assert_eq!(
        halo_count, 8,
        "the halo technique is eight stamps; this test's shape assumes it"
    );
    assert_eq!(
        stamps.len(),
        halo_count + 2,
        "one outlined element's stamps plus its glyph, then the plain element's glyph"
    );

    // The outlined element: eight rings, then the glyph at the
    // anchor. Every stamp before the last must be off-anchor, or
    // "halos first" is being read off an accident.
    assert!(
        stamps[..halo_count].iter().all(|off| *off != (0.0, 0.0)),
        "the first {halo_count} buffers of an outlined element are its halo stamps, so none of \
         them sits at the anchor: {stamps:?}"
    );
    assert_eq!(
        stamps[halo_count],
        (0.0, 0.0),
        "the glyph the halos ring is emitted last, so it draws on top of them: {stamps:?}"
    );
    assert_eq!(
        stamps[halo_count + 1],
        (0.0, 0.0),
        "and the plain element that follows contributes its single glyph after all of that"
    );

    // Element order across the stream, so a reversal that kept each
    // element's internals intact is caught here too.
    assert_eq!(
        canvas_text_stream(&shaped)
            .iter()
            .map(|pos| pos.0)
            .collect::<Vec<_>>(),
        vec![40.0; halo_count]
            .into_iter()
            .zip(outline.offsets())
            .map(|(anchor, (dx, _))| anchor + dx)
            .chain(std::iter::once(40.0))
            .chain(std::iter::once(80.0))
            .collect::<Vec<_>>(),
        "the outlined element's buffers all precede the plain one's"
    );
}

// ====================================================================
// Halo stamp bookkeeping
// ====================================================================

/// Every buffer the walker emits records the stamp offset it was
/// emitted at, and its `pos` is the area's anchor plus that offset
/// plus the tree offset.
///
/// `patch_drag_positions` re-derives `pos = new_anchor +
/// emission_offset` from nothing but this invariant, so if the
/// walker's stamp geometry and the recorded offset ever disagreed,
/// a drag would silently scatter (or collapse) an outlined
/// element's halo. Pinned here rather than at the patch, which is
/// on `Renderer` and so out of reach of a test (§T8).
#[test]
fn test_walker_records_the_stamp_offset_of_every_emitted_buffer() {
    baumhard::font::fonts::init();

    let anchor = Vec2::new(11.0, 23.0);
    let tree_offset = Vec2::new(100.0, -50.0);
    let outline = OutlineStyle {
        color: [255, 0, 0, 255],
        px: 3.0,
    };
    let mut area = GlyphArea::new_with_str("halo", 16.0, 18.0, anchor, Vec2::new(80.0, 24.0));
    area.outline = Some(outline);
    let element = GfxElement::new_area_non_indexed_with_id(area, 1, 1);

    let mut emitted: Vec<((f32, f32), (f32, f32))> = Vec::new();
    {
        let mut fs = baumhard::font::fonts::acquire_font_system_write("renderer::tests (halo stamps)");
        super::tree_walker::shape_one_element_into_buffers(
            &element,
            tree_offset,
            &mut fs,
            &mut |_uid, buffer| emitted.push((buffer.emission_offset, buffer.pos)),
            &mut |_rect| {},
        );
    }

    // Eight halo stamps then the main glyph, in that order — the
    // main glyph draws last so it sits on top.
    let expected: Vec<(f32, f32)> = outline.offsets().chain(std::iter::once((0.0, 0.0))).collect();
    assert_eq!(expected.len(), 9, "the halo technique is 8 stamps plus the glyph");
    assert_eq!(
        emitted.iter().map(|(off, _)| *off).collect::<Vec<_>>(),
        expected,
        "recorded stamp offsets must be the offsets the walker actually stamped at"
    );
    for (offset, pos) in &emitted {
        assert_eq!(
            *pos,
            (
                anchor.x + offset.0 + tree_offset.x,
                anchor.y + offset.1 + tree_offset.y
            ),
            "a buffer's position must be its anchor plus its recorded stamp offset"
        );
    }
}

// ====================================================================
// Background-fill draw order
// ====================================================================

use super::tree_buffers::BackgroundRectSlot;

/// A synthetic fill carrying nothing but the identity the draw-order
/// bookkeeping keys on. `Renderer::reshape_buffer_for` matches rects
/// by `unique_id` and never reads the geometry while re-slotting
/// them, so the rest is filler.
fn background_rect(unique_id: usize) -> NodeBackgroundRect {
    NodeBackgroundRect {
        position: Vec2::ZERO,
        size: Vec2::new(1.0, 1.0),
        color: [0, 0, 0, 255],
        shape_id: 0,
        zoom_visibility: ZoomVisibility::unbounded(),
        unique_id,
    }
}

fn background_rect_order(rects: &[NodeBackgroundRect]) -> Vec<usize> {
    rects.iter().map(|rect| rect.unique_id).collect()
}

/// Re-shaping one element leaves the background-fill draw order
/// alone.
///
/// `render.rs` paints `node_background_rects` in index order, so the
/// last entry covers every earlier one it overlaps.
/// `reshape_buffer_for` runs on every drained frame of a
/// section-resize drag and on every keystroke of a text edit; had it
/// re-appended the fill it just re-collected, the element being
/// edited would have spent the whole gesture painted over its
/// neighbors and snapped back on the next full rebuild — a defect no
/// assertion in this repo can see in pixels (§T8), which is why the
/// ordering is asserted on the list instead.
#[test]
fn test_reshaping_an_element_keeps_its_background_fill_in_draw_order() {
    let mut rects: Vec<NodeBackgroundRect> = (0..5).map(background_rect).collect();
    assert_eq!(background_rect_order(&rects), vec![0, 1, 2, 3, 4]);

    let mut slot = BackgroundRectSlot::take_over(&mut rects, 2);
    slot.push(background_rect(2));
    assert_eq!(
        background_rect_order(&rects),
        vec![0, 1, 2, 3, 4],
        "the re-collected fill belongs where the stale one was, not on top of the map"
    );

    // An element the walker emits more than one fill for keeps them
    // adjacent and in emission order — `push` advances rather than
    // inserting each one at the same index.
    let mut slot = BackgroundRectSlot::take_over(&mut rects, 1);
    slot.push(background_rect(1));
    slot.push(background_rect(101));
    assert_eq!(background_rect_order(&rects), vec![0, 1, 101, 2, 3, 4]);
}

/// An element that had no fill before the re-shape appends, because
/// there is no earlier slot to restore. The `unwrap_or(len)` branch.
#[test]
fn test_reshaping_an_element_that_gained_a_fill_appends_it() {
    let mut rects: Vec<NodeBackgroundRect> = (0..3).map(background_rect).collect();
    let mut slot = BackgroundRectSlot::take_over(&mut rects, 77);
    slot.push(background_rect(77));
    assert_eq!(background_rect_order(&rects), vec![0, 1, 2, 77]);
}

/// An element that *lost* its fill leaves no hole and no stale rect
/// — `take_over` with no `push` is the whole of the removal.
#[test]
fn test_reshaping_an_element_that_lost_its_fill_drops_it() {
    let mut rects: Vec<NodeBackgroundRect> = (0..3).map(background_rect).collect();
    let _ = BackgroundRectSlot::take_over(&mut rects, 1);
    assert_eq!(background_rect_order(&rects), vec![0, 2]);
}

// ====================================================================
// Glyph advance measurement
// ====================================================================

/// Shape one cluster through a scratch `cosmic_text::Buffer` and
/// report `(widest layout glyph, summed layout glyphs, glyph count)`.
///
/// Deliberately *not* routed through
/// [`baumhard::font::metric_cache::glyph_advance`]: that is the path
/// `measure_max_glyph_advance` now takes, and an expectation
/// computed from the code under test cannot see a change in that
/// code. This is the scratch-buffer shaping the subject used before
/// the metric cache, kept here as an independent second opinion —
/// the first element of the tuple is precisely the answer the old
/// implementation gave.
fn shape_cluster_advances(
    font_system: &mut baumhard::font::FontSystem,
    cluster: &str,
    font_size: f32,
) -> (f32, f32, usize) {
    use baumhard::font::{buffer, SHAPING_ADVANCED};

    let mut buf = buffer::create_square(font_system, font_size);
    buf.set_text(cluster, &Attrs::new(), SHAPING_ADVANCED, None);
    buf.shape_until_scroll(font_system, false);
    let (mut widest, mut summed, mut count) = (0.0f32, 0.0f32, 0usize);
    for run in buf.layout_runs() {
        for glyph in run.glyphs.iter() {
            widest = widest.max(glyph.w);
            summed += glyph.w;
            count += 1;
        }
    }
    (widest, summed, count)
}

/// `measure_max_glyph_advance` answers with the widest of the
/// clusters it was given, and falls back to the monospace
/// approximation only when nothing shaped at all.
///
/// The measurement runs through baumhard's `(face, size, cluster)`
/// metric cache rather than a scratch `Buffer` per call, and that
/// move carried a semantic correction with it: a cluster that lays
/// out as several glyphs is now worth the *sum* of their advances,
/// where the scratch-buffer version took the widest one inside the
/// cluster and so under-measured it. The third assertion below is
/// the one that can see that: its cluster is a Devanagari
/// consonant-plus-vowel-sign that lays out as two glyphs, so sum and
/// max are different numbers and only one of them can pass.
///
/// Every expectation here comes from [`shape_cluster_advances`],
/// which shapes independently rather than asking the metric cache —
/// otherwise the test would be quoting the subject back at itself.
#[test]
fn test_measure_max_glyph_advance_takes_the_widest_and_falls_back_on_nothing() {
    use baumhard::font::metrics::monospace_advance;

    baumhard::font::fonts::init();
    let size = 18.0;

    let mut fs = baumhard::font::fonts::acquire_font_system_write("renderer::tests (advance)");

    // A light horizontal rule and a full-block, which are not the
    // same width in any face this ships with. Both single-glyph, so
    // sum and max agree and this pair pins only the "max across the
    // set" half.
    let (narrow, wide) = ("\u{2500}", "\u{2588}");
    let (_, narrow_w, narrow_n) = shape_cluster_advances(&mut fs, narrow, size);
    let (_, wide_w, wide_n) = shape_cluster_advances(&mut fs, wide, size);
    assert!(narrow_w > 0.0 && wide_w > 0.0, "both clusters must shape");
    assert_eq!((narrow_n, wide_n), (1, 1), "both are single-glyph clusters");
    assert!(
        narrow_w < wide_w,
        "the two must differ, or 'widest' proves nothing"
    );
    assert_eq!(
        measure_max_glyph_advance(&mut fs, &[narrow, wide], size),
        wide_w,
        "the answer is the widest cluster in the set"
    );

    // Devanagari NA + vowel sign I: two layout glyphs, so the
    // cluster's summed advance and its widest single glyph are
    // different numbers. The scratch-buffer implementation answered
    // with the max and under-measured the cluster by the rest of it.
    let two_glyph = "\u{0928}\u{093F}";
    let (widest_in_cluster, summed, count) = shape_cluster_advances(&mut fs, two_glyph, size);
    assert!(
        count > 1 && summed > widest_in_cluster,
        "this cluster has to lay out as more than one glyph, or the routing below is untested: \
         {count} glyph(s), sum {summed}, widest {widest_in_cluster}"
    );
    assert_eq!(
        measure_max_glyph_advance(&mut fs, &[two_glyph], size),
        summed,
        "a multi-glyph cluster is worth the width it occupies, not the width of its widest piece"
    );

    // An empty cluster lays out no glyphs at all, so the max is
    // zero and the monospace approximation takes over — the branch
    // that keeps a tofu-only glyph set from collapsing the console
    // frame to zero columns.
    assert_eq!(
        measure_max_glyph_advance(&mut fs, &[""], size),
        monospace_advance(size),
        "a set that shapes to nothing must fall back rather than return zero"
    );
}

// ====================================================================
// WGSL <-> Rust shape lock-step
// ====================================================================
//
// `RECT_SHADER_WGSL` and `NodeShape::shader_id` are two halves of one
// wire format, and until this section existed the only thing holding
// them together was a pair of comments asking each side to remember
// the other. `do_shape_shader_ids_are_stable` (baumhard) pins the
// Rust half against its own constants; nothing read the shader. The
// checks below close that by reading the shader as text — no wgpu
// device, no GPU, so §T8 is satisfied by construction.

/// The WGSL constant each `NodeShape` variant expects the fragment
/// shader to declare.
///
/// Exhaustive on purpose, and that is the point: a new variant does
/// not compile until it names its shader constant here, which is step
/// 3 of the extension recipe in `baumhard::gfx_structs::shape`'s
/// module header. The mapping is not derivable — `Rectangle` is
/// `SHAPE_RECT`, not `SHAPE_RECTANGLE` — so it has to be written
/// somewhere, and a test-local match keeps it out of the production
/// surface while staying compiler-forced.
fn wgsl_shape_const_name(shape: baumhard::gfx_structs::shape::NodeShape) -> &'static str {
    use baumhard::gfx_structs::shape::NodeShape;
    match shape {
        NodeShape::Rectangle => "SHAPE_RECT",
        NodeShape::Ellipse => "SHAPE_ELLIPSE",
    }
}

/// Every `const <NAME>: u32 = <n>u;` declaration in `wgsl`, as
/// `(name, value)` pairs, plus the number of lines that opened with
/// `const` at all.
///
/// The second half of the return is what keeps the parser honest. A
/// scanner that skips what it cannot read turns a spelling change in
/// the shader into an empty result set, and an empty set satisfies
/// every "no orphan constant" assertion vacuously. Callers compare
/// the two numbers and refuse to proceed when they differ, so an
/// unreadable declaration is reported rather than dropped.
fn wgsl_u32_consts(wgsl: &str) -> (Vec<(String, u32)>, usize) {
    let mut parsed = Vec::new();
    let mut seen = 0usize;
    for line in wgsl.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("const ") else {
            continue;
        };
        seen += 1;
        let Some((name, value)) = rest.split_once(':') else {
            continue;
        };
        let Some(value) = value.trim().strip_prefix("u32") else {
            continue;
        };
        let Some(value) = value.trim().strip_prefix('=') else {
            continue;
        };
        let value = value.trim().trim_end_matches(';').trim().trim_end_matches('u');
        let Ok(n) = value.trim().parse::<u32>() else {
            continue;
        };
        parsed.push((name.trim().to_string(), n));
    }
    (parsed, seen)
}

/// The scalar kind a vertex attribute delivers, and that a WGSL
/// vertex input is declared to receive. WebGPU requires the two to
/// agree — a `Uint32` attribute cannot feed an `f32` input, and no
/// conversion happens at the boundary — so this is the axis a
/// "compatible format" check turns on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ShaderScalar {
    Float,
    Uint,
    Sint,
}

/// `(scalar kind, component count)` for the 32-bit vertex formats.
///
/// Those are the only ones the rect table can use: its vertices are
/// accumulated as a packed `Vec<f32>` (`RECT_VERTEX_FLOATS` slots of
/// four bytes each), so a `Unorm8x4` or a `Float16x2` would have to
/// straddle a slot. Any other format therefore panics rather than
/// returning `None` — the caller is asserting *about* the pair, and
/// a format that quietly mapped to nothing would take its assertion
/// down with it.
///
/// The component count is written out rather than derived because
/// `wgpu::VertexFormat` exposes no accessor for it, and it is then
/// checked against `VertexFormat::size()`, which wgpu does expose.
/// That is what keeps this table from being a second, drifting copy
/// of wgpu's own: a wrong count here fails on the next line instead
/// of silently widening what "compatible" means. Kind cannot be
/// cross-checked that way — `Float32` and `Uint32` are both four
/// bytes — so the kinds are the one thing here taken on trust, and
/// they are the part `wgpu::VertexFormat`'s own variant names state
/// unambiguously.
fn attribute_scalar_and_components(format: wgpu::VertexFormat) -> (ShaderScalar, u64) {
    use wgpu::VertexFormat as F;
    let (scalar, components) = match format {
        F::Float32 => (ShaderScalar::Float, 1),
        F::Float32x2 => (ShaderScalar::Float, 2),
        F::Float32x3 => (ShaderScalar::Float, 3),
        F::Float32x4 => (ShaderScalar::Float, 4),
        F::Uint32 => (ShaderScalar::Uint, 1),
        F::Uint32x2 => (ShaderScalar::Uint, 2),
        F::Uint32x3 => (ShaderScalar::Uint, 3),
        F::Uint32x4 => (ShaderScalar::Uint, 4),
        F::Sint32 => (ShaderScalar::Sint, 1),
        F::Sint32x2 => (ShaderScalar::Sint, 2),
        F::Sint32x3 => (ShaderScalar::Sint, 3),
        F::Sint32x4 => (ShaderScalar::Sint, 4),
        other => panic!(
            "{other:?} has no WGSL spelling recorded here, so this pin cannot say \
             whether it matches the `VsIn` field it feeds. The rect vertex is a \
             packed `Vec<f32>`, four bytes per slot, which is why only the 32-bit \
             formats are listed; a format outside that family needs its WGSL type \
             written into this match before it can appear in the table."
        ),
    };
    assert_eq!(
        format.size(),
        4 * components,
        "{format:?} is recorded here as {components} × 4 bytes but wgpu reports \
         {} — the component count above is wrong, and every format comparison \
         built on it is comparing the wrong width",
        format.size()
    );
    (scalar, components)
}

/// `(scalar kind, component count)` for the WGSL types a vertex
/// input may be declared as, or `None` for a spelling that is not one
/// of them.
///
/// `None` rather than a panic, unlike its Rust-side counterpart: the
/// caller has the field name and the struct it came from, so it can
/// say *which* declaration it could not read, which the type alone
/// cannot.
///
/// Both spellings of a vector are read: the constructed `vecN<f32>`
/// and WGSL's predeclared alias for it, `vecNf`. A shader rewritten
/// in the modern idiom is not a broken shader, and a reader that
/// returned `None` for `vec2f` would have its caller report the
/// declaration as one a vertex attribute cannot feed — an accusation
/// against correct code. The `h` family (`vec2h` = `vec2<f16>`) is
/// deliberately absent: `f16` needs an `enable` directive and pairs
/// with no format in `attribute_scalar_and_components`' table, so it
/// belongs in the caller's "this reader cannot read that" panic.
fn wgsl_scalar_and_components(ty: &str) -> Option<(ShaderScalar, u64)> {
    fn scalar(name: &str) -> Option<ShaderScalar> {
        match name {
            "f32" => Some(ShaderScalar::Float),
            "u32" => Some(ShaderScalar::Uint),
            "i32" => Some(ShaderScalar::Sint),
            _ => None,
        }
    }
    let ty = ty.trim();
    for n in 2..=4u64 {
        if let Some(rest) = ty.strip_prefix(&format!("vec{n}<")) {
            let inner = rest.strip_suffix('>')?;
            return scalar(inner.trim()).map(|s| (s, n));
        }
        let alias = match ty.strip_prefix(&format!("vec{n}")) {
            Some("f") => Some(ShaderScalar::Float),
            Some("u") => Some(ShaderScalar::Uint),
            Some("i") => Some(ShaderScalar::Sint),
            _ => None,
        };
        if let Some(alias) = alias {
            return Some((alias, n));
        }
    }
    scalar(ty).map(|s| (s, 1))
}

/// The `@location(N) name: type` fields of `struct <struct_name>` in
/// `wgsl`, in declaration order, as `(location, name, type)`.
///
/// Anchored to the one struct rather than scanning the shader. A
/// whole-shader `contains("@location(3) shape_id: f32")` is satisfied
/// by any struct that happens to declare a field of that name at that
/// location, and this shader declares `shape_id` twice — `VsIn` takes
/// it as an `f32` from the vertex stream, `VsOut` carries it on as a
/// `u32`. Today the two spellings differ, so nothing crosses; the
/// pin should not depend on that staying true.
///
/// Panics — never returns a short list — when the struct is missing,
/// when its body never closes, or when a field inside it is neither
/// `@builtin(…)` nor `@location(N) name: type`. Every caller asserts
/// *over* the list, and a field this reader dropped would shorten it
/// rather than redden anything: a table pin that skips the slot it
/// cannot read is exactly the pin that proves nothing.
fn wgsl_struct_locations(wgsl: &str, struct_name: &str) -> Vec<(u32, String, String)> {
    use baumhard::util::rust_source::braced_block_after;

    let header = format!("struct {struct_name}");
    let at = wgsl
        .find(&header)
        .unwrap_or_else(|| panic!("the rect shader must still declare `{header}`"));
    let tail = &wgsl[at..];
    // `braced_block_after` matches its header as a substring, which
    // would hand back `struct VsInstance`'s body for a `struct VsIn`
    // that no longer exists. Require the body to open right after the
    // name so a longer name is a loud miss rather than a quiet
    // redirection onto the wrong struct.
    assert!(
        tail[header.len()..].trim_start().starts_with('{'),
        "`{header}` is followed by `{}`, not by its body: the shader declares a \
         longer name that merely starts the same way, and reading that struct's \
         fields would pin the wrong one",
        tail[header.len()..]
            .trim_start()
            .chars()
            .take(24)
            .collect::<String>()
    );
    let block = braced_block_after(tail, &header)
        .unwrap_or_else(|| panic!("`{header}`'s body never closes in the rect shader"));
    // `braced_block_after` returns the braces as well; the fields are
    // what sits between them, comma-separated. A WGSL attribute may
    // carry an argument list, but none of the ones this shader can
    // write puts a comma inside it, so splitting on `,` cuts between
    // fields and nowhere else.
    let body = block
        .strip_prefix('{')
        .and_then(|b| b.strip_suffix('}'))
        .expect("braced_block_after returns a brace-delimited run");

    let mut fields = Vec::new();
    for field in body.split(',') {
        let field = field.trim();
        if field.is_empty() {
            continue;
        }
        // Peel the leading `@attribute(arg)` run rather than assuming
        // a lone `@location`. `VsIn` writes only that one today; the
        // shader's other located struct, `VsOut`, stacks
        // `@location(2) @interpolate(flat)`, and this reader takes a
        // struct name so that it can be pointed at either. Only
        // `"VsIn"` is passed to it today.
        let mut rest = field;
        let mut location: Option<u32> = None;
        let mut is_builtin = false;
        while let Some(after_at) = rest.strip_prefix('@') {
            let Some(open) = after_at.find('(') else {
                panic!("`{field}` in `{header}`: `@` attribute with no argument list — unreadable")
            };
            let Some(close) = after_at[open..].find(')') else {
                panic!("`{field}` in `{header}`: `@` attribute argument list never closes")
            };
            let name = after_at[..open].trim();
            let arg = after_at[open + 1..open + close].trim();
            match name {
                "location" => {
                    location = Some(arg.parse().unwrap_or_else(|_| {
                        panic!("`{field}` in `{header}`: `@location({arg})` is not a number")
                    }))
                }
                "builtin" => is_builtin = true,
                _ => {}
            }
            rest = after_at[open + close + 1..].trim_start();
        }
        if is_builtin {
            // A `@builtin` field is produced by the stage, not fed
            // from a vertex buffer, so it holds no slot in the table.
            continue;
        }
        let Some(location) = location else {
            panic!("`{field}` in `{header}` carries neither `@location` nor `@builtin`, so this reader cannot say which vertex slot feeds it")
        };
        let Some((name, ty)) = rest.split_once(':') else {
            panic!("`{field}` in `{header}` is not `name: type` after its attributes")
        };
        fields.push((location, name.trim().to_string(), ty.trim().to_string()));
    }

    assert!(
        !fields.is_empty(),
        "`{header}` parsed to no located fields at all — the reader and the shader \
         have drifted apart, and every comparison against this list would hold \
         over an empty set"
    );
    fields
}

/// The expression `Renderer::new` assigns to the descriptor field
/// `<field>`, as text with its internal whitespace collapsed.
///
/// Three facts about the rect pipeline are genuinely source-level —
/// *which named item* `Renderer::new` hands it: the attribute table,
/// the stride, and the shader source. Each is a `const` at module
/// scope that the rest of this file reads as a value, and each can
/// therefore be orphaned. A second const beside it, a slice of it, or
/// a literal written in its place all leave every value-level
/// assertion here holding over data no GPU is ever given.
///
/// **Equality, not `contains`.** A substring needle over
/// `attributes: &RECT_VERTEX_ATTRIBUTES` is satisfied by
/// `&RECT_VERTEX_ATTRIBUTES_2` and by `&RECT_VERTEX_ATTRIBUTES[..3]`
/// — the identifier-prefix collision `wgsl_struct_locations` guards
/// against on the WGSL side of this same file, and the one
/// `baumhard`'s `test_a_longer_identifier_does_not_satisfy_a_shorter_body`
/// exists for. Reading the whole expression and comparing it closes
/// both, and prints what it found when it does not match.
///
/// The expression ends at the first `,`, `}` or `]` after the field
/// name. That is where a struct-literal field ends whether or not it
/// carries a trailing comma, and it cuts *inside* any nested literal
/// written there — an inline array grown back in place of the const
/// reads as a truncated expression and fails the comparison rather
/// than satisfying it.
///
/// The cost, stated because it is a real one: this pins the
/// expression's spelling. Rewriting it in an equivalent form —
/// binding the table to a local first, or reaching the shader source
/// through a different `Cow` constructor — reddens here. That is the
/// loud direction, and the failure message says which text it wanted.
///
/// Panics when the field is absent, and when it appears more than
/// once: a second occurrence is a second descriptor, and this reader
/// would report only the first of them.
fn renderer_new_field_expression(new_body: &str, field: &str) -> String {
    let needle = format!("{field}:");
    let found = new_body.matches(needle.as_str()).count();
    assert_eq!(
        found, 1,
        "`Renderer::new` must write `{needle}` exactly once for this pin to know which \
         descriptor it read; it appears {found} times, and this reader would compare \
         only the first"
    );
    let at = new_body
        .find(needle.as_str())
        .expect("just counted one occurrence");
    let rest = &new_body[at + needle.len()..];
    let end = rest.find([',', '}', ']']).unwrap_or(rest.len());
    rest[..end].split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Require `name` to appear exactly once in `Renderer::new`'s body.
///
/// [`renderer_new_field_expression`] compares text, so it reads
/// `&RECT_VERTEX_ATTRIBUTES` as the module-level table on the
/// strength of the name alone. A `const RECT_VERTEX_ATTRIBUTES`
/// declared *inside* `Renderer::new` would shadow it: the text the
/// pin reads is unchanged while the name resolves to something the
/// test never saw, which is the one silent bypass left after the
/// three orphans were closed. Nothing under §T8 can resolve a name —
/// that needs a `Renderer`, and building one needs a GPU — but a
/// shadowing declaration has to be *written*, and writing it spends
/// the name a second time.
///
/// The input that makes this fail: add `const RECT_VERTEX_SIZE: u64
/// = 40;` inside `Renderer::new`. Each of the three names occurs
/// exactly once today, in the expression the pipeline is handed.
fn assert_name_is_not_shadowed_in_renderer_new(new_body: &str, name: &str) {
    let found = new_body.matches(name).count();
    let diagnosis = if found == 0 {
        "zero mentions means `Renderer::new` no longer hands this item to the pipeline at \
         all — the assertions below would then hold over data no GPU is given, which is the \
         orphan the expression pins exist to catch"
    } else {
        "more than one means either a shadowing declaration — which leaves the pinned text \
         intact while the name resolves to something this test never saw — or a second use \
         a text reader cannot tell apart from one"
    };
    assert_eq!(
        found, 1,
        "`{name}` must appear exactly once in `Renderer::new`, and appears {found} times. \
         The pins below read it as text: {diagnosis}."
    );
}

/// The per-vertex values the closure in `push_rect_ndc` pushes, in
/// order, exactly as `renderer/render.rs` spells them.
///
/// The one thing about the vertex layout that is genuinely a
/// source-level property: the slice is an anonymous
/// `[x, y, u, v, r, g, b, a, sid]`, so neither its width nor its
/// order is visible to any runtime assertion —
/// `main_rect_vertices` is a flat `Vec<f32>` that has forgotten where
/// one vertex ended.
///
/// The names are returned as well as counted, and the caller pins
/// them in order against a literal. **The price is any edit to this
/// slice, not only a rename**: growing the vertex — #147's own
/// third acceptance case, where a scalar is added and the WGSL
/// updated correctly — reddens here until the literal in the test is
/// updated too. That is one extra edit, it fails loudly, and the
/// message prints both lists side by side.
///
/// It is worth that price because the alternative was worse: reading
/// only the width let a reorder of this slice alone pass, which is
/// one line, breaks the layout silently, and was the input that
/// falsified this function's previous doc. A layout test that has to
/// be edited when the layout changes is the honest shape; a layout
/// test that stays green when the layout is scrambled is not.
/// What it still cannot prove is that `x` is a coordinate; see the
/// residual named in the test below.
///
/// Panics on anything it cannot read, and on a second
/// `extend_from_slice` in the same body: two pushes of different
/// widths is a layout this reader would report the first half of.
fn push_rect_ndc_slots() -> Vec<String> {
    use baumhard::util::rust_source::{braced_block_after, production_code};

    let source = production_code("src/application/renderer/render.rs");
    let body = braced_block_after(&source, "fn push_rect_ndc")
        .expect("`push_rect_ndc` must still be a braced item in renderer/render.rs");

    const PUSH: &str = "extend_from_slice(&[";
    assert_eq!(
        body.matches(PUSH).count(),
        1,
        "`push_rect_ndc` must build each vertex with exactly one `{PUSH}…]`; \
         found {} — with more than one, the width this reads is the width of \
         whichever came first",
        body.matches(PUSH).count()
    );
    let at = body.find(PUSH).expect("just counted one occurrence");
    let rest = &body[at + PUSH.len()..];
    let close = rest
        .find(']')
        .unwrap_or_else(|| panic!("the `{PUSH}` slice in `push_rect_ndc` never closes"));
    let slice = &rest[..close];
    assert!(
        !slice.contains('['),
        "the per-vertex slice in `push_rect_ndc` nests another `[`, which this \
         reader would miscount: `{slice}`"
    );
    slice
        .split(',')
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .map(str::to_string)
        .collect()
}

/// The pipeline's vertex-attribute table and the shader's `VsIn`
/// describe the same vertex: slot for slot, same `@location`, same
/// scalar kind and width, offsets tiling `RECT_VERTEX_SIZE` with no
/// gap and no overlap.
///
/// Fails when: `pos`'s and `uv`'s `shader_location` values are
/// swapped (both are `Float32x2`, so the resulting pipeline is
/// byte-identical and `createRenderPipeline` cannot object — every
/// quad would draw into the `[0, 1]²` corner of NDC); `shape_id`'s
/// format becomes `Uint32` while `VsIn` still declares it `f32`; an
/// attribute is added, removed or given an offset that is not where
/// the previous one ended; `RECT_VERTEX_SIZE` stops being the sum of
/// the formats' sizes; `RECT_VERTEX_FLOATS` stops being that sum in
/// four-byte slots; or `push_rect_ndc` writes a different number of
/// values per vertex, or writes them in a different order, than the
/// table describes.
///
/// It fails one step earlier, too, when `Renderer::new` stops handing
/// the pipeline the three module-level items this test reads as
/// values. All three are hoisted `const`s, so all three can be
/// orphaned while every assertion below goes on holding over data no
/// GPU is given, and each is pinned as the text of the expression
/// `Renderer::new` writes:
///
/// - `attributes: &RECT_VERTEX_ATTRIBUTES` — the table itself. A
///   decoy `RECT_VERTEX_ATTRIBUTES_2` beside it, or a
///   `&RECT_VERTEX_ATTRIBUTES[..3]`, is what a substring needle would
///   have accepted.
/// - `array_stride: RECT_VERTEX_SIZE` — the running sum below is
///   called the stride, and this is what makes that true. wgpu
///   validates only that a stride is a multiple of four and leaves
///   room for every attribute (`wgpu-core`'s `device/resource.rs`),
///   so an over-large literal builds cleanly and reads every vertex
///   after the first from the wrong byte.
/// - `source: … Cow::Borrowed(RECT_SHADER_WGSL)` — the shader module.
///   `VsIn` is read out of that const, and a decoy handed to
///   `create_shader_module` would leave this test, and every
///   assertion in
///   `test_every_node_shape_has_a_matching_wgsl_constant_and_case_arm`,
///   pinning a string the GPU never compiles.
///
/// The join is by `@location`, which is what the hardware joins on —
/// a `shader_location` names a slot, and the field carrying that
/// `@location` is the one it feeds. The Rust side has no field names
/// at all, so `shape_id` is identified here as "the field the
/// attribute at location 3 lands in" rather than as "the attribute at
/// byte offset 32", which is what the pin this replaces did: an
/// offset is a fact about today's layout, and adding one per-vertex
/// scalar ahead of `shape_id` used to leave that pin green on the
/// broken shader and red on the corrected one.
///
/// Offsets are then anchored by arithmetic rather than by literals:
/// each is required to be the running sum of the formats declared
/// before it, and the total to be `RECT_VERTEX_SIZE`. That is what
/// makes the table describe a *packed* vertex, and it is why the
/// stride needs no separate literal in this file.
///
/// ## What this deliberately does not claim
///
/// **That byte 0 is a position.** The chain pinned here runs from the
/// offsets outward: offsets → `shader_location` → `@location(N) name`
/// → the field the shader body reads. Its far end is anchored by the
/// WGSL names; its near end is not. `push_rect_ndc` writes an
/// anonymous `[x, y, u, v, r, g, b, a, sid]`, and nothing the
/// compiler can see relates that `x` to `VsIn.pos` — only a human
/// convention does.
///
/// That slice is pinned here element for element, so the reorder of
/// the slice alone is closed: `&[u, v, x, y, r, g, b, a, sid]`
/// reddens. The pin holds the names in order, not what the names
/// hold, and three inputs get past it — each of them swaps two
/// fields, and each was run against this test and observed green:
///
/// - reordering the closure's own parameter list, `|out, y, x, u, v|`
///   — one line, after which `x` names the coordinate `y` named;
/// - swapping two arguments at one of that closure's call sites,
///   `push(out, ry, lx, 0.0, 0.0)` — also one line;
/// - a reordering applied *consistently* to the table and `VsIn` —
///   attribute 0 becoming `uv` at location 1 while `VsIn` lists `uv`
///   first — two declarations, edited in step.
///
/// The first two are the residual as it really stands: one line is
/// still enough, which is why the slice order is now pinned rather
/// than left to the width alone. What the text pin costs in exchange
/// is a red run on a rename of `x`, `y`, `u`, `v` or `sid`;
/// `push_rect_ndc_slots` records that trade.
///
/// **That the shader body uses the fields it declares.** A `VsIn`
/// field bound correctly and then read nowhere in `vs_main` is a
/// value delivered and thrown away, and every assertion here would
/// still hold. That is
/// `test_every_node_shape_has_a_matching_wgsl_constant_and_case_arm`'s
/// concern — it asserts the forwarding of all three interpolated
/// fields — not this test's.
#[test]
fn test_every_rect_vertex_attribute_lands_in_the_vs_in_field_at_its_location() {
    use baumhard::util::rust_source::{braced_block_after, production_code, strip_comments};

    // Preconditions: the three consts read below are the three the
    // pipeline is built from. Each is read here as a value, so each
    // can be orphaned — a decoy beside it, a slice of it, or a
    // literal in its place — and every assertion after this point
    // would then be about dead data.
    let renderer = production_code("src/application/renderer/mod.rs");
    let new_body = braced_block_after(&renderer, "async fn new(")
        .expect("`Renderer::new` must still be a braced item in renderer/mod.rs");

    for name in ["RECT_VERTEX_ATTRIBUTES", "RECT_VERTEX_SIZE", "RECT_SHADER_WGSL"] {
        assert_name_is_not_shadowed_in_renderer_new(new_body, name);
    }

    let attributes_expression = renderer_new_field_expression(new_body, "attributes");
    assert_eq!(
        attributes_expression, "&RECT_VERTEX_ATTRIBUTES",
        "`Renderer::new` hands the rect pipeline `{attributes_expression}`, not \
         `&RECT_VERTEX_ATTRIBUTES`, so the table walked below is not the one the GPU \
         is given and every assertion here is about dead data. Compared whole rather \
         than as a substring: a decoy `RECT_VERTEX_ATTRIBUTES_2` with `pos` and `uv` \
         swapped, and `&RECT_VERTEX_ATTRIBUTES[..3]`, both satisfy a needle that only \
         asks for the name to appear"
    );

    let stride_expression = renderer_new_field_expression(new_body, "array_stride");
    assert_eq!(
        stride_expression, "RECT_VERTEX_SIZE",
        "the rect pipeline's `array_stride` is `{stride_expression}`, not \
         `RECT_VERTEX_SIZE`. The formats are summed below and that sum is called the \
         stride; wgpu asks only that a stride be a multiple of four and leave room \
         for every attribute, so a larger literal builds a pipeline that reads every \
         vertex after the first from the wrong byte, silently"
    );

    let shader_expression = renderer_new_field_expression(new_body, "source");
    assert_eq!(
        shader_expression, "wgpu::ShaderSource::Wgsl(Cow::Borrowed(RECT_SHADER_WGSL))",
        "the rect shader module is compiled from `{shader_expression}`, not from \
         `RECT_SHADER_WGSL`. `VsIn` is read out of that const just below, so a decoy \
         string handed to `create_shader_module` leaves this test — and the \
         forwarding assertions in \
         `test_every_node_shape_has_a_matching_wgsl_constant_and_case_arm` — pinning \
         a shader the GPU never compiles"
    );

    let wgsl = strip_comments(RECT_SHADER_WGSL);
    let fields = wgsl_struct_locations(&wgsl, "VsIn");

    assert_eq!(
        RECT_VERTEX_ATTRIBUTES.len(),
        fields.len(),
        "the pipeline declares {} vertex attributes but `VsIn` declares {} located \
         fields ({:?}): one side grew a slot the other does not know about",
        RECT_VERTEX_ATTRIBUTES.len(),
        fields.len(),
        fields.iter().map(|(l, n, _)| (l, n)).collect::<Vec<_>>()
    );

    let mut offset = 0u64;
    for (slot, (attribute, (location, name, ty))) in RECT_VERTEX_ATTRIBUTES.iter().zip(&fields).enumerate() {
        assert_eq!(
            attribute.shader_location, *location,
            "vertex slot {slot} is declared at shader_location {} but the {slot}th \
             field of `VsIn` is `{name}` at @location({location}). Two attributes of \
             the same format are interchangeable to the pipeline — swapping `pos`'s \
             and `uv`'s locations builds cleanly and draws every quad into the \
             [0, 1]² corner of NDC",
            attribute.shader_location
        );

        let (attribute_scalar, attribute_components) = attribute_scalar_and_components(attribute.format);
        let Some((field_scalar, field_components)) = wgsl_scalar_and_components(ty) else {
            panic!(
                "`VsIn.{name}` is declared `{ty}`, which `wgsl_scalar_and_components` has no \
                 scalar kind and width for. It reads `f32`, `u32`, `i32`, `vecN<…>` of those \
                 and their predeclared aliases `vecNf`/`vecNu`/`vecNi`; a declaration outside \
                 that set has to be written into that reader before this pin can say whether \
                 the attribute feeding it matches"
            )
        };
        assert_eq!(
            (attribute_scalar, attribute_components),
            (field_scalar, field_components),
            "vertex slot {slot} delivers {:?} ({attribute_scalar:?} × \
             {attribute_components}) into `VsIn.{name}: {ty}` ({field_scalar:?} × \
             {field_components}). WebGPU converts nothing at this boundary: the \
             kinds and the component counts have to match. `shape_id` is \
             deliberately `Float32` against `f32` — an integer vertex attribute is \
             a WebGL2 feature gate on some browsers — and that pairing is a match \
             here, not an exception to it",
            attribute.format
        );

        assert_eq!(
            attribute.offset, offset,
            "vertex slot {slot} (`{name}`) claims byte offset {} but the {offset} \
             bytes before it are exactly the formats of slots 0..{slot}. An offset \
             that is not the running sum leaves a hole or an overlap in a vertex \
             `push_rect_ndc` packs solid",
            attribute.offset
        );
        offset += attribute.format.size();
    }

    assert_eq!(
        offset, RECT_VERTEX_SIZE,
        "the attribute formats sum to {offset} bytes but `RECT_VERTEX_SIZE` — the \
         pipeline's `array_stride` — is {RECT_VERTEX_SIZE}: consecutive vertices \
         would be read from the wrong byte"
    );
    assert_eq!(
        RECT_VERTEX_SIZE,
        RECT_VERTEX_FLOATS as u64 * 4,
        "`RECT_VERTEX_FLOATS` is {RECT_VERTEX_FLOATS} four-byte slots = {} bytes, \
         against a stride of {RECT_VERTEX_SIZE}. The draw divides the length of a \
         flat `Vec<f32>` by that constant to get its vertex count, so the two \
         disagreeing means the wrong number of vertices is drawn",
        RECT_VERTEX_FLOATS * 4
    );
    let push_slots = push_rect_ndc_slots();
    assert_eq!(
        push_slots.len(),
        RECT_VERTEX_FLOATS,
        "`push_rect_ndc` writes {} values per vertex against `RECT_VERTEX_FLOATS` \
         = {RECT_VERTEX_FLOATS}: the CPU writer and the table describe vertices of \
         different widths, and every vertex after the first is read from the wrong \
         offset",
        push_slots.len()
    );
    assert_eq!(
        push_slots,
        ["x", "y", "u", "v", "r", "g", "b", "a", "sid"],
        "`push_rect_ndc` packs its vertex as {push_slots:?}. The order of that slice \
         is what decides that bytes 0..8 are the position rather than the texture \
         coordinate, and nothing the compiler sees ties it to the table above — so a \
         reorder of this one line swaps two fields, builds cleanly, and every other \
         assertion in this test stays green. Renaming one of these bindings reddens \
         here too; the names are the only handle there is"
    );

    let mut locations: Vec<u32> = RECT_VERTEX_ATTRIBUTES.iter().map(|a| a.shader_location).collect();
    locations.sort_unstable();
    let distinct = {
        let mut d = locations.clone();
        d.dedup();
        d.len()
    };
    assert_eq!(
        distinct,
        locations.len(),
        "two vertex attributes claim the same shader_location ({locations:?}). The \
         walk above pairs the two lists position by position, which makes them a \
         bijection only while the locations are distinct: duplicated on both \
         sides, two attributes feed one `VsIn` field and the walk still passes"
    );
}

/// Every `NodeShape` variant has a WGSL constant carrying exactly its
/// `shader_id`, the shader declares no shape constant no variant
/// claims, and every variant's fill is reachable from the `switch` —
/// by its own `case` arm, or, for the default variant, by the
/// `default` arm that also catches ids from a future build.
///
/// Fails when: a variant is added with no `SHAPE_*` constant, a
/// constant's value is renumbered on either side, the two names are
/// swapped so `SHAPE_RECT` carries the ellipse's id, a `case` arm is
/// deleted (every node of that shape would then silently draw as a
/// rectangle — the exact failure the module headers warn about), or
/// the `default` arm goes and unknown ids stop drawing at all.
///
/// It fails for that same reason one step earlier, too: both ends of
/// the `shape_id` wire *inside the shader* are asserted by name. A
/// `switch (0u)` in the fragment stage, or a vertex stage that writes
/// a constant into `out.shape_id`, leaves every arm above intact and
/// correct while every node on screen draws the default fill — a
/// shader in perfect lock-step with a Rust enum it no longer reads.
///
/// `shape_id`'s two neighbors are asserted the same way, because the
/// same mutation is available to them and one of them is just as
/// quiet: a vertex stage that stops forwarding `uv` discards every
/// ellipse — the SDF reads `uv` as the quad's local frame — while the
/// rectangles, which never look at it, go on drawing. Losing `pos`
/// is the loud one; it is asserted next to the others rather than
/// left to the eye.
///
/// Upstream of that, where the attribute enters `VsIn` at all, is
/// `test_every_rect_vertex_attribute_lands_in_the_vs_in_field_at_its_location`'s
/// concern and not this test's. Nothing here reads a
/// `shader_location`. That test also pins the one fact this one reads
/// by value and cannot check for itself: that `RECT_SHADER_WGSL` is
/// the string `Renderer::new` compiles. Without it a decoy shader
/// module would leave every assertion below asserting over a string
/// no GPU sees.
///
/// Comments are stripped before anything is matched, so a `case
/// SHAPE_X:` written in prose cannot satisfy an arm assertion — the
/// shader carries one such mention today, naming `SHAPE_RECT` in the
/// comment above the `default` arm.
#[test]
fn test_every_node_shape_has_a_matching_wgsl_constant_and_case_arm() {
    use baumhard::gfx_structs::shape::NodeShape;
    use baumhard::util::rust_source::strip_comments;
    use strum::IntoEnumIterator;

    let wgsl = strip_comments(RECT_SHADER_WGSL);
    let (consts, const_lines) = wgsl_u32_consts(&wgsl);

    // Preconditions. Without these the assertions below hold over an
    // empty set and prove nothing about the shader.
    assert!(
        !consts.is_empty(),
        "no `const <NAME>: u32 = <n>u;` found in the rect shader — the parser \
         and the shader have drifted apart, and every check below is vacuous"
    );
    assert_eq!(
        consts.len(),
        const_lines,
        "{} of the shader's {const_lines} `const` declarations did not parse; \
         a declaration this scanner cannot read must be reported, not skipped",
        const_lines - consts.len()
    );
    assert!(
        wgsl.contains("default: {"),
        "the fragment switch must keep a `default` arm: it is the fill for the \
         default shape and the safe landing for an id this build does not know"
    );
    // The arms are only worth checking if something selects between
    // them. Both ends of the wire, in the same literal-string style:
    // the vertex stage forwards the per-instance attribute, and the
    // fragment stage switches on what it forwarded.
    assert!(
        wgsl.contains("out.shape_id = u32(round(in.shape_id));"),
        "the vertex stage must forward the per-vertex `shape_id` attribute as exactly \
         `out.shape_id = u32(round(in.shape_id));` — a constant written here reaches \
         every fragment with the same value and the `case` arms below stop being \
         reachable. The trailing `;` is load-bearing: without it the needle is a \
         prefix, and `… * 0u;` satisfies it while sending every fragment the \
         rectangle id"
    );
    assert!(
        wgsl.contains("switch (in.shape_id)"),
        "the fragment switch must select on `in.shape_id`: switching on anything \
         else draws one fill for every node while each arm still matches its \
         `NodeShape` constant perfectly"
    );
    // `shape_id`'s two neighbors on the same journey. Binding an
    // attribute correctly and then not forwarding it is invisible to
    // the table pin, which stops at `VsIn`.
    assert!(
        wgsl.contains("out.uv = in.uv;"),
        "the vertex stage must forward `uv` as exactly `out.uv = in.uv;`: the ellipse \
         arm's SDF reads it as the quad's local frame, so anything written here that \
         is not the coordinate itself discards every ellipse while the rectangles — \
         which never look at `uv` — keep drawing. The needle is that whole statement, \
         terminator included, because `out.uv = in.uv * 0.0;` satisfies the prefix"
    );
    assert!(
        wgsl.contains("out.pos = vec4<f32>(in.pos, 0.0, 1.0);"),
        "the vertex stage must forward `pos` into clip space as exactly \
         `out.pos = vec4<f32>(in.pos, 0.0, 1.0);`. This is a text pin on that one \
         statement, so an equivalent spelling reddens here too and the fix is to \
         update the needle; what it exists to catch is the inequivalent one — a \
         constant, or `in.pos` scaled or transposed — which collapses every quad and \
         leaves nothing on the canvas with a background. The terminator is part of \
         the needle: `out.pos = vec4<f32>(in.pos, 0.0, 1.0) * 0.0;` satisfies the \
         prefix and collapses every quad to a point"
    );

    let mut claimed: Vec<&str> = Vec::new();
    let mut ids: Vec<u32> = Vec::new();
    for shape in NodeShape::iter() {
        let name = wgsl_shape_const_name(shape);
        claimed.push(name);
        ids.push(shape.shader_id());

        let declared = consts
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("{shape:?} expects the shader to declare `{name}`"));
        assert_eq!(
            declared.1,
            shape.shader_id(),
            "`{name}` is {} in the shader but {shape:?}::shader_id() is {}",
            declared.1,
            shape.shader_id()
        );

        if shape == NodeShape::default() {
            // The default variant is what the `default` arm draws;
            // asserting a `case` arm for it would fail against the
            // shader as designed.
            continue;
        }
        assert!(
            wgsl.contains(&format!("case {name}:")),
            "{shape:?} has a shader id but no `case {name}:` arm — every node \
             of this shape would draw as the default shape instead"
        );
    }

    ids.sort_unstable();
    let distinct = {
        let mut d = ids.clone();
        d.dedup();
        d.len()
    };
    assert_eq!(
        distinct,
        ids.len(),
        "two NodeShape variants share a shader id, so one of them cannot be \
         selected: {ids:?}"
    );

    for (name, value) in &consts {
        if !name.starts_with("SHAPE_") {
            continue;
        }
        assert!(
            claimed.contains(&name.as_str()),
            "the shader declares `{name}` = {value} but no NodeShape variant \
             claims it — either a variant was removed without its constant, or \
             wgsl_shape_const_name is a name behind"
        );
    }
}

// ====================================================================
// Selection-highlight single-sourcing
// ====================================================================

/// Every reading of the selection cyan carries one RGB triple. The
/// four items compared here are declared in four different modules
/// — baumhard's hex literal, the document layer's float quad, and
/// the renderer's two cosmic-text constants — and the assertion
/// reads each of them as a value rather than recomputing any of
/// them, so it is the re-inlining of a literal that turns it red.
///
/// That is exactly the input that made it red before this test
/// existed: the tree carried `#00E5FF` (green 229),
/// `[0.0, 0.9, 1.0, 1.0]` (229.5) and `Color::rgba(0, 230, 255, ..)`
/// twice (230) — three byte values for one affordance, each
/// individually plausible and none of them reachable from the
/// others.
///
/// Alpha is compared separately and only for the two opaque forms:
/// the rubber band's reduced alpha is the one thing about its color
/// that is genuinely its own.
#[test]
fn test_every_selection_highlight_form_carries_one_rgb_triple() {
    let from_hex = baumhard::util::color_conversion::hex_to_color(baumhard::mindmap::SELECTION_HIGHLIGHT_HEX)
        .expect("the canonical hex literal must parse");
    let hex_rgb = [from_hex.rgba[0], from_hex.rgba[1], from_hex.rgba[2]];

    let doc =
        baumhard::util::color_conversion::convert_f32_to_u8(&crate::application::document::HIGHLIGHT_COLOR);
    assert_eq!(
        [doc[0], doc[1], doc[2]],
        hex_rgb,
        "document::HIGHLIGHT_COLOR quantises to a different cyan than the model emits"
    );
    assert_eq!(
        doc[3],
        baumhard::util::color::VAL_MAX,
        "the selection tint is opaque"
    );

    let status = MODE_STATUS_COLOR;
    assert_eq!(
        [status.r(), status.g(), status.b()],
        hex_rgb,
        "the mode-status row is a different cyan than the selection tint"
    );
    assert_eq!(
        status.a(),
        baumhard::util::color::VAL_MAX,
        "the status row is opaque"
    );

    let band = super::selection_overlay::SELECTION_RECT_COLOR;
    assert_eq!(
        [band.r(), band.g(), band.b()],
        hex_rgb,
        "the rubber band is a different cyan than the selection tint"
    );
    assert!(
        band.a() < baumhard::util::color::VAL_MAX,
        "the rubber band is deliberately translucent; an opaque one means the alpha was lost \
         on the way through the shared definition"
    );
}
