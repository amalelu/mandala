// SPDX-License-Identifier: MPL-2.0

//! Tests for the `CustomMutation` carrier — serde roundtrip,
//! backward-compat with pre-unification JSON, and the
//! `contexts` / `is_internal` / `targets_map` helpers.

use super::*;
use crate::gfx_structs::area::GlyphAreaCommand;
use crate::gfx_structs::mutator::Mutation;

fn sample(id: &str) -> CustomMutation {
    CustomMutation {
        id: id.to_string(),
        name: id.to_string(),
        description: String::new(),
        contexts: vec![],
        mutator: Some(scope::self_only(vec![Mutation::area_command(
            GlyphAreaCommand::NudgeRight(10.0),
        )])),
        target_scope: TargetScope::SelfOnly,
        behavior: MutationBehavior::Persistent,
        predicate: None,
        document_actions: vec![],
        timing: None,
    }
}

#[test]
fn test_roundtrip_preserves_all_fields() {
    let mut cm = sample("nudge");
    cm.description = "Nudge every selected node one pixel right.".into();
    cm.contexts = vec![contexts::MAP_NODE.into()];
    let json = serde_json::to_string(&cm).unwrap();
    let back: CustomMutation = serde_json::from_str(&json).unwrap();
    assert_eq!(back.id, "nudge");
    assert_eq!(back.description, cm.description);
    assert_eq!(back.contexts, cm.contexts);
    assert_eq!(back.target_scope, TargetScope::SelfOnly);
}

#[test]
fn test_empty_description_and_contexts_omitted_from_json() {
    let cm = sample("bare");
    let json = serde_json::to_string(&cm).unwrap();
    assert!(!json.contains("\"description\""));
    assert!(!json.contains("\"contexts\""));
}

#[test]
fn test_legacy_json_with_mutations_and_scope_loads_into_new_shape() {
    // Pre-unification shape: `mutations` + `target_scope`, no
    // `mutator` field. The backward-compat deserializer
    // synthesizes the MutatorNode via `scope` helpers so old maps
    // load unchanged.
    let legacy = r#"{
        "id": "old",
        "name": "Old",
        "mutations": [{"AreaCommand": {"NudgeRight": 5.0}}],
        "target_scope": "Descendants"
    }"#;
    let cm: CustomMutation = serde_json::from_str(legacy).unwrap();
    assert_eq!(cm.id, "old");
    assert_eq!(cm.target_scope, TargetScope::Descendants);
    // Synthesized MutatorNode: Instruction(RepeatWhile) wrapping Macro.
    match &cm.mutator {
        Some(crate::mutator_builder::MutatorNode::Instruction { .. }) => {}
        other => panic!("expected Instruction mutator, got {:?}", other),
    }
}

#[test]
fn test_legacy_json_with_only_document_actions_loads_with_no_mutator() {
    // `theme_demo.mindmap.json` shape: document_actions only.
    // `mutations` absent (serde-default) + `target_scope` present.
    // The synthesized mutator is None — no tree payload.
    let legacy = r#"{
        "id": "switch-dark",
        "name": "Switch to dark theme",
        "target_scope": "SelfOnly",
        "document_actions": [{"SetThemeVariant": "dark"}]
    }"#;
    let cm: CustomMutation = serde_json::from_str(legacy).unwrap();
    assert_eq!(cm.id, "switch-dark");
    assert!(cm.mutator.is_none());
    assert_eq!(cm.document_actions.len(), 1);
}

/// Full round-trip on real-world legacy shape: load
/// `theme_demo.mindmap.json` (three document-actions-only
/// mutations in the pre-unification shape), save it to a temp
/// file through the canonical on-save path, reload, and assert
/// every `custom_mutation` still reflects the source file's
/// semantic content. Guards the silent upgrade-on-save contract
/// the review flagged — a user opening + saving a legacy map
/// should never lose information.
#[test]
fn test_legacy_theme_demo_map_round_trips_through_canonical_save() {
    use crate::mindmap::loader::{load_from_file, save_to_file};
    use std::path::PathBuf;

    let repo_root: PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", ".."].iter().collect();
    let src_path = repo_root.join("maps").join("theme_demo.mindmap.json");
    // Some test runners may not have access to the full repo tree
    // (e.g. crate-scoped cargo test); skip gracefully rather than
    // fail a CI environment that doesn't ship the fixture.
    if !src_path.exists() {
        return;
    }

    let original = load_from_file(&src_path).expect("legacy theme_demo loads");
    assert!(
        !original.custom_mutations.is_empty(),
        "fixture expected to have custom_mutations"
    );

    // Save to a temp file using the canonical on-save path.
    let scratch = crate::util::test_temp::TempDir::new("theme-demo-roundtrip");
    let tmp_path = scratch.join("map.mindmap.json");
    save_to_file(&tmp_path, &original).expect("save_to_file succeeds");

    // Reload and compare field-by-field on every custom_mutation.
    let reloaded = load_from_file(&tmp_path).expect("canonical reloads");
    assert_eq!(reloaded.custom_mutations.len(), original.custom_mutations.len());
    for (before, after) in original
        .custom_mutations
        .iter()
        .zip(reloaded.custom_mutations.iter())
    {
        assert_eq!(before.id, after.id);
        assert_eq!(before.name, after.name);
        assert_eq!(before.target_scope, after.target_scope);
        assert_eq!(before.document_actions, after.document_actions);
        // The synthesized `mutator` is stable across the round-trip:
        // legacy had no `mutations` to translate, so `None` persists.
        assert_eq!(before.mutator.is_none(), after.mutator.is_none());
    }

    let _ = std::fs::remove_file(&tmp_path);
}

