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
use baumhard::mindmap::model::TextRun;
use baumhard::mindmap::tree_builder::MindMapTree;
use baumhard::util::color_conversion::{is_var_ref, rgba_to_hex};

use super::super::nodes::clamp_runs_to_text;
use super::super::MindMapDocument;

/// Default text-run color when neither the tree-side region nor
/// a prior model run carries one. Matches the renderer's
/// fall-through-to-`#ffffff` floor on a node with no explicit
/// `style.text_color` override — the same white
/// [`crate::application::document::defaults::DEFAULT_RUN_COLOR`]
/// gives a freshly-authored run.
pub(super) const DEFAULT_TEXT_RUN_COLOR: &str = "#ffffff";

/// Default font-size the *renderer* uses when a section pins none,
/// pinned to the forward path's
/// [`baumhard::mindmap::tree_builder::DEFAULT_SECTION_FONT_SCALE`]
/// so the reverse converter's delta arithmetic can never drift
/// from the scale the forward converter actually wrote.
///
/// Deliberately **not**
/// [`crate::application::document::defaults::DEFAULT_RUN_SIZE_PT`]
/// (24): that is the *authoring* default for a run the user
/// creates, while this is the size a run-less section is already
/// being rendered at. Answering "what size is this section on
/// screen right now?" with the authoring default would make every
/// `grow-font` on a run-less section jump 10pt.
///
/// The `f32 → u32` narrowing is checked, not silent: model
/// `size_pt` is integral, so a future fractional scale (`14.5`)
/// is a real design question about how the reverse converter
/// should round it, not something to truncate away. The `assert!`
/// evaluates at compile time and fails the build instead.
pub(super) const DEFAULT_TEXT_RUN_SIZE_PT: u32 = {
    let scale = baumhard::mindmap::tree_builder::DEFAULT_SECTION_FONT_SCALE;
    let truncated = scale as u32;
    assert!(
        truncated as f32 == scale,
        "DEFAULT_SECTION_FONT_SCALE is not integral; decide how the reverse \
         converter should round it rather than letting the cast truncate"
    );
    truncated
};

