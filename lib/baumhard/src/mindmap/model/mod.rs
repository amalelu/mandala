// SPDX-License-Identifier: MPL-2.0

//! Mindmap data model — what the loader deserializes from
//! `.mindmap.json` and the document layer mutates. This module
//! owns the top-level `MindMap` struct plus its tree-shape queries
//! (root / ancestry / descendants).
//!
//! **Every deserializable type here is closed**
//! (`#[serde(deny_unknown_fields)]`), and so is every type reachable
//! from one. The app resaves the whole model, so a key serde ignored
//! at load is a key erased from the author's file at save; rejecting
//! it is the only outcome that leaves the file intact. The rule is
//! not maintained by hand — `mindmap::loader`'s
//! `test_every_loadable_type_rejects_unknown_keys` walks the model's
//! own source and fails until a newly reachable type opts in. See
//! `format/schema.md` §"Unknown keys are rejected".

pub mod canvas;
pub mod edge;
pub mod node;
pub mod palette;
pub mod text_run_ops;
pub mod validate;

pub use canvas::Canvas;
pub use edge::{
    is_portal_edge, portal_endpoint_state, portal_endpoint_state_mut, ControlPoint, EdgeLabelConfig,
    GlyphConnectionConfig, MindEdge, PortalEndpointState, DEFAULT_LABEL_SIZE_FACTOR, DISPLAY_MODE_LINE,
    DISPLAY_MODE_PORTAL, PORTAL_GLYPH_PRESETS,
};
pub use node::{
    ColorGroup, ColorSchema, CustomBorderGlyphs, GlyphBorderConfig, MindNode, MindSection, NodeLayout,
    NodeStyle, Position, Size, TextRun, MAX_NODE_AXIS, MAX_SECTIONS_PER_NODE,
};
pub use palette::Palette;

use crate::mindmap::custom_mutation::CustomMutation;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// The whole-map value type — what [`crate::mindmap::loader`]
/// deserializes from a `.mindmap.json` file and what the document
/// layer mutates and persists. Carries the version, name, shared
/// canvas state, named palettes, the node map (keyed by Dewey-decimal
/// id), the edge list, and any map-level custom mutations.
///
/// Plain data; no runtime cost beyond the `HashMap` / `Vec`
/// allocations serde performs. Tree-shape queries
/// ([`Self::root_nodes`], [`Self::children_of`],
/// [`Self::is_ancestor_or_self`], etc.) walk the node map lazily —
/// see each method for its per-call cost. For bulk walks, build a
/// [`ChildIndex`] with [`Self::child_index`] or the fold-hidden set
/// with [`Self::fold_hidden_set`] once rather than paying the
/// per-call cost repeatedly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MindMap {
    pub version: String,
    pub name: String,
    pub canvas: Canvas,
    /// Named color palettes referenced by nodes' color_schema.palette field.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub palettes: HashMap<String, Palette>,
    pub nodes: HashMap<String, MindNode>,
    pub edges: Vec<MindEdge>,
    /// Map-level custom mutation definitions, available to all nodes in this map.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_mutations: Vec<CustomMutation>,
    /// Map-level macro definitions, opaque to baumhard. Loaded into
    /// the application-side `MacroRegistry` at the `Map` tier;
    /// stored as untyped JSON values here because the typed
    /// `Macro` lives in the application crate (whose `Action` enum
    /// would otherwise force a circular dependency). The
    /// application-side loader parses each entry per-Macro;
    /// per-entry failures log a `warn!` and skip rather than
    /// failing the whole map load.
    ///
    /// Privilege model: Map-tier macros cannot run `ConsoleLine`
    /// or destructive Actions — see `format/macros.md`. The
    /// privilege gate is enforced at dispatch time in the
    /// application's `dispatch_macro`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub macros: Vec<serde_json::Value>,
}

impl MindMap {
    /// Construct an empty `MindMap` with the given name. The canvas
    /// uses the same default background as fixture maps (`#000000`)
    /// and no theme variants. Nodes and edges start empty — ready to
    /// be populated by the `new` console command (or by direct user
    /// editing once a save target is bound).
    pub fn new_blank(name: impl Into<String>) -> Self {
        MindMap {
            version: "1.0".to_string(),
            name: name.into(),
            canvas: Canvas {
                background_color: "#000000".to_string(),
                default_border: None,
                default_connection: None,
                default_section_frame_border: None,
                default_focused_section_frame_border: None,
                theme_variables: HashMap::new(),
                theme_variants: HashMap::new(),
            },
            palettes: HashMap::new(),
            nodes: HashMap::new(),
            edges: Vec::new(),
            custom_mutations: Vec::new(),
            macros: Vec::new(),
        }
    }

    /// Every node paired with its canonical location stamp (the
    /// node's id). HashMap iteration order; consumers that need
    /// stability sort downstream. Cost: one `String` clone per node.
    pub fn node_locations(&self) -> impl Iterator<Item = (String, &MindNode)> {
        self.nodes.values().map(|n| (n.id.clone(), n))
    }

