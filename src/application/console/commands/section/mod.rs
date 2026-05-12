// SPDX-License-Identifier: MPL-2.0

//! `section …` — kv-form per-section verbs targeting either the
//! selection's section (when the selection is
//! `SelectionState::Section` / `SectionRange`) or an explicit
//! `section=K` kv (when the selection is a single node). Subverbs
//! (per `SECTIONS_BORDERS_RESIZE_PLAN.md` §4.5):
//!
//! - `section show [section=<idx>]` — multi-line resolved-property
//!   readout (text preview / runs / offset / size / channel /
//!   bindings / frame override).
//! - `section move dx=<f64> dy=<f64>` (delta) or
//!   `section move x=<f64> y=<f64>` (absolute) — delta and
//!   absolute forms are mutually exclusive; mixing rejects.
//! - `section resize w=<f64> h=<f64>` or `section resize fill` —
//!   the `fill` literal renames the prior `none` (which read as
//!   "remove the section"); `fill` clears `size = None` so the
//!   tree builder fills the parent's AABB.
//! - `section text "<text>" [runs=preserve|clear]` — replace
//!   text with optional run handling.
//! - `section add [at=<idx>] [text="<text>"]` — insert.
//! - `section delete [section=<idx>]` — remove.
//! - `section split [section=<idx>] at=<grapheme>` — split in
//!   two at a grapheme boundary.
//!
//! Validation messages on `move` / `resize` mirror
//! `crates/maptool/src/verify/sections.rs` so a verb-rejected
//! mutation and a `verify` violation read identically.
//!
//! ## `section frame …`
//!
//! Sibling subverb in [`frame`]: mirrors the top-level `border …`
//! kv vocabulary but writes to a section's
//! [`baumhard::mindmap::model::MindSection::frame_border`].
//! Dispatched here so all per-section verbs share the same parent
//! command surface in completion + help.

mod frame;

use super::Command;
use crate::application::console::completion::{
    kv_key_completions_with_hints, prefix_filter, Completion, CompletionContext, CompletionState,
};
use crate::application::console::parser::Args;
use crate::application::console::predicates::node_or_section_selected_single_node;
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::{MindMapDocument, SectionSel, SelectionState};

pub const KEYS: &[&str] = &["section"];
pub const VERBS: &[&str] = &[
    "move", "resize", "show", "text", "edit", "add", "delete", "split", "frame",
];

pub const COMMAND: Command = Command {
    name: "section",
    aliases: &[],
    summary: "Inspect, move, resize, edit text, or structurally modify a section (add / delete / split)",
    usage:
        "section show [section=<idx>] | section move dx=<f64> dy=<f64> [section=<idx>] | section move x=<f64> y=<f64> [section=<idx>] | section resize w=<f64> h=<f64> [section=<idx>] | section resize fill [section=<idx>] | section text \"<text>\" [section=<idx>] [runs=preserve|clear] | section edit [section=<idx>] | section add [at=<idx>] [text=\"<text>\"] | section delete [section=<idx>] | section split [section=<idx>] at=<grapheme> | section frame show|reset|<key>=<value> … [section=<idx>] | section frame preview <key>=<value> …|commit|cancel [section=<idx>]",
    tags: &[
        "section", "show", "info", "move", "resize", "offset", "size", "text", "add", "delete",
        "split", "frame", "border", "preset", "glyph", "preview",
    ],
    // Stricter than `border` — section subverbs need a single
    // node target (or section), so `Multi(_)` is excluded. Pre-
    // fix the shared predicate admitted Multi but the section
    // runtime rejected it with a generic catch-all error,
    // reintroducing the UX-vs-runtime mismatch Critical #5 was
    // meant to fix.
    applicable: node_or_section_selected_single_node,
    complete: complete_section,
    execute: execute_section,
};

fn complete_section(state: &CompletionState, ctx: &ConsoleContext) -> Vec<Completion> {
    // `state.tokens[0]` is the command name ("section"); the first
    // arg (`move`, `resize`, or `frame`) lives at index 1. The
    // engine's `Token { index }` already counts past the command,
    // so `index: 0` means "the user is typing the first positional
    // after `section`."
    let first_arg = state.tokens.get(1).map(String::as_str);
    // `frame` opens a sub-verb tree — once the user has typed
    // `section frame …` we delegate every later token to the
    // frame-specific completer (which surfaces the same kv keys
    // the `border …` verb uses).
    if first_arg == Some("frame") {
        return frame::complete_section_frame(state, ctx);
    }
    match &state.context {
        CompletionContext::Token { index: 0 } => verb_completions(state.partial),
        CompletionContext::Token { index: 1 } => match first_arg {
            // `section resize fill` is the only positional sentinel
            // — every other subverb takes kvs.
            Some("resize") => {
                let mut out = prefix_filter(&["fill"], state.partial);
                out.extend(kv_key_completions_with_hints(
                    &["w", "h", "section"],
                    state.partial,
                    kv_hint,
                ));
                out
            }
            Some("move") => {
                kv_key_completions_with_hints(&["dx", "dy", "x", "y", "section"], state.partial, kv_hint)
            }
            Some("text") => {
                kv_key_completions_with_hints(&["text", "runs", "section"], state.partial, kv_hint)
            }
            Some("add") => kv_key_completions_with_hints(&["at", "text"], state.partial, kv_hint),
            Some("split") => kv_key_completions_with_hints(&["at", "section"], state.partial, kv_hint),
            Some("delete") | Some("show") | Some("edit") => {
                kv_key_completions_with_hints(&["section"], state.partial, kv_hint)
            }
            _ => Vec::new(),
        },
        CompletionContext::Token { .. } => kv_key_completions_with_hints(KEYS, state.partial, kv_hint),
        // Value-side completers. The plan §4.5 spec calls for
        // selection-aware integer completion on `section=K`
        // showing `0..node.sections.len()` with each row's
        // hint surfacing the section's preview text.
        CompletionContext::KvValue { key } if key == "section" => {
            section_idx_value_completions(ctx, state.partial)
        }
        // `runs=preserve|clear` — static two-value enum.
        CompletionContext::KvValue { key } if key == "runs" => {
            prefix_filter(&["preserve", "clear"], state.partial)
        }
        _ => Vec::new(),
    }
}

/// `section <TAB>` at token 0 — surface every subverb with a
/// one-line hint per the sibling-consistency reviewer
/// (`border` / `font` / `color` already do this; section was
/// the outlier shipping hint-less verb rows).
fn verb_completions(partial: &str) -> Vec<Completion> {
    VERBS
        .iter()
        .filter(|v| v.starts_with(partial))
        .map(|v| Completion {
            text: v.to_string(),
            display: v.to_string(),
            hint: Some(verb_hint(v).to_string()),
            font_family: None,
        })
        .collect()
}

fn verb_hint(v: &str) -> &'static str {
    match v {
        "show" => "print the resolved per-section properties",
        "move" => "shift section offset (dx/dy delta or x/y absolute)",
        "resize" => "pin section size (w/h) or clear to fill-parent",
        "text" => "replace section text (runs=preserve|clear)",
        "add" => "insert a new section",
        "delete" => "remove the section (errors when only one remains)",
        "split" => "split a section in two at a grapheme boundary",
        "edit" => "open the section text editor on the resolved target",
        "frame" => "configure the section's frame border (subverb tree)",
        _ => "",
    }
}

/// Selection-aware integer completer for `section=<TAB>`.
/// Surfaces `0..node.sections.len()` for the selection's
/// primary node, with each row's hint showing a short text
/// preview so the user can tell which section is which.
fn section_idx_value_completions(ctx: &ConsoleContext, partial: &str) -> Vec<Completion> {
    use unicode_segmentation::UnicodeSegmentation;
    let Some(primary_id) = ctx.document.selection.primary_node_id() else {
        return Vec::new();
    };
    let Some(node) = ctx.document.mindmap.nodes.get(primary_id) else {
        return Vec::new();
    };
    node.sections
        .iter()
        .enumerate()
        .filter(|(idx, _)| idx.to_string().starts_with(partial))
        .map(|(idx, section)| {
            // Short text preview (≤20 graphemes, grapheme-aware so
            // a multi-codepoint emoji doesn't slice mid-cluster).
            // Empty sections render `(empty)` so the row isn't
            // a bare bullet.
            let preview: String = section.text.graphemes(true).take(20).collect();
            let hint = if preview.is_empty() {
                "(empty)".to_string()
            } else if section.text.graphemes(true).count() > 20 {
                format!("\"{}…\"", preview)
            } else {
                format!("\"{}\"", preview)
            };
            Completion {
                text: idx.to_string(),
                display: idx.to_string(),
                hint: Some(hint),
                font_family: None,
            }
        })
        .collect()
}

fn kv_hint(key: &str) -> Option<&'static str> {
    match key {
        "section" => Some("target section index inside a multi-section node"),
        "dx" => Some("relative move along x axis (canvas units)"),
        "dy" => Some("relative move along y axis (canvas units)"),
        "x" => Some("absolute x offset within parent node"),
        "y" => Some("absolute y offset within parent node"),
        "w" => Some("section width (canvas units)"),
        "h" => Some("section height (canvas units)"),
        "text" => Some("section text payload (quote multi-word values)"),
        "runs" => Some("preserve|clear — keep or drop per-grapheme styling"),
        "at" => Some("insertion / split index"),
        _ => None,
    }
}