#[test]
fn test_matches_context_hits_exact_and_dotted_descendants() {
    let mut cm = sample("x");
    cm.contexts = vec![contexts::MAP_NODE.into(), contexts::MAP_TREE.into()];
    assert!(cm.matches_context(contexts::MAP));
    assert!(cm.matches_context(contexts::MAP_NODE));
    assert!(cm.matches_context(contexts::MAP_TREE));
    assert!(!cm.matches_context(contexts::INTERNAL));
}

#[test]
fn test_is_internal_is_true_on_empty_contexts() {
    let cm = sample("empty");
    assert!(cm.is_internal());
}

#[test]
fn test_is_internal_is_true_when_tag_present() {
    let mut cm = sample("tagged");
    cm.contexts = vec![contexts::INTERNAL.into(), contexts::MAP_NODE.into()];
    assert!(cm.is_internal());
    // targets_map also true because MAP_NODE is under MAP.
    assert!(cm.targets_map());
}

#[test]
fn test_targets_map_hits_any_map_sub_namespace() {
    let mut cm = sample("m");
    cm.contexts = vec![contexts::MAP_TREE.into()];
    assert!(cm.targets_map());
    assert!(!cm.is_internal());
}

#[test]
fn test_trigger_binding_roundtrip() {
    let binding = TriggerBinding {
        trigger: Trigger::OnKey("r".to_string()),
        mutation_id: "highlight-red".to_string(),
        contexts: vec![PlatformContext::Desktop, PlatformContext::Web],
    };
    let json = serde_json::to_string(&binding).unwrap();
    let back: TriggerBinding = serde_json::from_str(&json).unwrap();
    assert_eq!(back.trigger, Trigger::OnKey("r".to_string()));
    assert_eq!(back.contexts.len(), 2);
}

#[test]
fn test_document_action_set_theme_variant_roundtrip() {
    let action = DocumentAction::SetThemeVariant("dark".to_string());
    let json = serde_json::to_string(&action).unwrap();
    let back: DocumentAction = serde_json::from_str(&json).unwrap();
    assert_eq!(back, action);
}

#[test]
fn test_mutation_behavior_default_is_persistent() {
    // Absent `behavior` key in source JSON deserializes to
    // `Persistent` via the struct-level serde default, so maps
    // written before the field existed still load.
    let src = serde_json::json!({
        "id": "b",
        "name": "b",
        "mutator": { "Macro": { "channel": 0, "mutations": { "Literal": [] } } },
        "target_scope": "SelfOnly"
    })
    .to_string();
    let cm: CustomMutation = serde_json::from_str(&src).unwrap();
    assert_eq!(cm.behavior, MutationBehavior::Persistent);
}

// The `all_target_scopes_serialize` that used to live here listed
// six of the seven variants by hand — it never covered
// `SectionsOnly`. Replaced by
// `reach_tests::test_every_target_scope_variant_serializes`, which is
// driven off `TargetScope::iter()` and cannot be one variant short.

#[test]
fn test_all_triggers_serialize() {
    for trigger in [
        Trigger::OnClick,
        Trigger::OnHover,
        Trigger::OnKey("space".to_string()),
        Trigger::OnLink("action:expand".to_string()),
    ] {
        let json = serde_json::to_string(&trigger).unwrap();
        let back: Trigger = serde_json::from_str(&json).unwrap();
        assert_eq!(back, trigger);
    }
}

#[cfg(test)]
mod reach_tests {
    use super::*;
    use crate::gfx_structs::area::GlyphAreaCommand;
    use crate::gfx_structs::mutator::Mutation;

    fn nudge() -> Mutation {
        Mutation::area_command(GlyphAreaCommand::NudgeRight(1.0))
    }

    #[test]
    fn test_scope_self_only_covers_only_self_only_reach() {
        assert!(TargetScope::SelfOnly.covers_reach(MutatorReach::SelfOnly));
        assert!(!TargetScope::SelfOnly.covers_reach(MutatorReach::Children));
        assert!(!TargetScope::SelfOnly.covers_reach(MutatorReach::Descendants));
    }

    /// Per-target anchoring: a `Children`-scoped mutation anchors
    /// the mutator at *each child*, so a `MutatorReach::Children`
    /// mutator reaches the anchor's **grandchildren** — nodes the
    /// `Children` snapshot (`collect_affected_node_ids`) never
    /// captured. Only `SelfOnly` reach is covered.
    #[test]
    fn test_scope_children_covers_only_self_only_reach() {
        assert!(TargetScope::Children.covers_reach(MutatorReach::SelfOnly));
        assert!(!TargetScope::Children.covers_reach(MutatorReach::Children));
        assert!(!TargetScope::Children.covers_reach(MutatorReach::Descendants));
    }

    /// Same argument as [`test_scope_children_covers_only_self_only_reach`]
    /// one level over: a sibling's children are not siblings.
    #[test]
    fn test_scope_siblings_covers_only_self_only_reach() {
        assert!(TargetScope::Siblings.covers_reach(MutatorReach::SelfOnly));
        assert!(!TargetScope::Siblings.covers_reach(MutatorReach::Children));
        assert!(!TargetScope::Siblings.covers_reach(MutatorReach::Descendants));
    }

    #[test]
    fn test_scope_parent_covers_only_self_only_reach() {
        assert!(TargetScope::Parent.covers_reach(MutatorReach::SelfOnly));
        assert!(!TargetScope::Parent.covers_reach(MutatorReach::Children));
        assert!(!TargetScope::Parent.covers_reach(MutatorReach::Descendants));
    }

    #[test]
    fn test_scope_sections_only_covers_only_self_only_reach() {
        assert!(TargetScope::SectionsOnly.covers_reach(MutatorReach::SelfOnly));
        assert!(!TargetScope::SectionsOnly.covers_reach(MutatorReach::Children));
        assert!(!TargetScope::SectionsOnly.covers_reach(MutatorReach::Descendants));
    }

