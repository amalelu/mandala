//! Border-tree builder: emits one per-node Void parent and four
//! `GlyphArea` runs (top, bottom, left, right) per framed node.
//! Sorted lexicographically by node id so the per-node Void
//! channel is stable across rebuilds — the precondition for the
//! in-place mutator path `build_border_mutator_tree_from_nodes`.

use std::collections::HashMap;

use glam::Vec2;
use indextree::NodeId;

use crate::core::primitives::{ColorFontRegion, ColorFontRegions, Range};
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::GfxMutator;
use crate::gfx_structs::shape::NodeShape;
use crate::gfx_structs::tree::Tree;
use crate::mindmap::border::{BORDER_APPROX_CHAR_WIDTH_FRAC, BORDER_CORNER_OVERLAP_FRAC};
use crate::mindmap::model::MindMap;
use crate::util::color;

/// Per-node data for the border tree — single source of truth
/// consumed by both [`build_border_tree`] (initial build) and
/// [`build_border_mutator_tree`] (in-place §B2 update). The
/// `parent_channel` is the 1-based index of this node in the
/// sorted visible-framed-nodes sequence, so the channel is
/// *stable across rebuilds* as long as the identity sequence
/// (see [`border_identity_sequence`]) is unchanged.
#[derive(Clone, Debug)]
pub struct BorderNodeData {
    pub node_id: String,
    pub parent_channel: usize,
    pub border_style: crate::mindmap::border::BorderStyle,
    pub color_rgba: [f32; 4],
    pub pos_x: f32,
    pub pos_y: f32,
    pub size_x: f32,
    pub size_y: f32,
}

/// Compute the border layout for the current `(map, offsets)`
/// state. Sorted lexicographically by `MindNode.id` so per-node
/// Void parents always land at the same channel — the
/// prerequisite for the in-place mutator path.
///
/// Skips hidden-by-fold and `show_frame = false` nodes, mirroring
/// the filter in `scene_builder::build_scene`.
pub fn border_node_data(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
) -> Vec<BorderNodeData> {
    use crate::mindmap::border::BorderStyle;

    let vars = &map.canvas.theme_variables;
    let mut sorted_ids: Vec<&String> = map.nodes.keys().collect();
    sorted_ids.sort();

    let mut out: Vec<BorderNodeData> = Vec::new();
    let mut parent_channel: usize = 1;
    for node_id in sorted_ids {
        let Some(node) = map.nodes.get(node_id) else {
            continue;
        };
        if map.is_hidden_by_fold(node) {
            continue;
        }
        if !node.style.show_frame {
            continue;
        }
        // The glyph frame is laid out as four axis-aligned text
        // runs along the node's bounding box, which only makes
        // sense for `NodeShape::Rectangle`. For any other shape we
        // suppress the frame; a curved / shape-aware border is
        // tracked as follow-up work (see CLAUDE.md). Authors still
        // round-trip the `show_frame` flag untouched — we simply
        // don't emit the glyphs.
        if NodeShape::from_style_string(&node.style.shape) != NodeShape::Rectangle {
            continue;
        }
        let (ox, oy) = offsets.get(&node.id).copied().unwrap_or((0.0, 0.0));
        let frame_color_hex = color::resolve_var(&node.style.frame_color, vars);
        let border_style = BorderStyle::default_with_color(frame_color_hex);
        let color_rgba = color::hex_to_rgba_safe(&border_style.color, [1.0, 1.0, 1.0, 1.0]);

        out.push(BorderNodeData {
            node_id: node.id.clone(),
            parent_channel,
            border_style,
            color_rgba,
            pos_x: node.position.x as f32 + ox,
            pos_y: node.position.y as f32 + oy,
            size_x: node.size.width as f32,
            size_y: node.size.height as f32,
        });
        parent_channel += 1;
    }
    out
}

