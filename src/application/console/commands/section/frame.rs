// SPDX-License-Identifier: MPL-2.0

//! `section frame …` — configure a section's per-section frame
//! border style.
//!
//! Mirrors the [`crate::application::console::commands::border`]
//! kv vocabulary (`preset`, `font`, `size`, `color`, `palette`,
//! `field`, `padding`, `top`, `bottom`, `left`, `right`, `tl`,
//! `tr`, `bl`, `br`) but writes to
//! [`baumhard::mindmap::model::MindSection::frame_border`] instead
//! of `MindNode.style.border`. Same parsing, same auto-promotion
//! behaviour (any `top=` / corner edit promotes `preset` to
//! `"custom"`), same per-side pattern grammar.
//!
//! Section frames don't carry a visibility flag — they're drawn
//! whenever the owning node is in `InteractionMode::NodeEdit`, so
//! there's no `section frame on` / `off`. Subverbs are:
//!
//! - `section frame show` — readout of the resolved frame style.
//! - `section frame reset` — drop the per-section override (falls
//!   back to `Canvas.default_section_frame_border` and then to the
//!   hardcoded floor).
//! - kv form: any combination of the keys above.
//!
//! The target section is resolved from the current selection
//! (`SelectionState::Section` / `SectionRange`) or via an explicit
//! `section=K` kv when the selection is `Single(node_id)`.

use baumhard::mindmap::border::resolve_section_frame_border;

use crate::application::console::commands::border::{
    custom_preset_hint, edits_has_glyph_field, nodes_in_selection, stage_kv, KEYS as BORDER_KEYS,
};
use crate::application::console::completion::{
    kv_key_completions_with_hints, prefix_filter, Completion, CompletionContext, CompletionState,
};
use crate::application::console::parser::Args;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::{
    BorderConfigEdits, BorderEditOutcome, OptionEdit, SectionSel, SelectionState,
};

/// Subverbs surfaced as token-2 completions after `section frame`.
pub const VERBS: &[&str] = &["show", "reset", "preview"];

pub fn complete_section_frame(state: &CompletionState, ctx: &ConsoleContext) -> Vec<Completion> {
    // After `section frame preview ` the user gets `commit` /
    // `cancel` plus the kv keys (preview accepts the same
    // vocabulary as the committing kv-form). The engine's
    // `Token { index }` counts past the parent command, so
    // `section frame preview <here>` lands at index 2.
    let after_preview = state.tokens.get(2).map(String::as_str) == Some("preview");
    match &state.context {
        // The engine's `Token { index }` is the count of non-kv
        // positionals *after* the command name, so for the input
        // `section frame ` the cursor sits at `index: 1`. (`index: 0`
        // is for `section <here>` — handled by `complete_section`.)
        // Anything past the `frame` subverb (so `index >= 1`) accepts
        // the same kv keyset the top-level `border …` verb does.
        CompletionContext::Token { index: 1 } => {
            let mut out = prefix_filter(VERBS, state.partial);
            out.extend(kv_key_completions_with_hints(BORDER_KEYS, state.partial, kv_hint));
            out
        }
        CompletionContext::Token { index: 2 } if after_preview => {
            // C12: surface commit/cancel with their hints (shared
            // helper) so the popup tells users what each does.
            let mut out =
                crate::application::console::commands::border::preview_subverb_completions(state.partial);
            out.extend(kv_key_completions_with_hints(BORDER_KEYS, state.partial, kv_hint));
            out
        }
        CompletionContext::Token { index: i } if *i > 1 => {
            kv_key_completions_with_hints(BORDER_KEYS, state.partial, kv_hint)
        }
        // KvValue completions for `preset=` / `palette=` / `font=` /
        // `color=` / `field=`. Mirror `border/complete.rs` so the
        // popup vocabulary is identical regardless of which border
        // surface (node / section / canvas) the user is editing.
        CompletionContext::KvValue { key } => {
            crate::application::console::commands::border::kv_value_completions(
                key.as_str(),
                state.partial,
                ctx,
            )
        }
        _ => Vec::new(),
    }
}

/// Per-key hint table — delegates to the shared
/// [`super::super::border::kv_hint`] so `border …`,
/// `section frame …`, and `canvas …` surface identical hints.
fn kv_hint(key: &str) -> Option<&'static str> {
    super::super::border::kv_hint(key)
}

