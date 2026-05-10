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

/// Mirror of `crate::application::document::types::SelectionState`,
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
}
