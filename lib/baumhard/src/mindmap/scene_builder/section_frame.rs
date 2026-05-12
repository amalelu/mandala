// SPDX-License-Identifier: MPL-2.0

//! Per-section frame pass for `InteractionMode::NodeEdit`. Emits
//! one [`SectionFrameElement`] per section of the active node so
//! the renderer can draw a glyph rectangle around each section —
//! the visual cue telling the user "these are the per-section
//! subdivisions you can pick from."
//!
//! The frame style is resolved through the same
//! [`crate::mindmap::border::resolve_section_frame_border`]
//! cascade that backs every other border in the system. Authors
//! who want a per-section frame style write to
//! `MindSection.frame_border`; map-wide defaults live on
//! `Canvas.default_section_frame_border` (and the focused
//! variant). When neither is set, a thin / heavy floor preset
//! flows through the same resolver so the returned `BorderStyle`
//! has the same shape every other border consumer sees.
//!
//! Skipped entirely in Default mode and for single-section active
//! nodes (where the frame would duplicate the border, and the
//! single-section short-circuit bypasses NodeEdit anyway). The
//! caller (`build_scene_with_cache`) gates emission on
//! `node_edit_for == Some(active)`.

use std::collections::HashMap;

use super::node_pass::section_aabb;
use super::{SectionFrameElement, SELECTED_EDGE_COLOR};
use crate::mindmap::border::resolve_palette_cycle;
use crate::mindmap::model::MindMap;
use crate::util::color::{hex_to_rgba_safe, resolve_var};

