// SPDX-License-Identifier: MPL-2.0

//! Document-level data structures: the animation runtime record,
//! the ref types (`EdgeRef`, `ReparentUndoData`), the selection
//! state enum, and the reparent-highlight / selection color
//! constants. Methods on `MindMapDocument` live in the sibling
//! submodules.

use baumhard::mindmap::animation::AnimationTiming;
use baumhard::mindmap::custom_mutation::CustomMutation;
use baumhard::mindmap::model::{MindEdge, MindNode};
use baumhard::mindmap::scene_cache::EdgeKey;

/// Selection highlight color: bright cyan [R, G, B, A]
pub const HIGHLIGHT_COLOR: [f32; 4] = [0.0, 0.9, 1.0, 1.0];

// `InteractionModeOverrides` lives in baumhard
// (`baumhard::mindmap::tree_builder::InteractionModeOverrides`) — its
// home is next to the `SceneSelectionContext` it feeds. Re-exported
// from `document/mod.rs` for the convenient `InteractionModeOverrides::none()`
// path that production callers use.

/// Per-active-mutation runtime record for the animation system.
/// Carries the from/to `MindNode` snapshot and the driving
/// `CustomMutation`; the dispatcher in
/// [`super::MindMapDocument::tick_animations`] interpolates
/// per-frame and writes the blended state back into
/// `mindmap.nodes`.
///
/// `cm` is the source of truth for the mutation list (consumed
/// at completion by `apply_custom_mutation`) and the
/// `mutation_id` (re-trigger dedup key). The stored `cm.timing`
/// is **stripped** to `None` at construction (see
/// `start_animation`) so this struct holds exactly one copy of
/// the timing data, on `self.timing` — the field the per-frame
/// tick reads. The original spec's `cm.timing` remains unchanged;
/// the runtime instance just doesn't carry a redundant snapshot.
#[derive(Debug, Clone)]
pub struct AnimationInstance {
    /// Node id this animation targets.
    pub target_id: String,
    /// Section index when the click that triggered this animation
    /// resolved to a specific section (multi-section node);
    /// `None` for whole-node-targeted triggers. The re-trigger
    /// dedup key in `start_animation` is `(mutation_id,
    /// target_id, section_idx)` so two simultaneous animations
    /// from different sections of the same node with the same
    /// mutation id (e.g. a section-scoped recolor wired to
    /// every section's `OnClick`) coexist instead of coalescing
    /// to one.
    pub section_idx: Option<usize>,
    /// Pre-mutation snapshot of the target node. Stored whole so
    /// any future per-field interpolator can pull the source.
    pub from_node: MindNode,
    /// Post-mutation snapshot of the target node, computed once
    /// at start by applying the mutation to a scratch copy.
    pub to_node: MindNode,
    /// Wall-clock timestamp (ms) when the animation started.
    pub start_ms: u64,
    /// Timing envelope carved from `cm.timing` at construction.
    /// Stored directly (not as `Option`) so the per-frame tick
    /// loop projects without unwrap.
    pub timing: AnimationTiming,
    /// The `CustomMutation` driving the animation. Carries the
    /// id (for re-trigger detection) and the mutation list (for
    /// the `apply_custom_mutation` commit at completion).
    pub cm: CustomMutation,
}

impl AnimationInstance {
    /// `CustomMutation.id` of the mutation being animated.
    /// Combined with `target_id`, identifies the instance for
    /// re-trigger no-op detection in `start_animation`.
    pub fn mutation_id(&self) -> &str {
        &self.cm.id
    }

    /// The timing envelope. Stored directly on the instance —
    /// `start_animation` carves it from `cm.timing` so the
    /// per-frame tick loop never has to unwrap.
    pub fn timing(&self) -> &AnimationTiming {
        &self.timing
    }
}

/// Reparent-mode source color: orange, used for nodes currently being reparented.
pub const REPARENT_SOURCE_COLOR: [f32; 4] = [1.0, 0.55, 0.0, 1.0];

