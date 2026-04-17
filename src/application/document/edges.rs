//! Edge mutations — every `set_edge_*` / `reset_edge_*` /
//! hit-test-handle method on `MindMapDocument`. Each goes through
//! `ensure_glyph_connection` (also here) so the first style edit
//! on a stock edge forks its `GlyphConnectionConfig` off the
//! canvas defaults before writing to it.

use glam::Vec2;

use baumhard::mindmap::model::{
    is_portal_edge, Canvas, GlyphConnectionConfig, MindEdge, DISPLAY_MODE_LINE,
    DISPLAY_MODE_PORTAL, PORTAL_GLYPH_PRESETS,
};
use baumhard::mindmap::scene_builder;

use super::defaults::default_portal_edge;
use super::types::{EdgeRef, SelectionState};
use super::undo_action::UndoAction;
use super::MindMapDocument;

impl MindMapDocument {

    /// Remove an edge matching `edge_ref` from the MindMap. Returns its
    /// original index in `mindmap.edges` and the removed edge so the caller
    /// can push a `DeleteEdge` undo action.
    pub fn remove_edge(&mut self, edge_ref: &EdgeRef) -> Option<(usize, MindEdge)> {
        let idx = self.mindmap.edges.iter().position(|e| edge_ref.matches(e))?;
        let edge = self.mindmap.edges.remove(idx);
        Some((idx, edge))
    }

