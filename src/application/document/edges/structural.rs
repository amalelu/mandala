// SPDX-License-Identifier: MPL-2.0

//! Edge structural mutations — hit-testing handles, position resets,
//! anchor/curve toggles, edge-index lookup. Houses the shared internal
//! helpers (`mutate_edge`, `commit_throttled_edge_drag`,
//! `ensure_glyph_connection`) that every per-axis style setter routes
//! through.

use glam::Vec2;

use baumhard::mindmap::model::{
    portal_endpoint_state_mut, Canvas, EdgeLabelConfig, GlyphConnectionConfig, MindEdge, PortalEndpointState,
};
use baumhard::mindmap::tree_builder;

use super::super::types::EdgeRef;
use super::super::undo_action::UndoAction;
use super::super::MindMapDocument;
use super::closure_helpers::ensure_glyph_connection_inline;

impl MindMapDocument {
    pub fn remove_edge(&mut self, edge_ref: &EdgeRef) -> Option<(usize, MindEdge)> {
        let idx = self.mindmap.edges.iter().position(|e| edge_ref.matches(e))?;
        let edge = self.mindmap.edges.remove(idx);
        Some((idx, edge))
    }

    /// Hit-test the grab-handles of a specific edge at `canvas_pos`.
    /// Returns the closest handle whose canvas-space position is
    /// within `tolerance` of the cursor, or `None` if nothing is in
    /// range. Called at mouse-down time by the edge-reshape drag flow
    /// when an edge is currently selected.
    ///
    /// Computed from the live edge (so any in-progress drag is
    /// reflected), without consulting the scene cache. Bounded cost:
    /// one `build_connection_path` + up to five distance comparisons.
    pub fn hit_test_edge_handle(
        &self,
        canvas_pos: Vec2,
        edge_ref: &EdgeRef,
        tolerance: f32,
    ) -> Option<(tree_builder::EdgeHandleKind, Vec2)> {
        let edge = self.mindmap.edges.iter().find(|e| edge_ref.matches(e))?;
        let from_node = self.mindmap.nodes.get(&edge.from_id)?;
        let to_node = self.mindmap.nodes.get(&edge.to_id)?;
        let from_pos = from_node.pos_vec2();
        let from_size = from_node.size_vec2();
        let to_pos = to_node.pos_vec2();
        let to_size = to_node.size_vec2();

        let edge_key = baumhard::mindmap::scene_cache::EdgeKey::from_edge(edge);
        let handles =
            tree_builder::build_edge_handles(edge, &edge_key, from_pos, from_size, to_pos, to_size);

        let mut best: Option<(tree_builder::EdgeHandleKind, Vec2, f32)> = None;
        for h in handles {
            let pos = Vec2::new(h.position.0, h.position.1);
            let dist = canvas_pos.distance(pos);
            if dist > tolerance {
                continue;
            }
            if best.as_ref().is_none_or(|(_, _, d)| dist < *d) {
                best = Some((h.kind, pos, dist));
            }
        }
        best.map(|(k, p, _)| (k, p))
    }

