// SPDX-License-Identifier: MPL-2.0

// -----------------------------------------------------------------
// Tests
//
// Border string generation is on every scene-rebuild hot path: one
// call to `top_border` / `bottom_border` per framed node, per frame.
// The loops look trivial today but are easy to break in ways that
// either quietly misalign corners or accidentally go quadratic. These
// tests double as perf regression guards.
// -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::mindmap::border::{border_run_specs, resolve_border_style, BorderGlyphSet, BorderStyle};
    use crate::util::grapheme_chad::count_grapheme_clusters;

    /// `border_run_specs` produces eight runs — four fill rails
    /// then four corners — in the contractually required channel
    /// order (top=1, bottom=2, left=3, right=4, then TL=5, TR=6,
    /// BL=7, BR=8) and assigns palette offsets that sweep
    /// continuously around the rectangle. The invariant the three border
    /// pipelines (initial-build tree, in-place mutator tree,
    /// section-frame tree) all rely on.
    #[test]
    fn border_run_specs_channels_and_palette_offsets() {
        let style = BorderStyle::default_with_color("#ffffff");
        let specs = border_run_specs(&style, (10.0, 20.0), (100.0, 50.0));
        // Plan revision 4: returns 8 specs (4 rails + 4 corners).
        assert_eq!(specs.len(), 8, "expected 8 specs (4 rails + 4 corners)");
        assert_eq!(specs[0].channel, 1, "top fill channel");
        assert_eq!(specs[1].channel, 2, "bottom fill channel");
        assert_eq!(specs[2].channel, 3, "left fill channel");
        assert_eq!(specs[3].channel, 4, "right fill channel");
        assert_eq!(specs[4].channel, 5, "TL corner channel");
        assert_eq!(specs[5].channel, 6, "TR corner channel");
        assert_eq!(specs[6].channel, 7, "BL corner channel");
        assert_eq!(specs[7].channel, 8, "BR corner channel");
        // TL palette offset is 0 (sweep starts at top-left corner).
        assert_eq!(specs[4].palette_offset, 0, "TL corner palette offset");
    }

    /// `border_run_specs` handles a font-PINNED style
    /// (`font_name = Some(registered family)`) without deadlocking.
    /// Every other border test uses the default `font_name: None`,
    /// so this is the only coverage of the `Some(face)` path through
    /// `border_run_specs_with` — face resolution,
    /// `face_family_name_for_pin`, and the guard-threaded
    /// `glyph_ink_with` — and of the wrapper's warm-before-guard
    /// step (`fonts::ensure_warm`) that keeps that path
    /// re-entrancy-free (issue P0-06). These tests do not call
    /// `fonts::init()`, so the wrapper's own warm is what makes the
    /// guarded face lookup safe.
    #[test]
    fn border_run_specs_with_font_pin_does_not_deadlock() {
        // `loaded_families_iter` yields names that round-trip through
        // `app_font_by_family` (see fonts_tests), so this pins a real
        // resolvable face — exercising the guarded pin lookup.
        let family = crate::font::fonts::loaded_families_iter()
            .next()
            .expect("at least one bundled family")
            .to_string();
        let mut style = BorderStyle::default_with_color("#ffffff");
        style.font_name = Some(family);
        let specs = border_run_specs(&style, (0.0, 0.0), (200.0, 80.0));
        assert_eq!(specs.len(), 8, "font-pinned border still emits 8 specs");
    }

    /// Each spec's `cluster_count` is consistent with
    /// `count_grapheme_clusters(text)` — the field exists so
    /// consumers handing the spec to `build_border_regions`
    /// don't re-walk the string, but the contract is that the
    /// pre-counted value matches a fresh count.
    #[test]
    fn border_run_specs_cluster_count_matches_text() {
        let style = BorderStyle::default_with_color("#ffffff");
        let specs = border_run_specs(&style, (0.0, 0.0), (200.0, 80.0));
        for spec in &specs {
            assert_eq!(
                spec.cluster_count,
                count_grapheme_clusters(&spec.text),
                "spec channel {} cluster_count mismatch",
                spec.channel
            );
        }
    }

    /// Whole-PR follow-up (plan revision 3): vertical-rail
    /// bounds are now `row_count × line_height` exactly, where
    /// `row_count = floor(node_height / line_height)`. The rail
    /// fits inside `node.height` rather than overflowing — no
    /// clip, no overshoot. position.y = `node_pos.1` (rail starts
    /// at the node's top edge; corner glyphs are in the top/bottom
    /// rails, which extend slightly above/below).
    #[test]
    fn border_run_specs_vertical_rail_fits_node_height() {
        let style = BorderStyle::default_with_color("#ffffff");
        // Testament Atomic-repeat dimensions verbatim.
        let specs = border_run_specs(&style, (0.0, 0.0), (360.0, 110.0));

        let left = &specs[2];
        let right = &specs[3];

        // Position.y is below the top corner (corner ink-height
        // offsets the rail downward). Must be > 0 (node top).
        assert!(
            left.position.1 > 0.0 && left.position.1 < 50.0,
            "left rail position.y = {} should sit below top corner (in (0, ~25] px)",
            left.position.1
        );
        assert!(
            right.position.1 > 0.0 && right.position.1 < 50.0,
            "right rail position.y = {}",
            right.position.1
        );

        // Rail position.y + bounds.1 must fit within node height
        // (so the rail doesn't overshoot the bottom corner).
        assert!(
            left.position.1 + left.bounds.1 <= 110.0,
            "left rail (y={} + h={}) = {} must fit within node height 110",
            left.position.1,
            left.bounds.1,
            left.position.1 + left.bounds.1
        );
        assert!(
            right.position.1 + right.bounds.1 <= 110.0,
            "right rail (y={} + h={}) = {} must fit within node height 110",
            right.position.1,
            right.bounds.1,
            right.position.1 + right.bounds.1
        );
    }

    /// Plan revision 4: corners are emitted as separate specs
    /// at exact node-corner positions. The right corners must
    /// land such that their right edge = node's right edge.
    #[test]
    fn border_run_specs_corners_land_at_exact_node_corners() {
        let style = BorderStyle::default_with_color("#ffffff");
        let specs = border_run_specs(&style, (0.0, 0.0), (360.0, 110.0));
        // Channels 5-8 are corners in order TL, TR, BL, BR.
        let tl = &specs[4];
        let tr = &specs[5];
        let bl = &specs[6];
        let br = &specs[7];

        // TL.position.x = node.x = 0.
        assert!(
            (tl.position.0 - 0.0).abs() < 0.01,
            "TL position.x = {} expected 0.0",
            tl.position.0
        );
        // TR.position.x + TR.bounds.0 should equal node.x + node.width.
        // bounds.0 is at least the corner advance, may include slack.
        // Looser invariant: TR's left edge < node.right, and TR's
        // bounds end at node.right ± small tolerance.
        let tr_right_edge = tr.position.0 + tr.bounds.0;
        assert!(
            (tr_right_edge - 360.0).abs() < 5.0,
            "TR right edge = {} expected ≈ 360.0",
            tr_right_edge
        );
        // BL.position.x = 0.
        assert!(
            (bl.position.0 - 0.0).abs() < 0.01,
            "BL position.x = {} expected 0.0",
            bl.position.0
        );
        // BR right edge ≈ 360.
        let br_right_edge = br.position.0 + br.bounds.0;
        assert!(
            (br_right_edge - 360.0).abs() < 5.0,
            "BR right edge = {} expected ≈ 360.0",
            br_right_edge
        );
    }

    /// Whole-PR (plan revision 3): horizontal-rail width tiles
    /// the node width WITHOUT overshooting. The rendered fill
    /// stops at `floor(available / cluster_width)` copies — the
    /// last sub-cluster gap before the right corner stays blank
    /// rather than producing a clipped overflow.
    ///
    /// This is the alignment defect users see: pre-fix
    /// `char_count = ceil(node_width / (font_size × 0.6)) + 2`
    /// overcounted, the rendered fill overshot the right corner,
    /// and the visible result was a misaligned rail.
    #[test]
    fn border_run_specs_horizontal_rail_does_not_overshoot_node_width() {
        let style = BorderStyle::default_with_color("#ffffff");
        // Testament Atomic-repeat dimensions verbatim.
        let specs = border_run_specs(&style, (0.0, 0.0), (360.0, 110.0));
        let top = &specs[0];
        let bottom = &specs[1];

        // Top + bottom fill rails position.x is INSIDE the node
        // (offset by tl_w / bl_w — the rail spans between corners).
        assert!(
            top.position.0 > 0.0 && top.position.0 < 50.0,
            "top fill position.x = {} should sit just after TL corner (~5-30 px)",
            top.position.0
        );
        assert!(
            bottom.position.0 > 0.0 && bottom.position.0 < 50.0,
            "bottom fill position.x = {} should sit just after BL corner",
            bottom.position.0
        );

        // Rail position.x + bounds.0 must fit within node width
        // (so the fill doesn't overshoot the right corner).
        assert!(
            top.position.0 + top.bounds.0 <= 360.0,
            "top rail (x={} + w={}) = {} must fit within node width 360",
            top.position.0,
            top.bounds.0,
            top.position.0 + top.bounds.0
        );
        assert!(
            bottom.position.0 + bottom.bounds.0 <= 360.0,
            "bottom rail (x={} + w={}) = {} must fit within node width 360",
            bottom.position.0,
            bottom.bounds.0,
            bottom.position.0 + bottom.bounds.0
        );

        // bounds.0 should be reasonably close to (node_width - 2*corner_w)
        // — the rail should USE most of the available space.
        assert!(
            top.bounds.0 >= 360.0 * 0.7,
            "top rail bounds.0 = {} should use ≥ 70% of node width {} (otherwise the rail leaves a huge gap)",
            top.bounds.0,
            360.0
        );
    }

    /// Plan revision 4: vertical rail row count is derived from
    /// MEASURED ink heights of the corner glyphs and the rail's
    /// fill glyph. The contract is no longer a fixed `floor()`
    /// over `node.height / font_size`; it's `floor(side_avail
    /// / line_height_pt)` where `side_avail = node.height -
    /// top_corner_h - bottom_corner_h`. The rail must always
    /// fit within the corner-bounded vertical region.
    #[test]
    fn border_run_specs_left_rail_fits_between_corners() {
        let style = BorderStyle::default_with_color("#ffffff");
        let specs = border_run_specs(&style, (0.0, 0.0), (100.0, 100.0));
        let left = &specs[2];
        // position.y > 0 (below top corner), bounds.1 such that
        // position.y + bounds.1 <= node.height.
        assert!(
            left.position.1 > 0.0,
            "left rail position.y = {} should be > 0 (below top corner)",
            left.position.1
        );
        assert!(
            left.position.1 + left.bounds.1 <= 100.0,
            "left rail (y={} + h={}) must fit within node.height 100",
            left.position.1,
            left.bounds.1
        );
        // At least 1 row of fill rendered (rail isn't empty).
        let left_rows = left.text.matches('\n').count() + 1;
        assert!(
            left_rows >= 1,
            "left rail should render ≥ 1 row, got {}",
            left_rows
        );
    }

    /// The light preset's top border at width 5 is corners + 3 fill
    /// characters. Structural invariant: first char is `top_left`, last
    /// is `top_right`, all middle chars equal `top`.
    #[test]
    fn test_top_border_light_basic_shape() {
        let glyphs = BorderGlyphSet::box_drawing_light();
        let border = glyphs.top_border(5);
        assert_eq!(border, "\u{250C}\u{2500}\u{2500}\u{2500}\u{2510}");
        let chars: Vec<char> = border.chars().collect();
        assert_eq!(chars.len(), 5);
        assert_eq!(chars[0], glyphs.top_left);
        assert_eq!(chars[4], glyphs.top_right);
        for c in &chars[1..4] {
            assert_eq!(*c, glyphs.top);
        }
    }

    /// Widths below 2 have no room for both corners, so the function
    /// returns an empty string. Guards the early-return branch.
    #[test]
    fn test_top_border_width_under_two_is_empty() {
        let glyphs = BorderGlyphSet::box_drawing_light();
        assert_eq!(glyphs.top_border(0), "");
        assert_eq!(glyphs.top_border(1), "");
        assert_eq!(glyphs.bottom_border(0), "");
        assert_eq!(glyphs.bottom_border(1), "");
    }

    /// The bottom border must use the `bottom_*` corners, not the
    /// `top_*` corners. Copy-paste slip guard.
    #[test]
    fn test_bottom_border_uses_bottom_corners() {
        let glyphs = BorderGlyphSet::box_drawing_light();
        let border = glyphs.bottom_border(4);
        let chars: Vec<char> = border.chars().collect();
        assert_eq!(chars.len(), 4);
        assert_eq!(chars[0], glyphs.bottom_left);
        assert_eq!(chars[3], glyphs.bottom_right);
        assert_ne!(chars[0], glyphs.top_left);
        assert_ne!(chars[3], glyphs.top_right);
    }

    /// Every preset must produce a length-N string for width N ≥ 2 on
    /// both top and bottom. Catches a preset accidentally missing a
    /// glyph field (serde would default it to `'\0'`, which would still
    /// produce a length-N string — so also spot-check the first char is
    /// non-null).
    #[test]
    fn test_all_four_presets_produce_non_empty_borders() {
        let presets = [
            BorderGlyphSet::box_drawing_light(),
            BorderGlyphSet::box_drawing_heavy(),
            BorderGlyphSet::box_drawing_double(),
            BorderGlyphSet::box_drawing_rounded(),
        ];
        for glyphs in &presets {
            let top = glyphs.top_border(6);
            let bottom = glyphs.bottom_border(6);
            assert_eq!(top.chars().count(), 6);
            assert_eq!(bottom.chars().count(), 6);
            assert_ne!(top.chars().next().unwrap(), '\0');
            assert_ne!(bottom.chars().next().unwrap(), '\0');
            assert_ne!(glyphs.left_char(), '\0');
            assert_ne!(glyphs.right_char(), '\0');
        }
    }

    /// `top_border(10_000)` must succeed without panic and produce
    /// exactly 10,000 characters. Guards against accidental integer
    /// overflow on `char_width.saturating_sub(2)` or a quadratic
    /// string-growth refactor.
    #[test]
    fn test_top_border_large_width_no_panic() {
        let glyphs = BorderGlyphSet::box_drawing_light();
        let border = glyphs.top_border(10_000);
        assert_eq!(border.chars().count(), 10_000);
        // First and last are still corners, not middle fill.
        let chars: Vec<char> = border.chars().collect();
        assert_eq!(chars[0], glyphs.top_left);
        assert_eq!(chars[9_999], glyphs.top_right);
    }

    /// `side_border(rows)` emits exactly `rows` glyphs separated by
    /// newlines — one glyph per logical row. Guards against an
    /// off-by-one on the trailing newline.
    #[test]
    fn test_side_border_exact_row_count() {
        let glyphs = BorderGlyphSet::box_drawing_rounded();
        assert_eq!(glyphs.side_border(0), "");
        assert_eq!(glyphs.side_border(1), "│");
        assert_eq!(glyphs.side_border(3), "│\n│\n│");
        // Each of the 3 rows is exactly the `left` char, no more.
        let border = glyphs.side_border(5);
        assert_eq!(border.lines().count(), 5);
        for line in border.lines() {
            assert_eq!(line.chars().count(), 1);
            assert_eq!(line.chars().next().unwrap(), glyphs.left);
        }
    }

    /// Right-side helper uses `self.right`; for the rounded preset
    /// that's the same as `left`, but the API keeps them distinct so
    /// callers don't have to know.
    #[test]
    fn test_right_side_border_uses_right_glyph() {
        let glyphs = BorderGlyphSet::box_drawing_rounded();
        let border = glyphs.right_side_border(4);
        for line in border.lines() {
            assert_eq!(line.chars().next().unwrap(), glyphs.right);
        }
    }

    /// `BorderStyle::default_with_color` is what the scene builder
    /// constructs for every framed node. Spot-check its fields.
    #[test]
    fn test_border_style_default_with_color() {
        let style = BorderStyle::default_with_color("#ff0000");
        assert_eq!(style.color, "#ff0000");
        assert!(style.visible);
        // Default preset is light — its corners extend to the cell
        // edges so they connect cleanly with the side glyphs.
        assert_eq!(
            style.glyph_set.top_left,
            BorderGlyphSet::box_drawing_light().top_left
        );
        assert_eq!(style.font_name, None);
    }

    /// `resolve_border_style(None, None, ...)` is the most common
    /// path: a framed node with no per-node `GlyphBorderConfig` and
    /// a canvas with no `default_border` falls all the way through
    /// the cascade to the hardcoded preset / font / size defaults.
    /// Pin that the corners and side patterns land on the light
    /// preset so a future flip of the default doesn't silently
    /// change the rendered look for every map that lacks an
    /// explicit border config.
    #[test]
    fn resolve_border_style_with_no_overrides_uses_light_preset() {
        let style = resolve_border_style(None, None, None, "#abcdef");
        let expected = BorderGlyphSet::box_drawing_light();
        assert_eq!(style.corners.top_left, expected.top_left.to_string());
        assert_eq!(style.corners.top_right, expected.top_right.to_string());
        assert_eq!(style.corners.bottom_left, expected.bottom_left.to_string());
        assert_eq!(style.corners.bottom_right, expected.bottom_right.to_string());
        assert_eq!(style.color, "#abcdef");
        assert_eq!(style.font_size_pt, 14.0);
        assert!(style.visible);
    }

    /// Every name the schema accepts has a non-empty description.
    /// The console's `border preset=` completion renders one row per
    /// entry of `BORDER_PRESETS` and takes its hint from
    /// `border_preset_hint`; before the hint moved into
    /// `PRESET_TABLE`, the app-side lookup was a hand-maintained
    /// `match` with a `_ => ""` arm, so a fifth preset would have
    /// completed with a blank description and nobody would have
    /// noticed. This pins the coverage even though the tuple shape
    /// now makes omitting one a compile error.
    #[test]
    fn every_border_preset_has_a_non_empty_hint() {
        use crate::mindmap::border::{border_preset_hint, BORDER_PRESETS};
        for preset in BORDER_PRESETS {
            let hint = border_preset_hint(preset)
                .unwrap_or_else(|| panic!("preset '{}' has no completion hint", preset));
            assert!(!hint.is_empty(), "preset '{}' has an empty hint", preset);
        }
    }

    /// Hint lookup is case-insensitive, matching `preset_glyph_set`
    /// — the schema accepts `"Rounded"` as readily as `"rounded"`,
    /// and a completion row must not lose its description over
    /// casing.
    #[test]
    fn border_preset_hint_is_case_insensitive() {
        use crate::mindmap::border::border_preset_hint;
        assert_eq!(border_preset_hint("ROUNDED"), border_preset_hint("rounded"));
        assert_eq!(border_preset_hint("Custom"), border_preset_hint("custom"));
    }

    /// An unknown preset has no hint rather than a blank one, so a
    /// caller can tell "no such preset" from "described as nothing".
    #[test]
    fn border_preset_hint_unknown_name_is_none() {
        use crate::mindmap::border::border_preset_hint;
        assert!(border_preset_hint("no-such-preset").is_none());
    }

    /// The three views of the preset table agree: the name list,
    /// the pair list a vocabulary is built from, and the lookup.
    ///
    /// The failing input is a `BORDER_PRESET_ROWS` whose order or
    /// content drifts from `BORDER_PRESETS` — which is reachable
    /// the moment either stops deriving from the other, and which
    /// a consumer that indexes one and looks up in the other would
    /// read as a silently mismatched hint rather than as an error.
    #[test]
    fn border_preset_rows_agree_with_the_name_list_and_the_lookup() {
        use crate::mindmap::border::{border_preset_hint, BORDER_PRESETS, BORDER_PRESET_ROWS};
        assert_eq!(BORDER_PRESET_ROWS.len(), BORDER_PRESETS.len());
        for (i, (name, hint)) in BORDER_PRESET_ROWS.iter().enumerate() {
            assert_eq!(*name, BORDER_PRESETS[i], "row {i} names a different preset");
            assert_eq!(
                border_preset_hint(name),
                Some(*hint),
                "row {i} hint differs from the lookup"
            );
        }
    }

    /// **Parity guard for the clip-AABB fast path.**
    /// `resolve_border_font_size_pt` exists so `node_clip_aabbs`
    /// can skip a whole `BorderStyle` allocation for the one `f32`
    /// it needs. If the two ever disagree, connection glyphs clip
    /// against a different frame thickness than the one drawn —
    /// a silent visual defect with no other test to catch it.
    ///
    /// Covers every arm of the cascade: per-node override, canvas
    /// default fall-through, per-node winning over canvas default,
    /// the unset floor, and (because the cheap resolver skips both)
    /// independence from the preset and the frame color.
    #[test]
    fn resolve_border_font_size_pt_matches_resolve_border_style() {
        use crate::mindmap::border::resolve_border_font_size_pt;
        use crate::mindmap::model::GlyphBorderConfig;

        fn cfg(preset: &str, size: f32) -> GlyphBorderConfig {
            GlyphBorderConfig {
                preset: preset.to_string(),
                font: None,
                font_size_pt: size,
                color: None,
                glyphs: None,
                padding: 4.0,
                color_palette: None,
                color_palette_field: None,
            }
        }

        let node = cfg("heavy", 22.0);
        let canvas = cfg("double", 9.5);
        let cases: [(Option<&GlyphBorderConfig>, Option<&GlyphBorderConfig>); 4] = [
            (Some(&node), Some(&canvas)),
            (Some(&node), None),
            (None, Some(&canvas)),
            (None, None),
        ];
        // Frame color must not move the answer — the cheap
        // resolver doesn't take one.
        for frame_color in ["#ffffff", "#123456", ""] {
            for (per_node, canvas_default) in cases {
                let full = resolve_border_style(per_node, canvas_default, None, frame_color).font_size_pt;
                let cheap = resolve_border_font_size_pt(per_node, canvas_default);
                assert!(
                    (full - cheap).abs() < f32::EPSILON,
                    "cascade drift for ({:?}, {:?}, {frame_color:?}): full {full} vs cheap {cheap}",
                    per_node.map(|c| c.font_size_pt),
                    canvas_default.map(|c| c.font_size_pt),
                );
            }
        }
        // And the documented floor.
        assert!((resolve_border_font_size_pt(None, None) - 14.0).abs() < f32::EPSILON);
    }
}