/// Identity sequence for a slice of [`BorderNodeData`] — the
/// sorted sequence of `node_id`s in tree-insertion order. Two
/// sequences match iff the same set of nodes is framed in the
/// same order. Drag, text-edit, color-preview, and preset-swap
/// all leave this stable (preset swaps change the character
/// content of each run but not the tree shape — the mutator's
/// `Text::Assign` picks up the new glyphs); adding or removing a
/// framed node, toggling `show_frame`, or folding an ancestor
/// drops the equality and forces a full rebuild via the
/// dispatcher in `update_border_tree_static`.
pub fn border_identity_sequence(nodes: &[BorderNodeData]) -> Vec<String> {
    nodes.iter().map(|n| n.node_id.clone()).collect()
}

/// Build the border tree from the given `MindMap` + drag offsets.
/// Convenience wrapper that calls [`border_node_data`] then
/// [`build_border_tree_from_nodes`].
///
/// Tree shape:
///
/// ```text
/// Void (root)
/// ├── Void (per node — channel = 1-based sorted index)
/// │   ├── GlyphArea (top run, channel = 1)
/// │   ├── GlyphArea (bottom run, channel = 2)
/// │   ├── GlyphArea (left column, channel = 3)
/// │   └── GlyphArea (right column, channel = 4)
/// ├── Void (next node)
/// │   └── ...
/// ```
///
/// Iteration order is the lexicographic order of `MindNode.id` —
/// stable across runs, so per-node Void parents always land at
/// the same channel. Without this, `MindMap.nodes` (a `HashMap`)
/// would yield nondeterministic order and make the in-place
/// mutator path unreliable.
///
/// # Costs
///
/// O(N log N) where N is the visible framed-node count (the sort
/// dominates for large maps). Allocates one tree arena plus one
/// `String` per run. Uses the same `BorderStyle` defaults as
/// `scene_builder::build_scene` so the two paths can't drift on
/// style choices.
pub fn build_border_tree(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
) -> Tree<GfxElement, GfxMutator> {
    build_border_tree_from_nodes(&border_node_data(map, offsets))
}

/// Variant of [`build_border_tree`] that consumes pre-computed
/// node data. Use this in the dispatch path that already called
/// [`border_node_data`] to derive the identity sequence — saves
/// one walk over `MindMap.nodes`.
pub fn build_border_tree_from_nodes(nodes: &[BorderNodeData]) -> Tree<GfxElement, GfxMutator> {
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    let mut unique_id: usize = 1;
    for node in nodes {
        append_border_sub_tree(
            &mut tree,
            &node.border_style,
            node.color_rgba,
            node.pos_x,
            node.pos_y,
            node.size_x,
            node.size_y,
            node.parent_channel,
            &mut unique_id,
        );
    }
    tree
}

/// Build a [`MutatorTree`](crate::gfx_structs::tree::MutatorTree)
/// that updates an already-registered border tree to the current
/// `(map, offsets)` state without rebuilding the arena. Pairs
/// with [`build_border_tree`] — both consume
/// [`border_node_data`], so applying this mutator to a tree built
/// from a node slice with the same
/// [`border_identity_sequence`] updates each run's variable
/// fields in place.
///
/// The hot-path case this closes: when the color picker is open,
/// every throttled `AboutToWait` drain re-runs the scene build,
/// which previously re-allocated the entire border tree every
/// frame. With this dispatch, picker hover leaves the border
/// tree's arena untouched and only overwrites text / position /
/// color fields on the existing GlyphAreas.
pub fn build_border_mutator_tree(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
) -> crate::gfx_structs::tree::MutatorTree<GfxMutator> {
    build_border_mutator_tree_from_nodes(&border_node_data(map, offsets))
}

