// SPDX-License-Identifier: MPL-2.0

//! Mindmap tree builder — projects a `MindMap` into Baumhard
//! `Tree<GfxElement, GfxMutator>`s, one per canvas role (nodes,
//! borders, portals, connections, connection-labels, section
//! frames, edge / node / section handles), which the app crate's
//! scene rebuilders register into `AppScene`.
//!
//! Every role file follows the same **courier** shape: a data pass
//! that resolves the model + this frame's
//! [`overrides`](crate::mindmap::tree_builder::overrides)
//! into plain per-element data, and a pair of projections that turn that data
//! into either a fresh tree or an in-place `MutatorTree`. Style is
//! resolved exactly once, in the data pass; the projections never
//! re-resolve it.
//!
//! Two inputs cross role boundaries rather than belonging to one:
//! [`node_clip_aabbs`](crate::mindmap::tree_builder::node_clip_aabbs),
//! which the connection sampler clips glyph samples against, and
//! [`EdgePathCache`](crate::mindmap::tree_builder::EdgePathCache),
//! which holds one `ConnectionPath` per edge per frame for the three
//! passes that each used to build their own. (Spelled absolute —
//! `mod.rs` merges an outer `///` into this header and a relative
//! link in the merged block resolves against no module; see
//! `CODE_CONVENTIONS.md` §8.)
//!
//! `MindMapTree` and `build_mindmap_tree` live here; per-role
//! builders are re-exported from the sibling files.

use std::collections::HashMap;

use indextree::NodeId;

use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::GfxMutator;
use crate::gfx_structs::tree::Tree;
use crate::mindmap::model::MindMap;

mod border;
mod connection;
mod connection_label;
mod edge_handle;
mod edge_path;
mod handle;
mod node;
mod node_clip;
mod node_resize_handle;
mod portal;
mod portal_style;
mod section_frame;
mod section_resize_handle;

pub mod overrides;

#[cfg(test)]
mod tests;

pub use border::{
    border_content_signature, border_node_data, border_structure_signature, build_border_mutator_tree,
    build_border_mutator_tree_from_nodes, build_border_tree, build_border_tree_from_nodes,
    BorderChromeOverrides, BorderNodeData,
};
pub use connection::{
    build_connection_elements, build_connection_mutator_tree, build_connection_tree,
    connection_identity_sequence, point_inside_any_node, ConnectionEdgeIdentity, ConnectionElement,
};
pub use connection_label::{
    build_connection_label_mutator_tree, build_connection_label_tree, build_label_elements,
    connection_label_identity_sequence, ConnectionLabelElement, ConnectionLabelHitIndex,
    ConnectionLabelMutator, ConnectionLabelTree,
};
pub use edge_handle::{build_edge_handles, edge_handle_channel_for, EdgeHandleElement, EdgeHandleKind};
pub use edge_path::{edge_endpoint_rects, offset_node_rect, EdgePathCache};
pub use handle::{build_handle_mutator_tree, build_handle_tree, handle_identity_sequence, HandleVisual};
pub use node::{
    effective_section_scale, section_default_regions, DEFAULT_SECTION_FONT_SCALE, LINE_HEIGHT_FACTOR,
};
pub use node_clip::{node_clip_aabbs, INACTIVE_NODE_ALPHA_MULTIPLIER};
pub use node_resize_handle::{
    build_node_resize_handles, build_selected_node_handles, NodeResizeHandleElement,
};
pub use overrides::{
    BorderConfigEditsView, BorderPreview, BorderPreviewTargetRef, EdgeColorPreview, EditView, FrameOverrides,
    InteractionModeOverrides, PortalColorPreview, PortalTextEditOverride, SceneSelectionContext,
};
pub use portal::{
    build_portal_mutator_tree, build_portal_mutator_tree_from_pairs, build_portal_tree,
    build_portal_tree_from_pairs, portal_content_signature, portal_pair_data, portal_structure_signature,
    EndpointAreas, PortalHit, PortalHitIndex, PortalIdentity, PortalMutator, PortalPairData, PortalPart,
    PortalTree, SelectedEdgeRef,
};
pub use portal_style::{
    resolve_portal_endpoint_style, resolve_portal_endpoint_text_style, ResolvedPortalStyle,
    ResolvedPortalTextStyle, SelectedPortalLabel,
};
pub use section_frame::{
    build_section_frame_tree, build_section_frames, section_frame_content_signature, SectionFrameElement,
};
pub use section_resize_handle::{
    build_section_resize_handles, build_selected_section_handles, infer_resize_anchor, ResizeHandleSide,
    SectionResizeHandleElement, SECTION_RESIZE_HANDLE_FONT_SIZE_PT, SECTION_RESIZE_HANDLE_GLYPH,
};

