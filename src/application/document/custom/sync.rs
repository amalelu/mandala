// SPDX-License-Identifier: MPL-2.0

//! Reverse converter — pulls live tree-side `(text, regions,
//! position, size)` per section back into the model's
//! `MindSection` shape after a custom mutation lands. The forward
//! direction (model → tree) lives in
//! `lib/baumhard/src/mindmap/tree_builder/node.rs::append_node_sections`;
//! this file is the dedicated reverse counterpart for the
//! `Persistent` apply path.
//!
//! Why split from `mod.rs`: the apply pipeline (200+ LOC) and
//! the reverse converter (200+ LOC) share `MindMapDocument`'s
//! `&mut self` access but no other state. Splitting along that
//! conceptual seam keeps each file to one job, matching the
//! `nodes/{mod, …}` precedent already in this directory.

use baumhard::core::primitives::ColorFontRegion;
use baumhard::font::fonts::{app_font_by_family, family_name_of};
use baumhard::mindmap::model::validate;
use baumhard::mindmap::model::TextRun;
use baumhard::mindmap::tree_builder::MindMapTree;
use baumhard::util::color_conversion::{is_var_ref, rgba_to_hex};

// The reverse converter's "this run has nothing to inherit from"
// answer is the authoring layer's answer — one question, one
// constant. It used to be a second `#ffffff` local to this file,
// which kept saying white after the authoring default became the
// empty string: a region with no color and no overlapping prior
// run baked opaque white into the model and opted those graphemes
// out of the palette for good, which is the exact defect the
// authoring default was changed to avoid.
use baumhard::mindmap::tree_builder::effective_section_scale;

use super::super::defaults::DEFAULT_RUN_COLOR;
use super::super::nodes::clamp_runs_to_text;
use super::super::MindMapDocument;

/// Default font-size the *renderer* uses when a section pins none,
/// pinned to the forward path's
/// [`baumhard::mindmap::tree_builder::DEFAULT_SECTION_FONT_SCALE`]
/// so the reverse converter's delta arithmetic can never drift
/// from the scale the forward converter actually wrote. (The model
/// re-exports the same number as
/// `baumhard::mindmap::model::DEFAULT_TEXT_RUN_SIZE_PT` — the
/// serde default a size-less run deserializes to — so all three
/// readings are one constant by construction.)
///
/// Deliberately **not**
/// [`crate::application::document::defaults::DEFAULT_RUN_SIZE_PT`]
/// (24): that is the *authoring* default for a run the user
/// creates, while this is the size a run-less section is already
/// being rendered at. Answering "what size is this section on
/// screen right now?" with the authoring default would make every
/// `grow-font` on a run-less section jump 10pt.
pub(super) const DEFAULT_TEXT_RUN_SIZE_PT: f32 = baumhard::mindmap::tree_builder::DEFAULT_SECTION_FONT_SCALE;

/// Floor the reverse converter clamps `size_pt` to. A
/// `shrink-font` mutation drives tree-side `scale` toward (and
/// past) zero without a floor of its own; without one here a
/// shrunk run would land at (or below) the loader's 0.5pt
/// minimum and render invisible, un-regrowable text. Clamp to
/// 1pt — deliberately above
/// `baumhard::font::fonts::MIN_FONT_SIZE_PT` — so a shrunk run
/// stays legible and can be grown back.
pub(in crate::application::document) const MIN_TEXT_RUN_SIZE_PT: f32 = 1.0;

/// Clamp a `size_pt` into the domain the loader would accept on the
/// way back in.
///
/// **Every writer that takes a caller-supplied size goes through
/// here.** (`section_structure` writes a hardcoded `12` when it
/// synthesizes a run; that is a constant inside the domain, not an
/// input.) The
/// loader rejects a run whose `size_pt` is under the 0.5pt floor
/// or past [`validate::MAX_FONT_SIZE_PT`], so any writer that can
/// leave that range produces a model the editor itself would
/// refuse to reopen. The reverse converter gets there in one
/// click, since a `grow-font` mutation adds an unbounded delta;
/// the console gets there in one line, since `parse_finite_pt`
/// accepts any positive finite `f32` and `font size=5000` is an
/// ordinary thing to type.
///
/// The floor and the ceiling are the same clamp. The floor is the
/// older half — a shrunk run must stay legible and re-growable
/// rather than shrinking into invisibility — and the ceiling is
/// what keeps the file reopenable. `NaN` (never authored, but
/// reachable from mutation arithmetic) lands on the floor.
///
pub(in crate::application::document) fn clamp_run_size_pt(size_pt: f32) -> f32 {
    if size_pt.is_nan() {
        return MIN_TEXT_RUN_SIZE_PT;
    }
    size_pt.clamp(MIN_TEXT_RUN_SIZE_PT, validate::MAX_FONT_SIZE_PT)
}

