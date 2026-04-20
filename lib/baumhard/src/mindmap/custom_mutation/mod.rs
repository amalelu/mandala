//! Custom mutation carrier — identity + metadata + payload.
//!
//! A `CustomMutation` is a named, reusable bundle the host application
//! dispatches in response to triggers (clicks, hovers, key bindings)
//! or console commands (`mutation apply`). The payload is a
//! [`MutatorNode`](crate::mutator_builder::MutatorNode) AST that the
//! [`crate::mutator_builder`] walker compiles to a
//! `MutatorTree<GfxMutator>` at apply time. Simple mutations bake
//! their `Vec<Mutation>` into the AST via
//! `scope::self_only`, `scope::descendants`, or
//! `scope::self_and_descendants`;
//! size-aware mutations use a `MutatorNode` with runtime holes and a
//! `SectionContext` registered by the host application under the
//! mutation's `id`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::gfx_structs::predicate::Predicate;
use crate::mutator_builder::MutatorNode;

/// Well-known context tags for `CustomMutation::contexts` — the
/// stable vocabulary for where/on-what a mutation applies.
pub mod contexts;
/// Constructor helpers producing a `MutatorNode` equivalent to each
/// legacy `TargetScope` variant.
pub mod scope;

mod serialized;

#[cfg(test)]
mod tests;

/// A named, reusable bundle of mutation operations attached to
/// nodes, triggered by user interaction, or invoked explicitly via
/// the console.
///
/// The serde impl accepts two shapes:
/// - **new** — a `mutator` field carrying a [`MutatorNode`] AST. Produced
///   by save + canonical for new authorship.
/// - **legacy** — `mutations: Vec<Mutation>` + `target_scope` (no
///   `mutator` field). Translated on load via [`scope`] helpers.
///   `maptool` does not yet auto-rewrite these files; they stay in
///   the legacy shape until resaved, at which point the canonical
///   new shape is emitted.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(from = "serialized::CustomMutationIn", into = "serialized::CustomMutationOut")]
pub struct CustomMutation {
    /// Unique identifier for lookup in the mutation registry.
    pub id: String,
    /// Human-readable name. One short line.
    pub name: String,
    /// One-line human-readable explanation shown in `mutation list`
    /// and expanded by `mutation help <id>`. Multi-paragraph text
    /// is permitted; the console renders line-by-line.
    pub description: String,
    /// Tags describing where and on what this mutation is meant to
    /// run. Dotted namespaces group related tags (`map.node`,
    /// `map.tree`); see [`contexts`] for well-known constants. Empty
    /// set is treated as `["internal"]` — see [`Self::is_internal`].
    pub contexts: Vec<String>,
    /// The mutation payload — a [`MutatorNode`] AST. `None` for
    /// mutations that only do document-level actions (theme switch,
    /// etc.) without touching any tree node. Runtime holes in the
    /// AST are resolved at apply time via a
    /// [`crate::mutator_builder::SectionContext`] the host application
    /// registers under this mutation's `id`.
    pub mutator: Option<MutatorNode>,
    /// Which nodes to snapshot + sync for undo. The payload
    /// [`MutatorNode`] is responsible for actually performing the
    /// changes; this field tells the undo path which model nodes to
    /// snapshot before apply and which to sync back after. Mirrors
    /// the MutatorNode's reach — a `SelfOnly` mutator pairs with
    /// `SelfOnly` scope, a `RepeatWhile`-over-descendants mutator
    /// pairs with `Descendants` scope, etc. [`scope`] carries
    /// matching MutatorNode constructors for each variant.
    pub target_scope: TargetScope,
    /// Whether this mutation persists to the model or is a visual toggle.
    #[serde(default)]
    pub behavior: MutationBehavior,
    /// **Reserved — not yet consumed by the apply path.** When
    /// wired (a future session), this will gate mutator application
    /// per target node: for each node the scope-collected set
    /// returns, the predicate will be tested against the node's
    /// GfxElement and only matching nodes receive the mutator's
    /// effect. Today the field round-trips through serde and is
    /// preserved on save, but `apply_custom_mutation` never checks
    /// it. Mutation authors may populate it now for forward
    /// compatibility; it is effectively a no-op until consumed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predicate: Option<Predicate>,
    /// Optional canvas/document-level actions that fire alongside the
    /// node mutations. These target the `MindMap` itself (theme, etc.)
    /// rather than any tree node, so they're dispatched separately
    /// from the node mutation path.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub document_actions: Vec<DocumentAction>,
    /// Optional timing envelope. When `Some(timing)` with non-zero
    /// `duration_ms`, the trigger dispatcher starts an
    /// `AnimationInstance` instead of applying the mutation
    /// instantly. `None` (or `Some` with `duration_ms == 0`) means
    /// apply instantly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timing: Option<crate::mindmap::animation::AnimationTiming>,
}

