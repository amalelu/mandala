// SPDX-License-Identifier: MPL-2.0

//! What a single-line editor is bound to.
//!
//! [`SingleLineEditTarget`] names one editable string in the model
//! and owns everything that differs between the two things a
//! single-line editor can edit today — a connection's label and a
//! portal endpoint's caption. Where to read the current value,
//! which preview slot feeds the renderer, which canvas role has to
//! be re-projected, which setter commits, and what "the pointer is
//! still on the thing I am editing" means all live here. The
//! lifecycle in [`super::editor`] is written against this surface
//! and knows nothing about edges or portals.

use baumhard::mindmap::scene_cache::EdgeKey;
use glam::Vec2;

use crate::application::document::{EdgeRef, MindMapDocument, SelectionState};
use crate::application::renderer::Renderer;
use crate::application::scene_host::AppScene;

use super::super::scene_rebuild::CanvasFrame;

/// One editable single-line string in the model.
///
/// Adding a third kind of single-line editable text is a variant
/// here plus an arm in each method below — the lifecycle, the modal
/// steal, the click-outside commit and the dispatch arms all stay
/// as they are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::application::app) enum SingleLineEditTarget {
    /// A connection's `MindEdge.label`, rendered along the
    /// connection path by the connection-label pass.
    EdgeLabel { edge_ref: EdgeRef },
    /// One portal endpoint's `PortalEndpointState.text`, rendered
    /// next to that endpoint's marker glyph by the portal pass.
    /// Portal captions are per-endpoint, so the identity needs both
    /// the edge and the endpoint node.
    PortalText {
        edge_ref: EdgeRef,
        endpoint_node_id: String,
    },
}

impl SingleLineEditTarget {
    /// The edge this target hangs off, whichever kind it is.
    pub(in crate::application::app) fn edge_ref(&self) -> &EdgeRef {
        match self {
            SingleLineEditTarget::EdgeLabel { edge_ref }
            | SingleLineEditTarget::PortalText { edge_ref, .. } => edge_ref,
        }
    }

    /// Resolve the target in `doc` at **open** time.
    ///
    /// The outer `None` means the target does not exist — the edge
    /// was deleted, or the endpoint is not an endpoint of that edge
    /// — and the editor must stay closed; this arm logs its own
    /// `warn!` so callers have nothing to report. The inner `None`
    /// means the target exists but carries no text yet.
    pub(in crate::application::app) fn find(&self, doc: &MindMapDocument) -> Option<Option<String>> {
        let edge_ref = self.edge_ref();
        let Some(edge) = doc.mindmap.edges.iter().find(|e| edge_ref.matches(e)) else {
            log::warn!(
                "single-line edit: edge {}→{} ({}) not found; editor stays closed",
                edge_ref.from_id,
                edge_ref.to_id,
                edge_ref.edge_type
            );
            return None;
        };
        match self {
            SingleLineEditTarget::EdgeLabel { .. } => Some(edge.label.clone()),
            SingleLineEditTarget::PortalText { endpoint_node_id, .. } => {
                if endpoint_node_id != &edge.from_id && endpoint_node_id != &edge.to_id {
                    log::warn!(
                        "single-line edit: endpoint {} is neither from ({}) nor to ({}); editor stays closed",
                        endpoint_node_id,
                        edge.from_id,
                        edge.to_id,
                    );
                    return None;
                }
                Some(
                    baumhard::mindmap::model::portal_endpoint_state(edge, endpoint_node_id)
                        .and_then(|s| s.text.clone()),
                )
            }
        }
    }

    /// Is the target still editable **mid-edit**?
    ///
    /// This is deliberately *not* [`Self::find`]. The two kinds
    /// answer differently, and the difference is load-bearing:
    ///
    /// - `PortalText` says no once the edge is gone or has left
    ///   portal mode (an Undo popping the edge, a console command
    ///   flipping `display_mode` to line). A caption on a line-mode
    ///   edge renders nowhere and `set_portal_label_text` would
    ///   silently no-op on commit, so the editor closes without
    ///   committing instead of re-installing a dangling preview on
    ///   every keystroke.
    /// - `EdgeLabel` always says yes. The edge-label editor has
    ///   never had a mid-edit guard: typing continues into a buffer
    ///   whose edge may be gone, and the commit then no-ops inside
    ///   `set_edge_label`. Giving it the portal guard would be a
    ///   user-visible behavior change (a keystroke would start
    ///   discarding the buffer), which is a product decision, not
    ///   part of unifying the two lifecycles — so the asymmetry is
    ///   preserved verbatim and pinned by
    ///   `test_oracle_keystroke_after_edge_deleted_diverges_by_design`.
    pub(in crate::application::app) fn still_editable(&self, doc: &MindMapDocument) -> bool {
        match self {
            SingleLineEditTarget::EdgeLabel { .. } => true,
            SingleLineEditTarget::PortalText {
                edge_ref,
                endpoint_node_id,
            } => doc
                .mindmap
                .edges
                .iter()
                .find(|e| edge_ref.matches(e))
                .map(|e| {
                    baumhard::mindmap::model::is_portal_edge(e)
                        && (endpoint_node_id == &e.from_id || endpoint_node_id == &e.to_id)
                })
                .unwrap_or(false),
        }
    }