/// Push the tree-side font `scale` back onto a section's model
/// runs — the reverse of the forward path's
/// `scale = max(run.size_pt)` collapse
/// (`tree_builder/node.rs::mindnode_section_area`). Without this,
/// the bundled `grow-font-2pt` / `shrink-font-2pt` mutations land
/// on the tree for one frame and then vanish on the next
/// rebuild-from-model, because no model field carried the size.
///
/// The forward map is lossy: it takes the **largest** `size_pt`
/// across a section's runs (or [`DEFAULT_TEXT_RUN_SIZE_PT`] when
/// the section has none — this is `effective_section_scale`) and
/// derives `line_height = scale * LINE_HEIGHT_FACTOR`.
/// The reverse therefore has to answer "the max just moved from A
/// to B — how do the individual runs move?". We distribute the
/// change as a **delta** (`tree_scale - old_scale`) added to every
/// run rather than overwriting each run with `tree_scale`, so the
/// *relative* sizing of a multi-run section survives: a
/// `[14pt, 74pt]` section grown 2pt becomes `[16pt, 76pt]`, not
/// `[76pt, 76pt]`. Grow/shrink-font are pure deltas so this is
/// exact for them; an absolute `SetFontSize` reduces to "shift the
/// section so its largest run hits the target", which keeps the
/// same relative spread — the only self-consistent inverse of a
/// max-collapsing forward map.
///
/// **Line-height** has no independent model home: the forward path
/// unconditionally recomputes it as `scale * LINE_HEIGHT_FACTOR`,
/// so persisting
/// `scale` is sufficient and the next rebuild reproduces the right
/// line-height for free. A mutation that touches *only* line-height
/// is surfaced at apply time by
/// `MindMapDocument::warn_unsupported_mutator_fields`.
///
/// **Runless sections** have nowhere to store a size, so the change
/// would evaporate. To honor it we synthesize one run spanning the
/// whole text carrying the new size and an **empty** `color`, so
/// rendering is unchanged except for the size.
///
/// Empty rather than the section's currently-effective color, which
/// is the whole point: an empty `color` is the model's spelling for
/// "take the node's section-level text color", which is exactly
/// what a run-less section was already doing. Baking the resolved
/// value in instead would render identically *today* and convert a
/// themed section into a hardcoded one — the next palette edit
/// would move every sibling section and leave this one behind. A
/// font-size mutation has no business deciding a section's color
/// (`format/palettes.md`).
///
/// `old_scale` is the section's effective scale **before** this
/// sync ran — i.e. the value the forward path put in the tree,
/// captured by the caller *before* the text/regions round-trip may
/// have rewritten `section.text_runs`. Taking it as a parameter
/// (rather than recomputing from the current runs) is load-bearing:
/// a text/region mutation that drops the largest run — `PopBack`
/// deleting the tail run, `DeleteColorFontRegion` / `ChangeRegionRange`
/// dropping the 40pt span of a `[14pt, 40pt]` section — would leave
/// a stale `tree_scale` (40) against a freshly-shrunk current max
/// (14), and a recomputed delta of +26 would wrongly inflate the
/// surviving run to 40pt and record a phantom font-size change.
///
/// Returns `true` when it wrote anything.
fn sync_section_font_size(
    section: &mut baumhard::mindmap::model::MindSection,
    tree_scale: f32,
    old_scale: f32,
) -> bool {
    use baumhard::util::grapheme_chad::count_grapheme_clusters;

    let delta = tree_scale - old_scale;
    // Exact-zero test, not an epsilon: `old_scale` is the model's
    // own max-run size (or the runless default), and the forward
    // builder copies exactly that value into the tree, so a
    // mutation that didn't touch size hands back a `tree_scale`
    // bit-identical to it and there is no float noise to absorb.
    // Anything else is a real size change, and fractional deltas
    // are first-class now that `size_pt` is an `f32` (a sub-half-
    // point delta used to be dropped because the integer field
    // rounded it to no change on every run).
    if delta == 0.0 {
        return false;
    }

    if section.text_runs.is_empty() {
        let end = count_grapheme_clusters(&section.text);
        if end == 0 {
            // Empty text: no glyphs to size, and a zero-length run
            // would be dropped by `clamp_runs_to_text` anyway.
            return false;
        }
        let size_pt = clamp_run_size_pt(tree_scale);
        section.text_runs.push(TextRun {
            start: 0,
            end,
            bold: false,
            italic: false,
            underline: false,
            font: String::new(),
            size_pt,
            color: String::new(),
            hyperlink: None,
        });
        return true;
    }

    let mut changed = false;
    for run in section.text_runs.iter_mut() {
        let new_size = clamp_run_size_pt(run.size_pt + delta);
        if new_size != run.size_pt {
            run.size_pt = new_size;
            changed = true;
        }
    }
    changed
}

