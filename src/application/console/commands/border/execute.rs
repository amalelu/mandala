// SPDX-License-Identifier: MPL-2.0

//! `border` execute path. Positional dispatch + atomic kv apply
//! into a single `BorderConfigEdits` bundle that the document
//! setter applies whole-or-nothing per node.
//!
//! Single-channel verb: every kv targets `GlyphBorderConfig`,
//! so the parse-then-dispatch shape (vs `apply_kvs`'s per-kv
//! trait dispatch) is the right one — see `font.rs` for the
//! sibling pattern.

use baumhard::mindmap::border::PaletteField;
use baumhard::mindmap::border_pattern::SidePattern;

use crate::application::console::parser::Args;
use crate::application::console::spec::descent::{descend, Stop};
use crate::application::console::spec::{kvs, usage, Descent};
use crate::application::console::traits::ColorValue;
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::document::{
    BorderConfigEdits, BorderEditOutcome, BorderSide, MindMapDocument, OptionEdit, SelectionState,
};

use super::finish::BorderEdit;
use super::grammar::BORDER;
use super::positional::{positional_subverb_to_edits, BorderSurface};
use super::show::execute_border_show;

pub fn execute_border(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let descent = descend(&BORDER, args.tokens());
    // `preview` is the one subverb with a level of its own; the
    // descent already stepped into it, so the staging surface owns
    // everything past this point.
    if descent.parent_name(0) == Some("preview") {
        return super::preview::execute_border_preview(&descent, args, eff);
    }
    match descent.stop {
        Stop::Matched(subverb) => match subverb.name {
            "on" => bare_subverb(&descent, args, eff, true, apply_set_visible),
            "off" => bare_subverb(&descent, args, eff, false, apply_set_visible),
            "toggle" => bare_subverb(&descent, args, eff, (), |eff, ()| apply_toggle_visible(eff)),
            "reset" => bare_subverb(&descent, args, eff, (), |eff, ()| apply_reset(eff)),
            "show" => execute_border_show(&descent, args, eff),
            _ => apply_positional(&descent, args, eff),
        },
        // A bare positional at a slot a kv already made kv-form
        // almost always means an unquoted multi-word value
        // (`border palette=My Palette` tokenizes as
        // `["palette=My", "Palette"]`). Hint at quoting rather than
        // at the subverb the second token coincidentally spells.
        Stop::KvForm => ExecResult::err(descent.quoting_hint(args)),
        Stop::Unknown => ExecResult::err(usage::unknown_subverb_message(
            descent.level,
            descent.typed.unwrap_or_default(),
        )),
        Stop::Bare => apply_composed(&descent, args, eff),
    }
}

/// The composed kv form: collect every key the level declares,
/// parse and validate before any mutation.
fn apply_composed(descent: &Descent, args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let pairs = match kvs::read_strict(descent, args) {
        Ok(pairs) => pairs,
        Err(msg) => return ExecResult::err(msg),
    };
    if pairs.is_empty() {
        return ExecResult::err(usage::no_arguments_message(&BORDER));
    }
    let mut edits = BorderConfigEdits::default();
    for pair in &pairs {
        if let Err(e) = stage_kv(&mut edits, pair.key.name, pair.value) {
            return ExecResult::err(e);
        }
    }
    apply_edits(eff, edits)
}

/// A subverb that takes no arguments: refuse anything on the line
/// past its own name, then run. The refusal is the engine's, so
/// `border on preset=heavy` names the key it will not read instead
/// of dropping it.
fn bare_subverb<T>(
    descent: &Descent,
    args: &Args,
    eff: &mut ConsoleEffects,
    payload: T,
    run: fn(&mut ConsoleEffects, T) -> ExecResult,
) -> ExecResult {
    match kvs::read_strict(descent, args) {
        Ok(_) => run(eff, payload),
        Err(msg) => ExecResult::err(msg),
    }
}

fn apply_set_visible(eff: &mut ConsoleEffects, on: bool) -> ExecResult {
    let ids = match nodes_in_selection(&eff.document().selection, "border") {
        Ok(ids) => ids,
        Err(e) => return e,
    };
    let mut changed = 0usize;
    for id in &ids {
        if eff.document_mut().set_node_border_visible(id, on) {
            changed += 1;
        }
    }
    if changed == 0 {
        return ExecResult::ok_msg(format!("border: already {}", if on { "on" } else { "off" }));
    }
    ExecResult::ok_msg(format!(
        "border {} on {} node(s)",
        if on { "on" } else { "off" },
        changed
    ))
}

