//! Connection tree builder: one `Void` per edge, with one cap /
//! body-glyph / cap GlyphArea per on-screen glyph. Channels are
//! layered so layout-stable changes (color, theme) take the
//! in-place mutator path via `build_connection_mutator_tree`.

use glam::Vec2;

use crate::core::primitives::{ColorFontRegion, ColorFontRegions, Range};
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::GfxMutator;
use crate::gfx_structs::tree::Tree;
use crate::util::color;

/// Channel layout inside each per-edge Void parent. Caps live
/// at the ends (1 for start, 1_000_001 for end so it sorts after
/// any plausible body count), body glyphs stride from 100.
const CONN_CAP_START_CHANNEL: usize = 1;
const CONN_BODY_BASE_CHANNEL: usize = 100;
#[allow(dead_code)]
const CONN_CAP_END_CHANNEL: usize = 1_000_001;

/// channel set ⇒ `align_child_walks` matches every child.
pub type ConnectionEdgeIdentity = (
    crate::mindmap::scene_cache::EdgeKey,
    /* has_cap_start */ bool,
    /* body_count    */ usize,
    /* has_cap_end   */ bool,
);

/// Identity sequence for a slice of `ConnectionElement`s. Used by
/// the dispatcher in `update_connection_tree` to choose between
/// full rebuild and the in-place mutator path. During endpoint
/// drag the body-glyph count typically shifts every few pixels and
/// the dispatcher takes the rebuild path; for selection / color
/// preview / theme switches it stays stable and the mutator path
/// runs.
pub fn connection_identity_sequence(
    elements: &[crate::mindmap::scene_builder::ConnectionElement],
) -> Vec<ConnectionEdgeIdentity> {
    elements
        .iter()
        .map(|e| {
            (
                e.edge_key.clone(),
                e.cap_start.is_some(),
                e.glyph_positions.len(),
                e.cap_end.is_some(),
            )
        })
        .collect()
}

/// Lay out the per-glyph shape data both connection build paths
/// emit. Returns `(edge_channel, Vec<(child_channel, GlyphArea)>)`
/// for one edge. Single source of truth — the initial-build path
/// ([`build_connection_tree`]) and the in-place mutator path
/// ([`build_connection_mutator_tree`]) cannot drift.
fn connection_edge_layout(
    edge_index: usize,
    elem: &crate::mindmap::scene_builder::ConnectionElement,
) -> (usize, Vec<(usize, GlyphArea)>) {
    let font_size = elem.font_size_pt;
    let half_glyph = font_size * 0.3;
    let half_height = font_size * 0.5;
    let glyph_bounds = Vec2::new(font_size, font_size);
    let color_rgba = color::hex_to_rgba_safe(&elem.color, [0.78, 0.78, 0.78, 1.0]);

    let mk_area = |text: &str, pos: Vec2| -> GlyphArea {
        let mut area =
            GlyphArea::new_with_str(text, font_size, font_size, pos, glyph_bounds);
        let cluster_count = crate::util::grapheme_chad::count_grapheme_clusters(text);
        if cluster_count > 0 {
            let mut regions = ColorFontRegions::new_empty();
            regions.submit_region(ColorFontRegion::new(
                Range::new(0, cluster_count),
                None,
                Some(color_rgba),
            ));
            area.regions = regions;
        }
        area
    };

    let mut children: Vec<(usize, GlyphArea)> = Vec::new();
    if let Some((glyph_text, (cx, cy))) = elem.cap_start.as_ref() {
        let pos = Vec2::new(cx - half_glyph, cy - half_height);
        children.push((CONN_CAP_START_CHANNEL, mk_area(glyph_text, pos)));
    }
    for (i, &(gx, gy)) in elem.glyph_positions.iter().enumerate() {
        let pos = Vec2::new(gx - half_glyph, gy - half_height);
        children.push((CONN_BODY_BASE_CHANNEL + i, mk_area(&elem.body_glyph, pos)));
    }
    if let Some((glyph_text, (cx, cy))) = elem.cap_end.as_ref() {
        let pos = Vec2::new(cx - half_glyph, cy - half_height);
        children.push((CONN_CAP_END_CHANNEL, mk_area(glyph_text, pos)));
    }

    // Per-edge channel: 1-based edge index. Stable across rebuilds
    // for the same identity sequence.
    (edge_index + 1, children)
}