/// **The border side caps, which are the sampler's twin.** Both take
/// an authored numerator over an authored denominator and size an
/// allocation with the quotient, so both need the same pinning: that
/// the cap binds rather than falling through, and that it binds in
/// *both* units — a grapheme is not a byte, and a side pattern is an
/// author-supplied string.
#[test]
fn test_border_side_fill_is_capped_in_both_units() {
    use crate::mindmap::border::{MAX_BORDER_SIDE_BYTES, MAX_BORDER_SIDE_GLYPHS};

    // Grapheme ceiling: a single-byte cluster against an enormous
    // width. `available / cluster_w` is astronomically larger than
    // the cap, so the cap is what decides.
    assert_eq!(
        crate::mindmap::border::fill_copies(1.0e9, 0.001, 1, 1, 0),
        MAX_BORDER_SIDE_GLYPHS,
        "the grapheme ceiling must bind, not fall through to a degenerate count"
    );

    // Byte ceiling: one cluster that is one grapheme but many bytes.
    // The grapheme cap alone would allow 100k copies; the byte cap
    // must bind first.
    let fat_cluster_bytes = 64;
    assert_eq!(
        crate::mindmap::border::fill_copies(1.0e9, 0.001, 1, fat_cluster_bytes, 0),
        MAX_BORDER_SIDE_BYTES / fat_cluster_bytes,
        "a grapheme is not a byte — the byte ceiling must bind when it is the tighter one"
    );

    // Non-finite and non-positive inputs yield no copies rather than
    // a saturating cast into the push loop.
    assert_eq!(crate::mindmap::border::fill_copies(f32::NAN, 1.0, 1, 1, 0), 0);
    assert_eq!(crate::mindmap::border::fill_copies(1.0e9, f32::NAN, 1, 1, 0), 0);
    assert_eq!(crate::mindmap::border::fill_copies(1.0e9, 0.0, 1, 1, 0), 0);
    assert_eq!(crate::mindmap::border::fill_copies(-5.0, 1.0, 1, 1, 0), 0);

    // An ordinary rail is untouched by any of it.
    assert_eq!(crate::mindmap::border::fill_copies(100.0, 10.0, 1, 1, 0), 10);
}

