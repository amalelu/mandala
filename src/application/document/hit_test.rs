//! Hit-test / rect-select / drag / highlight helpers. None live on
//! `MindMapDocument` — they all take a `MindMapTree` / `MindMap` +
//! screen coordinates and return values, so unit tests don't need a
//! GPU or an event loop.
//!
//! `hit_test` takes `&mut MindMapTree` because the BVH descent may
//! trigger a lazy subtree-AABB recomputation on the first call after
//! a mutation. All other helpers remain read-only.

use glam::Vec2;

use baumhard::core::primitives::Range;
use baumhard::gfx_structs::area::GlyphAreaCommand;
use baumhard::gfx_structs::mutator::{GfxMutator, Mutation};
use baumhard::gfx_structs::tree::MutatorTree;
use baumhard::gfx_structs::tree_walker::walk_tree_from;
use baumhard::mindmap::connection;
use baumhard::mindmap::model::MindMap;
use baumhard::mindmap::tree_builder::MindMapTree;

use super::types::EdgeRef;

/// Find the mindmap node ID under `canvas_pos` using BVH-accelerated
/// tree descent. Returns the innermost (smallest-area) hit, or `None`.
///
/// # Costs
///
/// O(branching_factor × depth) when subtrees are spatially disjoint;
/// O(n) worst case. One `Vec` allocation on the first call after a
/// mutation (subtree AABB recomputation); O(1) on subsequent calls.
pub fn hit_test(canvas_pos: Vec2, tree: &mut MindMapTree) -> Option<String> {
    tree.tree
        .descendant_at(canvas_pos)
        .and_then(|nid| tree.mind_id_for_node(nid))
        .map(|s| s.to_owned())
}

/// Is `canvas_pos` inside the AABB of node `node_id`? Reads the tree-side
/// glyph area so drag-preview positions count (tree is authoritative
/// during in-flight mutations; identical to the model when idle).
///
/// Unlike `hit_test`, this answers a point-in-specific-node question —
/// a click over a child of `node_id` still counts as "inside" `node_id`,
/// which is what the text editor's click-outside-commit gesture wants.
pub fn point_in_node_aabb(canvas_pos: Vec2, node_id: &str, tree: &MindMapTree) -> bool {
    tree.node_map
        .get(node_id)
        .and_then(|nid| tree.tree.arena.get(*nid))
        .and_then(|n| n.get().glyph_area())
        .map(|area| {
            let x = area.position.x.0;
            let y = area.position.y.0;
            let w = area.render_bounds.x.0;
            let h = area.render_bounds.y.0;
            canvas_pos.x >= x
                && canvas_pos.x <= x + w
                && canvas_pos.y >= y
                && canvas_pos.y <= y + h
        })
        .unwrap_or(false)
}