fn apply_reset(eff: &mut ConsoleEffects) -> ExecResult {
    let edits = BorderConfigEdits {
        clear: true,
        ..BorderConfigEdits::default()
    };
    apply_edits(eff, edits)
}

/// Resolved border preset for one node: its own stored preset,
/// else the canvas default's, else `"light"` (the model floor).
/// The single cascade behind `border preset cycle`, the
/// `side|corner … reset` glyph source, the `preset custom` gate,
/// and `Action::CycleBorderPreset` — four places that each used to
/// re-derive it inline.
fn resolved_node_preset(doc: &MindMapDocument, node_id: Option<&str>) -> String {
    node_id
        .and_then(|id| doc.mindmap.nodes.get(id))
        .and_then(|n| n.style.border.as_ref())
        .map(|c| c.preset.clone())
        .or_else(|| {
            doc.mindmap
                .canvas
                .default_border
                .as_ref()
                .map(|c| c.preset.clone())
        })
        .unwrap_or_else(|| "light".to_string())
}

/// First selected node's resolved preset, or `"light"` when the
/// selection can't carry a border at all. Shared by the
/// `border preset cycle` resolver and the `border side|corner
/// reset` resolver through
/// [`super::positional::BorderSurface::Selection`].
pub(super) fn first_selection_preset(doc: &MindMapDocument) -> String {
    let ids = match nodes_in_selection(&doc.selection, "border") {
        Ok(ids) => ids,
        Err(_) => return "light".to_string(),
    };
    resolved_node_preset(doc, ids.first().map(String::as_str))
}

/// First selected node whose resolved preset isn't `custom`, or
/// `None` if every node is already on custom. Walks the whole
/// selection so a heterogeneous `Multi` trips the gate too.
pub(super) fn first_non_custom_preset(doc: &MindMapDocument) -> Option<String> {
    let ids = nodes_in_selection(&doc.selection, "border").ok()?;
    ids.iter()
        .map(|id| resolved_node_preset(doc, Some(id)))
        .find(|preset| !preset.eq_ignore_ascii_case("custom"))
}

/// Mutation core behind `Action::CycleBorderPreset`: advance the
/// selection's border preset one step through `BORDER_PRESETS`,
/// wrapping.
///
/// The *decision* comes from
/// [`BorderSurface::next_preset`] under the `Selection` surface —
/// the very call `border preset cycle` makes — so the keybind and
/// the verb cannot disagree about what "next" means or about which
/// preset the sample starts from. Only the delivery differs, by
/// design: the verb stages the answer into the atomic
/// `BorderConfigEdits` bundle so it composes with the rest of a
/// composed edit and with `apply_edits`' reporting, while this arm
/// has nothing to report and writes straight through.
///
/// Returns `true` when at least one node actually changed — the
/// Action arm uses the bool to gate the scene rebuild.
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(crate) fn cycle_border_preset_on_selection(doc: &mut MindMapDocument) -> bool {
    if nodes_in_selection(&doc.selection, "border").is_err() {
        log::warn!("cycle border preset: no border-applicable selection");
        return false;
    }
    let target = BorderSurface::Selection.next_preset(doc);
    apply_border_field_to_selection(doc, "preset", &target)
}

/// Per-node tally from [`toggle_border_visible_on_selection`].
/// The verb renders it as scrollback; the Action arm only reads
/// `toggled` to gate the rebuild.
pub(crate) struct BorderToggleReport {
    /// Nodes whose `show_frame` actually flipped.
    pub(crate) toggled: usize,
    /// Of those, how many ended up visible.
    pub(crate) now_on: usize,
    /// Of those, how many ended up hidden.
    pub(crate) now_off: usize,
}

/// Mutation core: flip `style.show_frame` on every node in the
/// selection, each independently. Shared by `border toggle` and
/// `Action::ToggleBorderVisible` so the read-flip-write loop
/// exists once.
pub(crate) fn toggle_border_visible_on_selection(
    doc: &mut MindMapDocument,
) -> Result<BorderToggleReport, ExecResult> {
    let ids = nodes_in_selection(&doc.selection, "border")?;
    let mut report = BorderToggleReport {
        toggled: 0,
        now_on: 0,
        now_off: 0,
    };
    for id in &ids {
        let cur = doc
            .mindmap
            .nodes
            .get(id)
            .map(|n| n.style.show_frame)
            .unwrap_or(true);
        if doc.set_node_border_visible(id, !cur) {
            report.toggled += 1;
            if cur {
                report.now_off += 1;
            } else {
                report.now_on += 1;
            }
        }
    }
    Ok(report)
}

