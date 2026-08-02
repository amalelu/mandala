// SPDX-License-Identifier: MPL-2.0

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
//! Color, glyph, font size, and text all resolve through
//! [`super::portal_style`] — [`resolve_portal_endpoint_style`],
//! [`resolve_portal_endpoint_text_style`], `layout_portal_label`,
//! and `layout_portal_text` — so this file owns projection only
//! and never re-derives a style.
//!
//! Click routing reads that same shape. The tree's BVH answers
//! *where* a click landed; [`PortalHitIndex`] turns the resulting
//! channel path — pair channel, endpoint channel, icon-vs-text
//! slot — back into `(EdgeKey, endpoint node id, sub-part)`. No
//! rectangles are stored twice.

use std::collections::{HashMap, HashSet};

use glam::Vec2;
use indextree::NodeId;

use crate::core::primitives::ColorFontRegions;
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::GfxMutator;
use crate::gfx_structs::tree::{BranchChannel, Tree};
use crate::mindmap::model::{is_portal_edge, portal_endpoint_state, MindMap, MindNode};
use crate::util::geometry::aabb_center;

use super::overrides::{PortalColorPreview, PortalTextEditOverride};
use super::portal_style::{
    layout_portal_label, layout_portal_text, resolve_portal_endpoint_style,
    resolve_portal_endpoint_text_style, SelectedPortalLabel,
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

/// Result of [`build_portal_tree`]: the tree plus the
/// [`PortalHitIndex`] that names what each of its leaves *is*.
/// Geometry is in the tree and only in the tree — the index
/// carries identity alone, so there is no second copy of the
/// AABBs to keep in sync.
pub struct PortalTree {
    pub tree: Tree<GfxElement, GfxMutator>,
    /// Channel-path → `(EdgeKey, endpoint)` identity for every
    /// leaf in `tree`. Feed a `NodeId` from
    /// [`Tree::descendant_at`](crate::gfx_structs::tree::Tree::descendant_at)
    /// (or [`Scene::component_in`](crate::gfx_structs::scene::Scene::component_in))
    /// into [`PortalHitIndex::resolve`] to name the hit.
    pub hit_index: PortalHitIndex,
}

/// Which clickable sub-part of a portal endpoint a hit landed on.
/// The two are separate tree leaves under the same endpoint void,
/// so a click resolves to exactly one of them.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum PortalPart {
    /// The portal marker glyph itself.
    Icon,
    /// The endpoint's adjacent text label. Never hit when the
    /// endpoint has no text: an empty text slot lays out at zero
    /// extent, and zero-extent areas are invisible to the BVH.
    Text,
}

/// A portal hit resolved back to model identity: the owning
/// edge, the endpoint node the marker floats above, and which
/// sub-part of that endpoint was hit.
///
/// The endpoint id is the *near* side — the double-click
/// navigation target is the other endpoint of `edge_key`.
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct PortalHit {
    /// The portal-mode edge the hit marker belongs to.
    pub edge_key: EdgeKey,
    /// The node this marker floats above — one of
    /// `edge_key.from_id` / `edge_key.to_id`.
    pub endpoint_node_id: String,
    /// Which of the endpoint's two leaves was hit.
    pub part: PortalPart,
}

/// Identity of one visible portal pair, indexed by its
/// `pair_channel`. Endpoint ids are in canonical order —
/// index 0 is `from_id`, index 1 is `to_id` — matching the
/// endpoint-void channel layout the tree builder emits.
#[derive(Clone, Debug, Eq, PartialEq)]
struct PortalPairIdentity {
    edge_key: EdgeKey,
    endpoint_node_ids: [String; 2],
}

/// The channel-path → portal-identity map for one built portal
/// tree.
///
/// The portal tree's shape *is* its identity encoding: a leaf's
/// own channel says icon-vs-text, its parent's channel says which
/// endpoint, and its grandparent's channel says which visible
/// pair. This index supplies the one thing the shape cannot —
/// the `(EdgeKey, endpoint node id)` behind a pair channel — so
/// hit-testing needs no parallel spatial structure: the BVH over
/// the tree answers *where*, this answers *what*.
///
/// # Costs
///
/// Construction is O(pairs) with two `String` clones per
/// endpoint. [`Self::resolve`] is O(1): two parent hops in the
/// arena plus one slice index.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PortalHitIndex {
    pairs: Vec<PortalPairIdentity>,
}