    /// Clear every position override the user can author on an
    /// edge, routing through one `UndoAction::EditEdge` per applied
    /// reset. The scope depends on the edge's display mode and on
    /// `endpoint`:
    ///
    /// - **Line-mode edge, `endpoint = None`.** Reset `anchor_from`
    ///   and `anchor_to` to `"auto"` (which lets the renderer pick
    ///   the closest border side per frame as either node moves)
    ///   and clear the label's `position_t` and
    ///   `perpendicular_offset`. Curve control points (handled by
    ///   `reset_edge_to_straight`) are deliberately left alone —
    ///   the user asked for "re-attach the edge to both nodes",
    ///   not "straighten it out".
    /// - **Portal-mode edge, `endpoint = None`.** Clear `border_t`
    ///   and `perpendicular_offset` on both `portal_from` and
    ///   `portal_to`. Either endpoint state that becomes all-
    ///   default after the clear is pruned back to `None` so the
    ///   serialized JSON stays minimal.
    /// - **Portal-mode edge, `endpoint = Some("<node_id>")`.**
    ///   Clear `border_t` and `perpendicular_offset` on just the
    ///   named endpoint. `endpoint` must match one of
    ///   `edge.from_id` / `edge.to_id` — any other value is a
    ///   no-op (the console layer ensures the id came from a
    ///   portal-label hit-test, so this guard is defensive, not
    ///   a user-visible behavior).
    /// - **Line-mode edge, `endpoint = Some(_)`.** Identical to
    ///   the `endpoint = None` line-mode case above — line-mode
    ///   edges have no per-endpoint state to reset, so the
    ///   `endpoint` argument is ignored. The console layer only
    ///   passes a `Some(endpoint)` for portal-label / portal-text
    ///   selections, which route to the portal branch above; the
    ///   line-mode-with-endpoint combination isn't a user-reachable
    ///   path today. Handled permissively rather than with a guard
    ///   so a future selection-kind addition doesn't accidentally
    ///   crash the interactive path.
    ///
    /// Returns `true` when at least one field actually changed;
    /// `false` when every target field was already at its default
    /// (the "nothing to reset" case — lets the console verb print
    /// a helpful "already at default" message instead of a dead
    /// undo entry).
    pub fn reset_edge_position(&mut self, edge_ref: &EdgeRef, endpoint: Option<&str>) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let before = self.mindmap.edges[idx].clone();
        let is_portal = baumhard::mindmap::model::is_portal_edge(&self.mindmap.edges[idx]);
        let mut changed = false;

        if is_portal {
            // Decide which endpoints to reset: the named one only,
            // or both. Collect owned ids up-front so the mutable
            // borrow on `edges[idx]` below doesn't collide.
            let edge = &self.mindmap.edges[idx];
            let targets: Vec<String> = match endpoint {
                Some(id) if id == edge.from_id || id == edge.to_id => {
                    vec![id.to_string()]
                }
                Some(_) => Vec::new(),
                None => vec![edge.from_id.clone(), edge.to_id.clone()],
            };
            for target in &targets {
                let slot = match portal_endpoint_state_mut(&mut self.mindmap.edges[idx], target) {
                    Some(s) => s,
                    None => continue,
                };
                if let Some(existing) = slot.as_mut() {
                    if existing.border_t.is_some() || existing.perpendicular_offset.is_some() {
                        existing.border_t = None;
                        existing.perpendicular_offset = None;
                        changed = true;
                        if existing == &PortalEndpointState::default() {
                            *slot = None;
                        }
                    }
                }
            }
        } else {
            // Line mode: reset anchors to "auto" and clear the
            // label's position overrides. Endpoint argument is
            // ignored for line-mode edges (no per-endpoint state).
            let edge = &mut self.mindmap.edges[idx];
            const AUTO: &str = "auto";
            if edge.anchor_from != AUTO {
                edge.anchor_from = AUTO.to_string();
                changed = true;
            }
            if edge.anchor_to != AUTO {
                edge.anchor_to = AUTO.to_string();
                changed = true;
            }
            if let Some(cfg) = edge.label_config.as_mut() {
                if cfg.position_t.is_some() || cfg.perpendicular_offset.is_some() {
                    cfg.position_t = None;
                    cfg.perpendicular_offset = None;
                    changed = true;
                    if cfg == &EdgeLabelConfig::default() {
                        edge.label_config = None;
                    }
                }
            }
        }