fn apply_toggle_visible(eff: &mut ConsoleEffects) -> ExecResult {
    let BorderToggleReport {
        toggled,
        now_on,
        now_off,
    } = match toggle_border_visible_on_selection(eff.document_mut()) {
        Ok(report) => report,
        Err(e) => return e,
    };
    if toggled == 0 {
        return ExecResult::ok_msg("border: no change");
    }
    if toggled == 1 {
        ExecResult::ok_msg(format!(
            "border toggled \u{2192} {}",
            if now_on == 1 { "on" } else { "off" }
        ))
    } else {
        ExecResult::ok_msg(format!(
            "border toggled on {} node(s) \u{2192} {} on, {} off",
            toggled, now_on, now_off
        ))
    }
}

/// Positional-subverb entry point. The grammar
/// (`preset` / `color` / `padding` / `palette` / `font` /
/// `side` / `corner`) is parsed by the surface-agnostic
/// [`super::positional::positional_subverb_to_edits`], which the
/// `canvas …` verbs share; this wrapper only supplies the
/// per-node surface and renders the outcome.
fn apply_positional(descent: &Descent, args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let staged = match positional_subverb_to_edits(descent, args, BorderSurface::Selection, eff.document()) {
        Ok(Some(staged)) => staged,
        // Unreachable through `execute_border`'s dispatch (which
        // only routes matched subverbs here), but the shared parser
        // is honest about unknown verbs and so is this arm.
        Ok(None) => {
            return ExecResult::err(usage::unknown_subverb_message(
                descent.level,
                descent.typed.unwrap_or_default(),
            ))
        }
        Err(msg) => return ExecResult::err(msg),
    };
    let outcome = apply_edits(eff, staged.edits);
    match staged.cycled_to {
        // On `cycle`, prepend the resolved preset so heterogeneous
        // Multi selections see what they converged to.
        Some(target) => prepend_line(outcome, format!("border preset \u{2192} '{}' (cycle)", target)),
        None => outcome,
    }
}

/// Prepend a synthetic header line to an `ExecResult`. `Err`
/// passes through unchanged. `Ok(_)` lifts to `Lines` so the
/// header survives.
pub(crate) fn prepend_line(result: ExecResult, header: String) -> ExecResult {
    use crate::application::console::OutputLine;
    match result {
        ExecResult::Err(_) => result,
        ExecResult::Ok(msg) => ExecResult::lines(vec![header, msg]),
        ExecResult::Lines(rows) => {
            let mut out = vec![OutputLine::plain(header)];
            out.extend(rows);
            ExecResult::Lines(out)
        }
    }
}

fn apply_edits(eff: &mut ConsoleEffects, edits: BorderConfigEdits) -> ExecResult {
    let ids = match nodes_in_selection(&eff.document().selection, "border") {
        Ok(ids) => ids,
        Err(e) => return e,
    };
    // Detect a bare `preset=custom` (no other glyph fields). The
    // `custom` preset is the canvas the per-node `top=` / `bottom=`
    // / `left=` / `right=` / `tl=` / `tr=` / `bl=` / `br=` fields
    // paint on; without any of those, it falls back to the same
    // single-cluster glyphs the `rounded` preset uses, which makes
    // the choice look like a no-op. Surface that explicitly so the
    // user knows what `preset=custom` is asking for.
    let bare_custom = matches!(
        edits.preset,
        OptionEdit::Set(ref s) if s.eq_ignore_ascii_case("custom")
    ) && !edits_has_glyph_field(&edits);
    let mut changed = 0usize;
    let mut auto_promoted: Option<String> = None;
    let mut rejected: Vec<String> = Vec::new();
    for id in &ids {
        let outcome: BorderEditOutcome = eff.document_mut().set_node_border_config(id, edits.clone());
        if outcome.changed {
            changed += 1;
        }
        if outcome.preset_auto_promoted && auto_promoted.is_none() {
            auto_promoted = outcome.requested_preset.clone();
        }
        if rejected.is_empty() {
            rejected = outcome.rejected;
        }
    }
    BorderEdit {
        label: "border",
        scope: "node",
        headline: (changed > 0).then(|| format!("border applied to {} node(s)", changed)),
        rejected,
        auto_promoted,
        bare_custom,
    }
    .finish()
}

