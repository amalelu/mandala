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
//!     └── GlyphArea (child mind-node, nested the same way)
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
use crate::util::grapheme_chad;
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
/// onto it, the authoring default applies instead.
///
/// Defined as the model's
/// [`DEFAULT_TEXT_RUN_SIZE_PT`](crate::mindmap::model::DEFAULT_TEXT_RUN_SIZE_PT)
/// rather than repeating the literal: a run that omits `size_pt`
/// deserializes to that constant, so "section with no runs" and
/// "run that names no size" cannot drift apart — both render at
/// exactly this scale by construction.
pub const DEFAULT_SECTION_FONT_SCALE: f32 = crate::mindmap::model::DEFAULT_TEXT_RUN_SIZE_PT;

/// Ratio of a section's line height to its font scale. Every
/// `GlyphArea` this builder emits gets `line_height = scale *
/// LINE_HEIGHT_FACTOR`, and the app's auto-size measurement has to
/// use the same ratio or a node grown to fit its text clips the
/// text it was grown for.
///
/// 1.2 is the typographic default cosmic-text and CSS both take for
/// unspecified leading. There is no per-section line-height field in
/// the model, so this is not a fallback — it is the whole answer,
/// and the reverse converter in the app's
/// `document::custom::sync` relies on that to avoid persisting a
/// line height at all.
pub const LINE_HEIGHT_FACTOR: f32 = 1.2;

