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

use super::range_kv::TARGET_KEYS;
use super::Command;
use crate::application::color_picker::{ColorTarget, NodeColorAxis, SectionColorAxis};
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::descent::{descend, Stop};
use crate::application::console::spec::{
    bare_words, free_words, kvs, usage, Bare, Descent, Form, Grammar, Key, Slot, Subverb, Vocabulary, Word,
};
use crate::application::console::traits::{
    apply_kvs, ColorValue, HasBgColor, HasBorderColor, HasTextColor, Outcome,
};
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::document::{SectionSel, SelectionState};

pub const VALUE_PRESETS: &[&str] = &["accent", "edge", "fg", "reset"];
const PRESET_WORDS: &[Word] = &bare_words::<4>(VALUE_PRESETS);

/// A color value: any `#hex` or `var(--name)`, plus the four preset
/// names the theme resolves.
const COLOR_VOCAB: Vocabulary = free_words("#hex|var(--name)", PRESET_WORDS);

const ON_OFF: &[Word] = &[
    Word::new("on", "open the picker as a persistent standalone palette"),
    Word::new("off", "close any open picker"),
];

const KEYS: &[Key] = &[
    Key::new("bg", "fill / background color", COLOR_VOCAB),
    Key::new("text", "text / label color", COLOR_VOCAB),
    Key::new("border", "frame / line color", COLOR_VOCAB),
];

/// The three axes are subverbs as well as keys: `color bg` opens the
/// glyph wheel on that axis, `color bg=#fff` writes it directly. The
/// two vocabularies are separate namespaces, so one word can be in
/// both — and the popup at the first slot offers each shape once.
const SUBVERBS: &[Subverb] = &[
    Subverb::bare("bg", "picker", "pick a fill color on the glyph wheel").gated(),
    Subverb::bare("text", "picker", "pick a text color on the glyph wheel").gated(),
    Subverb::bare("border", "picker", "pick a frame color on the glyph wheel").gated(),
    Subverb::bare("pick", "picker", "pick a color for the current selection").gated(),
    Subverb::bare("picker", "picker", "open or close the standalone palette")
        .taking(&[Form::slots(&[Slot::req(Vocabulary::Words(ON_OFF))])])
        .gated(),
];

pub static GRAMMAR: Grammar = Grammar {
    label: "color",
    subverb_sets: &[SUBVERBS],
    key_sets: &[KEYS, TARGET_KEYS],
    bare: Some(Bare::new(
        "composed",
        &[Form::opt(&["bg", "text", "border", "section", "range"])],
    )),
};

pub const COMMAND: Command = Command {
    name: "color",
    aliases: &[],
    summary: "Set bg/text/border color, or pick via the glyph wheel",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["wheel"],
    execute: execute_color,
};

