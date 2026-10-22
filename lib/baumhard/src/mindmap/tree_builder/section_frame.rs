// SPDX-License-Identifier: MPL-2.0

//! Section frames — the `InteractionMode::NodeEdit` per-section
//! emission pass and the tree it feeds.
//!
//! [`build_section_frames`] emits one [`SectionFrameElement`] per
//! section of the active node so the user sees a glyph rectangle
//! around each section — the cue telling them "these are the
//! per-section subdivisions you can pick from."
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
//! Emission is skipped entirely in Default mode and for
//! single-section active nodes (where the frame would duplicate
//! the border, and the single-section short-circuit bypasses
//! NodeEdit anyway).
//!
//! The tree half emits one per-section Void parent and the eight
//! `GlyphArea` runs — four fill rails plus four corners — per
//! element. The run geometry — four fill rails and four
//! corners — comes from
//! [`crate::mindmap::border::border_run_specs`] — the **same** path
//! node borders use — so any preset, any per-side `SidePattern`,
//! any per-corner glyph, any palette cycle that node borders
//! support flows through section frames identically.
//!
//! Stable identity = the order [`SectionFrameElement`]s are
//! emitted in. Per-frame Void parent's channel is the 1-based
//! sorted index so distinct frames never collide across rebuilds.

use std::collections::{HashMap, HashSet};

use indextree::NodeId;

use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::mutator::GfxMutator;
use crate::gfx_structs::tree::Tree;
use crate::mindmap::border::{resolve_palette_cycle, BorderStyle};
use crate::mindmap::model::{GlyphBorderConfig, MindMap};
use crate::mindmap::SELECTION_HIGHLIGHT_HEX;
use crate::util::color::{hex_to_rgba_safe, resolve_var};

use super::node::renderable_section;
use super::node_clip::section_aabb;
use super::overrides::{BorderConfigEditsView, BorderPreview, BorderPreviewTargetRef};

