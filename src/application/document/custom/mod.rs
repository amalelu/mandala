// SPDX-License-Identifier: MPL-2.0

//! Custom-mutation infrastructure — `apply_custom_mutation` and
//! its helpers. The bridge between the declarative
//! `CustomMutation` shape and the document's mutation-and-undo
//! plumbing.
//!
//! Layout:
//! - This file (`mod.rs`) — the apply pipeline:
//!   `apply_custom_mutation`, `apply_to_tree`,
//!   `apply_document_actions`, `collect_affected_section_targets`,
//!   `will_dispatch_to_handler`, plus the
//!   predicate-filtered-everything warn helper.
//! - [`sync`] — the reverse converter: `sync_node_from_tree`
//!   pulls the live tree back into the model after a `Persistent`
//!   mutation, with `region_to_text_run` and
//!   `exact_or_dominant_overlap` as private merge primitives.

pub(super) mod sync;

use baumhard::mindmap::custom_mutation::{
    apply_mutations_to_element, flat_mutations, mutator_reach, CustomMutation, DocumentAction,
    MutationBehavior, TargetScope,
};
use baumhard::mindmap::model::MindNode;
use baumhard::mindmap::tree_builder::MindMapTree;

use super::undo_action::UndoAction;
use super::MindMapDocument;
use crate::application::source_tier::SourceTier;

/// Warn at apply time when a predicate gate filtered every
/// candidate out. Catches both authoring mistakes (a bare
/// `Predicate::new()` with no fields and `always_match=false` —
/// matches nothing) and structurally-impossible combos (e.g.
/// `target_scope: SectionsOnly` paired with
/// `(Flag(SectionRoot), Equals(true))`, where the gate matches
/// "flag clear" but every candidate has the flag set). Either
/// produces a silent no-op without this check; the warn surfaces
/// the issue at the dispatch site so the author can fix it
/// rather than chase a missing visual change.
/// `true` when this `Mutation` writes an *absolute* position
/// (`MoveTo`, `Assign` on the position field). Used by the
/// section fan-out in [`MindMapDocument::apply_to_tree`] to
/// route absolute-position commands to the container only —
/// applying them to every section-area would set every
/// section's canvas position equal to the container's, which
/// then collapses `section.offset` to `(0, 0)` after sync-back.
/// Delta commands (`Nudge*`) and non-position mutations are NOT
/// matched; those fan out safely.
fn mutation_targets_absolute_position(m: &baumhard::gfx_structs::mutator::Mutation) -> bool {
    use baumhard::core::primitives::ApplyOperation;
    use baumhard::gfx_structs::area::GlyphAreaCommand;
    use baumhard::gfx_structs::area_fields::{GlyphAreaField, GlyphAreaFieldType};
    use baumhard::gfx_structs::mutator::Mutation;
    match m {
        Mutation::AreaCommand(cmd) => matches!(**cmd, GlyphAreaCommand::MoveTo(_, _)),
        Mutation::AreaDelta(delta) => {
            // `Assign` on the position field sets it absolutely;
            // `Add` / `Sub` are deltas (safe to fan out).
            // `DeltaGlyphArea.fields` carries the touched fields
            // and a sibling `GlyphAreaFieldType::Operation`
            // entry naming the global op. Inspect both.
            if !delta.fields.contains_key(&GlyphAreaFieldType::Position) {
                return false;
            }
            matches!(
                delta.fields.get(&GlyphAreaFieldType::Operation),
                Some(GlyphAreaField::Operation(ApplyOperation::Assign))
            )
        }
        _ => false,
    }
}

/// The tree-side `GlyphArea` fields a single
/// [`baumhard::gfx_structs::mutator::Mutation`] touches
/// that [`MindMapDocument::sync_node_from_tree`] has **no model
/// home for** — so a mutation writing them lands on the display
/// tree for one frame and reverts on the next rebuild-from-model.
///
/// The sync-back persists position, section offset / size, text,
/// color / font runs, and font size (`scale`). Everything else a
/// `GfxMutator` can reach — line-height (derived as `scale * 1.2`,
/// no independent home), the outline halo, the node shape, and the
/// zoom-visibility window — has a tree representation but no
/// reverse converter, so it can't survive a `rebuild_all`. Returns
/// the human-readable names of any such fields the mutation writes.
fn unsupported_fields_of_mutation(m: &baumhard::gfx_structs::mutator::Mutation) -> Vec<&'static str> {
    use baumhard::gfx_structs::area::GlyphAreaCommand as Cmd;
    use baumhard::gfx_structs::area_fields::GlyphAreaFieldType as FieldType;
    use baumhard::gfx_structs::mutator::Mutation;
    let mut out = Vec::new();
    match m {
        Mutation::AreaCommand(cmd) => {
            if matches!(
                **cmd,
                Cmd::SetLineHeight(_) | Cmd::GrowLineHeight(_) | Cmd::ShrinkLineHeight(_)
            ) {
                out.push("line-height");
            }
        }
        Mutation::AreaDelta(delta) => {
            // A delta can touch several fields at once; report each
            // unsupported one so the warning names the full gap.
            for key in delta.fields.keys() {
                match key {
                    FieldType::LineHeight => out.push("line-height"),
                    FieldType::Outline => out.push("outline"),
                    FieldType::Shape => out.push("shape"),
                    FieldType::ZoomVisibility => out.push("zoom-visibility"),
                    _ => {}
                }
            }
        }
        // `ModelDelta` / `ModelCommand` / `Event` / `None` don't
        // reach the section-area sync path at all.
        _ => {}
    }
    out
}

