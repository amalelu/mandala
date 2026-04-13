use std::collections::HashMap;

use glam::Vec2;
use indextree::NodeId;

use crate::core::primitives::{ColorFontRegion, ColorFontRegions, Range};
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::GfxMutator;
use crate::gfx_structs::tree::Tree;
use crate::mindmap::model::{MindMap, MindNode};
use crate::util::color;

/// Result of building a Baumhard tree from a MindMap.
/// The tree mirrors the MindMap's parent-child hierarchy,
/// with each MindNode represented as a GlyphArea element.
pub struct MindMapTree {
    pub tree: Tree<GfxElement, GfxMutator>,
    /// Maps MindNode ID → indextree NodeId for later lookup.
    pub node_map: HashMap<String, NodeId>,
}

/// Builds a `Tree<GfxElement, GfxMutator>` from a MindMap's hierarchy.
///
/// The tree structure mirrors the MindMap's parent-child relationships:
/// - A Void root node at the top
/// - Each root MindNode (parent_id is None) as a child of the Void root
/// - Children nested recursively following parent_id
/// - Nodes hidden by fold state are excluded
///
/// Each MindNode becomes a GlyphArea element with its text, position,
/// size, and color regions.
pub fn build_mindmap_tree(map: &MindMap) -> MindMapTree {
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    let mut node_map: HashMap<String, NodeId> = HashMap::new();
    let mut id_counter: usize = 1; // 0 is reserved for the Void root

    let vars = &map.canvas.theme_variables;
    let roots = map.root_nodes();
    for root in &roots {
        if map.is_hidden_by_fold(root) {
            continue;
        }
        let area = mindnode_to_glyph_area(root, vars);
        let element = GfxElement::new_area_non_indexed_with_id(area, 0, id_counter);
        id_counter += 1;

        let node_id = tree.arena.new_node(element);
        tree.root.append(node_id, &mut tree.arena);
        node_map.insert(root.id.clone(), node_id);

        build_children_recursive(map, &root.id, node_id, &mut tree, &mut node_map, &mut id_counter);
    }

    MindMapTree { tree, node_map }
}

fn build_children_recursive(
    map: &MindMap,
    parent_mind_id: &str,
    parent_node_id: NodeId,
    tree: &mut Tree<GfxElement, GfxMutator>,
    node_map: &mut HashMap<String, NodeId>,
    id_counter: &mut usize,
) {
    let vars = &map.canvas.theme_variables;
    let children = map.children_of(parent_mind_id);
    for child in &children {
        if map.is_hidden_by_fold(child) {
            continue;
        }
        let area = mindnode_to_glyph_area(child, vars);
        let element = GfxElement::new_area_non_indexed_with_id(area, 0, *id_counter);
        *id_counter += 1;

        let child_node_id = tree.arena.new_node(element);
        parent_node_id.append(child_node_id, &mut tree.arena);
        node_map.insert(child.id.clone(), child_node_id);

        build_children_recursive(map, &child.id, child_node_id, tree, node_map, id_counter);
    }
}

/// Converts a MindNode's data into a Baumhard GlyphArea. Text-run colors
/// are resolved through the map's theme variables before being converted
/// to RGBA; unknown references and malformed hex fall back to transparent
/// black rather than panicking so a theme typo can't crash the render.
fn mindnode_to_glyph_area(node: &MindNode, vars: &HashMap<String, String>) -> GlyphArea {
    let scale = node
        .text_runs
        .first()
        .map(|r| r.size_pt as f32)
        .unwrap_or(14.0);
    let line_height = scale * 1.2;
    let position = Vec2::new(node.position.x as f32, node.position.y as f32);
    let bounds = Vec2::new(node.size.width as f32, node.size.height as f32);

    let mut area = GlyphArea::new_with_str(&node.text, scale, line_height, position, bounds);

    // Resolve the node's background color through theme variables and
    // pack it as u8 RGBA onto the tree element. The renderer's rect
    // pipeline reads it back out during `rebuild_buffers_from_tree`
    // and emits a solid quad behind the text glyphs.
    //
    // `None` means "no fill" — the canvas background shows through.
    // Both an empty string and a fully-transparent alpha ("#00000000"
    // / "#0000") map to `None`. Bad hex degrades to `None` as well,
    // so a theme typo leaves the node transparent rather than
    // painting it opaque black.
    area.background_color = {
        let raw = &node.style.background_color;
        if raw.is_empty() {
            None
        } else {
            let resolved = color::resolve_var(raw, vars);
            // Sentinel alpha = 0 means "parse failed" here because
            // the fallback is fully transparent. Authors can also
            // opt out with an explicit `#00000000` / `#0000`, which
            // lands on the same sentinel for free.
            let rgba = color::hex_to_rgba_safe(resolved, [0.0, 0.0, 0.0, 0.0]);
            if rgba[3] <= 0.0 {
                None
            } else {
                Some(color::convert_f32_to_u8(&rgba))
            }
        }
    };

    // Convert text runs to ColorFontRegions
    let mut regions = ColorFontRegions::new_empty();
    for run in &node.text_runs {
        let resolved = color::resolve_var(&run.color, vars);
        let rgba = color::hex_to_rgba_safe(resolved, [0.0, 0.0, 0.0, 1.0]);
        regions.submit_region(ColorFontRegion::new(
            Range::new(run.start, run.end),
            None, // Font: use default (cosmic-text resolves family names at render time)
            Some(rgba),
        ));
    }
    area.regions = regions;

    area
}

// =====================================================================
// Border tree builder
//
// Emits one baumhard `Tree<GfxElement, GfxMutator>` that, when walked
// into cosmic-text buffers, reproduces the same four box-drawing runs
// per framed node that the legacy `scene_builder::BorderElement` +
// `renderer::rebuild_border_buffers_keyed` pair produces.
//
// Layout constants (`BORDER_CORNER_OVERLAP_FRAC`,
// `BORDER_APPROX_CHAR_WIDTH_FRAC`) live on `crate::mindmap::border`
// so the renderer's keyed-buffer rebuild and this builder share one
// source of truth.
// =====================================================================

use crate::mindmap::border::{BORDER_APPROX_CHAR_WIDTH_FRAC, BORDER_CORNER_OVERLAP_FRAC};

