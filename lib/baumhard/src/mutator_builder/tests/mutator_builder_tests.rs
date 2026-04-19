//! Pure-data tests for the mutator-builder walker. These exercise
//! the builder against a stub `SectionContext` so we don't need any
//! picker / widget / GPU state.

use crate::core::primitives::{ApplyOperation, ColorFontRegions};
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::tree::BranchChannel;
use crate::mutator_builder::{
    build, iter_section_channels, CellField, ChannelSrc, CountSrc, InstructionSpec,
    MutationListSrc, MutationSrc, MutatorNode, SectionContext,
};
use glam::Vec2;
use std::collections::HashMap;

/// Stub context: yields one pre-built `GlyphArea` per index, and
/// honours runtime-count queries out of a `HashMap`. Per-section
/// routing is deliberately minimal — the tests below don't need
/// distinct areas per section, only per index.
struct StubCtx {
    areas: Vec<GlyphArea>,
    runtime_counts: HashMap<String, usize>,
}

impl StubCtx {
    fn with_areas(n: usize) -> Self {
        let mut areas = Vec::with_capacity(n);
        for i in 0..n {
            let text = format!("cell_{}", i);
            let mut a = GlyphArea::new_with_str(
                &text,
                10.0,
                12.0,
                Vec2::new(i as f32, 0.0),
                Vec2::new(20.0, 30.0),
            );
            a.regions = ColorFontRegions::single_span(text.chars().count(), None, None);
            areas.push(a);
        }
        Self {
            areas,
            runtime_counts: HashMap::new(),
        }
    }
}

impl SectionContext for StubCtx {
    fn area(&self, _section: &str, index: usize) -> &GlyphArea {
        &self.areas[index]
    }
    fn count(&self, name: &str) -> usize {
        *self.runtime_counts.get(name).unwrap_or(&0)
    }
}

fn single_with_text() -> Box<MutatorNode> {
    Box::new(MutatorNode::Single {
        channel: ChannelSrc::SectionIndex,
        mutation: MutationSrc::AreaDelta(vec![
            CellField::Text,
            CellField::Operation(ApplyOperation::Assign),
        ]),
    })
}

/// Repeat with literal count expands to the right number of children
/// at consecutive channels in declaration order.
#[test]
fn repeat_literal_expands_to_consecutive_channels() {
    let node = MutatorNode::Void {
        channel: 0,
        children: vec![MutatorNode::Repeat {
            section: "cells".into(),
            channel_base: 100,
            count: CountSrc::Literal(5),
            skip_indices: vec![],
            template: single_with_text(),
        }],
    };
    let ctx = StubCtx::with_areas(5);
    let mt = build(&node, &ctx);
    let channels: Vec<usize> = mt
        .root
        .children(&mt.arena)
        .map(|id| mt.arena.get(id).unwrap().get().channel())
        .collect();
    assert_eq!(channels, vec![100, 101, 102, 103, 104]);
}

/// `skip_indices` skips the named iteration indices; channels are
/// strided (skipped channels are absent, not renumbered).
#[test]
fn repeat_skip_indices_stride_channels() {
    let node = MutatorNode::Void {
        channel: 0,
        children: vec![MutatorNode::Repeat {
            section: "cells".into(),
            channel_base: 300,
            count: CountSrc::Literal(5),
            skip_indices: vec![2],
            template: single_with_text(),
        }],
    };
    let ctx = StubCtx::with_areas(5);
    let mt = build(&node, &ctx);
    let channels: Vec<usize> = mt
        .root
        .children(&mt.arena)
        .map(|id| mt.arena.get(id).unwrap().get().channel())
        .collect();
    assert_eq!(channels, vec![300, 301, 303, 304]);
}

/// Multiple sections concatenate in declaration order; bands stay
/// disjoint.
#[test]
fn multiple_sections_concatenate_in_declaration_order() {
    let node = MutatorNode::Void {
        channel: 0,
        children: vec![
            MutatorNode::Repeat {
                section: "a".into(),
                channel_base: 1,
                count: CountSrc::Literal(1),
                skip_indices: vec![],
                template: single_with_text(),
            },
            MutatorNode::Repeat {
                section: "b".into(),
                channel_base: 100,
                count: CountSrc::Literal(3),
                skip_indices: vec![],
                template: single_with_text(),
            },
            MutatorNode::Repeat {
                section: "c".into(),
                channel_base: 200,
                count: CountSrc::Literal(1),
                skip_indices: vec![],
                template: single_with_text(),
            },
        ],
    };
    let ctx = StubCtx::with_areas(3);
    let mt = build(&node, &ctx);
    let channels: Vec<usize> = mt
        .root
        .children(&mt.arena)
        .map(|id| mt.arena.get(id).unwrap().get().channel())
        .collect();
    assert_eq!(channels, vec![1, 100, 101, 102, 200]);
}

