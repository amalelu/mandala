// SPDX-License-Identifier: MPL-2.0

//! Border-tree builder: emits one per-node Void parent and four
//! `GlyphArea` runs (top, bottom, left, right) per framed node.
//! Sorted lexicographically by node id so the per-node Void
//! channel is stable across rebuilds — the precondition for the
//! in-place mutator path `build_border_mutator_tree_from_nodes`.

use std::collections::{HashMap, HashSet};

use glam::Vec2;
use indextree::NodeId;

use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::GfxMutator;
use crate::gfx_structs::shape::NodeShape;
use crate::gfx_structs::tree::Tree;
use crate::gfx_structs::zoom_visibility::ZoomVisibility;
use crate::mindmap::border::{build_border_regions, resolve_border_style};
use crate::mindmap::model::{GlyphBorderConfig, MindMap};
use crate::util::color;

use super::node_clip::INACTIVE_NODE_ALPHA_MULTIPLIER;
use super::overrides::{BorderConfigEditsView, BorderPreview, BorderPreviewTargetRef};

/// Per-node data for the border tree — single source of truth
/// consumed by both [`build_border_tree`] (initial build) and
/// [`build_border_mutator_tree`] (in-place §B2 update). The
/// `parent_channel` is the 1-based index of this node in the
/// sorted visible-framed-nodes sequence, so the channel is
/// *stable across rebuilds* as long as the identity sequence
/// (see [`border_identity_sequence`]) is unchanged.
#[derive(Clone, Debug)]
pub struct BorderNodeData {
    pub node_id: String,
    pub parent_channel: usize,
    pub border_style: crate::mindmap::border::BorderStyle,
    pub color_rgba: [f32; 4],
    pub pos_x: f32,
    pub pos_y: f32,
    pub size_x: f32,
    pub size_y: f32,
    /// Zoom window inherited from the owning node. Stamped onto
    /// each of the four border runs at both initial-build and
    /// mutator-update time so the frame disappears atomically
    /// with its node at any zoom level.
    pub zoom_visibility: ZoomVisibility,
    /// Resolved per-cycle-position colors for palette cycling,
    /// or empty when `border_style.color_palette` is unset / the
    /// named palette doesn't exist. Pre-resolved upstream so the
    /// mutator path doesn't need to thread `&MindMap` through
    /// `build_border_mutator_tree_from_nodes`. One entry per
    /// `ColorGroup` in the palette, reading the configured
    /// `palette_field` channel.
    pub palette_cycle: Vec<[f32; 4]>,
}

/// View-side chrome overrides for one border-pass call. Both
/// fields are frame-local: they never touch the committed model,
/// and both default to "no override".
///
/// Bundled rather than passed as two more positional arguments
/// because they arrive together — the application layer derives
/// both from the same per-frame `(document, InteractionMode)`
/// snapshot — and because a third mode-derived border override
/// should extend this struct rather than the signature.
#[derive(Debug, Default, Clone, Copy)]
pub struct BorderChromeOverrides<'a> {
    /// Staged `border preview …` edits for the `Nodes` /
    /// `CanvasDefault` targets. Folded into a clone of the
    /// matching slot before resolution, so the preview renders
    /// without a model write. The `Sections` /
    /// `CanvasSectionFrame*` targets belong to
    /// [`super::build_section_frames`] and are ignored here.
    pub preview: Option<BorderPreview<'a>>,
    /// Active `InteractionMode::NodeEdit` target. When `Some`,
    /// every *other* node's frame renders at
    /// [`INACTIVE_NODE_ALPHA_MULTIPLIER`] alpha — the "you are
    /// inside this node" affordance. `None` (Default mode) is the
    /// no-op fast path.
    pub node_edit_for: Option<&'a str>,
}

