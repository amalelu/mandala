use super::*;

#[test]
fn cull_accepts_center_of_viewport() {
    let vp_min = Vec2::new(0.0, 0.0);
    let vp_max = Vec2::new(100.0, 100.0);
    assert!(glyph_position_in_viewport(50.0, 50.0, vp_min, vp_max, 12.0));
}

#[test]
fn cull_accepts_glyph_just_inside_edge() {
    let vp_min = Vec2::new(0.0, 0.0);
    let vp_max = Vec2::new(100.0, 100.0);
    // Right on the boundary — inclusive on both sides.
    assert!(glyph_position_in_viewport(0.0, 0.0, vp_min, vp_max, 0.0));
    assert!(glyph_position_in_viewport(100.0, 100.0, vp_min, vp_max, 0.0));
}

#[test]
fn cull_rejects_far_off_screen() {
    let vp_min = Vec2::new(0.0, 0.0);
    let vp_max = Vec2::new(100.0, 100.0);
    // Way off to the right, far beyond any reasonable margin.
    assert!(!glyph_position_in_viewport(10_000.0, 50.0, vp_min, vp_max, 12.0));
    assert!(!glyph_position_in_viewport(50.0, 10_000.0, vp_min, vp_max, 12.0));
    assert!(!glyph_position_in_viewport(-10_000.0, 50.0, vp_min, vp_max, 12.0));
    assert!(!glyph_position_in_viewport(50.0, -10_000.0, vp_min, vp_max, 12.0));
}

#[test]
fn cull_margin_extends_visible_rect() {
    let vp_min = Vec2::new(0.0, 0.0);
    let vp_max = Vec2::new(100.0, 100.0);
    // Just outside the rect but within the margin — should be included
    // so there's no visible popping at viewport edges during pan.
    assert!(glyph_position_in_viewport(-10.0, 50.0, vp_min, vp_max, 12.0));
    assert!(glyph_position_in_viewport(110.0, 50.0, vp_min, vp_max, 12.0));
    assert!(glyph_position_in_viewport(50.0, -10.0, vp_min, vp_max, 12.0));
    assert!(glyph_position_in_viewport(50.0, 110.0, vp_min, vp_max, 12.0));
}

#[test]
fn cull_rejects_just_beyond_margin() {
    let vp_min = Vec2::new(0.0, 0.0);
    let vp_max = Vec2::new(100.0, 100.0);
    let margin = 12.0;
    // One epsilon past the padded boundary → excluded.
    assert!(!glyph_position_in_viewport(
        vp_max.x + margin + 0.001,
        50.0,
        vp_min,
        vp_max,
        margin
    ));
    assert!(!glyph_position_in_viewport(
        vp_min.x - margin - 0.001,
        50.0,
        vp_min,
        vp_max,
        margin
    ));
}

#[test]
fn cull_handles_non_origin_viewport() {
    // Viewport not at origin (pan offset).
    let vp_min = Vec2::new(500.0, 1000.0);
    let vp_max = Vec2::new(700.0, 1200.0);
    assert!(glyph_position_in_viewport(600.0, 1100.0, vp_min, vp_max, 12.0));
    assert!(!glyph_position_in_viewport(100.0, 100.0, vp_min, vp_max, 12.0));
}