fn warn_if_predicate_filtered_everything(mutation_id: &str, has_predicate: bool, seen: usize, passed: usize) {
    if has_predicate && seen > 0 && passed == 0 {
        log::warn!(
            "mutation '{}': top-level predicate filtered every candidate ({} elements seen, 0 passed); \
             the apply path completed but no element was mutated",
            mutation_id,
            seen
        );
    }
}

impl MindMapDocument {
    /// `true` when the registered mutation at `mutation_id` will
    /// dispatch through its Rust [`super::mutations::DynamicMutationHandler`]
    /// at apply time. Two conditions must hold:
    ///
    /// - A handler is registered for this id.
    /// - The mutation's source layer is [`SourceTier::App`] — i.e.
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
            && self.mutation_sources.get(mutation_id) == Some(&SourceTier::App)
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
        tree: Option<&mut MindMapTree>,
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
                self.active_toggles.retain(|k| k != &key);
                self.dirty = true;
                return;
            }
            // Not yet active (the `contains` above returned false), so
            // append — preserving activation order for the ordered
            // re-stamp in `reapply_active_toggles`.
            self.active_toggles.push(key);
            // Toggle mutations apply to tree only (visual), no model sync.
            // No `as_deref_mut()` reborrow: this branch `return`s
            // immediately below, so `tree` is at its last use and can
            // be moved out of (same reasoning as the `else if let
            // Some(tree) = tree` on the persistent path).
            if let Some(tree) = tree {
                self.warn_about_mutator(custom);
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
        // user's mutator is honored. See
        // [`Self::will_dispatch_to_handler`] for the rationale.
        //
        // `changed` is the load-bearing verdict: only a mutation that
        // actually moved the model earns an undo entry and the dirty
        // flag. Pre-fix every apply pushed undo unconditionally, so
        // a `grow-font` (whose scale change wasn't synced back), a
        // `flat_mutations`-failed skip, or a predicate that filtered
        // every candidate all left a dead undo entry that ate a real
        // Ctrl-Z step.
        let changed = if self.will_dispatch_to_handler(&custom.id) {
            if let Some(handler) = self.mutation_handlers.get(&custom.id).copied() {
                handler(self, node_id);
            }
            // Imperative handlers (flower-layout, tree-cascade) mutate
            // the model directly — they're layout algorithms that
            // reposition their targets, and there's no cheap post-hoc
            // model diff to gate on. Treat a dispatched handler as a
            // real change; the snapshot taken above is its undo home.
            true
        } else if let Some(tree) = tree {
            // Surface a declined AST, or any mutator field the
            // sync-back can't persist, *before* applying — so a
            // partially-supported mutation doesn't silently drop half
            // its effect on the next rebuild (§5 no half-features).
            // Hoisted here rather than left inside `apply_to_tree`
            // because the persistent path calls that twice (once for
            // the caller's interactive tree, once for the pure tree),
            // which logged every diagnosis twice per apply.
            self.warn_about_mutator(custom);
            // Apply to the caller's interactive tree so the change is
            // immediately visible / hit-testable before the next
            // rebuild (the render, keybind, and click paths read this
            // tree's post-apply state).
            self.apply_to_tree(custom, node_id, tree);
            // But sync from a **fresh, pure** projection, never the
            // caller's tree. The interactive tree (the stored render
            // tree the keybind / click dispatchers pass) can carry
            // render-layer overlays — selection highlights and
            // active-toggle visuals stamped in `rebuild_all` — that
            // must NEVER round-trip into the persisted model (a nudge
            // toggle would become a permanent move; a selection
            // highlight would repaint the run cyan on disk). Applying
            // the same mutation to an overlay-free `build_tree` and
            // syncing from that writes back exactly the mutation's own
            // effect and nothing else.
            let mut pure_tree = self.build_tree();
            self.apply_to_tree(custom, node_id, &mut pure_tree);
            // Sync every affected node and OR the per-node verdicts.
            // An explicit loop (not `|=`) keeps `sync_node_from_tree`
            // — which is `#[must_use]` — running for every node.
            let mut any_changed = false;
            for id in &affected_ids {
                if self.sync_node_from_tree(id, &pure_tree) {
                    any_changed = true;
                }
            }
            any_changed
        } else {
            log::warn!(
                "apply_custom_mutation: declarative mutation '{}' called with None tree; \
                 flat-apply skipped. Pass Some(&mut tree) when the mutation isn't \
                 handler-dispatched.",
                custom.id
            );
            // Nothing applied, nothing to sync — leave the undo stack
            // and dirty flag untouched.
            return;
        };

        if changed && !snapshots.is_empty() {
            self.undo_stack.push(UndoAction::CustomMutation {
                node_snapshots: snapshots,
            });
            self.dirty = true;
        }
    }

    /// Diagnose everything wrong with `custom`'s mutator that the
    /// apply is about to paper over, and log one `warn!` per problem.
    /// Two of them:
    ///
    /// - **The AST is declined by the flat-apply path** — a runtime
    ///   hole, a filtering `RepeatWhile`, a `Single` root, or nested
    ///   payloads that disagree (see
    ///   [`flat_mutations`]). Nothing will be applied at all.
    /// - **The flat list writes a field the sync-back can't
    ///   persist** (line-height, outline, shape, zoom-visibility).
    ///   The change flashes for one frame and then reverts, and the
    ///   author is left chasing a vanishing effect.
    ///
    /// Both are silent-failure shapes, which is the worst outcome;
    /// naming them at apply time turns each into a diagnosable event
    /// (§5 no half-features).
    ///
    /// **This is the one place either is reported.**
    /// [`Self::apply_to_tree`] deliberately stays quiet on a declined
    /// AST: the persistent path calls it twice per apply (interactive
    /// tree, then pure tree), so warning there logged everything
    /// twice, and [`Self::reapply_active_toggles`] calls it once per
    /// active toggle on *every* rebuild, which would turn a single
    /// bad toggle into per-frame log spam. Callers that apply on
    /// behalf of a user gesture call this first; the re-stamp path
    /// does not, because its toggle was diagnosed when the user
    /// turned it on.
    ///
    /// Mutator-less (document-action-only) mutations are not a
    /// problem shape and are skipped.
    fn warn_about_mutator(&self, custom: &CustomMutation) {
        let Some(mutator) = custom.mutator.as_ref() else {
            return;
        };
        let Some(mutations) = flat_mutations(mutator) else {
            log::warn!(
                "mutation '{}': mutator AST is not flat-extractable — the flat-apply path \
                 can't evaluate this shape, so the apply is skipped. Causes: a root that \
                 isn't a `Macro` with a literal list, a `RepeatWhile` predicate that isn't \
                 `always_match`, a runtime-supplied payload, or two nested payloads that \
                 disagree. Use `scope::self_only` / `scope::descendants` / \
                 `scope::self_and_descendants` for now, or wait for the walker-based \
                 `apply_custom_mutation` path.",
                custom.id
            );
            return;
        };
        let mut fields: Vec<&'static str> = Vec::new();
        for m in &mutations {
            for name in unsupported_fields_of_mutation(m) {
                if !fields.contains(&name) {
                    fields.push(name);
                }
            }
        }
        if !fields.is_empty() {
            log::warn!(
                "mutation '{}': writes field(s) [{}] that have no model home; the change \
                 applies to the display tree but is NOT persisted and reverts on the next \
                 rebuild. Persisted fields: position, section offset/size, text, color/font \
                 runs, font size.",
                custom.id,
                fields.join(", "),
            );
        }
    }

    /// Re-stamp every active toggle's tree-side visual onto a
    /// freshly-built tree. Called by
    /// [`MindMapDocument::build_tree`](super::MindMapDocument::build_tree)
    /// after the model→tree projection, because a `rebuild_all`
    /// throws the prior tree away and rebuilds from the model — and
    /// Toggle mutations, by design, live only on the tree (they
    /// never sync to the model, per CONCEPTS §4). Without this
    /// re-application a toggle-on's visual would die at the end of
    /// the same dispatch that turned it on: nothing re-applied it,
    /// so "second trigger reverses" had no first-trigger effect left
    /// to reverse.
    ///
    /// Toggles always take the declarative flat-apply path — the
    /// Toggle branch of [`Self::apply_custom_mutation`] never
    /// dispatches to a Rust handler — so re-application is the same
    /// [`Self::apply_to_tree`] call, keyed by the `(node_id,
    /// mutation_id)` pairs in `active_toggles`. A pair whose mutation
    /// left the registry, or whose node left the model, is skipped
    /// (the lookups return `None` / `apply_to_tree` no-ops on a
    /// missing arena id).
    ///
    /// Iterated in insertion order (`active_toggles` is an ordered
    /// list, not a hash set) so a rebuild re-stamps non-commutative
    /// toggles in the same sequence the user activated them — the
    /// post-rebuild visual matches the pre-rebuild one.
    pub(in crate::application) fn reapply_active_toggles(&self, tree: &mut MindMapTree) {
        if self.active_toggles.is_empty() {
            return;
        }
        for (node_id, mutation_id) in &self.active_toggles {
            let Some(custom) = self.mutation_registry.get(mutation_id) else {
                continue;
            };
            self.apply_to_tree(custom, node_id, tree);
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
                            self.mindmap.canvas.theme_variables.insert(k.clone(), v.clone());
                            changed = true;
                        }
                    }
                }
                // `DocumentAction` is `#[non_exhaustive]` so future
                // variants can be added without breaking dependents.
                // Any new variant that performs file I/O, network
                // access, or arbitrary content load MUST be gated
                // at the macro-dispatcher site — see
                // `SourceTier::allows_console_line` and
                // `lib/baumhard/src/mindmap/custom_mutation/mod.rs`
                // doc-comment on `DocumentAction`.
                _ => {
                    log::warn!("apply_document_actions: unknown DocumentAction variant; skipping");
                }
            }
        }
        if changed {
            self.undo_stack
                .push(UndoAction::CanvasSnapshot { canvas: snapshot });
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
    ///
    /// **Section fan-out.** Post-section the per-node element shape is
    /// "chrome-only container + N section-areas + N section-models". A
    /// pre-section custom mutation that mutated the per-node area (text,
    /// runs, scale) used to land on the one-area-per-node entry; today
    /// it has to fan out across every section-area, otherwise the
    /// mutation lands on a no-glyph container with no visible effect.
    /// Position-affecting mutations apply to *both* container and
    /// sections so the whole node moves in lockstep (sections store
    /// absolute canvas positions in the tree).
    fn apply_to_tree(&self, custom: &CustomMutation, node_id: &str, tree: &mut MindMapTree) {
        use baumhard::core::primitives::{Flag, Flaggable};

        let Some(mutator) = custom.mutator.as_ref() else { return };
        let Some(mutations) = flat_mutations(mutator) else {
            // Declined AST — the flat-apply path can't extract a
            // mutation list from this shape, so nothing is applied.
            // The warning lives in
            // [`Self::warn_about_mutator`], which every
            // user-gesture caller runs exactly once before reaching
            // here; see that doc for why it is not logged in this
            // function. The richer `mutator_builder` walker path is
            // the eventual home for these shapes (size-aware /
            // predicate-filtered descendant iteration).
            return;
        };
        // Top-level predicate gate (item E3). When `Some`, every
        // candidate element must satisfy `predicate.test(element)`
        // before mutations land on it. Authors typically reach for
        // this to add an element-level filter on top of the
        // structural `target_scope` — e.g.
        // `Predicate { fields: [(Flag(SectionRoot), Equals(false))], … }`
        // matches every element with `SectionRoot` set (i.e.
        // sections), so paired with `target_scope: SelfOnly` it
        // lands the mutation on sections only and skips the
        // chrome-only container. The inverse
        // `(Flag(SectionRoot), Equals(true))` matches the
        // container + sibling mind-nodes (anything with
        // `SectionRoot` *clear*).
        //
        // **§9 footgun guard.** A bare `Predicate::new()` (`fields=[]`,
        // `always_match=false`) matches nothing; pairing it as
        // `predicate: Some(Predicate::new())` silently filters every
        // candidate out, the apply path runs but no element changes.
        // Warn so authors notice the typo at apply time. Same warn
        // covers the "predicate filtered everything" case below
        // (e.g. `SectionsOnly` + `(Flag(SectionRoot), Equals(true))`
        // — structurally impossible because every `SectionsOnly`
        // candidate has `SectionRoot` set).
        let predicate_gate = custom.predicate.as_ref();
        if let Some(p) = predicate_gate {
            if p.fields.is_empty() && !p.always_match {
                log::warn!(
                    "mutation '{}': predicate has no fields and `always_match` is false; \
                     every candidate element will be filtered out (apply runs but produces no change)",
                    custom.id
                );
            }
        }
        let passes = |element: &baumhard::gfx_structs::element::GfxElement| -> bool {
            predicate_gate.is_none_or(|p| p.test(element))
        };
        let mut candidates_seen: usize = 0;
        let mut candidates_passed: usize = 0;

        // `SectionsOnly` (item E4) bypasses the container fan-out:
        // mutations land on every section-area directly. Channel
        // collisions with sibling mind-nodes don't reach in here
        // because we walk the section_map by `(triggering_node_id,
        // section_idx)` tuples — child mind-nodes that share a
        // channel with a section live elsewhere in the arena.
        if custom.target_scope == TargetScope::SectionsOnly {
            let targets = self.collect_affected_section_targets(node_id);
            for (mind_id, section_idx) in &targets {
                let Some(section_arena_id) = tree.section_arena_id(mind_id, *section_idx) else {
                    continue;
                };
                if let Some(node) = tree.tree.arena.get_mut(section_arena_id) {
                    candidates_seen += 1;
                    if passes(node.get()) {
                        candidates_passed += 1;
                        apply_mutations_to_element(&mutations, node.get_mut());
                    }
                }
            }
            warn_if_predicate_filtered_everything(
                &custom.id,
                predicate_gate.is_some(),
                candidates_seen,
                candidates_passed,
            );
            // Invalidate the tree's `subtree_aabb` and other geometry
            // caches — `apply_mutations_to_element` mutated arena
            // element fields directly (bypassing
            // `MutatorTree::apply_to`'s wrapper that owns the
            // invalidation). Pre-fix the dirty flag stayed false
            // so a downstream `ensure_subtree_aabbs()` call (e.g.
            // the editor click-outside-commit's overflow-aware
            // `point_in_node_aabb`) was a no-op against a stale
            // cache. Same shape as `MutatorTree::apply_to`'s
            // invalidation (`lib/baumhard/src/gfx_structs/tree.rs`).
            tree.tree.invalidate_caches();
            return;
        }

        let affected = self.collect_affected_node_ids(node_id, &custom.target_scope);
        for id in &affected {
            let Some(container_arena_id) = tree.arena_id_for(id.as_str()) else {
                continue;
            };
            // Apply to the container (carries position; text/regions
            // are empty so text-affecting mutations are visually no-op
            // here, but `area.scale` and position deltas land cleanly).
            // Container is `SectionRoot`-clear, so a
            // `(Flag(SectionRoot), Equals(false))` predicate (matches
            // when the flag IS set) filters the container out — only
            // sections survive, which is the documented "sections
            // only" idiom.
            // Snapshot the container's pre-mutation position so
            // we can compute the implied delta below. Absolute-
            // position commands (`MoveTo`, `Position(Assign)`)
            // need to translate sections by the same delta to
            // keep `section.offset` invariant; without this,
            // sync-back would write
            // `section.offset = old_section_pos - new_node_pos`
            // and silently collapse the offset.
            let pre_container_pos = tree
                .tree
                .arena
                .get(container_arena_id)
                .and_then(|n| n.get().glyph_area())
                .map(|a| (a.position.x.0, a.position.y.0));

            if let Some(node) = tree.tree.arena.get_mut(container_arena_id) {
                candidates_seen += 1;
                if passes(node.get()) {
                    candidates_passed += 1;
                    apply_mutations_to_element(&mutations, node.get_mut());
                }
            }

            // Derive the container's post-mutation delta. If the
            // mutations included an absolute MoveTo or position
            // Assign, the delta will be non-zero; for delta-only
            // commands (`Nudge*`) the section fan-out below will
            // re-apply the same Nudge directly so this path is
            // redundant (the section's tree-pos already moved by
            // the same amount as the container).
            let container_translate_delta = pre_container_pos.and_then(|(pre_x, pre_y)| {
                tree.tree
                    .arena
                    .get(container_arena_id)
                    .and_then(|n| n.get().glyph_area())
                    .map(|a| (a.position.x.0 - pre_x, a.position.y.0 - pre_y))
            });
            let absolute_position_in_mutations = mutations.iter().any(mutation_targets_absolute_position);
            // Fan out across every immediate `Flag::SectionRoot` child
            // of the container — that's where `text`, `text_runs`
            // (regions), and per-section `scale` actually live in the
            // post-section tree shape. Without this fan-out every
            // text/font/region custom mutation silently no-ops on the
            // empty container. Mirrors `apply_tree_highlights`'s
            // walk so mutations and highlights both reach sections by
            // the same primitive.
            //
            // §B7 borrow-split note: collecting into a
            // `Vec<indextree::NodeId>` here is the smallest correct
            // shape. `container_arena_id.children(&tree.tree.arena)`
            // borrows the arena immutably; the loop body needs
            // `&mut tree.tree.arena.get_mut(...)` to apply the
            // mutation. Holding the iterator across the mutable
            // borrow won't compile, so the fix is to materialize the
            // section ids first and drop the immutable borrow
            // before the mutation pass starts. The vec is `O(sections
            // per node)` — bounded by user authoring (typically 1–4
            // per node), per affected-node-per-call, so allocation
            // cost is negligible relative to the mutation walker.
            let section_arena_ids: Vec<indextree::NodeId> = container_arena_id
                .children(&tree.tree.arena)
                .filter(|cid| {
                    tree.tree
                        .arena
                        .get(*cid)
                        .map(|n| n.get().flag_is_set(Flag::SectionRoot))
                        .unwrap_or(false)
                })
                .collect();
            // Filter out mutations that would set absolute
            // position before fanning to sections. Section-area
            // canvas position is computed as
            // `node.pos + section.offset`; an absolute
            // `MoveTo(x, y)` lands every section at `(x, y)` —
            // identical to the container's position — and the
            // sync-back step then reads `section.offset =
            // section_pos - node_pos = (0, 0)`, silently
            // collapsing every authored offset. Delta commands
            // (`Nudge*`) move the section by the same delta as
            // the container, preserving offset; absolute commands
            // need to land on the container only.
            let section_safe_mutations: Vec<_> = mutations
                .iter()
                .filter(|m| !mutation_targets_absolute_position(m))
                .cloned()
                .collect();
            for sid in section_arena_ids {
                if let Some(node) = tree.tree.arena.get_mut(sid) {
                    candidates_seen += 1;
                    if passes(node.get()) {
                        candidates_passed += 1;
                        apply_mutations_to_element(&section_safe_mutations, node.get_mut());
                        // For absolute-position mutations on the
                        // container (filtered out of
                        // `section_safe_mutations` above),
                        // translate the section by the container's
                        // implied delta so it moves *with* the
                        // container — preserving `section.offset`
                        // through the sync-back step. Pre-fix
                        // sections stayed frozen and offset
                        // collapsed.
                        if absolute_position_in_mutations {
                            if let Some((dx, dy)) = container_translate_delta {
                                if let Some(area) = node.get_mut().glyph_area_mut() {
                                    area.move_position(dx, dy);
                                }
                            }
                        }
                    }
                }
            }
        }
        warn_if_predicate_filtered_everything(
            &custom.id,
            predicate_gate.is_some(),
            candidates_seen,
            candidates_passed,
        );
        // Same invalidation as the `SectionsOnly` branch above:
        // `apply_mutations_to_element` writes arena element fields
        // directly without going through `MutatorTree::apply_to`,
        // so the tree's `subtree_aabbs_dirty` flag is unset until
        // we explicitly mark it. Downstream callers that read
        // `subtree_aabb()` (notably the overflow-aware
        // `point_in_node_aabb` from the editor click-outside-commit)
        // would otherwise hit a stale cache.
        tree.tree.invalidate_caches();
    }

    /// Collect the section targets affected by a `SectionsOnly`
    /// mutation: every section of `node_id`. Returns `(mind_id,
    /// section_idx)` tuples — the right shape for `tree
    /// .section_arena_id(...)` lookups in the apply-to-tree branch.
    /// Empty when `node_id` doesn't exist in the model or carries
    /// zero sections (loader-rejected at load time, but defensive
    /// for synthesized in-memory docs).
    fn collect_affected_section_targets(&self, node_id: &str) -> Vec<(String, usize)> {
        let Some(node) = self.mindmap.nodes.get(node_id) else {
            return Vec::new();
        };
        (0..node.sections.len())
            .map(|idx| (node_id.to_string(), idx))
            .collect()
    }

    /// Collect the IDs of all nodes affected by a mutation with the
    /// given scope. This list is simultaneously the apply-time target
    /// set and the undo-snapshot window — the equivalence CONCEPTS §4
    /// calls the load-bearing detail — so the two can never drift.
    ///
    /// Two scopes resolve to the empty set on a root node, by design:
    /// `Parent` (a root has no parent) and `Siblings` (a root shares
    /// no parent, so the other roots of a multi-root map are *not*
    /// siblings — see [`TargetScope::Siblings`]). Both are no-ops
    /// rather than errors: nothing is snapshotted, `apply_to_tree`
    /// iterates an empty list, and the `changed` verdict stays
    /// `false` so no dead undo entry is pushed.
    pub(super) fn collect_affected_node_ids(&self, node_id: &str, scope: &TargetScope) -> Vec<String> {
        match scope {
            // `SectionsOnly` lives on the triggering node — every
            // affected section belongs to that one node. The
            // node-level affected list is the snapshot window for
            // undo (whole-`MindNode` clone covers section state),
            // so a single entry is correct.
            TargetScope::SelfOnly | TargetScope::SectionsOnly => vec![node_id.to_string()],
            // Tree-walking scopes build the child index once, inside
            // the arm that actually needs it. Building it before the
            // match would make self-only / parent-only mutations O(N).
            TargetScope::Children => self
                .mindmap
                .child_index()
                .children_of(node_id)
                .iter()
                .map(|n| n.id.clone())
                .collect(),
            TargetScope::Descendants => self
                .mindmap
                .child_index()
                .all_descendant_ids(node_id, self.mindmap.nodes.len()),
            TargetScope::SelfAndDescendants => {
                let mut ids = vec![node_id.to_string()];
                ids.extend(
                    self.mindmap
                        .child_index()
                        .all_descendant_ids(node_id, self.mindmap.nodes.len()),
                );
                ids
            }
            TargetScope::Parent => self
                .mindmap
                .nodes
                .get(node_id)
                .and_then(|n| n.parent_id.clone())
                .into_iter()
                .collect(),
            // A root node (`parent_id == None`) short-circuits to
            // the empty set here — `and_then` yields `None` and
            // `unwrap_or_default` returns an empty `Vec`. That is the
            // documented semantic, not an oversight: see
            // [`TargetScope::Siblings`].
            TargetScope::Siblings => {
                let parent_id = self.mindmap.nodes.get(node_id).and_then(|n| n.parent_id.clone());
                let index = self.mindmap.child_index();
                parent_id
                    .map(|pid| {
                        index
                            .children_of(&pid)
                            .iter()
                            .filter(|n| n.id != node_id)
                            .map(|n| n.id.clone())
                            .collect()
                    })
                    .unwrap_or_default()
            }
        }
    }
}

