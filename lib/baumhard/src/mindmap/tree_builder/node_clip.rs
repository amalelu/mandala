// SPDX-License-Identifier: MPL-2.0

//! Per-node clip AABBs — the one thing the connection pass needs
//! from the node layer.
//!
//! A connection glyph that lands inside a node's rectangle reads as
//! a line running *through* the node, so the connection sampler
//! drops every sample that falls inside one of these boxes. For a
//! framed node the box is grown by the rendered border's extent
//! (roughly one border `font_size` vertically and one
//! `approx_char_width` horizontally) so glyphs stop at the frame
//! rather than under it.
//!
//! Only the *size* half of the border cascade is resolved here —
//! the visible frame itself is built by [`super::border`], which
//! resolves the whole thing once. This pass needs a single `f32`,
//! so it calls
//! [`resolve_border_font_size_pt`](crate::mindmap::border::resolve_border_font_size_pt)
//! rather than `resolve_border_style`: no glyph set, no side
//! patterns, no palette, no color string, no allocation on any
//! path where the node is not the target of a border preview.

use std::collections::{HashMap, HashSet};

use glam::Vec2;

use crate::mindmap::border::resolve_border_font_size_pt;
use crate::mindmap::model::{GlyphBorderConfig, MindMap};

use super::overrides::{BorderConfigEditsView, BorderPreview, BorderPreviewTargetRef};

/// Alpha multiplier applied to the text-run + border colors of
/// every node that is **not** the active `NodeEdit` target.
/// Half-alpha is the "you are inside this node" affordance: the
/// active node stays vivid while the rest of the canvas falls
/// back. One constant so the border pass
/// ([`super::border::border_node_data`]) and the application
/// crate's node-text dimming overlay share the same shade.
pub const INACTIVE_NODE_ALPHA_MULTIPLIER: f32 = 0.5;

/// Compute the absolute (canvas-space) position + size of a
/// [`MindSection`](crate::mindmap::model::MindSection) given its
/// owning node's already-resolved `(pos_x, pos_y)` + `(size_x,
/// size_y)`. Pulls in the section's `offset` (always present;
/// defaults to `(0, 0)`) and `size` (`None` = fill the parent).
///
/// Inlined so per-section iteration stays branchless on the
/// happy path — most authored sections fill the node, so the
/// `size.is_none()` branch is the predicted side.
#[inline]
pub(super) fn section_aabb(
    section: &crate::mindmap::model::MindSection,
    node_pos_x: f32,
    node_pos_y: f32,
    node_size_x: f32,
    node_size_y: f32,
) -> ((f32, f32), (f32, f32)) {
    let pos_x = node_pos_x + section.offset.x as f32;
    let pos_y = node_pos_y + section.offset.y as f32;
    let (size_x, size_y) = match &section.size {
        Some(sz) => (sz.width as f32, sz.height as f32),
        None => (node_size_x, node_size_y),
    };
    ((pos_x, pos_y), (size_x, size_y))
}

/// Walk every visible node and return its clip AABB as
/// `(top_left, size)` in canvas space.
///
/// Hidden-by-fold nodes contribute nothing. A frameless node
/// contributes its raw rect so connection glyphs can run right up
/// to the node edge; a framed node contributes the border-expanded
/// rect described in the module header.
///
/// `border_preview` participates because a preview can change the
/// resolved border `font_size_pt` (and, through `force_show_frame`,
/// whether a frame exists at all) — the clip box has to track what
/// the user is actually looking at, or previewed glyphs would clip
/// against the committed geometry.
///
/// # Costs
///
/// O(visible nodes), two `Option` derefs and an `f32` copy each.
/// **No allocation on the steady-state path** beyond the returned
/// `Vec` — the whole point of
/// [`resolve_border_font_size_pt`](crate::mindmap::border::resolve_border_font_size_pt)
/// over the full resolver. A `border_preview` that targets a node
/// clones that one node's `GlyphBorderConfig` slot.
pub fn node_clip_aabbs(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    border_preview: Option<BorderPreview<'_>>,
    hidden_set: &HashSet<&str>,
) -> Vec<(Vec2, Vec2)> {
    // Hoist the preview-target match out of the per-node loop:
    // most rebuilds run with `border_preview = None` and we want
    // the steady-state branch to be a single `is_none()` check
    // per node. Match each preview target shape once here.
    let preview_node_ids: Option<&[String]> = border_preview.and_then(|p| match p.target {
        BorderPreviewTargetRef::Nodes(ids) => Some(ids),
        _ => None,
    });
    let preview_canvas_default: Option<BorderConfigEditsView<'_>> =
        border_preview.and_then(|p| match p.target {
            BorderPreviewTargetRef::CanvasDefault => Some(p.edits),
            _ => None,
        });
    let preview_force_show_frame = border_preview.map(|p| p.force_show_frame).unwrap_or(false);
    // Hoist the canvas-default-with-preview-folded-in clone OUT of
    // the per-node loop. With `preview_canvas_default = None` (the
    // common case) we keep the clone-free `Option<&GlyphBorderConfig>`
    // borrow into the model; only when a canvas-default preview is
    // active do we materialize an owned cloned-and-mutated slot.
    // §B7: pre-fix this clone fired per-node-per-frame regardless
    // of whether any preview was active.
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
    let mut node_aabbs: Vec<(Vec2, Vec2)> = Vec::with_capacity(map.nodes.len());
    for node in map.nodes.values() {
        if hidden_set.contains(node.id.as_str()) {
            continue;
        }

        let (ox, oy) = offsets.get(&node.id).copied().unwrap_or((0.0, 0.0));
        let pos = node.pos_vec2();
        let size = node.size_vec2();
        let pos_x = pos.x + ox;
        let pos_y = pos.y + oy;

        let preview_targets_this_node = preview_node_ids
            .map(|ids| ids.iter().any(|i| i == &node.id))
            .unwrap_or(false);
        if !(node.style.show_frame || (preview_targets_this_node && preview_force_show_frame)) {
            node_aabbs.push((Vec2::new(pos_x, pos_y), Vec2::new(size.x, size.y)));
            continue;
        }

        // `node_slot_owned_for_preview` is only allocated when a
        // preview folds into this node's slot. Holds the cloned-
        // and-mutated slot for the resolver to borrow from.
        let node_slot_owned_for_preview: Option<Option<GlyphBorderConfig>> = if preview_targets_this_node {
            let view = border_preview
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
        let bf = resolve_border_font_size_pt(node_slot_ref, canvas_default_ref);
        let bcw = bf * crate::mindmap::border::BORDER_APPROX_CHAR_WIDTH_FRAC;
        node_aabbs.push((
            Vec2::new(pos_x - bcw, pos_y - bf),
            Vec2::new(size.x + 2.0 * bcw, size.y + 2.0 * bf),
        ));
    }

    node_aabbs
}