/// Reparent-mode target color: green, used for the node currently hovered as
/// a potential reparent target.
pub const REPARENT_TARGET_COLOR: [f32; 4] = [0.2, 1.0, 0.4, 1.0];

/// Identifies an edge in the MindMap by its endpoints and type. Edges
/// have no stable ID, so this triple is the canonical reference — the
/// same shape `apply_reparent` uses when it looks up parent_child
/// edges for rewrites.
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
        self.from_id == edge.from_id && self.to_id == edge.to_id && self.edge_type == edge.edge_type
    }
}

/// `EdgeRef` → `EdgeKey` (same `(from, to, type)` shape).
impl From<&EdgeRef> for EdgeKey {
    fn from(er: &EdgeRef) -> Self {
        EdgeKey::new(&er.from_id, &er.to_id, &er.edge_type)
    }
}

/// `EdgeKey` → `EdgeRef`.
impl From<&EdgeKey> for EdgeRef {
    fn from(key: &EdgeKey) -> Self {
        EdgeRef::new(&key.from_id, &key.to_id, &key.edge_type)
    }
}

/// Tracks what is currently selected in the document. The
/// variants are mutually exclusive — selecting one kind clears
/// any prior selection of the others, enforced by construction
/// (every write to `document.selection` replaces the whole enum
/// value; there's no additive "add this to the selection" API
/// for variants of different kinds). Downstream code can rely on
/// at most one of `Edge`, `EdgeLabel`, `PortalLabel`, `PortalText`
/// being active at the same moment: the scene builder uses that
/// invariant when it picks which cyan highlight to apply.
///
/// **Four edge-adjacent forms.** An edge carries a body + a
/// label (line mode) or two per-endpoint markers + their text
/// siblings (portal mode). Each of those is selectable on its
/// own so color / font / clipboard routing can target one
/// channel without affecting the others:
///
/// - `Edge(EdgeRef)` — the body (line or the whole portal).
///   Reached through direct edge hit-testing or console.
/// - `EdgeLabel(EdgeLabelSel)` — the text label sitting along
///   a line-mode edge's path. Click + drag moves it; `color`
///   / `font` route to `label_config`.
/// - `PortalLabel(PortalLabelSel)` — a portal mode edge's
///   per-endpoint icon glyph.
/// - `PortalText(PortalLabelSel)` — the per-endpoint text
///   sibling of a portal icon. Shares the same
///   `(edge_key, endpoint_node_id)` identity as its sibling
///   `PortalLabel`; the variant tag is the only difference.
///
/// Portal-mode edges can still be selected through `Edge` (via
/// the console) for whole-edge operations like flipping
/// display mode.
#[derive(Clone, Debug, PartialEq)]
pub enum SelectionState {
    None,
    Single(String),
    Multi(Vec<String>),
    /// One section of one node — emitted when the user clicks on a
    /// section-area in a multi-section node and routes per-section
    /// edits (text, font, color) to that specific section. Single-
    /// section migrated nodes prefer [`Self::Single`] so today's
    /// per-node verbs continue to fire on the whole-node target;
    /// the section variant is the seam that surfaces when richer
    /// authoring needs the discrimination.
    Section(SectionSel),
    /// Two or more sections — possibly across distinct nodes —
    /// each addressed by `(node_id, section_idx)`. Per-section
    /// verbs (`color text=…`, `font size=…`, `font family=…`)
    /// fan out via [`super::super::console::traits::selection_targets`]
    /// and apply to every section in the set. Resize and move
    /// gestures stay single-target — a `MultiSection` selection
    /// emits no resize handles, and threshold-cross promotion
    /// from a multi-section press collapses to one entry. The
    /// invariant `len() >= 2` is upheld by [`Self::from_sections`];
    /// callers that may produce 0 / 1 entries should route
    /// through that constructor.
    MultiSection(Vec<SectionSel>),
    /// `range` is a `(lo, hi)` pair of **section indices** on
    /// the owning node, `[lo, hi]` inclusive. Fanned out by
    /// `border` / `style` verbs through
    /// `border.rs::expand_section_pairs` and
    /// `cross_dispatch/style.rs::commit_border_preview`. Clamped
    /// by `cleanup_after_structural_mutation` on add/delete/split
    /// against `sections.len()`. Per-section verbs see the
    /// range and route to the range-aware setter; accessors that
    /// only care about the owning section (`selected_section`,
    /// `selected_ids`, `is_selected`) treat it identically to
    /// [`Self::Section`].
    SectionRange { sel: SectionSel, range: (usize, usize) },
    Edge(EdgeRef),
    /// Line-mode label selection: the edge's text label sits
    /// along the connection path and is selected independently
    /// from the edge body. See [`EdgeLabelSel`].
    EdgeLabel(EdgeLabelSel),
    /// One endpoint's portal **icon** glyph on a portal-mode edge.
    /// See [`PortalLabelSel`] for field documentation.
    PortalLabel(PortalLabelSel),
    /// One endpoint's portal **text** label — the glyph area
    /// sitting alongside a portal icon. Reuses [`PortalLabelSel`]
    /// because the identity (`edge_key`, `endpoint_node_id`) is
    /// identical to the icon; only the selection target differs.
    PortalText(PortalLabelSel),
}