use crate::mindmap::model::ChildIndex;
use node::{append_node_sections, build_descendants, mindnode_container_area};

/// Result of building a Baumhard tree from a MindMap. The tree
/// mirrors the MindMap's parent-child hierarchy. Each MindNode
/// produces a three-deep subtree:
///
/// - one **container** `GlyphArea` (chrome only — background, frame
///   padding, shape, zoom window),
/// - one **section-area** `GlyphArea` per [`MindSection`], carrying
///   the section's text + theme-resolved regions (these are the
///   buffers the renderer's tree walker shapes),
/// - one **section-model** `GlyphModel` child per section-area as a
///   structural seam for future per-component / per-grapheme
///   mutations.
///
/// [`MindSection`]: crate::mindmap::model::MindSection
pub struct MindMapTree {
    pub tree: Tree<GfxElement, GfxMutator>,
    /// Maps MindNode ID → arena `NodeId` of the *container* area.
    /// Private to keep §B10 — callers reach this through
    /// [`Self::arena_id_for`] / [`Self::node_ids`] accessors so a
    /// future internal-representation change (different map type,
    /// extra metadata per entry) doesn't ripple through every
    /// consumer.
    node_map: HashMap<String, NodeId>,
    /// Maps `(MindNode ID, section index)` → arena `NodeId` of
    /// the section-area. Empty for nodes whose sections were
    /// excluded by fold (the same fold path that excludes a whole
    /// node from `node_map`). Section-models are reachable as the
    /// only child of the section-area inside the arena, so no
    /// separate map is needed for them.
    ///
    /// Private — callers use [`Self::section_arena_id`].
    section_map: HashMap<(String, usize), NodeId>,
    /// Reverse map: arena `NodeId` → MindNode ID. Populated for
    /// container areas only — section-areas live in
    /// `section_map`'s values, not here, so a hit on a section
    /// element returns `None` from [`Self::mind_id_for_node`]
    /// (callers walk one level up via the arena to find the
    /// container).
    ///
    /// Private to preserve forward-compatible API (§B10) — callers
    /// use [`MindMapTree::mind_id_for_node`] /
    /// [`MindMapTree::section_for_node`] instead.
    reverse_node_map: HashMap<NodeId, String>,
    /// Reverse map for sections: arena `NodeId` of a section-area
    /// → `(MindNode ID, section index)`. Populated alongside
    /// `section_map` during tree construction so hit tests that
    /// land on a section can recover the (node_id, section_idx)
    /// pair in O(1) without an arena climb.
    reverse_section_map: HashMap<NodeId, (String, usize)>,
    /// Per-mind-id section count. The hit-test single-section-
    /// fold heuristic asks "does this MindNode have more than one
    /// section?" on every click — caching the count once at build
    /// time avoids the per-click arena children walk.
    section_counts: HashMap<String, usize>,
}