/// Entry point dispatched from `section/mod.rs::execute_section`
/// when the user typed `section frame …`. Args still includes the
/// `frame` token at positional(0) (the parent dispatcher only
/// consumed `section`); we read positional(1) to peek at the
/// optional `show` / `reset` subverb.
pub fn execute_section_frame(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    if let Some(verb) = args.positional(1) {
        // C14: case-insensitive match — same posture as `border
        // preview commit` / `cancel` already use, and as the
        // committing `border …` verb arms. Without normalising
        // here `Show` / `RESET` / `Preview` would route through
        // the kv-form path and produce a confusing error.
        match verb.to_ascii_lowercase().as_str() {
            "show" => return execute_show(args, eff),
            "reset" => return apply_reset(args, eff),
            "preview" => return execute_section_frame_preview(args, eff),
            other if !other.contains('=') => {
                if args.kvs().next().is_some() {
                    return ExecResult::err(format!(
                        "section frame: unexpected positional '{}' alongside a kv pair — \
                         did you mean to quote a multi-word value? \
                         e.g. `section frame palette=\"{}\"`",
                        verb, verb
                    ));
                }
                return ExecResult::err(format!(
                    "section frame: unknown subverb '{}'\n  \
                     positional today: show | reset | preview\n  \
                     per-field grammar lives in the kv form for now — \
                     `section frame preset=heavy padding=8` etc. Per-node \
                     `border` and `canvas border` accept positional \
                     `preset / color / padding / palette / font / side / corner`; \
                     section-frame parity is tracked as a follow-up.\n  \
                     staged: preview <kv>=… | preview commit | preview cancel\n  \
                     composed: <key>=<value> [<key>=<value> …]",
                    verb
                ));
            }
            _ => {}
        }
    }

    let mut edits = BorderConfigEdits::default();
    let mut saw_any = false;
    for (k, v) in args.kvs() {
        if k == "section" {
            // The `section=K` kv targets the section to write to;
            // it's not a border field. Skip it on the staging
            // pass — the resolver below consumes it separately.
            continue;
        }
        saw_any = true;
        if let Err(e) = stage_kv(&mut edits, k, v) {
            return ExecResult::err(e);
        }
    }
    if !saw_any {
        return ExecResult::err(
            "usage: section frame show|reset | section frame <key>=<value> … [section=<idx>]",
        );
    }
    apply_edits(args, eff, edits)
}

fn apply_reset(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let edits = BorderConfigEdits {
        clear: true,
        ..BorderConfigEdits::default()
    };
    apply_edits(args, eff, edits)
}

fn apply_edits(args: &Args, eff: &mut ConsoleEffects, edits: BorderConfigEdits) -> ExecResult {
    // Surface the specific not-applicable variant from
    // `nodes_in_selection` rather than collapsing all five branches
    // (no selection / edge / edge-label / portal-label / portal-text)
    // into a single misleading "select a section" message.
    let node_ids = match nodes_in_selection(&eff.document.selection, "section frame") {
        Ok(ids) => ids,
        Err(e) => return e,
    };
    let kv_idx = match parse_section_kv(args) {
        Ok(v) => v,
        Err(msg) => return ExecResult::err(msg),
    };

    let bare_custom = matches!(
        edits.preset,
        OptionEdit::Set(ref s) if s.eq_ignore_ascii_case("custom")
    ) && !edits_has_glyph_field(&edits);

    // Parse-then-dispatch: resolve every (node_id, section_idx)
    // pair and verify the section exists BEFORE any mutation. A
    // mid-loop error after some nodes had their `frame_border`
    // written would leave the document in a half-mutated state with
    // an undo entry per touched node — undo would only reverse one
    // node at a time and the user would see the error message
    // hiding a partial commit.
    let mut targets: Vec<(String, usize)> = Vec::with_capacity(node_ids.len());
    for node_id in &node_ids {
        let n_sections = eff
            .document
            .mindmap
            .nodes
            .get(node_id)
            .map(|n| n.sections.len())
            .unwrap_or(0);
        let section_idx = match resolve_section_idx_for(&eff.document.selection, node_id, kv_idx, n_sections)
        {
            Ok(idx) => idx,
            Err(msg) => return ExecResult::err(msg),
        };
        let count = eff
            .document
            .mindmap
            .nodes
            .get(node_id)
            .map(|n| n.sections.len())
            .unwrap_or(0);
        if section_idx >= count {
            return ExecResult::err(format!(
                "section[{}] not found on node '{}'",
                section_idx, node_id
            ));
        }
        targets.push((node_id.clone(), section_idx));
    }

    // Parity with `border` / `canvas border`: glyph-field writes
    // (per-side / per-corner overrides) only render against a
    // `custom` preset. Pre-fix the kv form silently auto-
    // promoted at the data layer; the verb-layer pre-check makes
    // the intent explicit and matches the per-node verb's posture.
    if edits_has_glyph_field(&edits) {
        for (node_id, section_idx) in &targets {
            let section = eff
                .document
                .mindmap
                .nodes
                .get(node_id)
                .and_then(|n| n.sections.get(*section_idx));
            let resolved_preset = section
                .and_then(|s| s.frame_border.as_ref())
                .map(|c| c.preset.as_str())
                .or_else(|| {
                    eff.document
                        .mindmap
                        .canvas
                        .default_section_frame_border
                        .as_ref()
                        .map(|c| c.preset.as_str())
                })
                .unwrap_or("light");
            if !resolved_preset.eq_ignore_ascii_case("custom") {
                return ExecResult::err(format!(
                    "section frame: cannot set side / corner glyph against \
                     preset '{}' on section[{}] of node '{}'. Run \
                     `section frame preset=custom` first, then set the glyph.",
                    resolved_preset, section_idx, node_id
                ));
            }
        }
    }

    let mut changed = 0usize;
    let mut auto_promoted: Option<String> = None;
    for (node_id, section_idx) in &targets {
        let outcome: BorderEditOutcome =
            eff.document
                .set_section_frame_border_config(node_id, *section_idx, edits.clone());
        if outcome.changed {
            changed += 1;
        }
        if outcome.preset_auto_promoted && auto_promoted.is_none() {
            auto_promoted = outcome.requested_preset.clone();
        }
    }

    let mut lines: Vec<String> = Vec::new();
    if changed == 0 {
        if bare_custom {
            lines.push("section frame: preset=custom set; no glyph fields were given".into());
            lines.push(custom_preset_hint("section frame"));
            return ExecResult::lines(lines);
        }
        return ExecResult::ok_msg("section frame: no change");
    }
    lines.push(format!("section frame applied to {} section(s)", changed));
    if let Some(name) = auto_promoted {
        lines.push(format!(
            "note: preset='{}' auto-promoted to 'custom' \
             (a side or corner glyph was set; non-custom presets \
             ignore the per-section glyph override)",
            name
        ));
    }
    if bare_custom {
        lines.push(custom_preset_hint("section frame"));
    }
    if lines.len() == 1 {
        ExecResult::ok_msg(lines.into_iter().next().expect("len==1"))
    } else {
        ExecResult::lines(lines)
    }
}