        if !changed {
            return false;
        }
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Clear an edge's `control_points` so it renders as a straight
    /// line. Returns `true` if the edge existed and had control
    /// points to clear; `false` if the edge was already straight or
    /// wasn't found. On success, a full snapshot of the pre-edit
    /// edge is pushed onto `undo_stack` as `UndoAction::EditEdge` and
    /// `dirty` is set. No-op for already-straight edges so repeated
    /// palette invocations don't pollute the undo stack.
    pub fn reset_edge_to_straight(&mut self, edge_ref: &EdgeRef) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        if self.mindmap.edges[idx].control_points.is_empty() {
            return false;
        }
        let before = self.mindmap.edges[idx].clone();
        self.mindmap.edges[idx].control_points.clear();
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Insert a single control point on `edge_ref` so a straight
    /// edge curves into a gentle quadratic Bezier. No-op if the
    /// edge is already curved (has ≥ 1 control point) so
    /// re-invocation from the console doesn't keep deforming the
    /// curve. Returns `true` on success, pushes `EditEdge` to the
    /// undo stack, sets `dirty`.
    ///
    /// The inserted control point sits at the midpoint of the
    /// current anchor line, pushed perpendicular to the line by
    /// a quarter of its length — the same cosmetic default the
    /// midpoint-handle drag produces on its first idle frame, so
    /// the keyboard path and the mouse path both land on a
    /// visually identical starting curve. The offset is stored as
    /// a relative vector from the source node's center (matching
    /// the `control_points[0]` encoding the scene builder expects).
    pub fn curve_straight_edge(&mut self, edge_ref: &EdgeRef) -> bool {
        use baumhard::mindmap::connection;
        use baumhard::mindmap::model::ControlPoint;
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        if !self.mindmap.edges[idx].control_points.is_empty() {
            return false;
        }
        // Resolve the actual path endpoints so the curve bulges
        // out relative to the rendered straight line, not the
        // (center-to-center) raw vector between nodes.
        let edge = &self.mindmap.edges[idx];
        let from_node = match self.mindmap.nodes.get(&edge.from_id) {
            Some(n) => n,
            None => return false,
        };
        let to_node = match self.mindmap.nodes.get(&edge.to_id) {
            Some(n) => n,
            None => return false,
        };
        let from_pos = from_node.pos_vec2();
        let from_size = from_node.size_vec2();
        let to_pos = to_node.pos_vec2();
        let to_size = to_node.size_vec2();
        let path = connection::build_connection_path(
            from_pos,
            from_size,
            &edge.anchor_from,
            to_pos,
            to_size,
            &edge.anchor_to,
            &[],
        );
        let (start, end) = match &path {
            connection::ConnectionPath::Straight { start, end } => (*start, *end),
            // Defensive branch — we guarded `control_points.is_empty()`
            // above, so this path builder should always return a
            // straight segment. If a future change makes that not
            // hold, bail rather than insert garbage.
            _ => return false,
        };
        // Zero-length guard — coincident endpoints produce a
        // degenerate normal (`Vec2::X` from the tangent fallback)
        // which would push the CP sideways instead of along a real
        // perpendicular. Bail so the edge stays straight.
        let length = (end - start).length();
        if length < f32::EPSILON {
            return false;
        }
        let mid = start.lerp(end, 0.5);
        // Reuse `connection::normal_at_t` rather than hand-rolling
        // the rotation — one source of truth for every path-normal
        // computation, and the helper's Y-down orientation note
        // applies here too. Quarter-length nudge reads as a gentle
        // curve without looking like a bug.
        let normal = connection::normal_at_t(&path, 0.5);
        let control_point_canvas = mid + normal * (length * 0.25);
        let from_center = Vec2::new(from_pos.x + from_size.x * 0.5, from_pos.y + from_size.y * 0.5);
        let offset = control_point_canvas - from_center;

        let before = self.mindmap.edges[idx].clone();
        self.mindmap.edges[idx].control_points.push(ControlPoint {
            x: offset.x as f64,
            y: offset.y as f64,
        });
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Set an edge's `anchor_from` (when `is_from == true`) or
    /// `anchor_to` (when `is_from == false`) to `value`. Valid values
    /// are 0 (auto) or 1..=4 (top/right/bottom/left). Returns `true`
    /// if the value changed, pushing an `EditEdge` undo snapshot and
    /// setting `dirty`. Returns `false` if the edge was not found or
    /// the anchor was already at the requested value.
    pub fn set_edge_anchor(&mut self, edge_ref: &EdgeRef, is_from: bool, value: &str) -> bool {
        self.mutate_edge(edge_ref, |edge, _canvas| {
            let slot = if is_from {
                &mut edge.anchor_from
            } else {
                &mut edge.anchor_to
            };
            if slot == value {
                return false;
            }
            *slot = value.to_string();
            true
        })
    }

    /// Look up the index of an edge in `mindmap.edges` matching the
    /// given `EdgeRef`. Returned for callers that need to snapshot
    /// the edge before mutating it in place (e.g. the edge-handle
    /// drag flow in `app.rs`).
    pub fn edge_index(&self, edge_ref: &EdgeRef) -> Option<usize> {
        self.mindmap.edges.iter().position(|e| edge_ref.matches(e))
    }

    // ========================================================================
    // Connection style and label mutation helpers.
    //
    // Every helper in this block mirrors the `reset_edge_to_straight` /
    // `set_edge_anchor` template exactly:
    //
    //   1. Locate the edge index via `edge_ref.matches`.
    //   2. Early-return `false` for no-op cases (value already matches, edge
    //      not found) so repeated palette invocations don't pollute the undo
    //      stack.
    //   3. Clone the full pre-edit edge into `before` — this must happen
    //      BEFORE any fork via `ensure_glyph_connection`, so undo restores
    //      the pre-fork `None` cleanly.
    //   4. Mutate the edge in place.
    //   5. Push `UndoAction::EditEdge { index, before }` and set `dirty`.
    //
    // The fork semantic: on the first style edit of an edge whose
    // `glyph_connection` is None, we materialize a concrete per-edge copy
    // from the effective resolved config (canvas default, else hardcoded
    // default). Subsequent canvas-default changes don't retroactively apply
    // to forked edges — mirroring how CSS "computed style" copies work.
    // ========================================================================

    /// Ensure `edge.glyph_connection` is `Some(_)`, forking from the
    /// canvas default (or the hardcoded default) on first edit. Returns
    /// a mutable reference to the freshly-installed or previously-set
    /// config so the caller can mutate a specific field.
    ///
    /// Must be called AFTER the `before` snapshot has been cloned so
    /// the undo entry still carries the pre-fork `None`.
    pub(super) fn ensure_glyph_connection<'a>(
        edge: &'a mut MindEdge,
        canvas: &Canvas,
    ) -> &'a mut GlyphConnectionConfig {
        ensure_glyph_connection_inline(edge, canvas)
    }

    /// Run `mutate` against the edge selected by `edge_ref` with the
    /// before-snapshot, rollback-on-no-op, and `EditEdge` undo-push
    /// scaffolding handled here. The closure returns `true` to commit
    /// (push undo + mark dirty) or `false` to rollback the edit
    /// (any in-place mutations on `edge` and any `glyph_connection`
    /// fork are reverted from the snapshot).
    ///
    /// Single source of truth for the "find idx → clone before →
    /// mutate → push undo" template documented in this module's
    /// "Connection style and label mutation helpers" comment block.
    /// Each remaining bespoke setter that touches a single field on
    /// a MindEdge collapses to a body that returns `true`/`false`
    /// from a closure, with no responsibility for the surrounding
    /// undo discipline.
    ///
    /// Why a method rather than a free fn: the closure receives
    /// `&mut MindEdge` and `&Canvas` (sibling fields of `MindMap`),
    /// which Rust's split-borrow rules allow under a single
    /// `&mut self` because both are direct field projections of
    /// `self.mindmap`. The closure can reach
    /// [`Self::ensure_glyph_connection`] through the free
    /// [`super::closure_helpers::ensure_glyph_connection_inline`]
    /// without having to re-fetch `&mut self`.
    pub(in crate::application::document) fn mutate_edge<F>(&mut self, edge_ref: &EdgeRef, mutate: F) -> bool
    where
        F: FnOnce(&mut MindEdge, &Canvas) -> bool,
    {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let before = self.mindmap.edges[idx].clone();
        let changed = mutate(&mut self.mindmap.edges[idx], &self.mindmap.canvas);
        if !changed {
            self.mindmap.edges[idx] = before;
            return false;
        }
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Commit the post-drag state of an edge by pushing a single
    /// [`UndoAction::EditEdge`] entry carrying the pre-drag
    /// snapshot, gated by `changed`. The drag threshold path
    /// (`EdgeHandle`) passes `|_, _| true` because crossing the
    /// threshold guarantees a state change; the per-frame paths
    /// (`PortalLabel` inspecting `portal_from`/`portal_to`,
    /// `EdgeLabel` inspecting `label_config`) pass per-variant
    /// predicates so an at-rest release that touches no field
    /// drops no undo entry.
    ///
    /// No-op when the edge no longer exists in `mindmap.edges`
    /// (the throttled drag's snapshot can outlive the edge if a
    /// concurrent delete removed it). Mirrors the drag-release
    /// arms in `event_mouse_click.rs::handle_mouse_input`,
    /// previously open-coded with byte-identical
    /// `if let Some(idx) = doc.edge_index(&edge_ref) { ... push +
    /// dirty ... }` scaffolding.
    pub(crate) fn commit_throttled_edge_drag<F>(&mut self, edge_ref: &EdgeRef, original: MindEdge, changed: F)
    where
        F: FnOnce(&MindEdge, &MindEdge) -> bool,
    {
        let idx = match self.edge_index(edge_ref) {
            Some(i) => i,
            None => return,
        };
        let current = &self.mindmap.edges[idx];
        if changed(current, &original) {
            self.undo_stack.push(UndoAction::EditEdge {
                index: idx,
                before: original,
            });
            self.dirty = true;
        }
    }
}