/// Compute the border layout for the current `(map, offsets)`
/// state. Sorted lexicographically by `MindNode.id` so per-node
/// Void parents always land at the same channel — the
/// prerequisite for the in-place mutator path.
///
/// Skips hidden-by-fold nodes, non-rectangular nodes, and
/// `show_frame = false` nodes — unless a `preview` with
/// `force_show_frame` targets them, so `border preview
/// preset=heavy` on a frameless node is visible instead of
/// silently doing nothing.
///
/// `hidden_set` is the output of [`MindMap::fold_hidden_set`]. The
/// caller should build it once per frame and share it across the
/// border / connection / portal passes instead of recomputing it
/// here.
///
/// # Costs
///
/// O(N log N) in the visible node count (the id sort dominates).
/// One [`resolve_border_style`] + one `resolve_palette_cycle` per
/// emitted node, and that is **the only resolution of this cascade
/// anywhere in the frame** — the clip-AABB pass
/// ([`super::node_clip_aabbs`]) takes the allocation-free
/// [`crate::mindmap::border::resolve_border_font_size_pt`] shortcut
/// instead of resolving a `BorderStyle` it would throw away.
/// `overrides` adds one `GlyphBorderConfig` clone per previewed
/// slot and, under `node_edit_for`, one hex parse/format per
/// distinct inactive frame color (memoized by input hex for the
/// duration of the call).
pub fn border_node_data(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    overrides: BorderChromeOverrides<'_>,
    hidden_set: &HashSet<&str>,
) -> Vec<BorderNodeData> {
    let vars = &map.canvas.theme_variables;
    let mut sorted_ids: Vec<&String> = map.nodes.keys().collect();
    sorted_ids.sort();

    // Hoist the preview-target match out of the per-node loop: the
    // steady-state rebuild runs with `preview = None` and we want
    // one `is_none()` check per node, not a re-match of the target
    // enum. Mirrors the same hoist in `node_clip::node_clip_aabbs`,
    // which resolves the same cascade for the clip box.
    let preview_node_ids: Option<&[String]> = overrides.preview.and_then(|p| match p.target {
        BorderPreviewTargetRef::Nodes(ids) => Some(ids),
        _ => None,
    });
    let preview_canvas_default: Option<BorderConfigEditsView<'_>> =
        overrides.preview.and_then(|p| match p.target {
            BorderPreviewTargetRef::CanvasDefault => Some(p.edits),
            _ => None,
        });
    let preview_force_show_frame = overrides.preview.map(|p| p.force_show_frame).unwrap_or(false);
    // Only materialize the previewed canvas-default slot when a
    // canvas-default preview is actually active; otherwise keep the
    // clone-free borrow into the model (§B7).
    let canvas_default_preview_owned: Option<Option<GlyphBorderConfig>> =
        preview_canvas_default.map(|view| {
            let mut slot = map.canvas.default_border.clone();
            crate::mindmap::border::apply_view_to_slot(&mut slot, &view);
            slot
        });
    let canvas_default_ref: Option<&GlyphBorderConfig> = match &canvas_default_preview_owned {
        Some(opt) => opt.as_ref(),
        None => map.canvas.default_border.as_ref(),
    };
    // Per-call dimming cache. `hex_with_alpha_scaled` parses →
    // multiplies → re-formats; on a dense map in NodeEdit mode the
    // same resolved frame hex recurs once per inactive node.
    // Bounded by the distinct-frame-color count of one frame.
    let mut dim_cache: HashMap<String, String> = HashMap::new();

    let mut out: Vec<BorderNodeData> = Vec::new();
    let mut parent_channel: usize = 1;
    for node_id in sorted_ids {
        let Some(node) = map.nodes.get(node_id) else {
            continue;
        };
        if hidden_set.contains(node.id.as_str()) {
            continue;
        }
        let preview_targets_this_node = preview_node_ids
            .map(|ids| ids.iter().any(|i| i == &node.id))
            .unwrap_or(false);
        if !(node.style.show_frame || (preview_targets_this_node && preview_force_show_frame)) {
            continue;
        }
        // The glyph frame is laid out as four axis-aligned text
        // runs along the node's bounding box, which only makes
        // sense for `NodeShape::Rectangle`. For any other shape we
        // suppress the frame; a curved / shape-aware border is
        // tracked as follow-up work (see CLAUDE.md). Authors still
        // round-trip the `show_frame` flag untouched — we simply
        // don't emit the glyphs.
        //
        // We re-parse `node.style.shape` here rather than reading
        // `area.shape` off the already-built tree because the
        // border tree builder runs from the model, not the node
        // tree — it has no `GlyphArea` in scope. The two parsers
        // are the same (`NodeShape::from_style_string`), so the
        // values agree today; the invariant to preserve if this
        // ever changes is "border pass and node pass resolve the
        // same string to the same `NodeShape`".
        if NodeShape::from_style_string(&node.style.shape) != NodeShape::Rectangle {
            continue;
        }
        let (ox, oy) = offsets.get(&node.id).copied().unwrap_or((0.0, 0.0));
        let frame_color_hex = color::resolve_var(&node.style.frame_color, vars);
        // Border preview: when this node is a `Nodes(ids)` target,
        // fold the staged edits into a clone of its committed slot
        // before resolution. A `CanvasDefault` preview instead
        // substitutes the cascade base (`canvas_default_ref`
        // above) and leaves per-node slots untouched. Either
        // flavor leaves the model itself unmodified.
        let node_slot_owned_for_preview: Option<Option<GlyphBorderConfig>> = if preview_targets_this_node {
            let view = overrides
                .preview
                .map(|p| p.edits)
                .expect("preview_targets_this_node implies preview is Some");
            let mut slot = node.style.border.clone();
            crate::mindmap::border::apply_view_to_slot(&mut slot, &view);
            Some(slot)
        } else {
            None
        };
        let node_slot_ref: Option<&GlyphBorderConfig> = match &node_slot_owned_for_preview {
            Some(opt) => opt.as_ref(),
            None => node.style.border.as_ref(),
        };
        // Routes through `resolve_border_style` so per-node
        // GlyphBorderConfig (preset / font / size / color /
        // pattern / palette) drives the rendered output. Without
        // this the data-model fields exist but the renderer
        // ignores them. This is the *only* resolution of the node
        // border cascade in a frame: the clip-AABB pass needs just
        // the extent and takes `resolve_border_font_size_pt`, and
        // nothing downstream of here resolves it again.
        let mut border_style = resolve_border_style(node_slot_ref, canvas_default_ref, frame_color_hex);
        // NodeEdit dimming: every node *other* than the active
        // target renders its frame at half alpha so the active
        // node visually pops. `node_edit_for == None` (Default
        // mode) is the no-op fast path.
        if overrides
            .node_edit_for
            .is_some_and(|active| active != node.id.as_str())
        {
            border_style.color = match dim_cache.get(&border_style.color) {
                Some(hit) => hit.clone(),
                None => {
                    let dimmed =
                        color::hex_with_alpha_scaled(&border_style.color, INACTIVE_NODE_ALPHA_MULTIPLIER);
                    dim_cache.insert(border_style.color.clone(), dimmed.clone());
                    dimmed
                }
            };
        }
        let color_rgba = color::hex_to_rgba_safe(&border_style.color, [1.0, 1.0, 1.0, 1.0]);
        let palette_cycle =
            crate::mindmap::border::resolve_palette_cycle(&map.palettes, &border_style, color_rgba);

        let pos = node.pos_vec2();
        let size = node.size_vec2();
        out.push(BorderNodeData {
            node_id: node.id.clone(),
            parent_channel,
            border_style,
            color_rgba,
            pos_x: pos.x + ox,
            pos_y: pos.y + oy,
            size_x: size.x,
            size_y: size.y,
            zoom_visibility: node.zoom_window(),
            palette_cycle,
        });
        parent_channel += 1;
    }
    out
}