/// Identity of a single
/// [`MindSection`](baumhard::mindmap::model::MindSection) in the
/// document — the owning MindNode id plus the section's index in
/// `MindNode.sections`. Stable across scene rebuilds for unchanged
/// nodes; same identity shape every per-section interaction
/// (selection, hit-test, scene rebuild keys) speaks.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SectionSel {
    pub node_id: String,
    pub section_idx: usize,
}

impl SectionSel {
    /// Construct a section selection from owned strings + index.
    /// Mirrors [`EdgeRef::new`] — same idiomatic shape across
    /// every selection-bearing identity in the document layer.
    pub fn new(node_id: impl Into<String>, section_idx: usize) -> Self {
        SectionSel {
            node_id: node_id.into(),
            section_idx,
        }
    }
}

impl SelectionState {
    /// Build a node selection from a flat list of IDs: empty becomes
    /// [`SelectionState::None`], a single element becomes [`Single`],
    /// anything longer becomes [`Multi`]. Shared by the three call
    /// sites (single click, drag-select preview, drag-select commit)
    /// that collapse a hit-set into a selection state — keeps the
    /// empty-vs-single-vs-multi split in one place so they cannot
    /// drift. Interactive-path safe: never panics on any input length
    /// (§9).
    ///
    /// [`Single`]: SelectionState::Single
    /// [`Multi`]: SelectionState::Multi
    pub fn from_ids(ids: Vec<String>) -> Self {
        let mut iter = ids.into_iter();
        match iter.next() {
            None => SelectionState::None,
            Some(first) => match iter.next() {
                None => SelectionState::Single(first),
                Some(second) => {
                    let mut all = Vec::with_capacity(2 + iter.size_hint().0);
                    all.push(first);
                    all.push(second);
                    all.extend(iter);
                    SelectionState::Multi(all)
                }
            },
        }
    }

    /// Build a section-set selection from a flat list of
    /// `SectionSel`, deduplicating by `(node_id, section_idx)`
    /// in first-seen order. Empty becomes
    /// [`SelectionState::None`], a single entry becomes
    /// [`Self::Section`], anything longer becomes
    /// [`Self::MultiSection`]. Keeps two invariants in one
    /// place: `MultiSection.len() >= 2` and "every entry is
    /// unique" — downstream consumers (`selection_targets`
    /// fan-out, highlight pipeline, font / color fan-out)
    /// implicitly assume uniqueness; duplicates would inflate
    /// fan-out counts and potentially write the same setter
    /// twice on the same section.
    ///
    /// Cost: O(n) with a transient `HashSet` of (node_id ref,
    /// section_idx) — bounded by the input length, which is
    /// in turn bounded by user authoring (typically ≤ 10).
    pub fn from_sections(secs: Vec<SectionSel>) -> Self {
        let mut seen: std::collections::HashSet<(String, usize)> =
            std::collections::HashSet::with_capacity(secs.len());
        let mut deduped: Vec<SectionSel> = Vec::with_capacity(secs.len());
        for s in secs {
            let key = (s.node_id.clone(), s.section_idx);
            if seen.insert(key) {
                deduped.push(s);
            }
        }
        let mut iter = deduped.into_iter();
        match iter.next() {
            None => SelectionState::None,
            Some(first) => match iter.next() {
                None => SelectionState::Section(first),
                Some(second) => {
                    let mut all = Vec::with_capacity(2 + iter.size_hint().0);
                    all.push(first);
                    all.push(second);
                    all.extend(iter);
                    SelectionState::MultiSection(all)
                }
            },
        }
    }