/// Floor the reverse converter clamps `size_pt` to. A
/// `shrink-font` mutation drives tree-side `scale` toward (and
/// past) zero without a floor of its own; model `size_pt` is a
/// `u32`, so a naive cast of a negative scale would saturate to 0
/// and render invisible, un-regrowable text. Clamp to 1pt so a
/// shrunk run stays legible and can be grown back.
pub(super) const MIN_TEXT_RUN_SIZE_PT: u32 = 1;

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
/// the section has none) and derives `line_height = scale * 1.2`.
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
/// unconditionally recomputes it as `scale * 1.2`, so persisting
/// `scale` is sufficient and the next rebuild reproduces the right
/// line-height for free. A mutation that touches *only* line-height
/// is surfaced at apply time by
/// `MindMapDocument::warn_unsupported_mutator_fields`.
///
/// **Runless sections** have nowhere to store a size, so the change
/// would evaporate. To honor it we synthesize one run spanning the
/// whole text carrying the new size and the section's effective
/// default color (`default_color`) so rendering is unchanged
/// except for the size.
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
    default_color: &str,
) -> bool {
    use baumhard::util::grapheme_chad::count_grapheme_clusters;

    let delta = tree_scale - old_scale;
    // `size_pt` is an integer point size, so a sub-half-point delta
    // rounds to no change on every run. Treat it as "scale
    // untouched" so a position-only or color-only mutation doesn't
    // churn run sizes (or spuriously report a change).
    if delta.abs() < 0.5 {
        return false;
    }

    if section.text_runs.is_empty() {
        let end = count_grapheme_clusters(&section.text);
        if end == 0 {
            // Empty text: no glyphs to size, and a zero-length run
            // would be dropped by `clamp_runs_to_text` anyway.
            return false;
        }
        let size_pt = tree_scale.round().max(MIN_TEXT_RUN_SIZE_PT as f32) as u32;
        let color = if default_color.is_empty() {
            DEFAULT_TEXT_RUN_COLOR.to_string()
        } else {
            default_color.to_string()
        };
        section.text_runs.push(TextRun {
            start: 0,
            end,
            bold: false,
            italic: false,
            underline: false,
            font: String::new(),
            size_pt,
            color,
            hyperlink: None,
        });
        return true;
    }

    let mut changed = false;
    for run in section.text_runs.iter_mut() {
        let new_size = (run.size_pt as f32 + delta)
            .round()
            .max(MIN_TEXT_RUN_SIZE_PT as f32) as u32;
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
/// Limitations:
/// - `var(--name)` color references collapse to their resolved
///   hex on the round trip *unless* the prior run shares the
///   region's range — see the `prior_var_color` short-circuit
///   below.
/// - Unknown `AppFont` (corrupt tree state) falls through to
///   the empty string, matching the loader's tolerance for
///   missing-font runs.
///
/// Visible to [`super::super::nodes`] so the editor commit path
/// can reuse the converter through `set_section_text_and_runs`.
pub(crate) fn region_to_text_run(region: &ColorFontRegion, prior: Option<&TextRun>) -> TextRun {
    // Preserve `var(--name)` references when the prior run
    // shares the region's range and carries one. Without theme-
    // variables resolution at sync time we can't tell whether a
    // mutation deliberately recolored the run away from the
    // variable; trusting the prior keeps the variable reference
    // verbatim across mutations that didn't touch the color.
    // Same documented trade-off as the selective gate: a
    // deliberate `SetRegionColor` on a `var()`-bearing run is
    // silently swallowed here — the run keeps the variable.
    let prior_var_color: Option<&str> = prior.and_then(|p| {
        if is_var_ref(&p.color) && p.start == region.range.start && p.end == region.range.end {
            Some(p.color.as_str())
        } else {
            None
        }
    });
    let color = match (prior_var_color, region.color) {
        (Some(var_color), _) => var_color.to_string(),
        (None, Some(rgba)) => rgba_to_hex(rgba),
        (None, None) => prior
            .map(|p| p.color.clone())
            .unwrap_or_else(|| DEFAULT_TEXT_RUN_COLOR.to_string()),
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
            model_node.position.x = new_pos.0;
            model_node.position.y = new_pos.1;
            changed = true;
        }
        let node_pos_x = tree_px;
        let node_pos_y = tree_py;
        let node_size_x = model_node.size.width as f32;
        let node_size_y = model_node.size.height as f32;
        // Effective default color for a runless section, captured
        // before the section loop takes `&mut section` — used to
        // color a synthesized run when a font-size mutation lands
        // on a section that carries no runs (see
        // [`sync_section_font_size`]).
        let node_text_color = model_node.style.text_color.clone();

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
            let pre_round_trip_scale = {
                let max = section
                    .text_runs
                    .iter()
                    .map(|r| r.size_pt as f32)
                    .fold(0.0_f32, f32::max);
                if max > 0.0 {
                    max
                } else {
                    DEFAULT_TEXT_RUN_SIZE_PT as f32
                }
            };

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
            // Write `section.size` back when the model carries an
            // explicit size. `None` size means "fill the parent
            // node", which the tree resolves to the node's full
            // render_bounds — *don't* eagerly materialize it as
            // `Some(node.size)`, that would surprise authors who
            // chose the inheriting shape. Materialize only when the
            // tree's render_bounds diverges from the node's full
            // size (i.e. the mutation explicitly resized the
            // section, or the model already carried a Some).
            let tree_size_diverges = (snapshot.tree_size.0 - node_size_x).abs() > f32::EPSILON
                || (snapshot.tree_size.1 - node_size_y).abs() > f32::EPSILON;
            if section.size.is_some() || tree_size_diverges {
                // Project the model's current size to f32 (fill-parent
                // `None` resolves to the node's size, exactly as the
                // forward path does) and only rewrite when the tree's
                // post-mutation bounds actually diverge — comparing the
                // model `f64` against the tree `f32` directly would flag
                // a phantom change for any non-`f32`-exact size.
                let (cur_w, cur_h) = match section.size {
                    Some(s) => (s.width as f32, s.height as f32),
                    None => (node_size_x, node_size_y),
                };
                if cur_w != snapshot.tree_size.0 || cur_h != snapshot.tree_size.1 {
                    section.size = Some(baumhard::mindmap::model::Size {
                        width: snapshot.tree_size.0 as f64,
                        height: snapshot.tree_size.1 as f64,
                    });
                    changed = true;
                }
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
            let model_runs_by_range: rustc_hash::FxHashMap<(usize, usize), &TextRun> =
                section.text_runs.iter().map(|r| ((r.start, r.end), r)).collect();
            let model_regions_match = model_runs_by_range.len() == snapshot.regions.len()
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
                });
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
                        region_to_text_run(region, prior)
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
            if sync_section_font_size(
                section,
                snapshot.tree_scale,
                pre_round_trip_scale,
                &node_text_color,
            ) {
                changed = true;
            }
        }
        changed
    }
}

