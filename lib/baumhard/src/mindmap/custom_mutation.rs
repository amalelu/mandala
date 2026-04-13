use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::{GfxMutator, Instruction, Mutation};
use crate::gfx_structs::predicate::Predicate;
use crate::gfx_structs::tree::MutatorTree;
use crate::core::primitives::Applicable;

/// A named, reusable bundle of mutation operations that can be attached
/// to nodes and triggered by user interaction.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CustomMutation {
    /// Unique identifier for lookup in the mutation registry.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// The mutation operations to apply (reuses existing Mutation enum).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mutations: Vec<Mutation>,
    /// Which nodes to target relative to the triggering node.
    pub target_scope: TargetScope,
    /// Whether this mutation persists to the model or is a visual toggle.
    #[serde(default)]
    pub behavior: MutationBehavior,
    /// Optional predicate filter — only apply to nodes matching this condition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predicate: Option<Predicate>,
    /// Optional canvas/document-level actions that fire alongside the
    /// node mutations. These target the `MindMap` itself (theme, etc.)
    /// rather than any tree node, so they're dispatched separately from
    /// the node mutation path.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub document_actions: Vec<DocumentAction>,
    /// Optional timing envelope. When `Some(timing)` with non-zero
    /// `duration_ms`, the trigger dispatcher starts an
    /// `AnimationInstance` instead of applying the mutation
    /// instantly — each tick produces an interpolated `MutatorTree`
    /// that lands on the live tree until the boundary commit.
    /// `None` (or `Some` with `duration_ms == 0`) means apply
    /// instantly, matching pre-Phase-4 behaviour. Serde-default so
    /// `.mindmap.json` files saved before the field existed still
    /// load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timing: Option<crate::mindmap::animation::AnimationTiming>,
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
    /// key in the new map overwrites the previous value. Pass an empty
    /// map to reset nothing — use `SetThemeVariant` with a preset for
    /// that.
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

/// Defines which nodes to target relative to the triggering node.
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

/// Build a MutatorTree for the SelfOnly or Descendants scope.
/// For SelfOnly, the root is a Macro containing all mutations.
/// For Descendants, uses an Instruction(RepeatWhile(always_true)) that
/// cascades through all descendants via the existing tree walker.
pub fn build_mutator_tree_for_scope(
    mutations: &[Mutation],
    scope: &TargetScope,
) -> MutatorTree<GfxMutator> {
    match scope {
        TargetScope::SelfOnly => {
            MutatorTree::new_with(
                GfxMutator::new_macro(mutations.to_vec(), 0)
            )
        }
        TargetScope::Descendants => {
            // Root = Instruction(RepeatWhile(always_true))
            // Child = Macro with the actual mutations
            let instruction = GfxMutator::Instruction {
                instruction: Instruction::RepeatWhile(Predicate::always_true()),
                channel: 0,
                mutation: Mutation::None,
            };
            let mut tree = MutatorTree::new_with(instruction);
            let child = GfxMutator::new_macro(mutations.to_vec(), 0);
            let child_id = tree.arena.new_node(child);
            tree.root.append(child_id, &mut tree.arena);
            tree
        }
        TargetScope::SelfAndDescendants => {
            // Root = Macro (applies to self)
            // Child = Instruction(RepeatWhile(always_true)) for descendants
            let root_mutator = GfxMutator::new_macro(mutations.to_vec(), 0);
            let mut tree = MutatorTree::new_with(root_mutator);

            let instruction = GfxMutator::Instruction {
                instruction: Instruction::RepeatWhile(Predicate::always_true()),
                channel: 0,
                mutation: Mutation::None,
            };
            let instr_id = tree.arena.new_node(instruction);
            tree.root.append(instr_id, &mut tree.arena);

            let child_macro = GfxMutator::new_macro(mutations.to_vec(), 0);
            let child_id = tree.arena.new_node(child_macro);
            instr_id.append(child_id, &mut tree.arena);

            tree
        }
        // Children, Parent, Siblings are handled by direct arena iteration
        // in the application layer, not via tree walker
        _ => MutatorTree::new_with(GfxMutator::new_macro(mutations.to_vec(), 0)),
    }
}

