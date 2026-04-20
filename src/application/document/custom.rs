//! Custom-mutation infrastructure — `apply_custom_mutation` and
//! its helpers. The bridge between the declarative
//! `CustomMutation` shape and the document's mutation-and-undo
//! plumbing.

use baumhard::mindmap::custom_mutation::{
    apply_mutations_to_element, flat_mutations, mutator_reach, CustomMutation, DocumentAction,
    MutationBehavior, TargetScope,
};
use baumhard::mindmap::model::MindNode;
use baumhard::mindmap::tree_builder::MindMapTree;

use super::mutations_loader::MutationSource;
use super::undo_action::UndoAction;
use super::MindMapDocument;

impl MindMapDocument {
    /// `true` when the registered mutation at `mutation_id` will
    /// dispatch through its Rust [`super::mutations::DynamicMutationHandler`]
    /// at apply time. Two conditions must hold:
    ///
    /// - A handler is registered for this id.
    /// - The mutation's source layer is [`MutationSource::App`] — i.e.
    ///   the definition the user sees actually is the one the handler
    ///   was written for. If the user / map / inline layer overrode
    ///   the id, their declarative shape wins and the bundled handler
    ///   is bypassed.
    ///
    /// This prevents a subtle hijack: a user mutation carrying the
    /// same id as a bundled handler (e.g. `"flower-layout"`) would
    /// otherwise win in the registry but still get executed by the
    /// bundled Rust algorithm, silently discarding the user's
    /// declared `mutator` and `target_scope`.
    pub fn will_dispatch_to_handler(&self, mutation_id: &str) -> bool {
        self.mutation_handlers.contains_key(mutation_id)
            && self.mutation_sources.get(mutation_id) == Some(&MutationSource::App)
    }

    /// Apply a custom mutation to the tree and optionally sync to the model.
    /// For Persistent mutations, snapshots affected nodes for undo and sets dirty flag.
    /// For Toggle mutations, tracks active state without model sync.
    ///
    /// The `tree` argument is only consulted on the declarative
    /// flat-apply path. When [`Self::will_dispatch_to_handler`]
    /// returns `true` for `custom.id` the handler mutates the model
    /// directly; callers that know ahead of time the handler will
    /// fire may pass `None` and skip the (expensive) tree build
    /// entirely. Passing `None` on the declarative path logs a
    /// warning and is otherwise a no-op (the mutation isn't applied).
    pub fn apply_custom_mutation(
        &mut self,
        custom: &CustomMutation,
        node_id: &str,
        mut tree: Option<&mut MindMapTree>,
    ) {
        // For toggle behavior, check if already active and reverse if so.
        if custom.behavior == MutationBehavior::Toggle {
            let key = (node_id.to_string(), custom.id.clone());
            if self.active_toggles.contains(&key) {
                // Second trigger: remove from active set. The tree
                // mutation from the first trigger is *not* inverted
                // in place — Mutations aren't guaranteed invertible.
                // The caller is expected to rebuild the tree from
                // the model next frame (the model is untouched
                // because Toggle skips the persistent-path model
                // sync). Console and event-loop callers both rebuild
                // scene state on every dispatch so this is the
                // conventional shape; trigger dispatchers that keep
                // a persistent tree across events must explicitly
                // call `build_tree()` after a toggle-off.
                self.active_toggles.remove(&key);
                self.dirty = true;
                return;
            }
            self.active_toggles.insert(key);
            // Toggle mutations apply to tree only (visual), no model sync.
            if let Some(tree) = tree.as_deref_mut() {
                self.apply_to_tree(custom, node_id, tree);
            } else {
                log::warn!(
                    "apply_custom_mutation: Toggle mutation '{}' called with None tree; \
                     visual toggle skipped. Pass Some(&mut tree) for Toggle mutations.",
                    custom.id
                );
            }
            return;
        }

        // Authoring-mistake guard: if the mutator AST walks deeper
        // than `target_scope` snapshots, undo will silently miss
        // the deeper edits. Log a `warn!` — still applies the
        // mutation (the author may have their own reasons, or the
        // warning may surface a bug worth fixing) but flags the
        // scope mismatch so it doesn't pass unnoticed.
        if let Some(mutator) = custom.mutator.as_ref() {
            let reach = mutator_reach(mutator);
            if !custom.target_scope.covers_reach(reach) {
                log::warn!(
                    "mutation '{}': mutator reach is {:?} but target_scope is \
                     {:?}; undo will not capture edits beyond the declared scope",
                    custom.id,
                    reach,
                    custom.target_scope
                );
            }
        }

        // Persistent: snapshot, apply, sync/push undo.
        let affected_ids = self.collect_affected_node_ids(node_id, &custom.target_scope);
        let snapshots: Vec<(String, MindNode)> = affected_ids
            .iter()
            .filter_map(|id| self.mindmap.nodes.get(id).map(|n| (id.clone(), n.clone())))
            .collect();

        // Handler dispatch: only fires when the mutation at this id
        // actually came from the app bundle — a user / map / inline
        // override of the same id keeps the declarative path so the
        // user's mutator is honoured. See
        // [`Self::will_dispatch_to_handler`] for the rationale.
        if self.will_dispatch_to_handler(&custom.id) {
            if let Some(handler) = self.mutation_handlers.get(&custom.id).copied() {
                handler(self, node_id);
            }
        } else if let Some(tree) = tree.as_deref_mut() {
            self.apply_to_tree(custom, node_id, tree);
            for id in &affected_ids {
                self.sync_node_from_tree(id, tree);
            }
        } else {
            log::warn!(
                "apply_custom_mutation: declarative mutation '{}' called with None tree; \
                 flat-apply skipped. Pass Some(&mut tree) when the mutation isn't \
                 handler-dispatched.",
                custom.id
            );
            // Fall through: document_actions still push undo below,
            // but no tree/model changes occurred.
            return;
        }

        if !snapshots.is_empty() {
            self.undo_stack
                .push(UndoAction::CustomMutation { node_snapshots: snapshots });
            self.dirty = true;
        }
    }

