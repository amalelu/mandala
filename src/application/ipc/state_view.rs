// SPDX-License-Identifier: MPL-2.0

//! Serializable view types for the `/state` endpoints. Built per
//! frame from the live `InitState` / `CoreState`, stored in
//! `Arc<RwLock<StateSnapshot>>`, read by HTTP handlers without
//! touching the main thread.
//!
//! We use view types rather than deriving `Serialize` on the
//! domain types: domain types carry borrows / foreign deps
//! (`Box<dyn Fn>`, `winit::Window`, `wgpu::Surface`, arena handles)
//! and `Serialize` would either fail to derive or leak
//! implementation details. View types let the wire format evolve
//! independently of the model.

use serde::Serialize;

/// Top-level snapshot. Rebuilt at the end of every drain frame
/// (windowed) or every tick (headless). Routes that need a
/// substate read this and downsample.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct StateSnapshot {
    pub document: DocumentView,
    pub selection: SelectionView,
    pub interaction_mode: InteractionModeView,
    pub camera: CameraView,
    pub console: ConsoleView,
    pub modifiers: ModifiersView,
    pub hovered_node: Option<String>,
    pub cursor_pos: (f64, f64),
    pub drag_kind: String,
    pub label_edit: Option<LabelEditView>,
    pub portal_text_edit: Option<PortalTextEditView>,
    pub text_edit: Option<TextEditView>,
    pub color_picker: Option<ColorPickerView>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct DocumentView {
    pub mindmap: Option<serde_json::Value>,
    pub file_path: Option<String>,
    pub dirty: bool,
    pub undo_depth: usize,
    pub active_animation_count: usize,
}

/// Mirror of `crate::application::document::SelectionState`,
/// flattened to a tagged enum that's stable across the IPC boundary.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SelectionView {
    None,
    Single { id: String },
    Multi { ids: Vec<String> },
    Section { node_id: String, section_idx: usize },
    MultiSection { sections: Vec<SectionSelView> },
    SectionRange {
        sel: SectionSelView,
        range_lo: usize,
        range_hi: usize,
    },
    Edge { from_id: String, to_id: String, edge_type: String },
    EdgeLabel { from_id: String, to_id: String, edge_type: String },
    PortalLabel { from_id: String, to_id: String, edge_type: String, endpoint_node_id: String },
    PortalText { from_id: String, to_id: String, edge_type: String, endpoint_node_id: String },
}

impl Default for SelectionView {
    fn default() -> Self {
        Self::None
    }
}