/// Identity sequence for a slice of [`BorderNodeData`] — the
/// sorted sequence of `node_id`s in tree-insertion order. Two
/// sequences match iff the same set of nodes is framed in the
/// same order. Drag, text-edit, color-preview, and preset-swap
/// all leave this stable (preset swaps change the character
/// content of each run but not the tree shape — the mutator's
/// `Text::Assign` picks up the new glyphs); adding or removing a
/// framed node, toggling `show_frame`, or folding an ancestor
/// drops the equality and forces a full rebuild via the
/// dispatcher in `CanvasFrame::update_border_tree`.
pub fn border_identity_sequence(nodes: &[BorderNodeData]) -> Vec<String> {
    nodes.iter().map(|n| n.node_id.clone()).collect()
}

/// Build the border tree from the given `MindMap` + drag offsets.
/// Convenience wrapper that calls [`border_node_data`] then
/// [`build_border_tree_from_nodes`].
///
/// Tree shape:
///
/// ```text
/// Void (root)
/// ├── Void (per node — channel = 1-based sorted index)
/// │   ├── GlyphArea (top run, channel = 1)
/// │   ├── GlyphArea (bottom run, channel = 2)
/// │   ├── GlyphArea (left column, channel = 3)
/// │   └── GlyphArea (right column, channel = 4)
/// ├── Void (next node)
/// │   └── ...
/// ```
///
/// Iteration order is the lexicographic order of `MindNode.id` —
/// stable across runs, so per-node Void parents always land at
/// the same channel. Without this, `MindMap.nodes` (a `HashMap`)
/// would yield nondeterministic order and make the in-place
/// mutator path unreliable.
///
/// # Costs
///
/// O(N log N) where N is the visible framed-node count (the sort
/// dominates for large maps). Allocates one tree arena plus one
/// `String` per run. Builds with [`BorderChromeOverrides::default`]
/// — the convenience entry point for callers with no preview and
/// no active `NodeEdit`; the app's per-frame path calls
/// [`border_node_data`] directly so it can thread both.
pub fn build_border_tree(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
) -> Tree<GfxElement, GfxMutator> {
    let hidden_set = map.fold_hidden_set();
    build_border_tree_from_nodes(&border_node_data(
        map,
        offsets,
        BorderChromeOverrides::default(),
        &hidden_set,
    ))
}