    #[test]
    fn test_scope_self_and_descendants_covers_everything() {
        assert!(TargetScope::SelfAndDescendants.covers_reach(MutatorReach::SelfOnly));
        assert!(TargetScope::SelfAndDescendants.covers_reach(MutatorReach::Children));
        assert!(TargetScope::SelfAndDescendants.covers_reach(MutatorReach::Descendants));
    }

    /// The `(scope, reach)` cells this table is expected to cover —
    /// one row per cell of the full cross product, spelled out by
    /// hand so a future edit to one `covers_reach` arm can't quietly
    /// widen another. The closure argument behind each row is in
    /// [`TargetScope::covers_reach`]'s doc table; the differential
    /// harness in `src/application/document/custom/mod.rs` derives
    /// the same table from real tree structure rather than by hand.
    ///
    /// Hand-written *values*, machine-checked *coverage*: the table
    /// is the pin, and
    /// [`test_covers_reach_table_covers_every_variant_pair`] is what makes
    /// the word "exhaustive" true rather than aspirational.
    fn covers_reach_expected_table() -> Vec<(TargetScope, MutatorReach, bool)> {
        use MutatorReach::{Children, Descendants, SelfOnly};
        vec![
            (TargetScope::SelfOnly, SelfOnly, true),
            (TargetScope::SelfOnly, Children, false),
            (TargetScope::SelfOnly, Descendants, false),
            (TargetScope::Children, SelfOnly, true),
            (TargetScope::Children, Children, false),
            (TargetScope::Children, Descendants, false),
            (TargetScope::Descendants, SelfOnly, true),
            (TargetScope::Descendants, Children, true),
            (TargetScope::Descendants, Descendants, true),
            (TargetScope::SelfAndDescendants, SelfOnly, true),
            (TargetScope::SelfAndDescendants, Children, true),
            (TargetScope::SelfAndDescendants, Descendants, true),
            (TargetScope::Parent, SelfOnly, true),
            (TargetScope::Parent, Children, false),
            (TargetScope::Parent, Descendants, false),
            (TargetScope::Siblings, SelfOnly, true),
            (TargetScope::Siblings, Children, false),
            (TargetScope::Siblings, Descendants, false),
            (TargetScope::SectionsOnly, SelfOnly, true),
            (TargetScope::SectionsOnly, Children, false),
            (TargetScope::SectionsOnly, Descendants, false),
        ]
    }

    /// Whole-table pin: every listed `(scope, reach)` cell agrees
    /// with [`TargetScope::covers_reach`].
    #[test]
    fn test_covers_reach_table_is_stable() {
        for (scope, reach, want) in covers_reach_expected_table() {
            assert_eq!(
                scope.covers_reach(reach),
                want,
                "covers_reach({:?}, {:?}) should be {}",
                scope,
                reach,
                want
            );
        }
    }

