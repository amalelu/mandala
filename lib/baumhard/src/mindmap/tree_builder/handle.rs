// SPDX-License-Identifier: MPL-2.0

//! Generic handle tree-builder. Edge handles, section resize
//! handles, and node resize handles all render as a small set of
//! glyphs — each with a position, glyph, color, font size, and a
//! stable per-handle channel that drives §B2 in-place mutator
//! dispatch. The three element types (`EdgeHandleElement`,
//! `SectionResizeHandleElement`, `NodeResizeHandleElement`) share
//! every layout and tree-construction step beyond "where does
//! the channel come from?", so the build / mutator / identity-
//! sequence functions are generic over a `HandleVisual` trait
//! that yields the per-instance fields. The three specialized
//! tree-builder files this module replaces had byte-identical
//! `*_layout` / `build_*_tree` / `build_*_mutator_tree` /
//! `*_identity_sequence` implementations.

use glam::Vec2;

use crate::core::primitives::ColorFontRegions;
use crate::gfx_structs::area::{DeltaGlyphArea, GlyphArea};
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::{GfxMutator, Mutation};
use crate::gfx_structs::tree::{MutatorTree, Tree};
use crate::util::color;

/// Per-instance view a handle element exposes to the generic
/// tree-builder. Implementors live alongside
/// their element struct.
pub trait HandleVisual {
    /// Canvas-space center of the handle. The layout fn shifts
    /// this by half-glyph for the `GlyphArea` top-left.
    fn position(&self) -> (f32, f32);
    /// Glyph string (usually one grapheme cluster).
    fn glyph(&self) -> &str;
    /// `#RRGGBB` hex color.
    fn color(&self) -> &str;
    /// Font size in points.
    fn font_size_pt(&self) -> f32;
    /// Stable channel — drives §B2 in-place mutator dispatch.
    /// Two elements with the same channel land on the same
    /// arena slot across rebuilds, so the channel must be
    /// derived from the handle's *kind / side*, not its
    /// position.
    fn channel(&self) -> usize;
}

/// Lay out one handle as the `(channel, GlyphArea)` pair both
/// the initial-build and in-place mutator paths emit. Single
/// source of truth — the two paths cannot drift.
fn handle_layout<E: HandleVisual>(elem: &E) -> (usize, GlyphArea) {
    let color_rgba = color::hex_to_rgba_safe(elem.color(), [0.0, 0.9, 1.0, 1.0]);
    // Handle glyphs are centered on the position with the same
    // half-glyph offset the legacy renderer used.
    let half_w = elem.font_size_pt() * 0.3;
    let half_h = elem.font_size_pt() * 0.5;
    let (px, py) = elem.position();
    let pos = Vec2::new(px - half_w, py - half_h);
    let bounds = Vec2::new(elem.font_size_pt(), elem.font_size_pt());

    let glyph = elem.glyph();
    let mut area = GlyphArea::new_with_str(glyph, elem.font_size_pt(), elem.font_size_pt(), pos, bounds);
    let cluster_count = crate::util::grapheme_chad::count_grapheme_clusters(glyph);
    area.regions = ColorFontRegions::single_span(cluster_count, Some(color_rgba), None);

    (elem.channel(), area)
}

/// Identity sequence for a set of handle elements — the channel
/// of each handle, in tree-insertion order. Two handle sets
/// produce the same identity iff their structural shape is
/// identical (same channels in the same order); the in-place
/// mutator path is sound only under that condition.
pub fn handle_identity_sequence<E: HandleVisual>(elements: &[E]) -> Vec<usize> {
    elements.iter().map(|e| e.channel()).collect()
}

/// Build a baumhard tree of every handle glyph from a pre-
/// computed slice. Channels come from `HandleVisual::channel`
/// so the in-place mutator path can target each leaf by the
/// same kind-derived channel across drag frames.
pub fn build_handle_tree<E: HandleVisual>(elements: &[E]) -> Tree<GfxElement, GfxMutator> {
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    let mut unique_id: usize = 1;

    for elem in elements {
        let (channel, area) = handle_layout(elem);
        let element_node = GfxElement::new_area_non_indexed_with_id(area, channel, unique_id);
        unique_id += 1;
        let leaf = tree.arena.new_node(element_node);
        tree.root.append(leaf, &mut tree.arena);
    }

    tree
}

/// Build a [`MutatorTree`] that updates an already-registered
/// handle tree to the current `elements` state without
/// rebuilding the arena. Pairs with [`build_handle_tree`] —
/// applying this mutator to a tree built from an element slice
/// with the same identity sequence (per
/// [`handle_identity_sequence`]) updates each handle's variable
/// fields in place via `DeltaGlyphArea::full_assign_from`.
pub fn build_handle_mutator_tree<E: HandleVisual>(elements: &[E]) -> MutatorTree<GfxMutator> {
    let mut mt: MutatorTree<GfxMutator> = MutatorTree::new_with(GfxMutator::new_void(0));
    for elem in elements {
        let (channel, area) = handle_layout(elem);
        let delta = DeltaGlyphArea::full_assign_from(&area);
        let leaf = mt
            .arena
            .new_node(GfxMutator::new(Mutation::AreaDelta(Box::new(delta)), channel));
        mt.root.append(leaf, &mut mt.arena);
    }
    mt
}
