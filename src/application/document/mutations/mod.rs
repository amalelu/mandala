//! Dynamic mutation handlers — imperative Rust implementations of
//! mutations too structural or size-aware to express as a pure-data
//! `MutatorNode` + `flat_mutations` reach.
//!
//! A `DynamicMutationHandler` is a function pointer registered under
//! a mutation's `id`. When `apply_custom_mutation` sees the id, it
//! dispatches to the handler instead of the default flat-apply path.
//! The handler mutates the `MindMap` model directly (positions,
//! sizes, style, …); the renderer rebuilds the tree on the next
//! frame, so there's no tree sync to manage here. The undo path is
//! driven by the mutation's declared `target_scope` — the caller
//! snapshots the affected nodes before the handler runs and pushes a
//! `CustomMutation` undo entry after.
//!
//! Registered at startup by `register_builtin_handlers` (called from
//! both the native and WASM app-init paths), so first-party app
//! mutations that ship in `assets/mutations/application.json` have
//! their Rust implementations wired automatically on both targets.

use super::MindMapDocument;

pub mod flower_layout;
pub mod tree_cascade;

/// Function pointer implementing one dynamic mutation. Takes the
/// document + the target node id (typically the current selection)
/// and mutates the model in place. Returns without a result — the
/// undo path is handled by `apply_custom_mutation` via the
/// mutation's declared `target_scope`.
pub type DynamicMutationHandler = fn(doc: &mut MindMapDocument, target_id: &str);

/// Register every first-party handler shipped with the binary. The
/// registration keys match the ids in
/// `assets/mutations/application.json` so the `mutation apply <id>`
/// console path dispatches to the right handler.
pub fn register_builtin_handlers(doc: &mut MindMapDocument) {
    doc.mutation_handlers
        .insert("flower-layout".to_string(), flower_layout::apply);
    doc.mutation_handlers
        .insert("tree-cascade".to_string(), tree_cascade::apply);
}
