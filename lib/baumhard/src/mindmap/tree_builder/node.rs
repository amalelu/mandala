// SPDX-License-Identifier: MPL-2.0

//! Node-tree helpers — project a `MindNode` and its
//! [`MindSection`]s into a three-deep `GfxElement` subtree:
//!
//! ```text
//! Tree
//! └── GlyphArea (node container — chrome only, no glyphs)
//!     ├── GlyphArea (section 0; carries text + regions)
//!     │   └── GlyphModel (section 0 model; structural seam for
//!     │                   per-component mutations)
//!     ├── GlyphArea (section 1)
//!     │   └── GlyphModel (section 1 model)
//!     └── GlyphArea (child mind-node, recursive)
//!         └── …
//! ```
//!
//! The container area owns the per-node visual chrome
//! (background fill, frame padding, shape, zoom window). The
//! section-areas are the text-bearing surfaces — the renderer's
//! tree walker (`renderer/tree_walker.rs`) iterates every
//! `GlyphArea` descendant and shapes each one's text into a
//! `cosmic_text::Buffer`, so sections become separate buffers
//! keyed by their `unique_id` with no special-case in the renderer.
//! The section-model is a `GfxElement::GlyphModel` child the
//! renderer skips today; it is a *named seam* for future
//! per-component / per-grapheme mutation work that wants to reach
//! into a section without rebuilding the arena (matches the
//! existing color-picker overlay pattern in
//! `src/application/color_picker_overlay/glyph_model.rs`).

use std::collections::HashMap;

use indextree::NodeId;

use crate::core::primitives::{ColorFontRegion, ColorFontRegions, Flag, Flaggable, Range};
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::model::{GlyphComponent, GlyphLine, GlyphModel};
use crate::gfx_structs::mutator::GfxMutator;
use crate::gfx_structs::shape::NodeShape;
use crate::gfx_structs::tree::Tree;
use crate::mindmap::border::{resolve_border_style, BORDER_APPROX_CHAR_WIDTH_FRAC};
use crate::mindmap::model::{ChildIndex, MindMap, MindNode, MindSection};
use crate::util::color::{self, Color as BaumhardColor};
use glam::Vec2;

/// Nominal font scale, in points, for a mindmap `GlyphArea` with
/// no `text_runs` to take a size from — the historical
/// `cosmic_text` fall-through this builder has always used.
///
/// Two areas land here. A **section** with no runs is measured
/// and laid out at this scale. A **node container area** carries
/// it as its nominal scale but renders no glyphs of its own
/// (sections do), so there it keeps the area's scale field
/// well-defined rather than sizing anything drawn.
///
/// This is the **renderer's** fallback, deliberately distinct from
/// the *authoring* default a newly-created run gets (24pt, in the
/// app crate's `document::defaults`). A run-less legacy section
/// keeps rendering at 14pt; the moment something authors a run
/// onto it, the authoring default applies instead. Named so the
/// reverse converter
/// (`document::custom::sync::DEFAULT_TEXT_RUN_SIZE_PT`) can pin
/// itself to the forward path's number rather than repeating the
/// literal and drifting from it.
pub const DEFAULT_SECTION_FONT_SCALE: f32 = 14.0;

