//! Portal tree builder — one `GlyphArea` per (portal-pair ×
//! endpoint). Mirrors the legacy `scene_builder::PortalElement`
//! emission rule: each `PortalPair` produces two markers, one
//! floating above each endpoint node's top-right corner.

use std::collections::HashMap;

use glam::Vec2;

use crate::core::primitives::{ColorFontRegion, ColorFontRegions, Range};
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::GfxMutator;
use crate::gfx_structs::tree::Tree;
use crate::mindmap::model::{MindMap, MindNode};
use crate::util::color;

// =====================================================================
// Portal tree builder
//
// Emits one baumhard `Tree<GfxElement, GfxMutator>` containing one
// `GlyphArea` per (portal-pair × endpoint). Mirrors the legacy
// `scene_builder::PortalElement` emission rule: each `PortalPair`
// produces two markers, one floating above each endpoint node's
// top-right corner.
// =====================================================================

/// Cyan highlight color for selected portals — kept in sync with
/// `scene_builder::SELECTED_PORTAL_COLOR_HEX`. Hardcoded as a
/// hex literal here too rather than re-exporting because the
/// scene_builder constant is private and the duplication is one
/// scalar.
const SELECTED_PORTAL_COLOR_HEX: &str = "#00E5FF";

/// Identifier for the currently selected portal pair, used to
/// route the cyan highlight color to the right two markers.
/// Tuple is `(label, endpoint_a, endpoint_b)` matching
/// [`crate::mindmap::scene_cache::PortalRefKey`]'s ordering.
pub type SelectedPortalRef<'a> = (&'a str, &'a str, &'a str);

/// Optional live preview of one portal pair's color, mirroring
/// `scene_builder::PortalColorPreview`. Wins over selection on
/// the previewed pair so the live HSV feedback is visible on
/// both markers.
#[derive(Debug, Clone, Copy)]
pub struct PortalColorPreviewRef<'a> {
    pub label: &'a str,
    pub endpoint_a: &'a str,
    pub endpoint_b: &'a str,
    pub color: &'a str,
}

/// Result of [`build_portal_tree`]. Bundles the tree with the
/// AABB-per-marker map the legacy `hit_test_portal` path needs
/// while it's still running. Session 5 wires hit-testing
/// through `Scene::component_at` and this auxiliary map goes
/// away.
pub struct PortalTree {
    pub tree: Tree<GfxElement, GfxMutator>,
    /// `((label, endpoint_a, endpoint_b), endpoint_node_id) → AABB`.
    pub hitboxes: HashMap<((String, String, String), String), (Vec2, Vec2)>,
}

/// Identity tuple for one portal pair: `(label, endpoint_a,
/// endpoint_b)`. Used to compare two consecutive
/// [`portal_pair_data`] outputs and decide whether a registered
/// portal tree's structure still matches — the prerequisite for
/// the in-place [`build_portal_mutator_tree`] path.
pub type PortalIdentity = (String, String, String);

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

