// SPDX-License-Identifier: MPL-2.0

//! Edge visual styling — body glyph, caps, color, font sizing/family, spacing.

use baumhard::mindmap::model::{
    portal_endpoint_state_mut, EdgeLabelConfig, GlyphConnectionConfig, PortalEndpointState,
};
use baumhard::util::geometry::almost_equal;

use super::super::types::EdgeRef;
use super::super::MindMapDocument;
use super::closure_helpers::{ensure_glyph_connection_inline, ensure_label_config_inline};
use super::font_triple::{resolve_font_triple, FontTriple};

impl MindMapDocument {
    /// Set the body glyph string for a connection. Empty strings are
    /// rejected (an empty body would produce no glyphs). Returns
    /// `true` if the edge existed and the body actually changed.
    pub fn set_edge_body_glyph(&mut self, edge_ref: &EdgeRef, body: &str) -> bool {
        if body.is_empty() {
            return false;
        }
        self.mutate_edge(edge_ref, |edge, canvas| {
            // Peek at the effective body before forking to detect no-ops.
            let default_body = GlyphConnectionConfig::default().body;
            let current_body = edge
                .glyph_connection
                .as_ref()
                .map(|c| c.body.as_str())
                .or_else(|| canvas.default_connection.as_ref().map(|c| c.body.as_str()))
                .unwrap_or(&default_body);
            if current_body == body {
                return false;
            }
            ensure_glyph_connection_inline(edge, canvas).body = body.to_string();
            true
        })
    }

    /// Set the `cap_start` glyph (or clear it with `None`). Returns
    /// `true` if the edge existed and the value changed.
    pub fn set_edge_cap_start(&mut self, edge_ref: &EdgeRef, cap: Option<&str>) -> bool {
        let new_val = cap.map(|s| s.to_string());
        self.mutate_edge(edge_ref, |edge, canvas| {
            let cfg = ensure_glyph_connection_inline(edge, canvas);
            if cfg.cap_start == new_val {
                return false;
            }
            cfg.cap_start = new_val;
            true
        })
    }

    /// Set the `cap_end` glyph (or clear it with `None`). Returns
    /// `true` if the edge existed and the value changed.
    pub fn set_edge_cap_end(&mut self, edge_ref: &EdgeRef, cap: Option<&str>) -> bool {
        let new_val = cap.map(|s| s.to_string());
        self.mutate_edge(edge_ref, |edge, canvas| {
            let cfg = ensure_glyph_connection_inline(edge, canvas);
            if cfg.cap_end == new_val {
                return false;
            }
            cfg.cap_end = new_val;
            true
        })
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
        let new_val = color.map(|s| s.to_string());
        self.mutate_edge(edge_ref, |edge, _canvas| {
            let current = edge.label_config.as_ref().and_then(|c| c.color.clone());
            if current == new_val {
                return false;
            }
            match new_val {
                Some(c) => {
                    ensure_label_config_inline(edge).color = Some(c);
                }
                None => {
                    if let Some(cfg) = edge.label_config.as_mut() {
                        cfg.color = None;
                        if cfg == &EdgeLabelConfig::default() {
                            edge.label_config = None;
                        }
                    }
                }
            }
            true
        })
    }

    /// Read the resolved **edge body** color for copy-to-clipboard.
    /// Walks the body cascade: `glyph_connection.color` →
    /// `edge.color`, with `var(--name)` references expanded
    /// through the theme variable map. Returns `None` only when
    /// the edge itself is missing; a no-override edge still
    /// produces a concrete hex (`edge.color` is always present
    /// in the model) so the user gets something pasteable. The
    /// clipboard copy on an `Edge` selection routes through this
    /// helper rather than duplicating the cascade inline, so a
    /// future change to the body cascade (e.g. a third tier) only
    /// touches one site.
    pub fn resolve_edge_color(&self, edge_ref: &EdgeRef) -> Option<String> {
        let edge = self.mindmap.edges.iter().find(|e| edge_ref.matches(e))?;
        Some(self.resolve_var_owned(edge.body_color(&self.mindmap.canvas)))
    }