    pub fn is_selected(&self, node_id: &str) -> bool {
        match self {
            SelectionState::None => false,
            SelectionState::Single(id) => id == node_id,
            SelectionState::Multi(ids) => ids.contains(&node_id.to_string()),
            SelectionState::Section(s) => s.node_id == node_id,
            SelectionState::MultiSection(secs) => secs.iter().any(|s| s.node_id == node_id),
            SelectionState::SectionRange { sel, .. } => sel.node_id == node_id,
            SelectionState::Edge(_)
            | SelectionState::EdgeLabel(_)
            | SelectionState::PortalLabel(_)
            | SelectionState::PortalText(_) => false,
        }
    }

    pub fn selected_ids(&self) -> Vec<&str> {
        match self {
            SelectionState::None => vec![],
            SelectionState::Single(id) => vec![id.as_str()],
            SelectionState::Multi(ids) => ids.iter().map(|s| s.as_str()).collect(),
            SelectionState::Section(s) => vec![s.node_id.as_str()],
            SelectionState::MultiSection(secs) => {
                let mut seen = std::collections::HashSet::new();
                let mut out = Vec::with_capacity(secs.len());
                for s in secs {
                    if seen.insert(s.node_id.as_str()) {
                        out.push(s.node_id.as_str());
                    }
                }
                out
            }
            SelectionState::SectionRange { sel, .. } => vec![sel.node_id.as_str()],
            SelectionState::Edge(_)
            | SelectionState::EdgeLabel(_)
            | SelectionState::PortalLabel(_)
            | SelectionState::PortalText(_) => vec![],
        }
    }

    /// Borrow the inner [`SectionSel`] for a section-bearing
    /// selection ([`Self::Section`] or [`Self::SectionRange`]),
    /// or `None` for any other variant — *including*
    /// [`Self::MultiSection`]. Verbs that need a single section
    /// target consult this; verbs that fan out over every
    /// selected section route through `selection_targets`.
    pub fn selected_section(&self) -> Option<&SectionSel> {
        match self {
            SelectionState::Section(s) => Some(s),
            SelectionState::SectionRange { sel, .. } => Some(sel),
            _ => None,
        }
    }

    /// Borrow every selected section as a slice. Returns a
    /// single-element slice for [`Self::Section`] /
    /// [`Self::SectionRange`], the inner vec for
    /// [`Self::MultiSection`], and an empty slice for every
    /// other variant. Used by `selection_targets` to fan out
    /// per-section verbs.
    pub fn selected_sections(&self) -> &[SectionSel] {
        match self {
            SelectionState::Section(s) => std::slice::from_ref(s),
            SelectionState::MultiSection(secs) => secs.as_slice(),
            SelectionState::SectionRange { sel, .. } => std::slice::from_ref(sel),
            _ => &[],
        }
    }

    /// Owning-node id when the selection points at exactly one
    /// node — `Single`, `Section`, or `SectionRange`. `None` for
    /// `Multi`, `MultiSection`, edge / portal variants, and
    /// `None`. Per-section verbs that need a single owning node
    /// call this rather than re-implementing the match.
    pub fn primary_node_id(&self) -> Option<&str> {
        match self {
            SelectionState::Single(id) => Some(id),
            SelectionState::Section(s) => Some(&s.node_id),
            SelectionState::SectionRange { sel, .. } => Some(&sel.node_id),
            _ => None,
        }
    }

