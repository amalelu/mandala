//! Layout-mutator output shape + round-trip equivalence between
//! a freshly-built tree and an in-place mutator apply.

use baumhard::core::primitives::Applicable;
use baumhard::gfx_structs::area::GlyphArea;
use baumhard::gfx_structs::mutator::{GfxMutator, Mutation, MutatorType};
use baumhard::gfx_structs::tree::BranchChannel;

use super::fixtures::picker_sample_geometry;
use crate::application::color_picker::{compute_color_picker_layout, PickerHit};
use crate::application::color_picker_overlay::picker_glyph_areas::{
    build_color_picker_overlay_mutator, build_color_picker_overlay_tree, picker_glyph_areas,
};

/// Spec-driven mutator output matches the picker's structural
/// invariants: root `Void` on channel 0, 60 `Single` children
/// (title + 24 hue + hint + 16 sat + 16 val + preview + hex), each
/// carrying an `AreaDelta` with exactly 8 fields. Guards against
/// drift between `widgets/color_picker.json`'s `mutator_spec` block
/// and the `mutator_builder` walker — if either side silently
/// changes shape, the picker's registered tree alignment breaks and
/// this test fires first.
#[test]
fn picker_mutator_output_matches_spec_shape() {
    let g = picker_sample_geometry();
    let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
    let mt = build_color_picker_overlay_mutator(&g, &layout);

    // Root must be a Void on channel 0 (the root_channel the JSON
    // declares).
    match mt.arena.get(mt.root).unwrap().get() {
        GfxMutator::Void { channel: 0 } => {}
        other => panic!("picker mutator root must be Void(0), got {other:?}"),
    }

    let children: Vec<&GfxMutator> = mt
        .root
        .children(&mt.arena)
        .map(|id| mt.arena.get(id).unwrap().get())
        .collect();
    assert_eq!(children.len(), 60, "picker emits 60 live cells");

    for child in &children {
        assert!(
            matches!(child.get_type(), MutatorType::Single),
            "every picker cell is a Single"
        );
        let GfxMutator::Single { mutation, .. } = child else { unreachable!() };
        let Mutation::AreaDelta(delta) = mutation else {
            panic!("picker cells carry AreaDelta");
        };
        assert_eq!(
            delta.fields.len(),
            8,
            "picker cell template has 8 fields (Text, position, bounds, scale, \
             line_height, ColorFontRegions, Outline, Operation::Assign)"
        );
    }
}

/// Mutator round-trip: the §B2 in-place update path keeps working
/// across the tree-shape change in `build_color_picker_overlay_tree`.
/// Build a tree at state A, apply a mutator computed from state B (a
/// different hue / sat / val), and verify the resulting GlyphArea
/// fields match a freshly-built state-B tree. Confirms the new
/// GlyphModel children don't interfere with channel alignment.
#[test]
fn picker_overlay_mutator_round_trips_across_paired_tree() {
    let mut g_a = picker_sample_geometry();
    g_a.hue_deg = 0.0;
    g_a.sat = 1.0;
    g_a.val = 1.0;
    let layout_a = compute_color_picker_layout(&g_a, 1280.0, 720.0);

    let mut g_b = picker_sample_geometry();
    g_b.hue_deg = 120.0;
    g_b.sat = 0.6;
    g_b.val = 0.4;
    let layout_b = compute_color_picker_layout(&g_b, 1280.0, 720.0);

    let mut tree = build_color_picker_overlay_tree(&g_a, &layout_a);
    let mutator = build_color_picker_overlay_mutator(&g_b, &layout_b);
    mutator.apply_to(&mut tree);

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
        assert_eq!(c_got, c_exp, "channel mismatch on round-trip");
        assert_eq!(a_got.text, a_exp.text, "text on ch {c_got}");
        assert_eq!(a_got.position, a_exp.position, "position on ch {c_got}");
        assert_eq!(a_got.regions, a_exp.regions, "regions on ch {c_got}");
    }
}

/// Round-trip: applying the mutator to a freshly-built tree should
/// leave every GlyphArea's variable state matching what a fresh
/// `picker_glyph_areas` call would emit. Pins the promise that the
/// §B2 in-place update path produces the same observable state as a
/// from-scratch rebuild.
#[test]
fn picker_mutator_round_trips_to_fresh_build() {
    let g_a = picker_sample_geometry();
    let mut g_b = picker_sample_geometry();
    g_b.hue_deg = 120.0;
    g_b.sat = 0.5;
    g_b.val = 0.7;
    g_b.hovered_hit = Some(PickerHit::Hue(3));

    let layout_a = compute_color_picker_layout(&g_a, 1280.0, 720.0);
    let layout_b = compute_color_picker_layout(&g_b, 1280.0, 720.0);

    // Build the picker tree at state A, then apply the mutator
    // computed from state B.
    let mut tree = build_color_picker_overlay_tree(&g_a, &layout_a);
    let mutator = build_color_picker_overlay_mutator(&g_b, &layout_b);
    mutator.apply_to(&mut tree);

    // Fresh build at state B, for comparison.
    let expected = picker_glyph_areas(&g_b, &layout_b);

    let mut got: Vec<(usize, GlyphArea)> = Vec::new();
    for descendant_id in tree.root().descendants(&tree.arena) {
        let node = tree.arena.get(descendant_id).expect("arena node");
        let element = node.get();
        if let Some(area) = element.glyph_area() {
            got.push((element.channel(), area.clone()));
        }
    }

    assert_eq!(
        got.len(),
        expected.len(),
        "post-mutation tree element count mismatch"
    );
    for ((c_got, a_got), (c_exp, a_exp)) in got.iter().zip(expected.iter()) {
        assert_eq!(c_got, c_exp, "channel mismatch");
        assert_eq!(a_got.text, a_exp.text, "text mismatch on channel {c_got}");
        assert_eq!(
            a_got.position, a_exp.position,
            "position mismatch on channel {c_got}"
        );
        assert_eq!(
            a_got.render_bounds, a_exp.render_bounds,
            "bounds mismatch on channel {c_got}"
        );
        assert_eq!(a_got.scale, a_exp.scale, "scale mismatch on channel {c_got}");
        assert_eq!(
            a_got.line_height, a_exp.line_height,
            "line_height mismatch on channel {c_got}"
        );
        assert_eq!(
            a_got.outline, a_exp.outline,
            "outline mismatch on channel {c_got}"
        );
        assert_eq!(
            a_got.regions, a_exp.regions,
            "regions mismatch on channel {c_got}"
        );
    }
}
