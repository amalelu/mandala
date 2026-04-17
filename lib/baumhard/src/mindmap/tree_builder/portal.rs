//! Portal tree builder — one subtree per (portal-mode edge ×
//! endpoint). Each endpoint subtree carries an **icon** glyph
//! (the portal marker) and a **text** glyph (the endpoint's
//! text label — empty string when no text is set). The text is
//! a sibling of the icon under a per-endpoint `Void` parent so
//! the baumhard tree shape encodes the "text belongs to this
//! portal symbol" relationship structurally.
//!
//! ```text
//! root
//! └── Void (pair_channel = visible-portal index)
//!     ├── Void (endpoint_channel = 1, for from_id)
//!     │   ├── GlyphArea slot=1 (icon)
//!     │   └── GlyphArea slot=2 (text)
//!     └── Void (endpoint_channel = 2, for to_id)
//!         ├── GlyphArea slot=1 (icon)
//!         └── GlyphArea slot=2 (text)
//! ```
//!
//! Mirrors the [`scene_builder::portal`] emission rule: color,
//! glyph, font size, and text all resolve through
//! [`scene_builder::portal::resolve_portal_endpoint_style`] plus
//! the `layout_portal_label` / `layout_portal_text` helpers so
//! the scene-path and tree-path cannot drift.
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
    layout_portal_label, layout_portal_text, node_center, resolve_portal_endpoint_style,
    SelectedPortalLabel,
};
use crate::mindmap::scene_builder::PortalTextEditOverride;
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
/// hitbox map the renderer consults for click dispatch. One
/// hitbox per endpoint, spanning both the icon AABB and the
/// text AABB — clicking the text behaves identically to
/// clicking the icon (both select the same portal label).
pub struct PortalTree {
    pub tree: Tree<GfxElement, GfxMutator>,
    /// `(edge_key, endpoint_node_id) → (min, max)` spanning the
    /// union of icon + text AABBs. Renderer keys this into its
    /// portal hit-test map so click dispatch finds the endpoint
    /// under the cursor regardless of which half of the label
    /// was hit.
    pub hitboxes: HashMap<(EdgeKey, String), (Vec2, Vec2)>,
}

/// Identity tuple for one portal-mode edge: the `EdgeKey` of the
/// owning edge. Used to compare two consecutive [`portal_pair_data`]
/// outputs and decide whether a registered portal tree's structure
/// still matches — the prerequisite for the in-place
/// [`build_portal_mutator_tree`] path.
pub type PortalIdentity = EdgeKey;

/// Per-endpoint tree-build output: the icon + text glyph areas
/// at their per-slot channels, the endpoint id, and a combined
/// AABB for hit testing. Each field is computed from the same
/// source of truth (`layout_portal_label` + `layout_portal_text`)
/// so the scene and tree paths cannot drift.
#[derive(Clone, Debug)]
pub struct EndpointAreas {
    /// Icon glyph area (always slot channel 1 under the
    /// endpoint void parent).
    pub icon: GlyphArea,
    /// Text glyph area (always slot channel 2). Text is the
    /// empty string when the endpoint has no committed text
    /// and no inline-edit override — the slot is always
    /// present to keep the channel layout stable across
    /// text-set / text-absent frames, letting the mutator-tree
    /// in-place update path keep working.
    pub text: GlyphArea,
    /// Endpoint node id (identical to one of `edge.from_id` /
    /// `edge.to_id`). Used by the renderer to key per-endpoint
    /// hitboxes and to route click dispatch.
    pub endpoint_node_id: String,
    /// Combined AABB spanning both icon and text. Stored on the
    /// pair so the renderer's hit-test map sees one rect per
    /// endpoint — makes clicks on the text behave identically
    /// to clicks on the icon.
    pub hitbox: (Vec2, Vec2),
}