/// Variant of [`build_border_tree`] that consumes pre-computed
/// node data. Use this in the dispatch path that already called
/// [`border_node_data`] to derive the identity sequence — saves
/// one walk over `MindMap.nodes`.
pub fn build_border_tree_from_nodes(nodes: &[BorderNodeData]) -> Tree<GfxElement, GfxMutator> {
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    let mut unique_id: usize = 1;
    for node in nodes {
        append_border_sub_tree(&mut tree, node, &mut unique_id);
    }
    tree
}

/// Build a [`MutatorTree`](crate::gfx_structs::tree::MutatorTree)
/// that updates an already-registered border tree to the current
/// `(map, offsets)` state without rebuilding the arena. Pairs
/// with [`build_border_tree`] — both consume
/// [`border_node_data`], so applying this mutator to a tree built
/// from a node slice with the same
/// [`border_identity_sequence`] updates each run's variable
/// fields in place.
///
/// The hot-path case this closes: when the color picker is open,
/// every throttled `AboutToWait` drain re-runs the scene build,
/// which previously re-allocated the entire border tree every
/// frame. With this dispatch, picker hover leaves the border
/// tree's arena untouched and only overwrites text / position /
/// color fields on the existing GlyphAreas.
pub fn build_border_mutator_tree(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
) -> crate::gfx_structs::tree::MutatorTree<GfxMutator> {
    let hidden_set = map.fold_hidden_set();
    build_border_mutator_tree_from_nodes(&border_node_data(
        map,
        offsets,
        BorderChromeOverrides::default(),
        &hidden_set,
    ))
}