    /// Read the resolved edge-label color for copy-to-clipboard.
    /// Walks the label color cascade: `label_config.color` →
    /// edge body cascade ([`Self::resolve_edge_color`]). The
    /// label channel's own override wins; absent override falls
    /// back to whatever the body cascade produces so the label
    /// visually matches the edge unless explicitly detached.
    pub fn resolve_edge_label_color(&self, edge_ref: &EdgeRef) -> Option<String> {
        let edge = self.mindmap.edges.iter().find(|e| edge_ref.matches(e))?;
        Some(self.resolve_var_owned(edge.label_color(&self.mindmap.canvas)))
    }

    /// Read the resolved portal-text color for copy-to-clipboard.
    /// Sibling of [`Self::resolve_portal_label_color`] targeting
    /// the text channel: cascade is `text_color` → icon color
    /// cascade (per-endpoint `color` → `glyph_connection.color` →
    /// `edge.color`). Returns `None` only when the edge is
    /// missing.
    pub fn resolve_portal_text_color(&self, edge_ref: &EdgeRef, endpoint_node_id: &str) -> Option<String> {
        let edge = self.mindmap.edges.iter().find(|e| edge_ref.matches(e))?;
        let state = baumhard::mindmap::model::portal_endpoint_state(edge, endpoint_node_id);
        Some(self.resolve_var_owned(edge.portal_endpoint_text_color(&self.mindmap.canvas, state)))
    }

    /// Expand `var(--name)` references in `raw` against the
    /// canvas theme map and own the result. The three
    /// `resolve_*_color` readers above all end in this same step;
    /// the cascade itself lives on [`baumhard::mindmap::model::MindEdge`].
    fn resolve_var_owned(&self, raw: &str) -> String {
        baumhard::util::color::resolve_var(raw, &self.mindmap.canvas.theme_variables).to_string()
    }

    /// Set the color override on a connection's glyph_connection config.
    /// Passing `None` clears the override so the edge inherits from
    /// `edge.color` (or the canvas default). Returns `true` if the edge
    /// existed and the value changed.
    pub fn set_edge_color(&mut self, edge_ref: &EdgeRef, color: Option<&str>) -> bool {
        let new_val = color.map(|s| s.to_string());
        self.mutate_edge(edge_ref, |edge, canvas| {
            let cfg = ensure_glyph_connection_inline(edge, canvas);
            if cfg.color == new_val {
                return false;
            }
            cfg.color = new_val;
            true
        })
    }

    /// Step the connection's base `font_size_pt` by `delta_pt`,
    /// clamped into `[min_font_size_pt, max_font_size_pt]`. Returns
    /// `true` if the clamp yielded a different value from the current
    /// (i.e. we're not already pinned at the relevant bound).
    ///
    /// **No-op tolerance.** The change-detection compares
    /// pre/post via `almost_equal` (1e-5). A `delta_pt` smaller
    /// than that magnitude (e.g. `1e-6`) silently no-ops — the
    /// stored value is f32 and sub-1e-5 deltas don't survive
    /// the clamp's float arithmetic anyway, so the no-op
    /// preserves the model's "bit-exact equality after
    /// round-trip" property. User-facing verbs should validate
    /// reasonable delta magnitudes upstream.
    pub fn set_edge_font_size_step(&mut self, edge_ref: &EdgeRef, delta_pt: f32) -> bool {
        self.mutate_edge(edge_ref, |edge, canvas| {
            let cfg = ensure_glyph_connection_inline(edge, canvas);
            let new_val = (cfg.font_size_pt + delta_pt).clamp(cfg.min_font_size_pt, cfg.max_font_size_pt);
            if almost_equal(cfg.font_size_pt, new_val) {
                return false;
            }
            cfg.font_size_pt = new_val;
            true
        })
    }

