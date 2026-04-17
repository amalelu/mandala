//! Serde intermediaries for [`super::CustomMutation`].
//!
//! `CustomMutationIn` is the on-disk deserialization shape; it
//! accepts both the new `mutator`-bearing form and the legacy
//! `mutations` + `target_scope` form and synthesizes a MutatorNode
//! for the legacy case. `CustomMutationOut` is the canonical
//! serialization shape — always writes the new form so resaving a
//! legacy map upgrades it in place.

use serde::{Deserialize, Serialize};

use crate::gfx_structs::mutator::Mutation;
use crate::gfx_structs::predicate::Predicate;
use crate::mutator_builder::MutatorNode;

use super::{
    scope, CustomMutation, DocumentAction, MutationBehavior, TargetScope,
};

/// Accepts both the new `mutator`-bearing form and the legacy
/// `mutations` + `target_scope` pair. `mutator` takes precedence
/// when both are present.
#[derive(Deserialize)]
pub(super) struct CustomMutationIn {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub contexts: Vec<String>,
    #[serde(default)]
    pub mutator: Option<MutatorNode>,
    /// Legacy flat-list payload. Combined with `target_scope` by
    /// [`scope`] helpers to synthesize a MutatorNode when `mutator`
    /// is absent.
    #[serde(default)]
    pub mutations: Vec<Mutation>,
    pub target_scope: TargetScope,
    #[serde(default)]
    pub behavior: MutationBehavior,
    #[serde(default)]
    pub predicate: Option<Predicate>,
    #[serde(default)]
    pub document_actions: Vec<DocumentAction>,
    #[serde(default)]
    pub timing: Option<crate::mindmap::animation::AnimationTiming>,
}

impl From<CustomMutationIn> for CustomMutation {
    fn from(v: CustomMutationIn) -> Self {
        let mutator = v.mutator.or_else(|| {
            // Translate legacy payload via scope helpers. Parent /
            // Siblings + flat mutations map to a `self_only` MutatorNode
            // (the app layer iterates structural targets and anchors
            // this mutator at each); all other scopes map directly.
            if v.mutations.is_empty() {
                None
            } else {
                Some(match v.target_scope {
                    TargetScope::SelfOnly
                    | TargetScope::Children
                    | TargetScope::Parent
                    | TargetScope::Siblings => scope::self_only(v.mutations),
                    TargetScope::Descendants => scope::descendants(v.mutations),
                    TargetScope::SelfAndDescendants => {
                        scope::self_and_descendants(v.mutations)
                    }
                })
            }
        });
        CustomMutation {
            id: v.id,
            name: v.name,
            description: v.description,
            contexts: v.contexts,
            mutator,
            target_scope: v.target_scope,
            behavior: v.behavior,
            predicate: v.predicate,
            document_actions: v.document_actions,
            timing: v.timing,
        }
    }
}

/// Canonical serialization shape — always writes the new `mutator`
/// form, omits empty-default fields for a terse on-disk shape.
#[derive(Serialize)]
pub(crate) struct CustomMutationOut {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub contexts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mutator: Option<MutatorNode>,
    pub target_scope: TargetScope,
    pub behavior: MutationBehavior,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predicate: Option<Predicate>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub document_actions: Vec<DocumentAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing: Option<crate::mindmap::animation::AnimationTiming>,
}

impl From<CustomMutation> for CustomMutationOut {
    fn from(v: CustomMutation) -> Self {
        CustomMutationOut {
            id: v.id,
            name: v.name,
            description: v.description,
            contexts: v.contexts,
            mutator: v.mutator,
            target_scope: v.target_scope,
            behavior: v.behavior,
            predicate: v.predicate,
            document_actions: v.document_actions,
            timing: v.timing,
        }
    }
}