/// Roll a tree-side [`ColorFontRegion`] back into a model-side
/// [`TextRun`], merging fields the tree dropped during the
/// forward conversion against a `prior` run when the prior
/// covered the same `Range`. The forward path
/// (`tree_builder/node.rs::append_node_sections`) only carries
/// `range`, `color`, and `font` onto the tree-side region;
/// `bold` / `italic` / `underline` / `size_pt` / `hyperlink`
/// disappear into the cosmic-text default attribute set. The
/// reverse path can recover them only when a matching prior run
/// is available — which is true for round-trips through the
/// custom-mutation pipeline (the tree is rebuilt from the model
/// just before each apply, so every region's range was an
/// authored run before the mutation ran).
///
/// `theme_text_rgba` is the pixel value the forward path paints a
/// *colorless* run in — [`baumhard::mindmap::model::MindMap::node_text_rgba`]
/// for the node that owns the section. It is what lets a run that
/// deferred to the theme survive the trip; see the
/// `prior_deferred_color` short-circuit below.
///
/// Limitations, both of them a *recolor* the converter cannot see
/// because the tree side keeps no record of what the model said:
/// - `var(--name)` color references collapse to their resolved
///   hex on the round trip *unless* the prior run shares the
///   region's range — see the `prior_deferred_color`
///   short-circuit below. A mutation that recolors a
///   `var()`-bearing run without resizing it is swallowed; the
///   run keeps the variable.
/// - A recolor of a **colorless** run to exactly
///   `theme_text_rgba` is likewise swallowed: the run stays
///   colorless. Same pixels either way, and the deferral is the
///   safer of the two readings — see the arm's own comment for
///   why no range test can separate them here.
/// - Unknown `AppFont` (corrupt tree state) falls through to
///   the empty string, matching the loader's tolerance for
///   missing-font runs.
///
/// Visible to [`super::super::nodes`] so the editor commit path
/// can reuse the converter through `set_section_text_and_runs`.
pub(crate) fn region_to_text_run(
    region: &ColorFontRegion,
    prior: Option<&TextRun>,
    theme_text_rgba: baumhard::util::color::FloatRgba,
) -> TextRun {
    // Two model-side color spellings have no tree-side
    // representation at all: a `var(--name)` reference and the
    // empty string that defers to the node's section-level text
    // color. Both reach the tree as a concrete rgba, so a naive
    // reverse conversion replaces them with a literal hex and the
    // authored indirection is gone for good — the variable stops
    // tracking the canvas theme, the empty run stops tracking the
    // palette. Recover both from the prior run when it shares the
    // region's exact range.
    //
    // The two differ in how much evidence they can muster, and so
    // in how strict they have to be.
    //
    // Resolving a `var()` here would need the canvas theme map the
    // converter is not handed, so that arm has nothing to check
    // against and simply trusts the prior — but only when the
    // prior covers the region's *exact* range, since a resized
    // span is no longer demonstrably the one that carried the
    // variable. The documented trade-off stands: a mutation
    // deliberately recoloring a `var()`-bearing run is silently
    // swallowed.
    //
    // The empty-color arm has evidence the `var()` arm lacks —
    // whether the region still paints `theme_text_rgba` — and so
    // needs no range test at all. That matters: the commonest way
    // a deferring run reaches this function is a keystroke in the
    // inline editor, which extends `[0,6)` to `[0,7)`. An exact-
    // range rule would bake the palette hex on the first character
    // typed, which is the whole defect this arm exists to stop.
    // A recolor that moves the region off the theme color lands as
    // a literal hex, exactly as it should.
    //
    // A recolor to *exactly* the theme color does not, and cannot:
    // the region carries a resolved rgba and nothing else, so
    // "still deferring" and "deliberately pinned to the color the
    // node happens to have" are the same four floats. This arm
    // reads them as the first, and committing the palette's own
    // text color onto a deferring run therefore comes back as the
    // empty string. Pixels are identical; the lost intent is "pin
    // these graphemes so a retheme cannot move them". It is the
    // same shape as the `var()` arm's swallow, traded the same
    // way, and the only thing that would distinguish the two cases
    // is the exact-range test whose cost is baking on the first
    // keystroke — much the commoner gesture, and destructive where
    // this is merely lossy.
    let prior_deferred_color: Option<&str> = prior.and_then(|p| {
        if p.color.is_empty() {
            return (region.color == Some(theme_text_rgba)).then_some("");
        }
        let same_range = p.start == region.range.start && p.end == region.range.end;
        (is_var_ref(&p.color) && same_range).then_some(p.color.as_str())
    });
    let color = match (prior_deferred_color, region.color) {
        (Some(deferred), _) => deferred.to_string(),
        (None, Some(rgba)) => rgba_to_hex(rgba),
        (None, None) => prior
            .map(|p| p.color.clone())
            .unwrap_or_else(|| DEFAULT_RUN_COLOR.to_string()),
    };
    let font = match region.font.and_then(family_name_of) {
        Some(name) => name.to_string(),
        None => prior.map(|p| p.font.clone()).unwrap_or_default(),
    };
    let bold = prior.is_some_and(|p| p.bold);
    let italic = prior.is_some_and(|p| p.italic);
    let underline = prior.is_some_and(|p| p.underline);
    let size_pt = prior.map(|p| p.size_pt).unwrap_or(DEFAULT_TEXT_RUN_SIZE_PT);
    let hyperlink = prior.and_then(|p| p.hyperlink.clone());
    TextRun {
        start: region.range.start,
        end: region.range.end,
        bold,
        italic,
        underline,
        font,
        size_pt,
        color,
        hyperlink,
    }
}

/// Find the prior `TextRun` for a tree-side region by range.
/// Prefers exact `(start, end)` match; falls back to the prior
/// run whose intersection with `[start, end)` is largest. Used by
/// `sync_node_from_tree`'s reverse converter so a custom mutation
/// that resizes / splits a region (e.g. `ChangeRegionRange`)
/// still inherits authored styling instead of zeroing every
/// field. Ties broken in favor of earlier `start`.
///
/// Returns `None` only when no prior run overlaps the new range
/// at all (e.g. a fresh region inserted by the mutation).
///
/// Visible to [`super::super::nodes`] so the editor commit path
/// can reuse the same lookup through `set_section_text_and_runs`.
pub(crate) fn exact_or_dominant_overlap<'a>(
    priors: &[&'a TextRun],
    start: usize,
    end: usize,
) -> Option<&'a TextRun> {
    if let Some(exact) = priors.iter().find(|r| r.start == start && r.end == end) {
        return Some(exact);
    }
    let mut best: Option<(&'a TextRun, usize)> = None;
    for run in priors.iter() {
        if run.end <= start || run.start >= end {
            continue;
        }
        let lo = run.start.max(start);
        let hi = run.end.min(end);
        if hi <= lo {
            continue;
        }
        let overlap = hi - lo;
        match best {
            None => best = Some((run, overlap)),
            Some((_, prev)) if overlap > prev => best = Some((run, overlap)),
            _ => {}
        }
    }
    best.map(|(r, _)| r)
}

