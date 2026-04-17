//! Animation runtime — the `MindMapDocument` methods that build
//! the mutation registry, evaluate triggered mutations, start /
//! tick / fast-forward animations. Also carries
//! `apply_position_mutations_to_node` — the scratch-node position
//! replay helper used during tick.

use baumhard::gfx_structs::area::GlyphAreaCommand;
use baumhard::gfx_structs::mutator::Mutation;
use baumhard::mindmap::animation::lerp_f32;
use baumhard::mindmap::custom_mutation::{CustomMutation, PlatformContext, Trigger};
use baumhard::mindmap::model::MindNode;
use baumhard::mindmap::tree_builder::MindMapTree;

use super::types::AnimationInstance;
use super::MindMapDocument;

/// Apply position-bearing `Mutation`s to a `MindNode` to derive
/// the `to` snapshot for an animation. Mirrors the GlyphArea
/// command vocabulary of the existing tree mutator path so that
/// "what does this mutation do" has only one definition,
/// regardless of whether it lands instantly or via tween. v1
/// only handles `NudgeLeft` / `NudgeRight` / `NudgeUp` /
/// `NudgeDown`; other commands are no-ops on the model snapshot
/// (their tree-side effect still runs at completion via
/// `apply_custom_mutation`).
fn apply_position_mutations_to_node(
    mutations: &[Mutation],
    node: &mut MindNode,
) {
    for mutation in mutations {
        if let Mutation::AreaCommand(cmd) = mutation {
            match cmd.as_ref() {
                GlyphAreaCommand::NudgeLeft(dx) => {
                    node.position.x -= *dx as f64;
                }
                GlyphAreaCommand::NudgeRight(dx) => {
                    node.position.x += *dx as f64;
                }
                GlyphAreaCommand::NudgeUp(dy) => {
                    node.position.y -= *dy as f64;
                }
                GlyphAreaCommand::NudgeDown(dy) => {
                    node.position.y += *dy as f64;
                }
                // Other commands don't move the node — their
                // visible effect lands at completion via
                // `apply_custom_mutation`.
                _ => {}
            }
        }
    }
}

/// Source layer a registered mutation came from. Reported by
/// `mutation help <id>` so authors know which file to edit and which
/// layer an override is winning from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MutationSource {
    /// Shipped with the binary via `assets/mutations/application.json`.
    App,
    /// Loaded from the user's `mutations.json` (XDG path on native,
    /// `?mutations=` or localStorage on WASM).
    User,
    /// Declared in the currently-loaded map's `custom_mutations` array.
    Map,
    /// Declared on a specific node's `inline_mutations` array.
    Inline,
}

impl MindMapDocument {
    /// Build the mutation registry from map-level and inline node mutations.
    /// Inline mutations override map-level mutations with the same id.
    pub fn build_mutation_registry(&mut self) {
        self.build_mutation_registry_with_app_and_user(&[], &[]);
    }

    /// Variant retained for callers that already supply a user slice.
    /// Delegates to the four-source builder with an empty app slice.
    pub fn build_mutation_registry_with_user(
        &mut self,
        user_mutations: &[CustomMutation],
    ) {
        self.build_mutation_registry_with_app_and_user(&[], user_mutations);
    }

    /// Build the registry from all four sources, in ascending
    /// precedence: application bundle < user file < map < inline.
    /// Later writers override earlier ones with the same `id`, so
    /// the inline-on-node shape wins last and the app bundle is the
    /// most easily overridable layer.
    pub fn build_mutation_registry_with_app_and_user(
        &mut self,
        app_mutations: &[CustomMutation],
        user_mutations: &[CustomMutation],
    ) {
        self.mutation_registry.clear();
        self.mutation_sources.clear();
        for cm in app_mutations {
            self.mutation_registry.insert(cm.id.clone(), cm.clone());
            self.mutation_sources
                .insert(cm.id.clone(), MutationSource::App);
        }
        for cm in user_mutations {
            self.mutation_registry.insert(cm.id.clone(), cm.clone());
            self.mutation_sources
                .insert(cm.id.clone(), MutationSource::User);
        }
        for cm in &self.mindmap.custom_mutations {
            self.mutation_registry.insert(cm.id.clone(), cm.clone());
            self.mutation_sources
                .insert(cm.id.clone(), MutationSource::Map);
        }
        for node in self.mindmap.nodes.values() {
            for cm in &node.inline_mutations {
                self.mutation_registry.insert(cm.id.clone(), cm.clone());
                self.mutation_sources
                    .insert(cm.id.clone(), MutationSource::Inline);
            }
        }
    }