/// Build a baumhard tree of connection glyphs from a slice of
/// pre-computed `ConnectionElement`s.
///
/// Tree shape per visible edge:
///
/// ```text
/// Void (per edge — channel = edge index, 1-based)
/// ├── GlyphArea (cap_start, channel = 1)              // optional
/// ├── GlyphArea (body glyph @ position 0, ch=100+0)
/// ├── GlyphArea (body glyph @ position 1, ch=100+1)
/// │   ...
/// └── GlyphArea (cap_end, channel = 1_000_001)        // optional
/// ```
///
/// Each GlyphArea is sized `(font_size, font_size)` and centred on
/// the connection-element's reported glyph position via the same
/// `(half_glyph, half_height)` offset the legacy
/// `Renderer::rebuild_connection_buffers_keyed` applied. Color is
/// baked into a single `ColorFontRegion` covering the body glyph.
///
/// Channels come from [`connection_edge_layout`] so the in-place
/// [`build_connection_mutator_tree`] path can target each leaf by
/// the same channel across calls when the structure (body glyph
/// count, cap presence) hasn't changed.
///
/// # Costs
///
/// O(sum of glyph_positions across all elements). Allocates the
/// tree arena. No font shaping happens here — that's the
/// renderer's `walk_tree_into_buffers` step.
pub fn build_connection_tree(
    elements: &[crate::mindmap::scene_builder::ConnectionElement],
) -> Tree<GfxElement, GfxMutator> {
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    let mut unique_id: usize = 1;

    for (idx, elem) in elements.iter().enumerate() {
        let (edge_channel, children) = connection_edge_layout(idx, elem);
        let edge_root = tree
            .arena
            .new_node(GfxElement::new_void_with_id(edge_channel, unique_id));
        unique_id += 1;
        tree.root.append(edge_root, &mut tree.arena);

        for (channel, area) in children {
            let element = GfxElement::new_area_non_indexed_with_id(area, channel, unique_id);
            unique_id += 1;
            let leaf = tree.arena.new_node(element);
            edge_root.append(leaf, &mut tree.arena);
        }
    }

    tree
}

/// Build a [`MutatorTree`](crate::gfx_structs::tree::MutatorTree)
/// that updates an already-registered connection tree to the
/// current `elements` state without rebuilding the arena. Pairs
/// with [`build_connection_tree`] — both consume
/// [`connection_edge_layout`], so applying this mutator to a tree
/// built from an element slice with the same
/// [`connection_identity_sequence`] updates each glyph's variable
/// fields in place.
pub fn build_connection_mutator_tree(
    elements: &[crate::mindmap::scene_builder::ConnectionElement],
) -> crate::gfx_structs::tree::MutatorTree<GfxMutator> {
    use crate::core::primitives::ApplyOperation;
    use crate::gfx_structs::area::{DeltaGlyphArea, GlyphAreaField};
    use crate::gfx_structs::mutator::Mutation;
    use crate::gfx_structs::tree::MutatorTree;

    let mut mt: MutatorTree<GfxMutator> = MutatorTree::new_with(GfxMutator::new_void(0));
    for (idx, elem) in elements.iter().enumerate() {
        let (edge_channel, children) = connection_edge_layout(idx, elem);
        let edge_node = mt.arena.new_node(GfxMutator::new_void(edge_channel));
        mt.root.append(edge_node, &mut mt.arena);

        for (channel, area) in children {
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
            let leaf = mt.arena.new_node(GfxMutator::new(
                Mutation::AreaDelta(Box::new(delta)),
                channel,
            ));
            edge_node.append(leaf, &mut mt.arena);
        }
    }
    mt
}