/// Differential harness for [`TargetScope::covers_reach`].
///
/// `covers_reach` is a *static* gate: it answers "is this
/// scope + reach pairing safe?" from two enum values alone. What it
/// is really claiming is a structural property of the real tree —
/// that the scope's target set (which is exactly the undo-snapshot
/// window, see [`MindMapDocument::collect_affected_node_ids`]) is
/// **closed** under the mutator's reach, given that the application
/// anchors the mutator at every target in turn.
///
/// This module rebuilds that property from scratch against the
/// testament map's 252 real nodes — the reference model — and
/// compares it cell-by-cell with what `covers_reach` returns. Neither
/// side consults the other: the reference walks `child_index()`, the
/// gate matches on enums.
///
/// Two directions are checked, because either alone is trivially
/// satisfiable:
///
/// - **Soundness** — every approved cell really is closed, for
///   *every* trigger node in the fixture. A single counterexample
///   means the gate green-lights a pairing whose undo snapshot is
///   narrower than what the mutator touches.
/// - **Tightness** — every rejected cell has a witness node where
///   closure actually fails. Without this a gate that returns
///   `false` everywhere would pass the soundness half.
#[cfg(test)]
mod covers_reach_differential_tests {
    use super::*;
    use crate::application::document::tests_common::load_test_doc;
    use baumhard::mindmap::custom_mutation::MutatorReach;
    use baumhard::mindmap::model::ChildIndex;
    use std::collections::BTreeSet;