    /// Stage `display` (buffer + caret) in the document's preview
    /// slot for this target's canvas role.
    ///
    /// Storing the preview on the document rather than the renderer
    /// is what lets every subsequent `doc.frame_overrides` call pick
    /// it up automatically — no renderer field, no read-time
    /// override, no belt-and-suspenders branch.
    pub(in crate::application::app) fn stage_preview(&self, doc: &mut MindMapDocument, display: String) {
        let edge_key = EdgeKey::from(self.edge_ref());
        match self {
            SingleLineEditTarget::EdgeLabel { .. } => {
                doc.label_edit_preview = Some((edge_key, display));
            }
            SingleLineEditTarget::PortalText { endpoint_node_id, .. } => {
                doc.portal_text_edit_preview = Some((edge_key, endpoint_node_id.clone(), display));
            }
        }
    }

    /// Drop this target's preview slot. Only the slot the target
    /// owns is cleared — the other editor's slot is not this
    /// target's to touch.
    pub(in crate::application::app) fn clear_preview(&self, doc: &mut MindMapDocument) {
        match self {
            SingleLineEditTarget::EdgeLabel { .. } => doc.label_edit_preview = None,
            SingleLineEditTarget::PortalText { .. } => doc.portal_text_edit_preview = None,
        }
    }

    /// Re-project the one canvas role that reads this target's
    /// preview, so the caret and edited text land on the next frame.
    ///
    /// Deliberately narrow: no other role reads either preview slot,
    /// so every other pass stays untouched on every keystroke. That
    /// is the whole "fast label typing" property. Resize-handle
    /// overrides are irrelevant here, hence
    /// `InteractionModeOverrides::none()` rather than threading the
    /// mode through every caller.
    pub(in crate::application::app) fn refresh(
        &self,
        doc: &MindMapDocument,
        app_scene: &mut AppScene,
        renderer: &Renderer,
    ) {
        let offsets = std::collections::HashMap::new();
        let frame = CanvasFrame::new(
            doc,
            &offsets,
            crate::application::document::InteractionModeOverrides::none(),
            renderer.camera_zoom(),
        );
        match self {
            SingleLineEditTarget::EdgeLabel { .. } => frame.update_connection_label_tree(app_scene),
            SingleLineEditTarget::PortalText { .. } => frame.update_portal_tree(app_scene),
        }
    }

    /// Write `value` to the model through the target's setter,
    /// pushing the undo entry the setter owns. Both setters
    /// normalize `Some("")` to `None`, and both no-op (no undo
    /// entry) when the target has since disappeared.
    pub(in crate::application::app) fn commit(&self, doc: &mut MindMapDocument, value: Option<String>) {
        match self {
            SingleLineEditTarget::EdgeLabel { edge_ref } => {
                doc.set_edge_label(edge_ref, value);
            }
            SingleLineEditTarget::PortalText {
                edge_ref,
                endpoint_node_id,
            } => {
                doc.set_portal_label_text(edge_ref, endpoint_node_id, value);
            }
        }
    }