/// Emit one [`SectionFrameElement`] per section of `active_node`.
/// Returns an empty vector for:
/// - `active_node = None` (Default mode — no frames anywhere).
/// - The named node missing from `map.nodes` (stale NodeEdit
///   target after a custom-mutation deletion).
/// - The named node hidden by fold (the frame would otherwise
///   render under collapsed chrome).
/// - The named node having `sections.len() <= 1` (frame would
///   duplicate the border; the single-section short-circuit
///   bypasses NodeEdit anyway).
/// - Any section with non-finite or non-positive size /
///   non-finite offset — same skip rules `node_pass` applies to
///   `TextElement` emission, so frames track the same set of
///   "renderable" sections.
///
/// Each emitted element carries a fully-resolved
/// [`crate::mindmap::border::BorderStyle`]
/// plus a `palette_cycle`. The matching
/// `(active_node, focused_section_idx)` section emits
/// `focused = true`; the resolver flips to the focused-variant
/// cascade for that one element so its style can differ from
/// its unfocused siblings.
pub fn build_section_frames(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    active_node: Option<&str>,
    focused_section: Option<(&str, usize)>,
    border_preview: Option<super::BorderPreview<'_>>,
) -> Vec<SectionFrameElement> {
    let Some(active_id) = active_node else {
        return Vec::new();
    };
    let Some(node) = map.nodes.get(active_id) else {
        return Vec::new();
    };
    if map.is_hidden_by_fold(node) {
        return Vec::new();
    }
    if node.sections.len() <= 1 {
        return Vec::new();
    }

    // Missing-offset for the active node means the layout/scene-
    // build invariant ("the active node has an offset entry") was
    // broken upstream — every other emission path consults the
    // same map and would also fall back to (0,0) silently. Log so
    // the regression is visible without panicking; per
    // `CODE_CONVENTIONS.md` §9 interactive paths must keep running.
    let (ox, oy) = match offsets.get(active_id).copied() {
        Some(v) => v,
        None => {
            log::warn!(
                "build_section_frames: active node {:?} missing from `offsets` map; \
                 falling back to (0, 0) — frames will render at the node's authored \
                 position. This indicates the layout/scene-build pass didn't populate \
                 the active node's offset.",
                active_id
            );
            (0.0, 0.0)
        }
    };
    let pos = node.pos_vec2();
    let size = node.size_vec2();
    // Active-node AABB sanity: NaN or non-positive size produces
    // degenerate frames (3-cluster runs from `border_run_specs`
    // when `size.x / (font_size * 0.6)` is NaN or 0). Bail out
    // cleanly — same defence-in-depth shape `node_pass.rs` uses
    // for `TextElement` emission. Pre-fix the per-section guard
    // below caught only authored-bad section sizes; the active
    // node's own size was unguarded.
    if !pos.x.is_finite()
        || !pos.y.is_finite()
        || !size.x.is_finite()
        || !size.y.is_finite()
        || size.x <= 0.0
        || size.y <= 0.0
    {
        return Vec::new();
    }
    let pos_x = pos.x + ox;
    let pos_y = pos.y + oy;
    let size_x = size.x;
    let size_y = size.y;

    let focused_idx = focused_section
        .filter(|(id, _)| *id == active_id)
        .map(|(_, idx)| idx);

    // The active-affordance signal (cyan SELECTED_EDGE_COLOR) sits
    // at the bottom of the cascade — authors who set
    // `frame_border.color` on their override fully replace it,
    // which is the desired shape for "make my borders tell a
    // story." Authors who want the cyan default just leave color
    // unset on their config.
    let frame_color_resolved = resolve_var(SELECTED_EDGE_COLOR, &map.canvas.theme_variables);

    // Hoist preview-target match out of the per-section loop. Most
    // rebuilds run with `border_preview = None` and we want the
    // steady-state per-section iteration to be one `is_none()`
    // check per branch.
    let preview_section_targets: Option<&[(String, usize)]> = border_preview.and_then(|p| match p.target {
        super::BorderPreviewTargetRef::Sections(ts) => Some(ts),
        _ => None,
    });
    let preview_canvas_unfocused: Option<super::BorderConfigEditsView<'_>> =
        border_preview.and_then(|p| match p.target {
            super::BorderPreviewTargetRef::CanvasSectionFrame => Some(p.edits),
            _ => None,
        });
    let preview_canvas_focused: Option<super::BorderConfigEditsView<'_>> =
        border_preview.and_then(|p| match p.target {
            super::BorderPreviewTargetRef::CanvasSectionFrameFocused => Some(p.edits),
            _ => None,
        });
    // Pre-clone the canvas defaults ONLY when a canvas-targeted
    // preview is active — steady-state keeps the clone-free
    // borrow into `map.canvas`. §B7: pre-fix this allocated two
    // `Option<GlyphBorderConfig>` per call regardless of preview
    // state.
    let canvas_unfocused_owned: Option<Option<crate::mindmap::model::GlyphBorderConfig>> =
        preview_canvas_unfocused.map(|view| {
            let mut slot = map.canvas.default_section_frame_border.clone();
            crate::mindmap::border::apply_view_to_slot(&mut slot, &view);
            slot
        });
    let canvas_focused_owned: Option<Option<crate::mindmap::model::GlyphBorderConfig>> =
        preview_canvas_focused.map(|view| {
            let mut slot = map.canvas.default_focused_section_frame_border.clone();
            crate::mindmap::border::apply_view_to_slot(&mut slot, &view);
            slot
        });
    let canvas_unfocused_default: Option<&crate::mindmap::model::GlyphBorderConfig> =
        match &canvas_unfocused_owned {
            Some(opt) => opt.as_ref(),
            None => map.canvas.default_section_frame_border.as_ref(),
        };
    let canvas_focused_default: Option<&crate::mindmap::model::GlyphBorderConfig> =
        match &canvas_focused_owned {
            Some(opt) => opt.as_ref(),
            None => map.canvas.default_focused_section_frame_border.as_ref(),
        };

    let mut out: Vec<SectionFrameElement> = Vec::with_capacity(node.sections.len());
    for (section_idx, section) in node.sections.iter().enumerate() {
        if !section.offset.x.is_finite() || !section.offset.y.is_finite() {
            continue;
        }
        if let Some(sz) = section.size.as_ref() {
            if !sz.width.is_finite() || !sz.height.is_finite() || sz.width <= 0.0 || sz.height <= 0.0 {
                continue;
            }
        }
        let ((sx, sy), (sw, sh)) = section_aabb(section, pos_x, pos_y, size_x, size_y);
        let focused = focused_idx == Some(section_idx);

        // Apply per-section preview to the section's `frame_border`
        // slot ONLY if this section is a `Sections((id, idx))`
        // target. Steady-state keeps the clone-free borrow into
        // `section.frame_border` — §B7: no allocations on the
        // common path.
        let section_targeted = preview_section_targets
            .map(|ts| ts.iter().any(|(id, idx)| id == active_id && *idx == section_idx))
            .unwrap_or(false);
        let section_owned_for_preview: Option<Option<crate::mindmap::model::GlyphBorderConfig>> =
            if section_targeted {
                let view = border_preview
                    .map(|p| p.edits)
                    .expect("section_targeted implies preview is Some");
                let mut slot = section.frame_border.clone();
                crate::mindmap::border::apply_view_to_slot(&mut slot, &view);
                Some(slot)
            } else {
                None
            };
        let section_slot_ref: Option<&crate::mindmap::model::GlyphBorderConfig> =
            match &section_owned_for_preview {
                Some(opt) => opt.as_ref(),
                None => section.frame_border.as_ref(),
            };

        // Resolve through the same cascade
        // `resolve_section_frame_border` would, but using the
        // possibly-previewed slot + canvas defaults. Floor (when
        // both layers are `None`) reuses the standard config the
        // resolver would have produced.
        let border_style = resolve_section_frame_border_with_overrides(
            section_slot_ref,
            canvas_unfocused_default,
            canvas_focused_default,
            focused,
            frame_color_resolved,
        );
        let fallback_rgba = hex_to_rgba_safe(&border_style.color, [1.0, 1.0, 1.0, 1.0]);
        let palette_cycle = resolve_palette_cycle(&map.palettes, &border_style, fallback_rgba);
        out.push(SectionFrameElement {
            node_id: active_id.to_string(),
            section_idx,
            position: (sx, sy),
            size: (sw, sh),
            border_style,
            palette_cycle,
            focused,
        });
    }
    out
}