/// Builds a `Tree<GfxElement, GfxMutator>` from a MindMap's
/// hierarchy.
///
/// The tree structure mirrors the MindMap's parent-child
/// relationships:
/// - A Void root node at the top
/// - Each root MindNode (parent_id is None) as a child of the
///   Void root
/// - Children nested recursively following parent_id
/// - Nodes hidden by fold state are excluded
///
/// Each MindNode produces three layers:
/// - one *container* `GlyphArea` (chrome only),
/// - one *section-area* `GlyphArea` per
///   [`MindSection`](crate::mindmap::model::MindSection) as a
///   sibling of any child mind-node-areas; sections carry the
///   text + regions the renderer shapes,
/// - one structural *section-model* `GlyphModel` child per
///   section-area (a future per-component-mutation seam — the
///   renderer skips it).
pub fn build_mindmap_tree(map: &MindMap) -> MindMapTree {
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    let mut node_map: HashMap<String, NodeId> = HashMap::new();
    let mut section_map: HashMap<(String, usize), NodeId> = HashMap::new();
    let mut id_counter: usize = 1; // 0 is reserved for the Void root

    let index = ChildIndex::build(map);
    for root in index.roots() {
        // Roots have no ancestors, so they are never hidden by fold.
        let area = mindnode_container_area(map, root);
        let element = GfxElement::new_area_non_indexed_with_id(area, root.channel, id_counter);
        id_counter += 1;

        let node_id = tree.root.append_value(element, &mut tree.arena);
        node_map.insert(root.id.clone(), node_id);

        append_node_sections(map, root, node_id, &mut tree, &mut section_map, &mut id_counter);

        build_descendants(
            map,
            &index,
            &root.id,
            root.folded,
            node_id,
            &mut tree,
            &mut node_map,
            &mut section_map,
            &mut id_counter,
        );
    }

    let reverse_node_map: HashMap<NodeId, String> = node_map
        .iter()
        .map(|(mind_id, &arena_id)| (arena_id, mind_id.clone()))
        .collect();
    let reverse_section_map: HashMap<NodeId, (String, usize)> = section_map
        .iter()
        .map(|((mind_id, idx), &arena_id)| (arena_id, (mind_id.clone(), *idx)))
        .collect();
    let mut section_counts: HashMap<String, usize> = HashMap::new();
    for (mind_id, _) in section_map.keys() {
        *section_counts.entry(mind_id.clone()).or_insert(0) += 1;
    }
    MindMapTree {
        tree,
        node_map,
        section_map,
        reverse_node_map,
        reverse_section_map,
        section_counts,
    }
}

impl MindMapTree {
    /// Look up the MindMap node ID for a *container* arena
    /// `NodeId`. Returns `None` for void roots, removed nodes,
    /// section-areas, and section-models — those are not
    /// node-containers. Use [`Self::section_for_node`] to resolve
    /// section-area arena ids; both maps together cover every
    /// hit-test target an interactive path can land on.
    ///
    /// O(1) hash lookup, no allocation.
    pub fn mind_id_for_node(&self, node_id: NodeId) -> Option<&str> {
        self.reverse_node_map.get(&node_id).map(|s| s.as_str())
    }

    /// Look up the `(MindNode ID, section index)` pair for an
    /// arena `NodeId`, returning `None` when the id is anything
    /// other than a section-area (containers, section-models,
    /// the void root, removed nodes).
    ///
    /// O(1) hash lookup, no allocation.
    pub fn section_for_node(&self, node_id: NodeId) -> Option<(&str, usize)> {
        self.reverse_section_map
            .get(&node_id)
            .map(|(mind_id, idx)| (mind_id.as_str(), *idx))
    }

    /// How many sections the named MindNode has in this tree, or
    /// `0` when the node is missing or folded out. The hit-test
    /// fold-to-NodeContainer heuristic ("single-section nodes
    /// behave like whole-node clicks") consults this to avoid
    /// per-click arena traversals; populated once at tree build.
    ///
    /// O(1) hash lookup, no allocation.
    pub fn section_count_for(&self, mind_id: &str) -> usize {
        self.section_counts.get(mind_id).copied().unwrap_or(0)
    }

    /// Container arena `NodeId` for a MindNode id, or `None` when
    /// the node is missing / folded out / not yet built. The
    /// post-section caller-facing accessor over the private
    /// `node_map`.
    ///
    /// O(1) hash lookup, no allocation.
    pub fn arena_id_for(&self, mind_id: &str) -> Option<NodeId> {
        self.node_map.get(mind_id).copied()
    }

    /// Section-area arena `NodeId` for a `(MindNode id, section
    /// index)` pair, or `None` when the section is missing.
    /// Caller-facing accessor over the private `section_map`.
    ///
    /// O(1) hash lookup, no allocation.
    pub fn section_arena_id(&self, mind_id: &str, section_idx: usize) -> Option<NodeId> {
        self.section_map.get(&(mind_id.to_string(), section_idx)).copied()
    }