/// `iter_section_channels` emits every `(section, index, channel)`
/// tuple in tree-insertion order; used by the initial-build path so
/// the channel set stays aligned with the mutator path.
#[test]
fn iter_section_channels_walks_in_order() {
    let node = MutatorNode::Void {
        channel: 0,
        children: vec![
            MutatorNode::Repeat {
                section: "a".into(),
                channel_base: 1,
                count: CountSrc::Literal(1),
                skip_indices: vec![],
                template: single_with_text(),
            },
            MutatorNode::Repeat {
                section: "b".into(),
                channel_base: 300,
                count: CountSrc::Literal(3),
                skip_indices: vec![1],
                template: single_with_text(),
            },
        ],
    };
    let ctx = StubCtx::with_areas(0);
    let mut out = Vec::new();
    iter_section_channels(&node, &ctx, &mut out);
    assert_eq!(
        out,
        vec![
            ("a".to_string(), 0, 1),
            ("b".to_string(), 0, 300),
            ("b".to_string(), 2, 302),
        ]
    );
}

/// Runtime counts: builder asks the context how many cells the
/// section has at apply time. Proves the design absorbs the
/// console overlay's `scrollback_rows`-style use case.
#[test]
fn repeat_runtime_count_consults_context() {
    let node = MutatorNode::Void {
        channel: 0,
        children: vec![MutatorNode::Repeat {
            section: "rows".into(),
            channel_base: 1000,
            count: CountSrc::Runtime("row_count".into()),
            skip_indices: vec![],
            template: single_with_text(),
        }],
    };
    let mut ctx = StubCtx::with_areas(7);
    ctx.runtime_counts.insert("row_count".into(), 3);
    let mt = build(&node, &ctx);
    let channels: Vec<usize> = mt
        .root
        .children(&mt.arena)
        .map(|id| mt.arena.get(id).unwrap().get().channel())
        .collect();
    assert_eq!(channels, vec![1000, 1001, 1002]);
}

// =============================================================
// Non-Repeat tree shapes — exercise the variants the picker spec
// doesn't use but that the AST has to carry for the named
// extensibility trajectory (console overlay, scope topologies,
// future script-API mutators).
// =============================================================

use crate::gfx_structs::mutator::{GfxMutator, Instruction, Mutation, MutatorType};
use crate::gfx_structs::predicate::Predicate;

/// Tree rooted at a `Void` with no children produces an empty root.
#[test]
fn void_root_with_no_children() {
    let node = MutatorNode::Void {
        channel: 42,
        children: vec![],
    };
    let ctx = StubCtx::with_areas(0);
    let mt = build(&node, &ctx);
    let root = mt.arena.get(mt.root).unwrap().get();
    assert!(matches!(root, GfxMutator::Void { channel: 42 }));
    assert_eq!(mt.root.children(&mt.arena).count(), 0);
}

/// `ChannelSrc::Literal` resolves without needing a `Repeat`
/// iteration context — proves a `Single` can sit at the tree root.
#[test]
fn single_as_tree_root_with_literal_channel() {
    let node = MutatorNode::Single {
        channel: ChannelSrc::Literal(7),
        mutation: MutationSrc::None,
    };
    // No-op context — Single with MutationSrc::None needs nothing.
    let ctx = StubCtx::with_areas(0);
    let mt = build(&node, &ctx);
    let root = mt.arena.get(mt.root).unwrap().get();
    match root {
        GfxMutator::Single { channel, mutation } => {
            assert_eq!(*channel, 7);
            assert!(matches!(mutation, Mutation::None));
        }
        other => panic!("expected Single root, got {other:?}"),
    }
}