    /// What makes [`test_covers_reach_table_is_stable`] *exhaustive*
    /// rather than merely long.
    ///
    /// A hand-typed table is green the day an eighth `TargetScope`
    /// lands with zero rows naming it: the compiler catches the
    /// missing production arms, but nothing catches the missing test
    /// coverage. So the table is checked against
    /// `TargetScope::iter() x MutatorReach::iter()` — every cell of
    /// the cross product present, exactly once, and no row naming a
    /// pair that no longer exists.
    ///
    /// The drive-by finding of this PR is that hand-kept lists drift;
    /// this is that lesson applied to the list the PR itself added
    /// (CLAUDE.md §5).
    #[test]
    fn test_covers_reach_table_covers_every_variant_pair() {
        use std::collections::BTreeSet;
        use strum::IntoEnumIterator;

        let table = covers_reach_expected_table();
        let listed: Vec<(String, String)> = table
            .iter()
            .map(|(scope, reach, _)| (format!("{:?}", scope), format!("{:?}", reach)))
            .collect();

        let mut missing: Vec<String> = Vec::new();
        for scope in TargetScope::iter() {
            for reach in MutatorReach::iter() {
                let cell = (format!("{:?}", scope), format!("{:?}", reach));
                let rows = listed.iter().filter(|c| **c == cell).count();
                if rows != 1 {
                    missing.push(format!("({:?}, {:?}) appears {} time(s)", scope, reach, rows));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "covers_reach_expected_table must name every (TargetScope, MutatorReach) \
             pair exactly once. Offending cells:\n  {}",
            missing.join("\n  ")
        );

        let cross = TargetScope::iter().count() * MutatorReach::iter().count();
        assert_eq!(
            table.len(),
            cross,
            "the table has {} rows but the cross product is {} — a row names a pair \
             that no longer exists",
            table.len(),
            cross
        );
        // Guard the guard: a `BTreeSet` collapse proves the rows are
        // distinct, so `cross` rows really are `cross` cells.
        let distinct: BTreeSet<_> = listed.iter().collect();
        assert_eq!(distinct.len(), table.len(), "duplicate rows in the table");
    }

    /// Every `TargetScope` variant round-trips through serde. Driven
    /// off `EnumIter` — the hand-written list this replaces was
    /// missing `SectionsOnly`, which is precisely the variant this
    /// PR had to add to three prose surfaces by hand.
    ///
    /// No count assertion: `iter()` visiting every variant is the
    /// guarantee `EnumIter` provides, so a floor would be a magic
    /// number that can only go stale and an `==` against
    /// `iter().count()` would be a tautology. Coverage of the enum
    /// is the derive's job; this test's job is the round trip.
    #[test]
    fn test_every_target_scope_variant_serializes() {
        use strum::IntoEnumIterator;
        for scope in TargetScope::iter() {
            let json = serde_json::to_string(&scope).unwrap();
            let back: TargetScope = serde_json::from_str(&json).unwrap();
            assert_eq!(back, scope);
        }
    }

    #[test]
    fn test_scope_descendants_covers_everything() {
        assert!(TargetScope::Descendants.covers_reach(MutatorReach::SelfOnly));
        assert!(TargetScope::Descendants.covers_reach(MutatorReach::Children));
        assert!(TargetScope::Descendants.covers_reach(MutatorReach::Descendants));
    }

    /// The spec surfaces that publish the `TargetScope` vocabulary
    /// must name **every** variant.
    ///
    /// `format/` is normative: a `target_scope` value the code
    /// accepts but the schema never lists is a value no author can
    /// discover, and one the schema lists but the code rejects is a
    /// map that fails to load. This PR had to hand-repair exactly
    /// that drift — `SectionsOnly` was missing from
    /// `format/schema.md`'s value list, from `format/mutations.md`'s
    /// scope table, and from CONCEPTS §4's variant sentence, which
    /// still read "Six variants". Hand-repairing it again next time
    /// is not the fix; failing the suite is (CLAUDE.md §5).
    ///
    /// Reads the docs themselves rather than a copy of them, per
    /// [`crate::util::doc_fixtures`] — so editing the doc is what
    /// moves the test, not editing a string literal beside it.
    ///
    /// Two ways this pin could be green while a surface is missing a
    /// variant, both closed here because both were real:
    ///
    /// 1. **Substring matching.** `Descendants` is a substring of
    ///    `SelfAndDescendants`, so `contains` reported the narrow
    ///    variant as published on the strength of the wide one — and
    ///    `Descendants` could be deleted from all three surfaces
    ///    with the suite green. Matched with
    ///    [`names_token`](crate::util::doc_fixtures::names_token)
    ///    instead, which requires a word boundary.
    /// 2. **Section-wide matching.** Every one of these sections
    ///    also discusses the variants in prose, so deleting a
    ///    variant from the construct that *publishes* it left the
    ///    section still naming it. Narrowed to the publishing block
    ///    with
    ///    [`documented_paragraph`](crate::util::doc_fixtures::documented_paragraph):
    ///    schema's value sentence, mutations' table, CONCEPTS'
    ///    `Variants:` listing.
    ///
    /// A pin that cannot fail is worse than no pin, because it is
    /// trusted.
    ///
    /// Native-only: the specs live on the filesystem and wasm32 has
    /// no filesystem to read them from (`TEST_CONVENTIONS.md` §T9).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn test_every_target_scope_variant_is_published_by_the_docs() {
        use crate::util::doc_fixtures::{documented_paragraph, names_token, repo_path};
        use strum::IntoEnumIterator;

        // (doc path relative to the repo root, heading, and the line
        // that opens the block publishing the vocabulary). All three
        // are surfaces this PR had to repair by hand.
        let surfaces = [
            (
                "format/schema.md",
                "## CustomMutation",
                "`target_scope` is one of",
            ),
            (
                "format/mutations.md",
                "## `target_scope`",
                "| Value | Nodes snapshotted / synced |",
            ),
            ("CONCEPTS.md", "### Target scopes", "Variants:"),
        ];
        let mut gaps: Vec<String> = Vec::new();
        for (doc, heading, marker) in surfaces {
            let body = documented_paragraph(&repo_path(doc), heading, marker);
            for scope in TargetScope::iter() {
                let name = format!("{:?}", scope);
                if !names_token(&body, &name) {
                    gaps.push(format!("{} §{} never names {:?}", doc, heading, scope));
                }
            }
        }
        assert!(
            gaps.is_empty(),
            "a TargetScope variant exists that the docs do not publish:\n  {}\n\
             Add it to the doc — the docs are the contract, this test only reads them.",
            gaps.join("\n  ")
        );
    }

    /// CONCEPTS §4 opens the scope section by counting the variants
    /// in words ("Seven variants telling the dispatcher..."). That
    /// sentence went stale once already — it read "Six" until this
    /// PR found `SectionsOnly` undocumented — and a prose numeral is
    /// exactly what no compiler checks. Pinned to
    /// `TargetScope::iter().count()`, with the neighboring numerals
    /// rejected so a stale one cannot sit alongside the right one.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn test_concepts_variant_count_matches_the_enum() {
        use crate::util::doc_fixtures::{repo_path, section_text};
        use strum::IntoEnumIterator;

        const WORDS: [&str; 8] = ["Three", "Four", "Five", "Six", "Seven", "Eight", "Nine", "Ten"];
        let count = TargetScope::iter().count();
        let expected = WORDS
            .get(count - 3)
            .unwrap_or_else(|| panic!("{count} variants is outside the range this test spells"));

        let body = section_text(&repo_path("CONCEPTS.md"), "### Target scopes");
        let phrase = format!("{expected} variants");
        assert!(
            body.contains(&phrase),
            "CONCEPTS §4 must open with {phrase:?} — there are {count} TargetScope variants"
        );
        for wrong in WORDS.iter().filter(|w| **w != *expected) {
            assert!(
                !body.contains(&format!("{wrong} variants")),
                "CONCEPTS §4 still says {wrong:?} variants; there are {count}"
            );
        }
    }

    #[test]
    fn test_reach_of_self_only_scope_helper_is_self_only() {
        let node = scope::self_only(vec![nudge()]);
        assert_eq!(mutator_reach(&node), MutatorReach::SelfOnly);
    }

    #[test]
    fn test_reach_of_descendants_scope_helper_is_descendants() {
        let node = scope::descendants(vec![nudge()]);
        assert_eq!(mutator_reach(&node), MutatorReach::Descendants);
    }

    #[test]
    fn test_reach_of_self_and_descendants_scope_helper_is_descendants() {
        let node = scope::self_and_descendants(vec![nudge()]);
        assert_eq!(mutator_reach(&node), MutatorReach::Descendants);
    }