    /// Set the connection's `font_size_pt` to an absolute value,
    /// clamped into `[min_font_size_pt, max_font_size_pt]`. Returns
    /// `true` if the clamped value differs from the current.
    ///
    /// Counterpart to [`Self::set_edge_font_size_step`] for the
    /// console's `font size=<pt>` kv form, where callers have an
    /// absolute target rather than a delta.
    pub fn set_edge_font_size(&mut self, edge_ref: &EdgeRef, pt: f32) -> bool {
        self.mutate_edge(edge_ref, |edge, canvas| {
            let cfg = ensure_glyph_connection_inline(edge, canvas);
            let new_val = pt.clamp(cfg.min_font_size_pt, cfg.max_font_size_pt);
            if almost_equal(cfg.font_size_pt, new_val) {
                return false;
            }
            cfg.font_size_pt = new_val;
            true
        })
    }

    /// Reset the connection's `font_size_pt` to the hardcoded default
    /// (12.0). Returns `true` if the value actually changed.
    pub fn reset_edge_font_size(&mut self, edge_ref: &EdgeRef) -> bool {
        let default_size = GlyphConnectionConfig::default().font_size_pt;
        self.mutate_edge(edge_ref, |edge, canvas| {
            let cfg = ensure_glyph_connection_inline(edge, canvas);
            if almost_equal(cfg.font_size_pt, default_size) {
                return false;
            }
            cfg.font_size_pt = default_size;
            true
        })
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
    /// **Inverted bounds guard.** The resolved `(min, max)` pair
    /// (after applying overrides on top of the existing struct)
    /// must satisfy `min ≤ max`. Inverted input returns `false`
    /// without mutating — landing an inverted pair would panic
    /// the next renderer frame via
    /// [`baumhard::mindmap::model::GlyphConnectionConfig::effective_font_size_pt`]'s
    /// `clamp` call (interactive-path invariant per §9). The
    /// console `font` command re-checks up-front so the user gets
    /// a clear error message; this boundary check is defence in
    /// depth for any other caller.
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
        self.mutate_edge(edge_ref, |edge, canvas| {
            let cfg = ensure_glyph_connection_inline(edge, canvas);
            // The body channel is the bottom of the cascade, so
            // its three values are always concrete — it inherits
            // its own current clamps as the fallback.
            let current = FontTriple {
                size: Some(cfg.font_size_pt),
                min: Some(cfg.min_font_size_pt),
                max: Some(cfg.max_font_size_pt),
            };
            let fallback = (cfg.min_font_size_pt, cfg.max_font_size_pt);
            let Some(resolved) = resolve_font_triple(current, fallback, size, min, max) else {
                return false;
            };
            if !resolved.differs_from(&current) {
                return false;
            }
            // Every field is `Some` here: `resolve_font_triple`
            // only ever carries a `None` through from `current`,
            // and `current` has none.
            cfg.font_size_pt = resolved.size.unwrap_or(cfg.font_size_pt);
            cfg.min_font_size_pt = resolved.min.unwrap_or(cfg.min_font_size_pt);
            cfg.max_font_size_pt = resolved.max.unwrap_or(cfg.max_font_size_pt);
            true
        })
    }

