// SPDX-License-Identifier: MPL-2.0

//! `color bg=#009c15 text=accent border=reset` — kv-form color
//! setter dispatched through the capability traits. Each key maps to
//! a trait (`bg` → HasBgColor, `text` → HasTextColor, `border` →
//! HasBorderColor). Fans out over the selection; reports per-pair
//! outcome so a pair that's not applicable to one target doesn't
//! sink the whole command.
//!
//! Axis-only positionals (`color bg`, `color text`, `color border`)
//! and the legacy `color pick` both hand off to the glyph-wheel
//! picker modal — `color bg` picks a color for that axis on the
//! current selection.

use super::Command;
use crate::application::color_picker::{ColorTarget, NodeColorAxis, SectionColorAxis};
use crate::application::console::completion::{
    kv_key_completions_with_hints, prefix_filter, Completion, CompletionContext, CompletionState,
};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::traits::{
    apply_kvs, ColorValue, HasBgColor, HasBorderColor, HasTextColor, Outcome,
};
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::{SectionSel, SelectionState};

pub const KEYS: &[&str] = &["bg", "text", "border", "section"];
pub const VALUE_PRESETS: &[&str] = &["accent", "edge", "fg", "reset"];

pub const COMMAND: Command = Command {
    name: "color",
    aliases: &[],
    summary: "Set bg/text/border color, or pick via the glyph wheel",
    usage: "color bg=<color> text=<color> border=<color>   |   color bg|text|border|pick",
    tags: &["color", "bg", "text", "border", "pick", "wheel"],
    applicable: always,
    complete: complete_color,
    execute: execute_color,
};

fn complete_color(state: &CompletionState, _ctx: &ConsoleContext) -> Vec<Completion> {
    match &state.context {
        CompletionContext::Token { index } => {
            let mut out = kv_key_completions_with_hints(KEYS, state.partial, kv_hint);
            // At token 0 the bare verbs — `pick` plus the axis
            // positionals `bg` / `text` / `border` — also hand off
            // to the glyph-wheel picker. Suggest them alongside the
            // kv-key forms.
            if *index == 0 {
                out.extend(prefix_filter(&["pick", "picker"], state.partial));
            }
            // `color picker` expects `on` / `off` as the next token.
            if *index == 1 && matches!(state.tokens.first().map(String::as_str), Some("picker")) {
                out.extend(prefix_filter(&["on", "off"], state.partial));
            }
            out
        }
        CompletionContext::KvValue { key } if KEYS.iter().any(|k| k == key) => {
            prefix_filter(VALUE_PRESETS, state.partial)
        }
        _ => Vec::new(),
    }
}

fn kv_hint(key: &str) -> Option<&'static str> {
    match key {
        "bg" => Some("fill / background color"),
        "text" => Some("text / label color"),
        "border" => Some("frame / line color"),
        "section" => Some("target section index inside a multi-section node"),
        _ => None,
    }
}

/// Outcome of resolving a positional `color` verb against the
/// current selection. Distinguishes "open the picker on this
/// target" from "the verb doesn't apply to this selection shape"
/// (descriptive message) from "no selection / unknown verb" (fall
/// through to the generic error). Lets `bg`/`border` on a section
/// surface a clearer reason than the generic fallback.
enum PickerTargetOutcome {
    Open(ColorTarget),
    NotApplicable(String),
    Unknown,
}