/// Build the *container* `GlyphArea` for a mind node — the chrome-
/// bearing area that owns background fill, border padding, shape,
/// and zoom window, but renders no glyphs of its own (sections do).
///
/// Empty `text` and empty `regions`: the renderer's `walk_tree_into_buffers`
/// short-circuits for empty-text areas after yielding the
/// background rect, so the container contributes one fill quad
/// and zero shaped buffers — the historical visual cost of an
/// untextured node.
///
/// `canvas_default_border` cascades into `background_padding` so
/// the fill extends out to the surrounding border glyphs (drawn
/// by the per-role border subtree). Same math as before the
/// section refactor; only the *target* of the math moved from
/// the text-bearing area to the chrome-only container.
pub(super) fn mindnode_container_area(
    node: &MindNode,
    vars: &HashMap<String, String>,
    canvas_default_border: Option<&crate::mindmap::model::GlyphBorderConfig>,
) -> GlyphArea {
    // Container metrics: scale and line_height are nominal — no
    // glyphs render here, but the area still needs valid metrics
    // so the subtree-AABB cache stays well-defined.
    let position = node.pos_vec2();
    let bounds = node.size_vec2();
    let mut area = GlyphArea::new(
        DEFAULT_SECTION_FONT_SCALE,
        DEFAULT_SECTION_FONT_SCALE * 1.2,
        position,
        bounds,
    );

    // `background_padding` math — see `mindmap/border.rs` for the
    // derivation. Same shape as pre-section nodes; the container
    // is the natural carrier because a section sits *inside* the
    // node AABB and never touches the surrounding border.
    if node.style.show_frame && NodeShape::from_style_string(&node.style.shape) == NodeShape::Rectangle {
        let frame_color_resolved = color::resolve_var(&node.style.frame_color, vars);
        let border_style = resolve_border_style(
            node.style.border.as_ref(),
            canvas_default_border,
            frame_color_resolved,
        );
        let fs = border_style.font_size_pt;
        let acw = fs * BORDER_APPROX_CHAR_WIDTH_FRAC;
        let corner_overlap = fs * crate::mindmap::border::BORDER_CORNER_OVERLAP_FRAC;
        let nw = node.size_vec2().x;
        let char_count = ((nw / acw) + 2.0).ceil().max(3.0);
        let pad_top_bottom = 0.5 * fs - corner_overlap;
        let pad_left = 0.5 * acw;
        let pad_right = char_count * acw - 1.5 * acw - nw;
        area.background_padding =
            crate::gfx_structs::area::EdgePadding::new(pad_top_bottom, pad_right, pad_top_bottom, pad_left);
    }

    // Background-color resolution — same trade-off as before:
    // empty / parse-fail / fully-transparent → `None` (canvas
    // shows through); otherwise pack as u8 RGBA.
    area.background_color = {
        let raw = &node.style.background_color;
        if raw.is_empty() {
            None
        } else {
            let resolved = color::resolve_var(raw, vars);
            let rgba = color::hex_to_rgba_safe(resolved, [0.0, 0.0, 0.0, 0.0]);
            if rgba[3] <= 0.0 {
                None
            } else {
                Some(color::convert_f32_to_u8(&rgba))
            }
        }
    };

    area.shape = NodeShape::from_style_string(&node.style.shape);
    area.zoom_visibility = node.zoom_window();
    area
}

/// Build a section-area `GlyphArea` for one [`MindSection`].
/// Carries the section's text and its theme-resolved
/// `ColorFontRegions`; the renderer's tree walker shapes this
/// directly into a cosmic-text buffer keyed by the area's
/// `unique_id`. Inherits the owning node's zoom window so a
/// section never outlives its node at any zoom level.
pub(super) fn mindnode_section_area(
    node: &MindNode,
    section: &MindSection,
    vars: &HashMap<String, String>,
) -> GlyphArea {
    // Effective scale: pick the *largest* run size so a multi-run
    // section with a small first run and a 96pt later run gets a
    // line-height tall enough to keep the larger glyphs from
    // clipping. Falls through to [`DEFAULT_SECTION_FONT_SCALE`]
    // when the section has no runs. The single-
    // section default-migration shape (one run spanning all of
    // `text`) round-trips with the pre-section behavior because
    // there's only one size to pick. Mirrors the same `max`
    // posture in `grow_one_node_to_fit_text`.
    let scale_max = section
        .text_runs
        .iter()
        .map(|r| r.size_pt as f32)
        .fold(0.0_f32, f32::max);
    let scale = if scale_max > 0.0 {
        scale_max
    } else {
        DEFAULT_SECTION_FONT_SCALE
    };
    let line_height = scale * 1.2;
    let position = node.pos_vec2() + Vec2::new(section.offset.x as f32, section.offset.y as f32);
    let bounds = section
        .size
        .as_ref()
        .map(|s| Vec2::new(s.width as f32, s.height as f32))
        .unwrap_or_else(|| node.size_vec2());

    let mut area = GlyphArea::new_with_str(&section.text, scale, line_height, position, bounds);

    // Section-areas inherit the owning node's zoom window —
    // they belong to the node and shouldn't outlive it at any
    // zoom level.
    area.zoom_visibility = node.zoom_window();

    // Resolve text-runs into a `ColorFontRegions`. Per-run
    // `color` cascades through theme variables; per-run `font`
    // resolves through `app_font_by_family`. Empty / unknown
    // family resolves to `None` (cosmic-text picks; warns at
    // attrs-build time).
    //
    // A section without runs keeps its `regions` empty, which the
    // renderer interprets as "use defaults" — `node.style.text_color`
    // never enters the region table.
    let mut regions = ColorFontRegions::new_empty();
    for run in &section.text_runs {
        let resolved = color::resolve_var(&run.color, vars);
        let rgba = color::hex_to_rgba_safe(resolved, [0.0, 0.0, 0.0, 1.0]);
        let font = if run.font.is_empty() {
            None
        } else {
            crate::font::fonts::app_font_by_family(&run.font)
        };
        regions.submit_region(ColorFontRegion::new(
            Range::new(run.start, run.end),
            font,
            Some(rgba),
        ));
    }
    area.regions = regions;
    area
}