#[test]
fn cull_kills_most_glyphs_on_a_very_long_edge() {
    // A 20,000 canvas-unit connection, sampled every 15 units
    // (default spacing), one endpoint at origin, the other at
    // (20000, 0). Viewport is the first 400x400 canvas units. With
    // font_size=12 margin, we should keep glyphs whose x is in
    // [-12, 412] — roughly 28 of ~1334 samples. Pins that the cull
    // actually culls on long cross-links instead of silently shaping
    // every sample.
    let vp_min = Vec2::new(0.0, 0.0);
    let vp_max = Vec2::new(400.0, 400.0);
    let margin = 12.0;
    let total = 1334;
    let kept = (0..total)
        .filter(|&i| {
            let x = i as f32 * 15.0;
            glyph_position_in_viewport(x, 0.0, vp_min, vp_max, margin)
        })
        .count();
    // Expect well under 5% retained.
    assert!(kept < total / 20, "kept {} of {}, expected < {}", kept, total, total / 20);
    // And at least a few — it's not zero.
    assert!(kept > 10, "kept {} of {}, expected at least 10", kept, total);
}

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
            },
            ConsoleOverlayLine {
                text: "commands:".to_string(),
                kind: ConsoleOverlayLineKind::Output,
            },
        ],
        completions: vec![
            ConsoleOverlayCompletion {
                text: "top".to_string(),
                hint: None,
            },
        ],
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
    });
    let mut g_two = g_one.clone();
    g_two.scrollback.push(ConsoleOverlayLine {
        text: "two".into(),
        kind: ConsoleOverlayLineKind::Output,
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
        },
        ConsoleOverlayLine {
            text: "two".into(),
            kind: ConsoleOverlayLineKind::Output,
        },
    ];
    g.completions = vec![ConsoleOverlayCompletion {
        text: "help".into(),
        hint: None,
    }];
    g.selected_completion = Some(0);
    let layout = compute_console_frame_layout(&g, 1920.0, 1080.0);

    let content_top = layout.top + layout.font_size + layout.inner_padding;
    let last_completion_end = content_top
        + layout.row_height * (layout.scrollback_rows + layout.completion_rows) as f32;
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
        let mut fs =
            baumhard::font::fonts::acquire_font_system_write("renderer::tests (overlay tree)");
        build_console_overlay_tree(&g_a, &layout_a, &mut fs)
    };
    let mutator = {
        let mut fs =
            baumhard::font::fonts::acquire_font_system_write("renderer::tests (overlay mutator)");
        build_console_overlay_mutator(&g_b, &layout_b, &mut fs)
    };
    mutator.apply_to(&mut tree);

    let expected = {
        let mut fs = baumhard::font::fonts::acquire_font_system_write(
            "renderer::tests (overlay areas expected)",
        );
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
    }];
    let layout_one = compute_console_frame_layout(&g_one, 1280.0, 720.0);

    let mut g_two = sample_console_geometry();
    g_two.scrollback = vec![
        ConsoleOverlayLine {
            text: "> help".into(),
            kind: ConsoleOverlayLineKind::Input,
        },
        ConsoleOverlayLine {
            text: "new output line".into(),
            kind: ConsoleOverlayLineKind::Output,
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
/// abort (§7). Pin the degraded behaviour: artificially shorten the
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
        })
        .collect();
    g.completions = Vec::new();
    let layout = compute_console_frame_layout(&g, 1280.0, 720.0);
    assert!(layout.scrollback_rows >= 1, "layout must reserve rows");

    // Evict scrollback so layout.scrollback_rows > geometry.scrollback.len().
    g.scrollback.clear();

    let areas = {
        let mut fs = baumhard::font::fonts::acquire_font_system_write(
            "renderer::tests (scrollback degrade)",
        );
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
        })
        .collect();
    g.selected_completion = Some(0);
    let layout = compute_console_frame_layout(&g, 1280.0, 720.0);
    assert!(layout.completion_rows >= 1, "layout must reserve rows");

    g.completions.clear();
    g.selected_completion = None;

    let areas = {
        let mut fs = baumhard::font::fonts::acquire_font_system_write(
            "renderer::tests (completion degrade)",
        );
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
    assert_eq!(
        clamp_surface_size_to_gpu_limit(10_000, 4096, 8192),
        (8192, 4096)
    );
    // Height over, width fine.
    assert_eq!(
        clamp_surface_size_to_gpu_limit(4096, 10_000, 8192),
        (4096, 8192)
    );
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
        zoom_visibility: ZoomVisibility { min: Some(1.0), max: Some(2.0) },
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
    };

    for z in [0.05_f32, 0.5, 1.0, 2.5, 5.0] {
        camera.zoom = z;
        assert!(rect.visible_at(&camera), "unbounded window must render at zoom {z}");
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
        zoom_visibility: ZoomVisibility { min: Some(1.0), max: Some(2.0) },
    };
    assert!(!rect.visible_at(&camera), "off-viewport rect must be culled regardless of zoom window");
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
    let expected_with_sentinel = (sentinel + 1_000u128 * (FPS_WINDOW as u128 - 1))
        / FPS_WINDOW as u128;
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