    /// Every edge paired with its `"edge[<idx>]"` location stamp,
    /// in edge-vector order. Cost: one `format!` per edge.
    pub fn edge_locations(&self) -> impl Iterator<Item = (String, &MindEdge)> {
        self.edges
            .iter()
            .enumerate()
            .map(|(i, e)| (format!("edge[{}]", i), e))
    }

    /// Returns root nodes (nodes with no parent), sorted by ID segment.
    ///
    /// Cost: O(N) scan + O(R log R) sort, where R is the root count.
    /// For repeated tree-shape walks, build a [`ChildIndex`] once with
    /// [`Self::child_index`] instead of calling this per node.
    pub fn root_nodes(&self) -> Vec<&MindNode> {
        let mut roots: Vec<&MindNode> = self.nodes.values().filter(|n| n.parent_id.is_none()).collect();
        roots.sort_by_key(|n| id_sort_key(&n.id));
        roots
    }

    /// Returns children of a given node, sorted by ID segment.
    ///
    /// Cost: O(N) scan + O(C log C) sort per call. Do not call this in
    /// a loop over every node — that is O(N²). Build a [`ChildIndex`]
    /// with [`Self::child_index`] and call [`ChildIndex::children_of`]
    /// instead for O(N) total.
    pub fn children_of(&self, parent_id: &str) -> Vec<&MindNode> {
        let mut children: Vec<&MindNode> = self
            .nodes
            .values()
            .filter(|n| n.parent_id.as_deref() == Some(parent_id))
            .collect();
        children.sort_by_key(|n| id_sort_key(&n.id));
        children
    }

    /// Returns true if any ancestor of this node is folded, meaning
    /// this node should be hidden from view.
    ///
    /// Cost: O(depth) parent-chain walk per call. When testing many
    /// nodes in one build (e.g. every node / edge endpoint in a scene),
    /// build the hidden set once with [`Self::fold_hidden_set`] and
    /// test membership instead.
    ///
    /// Defense in depth against a `parent_id` cycle that somehow
    /// reaches this walker despite the loader's load-time rejection
    /// (e.g. a cycle introduced by a future mutation path): the walk
    /// is capped at `self.nodes.len()` steps, since a valid parent
    /// chain can never exceed the node count. Hitting the cap logs
    /// and treats the node as visible rather than hanging — see
    /// CODE_CONVENTIONS §9 ("interactive paths must not panic").
    pub fn is_hidden_by_fold(&self, node: &MindNode) -> bool {
        let mut current_id = node.parent_id.as_deref();
        let mut steps = 0usize;
        while let Some(pid) = current_id {
            if steps > self.nodes.len() {
                log::error!(
                    "is_hidden_by_fold: parent_id cycle detected walking up from node {:?}; treating as visible",
                    node.id
                );
                return false;
            }
            steps += 1;
            match self.nodes.get(pid) {
                Some(parent) => {
                    if parent.folded {
                        return true;
                    }
                    current_id = parent.parent_id.as_deref();
                }
                None => return false,
            }
        }
        false
    }

    /// Collect all descendant IDs of a node (recursive), not including the node itself.
    ///
    /// Cost: builds a [`ChildIndex`] once (O(N)) then walks the subtree
    /// in O(descendants). Prefer [`ChildIndex::all_descendant_ids`] when
    /// you already hold an index.
    pub fn all_descendants(&self, node_id: &str) -> Vec<String> {
        let index = ChildIndex::build(self);
        index.all_descendant_ids(node_id, self.nodes.len())
    }

    /// Returns the set of node IDs hidden because an ancestor is folded.
    /// Computed in one O(N) pass over a [`ChildIndex`]; use this once
    /// per scene / tree build and thread the result through the passes
    /// instead of calling [`Self::is_hidden_by_fold`] per element.
    pub fn fold_hidden_set(&self) -> HashSet<&str> {
        let index = ChildIndex::build(self);
        let mut hidden = HashSet::new();
        // True roots first...
        for root in index.roots() {
            Self::mark_hidden_with_index(&index, root, false, &mut hidden);
        }
        // ...then any node whose parent_id is missing from the map.
        // The loader treats these as root-like, and their children must
        // still be hidden when the node is folded (the old
        // `is_hidden_by_fold` parent-chain walk produced that result).
        for node in self.nodes.values() {
            if let Some(pid) = &node.parent_id {
                if !self.nodes.contains_key(pid) {
                    Self::mark_hidden_with_index(&index, node, false, &mut hidden);
                }
            }
        }
        hidden
    }

    fn mark_hidden_with_index<'a, 'b>(
        index: &'b ChildIndex<'a>,
        node: &'a MindNode,
        ancestors_folded: bool,
        hidden: &mut HashSet<&'a str>,
    ) {
        if ancestors_folded {
            hidden.insert(&node.id);
        }
        for child in index.children_of(&node.id) {
            Self::mark_hidden_with_index(index, child, ancestors_folded || node.folded, hidden);
        }
    }

