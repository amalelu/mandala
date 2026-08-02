// SPDX-License-Identifier: MPL-2.0

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
    /// Top-level filter gate. When `Some`, every candidate
    /// element collected by `target_scope` is tested against the
    /// predicate; mutations only land on elements that match.
    /// Wired by the application's `apply_custom_mutation` →
    /// `apply_to_tree` path (`src/application/document/custom.rs`).
    ///
    /// Common idioms:
    ///
    /// - `Predicate { fields: [(GfxElementField::Flag(Flag::SectionRoot),
    ///   Comparator::Equals(false))], … }` — match elements with
    ///   `SectionRoot` set; pairs with `target_scope: SelfOnly` to
    ///   land mutations on every section but skip the chrome-only
    ///   container.
    /// - The negation form (`Comparator::Equals(true)`) matches
    ///   elements with `SectionRoot` clear (the container + child
    ///   mind-nodes).
    /// - `Predicate::new()` (empty fields, `always_match=false`)
    ///   matches nothing — a footgun. The apply path warns when a
    ///   gate filters every candidate so authors notice the
    ///   no-op rather than chase a missing visual change.
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
        self.contexts.iter().any(|c| c == query || c.starts_with(&dotted))
    }

    /// `true` when the mutation should be hidden from user-facing
    /// surfaces (console listing, future palette). An empty
    /// `contexts` set is treated as internal so mutations shipped
    /// by internal app code without declaring contexts stay hidden.
    pub fn is_internal(&self) -> bool {
        self.contexts.is_empty() || self.contexts.iter().any(|c| c == contexts::INTERNAL)
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
///
/// **Privilege model.** `MacroStep::CustomMutation` (in
/// `crate::application::macros`) can carry these from any macro tier,
/// including non-User tiers shipped from untrusted sources. Today
/// every variant is a pure in-memory canvas-theme write, which is
/// safe to expose. **Any new variant that performs file I/O, network
/// access, arbitrary content load, or cross-process side effects MUST
/// be gated at the `dispatch_macro` site** (parallel to the
/// `SourceTier::allows_console_line` gate) to prevent a hostile
/// shared mindmap from invoking it. Marked `#[non_exhaustive]` so
/// adding such a variant outside this crate is impossible — review
/// the privilege model when extending.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
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
///
/// `EnumIter` is load-bearing, not decoration. Adding a variant here
/// must fail the suite rather than leave a hand-kept list one row
/// short: the `covers_reach` table pin, the serde round-trip, the
/// differential harness's scope list, and the `target_scope` value
/// list published in `format/schema.md` are all driven off
/// `TargetScope::iter()`, so none of them can silently omit a new
/// variant (CLAUDE.md §5 — make the drift impossible, don't fix it
/// once).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, strum_macros::EnumIter)]
pub enum TargetScope {
    /// Apply to the triggering node itself.
    SelfOnly,
    /// Apply to direct children of the triggering node.
    Children,
    /// Apply to all descendants recursively.
    Descendants,
    /// Apply to the triggering node AND all descendants.
    SelfAndDescendants,
    /// Apply to the parent of the triggering node. Resolves to the
    /// empty set on a root node, which has no parent — the mutation
    /// then snapshots nothing, mutates nothing, and pushes no undo
    /// entry.
    Parent,
    /// Apply to all siblings of the triggering node — every *other*
    /// child of its parent. The triggering node itself is excluded;
    /// pair with `SelfAndDescendants` on the parent if you want it
    /// included.
    ///
    /// **A root node has no siblings.** "Sibling" here means "shares
    /// my parent", and a root has no parent to share, so the other
    /// roots of a multi-root map are deliberately *not* treated as
    /// siblings — there is no `parent_id` the application layer could
    /// key the sibling list on, and "every other root" is a different
    /// relation that would silently fan a node-local mutation across
    /// unrelated trees. A `Siblings` mutation triggered on a root is
    /// therefore a well-defined no-op: the target set is empty,
    /// nothing is snapshotted, and no undo entry is pushed.
    Siblings,
    /// Apply to every section of the triggering node — bypasses
    /// the container fan-out so text / font / region mutations
    /// land on the section-areas only. The structural counterpart
    /// to the `GfxElementField::Flag(SectionRoot)` predicate
    /// gate: `SectionsOnly` selects sections by tree shape,
    /// the predicate selects them by element flag. Channel-space
    /// disambiguation: `SectionsOnly` won't reach a child mind-
    /// node that shares the section's channel (the predicate
    /// alone cannot guarantee that on a triggering node whose
    /// scope spreads beyond `SelfOnly`).
    SectionsOnly,
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
///
/// # Extraction is all-or-nothing, and it keys on payload equality
///
/// The extracted list is applied **once per scope-collected target**.
/// For that single list to mean what the AST says, two things have to
/// hold of every node beneath the root:
///
/// 1. **It must be evaluatable by the flat path.** A single
///    unevaluatable node anywhere in the AST declines the whole
///    mutator, so the apply site warns and skips instead of applying
///    a partial reading of it. Accepting
///    `Macro{[L1], children: [Instruction{RepeatWhile(matches-nothing)}
///    {Macro[L2]}]}` on the strength of its root alone would land
///    `L1` everywhere and `L2` nowhere — the same "applied result
///    differs from the authored AST" defect the `always_match` guard
///    below exists to prevent, just one level down and without the
///    warning.
/// 2. **Every payload it carries must equal the one returned.**
///    Extractability alone is not enough, and this is the sharper of
///    the two rules: `Macro{[L1], children: [Macro{[L2]}]}` is
///    *entirely* extractable and still cannot be collapsed, because
///    one flat list cannot be both `L1` and `L2`. Returning `L1` there
///    would blanket every target with `L1` and land `L2` nowhere —
///    exactly the failure rule 1 exists to prevent, reached by a
///    different road. So a differing payload declines too.
///
///    The rule is deliberately conservative rather than permissive:
///    concatenating the descendant payloads instead would turn
///    [`scope::self_and_descendants`] — whose root and nested `Macro`
///    carry two clones of the *same* list — into a double-apply.
///    Agreement is what makes that helper's duplicate payload a
///    no-op rather than a special case.
///
///    "Agree" is
///    [`Mutation::writes_the_same`](crate::gfx_structs::mutator::Mutation::writes_the_same),
///    not `==`. Derived `PartialEq` over `f32` is not reflexive, so
///    under `==` a `NaN` in that helper's duplicated payload would
///    make it disagree with **itself** and decline, while the same
///    payload under [`scope::self_only`] — which has nothing to
///    compare — applied. Two helpers differing only in scope must
///    not differ in whether the mutation runs.
///
/// [`MutationSrc`](crate::mutator_builder::MutationSrc) payloads on an
/// `Instruction` wrapper are unevaluatable by rule 1 for the same
/// reason: `MutationSrc::Runtime` is resolved by a
/// [`SectionContext`](crate::mutator_builder::SectionContext) the flat
/// path never consults, and `MutationSrc::AreaDelta` is a per-cell
/// template. Both scope helpers that build wrappers set
/// `mutation: MutationSrc::None`, so requiring it costs nothing.
///
/// A [`MutatorNode::Void`] on **channel 0** carries no mutation, so
/// it is transparent: it passes its children's payload through, and
/// an empty one can lose nothing — declining on it would kill a
/// mutator to save a payload that does not exist.
///
/// Channel 0 is the condition, not a formality. A `Void`'s channel is
/// branch routing: [`build`](crate::mutator_builder::build) emits
/// `GfxMutator::new_void(channel)` and the walker aligns that node's
/// children only against the target child on the matching channel, so
/// `Void{channel: 7}` is a routing gate rather than pure grouping.
/// The flat path is channel-blind by construction — it produces one
/// list applied to whole elements, with nothing to route on — and
/// `Macro` has always inherited that limitation (load-bearing for the
/// `SectionsOnly` fan-out, where a channel-0 `Macro` is deliberately
/// applied to section-areas on channels 1..n). What is *not* on offer
/// is widening the accepted set on the strength of a rationale that
/// does not hold: a `Void` used off channel 0 says something the flat
/// list cannot say, so it keeps the `7f3a87c` behavior and declines.
/// Every scope helper builds on channel 0, so nothing this
/// transparency was introduced for is affected.
///
/// `Single` declines in every form, including
/// `Single { mutation: MutationSrc::None }` — which is as payload-free
/// as an empty `Void` and could safely be admitted. It is not,
/// because `Single`'s reason to exist is its `ChannelSrc`: unlike
/// `Void` it is a *leaf* whose channel selects the target it writes,
/// so admitting the payload-free case would be widening into the one
/// node kind whose whole meaning is the routing the flat path drops,
/// to gain a node that does nothing. Conservative on purpose.
///
/// Costs: O(AST size). Clones each payload-bearing node's literal
/// list once — including the clones it compares and discards — so a
/// declined mutator still allocates. The comparison itself allocates
/// only when two payloads are not `==` (see
/// [`Mutation::writes_the_same`](crate::gfx_structs::mutator::Mutation::writes_the_same)).
/// Called on every apply (three times: once by the unsupported-field
/// warning and once per tree), plus once per animation start; none of
/// those is a frame path.
pub fn flat_mutations(mutator: &MutatorNode) -> Option<Vec<crate::gfx_structs::mutator::Mutation>> {
    // The outer `Option` is the decline verdict; the inner one
    // distinguishes "carries this payload" from "payload-free
    // structure". A root that reaches the end payload-free (a bare
    // `Void`, a childless wrapper) has no list to apply and declines,
    // which is what makes the apply site warn rather than silently
    // run an empty mutation.
    extract(mutator)?
}

/// One node's contribution to the flat list.
///
/// - `None` — decline: this node (or something under it) is not
///   evaluatable by the flat path, or two payloads below it disagree.
/// - `Some(None)` — evaluatable, but payload-free structure.
/// - `Some(Some(list))` — evaluatable, and the subtree agrees on
///   `list`.
///
/// Cost: see [`flat_mutations`].
fn extract(node: &MutatorNode) -> Option<Option<Vec<crate::gfx_structs::mutator::Mutation>>> {
    use crate::mutator_builder::{InstructionSpec, MutationListSrc, MutationSrc, MutatorNode as N};
    let (mut payload, children) = match node {
        // Pure structural grouping — no payload of its own. Channel
        // 0 only: off channel 0 a `Void` is a routing gate the flat
        // path cannot honor, so it falls through and declines.
        N::Void { channel: 0, children } => (None, children.as_slice()),
        // The literal list is this node's payload; the children below
        // must agree with it.
        N::Macro {
            mutations: MutationListSrc::Literal(list),
            children,
            ..
        } => (Some(list.clone()), children.as_slice()),
        // `Instruction(RepeatWhile(always_true))` wrappers (built by
        // `scope::descendants` and the inner branch of
        // `scope::self_and_descendants`) carry a child Macro with the
        // actual mutations. The flat-apply path drives the iteration
        // via `collect_affected_node_ids`, so the wrapper's
        // repeat-while semantics are redundant and its children's
        // literal list can be extracted directly.
        //
        // `mutation: MutationSrc::None` is required: a wrapper
        // carrying its own per-step `Runtime` or `AreaDelta` payload
        // is a payload the flat path provably cannot evaluate, and
        // unwrapping past it would drop it in silence.
        N::Instruction {
            instruction: InstructionSpec::RepeatWhileAlwaysTrue,
            mutation: MutationSrc::None,
            children,
            ..
        } => (None, children.as_slice()),
        // `always_match` is the *only* admissible predicate test,
        // because it is the only thing [`Predicate::test`]
        // short-circuits on. Everything else — including an empty
        // field list — is a predicate that filters. In particular a
        // bare `Predicate::new()` (`fields: []`,
        // `always_match: false`) matches **nothing**, the footgun
        // documented on [`CustomMutation::predicate`]; unwrapping it
        // here would land the Macro's payload on every node the scope
        // collected, which is the exact inverse of what the AST
        // authorizes.
        N::Instruction {
            instruction: InstructionSpec::RepeatWhile(p),
            mutation: MutationSrc::None,
            children,
            ..
        } if p.always_match => (None, children.as_slice()),
        // Everything else declines: `MapChildren` and the other
        // walking instructions have no flat equivalent, a
        // `MutationListSrc::Runtime` Macro needs a section context,
        // `Single` is a leaf whose `ChannelSrc` selects the target it
        // writes and the flat path has nowhere to put that, a `Void`
        // off channel 0 is a routing gate for the same reason, and
        // `Repeat` expands at build time.
        _ => return None,
    };
    for child in children {
        match (&payload, extract(child)?) {
            // Payload-free child: nothing to reconcile.
            (_, None) => {}
            // First payload found in this subtree.
            (None, Some(list)) => payload = Some(list),
            // A second payload must say the same thing, or the flat
            // list cannot represent both — decline (rule 2 above).
            (Some(have), Some(list)) if !lists_agree(have, &list) => return None,
            (Some(_), Some(_)) => {}
        }
    }
    Some(payload)
}

/// Whether two payload lists would write the same thing, element by
/// element.
///
/// Element-wise
/// [`Mutation::writes_the_same`](crate::gfx_structs::mutator::Mutation::writes_the_same)
/// rather than `==` on the slices, because the derived `PartialEq`
/// over `f32` is not reflexive and this predicate decides whether a
/// mutator runs at all — see that method for why a `NaN` otherwise
/// makes [`scope::self_and_descendants`] decline a payload
/// [`scope::self_only`] applies.
///
/// Element-wise rather than whole-list so each element gets the
/// full union: `[NaN, 0.0]` agrees with `[NaN, -0.0]`, where a
/// list-level comparison would find neither disjunct true of the
/// pair as a whole.
///
/// Cost: O(n) comparisons, no allocation unless an element
/// disagrees under `==`.
fn lists_agree(
    a: &[crate::gfx_structs::mutator::Mutation],
    b: &[crate::gfx_structs::mutator::Mutation],
) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.writes_the_same(y))
}

