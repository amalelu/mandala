//! Portal tree builder — one `GlyphArea` per (portal-mode edge ×
//! endpoint). Mirrors the [`scene_builder::portal`] emission rule:
//! each edge with `display_mode = "portal"` produces two markers,
//! anchored to their owning node's border at the directional
//! default (or user-dragged `border_t`). Color, glyph, and font
//! size come from [`scene_builder::portal::resolve_portal_endpoint_style`]
//! so this path cannot drift from the scene-emission path.
//!
//! [`scene_builder::portal`]: crate::mindmap::scene_builder::portal
//! [`scene_builder::portal::resolve_portal_endpoint_style`]: crate::mindmap::scene_builder::portal::resolve_portal_endpoint_style

use std::collections::HashMap;

use glam::Vec2;

use crate::core::primitives::{ColorFontRegion, ColorFontRegions, Range};
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::GfxMutator;
use crate::gfx_structs::tree::Tree;
use crate::mindmap::model::{is_portal_edge, portal_endpoint_state, MindMap, MindNode};
use crate::mindmap::scene_builder::portal::{
    layout_portal_label, node_center, resolve_portal_endpoint_style, SelectedPortalLabel,
};
use crate::mindmap::scene_cache::EdgeKey;
use crate::mindmap::SELECTION_HIGHLIGHT_HEX;
use crate::util::color;

/// Identifier for the currently selected edge, used to route the
/// cyan highlight color to both markers of a selected portal-mode
/// edge. Tuple is `(from_id, to_id, edge_type)` matching the
/// `EdgeKey` shape elsewhere. Per-label selection travels through
/// `SelectedPortalLabel` instead.
pub type SelectedEdgeRef<'a> = (&'a str, &'a str, &'a str);

/// Optional live preview of one portal-mode edge's color, mirroring
/// `scene_builder::PortalColorPreview`. Wins over selection on the
/// previewed edge so the live HSV feedback is visible on both
/// markers.
#[derive(Debug, Clone, Copy)]
pub struct PortalColorPreviewRef<'a> {
    pub edge_key: &'a EdgeKey,
    pub color: &'a str,
}

/// Result of [`build_portal_tree`]. Bundles the tree with the
/// AABB-per-marker map the legacy `hit_test_portal` path needs
/// while it's still running.
pub struct PortalTree {
    pub tree: Tree<GfxElement, GfxMutator>,
    /// `(edge_key, endpoint_node_id) → AABB`.
    pub hitboxes: HashMap<(EdgeKey, String), (Vec2, Vec2)>,
}

/// Identity tuple for one portal-mode edge: the `EdgeKey` of the
/// owning edge. Used to compare two consecutive [`portal_pair_data`]
/// outputs and decide whether a registered portal tree's structure
/// still matches — the prerequisite for the in-place
/// [`build_portal_mutator_tree`] path.
pub type PortalIdentity = EdgeKey;

/// Per-pair output of [`portal_pair_data`]. Single source of truth
/// for portal layout consumed by both [`build_portal_tree`] (initial
/// build) and [`build_portal_mutator_tree`] (in-place §B2 update).
///
/// `pair_channel` is sequential by visible-portal index — stable
/// across two calls **iff** their visible-portal sequences are
/// identical (same identities in the same order). Callers detect
/// drift by comparing identity slices and fall back to a full
/// rebuild when they disagree.
#[derive(Clone, Debug)]
pub struct PortalPairData {
    pub identity: PortalIdentity,
    pub pair_channel: usize,
    /// Per endpoint: `(slot_channel, area, hitbox, endpoint_node_id)`.
    /// Slot channels are `1` and `2`, fixed by tree-shape contract.
    pub endpoints: [(usize, GlyphArea, (Vec2, Vec2), String); 2],
}