    /// Does a pointer release at `canvas_pt` land back on the
    /// element under edit?
    ///
    /// `true` keeps the editor open (the release is fully consumed);
    /// `false` is the click-outside that commits. The portal arm
    /// matches `(edge, endpoint)` *and* requires the caption
    /// sub-part: releasing on the endpoint's icon is a click-outside
    /// for the caption editor, and releasing on the *other*
    /// endpoint of the same portal edge commits this side before the
    /// click routes as a fresh selection.
    pub(in crate::application::app) fn release_stays_on_target(
        &self,
        app_scene: &mut AppScene,
        canvas_pt: Vec2,
    ) -> bool {
        match self {
            SingleLineEditTarget::EdgeLabel { edge_ref } => app_scene
                .edge_label_at(canvas_pt)
                .map(|hit| key_matches(&hit, edge_ref))
                .unwrap_or(false),
            SingleLineEditTarget::PortalText {
                edge_ref,
                endpoint_node_id,
            } => app_scene
                .portal_at(canvas_pt)
                .map(|hit| {
                    hit.part == baumhard::mindmap::tree_builder::PortalPart::Text
                        && key_matches(&hit.edge_key, edge_ref)
                        && hit.endpoint_node_id == *endpoint_node_id
                })
                .unwrap_or(false),
        }
    }

    /// Does one of the press-time hits name the element under edit?
    ///
    /// Consumes the hits `compute_click_hit` already resolved (with
    /// their full priority chain) rather than re-hit-testing, and
    /// each target kind reads only the hit for its own role. Used by
    /// the double-click guard: re-opening an editor on the element
    /// it is already editing would re-seed the buffer from the
    /// committed model value and silently destroy the in-progress
    /// edit.
    pub(in crate::application::app) fn matches_press_hit(
        &self,
        edge_label_hit: Option<&EdgeKey>,
        portal_text_hit: Option<&(EdgeKey, String)>,
    ) -> bool {
        match self {
            SingleLineEditTarget::EdgeLabel { edge_ref } => {
                edge_label_hit.is_some_and(|hit| key_matches(hit, edge_ref))
            }
            SingleLineEditTarget::PortalText {
                edge_ref,
                endpoint_node_id,
            } => portal_text_hit.is_some_and(|(hit_key, hit_ep)| {
                key_matches(hit_key, edge_ref) && hit_ep == endpoint_node_id
            }),
        }
    }
}

/// `EdgeKey`-vs-`EdgeRef` identity, field by field.
///
/// Deliberately not `EdgeKey::from(edge_ref) == *key`: that form
/// allocates three `String`s per comparison, and these run on the
/// press / release path where CODE_CONVENTIONS §4's mobile budget
/// applies.
fn key_matches(key: &EdgeKey, edge_ref: &EdgeRef) -> bool {
    key.from_id == edge_ref.from_id.as_str()
        && key.to_id == edge_ref.to_id.as_str()
        && key.edge_type == edge_ref.edge_type.as_str()
}

/// Resolve the [`SingleLineEditTarget`] a selection names, or
/// `None` when the selection belongs to the node text editor (or to
/// nothing at all). Pure — no document lookup, no renderer — so the
/// routing is testable without standing up the app.
///
/// Three call sites open a single-line editor off the current
/// selection: `Action::EditSelection` / `EditSelectionClean`'s
/// non-node fall-through, `Action::LabelEditOnSelection`, and the
/// type-to-edit path in `event_keyboard.rs`. They used to carry
/// three copies of this match; this is the one copy.
///
/// `PortalLabel` and `PortalText` both resolve to the caption
/// editor: the label is the endpoint's glyph and the text is its
/// caption, and the editor edits the caption in both cases.
///
/// The match is deliberately **exhaustive** rather than
/// `_ => None`: `SelectionState` has ten variants today, and an
/// eleventh should be a build error here, so whoever adds it has to
/// decide whether it names a single-line editor instead of silently
/// inheriting "no".
pub(in crate::application::app) fn resolve_single_line_target(
    selection: &SelectionState,
) -> Option<SingleLineEditTarget> {
    match selection {
        SelectionState::EdgeLabel(s) => Some(SingleLineEditTarget::EdgeLabel {
            edge_ref: s.edge_ref.clone(),
        }),
        SelectionState::PortalLabel(s) | SelectionState::PortalText(s) => {
            Some(SingleLineEditTarget::PortalText {
                edge_ref: s.edge_ref(),
                endpoint_node_id: s.endpoint_node_id.clone(),
            })
        }
        // Node-scoped selections belong to the node text editor
        // (`apply_enter_node_edit` owns them, cross-platform); a
        // bare `Edge` selection has a body but no label under edit;
        // `None` has no target at all.
        SelectionState::None
        | SelectionState::Single(_)
        | SelectionState::Multi(_)
        | SelectionState::Section(_)
        | SelectionState::MultiSection(_)
        | SelectionState::SectionRange { .. }
        | SelectionState::Edge(_) => None,
    }
}