impl PortalHitIndex {
    /// Build the index from the same [`PortalPairData`] slice the
    /// tree (or mutator) is built from, so the two cannot
    /// disagree about which pair owns which channel.
    ///
    /// # Costs
    ///
    /// O(pairs); one allocation for the vector plus two `String`
    /// clones per pair.
    pub fn from_pairs(pairs: &[PortalPairData]) -> Self {
        PortalHitIndex {
            pairs: pairs
                .iter()
                .map(|p| PortalPairIdentity {
                    edge_key: p.identity.clone(),
                    endpoint_node_ids: [
                        p.endpoints[0].endpoint_node_id.clone(),
                        p.endpoints[1].endpoint_node_id.clone(),
                    ],
                })
                .collect(),
        }
    }

    /// Number of visible portal pairs the index names. Two
    /// clickable endpoints each.
    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    /// Whether the index names no pairs — i.e. the portal tree is
    /// empty and no click can resolve to a portal.
    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// Name the portal sub-part a hit `NodeId` belongs to.
    ///
    /// `hit` must come from the tree this index was built
    /// alongside. Returns `None` for any node that is not a
    /// portal leaf (the root, a pair void, an endpoint void).
    ///
    /// An index *shorter* than the tree degrades to "no hit" on
    /// the uncovered pair channels. An index of the right length
    /// built from a different pair *order* would name the wrong
    /// portal — nothing in the data can catch that, so the
    /// contract is that the index is stamped in the same call that
    /// registers or mutates the tree (see
    /// `CanvasFrame::update_portal_tree`, which does both on both
    /// §B2 arms).
    ///
    /// # Costs
    ///
    /// O(1) — two `parent()` hops plus a slice index. No
    /// allocation beyond the two `String` clones in the returned
    /// [`PortalHit`].
    pub fn resolve(&self, tree: &Tree<GfxElement, GfxMutator>, hit: NodeId) -> Option<PortalHit> {
        let leaf = tree.arena.get(hit)?;
        let part = match leaf.get().channel() {
            ICON_SLOT => PortalPart::Icon,
            TEXT_SLOT => PortalPart::Text,
            _ => return None,
        };
        let endpoint_void_id = leaf.parent()?;
        let endpoint_void = tree.arena.get(endpoint_void_id)?;
        // Endpoint voids are 1-based by the tree-shape contract:
        // channel 1 is `from_id`, channel 2 is `to_id`.
        let endpoint_idx = endpoint_void.get().channel().checked_sub(1)?;
        let pair_void_id = endpoint_void.parent()?;
        let pair_channel = tree.arena.get(pair_void_id)?.get().channel();
        let pair = self.pairs.get(pair_channel.checked_sub(1)?)?;
        Some(PortalHit {
            edge_key: pair.edge_key.clone(),
            endpoint_node_id: pair.endpoint_node_ids.get(endpoint_idx)?.clone(),
            part,
        })
    }
}

/// Identity tuple for one portal-mode edge: the `EdgeKey` of the
/// owning edge. Used to compare two consecutive [`portal_pair_data`]
/// outputs and decide whether a registered portal tree's structure
/// still matches — the prerequisite for the in-place
/// [`build_portal_mutator_tree`] path.
pub type PortalIdentity = EdgeKey;