/// `MutationSrc::Runtime` consults the context via
/// [`SectionContext::mutation`]; the `label` argument is the
/// enclosing section name, or `""` when called from a non-Repeat
/// context (e.g. a Single-as-root).
#[test]
fn mutation_src_runtime_consults_context() {
    struct RuntimeCtx {
        asked_with: std::cell::RefCell<Vec<String>>,
    }
    impl SectionContext for RuntimeCtx {
        fn mutation(&self, label: &str) -> Mutation {
            self.asked_with.borrow_mut().push(label.to_string());
            Mutation::None
        }
    }
    let node = MutatorNode::Single {
        channel: ChannelSrc::Literal(5),
        mutation: MutationSrc::Runtime,
    };
    let ctx = RuntimeCtx {
        asked_with: std::cell::RefCell::new(Vec::new()),
    };
    let _ = build(&node, &ctx);
    assert_eq!(ctx.asked_with.borrow().as_slice(), &["".to_string()]);
}

/// `Macro` with `MutationListSrc::Runtime` pulls its `Vec<Mutation>`
/// from the context — proves the AST carries the scope-topology
/// shape (which uses Macro with runtime payloads).
#[test]
fn macro_pulls_mutation_list_from_runtime_context() {
    struct MacroCtx;
    impl SectionContext for MacroCtx {
        fn mutation_list(&self, label: &str) -> Vec<Mutation> {
            assert_eq!(label, "scope_mutations");
            vec![Mutation::None, Mutation::None]
        }
    }
    let node = MutatorNode::Macro {
        channel: 0,
        mutations: MutationListSrc::Runtime("scope_mutations".into()),
        children: vec![],
    };
    let ctx = MacroCtx;
    let mt = build(&node, &ctx);
    let root = mt.arena.get(mt.root).unwrap().get();
    match root {
        GfxMutator::Macro { channel, mutations } => {
            assert_eq!(*channel, 0);
            assert_eq!(mutations.len(), 2);
        }
        other => panic!("expected Macro root, got {other:?}"),
    }
}

/// `Instruction` with children mirrors the `SelfAndDescendants`
/// scope-topology shape: the root Instruction wraps one Macro child
/// whose mutations come from runtime. The walker descends into
/// Instruction children the same way it descends into Void children.
#[test]
fn instruction_with_children_mirrors_scope_topology() {
    struct ScopeCtx;
    impl SectionContext for ScopeCtx {
        fn mutation_list(&self, _label: &str) -> Vec<Mutation> {
            vec![Mutation::None]
        }
    }
    let node = MutatorNode::Instruction {
        channel: 0,
        instruction: InstructionSpec::RepeatWhileAlwaysTrue,
        mutation: MutationSrc::None,
        children: vec![MutatorNode::Macro {
            channel: 0,
            mutations: MutationListSrc::Runtime("leaf".into()),
            children: vec![],
        }],
    };
    let mt = build(&node, &ScopeCtx);
    let root = mt.arena.get(mt.root).unwrap().get();
    assert!(matches!(root.get_type(), MutatorType::Instruction));
    let child_ids: Vec<_> = mt.root.children(&mt.arena).collect();
    assert_eq!(child_ids.len(), 1);
    let child = mt.arena.get(child_ids[0]).unwrap().get();
    assert!(matches!(child.get_type(), MutatorType::Macro));
}

/// `InstructionSpec::RepeatWhileAlwaysTrue` materializes to a real
/// `Instruction::RepeatWhile(always_true())` without needing the
/// consumer to spell the predicate out.
#[test]
fn instruction_spec_repeat_while_always_true_materializes_predicate() {
    let node = MutatorNode::Instruction {
        channel: 5,
        instruction: InstructionSpec::RepeatWhileAlwaysTrue,
        mutation: MutationSrc::None,
        children: vec![],
    };
    let ctx = StubCtx::with_areas(0);
    let mt = build(&node, &ctx);
    match mt.arena.get(mt.root).unwrap().get() {
        GfxMutator::Instruction {
            instruction: Instruction::RepeatWhile(_),
            channel,
            ..
        } => {
            assert_eq!(*channel, 5);
        }
        other => panic!("expected Instruction::RepeatWhile, got {other:?}"),
    }
}

