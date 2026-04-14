//! Recursive walker that turns a [`MutatorNode`] + [`SectionContext`]
//! into a concrete `MutatorTree<GfxMutator>`.

use baumhard::gfx_structs::area::{DeltaGlyphArea, GlyphAreaField};
use baumhard::gfx_structs::mutator::{GfxMutator, Mutation};
use baumhard::gfx_structs::tree::MutatorTree;
use indextree::NodeId;

use super::ast::{ChannelSrc, CountSrc, MutationListSrc, MutationSrc, MutatorNode};
use super::context::SectionContext;

/// Build a `MutatorTree<GfxMutator>` from `node` + `ctx`. The tree's
/// root is `node` itself — which must be a `Void` / `Single` /
/// `Macro` / `Instruction`. A `Repeat` only makes sense as a child,
/// so using one as the root panics.
pub fn build<C: SectionContext + ?Sized>(
    node: &MutatorNode,
    ctx: &C,
) -> MutatorTree<GfxMutator> {
    if let MutatorNode::Repeat { .. } = node {
        panic!("Repeat can only appear as a child, not as a tree root");
    }
    let root_mutator = materialize_node(node, ctx, None);
    let mut mt = MutatorTree::new_with(root_mutator);
    let root = mt.root;
    for child in node_children(node) {
        append(child, root, &mut mt, ctx, None);
    }
    mt
}

/// Iterate every `(section, index, channel)` tuple the spec will emit
/// at apply time, in tree-insertion (channel-ascending) order. Used
/// by the initial-build path (which builds a `Tree<GfxElement, _>`)
/// so its channel set matches what the mutator path will target.
pub fn iter_section_channels<C: SectionContext + ?Sized>(
    node: &MutatorNode,
    ctx: &C,
    out: &mut Vec<(String, usize, usize)>,
) {
    match node {
        MutatorNode::Repeat {
            section,
            channel_base,
            count,
            skip_indices,
            ..
        } => {
            let n = resolve_count(count, ctx);
            for i in 0..n {
                if skip_indices.contains(&i) {
                    continue;
                }
                out.push((section.clone(), i, channel_base + i));
            }
        }
        MutatorNode::Void { children, .. } | MutatorNode::Instruction { children, .. } => {
            for child in children {
                iter_section_channels(child, ctx, out);
            }
        }
        MutatorNode::Single { .. } | MutatorNode::Macro { .. } => {}
    }
}

/// Per-iteration state the builder threads into a Repeat template.
#[derive(Clone, Copy)]
struct IterCtx<'a> {
    section: &'a str,
    index: usize,
    channel: usize,
}

fn node_children(node: &MutatorNode) -> &[MutatorNode] {
    match node {
        MutatorNode::Void { children, .. } => children,
        MutatorNode::Instruction { children, .. } => children,
        MutatorNode::Single { .. } | MutatorNode::Macro { .. } | MutatorNode::Repeat { .. } => &[],
    }
}

fn resolve_count<C: SectionContext + ?Sized>(src: &CountSrc, ctx: &C) -> usize {
    match src {
        CountSrc::Literal(n) => *n,
        CountSrc::Runtime(name) => ctx.count(name),
    }
}

fn resolve_channel(src: &ChannelSrc, iter: Option<IterCtx<'_>>) -> usize {
    match src {
        ChannelSrc::Literal(c) => *c,
        ChannelSrc::SectionIndex => {
            iter.expect("ChannelSrc::SectionIndex used outside a Repeat template")
                .channel
        }
    }
}

fn materialize_node<C: SectionContext + ?Sized>(
    node: &MutatorNode,
    ctx: &C,
    iter: Option<IterCtx<'_>>,
) -> GfxMutator {
    match node {
        MutatorNode::Void { channel, .. } => GfxMutator::new_void(*channel),
        MutatorNode::Single { channel, mutation } => {
            let ch = resolve_channel(channel, iter);
            let m = materialize_mutation(mutation, ctx, iter);
            GfxMutator::new(m, ch)
        }
        MutatorNode::Macro { channel, mutations } => match mutations {
            MutationListSrc::Runtime(label) => {
                GfxMutator::new_macro(ctx.mutation_list(label), *channel)
            }
        },
        MutatorNode::Instruction {
            channel,
            instruction,
            mutation,
            ..
        } => GfxMutator::Instruction {
            channel: *channel,
            instruction: instruction.clone().into_instruction(),
            mutation: materialize_mutation(mutation, ctx, iter),
        },
        MutatorNode::Repeat { .. } => unreachable!("handled in append"),
    }
}

fn append<C: SectionContext + ?Sized>(
    node: &MutatorNode,
    parent: NodeId,
    mt: &mut MutatorTree<GfxMutator>,
    ctx: &C,
    iter: Option<IterCtx<'_>>,
) {
    match node {
        MutatorNode::Repeat {
            section,
            channel_base,
            count,
            skip_indices,
            template,
        } => {
            let n = resolve_count(count, ctx);
            for i in 0..n {
                if skip_indices.contains(&i) {
                    continue;
                }
                let ch = channel_base + i;
                let sub_iter = Some(IterCtx {
                    section: section.as_str(),
                    index: i,
                    channel: ch,
                });
                let mutator = materialize_node(template, ctx, sub_iter);
                let id = mt.arena.new_node(mutator);
                parent.append(id, &mut mt.arena);
                for grandchild in node_children(template) {
                    append(grandchild, id, mt, ctx, sub_iter);
                }
            }
        }
        _ => {
            let mutator = materialize_node(node, ctx, iter);
            let id = mt.arena.new_node(mutator);
            parent.append(id, &mut mt.arena);
            for child in node_children(node) {
                append(child, id, mt, ctx, iter);
            }
        }
    }
}

fn materialize_mutation<C: SectionContext + ?Sized>(
    src: &MutationSrc,
    ctx: &C,
    iter: Option<IterCtx<'_>>,
) -> Mutation {
    match src {
        MutationSrc::None => Mutation::None,
        MutationSrc::Runtime => {
            let label = iter.map(|i| i.section).unwrap_or("");
            ctx.mutation(label)
        }
        MutationSrc::AreaDelta(template) => {
            let it = iter.expect(
                "MutationSrc::AreaDelta requires a Repeat-templated context \
                 (needs a section name + iteration index)",
            );
            let fields: Vec<GlyphAreaField> = template
                .iter()
                .map(|f| ctx.field(it.section, it.index, f))
                .collect();
            Mutation::AreaDelta(Box::new(DeltaGlyphArea::new(fields)))
        }
    }
}
