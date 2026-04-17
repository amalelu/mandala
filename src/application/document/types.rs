//! Data structures pulled out of the pre-split `document.rs`:
//! the animation runtime record, the ref types (`EdgeRef`,
//! `ReparentUndoData`), the selection state enum, the
//! reparent-highlight constants, and `HIGHLIGHT_COLOR`.
//! No methods on `MindMapDocument` live here — those are
//! sharded across the other submodules.

use baumhard::mindmap::animation::AnimationTiming;
use baumhard::mindmap::custom_mutation::CustomMutation;
use baumhard::mindmap::model::{MindEdge, MindNode};
use baumhard::mindmap::scene_cache::EdgeKey;

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

/// Tracks what is currently selected in the document. The
/// variants are mutually exclusive — selecting one kind clears
/// any prior selection of the others, enforced by construction
/// (every write to `document.selection` replaces the whole enum
/// value; there's no additive "add this to the selection" API
/// for variants of different kinds). Downstream code can rely on
/// `Edge` and `PortalLabel` in particular never being active at
/// the same moment: the scene builder uses that invariant when
/// it picks which cyan highlight to apply (both-markers for
/// `Edge`, single-marker for `PortalLabel`).
///
/// Portal-mode edges have two selectable forms: selecting the
/// edge body (currently reachable only through the console)
/// goes through `Edge`; selecting a single portal label (the
/// glyph attached to one endpoint node) goes through
/// `PortalLabel`, which carries the endpoint identity alongside
/// the owning edge.
#[derive(Clone, Debug)]
pub enum SelectionState {
    None,
    Single(String),
    Multi(Vec<String>),
    Edge(EdgeRef),
    /// One endpoint's portal label on a portal-mode edge. See
    /// [`PortalLabelSel`] for field documentation.
    PortalLabel(PortalLabelSel),
}

impl SelectionState {
    pub fn is_selected(&self, node_id: &str) -> bool {
        match self {
            SelectionState::None => false,
            SelectionState::Single(id) => id == node_id,
            SelectionState::Multi(ids) => ids.contains(&node_id.to_string()),
            SelectionState::Edge(_) => false,
            SelectionState::PortalLabel(_) => false,
        }
    }

    pub fn selected_ids(&self) -> Vec<&str> {
        match self {
            SelectionState::None => vec![],
            SelectionState::Single(id) => vec![id.as_str()],
            SelectionState::Multi(ids) => ids.iter().map(|s| s.as_str()).collect(),
            SelectionState::Edge(_) => vec![],
            SelectionState::PortalLabel(_) => vec![],
        }
    }

    /// Returns the selected edge, if any. A `PortalLabel`
    /// selection does **not** report through this accessor —
    /// portal-label selection is a distinct state and whole-edge
    /// operations (recolor the edge body, etc.) should treat it
    /// as "nothing selected".
    pub fn selected_edge(&self) -> Option<&EdgeRef> {
        match self {
            SelectionState::Edge(e) => Some(e),
            _ => None,
        }
    }

    /// Borrow the inner `PortalLabelSel` for a `PortalLabel`
    /// selection, or `None` for any other variant.
    /// Complements `selected_edge` — the two are mutually
    /// exclusive so at most one returns `Some` for any given
    /// state.
    pub fn selected_portal_label(&self) -> Option<&PortalLabelSel> {
        match self {
            SelectionState::PortalLabel(s) => Some(s),
            _ => None,
        }
    }

    /// A cached `EdgeKey` borrow for the selected portal label,
    /// if any, suitable for building a
    /// [`baumhard::mindmap::scene_builder::SelectedPortalLabel`]
    /// without allocating a fresh key per frame. `EdgeKey` and
    /// `EdgeRef` share the same `(from, to, type)` shape — we
    /// store the key form inside `SelectionState::PortalLabel`
    /// specifically so this borrow path is trivial.
    pub fn selected_portal_label_scene_ref(
        &self,
    ) -> Option<baumhard::mindmap::scene_builder::SelectedPortalLabel<'_>> {
        let PortalLabelSel { edge_key, endpoint_node_id } = match self {
            SelectionState::PortalLabel(s) => s,
            _ => return None,
        };
        Some(baumhard::mindmap::scene_builder::SelectedPortalLabel {
            edge_key,
            endpoint_node_id: endpoint_node_id.as_str(),
        })
    }
}

/// Inner state for [`SelectionState::PortalLabel`]. Stored as a
/// named struct rather than two tuple-variant fields so the
/// `selected_portal_label_scene_ref` accessor can return a single
/// borrow without re-parsing the selection variant.
///
/// **Why `EdgeKey` instead of `EdgeRef`?** Every other selection
/// variant that references an edge uses `EdgeRef` (e.g.
/// `SelectionState::Edge`). `PortalLabel` intentionally deviates:
/// the scene builder's `SelectedPortalLabel<'_>` borrows an
/// `&EdgeKey`, and storing the key form directly lets
/// [`SelectionState::selected_portal_label_scene_ref`] hand out a
/// zero-copy borrow each frame. Converting in the other direction
/// is cheap — [`Self::edge_ref`] rebuilds an `EdgeRef` from the
/// three strings. The asymmetry is a deliberate hot-path trade:
/// per-frame scene builds stay allocation-free; the much rarer
/// document-mutation path pays one conversion.
#[derive(Clone, Debug)]
pub struct PortalLabelSel {
    /// Owning edge — kept as an `EdgeKey` (not `EdgeRef`) so the
    /// scene builder's `SelectedPortalLabel` can borrow it
    /// directly. Callers that need the `EdgeRef` form
    /// reconstruct it via [`PortalLabelSel::edge_ref`].
    pub edge_key: EdgeKey,
    /// Node id the selected marker sits against (identical to the
    /// endpoint id produced by the portal hit test).
    pub endpoint_node_id: String,
}

impl PortalLabelSel {
    /// `EdgeRef` form of the owning edge. Freshly allocated each
    /// call — the document mutation layer uses `EdgeRef` pervasively,
    /// and one conversion per user action is negligible.
    pub fn edge_ref(&self) -> EdgeRef {
        EdgeRef::new(
            self.edge_key.from_id.as_str(),
            self.edge_key.to_id.as_str(),
            self.edge_key.edge_type.as_str(),
        )
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