fn execute_show(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // Surface the specific not-applicable variant — same shape as
    // `apply_edits`. See its comment for the rationale.
    let node_ids = match nodes_in_selection(&eff.document.selection, "section frame show") {
        Ok(ids) => ids,
        Err(e) => return e,
    };
    if node_ids.len() != 1 {
        return ExecResult::err("section frame show: single-section target only; pick one section first");
    }
    let node_id = node_ids.into_iter().next().expect("len==1");
    let kv_idx = match parse_section_kv(args) {
        Ok(v) => v,
        Err(msg) => return ExecResult::err(msg),
    };
    let n_sections = eff
        .document
        .mindmap
        .nodes
        .get(&node_id)
        .map(|n| n.sections.len())
        .unwrap_or(0);
    let section_idx = match resolve_section_idx_for(&eff.document.selection, &node_id, kv_idx, n_sections) {
        Ok(idx) => idx,
        Err(msg) => return ExecResult::err(msg),
    };
    let map = &eff.document.mindmap;
    let Some(node) = map.nodes.get(&node_id) else {
        return ExecResult::err(format!("section frame show: node '{}' not found", node_id));
    };
    let Some(section) = node.sections.get(section_idx) else {
        return ExecResult::err(format!(
            "section[{}] not found on node '{}'",
            section_idx, node_id
        ));
    };

    // Whether this specific section is currently focused at render
    // time is interaction-mode runtime state (NodeEdit + cursor),
    // not document state, so the verb path can't tell. Surface
    // BOTH cascade branches so the user sees what unfocused and
    // focused frames will actually look like — same posture as
    // `canvas section-frame show` vs `canvas section-frame focused
    // show`. C10 fix: the prior shape hardcoded `focused=false`
    // and silently lied for sections being edited.
    let mut lines = vec![format!(
        "section frame: node='{}' section={}",
        node_id, section_idx
    )];
    push_resolved_section_frame(&mut lines, section, &map.canvas, /* focused */ false);
    push_resolved_section_frame(&mut lines, section, &map.canvas, /* focused */ true);
    ExecResult::lines(lines)
}

/// Append the resolved-style readout for one focus state. The
/// labelled `source` walks the cascade the same way
/// `resolve_section_frame_border` does — per-section override,
/// then focused canvas default (focused branch only), then
/// unfocused canvas default (focused branch falls through to
/// it), then the hardcoded floor.
fn push_resolved_section_frame(
    lines: &mut Vec<String>,
    section: &baumhard::mindmap::model::MindSection,
    canvas: &baumhard::mindmap::model::Canvas,
    focused: bool,
) {
    use baumhard::mindmap::model::GlyphBorderConfig;
    let header = if focused { "focused" } else { "unfocused" };
    let source = if section.frame_border.is_some() {
        "per-section override"
    } else if focused {
        match (
            canvas.default_focused_section_frame_border.is_some(),
            canvas.default_section_frame_border.is_some(),
        ) {
            (true, _) => "focused canvas default",
            (false, true) => "unfocused canvas default (focused fallback)",
            (false, false) => "hardcoded heavy floor",
        }
    } else if canvas.default_section_frame_border.is_some() {
        "unfocused canvas default"
    } else {
        "hardcoded light floor"
    };
    let resolved = resolve_section_frame_border(section, canvas, focused, "#00E5FF");
    // Cascade pick — same logic `resolve_section_frame_border`
    // walks. We re-derive it for the per-side / per-corner readout
    // because `resolve_border_style` discards the source `glyphs`
    // (it bakes them into resolved corners only when `preset =
    // custom`); we need the source slot to surface what the
    // author wrote regardless of preset.
    let canvas_default_for_focus: Option<&GlyphBorderConfig> = if focused {
        canvas
            .default_focused_section_frame_border
            .as_ref()
            .or(canvas.default_section_frame_border.as_ref())
    } else {
        canvas.default_section_frame_border.as_ref()
    };
    let chosen_cfg: Option<&GlyphBorderConfig> = section.frame_border.as_ref().or(canvas_default_for_focus);
    lines.push(format!("  [{}]", header));
    lines.push(format!("    source:    {}", source));
    lines.push(format!(
        "    preset:    {}",
        chosen_cfg
            .map(|c| c.preset.as_str())
            .unwrap_or(if focused { "heavy" } else { "light" })
    ));
    lines.push(format!(
        "    font:      {}",
        resolved.font_name.as_deref().unwrap_or("(default)")
    ));
    lines.push(format!("    size:      {} pt", resolved.font_size_pt));
    lines.push(format!("    color:     {}", resolved.color));
    lines.push(format!(
        "    palette:   {}",
        resolved
            .color_palette
            .as_deref()
            .map(|n| format!("{} (field={})", n, resolved.palette_field.as_str()))
            .unwrap_or_else(|| "(none)".into())
    ));
    if let Some(g) = chosen_cfg.and_then(|c| c.glyphs.as_ref()) {
        lines.push(format!("    top:       {}", g.top));
        lines.push(format!("    bottom:    {}", g.bottom));
        lines.push(format!("    left:      {}", g.left));
        lines.push(format!("    right:     {}", g.right));
        lines.push(format!(
            "    corners:   tl={}  tr={}  bl={}  br={}",
            g.top_left, g.top_right, g.bottom_left, g.bottom_right
        ));
    } else {
        let preset = chosen_cfg
            .map(|c| c.preset.as_str())
            .unwrap_or(if focused { "heavy" } else { "light" });
        lines.push(format!("    glyphs:    (preset '{}' defaults)", preset));
    }
}