/// Which node sets, relative to the anchor, the mutator AST could
/// touch at apply time. Returned by [`mutator_reach`] and compared
/// against the declared [`CustomMutation::target_scope`] to catch
/// authoring mistakes where the undo-snapshot scope is narrower
/// than the mutator's actual reach (which silently loses edits on
/// undo). Ordered from narrowest to widest.
///
/// `EnumIter` for the same reason as [`TargetScope`]: the
/// `covers_reach` table is pinned over both axes, so a new reach can
/// no more slip past the tests than a new scope can.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, strum_macros::EnumIter)]
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
    ///
    /// # The anchoring model this encodes
    ///
    /// The application layer resolves a scope to a **target set**
    /// (`collect_affected_node_ids` in
    /// `src/application/document/custom/mod.rs`) and then anchors the
    /// mutator **at each target in turn** — see the [`scope`] module
    /// header: "the application layer is responsible for iterating
    /// the right set of targets ... and anchoring the mutator at each
    /// of them", which is what the flat-apply path does for every
    /// scope and what the announced walker path will do too. The undo
    /// snapshot is exactly that same target set.
    ///
    /// So the pairing has complete **undo coverage** iff the target
    /// set is **closed** under the mutator's reach: for every target
    /// `t`, everything a `reach`-wide mutator anchored at `t` can
    /// write is itself a target. Note this is a property of the
    /// *target set*, not of the triggering node — the trigger is only
    /// in the set for the three scopes that name it.
    ///
    /// **Closure is about undo coverage and nothing else.** It is not
    /// a claim that the payload lands once per node. Per-target
    /// anchoring runs the mutator once for every target that reaches
    /// a given node, so `SelfAndDescendants` paired with a
    /// `Descendants` reach anchors at each node of the subtree and
    /// writes a node at depth `k` below the anchor `k + 1` times.
    /// `covers_reach` still returns `true`, correctly — the snapshot
    /// covers every one of those writes — but a non-idempotent
    /// payload (a relative nudge, say) compounds. Today's flat-apply
    /// path collapses the AST to one list and applies it once per
    /// target, so nothing compounds yet; the walker path is where
    /// this needs an answer.
    ///
    /// Applying that test to each scope, with `n` the triggering node:
    ///
    /// | scope | target set | closed under `Children` reach? |
    /// |---|---|---|
    /// | `SelfOnly` | `{n}` | no — `n`'s children are not targets |
    /// | `Parent` | `{parent(n)}` | no — `n` itself is not a target |
    /// | `Children` | `children(n)` | no — the **grandchildren** are not targets |
    /// | `Siblings` | `siblings(n)` | no — a sibling's children are not targets |
    /// | `SectionsOnly` | `{n}` | conservatively no — see below |
    /// | `Descendants` | `descendants(n)` | yes — a descendant's children are descendants |
    /// | `SelfAndDescendants` | `{n} ∪ descendants(n)` | yes — same closure |
    ///
    /// Only the two descendant scopes are closed under anything wider
    /// than the anchor itself, so every other scope requires
    /// `MutatorReach::SelfOnly`. The `Children` and `Siblings` rows
    /// are the ones that used to read `reach <= Children`: that
    /// encoded an anchor-at-the-trigger model the application layer
    /// never implemented, and approved pairings whose snapshot was a
    /// level shallower than the write set (issue #19-B).
    ///
    /// **`SectionsOnly` is deliberately stricter than it has to be.**
    /// Its targets are the anchor's section-areas, and a section's
    /// arena subtree holds only that section's own `GlyphModel` — all
    /// of it inside the anchor's `MindNode`, hence inside the
    /// one-node snapshot. A wider reach would in fact be covered.
    /// The gate still demands `SelfOnly` because the asymmetry is
    /// not symmetric in cost: over-strict yields a spurious `warn!`,
    /// over-permissive yields silently unrecoverable undo.
    ///
    /// This gate is advisory — the apply path logs a `warn!` on a
    /// `false` verdict and still applies the mutation. It does not
    /// widen or narrow the snapshot, which is governed solely by
    /// `collect_affected_node_ids`.
    ///
    /// Costs: O(1) — a match over two payload-free enums, no
    /// allocation, no tree access.
    pub fn covers_reach(&self, reach: MutatorReach) -> bool {
        match self {
            TargetScope::SelfOnly
            | TargetScope::Parent
            | TargetScope::SectionsOnly
            | TargetScope::Children
            | TargetScope::Siblings => reach == MutatorReach::SelfOnly,
            TargetScope::Descendants | TargetScope::SelfAndDescendants => true,
        }
    }
}

// **Note on the predicate-gate / target_scope interaction.**
// `covers_reach` checks the AST-vs-scope pairing only — it does
// not consult `CustomMutation.predicate`. A predicate that
// narrows the apply-time element set (e.g. a `Flag(SectionRoot)`
// gate that lands the mutation only on sections) does NOT reduce
// the undo-snapshot window: the snapshot still covers every
// node in the `target_scope`-collected set, even though only a
// subset of those nodes' elements actually mutated. This is by
// design — the predicate-filtered fallout is *node-scoped*, not
// *element-scoped*, and rebuilding the model from a whole-node
// clone trivially restores any per-element subset that landed.
// The over-clone is wasteful but never lossy.
//
// The reverse direction is the bug shape `covers_reach` does
// catch: a `MutatorReach::Children` mutator paired with
// `target_scope: SelfOnly` writes the anchor's children, which
// the one-node snapshot didn't capture, dropping their state on
// undo. The same argument one level down is why `Children` and
// `Siblings` also demand `SelfOnly` reach — see the closure table
// on `covers_reach`.
