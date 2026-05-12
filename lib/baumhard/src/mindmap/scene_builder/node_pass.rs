// SPDX-License-Identifier: MPL-2.0

//! Per-node pass — emits `TextElement`s + `BorderElement`s and
//! computes the per-node clip AABBs (`node_aabbs`) in a single
//! iteration over visible nodes. Kept as one pass rather than split
//! into peer `text.rs` + `border.rs` modules because:
//!
//! - the `TextElement`, `BorderElement`, and AABB all derive from
//!   the same `(pos, size, offset, frame_color)` tuple
//! - the clip AABB's expansion-for-border math (see `clip_pos` /
//!   `clip_size` below) is the same `BorderStyle::default_with_color`
//!   resolution the border element uses
//!
//! Splitting them would force either a two-pass walk over
//! `map.nodes.values()` (perf regression on the hot drag path) or
//! an imbalanced `layout.rs` that returns three vectors — defeating
//! the role-per-file goal.

use std::collections::HashMap;

use glam::Vec2;

use crate::mindmap::border::resolve_border_style;
use crate::mindmap::model::{GlyphBorderConfig, MindMap, TextRun};
use crate::util::color::{hex_with_alpha_scaled, resolve_var};

use super::{BorderElement, TextElement};

/// Alpha multiplier applied to text-run + border colors of every
/// node that is **not** the active NodeEdit target. Half-alpha is
/// the "you are inside this node" affordance: the active node
/// stays vivid while the rest of the canvas falls back. Single
/// constant so the section-frame pass and any future inactive-
/// chrome consumer share the dim shade.
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

