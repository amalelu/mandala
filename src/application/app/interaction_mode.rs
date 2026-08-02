// SPDX-License-Identifier: MPL-2.0

//! `InteractionMode` — the high-level interaction mode for the application.
//!
//! Drives:
//! - which clicks are absorbed (Reparent / Connect intercept the next
//!   left-click; NodeEdit reroutes section clicks; Resize gates the
//!   handle hit-test).
//! - which mode-gated chrome is rendered (resize anchors, section frames,
//!   mode-specific highlight colors).
//! - how a `SelectionState` click resolves (e.g. a click on a section-area
//!   in NodeEdit produces `SelectionState::Section`; in Default it folds
//!   to `SelectionState::Single`).
//!
//! Cross-platform. The enum carries no GPU handles; it depends only on
//! owned `String` ids and the value types defined here, so both targets
//! compile it.
//!
//! `SectionEdit` is intentionally **not** a variant. Section-text editing
//! is carried by `TextEditState::Open { node_id, section_idx, .. }` in
//! the modal-stealer cascade. The invariant `TextEditState::Open` ⇒
//! `InteractionMode::NodeEdit { node_id }` (matching id) is enforced by
//! the dispatcher arms that open / close the editor; this module only
//! defines the mode shape.
//!
//! The `click_resolves_to_section` predicate is consumed by
//! `app/click.rs` per-frame to route clicks to the active section
//! while in NodeEdit mode.

/// What the user is doing right now at the canvas level.
///
/// One of these is always active. Transitions go through `Action`
/// dispatch — there is no path that flips a mode by side-effect.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum InteractionMode {
    /// Default canvas mode. Click selects, drag moves, no mode-gated chrome.
    Default,

    /// User is choosing a new parent for `sources`. The next left-click on
    /// a node attaches them; left-click on empty canvas promotes them to
    /// root; Esc cancels. Direct migration from `AppMode::Reparent`.
    Reparent {
        sources: Vec<String>,
    },

    /// User is drawing a new cross_link edge from `source`. The next
    /// left-click on a target node creates the edge; left-click on empty
    /// canvas cancels. Esc also cancels. Direct migration from
    /// `AppMode::Connect`.
    Connect {
        source: String,
    },

    /// Editing the contents of `node_id`. Section-area clicks select the
    /// hit section (per `SelectionState::Section`); section drags move
    /// the section relative to the node. Section-text editing is reached
    /// by opening `TextEditState::Open` on a section while in this mode.
    /// Out-of-AABB click exits to `Default`.
    ///
    /// Wired in Batch 3 of the plan.
    NodeEdit {
        node_id: String,
    },

    /// Resize anchors are visible on `target`. Anchor drag transitions
    /// `DragState` through the existing `Throttled(NodeResize)` /
    /// `Throttled(SectionResize)` gestures. Click on the body (not on
    /// an anchor) exits to `Default`. Esc also exits.
    ///
    /// Wired in Batch 2 of the plan.
    Resize {
        target: ResizeTarget,
    },
}

/// What a `Resize` mode targets — a whole node or one section of one node.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResizeTarget {
    Node(String),
    Section { node_id: String, section_idx: usize },
}

impl Default for InteractionMode {
    fn default() -> Self {
        InteractionMode::Default
    }
}

impl InteractionMode {
    /// True when this mode wants to absorb the next left-click as a
    /// mode-specific gesture rather than letting the standard click
    /// router handle it.
    ///
    /// Reparent / Connect intercept their next click as a "choose
    /// target" gesture. Resize / NodeEdit / Default do not intercept;
    /// Resize relies on a separate handle-hit-test path
    /// (`event_mouse_click.rs`) gated on `resize_handle_*()` rather
    /// than on a click intercept.
    pub fn intercepts_left_click(&self) -> bool {
        matches!(
            self,
            InteractionMode::Reparent { .. } | InteractionMode::Connect { .. }
        )
    }

    /// True when a click on a section-area should produce
    /// `SelectionState::Section { node_id, section_idx }` rather than
    /// folding to `SelectionState::Single(node_id)`.
    ///
    /// In `Default` mode, every section-click folds to the owning
    /// `Single(node)` (single-section nodes via the `hit_test_target`
    /// fold; multi-section nodes via the click router's mode gate).
    /// Inside `NodeEdit { node_id }` for the matching node, clicks on
    /// section-areas resolve to `Section`. Consumed by Batch 3's
    /// rewrite of `app/click.rs`'s click-routing fork.
    pub fn click_resolves_to_section(&self, hit_node: &str) -> bool {
        match self {
            InteractionMode::NodeEdit { node_id } => node_id == hit_node,
            _ => false,
        }
    }