/// Hit test edges: find the nearest visible edge within `tolerance` canvas
/// units of `canvas_pos`. Returns an `EdgeRef` for the closest edge, or
/// `None` if nothing is within range.
///
/// Visibility filter mirrors `scene_builder::build_scene_with_offsets` — an
/// edge is eligible only if `edge.visible` is true, both endpoint nodes
/// exist, and neither endpoint is hidden by fold state.
pub fn hit_test_edge(canvas_pos: Vec2, map: &MindMap, tolerance: f32) -> Option<EdgeRef> {
    let mut best: Option<(EdgeRef, f32)> = None;
    for edge in &map.edges {
        if !edge.visible {
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
        if map.is_hidden_by_fold(from_node) || map.is_hidden_by_fold(to_node) {
            continue;
        }

        let from_pos = Vec2::new(from_node.position.x as f32, from_node.position.y as f32);
        let from_size = Vec2::new(from_node.size.width as f32, from_node.size.height as f32);
        let to_pos = Vec2::new(to_node.position.x as f32, to_node.position.y as f32);
        let to_size = Vec2::new(to_node.size.width as f32, to_node.size.height as f32);

        let path = connection::build_connection_path(
            from_pos, from_size, edge.anchor_from,
            to_pos, to_size, edge.anchor_to,
            &edge.control_points,
        );
        let dist = connection::distance_to_path(canvas_pos, &path);
        if dist > tolerance {
            continue;
        }
        if best.as_ref().map_or(true, |(_, best_dist)| dist < *best_dist) {
            best = Some((
                EdgeRef::new(&edge.from_id, &edge.to_id, &edge.edge_type),
                dist,
            ));
        }
    }
    best.map(|(e, _)| e)
}

/// Find all node IDs whose bounds intersect the given canvas-space rectangle.
/// The rectangle is defined by two opposite corners (min and max are computed internally).
pub fn rect_select(corner_a: Vec2, corner_b: Vec2, tree: &MindMapTree) -> Vec<String> {
    let min_x = corner_a.x.min(corner_b.x);
    let min_y = corner_a.y.min(corner_b.y);
    let max_x = corner_a.x.max(corner_b.x);
    let max_y = corner_a.y.max(corner_b.y);

    let mut hits = Vec::new();
    for (mind_id, &node_id) in &tree.node_map {
        let area = match tree.tree.arena.get(node_id).and_then(|n| n.get().glyph_area()) {
            Some(a) => a,
            None => continue,
        };
        let x = area.position.x.0;
        let y = area.position.y.0;
        let w = area.render_bounds.x.0;
        let h = area.render_bounds.y.0;

        // AABB overlap test
        if x + w >= min_x && x <= max_x && y + h >= min_y && y <= max_y {
            hits.push(mind_id.clone());
        }
    }
    hits
}

/// Apply a set of node highlights as baumhard mutations. For each
/// `(mind_node_id, color)` pair, the node's existing text-run ranges
/// are collected from its `GlyphArea` and a `GfxMutator::Macro` of one
/// `SetRegionColor(range, color)` mutation per range is applied through
/// `walk_tree_from` — i.e. the highlight is expressed in the same
/// mutation language as the rest of baumhard's tree-walker flow rather
/// than reaching into the arena imperatively.
///
/// Later pairs override earlier ones when the same node appears twice,
/// which is what the reparent/connect modes rely on: callers pass
/// selection highlights first (cyan), then source (orange), then target
/// (green), and the last write wins on conflicts.
///
/// Architectural note: this replaces the earlier trio of
/// `apply_selection_highlight` / `apply_reparent_source_highlight` /
/// `apply_reparent_target_highlight` helpers, which all did the same
/// direct arena patching with different constants. The single function
/// here is both shorter and aligns with architectural decision #6 in
/// ROADMAP.md (mutations as the interaction model).
pub fn apply_tree_highlights<'a, I>(tree: &mut MindMapTree, highlights: I)
where
    I: IntoIterator<Item = (&'a str, [f32; 4])>,
{
    for (mind_id, color) in highlights {
        let Some(&node_id) = tree.node_map.get(mind_id) else { continue };

        // Collect existing region ranges up front. The SetRegionColor
        // mutation needs the exact `Range` of each target region so that
        // the underlying `set_or_insert` finds a match and updates
        // in-place rather than inserting a duplicate region.
        let (ranges, target_channel): (Vec<Range>, usize) = {
            let Some(node) = tree.tree.arena.get(node_id) else { continue };
            let element = node.get();
            let Some(area) = element.glyph_area() else { continue };
            let ranges = area.regions.all_regions().iter().map(|r| r.range).collect();
            // Match the element's channel so the walker's channel-
            // alignment check in `apply_if_matching_channel` passes.
            let channel = {
                use baumhard::gfx_structs::tree::BranchChannel;
                element.channel()
            };
            (ranges, channel)
        };
        if ranges.is_empty() {
            continue;
        }

        let mutations: Vec<Mutation> = ranges
            .into_iter()
            .map(|r| Mutation::area_command(GlyphAreaCommand::SetRegionColor(r, color)))
            .collect();
        let mutator_tree = MutatorTree::new_with(GfxMutator::new_macro(mutations, target_channel));

        // `walk_tree_from` applied at a specific target_id with a
        // single-node MutatorTree runs the macro on that element only
        // (no descendants are touched because the mutator tree has no
        // children, so `align_child_walks` is a no-op). This is the
        // idiomatic "one-shot mutation to a specific node" shape.
        walk_tree_from(&mut tree.tree, &mutator_tree, node_id, mutator_tree.root);
    }
}

/// Apply a position delta directly to nodes in the Baumhard tree (in-place mutation).
/// Used during drag for fast visual preview without rebuilding from the MindMap model.
pub fn apply_drag_delta(tree: &mut MindMapTree, node_id: &str, dx: f32, dy: f32, include_descendants: bool) {
    let tree_node_id = match tree.node_map.get(node_id) {
        Some(&id) => id,
        None => return,
    };

    if include_descendants {
        apply_delta_recursive(&mut tree.tree.arena, tree_node_id, dx, dy);
    } else if let Some(node) = tree.tree.arena.get_mut(tree_node_id) {
        if let Some(area) = node.get_mut().glyph_area_mut() {
            area.move_position(dx, dy);
        }
    }
}

/// Apply a position delta and return `(unique_id, new_position)` for
/// every node that was moved. The renderer uses these patches to
/// update buffer positions in-place without reshaping text.
///
/// O(moved_nodes) — no text shaping, no font-system lock. Uses
/// `first_child` / `next_sibling` iteration instead of collecting
/// descendants into a `Vec` (§B7).
pub fn apply_drag_delta_and_collect_patches(
    tree: &mut MindMapTree,
    node_id: &str,
    dx: f32, dy: f32,
    include_descendants: bool,
    patches: &mut Vec<(usize, (f32, f32))>,
) {
    let tree_node_id = match tree.node_map.get(node_id) {
        Some(&id) => id,
        None => return,
    };

    if include_descendants {
        collect_patches_recursive(&mut tree.tree.arena, tree_node_id, dx, dy, patches);
    } else {
        if let Some(node) = tree.tree.arena.get_mut(tree_node_id) {
            let elem = node.get_mut();
            if let Some(area) = elem.glyph_area_mut() {
                area.move_position(dx, dy);
            }
            let pos = elem.position();
            patches.push((elem.unique_id(), (pos.x, pos.y)));
        }
    }
}

/// Recursively apply delta and collect patches via `first_child` /
/// `next_sibling` — zero allocations per call (§B7).
fn apply_delta_recursive(
    arena: &mut indextree::Arena<baumhard::gfx_structs::element::GfxElement>,
    node_id: indextree::NodeId,
    dx: f32, dy: f32,
) {
    // Move this node.
    if let Some(node) = arena.get_mut(node_id) {
        if let Some(area) = node.get_mut().glyph_area_mut() {
            area.move_position(dx, dy);
        }
    }
    // Recurse into children.
    let mut child = arena.get(node_id).and_then(|n| n.first_child());
    while let Some(cid) = child {
        child = arena.get(cid).and_then(|n| n.next_sibling());
        apply_delta_recursive(arena, cid, dx, dy);
    }
}

/// Recursively apply delta, collect patches, via `first_child` /
/// `next_sibling` — zero allocations per call (§B7).
fn collect_patches_recursive(
    arena: &mut indextree::Arena<baumhard::gfx_structs::element::GfxElement>,
    node_id: indextree::NodeId,
    dx: f32, dy: f32,
    patches: &mut Vec<(usize, (f32, f32))>,
) {
    // Move this node and collect patch.
    if let Some(node) = arena.get_mut(node_id) {
        let elem = node.get_mut();
        if let Some(area) = elem.glyph_area_mut() {
            area.move_position(dx, dy);
        }
        let pos = elem.position();
        patches.push((elem.unique_id(), (pos.x, pos.y)));
    }
    // Recurse into children.
    let mut child = arena.get(node_id).and_then(|n| n.first_child());
    while let Some(cid) = child {
        child = arena.get(cid).and_then(|n| n.next_sibling());
        collect_patches_recursive(arena, cid, dx, dy, patches);
    }
}