fn execute_section(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let verb = match args.positional(0) {
        Some(v) => v,
        None => {
            return ExecResult::err(
                "usage: section move dx=<f64> dy=<f64> | section move x=<f64> y=<f64> | section resize w=<f64> h=<f64> | section resize fill | section show | section text \"<text>\" | section add | section delete | section split | section frame …",
            )
        }
    };
    // `frame` is a kv-form subverb whose own selection rules differ
    // from move/resize (it tolerates Single + section=K, walks
    // multiple nodes, doesn't require a positional dx/dy). Hand
    // off before move/resize's resolver runs.
    if verb == "frame" {
        return frame::execute_section_frame(args, eff);
    }
    // `add` resolves its own target — the `at=` kv supplies the
    // insertion index, and the parent node id comes from
    // `selection.primary_node_id()`. Route before the per-section
    // resolver so a Single(node) selection (no section pre-
    // selected) doesn't trip the "select a specific section"
    // error.
    if verb == "add" {
        let node_id = match resolve_node_id(&eff.document.selection) {
            Ok(id) => id,
            Err(msg) => return ExecResult::err(msg),
        };
        return execute_add(args, eff.document, &node_id);
    }
    // CRIT-1 (whole-PR review): validate the verb against the known
    // set BEFORE the per-section resolver runs. Pre-fix a `section
    // <typo>` against `Single(node)` (multi-section) ran
    // `resolve_section_idx` first, which surfaced the
    // "node 'X' has N sections — pick one" error — making the
    // typo masquerade as a selection problem. Surface the actual
    // typo with a grouped subverb listing (mirrors the `border`
    // verb's error shape at `border/execute.rs::unknown_subverb_message`).
    const KNOWN_VERBS: &[&str] = &[
        "move", "resize", "show", "text", "edit", "delete", "split", "frame", "add",
    ];
    if !KNOWN_VERBS.iter().any(|v| *v == verb) {
        return ExecResult::err(format!(
            "section: unknown subverb '{}'\n  \
             readout:   show\n  \
             text:      text \"<text>\" [runs=preserve|clear]\n  \
             geometry:  move dx=… dy=… | move x=… y=… | resize w=… h=… | resize fill\n  \
             structure: add | delete | split\n  \
             subject:   frame …  (per-section frame border)\n  \
             editor:    edit",
            verb
        ));
    }
    // `move` on a `MultiSection` selection with the **delta**
    // form (`dx=` / `dy=`) fans out to every targeted section —
    // each section's offset shifts by the same `(dx, dy)`. Plan
    // §4.5 rule 4 / §9.1.3. The absolute form (`x=` / `y=`) and
    // every other subverb stay single-target on MultiSection
    // (different sections + same absolute coords would all pile
    // up at the same offset, which is never the intent).
    if verb == "move" {
        if let SelectionState::MultiSection(_) = &eff.document.selection {
            // Peek the form before dispatching; only delta fans
            // out. The form-mismatch reject for absolute survives
            // through `execute_move` → `parse_move_kvs` paths.
            let has_delta = args.kvs().any(|(k, _)| k == "dx" || k == "dy");
            let has_abs = args.kvs().any(|(k, _)| k == "x" || k == "y");
            if has_delta && !has_abs {
                return execute_move_fan_out_multisection(args, eff.document);
            }
        }
    }
    let target_idx = match resolve_section_idx(args, &eff.document.selection, eff.document) {
        Ok(idx) => idx,
        Err(msg) => return ExecResult::err(msg),
    };
    let node_id = match resolve_node_id(&eff.document.selection) {
        Ok(id) => id,
        Err(msg) => return ExecResult::err(msg),
    };
    // Verify the index resolves before delegating — explicit
    // `section=99` should error, not silently return "no change"
    // (indistinguishable from a successful idempotent set).
    let section_count = eff
        .document
        .mindmap
        .nodes
        .get(&node_id)
        .map(|n| n.sections.len())
        .unwrap_or(0);
    if target_idx >= section_count {
        return ExecResult::err(format!("section[{}] not found on node '{}'", target_idx, node_id));
    }
    match verb {
        "move" => execute_move(args, eff.document, &node_id, target_idx),
        "resize" => execute_resize(args, eff.document, &node_id, target_idx),
        "show" => execute_show(args, eff.document, &node_id, target_idx),
        "text" => execute_text(args, eff.document, &node_id, target_idx),
        "edit" => execute_edit(args, eff, &node_id, target_idx),
        "delete" => execute_delete(args, eff.document, &node_id, target_idx),
        "split" => execute_split(args, eff.document, &node_id, target_idx),
        "add" => {
            // Should be routed earlier; defensive Err per
            // CODE_CONVENTIONS §9 (interactive paths must not panic).
            log::error!("section add reached the per-section dispatcher; expected upstream routing");
            ExecResult::err("internal: section add routing miss")
        }
        other => ExecResult::err(format!("section: unknown subverb '{}'", other)),
    }
}

/// Resolve `(node_id, section_idx)` from the current selection +
/// optional `section=K` kv.§906-920 selection rules:
///
/// 1. `section=K` kv → that index, with the selection's
///    primary node id.
/// 2. `Section(s)` / `SectionRange { sel, .. }` → `s.section_idx`.
/// 3. `Single(id)` AND `mindmap.nodes[id].sections.len() == 1`
///    → `(id, 0)` — single-section nodes don't need an explicit
///    `section=K` because there's only one option.///    rule 3 (line 914): closes the §5.7 hostile error.
/// 4. `Single(id)` on a multi-section node → error (the user
///    needs to pick one).
/// 5. `MultiSection(_)` → error with the single-target hint.
fn resolve_section_idx(
    args: &Args,
    selection: &SelectionState,
    doc: &MindMapDocument,
) -> Result<usize, String> {
    let kv_idx = parse_section_kv(args)?;
    match (selection, kv_idx) {
        (_, Some(idx)) => Ok(idx),
        (SelectionState::Section(SectionSel { section_idx, .. }), None) => Ok(*section_idx),
        (SelectionState::SectionRange { sel: SectionSel { section_idx, .. }, .. }, None) => {
            Ok(*section_idx)
        }
        (SelectionState::Single(id), None) => {
            //rule 3: a single-section node implies idx 0.
            // Multi-section nodes still require explicit selection.
            let n_sections = doc.mindmap.nodes.get(id).map(|n| n.sections.len()).unwrap_or(0);
            if n_sections == 1 {
                Ok(0)
            } else {
                Err(format!(
                    "section: node '{}' has {} sections — pick one (click) or pass section=<idx>",
                    id, n_sections
                ))
            }
        }
        (SelectionState::MultiSection(_), None) => Err(
            "section: multi-section selection — single-target only; pass section=<idx> or click one section first".into(),
        ),
        _ => Err("section: requires a node or section selection".into()),
    }
}

fn resolve_node_id(selection: &SelectionState) -> Result<String, String> {
    if let Some(id) = selection.primary_node_id() {
        return Ok(id.to_string());
    }
    if matches!(selection, SelectionState::MultiSection(_)) {
        return Err(
            "section: multi-section selection — single-target only; pass section=<idx> or click one section first".into(),
        );
    }
    Err("section: requires a node or section selection".into())
}

fn parse_section_kv(args: &Args) -> Result<Option<usize>, String> {
    for (k, v) in args.kvs() {
        if k == "section" {
            return super::range_kv::parse_section_kv("section", v).map(Some);
        }
    }
    Ok(None)
}

/// Reject any kv whose key isn't in `allowed`. Used by each
/// subverb (`show`, `text`, `add`, `delete`, `split`) to catch
/// typos like `sectoin=0` that pre-fix silently no-op'd. The
/// `move` and `resize` parsers already do this inline; this
/// helper covers the verbs that previously didn't.
fn reject_unknown_kvs(args: &Args, verb: &str, allowed: &[&str]) -> Result<(), String> {
    for (k, _) in args.kvs() {
        if !allowed.iter().any(|a| *a == k) {
            return Err(format!(
                "section {}: unknown key '{}'; use {}",
                verb,
                k,
                allowed.join("|")
            ));
        }
    }
    Ok(())
}