    /// Every scope whose target set is a set of *model nodes*.
    /// `SectionsOnly` is excluded — its anchors are section-areas
    /// inside one `MindNode`, not model nodes, so the closure
    /// argument doesn't transfer. It gets
    /// [`test_sections_only_gate_is_sound_and_deliberately_conservative`]
    /// instead.
    ///
    /// Derived from `TargetScope::iter()` with that one subtraction,
    /// not hand-listed: an eighth variant has to be either covered
    /// here or explicitly excluded, and the exclusion is one named
    /// arm rather than a silent omission. A hand-typed array would
    /// simply run six cases forever
    /// ([`test_node_scopes_accounts_for_every_target_scope`] is the pin).
    fn node_scopes() -> Vec<TargetScope> {
        use strum::IntoEnumIterator;
        TargetScope::iter()
            .filter(|scope| !matches!(scope, TargetScope::SectionsOnly))
            .collect()
    }

    fn reaches() -> Vec<MutatorReach> {
        use strum::IntoEnumIterator;
        MutatorReach::iter().collect()
    }

    /// Every `TargetScope` is either exercised by the differential
    /// run or deliberately routed to the `SectionsOnly` test — no
    /// variant may fall between the two.
    #[test]
    fn test_node_scopes_accounts_for_every_target_scope() {
        use strum::IntoEnumIterator;
        for scope in TargetScope::iter() {
            let covered = node_scopes().contains(&scope) || scope == TargetScope::SectionsOnly;
            assert!(
                covered,
                "TargetScope::{:?} is in neither the differential run nor the \
                 SectionsOnly carve-out — give it a home before shipping it",
                scope
            );
        }
    }