/// Map a bare positional verb (`pick`, `bg`, `text`, `border`) to a
/// concrete `ColorTarget` based on the current selection.
///
/// Node targets carry the axis directly. Edge / portal targets
/// collapse axis into their one color field: `bg`/`border` on an
/// edge both resolve to the edge's line color; `bg` on a portal
/// resolves to the portal's fill. Section targets honor the `text`
/// axis and report NotApplicable for `bg` / `border` (sections have
/// no chrome by spec — see `format/sections.md`).
fn picker_target_for(verb: &str, selection: &SelectionState) -> PickerTargetOutcome {
    let axis = match verb {
        "bg" => Some(NodeColorAxis::Bg),
        "text" => Some(NodeColorAxis::Text),
        "border" => Some(NodeColorAxis::Border),
        "pick" => None, // axis-agnostic legacy flow
        _ => return PickerTargetOutcome::Unknown,
    };
    match selection {
        SelectionState::Single(id) => match axis {
            Some(a) => PickerTargetOutcome::Open(ColorTarget::Node {
                id: id.clone(),
                axis: a,
            }),
            // `color pick` on a node defaults to bg.
            None => PickerTargetOutcome::Open(ColorTarget::Node {
                id: id.clone(),
                axis: NodeColorAxis::Bg,
            }),
        },
        // Section selection: route the picker to the targeted
        // section so commit lands on `set_section_text_color`,
        // leaving sibling sections untouched. `bg`/`border` have
        // no section-level fields (matches the kv-form
        // `apply_section_colors` arm below) — surface a clear
        // NotApplicable message rather than collapsing to the
        // owning node, which would silently broaden the user's
        // intent.
        SelectionState::Section(SectionSel { node_id, section_idx }) => match axis {
            Some(NodeColorAxis::Text) | None => PickerTargetOutcome::Open(ColorTarget::Section {
                node_id: node_id.clone(),
                section_idx: *section_idx,
                axis: SectionColorAxis::Text,
                range: None,
            }),
            Some(NodeColorAxis::Bg) => PickerTargetOutcome::NotApplicable(
                "color bg: not applicable to a section (section-level chrome doesn't exist)".to_string(),
            ),
            Some(NodeColorAxis::Border) => PickerTargetOutcome::NotApplicable(
                "color border: not applicable to a section (section-level chrome doesn't exist)".to_string(),
            ),
        },
        SelectionState::Multi(ids) => {
            // The picker is single-target; pick the first node in
            // the multi-selection. Fanout through the picker is
            // a future addition.
            match ids.first() {
                Some(id) => PickerTargetOutcome::Open(ColorTarget::Node {
                    id: id.clone(),
                    axis: axis.unwrap_or(NodeColorAxis::Bg),
                }),
                None => PickerTargetOutcome::Unknown,
            }
        }
        // Multi-section: same single-target picker shape as
        // `Multi(ids)` — opens on the first selected section's
        // text axis (the only section-level color axis;
        // `bg` / `border` are NotApplicable for sections).
        // Per-section fanout commit happens through the
        // selection_targets dispatch on close, not here.
        SelectionState::MultiSection(secs) => match secs.first() {
            Some(SectionSel { node_id, section_idx }) => match axis {
                Some(NodeColorAxis::Text) | None => PickerTargetOutcome::Open(ColorTarget::Section {
                    node_id: node_id.clone(),
                    section_idx: *section_idx,
                    axis: SectionColorAxis::Text,
                    range: None,
                }),
                Some(NodeColorAxis::Bg) => PickerTargetOutcome::NotApplicable(
                    "color bg: not applicable to a section (section-level chrome doesn't exist)".to_string(),
                ),
                Some(NodeColorAxis::Border) => PickerTargetOutcome::NotApplicable(
                    "color border: not applicable to a section (section-level chrome doesn't exist)"
                        .to_string(),
                ),
            },
            None => PickerTargetOutcome::Unknown,
        },
        // SectionRange: route the picker to the targeted section
        // AND plumb the sub-range so the commit fires through
        // `set_section_text_color_range`.
        SelectionState::SectionRange {
            sel: SectionSel { node_id, section_idx },
            range,
        } => match axis {
            Some(NodeColorAxis::Text) | None => PickerTargetOutcome::Open(ColorTarget::Section {
                node_id: node_id.clone(),
                section_idx: *section_idx,
                axis: SectionColorAxis::Text,
                range: Some(*range),
            }),
            Some(NodeColorAxis::Bg) => PickerTargetOutcome::NotApplicable(
                "color bg: not applicable to a section (section-level chrome doesn't exist)".to_string(),
            ),
            Some(NodeColorAxis::Border) => PickerTargetOutcome::NotApplicable(
                "color border: not applicable to a section (section-level chrome doesn't exist)".to_string(),
            ),
        },
        SelectionState::Edge(er) => {
            // Edges (line-mode or portal-mode) have one color
            // field. `border` maps to it, `text` also currently
            // maps to it (edge label + line share `MindEdge.color`),
            // and for portal-mode edges `bg` is accepted as an
            // alias because "fill" reads more natural there.
            PickerTargetOutcome::Open(ColorTarget::Edge(er.clone()))
        }
        SelectionState::PortalLabel(s) | SelectionState::PortalText(s) => {
            // Portal icon or portal text — both share the same
            // owning edge identity. The axis is irrelevant at the
            // picker level (one color field per endpoint channel);
            // the commit path reads the active selection variant
            // to decide whether to write `color` (icon) or
            // `text_color`. Returning the owning-edge target here
            // keeps the picker target resolution shape identical
            // to the `Edge` branch; per-variant routing lives in
            // the commit path.
            PickerTargetOutcome::Open(ColorTarget::Edge(s.edge_ref()))
        }
        SelectionState::EdgeLabel(s) => {
            // Line-mode label: same owning-edge shape as `Edge`;
            // the commit path discriminates between edge-body and
            // label color writes via the active selection variant.
            PickerTargetOutcome::Open(ColorTarget::Edge(s.edge_ref.clone()))
        }
        SelectionState::None => PickerTargetOutcome::Unknown,
    }
}