impl MindMapDocument {
    /// Sync the document model from the live tree — pull
    /// `node.position` from the container's glyph area and every
    /// section's `(text, text_runs, offset, size, font size)` from
    /// its section-area, with a per-section selective gate that
    /// skips the lossy text/regions round-trip when the tree side
    /// hasn't diverged from the model. Position / offset / size /
    /// font-size always write back; text + runs gate on the
    /// `(range, color, font)` triple.
    ///
    /// Used by the `Persistent` apply path to commit a custom
    /// mutation's tree-side mutations to the model so the next
    /// `rebuild_all` doesn't revert them. The selective gate
    /// matters because the forward conversion drops
    /// `bold` / `italic` / `underline` / `size_pt` / `hyperlink`;
    /// an unconditional round-trip would silently strip those
    /// fields from sections the mutation didn't touch.
    ///
    /// Returns `true` when this call actually changed the model.
    /// The caller ([`super::MindMapDocument::apply_custom_mutation`])
    /// uses the verdict to gate the undo-stack push and the `dirty`
    /// flag — a mutation whose tree edits round-trip to no model
    /// change (a no-op apply, a `flat_mutations`-failed skip, or a
    /// predicate that filtered every candidate) must not leave a
    /// dead undo entry behind.
    #[must_use]
    pub(super) fn sync_node_from_tree(&mut self, node_id: &str, tree: &MindMapTree) -> bool {
        let Some(tree_nid) = tree.arena_id_for(node_id) else {
            return false;
        };
        let Some(element) = tree.tree.arena.get(tree_nid).map(|n| n.get()) else {
            return false;
        };
        let Some(area) = element.glyph_area() else {
            return false;
        };
        let new_pos = (area.position.x.0 as f64, area.position.y.0 as f64);

        // Gather every section's tree-side `(text, regions, position,
        // size)` before we acquire `&mut` on the model. The arena
        // lookup needs `&tree`; the model write needs `&mut self`;
        // sequencing them avoids overlapping borrows on
        // `self.mindmap`. Capturing position + size lets us write
        // `section.offset` / `section.size` back from the tree, so a
        // `SectionsOnly` mutation that translates / resizes a
        // section persists past the next `rebuild_all`.
        let section_count = self
            .mindmap
            .nodes
            .get(node_id)
            .map(|n| n.sections.len())
            .unwrap_or(0);
        struct SectionSnapshot {
            text: String,
            regions: Vec<ColorFontRegion>,
            tree_position: (f32, f32),
            tree_size: (f32, f32),
            /// Tree-side font scale (points). The forward path sets
            /// this to the largest `run.size_pt`; the reverse
            /// distributes any change back across the runs. See
            /// [`sync_section_font_size`].
            tree_scale: f32,
        }
        let mut section_snapshots: Vec<Option<SectionSnapshot>> = Vec::with_capacity(section_count);
        for idx in 0..section_count {
            let snapshot = tree
                .section_arena_id(node_id, idx)
                .and_then(|sid| tree.tree.arena.get(sid))
                .and_then(|n| n.get().glyph_area())
                .map(|sec_area| SectionSnapshot {
                    text: sec_area.text.clone(),
                    regions: sec_area
                        .regions
                        .all_regions()
                        .into_iter()
                        .copied()
                        .collect::<Vec<ColorFontRegion>>(),
                    tree_position: (sec_area.position.x.0, sec_area.position.y.0),
                    tree_size: (sec_area.render_bounds.x.0, sec_area.render_bounds.y.0),
                    tree_scale: sec_area.scale.0,
                });
            section_snapshots.push(snapshot);
        }

        // Everything the reverse comparison needs from the *map*
        // rather than from the node alone, captured before the
        // mutable borrow below takes `self.mindmap` exclusively.
        //
        // Both entries answer the same question — "what would the
        // forward path have produced from the model as it stands?" —
        // for the two places a section's colors come from something
        // other than an explicitly-colored `TextRun`: the palette
        // cascade, as the pixel value a colorless run is painted in
        // (`MindMap::node_text_rgba`), and, for a run-less section,
        // the default region table the tree builder synthesizes.
        let Some(read_node) = self.mindmap.nodes.get(node_id) else {
            return false;
        };
        let node_text_rgba = self.mindmap.node_text_rgba(read_node);
        let projected_default_regions: Vec<Vec<ColorFontRegion>> = read_node
            .sections
            .iter()
            .enumerate()
            .map(|(idx, section)| {
                baumhard::mindmap::tree_builder::section_default_regions(
                    &self.mindmap,
                    read_node,
                    section,
                    idx,
                )
            })
            .collect();

        let Some(model_node) = self.mindmap.nodes.get_mut(node_id) else {
            return false;
        };
        let mut changed = false;
        // Compare in f32 space — the tree stores positions as `f32`,
        // so projecting the model down to `f32` is exactly the value
        // the forward path put in the tree. Comparing the wider model
        // `f64` against the narrower tree `f32` would flag a spurious
        // change for every node whose authored `f64` position isn't
        // exactly `f32`-representable, and that false "changed"
        // verdict would push a dead undo entry for a no-op mutation.
        let tree_px = new_pos.0 as f32;
        let tree_py = new_pos.1 as f32;
        if model_node.position.x as f32 != tree_px || model_node.position.y as f32 != tree_py {
            model_node.set_position_clamped(new_pos.0, new_pos.1);
            changed = true;
        }
        let node_pos_x = tree_px;
        let node_pos_y = tree_py;
        let node_size_x = model_node.size.width as f32;
        let node_size_y = model_node.size.height as f32;

        for (idx, snapshot) in section_snapshots.into_iter().enumerate() {
            let Some(snapshot) = snapshot else {
                continue;
            };
            let Some(section) = model_node.sections.get_mut(idx) else {
                continue;
            };

            // Capture the section's effective scale BEFORE the
            // text/regions round-trip below can rewrite `text_runs`.
            // This is the value the forward path put in the tree
            // (`scale = max(run.size_pt)`, or the default for a
            // runless section), and the correct baseline for the
            // font-size delta — recomputing it after the round-trip
            // would misread a run-dropping mutation as a size change.
            let pre_round_trip_scale = effective_section_scale(section);

            // Write `section.offset` back from the tree's section-
            // area position so a `SectionsOnly` translate mutation
            // persists. The forward path computes
            // `section_area.position = node.pos + section.offset`,
            // so the inverse is `section.offset = section_area.position
            // - node.pos`. Section-area position is canvas-space
            // float; model `Position` is canvas-space f64 — same
            // unit, just wider. Without this, a `Translate` /
            // `MoveTo` on a section-area lands on the live tree
            // and reverts on the next `rebuild_all`.
            // Compare in f32 space (see the node-position note above):
            // the tree carries `node_pos + section.offset` as `f32`,
            // so project the model offset the same way. A raw `f64`
            // compare would flag a phantom change for any authored
            // offset that isn't `f32`-exact and push a dead undo entry.
            let projected_sx = node_pos_x + section.offset.x as f32;
            let projected_sy = node_pos_y + section.offset.y as f32;
            if projected_sx != snapshot.tree_position.0 || projected_sy != snapshot.tree_position.1 {
                section.offset.x = (snapshot.tree_position.0 - node_pos_x) as f64;
                section.offset.y = (snapshot.tree_position.1 - node_pos_y) as f64;
                changed = true;
            }
            // Write `section.size` back when the tree's post-mutation
            // bounds differ from what the model already says. `None`
            // size means "fill the parent node", which the tree
            // resolves to the node's full render_bounds — *don't*
            // eagerly materialize it as `Some(node.size)`, that would
            // surprise authors who chose the inheriting shape. So the
            // `None` case compares against the node's own size, which
            // is what the forward path drew, and only a mutation that
            // actually resized the section pins one.
            //
            // Compare in f32 space, for the same reason the position
            // writeback above does: the tree carries f32 and the model
            // f64, so a raw f64 compare would flag a phantom change for
            // any size that is not f32-exact.
            //
            // The two arms answer with different rulers, deliberately.
            // An explicit `Some` is already a pinned size, so any
            // f32-visible divergence from it is a real edit, and the
            // exact `!=` is right — the same posture the position
            // writeback above documents. A `None` is a *shape*, not a
            // size, and demoting it to a fixed one is a one-way door for
            // the author, so that arm holds the shape until the resize
            // clears an absolute `f32::EPSILON` floor.
            //
            // The floor is not a restatement of `!=`. It can only ever
            // differ from one when *both* operands are `<= 2.0`:
            // `ULP(x) == f32::EPSILON` exactly on `[1, 2)` and doubles
            // at every binade above, so any two distinct f32s that are
            // both at 2.0 or above already differ by more than
            // `f32::EPSILON`, and so does any pair straddling 2.0.
            // (There is no band "below one ULP" for the floor to
            // swallow — two distinct f32s cannot differ by less than
            // one ULP.) Under 2.0 the two spellings genuinely part
            // company, and that is reachable: `validate.rs` puts no
            // lower bound on `node.size` beyond finite and positive,
            // and `format/sections.md` documents `SetBounds` under
            // `SectionsOnly` as writing through here. Pinned by
            // `test_sync_node_from_tree_holds_fill_parent_none_at_a_sub_epsilon_resize`.
            let size_diverges = match section.size {
                Some(s) => s.width as f32 != snapshot.tree_size.0 || s.height as f32 != snapshot.tree_size.1,
                None => {
                    (snapshot.tree_size.0 - node_size_x).abs() > f32::EPSILON
                        || (snapshot.tree_size.1 - node_size_y).abs() > f32::EPSILON
                }
            };
            if size_diverges {
                section.size = Some(baumhard::mindmap::model::Size {
                    width: snapshot.tree_size.0 as f64,
                    height: snapshot.tree_size.1 as f64,
                });
                changed = true;
            }

            // Selective gate: tree-side state matches the model
            // snapshot? Skip the text/regions round-trip so
            // untouched sections keep their bold / italic /
            // underline / size_pt / hyperlink. Range / color /
            // font are everything the forward conversion
            // preserves.
            //
            // **Range-keyed comparison.** Tree-side
            // `all_regions()` returns runs in `Range` order
            // (`BTreeSet`-keyed); model `text_runs: Vec<TextRun>`
            // is load-order. A positional `zip` would mis-align
            // any model whose runs were authored out of range
            // order, trip a false mismatch, and run the lossy
            // round-trip — silently stripping the prior styling
            // from sections the mutation didn't touch. Build a
            // map keyed by `(start, end)` and compare each
            // tree-side region against the same-range prior.
            //
            // **A run-less section is not a region-less section.**
            // The forward path emits the node's color defaults as
            // explicit regions for a section with no runs
            // (`tree_builder::section_default_regions`), so comparing
            // the tree's regions against an empty run list would
            // report divergence on every apply and synthesize a
            // phantom `TextRun` out of the default. Project the same
            // defaults from the model and compare against those
            // instead; a mutation that genuinely recolored the
            // section still differs and still falls through to the
            // round trip below.
            let model_regions_match = if section.text_runs.is_empty() {
                // The run-less arm: compare against the projected
                // defaults, which the forward path derives with the
                // same function. Both sides come out of
                // `hex_to_rgba_safe` over the same authored string,
                // so the `f32` channels are bit-identical and an
                // exact compare is the right one here — no
                // case-folding tier is needed, because neither side
                // is a hand-authored string.
                projected_default_regions.get(idx).is_some_and(|expected| {
                    expected.len() == snapshot.regions.len()
                        && expected
                            .iter()
                            .zip(snapshot.regions.iter())
                            .all(|(a, b)| a.range == b.range && a.color == b.color && a.font == b.font)
                })
            } else {
                let model_runs_by_range: rustc_hash::FxHashMap<(usize, usize), &TextRun> =
                    section.text_runs.iter().map(|r| ((r.start, r.end), r)).collect();
                model_runs_by_range.len() == snapshot.regions.len()
                    && snapshot.regions.iter().all(|region| {
                        let key = (region.range.start, region.range.end);
                        let Some(run) = model_runs_by_range.get(&key) else {
                            return false;
                        };
                        // Color comparison is **case-insensitive on
                        // hex**: `rgba_to_hex` always emits lowercase,
                        // but model-side `run.color` may have been
                        // hand-authored as `#FFFFFF` or mixed case. A
                        // byte-equal `==` would always-mismatch those
                        // and trigger the lossy round-trip on every
                        // apply_to_tree call.
                        let region_color_hex = region.color.map(rgba_to_hex);
                        let model_color_hex = if run.color.starts_with('#') {
                            Some(run.color.clone())
                        } else {
                            None
                        };
                        let model_is_var = is_var_ref(&run.color);
                        let colors_equal = match (region_color_hex.as_deref(), model_color_hex.as_deref()) {
                            (Some(a), Some(b)) => str::eq_ignore_ascii_case(a, b),
                            (None, None) => true,
                            // `(Some(hex), None)` with the model carrying
                            // a `var(--…)` reference: presume the
                            // variable resolves to the tree-side hex
                            // and treat as equal. Documented limit: a
                            // custom mutation that *deliberately*
                            // recolors a `var()`-bearing run is
                            // silently swallowed; the run keeps the
                            // variable.
                            (Some(_), None) if model_is_var => true,
                            // `(Some(hex), None)` with the model
                            // carrying an **empty** color: the run
                            // declined to name one and defers to the
                            // node's section-level text color — the
                            // palette group's `text` on a themed
                            // node (`format/palettes.md`: "The theme
                            // reaches text through runs that leave
                            // `color` empty"). The forward path
                            // paints such a run with exactly
                            // `node_text_rgba`, so that — not a
                            // literal hex the model never held — is
                            // what the region has to be compared
                            // against. Without this arm every
                            // section holding a colorless run reads
                            // as divergent on *every* apply, and the
                            // round trip below bakes the palette hex
                            // into the run, severing the cascade for
                            // good on a mutation that moved nothing.
                            (Some(_), None) if run.color.is_empty() => region.color == Some(node_text_rgba),
                            _ => false,
                        };
                        if !colors_equal {
                            return false;
                        }
                        // Font comparison **forward-projects the model**,
                        // exactly like the position / offset / size
                        // comparisons above: run the model's family
                        // string through `app_font_by_family` (empty →
                        // `None`, matching the forward path in
                        // `tree_builder/node.rs::mindnode_section_area`)
                        // and compare the resulting `Option<AppFont>`
                        // against what the tree actually carries.
                        //
                        // Comparing the tree-side font back-projected to
                        // a *name* against the model's raw string is
                        // asymmetric and can never succeed for a family
                        // the font database doesn't know: the forward
                        // path maps an unresolvable family to `None`, so
                        // `family_name_of(None)` yields `None` while the
                        // model still holds the authored string. Every
                        // section then reads as divergent on every apply
                        // — the lossy text/regions round-trip runs
                        // needlessly and `changed` comes back `true` for
                        // a genuine no-op, resurrecting exactly the dead
                        // undo entries P0-02 (#2) removed. The testament
                        // map hits this on all 252 nodes: it authors
                        // `"LiberationSans"` while the face registers
                        // `"Liberation Sans"`.
                        let model_font = if run.font.is_empty() {
                            None
                        } else {
                            app_font_by_family(&run.font)
                        };
                        region.font == model_font
                    })
            };
            // Selective gate: only run the lossy text/regions
            // round-trip when the tree side diverged. Note this is
            // NOT a `continue` — the font-size sync below must run
            // regardless, since a pure `grow-font` mutation leaves
            // text and regions byte-identical yet still needs its
            // `scale` change persisted.
            if !(section.text == snapshot.text && model_regions_match) {
                // Build the new run list by merging each tree-side
                // region with the prior run sharing the **same
                // range, or the dominant overlapping range** when
                // the mutation resized / split / shifted the run
                // boundary. A range-strict lookup loses every prior
                // styling (bold / italic / underline / size_pt /
                // hyperlink) on `ChangeRegionRange`-style mutations
                // because no prior matches the new range exactly;
                // the overlap fallback inherits from the prior whose
                // intersection is largest, preserving authored
                // styling across range edits.
                let prior_runs: Vec<&TextRun> = section.text_runs.iter().collect();
                let new_runs: Vec<TextRun> = snapshot
                    .regions
                    .iter()
                    .map(|region| {
                        let prior =
                            exact_or_dominant_overlap(&prior_runs, region.range.start, region.range.end);
                        region_to_text_run(region, prior, node_text_rgba)
                    })
                    .collect();

                section.text = snapshot.text;
                section.text_runs = new_runs;
                // Ensure no run extends past the new grapheme count —
                // `clamp_runs_to_text` is already idempotent on
                // already-clean run lists.
                clamp_runs_to_text(section);
                changed = true;
            }

            // Font-size sync — runs *after* the text/runs round-trip
            // (so it operates on the final run list and isn't
            // clobbered by it) and *unconditionally* (so a
            // scale-only mutation that skips the round-trip above is
            // still persisted). Distributes the tree-side `scale`
            // delta across the section's runs; see
            // [`sync_section_font_size`].
            if sync_section_font_size(section, snapshot.tree_scale, pre_round_trip_scale) {
                changed = true;
            }
        }
        changed
    }
}

