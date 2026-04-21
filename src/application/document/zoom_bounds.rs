//! Document-level setters for the zoom-visibility pair
//! (`min_zoom_to_render` / `max_zoom_to_render`) across every
//! authored site: `MindNode`, `MindEdge`, `EdgeLabelConfig`,
//! `PortalEndpointState`.
//!
//! The setters share a common posture: each takes a pair of
//! [`ZoomBoundEdit`] values (one for `min`, one for `max`), so the
//! console — which can receive `zoom min=1.5`,
//! `zoom max=unset`, or both together — maps cleanly onto one
//! atomic call. Each returns `true` when the value actually
//! changed so callers can report "no-op" vs. "changed" without
//! re-reading the model.
//!
//! Validation mirrors the verifier + `ZoomVisibility::try_new`:
//! non-finite bounds and inverted (`min > max`) pairs are
//! rejected as a no-op with `false`. Interactive paths must not
//! panic (`CODE_CONVENTIONS.md` §9), so these setters log a
//! warning and return `false` rather than raising.

use log::warn;

use super::nodes::validate_zoom_pair;
use super::undo_action::UndoAction;
use super::{EdgeRef, MindMapDocument};

/// One console-argument's worth of edit to a single zoom bound.
/// The console parses a keyword argument and folds it into a
/// value of this type:
///
/// - omitted entirely → [`ZoomBoundEdit::Keep`] (leave the side alone)
/// - `min=unset` → [`ZoomBoundEdit::Clear`] (write `None`)
/// - `min=1.5` → [`ZoomBoundEdit::Set`] with the parsed value
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ZoomBoundEdit {
    /// User did not pass this side. Preserve the existing value.
    Keep,
    /// User passed `<side>=unset` / `<side>=`. Write `None`.
    Clear,
    /// User passed `<side>=<value>`. Write `Some(value)`.
    Set(f32),
}

impl ZoomBoundEdit {
    /// Apply this edit to a current `Option<f32>` bound, yielding
    /// the new value. Pure, O(1).
    pub fn apply(self, current: Option<f32>) -> Option<f32> {
        match self {
            ZoomBoundEdit::Keep => current,
            ZoomBoundEdit::Clear => None,
            ZoomBoundEdit::Set(v) => Some(v),
        }
    }

    /// Whether this edit asks for any change at all. Setters
    /// short-circuit when both sides are `Keep`.
    pub fn is_noop(&self) -> bool {
        matches!(self, ZoomBoundEdit::Keep)
    }
}

impl MindMapDocument {
    /// Write the edge's top-level zoom-visibility window. The
    /// full edge is snapshotted into the undo stack via
    /// [`UndoAction::EditEdge`]. Returns `true` when either side
    /// changed. Rejects non-finite or inverted pairs as a
    /// no-op. See [`ZoomBoundEdit`] for per-side semantics.
    pub fn set_edge_zoom_visibility(
        &mut self,
        edge_ref: &EdgeRef,
        min: ZoomBoundEdit,
        max: ZoomBoundEdit,
    ) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let before = self.mindmap.edges[idx].clone();
        let new_min = min.apply(before.min_zoom_to_render);
        let new_max = max.apply(before.max_zoom_to_render);
        if !validate_zoom_pair(new_min, new_max) {
            warn!(
                "set_edge_zoom_visibility: rejected invalid pair min={:?} max={:?}",
                new_min, new_max
            );
            return false;
        }
        if new_min == before.min_zoom_to_render && new_max == before.max_zoom_to_render {
            return false;
        }
        let e = &mut self.mindmap.edges[idx];
        e.min_zoom_to_render = new_min;
        e.max_zoom_to_render = new_max;
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Write the edge's label-level zoom-visibility window. Forks
    /// a fresh `EdgeLabelConfig` on the edge if one wasn't already
    /// present — mirrors how
    /// [`MindMapDocument::set_edge_label_position`] and sibling
    /// label setters handle the config's lazy allocation. Returns
    /// `true` when either side changed.
    pub fn set_edge_label_zoom_visibility(
        &mut self,
        edge_ref: &EdgeRef,
        min: ZoomBoundEdit,
        max: ZoomBoundEdit,
    ) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let before = self.mindmap.edges[idx].clone();
        let (cur_min, cur_max) = before
            .label_config
            .as_ref()
            .map(|c| (c.min_zoom_to_render, c.max_zoom_to_render))
            .unwrap_or((None, None));
        let new_min = min.apply(cur_min);
        let new_max = max.apply(cur_max);
        if !validate_zoom_pair(new_min, new_max) {
            warn!(
                "set_edge_label_zoom_visibility: rejected invalid pair min={:?} max={:?}",
                new_min, new_max
            );
            return false;
        }
        if new_min == cur_min && new_max == cur_max {
            return false;
        }
        let cfg = self.mindmap.edges[idx]
            .label_config
            .get_or_insert_with(Default::default);
        cfg.min_zoom_to_render = new_min;
        cfg.max_zoom_to_render = new_max;
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Write a portal endpoint's zoom-visibility window. Forks a
    /// fresh `PortalEndpointState` on the owning edge if one
    /// wasn't already present. `endpoint_node_id` must equal
    /// either `edge.from_id` (writes `portal_from`) or
    /// `edge.to_id` (writes `portal_to`); any other value returns
    /// `false`.
    pub fn set_portal_endpoint_zoom_visibility(
        &mut self,
        edge_ref: &EdgeRef,
        endpoint_node_id: &str,
        min: ZoomBoundEdit,
        max: ZoomBoundEdit,
    ) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let before = self.mindmap.edges[idx].clone();
        let is_from = endpoint_node_id == before.from_id;
        let is_to = endpoint_node_id == before.to_id;
        if !is_from && !is_to {
            return false;
        }
        let cur = if is_from {
            before.portal_from.as_ref()
        } else {
            before.portal_to.as_ref()
        };
        let (cur_min, cur_max) = cur
            .map(|s| (s.min_zoom_to_render, s.max_zoom_to_render))
            .unwrap_or((None, None));
        let new_min = min.apply(cur_min);
        let new_max = max.apply(cur_max);
        if !validate_zoom_pair(new_min, new_max) {
            warn!(
                "set_portal_endpoint_zoom_visibility: rejected invalid pair min={:?} max={:?}",
                new_min, new_max
            );
            return false;
        }
        if new_min == cur_min && new_max == cur_max {
            return false;
        }
        let e = &mut self.mindmap.edges[idx];
        let slot = if is_from { &mut e.portal_from } else { &mut e.portal_to };
        let state = slot.get_or_insert_with(Default::default);
        state.min_zoom_to_render = new_min;
        state.max_zoom_to_render = new_max;
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::defaults::{
        default_orphan_node, default_parent_child_edge,
    };
    use baumhard::mindmap::model::MindMap;
    use glam::Vec2;
    use std::collections::{HashMap, HashSet};