/// Multi-line readout of one section's resolved properties:
/// text preview, run count breakdown, offset, size (with the
/// fill-parent fallback noted), channel (with the index-default
/// noted), and trigger-binding count. Mirrors `border show`'s
/// shape — purely informational, no mutation.
fn execute_show(args: &Args, doc: &MindMapDocument, node_id: &str, idx: usize) -> ExecResult {
    if let Err(msg) = reject_unknown_kvs(args, "show", &["section"]) {
        return ExecResult::err(msg);
    }
    let Some(node) = doc.mindmap.nodes.get(node_id) else {
        return ExecResult::err(format!("section show: node '{}' not found", node_id));
    };
    let Some(section) = node.sections.get(idx) else {
        return ExecResult::err(format!(
            "section show: section[{}] not found on node '{}'",
            idx, node_id
        ));
    };

    // Run breakdown: count unique flag-bearing runs by axis. Two
    // bold runs spanning disjoint ranges count as 2; a single run
    // that's both bold + italic counts as 1 in each.
    let total_runs = section.text_runs.len();
    let bold = section.text_runs.iter().filter(|r| r.bold).count();
    let italic = section.text_runs.iter().filter(|r| r.italic).count();
    let underline = section.text_runs.iter().filter(|r| r.underline).count();
    let hyperlink = section.text_runs.iter().filter(|r| r.hyperlink.is_some()).count();

    // Text preview: cap at ~40 graphemes so a long section
    // doesn't overflow the readout. Stay grapheme-aware so we
    // don't slice mid-cluster. Single-pass: take 41 graphemes;
    // if 41 came back, we're past the cap and need an ellipsis.
    // Pre-fix this walked the iterator twice (`take(40).collect()`
    // + `count() > 40`), which is O(n) per call for the second
    // walk. The completion popup hits this path on every key
    // press in some flows.
    use unicode_segmentation::UnicodeSegmentation;
    let mut preview = String::with_capacity(160);
    let mut grapheme_count = 0usize;
    let mut overflow = false;
    for g in section.text.graphemes(true) {
        if grapheme_count == 40 {
            overflow = true;
            break;
        }
        preview.push_str(g);
        grapheme_count += 1;
    }
    let text_display = if overflow {
        format!("\"{}…\"", preview)
    } else {
        format!("\"{}\"", preview)
    };

    // Size readout: show the explicit Some pin, or annotate the
    // None case with the parent-derived effective size so the
    // user sees what the renderer is using. `{:.1}` keeps the
    // f64 type signal visible (0.0 prints as `0.0`, not `0`)
    // and bounds tiny-value rendering at one decimal place
    // (default `Display` for f64 prints `1e-20` as a string of
    // 20 zero digits).
    let size_display = match section.size {
        Some(s) => format!("Some({:.1} × {:.1}) [explicit pin]", s.width, s.height),
        None => format!(
            "None [fill parent: {:.1} × {:.1}]",
            node.size.width, node.size.height
        ),
    };

    // Channel readout: show the explicit Some, or annotate the
    // None case with the index the tree builder substitutes.
    let channel_display = match section.channel {
        Some(c) => format!("Some({})", c),
        None => format!("None [→ index {}]", idx),
    };

    // Plural / singular agreement on the bindings line —
    // "1 trigger" reads natural, "0 triggers" / "5 triggers"
    // also natural; "1 trigger(s)" was the pre-fix awkwardness
    // the section-show reviewer flagged.
    let n_bindings = section.trigger_bindings.len();
    let bindings_word = if n_bindings == 1 { "trigger" } else { "triggers" };

    // Run breakdown wording: the four sub-counts (bold /
    // italic / underline / hyperlink) overlap freely (a single
    // run can carry multiple flags), so the parenthetical isn't
    // a partition of the total. "flags:" reads as "this is a
    // breakdown across orthogonal axes" rather than "these
    // numbers sum to the total".
    let mut lines = vec![
        format!("section[{}] of node \"{}\"", idx, node_id),
        format!("  text:     {}", text_display),
        format!(
            "  runs:     {} runs (flags: {} bold, {} italic, {} underline, {} hyperlink)",
            total_runs, bold, italic, underline, hyperlink
        ),
        format!("  offset:   ({:.1}, {:.1})", section.offset.x, section.offset.y),
        format!("  size:     {}", size_display),
        format!("  channel:  {}", channel_display),
        format!("  bindings: {} {}", n_bindings, bindings_word),
    ];
    // Surface frame_border state. When a per-section override
    // is set, also surface the preset (the most useful one-line
    // identifier of the override's shape) so the user sees
    // *which* override is in force without running `section
    // frame show`. The richer-than-pre-fix readout was flagged
    // by the section-show reviewer.
    let frame_status = match &section.frame_border {
        Some(cfg) => format!("per-section override (preset={})", cfg.preset),
        None => "(falls back to canvas default / floor)".to_string(),
    };
    lines.push(format!("  frame:    {}", frame_status));
    ExecResult::lines(lines)
}

/// `section text "<text>" [section=<idx>] [runs=preserve|clear]` —
/// replace one section's text.
///
/// - `runs=preserve` (default) keeps existing runs to the extent
///   the new text supports them. Runs wholly inside the new
///   grapheme range carry through unchanged; runs straddling
///   the new end clip at `new_grapheme_count`; runs entirely
///   past the new end drop. Backed by
///   `set_section_text_preserving_runs`.
///
/// - `runs=clear` drops every prior run and lays down a single
///   run cloned from the first prior run's style attributes
///   (so the new text inherits the section's effective color /
///   font / size). Backed by `set_section_text`.
///
///§9.8: closes the "console paths can't change a
/// section's text" gap. Pre-fix `runs=preserve` was a phantom
/// kv — both branches called `set_section_text` (which collapses
/// runs unconditionally), so preserve and clear produced
/// identical output.
fn execute_text(args: &Args, doc: &mut MindMapDocument, node_id: &str, idx: usize) -> ExecResult {
    if let Err(msg) = reject_unknown_kvs(args, "text", &["text", "runs", "section"]) {
        return ExecResult::err(msg);
    }
    // Resolve the text payload: positional(1) or `text=` kv.
    // `text=` wins when both are present (the kv is the
    // explicit-named form; the positional is the convenient
    // shorthand).
    let kv_text = args.kvs().find(|(k, _)| *k == "text").map(|(_, v)| v.to_string());
    let new_text = match kv_text {
        Some(t) => t,
        None => match args.positional(1) {
            Some(t) => t.to_string(),
            None => {
                return ExecResult::err(
                    "usage: section text \"<text>\" [section=<idx>] [runs=preserve|clear]",
                )
            }
        },
    };

    // `runs=preserve|clear` controls run handling.
    let runs_mode = args.kvs().find(|(k, _)| *k == "runs").map(|(_, v)| v.to_string());
    let clear_runs = match runs_mode.as_deref() {
        Some("clear") => true,
        Some("preserve") | None => false,
        Some(other) => {
            return ExecResult::err(format!(
                "section text: runs='{}' not recognised; use 'preserve' or 'clear'",
                other
            ));
        }
    };

    if clear_runs {
        // Drop runs — `set_section_text` collapses to a single
        // run inheriting from the first prior run's color/font.
        let changed = doc.set_section_text(node_id, idx, new_text);
        return if changed {
            ExecResult::ok_msg(format!("section[{}] text replaced (runs cleared)", idx))
        } else {
            ExecResult::ok_msg("section: no change")
        };
    }
    // Preserve mode: keep prior runs clipped to the new text
    // length. Per-grapheme styling on overlapping ranges
    // survives; uncovered tail (when the new text is longer
    // than every prior run's `end`) falls through to section /
    // node defaults per `format/text-runs.md`.
    let changed = doc.set_section_text_preserving_runs(node_id, idx, new_text);
    if changed {
        ExecResult::ok_msg(format!("section[{}] text replaced (runs preserved)", idx))
    } else {
        ExecResult::ok_msg("section: no change")
    }
}

/// `section edit [section=<idx>]` — open the section text
/// editor on the resolved target.Routes through
/// `ConsoleSideEffect::OpenSectionEdit`; closes the console
/// (modal handoff to the editor).
fn execute_edit(args: &Args, eff: &mut ConsoleEffects, node_id: &str, idx: usize) -> ExecResult {
    if let Err(msg) = reject_unknown_kvs(args, "edit", &["section"]) {
        return ExecResult::err(msg);
    }
    eff.side_effect = Some(crate::application::console::ConsoleSideEffect::OpenSectionEdit {
        node_id: node_id.to_string(),
        section_idx: idx,
    });
    eff.close_console = true;
    ExecResult::ok_msg(format!("opening editor on section[{}]…", idx))
}

/// `section add [at=<idx>] [text="<text>"]` — insert a new
/// section. Routes through `MindMapDocument::add_section`. Plan
/// §4.5.
///
/// `at=` defaults to "append" (`None`); `text=` defaults to
/// empty string. The new section inherits the AABB / channel /
/// frame defaults documented on `MindSection`'s field-level
/// serde defaults — `offset = (0, 0)`, `size = None` (fill
/// parent), `channel = None` (→ index), `text_runs = []`,
/// `trigger_bindings = []`, `frame_border = None`.
fn execute_add(args: &Args, doc: &mut MindMapDocument, node_id: &str) -> ExecResult {
    use baumhard::mindmap::model::MindSection;

    if let Err(msg) = reject_unknown_kvs(args, "add", &["at", "text"]) {
        return ExecResult::err(msg);
    }
    let at_kv = match args.kvs().find(|(k, _)| *k == "at").map(|(_, v)| v.to_string()) {
        Some(v) => match v.parse::<usize>() {
            Ok(n) => Some(n),
            Err(_) => {
                return ExecResult::err(format!("section add: at='{}' is not a non-negative integer", v));
            }
        },
        None => None,
    };
    let text = args
        .kvs()
        .find(|(k, _)| *k == "text")
        .map(|(_, v)| v.to_string())
        .unwrap_or_default();

    let section = MindSection::new_default(text, Vec::new());

    match doc.add_section(node_id, at_kv, section) {
        Ok(idx) => ExecResult::ok_msg(format!("section[{}] added on node '{}'", idx, node_id)),
        Err(msg) => ExecResult::err(msg),
    }
}