    #[test]
    fn test_reach_of_mapchildren_is_children() {
        use crate::mutator_builder::{InstructionSpec, MutationSrc, MutatorNode};
        let node = MutatorNode::Instruction {
            channel: 0,
            instruction: InstructionSpec::MapChildren,
            mutation: MutationSrc::None,
            children: vec![],
        };
        assert_eq!(mutator_reach(&node), MutatorReach::Children);
    }
}

/// [`flat_mutations`] guard coverage.
///
/// The guard decides which `Instruction(RepeatWhile(..))` wrappers
/// the flat-apply path is allowed to unwrap into a literal mutation
/// list. Unwrapping one whose predicate is *not* always-true makes
/// the applied result differ from the authored AST — the flat path
/// has no predicate evaluator, so the Macro's mutations land on
/// every target the scope collected instead of the filtered subset.
/// `always_match` is the only admissible test because it is the only
/// thing [`Predicate::test`] short-circuits on.
#[cfg(test)]
mod flat_mutations_tests {
    use super::*;
    use crate::gfx_structs::area::GlyphAreaCommand;
    use crate::gfx_structs::element::GfxElementField;
    use crate::gfx_structs::mutator::Mutation;
    use crate::gfx_structs::predicate::{Comparator, Predicate};
    use crate::mutator_builder::{InstructionSpec, MutationListSrc, MutationSrc, MutatorNode};

    fn nudge() -> Mutation {
        Mutation::area_command(GlyphAreaCommand::NudgeRight(1.0))
    }

    fn macro_child() -> MutatorNode {
        MutatorNode::Macro {
            channel: 0,
            mutations: MutationListSrc::Literal(vec![nudge()]),
            children: vec![],
        }
    }

    fn repeat_while(predicate: Predicate) -> MutatorNode {
        MutatorNode::Instruction {
            channel: 0,
            instruction: InstructionSpec::RepeatWhile(predicate),
            mutation: MutationSrc::None,
            children: vec![macro_child()],
        }
    }

    /// A bare `Predicate::new()` (`fields: []`, `always_match: false`)
    /// matches **nothing** — the footgun documented on
    /// [`CustomMutation::predicate`]. A `RepeatWhile` wrapping it
    /// must therefore fall through to `None` so the apply site warns
    /// and skips, rather than blanket-applying the Macro's payload
    /// to the whole scope set.
    ///
    /// Red on `eddd53a`: the guard read
    /// `always_match || all(Equals) && fields.is_empty()`, which
    /// parses as `a || (b && c)`. `b` is vacuously true exactly when
    /// `c` is, so the guard collapsed to
    /// `always_match || fields.is_empty()` and admitted this shape.
    #[test]
    fn test_match_nothing_repeat_while_is_not_flat_extractable() {
        let node = repeat_while(Predicate::new());
        assert!(
            flat_mutations(&node).is_none(),
            "a match-nothing RepeatWhile must not flat-apply"
        );
    }

    /// The always-true wrapper `scope::descendants` builds is the
    /// shape the guard exists to admit.
    #[test]
    fn test_always_true_repeat_while_is_flat_extractable() {
        let node = repeat_while(Predicate::always_true());
        assert_eq!(flat_mutations(&node).map(|m| m.len()), Some(1));
    }

    /// `always_match` short-circuits [`Predicate::test`] before the
    /// field list is consulted, so a predicate carrying fields *and*
    /// the flag is still semantically always-true and still
    /// extractable. Pins that the guard keys on the flag, not on
    /// `fields.is_empty()`.
    #[test]
    fn test_always_match_with_fields_is_still_flat_extractable() {
        let predicate = Predicate {
            fields: vec![(GfxElementField::Channel(3), Comparator::equals())],
            always_match: true,
        };
        assert_eq!(flat_mutations(&repeat_while(predicate)).map(|m| m.len()), Some(1));
    }

    /// A genuinely filtering predicate needs walker-based
    /// evaluation the flat path can't provide.
    #[test]
    fn test_field_filtered_repeat_while_is_not_flat_extractable() {
        let predicate = Predicate {
            fields: vec![(GfxElementField::Channel(3), Comparator::equals())],
            always_match: false,
        };
        assert!(flat_mutations(&repeat_while(predicate)).is_none());
    }

    /// The `RepeatWhileAlwaysTrue` sugar variant takes its own arm
    /// and stays extractable.
    #[test]
    fn test_repeat_while_always_true_sugar_is_flat_extractable() {
        let node = MutatorNode::Instruction {
            channel: 0,
            instruction: InstructionSpec::RepeatWhileAlwaysTrue,
            mutation: MutationSrc::None,
            children: vec![macro_child()],
        };
        assert_eq!(flat_mutations(&node).map(|m| m.len()), Some(1));
    }

    /// The scope helpers stay extractable — they are the shapes the
    /// flat-apply path is built around.
    #[test]
    fn test_scope_helpers_stay_flat_extractable() {
        assert_eq!(
            flat_mutations(&scope::self_only(vec![nudge()])).map(|m| m.len()),
            Some(1)
        );
        assert_eq!(
            flat_mutations(&scope::descendants(vec![nudge()])).map(|m| m.len()),
            Some(1)
        );
        assert_eq!(
            flat_mutations(&scope::self_and_descendants(vec![nudge()])).map(|m| m.len()),
            Some(1)
        );
    }

    /// A match-nothing `RepeatWhile` nested *below* a `Macro` root is
    /// the same defect one level down: the root arm used to return its
    /// literal list regardless of `children`, so `L1` blanketed the
    /// whole scope set, `L2` landed nowhere, and — unlike the root
    /// case — nothing warned. Extraction is all-or-nothing precisely
    /// so this shape declines.
    #[test]
    fn test_macro_root_over_a_match_nothing_repeat_while_is_not_flat_extractable() {
        let node = MutatorNode::Macro {
            channel: 0,
            mutations: MutationListSrc::Literal(vec![nudge()]),
            children: vec![repeat_while(Predicate::new())],
        };
        assert!(
            flat_mutations(&node).is_none(),
            "a Macro root must not extract past an unevaluatable child"
        );
    }

