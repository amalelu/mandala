//! Pure-data tests for the mutator-builder walker. These exercise
//! the builder against a stub `SectionContext` so we don't need any
//! picker / widget / GPU state.

use super::*;
use baumhard::core::primitives::{ApplyOperation, ColorFontRegions};
use baumhard::gfx_structs::area::GlyphArea;
use baumhard::gfx_structs::tree::BranchChannel;
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