    fn doc_with_node() -> MindMapDocument {
        let mut doc = MindMapDocument {
            mindmap: MindMap::new_blank("t"),
            file_path: None,
            dirty: false,
            selection: super::super::SelectionState::None,
            undo_stack: Vec::new(),
            mutation_registry: HashMap::new(),
            mutation_sources: HashMap::new(),
            mutation_handlers: HashMap::new(),
            active_toggles: HashSet::new(),
            label_edit_preview: None,
            portal_text_edit_preview: None,
            color_picker_preview: None,
            active_animations: Vec::new(),
        };
        let node = default_orphan_node("0", Vec2::ZERO);
        doc.mindmap.nodes.insert("0".to_string(), node);
        doc
    }

    fn doc_with_edge() -> (MindMapDocument, EdgeRef) {
        let mut doc = doc_with_node();
        doc.mindmap
            .nodes
            .insert("1".to_string(), default_orphan_node("1", Vec2::ZERO));
        let edge = default_parent_child_edge("0", "1");
        let er = EdgeRef::new(&edge.from_id, &edge.to_id, &edge.edge_type);
        doc.mindmap.edges.push(edge);
        (doc, er)
    }

    #[test]
    fn zoom_bound_edit_apply_is_keep_clear_set() {
        assert_eq!(ZoomBoundEdit::Keep.apply(Some(1.0)), Some(1.0));
        assert_eq!(ZoomBoundEdit::Keep.apply(None), None);
        assert_eq!(ZoomBoundEdit::Clear.apply(Some(1.0)), None);
        assert_eq!(ZoomBoundEdit::Set(2.0).apply(Some(1.0)), Some(2.0));
        assert_eq!(ZoomBoundEdit::Set(2.0).apply(None), Some(2.0));
    }

    #[test]
    fn set_node_zoom_sets_both_bounds() {
        let mut doc = doc_with_node();
        let changed = doc.set_node_zoom_visibility(
            "0",
            ZoomBoundEdit::Set(0.5),
            ZoomBoundEdit::Set(2.0),
        );
        assert!(changed);
        let node = doc.mindmap.nodes.get("0").unwrap();
        assert_eq!(node.min_zoom_to_render, Some(0.5));
        assert_eq!(node.max_zoom_to_render, Some(2.0));
    }

    #[test]
    fn set_node_zoom_keep_leaves_value_untouched() {
        let mut doc = doc_with_node();
        // Seed with a bound then only edit the other side.
        doc.set_node_zoom_visibility(
            "0",
            ZoomBoundEdit::Set(0.5),
            ZoomBoundEdit::Set(2.0),
        );
        let changed = doc.set_node_zoom_visibility(
            "0",
            ZoomBoundEdit::Keep,
            ZoomBoundEdit::Set(3.0),
        );
        assert!(changed);
        let node = doc.mindmap.nodes.get("0").unwrap();
        assert_eq!(node.min_zoom_to_render, Some(0.5));
        assert_eq!(node.max_zoom_to_render, Some(3.0));
    }