#[cfg(test)]
mod region_converter_tests {
    use super::{exact_or_dominant_overlap, region_to_text_run, DEFAULT_RUN_COLOR, DEFAULT_TEXT_RUN_SIZE_PT};
    use baumhard::core::primitives::{ColorFontRegion, Range};
    use baumhard::mindmap::model::TextRun;

    /// Stand-in for the owning node's effective text color as the
    /// forward path paints it. Every channel is exactly
    /// representable in `f32`, so the equality the converter does
    /// against a region's color is decided by the test's intent
    /// rather than by a rounding accident.
    const THEME_TEXT_RGBA: baumhard::util::color::FloatRgba = [0.25, 0.5, 0.75, 1.0];

    fn run(start: usize, end: usize, color: &str, font: &str) -> TextRun {
        TextRun {
            start,
            end,
            bold: false,
            italic: false,
            underline: false,
            font: font.into(),
            size_pt: 14.0,
            color: color.into(),
            hyperlink: None,
        }
    }

    fn styled_run(start: usize, end: usize) -> TextRun {
        TextRun {
            start,
            end,
            bold: true,
            italic: true,
            underline: true,
            font: "LiberationSans".into(),
            size_pt: 21.0,
            color: "#aabbcc".into(),
            hyperlink: Some("https://example.org".into()),
        }
    }