/// Variant of [`build_border_mutator_tree`] that consumes
/// pre-computed node data. Use this in the dispatch path that
/// already called [`border_node_data`].
pub fn build_border_mutator_tree_from_nodes(
    nodes: &[BorderNodeData],
) -> crate::gfx_structs::tree::MutatorTree<GfxMutator> {
    use crate::core::primitives::ApplyOperation;
    use crate::gfx_structs::area::{DeltaGlyphArea, GlyphAreaField};
    use crate::gfx_structs::mutator::Mutation;
    use crate::gfx_structs::tree::MutatorTree;

    let mut mt: MutatorTree<GfxMutator> = MutatorTree::new_with(GfxMutator::new_void(0));
    for node in nodes {
        let parent_node = mt
            .arena
            .new_node(GfxMutator::new_void(node.parent_channel));
        mt.root.append(parent_node, &mut mt.arena);

        // Recompute the same layout the initial-build path uses.
        // Split out here because the mutator needs each run's text
        // / position / bounds as assign deltas, not as appends.
        let font_size = node.border_style.font_size_pt;
        let approx_char_width = font_size * BORDER_APPROX_CHAR_WIDTH_FRAC;
        let char_count =
            ((node.size_x / approx_char_width) + 2.0).ceil().max(3.0) as usize;
        let right_corner_x = node.pos_x - approx_char_width
            + (char_count - 1) as f32 * approx_char_width;
        let corner_overlap = font_size * BORDER_CORNER_OVERLAP_FRAC;
        let top_y = node.pos_y - font_size + corner_overlap;
        let bottom_y = node.pos_y + node.size_y - corner_overlap;
        let h_width = (char_count as f32 + 1.0) * approx_char_width;
        let v_width = approx_char_width * 2.0;
        let row_count = (node.size_y / font_size).round().max(1.0) as usize;

        let glyph_set = &node.border_style.glyph_set;
        let top_text = glyph_set.top_border(char_count);
        let bottom_text = glyph_set.bottom_border(char_count);
        let left_text: String =
            std::iter::repeat_n(format!("{}\n", glyph_set.left_char()), row_count).collect();
        let right_text: String =
            std::iter::repeat_n(format!("{}\n", glyph_set.right_char()), row_count).collect();

        let runs = [
            (
                1usize,
                top_text,
                font_size,
                (node.pos_x - approx_char_width, top_y),
                (h_width, font_size * 1.5),
            ),
            (
                2usize,
                bottom_text,
                font_size,
                (node.pos_x - approx_char_width, bottom_y),
                (h_width, font_size * 1.5),
            ),
            (
                3usize,
                left_text,
                font_size,
                (node.pos_x - approx_char_width, node.pos_y),
                (v_width, node.size_y),
            ),
            (
                4usize,
                right_text,
                font_size,
                (right_corner_x, node.pos_y),
                (v_width, node.size_y),
            ),
        ];

        for (channel, text, fs, pos, bounds) in runs {
            let cluster_count = crate::util::grapheme_chad::count_grapheme_clusters(&text);
            let mut regions = ColorFontRegions::new_empty();
            if cluster_count > 0 {
                regions.submit_region(ColorFontRegion::new(
                    Range::new(0, cluster_count),
                    None,
                    Some(node.color_rgba),
                ));
            }
            let delta = DeltaGlyphArea::new(vec![
                GlyphAreaField::Text(text),
                GlyphAreaField::position(pos.0, pos.1),
                GlyphAreaField::bounds(bounds.0, bounds.1),
                GlyphAreaField::scale(fs),
                GlyphAreaField::line_height(fs),
                GlyphAreaField::ColorFontRegions(regions),
                GlyphAreaField::Outline(None),
                GlyphAreaField::Operation(ApplyOperation::Assign),
            ]);
            let leaf = mt.arena.new_node(GfxMutator::new(
                Mutation::AreaDelta(Box::new(delta)),
                channel,
            ));
            parent_node.append(leaf, &mut mt.arena);
        }
    }
    mt
}

