// SPDX-License-Identifier: MPL-2.0

//! Connection labels — the per-edge emission pass and the tree it
//! feeds.
//!
//! [`build_label_elements`] runs as a post-pass over `map.edges`:
//! labels are at most one per edge and rebuilt each frame at
//! trivial cost (no cache). Two internal passes:
//!
//! 1. Emit a [`ConnectionLabelElement`] for every visible edge with
//!    a non-empty committed `label`. If `label_edit_override`
//!    targets this edge, substitute the live buffer (+ caret) for
//!    the committed text so inline editing shows up on the next
//!    frame.
//!
//! 2. If `label_edit_override` targets an edge whose committed
//!    label is empty / `None` (so the first pass skipped it),
//!    synthesize a label element anyway. Without this, the caret
//!    for the very first character of a fresh label would not be
//!    visible.
//!
//! The tree half emits one `GlyphArea` per labeled edge at a
//! 1-based insertion channel, and returns a
//! [`ConnectionLabelHitIndex`] mapping that channel back to the
//! owning `EdgeKey` — so a BVH hit inside the tree resolves to an
//! edge without a second copy of the label rectangles.

use std::collections::{HashMap, HashSet};

use glam::Vec2;
use indextree::NodeId;

use crate::core::primitives::ColorFontRegions;
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::GfxMutator;
use crate::gfx_structs::tree::{BranchChannel, Tree};
use crate::mindmap::connection;
use crate::mindmap::model::{EdgeLabelConfig, GlyphConnectionConfig, MindEdge, MindMap, MindNode};
use crate::mindmap::scene_cache::EdgeKey;
use crate::mindmap::SELECTION_HIGHLIGHT_HEX;
use crate::util::color;
use crate::util::color::resolve_var;
use crate::util::geometry::almost_equal;
use crate::util::grapheme_chad::count_grapheme_clusters;

use super::overrides::EdgeColorPreview;

/// A text label attached to a connection edge. Rendered as a
/// cosmic-text buffer positioned along the edge's path at a
/// parameter-space `t` derived from
/// `MindEdge.label_config.position_t`, optionally shifted
/// perpendicular to the path by
/// `MindEdge.label_config.perpendicular_offset`.
///
/// The AABB (`position`, `bounds`) becomes the leaf `GlyphArea`'s
/// rectangle, which serves double duty: the renderer shapes the
/// text buffer inside it, and the tree's BVH hit-tests against it
/// so the app can route clicks on the label to inline editing.
pub struct ConnectionLabelElement {
    /// Stable identity of the edge carrying this label.
    pub edge_key: EdgeKey,
    /// The label text (guaranteed non-empty — labels with empty or
    /// missing text are not emitted).
    pub text: String,
    /// Top-left corner of the label's AABB, in canvas coordinates.
    /// Centered horizontally and vertically on the path point.
    pub position: (f32, f32),
    /// Width and height of the label's AABB. Sized loosely from the
    /// character count × an approximate glyph width.
    pub bounds: (f32, f32),
    /// Resolved color (hex) — `var(--name)` references already
    /// expanded through the theme variable map.
    pub color: String,
    /// Optional font family override. `None` falls back to the
    /// renderer's default font.
    pub font: Option<String>,
    /// Font size in points, already multiplied by the label's size
    /// factor (1.1× the body glyph size by default) and clamped by
    /// `GlyphConnectionConfig::effective_font_size_pt`.
    pub font_size_pt: f32,
    /// Zoom window for the label. Resolved with the replace-not-
    /// intersect cascade: `EdgeLabelConfig.min/max_zoom_to_render`
    /// override `MindEdge.min/max_zoom_to_render` when any of the
    /// pair is `Some`; otherwise inherit the edge window.
    pub zoom_visibility: crate::gfx_structs::zoom_visibility::ZoomVisibility,
}