/// Compute the visible portal-mode-edge layout for the given map
/// state. Single source of truth shared by [`build_portal_tree`]
/// and [`build_portal_mutator_tree`] so the two paths cannot drift.
///
/// # Costs
///
/// O(portal-mode edges). Allocates a `Vec` plus two
/// `ColorFontRegions` per edge. Color resolution delegates to
/// `resolve_portal_endpoint_style`.
pub fn portal_pair_data(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    selected_edge: Option<SelectedEdgeRef>,
    selected_portal_label: Option<SelectedPortalLabel<'_>>,
    color_preview: Option<PortalColorPreviewRef>,
) -> Vec<PortalPairData> {
    let mut pairs: Vec<PortalPairData> = Vec::new();
    let mut pair_channel: usize = 1;

    for edge in &map.edges {
        if !is_portal_edge(edge) {
            continue;
        }
        if !edge.visible {
            continue;
        }
        let Some(node_a) = map.nodes.get(&edge.from_id) else {
            continue;
        };
        let Some(node_b) = map.nodes.get(&edge.to_id) else {
            continue;
        };
        if map.is_hidden_by_fold(node_a) || map.is_hidden_by_fold(node_b) {
            continue;
        }

        let edge_key = EdgeKey::from_edge(edge);
        let is_edge_selected = selected_edge.map_or(false, |(f, t, ty)| {
            f == edge.from_id && t == edge.to_id && ty == edge.edge_type
        });
        let preview_for_this_edge: Option<&str> = color_preview.and_then(|p| {
            if *p.edge_key == edge_key {
                Some(p.color)
            } else {
                None
            }
        });

        let make_endpoint =
            |slot: usize, owner: &MindNode, partner: &MindNode|
                -> (usize, GlyphArea, (Vec2, Vec2), String) {
                let (ox, oy) = offsets.get(&owner.id).copied().unwrap_or((0.0, 0.0));
                let owner_pos =
                    Vec2::new(owner.position.x as f32 + ox, owner.position.y as f32 + oy);
                let owner_size = Vec2::new(owner.size.width as f32, owner.size.height as f32);
                let (px, py) = offsets.get(&partner.id).copied().unwrap_or((0.0, 0.0));
                let partner_pos = Vec2::new(
                    partner.position.x as f32 + px,
                    partner.position.y as f32 + py,
                );
                let partner_size =
                    Vec2::new(partner.size.width as f32, partner.size.height as f32);

                let endpoint_state = portal_endpoint_state(edge, &owner.id);
                let is_this_label_selected = selected_portal_label.map_or(false, |s| {
                    *s.edge_key == edge_key && s.endpoint_node_id == owner.id
                });
                let raw_color_override: Option<&str> = if let Some(p) = preview_for_this_edge {
                    Some(p)
                } else if is_edge_selected || is_this_label_selected {
                    Some(SELECTION_HIGHLIGHT_HEX)
                } else {
                    None
                };

                let style = resolve_portal_endpoint_style(
                    edge,
                    endpoint_state,
                    &map.canvas,
                    raw_color_override,
                );
                let layout = layout_portal_label(
                    owner_pos,
                    owner_size,
                    node_center(partner_pos, partner_size),
                    endpoint_state,
                    style.font_size_pt,
                );
                let color_rgba =
                    color::hex_to_rgba_safe(&style.color, [0.92, 0.92, 0.92, 1.0]);

                let mut area = GlyphArea::new_with_str(
                    &style.glyph,
                    style.font_size_pt,
                    style.font_size_pt,
                    layout.top_left,
                    layout.bounds,
                );
                let cluster_count =
                    crate::util::grapheme_chad::count_grapheme_clusters(&style.glyph);
                if cluster_count > 0 {
                    let mut regions = ColorFontRegions::new_empty();
                    regions.submit_region(ColorFontRegion::new(
                        Range::new(0, cluster_count),
                        None,
                        Some(color_rgba),
                    ));
                    area.regions = regions;
                }

                let max = layout.top_left + layout.bounds;
                (slot, area, (layout.top_left, max), owner.id.clone())
            };

        let endpoints = [
            make_endpoint(1, node_a, node_b),
            make_endpoint(2, node_b, node_a),
        ];
        pairs.push(PortalPairData {
            identity: edge_key,
            pair_channel,
            endpoints,
        });
        pair_channel += 1;
    }

    pairs
}

/// Identity sequence for a slice of [`PortalPairData`]. Compared
/// element-wise against a cached sequence to decide whether the
/// in-place [`build_portal_mutator_tree`] path is sound.
pub fn portal_identity_sequence(pairs: &[PortalPairData]) -> Vec<PortalIdentity> {
    pairs.iter().map(|p| p.identity.clone()).collect()
}