impl CustomMutation {
    /// `true` iff any context equals `query` exactly or sits within
    /// the `query.` dotted sub-namespace. `matches_context("map")`
    /// hits both `map.node` and `map.tree` entries.
    pub fn matches_context(&self, query: &str) -> bool {
        let dotted = format!("{}.", query);
        self.contexts
            .iter()
            .any(|c| c == query || c.starts_with(&dotted))
    }

    /// `true` when the mutation should be hidden from user-facing
    /// surfaces (console listing, future palette). An empty
    /// `contexts` set is treated as internal so mutations shipped
    /// by internal app code without declaring contexts stay hidden.
    pub fn is_internal(&self) -> bool {
        self.contexts.is_empty()
            || self.contexts.iter().any(|c| c == contexts::INTERNAL)
    }

    /// `true` when the mutation targets a mindmap — the default
    /// filter for `mutation list`.
    pub fn targets_map(&self) -> bool {
        self.matches_context(contexts::MAP)
    }
}

/// An action that operates on the map/document state rather than any
/// specific tree node. Delivered alongside node mutations via the same
/// `CustomMutation` carrier so a single trigger can do both at once.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum DocumentAction {
    /// Copy a named preset from `canvas.theme_variants` into the live
    /// `canvas.theme_variables`. Silently ignored if the variant does
    /// not exist (graceful — matches how `resolve_var` handles misses).
    SetThemeVariant(String),
    /// Overwrite the live `canvas.theme_variables` with an ad-hoc map.
    /// Existing keys not mentioned in the new map are preserved; any
    /// key in the new map overwrites the previous value.
    SetThemeVariables(HashMap<String, String>),
}

/// Controls whether a mutation is a one-shot persistent change or a toggle.
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub enum MutationBehavior {
    /// Apply once, sync to model, persist with Ctrl+S.
    #[default]
    Persistent,
    /// Toggle: first trigger applies, second trigger reverses.
    /// For OnHover: applies on enter, reverses on leave.
    Toggle,
}

/// Scope hint the undo / model-sync path uses to decide which model
/// nodes to snapshot before apply and which to sync back afterward.
/// The mutation payload [`CustomMutation::mutator`] performs the
/// actual tree edits; this enum just tells the app layer which set
/// of MindNodes are in the reach.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum TargetScope {
    /// Apply to the triggering node itself.
    SelfOnly,
    /// Apply to direct children of the triggering node.
    Children,
    /// Apply to all descendants recursively.
    Descendants,
    /// Apply to the triggering node AND all descendants.
    SelfAndDescendants,
    /// Apply to the parent of the triggering node.
    Parent,
    /// Apply to all siblings of the triggering node.
    Siblings,
}

/// An event condition that causes a CustomMutation to fire.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Trigger {
    /// Fired on mouse click on the node.
    OnClick,
    /// Fired on mouse hover entering the node bounds.
    OnHover,
    /// Fired when a specific key is pressed while node is selected.
    OnKey(String),
    /// Fired when a hyperlink-style text span is clicked.
    OnLink(String),
}

/// Associates a Trigger with a CustomMutation, optionally filtered by platform.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TriggerBinding {
    /// Which trigger fires this binding.
    pub trigger: Trigger,
    /// The custom mutation ID to execute (references CustomMutation.id).
    pub mutation_id: String,
    /// Platform contexts where this trigger is active. Empty = all platforms.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contexts: Vec<PlatformContext>,
}

/// Runtime context for filtering triggers. Detected at startup.
/// On WASM, the web layer detects touch vs pointer and passes it in.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum PlatformContext {
    /// Native builds with pointer and keyboard.
    Desktop,
    /// WASM builds with pointer and keyboard.
    Web,
    /// Touch-only devices (no hover, limited keyboard).
    Touch,
}