    /// Set the edge body's `glyph_connection.font` family override.
    /// `Some("Norse")` pins the edge glyphs to that family; `None`
    /// clears the override (edge falls back to the canvas default
    /// font).
    ///
    /// Forks a fresh `GlyphConnectionConfig` on first edit via
    /// `ensure_glyph_connection`. A single `UndoAction::EditEdge`
    /// entry covers the change so Ctrl+Z reverses cleanly.
    /// Family-name validation is the caller's job — the data model
    /// stores the string verbatim and the tree builder resolves
    /// it through `baumhard::font::fonts::app_font_by_family` at
    /// render time, falling back to monospace with a warning if
    /// the family is unknown.
    pub fn set_edge_font_family(&mut self, edge_ref: &EdgeRef, family: Option<&str>) -> bool {
        let target = family.filter(|s| !s.is_empty()).map(|s| s.to_string());
        self.mutate_edge(edge_ref, |edge, canvas| {
            // Peek the authored family *before* forking so a
            // no-op clear (`None` on an edge that has no override
            // yet) doesn't mint an undo entry. `mutate_edge`
            // would roll the fork back either way; short-
            // circuiting here also skips the clone.
            let current = edge.glyph_connection.as_ref().and_then(|c| c.font.clone());
            if current == target {
                return false;
            }
            ensure_glyph_connection_inline(edge, canvas).font = target;
            true
        })
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
        self.mutate_edge(edge_ref, |edge, canvas| {
            // The label channel inherits the body's clamps when
            // it authors none of its own — same cascade
            // `EdgeLabelConfig::effective_font_size_pt` reads at
            // render time.
            let body = GlyphConnectionConfig::resolved_for(edge, canvas);
            let fallback = (body.min_font_size_pt, body.max_font_size_pt);
            let label_cfg = edge.label_config.as_ref();
            let current = FontTriple {
                size: label_cfg.and_then(|c| c.font_size_pt),
                min: label_cfg.and_then(|c| c.min_font_size_pt),
                max: label_cfg.and_then(|c| c.max_font_size_pt),
            };
            let Some(resolved) = resolve_font_triple(current, fallback, size, min, max) else {
                return false;
            };
            if !resolved.differs_from(&current) {
                return false;
            }
            // No all-default scrub here, unlike the pre-fold
            // version: a `true` verdict means at least one of the
            // three resolved values is `Some`, so the config
            // cannot come out empty — and the no-change path
            // above returns `false`, which makes `mutate_edge`
            // roll the fork back wholesale. The scrub was
            // guarding a state that can no longer be reached.
            let cfg = ensure_label_config_inline(edge);
            cfg.font_size_pt = resolved.size;
            cfg.min_font_size_pt = resolved.min;
            cfg.max_font_size_pt = resolved.max;
            true
        })
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
        self.mutate_edge(edge_ref, |edge, canvas| {
            // Same body-clamp fallback as the label channel; the
            // portal text resolver reads the identical cascade.
            let body = GlyphConnectionConfig::resolved_for(edge, canvas);
            let fallback = (body.min_font_size_pt, body.max_font_size_pt);
            let Some(slot) = portal_endpoint_state_mut(edge, endpoint_node_id) else {
                return false;
            };
            let state = slot.as_ref();
            let current = FontTriple {
                size: state.and_then(|s| s.text_font_size_pt),
                min: state.and_then(|s| s.text_min_font_size_pt),
                max: state.and_then(|s| s.text_max_font_size_pt),
            };
            let Some(resolved) = resolve_font_triple(current, fallback, size, min, max) else {
                return false;
            };
            if !resolved.differs_from(&current) {
                return false;
            }
            // Same reasoning as `set_edge_label_font`: a `true`
            // verdict guarantees a non-default state, and the
            // `false` paths above never reach the fork because
            // `mutate_edge` restores the whole edge. The
            // hand-rolled `forked_default` scrub the pre-fold
            // version carried is unreachable now.
            let state = slot.get_or_insert_with(PortalEndpointState::default);
            state.text_font_size_pt = resolved.size;
            state.text_min_font_size_pt = resolved.min;
            state.text_max_font_size_pt = resolved.max;
            true
        })
    }

    /// Set the connection's glyph `spacing` (canvas units between
    /// adjacent body glyphs). Returns `true` if the value actually
    /// changed.
    pub fn set_edge_spacing(&mut self, edge_ref: &EdgeRef, spacing: f32) -> bool {
        self.mutate_edge(edge_ref, |edge, canvas| {
            let cfg = ensure_glyph_connection_inline(edge, canvas);
            if almost_equal(cfg.spacing, spacing) {
                return false;
            }
            cfg.spacing = spacing;
            true
        })
    }
}