/// `section delete [section=<idx>]` — remove a section. Routes
/// through `MindMapDocument::delete_section`.Errors
/// when the node has only one section (model invariant) or the
/// idx is out of range.
fn execute_delete(args: &Args, doc: &mut MindMapDocument, node_id: &str, idx: usize) -> ExecResult {
    if let Err(msg) = reject_unknown_kvs(args, "delete", &["section"]) {
        return ExecResult::err(msg);
    }
    match doc.delete_section(node_id, idx) {
        Ok(_removed) => ExecResult::ok_msg(format!("section[{}] deleted from node '{}'", idx, node_id)),
        Err(msg) => ExecResult::err(msg),
    }
}

/// `section split section=<idx> at=<grapheme>` — split a
/// section in two at the given grapheme boundary. Routes through
/// `MindMapDocument::split_section`.`at=` is now
/// **required** (per the Full-Nelson API/UX reviewer's foot-gun
/// finding): pre-fix the default was end-of-text, which created
/// an empty suffix section the user almost never wanted, and
/// the success message gave no hint that the new section was
/// empty. Forcing the user to spell out `at=` makes the intent
/// explicit; `section show` surfaces the section's grapheme
/// count so picking a value is one verb away.
fn execute_split(args: &Args, doc: &mut MindMapDocument, node_id: &str, idx: usize) -> ExecResult {
    if let Err(msg) = reject_unknown_kvs(args, "split", &["at", "section"]) {
        return ExecResult::err(msg);
    }
    let at_str = match args.kvs().find(|(k, _)| *k == "at").map(|(_, v)| v.to_string()) {
        Some(v) => v,
        None => {
            return ExecResult::err(
                "section split: at=<grapheme> required — \
                 pass an integer grapheme index (use `section show` to see \
                 the current section's grapheme count); split-at-end-of-text \
                 (the prior default) silently created an empty suffix section",
            );
        }
    };
    let at_grapheme = match at_str.parse::<usize>() {
        Ok(n) => n,
        Err(_) => {
            return ExecResult::err(format!(
                "section split: at='{}' is not a non-negative integer",
                at_str
            ));
        }
    };

    match doc.split_section(node_id, idx, Some(at_grapheme)) {
        Ok(new_idx) => ExecResult::ok_msg(format!(
            "section[{}] split — new section at index {}",
            idx, new_idx
        )),
        Err(msg) => ExecResult::err(msg),
    }
}

/// `section move dx=<f64> dy=<f64>` (delta) or `section move
/// x=<f64> y=<f64>` (absolute).kv form replaces the
/// pre-Batch-5 positional `<dx> <dy>`/// — no compatibility shim, users update muscle memory.
///
/// `dx`/`dy` and `x`/`y` are mutually exclusive: passing both
/// (`dx=1 x=2 dy=0 y=0`) is rejected at parse time so the user
/// gets a clear "pick one form" error rather than a silent
/// last-write-wins.
/// `section move dx=X dy=Y` against a `MultiSection` selection:
/// fan out the same `(dx, dy)` delta across every targeted
/// `(node_id, section_idx)` pair.rule 4 / §9.1.3 —
/// the only form where multi-target makes semantic sense
/// (every section shifts by the same delta; absolute coords
/// would collide).
///
/// **Atomicity**: parse-then-dispatch — pre-validate every
/// pair's would-be AABB via
/// `MindMapDocument::validate_section_offset_change`; if any
/// fails, abort the whole fan-out before mutating. Mirrors
/// `section/frame.rs::apply_edits`. On success, an N-section
/// fan-out produces N `EditNodeStyle` undo entries (same as
/// the per-pair setter — undo unwinds one section at a time).
fn execute_move_fan_out_multisection(args: &Args, doc: &mut MindMapDocument) -> ExecResult {
    let parsed = match parse_move_kvs(args) {
        Ok(p) => p,
        Err(msg) => return ExecResult::err(msg),
    };
    let (dx, dy) = match parsed {
        MoveTarget::Delta { dx, dy } => (dx, dy),
        MoveTarget::Absolute { .. } => {
            return ExecResult::err(
                "section move: absolute form (x=/y=) is single-target only on MultiSection",
            );
        }
    };

    let pairs: Vec<(String, usize)> = match &doc.selection {
        SelectionState::MultiSection(sels) => {
            sels.iter().map(|s| (s.node_id.clone(), s.section_idx)).collect()
        }
        _ => return ExecResult::err("section move: not a MultiSection selection"),
    };

    // Phase 1 — validate every pair's would-be offset. The first
    // rejection aborts the whole fan-out so partial mutation
    // can't land.
    let mut targets: Vec<(String, usize, f64, f64)> = Vec::with_capacity(pairs.len());
    for (node_id, idx) in &pairs {
        let Some(section) = doc.mindmap.nodes.get(node_id).and_then(|n| n.sections.get(*idx)) else {
            // Stale (node, idx) — the setter would silently
            // Ok(false). Skip without recording a target.
            continue;
        };
        let new_x = section.offset.x + dx;
        let new_y = section.offset.y + dy;
        if let Err(msg) = doc.validate_section_offset_change(node_id, *idx, new_x, new_y) {
            return ExecResult::err(format!(
                "section move: aborted on {}[{}] — {} (no sections moved)",
                node_id, idx, msg
            ));
        }
        targets.push((node_id.clone(), *idx, new_x, new_y));
    }

    // Phase 2 — apply. Validation already passed; the setter's
    // own validator is idempotent (re-runs the same checks) but
    // we trust phase 1's parse-then-dispatch.
    let mut moved = 0usize;
    for (node_id, idx, x, y) in &targets {
        match doc.set_section_offset(node_id, *idx, *x, *y) {
            Ok(true) => moved += 1,
            Ok(false) => {} // no-op (already at target offset)
            Err(msg) => {
                // Phase 1 said this would pass; the setter
                // disagreeing is a bug — log and continue
                // applying the rest (giving up halfway would be
                // worse than the loud-but-partial outcome).
                log::warn!(
                    "section move fan-out: setter rejected post-validation on {}[{}]: {} \
                     (validate_section_offset_change → set_section_offset drift)",
                    node_id,
                    idx,
                    msg
                );
            }
        }
    }

    ExecResult::ok_msg(format!(
        "section move: {} section(s) moved by ({}, {})",
        moved, dx, dy
    ))
}

fn execute_move(args: &Args, doc: &mut MindMapDocument, node_id: &str, idx: usize) -> ExecResult {
    let parsed = match parse_move_kvs(args) {
        Ok(p) => p,
        Err(msg) => return ExecResult::err(msg),
    };
    let (target_x, target_y) = match parsed {
        MoveTarget::Delta { dx, dy } => {
            let (current_x, current_y) = match doc
                .mindmap
                .nodes
                .get(node_id)
                .and_then(|n| n.sections.get(idx))
                .map(|s| (s.offset.x, s.offset.y))
            {
                Some(p) => p,
                None => {
                    return ExecResult::err(format!("section[{}] not found on node '{}'", idx, node_id));
                }
            };
            (current_x + dx, current_y + dy)
        }
        MoveTarget::Absolute { x, y } => (x, y),
    };
    match doc.set_section_offset(node_id, idx, target_x, target_y) {
        Ok(true) => ExecResult::ok_msg(format!("section[{}] moved", idx)),
        Ok(false) => ExecResult::ok_msg("section: no change"),
        Err(msg) => ExecResult::err(msg),
    }
}

/// Parsed `section move` arguments — either delta (`dx`/`dy`) or
/// absolute (`x`/`y`). Mixed forms (any of dx/dy combined with
/// any of x/y) reject at the parser level.
#[derive(Debug, Clone, Copy)]
enum MoveTarget {
    Delta { dx: f64, dy: f64 },
    Absolute { x: f64, y: f64 },
}