#[cfg(test)]
mod tests {
    //! Tests for the shared internal helpers `mutate_edge` and
    //! `commit_throttled_edge_drag`. Free-fn helpers
    //! (`ensure_glyph_connection_inline` etc.) are tested in
    //! `closure_helpers.rs::tests` next to their definitions.
    use super::super::closure_helpers::ensure_glyph_connection_inline;
    use super::*;
    use crate::application::document::tests_common::doc_with_one_edge as doc_with_edge;

    /// `mutate_edge` returning `false` from the closure rolls
    /// back any in-place fork via `ensure_glyph_connection_inline`
    /// — the live model returns to the pre-closure state, no
    /// `EditEdge` undo entry is pushed, and `dirty` stays where
    /// it was. Critical for the no-op contract every per-field
    /// setter (`set_edge_color`, `set_edge_cap_start`, ...) routes
    /// through.
    #[test]
    fn mutate_edge_rolls_back_fork_on_false_return() {
        let (mut doc, er) = doc_with_edge();
        // Pre-condition: this edge has no glyph_connection.
        assert!(doc.mindmap.edges[0].glyph_connection.is_none());
        doc.dirty = false;

        let returned = doc.mutate_edge(&er, |edge, canvas| {
            // Force a fork — but then return false.
            let _cfg = ensure_glyph_connection_inline(edge, canvas);
            false
        });

        assert!(!returned);
        // Fork must have rolled back.
        assert!(
            doc.mindmap.edges[0].glyph_connection.is_none(),
            "the fork survived a false-return rollback"
        );
        assert!(doc.undo_stack.is_empty(), "no undo entry on false-return");
        assert!(!doc.dirty, "dirty stays unset on false-return");
    }