/// Build a baumhard tree representing every framed node's border
/// glyphs. The tree's shape is:
///
/// ```text
/// Void (root)
/// ├── Void (per node — channel = id_counter)
/// │   ├── GlyphArea (top run, channel = 1)
/// │   ├── GlyphArea (bottom run, channel = 2)
/// │   ├── GlyphArea (left column, channel = 3)
/// │   └── GlyphArea (right column, channel = 4)
/// ├── Void (next node)
/// │   └── ...
/// ```
///
/// The per-node Void parent is not strictly necessary for rendering
/// but it gives mutator trees a natural target for whole-node
/// border changes (e.g. color change across all four runs).
///
/// Iteration order is the lexicographic order of `MindNode.id` —
/// stable across runs so per-node Void parents always land in the
/// same arena slot. Without this, `MindMap.nodes` (a `HashMap`)
/// would yield nondeterministic order, making mutator-tree
/// authoring against "the third framed node" unreliable.
///
/// # Costs
///
/// O(N log N) where N is the visible framed-node count (the sort
/// dominates for large maps). Allocates one tree arena, one
/// `Vec<&str>` for the sort, and one `String` per run. Uses the
/// same `BorderStyle` defaults as `scene_builder::build_scene` so
/// the two paths can't drift on style choices.
pub fn build_border_tree(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
) -> Tree<GfxElement, GfxMutator> {
    use crate::mindmap::border::BorderStyle;

    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    let vars = &map.canvas.theme_variables;
    let mut id_counter: usize = 1;

    let mut sorted_ids: Vec<&String> = map.nodes.keys().collect();
    sorted_ids.sort();

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

        let (ox, oy) = offsets.get(&node.id).copied().unwrap_or((0.0, 0.0));
        let pos_x = node.position.x as f32 + ox;
        let pos_y = node.position.y as f32 + oy;
        let size_x = node.size.width as f32;
        let size_y = node.size.height as f32;

        let frame_color_hex = color::resolve_var(&node.style.frame_color, vars);
        let border_style = BorderStyle::default_with_color(frame_color_hex);
        let color_rgba = color::hex_to_rgba_safe(&border_style.color, [1.0, 1.0, 1.0, 1.0]);

        append_border_sub_tree(
            &mut tree,
            &border_style,
            color_rgba,
            pos_x,
            pos_y,
            size_x,
            size_y,
            &mut id_counter,
        );
    }

    tree
}

/// Build one per-node sub-tree (Void parent + 4 GlyphArea runs) and
/// append it under `tree.root`. Kept as a private helper so
/// `build_border_tree` stays readable.
fn append_border_sub_tree(
    tree: &mut Tree<GfxElement, GfxMutator>,
    border_style: &crate::mindmap::border::BorderStyle,
    color_rgba: [f32; 4],
    pos_x: f32,
    pos_y: f32,
    size_x: f32,
    size_y: f32,
    id_counter: &mut usize,
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
    // mutation. The parent's channel is the counter so distinct
    // nodes never collide.
    let parent_channel = *id_counter;
    let parent_id = tree
        .arena
        .new_node(GfxElement::new_void_with_id(parent_channel, parent_channel));
    tree.root.append(parent_id, &mut tree.arena);
    *id_counter += 1;

    // Stable channels 1..=4 inside each border sub-tree. The
    // per-node Void parent already disambiguates across nodes.
    append_border_run(
        tree,
        parent_id,
        1,
        *id_counter,
        &top_text,
        font_size,
        (pos_x - approx_char_width, top_y),
        (h_width, font_size * 1.5),
        color_rgba,
    );
    *id_counter += 1;
    append_border_run(
        tree,
        parent_id,
        2,
        *id_counter,
        &bottom_text,
        font_size,
        (pos_x - approx_char_width, bottom_y),
        (h_width, font_size * 1.5),
        color_rgba,
    );
    *id_counter += 1;
    append_border_run(
        tree,
        parent_id,
        3,
        *id_counter,
        &left_text,
        font_size,
        (pos_x - approx_char_width, pos_y),
        (v_width, size_y),
        color_rgba,
    );
    *id_counter += 1;
    append_border_run(
        tree,
        parent_id,
        4,
        *id_counter,
        &right_text,
        font_size,
        (right_corner_x, pos_y),
        (v_width, size_y),
        color_rgba,
    );
    *id_counter += 1;
}

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

// =====================================================================
// Connection tree builder
//
// Converts a `&[ConnectionElement]` (the flat-scene representation
// `scene_builder::build_scene*` already produces) into a baumhard
// `Tree<GfxElement, GfxMutator>`. Each edge becomes one Void parent
// with one GlyphArea per glyph along its path (caps included).
//
// This deliberately does NOT recompute the geometry. The
// scene_builder still owns bezier sampling, theme variable
// resolution, selection / preview color routing, and the drag cache
// (`SceneConnectionCache`). The tree builder is a structural
// re-shape from the flat list to the tree form so the canvas-scene
// renderer can consume connections through the same
// `walk_tree_into_buffers` pipeline as borders and portals.
// =====================================================================

/// Channel band offsets for the connection sub-tree. `cap_start`
/// always sits at channel 1 (when present), the body glyphs occupy
/// channels `BODY_BASE..BODY_BASE+N`, and `cap_end` sits at
/// `CAP_END_CHANNEL` (chosen high enough that no body run can
/// overrun it). Bands give the in-place
/// [`build_connection_mutator_tree`] path stable channels even when
/// the body glyph count grows or shrinks frame-to-frame, and keep
/// the strict-ascending invariant that `align_child_walks` needs.
const CONN_CAP_START_CHANNEL: usize = 1;
const CONN_BODY_BASE_CHANNEL: usize = 100;
/// One million leaves headroom for `BODY_BASE + body_count` before
/// the cap-end channel — far more than any realistic edge can
/// sample.
const CONN_CAP_END_CHANNEL: usize = 1_000_001;

/// Identity slice for one `ConnectionElement` — captures the
/// structural shape (presence of caps, body glyph count) per edge.
/// Two slices match iff the structure is identical, which is the
/// precondition for the in-place mutator path: same shape ⇒ same
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
        let cluster_count = text.chars().count();
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

// =====================================================================
// Connection-label and edge-handle tree builders
//
// Both are flat — one GlyphArea per element directly under the
// tree root, no per-element Void wrapper, because nothing needs to
// target a single label or handle as a structural mutator group
// today (each element has only one renderable glyph). If grouping
// becomes necessary, wrap them like portals / borders.
//
// Both functions also produce auxiliary AABB maps so the renderer's
// existing hit-test paths (`hit_test_edge_label`,
// `hit_test_edge_handle`-equivalents) keep working until Session 5
// routes hit-testing through `Scene::component_at`.
// =====================================================================

/// Result of [`build_connection_label_tree`]. Pairs the tree with
/// the AABB-per-edge hitbox map the legacy
/// `Renderer::hit_test_edge_label` path needs.
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
    let cluster_count = elem.text.chars().count();
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

/// Stable channel for an [`EdgeHandleKind`]. The channel doubles as
/// the structural identity of one slot in the edge-handle tree —
/// `align_child_walks` matches mutator children against target
/// children by ascending channel, so using a `kind`-derived channel
/// keeps the in-place [`build_edge_handle_mutator_tree`] path
/// aligned across drag frames.
///
/// Bands are wide enough to add new handle kinds without
/// renumbering: anchors and midpoint live in 1..=3, control points
/// in `100..`. **Order matters** — values must be strictly ascending
/// in tree-insertion order (anchors first, then midpoint or
/// control-points).
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