/// **The rail cycles the whole pattern, so the byte ceiling has to
/// price the whole pattern.** `fill_copies` is correct and its unit
/// test above passes it correct arguments; the defect this pins was
/// at the *call site*, which measured only the pattern's first
/// grapheme. A rail whose first cluster is one byte and whose second
/// is a thousand then set its ceiling from the one byte and went on
/// to emit the thousand, once per row — a 3 KB map produced a
/// hundred-megabyte rail, and the factor scales with the authored
/// cluster.
///
/// This drives `border_run_specs`, not `fill_copies`, because that
/// is where the arguments are chosen and where the bug lived.
#[cfg(test)]
mod side_rail_byte_ceiling_tests {
    use crate::mindmap::border::{border_run_specs, BorderStyle, MAX_BORDER_SIDE_BYTES};
    use crate::mindmap::border_pattern::SidePattern;

    #[test]
    fn test_multi_cluster_side_rail_respects_the_byte_ceiling() {
        let mut style = BorderStyle::default_with_color("#ffffff");
        // Two clusters: a one-byte "A", then a base plus 1,000
        // combining acutes. Reading only the first gives 1 byte per
        // row; the rail actually averages ~1,000.
        let fat = format!("A{}", "\u{0301}".repeat(1_000));
        style.side_patterns.left = SidePattern::AtomicRepeat {
            cluster: vec!["A".to_string(), fat],
        };

        // A tall node, so the row count is bounded by a ceiling
        // rather than by the available space.
        let specs = border_run_specs(&style, (0.0, 0.0), (100.0, 1_000_000.0));
        let left = specs
            .iter()
            .find(|s| s.channel == 3)
            .expect("channel 3 is the left rail");

        assert!(
            left.text.len() <= MAX_BORDER_SIDE_BYTES,
            "left rail is {} bytes, over the {MAX_BORDER_SIDE_BYTES} ceiling",
            left.text.len()
        );
    }