/// Compute the geometry + cosmic-text-ready fields for one
/// connection-label, given pre-resolved text + raw color. Single
/// source for both the main-pass and synthesized-pass branches —
/// they previously open-coded the same path build, font-size
/// derivation, AABB sizing, and `resolve_var` step.
// `clippy::too_many_arguments`: this is the de-duplication of two
// call sites that each had these nine values in scope already.
// Bundling them into a struct would just rename the same data at
// both sites with no readability win.
#[allow(clippy::too_many_arguments)]
fn compute_label_layout(
    edge: &MindEdge,
    edge_key: EdgeKey,
    from_node: &MindNode,
    to_node: &MindNode,
    offsets: &HashMap<String, (f32, f32)>,
    map: &MindMap,
    camera_zoom: f32,
    rendered_label: String,
    raw_color: &str,
) -> ConnectionLabelElement {
    let (fox, foy) = offsets.get(&from_node.id).copied().unwrap_or((0.0, 0.0));
    let (tox, toy) = offsets.get(&to_node.id).copied().unwrap_or((0.0, 0.0));
    let from_pos = from_node.pos_vec2() + Vec2::new(fox, foy);
    let from_size = from_node.size_vec2();
    let to_pos = to_node.pos_vec2() + Vec2::new(tox, toy);
    let to_size = to_node.size_vec2();
    let path = connection::build_connection_path(
        from_pos,
        from_size,
        &edge.anchor_from,
        to_pos,
        to_size,
        &edge.anchor_to,
        &edge.control_points,
    );
    let label_cfg = edge.label_config.as_ref();
    let t = EdgeLabelConfig::effective_position_t(label_cfg);
    let perp = EdgeLabelConfig::effective_perpendicular_offset(label_cfg);
    let anchor_on_path = connection::point_at_t(&path, t);
    let anchor = apply_perpendicular_offset(&path, t, anchor_on_path, perp);
    let config = GlyphConnectionConfig::resolved_for(edge, &map.canvas);
    let font_size_pt = EdgeLabelConfig::effective_font_size_pt(label_cfg, edge, &map.canvas, camera_zoom);
    let color = resolve_var(raw_color, &map.canvas.theme_variables).to_string();

    // Loose AABB sized from the grapheme-count approximation
    // (`font_size * 0.6` per grapheme — same constant the connection
    // body sampler uses). Counting graphemes (not Unicode scalars)
    // keeps a family-ZWJ emoji at one slot wide instead of eleven
    // (§B3).
    let grapheme_count = count_grapheme_clusters(&rendered_label) as f32;
    let bounds_w = (grapheme_count * font_size_pt * 0.6).max(font_size_pt);
    let bounds_h = font_size_pt * 1.3;
    let top_left = (anchor.x - bounds_w * 0.5, anchor.y - bounds_h * 0.5);

    ConnectionLabelElement {
        edge_key,
        text: rendered_label,
        position: top_left,
        bounds: (bounds_w, bounds_h),
        color,
        font: config.font.clone(),
        font_size_pt,
        zoom_visibility: edge.label_zoom_window(label_cfg),
    }
}