/// Mutation core: apply a single `field=value` edit to every node
/// in the current selection. Both the kv-form `border` console verb
/// (which stages multiple kvs at once) and the parametric
/// `Action::SetBorderField` (single kv per binding) route through
/// the underlying `set_node_border_config` setter — this helper is
/// the single-kv wrapper the Action arm calls.
///
/// Returns `true` when at least one node actually changed; `false`
/// when no node selection exists, the field/value pair fails to
/// stage, the setter refuses the edit, or every selected node was
/// already at the requested value. The Action arm uses the bool to
/// decide whether to trigger a scene rebuild.
///
/// **Every `false` that is not "already at that value" says why.**
/// There is no scrollback on this path — the caller is a keybind,
/// a palette press or a macro step — so a returned `false` is the
/// whole of what the user sees, and a binding that can never work
/// is indistinguishable from one that has nothing to do. The two
/// kinds of `false` take different levels, per CODE_CONVENTIONS §9:
///
/// - **`warn!` for the permanent ones.** A `field=value` the
///   staging parser rejects, and an edit the setter refuses, are
///   properties of the binding and the value, not of the moment.
///   The binding is dead until the config changes, and `warn!`
///   survives into release, where the user who has to change it is.
///   The parser's and the setter's own messages are carried through
///   — they are the diagnosis, and this function used to discard
///   them.
/// - **`debug!` for the transient one.** "Nothing selected" is
///   answered by selecting something. It is also the state a stray
///   keypress lands in, so a `warn!` would flood a release build's
///   stderr with a condition that is not a defect.
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(crate) fn apply_border_field_to_selection(
    doc: &mut crate::application::document::MindMapDocument,
    field: &str,
    value: &str,
) -> bool {
    let mut edits = BorderConfigEdits::default();
    if let Err(why) = stage_kv(&mut edits, field, value) {
        log::warn!(
            "border: binding sets {}={:?}, which is not a valid border edit: {} \
             (no scrollback on the keybind path; this binding cannot apply)",
            field,
            value,
            why
        );
        return false;
    }
    let ids = match nodes_in_selection(&doc.selection, "border") {
        Ok(ids) => ids,
        Err(_) => {
            log::debug!(
                "border: {}={:?} has no node in the current selection to apply to",
                field,
                value
            );
            return false;
        }
    };
    let mut changed = false;
    for id in &ids {
        let outcome = doc.set_node_border_config(id, edits.clone());
        if !outcome.rejected.is_empty() {
            log::warn!(
                "border: node {:?} refused {}={:?}: {}",
                id,
                field,
                value,
                outcome.rejected.join("; ")
            );
        }
        if outcome.changed {
            changed = true;
        }
    }
    changed
}

/// `true` iff the staged edits include any side-pattern or corner
/// override — the fields that make `preset=custom` actually
/// distinguishable from `rounded`. Shared with the
/// `section frame …` and `canvas …` verbs so the bare-custom hint
/// fires under the same conditions everywhere.
pub(crate) fn edits_has_glyph_field(edits: &BorderConfigEdits) -> bool {
    !matches!(edits.side_top, OptionEdit::Keep)
        || !matches!(edits.side_bottom, OptionEdit::Keep)
        || !matches!(edits.side_left, OptionEdit::Keep)
        || !matches!(edits.side_right, OptionEdit::Keep)
        || !matches!(edits.corner_top_left, OptionEdit::Keep)
        || !matches!(edits.corner_top_right, OptionEdit::Keep)
        || !matches!(edits.corner_bottom_left, OptionEdit::Keep)
        || !matches!(edits.corner_bottom_right, OptionEdit::Keep)
}