    /// The mean is what the rail actually spends per row, and it is
    /// read from every cluster rather than the first.
    #[test]
    fn test_bytes_per_row_reads_the_whole_pattern() {
        // "A" (1) + "BB" (2) + "CCC" (3) = 6 bytes over 3 clusters.
        let pattern = SidePattern::AtomicRepeat {
            cluster: vec!["A".to_string(), "BB".to_string(), "CCC".to_string()],
        };
        assert_eq!(
            crate::mindmap::border::side_pattern_bytes_per_row(&pattern),
            2,
            "6 bytes over 3 clusters is 2 per row — not the first cluster's 1"
        );

        // Rounds up, so a ceiling never under-charges a row.
        let uneven = SidePattern::AtomicRepeat {
            cluster: vec!["A".to_string(), "BBBB".to_string()],
        };
        assert_eq!(
            crate::mindmap::border::side_pattern_bytes_per_row(&uneven),
            3,
            "5 bytes over 2 clusters rounds up to 3"
        );

        // An empty pattern charges one byte rather than dividing by
        // zero or reporting a free row.
        let empty = SidePattern::AtomicRepeat { cluster: vec![] };
        assert_eq!(crate::mindmap::border::side_pattern_bytes_per_row(&empty), 1);
    }
}

/// **A ceiling that prices only the repeating part is not a ceiling.**
///
/// `SidePattern::render` writes a `PrefixFillSuffix` pattern's prefix
/// and suffix once each around the repeated fill. Both were outside
/// the byte ceiling entirely: `fill_copies` never saw them, so no
/// value of `MAX_BORDER_SIDE_BYTES` could constrain them and an
/// authored prefix rode straight through. Charging the fixed part
/// first is what makes the constant bound the emitted string.
#[cfg(test)]
mod fixed_part_ceiling_tests {
    use crate::mindmap::border::{fill_copies, side_pattern_fixed_bytes, MAX_BORDER_SIDE_BYTES};
    use crate::mindmap::border_pattern::SidePattern;