/// Variant of [`build_border_mutator_tree`] that consumes
/// pre-computed node data. Use this in the dispatch path that
/// already called [`border_node_data`].
pub fn build_border_mutator_tree_from_nodes(
    nodes: &[BorderNodeData],
) -> crate::gfx_structs::tree::MutatorTree<GfxMutator> {
    use crate::gfx_structs::area::DeltaGlyphArea;
    use crate::gfx_structs::mutator::Mutation;
    use crate::gfx_structs::tree::MutatorTree;

    let mut mt: MutatorTree<GfxMutator> = MutatorTree::new_with(GfxMutator::new_void(0));
    for node in nodes {
        let parent_node = mt.arena.new_node(GfxMutator::new_void(node.parent_channel));
        mt.root.append(parent_node, &mut mt.arena);

        let specs = crate::mindmap::border::border_run_specs(
            &node.border_style,
            (node.pos_x, node.pos_y),
            (node.size_x, node.size_y),
        );
        for spec in specs {
            let regions = build_border_regions(
                spec.cluster_count,
                &node.palette_cycle,
                node.color_rgba,
                spec.palette_offset,
            );
            let mut area = GlyphArea::new_with_str(
                &spec.text,
                spec.font_size_pt,
                spec.line_height_pt,
                Vec2::new(spec.position.0, spec.position.1),
                Vec2::new(spec.bounds.0, spec.bounds.1),
            );
            area.regions = regions;
            area.zoom_visibility = node.zoom_visibility;
            let delta = DeltaGlyphArea::full_assign_from(&area);
            let leaf = mt.arena.new_node(GfxMutator::new(
                Mutation::AreaDelta(Box::new(delta)),
                spec.channel,
            ));
            parent_node.append(leaf, &mut mt.arena);
        }
    }
    mt
}

/// Build one per-node sub-tree (Void parent + 4 GlyphArea runs) and
/// append it under `tree.root`. Kept as a private helper so
/// `build_border_tree` stays readable. `BorderNodeData.parent_channel`
/// is the stable 1-based sorted-index channel — see
/// [`BorderNodeData::parent_channel`].
fn append_border_sub_tree(
    tree: &mut Tree<GfxElement, GfxMutator>,
    node: &BorderNodeData,
    unique_id: &mut usize,
) {
    let specs = crate::mindmap::border::border_run_specs(
        &node.border_style,
        (node.pos_x, node.pos_y),
        (node.size_x, node.size_y),
    );

    // Per-node Void parent — groups the four runs for targeted
    // mutation. The parent's channel is the stable sorted-index
    // value so distinct nodes never collide across rebuilds.
    let parent_id = tree
        .arena
        .new_node(GfxElement::new_void_with_id(node.parent_channel, *unique_id));
    tree.root.append(parent_id, &mut tree.arena);
    *unique_id += 1;

    // Stable channels 1..=4 inside each border sub-tree. The
    // per-node Void parent already disambiguates across nodes.
    // Palette offsets sweep top → right → bottom → left around
    // the rectangle so a color cycle wraps cleanly. See
    // `BorderRunSpec` for the spec contract.
    for spec in &specs {
        append_border_run(
            tree,
            parent_id,
            spec.channel,
            *unique_id,
            &spec.text,
            spec.font_size_pt,
            spec.line_height_pt,
            spec.position,
            spec.bounds,
            node.color_rgba,
            node.zoom_visibility,
            &node.palette_cycle,
            spec.palette_offset,
            spec.cluster_count,
        );
        *unique_id += 1;
    }
}

// Each parameter is an independently varying per-glyph render input
// the tree-builder threads from the layout walk; bundling them into
// a struct would just rename the same data with no readability win.
#[allow(clippy::too_many_arguments)]
pub(super) fn append_border_run(
    tree: &mut Tree<GfxElement, GfxMutator>,
    parent_id: NodeId,
    channel: usize,
    unique_id: usize,
    text: &str,
    font_size: f32,
    line_height: f32,
    position: (f32, f32),
    bounds: (f32, f32),
    color_rgba: [f32; 4],
    zoom_visibility: ZoomVisibility,
    palette_cycle: &[[f32; 4]],
    palette_offset: usize,
    cluster_count: usize,
) {
    let mut area = GlyphArea::new_with_str(
        text,
        font_size,
        line_height,
        Vec2::new(position.0, position.1),
        Vec2::new(bounds.0, bounds.1),
    );
    area.zoom_visibility = zoom_visibility;

    // Per-cluster ColorFontRegions when the user opts into palette
    // cycling, otherwise a single uniform region (matches the
    // pre-pattern path's cost). `cluster_count` is pre-computed by
    // `border_run_specs` so this body never re-walks the string.
    area.regions = build_border_regions(cluster_count, palette_cycle, color_rgba, palette_offset);

    let element = GfxElement::new_area_non_indexed_with_id(area, channel, unique_id);
    let node = tree.arena.new_node(element);
    parent_id.append(node, &mut tree.arena);
}