    /// The wrapper arms have to be all-or-nothing for the same reason:
    /// `find_map` took the first extractable child and silently
    /// dropped a match-nothing sibling, which is the `Macro`-root
    /// defect reachable through a `scope::descendants`-shaped root.
    #[test]
    fn test_repeat_while_wrapper_declines_when_any_child_is_unevaluatable() {
        let node = MutatorNode::Instruction {
            channel: 0,
            instruction: InstructionSpec::RepeatWhileAlwaysTrue,
            mutation: MutationSrc::None,
            children: vec![macro_child(), repeat_while(Predicate::new())],
        };
        assert!(
            flat_mutations(&node).is_none(),
            "an extractable first child must not mask an unevaluatable sibling"
        );
    }

    /// The rule is not a child *count* limit: several children are
    /// fine as long as they agree. Asserting on the returned list's
    /// **contents**, not its length — a length assertion here cannot
    /// tell "both children said `nudge(1)`" from "the second child's
    /// list was thrown away", which is the whole question this test
    /// sits next to. The disagreeing twin is
    /// [`test_repeat_while_wrapper_declines_when_children_payloads_differ`],
    /// which returns `None` rather than picking a winner.
    #[test]
    fn test_repeat_while_wrapper_with_several_agreeing_children_still_extracts() {
        let node = MutatorNode::Instruction {
            channel: 0,
            instruction: InstructionSpec::RepeatWhileAlwaysTrue,
            mutation: MutationSrc::None,
            children: vec![macro_child(), macro_child()],
        };
        assert_eq!(flat_mutations(&node), Some(vec![nudge()]));
    }

    /// The scope helpers' payloads survive by value, not just by
    /// count — `scope::self_and_descendants` in particular carries
    /// two clones of the same list (root Macro + nested Macro), and
    /// the equality rule is what makes that a no-op instead of a
    /// double-apply or a decline.
    #[test]
    fn test_scope_helper_payloads_extract_by_value() {
        for node in [
            scope::self_only(vec![nudge()]),
            scope::descendants(vec![nudge()]),
            scope::self_and_descendants(vec![nudge()]),
        ] {
            assert_eq!(flat_mutations(&node), Some(vec![nudge()]));
        }
    }