    #[test]
    fn test_fixed_bytes_reads_prefix_and_suffix() {
        // "AB(c)DE" — prefix AB, fill c, suffix DE.
        let p = SidePattern::parse("AB(c)DE").expect("parses");
        assert_eq!(
            side_pattern_fixed_bytes(&p),
            4,
            "prefix and suffix are two clusters each, one byte apiece"
        );

        // An atomic pattern has no fixed part.
        let atomic = SidePattern::parse("+=#").expect("parses");
        assert_eq!(side_pattern_fixed_bytes(&atomic), 0);
    }

    #[test]
    fn test_fixed_bytes_are_charged_before_the_repeats() {
        // With no fixed part the full ceiling is available.
        let free = fill_copies(1.0e9, 0.001, 1, 1, 0);
        // With the ceiling already spent, no repeats are allowed —
        // and it saturates rather than underflowing.
        let spent = fill_copies(1.0e9, 0.001, 1, 1, MAX_BORDER_SIDE_BYTES);
        let over = fill_copies(1.0e9, 0.001, 1, 1, MAX_BORDER_SIDE_BYTES * 4);
        assert!(free > 0, "a pattern with no fixed part still repeats");
        assert_eq!(spent, 0, "a fixed part at the ceiling leaves no repeats");
        assert_eq!(over, 0, "a fixed part past the ceiling must not underflow");

        // Half the ceiling spent leaves half the byte budget. Priced
        // at 64 bytes per cluster so the *byte* ceiling is the one
        // that binds — at one byte per cluster the grapheme ceiling
        // (100,000) is tighter and this would measure that instead.
        const FAT: usize = 64;
        let whole = fill_copies(1.0e9, 0.001, 1, FAT, 0);
        let half = fill_copies(1.0e9, 0.001, 1, FAT, MAX_BORDER_SIDE_BYTES / 2);
        assert_eq!(
            whole,
            MAX_BORDER_SIDE_BYTES / FAT,
            "the byte ceiling must be the binding one"
        );
        assert_eq!(
            half,
            (MAX_BORDER_SIDE_BYTES - MAX_BORDER_SIDE_BYTES / 2) / FAT,
            "the remaining budget is what decides the repeat count"
        );
    }
}

