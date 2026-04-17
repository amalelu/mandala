//! Mindmap tree builder — projects a `MindMap` into a Baumhard
//! `Tree<GfxElement, GfxMutator>` and exposes per-canvas-role
//! builders (borders, portals, connections, connection-labels,
//! edge-handles) that the app crate's scene rebuilders consume.
//!
//! Split by role so each file stays focused:
//! - [`node`] — `MindNode` → `GlyphArea` projection + the recursive
//!   child-insertion walker `build_mindmap_tree` drives.
//! - [`border`] — framed-node border tree + §B2 mutator-tree builder.
//! - [`portal`] — portal-pair markers + §B2 mutator.
//! - [`connection`] — glyph-path edges (caps + body glyphs).
//! - [`connection_label`] — per-edge label glyphs + hitbox map.
//! - [`edge_handle`] — selected-edge handle glyphs (anchors,
//!   midpoint, control points).
//!
//! The `MindMapTree` struct and `build_mindmap_tree` entry point
//! live in this module; everything else is re-exported from the
//! sibling files so call-sites keep the pre-split import paths.

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
mod node;
mod portal;

#[cfg(test)]
mod tests;

pub use border::{
    border_identity_sequence, build_border_mutator_tree, build_border_mutator_tree_from_nodes,
    build_border_tree, build_border_tree_from_nodes, border_node_data, BorderNodeData,
};
pub use connection::{
    build_connection_mutator_tree, build_connection_tree, connection_identity_sequence,
    ConnectionEdgeIdentity,
};
pub use connection_label::{
    build_connection_label_mutator_tree, build_connection_label_tree,
    connection_label_identity_sequence, ConnectionLabelMutator, ConnectionLabelTree,
};
pub use edge_handle::{
    build_edge_handle_mutator_tree, build_edge_handle_tree, edge_handle_channel_for,
    edge_handle_identity_sequence,
};
pub use portal::{
    build_portal_mutator_tree, build_portal_mutator_tree_from_pairs, build_portal_tree,
    build_portal_tree_from_pairs, portal_identity_sequence, portal_pair_data,
    PortalColorPreviewRef, PortalIdentity, PortalMutator, PortalPairData, PortalTree,
    SelectedEdgeRef,
};

use node::{build_children_recursive, mindnode_to_glyph_area};

/// Result of building a Baumhard tree from a MindMap. The tree
/// mirrors the MindMap's parent-child hierarchy, with each
/// MindNode represented as a GlyphArea element.
pub struct MindMapTree {
    pub tree: Tree<GfxElement, GfxMutator>,
    /// Maps MindNode ID → indextree NodeId for later lookup.
    pub node_map: HashMap<String, NodeId>,
    /// Reverse map: indextree NodeId → MindNode ID. Built alongside
    /// `node_map` during tree construction. Enables O(1) lookup when
    /// the BVH descent returns a `NodeId` and the caller needs the
    /// corresponding mindmap node ID.
    ///
    /// Private to preserve forward-compatible API (§B10) — callers
    /// use [`MindMapTree::mind_id_for_node`] instead.
    reverse_node_map: HashMap<NodeId, String>,
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
/// Each MindNode becomes a GlyphArea element with its text,
/// position, size, and color regions.
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
        let element = GfxElement::new_area_non_indexed_with_id(area, root.channel, id_counter);
        id_counter += 1;

        let node_id = tree.arena.new_node(element);
        tree.root.append(node_id, &mut tree.arena);
        node_map.insert(root.id.clone(), node_id);

        build_children_recursive(map, &root.id, node_id, &mut tree, &mut node_map, &mut id_counter);
    }

    let reverse_node_map: HashMap<NodeId, String> = node_map
        .iter()
        .map(|(mind_id, &node_id)| (node_id, mind_id.clone()))
        .collect();
    MindMapTree { tree, node_map, reverse_node_map }
}

impl MindMapTree {
    /// Look up the MindMap node ID for a given arena `NodeId`.
    ///
    /// O(1) hash lookup. Returns `None` if `node_id` does not
    /// correspond to a MindNode (e.g. it is the void root or was
    /// removed).
    pub fn mind_id_for_node(&self, node_id: NodeId) -> Option<&str> {
        self.reverse_node_map.get(&node_id).map(|s| s.as_str())
    }
}