    /// Iterator over every container `(mind_id, arena_id)` pair.
    /// Replaces direct `&node_map` borrowing — keeps the storage
    /// representation private. Iteration order is HashMap-defined
    /// (caller sorts when stability matters).
    pub fn node_ids(&self) -> impl Iterator<Item = (&str, NodeId)> + '_ {
        self.node_map.iter().map(|(k, v)| (k.as_str(), *v))
    }

    /// `true` if this tree owns a container for `mind_id`.
    /// Convenience wrapper over [`Self::arena_id_for`] for
    /// presence checks.
    pub fn contains_node(&self, mind_id: &str) -> bool {
        self.node_map.contains_key(mind_id)
    }

    /// Number of MindNode containers in this tree (sections /
    /// section-models excluded). Equivalent to
    /// `self.node_ids().count()` but O(1).
    pub fn node_count(&self) -> usize {
        self.node_map.len()
    }

    /// Iterator over every section as `((mind_id, section_idx),
    /// arena_id)`. Replaces direct `&section_map` borrowing —
    /// keeps the storage representation private. HashMap
    /// iteration order; caller sorts when stability matters.
    pub fn section_ids(&self) -> impl Iterator<Item = ((&str, usize), NodeId)> + '_ {
        self.section_map
            .iter()
            .map(|((mid, idx), arena)| ((mid.as_str(), *idx), *arena))
    }

    /// Resolve a hit-tested arena `NodeId` to the owning MindNode
    /// id. Whether the user landed on the container, a section-
    /// area, or a section-model, the caller almost always wants
    /// the MindNode id; this helper consolidates the climb so
    /// every hit-test site doesn't reimplement the dispatch.
    ///
    /// Climbs the parent chain until a container arena id is hit
    /// (i.e. `mind_id_for_node` returns `Some`) or the root is
    /// reached. The depth is bounded by the section subtree's
    /// natural shape today (model → section → container = 3
    /// edges) but the climb is *not* hardcoded to 3, so a future
    /// per-component refinement that deepens a section's subtree
    /// (e.g. additional `GlyphModel` children for richer
    /// composition) keeps resolving correctly without a code
    /// change here.
    ///
    /// Cost: O(climb depth) — one arena lookup per edge. Bounded
    /// in practice by the depth of the section subtree which is
    /// 2 today (3 if you count the container hit).
    pub fn owning_mind_id(&self, mut node_id: NodeId) -> Option<&str> {
        loop {
            if let Some(id) = self.mind_id_for_node(node_id) {
                return Some(id);
            }
            node_id = self.tree.arena.get(node_id)?.parent()?;
        }
    }
}

#[cfg(test)]
mod append_cost_tests {
    /// **The scene build must stay linear in the node count.**
    ///
    /// `indextree::NodeId::append` calls `checked_append`, which walks
    /// `self.ancestors(arena)` looking for a cycle before every append.
    /// A `.mindmap.json` may declare a linear `parent_id` chain — a
    /// legal acyclic tree the loader accepts, with no depth bound
    /// anywhere on the load path — so depth equals the node count and
    /// the walk makes `build_mindmap_tree` O(N²).
    ///
    /// That is reachable from a file alone, and it is worse than a
    /// slow load: `FreezeWatchdog` calls `std::process::abort()` when
    /// the main thread is silent past ten seconds, so a rebuild
    /// triggered by an ordinary click on a large chain map kills the
    /// process with unsaved work — the SIGABRT-with-nothing-to-degrade
    /// outcome the recursion-to-iteration conversion was supposed to
    /// have removed. Converting the walker to iteration fixed the
    /// stack, and left the quadratic in place.
    ///
    /// Every append here is of a node `arena.new_node` produced on the
    /// line above: detached by construction, so the ancestor check can
    /// never fire and `append_value` — indextree's documented O(1)
    /// fast path — is exactly equivalent. This pins the source rather
    /// than a duration, because a timing assertion on a shared runner
    /// is a flake, and because the property that matters is structural.
    #[test]
    fn test_the_tree_builder_never_pays_for_a_cycle_check() {
        let found = crate::util::source_scan::tree_builder_checked_appends();
        assert!(
            found.is_empty(),
            "these appends re-check for a cycle on every call, which makes the scene build \
             O(N²) on a linear parent_id chain a hostile file can declare:\n{}\n\n\
             Use `parent.append_value(element, &mut arena)`, which returns the new NodeId \
             and skips the ancestor walk. It is sound here because the node being appended \
             was just created and is detached.",
            found
                .iter()
                .map(|(f, n, line)| format!("  {f}:{n}   {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}