    #[test]
    fn test_region_to_text_run_merges_with_prior() {
        let region = ColorFontRegion::new(Range::new(0, 5), None, Some([1.0, 0.0, 0.0, 1.0]));
        let prior = styled_run(0, 5);
        let out = region_to_text_run(&region, Some(&prior), THEME_TEXT_RGBA);
        assert_eq!(out.start, 0);
        assert_eq!(out.end, 5);
        assert_eq!(out.color, "#ff0000");
        assert!(out.bold);
        assert!(out.italic);
        assert!(out.underline);
        assert_eq!(out.size_pt, 21.0);
        assert_eq!(out.hyperlink.as_deref(), Some("https://example.org"));
    }

    #[test]
    fn test_region_to_text_run_falls_back_to_defaults_without_prior() {
        let region = ColorFontRegion::new(Range::new(0, 5), None, None);
        let out = region_to_text_run(&region, None, THEME_TEXT_RGBA);
        assert!(!out.bold);
        assert!(!out.italic);
        assert!(!out.underline);
        assert_eq!(out.size_pt, DEFAULT_TEXT_RUN_SIZE_PT);
        assert_eq!(out.hyperlink, None);
        assert_eq!(out.font, "");
        // Spelled out rather than compared to the constant: the
        // point of the assertion is that a run with nothing to
        // inherit from *defers*, and an assertion against the
        // constant would follow the constant anywhere it went —
        // including back to the `#ffffff` that baked white into
        // the model and severed the graphemes from the palette.
        assert_eq!(out.color, "", "a colorless region with no prior must defer");
        assert_eq!(out.color, DEFAULT_RUN_COLOR, "and that is the authoring default");
    }