/// **The ceiling must bound the string that is actually emitted.**
///
/// Two overshoots survived the first byte-ceiling pass, both because
/// they add bytes *after* `fill_copies` has done its arithmetic: the
/// greedy partial-cluster fill appends whole clusters checking only
/// width, and the vertical rails are newline-joined after rendering.
/// Measured at 1,049,598 bytes against a 1,048,576 ceiling on the
/// horizontal path, and up to ~100 KB of separators on the vertical
/// one.
///
/// This drives `border_run_specs` on the worst shape that still
/// passes every loader ceiling and asserts on the emitted text, not
/// on the arithmetic that produced it.
#[cfg(test)]
mod emitted_rail_ceiling_tests {
    use crate::mindmap::border::{border_run_specs, BorderStyle, MAX_BORDER_SIDE_BYTES};
    use crate::mindmap::border_pattern::SidePattern;

    #[test]
    fn test_every_emitted_rail_respects_the_byte_ceiling() {
        // One grapheme cluster of 1023 bytes — inside both authored
        // ceilings (64 clusters, 1024 bytes), so a map carrying this
        // loads. The old code then emitted 1,022 bytes past the rail
        // ceiling on top of it.
        let fat = format!("A{}", "\u{0301}".repeat(511));
        assert!(
            fat.len() <= crate::mindmap::model::validate::MAX_BORDER_GLYPH_BYTES,
            "the fixture must be a glyph the loader accepts, or it tests the wrong thing"
        );

        let mut style = BorderStyle::default_with_color("#ffffff");
        for slot in [
            &mut style.side_patterns.top,
            &mut style.side_patterns.bottom,
            &mut style.side_patterns.left,
            &mut style.side_patterns.right,
        ] {
            *slot = SidePattern::AtomicRepeat {
                cluster: vec![fat.clone()],
            };
        }

        // MAX_NODE_AXIS on both axes — the largest node the loader
        // accepts, so the row and column counts are as large as an
        // authored map can make them.
        let specs = border_run_specs(&style, (0.0, 0.0), (1_000_000.0, 1_000_000.0));
        for spec in &specs {
            assert!(
                spec.text.len() <= MAX_BORDER_SIDE_BYTES,
                "channel {} emitted {} bytes, over the {MAX_BORDER_SIDE_BYTES} ceiling",
                spec.channel,
                spec.text.len()
            );
        }
    }

    /// The vertical rails specifically, because their overshoot is
    /// the newline separators rather than a partial cluster — a
    /// different mechanism that the horizontal case would not catch.
    #[test]
    fn test_vertical_rail_separators_are_inside_the_ceiling() {
        let mut style = BorderStyle::default_with_color("#ffffff");
        // Fat clusters, so the *byte* ceiling is what caps the row
        // count. That is the only configuration where the
        // separators can push the result over: with single-byte
        // clusters the grapheme ceiling (100,000 rows) binds first
        // and the separators top out around 200 KB, comfortably
        // inside 1 MiB. A single-byte fixture here passes whether
        // or not the separator is charged, and proves nothing.
        let fat = format!("A{}", "\u{0301}".repeat(511));
        style.side_patterns.left = SidePattern::AtomicRepeat { cluster: vec![fat] };
        let specs = border_run_specs(&style, (0.0, 0.0), (100.0, 1_000_000.0));
        let left = specs
            .iter()
            .find(|s| s.channel == 3)
            .expect("channel 3 is the left rail");
        assert!(
            left.text.len() <= MAX_BORDER_SIDE_BYTES,
            "left rail emitted {} bytes including separators, over the ceiling",
            left.text.len()
        );
    }
}