/// Multi-line orientation for users who set `preset=custom` without
/// any glyph fields. Lists the eight overrides the preset takes and
/// shows one example so a user can copy-paste a starting point.
/// `verb_label` is the verb prefix the example shows (`"border"`,
/// `"section frame"`, `"canvas border"`, etc.) so the hint is
/// always idiomatic for the verb the user just typed.
pub(crate) fn custom_preset_hint(verb_label: &str) -> String {
    format!(
        "hint: 'custom' is the preset that lets you author per-side / per-corner glyphs. \
         Combine it with any of: top=… bottom=… left=… right=… tl=… tr=… bl=… br=…  \
         e.g. `{} preset=custom top=\"###(*)###\" tl=\"+\" tr=\"+\" bl=\"+\" br=\"+\"`. \
         See `format/border-patterns.md` for the side-pattern grammar.",
        verb_label
    )
}

/// Resolve the selection into a list of node ids — borders
/// attach to the node, so section-shaped selections collapse to
/// their owning node. `verb_label` prefixes every not-applicable
/// error so the same helper serves `border` / `section frame` /
/// `canvas …` and reports which verb the user typed.
pub(crate) fn nodes_in_selection(sel: &SelectionState, verb_label: &str) -> Result<Vec<String>, ExecResult> {
    match sel {
        SelectionState::Single(id) => Ok(vec![id.clone()]),
        SelectionState::Multi(ids) => Ok(ids.clone()),
        // Borders attach to the node, not the section — a section
        // selection collapses to its owning node for border verbs.
        SelectionState::Section(s) => Ok(vec![s.node_id.clone()]),
        SelectionState::SectionRange { sel: s, .. } => Ok(vec![s.node_id.clone()]),
        // Multi-section: collapse to the deduplicated set of
        // owning nodes via the shared
        // `dedup_owning_node_ids` helper.
        SelectionState::MultiSection(_) => Ok(sel.dedup_owning_node_ids()),
        SelectionState::None => Err(ExecResult::err(format!(
            "{}: no selection (select a node first)",
            verb_label
        ))),
        SelectionState::Edge(_) => Err(ExecResult::err(format!(
            "{}: not applicable to edges",
            verb_label
        ))),
        SelectionState::EdgeLabel(_) => Err(ExecResult::err(format!(
            "{}: not applicable to edge labels",
            verb_label
        ))),
        SelectionState::PortalLabel(_) => Err(ExecResult::err(format!(
            "{}: not applicable to portal labels",
            verb_label
        ))),
        SelectionState::PortalText(_) => Err(ExecResult::err(format!(
            "{}: not applicable to portal text",
            verb_label
        ))),
    }
}

/// Parse one `key=value` pair into the appropriate slot on
/// `edits`. Returns the same error string the user sees in the
/// console — kept verbatim so `border top="a)"` reports the parser
/// output ("unmatched ')'…") with a `top: ` prefix.
pub(crate) fn stage_kv(edits: &mut BorderConfigEdits, key: &str, value: &str) -> Result<(), String> {
    match key {
        "preset" => stage_preset(edits, value),
        "font" => stage_font(edits, value),
        "size" => stage_size(edits, value),
        "color" => stage_color(edits, value),
        "padding" => stage_padding(edits, value),
        "palette" => stage_palette(edits, value),
        "field" => stage_field(edits, value),
        "top" => edits.with_side_pattern(BorderSide::Top, value),
        "bottom" => edits.with_side_pattern(BorderSide::Bottom, value),
        "left" => edits.with_side_pattern(BorderSide::Left, value),
        "right" => edits.with_side_pattern(BorderSide::Right, value),
        "tl" => stage_corner_or_err(&mut edits.corner_top_left, "tl", value),
        "tr" => stage_corner_or_err(&mut edits.corner_top_right, "tr", value),
        "bl" => stage_corner_or_err(&mut edits.corner_bottom_left, "bl", value),
        "br" => stage_corner_or_err(&mut edits.corner_bottom_right, "br", value),
        other => Err(format!(
            "unknown key '{}'; valid keys: {}",
            other,
            super::KEYS.join(" | ")
        )),
    }
}

fn stage_preset(edits: &mut BorderConfigEdits, value: &str) -> Result<(), String> {
    let v = value.to_ascii_lowercase();
    if !super::PRESETS.iter().any(|p| *p == v) {
        return Err(format!(
            "preset '{}' unknown; pick one of {}",
            value,
            super::PRESETS.join(" | ")
        ));
    }
    edits.preset = OptionEdit::Set(v);
    Ok(())
}