/// Per-pair output of [`portal_pair_data`]. Single source of
/// truth for portal layout consumed by both
/// [`build_portal_tree`] (initial build) and
/// [`build_portal_mutator_tree`] (in-place update).
///
/// `pair_channel` is sequential by visible-portal index —
/// stable across two calls **iff** their visible-portal
/// sequences are identical (same identities in the same order).
/// Callers detect drift by comparing identity slices and fall
/// back to a full rebuild when they disagree.
#[derive(Clone, Debug)]
pub struct PortalPairData {
    pub identity: PortalIdentity,
    pub pair_channel: usize,
    /// Per endpoint, in canonical order: index 0 = `from_id`,
    /// index 1 = `to_id`. Endpoint-void channels are 1 and 2
    /// respectively, fixed by the tree-shape contract.
    pub endpoints: [EndpointAreas; 2],
}

/// Compute the visible portal-mode-edge layout for the given
/// map state. Single source of truth shared by
/// [`build_portal_tree`] and [`build_portal_mutator_tree`].
pub fn portal_pair_data(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    selected_edge: Option<SelectedEdgeRef>,
    selected_portal_label: Option<SelectedPortalLabel<'_>>,
    color_preview: Option<PortalColorPreviewRef>,
    portal_text_edit: Option<PortalTextEditOverride<'_>>,
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

        let make_endpoint = |owner: &MindNode, partner: &MindNode| -> EndpointAreas {
            let (ox, oy) = offsets.get(&owner.id).copied().unwrap_or((0.0, 0.0));
            let owner_pos =
                Vec2::new(owner.position.x as f32 + ox, owner.position.y as f32 + oy);
            let owner_size = Vec2::new(owner.size.width as f32, owner.size.height as f32);
            let (px, py) = offsets.get(&partner.id).copied().unwrap_or((0.0, 0.0));
            let partner_pos =
                Vec2::new(partner.position.x as f32 + px, partner.position.y as f32 + py);
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
            let icon_layout = layout_portal_label(
                owner_pos,
                owner_size,
                node_center(partner_pos, partner_size),
                endpoint_state,
                style.font_size_pt,
            );
            let color_rgba =
                color::hex_to_rgba_safe(&style.color, [0.92, 0.92, 0.92, 1.0]);

            // Inline edit preview wins over the committed `text`
            // so the user sees their buffer live. Empty string
            // (not absent) when neither source carries text —
            // the slot is always emitted so the mutator-tree
            // channel layout stays stable frame-to-frame.
            let text_string = match portal_text_edit {
                Some(p) if *p.edge_key == edge_key && p.endpoint_node_id == owner.id => {
                    p.buffer.to_string()
                }
                _ => endpoint_state
                    .and_then(|s| s.text.clone())
                    .unwrap_or_default(),
            };
            let text_layout = layout_portal_text(
                icon_layout,
                owner_pos,
                owner_size,
                node_center(partner_pos, partner_size),
                endpoint_state,
                style.font_size_pt,
                &text_string,
            );

            let mut icon_area = GlyphArea::new_with_str(
                &style.glyph,
                style.font_size_pt,
                style.font_size_pt,
                icon_layout.top_left,
                icon_layout.bounds,
            );
            let icon_clusters =
                crate::util::grapheme_chad::count_grapheme_clusters(&style.glyph);
            if icon_clusters > 0 {
                let mut regions = ColorFontRegions::new_empty();
                regions.submit_region(ColorFontRegion::new(
                    Range::new(0, icon_clusters),
                    None,
                    Some(color_rgba),
                ));
                icon_area.regions = regions;
            }

            let mut text_area = GlyphArea::new_with_str(
                &text_string,
                style.font_size_pt,
                style.font_size_pt,
                text_layout.top_left,
                text_layout.bounds,
            );
            let text_clusters =
                crate::util::grapheme_chad::count_grapheme_clusters(&text_string);
            if text_clusters > 0 {
                let mut regions = ColorFontRegions::new_empty();
                regions.submit_region(ColorFontRegion::new(
                    Range::new(0, text_clusters),
                    None,
                    Some(color_rgba),
                ));
                text_area.regions = regions;
            }

            // Combined hitbox: rectangular union of icon + text
            // AABBs. Clicking anywhere in this rect dispatches
            // as a click on this portal label — icon and text
            // behave as one target.
            let icon_min = icon_layout.top_left;
            let icon_max = icon_layout.top_left + icon_layout.bounds;
            let text_min = text_layout.top_left;
            let text_max = text_layout.top_left + text_layout.bounds;
            let hitbox_min = Vec2::new(icon_min.x.min(text_min.x), icon_min.y.min(text_min.y));
            let hitbox_max = Vec2::new(icon_max.x.max(text_max.x), icon_max.y.max(text_max.y));

            EndpointAreas {
                icon: icon_area,
                text: text_area,
                endpoint_node_id: owner.id.clone(),
                hitbox: (hitbox_min, hitbox_max),
            }
        };

        let endpoints = [make_endpoint(node_a, node_b), make_endpoint(node_b, node_a)];
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

// Tree-shape channel constants — fixed by contract so the
// mutator path can align against the initial build.
const ICON_SLOT: usize = 1;
const TEXT_SLOT: usize = 2;

/// Build a baumhard tree of every visible portal marker.
pub fn build_portal_tree(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    selected_edge: Option<SelectedEdgeRef>,
    selected_portal_label: Option<SelectedPortalLabel<'_>>,
    color_preview: Option<PortalColorPreviewRef>,
    portal_text_edit: Option<PortalTextEditOverride<'_>>,
) -> PortalTree {
    let pairs = portal_pair_data(
        map,
        offsets,
        selected_edge,
        selected_portal_label,
        color_preview,
        portal_text_edit,
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

        for (endpoint_idx, ep) in pair.endpoints.iter().enumerate() {
            let endpoint_channel = endpoint_idx + 1;
            let endpoint_void = tree.arena.new_node(GfxElement::new_void_with_id(
                endpoint_channel,
                unique_id,
            ));
            unique_id += 1;
            pair_root.append(endpoint_void, &mut tree.arena);

            let icon_leaf = tree.arena.new_node(GfxElement::new_area_non_indexed_with_id(
                ep.icon.clone(),
                ICON_SLOT,
                unique_id,
            ));
            unique_id += 1;
            endpoint_void.append(icon_leaf, &mut tree.arena);

            let text_leaf = tree.arena.new_node(GfxElement::new_area_non_indexed_with_id(
                ep.text.clone(),
                TEXT_SLOT,
                unique_id,
            ));
            unique_id += 1;
            endpoint_void.append(text_leaf, &mut tree.arena);

            hitboxes.insert(
                (pair.identity.clone(), ep.endpoint_node_id.clone()),
                ep.hitbox,
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
    portal_text_edit: Option<PortalTextEditOverride<'_>>,
) -> PortalMutator {
    let pairs = portal_pair_data(
        map,
        offsets,
        selected_edge,
        selected_portal_label,
        color_preview,
        portal_text_edit,
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

        for (endpoint_idx, ep) in pair.endpoints.iter().enumerate() {
            let endpoint_channel = endpoint_idx + 1;
            let endpoint_void = mt.arena.new_node(GfxMutator::new_void(endpoint_channel));
            pair_node.append(endpoint_void, &mut mt.arena);

            for (slot, area) in [(ICON_SLOT, &ep.icon), (TEXT_SLOT, &ep.text)] {
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
                    slot,
                ));
                endpoint_void.append(leaf, &mut mt.arena);
            }

            hitboxes.insert(
                (pair.identity.clone(), ep.endpoint_node_id.clone()),
                ep.hitbox,
            );
        }
    }

    PortalMutator { mutator: mt, hitboxes }
}