/// Build a baumhard tree of every visible portal marker.
pub fn build_portal_tree(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    selected_edge: Option<SelectedEdgeRef>,
    selected_portal_label: Option<SelectedPortalLabel<'_>>,
    color_preview: Option<PortalColorPreviewRef>,
) -> PortalTree {
    let pairs = portal_pair_data(
        map,
        offsets,
        selected_edge,
        selected_portal_label,
        color_preview,
    );
    build_portal_tree_from_pairs(&pairs)
}

/// Variant of [`build_portal_tree`] that consumes pre-computed
/// pair data.
pub fn build_portal_tree_from_pairs(pairs: &[PortalPairData]) -> PortalTree {
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    let mut hitboxes: HashMap<(EdgeKey, String), (Vec2, Vec2)> = HashMap::new();
    let mut unique_id: usize = 1;

    for pair in pairs {
        let pair_root = tree.arena.new_node(GfxElement::new_void_with_id(
            pair.pair_channel,
            unique_id,
        ));
        unique_id += 1;
        tree.root.append(pair_root, &mut tree.arena);

        for (slot, area, hitbox, endpoint_id) in pair.endpoints.iter() {
            let element = GfxElement::new_area_non_indexed_with_id(
                area.clone(),
                *slot,
                unique_id,
            );
            unique_id += 1;
            let leaf = tree.arena.new_node(element);
            pair_root.append(leaf, &mut tree.arena);
            hitboxes.insert(
                (pair.identity.clone(), endpoint_id.clone()),
                *hitbox,
            );
        }
    }

    PortalTree { tree, hitboxes }
}

/// Result of [`build_portal_mutator_tree`].
pub struct PortalMutator {
    pub mutator: crate::gfx_structs::tree::MutatorTree<GfxMutator>,
    pub hitboxes: HashMap<(EdgeKey, String), (Vec2, Vec2)>,
}

/// Build a [`MutatorTree`](crate::gfx_structs::tree::MutatorTree)
/// that updates an already-registered portal tree to the current
/// state without rebuilding the arena.
pub fn build_portal_mutator_tree(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    selected_edge: Option<SelectedEdgeRef>,
    selected_portal_label: Option<SelectedPortalLabel<'_>>,
    color_preview: Option<PortalColorPreviewRef>,
) -> PortalMutator {
    let pairs = portal_pair_data(
        map,
        offsets,
        selected_edge,
        selected_portal_label,
        color_preview,
    );
    build_portal_mutator_tree_from_pairs(&pairs)
}

/// Variant of [`build_portal_mutator_tree`] that consumes
/// pre-computed pair data.
pub fn build_portal_mutator_tree_from_pairs(pairs: &[PortalPairData]) -> PortalMutator {
    use crate::core::primitives::ApplyOperation;
    use crate::gfx_structs::area::{DeltaGlyphArea, GlyphAreaField};
    use crate::gfx_structs::mutator::Mutation;
    use crate::gfx_structs::tree::MutatorTree;

    let mut mt: MutatorTree<GfxMutator> = MutatorTree::new_with(GfxMutator::new_void(0));
    let mut hitboxes: HashMap<(EdgeKey, String), (Vec2, Vec2)> = HashMap::new();

    for pair in pairs {
        let pair_node = mt.arena.new_node(GfxMutator::new_void(pair.pair_channel));
        mt.root.append(pair_node, &mut mt.arena);

        for (slot, area, hitbox, endpoint_id) in pair.endpoints.iter() {
            let delta = DeltaGlyphArea::new(vec![
                GlyphAreaField::Text(area.text.clone()),
                GlyphAreaField::position(area.position.x.0, area.position.y.0),
                GlyphAreaField::bounds(area.render_bounds.x.0, area.render_bounds.y.0),
                GlyphAreaField::scale(area.scale.0),
                GlyphAreaField::line_height(area.line_height.0),
                GlyphAreaField::ColorFontRegions(area.regions.clone()),
                GlyphAreaField::Outline(area.outline.clone()),
                GlyphAreaField::Operation(ApplyOperation::Assign),
            ]);
            let leaf = mt.arena.new_node(GfxMutator::new(
                Mutation::AreaDelta(Box::new(delta)),
                *slot,
            ));
            pair_node.append(leaf, &mut mt.arena);
            hitboxes.insert(
                (pair.identity.clone(), endpoint_id.clone()),
                *hitbox,
            );
        }
    }

    PortalMutator { mutator: mt, hitboxes }
}