/// `InstructionSpec::RotateWhile` round-trips its angle + predicate
/// payload through `into_instruction`.
#[test]
fn instruction_spec_rotate_while_carries_angle_and_predicate() {
    let node = MutatorNode::Instruction {
        channel: 0,
        instruction: InstructionSpec::RotateWhile(42.0, Predicate::always_true()),
        mutation: MutationSrc::None,
        children: vec![],
    };
    let ctx = StubCtx::with_areas(0);
    let mt = build(&node, &ctx);
    match mt.arena.get(mt.root).unwrap().get() {
        GfxMutator::Instruction {
            instruction: Instruction::RotateWhile(angle, _),
            ..
        } => {
            assert!((*angle - 42.0).abs() < 1e-6);
        }
        other => panic!("expected Instruction::RotateWhile, got {other:?}"),
    }
}

/// Nested `Repeat`-inside-`Void` template: the builder threads the
/// per-iteration channel through intermediate wrapper nodes so an
/// inner `ChannelSrc::SectionIndex` still resolves to
/// `channel_base + i`. Exercises the iter-threading that fires
/// when a consumer wraps its cell template in a grouping Void (e.g.
/// for future per-cell decorations).
#[test]
fn repeat_template_wrapping_void_threads_iter_channel() {
    let node = MutatorNode::Void {
        channel: 0,
        children: vec![MutatorNode::Repeat {
            section: "wrapped".into(),
            channel_base: 50,
            count: CountSrc::Literal(3),
            skip_indices: vec![],
            template: Box::new(MutatorNode::Void {
                channel: 999, // Void's own channel — literal, not iterated.
                children: vec![MutatorNode::Single {
                    channel: ChannelSrc::SectionIndex,
                    mutation: MutationSrc::None,
                }],
            }),
        }],
    };
    let ctx = StubCtx::with_areas(0);
    let mt = build(&node, &ctx);
    // Root Void → 3 wrapper Voids → each wrapper Void has 1 Single child.
    let wrapper_ids: Vec<_> = mt.root.children(&mt.arena).collect();
    assert_eq!(wrapper_ids.len(), 3);
    for (expected_inner_ch, wrapper_id) in (50..).zip(wrapper_ids.iter()) {
        let wrapper = mt.arena.get(*wrapper_id).unwrap().get();
        assert!(matches!(wrapper, GfxMutator::Void { channel: 999 }));
        let inner_ids: Vec<_> = wrapper_id.children(&mt.arena).collect();
        assert_eq!(inner_ids.len(), 1);
        let inner = mt.arena.get(inner_ids[0]).unwrap().get();
        match inner {
            GfxMutator::Single { channel, .. } => {
                assert_eq!(*channel, expected_inner_ch);
            }
            other => panic!("expected Single inner, got {other:?}"),
        }
    }
}

/// A `Repeat` at the tree root is a misuse — Repeats only describe
/// children expansion. The builder catches this rather than silently
/// producing a malformed tree.
#[test]
#[should_panic(expected = "Repeat can only appear as a child")]
fn repeat_at_tree_root_panics() {
    let node = MutatorNode::Repeat {
        section: "x".into(),
        channel_base: 0,
        count: CountSrc::Literal(1),
        skip_indices: vec![],
        template: single_with_text(),
    };
    let ctx = StubCtx::with_areas(1);
    let _ = build(&node, &ctx);
}

/// `ChannelSrc::SectionIndex` only makes sense inside a `Repeat`
/// template. Using it at a tree root (or in any non-iterated
/// position) is a contract violation surfaced immediately.
#[test]
#[should_panic(expected = "ChannelSrc::SectionIndex used outside a Repeat template")]
fn channel_src_section_index_outside_repeat_panics() {
    let node = MutatorNode::Single {
        channel: ChannelSrc::SectionIndex,
        mutation: MutationSrc::None,
    };
    let ctx = StubCtx::with_areas(0);
    let _ = build(&node, &ctx);
}

/// `MutationSrc::AreaDelta` needs a `Repeat` iteration context to
/// resolve its per-cell area — used outside one is a contract bug.
#[test]
#[should_panic(expected = "MutationSrc::AreaDelta requires a Repeat-templated context")]
fn area_delta_outside_repeat_panics() {
    let node = MutatorNode::Single {
        channel: ChannelSrc::Literal(0),
        mutation: MutationSrc::AreaDelta(vec![CellField::Text]),
    };
    let ctx = StubCtx::with_areas(1);
    let _ = build(&node, &ctx);
}

// =============================================================
// JSON roundtrip — the AST's load-bearing purpose is deserializing
// from JSON (widgets, custom mutations, future user scripts). Drift
// in serde renames / variant shapes must surface here rather than
// silently at picker-open time.
// =============================================================