/// The five picker subverbs. `picker on|off` opens or closes the
/// standalone palette; the other four resolve a target from the
/// selection and hand off to the glyph wheel.
fn execute_picker(
    descent: &Descent,
    args: &Args,
    eff: &mut ConsoleEffects,
    name: &'static str,
) -> ExecResult {
    if name == "picker" {
        return match descent
            .slot_value(args)
            .get(0)
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("on") => {
                eff.side_effect = Some(super::super::ConsoleSideEffect::OpenColorPickerStandalone);
                eff.close_console = true;
                ExecResult::ok_empty()
            }
            Some("off") => {
                eff.side_effect = Some(super::super::ConsoleSideEffect::CloseColorPicker);
                eff.close_console = true;
                ExecResult::ok_empty()
            }
            _ => ExecResult::err(usage::subverb_usage(
                descent.level,
                descent.subverb().unwrap_or(&SUBVERBS[4]),
            )),
        };
    }
    match picker_target_for(name, &eff.document().selection) {
        PickerTargetOutcome::Open(target) => {
            eff.side_effect = Some(super::super::ConsoleSideEffect::OpenColorPicker(target));
            eff.close_console = true;
            ExecResult::ok_empty()
        }
        PickerTargetOutcome::NotApplicable(msg) => ExecResult::err(msg),
        // The four axis subverbs are all `picker_target_for` knows;
        // an `Unknown` here means the selection carries no target.
        PickerTargetOutcome::Unknown => ExecResult::err(format!(
            "color {}: nothing to pick for this selection",
            descent.typed.unwrap_or(name)
        )),
    }
}

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
        // AND plumb the grapheme sub-range so the commit fires
        // through `set_section_text_color_range`.
        SelectionState::SectionRange {
            sel: SectionSel { node_id, section_idx },
            grapheme_range,
            ..
        } => match axis {
            Some(NodeColorAxis::Text) | None => PickerTargetOutcome::Open(ColorTarget::Section {
                node_id: node_id.clone(),
                section_idx: *section_idx,
                axis: SectionColorAxis::Text,
                range: Some(*grapheme_range),
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
    let descent = descend(&GRAMMAR, args.tokens());
    let pairs = match kvs::read_strict(&descent, args) {
        Ok(pairs) => pairs,
        Err(msg) => return ExecResult::err(msg),
    };
    match descent.stop {
        Stop::Matched(subverb) => return execute_picker(&descent, args, eff, subverb.name),
        Stop::KvForm => return ExecResult::err(descent.quoting_hint(args)),
        Stop::Unknown => {
            return ExecResult::err(usage::unknown_subverb_message(
                descent.level,
                descent.typed.unwrap_or_default(),
            ))
        }
        Stop::Bare => {}
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
    let target = match super::range_kv::extract_section_range_kvs(&pairs, "color") {
        Ok(t) => t,
        Err(msg) => return ExecResult::err(msg),
    };
    let color_kvs: Vec<(String, String)> = pairs
        .iter()
        .filter(|p| matches!(p.key.name, "bg" | "text" | "border"))
        .map(|p| (p.key.name.to_string(), p.value.to_string()))
        .collect();
    if color_kvs.is_empty() && target.section.is_none() {
        return ExecResult::err(usage::no_arguments_message(&GRAMMAR));
    }
    if color_kvs.is_empty() {
        return ExecResult::err("color: section=N requires at least one color axis (e.g. text=#ff0000)");
    }

    if let Some(idx) = target.section {
        return apply_section_colors(eff.document_mut(), idx, target.range, &color_kvs);
    }

    let report = apply_kvs(eff.document_mut(), &color_kvs, stage_color_axis);

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
    range: Option<crate::application::document::GraphemeRange>,
    kvs: &[(String, String)],
) -> ExecResult {
    let node_id = match doc.selection.primary_node_id() {
        Some(id) => id.to_string(),
        None => return ExecResult::err("color: section=N requires a node or section selection"),
    };
    // Shared with `font`: without the pre-flight the setter
    // silently no-ops and the verb prints "no change",
    // indistinguishable from "you set red on already-red text".
    if let Err(msg) = super::range_kv::preflight_range(doc, &node_id, section_idx, range, "color") {
        return ExecResult::err(msg);
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
                // Routed through the very `TargetView::Section`
                // arm the collapsed `SelectionState::Section`
                // path uses, rather than re-deriving the same
                // call. The copy that used to live here had
                // drifted already: it resolved `reset` to a
                // literal `#ffffff` and baked it onto the runs,
                // where the trait arm hands the run tier its own
                // "no color of my own" — the empty string.
                let mut view = crate::application::console::traits::TargetView::Section {
                    doc: &mut *doc,
                    id: node_id.clone(),
                    section_idx,
                    range,
                };
                let applied = view.set_text_color(color_value) == Outcome::Applied;
                if !applied {
                    if let Some(r) = range {
                        // Mirror the picker path's stale-range
                        // diagnostic: the pre-flight `rs >= total`
                        // check above already rejects ranges past
                        // the section's grapheme count, so no
                        // change here means either the node /
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
                            r.start(),
                            r.end()
                        );
                    }
                }
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
            // Unreachable: the engine refused every key this form
            // does not read before the loop ran (§9 degrade).
            other => log::error!("color: engine admitted unread key '{other}'"),
        }
    }
    if any_applied && messages.is_empty() {
        let scope = match range {
            Some(r) => format!("section {} range {}..{}", section_idx, r.start(), r.end()),
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

/// Surface a silent no-op on the parametric Action path where the
/// dispatcher's scrollback messages would otherwise vanish. Action
/// arms (keybind / palette / macro) have no scrollback to pipe
/// per-pair outcomes into; without this hook a `SetColor { axis:
/// Bg }` triggered against a `Section` selection (where the
/// `HasBgColor` arm returns NotApplicable per the Tier-2A trait
/// split) would silently no-op with no signal in the log either.
/// Verb path keeps full per-pair reporting via `finalize_report`
/// and ignores this hook.
///
/// Two outcomes reach here and they are not the same condition, so
/// they do not take the same level (CODE_CONVENTIONS §9):
///
/// - **NotApplicable** is transient — the axis is fine, the
///   selection is the wrong kind, and the same binding works on the
///   next selection. `info!`, which is compiled out of release,
///   because a stray keypress against the wrong selection is not a
///   defect and must not accumulate in a user's stderr.
/// - **Invalid** is permanent: the *value* the binding carries is
///   one the parser rejects, so the binding cannot ever apply, to
///   any selection. That is a defect in a config file the user
///   owns and cannot see from inside the app — `warn!`, which
///   survives into release where they are.
///
/// Before this split, an `Invalid` outcome matched neither the
/// "not applicable" substring nor anything else, so a keybinding
/// carrying an unparseable color was the quietest failure on the
/// path: no scrollback, no log, no change.
fn log_not_applicable_if_silent(
    report: &crate::application::console::traits::DispatchReport,
    verb: &str,
    axis: &str,
) {
    if report.any_applied {
        return;
    }
    if !report.invalid.is_empty() {
        log::warn!(
            "{} {}: value rejected, so this binding cannot apply to any selection \
             (Action path; no scrollback): {}",
            verb,
            axis,
            report.invalid.join("; "),
        );
    } else if report.messages.iter().any(|m| m.contains("not applicable")) {
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

    /// The popup rows for `line` with the cursor at its end.
    fn popup(line: &str, doc: &crate::application::document::MindMapDocument) -> Vec<String> {
        let ctx = crate::application::console::ConsoleContext::from_document(doc);
        crate::application::console::completion::complete(line, line.len(), &ctx)
            .into_iter()
            .map(|c| c.text)
            .collect()
    }

    /// `color picker` accepts `on` and `off` and rejects every kv
    /// pair. The popup used to offer the kv keys and neither of the
    /// two words, because its guard asked `tokens.first()` — the
    /// verb name — whether it was `picker`.
    #[test]
    fn test_color_picker_completion_offers_on_and_off_only() {
        let doc = load_test_doc();
        assert_eq!(popup("color picker ", &doc), vec!["on", "off"]);
        assert_eq!(popup("color picker o", &doc), vec!["on", "off"]);
    }

    /// `section=` takes an index, not a color name.
    #[test]
    fn test_color_section_value_completion_offers_section_indices() {
        let (mut doc, id) = crate::application::document::tests_common::pinned_two_section_node();
        doc.selection = SelectionState::Single(id);
        assert_eq!(popup("color section=", &doc), vec!["0", "1"]);
        assert_eq!(popup("color bg=", &doc), VALUE_PRESETS.to_vec());
    }

    /// `color range=A..B` reached `execute_color` and the verb's own
    /// error text without ever being offered by a completer, exactly
    /// as it had on `font` — accepted, named in the rejection the
    /// user gets for omitting `section=`, and discoverable nowhere.
    /// `section=` was the mirror gap: parsed, and absent from the
    /// usage line.
    ///
    /// Both are structural now. The key is one row of the level's
    /// declaration, and `spec::tests` holds every level's keys
    /// against the forms that print them in both directions, over
    /// the whole registry — so what used to be this verb's private
    /// assertion is an invariant. What stays here is the popup at
    /// the two slots, which the declaration does not by itself
    /// prove reaches the user.
    #[test]
    fn test_color_completion_offers_the_range_key() {
        let doc = load_test_doc();
        assert!(popup("color ", &doc).iter().any(|t| t == "range="));
        assert_eq!(popup("color ra", &doc), vec!["range="]);
        // …and `help color` names it, from the same declaration.
        assert!(
            COMMAND.key_lines().iter().any(|l| l.starts_with("range=")),
            "`help color` must publish the key: {:?}",
            COMMAND.key_lines()
        );
    }

    /// `color range=A..B` without `section=` is a usage error, and
    /// the message names the key the popup now offers — the two
    /// halves the drift had apart.
    #[test]
    fn test_color_range_without_section_is_rejected_by_name() {
        let (mut doc, id) = crate::application::document::tests_common::pinned_two_section_node();
        doc.selection = SelectionState::Single(id);
        use crate::application::console::tests::fixtures::run;
        match run("color range=0..2 text=accent", &mut doc) {
            ExecResult::Err(m) => assert!(m.contains("range=A..B requires section=N"), "{}", m),
            other => panic!("expected Err, got {:?}", other),
        }
        match run("color section=0 range=0..2 text=accent", &mut doc) {
            ExecResult::Ok(m) => assert!(m.contains("range 0..2"), "{}", m),
            other => panic!("expected Ok, got {:?}", other),
        }
    }

    /// Read back through the palette cascade rather than off
    /// `style`: every testament node is themed, so its fill comes
    /// from its palette group and a write that landed in `style`
    /// would report success while the node kept painting the
    /// palette color (`format/palettes.md`).
    #[test]
    fn apply_color_axis_writes_bg_to_node() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.selection = SelectionState::Single(id.clone());
        let changed = apply_color_axis_to_selection(&mut doc, "bg", "#fafafa");
        assert!(changed);
        let node = doc.mindmap.nodes.get(&id).unwrap();
        assert_eq!(doc.mindmap.node_background_color(node), "#fafafa");
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
                    size_pt: 14.0,
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
            14.0,
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
            14.0,
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
        use crate::application::document::{GraphemeRange, SectionSel, SectionSpan};
        let (mut doc, id) = doc_with_two_sections();
        doc.selection = SelectionState::SectionRange {
            sel: SectionSel {
                node_id: id.clone(),
                section_idx: 1,
            },
            section_span: SectionSpan::single(1),
            grapheme_range: GraphemeRange::new(3, 7),
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
                assert_eq!(range, Some(GraphemeRange::new(3, 7)));
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

    /// `color text=reset section=K` unpins the section's runs
    /// instead of baking a literal white onto them.
    ///
    /// A run's color is the *most* specific tier in the cascade, so
    /// a literal written here survives every retheme — which is the
    /// opposite of what "reset" promises. The run tier's own
    /// spelling for "no color of my own" is the empty string, and
    /// that is what a reset has to produce.
    #[test]
    fn color_text_section_reset_unpins_the_runs() {
        use crate::application::console::tests::fixtures::{assert_exec_ok, run};

        let (mut doc, id) = doc_with_two_sections();
        doc.selection = SelectionState::Single(id.clone());
        assert_exec_ok(run("color text=reset section=1", &mut doc));
        let node = doc.mindmap.nodes.get(&id).unwrap();
        assert!(
            node.sections[1].text_runs.iter().all(|r| r.color.is_empty()),
            "reset must leave the runs deferring, got {:?}",
            node.sections[1].text_runs
        );
        assert!(
            node.sections[0].text_runs.iter().all(|r| r.color == "#aaaaaa"),
            "the sibling section is not part of the gesture"
        );
    }

    /// `color bg=reset` / `border=reset` / `text=reset` on a themed
    /// node, end to end through the verb.
    ///
    /// The node has to come back onto its palette. Before the
    /// override tier existed the reset literals landed in `style`,
    /// which the palette shadows, so the gesture was a no-op nobody
    /// noticed; landing them in the override tier instead would
    /// have made it a permanent exception with no verb to lift it.
    #[test]
    fn color_reset_on_a_themed_node_hands_it_back_to_the_palette() {
        use crate::application::console::tests::fixtures::{assert_exec_ok, run};
        use crate::application::document::tests_common::theme_node_with_probe_palette;
        use baumhard::mindmap::model::ColorGroup;

        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        let group = theme_node_with_probe_palette(
            &mut doc,
            &id,
            "verb-reset-probe",
            ColorGroup {
                background: "#a9decb".into(),
                frame: "#30b082".into(),
                text: "#0f0f0f".into(),
                title: String::new(),
            },
        );
        doc.selection = SelectionState::Single(id.clone());
        assert_exec_ok(run("color bg=#111111 border=#222222 text=#333333", &mut doc));
        assert_exec_ok(run("color bg=reset border=reset text=reset", &mut doc));

        let node = doc.mindmap.nodes.get(&id).unwrap();
        assert!(
            node.color_schema
                .as_ref()
                .expect("still themed")
                .overrides
                .is_empty(),
            "every channel must be back to having no opinion"
        );
        assert_eq!(doc.mindmap.node_background_color(node), group.background);
        assert_eq!(doc.mindmap.node_frame_color(node), group.frame);
        assert_eq!(doc.mindmap.node_text_color(node), group.text);
    }

    /// The parametric path reports an unusable *value* at `warn!`
    /// and a wrong *selection* at `info!`, which the release cap
    /// compiles out.
    ///
    /// The distinction is the point: a keybinding carrying
    /// `bg=zzz-not-a-color` can never apply, to any selection, and
    /// the user cannot see why from inside the app; a `bg` press
    /// against a section selection works again on the next node.
    /// Before the `invalid` field on `DispatchReport` existed, the
    /// first case matched no substring the hook looked for and was
    /// the quietest failure on the path — no scrollback, no log, no
    /// change.
    ///
    /// Fails when: the `warn!` arm goes or is downgraded (the
    /// recorder keeps `warn!` and above, so a lower level
    /// disappears), or when the not-applicable case is raised to
    /// `warn!` (the control block finds a line, and every stray
    /// keypress against the wrong selection would reach a release
    /// build's stderr).
    #[test]
    fn test_parametric_color_warns_only_for_a_value_that_can_never_apply() {
        baumhard::util::test_logger::install();

        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.selection = SelectionState::Single(id.clone());
        assert!(!super::apply_color_axis_to_selection(
            &mut doc,
            "bg",
            "zzz-not-a-color-9f31"
        ));
        let logged = baumhard::util::test_logger::lines_containing("zzz-not-a-color-9f31");
        assert!(
            logged.iter().any(|l| l.starts_with("WARN ")),
            "an unparseable value must be reported at warn level, logged: {logged:?}"
        );

        // Control: a perfectly good color against a selection the
        // axis does not reach. `bg` on a section is the
        // NotApplicable case the hook was written for.
        let mut doc = load_test_doc();
        doc.selection = SelectionState::Section(crate::application::document::SectionSel::new(&id, 0));
        assert!(!super::apply_color_axis_to_selection(&mut doc, "bg", "#b2c3d4"));
        let logged = baumhard::util::test_logger::lines_containing("#b2c3d4");
        assert!(
            logged.is_empty(),
            "a wrong-selection press is transient and must not warn: {logged:?}"
        );
    }
}
