//! Portal mutations — create, delete, edit, plus the two
//! field-specific setters (`set_portal_glyph` /
//! `set_portal_color`) that wrap `apply_edit_portal` for the
//! console / keyboard call sites.

use baumhard::mindmap::model::{PortalPair, Position, PORTAL_GLYPH_PRESETS};

use super::types::PortalRef;
use super::undo_action::UndoAction;
use super::MindMapDocument;

impl MindMapDocument {
    // Session 6E — portal mutation helpers
    // ============================================================

    /// Create a new portal pair linking `node_a` and `node_b`.
    ///
    /// Fails (returns `Err`) if the two ids are identical or if either
    /// node is missing from `mindmap.nodes` — defense in depth, since
    /// the 2-node `Multi` selection path in the palette already rules
    /// out these cases in normal use.
    ///
    /// The new portal gets the lowest unused label in column-letter
    /// order (A, B, ..., Z, AA, ...) from `MindMap::next_portal_label`,
    /// and its glyph is picked by rotating through
    /// `PORTAL_GLYPH_PRESETS` so each new pair looks distinct at a
    /// glance. The color defaults to the same `#aa88cc` used by
    /// `default_cross_link_edge`.
    ///
    /// Pushes `UndoAction::CreatePortal { index }`, marks the document
    /// dirty, and returns a fresh `PortalRef` identifying the new pair.
    pub fn apply_create_portal(
        &mut self,
        node_a: &str,
        node_b: &str,
    ) -> Result<PortalRef, String> {
        if node_a == node_b {
            return Err("cannot create a portal between a node and itself".to_string());
        }
        if !self.mindmap.nodes.contains_key(node_a) {
            return Err(format!("unknown node id: {node_a}"));
        }
        if !self.mindmap.nodes.contains_key(node_b) {
            return Err(format!("unknown node id: {node_b}"));
        }
        let label = self.mindmap.next_portal_label();
        let glyph_idx = self.mindmap.portals.len() % PORTAL_GLYPH_PRESETS.len();
        let portal = PortalPair {
            endpoint_a: node_a.to_string(),
            endpoint_b: node_b.to_string(),
            label: label.clone(),
            glyph: PORTAL_GLYPH_PRESETS[glyph_idx].to_string(),
            color: "#aa88cc".to_string(),
            font_size_pt: 16.0,
            font: None,
        };
        let index = self.mindmap.portals.len();
        let pref = PortalRef::from_portal(&portal);
        self.mindmap.portals.push(portal);
        self.undo_stack.push(UndoAction::CreatePortal { index });
        self.dirty = true;
        Ok(pref)
    }

    /// Delete the portal pair identified by `portal_ref`. Records a
    /// `DeletePortal` undo entry so Ctrl+Z restores it at the same
    /// index. Returns the removed pair on success, `None` if the ref
    /// did not match any portal.
    pub fn apply_delete_portal(
        &mut self,
        portal_ref: &PortalRef,
    ) -> Option<PortalPair> {
        let idx = self.mindmap.portals.iter().position(|p| portal_ref.matches(p))?;
        let portal = self.mindmap.portals.remove(idx);
        self.undo_stack.push(UndoAction::DeletePortal { index: idx, portal: portal.clone() });
        self.dirty = true;
        Some(portal)
    }

    /// Edit a portal in place via a mutation closure. The pre-edit
    /// snapshot is taken before `f` runs and pushed as
    /// `UndoAction::EditPortal`, so Ctrl+Z restores the original
    /// fields wholesale. Returns `true` if the ref matched a portal.
    ///
    /// Used by `set_portal_glyph` / `set_portal_color` / future field
    /// setters in the same way `apply_edit_portal` is the single
    /// "write + record undo" chokepoint for portal mutations.
    pub fn apply_edit_portal<F>(&mut self, portal_ref: &PortalRef, f: F) -> bool
    where
        F: FnOnce(&mut PortalPair),
    {
        let idx = match self.mindmap.portals.iter().position(|p| portal_ref.matches(p)) {
            Some(i) => i,
            None => return false,
        };
        let before = self.mindmap.portals[idx].clone();
        f(&mut self.mindmap.portals[idx]);
        self.undo_stack.push(UndoAction::EditPortal { index: idx, before });
        self.dirty = true;
        true
    }

    /// Set the visible glyph of a portal pair. Wraps
    /// `apply_edit_portal` so undo works via `EditPortal`.
    pub fn set_portal_glyph(&mut self, portal_ref: &PortalRef, glyph: &str) -> bool {
        let glyph_owned = glyph.to_string();
        self.apply_edit_portal(portal_ref, move |p| p.glyph = glyph_owned)
    }

    /// Set the color of a portal pair. Accepts a raw `#RRGGBB` hex or
    /// a theme-variable reference like `var(--accent)`. The color is
    /// resolved at scene-build time so theme swaps auto-restyle
    /// var-referencing portals.
    pub fn set_portal_color(&mut self, portal_ref: &PortalRef, color: &str) -> bool {
        let color_owned = color.to_string();
        self.apply_edit_portal(portal_ref, move |p| p.color = color_owned)
    }
}