    /// On a `true` return, `mutate_edge` pushes one `EditEdge`
    /// undo entry carrying the *pre-fork* state, sets `dirty`,
    /// and leaves the closure's mutations in place. Locks the
    /// other half of the contract.
    #[test]
    fn mutate_edge_pushes_undo_with_pre_fork_state_on_true() {
        let (mut doc, er) = doc_with_edge();
        assert!(doc.mindmap.edges[0].glyph_connection.is_none());
        doc.dirty = false;

        let returned = doc.mutate_edge(&er, |edge, canvas| {
            ensure_glyph_connection_inline(edge, canvas).body = "X".to_string();
            true
        });

        assert!(returned);
        assert_eq!(doc.mindmap.edges[0].glyph_connection.as_ref().unwrap().body, "X");
        assert!(doc.dirty);
        assert_eq!(doc.undo_stack.len(), 1);
        match &doc.undo_stack[0] {
            UndoAction::EditEdge { before, .. } => {
                assert!(
                    before.glyph_connection.is_none(),
                    "undo snapshot must carry the pre-fork None"
                );
            }
            other => panic!("expected EditEdge, got {:?}", other),
        }
    }

    /// `commit_throttled_edge_drag` with a `|_, _| true`
    /// predicate (the EdgeHandle release path) pushes one
    /// `EditEdge` undo entry carrying the supplied `original`
    /// snapshot, regardless of whether the live edge changed.
    /// The drag-threshold contract (every reaching release is
    /// post-mutation, so always commit).
    #[test]
    fn commit_throttled_edge_drag_always_commits_on_true_predicate() {
        let (mut doc, er) = doc_with_edge();
        doc.dirty = false;
        let original = doc.mindmap.edges[0].clone();
        doc.commit_throttled_edge_drag(&er, original, |_, _| true);
        assert!(doc.dirty, "true-predicate must mark dirty");
        assert_eq!(doc.undo_stack.len(), 1);
        assert!(matches!(&doc.undo_stack[0], UndoAction::EditEdge { .. }));
    }