impl SelectionView {
    /// Project a live [`SelectionState`](crate::application::document::SelectionState)
    /// into the wire-format view. The view flattens edge identities
    /// (`EdgeRef` / `EdgeKey`) into three explicit fields so clients
    /// don't need to know about the two internal representations.
    pub fn from_state(s: &crate::application::document::SelectionState) -> Self {
        use crate::application::document::SelectionState as S;
        match s {
            S::None => Self::None,
            S::Single(id) => Self::Single { id: id.clone() },
            S::Multi(ids) => Self::Multi { ids: ids.clone() },
            S::Section(sel) => Self::Section {
                node_id: sel.node_id.clone(),
                section_idx: sel.section_idx,
            },
            S::MultiSection(secs) => Self::MultiSection {
                sections: secs
                    .iter()
                    .map(|s| SectionSelView {
                        node_id: s.node_id.clone(),
                        section_idx: s.section_idx,
                    })
                    .collect(),
            },
            S::SectionRange {
                sel,
                range: (lo, hi),
            } => Self::SectionRange {
                sel: SectionSelView {
                    node_id: sel.node_id.clone(),
                    section_idx: sel.section_idx,
                },
                range_lo: *lo,
                range_hi: *hi,
            },
            S::Edge(er) => Self::Edge {
                from_id: er.from_id.clone(),
                to_id: er.to_id.clone(),
                edge_type: er.edge_type.clone(),
            },
            S::EdgeLabel(sel) => Self::EdgeLabel {
                from_id: sel.edge_ref.from_id.clone(),
                to_id: sel.edge_ref.to_id.clone(),
                edge_type: sel.edge_ref.edge_type.clone(),
            },
            S::PortalLabel(sel) => {
                let er = sel.edge_ref();
                Self::PortalLabel {
                    from_id: er.from_id,
                    to_id: er.to_id,
                    edge_type: er.edge_type,
                    endpoint_node_id: sel.endpoint_node_id.clone(),
                }
            }
            S::PortalText(sel) => {
                let er = sel.edge_ref();
                Self::PortalText {
                    from_id: er.from_id,
                    to_id: er.to_id,
                    edge_type: er.edge_type,
                    endpoint_node_id: sel.endpoint_node_id.clone(),
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SectionSelView {
    pub node_id: String,
    pub section_idx: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InteractionModeView {
    #[default]
    Idle,
    Reparent,
    Connect,
    Resize,
    NodeEdit,
    Other { name: String },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct CameraView {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ConsoleView {
    pub open: bool,
    pub input: String,
    pub cursor: usize,
    pub scrollback: Vec<ConsoleLineView>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ConsoleLineView {
    pub kind: String,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ModifiersView {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub super_: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LabelEditView {
    pub target_edge: Option<String>,
    pub buffer: String,
    pub cursor: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PortalTextEditView {
    pub target_edge: Option<String>,
    pub endpoint_node_id: Option<String>,
    pub buffer: String,
    pub cursor: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TextEditView {
    pub target_id: Option<String>,
    pub buffer: String,
    pub cursor: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ColorPickerView {
    pub axis: Option<String>,
    pub value: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_snapshot_serialises() {
        let snap = StateSnapshot::default();
        let v = serde_json::to_value(&snap).expect("serialise");
        // Sanity: every top-level key present.
        for key in [
            "document",
            "selection",
            "interaction_mode",
            "camera",
            "console",
            "modifiers",
            "hovered_node",
            "cursor_pos",
            "drag_kind",
            "label_edit",
            "portal_text_edit",
            "text_edit",
            "color_picker",
        ] {
            assert!(
                v.get(key).is_some(),
                "missing top-level key {key}: {v}"
            );
        }
    }

    #[test]
    fn test_selection_view_serialises_with_kind_tag() {
        for view in [
            SelectionView::None,
            SelectionView::Single { id: "a".into() },
            SelectionView::Multi {
                ids: vec!["a".into(), "b".into()],
            },
            SelectionView::Section {
                node_id: "n".into(),
                section_idx: 0,
            },
            SelectionView::Edge {
                from_id: "a".into(),
                to_id: "b".into(),
                edge_type: "parent_child".into(),
            },
        ] {
            let v = serde_json::to_value(&view).expect("serialise");
            assert!(v.get("kind").is_some(), "missing kind: {v}");
        }
    }

    #[test]
    fn test_interaction_mode_default_is_idle() {
        let v = serde_json::to_value(&InteractionModeView::default()).unwrap();
        assert_eq!(v["kind"], "idle");
    }

    #[test]
    fn test_console_view_default_closed() {
        let v = ConsoleView::default();
        assert!(!v.open);
        assert!(v.scrollback.is_empty());
    }

    #[test]
    fn test_selection_view_from_state_covers_every_variant() {
        use crate::application::document::{
            EdgeLabelSel, EdgeRef, SectionSel, SelectionState,
        };
        use baumhard::mindmap::scene_cache::EdgeKey;
        use crate::application::document::PortalLabelSel;

        let er = || EdgeRef::new("a", "b", "parent_child");
        let pls = || PortalLabelSel {
            edge_key: EdgeKey::new("a", "b", "portal"),
            endpoint_node_id: "a".into(),
        };
        let sec = || SectionSel {
            node_id: "n".into(),
            section_idx: 2,
        };

        // (input state, expected view kind tag)
        let cases: Vec<(SelectionState, &'static str)> = vec![
            (SelectionState::None, "none"),
            (SelectionState::Single("a".into()), "single"),
            (SelectionState::Multi(vec!["a".into(), "b".into()]), "multi"),
            (SelectionState::Section(sec()), "section"),
            (
                SelectionState::MultiSection(vec![sec(), sec()]),
                "multi_section",
            ),
            (
                SelectionState::SectionRange {
                    sel: sec(),
                    range: (1, 4),
                },
                "section_range",
            ),
            (SelectionState::Edge(er()), "edge"),
            (
                SelectionState::EdgeLabel(EdgeLabelSel::new(er())),
                "edge_label",
            ),
            (SelectionState::PortalLabel(pls()), "portal_label"),
            (SelectionState::PortalText(pls()), "portal_text"),
        ];

        for (state, expected_kind) in cases {
            let view = SelectionView::from_state(&state);
            let v = serde_json::to_value(&view).expect("serialise");
            assert_eq!(
                v["kind"], expected_kind,
                "kind tag mismatch for state {state:?}; view JSON {v}"
            );
        }
    }

    #[test]
    fn test_selection_view_from_state_preserves_edge_identity() {
        use crate::application::document::{EdgeRef, SelectionState};
        let state = SelectionState::Edge(EdgeRef::new("foo", "bar", "parent_child"));
        let view = SelectionView::from_state(&state);
        match view {
            SelectionView::Edge {
                from_id,
                to_id,
                edge_type,
            } => {
                assert_eq!(from_id, "foo");
                assert_eq!(to_id, "bar");
                assert_eq!(edge_type, "parent_child");
            }
            other => panic!("expected Edge variant, got {other:?}"),
        }
    }
}
