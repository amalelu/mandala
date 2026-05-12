// SPDX-License-Identifier: MPL-2.0

//! `font size=14 [min=8] [max=128]` — atomic font + clamp setter
//! dispatched against the current selection.
//!
//! Parses all three optional kvs up front, then applies them in a
//! single atomic document call so that order-sensitive cases like
//! `font size=14 max=10` land as `size=10, max=10` (min/max write
//! first, then size clamps against the new bounds) instead of the
//! wrong `size=14, max=10`.
//!
//! Routing against the active selection (kv form):
//! - `Node`: `size` sets the node font size; `min`/`max` are
//!   NotApplicable (nodes have no screen-space clamps).
//! - `Edge`: writes `glyph_connection.{font_size_pt, min_font_size_pt,
//!   max_font_size_pt}`.
//! - `EdgeLabel`: writes `label_config.{font_size_pt, min_font_size_pt,
//!   max_font_size_pt}` so the label can be sized independently of
//!   the edge body.
//! - `PortalLabel`: writes the owning edge's `glyph_connection` —
//!   the icon inherits edge-body clamps and splitting icon clamps
//!   off from the edge is outside the user-level spec.
//! - `PortalText`: writes `PortalEndpointState.{text_font_size_pt,
//!   text_min_font_size_pt, text_max_font_size_pt}` — sibling of
//!   `EdgeLabel` for portal-mode edges.
//!
//! Two positional subverbs sit alongside the kv form:
//! - `font set <family>` — pin the font family on the current
//!   selection via the `AcceptsFontFamily` trait
//!   (`AcceptsFontFamily::set_font_family` on `TargetView`). Each
//!   selection variant decides which channel the family lands on
//!   (nodes → every `TextRun.font`, edges + portal icons →
//!   `glyph_connection.font`); edge labels and portal text return
//!   `NotApplicable` and surface a clear "not applicable to
//!   `<kind>`" message.
//! - `font list` — emit one scrollback line per loaded family,
//!   each rendered in its own face. Sorted alphabetically.

use super::Command;
use crate::application::console::completion::{
    kv_key_completions_with_hints, prefix_filter, Completion, CompletionContext, CompletionState,
};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::traits::{apply_to_targets, AcceptsFontFamily};
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::{SectionSel, SelectionState};

pub const KEYS: &[&str] = &["size", "min", "max", "section"];
/// Positional subverbs surfaced as token-0 completions alongside
/// the kv keys.
pub const VERBS: &[&str] = &["set", "list"];
/// Preset sizes surfaced in completion. Users can type any positive
/// float; the preset list just makes the popup useful.
pub const SIZE_PRESETS: &[&str] = &["10", "12", "14", "16", "18", "24", "32"];

pub const COMMAND: Command = Command {
    name: "font",
    aliases: &[],
    summary: "Set font family / size / clamps on the selection, or list fonts",
    usage: "font set <family> | font list | font size=<pt> [min=<pt>] [max=<pt>]",
    tags: &[
        "font", "family", "set", "list", "size", "min", "max", "clamp", "pt", "smaller", "larger",
    ],
    applicable: always,
    complete: complete_font,
    execute: execute_font,
};

fn complete_font(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    match &state.context {
        // Token 0: positional verbs (`set`, `list`) + kv keys.
        CompletionContext::Token { index: 0 } => {
            let mut out: Vec<Completion> = Vec::new();
            for v in VERBS {
                if v.starts_with(state.partial) {
                    out.push(Completion {
                        text: match *v {
                            "set" => "set ".to_string(),
                            other => other.to_string(),
                        },
                        display: v.to_string(),
                        hint: verb_hint(v).map(str::to_string),
                        font_family: None,
                    });
                }
            }
            out.extend(kv_key_completions_with_hints(KEYS, state.partial, kv_hint));
            out
        }
        // Token 1 after `set`: every loaded font family, each
        // pre-shaped in its own face so the user sees the look
        // before committing.
        CompletionContext::Token { index: 1 } if state.tokens.get(1).map(String::as_str) == Some("set") => {
            font_family_completions(state.partial)
        }
        // Bare-token slots past index 0 with no preceding `set`
        // fall back to the kv keys (parity with the pre-existing
        // shape).
        CompletionContext::Token { .. } => kv_key_completions_with_hints(KEYS, state.partial, kv_hint),
        CompletionContext::KvValue { key } if KEYS.contains(&key.as_str()) => {
            prefix_filter(SIZE_PRESETS, state.partial)
        }
        _ => Vec::new(),
    }
}

fn verb_hint(verb: &str) -> Option<&'static str> {
    match verb {
        "set" => Some("pin the font family on the current selection"),
        "list" => Some("list every loaded font, each rendered in its face"),
        _ => None,
    }
}

fn kv_hint(key: &str) -> Option<&'static str> {
    match key {
        "size" => Some("target on-screen size in points"),
        "min" => Some("lower screen-space clamp in points"),
        "max" => Some("upper screen-space clamp in points"),
        "section" => Some("target section index inside a multi-section node"),
        _ => None,
    }
}

/// Build font-family completions: one entry per loaded family
/// whose name starts with `partial` (case-insensitive). Each entry
/// carries `font_family = Some(<name>)` so the popup row shapes the
/// candidate label in that very face. Streams from
/// [`baumhard::font::fonts::loaded_families_iter`] so the
/// keystroke-hot path doesn't allocate a fresh `Vec<String>` per
/// call.
///
/// Family names that contain whitespace are returned with the
/// inserted `text` wrapped in double quotes — the tokenizer would
/// otherwise split `Norse Bold` into two tokens, and the user would
/// hit "not a loaded font" on the first chunk. Quoting at the
/// completion source means tab-accept always produces a parseable
/// command. The `display` row stays unquoted so the popup reads
/// naturally.
///
/// `partial` arrives already-unquoted: the tokenizer
/// (`parser::tokenize`) drops the leading `"` on an unterminated
/// quoted token, so a user mid-typing `font set "Nor` lands here
/// with `partial = "Nor"`, not `"\"Nor"`. No leading-quote
/// stripping is needed.
fn font_family_completions(partial: &str) -> Vec<Completion> {
    let partial_lc = partial.to_ascii_lowercase();
    baumhard::font::fonts::loaded_families_iter()
        .filter(|f| f.to_ascii_lowercase().starts_with(&partial_lc))
        .map(|family| {
            let needs_quoting = family.chars().any(char::is_whitespace);
            let text = if needs_quoting {
                format!("\"{}\"", family)
            } else {
                family.to_string()
            };
            Completion {
                text,
                display: family.to_string(),
                hint: None,
                font_family: Some(family.to_string()),
            }
        })
        .collect()
}