/// Build one per-node sub-tree (Void parent + 4 GlyphArea runs) and
/// append it under `tree.root`. Kept as a private helper so
/// `build_border_tree` stays readable. `parent_channel` is the
/// stable 1-based sorted-index channel — see
/// [`BorderNodeData::parent_channel`].
fn append_border_sub_tree(
    tree: &mut Tree<GfxElement, GfxMutator>,
    border_style: &crate::mindmap::border::BorderStyle,
    color_rgba: [f32; 4],
    pos_x: f32,
    pos_y: f32,
    size_x: f32,
    size_y: f32,
    parent_channel: usize,
    unique_id: &mut usize,
) {
    let font_size = border_style.font_size_pt;
    let approx_char_width = font_size * BORDER_APPROX_CHAR_WIDTH_FRAC;
    let char_count = ((size_x / approx_char_width) + 2.0).ceil().max(3.0) as usize;
    let right_corner_x =
        pos_x - approx_char_width + (char_count - 1) as f32 * approx_char_width;
    let corner_overlap = font_size * BORDER_CORNER_OVERLAP_FRAC;
    let top_y = pos_y - font_size + corner_overlap;
    let bottom_y = pos_y + size_y - corner_overlap;
    let h_width = (char_count as f32 + 1.0) * approx_char_width;
    let v_width = approx_char_width * 2.0;
    let row_count = (size_y / font_size).round().max(1.0) as usize;

    let glyph_set = &border_style.glyph_set;
    let top_text = glyph_set.top_border(char_count);
    let bottom_text = glyph_set.bottom_border(char_count);
    let left_text: String =
        std::iter::repeat_n(format!("{}\n", glyph_set.left_char()), row_count).collect();
    let right_text: String =
        std::iter::repeat_n(format!("{}\n", glyph_set.right_char()), row_count).collect();

    // Per-node Void parent — groups the four runs for targeted
    // mutation. The parent's channel is the stable sorted-index
    // value so distinct nodes never collide across rebuilds.
    let parent_id = tree
        .arena
        .new_node(GfxElement::new_void_with_id(parent_channel, *unique_id));
    tree.root.append(parent_id, &mut tree.arena);
    *unique_id += 1;

    // Stable channels 1..=4 inside each border sub-tree. The
    // per-node Void parent already disambiguates across nodes.
    append_border_run(
        tree,
        parent_id,
        1,
        *unique_id,
        &top_text,
        font_size,
        (pos_x - approx_char_width, top_y),
        (h_width, font_size * 1.5),
        color_rgba,
    );
    *unique_id += 1;
    append_border_run(
        tree,
        parent_id,
        2,
        *unique_id,
        &bottom_text,
        font_size,
        (pos_x - approx_char_width, bottom_y),
        (h_width, font_size * 1.5),
        color_rgba,
    );
    *unique_id += 1;
    append_border_run(
        tree,
        parent_id,
        3,
        *unique_id,
        &left_text,
        font_size,
        (pos_x - approx_char_width, pos_y),
        (v_width, size_y),
        color_rgba,
    );
    *unique_id += 1;
    append_border_run(
        tree,
        parent_id,
        4,
        *unique_id,
        &right_text,
        font_size,
        (right_corner_x, pos_y),
        (v_width, size_y),
        color_rgba,
    );
    *unique_id += 1;
}

pub(super) fn append_border_run(
    tree: &mut Tree<GfxElement, GfxMutator>,
    parent_id: NodeId,
    channel: usize,
    unique_id: usize,
    text: &str,
    font_size: f32,
    position: (f32, f32),
    bounds: (f32, f32),
    color_rgba: [f32; 4],
) {
    let mut area = GlyphArea::new_with_str(
        text,
        font_size,
        font_size,
        Vec2::new(position.0, position.1),
        Vec2::new(bounds.0, bounds.1),
    );

    // Single ColorFontRegion covering the whole run — the renderer
    // walker translates this into a cosmic-text `Attrs::color`
    // span. The grapheme counter matches `chars().count()` on the
    // default box-drawing presets (single-scalar codepoints) but
    // keeps the math correct if a future preset or custom border
    // mixes in combining marks or ZWJ sequences — §1 prescribes
    // grapheme-aware counts everywhere a region is derived from
    // reachable user text.
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

    let element = GfxElement::new_area_non_indexed_with_id(area, channel, unique_id);
    let node = tree.arena.new_node(element);
    parent_id.append(node, &mut tree.arena);
}