    /// Hit-test the grab-handles of a specific edge at `canvas_pos`.
    /// Returns the closest handle whose canvas-space position is
    /// within `tolerance` of the cursor, or `None` if nothing is in
    /// range. Used by the Session 6C edge-reshape drag flow — called
    /// at mouse-down time when an edge is currently selected.
    ///
    /// Computed from the live edge (so any in-progress drag is
    /// reflected), without consulting the scene cache. Bounded cost:
    /// one `build_connection_path` + up to five distance comparisons.
    pub fn hit_test_edge_handle(
        &self,
        canvas_pos: Vec2,
        edge_ref: &EdgeRef,
        tolerance: f32,
    ) -> Option<(scene_builder::EdgeHandleKind, Vec2)> {
        let edge = self.mindmap.edges.iter().find(|e| edge_ref.matches(e))?;
        let from_node = self.mindmap.nodes.get(&edge.from_id)?;
        let to_node = self.mindmap.nodes.get(&edge.to_id)?;
        let from_pos = Vec2::new(from_node.position.x as f32, from_node.position.y as f32);
        let from_size = Vec2::new(from_node.size.width as f32, from_node.size.height as f32);
        let to_pos = Vec2::new(to_node.position.x as f32, to_node.position.y as f32);
        let to_size = Vec2::new(to_node.size.width as f32, to_node.size.height as f32);

        let edge_key = baumhard::mindmap::scene_cache::EdgeKey::from_edge(edge);
        let handles = scene_builder::build_edge_handles(
            edge, &edge_key, from_pos, from_size, to_pos, to_size,
        );

        let mut best: Option<(scene_builder::EdgeHandleKind, Vec2, f32)> = None;
        for h in handles {
            let pos = Vec2::new(h.position.0, h.position.1);
            let dist = canvas_pos.distance(pos);
            if dist > tolerance {
                continue;
            }
            if best.as_ref().map_or(true, |(_, _, d)| dist < *d) {
                best = Some((h.kind, pos, dist));
            }
        }
        best.map(|(k, p, _)| (k, p))
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

    /// Set an edge's `anchor_from` (when `is_from == true`) or
    /// `anchor_to` (when `is_from == false`) to `value`. Valid values
    /// are 0 (auto) or 1..=4 (top/right/bottom/left). Returns `true`
    /// if the value changed, pushing an `EditEdge` undo snapshot and
    /// setting `dirty`. Returns `false` if the edge was not found or
    /// the anchor was already at the requested value.
    pub fn set_edge_anchor(&mut self, edge_ref: &EdgeRef, is_from: bool, value: &str) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let current = if is_from {
            &self.mindmap.edges[idx].anchor_from
        } else {
            &self.mindmap.edges[idx].anchor_to
        };
        if current == value {
            return false;
        }
        let before = self.mindmap.edges[idx].clone();
        if is_from {
            self.mindmap.edges[idx].anchor_from = value.to_string();
        } else {
            self.mindmap.edges[idx].anchor_to = value.to_string();
        }
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Look up the index of an edge in `mindmap.edges` matching the
    /// given `EdgeRef`. Returned for callers that need to snapshot
    /// the edge before mutating it in place (e.g. the edge-handle
    /// drag flow in `app.rs`).
    pub fn edge_index(&self, edge_ref: &EdgeRef) -> Option<usize> {
        self.mindmap.edges.iter().position(|e| edge_ref.matches(e))
    }

    // ========================================================================
    // Session 6D — connection style and label mutation helpers
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
    fn ensure_glyph_connection<'a>(
        edge: &'a mut MindEdge,
        canvas: &Canvas,
    ) -> &'a mut GlyphConnectionConfig {
        if edge.glyph_connection.is_none() {
            let seed = canvas
                .default_connection
                .clone()
                .unwrap_or_default();
            edge.glyph_connection = Some(seed);
        }
        edge.glyph_connection.as_mut().expect("just installed")
    }

    /// Set the body glyph string for a connection. Empty strings are
    /// rejected (an empty body would produce no glyphs). Returns
    /// `true` if the edge existed and the body actually changed.
    pub fn set_edge_body_glyph(&mut self, edge_ref: &EdgeRef, body: &str) -> bool {
        if body.is_empty() {
            return false;
        }
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        // Peek at the effective body before forking to detect no-ops.
        let current_body = self.mindmap.edges[idx]
            .glyph_connection
            .as_ref()
            .map(|c| c.body.as_str())
            .or_else(|| self.mindmap.canvas.default_connection.as_ref().map(|c| c.body.as_str()))
            .unwrap_or(&GlyphConnectionConfig::default().body.clone())
            .to_string();
        if current_body == body {
            return false;
        }
        let before = self.mindmap.edges[idx].clone();
        let cfg = Self::ensure_glyph_connection(
            &mut self.mindmap.edges[idx],
            &self.mindmap.canvas,
        );
        cfg.body = body.to_string();
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Set the `cap_start` glyph (or clear it with `None`). Returns
    /// `true` if the edge existed and the value changed.
    pub fn set_edge_cap_start(&mut self, edge_ref: &EdgeRef, cap: Option<&str>) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let before = self.mindmap.edges[idx].clone();
        let cfg = Self::ensure_glyph_connection(
            &mut self.mindmap.edges[idx],
            &self.mindmap.canvas,
        );
        let new_val = cap.map(|s| s.to_string());
        if cfg.cap_start == new_val {
            // Roll back the fork if nothing actually changed (ensure_glyph_connection
            // may have installed a default when the edge previously had
            // glyph_connection = None).
            self.mindmap.edges[idx] = before;
            return false;
        }
        cfg.cap_start = new_val;
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Set the `cap_end` glyph (or clear it with `None`). Returns
    /// `true` if the edge existed and the value changed.
    pub fn set_edge_cap_end(&mut self, edge_ref: &EdgeRef, cap: Option<&str>) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let before = self.mindmap.edges[idx].clone();
        let cfg = Self::ensure_glyph_connection(
            &mut self.mindmap.edges[idx],
            &self.mindmap.canvas,
        );
        let new_val = cap.map(|s| s.to_string());
        if cfg.cap_end == new_val {
            self.mindmap.edges[idx] = before;
            return false;
        }
        cfg.cap_end = new_val;
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Set the color override on a connection's glyph_connection config.
    /// Passing `None` clears the override so the edge inherits from
    /// `edge.color` (or the canvas default). Returns `true` if the edge
    /// existed and the value changed.
    pub fn set_edge_color(&mut self, edge_ref: &EdgeRef, color: Option<&str>) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let before = self.mindmap.edges[idx].clone();
        let cfg = Self::ensure_glyph_connection(
            &mut self.mindmap.edges[idx],
            &self.mindmap.canvas,
        );
        let new_val = color.map(|s| s.to_string());
        if cfg.color == new_val {
            self.mindmap.edges[idx] = before;
            return false;
        }
        cfg.color = new_val;
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Step the connection's base `font_size_pt` by `delta_pt`,
    /// clamped into `[min_font_size_pt, max_font_size_pt]`. Returns
    /// `true` if the clamp yielded a different value from the current
    /// (i.e. we're not already pinned at the relevant bound).
    pub fn set_edge_font_size_step(&mut self, edge_ref: &EdgeRef, delta_pt: f32) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let before = self.mindmap.edges[idx].clone();
        let cfg = Self::ensure_glyph_connection(
            &mut self.mindmap.edges[idx],
            &self.mindmap.canvas,
        );
        let new_val = (cfg.font_size_pt + delta_pt)
            .clamp(cfg.min_font_size_pt, cfg.max_font_size_pt);
        if (cfg.font_size_pt - new_val).abs() < f32::EPSILON {
            self.mindmap.edges[idx] = before;
            return false;
        }
        cfg.font_size_pt = new_val;
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Set the connection's `font_size_pt` to an absolute value,
    /// clamped into `[min_font_size_pt, max_font_size_pt]`. Returns
    /// `true` if the clamped value differs from the current.
    ///
    /// Counterpart to [`set_edge_font_size_step`] for the console's
    /// `font size=<pt>` kv form, where callers have an absolute
    /// target rather than a delta.
    pub fn set_edge_font_size(&mut self, edge_ref: &EdgeRef, pt: f32) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let before = self.mindmap.edges[idx].clone();
        let cfg = Self::ensure_glyph_connection(
            &mut self.mindmap.edges[idx],
            &self.mindmap.canvas,
        );
        let new_val = pt.clamp(cfg.min_font_size_pt, cfg.max_font_size_pt);
        if (cfg.font_size_pt - new_val).abs() < f32::EPSILON {
            self.mindmap.edges[idx] = before;
            return false;
        }
        cfg.font_size_pt = new_val;
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Reset the connection's `font_size_pt` to the hardcoded default
    /// (12.0). Returns `true` if the value actually changed.
    pub fn reset_edge_font_size(&mut self, edge_ref: &EdgeRef) -> bool {
        let default_size = GlyphConnectionConfig::default().font_size_pt;
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let before = self.mindmap.edges[idx].clone();
        let cfg = Self::ensure_glyph_connection(
            &mut self.mindmap.edges[idx],
            &self.mindmap.canvas,
        );
        if (cfg.font_size_pt - default_size).abs() < f32::EPSILON {
            self.mindmap.edges[idx] = before;
            return false;
        }
        cfg.font_size_pt = default_size;
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Set the connection's glyph `spacing` (canvas units between
    /// adjacent body glyphs). Returns `true` if the value actually
    /// changed.
    pub fn set_edge_spacing(&mut self, edge_ref: &EdgeRef, spacing: f32) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let before = self.mindmap.edges[idx].clone();
        let cfg = Self::ensure_glyph_connection(
            &mut self.mindmap.edges[idx],
            &self.mindmap.canvas,
        );
        if (cfg.spacing - spacing).abs() < f32::EPSILON {
            self.mindmap.edges[idx] = before;
            return false;
        }
        cfg.spacing = spacing;
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Set the label text on an edge. Passing `None` (or `Some("")`)
    /// clears the label. Returns `true` if the value actually changed.
    pub fn set_edge_label(&mut self, edge_ref: &EdgeRef, text: Option<String>) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        // Normalize empty string to None so hit testing and rendering
        // only need to check one absence case.
        let new_val = match text {
            Some(s) if s.is_empty() => None,
            other => other,
        };
        if self.mindmap.edges[idx].label == new_val {
            return false;
        }
        let before = self.mindmap.edges[idx].clone();
        self.mindmap.edges[idx].label = new_val;
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Set the label's position along the connection path. `t` is
    /// clamped into `[0.0, 1.0]` — values outside that range are
    /// silently pulled back. Returns `true` if the clamped value
    /// actually differs from the current.
    pub fn set_edge_label_position(&mut self, edge_ref: &EdgeRef, t: f32) -> bool {
        let clamped = t.clamp(0.0, 1.0);
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let current = self.mindmap.edges[idx].label_position_t.unwrap_or(0.5);
        if (current - clamped).abs() < f32::EPSILON {
            return false;
        }
        let before = self.mindmap.edges[idx].clone();
        self.mindmap.edges[idx].label_position_t = Some(clamped);
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Change the `edge_type` of an edge. Refuses the change (returns
    /// `false`) if it would create a duplicate `(from_id, to_id,
    /// new_type)` against another edge. On success updates
    /// `self.selection` to a fresh `EdgeRef` with the new type so the
    /// edge stays selected.
    pub fn set_edge_type(&mut self, edge_ref: &EdgeRef, new_type: &str) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        if self.mindmap.edges[idx].edge_type == new_type {
            return false;
        }
        // Duplicate guard: refuse if some OTHER edge already has the same
        // (from_id, to_id, new_type) triple.
        let from_id = self.mindmap.edges[idx].from_id.clone();
        let to_id = self.mindmap.edges[idx].to_id.clone();
        let duplicate = self.mindmap.edges.iter().enumerate().any(|(i, e)| {
            i != idx
                && e.from_id == from_id
                && e.to_id == to_id
                && e.edge_type == new_type
        });
        if duplicate {
            return false;
        }
        let before = self.mindmap.edges[idx].clone();
        self.mindmap.edges[idx].edge_type = new_type.to_string();
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        // Refresh the selection EdgeRef so the app keeps the edge selected
        // under its new identity.
        if let SelectionState::Edge(ref cur) = self.selection {
            if cur == edge_ref {
                self.selection = SelectionState::Edge(EdgeRef::new(
                    from_id,
                    to_id,
                    new_type,
                ));
            }
        }
        true
    }

    /// Clear `glyph_connection` on the edge, reverting it to the
    /// canvas-level default style. Returns `true` if the edge existed
    /// and had a per-edge override to clear.
    pub fn reset_edge_style_to_default(&mut self, edge_ref: &EdgeRef) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        if self.mindmap.edges[idx].glyph_connection.is_none() {
            return false;
        }
        let before = self.mindmap.edges[idx].clone();
        self.mindmap.edges[idx].glyph_connection = None;
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Switch an edge's `display_mode` between `"line"` and `"portal"`.
    /// `None` / `"line"` → the usual path form; `"portal"` → two
    /// glyph markers above each endpoint, no line between. Unknown
    /// values are rejected with `false`. Returns `false` on no-op
    /// (value already matches, edge not found). Undoes via the
    /// standard `EditEdge { index, before }` path.
    pub fn set_edge_display_mode(&mut self, edge_ref: &EdgeRef, mode: &str) -> bool {
        if mode != DISPLAY_MODE_LINE && mode != DISPLAY_MODE_PORTAL {
            return false;
        }
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let current_is_portal = is_portal_edge(&self.mindmap.edges[idx]);
        let want_portal = mode == DISPLAY_MODE_PORTAL;
        if current_is_portal == want_portal {
            return false;
        }
        let before = self.mindmap.edges[idx].clone();
        self.mindmap.edges[idx].display_mode = if want_portal {
            Some(DISPLAY_MODE_PORTAL.to_string())
        } else {
            None
        };
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Create a new portal-mode edge between two nodes. Validation
    /// mirrors `create_cross_link_edge` — rejects self-edges, missing
    /// endpoints, and duplicate `(from, to, cross_link)` triples. The
    /// marker glyph is picked by rotating `PORTAL_GLYPH_PRESETS` via
    /// the count of existing portal-mode edges, so successive portal
    /// creations look distinct at a glance. Returns the new edge's
    /// index on success.
    pub fn create_portal_edge(
        &mut self,
        source_id: &str,
        target_id: &str,
    ) -> Option<usize> {
        if source_id == target_id {
            return None;
        }
        if !self.mindmap.nodes.contains_key(source_id)
            || !self.mindmap.nodes.contains_key(target_id)
        {
            return None;
        }
        let exists = self.mindmap.edges.iter().any(|e| {
            e.edge_type == "cross_link"
                && e.from_id == source_id
                && e.to_id == target_id
        });
        if exists {
            return None;
        }
        let portal_count = self
            .mindmap
            .edges
            .iter()
            .filter(|e| is_portal_edge(e))
            .count();
        let glyph = PORTAL_GLYPH_PRESETS[portal_count % PORTAL_GLYPH_PRESETS.len()];
        let edge = default_portal_edge(source_id, target_id, glyph);
        self.mindmap.edges.push(edge);
        Some(self.mindmap.edges.len() - 1)
    }

}