/// Emit connection labels for the given map + overrides. Returns
/// the two-pass union: committed labels first, then (optionally) a
/// synthesized label for the inline-edited edge if its committed
/// label was empty.
pub fn build_label_elements(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    label_edit_override: Option<(&EdgeKey, &str)>,
    edge_color_preview: Option<EdgeColorPreview<'_>>,
    selected_edge_label: Option<&EdgeKey>,
    camera_zoom: f32,
    hidden_set: &HashSet<&str>,
) -> Vec<ConnectionLabelElement> {
    let mut connection_label_elements: Vec<ConnectionLabelElement> = Vec::new();
    let mut label_override_emitted = false;

    for edge in &map.edges {
        if !edge.visible {
            continue;
        }
        // Portal-mode edges have no path, so no label position along
        // a path — skip them here.
        if crate::mindmap::model::is_portal_edge(edge) {
            continue;
        }
        let from_node = match map.nodes.get(&edge.from_id) {
            Some(n) => n,
            None => continue,
        };
        let to_node = match map.nodes.get(&edge.to_id) {
            Some(n) => n,
            None => continue,
        };
        if hidden_set.contains(from_node.id.as_str()) || hidden_set.contains(to_node.id.as_str()) {
            continue;
        }

        let edge_key = EdgeKey::from_edge(edge);
        let is_edited = label_edit_override.is_some_and(|(k, _)| *k == edge_key);

        // The label text to render: either the inline edit buffer
        // (plus caret) for the currently-edited edge, or the
        // committed label. Committed-empty non-edited edges skip
        // emission entirely.
        let rendered_label: String = if is_edited {
            let (_, buf) = label_edit_override.unwrap();
            format!("{buf}\u{258C}")
        } else {
            match edge.label.as_deref() {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => continue,
            }
        };

        // Color cascade, highest priority first:
        //   1. Edge-label selection highlight — cyan tint so the
        //      user sees which label carries focus. Wins over
        //      every other source because "I clicked this label"
        //      is the strongest intent signal on the screen.
        //   2. Color picker hover preview — substitutes the
        //      wheel-picker's in-flight hex while the user drags
        //      around the swatch.
        //   3. Committed color cascade — `label_config.color` →
        //      `glyph_connection.color` → `edge.color`.
        // `resolve_var` (inside compute_label_layout) runs after
        // selection so a preview value like `var(--accent)` still
        // theme-resolves correctly.
        let is_selected = selected_edge_label.is_some_and(|key| *key == edge_key);
        let raw_color: &str = if is_selected {
            SELECTION_HIGHLIGHT_HEX
        } else {
            edge_color_preview
                .and_then(|p| {
                    if *p.edge_key == edge_key {
                        Some(p.color)
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| edge.label_color(&map.canvas))
        };

        if is_edited {
            label_override_emitted = true;
        }

        connection_label_elements.push(compute_label_layout(
            edge,
            edge_key,
            from_node,
            to_node,
            offsets,
            map,
            camera_zoom,
            rendered_label,
            raw_color,
        ));
    }

    // Synthesized-label pass: if `label_edit_override` targets an
    // edge whose committed label was empty / None (so the first
    // pass skipped it), emit a label element anyway so the caret
    // for the very first character is visible. Fills the gap in
    // the previous renderer-side override path, whose
    // "belt and suspenders" branch was a dead no-op for this case.
    if let Some((target_key, buffer)) = label_edit_override {
        if !label_override_emitted {
            if let Some(edge) = map
                .edges
                .iter()
                .find(|e| e.visible && EdgeKey::from_edge(e) == *target_key)
            {
                if let (Some(from_node), Some(to_node)) =
                    (map.nodes.get(&edge.from_id), map.nodes.get(&edge.to_id))
                {
                    if !hidden_set.contains(from_node.id.as_str())
                        && !hidden_set.contains(to_node.id.as_str())
                    {
                        // Synthesized path is for an edge being
                        // edited with an empty committed label — no
                        // selection-highlight branch here (the edited
                        // edge IS the label, and the color picker
                        // preview wins where it applies).
                        let raw_color: &str = edge_color_preview
                            .and_then(|p| {
                                if p.edge_key == target_key {
                                    Some(p.color)
                                } else {
                                    None
                                }
                            })
                            .unwrap_or_else(|| edge.label_color(&map.canvas));
                        connection_label_elements.push(compute_label_layout(
                            edge,
                            target_key.clone(),
                            from_node,
                            to_node,
                            offsets,
                            map,
                            camera_zoom,
                            format!("{buffer}\u{258C}"),
                            raw_color,
                        ));
                    }
                }
            }
        }
    }

    connection_label_elements
}

/// Apply a signed perpendicular offset to an on-path anchor,
/// shifting it along the path normal at parameter `t`. The
/// normal is [`connection::normal_at_t`] — a canvas-coords 90°
/// rotation of the tangent, whose orientation vs. "direction of
/// travel" is documented on that helper (Y-down canvas). A
/// positive offset moves the label in the returned normal
/// direction, a negative one reverses. Zero is an early-out so
/// labels with no `perpendicular_offset` skip the tangent
/// computation entirely.
fn apply_perpendicular_offset(path: &connection::ConnectionPath, t: f32, anchor: Vec2, perp: f32) -> Vec2 {
    if almost_equal(perp, 0.0) {
        return anchor;
    }
    anchor + connection::normal_at_t(path, t) * perp
}

/// Output of [`build_connection_label_tree`]: the baumhard tree of
/// per-label GlyphAreas plus the [`ConnectionLabelHitIndex`] that
/// names them. The AABBs live in the tree's `GlyphArea`s and
/// nowhere else; the index carries only channel → edge identity.
pub struct ConnectionLabelTree {
    pub tree: Tree<GfxElement, GfxMutator>,
    /// Channel → owning [`EdgeKey`] for every label leaf in
    /// `tree`.
    pub hit_index: ConnectionLabelHitIndex,
}

/// The channel → edge-identity map for one built connection-label
/// tree.
///
/// The label tree is flat: one `GlyphArea` leaf per labeled edge,
/// at the 1-based insertion channel. A BVH hit inside the tree
/// therefore names its edge by reading the hit leaf's own channel
/// — this index supplies the `EdgeKey` behind that channel, and
/// nothing else. Sibling of
/// [`PortalHitIndex`](super::portal::PortalHitIndex) for the
/// deeper portal shape.
///
/// # Costs
///
/// Construction is O(labels) with one `EdgeKey` clone each.
/// [`Self::resolve`] is O(1): one arena lookup plus a slice index.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConnectionLabelHitIndex {
    edges: Vec<EdgeKey>,
}

impl ConnectionLabelHitIndex {
    /// Build the index from the same element slice the tree (or
    /// mutator) is built from, so channel assignment cannot
    /// diverge between the two.
    ///
    /// # Costs
    ///
    /// O(labels); one vector allocation plus one `EdgeKey` clone
    /// per label.
    pub fn from_elements(elements: &[ConnectionLabelElement]) -> Self {
        ConnectionLabelHitIndex {
            edges: connection_label_identity_sequence(elements),
        }
    }

    /// Number of labeled edges the index names.
    pub fn len(&self) -> usize {
        self.edges.len()
    }

    /// Whether the index names no labels — i.e. no click can
    /// resolve to an edge label.
    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }

    /// Name the edge a hit `NodeId` belongs to.
    ///
    /// `hit` must come from the tree this index was built
    /// alongside. Returns `None` for the tree root and for a
    /// channel the index does not cover, so a mismatched
    /// tree/index pair degrades to "no hit" rather than naming
    /// the wrong edge.
    ///
    /// # Costs
    ///
    /// O(1) — one arena lookup, one slice index, one `EdgeKey`
    /// clone on a hit.
    pub fn resolve(&self, tree: &Tree<GfxElement, GfxMutator>, hit: NodeId) -> Option<EdgeKey> {
        let channel = tree.arena.get(hit)?.get().channel();
        self.edges.get(channel.checked_sub(1)?).cloned()
    }
}

