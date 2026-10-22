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

use super::range_kv::TARGET_KEYS;
use super::Command;
use crate::application::console::completion::{prefix_filter, Completion};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::descent::{descend, Stop};
use crate::application::console::spec::{
    kvs, usage, Bare, Descent, Form, Grammar, Key, Slot, Subverb, Vocabulary,
};
use crate::application::console::traits::{apply_to_targets, AcceptsFontFamily};
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::SelectionState;

/// Point sizes the popup suggests. Any positive float is legal, so
/// these are *suggestions* rather than a vocabulary — hence
/// [`Vocabulary::Rows`], whose usage line prints `<pt>` and whose
/// popup prints the list.
pub const SIZE_PRESETS: &[&str] = &["10", "12", "14", "16", "18", "24", "32"];

fn size_rows(_ctx: &ConsoleContext, partial: &str) -> Vec<Completion> {
    prefix_filter(SIZE_PRESETS, partial)
}

const SIZE_VOCAB: Vocabulary = Vocabulary::Rows {
    placeholder: "pt",
    rows: size_rows,
    sentinels: &[],
};

fn family_rows(_ctx: &ConsoleContext, partial: &str) -> Vec<Completion> {
    font_family_completions(partial)
}

const FAMILY_VOCAB: Vocabulary = Vocabulary::Rows {
    placeholder: "family",
    rows: family_rows,
    sentinels: &[],
};

const KEYS: &[Key] = &[
    Key::new("size", "target on-screen size in points", SIZE_VOCAB),
    Key::new("min", "lower screen-space clamp in points", SIZE_VOCAB),
    Key::new("max", "upper screen-space clamp in points", SIZE_VOCAB),
];

const SUBVERBS: &[Subverb] = &[
    Subverb::bare("set", "family", "pin the font family on the current selection")
        .taking(&[Form::slots(&[Slot::req(FAMILY_VOCAB)]).reading(&["section", "range"])]),
    Subverb::bare(
        "list",
        "family",
        "list every loaded font, each rendered in its face",
    ),
];

pub static GRAMMAR: Grammar = Grammar {
    label: "font",
    subverb_sets: &[SUBVERBS],
    key_sets: &[KEYS, TARGET_KEYS],
    bare: Some(Bare::new(
        "composed",
        &[Form::keys(&["size"], &["min", "max", "section", "range"])],
    )),
};