    #[test]
    fn set_node_zoom_clear_sets_to_none() {
        let mut doc = doc_with_node();
        doc.set_node_zoom_visibility(
            "0",
            ZoomBoundEdit::Set(0.5),
            ZoomBoundEdit::Set(2.0),
        );
        let changed = doc.set_node_zoom_visibility(
            "0",
            ZoomBoundEdit::Clear,
            ZoomBoundEdit::Clear,
        );
        assert!(changed);
        let node = doc.mindmap.nodes.get("0").unwrap();
        assert!(node.min_zoom_to_render.is_none());
        assert!(node.max_zoom_to_render.is_none());
    }

    #[test]
    fn set_node_zoom_rejects_inverted_pair() {
        let mut doc = doc_with_node();
        let changed = doc.set_node_zoom_visibility(
            "0",
            ZoomBoundEdit::Set(3.0),
            ZoomBoundEdit::Set(1.0),
        );
        assert!(!changed, "inverted pair must be rejected as no-op");
        let node = doc.mindmap.nodes.get("0").unwrap();
        assert!(node.min_zoom_to_render.is_none());
        assert!(node.max_zoom_to_render.is_none());
    }

    #[test]
    fn set_node_zoom_rejects_non_finite() {
        let mut doc = doc_with_node();
        assert!(!doc.set_node_zoom_visibility(
            "0",
            ZoomBoundEdit::Set(f32::NAN),
            ZoomBoundEdit::Keep,
        ));
        assert!(!doc.set_node_zoom_visibility(
            "0",
            ZoomBoundEdit::Keep,
            ZoomBoundEdit::Set(f32::INFINITY),
        ));
    }

    #[test]
    fn set_node_zoom_undo_restores_previous_pair() {
        let mut doc = doc_with_node();
        doc.set_node_zoom_visibility(
            "0",
            ZoomBoundEdit::Set(0.5),
            ZoomBoundEdit::Set(2.0),
        );
        doc.set_node_zoom_visibility(
            "0",
            ZoomBoundEdit::Set(1.0),
            ZoomBoundEdit::Set(3.0),
        );
        assert!(doc.undo());
        let node = doc.mindmap.nodes.get("0").unwrap();
        assert_eq!(node.min_zoom_to_render, Some(0.5));
        assert_eq!(node.max_zoom_to_render, Some(2.0));
    }

    #[test]
    fn set_edge_zoom_round_trips_through_undo() {
        let (mut doc, er) = doc_with_edge();
        doc.set_edge_zoom_visibility(
            &er,
            ZoomBoundEdit::Set(0.5),
            ZoomBoundEdit::Set(2.0),
        );
        let before_idx = doc
            .mindmap
            .edges
            .iter()
            .position(|e| er.matches(e))
            .unwrap();
        assert_eq!(doc.mindmap.edges[before_idx].min_zoom_to_render, Some(0.5));
        assert_eq!(doc.mindmap.edges[before_idx].max_zoom_to_render, Some(2.0));
        assert!(doc.undo());
        assert!(doc.mindmap.edges[before_idx].min_zoom_to_render.is_none());
        assert!(doc.mindmap.edges[before_idx].max_zoom_to_render.is_none());
    }

    #[test]
    fn set_edge_label_zoom_forks_label_config() {
        let (mut doc, er) = doc_with_edge();
        assert!(doc.mindmap.edges[0].label_config.is_none());
        let changed = doc.set_edge_label_zoom_visibility(
            &er,
            ZoomBoundEdit::Set(1.5),
            ZoomBoundEdit::Keep,
        );
        assert!(changed);
        let cfg = doc.mindmap.edges[0]
            .label_config
            .as_ref()
            .expect("label_config forked");
        assert_eq!(cfg.min_zoom_to_render, Some(1.5));
        assert!(cfg.max_zoom_to_render.is_none());
    }

    #[test]
    fn set_portal_endpoint_zoom_routes_by_endpoint_node_id() {
        let (mut doc, er) = doc_with_edge();
        assert!(doc.set_portal_endpoint_zoom_visibility(
            &er,
            "0",
            ZoomBoundEdit::Set(1.0),
            ZoomBoundEdit::Set(4.0),
        ));
        let e = &doc.mindmap.edges[0];
        assert!(e.portal_from.as_ref().is_some());
        assert!(e.portal_to.is_none(), "other endpoint untouched");
        let from = e.portal_from.as_ref().unwrap();
        assert_eq!(from.min_zoom_to_render, Some(1.0));
        assert_eq!(from.max_zoom_to_render, Some(4.0));
    }

    #[test]
    fn set_portal_endpoint_zoom_rejects_stranger_node_id() {
        let (mut doc, er) = doc_with_edge();
        assert!(!doc.set_portal_endpoint_zoom_visibility(
            &er,
            "not_an_endpoint",
            ZoomBoundEdit::Set(1.0),
            ZoomBoundEdit::Keep,
        ));
    }

    #[test]
    fn setters_short_circuit_noop_when_all_keep() {
        let mut doc = doc_with_node();
        let changed = doc.set_node_zoom_visibility(
            "0",
            ZoomBoundEdit::Keep,
            ZoomBoundEdit::Keep,
        );
        assert!(!changed);
        assert!(doc.undo_stack.is_empty(), "no-op must not push undo");
    }
}