    /// Apply any document-level actions carried by a custom mutation. These
    /// operate on `self.mindmap.canvas` rather than any tree node, so they
    /// run independently of `apply_custom_mutation`'s tree walk. When any
    /// action would actually change state, a `CanvasSnapshot` undo entry is
    /// pushed capturing the pre-action canvas, and the document is marked
    /// dirty. Returns true if the canvas was modified.
    pub fn apply_document_actions(&mut self, custom: &CustomMutation) -> bool {
        if custom.document_actions.is_empty() {
            return false;
        }
        let snapshot = self.mindmap.canvas.clone();
        let mut changed = false;
        for action in &custom.document_actions {
            match action {
                DocumentAction::SetThemeVariant(name) => {
                    if let Some(preset) = self.mindmap.canvas.theme_variants.get(name) {
                        let new_vars = preset.clone();
                        if new_vars != self.mindmap.canvas.theme_variables {
                            self.mindmap.canvas.theme_variables = new_vars;
                            changed = true;
                        }
                    }
                    // Unknown variant: silently ignored (graceful).
                }
                DocumentAction::SetThemeVariables(map) => {
                    for (k, v) in map {
                        let existing = self.mindmap.canvas.theme_variables.get(k);
                        if existing.map(|s| s != v).unwrap_or(true) {
                            self.mindmap.canvas.theme_variables
                                .insert(k.clone(), v.clone());
                            changed = true;
                        }
                    }
                }
            }
        }
        if changed {
            self.undo_stack.push(UndoAction::CanvasSnapshot { canvas: snapshot });
            self.dirty = true;
        }
        changed
    }

    /// Apply a custom mutation's payload to the Baumhard tree, iterating
    /// every affected model node and applying the flat `Vec<Mutation>`
    /// extracted from the MutatorNode to each target element. Mutations
    /// without a `mutator` (document-actions-only) are no-ops here —
    /// [`Self::apply_document_actions`] handles their canvas effects
    /// separately. Mutations whose MutatorNode can't be reduced to a
    /// flat list (runtime-hole-bearing, size-aware) are skipped at this
    /// layer; a later session wires the richer `mutator_builder::build`
    /// path for those.
    fn apply_to_tree(
        &self,
        custom: &CustomMutation,
        node_id: &str,
        tree: &mut MindMapTree,
    ) {
        let Some(mutator) = custom.mutator.as_ref() else { return };
        let Some(mutations) = flat_mutations(mutator) else { return };
        let affected = self.collect_affected_node_ids(node_id, &custom.target_scope);
        for id in &affected {
            if let Some(&nid) = tree.node_map.get(id.as_str()) {
                if let Some(node) = tree.tree.arena.get_mut(nid) {
                    apply_mutations_to_element(&mutations, node.get_mut());
                }
            }
        }
    }

    /// Collect the IDs of all nodes affected by a mutation with the given scope.
    pub(super) fn collect_affected_node_ids(&self, node_id: &str, scope: &TargetScope) -> Vec<String> {
        match scope {
            TargetScope::SelfOnly => vec![node_id.to_string()],
            TargetScope::Children => {
                self.mindmap.children_of(node_id).iter().map(|n| n.id.clone()).collect()
            }
            TargetScope::Descendants => self.mindmap.all_descendants(node_id),
            TargetScope::SelfAndDescendants => {
                let mut ids = vec![node_id.to_string()];
                ids.extend(self.mindmap.all_descendants(node_id));
                ids
            }
            TargetScope::Parent => {
                self.mindmap.nodes.get(node_id)
                    .and_then(|n| n.parent_id.clone())
                    .into_iter().collect()
            }
            TargetScope::Siblings => {
                self.mindmap.nodes.get(node_id)
                    .and_then(|n| n.parent_id.as_deref())
                    .map(|pid| {
                        self.mindmap.children_of(pid).iter()
                            .filter(|n| n.id != node_id)
                            .map(|n| n.id.clone())
                            .collect()
                    })
                    .unwrap_or_default()
            }
        }
    }

    /// Sync a node's position from the Baumhard tree back to the MindMap model.
    /// Used after persistent mutations to ensure the model reflects tree state.
    fn sync_node_from_tree(&mut self, node_id: &str, tree: &MindMapTree) {
        let tree_nid = match tree.node_map.get(node_id) {
            Some(&nid) => nid,
            None => return,
        };
        let element = match tree.tree.arena.get(tree_nid) {
            Some(n) => n.get(),
            None => return,
        };
        let area = match element.glyph_area() {
            Some(a) => a,
            None => return,
        };
        if let Some(model_node) = self.mindmap.nodes.get_mut(node_id) {
            model_node.position.x = area.position.x.0 as f64;
            model_node.position.y = area.position.y.0 as f64;
        }
    }
}