    /// Sub-range carried by [`Self::SectionRange`], or `None`
    /// for every other variant. Per-section verbs route
    /// range-targeted setters through this when the user has
    /// shift-selected a sub-range inside a section.
    pub fn selected_range(&self) -> Option<(usize, usize)> {
        match self {
            SelectionState::SectionRange { range, .. } => Some(*range),
            _ => None,
        }
    }

    /// Owning-node ids for every selected target, deduplicated
    /// by `node_id` in first-seen order, returned as `Vec<String>`
    /// for callers that need owned strings (most node-fanout
    /// verb arms — `border` / `zoom` / `topology` Delete).
    /// `Multi` and `MultiSection` both dedup correctly:
    /// `Multi(["a", "a", "b"])` → `["a", "b"]`,
    /// `MultiSection([a/0, a/1, b/0])` → `["a", "b"]`. Other
    /// variants return their natural single owner (or empty).
    pub fn dedup_owning_node_ids(&self) -> Vec<String> {
        let ids = self.selected_ids();
        let mut seen: std::collections::HashSet<&str> =
            std::collections::HashSet::with_capacity(ids.len());
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if seen.insert(id) {
                out.push(id.to_string());
            }
        }
        out
    }

    /// Returns the selected edge, if any. The other edge-adjacent
    /// variants (`EdgeLabel`, `PortalLabel`, `PortalText`) do **not**
    /// report through this accessor — each is its own distinct
    /// state, and whole-edge operations (recolor the edge body,
    /// flip display mode) should treat them as "nothing (whole-edge)
    /// selected". Pair with [`Self::selected_edge_or_portal_edge`]
    /// for the wider "any edge-adjacent selection → owning edge"
    /// collapser.
    pub fn selected_edge(&self) -> Option<&EdgeRef> {
        match self {
            SelectionState::Edge(e) => Some(e),
            _ => None,
        }
    }

    /// Borrow the inner `EdgeLabelSel` for an `EdgeLabel`
    /// selection, or `None` for any other variant. Mirrors
    /// [`Self::selected_portal_label`] — label-target operations
    /// that specifically want the label (not the edge body)
    /// consult this accessor.
    pub fn selected_edge_label(&self) -> Option<&EdgeLabelSel> {
        match self {
            SelectionState::EdgeLabel(s) => Some(s),
            _ => None,
        }
    }

    /// Borrow the inner `PortalLabelSel` for a `PortalLabel`
    /// selection (the portal **icon**), or `None` for any other
    /// variant. Complements `selected_edge` — the variants are
    /// mutually exclusive so at most one returns `Some` for any
    /// given state.
    pub fn selected_portal_label(&self) -> Option<&PortalLabelSel> {
        match self {
            SelectionState::PortalLabel(s) => Some(s),
            _ => None,
        }
    }

    /// Borrow the inner `PortalLabelSel` for a `PortalText`
    /// selection (the portal **text** sibling of an icon), or
    /// `None` for any other variant. Distinct from
    /// [`Self::selected_portal_label`]: both use `PortalLabelSel`
    /// as the identity-carrying struct, but only one of them is
    /// active at any moment (they're separate variants).
    pub fn selected_portal_text(&self) -> Option<&PortalLabelSel> {
        match self {
            SelectionState::PortalText(s) => Some(s),
            _ => None,
        }
    }

    /// Return the owning `EdgeRef` for any edge-adjacent
    /// selection — `Edge`, `EdgeLabel`, `PortalLabel`, or
    /// `PortalText` — collapsing all four into "which edge is
    /// the user pointing at". Used by console predicates + the
    /// `edge` command so commands targeting the edge as a whole
    /// (type change, display mode flip, reset) keep working after
    /// a user clicks any sub-part of an edge. Pair with
    /// [`Self::selected_edge`] for the narrower "whole-edge, not
    /// a label or text or icon" form on paths that need to
    /// disambiguate.
    pub fn selected_edge_or_portal_edge(&self) -> Option<EdgeRef> {
        match self {
            SelectionState::Edge(e) => Some(e.clone()),
            SelectionState::EdgeLabel(s) => Some(s.edge_ref.clone()),
            SelectionState::PortalLabel(s) | SelectionState::PortalText(s) => Some(s.edge_ref()),
            _ => None,
        }
    }

    /// Borrow the `PortalLabelSel` for either a `PortalLabel`
    /// (icon) or `PortalText` (sibling text) selection — the
    /// "user pointed at this portal endpoint" form, collapsing
    /// the two portal sub-selections. Returns `None` for any
    /// non-portal selection. Useful for operations that target
    /// the whole portal endpoint (e.g. the `body glyph=` console
    /// verb applies to the icon regardless of whether the user
    /// clicked icon or text).
    pub fn selected_portal_endpoint(&self) -> Option<&PortalLabelSel> {
        match self {
            SelectionState::PortalLabel(s) | SelectionState::PortalText(s) => Some(s),
            _ => None,
        }
    }

    /// A cached `EdgeKey` borrow for the selected portal label,
    /// if any, suitable for building a
    /// [`baumhard::mindmap::tree_builder::SelectedPortalLabel`]
    /// without allocating a fresh key per frame. `EdgeKey` and
    /// `EdgeRef` share the same `(from, to, type)` shape — we
    /// store the key form inside `SelectionState::PortalLabel`
    /// specifically so this borrow path is trivial. `PortalText`
    /// selections also resolve to the same scene ref so the
    /// highlight cascade treats icon and text as one endpoint
    /// target on the selection path; divergent highlight
    /// behavior (icon vs. text) is a follow-up refinement.
    pub fn selected_portal_label_scene_ref(
        &self,
    ) -> Option<baumhard::mindmap::tree_builder::SelectedPortalLabel<'_>> {
        let PortalLabelSel {
            edge_key,
            endpoint_node_id,
        } = match self {
            SelectionState::PortalLabel(s) | SelectionState::PortalText(s) => s,
            _ => return None,
        };
        Some(baumhard::mindmap::tree_builder::SelectedPortalLabel {
            edge_key,
            endpoint_node_id: endpoint_node_id.as_str(),
        })
    }
}