/// Walk every visible node and emit its text element + border
/// element + clip AABB. Returns the three collections in a tuple —
/// the connection pass downstream consumes `node_aabbs` for its
/// clip filter, so this walk must complete before connections start.
///
/// Hidden-by-fold nodes are skipped entirely. Empty-text nodes skip
/// the `TextElement` push but still contribute an AABB. Frameless
/// nodes skip the `BorderElement` push and use a raw-rect AABB
/// (no border-expansion) so connection glyphs can run right up to
/// the node edge.
pub(super) fn build_node_elements(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    node_edit_for: Option<&str>,
    border_preview: Option<super::BorderPreview<'_>>,
) -> (Vec<TextElement>, Vec<BorderElement>, Vec<(Vec2, Vec2)>) {
    // Hoist the preview-target match out of the per-node loop:
    // most rebuilds run with `border_preview = None` and we want
    // the steady-state branch to be a single `is_none()` check
    // per node. Match each preview target shape once here.
    let preview_node_ids: Option<&[String]> = border_preview.and_then(|p| match p.target {
        super::BorderPreviewTargetRef::Nodes(ids) => Some(ids),
        _ => None,
    });
    let preview_canvas_default: Option<super::BorderConfigEditsView<'_>> =
        border_preview.and_then(|p| match p.target {
            super::BorderPreviewTargetRef::CanvasDefault => Some(p.edits),
            _ => None,
        });
    let preview_force_show_frame = border_preview.map(|p| p.force_show_frame).unwrap_or(false);
    // Hoist the canvas-default-with-preview-folded-in clone OUT of
    // the per-node loop. With `preview_canvas_default = None` (the
    // common case) we keep the clone-free `Option<&GlyphBorderConfig>`
    // borrow into the model; only when a canvas-default preview is
    // active do we materialise an owned cloned-and-mutated slot.
    // §B7: pre-fix this clone fired per-node-per-frame regardless
    // of whether any preview was active.
    //
    // `apply_view_to_slot` can empty the slot when `view.clear ==
    // true` — keep the result as `Option<GlyphBorderConfig>` and
    // only borrow `.as_ref()` for the resolver path.
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
    let vars = &map.canvas.theme_variables;
    let mut text_elements = Vec::new();
    let mut border_elements = Vec::new();
    let mut node_aabbs: Vec<(Vec2, Vec2)> = Vec::new();
    // Per-call dimming-color cache. `hex_with_alpha_scaled` parses
    // → multiplies → re-formats; on a dense map in NodeEdit mode
    // the same `(resolved_hex, factor)` pair recurs once per text
    // run + once per border, often dozens of times. Caching by
    // input hex amortises the parse cost. Key is `String` because
    // resolved hex strings outlive the caller's borrow scope; the
    // cache is local to one frame's `build_node_elements` call so
    // size stays bounded by visible-node count.
    let mut dim_cache: HashMap<String, String> = HashMap::new();

    for node in map.nodes.values() {
        if map.is_hidden_by_fold(node) {
            continue;
        }

        let (ox, oy) = offsets.get(&node.id).copied().unwrap_or((0.0, 0.0));
        let pos = node.pos_vec2();
        let size = node.size_vec2();
        let pos_x = pos.x + ox;
        let pos_y = pos.y + oy;
        let size_x = size.x;
        let size_y = size.y;

        // NodeEdit dimming: every node *other* than the NodeEdit target
        // renders chrome + text at INACTIVE_NODE_ALPHA_MULTIPLIER alpha so
        // the active node visually pops. `node_edit_for == None` (the
        // Default-mode case) is the no-op fast path.
        let dim_this_node = node_edit_for
            .map(|active| active != node.id.as_str())
            .unwrap_or(false);

        // Resolve the frame color through theme variables once — used for
        // both the clip AABB sizing and the border element below.
        let frame_color = resolve_var(&node.style.frame_color, vars);

        // Resolve the per-node border config once when the frame is
        // visible, then reuse for clip-AABB math, palette-cycle
        // resolution, and the emitted `BorderElement`. The cascade
        // walks `node.style.border` → `canvas.default_border` →
        // hardcoded defaults; doing this twice per visible node was a
        // hot-path regression — the resolver also reparses each side
        // pattern, so the cost compounds.
        // Border preview: when this node is in the preview's
        // `Nodes(ids)` target, fold the staged edits into a clone
        // of the committed slot before resolution. When the
        // preview targets `CanvasDefault`, fold the edits into a
        // clone of `canvas.default_border` and pass that to the
        // resolver as the cascade base — the per-node slot stays
        // unchanged. Either flavour leaves the model untouched.
        // Per-node preview: target this node only when a `Nodes`
        // preview is active AND this node's id is in its target
        // list. The steady-state path (no preview / preview
        // targets a different node) keeps the clone-free borrow
        // into `node.style.border` — §B7: no allocations on the
        // common path.
        let preview_targets_this_node = preview_node_ids
            .map(|ids| ids.iter().any(|i| i == &node.id))
            .unwrap_or(false);
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
        let visible = node.style.show_frame || (preview_targets_this_node && preview_force_show_frame);
        let resolved_border = if visible {
            Some(resolve_border_style(
                node_slot_ref,
                canvas_default_ref,
                frame_color,
            ))
        } else {
            None
        };

        // Clip AABB: when a node has a visible frame, the rendered border
        // extends beyond the raw node rect by roughly one border
        // `font_size` vertically and one `approx_char_width` horizontally.
        // Expand the clip box to match so connection glyphs don't land
        // inside the visible frame area (see renderer::rebuild_border_buffers
        // for the matching layout math).
        let (clip_pos, clip_size) = if let Some(border_style) = &resolved_border {
            let bf = border_style.font_size_pt;
            let bcw = bf * crate::mindmap::border::BORDER_APPROX_CHAR_WIDTH_FRAC;
            (
                Vec2::new(pos_x - bcw, pos_y - bf),
                Vec2::new(size_x + 2.0 * bcw, size_y + 2.0 * bf),
            )
        } else {
            (Vec2::new(pos_x, pos_y), Vec2::new(size_x, size_y))
        };
        node_aabbs.push((clip_pos, clip_size));

        // One TextElement per section with non-empty text.
        // Empty-text sections (a freshly-created orphan node's
        // default section before the user types anything) skip
        // emission — the same fast-path as the pre-section
        // empty-text node behaviour. Sections with explicit zero /
        // negative / non-finite size or non-finite offset also skip
        // emission: they would render at degenerate or NaN bounds
        // and confuse downstream renderer / hit-test math. The
        // verifier flags these so authors can fix the source.
        for (section_idx, section) in node.sections.iter().enumerate() {
            if section.text.is_empty() {
                continue;
            }
            if !section.offset.x.is_finite() || !section.offset.y.is_finite() {
                continue;
            }
            if let Some(sz) = section.size.as_ref() {
                if !sz.width.is_finite() || !sz.height.is_finite() || sz.width <= 0.0 || sz.height <= 0.0 {
                    continue;
                }
            }
            let resolved_runs: Vec<TextRun> = section
                .text_runs
                .iter()
                .map(|run| {
                    let mut r = run.clone();
                    let resolved = resolve_var(&run.color, vars);
                    r.color = if dim_this_node {
                        // Cache lookup keyed by the resolved hex —
                        // every text run with the same color shares
                        // one parse / multiply / format round trip.
                        if let Some(hit) = dim_cache.get(resolved) {
                            hit.clone()
                        } else {
                            let dimmed = hex_with_alpha_scaled(resolved, INACTIVE_NODE_ALPHA_MULTIPLIER);
                            dim_cache.insert(resolved.to_string(), dimmed.clone());
                            dimmed
                        }
                    } else {
                        resolved.to_string()
                    };
                    r
                })
                .collect();
            let ((sx, sy), (sw, sh)) = section_aabb(section, pos_x, pos_y, size_x, size_y);
            text_elements.push(TextElement {
                node_id: node.id.clone(),
                section_idx,
                text: section.text.clone(),
                text_runs: resolved_runs,
                position: (sx, sy),
                size: (sw, sh),
            });
        }

        // Border element — inherits the owning node's zoom window
        // so the frame never outlives its node at any zoom level.
        // Reuses the `resolved_border` populated above so the
        // resolver runs at most once per visible framed node.
        if let Some(mut border_style) = resolved_border {
            if dim_this_node {
                // Same per-call cache as the text-run branch above.
                border_style.color = if let Some(hit) = dim_cache.get(&border_style.color) {
                    hit.clone()
                } else {
                    let dimmed = hex_with_alpha_scaled(&border_style.color, INACTIVE_NODE_ALPHA_MULTIPLIER);
                    dim_cache.insert(border_style.color.clone(), dimmed.clone());
                    dimmed
                };
            }
            let fallback_rgba =
                crate::util::color::hex_to_rgba_safe(&border_style.color, [1.0, 1.0, 1.0, 1.0]);
            let palette_cycle =
                crate::mindmap::border::resolve_palette_cycle(&map.palettes, &border_style, fallback_rgba);
            border_elements.push(BorderElement {
                node_id: node.id.clone(),
                border_style,
                node_position: (pos_x, pos_y),
                node_size: (size_x, size_y),
                zoom_visibility: node.zoom_window(),
                palette_cycle,
            });
        }
    }

    (text_elements, border_elements, node_aabbs)
}