    /// The agreement predicate has to be **reflexive**, or the two
    /// helpers above stop agreeing with each other.
    ///
    /// `scope::self_and_descendants` clones one list into two `Macro`
    /// nodes, so the rule compares a payload against itself. Derived
    /// `PartialEq` over `f32` is not reflexive, so under `==` a `NaN`
    /// made that comparison fail:
    ///
    /// ```text
    /// PROBE NaN self_and_descendants => false   # declined
    /// PROBE NaN self_only            => true    # same payload, applied
    /// PROBE inf self_and_descendants => true    # infinities are fine
    /// ```
    ///
    /// Two helpers differing only in scope must not differ in whether
    /// the mutation runs at all — and neither is allowed to depend on
    /// which float the author picked. `inf` is the control row: it is
    /// reachable from JSON (an out-of-range literal) and it always
    /// worked, so a fix that reached it would be over-broad.
    #[test]
    fn test_a_nan_payload_extracts_the_same_under_every_scope_helper() {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.0] {
            let payload = || vec![Mutation::area_command(GlyphAreaCommand::NudgeRight(value))];
            for node in [
                scope::self_only(payload()),
                scope::descendants(payload()),
                scope::self_and_descendants(payload()),
            ] {
                let got = flat_mutations(&node);
                assert!(
                    got.is_some(),
                    "a {value} payload must extract under every scope helper; got {got:?}"
                );
                let got = got.expect("checked above");
                assert_eq!(got.len(), 1, "one payload in, one payload out: {got:?}");
                assert!(
                    got[0].writes_the_same(&payload()[0]),
                    "the extracted payload must be the authored one; got {got:?}"
                );
            }
        }
    }

    /// The converse of the reflexivity fix: a genuine disagreement
    /// still declines even when one side is a `NaN`. Restoring
    /// reflexivity must not degrade into "anything unordered agrees".
    #[test]
    fn test_a_nan_payload_still_declines_against_a_different_payload() {
        let nan = Mutation::area_command(GlyphAreaCommand::NudgeRight(f32::NAN));
        let node = MutatorNode::Macro {
            channel: 0,
            mutations: MutationListSrc::Literal(vec![nan]),
            children: vec![macro_child()],
        };
        assert!(
            flat_mutations(&node).is_none(),
            "NaN and nudge(1) do not write the same thing; got {:?}",
            flat_mutations(&node)
        );
    }

    /// `0.0` and `-0.0` write the same thing, so they agree — the
    /// reflexivity fix keeps IEEE equality as one of its two
    /// disjuncts rather than replacing it with bit identity, which
    /// would have turned this into a decline.
    #[test]
    fn test_signed_zero_payloads_agree() {
        let node = MutatorNode::Macro {
            channel: 0,
            mutations: MutationListSrc::Literal(vec![Mutation::area_command(GlyphAreaCommand::NudgeRight(
                0.0,
            ))]),
            children: vec![MutatorNode::Macro {
                channel: 0,
                mutations: MutationListSrc::Literal(vec![Mutation::area_command(
                    GlyphAreaCommand::NudgeRight(-0.0),
                )]),
                children: vec![],
            }],
        };
        assert_eq!(
            flat_mutations(&node),
            Some(vec![Mutation::area_command(GlyphAreaCommand::NudgeRight(0.0))])
        );
    }

    /// A `NaN` must not be allowed to agree with an `inf`. This is
    /// why the agreement predicate is not defined over the
    /// *serialized* form: `serde_json` writes every non-finite float
    /// as `null`, and `inf` is reachable from a `.mindmap.json`, so
    /// that definition would silently drop a real payload rather than
    /// merely over-decline.
    #[test]
    fn test_a_nan_payload_does_not_agree_with_an_infinity() {
        let node = MutatorNode::Macro {
            channel: 0,
            mutations: MutationListSrc::Literal(vec![Mutation::area_command(GlyphAreaCommand::NudgeRight(
                f32::NAN,
            ))]),
            children: vec![MutatorNode::Macro {
                channel: 0,
                mutations: MutationListSrc::Literal(vec![Mutation::area_command(
                    GlyphAreaCommand::NudgeRight(f32::INFINITY),
                )]),
                children: vec![],
            }],
        };
        assert!(
            flat_mutations(&node).is_none(),
            "NaN and inf must not collapse; got {:?}",
            flat_mutations(&node)
        );
    }

    // ---- Payload equivalence, not mere extractability ----
    //
    // Four shapes that are *entirely* extractable node-by-node and
    // still cannot be collapsed into one flat list. Each returned
    // `Some([NudgeRight(1.0)])` before the equality rule landed,
    // silently discarding the payload named in the assertion message.

    fn nudge_999() -> Mutation {
        Mutation::area_command(GlyphAreaCommand::NudgeRight(999.0))
    }

    fn macro_child_999() -> MutatorNode {
        MutatorNode::Macro {
            channel: 0,
            mutations: MutationListSrc::Literal(vec![nudge_999()]),
            children: vec![],
        }
    }

    /// A nested `Macro` whose literal list *differs* from the root's.
    /// Both nodes are individually extractable, so the extractability
    /// rule admits the shape — and then one flat list has to stand in
    /// for two different payloads, which it cannot. Returning the
    /// root's list blankets every target with `nudge(1)` and lands
    /// `nudge(999)` nowhere, with no warning: the exact failure the
    /// nested rule exists to prevent, reached without a single
    /// unevaluatable node.
    #[test]
    fn test_macro_root_over_a_differing_nested_macro_is_not_flat_extractable() {
        let node = MutatorNode::Macro {
            channel: 0,
            mutations: MutationListSrc::Literal(vec![nudge()]),
            children: vec![macro_child_999()],
        };
        assert!(
            flat_mutations(&node).is_none(),
            "the nested nudge(999) would be silently dropped; got {:?}",
            flat_mutations(&node)
        );
    }

    /// The wrapper flavor of the same defect: two extractable
    /// children carrying different lists. Keeping the first and
    /// discarding the rest is what `payload.get_or_insert` used to
    /// do.
    #[test]
    fn test_repeat_while_wrapper_declines_when_children_payloads_differ() {
        let node = MutatorNode::Instruction {
            channel: 0,
            instruction: InstructionSpec::RepeatWhileAlwaysTrue,
            mutation: MutationSrc::None,
            children: vec![macro_child(), macro_child_999()],
        };
        assert!(
            flat_mutations(&node).is_none(),
            "the second child's nudge(999) would be silently dropped; got {:?}",
            flat_mutations(&node)
        );
    }

    /// An `Instruction` wrapper carrying its own per-step
    /// `MutationSrc::AreaDelta` — a per-cell template resolved
    /// against the area lookup, which the flat path has no access to.
    /// Unwrapping past it drops the delta entirely. Both scope
    /// helpers set `mutation: MutationSrc::None`, so requiring that
    /// costs nothing.
    #[test]
    fn test_repeat_while_wrapper_with_its_own_area_delta_is_not_flat_extractable() {
        use crate::mutator_builder::CellField;
        let node = MutatorNode::Instruction {
            channel: 0,
            instruction: InstructionSpec::RepeatWhileAlwaysTrue,
            mutation: MutationSrc::AreaDelta(vec![CellField::position]),
            children: vec![macro_child()],
        };
        assert!(
            flat_mutations(&node).is_none(),
            "the wrapper's own AreaDelta would be silently dropped; got {:?}",
            flat_mutations(&node)
        );
    }

    /// Same wrapper, `MutationSrc::Runtime` — documented as
    /// "entirely runtime-supplied", resolved from a `SectionContext`
    /// the flat path never builds. Provably unevaluatable here.
    #[test]
    fn test_repeat_while_wrapper_with_a_runtime_hole_is_not_flat_extractable() {
        let node = MutatorNode::Instruction {
            channel: 0,
            instruction: InstructionSpec::RepeatWhileAlwaysTrue,
            mutation: MutationSrc::Runtime,
            children: vec![macro_child()],
        };
        assert!(
            flat_mutations(&node).is_none(),
            "the wrapper's runtime hole would be silently dropped; got {:?}",
            flat_mutations(&node)
        );
    }

    /// The filtering-predicate wrapper is declined for its predicate
    /// whether or not it also carries a payload — the two guards are
    /// independent, and a shape failing both must not sneak through
    /// on some future arm reshuffle.
    #[test]
    fn test_match_nothing_repeat_while_with_a_runtime_hole_is_not_flat_extractable() {
        let node = MutatorNode::Instruction {
            channel: 0,
            instruction: InstructionSpec::RepeatWhile(Predicate::new()),
            mutation: MutationSrc::Runtime,
            children: vec![macro_child()],
        };
        assert!(flat_mutations(&node).is_none());
    }

    // ---- `Void` is payload-free structure, so it is transparent ----

    /// An **empty** `Void` child carries nothing, so nothing can be
    /// lost by keeping the root's payload. Declining here would kill
    /// a well-formed mutator to protect a payload that does not
    /// exist. Contrast [`test_single_child_is_not_flat_extractable`].
    #[test]
    fn test_empty_void_child_does_not_decline_the_mutator() {
        let node = MutatorNode::Macro {
            channel: 0,
            mutations: MutationListSrc::Literal(vec![nudge()]),
            children: vec![MutatorNode::Void {
                channel: 0,
                children: vec![],
            }],
        };
        assert_eq!(flat_mutations(&node), Some(vec![nudge()]));
    }

    /// A `Void` passes an **agreeing** payload through: the shape
    /// still extracts, so the `Void` neither swallowed the payload
    /// nor declined on its own account.
    ///
    /// This is the half of the pair that
    /// [`test_void_child_forwards_a_disagreeing_payload_and_declines`]
    /// cannot observe. That test asserts `is_none()`, which any
    /// decline satisfies — including "the `Void` declined for its own
    /// reasons", the opposite of forwarding — so it survives a mutant
    /// that reverts `Void` transparency outright. Together the two
    /// distinguish forwarding from declining, which is what the other
    /// test's name claims.
    #[test]
    fn test_void_child_forwards_an_agreeing_payload_and_extracts() {
        let node = MutatorNode::Macro {
            channel: 0,
            mutations: MutationListSrc::Literal(vec![nudge()]),
            children: vec![MutatorNode::Void {
                channel: 0,
                children: vec![macro_child()],
            }],
        };
        assert_eq!(flat_mutations(&node), Some(vec![nudge()]));
    }

    /// A `Void`'s channel is branch routing, not decoration: `build`
    /// emits `GfxMutator::new_void(channel)` and the walker aligns
    /// that node's children only against the target child on the
    /// matching channel. The flat path has nothing to route on, so a
    /// `Void` off channel 0 says something the flat list cannot say
    /// and declines — the `7f3a87c` behavior for every `Void`, kept
    /// for exactly the case where transparency would lose meaning.
    ///
    /// At `c445fab` this returned `Some([NudgeRight(1.0)])`.
    #[test]
    fn test_void_child_on_a_nonzero_channel_declines() {
        let node = MutatorNode::Macro {
            channel: 0,
            mutations: MutationListSrc::Literal(vec![nudge()]),
            children: vec![MutatorNode::Void {
                channel: 7,
                children: vec![],
            }],
        };
        assert!(
            flat_mutations(&node).is_none(),
            "a channel-7 Void is a routing gate, not grouping; got {:?}",
            flat_mutations(&node)
        );
    }

    /// A `Void` passes its children's payload through rather than
    /// swallowing it — grouping is not a payload boundary. Here the
    /// grouped list *disagrees* with the root's, and the disagreement
    /// still has to surface through the wrapper.
    ///
    /// Paired with
    /// [`test_void_child_forwards_an_agreeing_payload_and_extracts`]:
    /// `is_none()` alone cannot tell forwarding from declining, so
    /// the two are only meaningful together.
    #[test]
    fn test_void_child_forwards_a_disagreeing_payload_and_declines() {
        let node = MutatorNode::Macro {
            channel: 0,
            mutations: MutationListSrc::Literal(vec![nudge()]),
            children: vec![MutatorNode::Void {
                channel: 0,
                children: vec![macro_child_999()],
            }],
        };
        assert!(
            flat_mutations(&node).is_none(),
            "a Void must not hide a differing payload; got {:?}",
            flat_mutations(&node)
        );
    }

    /// A `Void` **root** has no payload of its own and no children to
    /// take one from, so there is no list to apply — decline, and let
    /// the apply site warn. Pins that transparency is not the same as
    /// "extracts to an empty list".
    #[test]
    fn test_empty_void_root_is_not_flat_extractable() {
        let node = MutatorNode::Void {
            channel: 0,
            children: vec![],
        };
        assert!(flat_mutations(&node).is_none());
    }

    /// `Single` carries a real channel-targeted `MutationSrc` that
    /// the flat path — which applies one list to whole elements —
    /// has nowhere to put. Unlike `Void` it is not payload-free, so
    /// it declines.
    #[test]
    fn test_single_child_is_not_flat_extractable_with_a_runtime_hole() {
        use crate::mutator_builder::ChannelSrc;
        let node = MutatorNode::Macro {
            channel: 0,
            mutations: MutationListSrc::Literal(vec![nudge()]),
            children: vec![MutatorNode::Single {
                channel: ChannelSrc::Literal(0),
                mutation: MutationSrc::Runtime,
            }],
        };
        assert!(flat_mutations(&node).is_none());
    }

    /// `Single { mutation: MutationSrc::None }` is as payload-free as
    /// an empty `Void` and still declines — deliberately, and pinned
    /// so the asymmetry with `Void` is a decision rather than an
    /// oversight. `Single` is a *leaf* whose `ChannelSrc` selects the
    /// target it writes; admitting the payload-free case would widen
    /// the accepted set into the one node kind whose entire meaning
    /// is the routing the flat path drops, to gain a node that does
    /// nothing.
    #[test]
    fn test_payload_free_single_child_still_declines() {
        use crate::mutator_builder::ChannelSrc;
        let node = MutatorNode::Macro {
            channel: 0,
            mutations: MutationListSrc::Literal(vec![nudge()]),
            children: vec![MutatorNode::Single {
                channel: ChannelSrc::Literal(0),
                mutation: MutationSrc::None,
            }],
        };
        assert!(
            flat_mutations(&node).is_none(),
            "Single declines in every form; got {:?}",
            flat_mutations(&node)
        );
    }

    /// `MapChildren` has no flat equivalent — the flat path iterates
    /// scope-collected model nodes, not the zip-by-position pairing
    /// the instruction means.
    #[test]
    fn test_map_children_is_not_flat_extractable() {
        let node = MutatorNode::Instruction {
            channel: 0,
            instruction: InstructionSpec::MapChildren,
            mutation: MutationSrc::None,
            children: vec![macro_child()],
        };
        assert!(flat_mutations(&node).is_none());
    }
}