    /// Find custom mutations triggered by a given trigger on a specific node.
    /// Checks the node's trigger_bindings and filters by platform context.
    pub fn find_triggered_mutations(
        &self,
        node_id: &str,
        trigger: &Trigger,
        platform: &PlatformContext,
    ) -> Vec<CustomMutation> {
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return vec![],
        };
        let mut results = Vec::new();
        for binding in &node.trigger_bindings {
            if &binding.trigger != trigger {
                continue;
            }
            // Check platform context filter
            if !binding.contexts.is_empty() && !binding.contexts.contains(platform) {
                continue;
            }
            if let Some(cm) = self.mutation_registry.get(&binding.mutation_id) {
                results.push(cm.clone());
            }
        }
        results
    }

    // ---- Animation lifecycle (Phase 4.2) ----
    //
    // Animations are an *envelope* on `apply_custom_mutation` — when
    // a `CustomMutation` carries `timing: Some(AnimationTiming { ... })`
    // with a non-zero `duration_ms`, the dispatcher routes it through
    // `start_animation` instead of applying instantly. Each tick
    // computes a blended `MindNode` snapshot and writes it back into
    // `mindmap.nodes` so the existing `rebuild_all` path sees the
    // in-progress state and repaints. The tree never sees the from
    // state mid-flight; the render pipeline reads model → builds tree
    // → walks → shapes, so the model write is the single source of
    // truth for the animated frame.
    //
    // The architecture mirrors the dragging / editing invariant: the
    // *tree* (or in this case, the model — drag and edit work
    // tree-only because they're transient previews) carries the
    // in-progress state, the model is the boundary commit. For
    // animations we write to the model directly because the
    // animation IS the commit — `apply_custom_mutation` would have
    // produced the same final state anyway, just in one step
    // instead of many. The undo entry is pushed once at completion.
    //
    // Per the original Phase 4 plan: position / size / color
    // interpolate; structural changes (text replacement, region
    // count shifts) snap at the boundary. v1 only interpolates
    // `position` because every other interpolated field needs
    // careful per-mutation snapshot logic that adds more lines than
    // the foundation justifies. Subsequent commits expand the
    // interpolated-field set as concrete consumers arrive.

    /// Start an animation for `cm` targeting `target_id`. Snapshots
    /// the current node state, applies the mutation to a scratch
    /// copy to derive the `to` snapshot, and pushes an
    /// [`AnimationInstance`] onto [`Self::active_animations`]. The
    /// caller has already verified
    /// `cm.timing.as_ref().is_some_and(|t| t.duration_ms > 0)`.
    ///
    /// **v1 restrictions** (lifted as concrete consumers arrive):
    /// only `TargetScope::SelfOnly` interpolates per-frame; other
    /// scopes apply at the boundary. Only `position` is lerped
    /// continuously; text / regions / structural fields snap at
    /// completion. `Followup` variants (`Reverse`, `Chain`, `Loop`)
    /// are recorded on the instance but not yet enacted.
    pub fn start_animation(
        &mut self,
        cm: &CustomMutation,
        target_id: &str,
        now_ms: u64,
    ) {
        // Invariant check the `AnimationInstance::timing()`
        // projection relies on: `cm.timing` must be Some with a
        // non-zero duration, else the caller should have taken
        // the instant-mutation path.
        if !cm
            .timing
            .as_ref()
            .is_some_and(|t| t.duration_ms > 0)
        {
            return;
        }

        // Re-trigger the same (mutation_id, node_id) mid-flight is a
        // silent no-op — same semantics as the original Phase 4 plan.
        if self
            .active_animations
            .iter()
            .any(|a| a.mutation_id() == cm.id && a.target_id == target_id)
        {
            return;
        }

        // Snapshot the from state.
        let Some(from_node) = self.mindmap.nodes.get(target_id).cloned() else {
            return;
        };

        // Compute the to state by applying the mutation to a scratch
        // copy of the document. The scratch path uses the existing
        // GlyphArea command vocabulary so animation receives the same
        // final state instant-mode would have landed on — there's
        // only one source of truth for "what does this mutation do".
        // Extract the flat Mutation list from the mutator AST for the
        // scratch-node replay. MutatorNode shapes with runtime holes
        // (size-aware mutations) can't be previewed against a single
        // model node — the scratch stays at `from` and the animation
        // lerps to whatever the mutator produces at completion.
        let mut scratch = from_node.clone();
        let flat = cm
            .mutator
            .as_ref()
            .and_then(baumhard::mindmap::custom_mutation::flat_mutations)
            .unwrap_or_default();
        apply_position_mutations_to_node(&flat, &mut scratch);
        let to_node = scratch;

        self.active_animations.push(AnimationInstance {
            target_id: target_id.to_string(),
            from_node,
            to_node,
            start_ms: now_ms,
            cm: cm.clone(),
        });
    }

    /// Tick every active animation against the wall clock at
    /// `now_ms`. For each instance, lerp position from `from_node`
    /// to `to_node` according to the easing curve and write the
    /// blended state back into `mindmap.nodes`. Returns `true` iff
    /// any animation advanced (so the caller knows to trigger a
    /// scene rebuild).
    ///
    /// Animations whose elapsed time has reached `duration_ms +
    /// delay_ms` complete: their final state is committed via
    /// `apply_custom_mutation` (so the standard
    /// model-sync + undo-push path runs exactly once), then the
    /// instance is dropped. Drain order is back-to-front so
    /// `swap_remove` is safe.
    pub fn tick_animations(
        &mut self,
        now_ms: u64,
        mut tree: Option<&mut MindMapTree>,
    ) -> bool {
        if self.active_animations.is_empty() {
            return false;
        }

        let mut completed_indices: Vec<usize> = Vec::new();
        let mut any_advanced = false;

        for (idx, anim) in self.active_animations.iter().enumerate() {
            let timing = anim.timing();
            let elapsed = now_ms.saturating_sub(anim.start_ms);
            let total = timing.delay_ms as u64 + timing.duration_ms as u64;
            if elapsed >= total {
                completed_indices.push(idx);
                continue;
            }
            // Skip the delay phase — node stays at `from` until the
            // delay elapses.
            if elapsed < timing.delay_ms as u64 {
                continue;
            }
            let progress =
                (elapsed - timing.delay_ms as u64) as f32 / timing.duration_ms as f32;
            let t = timing.easing.evaluate(progress);

            let node = match self.mindmap.nodes.get_mut(&anim.target_id) {
                Some(n) => n,
                None => continue,
            };
            node.position.x = lerp_f32(
                anim.from_node.position.x as f32,
                anim.to_node.position.x as f32,
                t,
            ) as f64;
            node.position.y = lerp_f32(
                anim.from_node.position.y as f32,
                anim.to_node.position.y as f32,
                t,
            ) as f64;
            any_advanced = true;
        }

        if !completed_indices.is_empty() {
            // Drain completed animations. Apply each one's final
            // state through `apply_custom_mutation` — that's the
            // single path that handles model-sync + undo-push for
            // both Persistent and Toggle behaviour, so the tree
            // animation's commit is indistinguishable from the
            // instant-mode equivalent.
            for idx in completed_indices.into_iter().rev() {
                let anim = self.active_animations.swap_remove(idx);
                if let Some(tree) = tree.as_deref_mut() {
                    self.apply_custom_mutation(&anim.cm, &anim.target_id, tree);
                } else {
                    // No tree available — at minimum restore the
                    // model to the `to` snapshot so the next
                    // rebuild_all sees the post-animation state.
                    if let Some(node) = self.mindmap.nodes.get_mut(&anim.target_id) {
                        node.position = anim.to_node.position.clone();
                    }
                }
                any_advanced = true;
            }
        }

        any_advanced
    }

    /// `true` while one or more animations are still ticking.
    /// Used by the event loop to decide whether to keep emitting
    /// `AboutToWait` work and rebuilding the scene.
    pub fn has_active_animations(&self) -> bool {
        !self.active_animations.is_empty()
    }

    /// Fast-forward every active animation to its `to` state and
    /// commit it through `apply_custom_mutation` (which pushes
    /// one undo entry per completed animation). Called by the
    /// Ctrl+Z handler before `undo()` so mid-animation Ctrl+Z
    /// has predictable semantics: the animation snaps to
    /// completion, the undo entry it would have pushed at the
    /// natural boundary is pushed immediately, and the
    /// subsequent `undo()` pops that entry — so Ctrl+Z during
    /// an animated transition reverses the animation's effect
    /// in one keystroke, same as Ctrl+Z after the animation
    /// completed naturally.
    ///
    /// Drains `active_animations` wholesale. Order within the
    /// drain doesn't matter because each instance commits
    /// independently and pushes its own undo entry.
    pub fn fast_forward_animations(&mut self, tree: Option<&mut MindMapTree>) {
        if self.active_animations.is_empty() {
            return;
        }
        let drained = std::mem::take(&mut self.active_animations);
        let mut tree = tree;
        for anim in drained {
            if let Some(tree) = tree.as_deref_mut() {
                self.apply_custom_mutation(&anim.cm, &anim.target_id, tree);
            } else if let Some(node) = self.mindmap.nodes.get_mut(&anim.target_id) {
                // No tree available — restore the model to the
                // `to` snapshot directly. Undo path is then the
                // caller's responsibility, matching what
                // `tick_animations` does on its no-tree path.
                node.position = anim.to_node.position.clone();
            }
        }
    }
}