fn parse_move_kvs(args: &Args) -> Result<MoveTarget, String> {
    let mut dx: Option<f64> = None;
    let mut dy: Option<f64> = None;
    let mut x: Option<f64> = None;
    let mut y: Option<f64> = None;
    for (k, v) in args.kvs() {
        let target = match k {
            "dx" => &mut dx,
            "dy" => &mut dy,
            "x" => &mut x,
            "y" => &mut y,
            "section" => continue, // consumed by the resolver
            other => {
                return Err(format!(
                    "section move: unknown key '{}'; use dx|dy|x|y|section",
                    other
                ));
            }
        };
        let parsed: f64 = v
            .parse()
            .map_err(|_| format!("section move: {}='{}' is not a number", k, v))?;
        if !parsed.is_finite() {
            return Err(format!("section move: {}={} is not finite", k, v));
        }
        *target = Some(parsed);
    }
    let any_delta = dx.is_some() || dy.is_some();
    let any_abs = x.is_some() || y.is_some();
    if any_delta && any_abs {
        return Err("section move: cannot mix delta form (dx/dy) and absolute form (x/y) — pick one".into());
    }
    if !any_delta && !any_abs {
        return Err(
            "usage: section move dx=<f64> dy=<f64> | section move x=<f64> y=<f64> [section=<idx>]".into(),
        );
    }
    if any_delta {
        Ok(MoveTarget::Delta {
            dx: dx.unwrap_or(0.0),
            dy: dy.unwrap_or(0.0),
        })
    } else {
        // Absolute: missing axis defaults to 0.0 (mirrors delta's
        // posture). Authors who want to set just one axis can
        // write `section move x=10` and the other axis stays at
        // 0; if they want "leave x untouched" they use the delta
        // form with `dx=0`.
        Ok(MoveTarget::Absolute {
            x: x.unwrap_or(0.0),
            y: y.unwrap_or(0.0),
        })
    }
}

/// `section resize w=<f64> h=<f64>` or `section resize fill`.
///kv form replaces the pre-Batch-5 positional `<w>
/// <h>`; the `fill` literal replaces `none` ("none" reads as
/// "remove the section" rather than "fill the parent" — `fill`
/// is the clearer rename).
fn execute_resize(args: &Args, doc: &mut MindMapDocument, node_id: &str, idx: usize) -> ExecResult {
    // `fill` arrives as the first positional. Match case-
    // insensitively so users typing "FILL" or "Fill" don't
    // surprise themselves with a "not a number" parse error.
    if args.positional(1).map(str::to_ascii_lowercase).as_deref() == Some("fill") {
        return match doc.set_section_size(node_id, idx, None) {
            Ok(true) => ExecResult::ok_msg(format!("section[{}] size cleared (fill parent)", idx)),
            Ok(false) => ExecResult::ok_msg("section: no change"),
            Err(msg) => ExecResult::err(msg),
        };
    }
    let (w, h) = match parse_resize_kvs(args) {
        Ok(p) => p,
        Err(msg) => return ExecResult::err(msg),
    };
    let new_size = baumhard::mindmap::model::Size { width: w, height: h };
    match doc.set_section_size(node_id, idx, Some(new_size)) {
        Ok(true) => ExecResult::ok_msg(format!("section[{}] resized", idx)),
        Ok(false) => ExecResult::ok_msg("section: no change"),
        Err(msg) => ExecResult::err(msg),
    }
}