fn stage_font(edits: &mut BorderConfigEdits, value: &str) -> Result<(), String> {
    // `eq_ignore_ascii_case`, as `stage_palette` and `stage_field`
    // read their own `off` — three sibling sentinels on one verb had
    // no business obeying two rules, and the exact compare answered
    // `border font OFF` with "font 'OFF' is not a loaded font" while
    // `border palette OFF` cleared. A loaded family named `off` is
    // unreachable either way; the sentinel is documented in this
    // subverb's usage line and a family is not.
    if value.eq_ignore_ascii_case("off") || value.is_empty() {
        edits.font = OptionEdit::Clear;
        return Ok(());
    }
    if baumhard::font::fonts::app_font_by_family(value).is_none() {
        return Err(format!("font '{}' is not a loaded font; try `font list`", value));
    }
    edits.font = OptionEdit::Set(value.to_string());
    Ok(())
}

fn stage_size(edits: &mut BorderConfigEdits, value: &str) -> Result<(), String> {
    let pt = parse_pt("size", value)?;
    edits.font_size_pt = OptionEdit::Set(pt);
    Ok(())
}

fn stage_padding(edits: &mut BorderConfigEdits, value: &str) -> Result<(), String> {
    let pt = parse_pt("padding", value)?;
    edits.padding = OptionEdit::Set(pt);
    Ok(())
}

fn stage_color(edits: &mut BorderConfigEdits, value: &str) -> Result<(), String> {
    let cv = ColorValue::parse(value).map_err(|e| format!("color: {}", e))?;
    edits.color = match cv {
        ColorValue::Reset => OptionEdit::Clear,
        other => OptionEdit::Set(
            other
                .as_model_string()
                .ok_or_else(|| "color: unexpected reset variant".to_string())?,
        ),
    };
    Ok(())
}

fn stage_palette(edits: &mut BorderConfigEdits, value: &str) -> Result<(), String> {
    if value.eq_ignore_ascii_case("off") || value.is_empty() {
        edits.color_palette = OptionEdit::Clear;
        return Ok(());
    }
    edits.color_palette = OptionEdit::Set(value.to_string());
    Ok(())
}

fn stage_field(edits: &mut BorderConfigEdits, value: &str) -> Result<(), String> {
    if value.eq_ignore_ascii_case("off") || value.is_empty() {
        edits.color_palette_field = OptionEdit::Clear;
        return Ok(());
    }
    let lower = value.to_ascii_lowercase();
    let parsed = match lower.as_str() {
        "frame" => PaletteField::Frame,
        "background" => PaletteField::Background,
        "text" => PaletteField::Text,
        "title" => PaletteField::Title,
        _ => {
            // Echo `value`, not the lowercased copy: the user reads
            // this back looking for their own typo. Its sibling
            // `stage_preset` nine lines up already does.
            return Err(format!(
                "field '{}' unknown; pick one of {}",
                value,
                super::FIELDS.join(" | ")
            ));
        }
    };
    edits.color_palette_field = OptionEdit::Set(parsed);
    Ok(())
}

fn stage_corner_or_err(slot: &mut OptionEdit<String>, label: &str, value: &str) -> Result<(), String> {
    // Corners pass through the same escape rules as side patterns
    // (so `\(` inside a corner means a literal `(`); we re-use
    // [`SidePattern::parse`] for that and unpack it back into a
    // single concatenated string of clusters. Any parser error
    // surfaces with the corner label.
    let parsed = SidePattern::parse(value).map_err(|e| format!("{}: {}", label, e))?;
    let collapsed = match parsed {
        SidePattern::AtomicRepeat { cluster } => cluster.join(""),
        SidePattern::PrefixFillSuffix { .. } => {
            return Err(format!(
                "{}: corner doesn't take a fill region — use a static glyph",
                label
            ));
        }
        // `SidePattern` is `#[non_exhaustive]` so an unrecognized
        // future variant degrades to a clear error rather than a
        // panic — interactive paths must never panic per
        // `CODE_CONVENTIONS.md` §9.
        _ => {
            return Err(format!("{}: unsupported pattern shape for a corner", label));
        }
    };
    if collapsed.is_empty() {
        return Err(format!("{}: empty corner glyph", label));
    }
    *slot = OptionEdit::Set(collapsed);
    Ok(())
}

fn parse_pt(key: &str, value: &str) -> Result<f32, String> {
    crate::application::console::helpers::parse_finite_pt(key, value)
}