/// Build a structural `GlyphModel` mirroring a section's text +
/// dominant style — present in the tree as a future-mutation seam
/// (matches the picker overlay pattern in
/// `src/application/color_picker_overlay/glyph_model.rs`). The
/// renderer's `walk_tree_into_buffers` skips `GlyphModel` /
/// `Void` variants, so this node has zero per-frame cost; it
/// exists so per-component / per-grapheme mutators can target
/// inside a section without rebuilding the arena.
pub(super) fn mindnode_section_model(section: &MindSection, area: &GlyphArea) -> GlyphModel {
    use crate::font::fonts::AppFont;

    let mut model = GlyphModel::new();
    model.position = area.position;

    if section.text.is_empty() {
        return model;
    }

    // Same dominant-style trick as the picker overlay: read the
    // first region's font + color as the model's effective
    // styling. Sections without runs fall through to
    // `(Any, black)`, mirroring cosmic-text's defaults — the
    // structural model is conservative; per-component refinement
    // is the user's job once the seam is wired.
    let regions = area.regions.all_regions();
    let (font, color) = match regions.first() {
        Some(r) => {
            let font = r.font.unwrap_or(AppFont::Any);
            let color = r
                .color
                .map(|fc| BaumhardColor::new_f32(&fc))
                .unwrap_or_else(BaumhardColor::black);
            (font, color)
        }
        None => (AppFont::Any, BaumhardColor::black()),
    };

    model.add_line(GlyphLine::new_with(GlyphComponent::text(
        &section.text,
        font,
        color,
    )));
    model
}

/// Whether a section has an on-screen surface at all.
///
/// A non-finite offset or a non-finite / non-positive explicit size
/// produces a degenerate or NaN AABB. Emitting one poisons the
/// subtree-AABB cache, hands cosmic-text a zero-area or NaN buffer,
/// and gives hit-testing a rectangle that can never be hit — so the
/// section is skipped entirely and `maptool verify` reports it to
/// the author instead.
///
/// Shared with [`super::build_section_frames`], which must frame
/// exactly the sections that render: the two would otherwise agree
/// only by both open-coding the same four comparisons, which is how
/// they drifted apart in the first place. (The clip-AABB pass is
/// node-level and has no section logic, so it is not a consumer.)
///
/// Note this is *not* an empty-text check: a section with no text
/// still owns a real rectangle (it is where the user's next
/// keystroke lands), so it keeps its area and its `section_map`
/// entry.
pub(super) fn renderable_section(section: &MindSection) -> bool {
    if !section.offset.x.is_finite() || !section.offset.y.is_finite() {
        return false;
    }
    match section.size.as_ref() {
        Some(sz) => sz.width.is_finite() && sz.height.is_finite() && sz.width > 0.0 && sz.height > 0.0,
        None => true,
    }
}