/// A glyph-drawn rectangle outlining one section of the active
/// NodeEdit node — the visual cue telling the user "this is the
/// per-section subdivision you can pick from."
///
/// Section frames flow through the same [`BorderStyle`] machinery
/// node borders do: any preset, any per-side `SidePattern`, any
/// per-corner glyph, any font, any color, any palette. The
/// resolver cascade (`resolve_section_frame_border` in
/// `crate::mindmap::border`) is:
///   1. `MindSection.frame_border` if `Some` (per-section author
///      override).
///   2. else `Canvas.default_section_frame_border` (or
///      `default_focused_section_frame_border` when `focused`).
///   3. else a hardcoded thin (default) / heavy (focused) floor.
///
/// One element per section of the active node when emitted. Empty
/// for: Default mode, NodeEdit on a single-section node (frame
/// would duplicate the border), NodeEdit on a missing /
/// hidden-by-fold node.
#[derive(Debug, Clone)]
pub struct SectionFrameElement {
    /// Owning MindNode id — same id every per-node element keys
    /// on. Multiple `SectionFrameElement`s share this id when the
    /// active node has multiple sections.
    pub node_id: String,
    /// Index into [`MindNode.sections`](crate::mindmap::model::MindNode::sections).
    /// Stable across rebuilds for unchanged nodes.
    pub section_idx: usize,
    /// Top-left of the section's effective AABB in canvas space —
    /// `node.position + section.offset`. Same value that the
    /// matching section area carries; the frame tree reads from
    /// here to place the perimeter glyph runs.
    pub position: (f32, f32),
    /// Size of the section's effective AABB —
    /// `section.size.unwrap_or(node.size)`.
    pub size: (f32, f32),
    /// Resolved per-frame [`BorderStyle`] — preset, side patterns,
    /// corners, font, size, color, palette field. Mirrors
    /// [`super::BorderNodeData::border_style`]; consumers feed it
    /// to `crate::mindmap::border::border_run_specs` for the
    /// eight-run geometry.
    pub border_style: BorderStyle,
    /// Resolved per-cycle-position colors when the frame uses a
    /// `color_palette`; empty otherwise. Mirrors
    /// [`super::BorderNodeData::palette_cycle`].
    pub palette_cycle: Vec<[f32; 4]>,
    /// `true` when this section is the focus of an active text
    /// editor. The renderer draws focused frames using the heavy-
    /// preset floor (or the canvas-level
    /// `default_focused_section_frame_border` when set) so the
    /// user sees which section is being edited among the active
    /// node's siblings. Plan §4.4.
    pub focused: bool,
}

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
///   non-finite offset — the same `renderable_section` predicate
///   the node pass applies to section areas, so frames track
///   exactly the sections that render.
///
/// Each emitted element carries a fully-resolved
/// [`BorderStyle`]
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
    border_preview: Option<BorderPreview<'_>>,
    hidden_set: &HashSet<&str>,
) -> Vec<SectionFrameElement> {
    let Some(active_id) = active_node else {
        return Vec::new();
    };
    let Some(node) = map.nodes.get(active_id) else {
        return Vec::new();
    };
    if hidden_set.contains(node.id.as_str()) {
        return Vec::new();
    }
    if node.sections.len() <= 1 {
        return Vec::new();
    }

    // `offsets` holds in-flight *drag* deltas and is empty on
    // every non-drag rebuild (`CanvasFrame::offsets`), so a
    // missing entry is the ordinary case, not a broken invariant:
    // NodeEdit mode on a multi-section node rebuilds constantly
    // while nothing is being dragged. This asserted an invariant
    // that does not exist and warned on every such rebuild —
    // `warn!` survives into release (CODE_CONVENTIONS §9), so the
    // message reached users' stderr at rebuild rate. Absent means
    // undragged, which is what the other twelve consulting sites
    // read it as.
    let (ox, oy) = offsets.get(active_id).copied().unwrap_or((0.0, 0.0));
    let pos = node.pos_vec2();
    let size = node.size_vec2();
    // Active-node AABB sanity: NaN or non-positive size produces
    // degenerate frames (3-cluster runs from `border_run_specs`
    // when `size.x / (font_size * 0.6)` is NaN or 0). Bail out
    // cleanly — the same defense-in-depth `renderable_section`
    // applies per section. Pre-fix the per-section guard
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

    // The active-affordance signal (cyan SELECTION_HIGHLIGHT_HEX) sits
    // at the bottom of the cascade — authors who set
    // `frame_border.color` on their override fully replace it,
    // which is the desired shape for "make my borders tell a
    // story." Authors who want the cyan default just leave color
    // unset on their config.
    let frame_color_resolved = resolve_var(SELECTION_HIGHLIGHT_HEX, &map.canvas.theme_variables);

    // Hoist preview-target match out of the per-section loop. Most
    // rebuilds run with `border_preview = None` and we want the
    // steady-state per-section iteration to be one `is_none()`
    // check per branch.
    let preview_section_targets: Option<&[(String, usize)]> = border_preview.and_then(|p| match p.target {
        BorderPreviewTargetRef::Sections(ts) => Some(ts),
        _ => None,
    });
    let preview_canvas_unfocused: Option<BorderConfigEditsView<'_>> =
        border_preview.and_then(|p| match p.target {
            BorderPreviewTargetRef::CanvasSectionFrame => Some(p.edits),
            _ => None,
        });
    let preview_canvas_focused: Option<BorderConfigEditsView<'_>> =
        border_preview.and_then(|p| match p.target {
            BorderPreviewTargetRef::CanvasSectionFrameFocused => Some(p.edits),
            _ => None,
        });
    // Pre-clone the canvas defaults ONLY when a canvas-targeted
    // preview is active — steady-state keeps the clone-free
    // borrow into `map.canvas`. §B7: pre-fix this allocated two
    // `Option<GlyphBorderConfig>` per call regardless of preview
    // state.
    let canvas_unfocused_owned: Option<Option<GlyphBorderConfig>> = preview_canvas_unfocused.map(|view| {
        let mut slot = map.canvas.default_section_frame_border.clone();
        crate::mindmap::border::apply_view_to_slot(&mut slot, &view);
        slot
    });
    let canvas_focused_owned: Option<Option<GlyphBorderConfig>> = preview_canvas_focused.map(|view| {
        let mut slot = map.canvas.default_focused_section_frame_border.clone();
        crate::mindmap::border::apply_view_to_slot(&mut slot, &view);
        slot
    });
    let canvas_unfocused_default: Option<&GlyphBorderConfig> = match &canvas_unfocused_owned {
        Some(opt) => opt.as_ref(),
        None => map.canvas.default_section_frame_border.as_ref(),
    };
    let canvas_focused_default: Option<&GlyphBorderConfig> = match &canvas_focused_owned {
        Some(opt) => opt.as_ref(),
        None => map.canvas.default_focused_section_frame_border.as_ref(),
    };

    let mut out: Vec<SectionFrameElement> = Vec::with_capacity(node.sections.len());
    for (section_idx, section) in node.sections.iter().enumerate() {
        // One predicate, shared with the node pass — a frame must
        // outline exactly the sections that render.
        if !renderable_section(section) {
            continue;
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
        let section_owned_for_preview: Option<Option<GlyphBorderConfig>> = if section_targeted {
            let view = border_preview
                .map(|p| p.edits)
                .expect("section_targeted implies preview is Some");
            let mut slot = section.frame_border.clone();
            crate::mindmap::border::apply_view_to_slot(&mut slot, &view);
            Some(slot)
        } else {
            None
        };
        let section_slot_ref: Option<&GlyphBorderConfig> = match &section_owned_for_preview {
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
    section_slot: Option<&GlyphBorderConfig>,
    canvas_unfocused_default: Option<&GlyphBorderConfig>,
    canvas_focused_default: Option<&GlyphBorderConfig>,
    focused: bool,
    frame_color_fallback: &str,
) -> BorderStyle {
    if let Some(cfg) = section_slot {
        return crate::mindmap::border::resolve_border_style(Some(cfg), None, None, frame_color_fallback);
    }
    let canvas_chosen = if focused {
        canvas_focused_default.or(canvas_unfocused_default)
    } else {
        canvas_unfocused_default
    };
    if let Some(cfg) = canvas_chosen {
        return crate::mindmap::border::resolve_border_style(Some(cfg), None, None, frame_color_fallback);
    }
    // Floor — same shape `resolve_section_frame_border` synthesizes.
    let floor = crate::mindmap::border::section_frame_floor_config(focused);
    crate::mindmap::border::resolve_border_style(Some(floor), None, None, frame_color_fallback)
}

/// Content signature for a section-frame element list — every
/// field the tree projection reads, hashed. Compared by
/// `CanvasFrame::update_section_frame_tree` to short-circuit
/// redundant rebuilds.
///
/// Captures every **input** to the rendered glyph runs: the id
/// triple (node_id, section_idx, focused), the resolved
/// [`BorderStyle`] (through
/// [`BorderStyle::hash_content`](crate::mindmap::border::BorderStyle::hash_content),
/// which destructures it), the position + bounds (so a node move
/// while in NodeEdit re-registers the frames), and the resolved
/// palette cycle (so an authored palette edit triggers a rebuild).
///
/// Section frames have no in-place mutator path, so this one
/// signature does both jobs the border and portal roles split
/// between a structural and a content signature: it decides
/// rebuild-or-skip outright.
///
/// Pre-fix the function materialized a `Vec` of per-frame
/// identity structs (a type since deleted) whose only consumer
/// was `hash_canvas_signature` — N×String
/// clones + a 7-field clone-Vec per call, all thrown away after
/// the hash compare. Combined with the `InPlaceMutator` early-
/// return in `update_section_frame_tree`, the signature path runs
/// on every NodeEdit-mode rebuild *to decide whether to skip*, so
/// the allocations were paying for nothing. Post-fix the function
/// streams the same fields directly into the hasher (no Vec, no
/// clones — `&str` borrows for String fields, `to_bits()` for
/// floats).
///
/// Combined with the `InPlaceMutator` early-return in
/// `update_section_frame_tree`, the completeness of this signature
/// is load-bearing: a missed delta means the dispatch declares
/// "no work needed" and the screen keeps stale glyphs. The body
/// destructures [`SectionFrameElement`], so a new field does not
/// compile until it is hashed here.
pub fn section_frame_content_signature(elements: &[SectionFrameElement]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    elements.len().hash(&mut h);
    for e in elements {
        let SectionFrameElement {
            node_id,
            section_idx,
            position,
            size,
            border_style,
            palette_cycle,
            focused,
        } = e;
        node_id.as_str().hash(&mut h);
        section_idx.hash(&mut h);
        focused.hash(&mut h);
        position.0.to_bits().hash(&mut h);
        position.1.to_bits().hash(&mut h);
        size.0.to_bits().hash(&mut h);
        size.1.to_bits().hash(&mut h);
        border_style.hash_content(&mut h);
        palette_cycle.len().hash(&mut h);
        for cycle in palette_cycle {
            cycle.map(f32::to_bits).hash(&mut h);
        }
    }
    h.finish()
}

/// Build a `Tree<GfxElement, GfxMutator>` from a slice of
/// [`SectionFrameElement`]s. Each element's resolved
/// [`crate::mindmap::border::BorderStyle`] is fed to
/// [`crate::mindmap::border::border_run_specs`] for the
/// eight-run geometry; the runs are appended via the same
/// `append_border_run` helper node borders use, so palette
/// cycling, multi-cluster fills, custom corners, and any future
/// border feature lights up on section frames automatically.
///
/// Empty input → empty tree (one void root, no children). The
/// caller (`scene_rebuild`) gates this against
/// `InteractionMode::NodeEdit` so non-NodeEdit rebuilds produce
/// a trivial tree.
pub fn build_section_frame_tree(elements: &[SectionFrameElement]) -> Tree<GfxElement, GfxMutator> {
    let mut tree: Tree<GfxElement, GfxMutator> = Tree::new_non_indexed();
    let mut unique_id: usize = 1;
    for (idx, frame) in elements.iter().enumerate() {
        let parent_channel = idx + 1;
        let parent_id = tree.root.append_value(
            GfxElement::new_void_with_id(parent_channel, unique_id),
            &mut tree.arena,
        );
        unique_id += 1;

        append_frame_runs(&mut tree, parent_id, frame, &mut unique_id);
    }
    tree
}

/// Layout the eight glyph runs for one section frame and
/// append them under `parent_id`. Delegates to
/// [`crate::mindmap::border::border_run_specs`] for the run
/// geometry and to
/// [`super::border::append_border_run`] for the per-run
/// `GlyphArea` construction. Section frames inherit every layout
/// detail node borders have — corner overlap, character-width
/// approximation, palette-offset sweep around the rectangle —
/// for free.
fn append_frame_runs(
    tree: &mut Tree<GfxElement, GfxMutator>,
    parent_id: NodeId,
    frame: &SectionFrameElement,
    unique_id: &mut usize,
) {
    let specs = crate::mindmap::border::border_run_specs(&frame.border_style, frame.position, frame.size);
    let color_rgba = hex_to_rgba_safe(&frame.border_style.color, [1.0, 1.0, 1.0, 1.0]);
    // Frames inherit the active node's zoom window implicitly —
    // they only render while the active node is visible (the
    // outer dispatch gates emission on `is_hidden_by_fold`), so a
    // permissive zoom window is the right default. A future
    // refinement could pin the frame's window to the node's, but
    // that's redundant with the emission gate today.
    let zoom_visibility = crate::gfx_structs::zoom_visibility::ZoomVisibility::default();
    for spec in &specs {
        super::border::append_border_run(
            tree,
            parent_id,
            spec.channel,
            *unique_id,
            &spec.text,
            spec.font_size_pt,
            spec.line_height_pt,
            spec.position,
            spec.bounds,
            color_rgba,
            zoom_visibility,
            &frame.palette_cycle,
            spec.palette_offset,
            spec.cluster_count,
        );
        *unique_id += 1;
    }
}