/// Identity sequence for a slice of `ConnectionLabelElement`s — the
/// edge-key of each labeled edge, in tree-insertion order. Two
/// label sets share an identity iff the *set of labeled edges* and
/// their order match. Label text, position, color, and font size
/// can vary inside that envelope; only adding, removing, or
/// reordering a labeled edge breaks the equality and forces a full
/// rebuild via the dispatcher in `update_connection_label_tree`.
pub fn connection_label_identity_sequence(elements: &[ConnectionLabelElement]) -> Vec<EdgeKey> {
    elements.iter().map(|e| e.edge_key.clone()).collect()
}

/// Lay out one connection-label as the `(channel, GlyphArea)`
/// pair both build paths emit. Channel is the 1-based label
/// index, matching the ascending insertion order. Single source
/// of truth — the initial-build path
/// ([`build_connection_label_tree`]) and the in-place mutator
/// path ([`build_connection_label_mutator_tree`]) cannot drift.
fn connection_label_layout(channel: usize, elem: &ConnectionLabelElement) -> (usize, GlyphArea) {
    let color_rgba = color::hex_to_rgba_safe(&elem.color, [0.92, 0.92, 0.92, 1.0]);
    let pos = Vec2::new(elem.position.0, elem.position.1);
    let bounds = Vec2::new(elem.bounds.0, elem.bounds.1);

    let mut area = GlyphArea::new_with_str(&elem.text, elem.font_size_pt, elem.font_size_pt, pos, bounds);
    area.zoom_visibility = elem.zoom_visibility;
    let cluster_count = crate::util::grapheme_chad::count_grapheme_clusters(&elem.text);
    area.regions = ColorFontRegions::single_span(cluster_count, Some(color_rgba), None);

    (channel, area)
}