    /// Reference model: the model nodes a mutator of `reach`, anchored
    /// at `anchor`, can write to. `MutatorReach` is cumulative
    /// (`SelfOnly <= Children <= Descendants`), so each set includes
    /// the narrower ones — the anchor itself is always in reach
    /// because every non-empty mutator carries a payload at its root.
    ///
    /// Derived from `child_index()` only; deliberately does not call
    /// `covers_reach` or any of its helpers.
    ///
    /// The index is passed in rather than rebuilt: this runs once per
    /// (trigger, target, reach) triple over a 252-node fixture, and
    /// `child_index()` is an O(N) scan plus a sort per child list —
    /// building it inside the loop dominated the whole test binary's
    /// runtime.
    fn reference_touch_set(
        index: &ChildIndex<'_>,
        budget: usize,
        anchor: &str,
        reach: MutatorReach,
    ) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        out.insert(anchor.to_string());
        match reach {
            MutatorReach::SelfOnly => {}
            MutatorReach::Children => {
                for child in index.children_of(anchor) {
                    out.insert(child.id.clone());
                }
            }
            MutatorReach::Descendants => {
                out.extend(index.all_descendant_ids(anchor, budget));
            }
        }
        out
    }

    /// Walk every node of the fixture as the trigger and report the
    /// first node where the scope's target set is *not* closed under
    /// `reach`, or `None` when the pairing is closed everywhere.
    ///
    /// The returned string names the trigger node and one node that
    /// escaped the snapshot, so a failure message points at a
    /// reproducible case rather than just "false".
    fn closure_witness(
        doc: &MindMapDocument,
        index: &ChildIndex<'_>,
        scope: TargetScope,
        reach: MutatorReach,
    ) -> Option<String> {
        let budget = doc.mindmap.nodes.len();
        let mut node_ids: Vec<&String> = doc.mindmap.nodes.keys().collect();
        node_ids.sort();
        for trigger in node_ids {
            let targets: BTreeSet<String> = doc
                .collect_affected_node_ids(trigger, &scope)
                .into_iter()
                .collect();
            for anchor in &targets {
                for touched in reference_touch_set(index, budget, anchor, reach) {
                    if !targets.contains(&touched) {
                        return Some(format!(
                            "trigger '{}' with scope {:?}: anchoring a {:?}-reach mutator at \
                             target '{}' touches '{}', which is not in the {}-node snapshot",
                            trigger,
                            scope,
                            reach,
                            anchor,
                            touched,
                            targets.len()
                        ));
                    }
                }
            }
        }
        None
    }

    /// The differential run: every node scope x every reach, gate
    /// verdict against reference verdict, both directions.
    #[test]
    fn test_covers_reach_matches_structural_closure_on_the_testament_map() {
        let doc = load_test_doc();
        assert!(
            doc.mindmap.nodes.len() > 100,
            "fixture too small to be a meaningful reference model"
        );
        // Built once and threaded through: the reference model
        // consults it 6 x 3 x (targets per trigger) times, and
        // rebuilding it per call is an O(N) scan each time.
        let index = doc.mindmap.child_index();
        let mut mismatches: Vec<String> = Vec::new();
        for scope in node_scopes() {
            for reach in reaches() {
                let witness = closure_witness(&doc, &index, scope.clone(), reach);
                let reference_says_safe = witness.is_none();
                let gate_says_safe = scope.covers_reach(reach);
                if gate_says_safe && !reference_says_safe {
                    mismatches.push(format!(
                        "UNSOUND: covers_reach({:?}, {:?}) == true but {}",
                        scope,
                        reach,
                        witness.unwrap_or_default()
                    ));
                } else if !gate_says_safe && reference_says_safe {
                    mismatches.push(format!(
                        "OVER-STRICT: covers_reach({:?}, {:?}) == false but the target set is \
                         closed under that reach for every node in the fixture",
                        scope, reach
                    ));
                }
            }
        }
        assert!(
            mismatches.is_empty(),
            "covers_reach disagrees with the structural reference model:\n  {}",
            mismatches.join("\n  ")
        );
    }

    /// `SectionsOnly`'s targets are the anchor's section-areas, which
    /// live inside the anchor's own `MindNode` — so the one-node
    /// snapshot covers them whatever the reach does *within* the
    /// section subtree. The gate still demands `SelfOnly`, which is
    /// strictly conservative: over-strict costs a spurious `warn!`,
    /// while over-permissive costs undo coverage. This pins the
    /// direction of the asymmetry so a later "optimization" can't
    /// flip it silently.
    #[test]
    fn test_sections_only_gate_is_sound_and_deliberately_conservative() {
        let doc = load_test_doc();
        let mut node_ids: Vec<&String> = doc.mindmap.nodes.keys().collect();
        node_ids.sort();
        for trigger in node_ids {
            let targets = doc.collect_affected_node_ids(trigger, &TargetScope::SectionsOnly);
            assert_eq!(
                targets,
                vec![trigger.clone()],
                "SectionsOnly snapshots exactly the anchor node"
            );
        }
        assert!(TargetScope::SectionsOnly.covers_reach(MutatorReach::SelfOnly));
        assert!(!TargetScope::SectionsOnly.covers_reach(MutatorReach::Children));
        assert!(!TargetScope::SectionsOnly.covers_reach(MutatorReach::Descendants));
    }

    /// `Siblings` on a root node resolves to the empty set: "sibling"
    /// means "shares my parent", and a root shares no parent — the
    /// other roots of a multi-root map are deliberately *not*
    /// siblings. The testament map has four roots, so this is a live
    /// case, not a hypothetical. Pins the documented semantics on
    /// [`TargetScope::Siblings`].
    #[test]
    fn test_siblings_of_a_root_node_is_the_empty_set() {
        let doc = load_test_doc();
        let roots: Vec<String> = doc
            .mindmap
            .nodes
            .values()
            .filter(|n| n.parent_id.is_none())
            .map(|n| n.id.clone())
            .collect();
        assert!(roots.len() > 1, "testament map is expected to have several roots");
        for root in &roots {
            assert!(
                doc.collect_affected_node_ids(root, &TargetScope::Siblings)
                    .is_empty(),
                "root '{}' must have no siblings even though {} other roots exist",
                root,
                roots.len() - 1
            );
        }
    }

    /// A non-root node's siblings exclude the node itself — the other
    /// half of the `Siblings` contract, and the reason
    /// `Siblings` + `MutatorReach::SelfOnly` is closed while
    /// `Siblings` + `Parent`-style reaches are not.
    #[test]
    fn test_siblings_excludes_the_trigger_node() {
        let doc = load_test_doc();
        let index = doc.mindmap.child_index();
        let mut checked = 0usize;
        let mut node_ids: Vec<&String> = doc.mindmap.nodes.keys().collect();
        node_ids.sort();
        for id in node_ids {
            let Some(parent_id) = doc.mindmap.nodes.get(id).and_then(|n| n.parent_id.clone()) else {
                continue;
            };
            if index.children_of(&parent_id).len() < 2 {
                continue;
            }
            let siblings = doc.collect_affected_node_ids(id, &TargetScope::Siblings);
            assert!(
                !siblings.contains(id),
                "node '{}' must not be its own sibling",
                id
            );
            assert_eq!(siblings.len(), index.children_of(&parent_id).len() - 1);
            checked += 1;
        }
        assert!(checked > 0, "fixture has no node with a sibling");
    }
}