fn append_border_run(
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
    // span. Grapheme cluster count matches `chars().count()` here
    // because box-drawing glyphs are all single-scalar ASCII-range
    // codepoints, but using the grapheme counter is cheap and
    // future-proof.
    let cluster_count = text.chars().count();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mindmap::loader;
    use std::path::PathBuf;

    fn test_map_path() -> PathBuf {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.pop(); // lib/baumhard -> lib
        path.pop(); // lib -> root
        path.push("maps/testament.mindmap.json");
        path
    }

    #[test]
    fn test_build_tree_structure() {
        let path = test_map_path();
        let map = loader::load_from_file(&path).unwrap();
        let result = build_mindmap_tree(&map);

        // Testament map has 243 nodes (none folded by default)
        assert_eq!(result.node_map.len(), 243);

        // Root of tree is Void, its children are the mindmap root nodes
        let root_children: Vec<_> = result.tree.root.children(&result.tree.arena).collect();
        let mindmap_roots = map.root_nodes();
        assert_eq!(root_children.len(), mindmap_roots.len());
    }

    #[test]
    fn test_tree_root_nodes_match_mindmap() {
        let path = test_map_path();
        let map = loader::load_from_file(&path).unwrap();
        let result = build_mindmap_tree(&map);

        let mindmap_roots = map.root_nodes();
        let tree_root_children: Vec<NodeId> =
            result.tree.root.children(&result.tree.arena).collect();

        // Each mindmap root should be in the node_map and a child of tree root
        for root in &mindmap_roots {
            let node_id = result.node_map.get(&root.id).expect("Root not in node_map");
            assert!(
                tree_root_children.contains(node_id),
                "Root {} not a child of tree root",
                root.id
            );
        }
    }

    #[test]
    fn test_glyph_area_properties() {
        let path = test_map_path();
        let map = loader::load_from_file(&path).unwrap();
        let result = build_mindmap_tree(&map);

        // Check "Lord God" node (id: 348068464)
        let lord_god = map.nodes.get("348068464").unwrap();
        let node_id = result.node_map.get("348068464").unwrap();
        let element = result.tree.arena.get(*node_id).unwrap().get();

        let area = element.glyph_area().expect("Expected GlyphArea");
        assert_eq!(area.text, "Lord God");
        assert_eq!(area.position.x.0, lord_god.position.x as f32);
        assert_eq!(area.position.y.0, lord_god.position.y as f32);
        assert_eq!(area.render_bounds.x.0, lord_god.size.width as f32);
        assert_eq!(area.render_bounds.y.0, lord_god.size.height as f32);
        assert_eq!(area.scale.0, lord_god.text_runs[0].size_pt as f32);
    }

    #[test]
    fn test_color_regions_from_text_runs() {
        let path = test_map_path();
        let map = loader::load_from_file(&path).unwrap();
        let result = build_mindmap_tree(&map);

        // Lord God has 1 text run with color #ffffff
        let node_id = result.node_map.get("348068464").unwrap();
        let element = result.tree.arena.get(*node_id).unwrap().get();
        let area = element.glyph_area().unwrap();

        assert_eq!(area.regions.num_regions(), 1);
        let region = area.regions.all_regions()[0];
        assert_eq!(region.range.start, 0);
        assert_eq!(region.range.end, 8);
        // White color: [1.0, 1.0, 1.0, 1.0]
        let c = region.color.unwrap();
        assert!((c[0] - 1.0).abs() < 0.01);
        assert!((c[1] - 1.0).abs() < 0.01);
        assert!((c[2] - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_parent_child_hierarchy_preserved() {
        let path = test_map_path();
        let map = loader::load_from_file(&path).unwrap();
        let result = build_mindmap_tree(&map);

        // Lord God's children in the mindmap should be children in the tree
        let lord_god_tree_id = result.node_map.get("348068464").unwrap();
        let mindmap_children = map.children_of("348068464");

        let tree_children: Vec<NodeId> = lord_god_tree_id
            .children(&result.tree.arena)
            .collect();
        assert_eq!(tree_children.len(), mindmap_children.len());

        for child in &mindmap_children {
            let child_tree_id = result.node_map.get(&child.id).expect("Child not in node_map");
            assert!(
                tree_children.contains(child_tree_id),
                "Child {} not a tree child of Lord God",
                child.id
            );
        }
    }

    #[test]
    fn test_unique_ids_are_unique() {
        let path = test_map_path();
        let map = loader::load_from_file(&path).unwrap();
        let result = build_mindmap_tree(&map);

        let mut seen_ids = std::collections::HashSet::new();
        for node_id in result.node_map.values() {
            let element = result.tree.arena.get(*node_id).unwrap().get();
            let uid = element.unique_id();
            assert!(seen_ids.insert(uid), "Duplicate unique_id: {}", uid);
        }
    }

    #[test]
    fn test_all_elements_are_glyph_areas() {
        let path = test_map_path();
        let map = loader::load_from_file(&path).unwrap();
        let result = build_mindmap_tree(&map);

        for node_id in result.node_map.values() {
            let element = result.tree.arena.get(*node_id).unwrap().get();
            assert!(
                element.glyph_area().is_some(),
                "Expected GlyphArea for node"
            );
        }
    }

    // -----------------------------------------------------------------
    // Scale / performance regression guards
    //
    // `build_mindmap_tree` runs on every mutation sync — any regression
    // from O(N) to O(N²) here would blow the drag budget on large maps
    // without being caught by the existing correctness tests (which load
    // the 243-node testament fixture).
    // -----------------------------------------------------------------

    use crate::mindmap::model::{
        Canvas, MindEdge, MindMap, MindNode, NodeLayout, NodeStyle, PortalPair, Position, Size,
    };
    use std::collections::HashMap;

    fn synthetic_node(id: &str, parent: Option<&str>, index: i32, x: f64, y: f64) -> MindNode {
        MindNode {
            id: id.to_string(),
            parent_id: parent.map(|s| s.to_string()),
            index,
            position: Position { x, y },
            size: Size { width: 80.0, height: 40.0 },
            text: id.to_string(),
            text_runs: vec![],
            style: NodeStyle {
                background_color: "#000".into(),
                frame_color: "#fff".into(),
                text_color: "#fff".into(),
                shape_type: 0,
                corner_radius_percent: 0.0,
                frame_thickness: 1.0,
                show_frame: true,
                show_shadow: false,
                border: None,
            },
            layout: NodeLayout { layout_type: 0, direction: 0, spacing: 0.0 },
            folded: false,
            notes: String::new(),
            color_schema: None,
            trigger_bindings: vec![],
            inline_mutations: vec![],
        }
    }

    fn synthetic_map(nodes_vec: Vec<MindNode>, edges: Vec<MindEdge>) -> MindMap {
        let mut nodes = HashMap::new();
        for n in nodes_vec {
            nodes.insert(n.id.clone(), n);
        }
        MindMap {
            version: "1.0".into(),
            name: "synthetic".into(),
            canvas: Canvas {
                background_color: "#000".into(),
                default_border: None,
                default_connection: None,
                theme_variables: HashMap::new(),
                theme_variants: HashMap::new(),
            },
            nodes,
            edges,
            custom_mutations: vec![],
            portals: vec![],
        }
    }

    /// Builds an N-node linear spine: `n0 -> n1 -> n2 -> ... -> n{N-1}`.
    /// Useful for depth-stress tests and O(N²) regression guards.
    fn mk_chain_map(n: usize) -> MindMap {
        assert!(n >= 1);
        let mut nodes = Vec::with_capacity(n);
        nodes.push(synthetic_node("c0", None, 0, 0.0, 0.0));
        for i in 1..n {
            let parent = format!("c{}", i - 1);
            let id = format!("c{}", i);
            nodes.push(synthetic_node(&id, Some(&parent), 0, 0.0, i as f64 * 50.0));
        }
        synthetic_map(nodes, vec![])
    }

    /// Builds a star: one root and `n - 1` sibling children.
    fn mk_star_map(n: usize) -> MindMap {
        assert!(n >= 1);
        let mut nodes = Vec::with_capacity(n);
        nodes.push(synthetic_node("root", None, 0, 0.0, 0.0));
        for i in 1..n {
            let id = format!("s{}", i);
            nodes.push(synthetic_node(
                &id,
                Some("root"),
                (i - 1) as i32,
                (i as f64) * 100.0,
                100.0,
            ));
        }
        synthetic_map(nodes, vec![])
    }

    /// Build a 1000-node chain and assert the resulting `node_map` size
    /// equals the input count. If a regression made the builder O(N²) it
    /// would not change this assertion — but the synthetic large-map
    /// scaffold becomes the natural place to plug a wall-clock bench if
    /// needed later, and this test proves the builder is linearly
    /// functional at scale. Also verifies correctness at size.
    #[test]
    fn test_build_tree_scale_1000_node_chain() {
        let map = mk_chain_map(1000);
        let result = build_mindmap_tree(&map);
        assert_eq!(result.node_map.len(), 1000);
        // The spine root is the only root, so the tree's root has one
        // child (the Void -> first chain node).
        let roots: Vec<_> = result.tree.root.children(&result.tree.arena).collect();
        assert_eq!(roots.len(), 1);
        // Every chain node is reachable via the node_map.
        for i in 0..1000 {
            let id = format!("c{}", i);
            assert!(result.node_map.contains_key(&id),
                "missing node {}", id);
        }
    }

    /// A 500-child star fans out from a single root. Guards the
    /// wide-breadth case — a regression that used Vec::insert(0, ...)
    /// or otherwise grew quadratically in the child list would still
    /// produce a correct node_map, but this test's companion 1000-node
    /// chain test plus this one together cover both topology extremes.
    #[test]
    fn test_build_tree_wide_fan_out_500() {
        let map = mk_star_map(500);
        let result = build_mindmap_tree(&map);
        assert_eq!(result.node_map.len(), 500);
        // Root is "root", all others are direct children.
        let root_tree_id = result.node_map.get("root").unwrap();
        let children: Vec<_> = root_tree_id.children(&result.tree.arena).collect();
        assert_eq!(children.len(), 499);
    }

    /// A 500-node deep spine must build without a stack overflow. The
    /// current `build_mindmap_tree` walks iteratively — this test
    /// guards against a future refactor silently introducing recursion
    /// over the hierarchy.
    #[test]
    fn test_build_tree_deep_chain_no_stack_overflow() {
        let map = mk_chain_map(500);
        let result = build_mindmap_tree(&map);
        assert_eq!(result.node_map.len(), 500);
        // Walk from the root down the spine and confirm depth == 500.
        let mut current = *result.node_map.get("c0").unwrap();
        let mut depth = 1;
        while let Some(child) = current.children(&result.tree.arena).next() {
            current = child;
            depth += 1;
        }
        assert_eq!(depth, 500);
    }

    // -----------------------------------------------------------------
    // Background color → GlyphArea.background_color plumbing
    //
    // Session 6C follow-up: node backgrounds now live on the Baumhard
    // tree (as `GlyphArea.background_color`) so they can be mutated
    // through the tree walker and efficiently rendered as filled
    // rectangles by the renderer. These tests lock in that
    // `NodeStyle.background_color` survives the tree build intact,
    // honors the theme-variable indirection, and degrades safely on
    // malformed input or the explicit `transparent` sentinel.
    // -----------------------------------------------------------------

    fn glyph_area_of<'a>(
        tree: &'a crate::gfx_structs::tree::Tree<
            crate::gfx_structs::element::GfxElement,
            crate::gfx_structs::mutator::GfxMutator,
        >,
        node_id: indextree::NodeId,
    ) -> &'a crate::gfx_structs::area::GlyphArea {
        tree.arena.get(node_id).unwrap().get().glyph_area().unwrap()
    }

    #[test]
    fn test_background_color_opaque_hex_populates_field() {
        let mut map = synthetic_map(
            vec![synthetic_node("n", None, 0, 0.0, 0.0)],
            vec![],
        );
        map.nodes.get_mut("n").unwrap().style.background_color = "#ff8800".into();
        let result = build_mindmap_tree(&map);
        let area = glyph_area_of(&result.tree, *result.node_map.get("n").unwrap());
        assert_eq!(area.background_color, Some([255, 136, 0, 255]));
    }

    #[test]
    fn test_background_color_empty_string_becomes_none() {
        let mut map = synthetic_map(
            vec![synthetic_node("n", None, 0, 0.0, 0.0)],
            vec![],
        );
        map.nodes.get_mut("n").unwrap().style.background_color = "".into();
        let result = build_mindmap_tree(&map);
        let area = glyph_area_of(&result.tree, *result.node_map.get("n").unwrap());
        assert!(area.background_color.is_none());
    }

    #[test]
    fn test_background_color_fully_transparent_becomes_none() {
        let mut map = synthetic_map(
            vec![synthetic_node("n", None, 0, 0.0, 0.0)],
            vec![],
        );
        // `#00000000` is the conventional "no fill" opt-out.
        map.nodes.get_mut("n").unwrap().style.background_color = "#00000000".into();
        let result = build_mindmap_tree(&map);
        let area = glyph_area_of(&result.tree, *result.node_map.get("n").unwrap());
        assert!(area.background_color.is_none());
    }

    #[test]
    fn test_background_color_resolves_theme_variable() {
        let mut map = synthetic_map(
            vec![synthetic_node("n", None, 0, 0.0, 0.0)],
            vec![],
        );
        map.canvas
            .theme_variables
            .insert("--panel".into(), "#112233".into());
        map.nodes.get_mut("n").unwrap().style.background_color = "var(--panel)".into();
        let result = build_mindmap_tree(&map);
        let area = glyph_area_of(&result.tree, *result.node_map.get("n").unwrap());
        assert_eq!(area.background_color, Some([17, 34, 51, 255]));
    }

    #[test]
    fn test_background_color_malformed_hex_degrades_to_none() {
        let mut map = synthetic_map(
            vec![synthetic_node("n", None, 0, 0.0, 0.0)],
            vec![],
        );
        // `hex_to_rgba_safe` degrades unknown/bad strings to the
        // fallback we passed in — `[0,0,0,0]` for background — which
        // then trips the transparent-alpha sentinel below and becomes
        // `None`. Keeps a typo from crashing the render.
        map.nodes.get_mut("n").unwrap().style.background_color = "not-a-color".into();
        let result = build_mindmap_tree(&map);
        let area = glyph_area_of(&result.tree, *result.node_map.get("n").unwrap());
        assert!(area.background_color.is_none());
    }

    #[test]
    fn test_background_color_three_digit_hex_works() {
        let mut map = synthetic_map(
            vec![synthetic_node("n", None, 0, 0.0, 0.0)],
            vec![],
        );
        // `#000` is the default in all the synthetic nodes above, and
        // it's opaque black — verify the builder treats it as a real
        // fill (not transparent) so the renderer draws the rect. A
        // future refactor that mis-parses short hex values would
        // regress this.
        map.nodes.get_mut("n").unwrap().style.background_color = "#000".into();
        let result = build_mindmap_tree(&map);
        let area = glyph_area_of(&result.tree, *result.node_map.get("n").unwrap());
        assert_eq!(area.background_color, Some([0, 0, 0, 255]));
    }

    // -----------------------------------------------------------------
    // Border tree builder
    // -----------------------------------------------------------------

    #[test]
    fn border_tree_has_one_void_parent_per_framed_node() {
        let map = synthetic_map(
            vec![
                synthetic_node("a", None, 0, 0.0, 0.0),
                synthetic_node("b", None, 1, 200.0, 0.0),
            ],
            vec![],
        );
        let tree = build_border_tree(&map, &HashMap::new());
        // Two framed nodes → two per-node Void parents under root.
        let parents: Vec<NodeId> = tree.root.children(&tree.arena).collect();
        assert_eq!(parents.len(), 2);
        for parent in parents {
            let element = tree.arena.get(parent).unwrap().get();
            assert!(element.glyph_area().is_none(), "per-node parent is Void");
            // Every parent has exactly 4 GlyphArea run children.
            let runs: Vec<NodeId> = parent.children(&tree.arena).collect();
            assert_eq!(runs.len(), 4);
            for run_id in runs {
                let run = tree.arena.get(run_id).unwrap().get();
                assert!(run.glyph_area().is_some(), "run is a GlyphArea");
            }
        }
    }

    #[test]
    fn border_tree_skips_nodes_with_show_frame_false() {
        let mut map = synthetic_map(
            vec![
                synthetic_node("a", None, 0, 0.0, 0.0),
                synthetic_node("b", None, 1, 200.0, 0.0),
            ],
            vec![],
        );
        map.nodes.get_mut("a").unwrap().style.show_frame = false;
        let tree = build_border_tree(&map, &HashMap::new());
        // Only `b` is framed → one per-node parent.
        let parents: Vec<NodeId> = tree.root.children(&tree.arena).collect();
        assert_eq!(parents.len(), 1);
    }

    #[test]
    fn border_tree_skips_folded_nodes() {
        let mut map = synthetic_map(
            vec![
                synthetic_node("parent", None, 0, 0.0, 0.0),
                synthetic_node("child", Some("parent"), 0, 0.0, 100.0),
            ],
            vec![],
        );
        map.nodes.get_mut("parent").unwrap().folded = true;
        let tree = build_border_tree(&map, &HashMap::new());
        // Parent itself still frames; child is hidden.
        let parents: Vec<NodeId> = tree.root.children(&tree.arena).collect();
        assert_eq!(parents.len(), 1);
    }

    #[test]
    fn border_tree_applies_drag_offset() {
        let map = synthetic_map(vec![synthetic_node("a", None, 0, 0.0, 0.0)], vec![]);
        let mut offsets: HashMap<String, (f32, f32)> = HashMap::new();
        offsets.insert("a".into(), (50.0, 25.0));
        let tree = build_border_tree(&map, &offsets);
        // Drag offset must show up on the *top* run's position.x
        // (which is `pos_x - approx_char_width`).
        let parent = tree.root.children(&tree.arena).next().unwrap();
        let top_run = parent.children(&tree.arena).next().unwrap();
        let area = tree
            .arena
            .get(top_run)
            .unwrap()
            .get()
            .glyph_area()
            .unwrap();
        // pos_x + offset = 0 + 50 = 50, then shifted by
        // -approx_char_width (0.6 * font_size).
        let font_size = 14.0_f32;
        let approx_char_width = font_size * BORDER_APPROX_CHAR_WIDTH_FRAC;
        let expected_x = 50.0 - approx_char_width;
        assert!(
            (area.position.x.0 - expected_x).abs() < 0.001,
            "top-run x ({}) should match drag-applied layout ({})",
            area.position.x.0,
            expected_x
        );
        // y follows pos_y + offset - font_size + corner_overlap.
        let corner_overlap = font_size * BORDER_CORNER_OVERLAP_FRAC;
        let expected_y = 25.0 - font_size + corner_overlap;
        assert!((area.position.y.0 - expected_y).abs() < 0.001);
    }

    #[test]
    fn border_tree_resolves_frame_color_through_theme_vars() {
        let mut map = synthetic_map(vec![synthetic_node("a", None, 0, 0.0, 0.0)], vec![]);
        // Theme variable keys include the leading `--`, matching
        // the CSS-ish `var(--name)` syntax used in mindmap JSON.
        map.canvas
            .theme_variables
            .insert("--my-frame".into(), "#ff0000".into());
        map.nodes.get_mut("a").unwrap().style.frame_color = "var(--my-frame)".into();
        let tree = build_border_tree(&map, &HashMap::new());
        let parent = tree.root.children(&tree.arena).next().unwrap();
        let top_run = parent.children(&tree.arena).next().unwrap();
        let area = tree
            .arena
            .get(top_run)
            .unwrap()
            .get()
            .glyph_area()
            .unwrap();
        let region = area.regions.all_regions()[0];
        let c = region.color.unwrap();
        // #ff0000 → red channel 1.0, green/blue 0.0.
        assert!((c[0] - 1.0).abs() < 0.01);
        assert!(c[1] < 0.01);
        assert!(c[2] < 0.01);
    }

    #[test]
    fn border_tree_run_channels_are_stable_1_to_4() {
        // Top=1, Bottom=2, Left=3, Right=4. Stability matters
        // because mutator trees target runs by channel.
        use crate::gfx_structs::tree::BranchChannel;
        let map = synthetic_map(vec![synthetic_node("a", None, 0, 0.0, 0.0)], vec![]);
        let tree = build_border_tree(&map, &HashMap::new());
        let parent = tree.root.children(&tree.arena).next().unwrap();
        let runs: Vec<_> = parent.children(&tree.arena).collect();
        let channels: Vec<usize> = runs
            .iter()
            .map(|id| tree.arena.get(*id).unwrap().get().channel())
            .collect();
        assert_eq!(channels, vec![1, 2, 3, 4]);
    }

    // -----------------------------------------------------------------
    // Portal tree builder
    // -----------------------------------------------------------------

    fn synthetic_portal(label: &str, a: &str, b: &str, color: &str) -> PortalPair {
        PortalPair {
            endpoint_a: a.into(),
            endpoint_b: b.into(),
            label: label.into(),
            glyph: "◈".into(),
            color: color.into(),
            font_size_pt: 16.0,
            font: None,
        }
    }

    #[test]
    fn portal_tree_emits_two_markers_per_pair() {
        let mut map = synthetic_map(
            vec![
                synthetic_node("a", None, 0, 0.0, 0.0),
                synthetic_node("b", None, 1, 200.0, 0.0),
            ],
            vec![],
        );
        map.portals.push(synthetic_portal("X", "a", "b", "#ff0000"));

        let result = build_portal_tree(&map, &HashMap::new(), None, None);
        let pairs: Vec<NodeId> = result.tree.root.children(&result.tree.arena).collect();
        assert_eq!(pairs.len(), 1);

        let markers: Vec<NodeId> = pairs[0].children(&result.tree.arena).collect();
        assert_eq!(markers.len(), 2);
        // Hitboxes: one entry per (pair, endpoint).
        assert_eq!(result.hitboxes.len(), 2);
    }

    #[test]
    fn portal_tree_skips_pair_with_folded_endpoint() {
        let mut map = synthetic_map(
            vec![
                synthetic_node("parent", None, 0, 0.0, 0.0),
                synthetic_node("child", Some("parent"), 0, 0.0, 100.0),
                synthetic_node("other", None, 1, 200.0, 0.0),
            ],
            vec![],
        );
        map.nodes.get_mut("parent").unwrap().folded = true;
        // Pair endpoints: hidden child + visible other. Should be
        // skipped wholesale because is_hidden_by_fold(child) is true.
        map.portals
            .push(synthetic_portal("Y", "child", "other", "#00ff00"));
        let result = build_portal_tree(&map, &HashMap::new(), None, None);
        assert_eq!(result.tree.root.children(&result.tree.arena).count(), 0);
        assert!(result.hitboxes.is_empty());
    }

    #[test]
    fn connection_tree_emits_one_void_per_edge_with_glyph_children() {
        use crate::mindmap::scene_builder::ConnectionElement;
        use crate::mindmap::scene_cache::EdgeKey;

        let elem = ConnectionElement {
            edge_key: EdgeKey::new("a", "b", "child"),
            glyph_positions: vec![(10.0, 0.0), (20.0, 0.0), (30.0, 0.0)],
            body_glyph: "·".into(),
            cap_start: Some(("◀".into(), (0.0, 0.0))),
            cap_end: Some(("▶".into(), (40.0, 0.0))),
            font: None,
            font_size_pt: 12.0,
            color: "#ff0000".into(),
        };
        let tree = build_connection_tree(&[elem]);
        let edge_parents: Vec<NodeId> = tree.root.children(&tree.arena).collect();
        assert_eq!(edge_parents.len(), 1);
        let glyphs: Vec<NodeId> = edge_parents[0].children(&tree.arena).collect();
        // 1 cap-start + 3 body + 1 cap-end = 5
        assert_eq!(glyphs.len(), 5);
        for id in &glyphs {
            assert!(tree.arena.get(*id).unwrap().get().glyph_area().is_some());
        }
    }

    #[test]
    fn connection_tree_skips_caps_when_absent() {
        use crate::mindmap::scene_builder::ConnectionElement;
        use crate::mindmap::scene_cache::EdgeKey;

        let elem = ConnectionElement {
            edge_key: EdgeKey::new("a", "b", "child"),
            glyph_positions: vec![(0.0, 0.0)],
            body_glyph: "·".into(),
            cap_start: None,
            cap_end: None,
            font: None,
            font_size_pt: 12.0,
            color: "#ffffff".into(),
        };
        let tree = build_connection_tree(&[elem]);
        let edge_parent = tree.root.children(&tree.arena).next().unwrap();
        assert_eq!(edge_parent.children(&tree.arena).count(), 1);
    }

    #[test]
    fn portal_tree_selection_overrides_color() {
        let mut map = synthetic_map(
            vec![
                synthetic_node("a", None, 0, 0.0, 0.0),
                synthetic_node("b", None, 1, 200.0, 0.0),
            ],
            vec![],
        );
        map.portals.push(synthetic_portal("Z", "a", "b", "#ff0000"));

        let selected = Some(("Z", "a", "b"));
        let result = build_portal_tree(&map, &HashMap::new(), selected, None);

        // Each marker's GlyphArea should carry the cyan color, not red.
        let pair = result.tree.root.children(&result.tree.arena).next().unwrap();
        for marker in pair.children(&result.tree.arena) {
            let area = result
                .tree
                .arena
                .get(marker)
                .unwrap()
                .get()
                .glyph_area()
                .unwrap();
            let region = area.regions.all_regions()[0];
            let c = region.color.unwrap();
            // #00E5FF: r=0, g≈229/255, b≈1.0
            assert!(c[0] < 0.05);
            assert!((c[1] - 229.0 / 255.0).abs() < 0.02);
            assert!((c[2] - 1.0).abs() < 0.02);
        }
    }

    /// `portal_pair_data` is the single source of truth for both
    /// [`build_portal_tree`] and [`build_portal_mutator_tree`]; the
    /// mutator path needs the resulting `pair_channel` set to be
    /// strictly ascending (Baumhard's `align_child_walks` pairs
    /// mutator children against target children by ascending
    /// channel and breaks alignment if the order is violated).
    #[test]
    fn portal_pair_channels_are_strictly_ascending() {
        let mut map = synthetic_map(
            vec![
                synthetic_node("a", None, 0, 0.0, 0.0),
                synthetic_node("b", None, 1, 200.0, 0.0),
                synthetic_node("c", None, 2, 400.0, 0.0),
            ],
            vec![],
        );
        map.portals.push(synthetic_portal("X", "a", "b", "#ff0000"));
        map.portals.push(synthetic_portal("Y", "b", "c", "#00ff00"));

        let pairs = portal_pair_data(&map, &HashMap::new(), None, None);
        assert_eq!(pairs.len(), 2);
        let channels: Vec<usize> = pairs.iter().map(|p| p.pair_channel).collect();
        let mut prev = 0;
        for c in &channels {
            assert!(*c > prev, "pair channels must be strictly ascending: {channels:?}");
            prev = *c;
        }
    }

    /// Round-trip: building a tree at state A and then applying the
    /// mutator computed from state B must produce a tree whose
    /// per-channel GlyphAreas match what `build_portal_tree(B)`
    /// would produce directly. Pins the canonical §B2
    /// "mutation, not rebuild" promise — the in-place path's
    /// observable output is identical to a full rebuild's, modulo
    /// the arena identity.
    #[test]
    fn portal_mutator_round_trip_matches_full_rebuild() {
        use crate::core::primitives::Applicable;
        let mut map = synthetic_map(
            vec![
                synthetic_node("a", None, 0, 0.0, 0.0),
                synthetic_node("b", None, 1, 200.0, 0.0),
            ],
            vec![],
        );
        map.portals.push(synthetic_portal("X", "a", "b", "#ff0000"));

        // State A: no offsets, no selection.
        let mut tree_a = build_portal_tree(&map, &HashMap::new(), None, None).tree;

        // State B: drag offset on `b`, plus selection.
        let mut offsets = HashMap::new();
        offsets.insert("b".to_string(), (10.0, -5.0));
        let selected = Some(("X", "a", "b"));

        let mutator = build_portal_mutator_tree(&map, &offsets, selected, None);
        mutator.mutator.apply_to(&mut tree_a);

        let expected = build_portal_tree(&map, &offsets, selected, None).tree;

        // Walk both: per pair, per slot, GlyphArea fields (text,
        // position, bounds, scale, line_height, regions, outline)
        // must match.
        let actual_pairs: Vec<NodeId> = tree_a.root.children(&tree_a.arena).collect();
        let expected_pairs: Vec<NodeId> = expected.root.children(&expected.arena).collect();
        assert_eq!(actual_pairs.len(), expected_pairs.len());
        for (a_pair, e_pair) in actual_pairs.iter().zip(expected_pairs.iter()) {
            let a_markers: Vec<NodeId> = a_pair.children(&tree_a.arena).collect();
            let e_markers: Vec<NodeId> = e_pair.children(&expected.arena).collect();
            assert_eq!(a_markers.len(), e_markers.len());
            for (a_m, e_m) in a_markers.iter().zip(e_markers.iter()) {
                let a_area = tree_a.arena.get(*a_m).unwrap().get().glyph_area().unwrap();
                let e_area = expected.arena.get(*e_m).unwrap().get().glyph_area().unwrap();
                assert_eq!(a_area.text, e_area.text);
                assert_eq!(a_area.position, e_area.position);
                assert_eq!(a_area.render_bounds, e_area.render_bounds);
                assert_eq!(a_area.scale, e_area.scale);
                assert_eq!(a_area.line_height, e_area.line_height);
                assert_eq!(a_area.regions, e_area.regions);
                assert_eq!(a_area.outline, e_area.outline);
            }
        }
    }

    /// Connection identity sequence captures cap presence and body
    /// glyph count per edge. A change in any of those is structural
    /// and must drop the equality so the dispatcher in
    /// `update_connection_tree` falls back to a full rebuild.
    #[test]
    fn connection_identity_sequence_changes_with_structural_shifts() {
        use crate::mindmap::scene_builder::ConnectionElement;
        use crate::mindmap::scene_cache::EdgeKey;

        let mk = |body_count: usize,
                  cap_start: Option<(String, (f32, f32))>,
                  cap_end: Option<(String, (f32, f32))>,
                  color: &str| ConnectionElement {
            edge_key: EdgeKey::new("a", "b", "child"),
            glyph_positions: (0..body_count).map(|i| (i as f32 * 10.0, 0.0)).collect(),
            body_glyph: "·".into(),
            cap_start,
            cap_end,
            font: None,
            font_size_pt: 12.0,
            color: color.into(),
        };

        let cap_start = Some(("◀".to_string(), (0.0, 0.0)));
        let cap_end = Some(("▶".to_string(), (30.0, 0.0)));
        let base = mk(2, cap_start.clone(), cap_end.clone(), "#ff0000");
        let id_base = connection_identity_sequence(std::slice::from_ref(&base));

        // Body count change (drag-shrinks-path): structural shift.
        let shorter = mk(1, cap_start.clone(), cap_end.clone(), "#ff0000");
        assert_ne!(
            id_base,
            connection_identity_sequence(std::slice::from_ref(&shorter))
        );

        // Cap removal: structural shift.
        let no_cap = mk(2, None, cap_end.clone(), "#ff0000");
        assert_ne!(
            id_base,
            connection_identity_sequence(std::slice::from_ref(&no_cap))
        );

        // Color change at fixed structure: identity preserved (the
        // mutator path is sound for color-only updates like
        // selection toggle and color preview).
        let recolored = mk(2, cap_start, cap_end, "#00E5FF");
        assert_eq!(
            id_base,
            connection_identity_sequence(std::slice::from_ref(&recolored))
        );
    }

    /// Round-trip: `build_connection_tree(A)` + the mutator from B
    /// reads identical to a fresh `build_connection_tree(B)` when A
    /// and B share an identity sequence (typical for selection /
    /// color preview / theme switches that do not move endpoints).
    #[test]
    fn connection_mutator_round_trip_matches_full_rebuild() {
        use crate::core::primitives::Applicable;
        use crate::mindmap::scene_builder::ConnectionElement;
        use crate::mindmap::scene_cache::EdgeKey;

        let mk = |color: &str| ConnectionElement {
            edge_key: EdgeKey::new("a", "b", "child"),
            glyph_positions: vec![(10.0, 0.0), (20.0, 0.0)],
            body_glyph: "·".into(),
            cap_start: Some(("◀".into(), (0.0, 0.0))),
            cap_end: Some(("▶".into(), (30.0, 0.0))),
            font: None,
            font_size_pt: 12.0,
            color: color.into(),
        };
        let elem_a = mk("#ff0000");
        let elem_b = mk("#00E5FF");

        let mut tree_a = build_connection_tree(std::slice::from_ref(&elem_a));
        let mutator = build_connection_mutator_tree(std::slice::from_ref(&elem_b));
        mutator.apply_to(&mut tree_a);

        let expected = build_connection_tree(std::slice::from_ref(&elem_b));

        let actual_edges: Vec<NodeId> = tree_a.root.children(&tree_a.arena).collect();
        let expected_edges: Vec<NodeId> = expected.root.children(&expected.arena).collect();
        assert_eq!(actual_edges.len(), expected_edges.len());
        for (a_e, e_e) in actual_edges.iter().zip(expected_edges.iter()) {
            let a_glyphs: Vec<NodeId> = a_e.children(&tree_a.arena).collect();
            let e_glyphs: Vec<NodeId> = e_e.children(&expected.arena).collect();
            assert_eq!(a_glyphs.len(), e_glyphs.len());
            // Full-field parity — every mutator-written field
            // must match what a fresh build produces. Missing one
            // would let silent drift accumulate on that field
            // across mutator updates.
            for (a, e) in a_glyphs.iter().zip(e_glyphs.iter()) {
                let a_area = tree_a.arena.get(*a).unwrap().get().glyph_area().unwrap();
                let e_area = expected.arena.get(*e).unwrap().get().glyph_area().unwrap();
                assert_eq!(a_area.text, e_area.text);
                assert_eq!(a_area.position, e_area.position);
                assert_eq!(a_area.render_bounds, e_area.render_bounds);
                assert_eq!(a_area.scale, e_area.scale);
                assert_eq!(a_area.line_height, e_area.line_height);
                assert_eq!(a_area.regions, e_area.regions);
                assert_eq!(a_area.outline, e_area.outline);
            }
        }
    }

    /// Connection-label round-trip with a label-text edit (the
    /// hot path for inline label editing in Phase 2.1): identity
    /// is the per-edge `EdgeKey` sequence, so changing the text
    /// alone keeps the identity stable and the in-place mutator
    /// path runs.
    #[test]
    fn connection_label_mutator_round_trip_handles_text_edit() {
        use crate::core::primitives::Applicable;
        use crate::mindmap::scene_builder::ConnectionLabelElement;
        use crate::mindmap::scene_cache::EdgeKey;

        let mk = |text: &str| ConnectionLabelElement {
            edge_key: EdgeKey::new("a", "b", "child"),
            text: text.into(),
            position: (10.0, 10.0),
            bounds: (40.0, 16.0),
            color: "#ffffff".into(),
            font: None,
            font_size_pt: 12.0,
        };
        let elem_a = mk("old");
        let elem_b = mk("new label");
        assert_eq!(
            connection_label_identity_sequence(std::slice::from_ref(&elem_a)),
            connection_label_identity_sequence(std::slice::from_ref(&elem_b))
        );

        let mut tree_a = build_connection_label_tree(std::slice::from_ref(&elem_a)).tree;
        let mutator = build_connection_label_mutator_tree(std::slice::from_ref(&elem_b));
        mutator.mutator.apply_to(&mut tree_a);

        let expected = build_connection_label_tree(std::slice::from_ref(&elem_b)).tree;
        let actual_leaves: Vec<NodeId> = tree_a.root.children(&tree_a.arena).collect();
        let expected_leaves: Vec<NodeId> = expected.root.children(&expected.arena).collect();
        assert_eq!(actual_leaves.len(), expected_leaves.len());
        // Full-field parity — see `connection_mutator_round_trip...`
        // for the rationale.
        for (a, e) in actual_leaves.iter().zip(expected_leaves.iter()) {
            let a_area = tree_a.arena.get(*a).unwrap().get().glyph_area().unwrap();
            let e_area = expected.arena.get(*e).unwrap().get().glyph_area().unwrap();
            assert_eq!(a_area.text, "new label");
            assert_eq!(a_area.text, e_area.text);
            assert_eq!(a_area.position, e_area.position);
            assert_eq!(a_area.render_bounds, e_area.render_bounds);
            assert_eq!(a_area.scale, e_area.scale);
            assert_eq!(a_area.line_height, e_area.line_height);
            assert_eq!(a_area.regions, e_area.regions);
            assert_eq!(a_area.outline, e_area.outline);
        }
    }

    /// `edge_handle_channel_for` keeps the AnchorFrom < AnchorTo <
    /// (Midpoint | ControlPoint) ordering that
    /// `align_child_walks` relies on. Channels also need to be
    /// distinct between Midpoint and any ControlPoint so a switch
    /// between a straight edge and a curved one shows up as a
    /// structural change in the identity sequence.
    #[test]
    fn edge_handle_channels_preserve_ordering_and_distinctness() {
        use crate::mindmap::scene_builder::EdgeHandleKind;
        let from = edge_handle_channel_for(EdgeHandleKind::AnchorFrom);
        let to = edge_handle_channel_for(EdgeHandleKind::AnchorTo);
        let mid = edge_handle_channel_for(EdgeHandleKind::Midpoint);
        let cp0 = edge_handle_channel_for(EdgeHandleKind::ControlPoint(0));
        let cp1 = edge_handle_channel_for(EdgeHandleKind::ControlPoint(1));
        assert!(from < to, "AnchorFrom < AnchorTo");
        assert!(to < mid, "AnchorTo < Midpoint");
        assert!(to < cp0, "AnchorTo < ControlPoint(0)");
        assert!(cp0 < cp1, "ControlPoint(0) < ControlPoint(1)");
        assert_ne!(mid, cp0, "Midpoint and ControlPoint(0) must occupy different channels");
    }

    /// Round-trip: a tree built from handle set A, with the mutator
    /// computed from handle set B applied, reads identical to a
    /// fresh `build_edge_handle_tree(B)` — provided B has the same
    /// identity sequence as A (same kind ordering). Pins the §B2
    /// "mutation, not rebuild" promise for the drag hot path: only
    /// positions move during a handle drag, so identity stays
    /// stable and the mutator path is sound.
    #[test]
    fn edge_handle_mutator_round_trip_matches_full_rebuild() {
        use crate::core::primitives::Applicable;
        use crate::mindmap::scene_builder::{EdgeHandleElement, EdgeHandleKind};
        use crate::mindmap::scene_cache::EdgeKey;

        let mk = |kind: EdgeHandleKind, x: f32, y: f32| EdgeHandleElement {
            edge_key: EdgeKey::new("a", "b", "child"),
            kind,
            position: (x, y),
            glyph: "◆".into(),
            color: "#00E5FF".into(),
            font_size_pt: 14.0,
        };

        let set_a = vec![
            mk(EdgeHandleKind::AnchorFrom, 0.0, 0.0),
            mk(EdgeHandleKind::AnchorTo, 100.0, 0.0),
            mk(EdgeHandleKind::Midpoint, 50.0, 0.0),
        ];
        let set_b = vec![
            mk(EdgeHandleKind::AnchorFrom, 5.0, -2.0),
            mk(EdgeHandleKind::AnchorTo, 110.0, -2.0),
            mk(EdgeHandleKind::Midpoint, 57.0, -2.0),
        ];
        assert_eq!(
            edge_handle_identity_sequence(&set_a),
            edge_handle_identity_sequence(&set_b),
            "drag preserves identity sequence; only positions move"
        );

        let mut tree_a = build_edge_handle_tree(&set_a);
        let mutator = build_edge_handle_mutator_tree(&set_b);
        mutator.apply_to(&mut tree_a);

        let expected = build_edge_handle_tree(&set_b);
        let actual_leaves: Vec<NodeId> =
            tree_a.root.children(&tree_a.arena).collect();
        let expected_leaves: Vec<NodeId> =
            expected.root.children(&expected.arena).collect();
        assert_eq!(actual_leaves.len(), expected_leaves.len());
        for (a, e) in actual_leaves.iter().zip(expected_leaves.iter()) {
            let a_area = tree_a.arena.get(*a).unwrap().get().glyph_area().unwrap();
            let e_area = expected.arena.get(*e).unwrap().get().glyph_area().unwrap();
            assert_eq!(a_area.text, e_area.text);
            assert_eq!(a_area.position, e_area.position);
            assert_eq!(a_area.render_bounds, e_area.render_bounds);
            assert_eq!(a_area.regions, e_area.regions);
        }
    }

    /// Adding a control point (drag-midpoint-creates-cp) or
    /// switching selection from a 0-CP edge to a 1-CP edge must
    /// register as a structural change in the identity sequence,
    /// so the dispatcher in `update_edge_handle_tree` falls back to
    /// a full rebuild rather than apply a mutator against a tree
    /// whose channel set has shifted.
    #[test]
    fn edge_handle_identity_sequence_changes_on_midpoint_to_cp() {
        use crate::mindmap::scene_builder::{EdgeHandleElement, EdgeHandleKind};
        use crate::mindmap::scene_cache::EdgeKey;

        let mk = |kind: EdgeHandleKind| EdgeHandleElement {
            edge_key: EdgeKey::new("a", "b", "child"),
            kind,
            position: (0.0, 0.0),
            glyph: "◆".into(),
            color: "#00E5FF".into(),
            font_size_pt: 14.0,
        };
        let straight = vec![
            mk(EdgeHandleKind::AnchorFrom),
            mk(EdgeHandleKind::AnchorTo),
            mk(EdgeHandleKind::Midpoint),
        ];
        let curved = vec![
            mk(EdgeHandleKind::AnchorFrom),
            mk(EdgeHandleKind::AnchorTo),
            mk(EdgeHandleKind::ControlPoint(0)),
        ];
        assert_ne!(
            edge_handle_identity_sequence(&straight),
            edge_handle_identity_sequence(&curved)
        );
    }

    /// `portal_identity_sequence` reflects the visible-portal order
    /// emitted by `portal_pair_data`. Folded endpoints drop their
    /// pair from the sequence — the in-place mutator path uses this
    /// to detect when a fold/unfold has changed the structure and
    /// trigger a full rebuild instead.
    #[test]
    fn portal_identity_sequence_drops_folded_pairs() {
        let mut map = synthetic_map(
            vec![
                synthetic_node("a", None, 0, 0.0, 0.0),
                synthetic_node("b", None, 1, 200.0, 0.0),
                synthetic_node("parent", None, 2, 400.0, 0.0),
                synthetic_node("child", Some("parent"), 0, 0.0, 100.0),
            ],
            vec![],
        );
        map.portals.push(synthetic_portal("X", "a", "b", "#ff0000"));
        map.portals
            .push(synthetic_portal("Y", "b", "child", "#00ff00"));

        let pairs_before = portal_pair_data(&map, &HashMap::new(), None, None);
        assert_eq!(
            portal_identity_sequence(&pairs_before),
            vec![
                ("X".into(), "a".into(), "b".into()),
                ("Y".into(), "b".into(), "child".into()),
            ]
        );

        map.nodes.get_mut("parent").unwrap().folded = true;
        let pairs_after = portal_pair_data(&map, &HashMap::new(), None, None);
        assert_eq!(
            portal_identity_sequence(&pairs_after),
            vec![("X".into(), "a".into(), "b".into())]
        );
    }
}
