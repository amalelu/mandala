//! Edge mutations — every `set_edge_*` / `reset_edge_*` /
//! hit-test-handle method on `MindMapDocument`. Each goes through
//! `ensure_glyph_connection` (also here) so the first style edit
//! on a stock edge forks its `GlyphConnectionConfig` off the
//! canvas defaults before writing to it.

use glam::Vec2;

use baumhard::mindmap::model::{
    is_portal_edge, portal_endpoint_state_mut, Canvas, EdgeLabelConfig, GlyphConnectionConfig,
    MindEdge, PortalEndpointState, DISPLAY_MODE_LINE, DISPLAY_MODE_PORTAL, PORTAL_GLYPH_PRESETS,
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
        // (centre-to-centre) raw vector between nodes.
        let edge = &self.mindmap.edges[idx];
        let from_node = match self.mindmap.nodes.get(&edge.from_id) {
            Some(n) => n,
            None => return false,
        };
        let to_node = match self.mindmap.nodes.get(&edge.to_id) {
            Some(n) => n,
            None => return false,
        };
        let from_pos = Vec2::new(from_node.position.x as f32, from_node.position.y as f32);
        let from_size =
            Vec2::new(from_node.size.width as f32, from_node.size.height as f32);
        let to_pos = Vec2::new(to_node.position.x as f32, to_node.position.y as f32);
        let to_size = Vec2::new(to_node.size.width as f32, to_node.size.height as f32);
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
        let from_center =
            Vec2::new(from_pos.x + from_size.x * 0.5, from_pos.y + from_size.y * 0.5);
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

    /// Set (or clear, with `color = None`) the `label_config.color`
    /// override on a line-mode edge's label. Sibling of
    /// [`Self::set_edge_color`], which targets the edge body cascade;
    /// this setter writes only the label channel so a coloured edge
    /// can carry a differently-coloured label. Forks a fresh
    /// `EdgeLabelConfig` on the edge if one isn't already present.
    /// Rolls back an all-default `EdgeLabelConfig` when clearing the
    /// color would leave the struct entirely empty, matching the
    /// rollback discipline on `set_portal_label_color` so unchanged
    /// selections don't leave undo droppings.
    pub fn set_edge_label_color(&mut self, edge_ref: &EdgeRef, color: Option<&str>) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let current = self.mindmap.edges[idx]
            .label_config
            .as_ref()
            .and_then(|c| c.color.clone());
        let new_val = color.map(|s| s.to_string());
        if current == new_val {
            return false;
        }
        let before = self.mindmap.edges[idx].clone();
        match new_val {
            Some(c) => {
                Self::ensure_label_config(&mut self.mindmap.edges[idx]).color = Some(c);
            }
            None => {
                if let Some(cfg) = self.mindmap.edges[idx].label_config.as_mut() {
                    cfg.color = None;
                    if cfg == &EdgeLabelConfig::default() {
                        self.mindmap.edges[idx].label_config = None;
                    }
                }
            }
        }
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Read the resolved edge-label color for copy-to-clipboard.
    /// Walks the label color cascade: `label_config.color` →
    /// `glyph_connection.color` → `edge.color`, with
    /// `var(--name)` references expanded through the theme
    /// variable map. Returns `None` only when the edge itself is
    /// missing; a no-override edge still produces a concrete hex
    /// (falls back to `edge.color`) so the user gets something
    /// pasteable in every case.
    pub fn resolve_edge_label_color(&self, edge_ref: &EdgeRef) -> Option<String> {
        let edge = self.mindmap.edges.iter().find(|e| edge_ref.matches(e))?;
        let cfg =
            baumhard::mindmap::model::GlyphConnectionConfig::resolved_for(edge, &self.mindmap.canvas);
        let raw = edge
            .label_config
            .as_ref()
            .and_then(|c| c.color.as_deref())
            .or(cfg.color.as_deref())
            .unwrap_or(edge.color.as_str());
        Some(
            baumhard::util::color::resolve_var(raw, &self.mindmap.canvas.theme_variables)
                .to_string(),
        )
    }

    /// Read the resolved portal-text color for copy-to-clipboard.
    /// Sibling of [`Self::resolve_portal_label_color`] targeting
    /// the text channel: cascade is `text_color` → icon color
    /// cascade (per-endpoint `color` → `glyph_connection.color` →
    /// `edge.color`). Returns `None` only when the edge is
    /// missing.
    pub fn resolve_portal_text_color(
        &self,
        edge_ref: &EdgeRef,
        endpoint_node_id: &str,
    ) -> Option<String> {
        let edge = self.mindmap.edges.iter().find(|e| edge_ref.matches(e))?;
        let state = baumhard::mindmap::model::portal_endpoint_state(edge, endpoint_node_id);
        // Text's own override wins; fall back to the icon's
        // already-resolved cascade via `resolve_portal_label_color`.
        if let Some(hex) = state.and_then(|s| s.text_color.as_deref()) {
            return Some(
                baumhard::util::color::resolve_var(hex, &self.mindmap.canvas.theme_variables)
                    .to_string(),
            );
        }
        self.resolve_portal_label_color(edge_ref, endpoint_node_id)
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

    /// Atomic `font size / min / max` setter for the edge body's
    /// `glyph_connection` channel. Applies `min` and `max` first,
    /// then clamps `size` against the **new** bounds, so the user-
    /// level command `font size=14 max=10` lands as `size=10, max=10`
    /// instead of the wrong `size=14, max=10` a naive one-at-a-time
    /// dispatch would produce. Each argument is optional; `None`
    /// leaves that field untouched. Returns `true` if any field
    /// changed. Rejects non-finite or non-positive values by
    /// leaving the field untouched.
    ///
    /// A single `EditEdge` undo entry covers the whole triple, so
    /// Ctrl+Z reverses the atomic edit in one step.
    pub fn set_edge_font(
        &mut self,
        edge_ref: &EdgeRef,
        size: Option<f32>,
        min: Option<f32>,
        max: Option<f32>,
    ) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let before = self.mindmap.edges[idx].clone();
        let cfg = Self::ensure_glyph_connection(
            &mut self.mindmap.edges[idx],
            &self.mindmap.canvas,
        );
        let mut changed = false;
        if let Some(m) = min.filter(|v| v.is_finite() && *v > 0.0) {
            if (cfg.min_font_size_pt - m).abs() >= f32::EPSILON {
                cfg.min_font_size_pt = m;
                changed = true;
            }
        }
        if let Some(m) = max.filter(|v| v.is_finite() && *v > 0.0) {
            if (cfg.max_font_size_pt - m).abs() >= f32::EPSILON {
                cfg.max_font_size_pt = m;
                changed = true;
            }
        }
        if let Some(s) = size.filter(|v| v.is_finite() && *v > 0.0) {
            let clamped = s.clamp(cfg.min_font_size_pt, cfg.max_font_size_pt);
            if (cfg.font_size_pt - clamped).abs() >= f32::EPSILON {
                cfg.font_size_pt = clamped;
                changed = true;
            }
        }
        if !changed {
            self.mindmap.edges[idx] = before;
            return false;
        }
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Sibling of [`Self::set_edge_font`] targeting the edge
    /// **label** channel (`label_config.font_size_pt` / `min` /
    /// `max`). Same atomic ordering — min/max write before the
    /// clamped size — so label-level clamps can be tightened
    /// without dropping a concurrent size write. Forks a fresh
    /// `EdgeLabelConfig` on first edit; rolls back an all-default
    /// struct when clearing to None leaves nothing interesting.
    ///
    /// Resolver fallbacks: a label with no own override inherits
    /// the edge's `glyph_connection` clamps (see
    /// `EdgeLabelConfig::effective_font_size_pt`). Clamping the
    /// user-facing `size` value here happens against the
    /// **resolved** clamps — own min/max when set, edge min/max
    /// otherwise — so a label that only overrides `size` clamps
    /// into the edge's bounds without needing a full triple.
    pub fn set_edge_label_font(
        &mut self,
        edge_ref: &EdgeRef,
        size: Option<f32>,
        min: Option<f32>,
        max: Option<f32>,
    ) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        // Compute the resolved body clamps once for fallback
        // when the label config doesn't carry its own.
        let body_min;
        let body_max;
        {
            let edge = &self.mindmap.edges[idx];
            let cfg = GlyphConnectionConfig::resolved_for(edge, &self.mindmap.canvas);
            body_min = cfg.min_font_size_pt;
            body_max = cfg.max_font_size_pt;
        }
        let before = self.mindmap.edges[idx].clone();
        let label_cfg = Self::ensure_label_config(&mut self.mindmap.edges[idx]);
        let mut changed = false;
        if let Some(m) = min.filter(|v| v.is_finite() && *v > 0.0) {
            if label_cfg.min_font_size_pt != Some(m) {
                label_cfg.min_font_size_pt = Some(m);
                changed = true;
            }
        }
        if let Some(m) = max.filter(|v| v.is_finite() && *v > 0.0) {
            if label_cfg.max_font_size_pt != Some(m) {
                label_cfg.max_font_size_pt = Some(m);
                changed = true;
            }
        }
        if let Some(s) = size.filter(|v| v.is_finite() && *v > 0.0) {
            let effective_min = label_cfg.min_font_size_pt.unwrap_or(body_min);
            let effective_max = label_cfg.max_font_size_pt.unwrap_or(body_max);
            let clamped = s.clamp(effective_min, effective_max);
            if label_cfg.font_size_pt != Some(clamped) {
                label_cfg.font_size_pt = Some(clamped);
                changed = true;
            }
        }
        // Rollback-on-noop + rollback-if-label-config-empty so
        // an unchanged triple doesn't leave an empty
        // `EdgeLabelConfig` behind.
        if !changed {
            self.mindmap.edges[idx] = before;
            return false;
        }
        if self.mindmap.edges[idx]
            .label_config
            .as_ref()
            .map_or(false, |c| c == &EdgeLabelConfig::default())
        {
            self.mindmap.edges[idx].label_config = None;
        }
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Sibling of [`Self::set_edge_font`] targeting a portal
    /// endpoint's **text** channel
    /// (`PortalEndpointState.text_font_size_pt` / `text_min_font_size_pt`
    /// / `text_max_font_size_pt`). Same atomic ordering. Forks
    /// `PortalEndpointState` on first edit; rolls back an all-default
    /// endpoint state on clear. Fallback clamps come from the
    /// resolved `glyph_connection` when the endpoint's own clamps
    /// aren't set, matching the label resolver.
    pub fn set_portal_text_font(
        &mut self,
        edge_ref: &EdgeRef,
        endpoint_node_id: &str,
        size: Option<f32>,
        min: Option<f32>,
        max: Option<f32>,
    ) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let body_min;
        let body_max;
        {
            let edge = &self.mindmap.edges[idx];
            let cfg = GlyphConnectionConfig::resolved_for(edge, &self.mindmap.canvas);
            body_min = cfg.min_font_size_pt;
            body_max = cfg.max_font_size_pt;
        }
        let before = self.mindmap.edges[idx].clone();
        let slot = match portal_endpoint_state_mut(
            &mut self.mindmap.edges[idx],
            endpoint_node_id,
        ) {
            Some(s) => s,
            None => return false,
        };
        let state = slot.get_or_insert_with(PortalEndpointState::default);
        let mut changed = false;
        if let Some(m) = min.filter(|v| v.is_finite() && *v > 0.0) {
            if state.text_min_font_size_pt != Some(m) {
                state.text_min_font_size_pt = Some(m);
                changed = true;
            }
        }
        if let Some(m) = max.filter(|v| v.is_finite() && *v > 0.0) {
            if state.text_max_font_size_pt != Some(m) {
                state.text_max_font_size_pt = Some(m);
                changed = true;
            }
        }
        if let Some(s) = size.filter(|v| v.is_finite() && *v > 0.0) {
            let effective_min = state.text_min_font_size_pt.unwrap_or(body_min);
            let effective_max = state.text_max_font_size_pt.unwrap_or(body_max);
            let clamped = s.clamp(effective_min, effective_max);
            if state.text_font_size_pt != Some(clamped) {
                state.text_font_size_pt = Some(clamped);
                changed = true;
            }
        }
        if !changed {
            self.mindmap.edges[idx] = before;
            return false;
        }
        if let Some(existing) = self.mindmap.edges[idx]
            .portal_from
            .as_ref()
            .filter(|s| *s == &PortalEndpointState::default())
        {
            let _ = existing;
            self.mindmap.edges[idx].portal_from = None;
        }
        if let Some(existing) = self.mindmap.edges[idx]
            .portal_to
            .as_ref()
            .filter(|s| *s == &PortalEndpointState::default())
        {
            let _ = existing;
            self.mindmap.edges[idx].portal_to = None;
        }
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

    /// Set the label's tangential position along the connection path.
    /// `t` is clamped into `[0.0, 1.0]` — values outside that range
    /// are silently pulled back. Returns `true` if the clamped value
    /// actually differs from the current. Forks a fresh
    /// `EdgeLabelConfig` on the edge if one isn't already present
    /// (mirrors `ensure_glyph_connection` on the body cascade).
    pub fn set_edge_label_position(&mut self, edge_ref: &EdgeRef, t: f32) -> bool {
        let clamped = t.clamp(0.0, 1.0);
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let current = EdgeLabelConfig::effective_position_t(
            self.mindmap.edges[idx].label_config.as_ref(),
        );
        if (current - clamped).abs() < f32::EPSILON {
            return false;
        }
        let before = self.mindmap.edges[idx].clone();
        Self::ensure_label_config(&mut self.mindmap.edges[idx]).position_t = Some(clamped);
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Return a mutable reference to `edge.label_config`, lazily
    /// inserting a default [`EdgeLabelConfig`] when absent. Mirrors
    /// [`Self::ensure_glyph_connection`] for the body cascade — the
    /// first edit on an unstyled label forks a config onto the edge,
    /// subsequent edits reuse it.
    fn ensure_label_config(edge: &mut MindEdge) -> &mut EdgeLabelConfig {
        edge.label_config.get_or_insert_with(EdgeLabelConfig::default)
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

    // ========================================================================
    // Portal label mutations — per-endpoint overrides for a portal-mode edge.
    // Each helper follows the same pattern as `set_edge_color` /
    // `set_edge_label`: locate the edge, clone the pre-edit snapshot, mutate
    // the per-endpoint `PortalEndpointState`, push `UndoAction::EditEdge`,
    // set `dirty`. The caller passes the `endpoint_node_id` identifying
    // *which* of the two endpoints is being targeted — this must equal
    // either `edge.from_id` or `edge.to_id`; other values return `false`
    // unchanged.
    // ========================================================================

    /// Set (or clear, with `color = None`) the per-endpoint color
    /// override on a portal-mode edge's label. Returns `true` if
    /// the value changed. No-op if the edge isn't found or the
    /// endpoint id doesn't match either side. Rolls back a newly
    /// installed empty `PortalEndpointState` when clearing a color
    /// would leave the state entirely default, so an unchanged
    /// selection doesn't leave undo droppings.
    pub fn set_portal_label_color(
        &mut self,
        edge_ref: &EdgeRef,
        endpoint_node_id: &str,
        color: Option<&str>,
    ) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let before = self.mindmap.edges[idx].clone();
        let slot = match portal_endpoint_state_mut(
            &mut self.mindmap.edges[idx],
            endpoint_node_id,
        ) {
            Some(s) => s,
            None => return false,
        };
        let current = slot.as_ref().and_then(|s| s.color.clone());
        let new_val = color.map(|s| s.to_string());
        if current == new_val {
            return false;
        }
        match new_val {
            Some(c) => {
                slot.get_or_insert_with(PortalEndpointState::default).color = Some(c);
            }
            None => {
                if let Some(existing) = slot.as_mut() {
                    existing.color = None;
                    if existing == &PortalEndpointState::default() {
                        *slot = None;
                    }
                }
            }
        }
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Set (or clear, with `t = None`) the per-endpoint
    /// `border_t` position on a portal-mode edge's label.
    /// Returns `true` if the value changed. `t` is wrapped into
    /// the canonical `[0, 4)` perimeter parameter; callers can
    /// pass any finite value and get the canonical wrap for free.
    pub fn set_portal_label_border_t(
        &mut self,
        edge_ref: &EdgeRef,
        endpoint_node_id: &str,
        t: Option<f32>,
    ) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let wrapped = t.map(baumhard::mindmap::portal_geometry::wrap_border_t);
        let before = self.mindmap.edges[idx].clone();
        let slot = match portal_endpoint_state_mut(
            &mut self.mindmap.edges[idx],
            endpoint_node_id,
        ) {
            Some(s) => s,
            None => return false,
        };
        let current = slot.as_ref().and_then(|s| s.border_t);
        let matched = match (current, wrapped) {
            (None, None) => true,
            (Some(a), Some(b)) => (a - b).abs() < f32::EPSILON,
            _ => false,
        };
        if matched {
            return false;
        }
        match wrapped {
            Some(t) => {
                slot.get_or_insert_with(PortalEndpointState::default).border_t = Some(t);
            }
            None => {
                if let Some(existing) = slot.as_mut() {
                    existing.border_t = None;
                    if existing == &PortalEndpointState::default() {
                        *slot = None;
                    }
                }
            }
        }
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Set (or clear, with `text = None`) the per-endpoint text
    /// label on a portal-mode edge. Empty strings are normalized
    /// to `None` so hit-test / render / serde only see one
    /// "absent" form. Returns `true` if the value changed.
    pub fn set_portal_label_text(
        &mut self,
        edge_ref: &EdgeRef,
        endpoint_node_id: &str,
        text: Option<String>,
    ) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let normalized = match text {
            Some(s) if s.is_empty() => None,
            other => other,
        };
        let before = self.mindmap.edges[idx].clone();
        let slot = match portal_endpoint_state_mut(
            &mut self.mindmap.edges[idx],
            endpoint_node_id,
        ) {
            Some(s) => s,
            None => return false,
        };
        let current = slot.as_ref().and_then(|s| s.text.clone());
        if current == normalized {
            return false;
        }
        match normalized {
            Some(t) => {
                slot.get_or_insert_with(PortalEndpointState::default).text = Some(t);
            }
            None => {
                if let Some(existing) = slot.as_mut() {
                    existing.text = None;
                    if existing == &PortalEndpointState::default() {
                        *slot = None;
                    }
                }
            }
        }
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Set (or clear, with `color = None`) the per-endpoint
    /// **text** color override on a portal-mode edge. Sibling of
    /// [`Self::set_portal_label_color`], which targets the icon
    /// cascade; this setter targets `PortalEndpointState.text_color`
    /// so a coloured badge can host a differently-coloured
    /// annotation. Returns `true` if the value changed. Rolls back
    /// a newly-installed empty `PortalEndpointState` when clearing
    /// a text color would leave the state entirely default, so an
    /// unchanged selection doesn't leave undo droppings — mirrors
    /// the `set_portal_label_color` rollback pattern.
    pub fn set_portal_label_text_color(
        &mut self,
        edge_ref: &EdgeRef,
        endpoint_node_id: &str,
        color: Option<&str>,
    ) -> bool {
        let idx = match self.mindmap.edges.iter().position(|e| edge_ref.matches(e)) {
            Some(i) => i,
            None => return false,
        };
        let before = self.mindmap.edges[idx].clone();
        let slot = match portal_endpoint_state_mut(
            &mut self.mindmap.edges[idx],
            endpoint_node_id,
        ) {
            Some(s) => s,
            None => return false,
        };
        let current = slot.as_ref().and_then(|s| s.text_color.clone());
        let new_val = color.map(|s| s.to_string());
        if current == new_val {
            return false;
        }
        match new_val {
            Some(c) => {
                slot.get_or_insert_with(PortalEndpointState::default).text_color = Some(c);
            }
            None => {
                if let Some(existing) = slot.as_mut() {
                    existing.text_color = None;
                    if existing == &PortalEndpointState::default() {
                        *slot = None;
                    }
                }
            }
        }
        self.undo_stack.push(UndoAction::EditEdge { index: idx, before });
        self.dirty = true;
        true
    }

    /// Read the current portal label text for one endpoint, if
    /// any. Returns the concrete string (not the hex-color
    /// cascade like [`Self::resolve_portal_label_color`]) —
    /// portal text has no inheritance cascade, it's either set
    /// on the endpoint or absent.
    pub fn portal_label_text(
        &self,
        edge_ref: &EdgeRef,
        endpoint_node_id: &str,
    ) -> Option<String> {
        let edge = self.mindmap.edges.iter().find(|e| edge_ref.matches(e))?;
        let state =
            baumhard::mindmap::model::portal_endpoint_state(edge, endpoint_node_id)?;
        state.text.clone()
    }

    /// Read the resolved portal label color for one endpoint.
    /// Walks the cascade — per-endpoint override >
    /// `glyph_connection.color` > `edge.color` — and returns the
    /// resolved string (with `var(--name)` references already
    /// expanded through the theme variable map). Used by clipboard
    /// copy: the user expects `copy` on a portal label to produce
    /// a real hex they can paste elsewhere, even when no override
    /// is set.
    pub fn resolve_portal_label_color(
        &self,
        edge_ref: &EdgeRef,
        endpoint_node_id: &str,
    ) -> Option<String> {
        let edge = self.mindmap.edges.iter().find(|e| edge_ref.matches(e))?;
        let endpoint_state =
            baumhard::mindmap::model::portal_endpoint_state(edge, endpoint_node_id);
        // Camera zoom is irrelevant for color resolution — pass
        // 1.0 so the font-size clamp path doesn't branch oddly.
        let style = baumhard::mindmap::scene_builder::portal::resolve_portal_endpoint_style(
            edge,
            endpoint_state,
            &self.mindmap.canvas,
            None,
            1.0,
        );
        Some(style.color)
    }
}