/// Compute the visible-portal-pair layout for the given map state.
///
/// Single source of truth shared by [`build_portal_tree`] and
/// [`build_portal_mutator_tree`] so the two paths cannot drift.
/// Pairs are returned in `MindMap.portals` order, skipping any pair
/// whose endpoint is hidden by a folded ancestor (mirrors
/// `scene_builder::build_scene`).
///
/// # Costs
///
/// O(visible portal-pairs). Allocates a `Vec` plus two
/// `ColorFontRegions` per pair. Color resolution uses
/// [`color::resolve_var`] for `var(--name)` references.
pub fn portal_pair_data(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    selected_portal: Option<SelectedPortalRef>,
    color_preview: Option<PortalColorPreviewRef>,
) -> Vec<PortalPairData> {
    let vars = &map.canvas.theme_variables;
    let mut pairs: Vec<PortalPairData> = Vec::new();
    let mut pair_channel: usize = 1;

    for portal in &map.portals {
        let Some(node_a) = map.nodes.get(&portal.endpoint_a) else {
            continue;
        };
        let Some(node_b) = map.nodes.get(&portal.endpoint_b) else {
            continue;
        };
        if map.is_hidden_by_fold(node_a) || map.is_hidden_by_fold(node_b) {
            continue;
        }

        let is_selected = selected_portal.map_or(false, |(l, a, b)| {
            l == portal.label && a == portal.endpoint_a && b == portal.endpoint_b
        });
        let preview_for_this_portal: Option<&str> = color_preview.and_then(|p| {
            if p.label == portal.label
                && p.endpoint_a == portal.endpoint_a
                && p.endpoint_b == portal.endpoint_b
            {
                Some(p.color)
            } else {
                None
            }
        });
        let raw_color: &str = if let Some(p) = preview_for_this_portal {
            p
        } else if is_selected {
            SELECTED_PORTAL_COLOR_HEX
        } else {
            portal.color.as_str()
        };
        let color_hex = color::resolve_var(raw_color, vars);
        let color_rgba =
            color::hex_to_rgba_safe(color_hex, [0.92, 0.92, 0.92, 1.0]);

        let identity: PortalIdentity = (
            portal.label.clone(),
            portal.endpoint_a.clone(),
            portal.endpoint_b.clone(),
        );

        let make_endpoint = |slot: usize, endpoint: &MindNode| -> (usize, GlyphArea, (Vec2, Vec2), String) {
            let (ox, oy) = offsets.get(&endpoint.id).copied().unwrap_or((0.0, 0.0));
            let node_x = endpoint.position.x as f32 + ox;
            let node_y = endpoint.position.y as f32 + oy;
            let node_w = endpoint.size.width as f32;

            let bounds_w = portal.font_size_pt * 1.4;
            let bounds_h = portal.font_size_pt * 1.4;
            let top_left = Vec2::new(
                node_x + node_w - bounds_w * 0.9,
                node_y - bounds_h - 8.0,
            );

            let mut area = GlyphArea::new_with_str(
                &portal.glyph,
                portal.font_size_pt,
                portal.font_size_pt,
                top_left,
                Vec2::new(bounds_w, bounds_h),
            );
            let cluster_count = portal.glyph.chars().count();
            if cluster_count > 0 {
                let mut regions = ColorFontRegions::new_empty();
                regions.submit_region(ColorFontRegion::new(
                    Range::new(0, cluster_count),
                    None,
                    Some(color_rgba),
                ));
                area.regions = regions;
            }

            let max = top_left + Vec2::new(bounds_w, bounds_h);
            (slot, area, (top_left, max), endpoint.id.clone())
        };

        pairs.push(PortalPairData {
            identity,
            pair_channel,
            endpoints: [make_endpoint(1, node_a), make_endpoint(2, node_b)],
        });
        pair_channel += 1;
    }

    pairs
}

/// Identity sequence for a slice of [`PortalPairData`]. Compared
/// element-wise against a cached sequence to decide whether the
/// in-place [`build_portal_mutator_tree`] path is sound — if the
/// sequences disagree, a portal was added, removed, or reordered
/// (or an endpoint folded), and the caller must fall back to
/// [`build_portal_tree`] to rebuild the arena.
pub fn portal_identity_sequence(pairs: &[PortalPairData]) -> Vec<PortalIdentity> {
    pairs.iter().map(|p| p.identity.clone()).collect()
}