    /// Returns true if `candidate_ancestor` equals `node_id` or is a (transitive)
    /// ancestor of it. Used to prevent reparenting a node under itself or under
    /// one of its own descendants (which would create a cycle).
    ///
    /// Cost: O(depth) parent-chain walk per call.
    ///
    /// Defense in depth: caps the walk at `self.nodes.len()` steps
    /// so a `parent_id` cycle can't hang this call — see
    /// `is_hidden_by_fold` for the same reasoning.
    pub fn is_ancestor_or_self(&self, candidate_ancestor: &str, node_id: &str) -> bool {
        if candidate_ancestor == node_id {
            return true;
        }
        let mut current = self.nodes.get(node_id).and_then(|n| n.parent_id.as_deref());
        let mut steps = 0usize;
        while let Some(pid) = current {
            if pid == candidate_ancestor {
                return true;
            }
            if steps > self.nodes.len() {
                log::error!(
                    "is_ancestor_or_self: parent_id cycle detected walking up from node {:?}; treating {:?} as not an ancestor",
                    node_id, candidate_ancestor
                );
                return false;
            }
            steps += 1;
            current = self.nodes.get(pid).and_then(|n| n.parent_id.as_deref());
        }
        false
    }

    /// Resolves the effective colors for a themed node by looking up
    /// the palette from the top-level palettes map.
    pub fn resolve_theme_colors<'a>(&'a self, node: &'a MindNode) -> Option<&'a ColorGroup> {
        let schema = node.color_schema.as_ref()?;
        let palette = self.palettes.get(&schema.palette)?;
        let level = schema.level as usize;
        if level < palette.groups.len() {
            Some(&palette.groups[level])
        } else {
            palette.groups.last()
        }
    }

    /// Build a one-pass parent → sorted-children index for tree-shape
    /// walks. Returns a [`ChildIndex`] that answers `children_of` in
    /// O(1) (plus the length of the child list) and `roots` in O(1).
    ///
    /// Cost: one O(N) scan over `nodes` plus sorting each child list.
    /// Use this whenever you would otherwise call [`Self::children_of`]
    /// in a loop — that pattern is O(N²); this index makes it O(N).
    pub fn child_index(&self) -> ChildIndex<'_> {
        ChildIndex::build(self)
    }
}

/// O(N) parent → sorted-children lookup. Build once with
/// [`MindMap::child_index`] and reuse for any walk that would otherwise
/// call [`MindMap::children_of`] in a loop.
#[derive(Debug, Clone)]
pub struct ChildIndex<'a> {
    roots: Vec<&'a MindNode>,
    by_parent: HashMap<&'a str, Vec<&'a MindNode>>,
}

impl<'a> ChildIndex<'a> {
    /// Build the index from `map`. Roots and each child list are sorted
    /// by [`id_sort_key`], matching the order returned by
    /// [`MindMap::root_nodes`] and [`MindMap::children_of`].
    pub fn build(map: &'a MindMap) -> Self {
        let mut roots: Vec<&'a MindNode> = Vec::new();
        let mut by_parent: HashMap<&'a str, Vec<&'a MindNode>> = HashMap::new();
        for node in map.nodes.values() {
            match &node.parent_id {
                None => roots.push(node),
                Some(pid) => by_parent.entry(pid.as_str()).or_default().push(node),
            }
        }
        roots.sort_by_key(|n| id_sort_key(&n.id));
        for children in by_parent.values_mut() {
            children.sort_by_key(|n| id_sort_key(&n.id));
        }
        Self { roots, by_parent }
    }

    /// Root nodes, sorted by ID segment.
    pub fn roots(&self) -> &[&'a MindNode] {
        &self.roots
    }

    /// Children of `parent_id`, sorted by ID segment. Returns an empty
    /// slice when the parent has no children.
    pub fn children_of(&self, parent_id: &str) -> &[&'a MindNode] {
        self.by_parent.get(parent_id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Collect all descendant IDs of `node_id` (recursive), not
    /// including `node_id` itself.
    ///
    /// `budget` bounds the walk as defense in depth against a
    /// `parent_id` cycle; a valid tree never reaches the budget.
    pub fn all_descendant_ids(&self, node_id: &str, budget: usize) -> Vec<String> {
        let mut result = Vec::new();
        self.collect_descendants(node_id, &mut result, budget);
        result
    }

    fn collect_descendants(&self, node_id: &str, result: &mut Vec<String>, budget: usize) {
        for child in self.children_of(node_id) {
            if result.len() >= budget {
                log::error!(
                    "collect_descendants: parent_id cycle detected walking down from node {:?}; truncating",
                    node_id
                );
                return;
            }
            result.push(child.id.clone());
            self.collect_descendants(&child.id, result, budget);
        }
    }
}

/// Extract the last segment of a Dewey-decimal ID as a numeric sort key.
/// `"1.2.3"` → `3`, `"0"` → `0`. Falls back to 0 for non-numeric IDs.
pub fn id_sort_key(id: &str) -> usize {
    id.rsplit('.')
        .next()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0)
}

/// Derive the parent ID from a Dewey-decimal node ID.
/// `"1.2.3"` → `Some("1.2")`, `"0"` → `None` (root node).
pub fn derive_parent_id(id: &str) -> Option<String> {
    let dot = id.rfind('.')?;
    Some(id[..dot].to_string())
}

#[cfg(test)]
mod tests;