#[cfg(test)]
mod unsupported_field_tests {
    use super::unsupported_fields_of_mutation;
    use baumhard::core::primitives::ApplyOperation;
    use baumhard::gfx_structs::area::{DeltaGlyphArea, GlyphArea, GlyphAreaCommand, GlyphAreaField};
    use baumhard::gfx_structs::mutator::Mutation;
    use baumhard::gfx_structs::shape::NodeShape;

    /// Font-size commands (`GrowFont` / `ShrinkFont` / `SetFontSize`)
    /// ARE persisted by the sync-back, so they must NOT be flagged —
    /// this is the whole point of the P0-02 fix.
    #[test]
    fn test_grow_and_shrink_font_are_supported() {
        for cmd in [
            GlyphAreaCommand::GrowFont(2.0),
            GlyphAreaCommand::ShrinkFont(2.0),
            GlyphAreaCommand::SetFontSize(20.0),
        ] {
            let m = Mutation::area_command(cmd);
            assert!(
                unsupported_fields_of_mutation(&m).is_empty(),
                "font-size command {:?} must be treated as supported",
                cmd
            );
        }
    }

    /// Position / bounds commands persist too (node position, section
    /// offset/size), so they're not flagged.
    #[test]
    fn test_position_and_bounds_commands_are_supported() {
        for cmd in [
            GlyphAreaCommand::NudgeRight(5.0),
            GlyphAreaCommand::MoveTo(1.0, 2.0),
            GlyphAreaCommand::SetBounds(10.0, 10.0),
        ] {
            let m = Mutation::area_command(cmd);
            assert!(unsupported_fields_of_mutation(&m).is_empty());
        }
    }

