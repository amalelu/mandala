//! Layout-then-dynamic apply composition: ensures the dynamic
//! mutator's slim per-cell delta lands on the same observable state
//! as a fresh build, when applied on top of a layout-built tree.

use baumhard::core::primitives::Applicable;
use baumhard::gfx_structs::area::GlyphArea;
use baumhard::gfx_structs::tree::BranchChannel;

use super::fixtures::picker_sample_geometry;
use crate::application::color_picker::{compute_color_picker_layout, PickerHit};
use crate::application::color_picker_overlay::picker_glyph_areas::{
    build_color_picker_overlay_dynamic_mutator, build_color_picker_overlay_mutator,
    build_color_picker_overlay_tree, picker_glyph_areas,
};

/// Layout-then-dynamic round-trip: building a tree at state A,
/// applying the layout mutator for state B, then the dynamic
/// mutator for state B, must end at the same observable state as a
/// fresh `picker_glyph_areas(B)`. Pins the contract that the dynamic
/// phase composes correctly on top of a layout-built tree — without
/// it the per-frame perf path could silently leave stale color or
/// scale values on cells the dynamic spec doesn't list.
///
/// Why dynamic-after-layout (not just dynamic alone): the dynamic
/// spec deliberately omits position / bounds / line_height / outline.
/// A dynamic-only apply onto a state-A tree would leave those four
/// fields at A's values — correct for hover/HSV (those fields don't
/// change) but incorrect across an explicit layout move. The layout
/// phase is what makes the dynamic phase safe.
#[test]
fn picker_dynamic_mutator_composes_on_layout_built_tree() {
    let g_a = picker_sample_geometry();
    let mut g_b = picker_sample_geometry();
    g_b.hue_deg = 200.0;
    g_b.sat = 0.4;
    g_b.val = 0.6;
    g_b.hovered_hit = Some(PickerHit::SatCell(2));

    let layout_a = compute_color_picker_layout(&g_a, 1280.0, 720.0);
    let layout_b = compute_color_picker_layout(&g_b, 1280.0, 720.0);

    let mut tree = build_color_picker_overlay_tree(&g_a, &layout_a);
    // Layout phase first (sets static fields for state B).
    build_color_picker_overlay_mutator(&g_b, &layout_b).apply_to(&mut tree);
    // Dynamic phase on top (refreshes per-frame fields for state B).
    build_color_picker_overlay_dynamic_mutator(&g_b, &layout_b).apply_to(&mut tree);

    let expected = picker_glyph_areas(&g_b, &layout_b);
    let mut got: Vec<(usize, GlyphArea)> = Vec::new();
    for descendant_id in tree.root().descendants(&tree.arena) {
        let node = tree.arena.get(descendant_id).expect("arena node");
        let element = node.get();
        if let Some(area) = element.glyph_area() {
            got.push((element.channel(), area.clone()));
        }
    }

    assert_eq!(got.len(), expected.len(), "post-mutation area count");
    for ((c_got, a_got), (c_exp, a_exp)) in got.iter().zip(expected.iter()) {
        assert_eq!(c_got, c_exp, "channel mismatch on dynamic compose");
        assert_eq!(a_got.text, a_exp.text, "text on ch {c_got}");
        assert_eq!(a_got.regions, a_exp.regions, "regions on ch {c_got}");
        assert_eq!(a_got.scale, a_exp.scale, "scale on ch {c_got}");
        // Static fields come from the layout phase — verify they're
        // still the layout-correct values.
        assert_eq!(a_got.position, a_exp.position, "position on ch {c_got}");
        assert_eq!(
            a_got.render_bounds, a_exp.render_bounds,
            "bounds on ch {c_got}"
        );
        assert_eq!(a_got.outline, a_exp.outline, "outline on ch {c_got}");
    }
}

/// Back-to-back dynamic applies with no layout mutator between them
/// — the common app path once the layout cache is seeded (hover A →
/// hover B → hover C, all under one layout). Pins the `Assign`-per-
/// field contract of the dynamic spec: an earlier apply's scale /
/// color / hex-text must not leak through to the later apply. If
/// any dynamic field drifted (e.g. a hovered cell's bumped scale
/// persisting across the un-hover), this test would fail before the
/// user saw the glitch.
#[test]
fn picker_dynamic_mutator_composes_across_repeated_applies() {
    let g_a = picker_sample_geometry();
    let mut g_b = picker_sample_geometry();
    g_b.hue_deg = 45.0;
    g_b.sat = 0.3;
    g_b.val = 0.9;
    g_b.hovered_hit = Some(PickerHit::Hue(5));
    let mut g_c = picker_sample_geometry();
    g_c.hue_deg = 290.0;
    g_c.sat = 0.8;
    g_c.val = 0.2;
    // Hover a different axis so B's hue-ring scale bump has to be
    // overwritten by C's normal scale on that same cell, *and* C's
    // val-bar scale bump has to stamp through.
    g_c.hovered_hit = Some(PickerHit::ValCell(12));

    // Layout is shared — the dynamic path runs iff layout_changed ==
    // false in `rebuild_color_picker_overlay`, so the layout we pass
    // to each dynamic mutator is whatever the initial A build used.
    let layout = compute_color_picker_layout(&g_a, 1280.0, 720.0);

    let mut tree = build_color_picker_overlay_tree(&g_a, &layout);
    build_color_picker_overlay_dynamic_mutator(&g_b, &layout).apply_to(&mut tree);
    build_color_picker_overlay_dynamic_mutator(&g_c, &layout).apply_to(&mut tree);

    // A dynamic apply of C onto a layout-A-built tree should leave
    // the dynamic fields at C and the static fields at A's layout —
    // picker_glyph_areas(g_c, layout_A) produces exactly that.
    let expected = picker_glyph_areas(&g_c, &layout);
    let mut got: Vec<(usize, GlyphArea)> = Vec::new();
    for descendant_id in tree.root().descendants(&tree.arena) {
        let node = tree.arena.get(descendant_id).expect("arena node");
        let element = node.get();
        if let Some(area) = element.glyph_area() {
            got.push((element.channel(), area.clone()));
        }
    }

    assert_eq!(got.len(), expected.len(), "post-mutation area count");
    for ((c_got, a_got), (c_exp, a_exp)) in got.iter().zip(expected.iter()) {
        assert_eq!(c_got, c_exp, "channel mismatch on repeated dynamic");
        assert_eq!(a_got.regions, a_exp.regions, "regions on ch {c_got}");
        assert_eq!(a_got.scale, a_exp.scale, "scale on ch {c_got}");
        assert_eq!(a_got.text, a_exp.text, "text on ch {c_got}");
    }
}