fn parse_section_kv(args: &Args) -> Result<Option<usize>, String> {
    for (k, v) in args.kvs() {
        if k == "section" {
            return super::super::range_kv::parse_section_kv("section", v).map(Some);
        }
    }
    Ok(None)
}

/// Resolve `(node_id, section_idx)` for a `section frame …` write.
///
/// Cascade matching `section/mod.rs::resolve_section_idx`'s rule
/// table (kept in sync; review-fix CRIT-2 closed the divergence
/// where this resolver lacked the rule-3 single-section
/// auto-resolve, so `section frame preset=heavy` against a
/// `Single(node)` with one section spuriously errored "node 'X'
/// has multiple sections" while `section preset=heavy` worked):
///
/// 1. `kv_idx` (from a `section=K` kv) → that idx.
/// 2. `Section` / `SectionRange` whose `node_id` matches → that idx.
/// 3. `n_sections == 1` (caller passes the section count for the
///    target node) → `Ok(0)`. Mirrors `section/mod.rs` rule 3.
/// 4. Multi-section node with no Section selection → error with
///    the count + the `section=<idx>` hint.
fn resolve_section_idx_for(
    sel: &SelectionState,
    node_id: &str,
    kv_idx: Option<usize>,
    n_sections: usize,
) -> Result<usize, String> {
    if let Some(idx) = kv_idx {
        return Ok(idx);
    }
    match sel {
        SelectionState::Section(SectionSel {
            node_id: nid,
            section_idx,
        }) if nid == node_id => Ok(*section_idx),
        SelectionState::SectionRange {
            sel: SectionSel {
                node_id: nid,
                section_idx,
            },
            ..
        } if nid == node_id => Ok(*section_idx),
        _ if n_sections == 1 => Ok(0),
        _ => Err(format!(
            "section frame: node '{}' has {} sections — pick one (click) \
             or pass section=<idx>",
            node_id, n_sections
        )),
    }
}

// `edits_has_glyph_field` and `custom_preset_hint` are shared with
// the `border …` and `canvas …` verbs through
// `border::edits_has_glyph_field` / `border::custom_preset_hint(label)`.
// They used to live here as byte-identical copies — see CODE_CONVENTIONS.md
// §5 for why that's forbidden.

/// `section frame preview …` — same kv vocabulary, no model
/// write. Routes to the shared
/// [`crate::application::console::commands::border::dispatch_border_preview`]
/// with a section-target resolver. Live `Section` /
/// `SectionRange` / `MultiSection` selections are turned into
/// the matching `(node_id, section_idx)` pairs; `Single(node_id)`
/// requires an explicit `section=K` kv (mirroring the committing
/// `section frame …` verb's posture). The preview's
/// `selection_snapshot` rides on `self.selection` at set time.
fn execute_section_frame_preview(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    use crate::application::document::BorderPreviewTarget;

    // The `section=K` kv (if any) overrides the selection's
    // section index — same shape `apply_edits` uses, copied here
    // because the preview path's target is fixed at dispatch
    // time (not inferred per-target like the committing path).
    let kv_idx = match parse_section_kv(args) {
        Ok(v) => v,
        Err(msg) => return ExecResult::err(msg),
    };
    // Snapshot per-node section counts before entering the closure
    // so the preview-side `resolve_section_idx_for` can apply the
    // single-section auto-resolve (CRIT-2: pre-fix `Single(node)`
    // with one section spuriously errored). Cloning the model into
    // a closure-captured map is cheaper than re-borrowing
    // `eff.document` inside the closure (the closure is `FnOnce`,
    // doc is already borrowed mutably for the dispatch_border_preview
    // call).
    let section_counts: std::collections::HashMap<String, usize> = eff
        .document
        .mindmap
        .nodes
        .iter()
        .map(|(id, n)| (id.clone(), n.sections.len()))
        .collect();
    super::super::border::dispatch_border_preview(
        args,
        eff,
        "section frame preview",
        /* subverb_pos */ 2,
        move |sel| {
            // `MultiSection([(A,0),(A,1),(B,0)])` already encodes
            // distinct (node_id, section_idx) pairs — drop them
            // straight into `BorderPreviewTarget::Sections` rather
            // than collapsing to node-ids and re-resolving via
            // `resolve_section_idx_for` (which doesn't know how
            // to pick one section idx out of a multi-pair shape).
            // Mirrors the resolver `apply_set_border_preview` in
            // `cross_dispatch/style.rs` already uses for the same
            // case.
            if let SelectionState::MultiSection(sels) = sel {
                let pairs: Vec<(String, usize)> =
                    sels.iter().map(|s| (s.node_id.clone(), s.section_idx)).collect();
                return Ok(BorderPreviewTarget::Sections(pairs));
            }
            let node_ids = nodes_in_selection(sel, "section frame preview")?;
            let mut pairs: Vec<(String, usize)> = Vec::with_capacity(node_ids.len());
            for nid in &node_ids {
                let n_sections = section_counts.get(nid).copied().unwrap_or(0);
                let idx = match resolve_section_idx_for(sel, nid, kv_idx, n_sections) {
                    Ok(i) => i,
                    Err(msg) => {
                        return Err(crate::application::console::ExecResult::err(msg));
                    }
                };
                pairs.push((nid.clone(), idx));
            }
            Ok(BorderPreviewTarget::Sections(pairs))
        },
    )
}