    /// Line-height commands have no model home (line-height is
    /// derived as `scale * 1.2` on every rebuild) — they must be
    /// flagged so the author isn't left chasing a vanishing change.
    #[test]
    fn test_line_height_commands_are_flagged() {
        for cmd in [
            GlyphAreaCommand::SetLineHeight(1.5),
            GlyphAreaCommand::GrowLineHeight(0.2),
            GlyphAreaCommand::ShrinkLineHeight(0.2),
        ] {
            let m = Mutation::area_command(cmd);
            assert_eq!(
                unsupported_fields_of_mutation(&m),
                vec!["line-height"],
                "line-height command {:?} must be flagged unsupported",
                cmd
            );
        }
    }

    /// A delta touching `shape` / `outline` / `zoom_visibility` — all
    /// tree-only fields with no reverse converter — is flagged, one
    /// name per unsupported field it writes.
    #[test]
    fn test_shape_outline_zoom_delta_fields_are_flagged() {
        let area = GlyphArea::new(14.0, 16.8, glam::Vec2::ZERO, glam::Vec2::new(10.0, 10.0));
        // `full_assign_from` emits Text/position/bounds/scale/
        // line_height/regions/Outline/ZoomVisibility/Operation — a
        // superset that exercises the delta-key scan. Add Shape too.
        let mut delta = DeltaGlyphArea::full_assign_from(&area);
        delta.fields.insert(
            baumhard::gfx_structs::area_fields::GlyphAreaFieldType::Shape,
            GlyphAreaField::Shape(NodeShape::Ellipse),
        );
        let m = Mutation::area_delta(delta);
        let mut flagged = unsupported_fields_of_mutation(&m);
        flagged.sort_unstable();
        assert_eq!(
            flagged,
            vec!["line-height", "outline", "shape", "zoom-visibility"]
        );
    }

    /// A delta that only touches persisted fields (position + scale)
    /// is NOT flagged.
    #[test]
    fn test_supported_only_delta_is_clean() {
        let delta = DeltaGlyphArea::new(vec![
            GlyphAreaField::position(1.0, 2.0),
            GlyphAreaField::scale(20.0),
            GlyphAreaField::Operation(ApplyOperation::Assign),
        ]);
        let m = Mutation::area_delta(delta);
        assert!(unsupported_fields_of_mutation(&m).is_empty());
    }
}
