// SPDX-License-Identifier: MPL-2.0

//! Free-fn body helpers reachable from closures passed into
//! [`super::super::MindMapDocument::mutate_edge`] without having
//! to capture `Self`. The corresponding methods on
//! [`super::super::MindMapDocument`] (`ensure_glyph_connection`,
//! `ensure_label_config`) delegate here.

use baumhard::mindmap::model::{
    Canvas, EdgeLabelConfig, GlyphConnectionConfig, MindEdge, PortalEndpointState,
};

/// Fork-on-first-edit for the connection style, callable from
/// closures passed into
/// [`super::super::MindMapDocument::mutate_edge`] without having
/// to capture `Self`. Forks the canvas-default connection style
/// onto the edge on first edit; subsequent edits reuse the
/// per-edge copy. Must be called AFTER the caller has snapshotted
/// the edge into an `UndoAction::EditEdge { before, .. }` so the
/// undo entry still carries the pre-fork `None`.
pub(super) fn ensure_glyph_connection_inline<'a>(
    edge: &'a mut MindEdge,
    canvas: &Canvas,
) -> &'a mut GlyphConnectionConfig {
    if edge.glyph_connection.is_none() {
        let seed = canvas.default_connection.clone().unwrap_or_default();
        edge.glyph_connection = Some(seed);
    }
    edge.glyph_connection.as_mut().expect("just installed")
}

/// Fork-on-first-edit for the label config, callable from closures
/// passed into
/// [`super::super::MindMapDocument::mutate_edge`]. Forks a default
/// [`EdgeLabelConfig`] onto the edge on first label edit;
/// subsequent edits reuse it. Mirrors
/// [`ensure_glyph_connection_inline`] for the label channel.
pub(super) fn ensure_label_config_inline(edge: &mut MindEdge) -> &mut EdgeLabelConfig {
    edge.label_config.get_or_insert_with(EdgeLabelConfig::default)
}

/// Write `value` to the `Option<T>` field on the
/// `PortalEndpointState` slot, lazily forking a default state on
/// `Some` and scrubbing the slot back to `None` on `Some(field) ->
/// None` clears that leave the state entirely default.
///
/// `setter` is given `(&mut state, value)` and writes the field
/// directly (`s.color = v`, `s.text = v`, etc.). The "scrub when
/// default" rollback ensures unchanged selections leave no undo
/// droppings — the rule the pre-helper portal setters each
/// hand-rolled with the same `if existing == &PortalEndpointState::
/// default() { *slot = None; }` block.
///
/// Caller is responsible for the equality / no-op short-circuit
/// before invoking — this helper unconditionally writes. Pairs
/// with [`super::super::MindMapDocument::mutate_edge`]: the
/// closure does
/// `if current == new_val { false } else { write_endpoint_field(...); true }`.
pub(super) fn write_endpoint_field<T, S>(slot: &mut Option<PortalEndpointState>, value: Option<T>, setter: S)
where
    S: FnOnce(&mut PortalEndpointState, Option<T>),
{
    match value {
        Some(v) => {
            let s = slot.get_or_insert_with(PortalEndpointState::default);
            setter(s, Some(v));
        }
        None => {
            if let Some(existing) = slot.as_mut() {
                setter(existing, None);
                if existing == &PortalEndpointState::default() {
                    *slot = None;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `write_endpoint_field` scrubs the slot back to `None` when
    /// a `None` write would leave the state entirely default —
    /// the "no undo droppings on unchanged selections" contract.
    #[test]
    fn write_endpoint_field_scrubs_default_state_on_clear() {
        let mut slot: Option<PortalEndpointState> = Some(PortalEndpointState {
            color: Some("#ff0000".to_string()),
            ..Default::default()
        });
        write_endpoint_field(&mut slot, None::<String>, |s, v| s.color = v);
        assert!(slot.is_none(), "scrub did not collapse to None");
    }

    /// When the slot has *other* fields set, scrubbing one field
    /// to `None` keeps the slot alive — only the all-default
    /// state collapses.
    #[test]
    fn write_endpoint_field_keeps_slot_when_other_fields_remain() {
        let mut slot: Option<PortalEndpointState> = Some(PortalEndpointState {
            color: Some("#ff0000".to_string()),
            text: Some("annotation".to_string()),
            ..Default::default()
        });
        write_endpoint_field(&mut slot, None::<String>, |s, v| s.color = v);
        assert!(slot.is_some(), "slot scrubbed despite remaining fields");
        let s = slot.unwrap();
        assert!(s.color.is_none());
        assert_eq!(s.text.as_deref(), Some("annotation"));
    }

    /// Writing `Some(value)` lazily forks a default slot when
    /// none was set yet — the sibling "lazy install" contract.
    #[test]
    fn write_endpoint_field_lazily_forks_default_slot() {
        let mut slot: Option<PortalEndpointState> = None;
        write_endpoint_field(&mut slot, Some("#abcdef".to_string()), |s, v| s.color = v);
        assert!(slot.is_some());
        assert_eq!(slot.unwrap().color.as_deref(), Some("#abcdef"));
    }
}