    /// The node that should receive auto-emitted resize handles this
    /// frame, or `None` if no node should. Read by the projection
    /// gate in `document/mod.rs::frame_overrides` (via
    /// `resize_handle_overrides()`).
    pub fn resize_handle_node(&self) -> Option<&str> {
        match self {
            InteractionMode::Resize {
                target: ResizeTarget::Node(id),
            } => Some(id.as_str()),
            _ => None,
        }
    }

    /// The section that should receive auto-emitted resize handles this
    /// frame, or `None` if no section should. Companion to
    /// `resize_handle_node`.
    pub fn resize_handle_section(&self) -> Option<(&str, usize)> {
        match self {
            InteractionMode::Resize {
                target: ResizeTarget::Section { node_id, section_idx },
            } => Some((node_id.as_str(), *section_idx)),
            _ => None,
        }
    }

    /// True if this mode is `Reparent` or `Connect` — the two pre-existing
    /// modes that capture the next click as a "choose target" gesture.
    /// Used by handlers that previously matched on `AppMode::Reparent { .. } |
    /// AppMode::Connect { .. }` to update hover highlights.
    pub fn is_target_picker(&self) -> bool {
        matches!(self, InteractionMode::Reparent { .. } | InteractionMode::Connect { .. })
    }

    /// The active NodeEdit target, or `None` for any non-NodeEdit
    /// mode. Drives inactive-node dimming: every node other than
    /// this one renders its frame (border pass) and its text
    /// (`apply_inactive_node_dimming` overlay) at half alpha while
    /// NodeEdit is open. Read by the projection gate in
    /// `document/mod.rs::frame_overrides` (via
    /// `resize_handle_overrides()`, which packs both the resize
    /// handle target and the dimming target into one bundle).
    pub fn node_edit_for(&self) -> Option<&str> {
        match self {
            InteractionMode::NodeEdit { node_id } => Some(node_id.as_str()),
            _ => None,
        }
    }

    /// Resolve this mode into the `InteractionModeOverrides` value the
    /// scene-builder consumes. Single source of truth for "what
    /// mode-driven chrome should this frame emit?" — every
    /// scene-rebuild call site reads through this method rather
    /// than reaching for the per-field predicates separately.
    pub fn resize_handle_overrides(
        &self,
    ) -> baumhard::mindmap::tree_builder::InteractionModeOverrides<'_> {
        baumhard::mindmap::tree_builder::InteractionModeOverrides {
            node: self.resize_handle_node(),
            section: self.resize_handle_section(),
            node_edit_for: self.node_edit_for(),
            // `focused_section` is filled by the application's
            // text-edit-aware caller (e.g. drain_frame.rs reads it
            // from `TextEditState::Open`); the mode predicate alone
            // doesn't know about open editors.
            focused_section: None,
        }
    }
}

/// Why a `SelectionState` could not be resolved into a `ResizeTarget`.
/// Distinguishing the failure mode lets the caller (a console verb,
/// the dispatch arm) format an appropriate user-facing message
/// without re-walking the selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResizeTargetError {
    /// `SelectionState::None`.
    NoSelection,
    /// `SelectionState::Multi` or `MultiSection` — Resize mode is
    /// single-target by design.
    MultiTarget,
    /// Section selected, but `section.size == None` (fill-parent) —
    /// no own AABB to stretch.
    SectionFillParent {
        node_id: String,
        section_idx: usize,
    },
    /// Edge / label / portal selection — not resizable surface.
    EdgeOrPortal,
}