/// The font scale a section is laid out at: the **largest**
/// `size_pt` among its `text_runs`, or [`DEFAULT_SECTION_FONT_SCALE`]
/// when it has none.
///
/// Largest rather than first so a multi-run section with a small
/// opening run and a 96pt later run gets a line height tall enough
/// to keep the larger glyphs from clipping. The single-section
/// default-migration shape (one run spanning all of `text`)
/// round-trips with the pre-section behavior because there is only
/// one size to pick.
///
/// This is the one answer to "what size is this section on screen
/// right now?", and three call sites need it: this builder, which
/// lays the section out; the app's auto-size measurement, which has
/// to measure what the builder will draw; and the app's reverse
/// converter, which reads the pre-mutation scale back out to
/// distribute a font-size delta across the runs. Open-coded in all
/// three, they were free to disagree about the run-less case.
///
/// **Cost.** O(runs) — one fold over `section.text_runs`, no heap.
pub fn effective_section_scale(section: &MindSection) -> f32 {
    let largest = section
        .text_runs
        .iter()
        .map(|r| r.size_pt)
        .fold(0.0_f32, f32::max);
    if largest > 0.0 {
        largest
    } else {
        DEFAULT_SECTION_FONT_SCALE
    }
}

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
/// The canvas default border cascades into `background_padding` so
/// the fill extends out to the surrounding border glyphs (drawn
/// by the per-role border subtree). Same math as before the
/// section refactor; only the *target* of the math moved from
/// the text-bearing area to the chrome-only container.
///
/// Takes the whole `map` rather than the two canvas fields it
/// reads because fill and frame both resolve through the palette
/// cascade ([`MindMap::node_background_color`],
/// [`MindMap::node_frame_color`]), which needs `map.palettes`.
pub(super) fn mindnode_container_area(map: &MindMap, node: &MindNode) -> GlyphArea {
    let vars = &map.canvas.theme_variables;
    let canvas_default_border = map.canvas.default_border.as_ref();
    // Container metrics: scale and line_height are nominal — no
    // glyphs render here, but the area still needs valid metrics
    // so the subtree-AABB cache stays well-defined.
    let position = node.pos_vec2();
    let bounds = node.size_vec2();
    let mut area = GlyphArea::new(
        DEFAULT_SECTION_FONT_SCALE,
        DEFAULT_SECTION_FONT_SCALE * LINE_HEIGHT_FACTOR,
        position,
        bounds,
    );

    // Parsed once and read twice below. The two reads used to parse
    // separately, which on an unrecognized spelling meant the same
    // node reported the same typo twice from inside one function.
    // The motive is the duplicate log line, not cost: the surviving
    // call is `ShapeSpelling::classify`, which walks more spellings
    // than the chain it replaced, and no claim either way is made or
    // measurable here (CLAUDE.md §7).
    let shape = NodeShape::from_style_string(&node.style.shape);

    // `background_padding` math — see `mindmap/border.rs` for the
    // derivation. Same shape as pre-section nodes; the container
    // is the natural carrier because a section sits *inside* the
    // node AABB and never touches the surrounding border.
    if node.style.show_frame && shape == NodeShape::Rectangle {
        let themed_frame = map
            .node_frame_theme_tier(node)
            .map(|c| color::resolve_var(c, vars));
        let style_frame = color::resolve_var(&node.style.frame_color, vars);
        let border_style = resolve_border_style(
            node.style.border.as_ref(),
            canvas_default_border,
            themed_frame,
            style_frame,
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
        let raw = map.node_background_color(node);
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

    area.shape = shape;
    area.zoom_visibility = node.zoom_window();
    area
}

/// Build a section-area `GlyphArea` for one [`MindSection`].
/// Carries the section's text and its theme-resolved
/// `ColorFontRegions`; the renderer's tree walker shapes this
/// directly into a cosmic-text buffer keyed by the area's
/// `unique_id`. Inherits the owning node's zoom window so a
/// section never outlives its node at any zoom level.
///
/// `section_idx` is the section's position in
/// [`MindNode::sections`] — index 0 is the node's title stratum,
/// the only one the palette's `title` channel reaches.
pub(super) fn mindnode_section_area(
    map: &MindMap,
    node: &MindNode,
    section: &MindSection,
    section_idx: usize,
) -> GlyphArea {
    let vars = &map.canvas.theme_variables;
    let scale = effective_section_scale(section);
    let line_height = scale * LINE_HEIGHT_FACTOR;
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
    // A run that names no color of its own inherits the node's
    // section-level default — the palette group's `text` when the
    // node is themed, `style.text_color` otherwise. That is the
    // contract `MindSection` and `TextRun` have always documented;
    // before the palette cascade was wired it was `hex_to_rgba_safe`
    // that answered, and it answered black.
    let node_text_rgba = map.node_text_rgba(node);
    let mut regions = ColorFontRegions::new_empty();
    for run in &section.text_runs {
        let rgba = if run.color.is_empty() {
            node_text_rgba
        } else {
            let resolved = color::resolve_var(&run.color, vars);
            color::hex_to_rgba_safe(resolved, [0.0, 0.0, 0.0, 1.0])
        };
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
    if section.text_runs.is_empty() {
        for region in section_default_regions(map, node, section, section_idx) {
            regions.submit_region(region);
        }
    }
    area.regions = regions;
    area
}

/// The region table a section with **no** `text_runs` gets: the
/// node's section-level color defaults, made explicit so they reach
/// the renderer.
///
/// One region normally, two when this is the node's title stratum
/// (`section_idx == 0`) and the palette gives it a distinct `title`
/// color: `[0, first_line_end)` in the title color and the rest in
/// the text color. "First line" is the first hard newline in the
/// section's own text — the only line boundary that exists before
/// cosmic-text has wrapped anything, and the one the miMind
/// title/body split came in on.
///
/// **Only the run-less case**, which is why the caller checks and
/// this function does not. A section that carries runs has an
/// authored coverage map, and `rich_text_spans_from_regions` renders
/// exactly the covered clusters: filling its gaps would make text
/// appear that the author's runs deliberately left out.
///
/// `pub` because it is half of a round trip. The document layer's
/// reverse sync
/// (`application::document::custom::sync`) has to know what the
/// forward path *would* have produced for a run-less section, so it
/// can tell "the mutation recolored this section" from "this is the
/// default the builder always emits". Projecting the model through
/// this same function is what keeps the two ends from drifting into
/// synthesizing a phantom `TextRun` on every apply.
///
/// Cost: one grapheme walk of the section text (for the cluster
/// count, plus the newline's cluster index when the title channel
/// applies), two color parses. Returns at most two regions.
pub fn section_default_regions(
    map: &MindMap,
    node: &MindNode,
    section: &MindSection,
    section_idx: usize,
) -> Vec<ColorFontRegion> {
    let clusters = grapheme_chad::count_grapheme_clusters(&section.text);
    if clusters == 0 {
        return Vec::new();
    }
    let text_rgba = map.node_text_rgba(node);
    let title_split = if section_idx == 0 {
        let vars = &map.canvas.theme_variables;
        let title = color::resolve_var(map.node_title_color(node), vars);
        let title_rgba = color::hex_to_rgba_safe(title, text_rgba);
        // Nothing to split when the title color is the text color —
        // one region says the same thing with less work.
        if title_rgba == text_rgba {
            None
        } else {
            first_line_cluster_end(&section.text, clusters).map(|end| (end, title_rgba))
        }
    } else {
        None
    };
    match title_split {
        Some((end, title_rgba)) => vec![
            ColorFontRegion::new(Range::new(0, end), None, Some(title_rgba)),
            ColorFontRegion::new(Range::new(end, clusters), None, Some(text_rgba)),
        ],
        None => vec![ColorFontRegion::new(
            Range::new(0, clusters),
            None,
            Some(text_rgba),
        )],
    }
}

/// Grapheme-cluster index just past the first line of `text` — the
/// index of the first `\n`, counted in clusters.
///
/// `None` when the text has no newline, or when the newline is the
/// last cluster: in both cases the "first line" is the whole
/// section and a two-region split would emit an empty tail.
///
/// Cost: one grapheme walk over `text`.
fn first_line_cluster_end(text: &str, clusters: usize) -> Option<usize> {
    let byte = text.find('\n')?;
    let end = grapheme_chad::count_grapheme_clusters(&text[..byte]);
    if end == 0 || end >= clusters {
        None
    } else {
        Some(end)
    }
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
    map: &MindMap,
    node: &MindNode,
    parent_node_id: NodeId,
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

        let section_area = mindnode_section_area(map, node, section, section_idx);
        let section_model = mindnode_section_model(section, &section_area);

        let mut section_element =
            GfxElement::new_area_non_indexed_with_id(section_area, channel, *id_counter);
        section_element.set_flag(Flag::SectionRoot);
        *id_counter += 1;

        let section_id = parent_node_id.append_value(section_element, &mut tree.arena);
        section_map.insert((node.id.clone(), section_idx), section_id);

        let mut model_element =
            GfxElement::new_model_non_indexed_with_id(section_model, channel, *id_counter);
        // The model inherits `SectionRoot` so a flag-based
        // descent walker can climb from "this is the model" to
        // "this is a section element" without re-checking the
        // arena edge.
        model_element.set_flag(Flag::SectionRoot);
        *id_counter += 1;
        section_id.append_value(model_element, &mut tree.arena);
    }
}

/// Descendant walker — for every visible mind-node under
/// `parent_mind_id`, append the container area then its section
/// subtree. Keeps the container, sections, and child mind-nodes as
/// a flat sibling list under the parent container — same shape as
/// the pre-section tree, just with extra section siblings.
///
/// `parent_folded` is `true` when an ancestor (including the
/// immediate parent) is folded. A folded node's subtree is hidden
/// by construction, so it is never descended into — which is also
/// why the fold check from the old `is_hidden_by_fold` path is
/// redundant here.
///
/// **The descent is iterative, and that is load-bearing.** This
/// walker's depth is `parent_id` nesting depth, authored in a
/// `.mindmap.json` — untrusted input, where a linear chain of N
/// nodes is a legal acyclic tree the loader's cycle check accepts.
/// The recursive form overflowed the thread stack on such a map
/// and took the process down with `SIGABRT`: not a panic, so there
/// was no frame to degrade and nothing to log
/// (CODE_CONVENTIONS §9). Depth now costs heap. The scene rebuild
/// runs this on every structural change, so a hostile map that
/// merely *opens* reached it.
///
/// Nodes are visited in the same pre-order the recursive form
/// produced — children are pushed in reverse so the lowest-sorted
/// sibling pops first. That order is not cosmetic: it fixes arena
/// sibling order and the `unique_id` values handed out from
/// `id_counter`, both of which the renderer and the scene cache
/// key on.
///
/// Cost: O(visible descendants). One heap vector holding the
/// frontier — the sum of the unprocessed sibling rows along the
/// current path — **one element for a linear chain**, since each
/// node's only child replaces it, O(n) for a shallow wide tree, and
/// O(depth x branching) in general.
// `clippy::too_many_arguments`: an arena walk threading four
// out-parameters (`tree`, `node_map`, `section_map`, `id_counter`)
// plus the read-only `(map, index, parent)` triple. Bundling the
// out-params into a struct would just add a borrow indirection.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_descendants<'a>(
    map: &MindMap,
    index: &ChildIndex<'a>,
    parent_mind_id: &str,
    parent_folded: bool,
    parent_node_id: NodeId,
    tree: &mut Tree<GfxElement, GfxMutator>,
    node_map: &mut HashMap<String, NodeId>,
    section_map: &mut HashMap<(String, usize), NodeId>,
    id_counter: &mut usize,
) {
    if parent_folded {
        return;
    }
    let mut pending: Vec<(&'a MindNode, NodeId)> = index
        .children_of(parent_mind_id)
        .iter()
        .rev()
        .map(|child| (*child, parent_node_id))
        .collect();

    while let Some((child, arena_parent)) = pending.pop() {
        let area = mindnode_container_area(map, child);
        let element = GfxElement::new_area_non_indexed_with_id(area, child.channel, *id_counter);
        *id_counter += 1;

        let child_node_id = arena_parent.append_value(element, &mut tree.arena);
        node_map.insert(child.id.clone(), child_node_id);

        append_node_sections(map, child, child_node_id, tree, section_map, id_counter);

        if !child.folded {
            for grandchild in index.children_of(&child.id).iter().rev() {
                pending.push((grandchild, child_node_id));
            }
        }
    }
}