/// Inner state for [`SelectionState::EdgeLabel`]. A newtype
/// wrapper around [`EdgeRef`] — kept as a named struct rather
/// than a tuple variant so the accessor returns a single borrow
/// with a stable type, matching the shape of [`PortalLabelSel`]
/// (mirror pattern for the four edge-adjacent selection forms).
///
/// The inner `edge_ref` points at the edge whose
/// `label_config` the selection will target for color / font /
/// drag operations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EdgeLabelSel {
    /// Owning edge — identifies which edge's `label_config`
    /// this selection targets.
    pub edge_ref: EdgeRef,
}

impl EdgeLabelSel {
    /// Construct an `EdgeLabelSel` for the given edge.
    pub fn new(edge_ref: EdgeRef) -> Self {
        Self { edge_ref }
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
#[derive(Clone, Debug, PartialEq, Eq)]
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `EdgeKey` and `EdgeRef` are structural twins; the two
    /// `From` impls round-trip cleanly for all three fields.
    #[test]
    fn edge_ref_to_edge_key_round_trips() {
        let er = EdgeRef::new("alpha", "beta", "cross_link");
        let key = EdgeKey::from(&er);
        let er2 = EdgeRef::from(&key);
        assert_eq!(er, er2);
        assert_eq!(key.from_id, "alpha");
        assert_eq!(key.to_id, "beta");
        assert_eq!(key.edge_type, "cross_link");
    }

    /// Conversion preserves the field values across the
    /// `String` boundary (no truncation, no normalization).
    #[test]
    fn edge_ref_edge_key_conversion_preserves_strings() {
        let er = EdgeRef::new("a-with-dashes-and-1.0.2", "b/with/slashes", "parent_child");
        let key = EdgeKey::from(&er);
        assert_eq!(key.from_id, er.from_id);
        assert_eq!(key.to_id, er.to_id);
        assert_eq!(key.edge_type, er.edge_type);
    }
}