/// A compact MutatorNode literal round-trips through serde_json
/// with the same shape we'd hand-build in Rust. Catches accidental
/// renames of variant tags or field names that would break the
/// picker's `mutator_spec` block without warning.
#[test]
fn json_roundtrip_minimal_repeat_tree() {
    let src = r#"{
        "Void": {
            "channel": 0,
            "children": [
                {
                    "Repeat": {
                        "section": "cells",
                        "channel_base": 100,
                        "count": { "Literal": 3 },
                        "template": {
                            "Single": {
                                "channel": "SectionIndex",
                                "mutation": { "AreaDelta": ["Text", { "Operation": "Assign" }] }
                            }
                        }
                    }
                }
            ]
        }
    }"#;
    let node: MutatorNode = serde_json::from_str(src).expect("parse MutatorNode");
    // Shape assertions — same structure as a hand-built AST.
    let MutatorNode::Void { channel, children } = &node else {
        panic!("root must be Void");
    };
    assert_eq!(*channel, 0);
    assert_eq!(children.len(), 1);
    let MutatorNode::Repeat {
        section,
        channel_base,
        count,
        skip_indices,
        template,
    } = &children[0]
    else {
        panic!("child must be Repeat");
    };
    assert_eq!(section, "cells");
    assert_eq!(*channel_base, 100);
    assert!(matches!(count, CountSrc::Literal(3)));
    assert!(skip_indices.is_empty());
    let MutatorNode::Single { channel, mutation } = template.as_ref() else {
        panic!("template must be Single");
    };
    assert!(matches!(channel, ChannelSrc::SectionIndex));
    let MutationSrc::AreaDelta(fields) = mutation else {
        panic!("mutation must be AreaDelta");
    };
    assert_eq!(fields.len(), 2);
    assert!(matches!(fields[0], CellField::Text));
    assert!(matches!(fields[1], CellField::Operation(ApplyOperation::Assign)));
}

/// `skip_indices` default to an empty Vec when omitted in JSON —
/// the overwhelming case, so the picker JSON stays terse for
/// sections that don't skip anything.
#[test]
fn json_skip_indices_defaults_to_empty() {
    let src = r#"{
        "Repeat": {
            "section": "cells",
            "channel_base": 0,
            "count": { "Literal": 1 },
            "template": { "Single": { "channel": "SectionIndex", "mutation": "None" } }
        }
    }"#;
    let node: MutatorNode = serde_json::from_str(src).expect("parse MutatorNode");
    let MutatorNode::Repeat { skip_indices, .. } = node else {
        panic!("expected Repeat");
    };
    assert!(skip_indices.is_empty());
}

/// `Instruction.mutation` defaults to `MutationSrc::None` when
/// omitted — scope-topology shapes (RepeatWhile over descendants)
/// don't need an explicit mutation payload on the instruction node
/// itself, and we want the JSON to reflect that.
#[test]
fn json_instruction_mutation_defaults_to_none() {
    let src = r#"{
        "Instruction": {
            "channel": 0,
            "instruction": "RepeatWhileAlwaysTrue"
        }
    }"#;
    let node: MutatorNode = serde_json::from_str(src).expect("parse MutatorNode");
    let MutatorNode::Instruction { mutation, .. } = node else {
        panic!("expected Instruction");
    };
    assert!(matches!(mutation, MutationSrc::None));
}

/// `InstructionSpec::MapChildren` round-trips through JSON and
/// materializes to `Instruction::MapChildren` via
/// `into_instruction`. Guards the seam authors rely on when
/// declaring MapChildren-shaped mutators in a custom_mutations
/// bundle.
#[test]
fn json_instruction_spec_map_children_materializes_correctly() {
    let src = r#"{
        "Instruction": {
            "channel": 3,
            "instruction": "MapChildren"
        }
    }"#;
    let node: MutatorNode = serde_json::from_str(src).expect("parse MutatorNode");
    let MutatorNode::Instruction { channel, instruction, .. } = node else {
        panic!("expected Instruction");
    };
    assert_eq!(channel, 3);
    // Materialize the InstructionSpec into a concrete Instruction and
    // verify the variant pairs with the walker primitive.
    let inst = instruction.clone().into_instruction();
    assert!(matches!(inst, Instruction::MapChildren));
}