#[cfg(test)]
mod tests {
    use crate::application::console::tests::fixtures::{
        assert_exec_err_contains, assert_exec_ok, assert_exec_ok_strict, run,
    };
    use crate::application::console::ExecResult;
    use crate::application::document::tests_common::{load_test_doc, pinned_two_section_node};
    use crate::application::document::{SectionSel, SelectionState};

    /// Whole-PR review CRIT-2: `section frame preset=heavy`
    /// against a `Single(node)` selection where the node has
    /// exactly one section auto-resolves to that section. Pre-fix
    /// the resolver lacked the rule-3 single-section auto-resolve
    /// that `section/mod.rs::resolve_section_idx` ships, so
    /// sibling verbs disagreed: `section preset=heavy` worked,
    /// `section frame preset=heavy` errored "node 'X' has multiple
    /// sections". Pin the parity.
    #[test]
    fn section_frame_single_node_auto_resolves_to_section_zero() {
        let mut doc = load_test_doc();
        let id = doc
            .mindmap
            .nodes
            .iter()
            .find(|(_, n)| n.sections.len() == 1)
            .map(|(id, _)| id.clone())
            .expect("testament map has at least one single-section node");
        doc.selection = SelectionState::Single(id.clone());
        // Should resolve and apply without errors — no `section=K`
        // required because the node only has one section.
        let result = run("section frame preset=heavy", &mut doc);
        match result {
            ExecResult::Ok(_) | ExecResult::Lines(_) => {}
            other => panic!("expected single-section auto-resolve to succeed, got {:?}", other),
        }
        let cfg = doc.mindmap.nodes.get(&id).unwrap().sections[0]
            .frame_border
            .as_ref()
            .expect("section[0]'s frame_border populated");
        assert_eq!(cfg.preset, "heavy");
    }