#[cfg(test)]
mod region_converter_tests {
    use super::{
        exact_or_dominant_overlap, region_to_text_run, DEFAULT_TEXT_RUN_COLOR, DEFAULT_TEXT_RUN_SIZE_PT,
    };
    use baumhard::core::primitives::{ColorFontRegion, Range};
    use baumhard::mindmap::model::TextRun;

    fn run(start: usize, end: usize, color: &str, font: &str) -> TextRun {
        TextRun {
            start,
            end,
            bold: false,
            italic: false,
            underline: false,
            font: font.into(),
            size_pt: 14,
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
            size_pt: 21,
            color: "#aabbcc".into(),
            hyperlink: Some("https://example.org".into()),
        }
    }

    #[test]
    fn test_region_to_text_run_merges_with_prior() {
        let region = ColorFontRegion::new(Range::new(0, 5), None, Some([1.0, 0.0, 0.0, 1.0]));
        let prior = styled_run(0, 5);
        let out = region_to_text_run(&region, Some(&prior));
        assert_eq!(out.start, 0);
        assert_eq!(out.end, 5);
        assert_eq!(out.color, "#ff0000");
        assert!(out.bold);
        assert!(out.italic);
        assert!(out.underline);
        assert_eq!(out.size_pt, 21);
        assert_eq!(out.hyperlink.as_deref(), Some("https://example.org"));
    }

    #[test]
    fn test_region_to_text_run_falls_back_to_defaults_without_prior() {
        let region = ColorFontRegion::new(Range::new(0, 5), None, None);
        let out = region_to_text_run(&region, None);
        assert!(!out.bold);
        assert!(!out.italic);
        assert!(!out.underline);
        assert_eq!(out.size_pt, DEFAULT_TEXT_RUN_SIZE_PT);
        assert_eq!(out.hyperlink, None);
        assert_eq!(out.font, "");
        assert_eq!(out.color, DEFAULT_TEXT_RUN_COLOR);
    }

    #[test]
    fn test_region_to_text_run_uses_region_color_without_prior() {
        let region = ColorFontRegion::new(Range::new(0, 3), None, Some([0.0, 1.0, 0.0, 1.0]));
        let out = region_to_text_run(&region, None);
        assert_eq!(out.color, "#00ff00");
    }

    #[test]
    fn test_region_to_text_run_preserves_var_color_when_range_matches() {
        let region = ColorFontRegion::new(Range::new(0, 5), None, Some([1.0, 0.0, 0.0, 1.0]));
        let prior_with_var = TextRun {
            color: "var(--accent)".into(),
            ..styled_run(0, 5)
        };
        let out = region_to_text_run(&region, Some(&prior_with_var));
        assert_eq!(out.color, "var(--accent)");
    }

    #[test]
    fn test_region_to_text_run_loses_var_color_on_range_change() {
        let region = ColorFontRegion::new(Range::new(0, 3), None, Some([1.0, 0.0, 0.0, 1.0]));
        let prior_with_var = TextRun {
            color: "var(--accent)".into(),
            ..styled_run(0, 5)
        };
        let out = region_to_text_run(&region, Some(&prior_with_var));
        assert_eq!(out.color, "#ff0000");
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
