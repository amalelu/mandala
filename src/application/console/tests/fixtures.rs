//! Shared fixtures for the console test suite. Used by every sibling
//! test module via `use super::fixtures::*;`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use baumhard::mindmap::loader;

use crate::application::console::parser::{parse, Args, ParseResult};
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::document::{EdgeRef, MindMapDocument, SelectionState};

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
        color_picker_preview: None,
        active_animations: Vec::new(),
    };
    doc.build_mutation_registry();
    doc
}

/// Pick the first edge in the map and point the selection at it.
/// Returns the edge ref so tests can assert against the mutated
/// fields afterwards.
pub(super) fn select_first_edge(doc: &mut MindMapDocument) -> EdgeRef {
    let edge = doc.mindmap.edges[0].clone();
    let er = EdgeRef::new(&edge.from_id, &edge.to_id, &edge.edge_type);
    doc.selection = SelectionState::Edge(er.clone());
    er
}

/// Parse `line`, run the resolved command against `doc`, and return
/// the `ExecResult`. Panics on parse failure — these are unit tests
/// with known-good input.
pub(super) fn run(line: &str, doc: &mut MindMapDocument) -> ExecResult {
    let (cmd, tokens) = match parse(line) {
        ParseResult::Ok { cmd, args } => (cmd, args),
        ParseResult::Empty => panic!("empty input: {:?}", line),
        ParseResult::Unknown(s) => panic!("unknown command '{}' in {:?}", s, line),
    };
    let mut eff = ConsoleEffects::new(doc);
    (cmd.execute)(&Args::new(&tokens), &mut eff)
}