/// Resolve a [`SelectionState`] into a [`ResizeTarget`].
///
/// Single source of truth shared by [`Action::EnterResizeMode`]'s
/// dispatch arm (`apply_enter_resize_mode`) and the `mode resize`
/// console verb. Pre-fix Batch 2 of
/// `SECTIONS_BORDERS_RESIZE_PLAN.md` had this logic duplicated
/// across both call sites, with subtly different error wording.
///
/// Reads only the document's selection + the `MindMap` model
/// (specifically section sizes). Cross-platform — no GPU, no
/// console, no renderer.
///
/// [`SelectionState`]: crate::application::document::SelectionState
/// [`Action::EnterResizeMode`]: crate::application::keybinds::Action::EnterResizeMode
pub fn resolve_resize_target(
    selection: &crate::application::document::SelectionState,
    map: &baumhard::mindmap::model::MindMap,
) -> Result<ResizeTarget, ResizeTargetError> {
    use crate::application::document::SelectionState;

    match selection {
        SelectionState::Single(node_id) => Ok(ResizeTarget::Node(node_id.clone())),
        SelectionState::Section(s) | SelectionState::SectionRange { sel: s, .. } => {
            let section_size = map
                .nodes
                .get(&s.node_id)
                .and_then(|n| n.sections.get(s.section_idx))
                .and_then(|sec| sec.size);
            if section_size.is_none() {
                Err(ResizeTargetError::SectionFillParent {
                    node_id: s.node_id.clone(),
                    section_idx: s.section_idx,
                })
            } else {
                Ok(ResizeTarget::Section {
                    node_id: s.node_id.clone(),
                    section_idx: s.section_idx,
                })
            }
        }
        SelectionState::None => Err(ResizeTargetError::NoSelection),
        SelectionState::Multi(_) | SelectionState::MultiSection(_) => Err(ResizeTargetError::MultiTarget),
        SelectionState::Edge(_)
        | SelectionState::EdgeLabel(_)
        | SelectionState::PortalLabel(_)
        | SelectionState::PortalText(_) => Err(ResizeTargetError::EdgeOrPortal),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mode_is_default() {
        assert_eq!(InteractionMode::default(), InteractionMode::Default);
    }

    #[test]
    fn default_mode_does_not_intercept_or_pick_targets() {
        let m = InteractionMode::Default;
        assert!(!m.intercepts_left_click());
        assert!(!m.click_resolves_to_section("any"));
        assert_eq!(m.resize_handle_node(), None);
        assert_eq!(m.resize_handle_section(), None);
        assert!(!m.is_target_picker());
    }

    #[test]
    fn reparent_intercepts_and_is_target_picker() {
        let m = InteractionMode::Reparent {
            sources: vec!["0".into()],
        };
        assert!(m.intercepts_left_click());
        assert!(m.is_target_picker());
        assert!(!m.click_resolves_to_section("0"));
        assert_eq!(m.resize_handle_node(), None);
    }

    #[test]
    fn connect_intercepts_and_is_target_picker() {
        let m = InteractionMode::Connect { source: "0".into() };
        assert!(m.intercepts_left_click());
        assert!(m.is_target_picker());
        assert!(!m.click_resolves_to_section("0"));
        assert_eq!(m.resize_handle_node(), None);
    }

    #[test]
    fn node_edit_routes_clicks_to_section_for_matching_node_only() {
        let m = InteractionMode::NodeEdit { node_id: "0.1".into() };
        assert!(m.click_resolves_to_section("0.1"));
        assert!(!m.click_resolves_to_section("0"));
        assert!(!m.click_resolves_to_section("0.2"));
        assert!(!m.intercepts_left_click());
        assert!(!m.is_target_picker());
        assert_eq!(m.resize_handle_node(), None);
        assert_eq!(m.resize_handle_section(), None);
    }

    #[test]
    fn resize_node_target_resolves_to_node_handles_only() {
        let m = InteractionMode::Resize {
            target: ResizeTarget::Node("0".into()),
        };
        assert_eq!(m.resize_handle_node(), Some("0"));
        assert_eq!(m.resize_handle_section(), None);
        assert!(!m.click_resolves_to_section("0"));
        assert!(!m.is_target_picker());
    }

    #[test]
    fn resize_section_target_resolves_to_section_handles_only() {
        let m = InteractionMode::Resize {
            target: ResizeTarget::Section {
                node_id: "0".into(),
                section_idx: 1,
            },
        };
        assert_eq!(m.resize_handle_node(), None);
        assert_eq!(m.resize_handle_section(), Some(("0", 1)));
    }

    #[test]
    fn resize_target_node_and_section_are_distinct() {
        let n = ResizeTarget::Node("0".into());
        let s = ResizeTarget::Section { node_id: "0".into(), section_idx: 0 };
        assert_ne!(n, s);
    }
}