/// Per-endpoint tree-build output: the icon + text glyph areas
/// at their per-slot channels, plus the endpoint id. Both areas
/// come from the same source of truth (`layout_portal_label` +
/// `layout_portal_text`) so the initial-build and in-place
/// mutator paths cannot drift.
///
/// The areas carry their own hit geometry — an area's
/// `position` + `render_bounds` *is* its clickable rectangle,
/// resolved through the tree's BVH. A text-less endpoint's text
/// area lays out at zero extent and is therefore unclickable
/// (see `layout_portal_text`).
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
    /// `edge.to_id`). Carried into [`PortalHitIndex`] so a hit
    /// on either leaf resolves back to the endpoint it belongs
    /// to.
    pub endpoint_node_id: String,
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
///
/// `hidden_set` is the output of [`MindMap::fold_hidden_set`]. The
/// caller should build it once per frame and share it across the
/// border / connection / portal passes instead of recomputing it
/// here.
// `clippy::too_many_arguments`: three of these are independent
// selection channels (whole edge, per-label, inline text edit) and
// two more are the color preview and the camera zoom. Collapsing
// them into one struct would hide which channel a caller is
// actually setting, which is the whole point of the split.
#[allow(clippy::too_many_arguments)]
pub fn portal_pair_data(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    selected_edge: Option<SelectedEdgeRef>,
    selected_portal_label: Option<SelectedPortalLabel<'_>>,
    color_preview: Option<PortalColorPreview<'_>>,
    portal_text_edit: Option<PortalTextEditOverride<'_>>,
    camera_zoom: f32,
    hidden_set: &HashSet<&str>,
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
        if hidden_set.contains(node_a.id.as_str()) || hidden_set.contains(node_b.id.as_str()) {
            continue;
        }

        let edge_key = EdgeKey::from_edge(edge);
        let is_edge_selected = selected_edge.is_some_and(|(f, t, ty)| {
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
            let owner_pos = owner.pos_vec2() + Vec2::new(ox, oy);
            let owner_size = owner.size_vec2();
            let (px, py) = offsets.get(&partner.id).copied().unwrap_or((0.0, 0.0));
            let partner_pos = partner.pos_vec2() + Vec2::new(px, py);
            let partner_size = partner.size_vec2();

            let endpoint_state = portal_endpoint_state(edge, &owner.id);
            let is_this_label_selected = selected_portal_label
                .is_some_and(|s| *s.edge_key == edge_key && s.endpoint_node_id == owner.id);
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
                camera_zoom,
            );
            let text_style = resolve_portal_endpoint_text_style(
                edge,
                endpoint_state,
                &map.canvas,
                raw_color_override,
                &style.color,
                camera_zoom,
            );
            let icon_layout = layout_portal_label(
                owner_pos,
                owner_size,
                aabb_center(partner_pos, partner_size),
                endpoint_state,
                style.font_size_pt,
            );
            let icon_color_rgba = color::hex_to_rgba_safe(&style.color, [0.92, 0.92, 0.92, 1.0]);
            let text_color_rgba = color::hex_to_rgba_safe(&text_style.color, [0.92, 0.92, 0.92, 1.0]);

            // Inline edit preview wins over the committed `text`
            // so the user sees their buffer live. Empty string
            // (not absent) when neither source carries text —
            // the slot is always emitted so the mutator-tree
            // channel layout stays stable frame-to-frame.
            let text_string = match portal_text_edit {
                Some(p) if *p.edge_key == edge_key && p.endpoint_node_id == owner.id => p.buffer.to_string(),
                _ => endpoint_state.and_then(|s| s.text.clone()).unwrap_or_default(),
            };
            let text_layout = layout_portal_text(
                icon_layout,
                owner_pos,
                owner_size,
                aabb_center(partner_pos, partner_size),
                endpoint_state,
                style.font_size_pt,
                text_style.font_size_pt,
                &text_string,
            );

            // Replace-not-intersect cascade — both the icon and
            // text areas share the same resolved window so they
            // appear / disappear together. Resolver lives on
            // `MindEdge` so scene-builder and tree-builder can't
            // drift on the cascade rule.
            let endpoint_zoom_window = edge.portal_endpoint_zoom_window(endpoint_state);

            let mut icon_area = GlyphArea::new_with_str(
                &style.glyph,
                style.font_size_pt,
                style.font_size_pt,
                icon_layout.top_left,
                icon_layout.bounds,
            );
            icon_area.zoom_visibility = endpoint_zoom_window;
            let icon_clusters = crate::util::grapheme_chad::count_grapheme_clusters(&style.glyph);
            icon_area.regions = ColorFontRegions::single_span(icon_clusters, Some(icon_color_rgba), None);

            let mut text_area = GlyphArea::new_with_str(
                &text_string,
                text_style.font_size_pt,
                text_style.font_size_pt,
                text_layout.top_left,
                text_layout.bounds,
            );
            text_area.zoom_visibility = endpoint_zoom_window;
            let text_clusters = crate::util::grapheme_chad::count_grapheme_clusters(&text_string);
            text_area.regions = ColorFontRegions::single_span(text_clusters, Some(text_color_rgba), None);

            EndpointAreas {
                icon: icon_area,
                text: text_area,
                endpoint_node_id: owner.id.clone(),
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
    color_preview: Option<PortalColorPreview<'_>>,
    portal_text_edit: Option<PortalTextEditOverride<'_>>,
    camera_zoom: f32,
) -> PortalTree {
    let hidden_set = map.fold_hidden_set();
    let pairs = portal_pair_data(
        map,
        offsets,
        selected_edge,
        selected_portal_label,
        color_preview,
        portal_text_edit,
        camera_zoom,
        &hidden_set,
    );
    build_portal_tree_from_pairs(&pairs)
}

/// Variant of [`build_portal_tree`] that consumes pre-computed
/// pair data.
pub fn build_portal_tree_from_pairs(pairs: &[PortalPairData]) -> PortalTree {
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    let mut unique_id: usize = 1;

    for pair in pairs {
        let pair_root = tree
            .arena
            .new_node(GfxElement::new_void_with_id(pair.pair_channel, unique_id));
        unique_id += 1;
        tree.root.append(pair_root, &mut tree.arena);

        for (endpoint_idx, ep) in pair.endpoints.iter().enumerate() {
            let endpoint_channel = endpoint_idx + 1;
            let endpoint_void = tree
                .arena
                .new_node(GfxElement::new_void_with_id(endpoint_channel, unique_id));
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
        }
    }

    PortalTree {
        hit_index: PortalHitIndex::from_pairs(pairs),
        tree,
    }
}

/// Result of [`build_portal_mutator_tree`]. Carries the same
/// [`PortalHitIndex`] as [`PortalTree`]: an in-place update can
/// move a portal's geometry without changing the pair identity
/// sequence, but the caller re-stamps the index anyway so the
/// two build paths hand back interchangeable values.
pub struct PortalMutator {
    pub mutator: crate::gfx_structs::tree::MutatorTree<GfxMutator>,
    pub hit_index: PortalHitIndex,
}

/// Build a [`MutatorTree`](crate::gfx_structs::tree::MutatorTree)
/// that updates an already-registered portal tree to the current
/// state without rebuilding the arena.
pub fn build_portal_mutator_tree(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    selected_edge: Option<SelectedEdgeRef>,
    selected_portal_label: Option<SelectedPortalLabel<'_>>,
    color_preview: Option<PortalColorPreview<'_>>,
    portal_text_edit: Option<PortalTextEditOverride<'_>>,
    camera_zoom: f32,
) -> PortalMutator {
    let hidden_set = map.fold_hidden_set();
    let pairs = portal_pair_data(
        map,
        offsets,
        selected_edge,
        selected_portal_label,
        color_preview,
        portal_text_edit,
        camera_zoom,
        &hidden_set,
    );
    build_portal_mutator_tree_from_pairs(&pairs)
}

/// Variant of [`build_portal_mutator_tree`] that consumes
/// pre-computed pair data.
pub fn build_portal_mutator_tree_from_pairs(pairs: &[PortalPairData]) -> PortalMutator {
    use crate::gfx_structs::area::DeltaGlyphArea;
    use crate::gfx_structs::mutator::Mutation;
    use crate::gfx_structs::tree::MutatorTree;

    let mut mt: MutatorTree<GfxMutator> = MutatorTree::new_with(GfxMutator::new_void(0));

    for pair in pairs {
        let pair_node = mt.arena.new_node(GfxMutator::new_void(pair.pair_channel));
        mt.root.append(pair_node, &mut mt.arena);

        for (endpoint_idx, ep) in pair.endpoints.iter().enumerate() {
            let endpoint_channel = endpoint_idx + 1;
            let endpoint_void = mt.arena.new_node(GfxMutator::new_void(endpoint_channel));
            pair_node.append(endpoint_void, &mut mt.arena);

            for (slot, area) in [(ICON_SLOT, &ep.icon), (TEXT_SLOT, &ep.text)] {
                let delta = DeltaGlyphArea::full_assign_from(area);
                let leaf = mt
                    .arena
                    .new_node(GfxMutator::new(Mutation::AreaDelta(Box::new(delta)), slot));
                endpoint_void.append(leaf, &mut mt.arena);
            }
        }
    }

    PortalMutator {
        mutator: mt,
        hit_index: PortalHitIndex::from_pairs(pairs),
    }
}
