//! Shared fixtures used by the `tests_*` submodules. Kept pub(super)
//! so a single definition covers every themed split without
//! forcing per-file duplication.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use baumhard::mindmap::loader;
use baumhard::mindmap::tree_builder::MindMapTree;

use super::types::SelectionState;
use super::MindMapDocument;

pub(super) fn test_map_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("maps/testament.mindmap.json");
    path
}

pub(super) fn load_test_doc() -> MindMapDocument {
    let map = loader::load_from_file(&test_map_path()).unwrap();
    let mut doc = MindMapDocument {
        mindmap: map,
        file_path: None,
        dirty: false,
        selection: SelectionState::None,
        undo_stack: Vec::new(),
        mutation_registry: HashMap::new(),
        active_toggles: HashSet::new(),
        label_edit_preview: None,
        portal_text_edit_preview: None,
        color_picker_preview: None,
        active_animations: Vec::new(),
    };
    doc.build_mutation_registry();
    doc
}

pub(super) fn load_test_tree() -> MindMapTree {
    load_test_doc().build_tree()
}

/// Pick a stable node id from the testament map that has a real
/// text value. The root node id is well-known from other tests.
pub(super) fn first_testament_node_id(_doc: &MindMapDocument) -> String {
    "0".to_string()
}

/// Pick the first visible edge and return its EdgeRef + a guaranteed
/// on-path sample point. Used by hit-test edge tests.
pub(super) fn pick_test_edge(doc: &MindMapDocument) -> (super::EdgeRef, glam::Vec2) {
    use glam::Vec2;
    let edge = doc.mindmap.edges.iter()
        .find(|e| e.visible)
        .expect("testament map has visible edges");
    let from = doc.mindmap.nodes.get(&edge.from_id).unwrap();
    let to = doc.mindmap.nodes.get(&edge.to_id).unwrap();
    let from_pos = Vec2::new(from.position.x as f32, from.position.y as f32);
    let from_size = Vec2::new(from.size.width as f32, from.size.height as f32);
    let to_pos = Vec2::new(to.position.x as f32, to.position.y as f32);
    let to_size = Vec2::new(to.size.width as f32, to.size.height as f32);
    let path = baumhard::mindmap::connection::build_connection_path(
        from_pos, from_size, &edge.anchor_from,
        to_pos, to_size, &edge.anchor_to,
        &edge.control_points,
    );
    let samples = baumhard::mindmap::connection::sample_path(&path, 4.0);
    let midpoint = samples[samples.len() / 2].position;
    let edge_ref = super::EdgeRef::new(&edge.from_id, &edge.to_id, &edge.edge_type);
    (edge_ref, midpoint)
}

/// Grab the first edge from the testament map and return its EdgeRef.
pub(super) fn first_testament_edge_ref(doc: &MindMapDocument) -> super::EdgeRef {
    let e = doc.mindmap.edges.first().expect("testament map has edges");
    super::EdgeRef::new(&e.from_id, &e.to_id, &e.edge_type)
}