    #[test]
    fn test_region_to_text_run_uses_region_color_without_prior() {
        let region = ColorFontRegion::new(Range::new(0, 3), None, Some([0.0, 1.0, 0.0, 1.0]));
        let out = region_to_text_run(&region, None, THEME_TEXT_RGBA);
        assert_eq!(out.color, "#00ff00");
    }

    #[test]
    fn test_region_to_text_run_preserves_var_color_when_range_matches() {
        let region = ColorFontRegion::new(Range::new(0, 5), None, Some([1.0, 0.0, 0.0, 1.0]));
        let prior_with_var = TextRun {
            color: "var(--accent)".into(),
            ..styled_run(0, 5)
        };
        let out = region_to_text_run(&region, Some(&prior_with_var), THEME_TEXT_RGBA);
        assert_eq!(out.color, "var(--accent)");
    }

    #[test]
    fn test_region_to_text_run_loses_var_color_on_range_change() {
        let region = ColorFontRegion::new(Range::new(0, 3), None, Some([1.0, 0.0, 0.0, 1.0]));
        let prior_with_var = TextRun {
            color: "var(--accent)".into(),
            ..styled_run(0, 5)
        };
        let out = region_to_text_run(&region, Some(&prior_with_var), THEME_TEXT_RGBA);
        assert_eq!(out.color, "#ff0000");
    }

    /// A run that left `color` empty defers to the node's
    /// section-level text color. The tree can only carry the
    /// resolved pixel value, so the converter has to recognize it
    /// and hand the deferral back — otherwise one round trip bakes
    /// the palette hex into the model and the node stops tracking
    /// its palette forever.
    #[test]
    fn test_region_to_text_run_keeps_empty_color_when_region_still_paints_the_theme() {
        let region = ColorFontRegion::new(Range::new(0, 5), None, Some(THEME_TEXT_RGBA));
        let prior_deferring = TextRun {
            color: String::new(),
            ..styled_run(0, 5)
        };
        let out = region_to_text_run(&region, Some(&prior_deferring), THEME_TEXT_RGBA);
        assert_eq!(out.color, "");
        // The rest of the merge is unaffected.
        assert!(out.bold);
        assert_eq!(out.size_pt, 21.0);
    }