pub const COMMAND: Command = Command {
    name: "font",
    aliases: &[],
    summary: "Set font family / size / clamps on the selection, or list fonts",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["family", "clamp", "pt", "smaller", "larger"],
    execute: execute_font,
};

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
///
/// `pub(crate)` because this is the console's *only* font-family
/// vocabulary, and the border family reaches it from seven slots:
/// `border font <TAB>` / `font=<TAB>`, `canvas border font` in both
/// forms, `canvas section-frame font=`, `section frame font=`, and
/// `border preview font=`. Those ran on a byte-near copy in
/// `border/complete.rs` that omitted the quoting above, so
/// tab-accepting any of the whitespace-bearing families — 43 of the
/// 77 this host loads — produced a line the verb then refused with
/// "'DejaVu' is not a loaded font". One body, so the two cannot
/// disagree about it again.
pub(crate) fn font_family_completions(partial: &str) -> Vec<Completion> {
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

/// One selection-wide font edit: the `(size, min, max)` triple
/// plus the optional `section=N` / `range=A..B` narrowing.
///
/// Built once by [`parse_font_args`] on the verb path and
/// synthesized from a single slot by
/// [`apply_font_kv_to_selection`] on the `Action` path. Both feed
/// the one selection dispatcher, [`apply_font_edit_to_selection`].
struct FontSizeEdit {
    size: Option<f32>,
    min: Option<f32>,
    max: Option<f32>,
    section_target: Option<usize>,
    range_target: Option<crate::application::document::GraphemeRange>,
}

/// Which surface a font edit landed on — the shape the verb needs
/// to render its scrollback line.
enum FontApplyScope {
    /// A whole node (every section's runs).
    Node,
    /// One section, optionally narrowed to a grapheme range.
    Section {
        idx: usize,
        range: Option<crate::application::document::GraphemeRange>,
    },
    /// A fan-out across `kind`-shaped targets (`"node"` /
    /// `"section"`), reported as a count.
    Fanout { kind: &'static str },
    /// A single edge-adjacent channel, named for the message
    /// (`"edge"`, `"edge label"`, `"portal label"`,
    /// `"portal text"`). These are the only surfaces that carry
    /// screen-space clamps.
    Channel { kind: &'static str },
}

/// Outcome of [`apply_font_edit_to_selection`].
///
/// `changed` is all the `Action` path needs (it gates the scene
/// rebuild); the other two fields carry exactly the extra context
/// the verb path needs to phrase its message. Having both paths
/// read one report is what keeps the keybind and the verb from
/// drifting — pre-dedup the two dispatchers disagreed on
/// `Multi` / `MultiSection` with `min` / `max`, where the verb
/// explained itself and the Action arm silently returned `false`.
struct FontApplyReport {
    /// Targets whose setter reported a real change.
    changed: usize,
    scope: FontApplyScope,
    /// `Some(noun)` when `min` / `max` were asked for on a surface
    /// that has no screen-space clamps.
    clamps_rejected: Option<&'static str>,
}

fn execute_font(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // The descent matches subverb names case-insensitively,
    // console-wide (`commands/mod.rs` § Casing). The *family* in
    // `set`'s slot is not: `font set Norse` names a loaded face,
    // and the loaded-family lookup owns that comparison.
    let descent = descend(&GRAMMAR, args.tokens());
    match descent.stop {
        Stop::Matched(subverb) => match subverb.name {
            "set" => execute_font_set(&descent, args, eff),
            _ => match kvs::read_strict(&descent, args) {
                Ok(_) => execute_font_list(),
                Err(msg) => ExecResult::err(msg),
            },
        },
        Stop::Bare => {
            let parsed = match parse_font_args(&descent, args) {
                Ok(p) => p,
                Err(e) => return e,
            };
            apply_font_args(eff.document_mut(), &parsed)
        }
        _ => ExecResult::err(usage::unknown_subverb_message(
            descent.level,
            descent.typed.unwrap_or_default(),
        )),
    }
}

/// Parse every recognized kv up front so the atomic application
/// sees a complete `FontSizeEdit` value. Returns `Err(ExecResult)`
/// for unknown keys, malformed values, mutually-exclusive
/// combinations, or empty input — all surface to the caller via
/// the same error channel `apply_font_args` would use.
fn parse_font_args(descent: &Descent, args: &Args) -> Result<FontSizeEdit, ExecResult> {
    let pairs = kvs::read_strict(descent, args).map_err(ExecResult::err)?;
    let target = super::range_kv::extract_section_range_kvs(&pairs, "font").map_err(ExecResult::err)?;
    let mut out = FontSizeEdit {
        size: None,
        min: None,
        max: None,
        section_target: target.section,
        range_target: target.range,
    };
    for pair in &pairs {
        let (k, v) = (pair.key.name, pair.value);
        match k {
            "size" => out.size = Some(parse_pt("size", v)?),
            "min" => out.min = Some(parse_pt("min", v)?),
            "max" => out.max = Some(parse_pt("max", v)?),
            // `section` / `range` were lifted out above; nothing
            // else reaches here, because the engine refused every
            // key this form does not read.
            _ => {}
        }
    }
    if out.range_target.is_some() && (out.min.is_some() || out.max.is_some()) {
        return Err(ExecResult::err(
            "font: min/max not applicable to a section range (per-grapheme clamps don't exist)",
        ));
    }
    if pairs.is_empty() {
        return Err(ExecResult::err(usage::no_arguments_message(&GRAMMAR)));
    }
    if out.size.is_none() && out.min.is_none() && out.max.is_none() {
        return Err(ExecResult::err("font: nothing to set"));
    }
    // Reject obviously-inverted explicit bounds up front so the
    // user sees a clear error instead of a silent no-op from the
    // setter's inverted-bounds guard. The setter still re-checks
    // against resolved (post-override) bounds for defense in
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

/// The one selection dispatcher for font edits.
///
/// Every font write — kv-form verb, `Action::SetFontKv` keybind,
/// macro step — resolves its target here. `Err` carries a
/// user-facing message (no selection, or a `range=` that starts
/// past the section's grapheme count); `Ok` reports what landed.
fn apply_font_edit_to_selection(
    doc: &mut crate::application::document::MindMapDocument,
    a: &FontSizeEdit,
) -> Result<FontApplyReport, String> {
    // Nodes and sections carry a font size but no screen-space
    // clamp pair — only the edge-adjacent channels do.
    let clamps_asked = a.min.is_some() || a.max.is_some();
    let section_report =
        |changed: usize, idx: usize, range: Option<crate::application::document::GraphemeRange>| {
            FontApplyReport {
                changed,
                scope: FontApplyScope::Section { idx, range },
                clamps_rejected: clamps_asked.then_some("section"),
            }
        };
    let channel_report = |changed: bool, kind: &'static str| FontApplyReport {
        changed: usize::from(changed),
        scope: FontApplyScope::Channel { kind },
        clamps_rejected: None,
    };
    match doc.selection.clone() {
        // `section=N` (E5 verb syntax) narrows to that section's
        // runs; absent, the write applies to the whole node.
        SelectionState::Single(id) => match a.section_target {
            Some(idx) => {
                let changed = write_section_font(doc, &id, idx, a.size, a.range_target)?;
                Ok(section_report(changed, idx, a.range_target))
            }
            None => Ok(FontApplyReport {
                changed: usize::from(a.size.map(|pt| doc.set_node_font_size(&id, pt)).unwrap_or(false)),
                scope: FontApplyScope::Node,
                clamps_rejected: clamps_asked.then_some("node"),
            }),
        },
        // Section selection: an explicit `section=N` overrides the
        // active section index; otherwise write the section the
        // user pointed at.
        SelectionState::Section(s) => {
            let idx = a.section_target.unwrap_or(s.section_idx);
            let changed = write_section_font(doc, &s.node_id, idx, a.size, a.range_target)?;
            Ok(section_report(changed, idx, a.range_target))
        }
        // SectionRange: layer an explicit `range=A..B` on top of
        // the selection's own grapheme range when both are set (kv
        // wins — explicit user input).
        SelectionState::SectionRange {
            sel, grapheme_range, ..
        } => {
            let idx = a.section_target.unwrap_or(sel.section_idx);
            let effective_range = a.range_target.or(Some(grapheme_range));
            let changed = write_section_font(doc, &sel.node_id, idx, a.size, effective_range)?;
            Ok(section_report(changed, idx, effective_range))
        }
        SelectionState::Multi(ids) => {
            let mut changed = 0usize;
            if let Some(pt) = a.size {
                for id in &ids {
                    changed += usize::from(doc.set_node_font_size(id, pt));
                }
            }
            Ok(FontApplyReport {
                changed,
                scope: FontApplyScope::Fanout { kind: "node" },
                clamps_rejected: clamps_asked.then_some("node"),
            })
        }
        SelectionState::MultiSection(secs) => {
            let mut changed = 0usize;
            if let Some(pt) = a.size {
                for s in &secs {
                    changed += usize::from(doc.set_section_font_size(&s.node_id, s.section_idx, pt));
                }
            }
            Ok(FontApplyReport {
                changed,
                scope: FontApplyScope::Fanout { kind: "section" },
                clamps_rejected: clamps_asked.then_some("section"),
            })
        }
        SelectionState::Edge(er) => Ok(channel_report(
            doc.set_edge_font(&er, a.size, a.min, a.max),
            "edge",
        )),
        SelectionState::EdgeLabel(s) => Ok(channel_report(
            doc.set_edge_label_font(&s.edge_ref, a.size, a.min, a.max),
            "edge label",
        )),
        // Portal icon routes to the owning edge's
        // `glyph_connection` channel (same sink as `Edge`).
        SelectionState::PortalLabel(s) => Ok(channel_report(
            doc.set_edge_font(&s.edge_ref(), a.size, a.min, a.max),
            "portal label",
        )),
        SelectionState::PortalText(s) => Ok(channel_report(
            doc.set_portal_text_font(&s.edge_ref(), &s.endpoint_node_id, a.size, a.min, a.max),
            "portal text",
        )),
        SelectionState::None => Err("font: no selection".to_string()),
    }
}

/// Write one section's font size, optionally narrowed to a
/// grapheme range. Returns the number of targets changed (0 or 1)
/// so the caller can fold it into a [`FontApplyReport`].
fn write_section_font(
    doc: &mut crate::application::document::MindMapDocument,
    node_id: &str,
    section_idx: usize,
    size: Option<f32>,
    range: Option<crate::application::document::GraphemeRange>,
) -> Result<usize, String> {
    // Pre-flight the range so an out-of-bounds start surfaces as a
    // clear error instead of the setter's silent no-op (which is
    // indistinguishable from "already that size").
    if let Some(rs) = range.map(|r| r.start()) {
        if let Some(section) = doc
            .mindmap
            .nodes
            .get(node_id)
            .and_then(|n| n.sections.get(section_idx))
        {
            let total = baumhard::util::grapheme_chad::count_grapheme_clusters(&section.text);
            if rs >= total {
                return Err(format!(
                    "font: range_start={} is past the section's grapheme count ({})",
                    rs, total
                ));
            }
        }
    }
    let Some(pt) = size else {
        return Ok(0);
    };
    let applied = match range {
        Some(r) => {
            let (rs, re) = (r.start(), r.end());
            let ok = doc.set_section_font_size_range(node_id, section_idx, rs, re, pt);
            if !ok {
                // Mirror the picker path's stale-range diagnostic
                // (see `color_picker_flow::commit`). The pre-flight
                // above already catches `rs >= total`; a `false`
                // here means `range_end` extends past the section's
                // grapheme count or the section was deleted.
                log::warn!(
                    "font on section {} of node {} range {}..{} produced no change \
                     (range may extend past the section's grapheme count or \
                     section was deleted)",
                    section_idx,
                    node_id,
                    rs,
                    re
                );
            }
            ok
        }
        None => doc.set_section_font_size(node_id, section_idx, pt),
    };
    Ok(usize::from(applied))
}

/// Verb-path adapter: run the shared core and render its report as
/// scrollback.
fn apply_font_args(doc: &mut crate::application::document::MindMapDocument, a: &FontSizeEdit) -> ExecResult {
    match apply_font_edit_to_selection(doc, a) {
        Ok(report) => font_report_to_exec(report),
        Err(msg) => ExecResult::err(msg),
    }
}

/// Render a [`FontApplyReport`] as the verb's scrollback line.
fn font_report_to_exec(report: FontApplyReport) -> ExecResult {
    let clamp_msg = report
        .clamps_rejected
        .map(|noun| format!("min/max: {}s have no screen-space clamps", noun));
    match report.scope {
        FontApplyScope::Node => single_target_result(report.changed, clamp_msg, "font applied"),
        FontApplyScope::Section { idx, range } => {
            let scope = match range {
                Some(r) => format!("section {} range {}..{}", idx, r.start(), r.end()),
                None => format!("section {}", idx),
            };
            single_target_result(report.changed, clamp_msg, &format!("font applied to {}", scope))
        }
        // The fan-out arms surface a single clamp message rather
        // than one per target — the sentence is identical across
        // targets; only the noun differs.
        FontApplyScope::Fanout { kind } => {
            if report.changed == 0 && clamp_msg.is_none() {
                return ExecResult::ok_msg("font: no change");
            }
            let mut lines = Vec::new();
            if report.changed > 0 {
                lines.push(format!("font: applied to {} {}(s)", report.changed, kind));
            }
            if let Some(m) = clamp_msg {
                lines.push(m);
            }
            ExecResult::lines(lines)
        }
        FontApplyScope::Channel { kind } => super::applied_or_no_change("font", kind, report.changed > 0),
    }
}

/// Shared tail for the single-target (node / section) arms: a
/// clamp rejection outranks the success line, and becomes a hard
/// error when nothing else applied.
fn single_target_result(changed: usize, clamp_msg: Option<String>, success: &str) -> ExecResult {
    if let Some(m) = clamp_msg {
        return if changed == 0 {
            ExecResult::err(m)
        } else {
            ExecResult::lines(vec![m])
        };
    }
    if changed == 0 {
        ExecResult::ok_msg("font: no change")
    } else {
        ExecResult::ok_msg(success.to_string())
    }
}

/// `font set <family>` — pin the font family on the current
/// selection through the `AcceptsFontFamily` trait. Validates the
/// family name against `list_loaded_families()` first; an unknown
/// name surfaces an error pointing the user at `font list`.
fn execute_font_set(descent: &Descent, args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // Collect every positional after `set` into one family name.
    // The tokenizer preserves quoted multi-word strings as one
    // token already (`font set "Norse Bold"`), but a user can also
    // bypass quoting entirely and just type `font set Norse Bold`
    // — joining positionals with a single space matches both shapes
    // and keeps the surface forgiving.
    // `set` reads a *run* of positionals rather than one slot, so
    // it takes `kvs::read` (keys only) rather than the strict read
    // its siblings use: `font set Norse Bold` is deliberately
    // accepted unquoted, and a strict read would refuse the second
    // word as a stray positional.
    let pairs = match kvs::read(descent, args) {
        Ok(pairs) => pairs,
        Err(msg) => return ExecResult::err(msg),
    };
    let positionals: Vec<&str> = args.positionals().collect();
    if positionals.len() < 2 {
        return ExecResult::err(usage::subverb_usage(descent.level, &SUBVERBS[0]));
    }
    let family = positionals[1..].join(" ");
    if family.is_empty() {
        return ExecResult::err(usage::subverb_usage(descent.level, &SUBVERBS[0]));
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
    let target = match super::range_kv::extract_section_range_kvs(&pairs, "font") {
        Ok(t) => t,
        Err(msg) => return ExecResult::err(msg),
    };
    let (section_target, range_target) = (target.section, target.range);
    if let (Some(idx), Some(range)) = (section_target, range_target) {
        let (rs, re) = (range.start(), range.end());
        let node_id = match eff.document().selection.clone() {
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
        let node_id = match eff.document().selection.clone() {
            SelectionState::Single(id) => id,
            SelectionState::Section(s) => s.node_id,
            SelectionState::SectionRange { sel, .. } => sel.node_id,
            _ => return ExecResult::err("font: section=N requires a node or section selection"),
        };
        let applied = eff
            .document_mut()
            .set_section_font_family(&node_id, idx, Some(&family));
        return if applied {
            ExecResult::ok_msg(format!("font set: {} on section {}", family, idx))
        } else {
            ExecResult::ok_msg("font: no change")
        };
    }
    let report = apply_to_targets(eff.document_mut(), |view| view.set_font_family(Some(&family)));
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

/// Mutation core: apply one font-size slot (`"size" | "min" |
/// "max"`) to the current selection for the parametric
/// `Action::SetFontKv` arm.
///
/// Routes through the same [`apply_font_edit_to_selection`]
/// dispatcher the kv-form verb uses, so the keybind and the verb
/// cannot disagree about which channel a selection writes to.
/// Rejections that the verb would print to scrollback are logged
/// here instead — the Action path has no scrollback, and a silent
/// no-op leaves the user with no signal at all.
///
/// Returns `true` on a real change; the bool gates the scene
/// rebuild.
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
    let edit = FontSizeEdit {
        size,
        min,
        max,
        section_target: None,
        range_target: None,
    };
    match apply_font_edit_to_selection(doc, &edit) {
        Ok(report) => {
            if let Some(noun) = report.clamps_rejected {
                log::info!(
                    "font {}: {}s have no screen-space clamps \
                     (Action path; no scrollback)",
                    which,
                    noun
                );
            }
            report.changed > 0
        }
        Err(msg) => {
            log::warn!("font {}: {} (Action path; no scrollback)", which, msg);
            false
        }
    }
}

/// `font list` — emit one scrollback line per loaded family, each
/// pinned to render in its own face (so a long list is a
/// font-by-font preview). Streams from `loaded_families_iter` so
/// no intermediate `Vec<String>` allocates.
fn execute_font_list() -> ExecResult {
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
    use crate::application::console::parser::{parse, ParseResult};
    use crate::application::console::tests::fixtures::{assert_exec_ok, first_node_id, join_lines, run};
    use crate::application::document::tests_common::load_test_doc as fixture_doc;
    use crate::application::document::{EdgeRef, SelectionState};

    /// The popup rows for `line` with the cursor at its end.
    fn popup(line: &str, doc: &crate::application::document::MindMapDocument) -> Vec<String> {
        let ctx = ConsoleContext::from_document(doc);
        crate::application::console::completion::complete(line, line.len(), &ctx)
            .into_iter()
            .map(|c| c.text)
            .collect()
    }

    /// `section=` is an index into the selected node's sections, so
    /// its popup must be those indices. It offered `SIZE_PRESETS`
    /// instead — the vocabulary of `size=`, a different key of the
    /// same verb — because the hand-written value-side arm matched
    /// the verb's whole key list and knew only one list to answer
    /// with. Each key declares its own vocabulary now, and this
    /// pins both halves of the pair the defect confused.
    #[test]
    fn test_font_section_value_completion_offers_section_indices() {
        let (mut doc, id) = crate::application::document::tests_common::pinned_two_section_node();
        doc.selection = SelectionState::Single(id);
        assert_eq!(popup("font section=", &doc), vec!["0", "1"]);
        assert_eq!(popup("font size=", &doc), SIZE_PRESETS.to_vec());
    }

    /// `range=` was accepted by `parse_font_args` and named in the
    /// verb's own error text, but reached no popup and no help page
    /// — the exact defect the declarative grammar exists to make
    /// impossible.
    ///
    /// It is structural now: the key is one row of `font`'s
    /// declaration, and `spec::tests` holds every level's keys
    /// against the forms that print them, in both directions, over
    /// the whole registry. What stays here is the popup and the
    /// `keys:` block, which the declaration alone does not prove
    /// reach the user.
    #[test]
    fn test_font_completion_offers_the_range_key() {
        let doc = fixture_doc();
        assert!(popup("font ", &doc).iter().any(|t| t == "range="));
        assert!(
            COMMAND.key_lines().iter().any(|l| l.starts_with("range=")),
            "`help font` must publish the key: {:?}",
            COMMAND.key_lines()
        );
    }

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

    /// Tab-accepting a multi-word family produces a line the verbs
    /// *run*. That is what the quoting in
    /// [`font_family_completions`] is for, and it is a stronger
    /// claim than the shape check in
    /// `completion_after_set_returns_loaded_families_in_their_face`,
    /// because it runs the inserted text back through the parser
    /// and the two verbs instead of comparing it to a formula.
    ///
    /// The bare spelling is the contrast, and it is checked as a
    /// token count rather than as a rejection: `border font Unifont
    /// CSUR` unquoted reaches `stage_font` as `Unifont`, which is
    /// itself a loaded family, so it succeeds while setting a font
    /// the user did not pick. Refusal is the *better* of the two
    /// failures an unquoted insert produces.
    ///
    /// This test used to build a `Completion` literal by hand and
    /// assert that same formula against it, exercising no
    /// production code at all: it passed unchanged while
    /// `border/complete.rs` served 43 of this host's 77 families
    /// from a completer that did not quote.
    #[test]
    fn completion_of_a_multi_word_family_round_trips_through_both_verbs() {
        baumhard::font::fonts::init();
        let mut doc = fixture_doc();
        doc.selection = SelectionState::Single(first_node_id(&doc));
        let multi_word: Vec<Completion> = font_family_completions("")
            .into_iter()
            .filter(|c| c.display.chars().any(char::is_whitespace))
            .collect();
        if multi_word.is_empty() {
            // Nothing to prove on a host whose fonts are all
            // single-word; say so rather than passing silently.
            eprintln!("no loaded family carries whitespace on this host — round-trip not exercised");
            return;
        }
        for c in &multi_word {
            // One token carrying the whole name — the property
            // quoting buys, read off the tokenizer rather than
            // inferred from the string's shape.
            match parse(&format!("border font {}", c.text)) {
                ParseResult::Ok { args, .. } => assert_eq!(
                    args,
                    vec!["font".to_string(), c.display.clone()],
                    "tab-accepting {:?} must reach the verb as one token",
                    c.display
                ),
                _ => panic!("`border font {}` should parse as a known command", c.text),
            }
            // …and the bare spelling, which is what the border
            // completer inserted until both surfaces shared one
            // body, does not.
            match parse(&format!("border font {}", c.display)) {
                ParseResult::Ok { args, .. } => assert!(
                    args.len() > 2,
                    "unquoted {:?} should split into several tokens",
                    c.display
                ),
                _ => panic!("`border font {}` should parse as a known command", c.display),
            }
            assert_exec_ok(run(&format!("font set {}", c.text), &mut doc));
            assert_exec_ok(run(&format!("border font {}", c.text), &mut doc));
        }
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
            assert_eq!(run.size_pt, 18.0);
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

    /// The named drift the dedup closed: on a `Multi` selection
    /// with `min` / `max`, the verb path used to surface an
    /// explanatory message while the Action path silently returned
    /// `false`. Both now route through the one dispatcher — the
    /// model outcome is identical and the verb still explains
    /// itself.
    #[test]
    fn min_max_on_multi_selection_agrees_between_verb_and_action() {
        let mut doc = fixture_doc();
        let ids: Vec<String> = doc.mindmap.nodes.keys().take(2).cloned().collect();
        doc.selection = SelectionState::Multi(ids);
        match run("font min=20", &mut doc) {
            ExecResult::Lines(rows) => assert!(
                join_lines(&rows).contains("nodes have no screen-space clamps"),
                "verb path must explain the clamp rejection: {:?}",
                rows
            ),
            other => panic!("expected Lines, got {:?}", other),
        }
        assert!(
            !super::apply_font_kv_to_selection(&mut doc, "min", 20.0),
            "Action path must agree with the verb: nothing changed"
        );
    }

    /// Same parity on `MultiSection`, where the noun differs.
    #[test]
    fn min_max_on_multi_section_agrees_between_verb_and_action() {
        let (mut doc, id) = crate::application::document::tests_common::pinned_two_section_node();
        doc.selection = SelectionState::MultiSection(vec![
            crate::application::document::SectionSel {
                node_id: id.clone(),
                section_idx: 0,
            },
            crate::application::document::SectionSel {
                node_id: id,
                section_idx: 1,
            },
        ]);
        match run("font max=40", &mut doc) {
            ExecResult::Lines(rows) => assert!(
                join_lines(&rows).contains("sections have no screen-space clamps"),
                "verb path must name the section noun: {:?}",
                rows
            ),
            other => panic!("expected Lines, got {:?}", other),
        }
        assert!(!super::apply_font_kv_to_selection(&mut doc, "max", 40.0));
    }

    /// A `Section` selection routes to that section on both paths
    /// — the verb through `section=`-less resolution, the Action
    /// through the same dispatcher. Sibling sections stay put.
    #[test]
    fn section_selection_size_agrees_between_verb_and_action() {
        let (mut doc, id) = crate::application::document::tests_common::pinned_two_section_node();
        doc.selection = SelectionState::Section(crate::application::document::SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        assert!(super::apply_font_kv_to_selection(&mut doc, "size", 21.0));
        let node = doc.mindmap.nodes.get(&id).expect("pinned node");
        for run in &node.sections[1].text_runs {
            assert_eq!(run.size_pt, 21.0, "Action write lands on the selected section");
        }
        // The verb reaches the same state, so re-running it is a
        // no-op rather than a second write.
        match run("font size=21", &mut doc) {
            ExecResult::Ok(msg) => assert!(
                msg.contains("no change"),
                "verb must agree the section is already at 21pt: {}",
                msg
            ),
            other => panic!("expected Ok, got {:?}", other),
        }
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
        let mut doc = doc_with_two_sections_for_font("LiberationSans", 14.0);
        doc.selection = SelectionState::Single("0".into());
        assert_exec_ok(run("font size=22 section=1", &mut doc));
        let node = doc.mindmap.nodes.get("0").unwrap();
        assert!(
            node.sections[0].text_runs.iter().all(|r| r.size_pt == 14.0),
            "section 0 must NOT receive the size change"
        );
        assert!(
            node.sections[1].text_runs.iter().all(|r| r.size_pt == 22.0),
            "section 1 must receive the new size"
        );
    }

    /// Build a node "0" with two sections, both pinned to the
    /// given `font` and `size_pt` so a per-section font/size write
    /// is observable. Thin wrapper around the shared
    /// `make_two_section_node_with_pinned_runs` helper.
    fn doc_with_two_sections_for_font(
        font: &str,
        size: f32,
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
        let mut doc = doc_with_two_sections_for_font("LiberationSans", 14.0);
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
    /// per-section behavior. Pre-Tier-2A this Action arm
    /// collapsed to whole-node, lagging behind the verb. Pins
    /// Item 10.
    #[test]
    fn font_size_action_section_writes_through_section_setter() {
        use crate::application::document::SectionSel;
        let mut doc = doc_with_two_sections_for_font("LiberationSans", 14.0);
        doc.selection = SelectionState::Section(SectionSel {
            node_id: "0".into(),
            section_idx: 1,
        });
        assert!(super::apply_font_kv_to_selection(&mut doc, "size", 22.0));
        let node = doc.mindmap.nodes.get("0").unwrap();
        assert!(
            node.sections[0].text_runs.iter().all(|r| r.size_pt == 14.0),
            "section 0 (sibling) must NOT change size"
        );
        assert!(
            node.sections[1].text_runs.iter().all(|r| r.size_pt == 22.0),
            "section 1 (selected) must receive the new size"
        );
    }

    /// `apply_font_family_to_selection(family)` (the parametric
    /// Action path) with a `SelectionState::Section` routes
    /// through `AcceptsFontFamily` → `set_section_font_family` —
    /// the same per-section behavior the verb path lands. Sister
    /// pin to `font_size_action_section_writes_through_section_setter`
    /// so both Action arms (size + family) have direct-call
    /// coverage on a Section selection, not just transitive
    /// coverage through the verb.
    #[test]
    fn font_family_action_section_writes_through_section_setter() {
        use crate::application::document::SectionSel;
        let family = first_loaded_family();
        let mut doc = doc_with_two_sections_for_font("LiberationSans", 14.0);
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