/// Apply a set of mutations directly to a single GfxElement. Retained
/// from the pre-unification shape because the app layer still uses it
/// for every [`TargetScope`] — it iterates the affected model nodes
/// and calls this helper per-target with the flat Mutation list
/// extracted from the MutatorNode via [`flat_mutations`].
pub fn apply_mutations_to_element(
    mutations: &[crate::gfx_structs::mutator::Mutation],
    target: &mut crate::gfx_structs::element::GfxElement,
) {
    for mutation in mutations {
        mutation.apply_to(target);
    }
}

/// Extract the flat `Vec<Mutation>` carried by a MutatorNode whose
/// root is a `Macro` with a `MutationListSrc::Literal` payload —
/// the shape emitted by [`scope::self_only`],
/// [`scope::self_and_descendants`], and any other scope helper.
/// Returns `None` for MutatorNode shapes the app layer's flat-apply
/// path can't evaluate (runtime holes, Single-rooted trees, or
/// Instruction roots without a Macro sibling).
///
/// The flat-apply path uses this to preserve today's iterate-targets-
/// and-apply-per-target semantics while the richer `mutator_builder`
/// walker path is phased in for size-aware mutations in a separate
/// session.
pub fn flat_mutations(
    mutator: &MutatorNode,
) -> Option<Vec<crate::gfx_structs::mutator::Mutation>> {
    use crate::mutator_builder::{MutationListSrc, MutatorNode as N};
    match mutator {
        N::Macro {
            mutations: MutationListSrc::Literal(list),
            ..
        } => Some(list.clone()),
        _ => None,
    }
}

/// Which node sets, relative to the anchor, the mutator AST could
/// touch at apply time. Returned by [`mutator_reach`] and compared
/// against the declared [`CustomMutation::target_scope`] to catch
/// authoring mistakes where the undo-snapshot scope is narrower
/// than the mutator's actual reach (which silently loses edits on
/// undo). Ordered from narrowest to widest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MutatorReach {
    /// Only the anchor node. Every non-empty mutator reaches this.
    SelfOnly,
    /// The anchor's direct children (reached via `MapChildren`).
    Children,
    /// Arbitrary descendants (reached via `RepeatWhile` /
    /// `RotateWhile` / `SpatialDescend`).
    Descendants,
}

/// Widest set of nodes the MutatorNode AST can reach. The app layer
/// uses this to detect mismatches between the declared
/// [`CustomMutation::target_scope`] (which governs undo snapshot +
/// model sync reach) and the mutator payload's actual reach — a
/// mutator that walks descendants paired with
/// `TargetScope::SelfOnly` will silently lose descendant edits on
/// undo.
///
/// Cost: O(AST size), no allocation.
pub fn mutator_reach(mutator: &MutatorNode) -> MutatorReach {
    use crate::mutator_builder::{InstructionSpec, MutatorNode as N};
    fn walk(node: &MutatorNode, reach: &mut MutatorReach) {
        let widen = |r: &mut MutatorReach, to: MutatorReach| {
            if to > *r {
                *r = to;
            }
        };
        match node {
            N::Void { children, .. } | N::Macro { children, .. } => {
                for c in children {
                    walk(c, reach);
                }
            }
            N::Instruction {
                instruction,
                children,
                ..
            } => {
                match instruction {
                    InstructionSpec::RepeatWhileAlwaysTrue
                    | InstructionSpec::RepeatWhile(_)
                    | InstructionSpec::RotateWhile(_, _)
                    | InstructionSpec::SpatialDescend(_) => {
                        widen(reach, MutatorReach::Descendants);
                    }
                    InstructionSpec::MapChildren => {
                        widen(reach, MutatorReach::Children);
                    }
                }
                for c in children {
                    walk(c, reach);
                }
            }
            N::Repeat { template, .. } => walk(template, reach),
            N::Single { .. } => {}
        }
    }
    let mut reach = MutatorReach::SelfOnly;
    walk(mutator, &mut reach);
    reach
}

impl TargetScope {
    /// `true` iff this scope's undo-snapshot window covers every
    /// node `reach` could touch. Used to flag mismatched scope +
    /// mutator pairs at apply time.
    pub fn covers_reach(&self, reach: MutatorReach) -> bool {
        match self {
            TargetScope::SelfOnly | TargetScope::Parent => {
                reach == MutatorReach::SelfOnly
            }
            TargetScope::Children | TargetScope::Siblings => {
                reach <= MutatorReach::Children
            }
            TargetScope::Descendants | TargetScope::SelfAndDescendants => true,
        }
    }
}