/// Build a baumhard tree of every connection-label glyph from a
/// pre-computed `ConnectionLabelElement` slice. Like the
/// connection tree, geometry comes from `build_label_elements`
/// above.
///
/// Channels are sequential (1-based) per labeled edge so the
/// in-place [`build_connection_label_mutator_tree`] path can target
/// each leaf by its insertion index when the identity sequence
/// matches.
pub fn build_connection_label_tree(elements: &[ConnectionLabelElement]) -> ConnectionLabelTree {
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    let mut unique_id: usize = 1;

    for (idx, elem) in elements.iter().enumerate() {
        let (channel, area) = connection_label_layout(idx + 1, elem);
        let element_node = GfxElement::new_area_non_indexed_with_id(area, channel, unique_id);
        unique_id += 1;
        let leaf = tree.arena.new_node(element_node);
        tree.root.append(leaf, &mut tree.arena);
    }

    ConnectionLabelTree {
        tree,
        hit_index: ConnectionLabelHitIndex::from_elements(elements),
    }
}

/// Result of [`build_connection_label_mutator_tree`]. The `mutator`
/// is applied to the tree returned by [`build_connection_label_tree`]
/// via `MutatorTree::apply_to`; `hit_index` is re-stamped alongside
/// it so both build paths hand back interchangeable values.
pub struct ConnectionLabelMutator {
    pub mutator: crate::gfx_structs::tree::MutatorTree<GfxMutator>,
    pub hit_index: ConnectionLabelHitIndex,
}

/// Build a [`MutatorTree`](crate::gfx_structs::tree::MutatorTree)
/// that updates an already-registered connection-label tree to the
/// current `elements` state without rebuilding the arena. Pairs
/// with [`build_connection_label_tree`] — channels and insertion
/// order match, so applying this mutator to a tree built from a
/// label slice with the same identity sequence (per
/// [`connection_label_identity_sequence`]) updates each label's
/// variable fields in place.
pub fn build_connection_label_mutator_tree(elements: &[ConnectionLabelElement]) -> ConnectionLabelMutator {
    use crate::gfx_structs::area::DeltaGlyphArea;
    use crate::gfx_structs::mutator::Mutation;
    use crate::gfx_structs::tree::MutatorTree;

    let mut mt: MutatorTree<GfxMutator> = MutatorTree::new_with(GfxMutator::new_void(0));

    for (idx, elem) in elements.iter().enumerate() {
        let (channel, area) = connection_label_layout(idx + 1, elem);
        let delta = DeltaGlyphArea::full_assign_from(&area);
        let leaf = mt
            .arena
            .new_node(GfxMutator::new(Mutation::AreaDelta(Box::new(delta)), channel));
        mt.root.append(leaf, &mut mt.arena);
    }

    ConnectionLabelMutator {
        mutator: mt,
        hit_index: ConnectionLabelHitIndex::from_elements(elements),
    }
}