/// Build a baumhard tree of every visible portal marker.
///
/// Tree shape:
///
/// ```text
/// Void (root)
/// ├── Void (per portal pair — channel = pair index, 1-based)
/// │   ├── GlyphArea (endpoint A marker, channel = 1)
/// │   └── GlyphArea (endpoint B marker, channel = 2)
/// ├── Void (next pair) ...
/// ```
///
/// Pairs are emitted in `MindMap.portals` order (which is a
/// `Vec`, deterministic). Markers attached to folded nodes are
/// skipped, mirroring `scene_builder::build_scene`.
///
/// # Costs
///
/// O(visible portal-pairs × 2). Allocates one tree arena plus
/// the auxiliary `hitboxes` HashMap. Internally calls
/// [`portal_pair_data`] — both this initial-build path and the
/// in-place [`build_portal_mutator_tree`] path share that helper
/// so they cannot drift.
pub fn build_portal_tree(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    selected_portal: Option<SelectedPortalRef>,
    color_preview: Option<PortalColorPreviewRef>,
) -> PortalTree {
    let pairs = portal_pair_data(map, offsets, selected_portal, color_preview);
    build_portal_tree_from_pairs(&pairs)
}

/// Variant of [`build_portal_tree`] that consumes pre-computed
/// pair data. Use this when the caller already called
/// [`portal_pair_data`] for the dispatch check between full-rebuild
/// and the in-place [`build_portal_mutator_tree_from_pairs`] path —
/// avoids re-walking `MindMap.portals` twice per frame.
pub fn build_portal_tree_from_pairs(pairs: &[PortalPairData]) -> PortalTree {
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    let mut hitboxes: HashMap<((String, String, String), String), (Vec2, Vec2)> =
        HashMap::new();
    // `unique_id` (the second arg to the `_with_id` constructors) is
    // monotonically increasing per element across the whole tree;
    // it's a debug / hit-test affordance independent of the channel
    // values that the mutator path aligns on.
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

/// Result of [`build_portal_mutator_tree`]. The `mutator` is
/// applied to the tree returned by [`build_portal_tree`] via
/// `MutatorTree::apply_to`; `hitboxes` replaces the renderer's
/// portal hitbox map (positions move with offsets even on the
/// in-place path).
pub struct PortalMutator {
    pub mutator: crate::gfx_structs::tree::MutatorTree<GfxMutator>,
    pub hitboxes: HashMap<((String, String, String), String), (Vec2, Vec2)>,
}

/// Build a [`MutatorTree`](crate::gfx_structs::tree::MutatorTree)
/// that updates an already-registered portal tree to the current
/// `(map, offsets, selected, preview)` state without rebuilding
/// the arena. Pairs with [`build_portal_tree`] — channels are
/// stable across both **iff** the visible-portal identity sequence
/// hasn't changed since the original build.
///
/// Callers must verify the identity sequence first via
/// [`portal_identity_sequence`]; applying this mutator to a tree
/// whose structure has drifted will silently misalign because
/// Baumhard's `align_child_walks` matches mutator children
/// against target children by ascending channel.
///
/// Mirrors the canonical pattern from `color_picker` (commit
/// `ceaeeb4`): every entry is an `Assign` `DeltaGlyphArea` that
/// overwrites the variable fields (text, position, bounds, scale,
/// line_height, regions, outline) so a change in any one is picked
/// up by the same mutator shape.
pub fn build_portal_mutator_tree(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    selected_portal: Option<SelectedPortalRef>,
    color_preview: Option<PortalColorPreviewRef>,
) -> PortalMutator {
    let pairs = portal_pair_data(map, offsets, selected_portal, color_preview);
    build_portal_mutator_tree_from_pairs(&pairs)
}

/// Variant of [`build_portal_mutator_tree`] that consumes
/// pre-computed pair data. Use this in the dispatch path that
/// already called [`portal_pair_data`] to derive the identity
/// sequence — saves one pass.
pub fn build_portal_mutator_tree_from_pairs(pairs: &[PortalPairData]) -> PortalMutator {
    use crate::core::primitives::ApplyOperation;
    use crate::gfx_structs::area::{DeltaGlyphArea, GlyphAreaField};
    use crate::gfx_structs::mutator::Mutation;
    use crate::gfx_structs::tree::MutatorTree;

    let mut mt: MutatorTree<GfxMutator> = MutatorTree::new_with(GfxMutator::new_void(0));
    let mut hitboxes: HashMap<((String, String, String), String), (Vec2, Vec2)> =
        HashMap::new();

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