    /// A `false` predicate skips the undo push and dirty flag
    /// — the per-frame paths' "no field changed" no-op.
    #[test]
    fn commit_throttled_edge_drag_skips_on_false_predicate() {
        let (mut doc, er) = doc_with_edge();
        doc.dirty = false;
        let original = doc.mindmap.edges[0].clone();
        doc.commit_throttled_edge_drag(&er, original, |_, _| false);
        assert!(!doc.dirty);
        assert!(doc.undo_stack.is_empty());
    }

    /// Predicate runs against the *current* edge state (may
    /// differ from `original`). Verifies the closure receives
    /// the live edge as its first arg and the snapshot as the
    /// second.
    #[test]
    fn commit_throttled_edge_drag_predicate_sees_current_and_original() {
        let (mut doc, er) = doc_with_edge();
        let original = doc.mindmap.edges[0].clone();
        // Mutate the live edge's color so current != original.
        doc.mindmap.edges[0].color = "#abcdef".to_string();
        let mut saw_current_color: Option<String> = None;
        let mut saw_original_color: Option<String> = None;
        doc.commit_throttled_edge_drag(&er, original, |current, original| {
            saw_current_color = Some(current.color.clone());
            saw_original_color = Some(original.color.clone());
            true
        });
        assert_eq!(saw_current_color.as_deref(), Some("#abcdef"));
        assert_ne!(saw_current_color, saw_original_color);
    }

    /// No-op when the edge_ref doesn't resolve — the snapshot
    /// can outlive the edge if a concurrent delete fires.
    #[test]
    fn commit_throttled_edge_drag_noop_when_edge_missing() {
        let (mut doc, _er) = doc_with_edge();
        let stale_er = EdgeRef::new("nope-from", "nope-to", "cross_link");
        let original = doc.mindmap.edges[0].clone();
        doc.dirty = false;
        doc.commit_throttled_edge_drag(&stale_er, original, |_, _| true);
        assert!(!doc.dirty);
        assert!(doc.undo_stack.is_empty());
    }
}