fn parse_resize_kvs(args: &Args) -> Result<(f64, f64), String> {
    let mut w: Option<f64> = None;
    let mut h: Option<f64> = None;
    for (k, v) in args.kvs() {
        let target = match k {
            "w" => &mut w,
            "h" => &mut h,
            "section" => continue,
            other => {
                return Err(format!(
                    "section resize: unknown key '{}'; use w|h|section",
                    other
                ));
            }
        };
        let parsed: f64 = v
            .parse()
            .map_err(|_| format!("section resize: {}='{}' is not a number", k, v))?;
        // Reject non-finite at the verb layer for parity with
        // `parse_move_kvs`. Pre-fix, NaN/Inf reached
        // `set_section_size`'s validator and surfaced a less
        // specific layer-mismatched message; rejecting here
        // keeps both forms diagnostically symmetric.
        if !parsed.is_finite() {
            return Err(format!("section resize: {}={} is not finite", k, v));
        }
        *target = Some(parsed);
    }
    let (Some(w), Some(h)) = (w, h) else {
        return Err("usage: section resize w=<f64> h=<f64> | section resize fill [section=<idx>]".into());
    };
    Ok((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::tests::fixtures::{assert_exec_err_contains, assert_exec_ok, run};
    use crate::application::console::ExecResult;
    use crate::application::document::tests_common::{load_test_doc, pinned_two_section_node};

    #[test]
    fn section_move_writes_offset_when_section_selection_supplies_idx() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        assert_exec_ok(run("section move dx=5 dy=7", &mut doc));
        let s = &doc.mindmap.nodes.get(&id).unwrap().sections[1];
        assert_eq!(s.offset.x, 15.0);
        assert_eq!(s.offset.y, 17.0);
    }

    #[test]
    fn section_move_kv_overrides_selection_idx() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Single(id.clone());
        assert_exec_ok(run("section move dx=3 dy=4 section=1", &mut doc));
        let s = &doc.mindmap.nodes.get(&id).unwrap().sections[1];
        assert_eq!(s.offset.x, 13.0);
        assert_eq!(s.offset.y, 14.0);
    }

    #[test]
    fn section_move_rejects_when_single_selection_lacks_section_kv() {
        //rule 3: `Single(id)` on a multi-section node
        // requires explicit `section=K`. A single-section node
        // would auto-resolve to (id, 0); the fixture here is
        // multi-section so the rejection branch runs.
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Single(id);
        assert_exec_err_contains(run("section move dx=3 dy=4", &mut doc), "has 2 sections");
    }

    ///rule 3: `Single(id)` on a single-section node
    /// implicitly resolves to `(id, 0)` — closes the §5.7
    /// "hostile error" the plan flagged.
    #[test]
    fn section_move_single_selection_auto_resolves_for_single_section_node() {
        // Use load_test_doc — many testament nodes are single-
        // section by construction.
        let mut doc = load_test_doc();
        // Pick the first node with exactly one section.
        let id = doc
            .mindmap
            .nodes
            .iter()
            .find(|(_, n)| n.sections.len() == 1)
            .map(|(id, _)| id.clone())
            .expect("testament map has at least one single-section node");
        doc.selection = SelectionState::Single(id.clone());
        // Should resolve and apply without errors — no `section=K`
        // required.
        assert_exec_ok(run("section move dx=0 dy=0", &mut doc));
    }

    #[test]
    fn section_move_rejects_aabb_overflow_with_verify_mirror_message() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        // section[1] starts at offset (10,10) size 50×30; node is
        // 200×100. Moving by (200,0) puts right edge at 260 > 200.
        assert_exec_err_contains(
            run("section move dx=200 dy=0", &mut doc),
            "extends past node right edge",
        );
    }

    #[test]
    fn section_move_rejects_negative_offset_with_verify_mirror_message() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        // Move (-50, 0) from offset (10,10) → -40, would-be negative.
        assert_exec_err_contains(
            run("section move dx=-50 dy=0", &mut doc),
            "section[1].offset.x is negative",
        );
    }

    #[test]
    fn section_move_rejects_unparseable_dx() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        assert_exec_err_contains(run("section move dx=not-a-number", &mut doc), "not a number");
    }

    #[test]
    fn section_move_no_change_returns_ok_msg() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        let result = run("section move dx=0 dy=0", &mut doc);
        assert!(matches!(result, ExecResult::Ok(_)));
    }

    #[test]
    fn section_move_round_trips_through_undo() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        assert_exec_ok(run("section move dx=7 dy=3", &mut doc));
        let s = &doc.mindmap.nodes.get(&id).unwrap().sections[1];
        assert_eq!(s.offset.x, 17.0);
        assert_eq!(s.offset.y, 13.0);
        assert!(doc.undo());
        let restored = &doc.mindmap.nodes.get(&id).unwrap().sections[1];
        assert_eq!(restored.offset.x, 10.0, "undo restores prior offset");
        assert_eq!(restored.offset.y, 10.0);
    }

    /// Out-of-range `section=K` errors at the verb layer rather
    /// than silently returning "no change" — pre-fix the setter's
    /// `Ok(false)` for unknown sections was indistinguishable
    /// from a successful idempotent set.
    #[test]
    fn section_move_out_of_range_section_kv_errors() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Single(id);
        assert_exec_err_contains(
            run("section move dx=1 dy=1 section=99", &mut doc),
            "not found on node",
        );
    }

    #[test]
    fn section_resize_writes_size() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        assert_exec_ok(run("section resize w=80 h=40", &mut doc));
        let s = &doc.mindmap.nodes.get(&id).unwrap().sections[1];
        assert_eq!(s.size.as_ref().unwrap().width, 80.0);
        assert_eq!(s.size.as_ref().unwrap().height, 40.0);
    }

    #[test]
    fn section_resize_none_clears_size() {
        let (mut doc, id) = pinned_two_section_node();
        // The fixture pins section[1] at offset (10, 10) with
        // an explicit size; `section resize fill` flatten-to-
        // fill-parent is only legal at offset (0, 0) post the
        // effective-size fix, so reset offset first.
        {
            let node = doc.mindmap.nodes.get_mut(&id).unwrap();
            node.sections[1].offset = baumhard::mindmap::model::Position { x: 0.0, y: 0.0 };
        }
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        assert_exec_ok(run("section resize fill", &mut doc));
        assert!(doc.mindmap.nodes.get(&id).unwrap().sections[1].size.is_none());
    }

    #[test]
    fn section_resize_rejects_overflow_with_verify_mirror_message() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        // Offset (10,10) + width 250 = 260 > node.size.width 200.
        assert_exec_err_contains(
            run("section resize w=250 h=30", &mut doc),
            "extends past node right edge",
        );
    }

    #[test]
    fn section_resize_rejects_zero_with_verify_mirror_message() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        assert_exec_err_contains(run("section resize w=0 h=30", &mut doc), "is not positive");
    }

    #[test]
    fn section_resize_rejects_astronomical_with_verify_mirror_message() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        // node.size.width=200, 100× = 20000. 25000 trips the typo guard.
        assert_exec_err_contains(
            run("section resize w=25000 h=30", &mut doc),
            "over 100× the node's width",
        );
    }

    #[test]
    fn section_resize_round_trips_through_undo() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        let before = doc.mindmap.nodes.get(&id).unwrap().sections[1].size.clone();
        assert_exec_ok(run("section resize w=80 h=40", &mut doc));
        assert!(doc.undo());
        let restored = doc.mindmap.nodes.get(&id).unwrap().sections[1].size.clone();
        assert_eq!(restored, before, "undo restores prior size");
    }

    #[test]
    fn section_unknown_subverb_errors() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        assert_exec_err_contains(run("section frobnicate 1 2", &mut doc), "unknown subverb");
    }

    /// Whole-PR review CRIT-1: a `section <typo>` against
    /// `Single(node)` (multi-section) used to run
    /// `resolve_section_idx` first, surfacing the
    /// "node 'X' has N sections — pick one" error and making the
    /// typo masquerade as a selection problem. After the fix, the
    /// verb-match runs first → the user sees "unknown subverb" +
    /// the grouped subverb listing.
    #[test]
    fn section_typo_against_single_selection_surfaces_unknown_subverb_not_selection_error() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Single(id);
        let result = run("section frobnicate", &mut doc);
        match result {
            ExecResult::Err(s) => {
                assert!(
                    s.contains("unknown subverb 'frobnicate'"),
                    "expected unknown-subverb error, got: {}",
                    s
                );
                assert!(
                    !s.contains("pick one (click) or pass section="),
                    "must not surface the selection-resolver error \
                     (the typo should not be misdiagnosed as a selection \
                     problem); got: {}",
                    s
                );
                // Grouped subverb listing surfaces the actual
                // vocabulary so the user can self-correct.
                for kind in &["readout:", "geometry:", "structure:", "subject:"] {
                    assert!(
                        s.contains(kind),
                        "expected grouped subverb listing to include '{}'; got: {}",
                        kind,
                        s
                    );
                }
            }
            other => panic!("expected Err for unknown subverb, got {:?}", other),
        }
    }

    ///NEW: absolute-move form via `x=` / `y=`.
    #[test]
    fn section_move_absolute_form_writes_offset_directly() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        // Section[1] starts at offset (10,10); absolute (3,7)
        // writes through to that exact offset.
        assert_exec_ok(run("section move x=3 y=7", &mut doc));
        let s = &doc.mindmap.nodes.get(&id).unwrap().sections[1];
        assert_eq!(s.offset.x, 3.0);
        assert_eq!(s.offset.y, 7.0);
    }

    /// Mixing delta and absolute kvs rejects with a clear
    /// diagnostic. Pre-fix, last-write-wins would have made the
    /// gesture's intent ambiguous.
    #[test]
    fn section_move_rejects_mixed_delta_and_absolute_form() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        assert_exec_err_contains(run("section move dx=1 x=2", &mut doc), "cannot mix delta form");
    }

    /// Empty kvs on `section move` yields the usage line, not
    /// a silent "no change" no-op (which would hide a missed
    /// argument from the user).
    #[test]
    fn section_move_no_kvs_errors_with_usage() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        assert_exec_err_contains(run("section move", &mut doc), "usage:");
    }

    /// Unknown kv on `section move` rejects with a key-list
    /// hint rather than silently accepting and producing a
    /// no-op.
    #[test]
    fn section_move_unknown_key_errors_with_hint() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        assert_exec_err_contains(run("section move foo=1", &mut doc), "unknown key 'foo'");
    }

    /// `section resize fill` (renamed from the prior `none`
    /// literal) clears `size` to fill-parent.    #[test]
    fn section_resize_fill_literal_clears_size() {
        let (mut doc, id) = pinned_two_section_node();
        // Move offset to (0,0) so the fill-parent state passes
        // section-AABB validation.
        {
            let node = doc.mindmap.nodes.get_mut(&id).unwrap();
            node.sections[1].offset = baumhard::mindmap::model::Position { x: 0.0, y: 0.0 };
        }
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        assert_exec_ok(run("section resize fill", &mut doc));
        assert!(doc.mindmap.nodes.get(&id).unwrap().sections[1].size.is_none());
    }

    #[test]
    fn section_no_selection_errors() {
        let mut doc = load_test_doc();
        doc.selection = SelectionState::None;
        assert_exec_err_contains(
            run("section move dx=1 dy=1", &mut doc),
            "requires a node or section selection",
        );
    }

    #[test]
    fn section_show_emits_resolved_readout() {
        let (mut doc, id) = pinned_two_section_node();
        doc.set_section_text(&id, 1, "hello world".to_string());
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        let result = run("section show", &mut doc);
        let blob = match result {
            ExecResult::Lines(ls) => ls.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n"),
            other => panic!("expected ExecResult::Lines, got {:?}", other),
        };
        assert!(blob.contains(&format!("section[1] of node \"{}\"", id)));
        assert!(blob.contains("text:"));
        assert!(
            blob.contains("hello world"),
            "preview must echo the text: {}",
            blob
        );
        assert!(blob.contains("offset:"));
        assert!(blob.contains("size:"));
        assert!(blob.contains("channel:"));
    }

    #[test]
    fn section_show_truncates_long_text_at_grapheme_boundary() {
        let (mut doc, id) = pinned_two_section_node();
        let long_text = "abcdefghijklmnopqrstuvwxyz1234567890ABCDEFGHIJ".to_string();
        doc.set_section_text(&id, 1, long_text);
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        let result = run("section show", &mut doc);
        let blob = match result {
            ExecResult::Lines(ls) => ls.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n"),
            other => panic!("expected ExecResult::Lines, got {:?}", other),
        };
        assert!(
            blob.contains("…"),
            "truncated preview must include ellipsis: {}",
            blob
        );
        assert!(
            !blob.contains("ABCDEFGHIJ"),
            "tail past 40 graphemes shouldn't appear"
        );
    }

    #[test]
    fn section_show_size_none_annotates_fill_parent() {
        let (mut doc, id) = pinned_two_section_node();
        // Section[1] starts with explicit size; clear to fill-
        // parent for this test (offset must be (0, 0) for the
        // None case to pass section-AABB validation).
        {
            let node = doc.mindmap.nodes.get_mut(&id).unwrap();
            node.sections[1].offset = baumhard::mindmap::model::Position { x: 0.0, y: 0.0 };
        }
        let _ = doc.set_section_size(&id, 1, None);
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        let result = run("section show", &mut doc);
        let blob = match result {
            ExecResult::Lines(ls) => ls.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n"),
            other => panic!("expected ExecResult::Lines, got {:?}", other),
        };
        assert!(
            blob.contains("None [fill parent:"),
            "fill-parent annotation missing: {}",
            blob
        );
    }

    #[test]
    fn section_show_channel_none_annotates_index_fallback() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        let result = run("section show", &mut doc);
        let blob = match result {
            ExecResult::Lines(ls) => ls.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n"),
            other => panic!("expected ExecResult::Lines, got {:?}", other),
        };
        assert!(
            blob.contains("None [→ index 1]"),
            "channel index-fallback annotation missing: {}",
            blob
        );
    }

    // ─── section text ──────────────────────────────────────────

    #[test]
    fn section_text_replaces_text_via_positional() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        assert_exec_ok(run("section text \"hello world\"", &mut doc));
        assert_eq!(
            doc.mindmap.nodes.get(&id).unwrap().sections[1].text,
            "hello world"
        );
    }

    #[test]
    fn section_text_kv_form_takes_precedence() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        assert_exec_ok(run("section text positional text=\"kv-wins\"", &mut doc));
        assert_eq!(doc.mindmap.nodes.get(&id).unwrap().sections[1].text, "kv-wins");
    }

    #[test]
    fn section_text_runs_clear_drops_runs() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        assert_exec_ok(run("section text \"plain text\" runs=clear", &mut doc));
        assert_eq!(doc.mindmap.nodes.get(&id).unwrap().sections[1].text, "plain text");
    }

    /// Pin the divergence between `runs=preserve` and
    /// `runs=clear`. Pre-fix both branches called
    /// `set_section_text` (which collapses runs), making the kv
    /// observably a phantom. The Full-Nelson runs-semantics
    /// reviewer flagged this as a critical bug.
    #[test]
    fn section_text_preserve_keeps_multi_runs_distinguishably_from_clear() {
        use baumhard::mindmap::model::TextRun;
        // Build two parallel docs from the same fixture so both
        // start with the same multi-run section[1]. `MindMapDocument`
        // doesn't impl Clone, so we set up each side identically
        // rather than clone.
        let seed_runs = vec![
            TextRun {
                start: 0,
                end: 3,
                bold: true,
                italic: false,
                underline: false,
                font: "Sans".into(),
                size_pt: 12,
                color: "#ff0000".into(),
                hyperlink: None,
            },
            TextRun {
                start: 3,
                end: 6,
                bold: false,
                italic: true,
                underline: false,
                font: "Sans".into(),
                size_pt: 12,
                color: "#00ff00".into(),
                hyperlink: None,
            },
        ];

        let (mut doc_preserve, id_p) = pinned_two_section_node();
        doc_preserve.set_section_text(&id_p, 1, "abcdef".to_string());
        doc_preserve.mindmap.nodes.get_mut(&id_p).unwrap().sections[1].text_runs = seed_runs.clone();
        doc_preserve.selection = SelectionState::Section(SectionSel {
            node_id: id_p.clone(),
            section_idx: 1,
        });

        let (mut doc_clear, id_c) = pinned_two_section_node();
        doc_clear.set_section_text(&id_c, 1, "abcdef".to_string());
        doc_clear.mindmap.nodes.get_mut(&id_c).unwrap().sections[1].text_runs = seed_runs;
        doc_clear.selection = SelectionState::Section(SectionSel {
            node_id: id_c.clone(),
            section_idx: 1,
        });

        // New text differs from prior so the setters' identity-
        // shortcircuit doesn't bypass the run handling.
        // Preserve: same length (6 graphemes) → both runs survive
        // intact at their original [0..3) and [3..6) positions.
        assert_exec_ok(run("section text \"ABCDEF\" runs=preserve", &mut doc_preserve));
        let preserve_runs = &doc_preserve.mindmap.nodes.get(&id_p).unwrap().sections[1].text_runs;
        assert_eq!(
            preserve_runs.len(),
            2,
            "runs=preserve must keep both runs: {:?}",
            preserve_runs
        );
        assert!(preserve_runs[0].bold);
        assert!(preserve_runs[1].italic);

        // Clear: collapses to one run regardless.
        assert_exec_ok(run("section text \"ABCDEF\" runs=clear", &mut doc_clear));
        let clear_runs = &doc_clear.mindmap.nodes.get(&id_c).unwrap().sections[1].text_runs;
        assert_eq!(
            clear_runs.len(),
            1,
            "runs=clear must collapse to one run: {:?}",
            clear_runs
        );
    }

    /// Preserve mode clips runs that straddle or overflow the
    /// new (shorter) text length. Uncovered tail falls through
    /// to section / node defaults per `format/text-runs.md`.
    #[test]
    fn section_text_preserve_clips_runs_to_shorter_text() {
        use baumhard::mindmap::model::TextRun;
        let (mut doc, id) = pinned_two_section_node();
        doc.set_section_text(&id, 1, "abcdef".to_string());
        {
            let node = doc.mindmap.nodes.get_mut(&id).unwrap();
            node.sections[1].text_runs = vec![TextRun {
                start: 0,
                end: 6,
                bold: true,
                italic: false,
                underline: false,
                font: "Sans".into(),
                size_pt: 12,
                color: "#ff0000".into(),
                hyperlink: None,
            }];
        }
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        // New text is 3 graphemes; the run [0..6) clips to [0..3).
        assert_exec_ok(run("section text \"abc\" runs=preserve", &mut doc));
        let runs = &doc.mindmap.nodes.get(&id).unwrap().sections[1].text_runs;
        assert_eq!(runs.len(), 1);
        assert_eq!(
            (runs[0].start, runs[0].end),
            (0, 3),
            "run must clip to new grapheme count"
        );
    }

    #[test]
    fn section_text_invalid_runs_value_errors() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        assert_exec_err_contains(run("section text \"x\" runs=invalid", &mut doc), "not recognised");
    }

    #[test]
    fn section_text_no_payload_errors_with_usage() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        assert_exec_err_contains(run("section text", &mut doc), "usage:");
    }

    // ─── section add ───────────────────────────────────────────

    #[test]
    fn section_add_appends_when_no_at_kv() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Single(id.clone());
        let original_len = doc.mindmap.nodes.get(&id).unwrap().sections.len();
        assert_exec_ok(run("section add text=\"appended\"", &mut doc));
        assert_eq!(
            doc.mindmap.nodes.get(&id).unwrap().sections.len(),
            original_len + 1
        );
        assert_eq!(
            doc.mindmap.nodes.get(&id).unwrap().sections[original_len].text,
            "appended"
        );
    }

    #[test]
    fn section_add_at_index_inserts() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Single(id.clone());
        assert_exec_ok(run("section add at=0 text=\"prepended\"", &mut doc));
        assert_eq!(doc.mindmap.nodes.get(&id).unwrap().sections[0].text, "prepended");
    }

    #[test]
    fn section_add_rejects_invalid_at() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Single(id);
        assert_exec_err_contains(
            run("section add at=not-a-number", &mut doc),
            "not a non-negative integer",
        );
    }

    // ─── section delete ────────────────────────────────────────

    #[test]
    fn section_delete_removes_at_selected_section_idx() {
        let (mut doc, id) = pinned_two_section_node();
        let len_before = doc.mindmap.nodes.get(&id).unwrap().sections.len();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        assert_exec_ok(run("section delete", &mut doc));
        assert_eq!(doc.mindmap.nodes.get(&id).unwrap().sections.len(), len_before - 1);
    }

    #[test]
    fn section_delete_kv_form_overrides_selection() {
        let (mut doc, id) = pinned_two_section_node();
        let len_before = doc.mindmap.nodes.get(&id).unwrap().sections.len();
        doc.selection = SelectionState::Single(id.clone());
        assert_exec_ok(run("section delete section=0", &mut doc));
        assert_eq!(doc.mindmap.nodes.get(&id).unwrap().sections.len(), len_before - 1);
    }

    #[test]
    fn section_delete_rejects_last_remaining_section() {
        let (mut doc, id) = pinned_two_section_node();
        // Force down to 1 section.
        let _ = doc.delete_section(&id, 1);
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 0,
        });
        assert_exec_err_contains(run("section delete", &mut doc), "only section");
    }

    // ─── section split ─────────────────────────────────────────

    #[test]
    fn section_split_at_grapheme_kv() {
        let (mut doc, id) = pinned_two_section_node();
        doc.set_section_text(&id, 1, "abcdef".to_string());
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        assert_exec_ok(run("section split at=3", &mut doc));
        let sections = &doc.mindmap.nodes.get(&id).unwrap().sections;
        assert_eq!(sections[1].text, "abc");
        assert_eq!(sections[2].text, "def");
    }

    /// Pre-fix `section split` defaulted to end-of-text (empty
    /// suffix), creating a useless empty section the user
    /// almost never wanted (Full-Nelson API/UX reviewer B2 —
    /// foot-gun finding). Post-fix `at=` is required; the bare
    /// `section split` form errors with a hint pointing at
    /// `section show` for the grapheme count.
    #[test]
    fn section_split_no_at_rejects_with_hint() {
        let (mut doc, id) = pinned_two_section_node();
        doc.set_section_text(&id, 1, "abc".to_string());
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        let result = run("section split", &mut doc);
        assert_exec_err_contains(result, "at=<grapheme> required");
    }

    /// Explicit `at=N` still works — pin the happy path post-
    /// requirement-tightening to match the old default
    /// (empty-suffix) behaviour at the user's explicit choice.
    #[test]
    fn section_split_at_end_of_text_creates_empty_suffix() {
        let (mut doc, id) = pinned_two_section_node();
        doc.set_section_text(&id, 1, "abc".to_string());
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        let len_before = doc.mindmap.nodes.get(&id).unwrap().sections.len();
        // 3 graphemes ("abc") → split at index 3 = end-of-text
        // (the prior silent default; now spelled out).
        assert_exec_ok(run("section split at=3", &mut doc));
        let sections = &doc.mindmap.nodes.get(&id).unwrap().sections;
        assert_eq!(sections.len(), len_before + 1);
        assert_eq!(sections[1].text, "abc");
        assert_eq!(sections[2].text, "");
    }

    #[test]
    fn section_split_rejects_invalid_at() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        assert_exec_err_contains(
            run("section split at=not-a-number", &mut doc),
            "not a non-negative integer",
        );
    }

    // ─── Round 2 review: typo-rejection + completion hints ─────

    /// Pin the silent-typo rejection added per the Full-Nelson
    /// UX reviewer. `section delete sectoin=0` (typo) was a
    /// silent no-op pre-fix because the only kv each subverb
    /// read was its named one — unknown kvs flowed through
    /// without complaint. Now every subverb error-rejects.
    #[test]
    fn section_text_rejects_unknown_kv_with_typo_hint() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        assert_exec_err_contains(
            run("section text \"x\" txet=hello", &mut doc),
            "unknown key 'txet'",
        );
    }

    #[test]
    fn section_delete_rejects_unknown_kv_with_typo_hint() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        assert_exec_err_contains(run("section delete sectoin=0", &mut doc), "unknown key 'sectoin'");
    }

    /// `section <TAB>` produces verb rows with hints — pre-fix
    /// the popup showed bare verb names. Sibling consistency
    /// with `border` / `font` / `color`.
    #[test]
    fn section_completion_token_zero_emits_hints() {
        use crate::application::console::completion::{CompletionContext, CompletionState};
        let doc = load_test_doc();
        let ctx = crate::application::console::ConsoleContext::from_document(&doc);
        let tokens = vec!["section".to_string()];
        let state = CompletionState {
            tokens: &tokens,
            cursor_token: 0,
            partial: "",
            context: CompletionContext::Token { index: 0 },
        };
        let out = complete_section(&state, &ctx);
        // Every verb has a hint.
        assert!(!out.is_empty());
        for c in &out {
            assert!(
                c.hint.as_ref().map_or(false, |h| !h.is_empty()),
                "verb '{}' missing hint",
                c.text
            );
        }
        // Spot-check one specific verb.
        let show = out.iter().find(|c| c.text == "show").expect("show in list");
        assert!(
            show.hint.as_ref().unwrap().contains("resolved"),
            "show hint mentions resolved properties: {:?}",
            show.hint
        );
    }

    /// Selection-aware integer completion for `section=<TAB>`:
    /// surfaces `0..node.sections.len()` for the selection's
    /// primary node, with each row's hint showing a text
    /// preview. Pre-fix the value side returned empty — Plan
    /// §4.5 line 981 spec'd this as the discoverability path.
    #[test]
    fn section_kv_value_completion_lists_indices_with_text_preview() {
        use crate::application::console::completion::{CompletionContext, CompletionState};
        let (mut doc, id) = pinned_two_section_node();
        // Seed distinct text on each section so the previews
        // round-trip distinguishably.
        doc.set_section_text(&id, 0, "first".to_string());
        doc.set_section_text(&id, 1, "second".to_string());
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 0,
        });
        let ctx = crate::application::console::ConsoleContext::from_document(&doc);
        let tokens = vec!["section".to_string(), "show".to_string()];
        let state = CompletionState {
            tokens: &tokens,
            cursor_token: 0,
            partial: "",
            context: CompletionContext::KvValue {
                key: "section".to_string(),
            },
        };
        let out = complete_section(&state, &ctx);
        let labels: Vec<&str> = out.iter().map(|c| c.text.as_str()).collect();
        assert!(
            labels.iter().any(|l| l == &"0"),
            "idx 0 in completion: {:?}",
            labels
        );
        assert!(
            labels.iter().any(|l| l == &"1"),
            "idx 1 in completion: {:?}",
            labels
        );
        // Hints surface the section text preview.
        let row0 = out.iter().find(|c| c.text == "0").unwrap();
        assert!(
            row0.hint.as_ref().unwrap().contains("first"),
            "row 0 hint must include text preview: {:?}",
            row0.hint
        );
    }

    /// `runs=<TAB>` surfaces the two-value enum.
    #[test]
    fn section_runs_kv_value_completion_lists_preserve_clear() {
        use crate::application::console::completion::{CompletionContext, CompletionState};
        let doc = load_test_doc();
        let ctx = crate::application::console::ConsoleContext::from_document(&doc);
        let tokens = vec!["section".to_string(), "text".to_string()];
        let state = CompletionState {
            tokens: &tokens,
            cursor_token: 0,
            partial: "",
            context: CompletionContext::KvValue {
                key: "runs".to_string(),
            },
        };
        let out = complete_section(&state, &ctx);
        let labels: Vec<&str> = out.iter().map(|c| c.text.as_str()).collect();
        assert!(labels.contains(&"preserve"));
        assert!(labels.contains(&"clear"));
    }

    // ───rule 4: MultiSection fan-out for `move dx/dy` ─

    /// `section move dx=X dy=Y` against a `MultiSection`
    /// selection shifts each targeted section by the same
    /// `(dx, dy)`. Pre-fix this rejected MultiSection
    /// uniformly; the plan §4.5 line 917 spec'd the fan-out
    /// for the delta form specifically (absolute coords would
    /// pile up).
    #[test]
    fn section_move_delta_fan_out_across_multi_section() {
        let (mut doc, id) = pinned_two_section_node();
        // Pre-fixture: section[0] is fill-parent (size=None) so
        // any non-zero offset on it would overflow the validator.
        // Pin both sections to explicit small sizes so both can
        // shift by the same delta without overflowing the
        // 200×100 parent AABB.
        {
            let node = doc.mindmap.nodes.get_mut(&id).unwrap();
            node.sections[0].offset = baumhard::mindmap::model::Position { x: 5.0, y: 5.0 };
            node.sections[0].size = Some(baumhard::mindmap::model::Size {
                width: 50.0,
                height: 30.0,
            });
            // section[1] keeps the fixture's pinned (10, 10) +
            // 50×30.
        }
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
        let before_0 = doc.mindmap.nodes.get(&id).unwrap().sections[0].offset;
        let before_1 = doc.mindmap.nodes.get(&id).unwrap().sections[1].offset;
        // +(5, 7) keeps both within the parent.
        let result = run("section move dx=5 dy=7", &mut doc);
        match result {
            ExecResult::Ok(_) => {}
            other => panic!("expected Ok, got {:?}", other),
        }
        let after_0 = doc.mindmap.nodes.get(&id).unwrap().sections[0].offset;
        let after_1 = doc.mindmap.nodes.get(&id).unwrap().sections[1].offset;
        assert_eq!(after_0.x, before_0.x + 5.0);
        assert_eq!(after_0.y, before_0.y + 7.0);
        assert_eq!(after_1.x, before_1.x + 5.0);
        assert_eq!(after_1.y, before_1.y + 7.0);
    }

    /// `section move x=A y=B` (absolute) against MultiSection
    /// stays single-target — fan-out would collide every
    /// section at the same offset, which is never the intent.
    /// The verb path falls through to `resolve_section_idx`
    /// which rejects with the existing single-target message.
    #[test]
    fn section_move_absolute_form_on_multi_section_rejects_single_target() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::MultiSection(vec![
            SectionSel {
                node_id: id.clone(),
                section_idx: 0,
            },
            SectionSel {
                node_id: id,
                section_idx: 1,
            },
        ]);
        assert_exec_err_contains(run("section move x=3 y=7", &mut doc), "single-target only");
    }

    /// Other subverbs on MultiSection still reject (resize /
    /// text / delete / split don't have a fan-out semantic).
    /// Pin to lock the asymmetry.
    #[test]
    fn section_resize_on_multi_section_rejects_single_target() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::MultiSection(vec![
            SectionSel {
                node_id: id.clone(),
                section_idx: 0,
            },
            SectionSel {
                node_id: id,
                section_idx: 1,
            },
        ]);
        assert_exec_err_contains(run("section resize w=80 h=40", &mut doc), "single-target only");
    }

    // ─── §4.5: section edit subverb ────────────────────────────

    /// `section edit` queues `OpenSectionEdit` side-effect with
    /// the resolved (node, idx). The actual editor open happens
    /// in the dispatcher (post-rebuild); the verb's job is to
    /// validate + emit the side-effect + close the console.
    #[test]
    fn section_edit_emits_open_section_edit_side_effect() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        let mut effects = crate::application::console::ConsoleEffects::new(&mut doc);
        let args_owned: Vec<String> = vec!["edit".to_string()];
        let result = execute_section(&Args::new(&args_owned), &mut effects);
        assert!(matches!(result, ExecResult::Ok(_)));
        match &effects.side_effect {
            Some(crate::application::console::ConsoleSideEffect::OpenSectionEdit {
                node_id,
                section_idx,
            }) => {
                assert_eq!(node_id, &id);
                assert_eq!(*section_idx, 1);
            }
            other => panic!("expected OpenSectionEdit, got {:?}", other),
        }
        assert!(effects.close_console);
    }

    /// `section edit` validates the resolved index against the
    /// node's section count before issuing the side-effect.
    /// Out-of-range errors cleanly without leaving a dangling
    /// modal-open request. Routes through `execute_section`
    /// (the upstream resolver), so the err message is the
    /// resolver's "section[99] not found on node 'X'", and
    /// the verb body never runs.
    #[test]
    fn section_edit_rejects_out_of_range_section_kv() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Single(id);
        assert_exec_err_contains(run("section edit section=99", &mut doc), "not found on node");
    }

    /// Sharpened pin per Test Quality #5/6: when the verb's
    /// own `reject_unknown_kvs` rejects (the only `execute_edit`
    /// path that fires before the side-effect emit), no
    /// `OpenSectionEdit` side-effect is emitted and
    /// `close_console` stays false. Catches a regression that
    /// might emit-then-error on a typo'd kv.
    #[test]
    fn section_edit_unknown_kv_emits_no_side_effect() {
        let (mut doc, id) = pinned_two_section_node();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 0,
        });
        let mut effects = crate::application::console::ConsoleEffects::new(&mut doc);
        let args_owned: Vec<String> = vec!["edit".to_string(), "bogus=42".to_string()];
        let result = execute_section(&Args::new(&args_owned), &mut effects);
        assert!(matches!(result, ExecResult::Err(_)));
        assert!(
            effects.side_effect.is_none(),
            "rejection must NOT emit a side-effect: {:?}",
            effects.side_effect
        );
        assert!(!effects.close_console, "rejection must NOT close the console");
    }
}