/// Section-frame cascade keyed off owned slot clones rather than a
/// `&Canvas` and `&MindSection` — same shape as
/// [`crate::mindmap::border::resolve_section_frame_border`] but
/// suitable for the preview path that wants to substitute the
/// per-section / canvas-default slots before resolution. With
/// `border_preview = None` at the call site the two paths produce
/// byte-identical output (parity contract).
fn resolve_section_frame_border_with_overrides(
    section_slot: Option<&crate::mindmap::model::GlyphBorderConfig>,
    canvas_unfocused_default: Option<&crate::mindmap::model::GlyphBorderConfig>,
    canvas_focused_default: Option<&crate::mindmap::model::GlyphBorderConfig>,
    focused: bool,
    frame_color_fallback: &str,
) -> crate::mindmap::border::BorderStyle {
    if let Some(cfg) = section_slot {
        return crate::mindmap::border::resolve_border_style(Some(cfg), None, frame_color_fallback);
    }
    let canvas_chosen = if focused {
        canvas_focused_default.or(canvas_unfocused_default)
    } else {
        canvas_unfocused_default
    };
    if let Some(cfg) = canvas_chosen {
        return crate::mindmap::border::resolve_border_style(Some(cfg), None, frame_color_fallback);
    }
    // Floor — same shape `resolve_section_frame_border` synthesises.
    let floor = crate::mindmap::border::section_frame_floor_config(focused);
    crate::mindmap::border::resolve_border_style(Some(floor), None, frame_color_fallback)
}