    /// Multi-section + Single selection still requires explicit
    /// `section=K` — pin the negative case so the auto-resolve
    /// doesn't accidentally trigger when the picker is genuinely
    /// ambiguous. Error message includes the section count + the
    /// `section=<idx>` hint (matches `section/mod.rs`'s wording).
    #[test]
    fn section_frame_multi_section_single_selection_errors_with_count_and_hint() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Single(id);
        let r = run("section frame preset=heavy", &mut doc);
        let msg = match r {
            ExecResult::Err(s) => s,
            other => panic!("expected Err, got {:?}", other),
        };
        assert!(msg.contains("has 2 sections"), "missing count: {}", msg);
        assert!(msg.contains("section=<idx>"), "missing kv hint: {}", msg);
    }

    #[test]
    fn section_frame_preset_writes_section_frame_border() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        // Strict-Ok: single-section preset write, no auto-promote,
        // no bare-custom — single-line success is the contract.
        assert_exec_ok_strict(run("section frame preset=heavy", &mut doc));
        let cfg = doc.mindmap.nodes.get(&id).unwrap().sections[1]
            .frame_border
            .as_ref()
            .expect("frame_border populated");
        assert_eq!(cfg.preset, "heavy");
    }

    #[test]
    fn section_frame_preset_does_not_touch_node_style_border() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        assert_exec_ok(run("section frame preset=heavy", &mut doc));
        assert!(
            doc.mindmap.nodes.get(&id).unwrap().style.border.is_none(),
            "node-level border must not be created by section frame edits"
        );
    }

    #[test]
    fn section_frame_kv_overrides_selection_index() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Single(id.clone());
        assert_exec_ok(run("section frame preset=double section=0", &mut doc));
        let cfg = doc.mindmap.nodes.get(&id).unwrap().sections[0]
            .frame_border
            .as_ref()
            .expect("section[0].frame_border populated");
        assert_eq!(cfg.preset, "double");
        assert!(
            doc.mindmap.nodes.get(&id).unwrap().sections[1]
                .frame_border
                .is_none(),
            "section[1] untouched"
        );
    }

    #[test]
    fn section_frame_glyph_against_non_custom_preset_errors() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        // `preset=heavy` plus a side glyph: parity with `border` /
        // `canvas border` — the verb gates glyph writes against
        // non-custom presets so the user picks `custom` explicitly
        // before authoring per-side overrides. Pre-fix the kv form
        // silently auto-promoted; post-fix it errors.
        let result = run("section frame preset=heavy top=\"###(*)###\"", &mut doc);
        match result {
            ExecResult::Err(s) => {
                assert!(
                    s.contains("section frame preset=custom"),
                    "error should hint at running preset=custom first: {}",
                    s
                );
            }
            other => panic!("expected Err on non-custom + glyph, got {:?}", other),
        }
    }

    #[test]
    fn section_frame_glyph_with_custom_preset_in_same_kv_bundle_succeeds() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        // Pre-set custom, then add the glyph in a follow-up kv —
        // the gate samples the resolved (post-write) preset, so
        // this two-step sequence works as expected.
        assert_exec_ok(run("section frame preset=custom", &mut doc));
        let result = run("section frame top=\"###(*)###\"", &mut doc);
        match result {
            ExecResult::Ok(_) | ExecResult::Lines(_) => {}
            other => panic!("expected success, got {:?}", other),
        }
        let cfg = doc.mindmap.nodes.get(&id).unwrap().sections[1]
            .frame_border
            .as_ref()
            .expect("frame_border populated");
        assert_eq!(cfg.preset, "custom");
        let glyphs = cfg.glyphs.as_ref().expect("glyphs populated by side edit");
        assert_eq!(glyphs.top, "###(*)###");
    }

    #[test]
    fn section_frame_reset_clears_per_section_override() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        assert_exec_ok(run("section frame preset=double", &mut doc));
        assert!(doc.mindmap.nodes.get(&id).unwrap().sections[1]
            .frame_border
            .is_some());
        assert_exec_ok(run("section frame reset", &mut doc));
        assert!(
            doc.mindmap.nodes.get(&id).unwrap().sections[1]
                .frame_border
                .is_none(),
            "reset must drop the per-section override"
        );
    }

    #[test]
    fn section_frame_round_trips_through_undo() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        assert_exec_ok(run("section frame preset=heavy color=#ff8800", &mut doc));
        assert!(doc.undo());
        assert!(
            doc.mindmap.nodes.get(&id).unwrap().sections[1]
                .frame_border
                .is_none(),
            "undo restores the absent prior frame_border"
        );
    }

    #[test]
    fn section_frame_no_change_does_not_grow_undo_stack() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        assert_exec_ok(run("section frame preset=heavy", &mut doc));
        let depth_after_first = doc.undo_stack.len();
        assert_exec_ok(run("section frame preset=heavy", &mut doc));
        assert_eq!(
            doc.undo_stack.len(),
            depth_after_first,
            "an idempotent edit must not push an undo entry"
        );
    }

    /// `dirty` must not flip on a no-op section-frame edit. Pre-fix
    /// the helper called `mutate_section_with_style_undo`
    /// unconditionally, which set `dirty = true`, then the verb
    /// would `undo_stack.pop()` to undo the snapshot push — but
    /// `dirty` stayed flipped, causing spurious "unsaved changes"
    /// prompts on a save-on-exit path. The fix moved the bool
    /// verdict into the helper itself; this test pins the
    /// regression so a future refactor that re-introduces a
    /// pop-the-snapshot pattern fails immediately.
    #[test]
    fn section_frame_no_change_does_not_flip_dirty() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        assert_exec_ok(run("section frame preset=heavy", &mut doc));
        doc.dirty = false; // baseline: post-real-edit, simulate save
        assert_exec_ok(run("section frame preset=heavy", &mut doc));
        assert!(
            !doc.dirty,
            "an idempotent section-frame edit must not flip `dirty`"
        );
    }

    #[test]
    fn section_frame_unknown_key_errors() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        assert_exec_err_contains(run("section frame frob=baz", &mut doc), "unknown key");
    }

    #[test]
    fn section_frame_out_of_range_section_kv_errors() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Single(id);
        assert_exec_err_contains(
            run("section frame preset=heavy section=99", &mut doc),
            "not found on node",
        );
    }

    #[test]
    fn section_frame_show_reports_resolved_config() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        assert_exec_ok(run("section frame preset=double color=#cd00cd", &mut doc));
        let result = run("section frame show", &mut doc);
        let lines = match result {
            ExecResult::Lines(ls) => ls,
            other => panic!("expected ExecResult::Lines, got {:?}", other),
        };
        let blob = lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            blob.contains("section="),
            "show must include section index: {}",
            blob
        );
        assert!(
            blob.contains("per-section override"),
            "show must label the source: {}",
            blob
        );
        assert!(blob.contains("#cd00cd"), "show must surface the color: {}", blob);
        // C10: surface both focused and unfocused branches so the
        // user sees what each will look like at render time.
        assert!(
            blob.contains("[unfocused]"),
            "show must include unfocused header: {}",
            blob
        );
        assert!(
            blob.contains("[focused]"),
            "show must include focused header: {}",
            blob
        );
    }

    #[test]
    fn section_frame_show_focused_branch_falls_through_to_unfocused_canvas_default() {
        // No per-section override, no focused canvas slot, just an
        // unfocused canvas slot — focused branch should report the
        // "focused fallback" source label, matching the renderer's
        // cascade in `resolve_section_frame_border`.
        let (mut doc, id) = pinned_two_section_node();
        doc.mindmap.canvas.default_section_frame_border = Some(baumhard::mindmap::model::GlyphBorderConfig {
            preset: "double".into(),
            font: None,
            font_size_pt: 12.0,
            color: Some("#abcdef".into()),
            glyphs: None,
            padding: 0.0,
            color_palette: None,
            color_palette_field: None,
        });
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        let result = run("section frame show", &mut doc);
        let blob = match result {
            ExecResult::Lines(ls) => ls.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n"),
            other => panic!("expected ExecResult::Lines, got {:?}", other),
        };
        assert!(
            blob.contains("unfocused canvas default (focused fallback)"),
            "focused branch should label the focused→unfocused fallback: {}",
            blob
        );
        assert!(
            blob.contains("unfocused canvas default"),
            "unfocused branch should label the canvas-default source: {}",
            blob
        );
    }

    #[test]
    fn section_frame_show_surfaces_per_side_glyphs_when_set() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        assert_exec_ok(run("section frame preset=custom", &mut doc));
        assert_exec_ok(run("section frame top=\"###(*)###\"", &mut doc));
        let result = run("section frame show", &mut doc);
        let blob = match result {
            ExecResult::Lines(ls) => ls.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n"),
            other => panic!("expected ExecResult::Lines, got {:?}", other),
        };
        assert!(
            blob.contains("top:"),
            "show must surface per-side glyph patterns when authored: {}",
            blob
        );
        assert!(
            blob.contains("###(*)###"),
            "show must echo the literal pattern: {}",
            blob
        );
        assert!(
            blob.contains("corners:"),
            "show must surface the per-corner readout: {}",
            blob
        );
    }

    #[test]
    fn section_frame_bare_preset_custom_emits_glyph_hint() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        let result = run("section frame preset=custom", &mut doc);
        let blob: String = match result {
            ExecResult::Lines(ls) => ls.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n"),
            ExecResult::Ok(s) => s,
            other => panic!("expected lines or ok, got {:?}", other),
        };
        assert!(
            blob.contains("preset=custom"),
            "bare preset=custom should mention what was set: {}",
            blob
        );
        assert!(
            blob.contains("top=") || blob.contains("glyph"),
            "bare preset=custom should hint at the glyph fields: {}",
            blob
        );
    }

    #[test]
    fn section_frame_palette_kv_lands_on_section_only() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 0,
        });
        assert_exec_ok(run(
            "section frame preset=light palette=rainbow field=frame",
            &mut doc,
        ));
        let cfg = doc.mindmap.nodes.get(&id).unwrap().sections[0]
            .frame_border
            .as_ref()
            .expect("frame_border populated");
        assert_eq!(cfg.color_palette.as_deref(), Some("rainbow"));
        assert_eq!(cfg.color_palette_field.as_deref(), Some("frame"));
        assert!(
            doc.mindmap.nodes.get(&id).unwrap().sections[1]
                .frame_border
                .is_none(),
            "section[1] must not be touched"
        );
    }

    #[test]
    fn section_frame_no_args_errors_with_usage() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        assert_exec_err_contains(run("section frame", &mut doc), "usage:");
    }

    /// Single-node selection without a `section=K` kv must error
    /// at the verb layer with a hint to pass `section=`. Pre-fix
    /// the only `Single` test always passed `section=K`, so this
    /// path was untested.
    #[test]
    fn section_frame_single_selection_without_kv_errors() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Single(id);
        assert_exec_err_contains(run("section frame preset=heavy", &mut doc), "pass section=");
    }

    /// `SelectionState::None` must surface a "no selection" error
    /// rather than the misleading "select a section" message the
    /// pre-fix swallow-all path emitted regardless of variant.
    #[test]
    fn section_frame_no_selection_errors_with_no_selection_message() {
        let (mut doc, _id) = pinned_two_section_node();
        doc.selection = SelectionState::None;
        assert_exec_err_contains(run("section frame preset=heavy", &mut doc), "no selection");
    }

    /// Edge selection must surface "not applicable to edges" — the
    /// border verbs collapse into the same diagnostic surface.
    #[test]
    fn section_frame_edge_selection_errors_with_not_applicable() {
        let (mut doc, _id) = pinned_two_section_node();
        // Synthesise an edge selection. Any edge will do — the
        // verb's branch fires before any per-edge inspection runs.
        if let Some(edge) = doc.mindmap.edges.first() {
            let edge_ref =
                crate::application::document::EdgeRef::new(&edge.from_id, &edge.to_id, &edge.edge_type);
            doc.selection = SelectionState::Edge(edge_ref);
            assert_exec_err_contains(
                run("section frame preset=heavy", &mut doc),
                "not applicable to edges",
            );
        }
    }

    /// Side-pattern parse error from the kv stage must surface
    /// verbatim — closes the negative-path coverage gap that
    /// `border` already pins via its own tests.
    #[test]
    fn section_frame_invalid_side_pattern_errors_with_parser_message() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        // `a)` is unmatched-close — the parser rejects it with a
        // message containing "unmatched". `stage_kv` prefixes the
        // side label.
        assert_exec_err_contains(run("section frame top=\"a)\"", &mut doc), "unmatched");
    }

    /// Re-applying the same edit after undo lands the same final
    /// state as the original. The document layer doesn't yet
    /// expose a `redo` API (`pub fn undo` is the only direction in
    /// `document/undo.rs`), so this test exercises the
    /// "undo-then-redo-by-replay" path instead — same correctness
    /// contract, just spelled out explicitly. The earlier
    /// `*_round_trips_through_undo` test is intentionally
    /// undo-only to match the `undo`-only API.
    #[test]
    fn section_frame_replay_after_undo_lands_same_state() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        assert_exec_ok(run("section frame preset=heavy color=#ff8800", &mut doc));
        let after_edit_preset = doc.mindmap.nodes.get(&id).unwrap().sections[1]
            .frame_border
            .as_ref()
            .map(|c| c.preset.clone());
        let after_edit_color = doc.mindmap.nodes.get(&id).unwrap().sections[1]
            .frame_border
            .as_ref()
            .and_then(|c| c.color.clone());
        assert!(doc.undo());
        assert!(
            doc.mindmap.nodes.get(&id).unwrap().sections[1]
                .frame_border
                .is_none(),
            "undo restores the absent prior frame_border"
        );
        assert_exec_ok(run("section frame preset=heavy color=#ff8800", &mut doc));
        let after_replay_preset = doc.mindmap.nodes.get(&id).unwrap().sections[1]
            .frame_border
            .as_ref()
            .map(|c| c.preset.clone());
        let after_replay_color = doc.mindmap.nodes.get(&id).unwrap().sections[1]
            .frame_border
            .as_ref()
            .and_then(|c| c.color.clone());
        assert_eq!(after_edit_preset, after_replay_preset);
        assert_eq!(after_edit_color, after_replay_color);
    }

    /// `section frame preview …` writes to `border_preview` with
    /// a `Sections([(node_id, idx)])` target — section index
    /// resolved from the live `Section` selection (or `section=K`
    /// kv).
    #[test]
    fn section_frame_preview_resolves_section_idx_from_selection() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        let result = run("section frame preview preset=heavy", &mut doc);
        match result {
            ExecResult::Ok(_) | ExecResult::Lines(_) => {}
            other => panic!("expected success, got {:?}", other),
        }
        let preview = doc.border_preview.as_ref().expect("preview slot populated");
        match &preview.target {
            crate::application::document::BorderPreviewTarget::Sections(pairs) => {
                assert_eq!(pairs.len(), 1);
                assert_eq!(pairs[0].0, id);
                assert_eq!(pairs[0].1, 1);
            }
            other => panic!("expected Sections target, got {:?}", other),
        }
        // Model is not touched.
        assert!(
            doc.mindmap.nodes.get(&id).unwrap().sections[1]
                .frame_border
                .is_none(),
            "preview must not write to the model"
        );
    }

    /// Commit dispatches to `set_section_frame_border_config` —
    /// the model picks up the staged preset on commit.
    #[test]
    fn section_frame_preview_commit_writes_through() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 0,
        });
        assert_exec_ok(run("section frame preview preset=double", &mut doc));
        let result = run("section frame preview commit", &mut doc);
        match result {
            ExecResult::Ok(_) | ExecResult::Lines(_) => {}
            other => panic!("expected success, got {:?}", other),
        }
        assert_eq!(
            doc.mindmap.nodes.get(&id).unwrap().sections[0]
                .frame_border
                .as_ref()
                .unwrap()
                .preset,
            "double"
        );
        assert!(doc.border_preview.is_none());
    }

    /// `section frame preview` against a `MultiSection` selection
    /// previews every (node_id, section_idx) pair. Pre-fix
    /// `MultiSection` collapsed to node-ids and re-resolved one
    /// idx per node — for a multi-pair shape this was either
    /// "no section" or the wrong section. Now the verb path
    /// drops the pairs straight into `Sections(...)`.
    #[test]
    fn section_frame_preview_handles_multi_section_selection() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::MultiSection(vec![
            SectionSel {
                node_id: id.clone(),
                section_idx: 0,
            },
            SectionSel {
                node_id: id.clone(),
                section_idx: 1,
            },
        ]);
        assert_exec_ok(run("section frame preview preset=heavy", &mut doc));
        // Preview slot must populate with both targets.
        let target = match doc.border_preview.as_ref() {
            Some(p) => &p.target,
            None => panic!("preview slot should be populated for MultiSection"),
        };
        match target {
            crate::application::document::BorderPreviewTarget::Sections(pairs) => {
                assert_eq!(pairs.len(), 2, "MultiSection must produce 2 pairs");
                assert!(pairs.iter().any(|(n, i)| n == &id && *i == 0));
                assert!(pairs.iter().any(|(n, i)| n == &id && *i == 1));
            }
            other => panic!("expected Sections target, got {:?}", other),
        }
        // Commit fans out to both sections.
        assert_exec_ok(run("section frame preview commit", &mut doc));
        for i in 0..=1 {
            assert_eq!(
                doc.mindmap.nodes.get(&id).unwrap().sections[i]
                    .frame_border
                    .as_ref()
                    .unwrap()
                    .preset,
                "heavy",
                "MultiSection commit must write to section[{}]",
                i
            );
        }
    }
}
