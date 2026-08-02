// SPDX-License-Identifier: MPL-2.0

//! Section text / colour / font / runs / payload setters. Every
//! setter in this file routes through the shared envelope in
//! `undo_envelope.rs` — `mutate_section_with_style_undo` for the
//! formatting-only edits, `mutate_section_with_text_undo` for the
//! ones that rewrite `section.text` — and picks the
//! [`NodeEditTail`] its edit actually needs. Range-targeted
//! mutations additionally share `mutate_section_runs_in_range`.

use baumhard::mindmap::model::TextRun;

use super::super::defaults::default_text_run;
use super::super::MindMapDocument;
use super::NodeEditTail;
use super::SectionPayload;

impl MindMapDocument {
    /// Write both `text` and `text_runs` atomically, merging the
    /// editor's `ColorFontRegions` back to `Vec<TextRun>` via
    /// `region_to_text_run` so per-run attributes the regions
    /// don't carry (bold / italic / underline / hyperlink) survive
    /// the round trip.
    pub fn set_section_text_and_runs(
        &mut self,
        node_id: &str,
        section_idx: usize,
        new_text: String,
        new_regions: &baumhard::core::primitives::ColorFontRegions,
    ) -> bool {
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return false,
        };
        let Some(section) = node.sections.get(section_idx) else {
            return false;
        };
        // Empty regions: fall back to `set_section_text` so a
        // plaintext-only edit doesn't wipe template-inherited
        // runs the editor never touched.
        if new_regions.all_regions().is_empty() {
            return self.set_section_text(node_id, section_idx, new_text);
        }
        let prior_runs: Vec<&TextRun> = section.text_runs.iter().collect();
        let new_runs: Vec<TextRun> = new_regions
            .all_regions()
            .iter()
            .map(|region| {
                let prior = super::super::custom::sync::exact_or_dominant_overlap(
                    &prior_runs,
                    region.range.start,
                    region.range.end,
                );
                super::super::custom::sync::region_to_text_run(region, prior)
            })
            .collect();
        self.mutate_section_with_text_undo(node_id, section_idx, NodeEditTail::Grow, move |s| {
            if s.text == new_text && s.text_runs == new_runs {
                return false;
            }
            s.text = new_text;
            s.text_runs = new_runs;
            clamp_runs_to_text(s);
            true
        })
    }

    /// Replace the section's `text` while preserving as much of
    /// the existing `text_runs` as the new text supports. Runs
    /// wholly inside the new text length carry through unchanged;
    /// runs that straddle the new end get clipped at the new
    /// `grapheme_count`; runs entirely past the new end are
    /// dropped. Uncovered ranges (anything past the last surviving
    /// run's `end`) fall through to section / node defaults per
    /// `format/text-runs.md`. No-op (returns `false`, no undo push)
    /// when the section doesn't exist or its text already matches.
    ///
    /// Distinct from [`Self::set_section_text`] which collapses
    /// every prior run to a single run cloned from
    /// `text_runs.first()` — that path is the right shape for
    /// "I want one uniform style on the new text"; this path is
    /// the right shape for "I want my multi-run styling to
    /// survive a text rewrite to the extent the new text covers
    /// the same graphemes". Backs the
    /// `section text "<text>" runs=preserve` console path.
    pub fn set_section_text_preserving_runs(
        &mut self,
        node_id: &str,
        section_idx: usize,
        new_text: String,
    ) -> bool {
        self.mutate_section_with_text_undo(node_id, section_idx, NodeEditTail::Grow, move |s| {
            if s.text == new_text {
                return false;
            }
            let new_grapheme_count = baumhard::util::grapheme_chad::count_grapheme_clusters(&new_text);
            // Clip runs to the new text length: keep runs whose
            // `start < new_grapheme_count`; clamp `end` down to
            // `new_grapheme_count`. Runs entirely past the new end
            // (start >= new_grapheme_count) drop out. The
            // text_run_ops invariants (sorted, no-overlap,
            // half-open) are preserved by clamping in-place.
            s.text_runs.retain_mut(|r| {
                if r.start >= new_grapheme_count {
                    return false;
                }
                if r.end > new_grapheme_count {
                    r.end = new_grapheme_count;
                }
                // After clamping, a run with start == end is
                // degenerate; drop it (the clamp can collapse a
                // run when the new text ends exactly at its
                // start).
                r.start < r.end
            });
            s.text = new_text;
            true
        })
    }

    /// Replace the section's `text`, collapsing every prior run to
    /// a single run spanning the new text and cloned from
    /// `text_runs.first()` (or the authoring defaults when the
    /// section carries no runs). See
    /// [`Self::set_section_text_preserving_runs`] for the
    /// run-preserving sibling.
    pub fn set_section_text(&mut self, node_id: &str, section_idx: usize, new_text: String) -> bool {
        self.mutate_section_with_text_undo(node_id, section_idx, NodeEditTail::Grow, move |s| {
            if s.text == new_text {
                return false;
            }
            let count = baumhard::util::grapheme_chad::count_grapheme_clusters(&new_text);
            let template = s
                .text_runs
                .first()
                .cloned()
                .unwrap_or_else(|| default_text_run(0));
            // Empty text yields an empty runs vec; a `TextRun {
            // start: 0, end: 0 }` would violate the
            // `text_run_ops` invariant `start < end` and panic in
            // debug builds on subsequent slice / splice /
            // find_run_containing calls.
            s.text_runs = if count == 0 {
                Vec::new()
            } else {
                vec![TextRun {
                    start: 0,
                    end: count,
                    ..template
                }]
            };
            s.text = new_text;
            true
        })
    }

    /// Rewrite every run on the section that matches the cascade
    /// predicate (unanimous run colour, or the node's
    /// `style.text_color` default) to `color`. Mixed-colour
    /// sections preserve their non-predicate runs. The node's own
    /// `style.text_color` is never touched.
    pub fn set_section_text_color(&mut self, node_id: &str, section_idx: usize, color: String) -> bool {
        // The predicate's fallback is the *node's*
        // `style.text_color`, so this one reaches for the
        // node-scoped envelope rather than the section wrapper.
        // `NodeEditTail::None`: color never shifts a glyph
        // advance.
        self.mutate_node_with_style_undo(node_id, NodeEditTail::None, move |node| {
            let node_default = node.style.text_color.clone();
            let section = node.sections.get_mut(section_idx)?;
            let predicate_color = section
                .text_runs
                .first()
                .filter(|first| section.text_runs.iter().all(|r| r.color == first.color))
                .map(|r| r.color.clone())
                .unwrap_or(node_default);
            let any_run_changes = section
                .text_runs
                .iter()
                .any(|r| r.color == predicate_color && r.color != color);
            if !any_run_changes {
                return None;
            }
            for run in section.text_runs.iter_mut() {
                if run.color == predicate_color {
                    run.color = color.clone();
                }
            }
            Some(())
        })
        .is_some()
    }

    /// Set the font size on one section's runs (bounded sibling
    /// of the whole-node [`Self::set_node_font_size`]). Rewrites
    /// every run's `size_pt` on the targeted section; sibling
    /// sections stay untouched. Triggers the same monotonic
    /// `grow_one_node_to_fit_text` floor as the whole-node setter
    /// — sections share the node's AABB, so a larger run on one
    /// section can grow the node.
    pub fn set_section_font_size(&mut self, node_id: &str, section_idx: usize, size_pt: f32) -> bool {
        if !size_pt.is_finite() {
            return false;
        }
        let size_u = size_pt.round().max(1.0) as u32;
        self.mutate_section_with_style_undo(node_id, section_idx, NodeEditTail::Grow, move |s| {
            if s.text_runs.iter().all(|r| r.size_pt == size_u) {
                return false;
            }
            for run in s.text_runs.iter_mut() {
                run.size_pt = size_u;
            }
            true
        })
    }

    /// Set the font family on one section's runs (bounded sibling
    /// of the whole-node [`Self::set_node_font_family`]).
    /// `Some(name)` pins each run to that family on the targeted
    /// section; `None` clears the pin. Triggers the same monotonic
    /// `grow_one_node_to_fit_text` re-measure as the whole-node
    /// setter — face changes can shift advance widths.
    pub fn set_section_font_family(
        &mut self,
        node_id: &str,
        section_idx: usize,
        family: Option<&str>,
    ) -> bool {
        let target = family.unwrap_or("").to_string();
        self.mutate_section_with_style_undo(node_id, section_idx, NodeEditTail::Grow, move |s| {
            if s.text_runs.iter().all(|r| r.font == target) {
                return false;
            }
            for run in s.text_runs.iter_mut() {
                run.font = target.clone();
            }
            true
        })
    }

    // ── Range-targeted section setters ─────────────────────────
    //
    // Range-aware mirrors of the uniform setters above; route
    // through `text_run_ops::mutate_in_range`.

    /// Set the text colour on a sub-range of one section's text.
    /// Bounded sibling of [`Self::set_section_text_color`] — that
    /// setter rewrites every run uniformly, this one targets
    /// `[range_start, range_end)` graphemes only. Ranges that
    /// partially or wholly cross uncovered gaps fill the gap
    /// with a fresh run inheriting the section / node cascade
    /// defaults plus the new colour, so the user's "make these
    /// graphemes red" intent is honoured even where no run
    /// exists today.
    ///
    /// `range_end` is clamped to the section's grapheme count;
    /// callers don't need to pre-clamp. No-op when the section
    /// is missing, the range is empty after clamping, or the
    /// post-mutation runs are unchanged from the pre-mutation
    /// runs.
    pub fn set_section_text_color_range(
        &mut self,
        node_id: &str,
        section_idx: usize,
        range_start: usize,
        range_end: usize,
        color: String,
    ) -> bool {
        // Text colour doesn't affect glyph advance — no grow.
        self.mutate_section_runs_in_range(
            node_id,
            section_idx,
            range_start,
            range_end,
            NodeEditTail::None,
            |r| r.color = color.clone(),
        )
    }

    /// Set the font size on a sub-range of one section's text.
    /// Triggers `grow_one_node_to_fit_text` — larger runs can
    /// grow the node.
    pub fn set_section_font_size_range(
        &mut self,
        node_id: &str,
        section_idx: usize,
        range_start: usize,
        range_end: usize,
        size_pt: f32,
    ) -> bool {
        if !size_pt.is_finite() {
            return false;
        }
        let size_u = size_pt.round().max(1.0) as u32;
        self.mutate_section_runs_in_range(
            node_id,
            section_idx,
            range_start,
            range_end,
            NodeEditTail::Grow,
            move |r| r.size_pt = size_u,
        )
    }

    /// Set the font family on a sub-range of one section's text.
    /// `Some(name)` pins each in-range run; `None` clears the pin
    /// (empty string = inherit cascade). Triggers grow — face
    /// changes shift advance widths.
    pub fn set_section_font_family_range(
        &mut self,
        node_id: &str,
        section_idx: usize,
        range_start: usize,
        range_end: usize,
        family: Option<&str>,
    ) -> bool {
        let target = family.unwrap_or("").to_string();
        self.mutate_section_runs_in_range(
            node_id,
            section_idx,
            range_start,
            range_end,
            NodeEditTail::Grow,
            move |r| r.font = target.clone(),
        )
    }

    /// Per-attribute range-aware setter shell. Clamps the range,
    /// applies `mutate_run` to every in-range run (and to the
    /// template that fills uncovered gaps), and reports an
    /// **honest** change verdict from inside the envelope: the
    /// pre-mutation runs are compared against the post-mutation
    /// runs before the closure returns, so a range edit that
    /// lands on already-matching runs is backed out and pushes
    /// nothing.
    ///
    /// Pre-fix this snapshotted outside the envelope, committed
    /// unconditionally, then reached for `undo_stack.pop()` on a
    /// no-op — which left `dirty = true` behind and broke the
    /// undo-LIFO invariant if any other entry slipped in between.
    /// That anti-pattern is exactly what this file's own header
    /// condemns.
    fn mutate_section_runs_in_range<F>(
        &mut self,
        node_id: &str,
        section_idx: usize,
        range_start: usize,
        range_end: usize,
        tail: NodeEditTail,
        mut mutate_run: F,
    ) -> bool
    where
        F: FnMut(&mut baumhard::mindmap::model::TextRun),
    {
        let (clamped_end, mut template) =
            match self.clamp_range_and_build_template(node_id, section_idx, range_end) {
                Some(pair) => pair,
                None => return false,
            };
        if range_start >= clamped_end {
            return false;
        }
        mutate_run(&mut template);
        self.mutate_section_with_style_undo(node_id, section_idx, tail, move |s| {
            let pre = s.text_runs.clone();
            baumhard::mindmap::model::text_run_ops::mutate_in_range(
                &mut s.text_runs,
                range_start,
                clamped_end,
                &template,
                &mut mutate_run,
            );
            s.text_runs != pre
        })
    }

    /// Range-setter pre-flight: clamps `range_end` to the
    /// section's grapheme count and builds the gap-fill
    /// template from the section's first run (cascade source)
    /// or the authoring defaults recolored to the node's
    /// `style.text_color` when the section has no runs. Caller
    /// overwrites the one attribute it's setting.
    fn clamp_range_and_build_template(
        &self,
        node_id: &str,
        section_idx: usize,
        range_end: usize,
    ) -> Option<(usize, baumhard::mindmap::model::TextRun)> {
        let node = self.mindmap.nodes.get(node_id)?;
        let section = node.sections.get(section_idx)?;
        let total = baumhard::util::grapheme_chad::count_grapheme_clusters(&section.text);
        let clamped_end = range_end.min(total);
        let template = section.text_runs.first().cloned().unwrap_or_else(|| TextRun {
            color: node.style.text_color.clone(),
            ..default_text_run(0)
        });
        Some((clamped_end, template))
    }

    /// Atomically replace one section's full payload (text +
    /// runs + offset + size + channel + bindings) under a single
    /// `EditNodeStyle` undo entry — a single Ctrl+Z restores the
    /// pre-write shape. Returns `true` on a real change; no-op
    /// when the section is missing or every field matches.
    ///
    /// Uses the *style* envelope even though it rewrites `text`,
    /// which cuts across the usual text/style split. Deliberate,
    /// and unchanged from before the envelope fold: the payload
    /// spans geometry (`offset`, `size`) and `channel` alongside
    /// the text, and `EditNodeText` carries no `before_style`, so
    /// it could not reverse the whole write. `EditNodeStyle`
    /// restores a superset and is the correct variant here, not
    /// merely the convenient one.
    pub fn apply_section_payload(
        &mut self,
        node_id: &str,
        section_idx: usize,
        text: String,
        payload: &SectionPayload,
    ) -> bool {
        let payload = payload.clone();
        self.mutate_section_with_style_undo(node_id, section_idx, NodeEditTail::Grow, move |s| {
            let unchanged = s.text == text
                && s.text_runs == payload.text_runs
                && s.offset == payload.offset
                && s.size == payload.size
                && s.channel == payload.channel
                && s.trigger_bindings == payload.trigger_bindings;
            if unchanged {
                return false;
            }
            s.text = text;
            s.text_runs = payload.text_runs;
            s.offset = payload.offset;
            s.size = payload.size;
            s.channel = payload.channel;
            s.trigger_bindings = payload.trigger_bindings;
            // Defensive: a future caller might pass mismatched
            // (text, runs) — the copy site never does, but the
            // public setter shouldn't trust its input enough to
            // leave runs whose ranges exceed the new text length.
            clamp_runs_to_text(s);
            true
        })
    }
}

/// Clamp a section's `text_runs` against its current text length
/// in grapheme clusters, dropping runs that became degenerate
/// (`start >= end`) and shrinking trailing runs that overshoot the
/// text. Defensive guard the per-section style setters call before
/// rewriting `color` / `size_pt` / `font` on each run — a previous
/// tree-walker mutation that shortened `section.text` may have
/// left runs whose `end` exceeds the current grapheme count, which
/// `cosmic_text` either ignores or panics on depending on build.
///
/// Cost: O(runs.len() * text grapheme count) — one
/// `count_grapheme_clusters` call per section, plus a linear pass
/// over the runs. Trivial for typical single-run sections.
pub(in crate::application::document) fn clamp_runs_to_text(
    section: &mut baumhard::mindmap::model::MindSection,
) {
    let max_end = baumhard::util::grapheme_chad::count_grapheme_clusters(&section.text);
    section.text_runs.retain_mut(|run| {
        if run.start >= max_end {
            return false;
        }
        if run.end > max_end {
            run.end = max_end;
        }
        run.start < run.end
    });
}