fn execute_color(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    // Positional handoffs to the glyph-wheel picker:
    //  - `color pick` — legacy edge/portal one-axis flow
    //  - `color bg | text | border` — pick a color for that axis on
    //    the current selection (node axis for nodes, single-color
    //    target for edges/portals)
    //  - `color picker on` — open the picker as a persistent
    //    standalone palette (no target; commit applies to selection)
    //  - `color picker off` — close any open picker
    if let Some(verb) = args.positional(0) {
        if verb == "picker" {
            match args.positional(1) {
                Some("on") => {
                    eff.side_effect = Some(super::super::ConsoleSideEffect::OpenColorPickerStandalone);
                    eff.close_console = true;
                    return ExecResult::ok_empty();
                }
                Some("off") => {
                    eff.side_effect = Some(super::super::ConsoleSideEffect::CloseColorPicker);
                    eff.close_console = true;
                    return ExecResult::ok_empty();
                }
                _ => return ExecResult::err("usage: color picker on | color picker off"),
            }
        }
        match picker_target_for(verb, &eff.document.selection) {
            PickerTargetOutcome::Open(target) => {
                eff.side_effect = Some(super::super::ConsoleSideEffect::OpenColorPicker(target));
                eff.close_console = true;
                return ExecResult::ok_empty();
            }
            PickerTargetOutcome::NotApplicable(msg) => {
                return ExecResult::err(msg);
            }
            PickerTargetOutcome::Unknown => {}
        }
        if matches!(verb, "pick" | "bg" | "text" | "border") {
            return ExecResult::err(format!("color {}: nothing to pick for this selection", verb));
        }
    }

    // Split out optional `section=N` and `range=A..B` from the
    // color kvs. When `section` is present, the verb routes
    // per-section through `set_section_text_color` rather than
    // the whole-node trait dispatcher — that's the only setter
    // today that accepts a section index. When `range` is
    // additionally present, it routes through the range-aware
    // sibling `set_section_text_color_range` introduced in N4-B.
    // `range` without `section` is a usage error: ranges target
    // grapheme indices inside one section's text, so the section
    // must be specified first.
    let mut section_target: Option<usize> = None;
    let mut range_target: Option<(usize, usize)> = None;
    let mut color_kvs: Vec<(String, String)> = Vec::new();
    for (k, v) in args.kvs() {
        if k == "section" {
            match super::range_kv::parse_section_kv("color", v) {
                Ok(idx) => section_target = Some(idx),
                Err(msg) => return ExecResult::err(msg),
            }
        } else if k == "range" {
            match super::range_kv::parse_range_kv(v) {
                Ok(pair) => range_target = Some(pair),
                Err(msg) => return ExecResult::err(format!("color: range='{}' — {}", v, msg)),
            }
        } else {
            color_kvs.push((k.to_string(), v.to_string()));
        }
    }
    if color_kvs.is_empty() && section_target.is_none() {
        return ExecResult::err("usage: color bg|text|border[=<color>]   |   color pick");
    }
    if color_kvs.is_empty() {
        return ExecResult::err("color: section=N requires at least one color axis (e.g. text=#ff0000)");
    }
    if range_target.is_some() && section_target.is_none() {
        return ExecResult::err(
            "color: range=A..B requires section=N — ranges target grapheme indices inside one section",
        );
    }

    if let Some(idx) = section_target {
        return apply_section_colors(eff.document, idx, range_target, &color_kvs);
    }

    let report = apply_kvs(eff.document, &color_kvs, stage_color_axis);

    finalize_report(report, "color")
}

/// The `key` → trait-method mapping every color write goes
/// through. Handed to [`apply_kvs`] by both the kv-form verb
/// (which stages several axes at once) and the single-axis
/// `Action` core, so the two cannot drift on which trait an axis
/// binds to or on how an unparseable color is reported.
///
/// `None` means "not a color key" — [`apply_kvs`] reports that
/// once for the pair rather than once per target.
fn stage_color_axis(
    view: &mut crate::application::console::traits::TargetView<'_>,
    key: &str,
    value: &str,
) -> Option<Outcome> {
    let color = match ColorValue::parse(value) {
        Ok(c) => c,
        Err(msg) => return Some(Outcome::Invalid(msg)),
    };
    match key {
        "bg" => Some(view.set_bg_color(color)),
        "text" => Some(view.set_text_color(color)),
        "border" => Some(view.set_border_color(color)),
        _ => None,
    }
}