/// Apply a set of mutations directly to a single GfxElement.
/// Used for scopes where we iterate targets explicitly (Children, Parent, Siblings).
pub fn apply_mutations_to_element(mutations: &[Mutation], target: &mut GfxElement) {
    for mutation in mutations {
        mutation.apply_to(target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx_structs::area::GlyphAreaCommand;

    #[test]
    fn test_custom_mutation_roundtrip() {
        let custom = CustomMutation {
            id: "nudge-right".to_string(),
            name: "Nudge Right".to_string(),
            mutations: vec![
                Mutation::area_command(GlyphAreaCommand::NudgeRight(10.0)),
            ],
            target_scope: TargetScope::Children,
            behavior: MutationBehavior::Persistent,
            predicate: None,
            document_actions: vec![],
            timing: None,
        };

        let json = serde_json::to_string(&custom).unwrap();
        let deserialized: CustomMutation = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, "nudge-right");
        assert_eq!(deserialized.name, "Nudge Right");
        assert_eq!(deserialized.target_scope, TargetScope::Children);
        assert_eq!(deserialized.behavior, MutationBehavior::Persistent);
        assert!(deserialized.predicate.is_none());
        assert_eq!(deserialized.mutations.len(), 1);
        assert!(deserialized.document_actions.is_empty());
    }

    #[test]
    fn test_trigger_binding_roundtrip() {
        let binding = TriggerBinding {
            trigger: Trigger::OnKey("r".to_string()),
            mutation_id: "highlight-red".to_string(),
            contexts: vec![PlatformContext::Desktop, PlatformContext::Web],
        };

        let json = serde_json::to_string(&binding).unwrap();
        let deserialized: TriggerBinding = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.trigger, Trigger::OnKey("r".to_string()));
        assert_eq!(deserialized.mutation_id, "highlight-red");
        assert_eq!(deserialized.contexts.len(), 2);
        assert_eq!(deserialized.contexts[0], PlatformContext::Desktop);
    }

    #[test]
    fn test_trigger_binding_empty_contexts_omitted() {
        let binding = TriggerBinding {
            trigger: Trigger::OnClick,
            mutation_id: "test".to_string(),
            contexts: vec![],
        };

        let json = serde_json::to_string(&binding).unwrap();
        assert!(!json.contains("contexts"), "Empty contexts should be omitted");
    }

    #[test]
    fn test_mutation_behavior_default_is_persistent() {
        let json = r#"{
            "id": "test",
            "name": "Test",
            "mutations": [],
            "target_scope": "SelfOnly"
        }"#;
        let custom: CustomMutation = serde_json::from_str(json).unwrap();
        assert_eq!(custom.behavior, MutationBehavior::Persistent);
    }

    #[test]
    fn test_toggle_behavior_roundtrip() {
        let custom = CustomMutation {
            id: "flash".to_string(),
            name: "Flash".to_string(),
            mutations: vec![],
            target_scope: TargetScope::SelfOnly,
            behavior: MutationBehavior::Toggle,
            predicate: None,
            document_actions: vec![],
            timing: None,
        };

        let json = serde_json::to_string(&custom).unwrap();
        let deserialized: CustomMutation = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.behavior, MutationBehavior::Toggle);
    }

    #[test]
    fn test_document_action_set_theme_variant_roundtrip() {
        let action = DocumentAction::SetThemeVariant("dark".to_string());
        let json = serde_json::to_string(&action).unwrap();
        let back: DocumentAction = serde_json::from_str(&json).unwrap();
        assert_eq!(back, action);
    }

    #[test]
    fn test_document_action_set_theme_variables_roundtrip() {
        let mut vars = HashMap::new();
        vars.insert("--bg".to_string(), "#111".to_string());
        vars.insert("--fg".to_string(), "#eee".to_string());
        let action = DocumentAction::SetThemeVariables(vars);
        let json = serde_json::to_string(&action).unwrap();
        let back: DocumentAction = serde_json::from_str(&json).unwrap();
        assert_eq!(back, action);
    }

    #[test]
    fn test_custom_mutation_with_document_actions_roundtrip() {
        let custom = CustomMutation {
            id: "switch-dark".to_string(),
            name: "Switch to dark".to_string(),
            mutations: vec![],
            target_scope: TargetScope::SelfOnly,
            behavior: MutationBehavior::Persistent,
            predicate: None,
            document_actions: vec![
                DocumentAction::SetThemeVariant("dark".to_string()),
            ],
            timing: None,
        };
        let json = serde_json::to_string(&custom).unwrap();
        let back: CustomMutation = serde_json::from_str(&json).unwrap();
        assert_eq!(back.document_actions.len(), 1);
        assert_eq!(
            back.document_actions[0],
            DocumentAction::SetThemeVariant("dark".to_string())
        );
    }

    #[test]
    fn test_custom_mutation_backwards_compat_without_document_actions() {
        // Old-style JSON without the document_actions field must still parse.
        let json = r#"{
            "id": "test",
            "name": "Test",
            "mutations": [],
            "target_scope": "SelfOnly"
        }"#;
        let custom: CustomMutation = serde_json::from_str(json).unwrap();
        assert!(custom.document_actions.is_empty());
    }

    #[test]
    fn test_all_target_scopes_serialize() {
        let scopes = vec![
            TargetScope::SelfOnly,
            TargetScope::Children,
            TargetScope::Descendants,
            TargetScope::SelfAndDescendants,
            TargetScope::Parent,
            TargetScope::Siblings,
        ];
        for scope in scopes {
            let json = serde_json::to_string(&scope).unwrap();
            let back: TargetScope = serde_json::from_str(&json).unwrap();
            assert_eq!(back, scope);
        }
    }

    #[test]
    fn test_all_triggers_serialize() {
        let triggers = vec![
            Trigger::OnClick,
            Trigger::OnHover,
            Trigger::OnKey("space".to_string()),
            Trigger::OnLink("action:expand".to_string()),
        ];
        for trigger in triggers {
            let json = serde_json::to_string(&trigger).unwrap();
            let back: Trigger = serde_json::from_str(&json).unwrap();
            assert_eq!(back, trigger);
        }
    }
}