/// Append the section subtree (one `GlyphArea` + one `GlyphModel`
/// per renderable [`MindSection`] — see [`renderable_section`])
/// under `parent_node_id` and record the
/// section-area's `NodeId` in `section_map`. Each section element
/// carries `Flag::SectionRoot` so click-routing and per-section
/// scene rebuild can discriminate them from sibling child mind-
/// node-areas in the same tree.
///
/// The section-area's `channel` is the section's authored channel
/// (defaulting to its index in `MindNode.sections`), so per-section
/// custom mutations targeting `Children` pair up by channel inside
/// the parent node-area. Channel collisions with sibling child
/// mind-nodes are accepted as a known authoring footgun — see the
/// `Predicate::IsSection` / `TargetScope::SectionsOnly` named-seam
/// note in CONCEPTS.md.
pub(super) fn append_node_sections(
    node: &MindNode,
    parent_node_id: NodeId,
    vars: &HashMap<String, String>,
    tree: &mut Tree<GfxElement, GfxMutator>,
    section_map: &mut HashMap<(String, usize), NodeId>,
    id_counter: &mut usize,
) {
    for (section_idx, section) in node.sections.iter().enumerate() {
        if !renderable_section(section) {
            continue;
        }
        // Effective channel: use the authored value when the
        // user explicitly set one (`Some(_)`); otherwise default
        // to the section's index. The `Option<usize>` shape
        // distinguishes "author wrote `0` explicitly" from
        // "default" — pre-`Option` migration silently overrode
        // explicit 0 for sections at idx > 0, which the author
        // had no way to override.
        let channel = section.channel.unwrap_or(section_idx);

        let section_area = mindnode_section_area(node, section, vars);
        let section_model = mindnode_section_model(section, &section_area);

        let mut section_element =
            GfxElement::new_area_non_indexed_with_id(section_area, channel, *id_counter);
        section_element.set_flag(Flag::SectionRoot);
        *id_counter += 1;

        let section_id = tree.arena.new_node(section_element);
        parent_node_id.append(section_id, &mut tree.arena);
        section_map.insert((node.id.clone(), section_idx), section_id);

        let mut model_element =
            GfxElement::new_model_non_indexed_with_id(section_model, channel, *id_counter);
        // The model inherits `SectionRoot` so a flag-based
        // descent walker can climb from "this is the model" to
        // "this is a section element" without re-checking the
        // arena edge.
        model_element.set_flag(Flag::SectionRoot);
        *id_counter += 1;
        let model_node_id = tree.arena.new_node(model_element);
        section_id.append(model_node_id, &mut tree.arena);
    }
}

/// Recursive child walker — for each non-folded child mind-node
/// of `parent_mind_id`, append the container area, then its
/// section subtree, then recurse into its own children. Keeps the
/// container, sections, and child mind-nodes as a flat sibling
/// list under the parent container — same shape as the
/// pre-section tree, just with extra section siblings.
///
/// `parent_folded` is `true` when an ancestor (including the
/// immediate parent) is folded. Children of a folded node are
/// hidden by construction, so the recursive fold check from the
/// old `is_hidden_by_fold` path is redundant here.
// `clippy::too_many_arguments`: a recursive arena walk threading
// four out-parameters (`tree`, `node_map`, `section_map`,
// `id_counter`) plus the read-only `(map, index, parent)` triple.
// Bundling the out-params into a struct would just add a borrow
// indirection on every recursion step.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_children_recursive(
    map: &MindMap,
    index: &ChildIndex<'_>,
    parent_mind_id: &str,
    parent_folded: bool,
    parent_node_id: NodeId,
    tree: &mut Tree<GfxElement, GfxMutator>,
    node_map: &mut HashMap<String, NodeId>,
    section_map: &mut HashMap<(String, usize), NodeId>,
    id_counter: &mut usize,
) {
    let vars = &map.canvas.theme_variables;
    let canvas_default_border = map.canvas.default_border.as_ref();
    let children = index.children_of(parent_mind_id);
    for child in children {
        if parent_folded {
            continue;
        }
        let area = mindnode_container_area(child, vars, canvas_default_border);
        let element = GfxElement::new_area_non_indexed_with_id(area, child.channel, *id_counter);
        *id_counter += 1;

        let child_node_id = tree.arena.new_node(element);
        parent_node_id.append(child_node_id, &mut tree.arena);
        node_map.insert(child.id.clone(), child_node_id);

        append_node_sections(child, child_node_id, vars, tree, section_map, id_counter);

        build_children_recursive(
            map,
            index,
            &child.id,
            child.folded,
            child_node_id,
            tree,
            node_map,
            section_map,
            id_counter,
        );
    }
}
