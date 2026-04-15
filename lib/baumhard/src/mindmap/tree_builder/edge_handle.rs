//! Edge-handle tree builder: one `GlyphArea` per visible handle
//! (AnchorFrom, AnchorTo, Midpoint, ControlPoint(n)). The
//! channel is derived from the handle kind so the identity
//! sequence is stable across drags that preserve handle shape.

use glam::Vec2;

use crate::core::primitives::{ColorFontRegion, ColorFontRegions, Range};
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::GfxMutator;
use crate::gfx_structs::tree::Tree;
use crate::util::color;

pub fn edge_handle_channel_for(
    kind: crate::mindmap::scene_builder::EdgeHandleKind,
) -> usize {
    use crate::mindmap::scene_builder::EdgeHandleKind;
    match kind {
        EdgeHandleKind::AnchorFrom => 1,
        EdgeHandleKind::AnchorTo => 2,
        EdgeHandleKind::Midpoint => 3,
        EdgeHandleKind::ControlPoint(n) => 100 + n,
    }
}

/// Identity sequence for a set of [`EdgeHandleElement`]s — the
/// kind-derived channel of each handle, in tree-insertion order.
/// Two handle sets produce the same identity iff their structural
/// shape is identical (same kinds in the same order); the in-place
/// [`build_edge_handle_mutator_tree`] path is sound only under that
/// condition. A change in `control_points.len()`, or a switch
/// between `Midpoint` and `ControlPoint`, drops the equality and
/// forces a full rebuild.
pub fn edge_handle_identity_sequence(
    elements: &[crate::mindmap::scene_builder::EdgeHandleElement],
) -> Vec<usize> {
    elements
        .iter()
        .map(|e| edge_handle_channel_for(e.kind))
        .collect()
}

/// Lay out one edge-handle as the `(channel, GlyphArea)` pair both
/// the initial-build path ([`build_edge_handle_tree`]) and the
/// in-place mutator path ([`build_edge_handle_mutator_tree`]) emit.
/// Single source of truth — the two paths cannot drift.
fn edge_handle_layout(
    elem: &crate::mindmap::scene_builder::EdgeHandleElement,
) -> (usize, GlyphArea) {
    let color_rgba = color::hex_to_rgba_safe(&elem.color, [0.0, 0.9, 1.0, 1.0]);
    // Handle glyphs are centered on the position with the same
    // half-glyph offset the legacy renderer used.
    let half_w = elem.font_size_pt * 0.3;
    let half_h = elem.font_size_pt * 0.5;
    let pos = Vec2::new(elem.position.0 - half_w, elem.position.1 - half_h);
    let bounds = Vec2::new(elem.font_size_pt, elem.font_size_pt);

    let mut area = GlyphArea::new_with_str(
        &elem.glyph,
        elem.font_size_pt,
        elem.font_size_pt,
        pos,
        bounds,
    );
    let cluster_count = elem.glyph.chars().count();
    if cluster_count > 0 {
        let mut regions = ColorFontRegions::new_empty();
        regions.submit_region(ColorFontRegion::new(
            Range::new(0, cluster_count),
            None,
            Some(color_rgba),
        ));
        area.regions = regions;
    }

    (edge_handle_channel_for(elem.kind), area)
}

/// Build a baumhard tree of every edge-handle glyph from a
/// pre-computed `EdgeHandleElement` slice. Handles only exist
/// while an edge is selected, so this tree is typically empty or
/// has ≤ 5 leaves.
///
/// Channels come from [`edge_handle_channel_for`] so the in-place
/// [`build_edge_handle_mutator_tree`] path can target each leaf by
/// the same kind-derived channel across drag frames.
pub fn build_edge_handle_tree(
    elements: &[crate::mindmap::scene_builder::EdgeHandleElement],
) -> Tree<GfxElement, GfxMutator> {
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    let mut unique_id: usize = 1;

    for elem in elements {
        let (channel, area) = edge_handle_layout(elem);
        let element_node =
            GfxElement::new_area_non_indexed_with_id(area, channel, unique_id);
        unique_id += 1;
        let leaf = tree.arena.new_node(element_node);
        tree.root.append(leaf, &mut tree.arena);
    }

    tree
}

/// Build a [`MutatorTree`](crate::gfx_structs::tree::MutatorTree)
/// that updates an already-registered edge-handle tree to the
/// current `elements` state without rebuilding the arena. Pairs
/// with [`build_edge_handle_tree`] — the channels emitted by both
/// come from [`edge_handle_channel_for`], so applying this mutator
/// to a tree built from an element slice with the same identity
/// sequence (per [`edge_handle_identity_sequence`]) updates each
/// handle's variable fields in place.
///
/// Variable fields covered: text, position, bounds, scale,
/// line_height, regions, outline. The `Assign` operation
/// overwrites whichever changed — same shape as the picker /
/// portal mutators so any shift in glyph or color is picked up.
pub fn build_edge_handle_mutator_tree(
    elements: &[crate::mindmap::scene_builder::EdgeHandleElement],
) -> crate::gfx_structs::tree::MutatorTree<GfxMutator> {
    use crate::core::primitives::ApplyOperation;
    use crate::gfx_structs::area::{DeltaGlyphArea, GlyphAreaField};
    use crate::gfx_structs::mutator::Mutation;
    use crate::gfx_structs::tree::MutatorTree;

    let mut mt: MutatorTree<GfxMutator> = MutatorTree::new_with(GfxMutator::new_void(0));
    for elem in elements {
        let (channel, area) = edge_handle_layout(elem);
        let delta = DeltaGlyphArea::new(vec![
            GlyphAreaField::Text(area.text),
            GlyphAreaField::position(area.position.x.0, area.position.y.0),
            GlyphAreaField::bounds(area.render_bounds.x.0, area.render_bounds.y.0),
            GlyphAreaField::scale(area.scale.0),
            GlyphAreaField::line_height(area.line_height.0),
            GlyphAreaField::ColorFontRegions(area.regions),
            GlyphAreaField::Outline(area.outline),
            GlyphAreaField::Operation(ApplyOperation::Assign),
        ]);
        let leaf = mt
            .arena
            .new_node(GfxMutator::new(Mutation::AreaDelta(Box::new(delta)), channel));
        mt.root.append(leaf, &mut mt.arena);
    }
    mt
}