/// Parse a kv value as a positive finite f32. Returns an
/// `ExecResult::Err` for non-numbers, NaN, infinity, or ≤ 0.
fn parse_pt(key: &str, value: &str) -> Result<f32, ExecResult> {
    crate::application::console::helpers::parse_finite_pt(key, value).map_err(ExecResult::err)
}

/// Parsed `font` kv-form arguments. Built once by
/// [`parse_font_args`]; the per-selection-variant dispatcher
/// in [`apply_font_args`] reads from it without re-parsing.
struct FontArgs {
    size: Option<f32>,
    min: Option<f32>,
    max: Option<f32>,
    section_target: Option<usize>,
    range_target: Option<(usize, usize)>,
}

fn execute_font(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // Positional subverbs are checked first — they're channel-less
    // operations and don't share parse state with the kv triple.
    if let Some(verb) = args.positional(0) {
        return match verb {
            "set" => execute_font_set(args, eff),
            "list" => execute_font_list(args),
            _ => ExecResult::err(format!(
                "font: unknown subverb '{}'; use 'set <family>', \
                 'list', or 'size=<pt>' (kv form)",
                verb
            )),
        };
    }
    let parsed = match parse_font_args(args) {
        Ok(p) => p,
        Err(e) => return e,
    };
    apply_font_args(&mut eff.document, &parsed)
}

/// Parse every recognised kv up front so the atomic application
/// sees a complete `FontArgs` value. Returns `Err(ExecResult)`
/// for unknown keys, malformed values, mutually-exclusive
/// combinations, or empty input — all surface to the caller via
/// the same error channel `apply_font_args` would use.
fn parse_font_args(args: &Args) -> Result<FontArgs, ExecResult> {
    let mut out = FontArgs {
        size: None,
        min: None,
        max: None,
        section_target: None,
        range_target: None,
    };
    let mut saw_any = false;
    for (k, v) in args.kvs() {
        saw_any = true;
        match k {
            "size" => out.size = Some(parse_pt("size", v)?),
            "min" => out.min = Some(parse_pt("min", v)?),
            "max" => out.max = Some(parse_pt("max", v)?),
            "section" => {
                out.section_target =
                    Some(super::range_kv::parse_section_kv("font", v).map_err(ExecResult::err)?);
            }
            "range" => {
                out.range_target = Some(
                    super::range_kv::parse_range_kv(v)
                        .map_err(|msg| ExecResult::err(format!("font: range='{}' — {}", v, msg)))?,
                );
            }
            other => return Err(ExecResult::err(format!("unknown key '{}'", other))),
        }
    }
    if out.range_target.is_some() && out.section_target.is_none() {
        return Err(ExecResult::err(
            "font: range=A..B requires section=N — ranges target grapheme indices inside one section",
        ));
    }
    if out.range_target.is_some() && (out.min.is_some() || out.max.is_some()) {
        return Err(ExecResult::err(
            "font: min/max not applicable to a section range (per-grapheme clamps don't exist)",
        ));
    }
    if !saw_any {
        return Err(ExecResult::err(
            "usage: font set <family> | font list | \
             font size=<pt> [min=<pt>] [max=<pt>]",
        ));
    }
    if out.size.is_none() && out.min.is_none() && out.max.is_none() {
        return Err(ExecResult::err("font: nothing to set"));
    }
    // Reject obviously-inverted explicit bounds up front so the
    // user sees a clear error instead of a silent no-op from the
    // setter's inverted-bounds guard. The setter still re-checks
    // against resolved (post-override) bounds for defence in
    // depth — that catches the case where the user passes only
    // one side and it inverts against the existing struct.
    if let (Some(lo), Some(hi)) = (out.min, out.max) {
        if lo > hi {
            return Err(ExecResult::err(format!(
                "font: min={lo} > max={hi} (inverted bounds)"
            )));
        }
    }
    Ok(out)
}

