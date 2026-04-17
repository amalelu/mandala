//! Data structures pulled out of the pre-split `document.rs`:
//! the animation runtime record, the ref types (`EdgeRef`,
//! `ReparentUndoData`), the selection state enum, the
//! reparent-highlight constants, and `HIGHLIGHT_COLOR`.
//! No methods on `MindMapDocument` live here — those are
//! sharded across the other submodules.

use baumhard::mindmap::animation::AnimationTiming;
use baumhard::mindmap::custom_mutation::CustomMutation;
use baumhard::mindmap::model::{MindEdge, MindNode};

/// Selection highlight color: bright cyan [R, G, B, A]
pub const HIGHLIGHT_COLOR: [f32; 4] = [0.0, 0.9, 1.0, 1.0];

/// Per-active-mutation runtime record for the Phase-4 animation
/// system. Carries the from/to `MindNode` snapshot and the
/// driving `CustomMutation`; the dispatcher in
/// [`MindMapDocument::tick_animations`] interpolates per-frame
/// and writes the blended state back into `mindmap.nodes`.
///
/// `cm` is the single source of truth — `mutation_id()` and
/// `timing()` project out the fields the dispatcher needs, so
/// there is no way for a mutation_id / timing copy to drift out
/// of sync with the underlying `CustomMutation`.
#[derive(Debug, Clone)]
pub struct AnimationInstance {
    /// Node id this animation targets.
    pub target_id: String,
    /// Pre-mutation snapshot of the target node. Stored whole so
    /// any future per-field interpolator can pull the source.
    pub from_node: MindNode,
    /// Post-mutation snapshot of the target node, computed once
    /// at start by applying the mutation to a scratch copy.
    pub to_node: MindNode,
    /// Wall-clock timestamp (ms) when the animation started.
    pub start_ms: u64,
    /// The `CustomMutation` driving the animation. Carries the
    /// id (for re-trigger detection), the `timing` envelope (for
    /// the tick loop), and the full mutation list (for the
    /// `apply_custom_mutation` commit at completion).
    pub cm: CustomMutation,
}

impl AnimationInstance {
    /// `CustomMutation.id` of the mutation being animated.
    /// Combined with `target_id`, identifies the instance for
    /// re-trigger no-op detection in `start_animation`.
    pub fn mutation_id(&self) -> &str {
        &self.cm.id
    }

    /// The timing envelope. Unwraps `cm.timing` — animations are
    /// only constructed through `start_animation`, which checks
    /// `cm.timing.is_some() && duration_ms > 0` before pushing,
    /// so this projection is always safe by construction.
    pub fn timing(&self) -> &AnimationTiming {
        self.cm
            .timing
            .as_ref()
            .expect("AnimationInstance invariant: cm.timing is always Some")
    }
}

/// Reparent-mode source color: orange, used for nodes currently being reparented.
pub const REPARENT_SOURCE_COLOR: [f32; 4] = [1.0, 0.55, 0.0, 1.0];

/// Reparent-mode target color: green, used for the node currently hovered as
/// a potential reparent target.
pub const REPARENT_TARGET_COLOR: [f32; 4] = [0.2, 1.0, 0.4, 1.0];

/// Identifies an edge in the MindMap by its endpoints and type. Edges have
/// no stable ID, so this triple is the canonical reference (matching how
/// `apply_reparent` looks up parent_child edges at document.rs:301).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EdgeRef {
    pub from_id: String,
    pub to_id: String,
    pub edge_type: String,
}

impl EdgeRef {
    pub fn new(from_id: impl Into<String>, to_id: impl Into<String>, edge_type: impl Into<String>) -> Self {
        Self {
            from_id: from_id.into(),
            to_id: to_id.into(),
            edge_type: edge_type.into(),
        }
    }

    /// Returns true if this ref identifies the given `MindEdge`.
    pub fn matches(&self, edge: &MindEdge) -> bool {
        self.from_id == edge.from_id
            && self.to_id == edge.to_id
            && self.edge_type == edge.edge_type
    }
}

/// Tracks what is currently selected in the document. Node and
/// edge selection are mutually exclusive — selecting one kind clears
/// any prior selection of the other. Portal-mode edges use the
/// `Edge` variant; there is no separate portal selection state
/// because a portal is a render form, not a distinct entity.
#[derive(Clone, Debug)]
pub enum SelectionState {
    None,
    Single(String),
    Multi(Vec<String>),
    Edge(EdgeRef),
}

impl SelectionState {
    pub fn is_selected(&self, node_id: &str) -> bool {
        match self {
            SelectionState::None => false,
            SelectionState::Single(id) => id == node_id,
            SelectionState::Multi(ids) => ids.contains(&node_id.to_string()),
            SelectionState::Edge(_) => false,
        }
    }

    pub fn selected_ids(&self) -> Vec<&str> {
        match self {
            SelectionState::None => vec![],
            SelectionState::Single(id) => vec![id.as_str()],
            SelectionState::Multi(ids) => ids.iter().map(|s| s.as_str()).collect(),
            SelectionState::Edge(_) => vec![],
        }
    }

    /// Returns the selected edge, if any.
    pub fn selected_edge(&self) -> Option<&EdgeRef> {
        match self {
            SelectionState::Edge(e) => Some(e),
            _ => None,
        }
    }
}

/// Return value of `MindMapDocument::apply_reparent`. Contains both the
/// per-node parent entries and a full snapshot of the edges Vec so that
/// edge rewrites can be reversed wholesale on undo.
#[derive(Clone, Debug)]
pub struct ReparentUndoData {
    pub entries: Vec<(String, Option<String>)>,
    pub old_edges: Vec<MindEdge>,
}
