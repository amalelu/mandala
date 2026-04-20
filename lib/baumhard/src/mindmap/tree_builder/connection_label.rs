//! Connection-label tree builder: one `GlyphArea` per labeled
//! edge, keyed by `EdgeKey`. Returns per-edge AABB hitboxes so
//! the renderer can resolve label clicks back to the edge.

use std::collections::HashMap;

use glam::Vec2;

use crate::core::primitives::{ColorFontRegion, ColorFontRegions, Range};
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::GfxMutator;
use crate::gfx_structs::tree::Tree;
use crate::util::color;

/// Output of [`build_connection_label_tree`]: the baumhard tree of
/// per-label GlyphAreas plus a per-edge AABB map so the renderer can
/// resolve label clicks back to the owning edge.
pub struct ConnectionLabelTree {
    pub tree: Tree<GfxElement, GfxMutator>,
    /// `EdgeKey → AABB` for click hit-testing on the label.
    pub hitboxes: HashMap<crate::mindmap::scene_cache::EdgeKey, (Vec2, Vec2)>,
}

/// Identity sequence for a slice of `ConnectionLabelElement`s — the
/// edge-key of each labeled edge, in tree-insertion order. Two
/// label sets share an identity iff the *set of labeled edges* and
/// their order match. Label text, position, color, and font size
/// can vary inside that envelope; only adding, removing, or
/// reordering a labeled edge breaks the equality and forces a full
/// rebuild via the dispatcher in `update_connection_label_tree`.
pub fn connection_label_identity_sequence(
    elements: &[crate::mindmap::scene_builder::ConnectionLabelElement],
) -> Vec<crate::mindmap::scene_cache::EdgeKey> {
    elements.iter().map(|e| e.edge_key.clone()).collect()
}

/// Lay out one connection-label as the
/// `(channel, GlyphArea, hitbox_min, hitbox_max)` tuple both build
/// paths emit. Channel is the 1-based label index, matching the
/// ascending insertion order. Single source of truth — the
/// initial-build path ([`build_connection_label_tree`]) and the
/// in-place mutator path ([`build_connection_label_mutator_tree`])
/// cannot drift.
fn connection_label_layout(
    channel: usize,
    elem: &crate::mindmap::scene_builder::ConnectionLabelElement,
) -> (usize, GlyphArea, Vec2, Vec2) {
    let color_rgba = color::hex_to_rgba_safe(&elem.color, [0.92, 0.92, 0.92, 1.0]);
    let pos = Vec2::new(elem.position.0, elem.position.1);
    let bounds = Vec2::new(elem.bounds.0, elem.bounds.1);

    let mut area = GlyphArea::new_with_str(
        &elem.text,
        elem.font_size_pt,
        elem.font_size_pt,
        pos,
        bounds,
    );
    let cluster_count = crate::util::grapheme_chad::count_grapheme_clusters(&elem.text);
    if cluster_count > 0 {
        let mut regions = ColorFontRegions::new_empty();
        regions.submit_region(ColorFontRegion::new(
            Range::new(0, cluster_count),
            None,
            Some(color_rgba),
        ));
        area.regions = regions;
    }

    (channel, area, pos, pos + bounds)
}

/// Build a baumhard tree of every connection-label glyph from a
/// pre-computed `ConnectionLabelElement` slice. Like the
/// connection tree, geometry comes from `scene_builder` upstream.
///
/// Channels are sequential (1-based) per labeled edge so the
/// in-place [`build_connection_label_mutator_tree`] path can target
/// each leaf by its insertion index when the identity sequence
/// matches.
pub fn build_connection_label_tree(
    elements: &[crate::mindmap::scene_builder::ConnectionLabelElement],
) -> ConnectionLabelTree {
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    let mut hitboxes: HashMap<crate::mindmap::scene_cache::EdgeKey, (Vec2, Vec2)> =
        HashMap::new();
    let mut unique_id: usize = 1;

    for (idx, elem) in elements.iter().enumerate() {
        let (channel, area, hb_min, hb_max) = connection_label_layout(idx + 1, elem);
        let element_node =
            GfxElement::new_area_non_indexed_with_id(area, channel, unique_id);
        unique_id += 1;
        let leaf = tree.arena.new_node(element_node);
        tree.root.append(leaf, &mut tree.arena);

        hitboxes.insert(elem.edge_key.clone(), (hb_min, hb_max));
    }

    ConnectionLabelTree { tree, hitboxes }
}

/// Result of [`build_connection_label_mutator_tree`]. The `mutator`
/// is applied to the tree returned by [`build_connection_label_tree`]
/// via `MutatorTree::apply_to`; `hitboxes` replaces the renderer's
/// label hitbox map (label position can move with the edge it
/// belongs to even when the structural identity is unchanged).
pub struct ConnectionLabelMutator {
    pub mutator: crate::gfx_structs::tree::MutatorTree<GfxMutator>,
    pub hitboxes: HashMap<crate::mindmap::scene_cache::EdgeKey, (Vec2, Vec2)>,
}

/// Build a [`MutatorTree`](crate::gfx_structs::tree::MutatorTree)
/// that updates an already-registered connection-label tree to the
/// current `elements` state without rebuilding the arena. Pairs
/// with [`build_connection_label_tree`] — channels and insertion
/// order match, so applying this mutator to a tree built from a
/// label slice with the same identity sequence (per
/// [`connection_label_identity_sequence`]) updates each label's
/// variable fields in place.
pub fn build_connection_label_mutator_tree(
    elements: &[crate::mindmap::scene_builder::ConnectionLabelElement],
) -> ConnectionLabelMutator {
    use crate::core::primitives::ApplyOperation;
    use crate::gfx_structs::area::{DeltaGlyphArea, GlyphAreaField};
    use crate::gfx_structs::mutator::Mutation;
    use crate::gfx_structs::tree::MutatorTree;

    let mut mt: MutatorTree<GfxMutator> = MutatorTree::new_with(GfxMutator::new_void(0));
    let mut hitboxes: HashMap<crate::mindmap::scene_cache::EdgeKey, (Vec2, Vec2)> =
        HashMap::new();

    for (idx, elem) in elements.iter().enumerate() {
        let (channel, area, hb_min, hb_max) = connection_label_layout(idx + 1, elem);
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
        hitboxes.insert(elem.edge_key.clone(), (hb_min, hb_max));
    }

    ConnectionLabelMutator { mutator: mt, hitboxes }
}