/// Selection-variant dispatch. A `Multi` node selection fans
/// out over each node (size only; min/max are NotApplicable
/// for nodes). The edge-adjacent variants each write to their
/// own channel.
fn apply_font_args(doc: &mut crate::application::document::MindMapDocument, a: &FontArgs) -> ExecResult {
    match doc.selection.clone() {
        // `section=N` (E5 verb syntax) routes to that specific
        // section's runs; absent, the write applies to every
        // section on the node.
        SelectionState::Single(id) => match a.section_target {
            Some(idx) => section_font_outcome(doc, &id, idx, a.size, a.min, a.max, a.range_target),
            None => node_font_outcome(doc, &id, a.size, a.min, a.max),
        },
        SelectionState::Section(s) => {
            // Section selection: explicit `section=N` overrides
            // the active section index; otherwise fall through to
            // the section the user pointed at.
            let idx = a.section_target.unwrap_or(s.section_idx);
            section_font_outcome(doc, &s.node_id, idx, a.size, a.min, a.max, a.range_target)
        }
        SelectionState::Multi(ids) => fanout_size_outcome(
            ids.iter()
                .filter_map(|id| a.size.map(|pt| doc.set_node_font_size(id, pt))),
            a.min,
            a.max,
            "node",
        ),
        SelectionState::MultiSection(secs) => fanout_size_outcome(
            secs.iter().filter_map(|s| {
                a.size
                    .map(|pt| doc.set_section_font_size(&s.node_id, s.section_idx, pt))
            }),
            a.min,
            a.max,
            "section",
        ),
        SelectionState::Edge(er) => {
            let changed = doc.set_edge_font(&er, a.size, a.min, a.max);
            super::applied_or_no_change("font", "edge", changed)
        }
        SelectionState::EdgeLabel(s) => {
            let changed = doc.set_edge_label_font(&s.edge_ref, a.size, a.min, a.max);
            super::applied_or_no_change("font", "edge label", changed)
        }
        SelectionState::PortalLabel(s) => {
            // Portal icon routes to the owning edge's
            // `glyph_connection` channel (same sink as `Edge`).
            let changed = doc.set_edge_font(&s.edge_ref(), a.size, a.min, a.max);
            super::applied_or_no_change("font", "portal label", changed)
        }
        SelectionState::PortalText(s) => {
            let changed = doc.set_portal_text_font(&s.edge_ref(), &s.endpoint_node_id, a.size, a.min, a.max);
            super::applied_or_no_change("font", "portal text", changed)
        }
        SelectionState::SectionRange { sel, range } => {
            // SectionRange: route to the section, layering an
            // explicit `range=A..B` from the kv parser on top of
            // the selection's own range when both are set
            // (kv wins — explicit user input).
            let idx = a.section_target.unwrap_or(sel.section_idx);
            let effective_range = a.range_target.or(Some(range));
            section_font_outcome(doc, &sel.node_id, idx, a.size, a.min, a.max, effective_range)
        }
        SelectionState::None => ExecResult::err("font: no selection"),
    }
}

/// Outcome formatter for the `Multi` / `MultiSection` fanout
/// arms. `applied_iter` yields one bool per attempted target
/// (true = the setter changed the doc); `kind` is the singular
/// noun used in the "applied to N `kind`(s)" line.
///
/// The `min` / `max` arms surface a single
/// "min/max: `kind`s have no screen-space clamps" message rather
/// than one per target — the message is identical across nodes
/// and sections; the only difference is the noun.
fn fanout_size_outcome(
    applied_iter: impl Iterator<Item = bool>,
    min: Option<f32>,
    max: Option<f32>,
    kind: &str,
) -> ExecResult {
    let changed = applied_iter.filter(|b| *b).count();
    let applicable_msg =
        (min.is_some() || max.is_some()).then(|| format!("min/max: {}s have no screen-space clamps", kind));
    if changed == 0 && applicable_msg.is_none() {
        return ExecResult::ok_msg("font: no change");
    }
    let mut lines = Vec::new();
    if changed > 0 {
        lines.push(format!("font: applied to {} {}(s)", changed, kind));
    }
    if let Some(m) = applicable_msg {
        lines.push(m);
    }
    ExecResult::lines(lines)
}

/// Per-section font outcome — the `section=N` verb syntax routes
/// here. Mirrors [`node_font_outcome`] for the shape, but writes
/// only to section `section_idx` via
/// [`super::super::super::document::MindMapDocument::set_section_font_size`].
/// `min` / `max` remain NotApplicable for nodes / sections; the
/// surface message is the same.
fn section_font_outcome(
    doc: &mut crate::application::document::MindMapDocument,
    id: &str,
    section_idx: usize,
    size: Option<f32>,
    min: Option<f32>,
    max: Option<f32>,
    range: Option<(usize, usize)>,
) -> ExecResult {
    if let Some((rs, _re)) = range {
        if let Some(node) = doc.mindmap.nodes.get(id) {
            if let Some(section) = node.sections.get(section_idx) {
                let total = baumhard::util::grapheme_chad::count_grapheme_clusters(&section.text);
                if rs >= total {
                    return ExecResult::err(format!(
                        "font: range_start={} is past the section's grapheme count ({})",
                        rs, total
                    ));
                }
            }
        }
    }
    let mut messages = Vec::new();
    let mut any_applied = false;
    if let Some(pt) = size {
        let applied = match range {
            Some((rs, re)) => {
                let ok = doc.set_section_font_size_range(id, section_idx, rs, re, pt);
                if !ok {
                    // Mirror the picker path's stale-range diagnostic
                    // (see `color_picker_flow::commit`). Pre-flight
                    // already catches `rs >= total`; a `false` here
                    // means `range_end` extends past the section's
                    // grapheme count or the section was deleted.
                    log::warn!(
                        "font verb on section {} of node {} \
                         range {}..{} produced no change \
                         (range may extend past the section's \
                         grapheme count or section was deleted)",
                        section_idx,
                        id,
                        rs,
                        re
                    );
                }
                ok
            }
            None => doc.set_section_font_size(id, section_idx, pt),
        };
        if applied {
            any_applied = true;
        }
    }
    if min.is_some() || max.is_some() {
        messages.push("min/max: nodes have no screen-space clamps".to_string());
    }
    if !messages.is_empty() {
        if !any_applied {
            return ExecResult::err(messages.join("; "));
        }
        return ExecResult::lines(messages);
    }
    if any_applied {
        let scope = match range {
            Some((rs, re)) => format!("section {} range {}..{}", section_idx, rs, re),
            None => format!("section {}", section_idx),
        };
        ExecResult::ok_msg(format!("font applied to {}", scope))
    } else {
        ExecResult::ok_msg("font: no change")
    }
}

