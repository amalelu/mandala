// SPDX-License-Identifier: MPL-2.0

//! Portal edge lifecycle and portal-label mutations.

use baumhard::mindmap::model::{is_portal_edge, portal_endpoint_state_mut, PORTAL_GLYPH_PRESETS};

use super::super::defaults::default_portal_edge;
use super::super::types::EdgeRef;
use super::super::MindMapDocument;
use super::closure_helpers::write_endpoint_field;

impl MindMapDocument {
    /// Create a new portal-mode edge between two nodes. Validation
    /// mirrors `create_cross_link_edge` — rejects self-edges, missing
    /// endpoints, and duplicate `(from, to, cross_link)` triples. The
    /// marker glyph is picked by rotating `PORTAL_GLYPH_PRESETS` via
    /// the count of existing portal-mode edges, so successive portal
    /// creations look distinct at a glance. Returns the new edge's
    /// index on success.
    pub fn create_portal_edge(&mut self, source_id: &str, target_id: &str) -> Option<usize> {
        if source_id == target_id {
            return None;
        }
        if !self.mindmap.nodes.contains_key(source_id) || !self.mindmap.nodes.contains_key(target_id) {
            return None;
        }
        let exists = self
            .mindmap
            .edges
            .iter()
            .any(|e| e.edge_type == "cross_link" && e.from_id == source_id && e.to_id == target_id);
        if exists {
            return None;
        }
        let portal_count = self.mindmap.edges.iter().filter(|e| is_portal_edge(e)).count();
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
        let new_val = color.map(|s| s.to_string());
        self.mutate_edge(edge_ref, |edge, _canvas| {
            let Some(slot) = portal_endpoint_state_mut(edge, endpoint_node_id) else {
                return false;
            };
            let current = slot.as_ref().and_then(|s| s.color.clone());
            if current == new_val {
                return false;
            }
            write_endpoint_field(slot, new_val, |s, v| s.color = v);
            true
        })
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
        let wrapped = t.map(baumhard::mindmap::portal_geometry::wrap_border_t);
        self.mutate_edge(edge_ref, |edge, _canvas| {
            let Some(slot) = portal_endpoint_state_mut(edge, endpoint_node_id) else {
                return false;
            };
            let current = slot.as_ref().and_then(|s| s.border_t);
            if baumhard::util::geometry::option_almost_equal(current, wrapped) {
                return false;
            }
            write_endpoint_field(slot, wrapped, |s, v| s.border_t = v);
            true
        })
    }

    /// Set (or clear, with `offset = None`) the per-endpoint
    /// `perpendicular_offset` on a portal-mode edge — the signed
    /// distance along the border's outward normal that pulls the
    /// label away from (or pushes it toward) the owning node.
    /// Used by the `label perpendicular=<f32>` console key on
    /// portal-label selections; the portal-label drag writes the
    /// field directly for per-frame performance. Rolls back an
    /// all-default `PortalEndpointState` on clear so unchanged
    /// selections leave no undo droppings.
    pub fn set_portal_label_perpendicular_offset(
        &mut self,
        edge_ref: &EdgeRef,
        endpoint_node_id: &str,
        offset: Option<f32>,
    ) -> bool {
        // Reject NaN / infinity at the boundary; the model stores
        // only finite values.
        if let Some(v) = offset {
            if !v.is_finite() {
                return false;
            }
        }
        self.mutate_edge(edge_ref, |edge, _canvas| {
            let Some(slot) = portal_endpoint_state_mut(edge, endpoint_node_id) else {
                return false;
            };
            let current = slot.as_ref().and_then(|s| s.perpendicular_offset);
            if baumhard::util::geometry::option_almost_equal(current, offset) {
                return false;
            }
            write_endpoint_field(slot, offset, |s, v| s.perpendicular_offset = v);
            true
        })
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
        let normalized = match text {
            Some(s) if s.is_empty() => None,
            other => other,
        };
        self.mutate_edge(edge_ref, |edge, _canvas| {
            let Some(slot) = portal_endpoint_state_mut(edge, endpoint_node_id) else {
                return false;
            };
            let current = slot.as_ref().and_then(|s| s.text.clone());
            if current == normalized {
                return false;
            }
            write_endpoint_field(slot, normalized, |s, v| s.text = v);
            true
        })
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
        let new_val = color.map(|s| s.to_string());
        self.mutate_edge(edge_ref, |edge, _canvas| {
            let Some(slot) = portal_endpoint_state_mut(edge, endpoint_node_id) else {
                return false;
            };
            let current = slot.as_ref().and_then(|s| s.text_color.clone());
            if current == new_val {
                return false;
            }
            write_endpoint_field(slot, new_val, |s, v| s.text_color = v);
            true
        })
    }

    /// Read the current portal label text for one endpoint, if
    /// any. Returns the concrete string (not the hex-color
    /// cascade like [`Self::resolve_portal_label_color`]) —
    /// portal text has no inheritance cascade, it's either set
    /// on the endpoint or absent.
    pub fn portal_label_text(&self, edge_ref: &EdgeRef, endpoint_node_id: &str) -> Option<String> {
        let edge = self.mindmap.edges.iter().find(|e| edge_ref.matches(e))?;
        let state = baumhard::mindmap::model::portal_endpoint_state(edge, endpoint_node_id)?;
        state.text.clone()
    }

    /// Read the resolved portal label (icon) color for one
    /// endpoint. Walks
    /// [`baumhard::mindmap::model::MindEdge::portal_endpoint_color`]
    /// — per-endpoint override > the edge body cascade — and
    /// expands any `var(--name)` reference through the theme
    /// variable map. Used by clipboard copy: the user expects
    /// `copy` on a portal label to produce a real hex they can
    /// paste elsewhere, even when no override is set.
    ///
    /// Reads the model cascade directly rather than calling the
    /// scene builder's `resolve_portal_endpoint_style`: that
    /// resolver also does glyph substitution and zoom-clamped
    /// font sizing, none of which a color read needs, and its
    /// color tier is now the same model helper.
    pub fn resolve_portal_label_color(&self, edge_ref: &EdgeRef, endpoint_node_id: &str) -> Option<String> {
        let edge = self.mindmap.edges.iter().find(|e| edge_ref.matches(e))?;
        let endpoint_state = baumhard::mindmap::model::portal_endpoint_state(edge, endpoint_node_id);
        let raw = edge.portal_endpoint_color(&self.mindmap.canvas, endpoint_state);
        Some(baumhard::util::color::resolve_var(raw, &self.mindmap.canvas.theme_variables).to_string())
    }
}