    /// The other half of the same rule: a mutation that genuinely
    /// recolored the run moves the region off the theme color, and
    /// then the literal hex is the honest answer. This is what
    /// separates the empty-color arm from the `var()` arm, which
    /// cannot check and so always trusts the prior.
    #[test]
    fn test_region_to_text_run_bakes_empty_color_when_region_was_recolored() {
        let region = ColorFontRegion::new(Range::new(0, 5), None, Some([1.0, 0.0, 0.0, 1.0]));
        let prior_deferring = TextRun {
            color: String::new(),
            ..styled_run(0, 5)
        };
        let out = region_to_text_run(&region, Some(&prior_deferring), THEME_TEXT_RGBA);
        assert_eq!(out.color, "#ff0000");
    }

    /// Unlike the `var()` arm, the deferral survives a range
    /// change — the region's color is the evidence, not the range.
    /// This is the inline-editor shape: one keystroke turns
    /// `[0,5)` into `[0,6)`, and an exact-range rule would bake the
    /// palette hex on the first character typed.
    #[test]
    fn test_region_to_text_run_keeps_empty_color_across_a_range_change() {
        let region = ColorFontRegion::new(Range::new(0, 6), None, Some(THEME_TEXT_RGBA));
        let prior_deferring = TextRun {
            color: String::new(),
            ..styled_run(0, 5)
        };
        let out = region_to_text_run(&region, Some(&prior_deferring), THEME_TEXT_RGBA);
        assert_eq!(out.color, "");
    }

    /// The documented cost of the range-insensitivity above: a
    /// recolor to *exactly* the node's current text color is
    /// indistinguishable from still deferring, because the region
    /// carries a resolved rgba and nothing else. The run comes back
    /// colorless. Pixels are identical; the lost intent is "pin
    /// these graphemes so a retheme cannot move them".
    ///
    /// Pinned rather than left implicit because the arm's comment
    /// used to claim the opposite — that a recolor always lands as
    /// a literal hex — which is true of every recolor except this
    /// one.
    #[test]
    fn test_region_to_text_run_swallows_a_recolor_to_the_theme_color_itself() {
        let region = ColorFontRegion::new(Range::new(0, 5), None, Some(THEME_TEXT_RGBA));
        let prior_deferring = TextRun {
            color: String::new(),
            ..styled_run(0, 5)
        };
        let out = region_to_text_run(&region, Some(&prior_deferring), THEME_TEXT_RGBA);
        assert_eq!(
            out.color, "",
            "a recolor onto the theme color is swallowed — the evidence to \
             tell it apart from a deferral does not reach this function"
        );
    }

    #[test]
    fn test_exact_overlap_match_wins_over_partial() {
        let r1 = run(0, 5, "#aabbcc", "");
        let r2 = run(2, 7, "#ddeeff", "");
        let priors = vec![&r1, &r2];
        let hit = exact_or_dominant_overlap(&priors, 0, 5).expect("exact match");
        assert_eq!(hit.color, "#aabbcc");
    }

    #[test]
    fn test_dominant_overlap_wins_when_no_exact_match() {
        let small = run(0, 1, "#000000", "");
        let large = run(0, 4, "#ffffff", "");
        let priors = vec![&small, &large];
        let hit = exact_or_dominant_overlap(&priors, 0, 5).expect("partial overlap");
        assert_eq!(hit.color, "#ffffff");
    }

    #[test]
    fn test_no_overlap_returns_none() {
        let r1 = run(0, 5, "#aabbcc", "");
        let priors = vec![&r1];
        assert!(exact_or_dominant_overlap(&priors, 10, 15).is_none());
    }
}

/// The idempotence contract behind the `changed` verdict.
///
/// `sync_node_from_tree` is `#[must_use]` precisely because
/// `apply_custom_mutation` gates the undo-stack push and the
/// `dirty` flag on it (P0-02, #2: "No undo entries for no-op
/// applies"). That gate is only worth anything if the verdict is
/// *tight*: a sync against a tree the model just produced, with no
/// mutation in between, must report `false` for every node.
///
/// Any selective-gate comparison that reads a lossy forward
/// conversion backwards breaks the property silently — the model
/// stays byte-identical, but the verdict says "changed" and a dead
/// undo entry lands on the stack, eating the user's next Ctrl-Z.
#[cfg(test)]
mod sync_verdict_tests {
    use crate::application::document::tests_common::load_test_doc;

    /// Model → tree → model with nothing in between changes nothing
    /// and must *report* nothing, for every node in the fixture.
    ///
    /// Red before the font-gate fix: all 252 testament nodes
    /// reported `changed == true` while serializing byte-identical,
    /// because the map authors runs as `"LiberationSans"` while the
    /// bundled face registers the family as `"Liberation Sans"`, so
    /// the forward path stored `None` and the back-projected
    /// comparison could never match.
    #[test]
    fn test_round_trip_with_no_mutation_reports_no_change() {
        let mut doc = load_test_doc();
        let before = doc.mindmap.clone();
        let tree = doc.build_tree();
        let mut ids: Vec<String> = doc.mindmap.nodes.keys().cloned().collect();
        ids.sort();
        let mut reported: Vec<String> = Vec::new();
        for id in &ids {
            if doc.sync_node_from_tree(id, &tree) {
                reported.push(id.clone());
            }
        }
        // The model really is untouched — so any `true` verdict is a
        // false positive, not a missed write.
        for id in &ids {
            assert_eq!(
                serde_json::to_string(before.nodes.get(id).unwrap()).unwrap(),
                serde_json::to_string(doc.mindmap.nodes.get(id).unwrap()).unwrap(),
                "node '{}' must survive a mutation-free round trip byte-identical",
                id
            );
        }
        assert!(
            reported.is_empty(),
            "{} of {} nodes reported a change with no mutation applied (first: {:?}); \
             every one of those is a dead undo entry",
            reported.len(),
            ids.len(),
            &reported[..reported.len().min(5)]
        );
    }
}