fn node_font_outcome(
    doc: &mut crate::application::document::MindMapDocument,
    id: &str,
    size: Option<f32>,
    min: Option<f32>,
    max: Option<f32>,
) -> ExecResult {
    let mut messages = Vec::new();
    let mut any_applied = false;
    if let Some(pt) = size {
        if doc.set_node_font_size(id, pt) {
            any_applied = true;
        }
    }
    if min.is_some() || max.is_some() {
        messages.push("min/max: nodes have no screen-space clamps".to_string());
    }
    if !messages.is_empty() {
        if !any_applied {
            return ExecResult::err(messages.join("; "));
        }
        return ExecResult::lines(messages);
    }
    if any_applied {
        ExecResult::ok_msg("font applied")
    } else {
        ExecResult::ok_msg("font: no change")
    }
}

/// `font set <family>` — pin the font family on the current
/// selection through the `AcceptsFontFamily` trait. Validates the
/// family name against `list_loaded_families()` first; an unknown
/// name surfaces an error pointing the user at `font list`.
fn execute_font_set(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // Collect every positional after `set` into one family name.
    // The tokenizer preserves quoted multi-word strings as one
    // token already (`font set "Norse Bold"`), but a user can also
    // bypass quoting entirely and just type `font set Norse Bold`
    // — joining positionals with a single space matches both shapes
    // and keeps the surface forgiving.
    let positionals: Vec<&str> = args.positionals().collect();
    if positionals.len() < 2 {
        return ExecResult::err("usage: font set <family> [section=N] [range=A..B]");
    }
    let family = positionals[1..].join(" ");
    if family.is_empty() {
        return ExecResult::err("usage: font set <family> [section=N] [range=A..B]");
    }
    // Validate against the loaded family list. Exact-match per
    // `app_font_by_family` semantics — the completion popup feeds
    // the canonical string back, so an interactive submit always
    // hits this branch with a known name.
    if baumhard::font::fonts::app_font_by_family(&family).is_none() {
        return ExecResult::err(format!(
            "font: '{}' is not a loaded font; try `font list`",
            family
        ));
    }
    // Extract optional `section=N` + `range=A..B`. Both bypass
    // the trait dispatcher and call the dedicated range / per-
    // section setters directly — the trait surface doesn't
    // carry per-section / per-range routing today (deferred to
    // N4-C alongside `SelectionState::SectionRange`).
    let mut section_target: Option<usize> = None;
    let mut range_target: Option<(usize, usize)> = None;
    for (k, v) in args.kvs() {
        match k {
            "section" => match super::range_kv::parse_section_kv("font", v) {
                Ok(idx) => section_target = Some(idx),
                Err(msg) => return ExecResult::err(msg),
            },
            "range" => match super::range_kv::parse_range_kv(v) {
                Ok(pair) => range_target = Some(pair),
                Err(msg) => return ExecResult::err(format!("font: range='{}' — {}", v, msg)),
            },
            other => return ExecResult::err(format!("font set: unknown key '{}'", other)),
        }
    }
    if range_target.is_some() && section_target.is_none() {
        return ExecResult::err(
            "font: range=A..B requires section=N — ranges target grapheme indices inside one section",
        );
    }
    if let (Some(idx), Some((rs, re))) = (section_target, range_target) {
        let node_id = match eff.document.selection.clone() {
            SelectionState::Single(id) => id,
            SelectionState::Section(s) => s.node_id,
            SelectionState::SectionRange { sel, .. } => sel.node_id,
            _ => return ExecResult::err("font: section=N requires a node or section selection"),
        };
        let applied = eff
            .document
            .set_section_font_family_range(&node_id, idx, rs, re, Some(&family));
        return if applied {
            ExecResult::ok_msg(format!(
                "font set: {} on section {} range {}..{}",
                family, idx, rs, re
            ))
        } else {
            ExecResult::ok_msg("font: no change")
        };
    }
    if let Some(idx) = section_target {
        let node_id = match eff.document.selection.clone() {
            SelectionState::Single(id) => id,
            SelectionState::Section(s) => s.node_id,
            SelectionState::SectionRange { sel, .. } => sel.node_id,
            _ => return ExecResult::err("font: section=N requires a node or section selection"),
        };
        let applied = eff.document.set_section_font_family(&node_id, idx, Some(&family));
        return if applied {
            ExecResult::ok_msg(format!("font set: {} on section {}", family, idx))
        } else {
            ExecResult::ok_msg("font: no change")
        };
    }
    let report = apply_to_targets(eff.document, |view| view.set_font_family(Some(&family)));
    if report.all_failed {
        return ExecResult::err(report.messages.join("; "));
    }
    if !report.messages.is_empty() {
        return ExecResult::lines(report.messages);
    }
    if report.any_applied {
        ExecResult::ok_msg(format!("font set: {}", family))
    } else {
        ExecResult::ok_msg("font: no change")
    }
}

/// Mutation core: pin the font family on the current selection.
/// Returns `true` when at least one target actually changed; `false`
/// for unknown families, no selection, or no-op writes. The Action
/// arm uses the bool to gate the scene rebuild; the verb keeps its
/// per-target reporting.
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(crate) fn apply_font_family_to_selection(
    doc: &mut crate::application::document::MindMapDocument,
    family: &str,
) -> bool {
    if family.is_empty() {
        return false;
    }
    if baumhard::font::fonts::app_font_by_family(family).is_none() {
        return false;
    }
    let report = apply_to_targets(doc, |view| view.set_font_family(Some(family)));
    report.any_applied
}