/// Per-section color write. `text` routes through
/// [`super::super::super::document::MindMapDocument::set_section_text_color`];
/// `bg` / `border` aren't section-level fields and surface a
/// NotApplicable message rather than landing on the whole-node
/// chrome (that would surprise authors who deliberately scoped
/// to one section).
fn apply_section_colors(
    doc: &mut crate::application::document::MindMapDocument,
    section_idx: usize,
    range: Option<(usize, usize)>,
    kvs: &[(String, String)],
) -> ExecResult {
    let node_id = match doc.selection.primary_node_id() {
        Some(id) => id.to_string(),
        None => return ExecResult::err("color: section=N requires a node or section selection"),
    };
    // Surface a clear error when `range_start` is past the
    // section's grapheme count — without this pre-flight the
    // setter silently no-ops and the verb prints "color: no
    // change", indistinguishable from "you set red on already-
    // red text".
    if let Some((rs, _re)) = range {
        if let Some(node) = doc.mindmap.nodes.get(&node_id) {
            if let Some(section) = node.sections.get(section_idx) {
                let total = baumhard::util::grapheme_chad::count_grapheme_clusters(&section.text);
                if rs >= total {
                    return ExecResult::err(format!(
                        "color: range_start={} is past the section's grapheme count ({})",
                        rs, total
                    ));
                }
            }
        }
    }
    let mut messages = Vec::new();
    let mut any_applied = false;
    for (k, v) in kvs {
        match k.as_str() {
            "text" => {
                let color_value = match ColorValue::parse(v) {
                    Ok(c) => c,
                    Err(msg) => {
                        messages.push(format!("text: {}", msg));
                        continue;
                    }
                };
                let resolved = color_value
                    .as_model_string()
                    .unwrap_or_else(|| "#ffffff".to_string());
                let applied = match range {
                    Some((rs, re)) => {
                        let ok = doc.set_section_text_color_range(&node_id, section_idx, rs, re, resolved);
                        if !ok {
                            // Mirror the picker path's stale-range
                            // diagnostic: the pre-flight `rs >= total`
                            // check above already rejects ranges past
                            // the section's grapheme count, so a
                            // `false` here means either the node /
                            // section was deleted concurrently or
                            // `range_end` exceeds total. Surface so
                            // it doesn't silently land as
                            // "color: no change".
                            log::warn!(
                                "color verb on section {} of node {} \
                                 range {}..{} produced no change \
                                 (range may extend past the section's \
                                 grapheme count or section was deleted)",
                                section_idx,
                                node_id,
                                rs,
                                re
                            );
                        }
                        ok
                    }
                    None => doc.set_section_text_color(&node_id, section_idx, resolved),
                };
                if applied {
                    any_applied = true;
                }
            }
            "bg" | "border" => {
                messages.push(format!(
                    "{}: not applicable to a section (section-level chrome doesn't exist)",
                    k
                ));
            }
            other => messages.push(format!("unknown key '{}'", other)),
        }
    }
    if any_applied && messages.is_empty() {
        let scope = match range {
            Some((rs, re)) => format!("section {} range {}..{}", section_idx, rs, re),
            None => format!("section {}", section_idx),
        };
        return ExecResult::ok_msg(format!("color applied to {}", scope));
    }
    if any_applied {
        return ExecResult::lines(messages);
    }
    if messages.is_empty() {
        return ExecResult::ok_msg("color: no change");
    }
    ExecResult::err(messages.join("; "))
}

/// Mutation core: apply a single color axis (`bg|text|border`) to
/// the current selection. Both the kv-form `color` console verb
/// (which dispatches multiple kvs at once via `apply_kvs`) and the
/// parametric `Action::SetColor*` Action arms route through the
/// same trait dispatch — this helper is the single-kv wrapper.
///
/// Returns `true` when at least one target actually changed; `false`
/// otherwise (no selection, invalid color string, every target was
/// already at the requested color, or the axis isn't applicable to
/// the selection kind). The Action arm uses the bool to decide
/// whether to trigger a scene rebuild; the verb keeps its full
/// per-pair outcome reporting.
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(crate) fn apply_color_axis_to_selection(
    doc: &mut crate::application::document::MindMapDocument,
    axis: &str,
    value: &str,
) -> bool {
    let kvs = vec![(axis.to_string(), value.to_string())];
    let report = apply_kvs(doc, &kvs, stage_color_axis);
    log_not_applicable_if_silent(&report, "color", axis);
    report.any_applied
}