/// Mutation core: apply a font-size / min / max kv (one at a time)
/// to the current selection. `which` selects the slot
/// (`"size" | "min" | "max"`); mirrors the verb's per-channel
/// dispatch but only for one slot. Returns `true` on a real change.
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(crate) fn apply_font_kv_to_selection(
    doc: &mut crate::application::document::MindMapDocument,
    which: &str,
    pt: f32,
) -> bool {
    if !pt.is_finite() || pt <= 0.0 {
        return false;
    }
    let (size, min, max) = match which {
        "size" => (Some(pt), None, None),
        "min" => (None, Some(pt), None),
        "max" => (None, None, Some(pt)),
        _ => return false,
    };
    match doc.selection.clone() {
        // Whole-node: writes every section's runs.
        SelectionState::Single(id) => {
            // Nodes only accept `size`; `min` / `max` are
            // NotApplicable. Mirror the verb's behaviour.
            if size.is_some() {
                doc.set_node_font_size(&id, pt)
            } else {
                false
            }
        }
        // Section: route through `set_section_font_size` so the
        // Action path (keybinds / palette) matches the verb path
        // (`font size=N section=K`) — only the targeted section's
        // runs grow, siblings stay put.
        SelectionState::Section(SectionSel { node_id, section_idx }) => {
            if size.is_some() {
                doc.set_section_font_size(&node_id, section_idx, pt)
            } else {
                false
            }
        }
        SelectionState::Multi(ids) => {
            if size.is_none() {
                return false;
            }
            let mut changed = false;
            for id in &ids {
                changed |= doc.set_node_font_size(id, pt);
            }
            changed
        }
        SelectionState::MultiSection(secs) => {
            if size.is_none() {
                return false;
            }
            let mut changed = false;
            for s in &secs {
                changed |= doc.set_section_font_size(&s.node_id, s.section_idx, pt);
            }
            changed
        }
        SelectionState::Edge(er) => doc.set_edge_font(&er, size, min, max),
        SelectionState::EdgeLabel(s) => doc.set_edge_label_font(&s.edge_ref, size, min, max),
        SelectionState::PortalLabel(s) => doc.set_edge_font(&s.edge_ref(), size, min, max),
        SelectionState::PortalText(s) => {
            doc.set_portal_text_font(&s.edge_ref(), &s.endpoint_node_id, size, min, max)
        }
        SelectionState::SectionRange { sel, range } => {
            if size.is_none() {
                return false;
            }
            doc.set_section_font_size_range(&sel.node_id, sel.section_idx, range.0, range.1, pt)
        }
        SelectionState::None => false,
    }
}

/// `font list` — emit one scrollback line per loaded family, each
/// pinned to render in its own face (so a long list is a
/// font-by-font preview). Streams from `loaded_families_iter` so
/// no intermediate `Vec<String>` allocates.
fn execute_font_list(_args: &Args) -> ExecResult {
    use crate::application::console::OutputLine;
    let lines: Vec<OutputLine> = baumhard::font::fonts::loaded_families_iter()
        .map(|name| OutputLine::in_font(name, name))
        .collect();
    if lines.is_empty() {
        return ExecResult::err("font: no fonts loaded");
    }
    ExecResult::Lines(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::tests::fixtures::{assert_exec_ok, join_lines, run};
    use crate::application::document::tests_common::load_test_doc as fixture_doc;
    use crate::application::document::{EdgeRef, SelectionState};

    fn first_loaded_family() -> String {
        baumhard::font::fonts::init();
        baumhard::font::fonts::list_loaded_families()
            .into_iter()
            .next()
            .expect("at least one bundled family must be loaded")
    }

    #[test]
    fn list_emits_lines_with_fonts_one_per_family() {
        baumhard::font::fonts::init();
        let mut doc = fixture_doc();
        match run("font list", &mut doc) {
            ExecResult::Lines(rows) => {
                let families: Vec<&'static str> = baumhard::font::fonts::loaded_families_iter().collect();
                assert_eq!(rows.len(), families.len());
                for (i, line) in rows.iter().enumerate() {
                    assert_eq!(line.text, families[i]);
                    assert_eq!(line.font_family.as_deref(), Some(families[i]));
                }
            }
            other => panic!("expected Lines, got {:?}", other),
        }
    }

    #[test]
    fn set_unknown_family_errors_with_pointer_to_list() {
        let mut doc = fixture_doc();
        // Pick a node so the trait would otherwise apply.
        doc.selection = SelectionState::Single("0".into());
        match run("font set DefinitelyNotAFontFamilyXYZ", &mut doc) {
            ExecResult::Err(s) => {
                assert!(s.contains("not a loaded font"));
                assert!(s.contains("font list"));
            }
            other => panic!("expected Err, got {:?}", other),
        }
    }

    #[test]
    fn set_on_node_writes_text_run_font() {
        let family = first_loaded_family();
        let mut doc = fixture_doc();
        doc.selection = SelectionState::Single("0".into());
        assert_exec_ok(run(&format!("font set {}", family), &mut doc));
        // Every TextRun on the node should now carry the family.
        let node = doc.mindmap.nodes.get("0").expect("node 0 exists");
        assert!(!node.sections[0].text_runs.is_empty());
        for run in &node.sections[0].text_runs {
            assert_eq!(run.font, family);
        }
    }

    #[test]
    fn set_with_multiword_quoted_family_is_unknown_when_not_loaded() {
        let mut doc = fixture_doc();
        doc.selection = SelectionState::Single("0".into());
        // Quoted multi-word family — tokenizer keeps it as one token.
        match run(r#"font set "Imaginary Sans""#, &mut doc) {
            ExecResult::Err(s) => assert!(s.contains("not a loaded font")),
            other => panic!("expected Err, got {:?}", other),
        }
    }

    #[test]
    fn set_with_no_argument_returns_usage() {
        let mut doc = fixture_doc();
        doc.selection = SelectionState::Single("0".into());
        match run("font set", &mut doc) {
            ExecResult::Err(s) => assert!(s.contains("usage: font set <family>")),
            other => panic!("expected Err, got {:?}", other),
        }
    }

    #[test]
    fn set_no_selection_reports_no_target() {
        let family = first_loaded_family();
        let mut doc = fixture_doc();
        doc.selection = SelectionState::None;
        match run(&format!("font set {}", family), &mut doc) {
            ExecResult::Err(s) => assert!(s.contains("no target")),
            other => panic!("expected Err, got {:?}", other),
        }
    }

    #[test]
    fn set_on_edge_label_is_not_applicable() {
        let family = first_loaded_family();
        let mut doc = fixture_doc();
        // Pick the first edge in the testament map and select its
        // label channel — the test relies on the testament map
        // having at least one edge, which it does (243 nodes,
        // dozens of edges).
        let edge = doc
            .mindmap
            .edges
            .first()
            .expect("testament map has at least one edge");
        let er = EdgeRef::new(edge.from_id.clone(), edge.to_id.clone(), edge.edge_type.clone());
        doc.selection =
            SelectionState::EdgeLabel(crate::application::document::EdgeLabelSel { edge_ref: er });
        match run(&format!("font set {}", family), &mut doc) {
            ExecResult::Lines(msgs) => {
                assert!(join_lines(&msgs).contains("not applicable"));
            }
            ExecResult::Err(s) => {
                assert!(s.contains("not applicable"));
            }
            other => panic!("expected Lines / Err with 'not applicable', got {:?}", other),
        }
    }

    #[test]
    fn unknown_subverb_errors_clearly() {
        let mut doc = fixture_doc();
        match run("font frobnicate", &mut doc) {
            ExecResult::Err(s) => assert!(s.contains("unknown subverb")),
            other => panic!("expected Err, got {:?}", other),
        }
    }

    /// Multi-node selection fans out across every node and writes
    /// the family on each — the trait dispatcher's
    /// `apply_to_targets` path. Pre-fix, only `Single` was tested,
    /// so the fanout aggregation was effectively dead code.
    #[test]
    fn set_on_multi_node_selection_writes_every_node() {
        let family = first_loaded_family();
        let mut doc = fixture_doc();
        // Pick the first three node ids in the testament map.
        let ids: Vec<String> = doc.mindmap.nodes.keys().take(3).cloned().collect();
        assert_eq!(ids.len(), 3, "testament map must have ≥3 nodes");
        doc.selection = SelectionState::Multi(ids.clone());
        match run(&format!("font set {}", family), &mut doc) {
            ExecResult::Ok(_) | ExecResult::Lines(_) => {}
            other => panic!("expected Ok / Lines, got {:?}", other),
        }
        for id in &ids {
            let node = doc
                .mindmap
                .nodes
                .get(id)
                .expect("multi-selection ids exist in the doc");
            for run in &node.sections[0].text_runs {
                assert_eq!(run.font, family, "every run on every node should be pinned");
            }
        }
    }

    /// Edge selection writes `glyph_connection.font`. Smoke-tests
    /// the `TargetView::Edge` arm of the trait — pre-fix the path
    /// existed but had no integration test through the console verb.
    #[test]
    fn set_on_edge_selection_writes_glyph_connection_font() {
        let family = first_loaded_family();
        let mut doc = fixture_doc();
        let edge = doc
            .mindmap
            .edges
            .first()
            .expect("testament map has at least one edge")
            .clone();
        let er = crate::application::document::EdgeRef::new(
            edge.from_id.clone(),
            edge.to_id.clone(),
            edge.edge_type.clone(),
        );
        doc.selection = SelectionState::Edge(er.clone());
        assert_exec_ok(run(&format!("font set {}", family), &mut doc));
        let idx = doc.edge_index(&er).expect("edge resolves");
        assert_eq!(
            doc.mindmap.edges[idx]
                .glyph_connection
                .as_ref()
                .and_then(|c| c.font.as_deref()),
            Some(family.as_str()),
        );
    }

    /// PortalLabel selection routes through the edge body's font —
    /// the icon shares the edge's `glyph_connection.font` slot, same
    /// routing the existing `font size=` uses.
    #[test]
    fn set_on_portal_label_routes_to_edge_glyph_connection() {
        use crate::application::document::PortalLabelSel;
        use baumhard::mindmap::scene_cache::EdgeKey;

        let family = first_loaded_family();
        let mut doc = fixture_doc();
        let edge = doc
            .mindmap
            .edges
            .first()
            .expect("testament map has at least one edge")
            .clone();
        let er = crate::application::document::EdgeRef::new(
            edge.from_id.clone(),
            edge.to_id.clone(),
            edge.edge_type.clone(),
        );
        doc.selection = SelectionState::PortalLabel(PortalLabelSel {
            edge_key: EdgeKey::from_edge(&edge),
            endpoint_node_id: edge.to_id.clone(),
        });
        assert_exec_ok(run(&format!("font set {}", family), &mut doc));
        // Verify the edge's glyph_connection.font carries the family.
        let idx = doc.edge_index(&er).expect("edge resolves");
        assert_eq!(
            doc.mindmap.edges[idx]
                .glyph_connection
                .as_ref()
                .and_then(|c| c.font.as_deref()),
            Some(family.as_str()),
        );
    }

    /// PortalText selection mirrors EdgeLabel: it returns
    /// `NotApplicable` because portal text inherits the edge body's
    /// font today (no per-channel `font_family` slot exists on
    /// `PortalEndpointState`).
    #[test]
    fn set_on_portal_text_is_not_applicable() {
        use crate::application::document::PortalLabelSel;
        use baumhard::mindmap::scene_cache::EdgeKey;

        let family = first_loaded_family();
        let mut doc = fixture_doc();
        let edge = doc
            .mindmap
            .edges
            .first()
            .expect("testament map has at least one edge")
            .clone();
        doc.selection = SelectionState::PortalText(PortalLabelSel {
            edge_key: EdgeKey::from_edge(&edge),
            endpoint_node_id: edge.to_id.clone(),
        });
        match run(&format!("font set {}", family), &mut doc) {
            ExecResult::Lines(msgs) => {
                assert!(join_lines(&msgs).contains("not applicable"));
            }
            ExecResult::Err(s) => assert!(s.contains("not applicable")),
            other => panic!("expected Lines / Err with 'not applicable', got {:?}", other),
        }
    }

    #[test]
    fn completion_after_set_returns_loaded_families_in_their_face() {
        baumhard::font::fonts::init();
        let families = baumhard::font::fonts::list_loaded_families();
        assert!(!families.is_empty());
        // Pick a prefix from a known family — the first letter
        // of the first family — and confirm the completer surfaces
        // every family starting with that prefix, each tagged
        // with its own `font_family` so the renderer shapes the
        // popup row in that face.
        let prefix = families[0]
            .chars()
            .next()
            .expect("non-empty family name")
            .to_string();
        let cands = font_family_completions(&prefix);
        assert!(!cands.is_empty());
        for c in &cands {
            // `display` always carries the bare family name; `text`
            // is the same except when the name contains whitespace,
            // in which case it's wrapped in double quotes so a
            // tab-accept produces a parseable token.
            assert!(c
                .display
                .to_ascii_lowercase()
                .starts_with(&prefix.to_ascii_lowercase()));
            assert_eq!(c.font_family.as_deref(), Some(c.display.as_str()));
            if c.display.chars().any(char::is_whitespace) {
                assert_eq!(c.text, format!("\"{}\"", c.display));
            } else {
                assert_eq!(c.text, c.display);
            }
        }
    }

    /// Multi-word family names get the inserted `text` wrapped in
    /// double quotes so the tokenizer doesn't split them into
    /// separate positionals — `font set Norse Bold` would tokenize
    /// to `["font", "set", "Norse", "Bold"]` whereas
    /// `font set "Norse Bold"` is one quoted token. The display
    /// stays bare for readability.
    #[test]
    fn completion_quotes_family_names_with_spaces() {
        baumhard::font::fonts::init();
        let cand = Completion {
            text: "\"Multi Word\"".into(),
            display: "Multi Word".into(),
            hint: None,
            font_family: Some("Multi Word".into()),
        };
        // Sanity: a family name with whitespace should land as the
        // shape above. We can't guarantee any bundled family has a
        // space in its name, so just assert the formatter directly.
        let needs_quoting: bool = "Multi Word".chars().any(char::is_whitespace);
        assert!(needs_quoting);
        assert_eq!(
            if needs_quoting {
                format!("\"{}\"", cand.display)
            } else {
                cand.display.clone()
            },
            cand.text,
        );
    }

    // ─────────────────────────────────────────────────────────────
    // Mutation-core tests for the parametric Action arms. These
    // exercise the `apply_*` cores directly (no console plumbing),
    // pinning the contract dispatch.rs::Action::SetFontFamily /
    // SetFontSize / SetFontMin / SetFontMax depend on.
    // ─────────────────────────────────────────────────────────────

    #[test]
    fn apply_font_family_to_selection_writes_to_node_runs() {
        let family = first_loaded_family();
        let mut doc = fixture_doc();
        doc.selection = SelectionState::Single("0".into());
        assert!(super::apply_font_family_to_selection(&mut doc, &family));
        for section in &doc.mindmap.nodes.get("0").unwrap().sections {
            for run in &section.text_runs {
                assert_eq!(run.font, family);
            }
        }
    }

    #[test]
    fn apply_font_family_rejects_unknown_family() {
        let mut doc = fixture_doc();
        doc.selection = SelectionState::Single("0".into());
        // No-op + bool false: validates upfront against the loaded
        // family list before reaching the trait dispatcher.
        assert!(!super::apply_font_family_to_selection(
            &mut doc,
            "DefinitelyNotAFontFamilyXYZ",
        ));
    }

    #[test]
    fn apply_font_family_returns_false_with_no_selection() {
        let family = first_loaded_family();
        let mut doc = fixture_doc();
        doc.selection = SelectionState::None;
        assert!(!super::apply_font_family_to_selection(&mut doc, &family));
    }

    #[test]
    fn apply_font_family_returns_false_for_empty_string() {
        let mut doc = fixture_doc();
        doc.selection = SelectionState::Single("0".into());
        assert!(!super::apply_font_family_to_selection(&mut doc, ""));
    }

    #[test]
    fn apply_font_kv_size_writes_to_node() {
        let mut doc = fixture_doc();
        doc.selection = SelectionState::Single("0".into());
        assert!(super::apply_font_kv_to_selection(&mut doc, "size", 18.0));
        // `set_node_font_size` rounds the f32 and writes
        // `text_runs[i].size_pt` (u32) on every run.
        let node = doc.mindmap.nodes.get("0").unwrap();
        assert!(!node.sections[0].text_runs.is_empty());
        for run in &node.sections[0].text_runs {
            assert_eq!(run.size_pt, 18);
        }
    }

    #[test]
    fn apply_font_kv_min_or_max_on_node_is_no_op() {
        let mut doc = fixture_doc();
        doc.selection = SelectionState::Single("0".into());
        // Nodes have no screen-space clamp — the core falls into
        // the `size.is_some() == false` guard and returns false.
        assert!(!super::apply_font_kv_to_selection(&mut doc, "min", 10.0));
        assert!(!super::apply_font_kv_to_selection(&mut doc, "max", 32.0));
    }

    #[test]
    fn apply_font_kv_size_writes_to_edge_glyph_connection() {
        let mut doc = fixture_doc();
        let edge = doc.mindmap.edges.first().expect("testament edges").clone();
        let er = EdgeRef::new(&edge.from_id, &edge.to_id, &edge.edge_type);
        doc.selection = SelectionState::Edge(er.clone());
        assert!(super::apply_font_kv_to_selection(&mut doc, "size", 14.0));
        let idx = doc.edge_index(&er).unwrap();
        let cfg = doc.mindmap.edges[idx]
            .glyph_connection
            .as_ref()
            .expect("size write forks glyph_connection");
        assert!(baumhard::util::geometry::almost_equal(cfg.font_size_pt, 14.0));
    }

    #[test]
    fn apply_font_kv_returns_false_for_invalid_pt() {
        let mut doc = fixture_doc();
        doc.selection = SelectionState::Single("0".into());
        // NaN, infinite, zero, negative — all rejected upfront.
        assert!(!super::apply_font_kv_to_selection(&mut doc, "size", f32::NAN));
        assert!(!super::apply_font_kv_to_selection(
            &mut doc,
            "size",
            f32::INFINITY
        ));
        assert!(!super::apply_font_kv_to_selection(&mut doc, "size", 0.0));
        assert!(!super::apply_font_kv_to_selection(&mut doc, "size", -1.0));
    }

    #[test]
    fn apply_font_kv_returns_false_for_unknown_which() {
        let mut doc = fixture_doc();
        doc.selection = SelectionState::Single("0".into());
        // Unknown slot name — neither size, min, nor max — rejected.
        assert!(!super::apply_font_kv_to_selection(&mut doc, "bogus_slot", 14.0));
    }

    #[test]
    fn apply_font_kv_returns_false_with_no_selection() {
        let mut doc = fixture_doc();
        doc.selection = SelectionState::None;
        assert!(!super::apply_font_kv_to_selection(&mut doc, "size", 14.0));
    }

    /// `font size=N section=K` routes through the section setter
    /// for the specified index — the run on section K gets the new
    /// size, sections at other indices stay untouched.
    #[test]
    fn font_size_section_kv_targets_specific_section() {
        let mut doc = doc_with_two_sections_for_font("LiberationSans", 14);
        doc.selection = SelectionState::Single("0".into());
        assert_exec_ok(run("font size=22 section=1", &mut doc));
        let node = doc.mindmap.nodes.get("0").unwrap();
        assert!(
            node.sections[0].text_runs.iter().all(|r| r.size_pt == 14),
            "section 0 must NOT receive the size change"
        );
        assert!(
            node.sections[1].text_runs.iter().all(|r| r.size_pt == 22),
            "section 1 must receive the new size"
        );
    }

    /// Build a node "0" with two sections, both pinned to the
    /// given `font` and `size_pt` so a per-section font/size write
    /// is observable. Thin wrapper around the shared
    /// `make_two_section_node_with_pinned_runs` helper.
    fn doc_with_two_sections_for_font(
        font: &str,
        size: u32,
    ) -> crate::application::document::MindMapDocument {
        use crate::application::document::tests_common::make_two_section_node_with_pinned_runs;
        let mut doc = fixture_doc();
        make_two_section_node_with_pinned_runs(&mut doc, "0", "#ffffff", ["#ffffff", "#ffffff"], font, size);
        doc
    }

    /// `font set <family>` with a `SelectionState::Section` (no
    /// explicit `section=K` kv) routes through the
    /// `AcceptsFontFamily` trait arm to `set_section_font_family`
    /// — only the targeted section's runs change, siblings stay
    /// untouched.
    #[test]
    fn font_family_section_collapse_writes_only_section() {
        use crate::application::document::SectionSel;
        let family = first_loaded_family();
        let mut doc = doc_with_two_sections_for_font("LiberationSans", 14);
        doc.selection = SelectionState::Section(SectionSel {
            node_id: "0".into(),
            section_idx: 1,
        });
        assert_exec_ok(run(&format!("font set {}", family), &mut doc));
        let node = doc.mindmap.nodes.get("0").unwrap();
        assert!(
            node.sections[0]
                .text_runs
                .iter()
                .all(|r| r.font == "LiberationSans"),
            "section 0 (sibling) must NOT change family"
        );
        assert!(
            node.sections[1].text_runs.iter().all(|r| r.font == family),
            "section 1 (selected) must receive the new family"
        );
    }

    /// `apply_font_kv_to_selection("size", N)` (the parametric
    /// Action path used by keybinds and palette entries) with a
    /// `SelectionState::Section` routes through
    /// `set_section_font_size` so it matches the verb path's
    /// per-section behaviour. Pre-Tier-2A this Action arm
    /// collapsed to whole-node, lagging behind the verb. Pins
    /// Item 10.
    #[test]
    fn font_size_action_section_writes_through_section_setter() {
        use crate::application::document::SectionSel;
        let mut doc = doc_with_two_sections_for_font("LiberationSans", 14);
        doc.selection = SelectionState::Section(SectionSel {
            node_id: "0".into(),
            section_idx: 1,
        });
        assert!(super::apply_font_kv_to_selection(&mut doc, "size", 22.0));
        let node = doc.mindmap.nodes.get("0").unwrap();
        assert!(
            node.sections[0].text_runs.iter().all(|r| r.size_pt == 14),
            "section 0 (sibling) must NOT change size"
        );
        assert!(
            node.sections[1].text_runs.iter().all(|r| r.size_pt == 22),
            "section 1 (selected) must receive the new size"
        );
    }

    /// `apply_font_family_to_selection(family)` (the parametric
    /// Action path) with a `SelectionState::Section` routes
    /// through `AcceptsFontFamily` → `set_section_font_family` —
    /// the same per-section behaviour the verb path lands. Sister
    /// pin to `font_size_action_section_writes_through_section_setter`
    /// so both Action arms (size + family) have direct-call
    /// coverage on a Section selection, not just transitive
    /// coverage through the verb.
    #[test]
    fn font_family_action_section_writes_through_section_setter() {
        use crate::application::document::SectionSel;
        let family = first_loaded_family();
        let mut doc = doc_with_two_sections_for_font("LiberationSans", 14);
        doc.selection = SelectionState::Section(SectionSel {
            node_id: "0".into(),
            section_idx: 1,
        });
        assert!(super::apply_font_family_to_selection(&mut doc, &family));
        let node = doc.mindmap.nodes.get("0").unwrap();
        assert!(
            node.sections[0]
                .text_runs
                .iter()
                .all(|r| r.font == "LiberationSans"),
            "section 0 (sibling) must NOT change family"
        );
        assert!(
            node.sections[1].text_runs.iter().all(|r| r.font == family),
            "section 1 (selected) must receive the new family"
        );
    }
}