/// Surface a NotApplicable outcome on the parametric Action path
/// where the dispatcher's scrollback messages would otherwise
/// vanish. Action arms (keybind / palette / macro) have no
/// scrollback to pipe per-pair outcomes into; without this hook
/// a `SetColor { axis: Bg }` triggered against a `Section`
/// selection (where the `HasBgColor` arm returns NotApplicable
/// per the Tier-2A trait split) would silently no-op with no
/// signal in the log either. Verb path keeps full per-pair
/// reporting via `finalize_report` and ignores this hook.
fn log_not_applicable_if_silent(
    report: &crate::application::console::traits::DispatchReport,
    verb: &str,
    axis: &str,
) {
    if !report.any_applied && report.messages.iter().any(|m| m.contains("not applicable")) {
        log::info!(
            "{} {}: not applicable to current selection (Action path; no scrollback). \
             Dispatcher messages: {}",
            verb,
            axis,
            report.messages.join("; "),
        );
    }
}

/// Common report-to-ExecResult conversion used by every
/// trait-dispatched command.
pub(super) fn finalize_report(
    report: crate::application::console::traits::DispatchReport,
    verb: &str,
) -> ExecResult {
    if report.all_failed {
        return ExecResult::err(report.messages.join("; "));
    }
    if !report.messages.is_empty() {
        return ExecResult::lines(report.messages);
    }
    if report.any_applied {
        ExecResult::ok_msg(format!("{} applied", verb))
    } else {
        ExecResult::ok_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::parser::{parse, ParseResult};
    use crate::application::document::tests_common::{first_testament_node_id, load_test_doc};

    #[test]
    fn apply_color_axis_writes_bg_to_node() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.selection = SelectionState::Single(id.clone());
        let changed = apply_color_axis_to_selection(&mut doc, "bg", "#fafafa");
        assert!(changed);
        let style = &doc.mindmap.nodes.get(&id).unwrap().style;
        assert_eq!(style.background_color, "#fafafa");
    }

    #[test]
    fn apply_color_axis_returns_false_with_no_selection() {
        let mut doc = load_test_doc();
        // Default selection is None — nothing to target.
        assert!(!apply_color_axis_to_selection(&mut doc, "bg", "#fafafa"));
    }

    #[test]
    fn apply_color_axis_returns_false_for_invalid_color() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.selection = SelectionState::Single(id.clone());
        // ColorValue::parse rejects this; the trait dispatcher
        // reports `Invalid` per target. `any_applied` stays false.
        assert!(!apply_color_axis_to_selection(
            &mut doc,
            "bg",
            "definitely-not-a-color"
        ));
    }

    #[test]
    fn apply_color_axis_returns_false_for_unknown_axis() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.selection = SelectionState::Single(id.clone());
        // The closure returns None for unknown keys; the dispatcher
        // surfaces "unknown key" as a message and `any_applied`
        // stays false.
        assert!(!apply_color_axis_to_selection(&mut doc, "bogus_axis", "#fafafa"));
    }

    /// `color text=#... section=K` routes through
    /// `set_section_text_color` for the specified index — runs on
    /// the targeted section get the new color, runs on other
    /// sections stay untouched.
    #[test]
    fn color_text_section_kv_targets_specific_section() {
        use crate::application::console::tests::fixtures::{assert_exec_ok, run};
        let (mut doc, id) = doc_with_two_sections();
        doc.selection = SelectionState::Single(id.clone());
        assert_exec_ok(run("color text=#ff0000 section=1", &mut doc));
        let node = doc.mindmap.nodes.get(&id).unwrap();
        assert!(
            node.sections[0].text_runs.iter().all(|r| r.color == "#aaaaaa"),
            "section 0 must NOT receive the color change"
        );
        assert!(
            node.sections[1].text_runs.iter().all(|r| r.color == "#ff0000"),
            "section 1 must receive the new color"
        );
    }

    /// `color text=<theme-token> section=K` takes the explicit
    /// section-index branch. That branch must persist the same
    /// complete `var(--name)` model string as the trait-dispatch
    /// branch, not wrap it a second time.
    #[test]
    fn color_text_section_kv_preserves_named_vars() {
        use crate::application::console::tests::fixtures::{assert_exec_ok, run};

        for (token, expected) in [
            ("accent", "var(--accent)"),
            ("fg", "var(--fg)"),
            ("edge", "var(--edge)"),
        ] {
            let (mut doc, id) = doc_with_two_sections();
            doc.selection = SelectionState::Single(id.clone());
            assert_exec_ok(run(&format!("color text={token} section=1"), &mut doc));
            let node = doc.mindmap.nodes.get(&id).unwrap();
            assert!(
                node.sections[1].text_runs.iter().all(|r| r.color == expected),
                "{token} must persist as {expected}, got {:?}",
                node.sections[1].text_runs
            );
        }
    }

    /// Same regression through the range-aware explicit
    /// `section=K range=A..B` path: the carved middle run must carry
    /// the complete `var(--name)` reference.
    #[test]
    fn color_text_section_range_kv_preserves_named_vars() {
        use crate::application::console::tests::fixtures::{assert_exec_ok, run};

        for (token, expected) in [
            ("accent", "var(--accent)"),
            ("fg", "var(--fg)"),
            ("edge", "var(--edge)"),
        ] {
            let (mut doc, id) = doc_with_two_sections();
            {
                let section = &mut doc.mindmap.nodes.get_mut(&id).unwrap().sections[1];
                section.text = "abcdefghij".into();
                section.text_runs = vec![baumhard::mindmap::model::TextRun {
                    start: 0,
                    end: 10,
                    bold: false,
                    italic: false,
                    underline: false,
                    font: "LiberationSans".into(),
                    size_pt: 14,
                    color: "#aaaaaa".into(),
                    hyperlink: None,
                }];
            }
            doc.selection = SelectionState::Single(id.clone());
            assert_exec_ok(run(&format!("color text={token} section=1 range=3..7"), &mut doc));
            let runs = &doc.mindmap.nodes.get(&id).unwrap().sections[1].text_runs;
            assert_eq!(runs.len(), 3);
            assert_eq!(runs[0].color, "#aaaaaa");
            assert_eq!(runs[1].color, expected);
            assert_eq!((runs[1].start, runs[1].end), (3, 7));
            assert_eq!(runs[2].color, "#aaaaaa");
        }
    }

    /// Build a node with two sections, both pinned to the cascade
    /// default `#aaaaaa`, returning `(doc, node_id)`. Thin wrapper
    /// around the shared `make_two_section_node_with_pinned_runs`
    /// helper.
    fn doc_with_two_sections() -> (crate::application::document::MindMapDocument, String) {
        use crate::application::document::tests_common::make_two_section_node_with_pinned_runs;
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        make_two_section_node_with_pinned_runs(
            &mut doc,
            &id,
            "#aaaaaa",
            ["#aaaaaa", "#aaaaaa"],
            "LiberationSans",
            14,
        );
        (doc, id)
    }

    /// `color text=#…` with a `SelectionState::Section` (no explicit
    /// `section=K` kv) routes through the `HasTextColor` trait arm
    /// to `set_section_text_color` — only the targeted section's
    /// runs change, siblings stay untouched.
    #[test]
    fn color_text_section_collapse_writes_only_section() {
        use crate::application::console::tests::fixtures::{assert_exec_ok, run};
        use crate::application::document::SectionSel;
        let (mut doc, id) = doc_with_two_sections();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        assert_exec_ok(run("color text=#00ff00", &mut doc));
        let node = doc.mindmap.nodes.get(&id).unwrap();
        assert!(
            node.sections[0].text_runs.iter().all(|r| r.color == "#aaaaaa"),
            "section 0 (sibling) must NOT receive the color change"
        );
        assert!(
            node.sections[1].text_runs.iter().all(|r| r.color == "#00ff00"),
            "section 1 (selected) must receive the new color"
        );
    }

    /// `set_section_text_color` rewrite predicate matches the
    /// **cascade source** the picker reads (unanimous run color
    /// when present; node default otherwise). A section whose runs
    /// unanimously carry a non-default color is therefore
    /// rewritable from the picker / kv-form path. Pre-fix the
    /// write only matched runs equal to `node.style.text_color` and
    /// silently no-op'd when the section was uniformly customized,
    /// closing the read/write seam where the picker would seed to
    /// the displayed color and the user's pick would silently
    /// vanish on commit.
    #[test]
    fn color_text_section_rewrites_unanimous_non_default_runs() {
        use crate::application::console::tests::fixtures::{assert_exec_ok, run};
        use crate::application::document::tests_common::make_two_section_node_with_pinned_runs;
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        // node default is #aaaaaa but section 1's runs unanimously
        // carry #abcdef — a uniformly customized section. Pre-fix
        // this case silently no-op'd because the write predicate
        // looked for runs matching the node default and found none.
        make_two_section_node_with_pinned_runs(
            &mut doc,
            &id,
            "#aaaaaa",
            ["#aaaaaa", "#abcdef"],
            "LiberationSans",
            14,
        );
        doc.selection = SelectionState::Single(id.clone());
        assert_exec_ok(run("color text=#00ff00 section=1", &mut doc));
        let node = doc.mindmap.nodes.get(&id).unwrap();
        assert!(
            node.sections[0].text_runs.iter().all(|r| r.color == "#aaaaaa"),
            "section 0 (untouched) must keep the cascade default"
        );
        assert!(
            node.sections[1].text_runs.iter().all(|r| r.color == "#00ff00"),
            "section 1's unanimous-non-default runs must be rewritten by the picker / kv path"
        );
    }

    /// `apply_color_axis_to_selection` returning `false` because
    /// every target reported NotApplicable (e.g. `bg` axis against
    /// a `Section` selection, where the trait arm collapses to
    /// `Outcome::NotApplicable` per Item 2) emits a `log::info!`
    /// note with the dispatcher's per-target messages — the
    /// Action path has no scrollback so without this hook a
    /// keybind for `SetColor { axis: Bg }` against a section
    /// would silently no-op with zero feedback. Pins X2.
    #[test]
    fn apply_color_axis_logs_when_all_targets_not_applicable() {
        use crate::application::document::SectionSel;
        let (mut doc, id) = doc_with_two_sections();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        // The bool surface is `false` — no scene rebuild fires.
        // The log line is emitted via `log::info!`; we assert the
        // boolean and trust the dispatcher's message-aggregation
        // path (already covered in `traits/tests.rs`) to put the
        // right text in `report.messages`. A regression here is
        // visible at the call-site contract level: a non-false
        // return with a section + bg axis means a silent
        // collapse re-introduced itself.
        let changed = apply_color_axis_to_selection(&mut doc, "bg", "#123456");
        assert!(
            !changed,
            "bg axis against a Section must report no change (NotApplicable)"
        );
    }

    /// `color text=accent` (or any well-known theme-variable
    /// shorthand) with a `SelectionState::Section` writes the
    /// literal `var(--accent)` string into the section's runs —
    /// not a resolved hex. Pins the verb-side of the var-preserve
    /// symmetry the picker now honors (`commit_color_picker`'s
    /// seed-var-ref short-circuit). A regression that resolves the
    /// var early at the verb layer would silently strip the
    /// theme reference.
    #[test]
    fn color_text_section_preserves_var_ref_round_trip() {
        use crate::application::console::tests::fixtures::{assert_exec_ok, run};
        use crate::application::document::SectionSel;
        let (mut doc, id) = doc_with_two_sections();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        assert_exec_ok(run("color text=accent", &mut doc));
        let node = doc.mindmap.nodes.get(&id).unwrap();
        assert!(
            node.sections[1]
                .text_runs
                .iter()
                .all(|r| r.color == "var(--accent)"),
            "section 1's runs must carry the literal var ref, not a resolved hex"
        );
    }

    /// `color bg=#…` with a `SelectionState::Section` reports
    /// NotApplicable rather than collapsing to the owning node's
    /// `background_color`. Sections have no bg-fill chrome by spec
    /// (`format/sections.md`). Pins Item 2.
    #[test]
    fn color_bg_section_returns_not_applicable() {
        use crate::application::console::tests::fixtures::{join_lines, run};
        use crate::application::document::SectionSel;
        let (mut doc, id) = doc_with_two_sections();
        let original_bg = doc.mindmap.nodes.get(&id).unwrap().style.background_color.clone();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        match run("color bg=#123456", &mut doc) {
            ExecResult::Lines(msgs) => assert!(
                join_lines(&msgs).contains("not applicable"),
                "expected NotApplicable surface; got {:?}",
                msgs
            ),
            ExecResult::Err(s) => assert!(s.contains("not applicable"), "got Err({:?})", s),
            other => panic!("expected Lines / Err with 'not applicable', got {:?}", other),
        }
        assert_eq!(
            doc.mindmap.nodes.get(&id).unwrap().style.background_color,
            original_bg,
            "node bg must NOT change when bg= targets a section selection"
        );
    }

    /// Mirror of `color_bg_section_returns_not_applicable` for the
    /// `border` axis — sections have no frame chrome either. Pins
    /// Item 3.
    #[test]
    fn color_border_section_returns_not_applicable() {
        use crate::application::console::tests::fixtures::{join_lines, run};
        use crate::application::document::SectionSel;
        let (mut doc, id) = doc_with_two_sections();
        let original_frame = doc.mindmap.nodes.get(&id).unwrap().style.frame_color.clone();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        match run("color border=#abcdef", &mut doc) {
            ExecResult::Lines(msgs) => assert!(
                join_lines(&msgs).contains("not applicable"),
                "expected NotApplicable surface; got {:?}",
                msgs
            ),
            ExecResult::Err(s) => assert!(s.contains("not applicable"), "got Err({:?})", s),
            other => panic!("expected Lines / Err with 'not applicable', got {:?}", other),
        }
        assert_eq!(
            doc.mindmap.nodes.get(&id).unwrap().style.frame_color,
            original_frame,
            "node frame must NOT change when border= targets a section selection"
        );
    }

    /// `color text` (no value) on a `SelectionState::Section` opens
    /// the picker bound to a `ColorTarget::Section` — the picker's
    /// commit then writes through `set_section_text_color`. Pins
    /// Item 7 (text-axis branch).
    #[test]
    fn picker_target_for_section_text_emits_section_target() {
        use crate::application::color_picker::{ColorTarget, SectionColorAxis};
        use crate::application::console::tests::fixtures::assert_exec_ok;
        use crate::application::document::SectionSel;
        let (mut doc, id) = doc_with_two_sections();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id.clone(),
            section_idx: 1,
        });
        let (cmd, toks) = match parse("color text") {
            ParseResult::Ok { cmd, args } => (cmd, args),
            _ => panic!("parse failed"),
        };
        let mut eff = ConsoleEffects::new(&mut doc);
        // assert_exec_ok catches a regression where the picker
        // opens AND the command surfaces an error (mixed signal).
        assert_exec_ok((cmd.execute)(&Args::new(&toks), &mut eff));
        match eff.side_effect {
            Some(crate::application::console::ConsoleSideEffect::OpenColorPicker(ColorTarget::Section {
                node_id,
                section_idx,
                axis,
                range,
            })) => {
                assert_eq!(node_id, id);
                assert_eq!(section_idx, 1);
                assert_eq!(axis, SectionColorAxis::Text);
                assert!(range.is_none(), "Section selection has no sub-range");
            }
            other => panic!("expected ColorTarget::Section/Text, got {:?}", other),
        }
    }

    /// `color text` on a `SelectionState::SectionRange` opens
    /// the picker bound to a `ColorTarget::Section` carrying the
    /// sub-range. The commit path then routes through
    /// `set_section_text_color_range`. Pins the N4-C.b.1
    /// extension.
    #[test]
    fn picker_target_for_section_range_carries_range() {
        use crate::application::color_picker::{ColorTarget, SectionColorAxis};
        use crate::application::console::tests::fixtures::assert_exec_ok;
        use crate::application::document::SectionSel;
        let (mut doc, id) = doc_with_two_sections();
        doc.selection = SelectionState::SectionRange {
            sel: SectionSel {
                node_id: id.clone(),
                section_idx: 1,
            },
            range: (3, 7),
        };
        let (cmd, toks) = match parse("color text") {
            ParseResult::Ok { cmd, args } => (cmd, args),
            _ => panic!("parse failed"),
        };
        let mut eff = ConsoleEffects::new(&mut doc);
        assert_exec_ok((cmd.execute)(&Args::new(&toks), &mut eff));
        match eff.side_effect {
            Some(crate::application::console::ConsoleSideEffect::OpenColorPicker(ColorTarget::Section {
                node_id,
                section_idx,
                axis,
                range,
            })) => {
                assert_eq!(node_id, id);
                assert_eq!(section_idx, 1);
                assert_eq!(axis, SectionColorAxis::Text);
                assert_eq!(range, Some((3, 7)));
            }
            other => panic!("expected ColorTarget::Section/Text with range, got {:?}", other),
        }
    }

    /// `color bg` on a `SelectionState::Section` returns
    /// NotApplicable with a descriptive message (no picker opens,
    /// no silent collapse to the owning node's bg axis). Pins Item
    /// 7 (bg/border-axis branch).
    #[test]
    fn picker_target_for_section_bg_returns_not_applicable_message() {
        use crate::application::console::tests::fixtures::assert_exec_err_contains;
        use crate::application::document::SectionSel;
        let (mut doc, id) = doc_with_two_sections();
        doc.selection = SelectionState::Section(SectionSel {
            node_id: id,
            section_idx: 1,
        });
        let (cmd, toks) = match parse("color bg") {
            ParseResult::Ok { cmd, args } => (cmd, args),
            _ => panic!("parse failed"),
        };
        let mut eff = ConsoleEffects::new(&mut doc);
        let result = (cmd.execute)(&Args::new(&toks), &mut eff);
        assert!(
            !matches!(
                eff.side_effect,
                Some(crate::application::console::ConsoleSideEffect::OpenColorPicker(_))
            ),
            "no picker should open for bg axis on a section selection"
        );
        assert_exec_err_contains(result, "not applicable");
    }
}
