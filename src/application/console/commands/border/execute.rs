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
use crate::application::console::traits::ColorValue;
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::document::{
    BorderConfigEdits, BorderEditOutcome, BorderSide, OptionEdit, SelectionState,
};

use super::show::execute_border_show;

pub fn execute_border(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    if let Some(verb) = args.positional(0) {
        // Discriminate "user typed positional subverb" from "user
        // typed kv form with an unquoted multi-word value". When
        // the first raw token is a kv (e.g. `palette=My`) and the
        // first positional comes later, the user clearly meant the
        // kv form — `args.positional(0)` happening to match a
        // subverb name (e.g. "Palette") is coincidental and should
        // route to the quoting-hint branch below, not the
        // positional dispatcher.
        let first_token_is_positional = args.tokens().first().map(|t| !t.contains('=')).unwrap_or(false);
        // C14: case-insensitive subverb match — same posture as
        // `border preview` already uses, and as `canvas …` and
        // top-level command lookup. Without normalising here,
        // `border Show` falls through to the unknown-subverb arm.
        match verb.to_ascii_lowercase().as_str() {
            // Bare-positional subverbs reject trailing arguments so
            // `border on preset=heavy` doesn't silently drop the kv.
            "on" => return reject_extras(args, "on", &[]).unwrap_or_else(|| apply_set_visible(eff, true)),
            "off" => return reject_extras(args, "off", &[]).unwrap_or_else(|| apply_set_visible(eff, false)),
            "toggle" => {
                return reject_extras(args, "toggle", &[]).unwrap_or_else(|| apply_toggle_visible(eff))
            }
            "show" => return execute_border_show(args, eff),
            "reset" => return reject_extras(args, "reset", &[]).unwrap_or_else(|| apply_reset(eff)),
            "preview" => return super::preview::execute_border_preview(args, eff),
            // Positional subverbs. Each pulls the next positional
            // as the value and routes through `apply_edits`. The
            // kv form (`border preset=heavy`) still works as the
            // keybind-friendly alias. Gated on
            // `first_token_is_positional` so an unquoted `palette=My
            // Palette` typo falls to the quoting-hint branch
            // rather than dispatching `apply_palette_positional`
            // with the wrong value.
            "preset" if first_token_is_positional => return apply_preset_positional(args, eff),
            "color" if first_token_is_positional => return apply_color_positional(args, eff),
            "padding" if first_token_is_positional => return apply_padding_positional(args, eff),
            "palette" if first_token_is_positional => return apply_palette_positional(args, eff),
            "font" if first_token_is_positional => return apply_font_positional(args, eff),
            "side" if first_token_is_positional => return apply_side_positional(args, eff),
            "corner" if first_token_is_positional => return apply_corner_positional(args, eff),
            other if !other.contains('=') => {
                // A bare positional alongside a recognised kv almost
                // always means the user typed an unquoted multi-word
                // value (`border palette=My Palette` → tokens are
                // `["palette=My", "Palette"]` because the tokenizer
                // splits on whitespace). Hint at quoting rather than
                // the generic "unknown subverb" message — the latter
                // is technically correct but unhelpful.
                if args.kvs().next().is_some() {
                    return ExecResult::err(format!(
                        "border: unexpected positional '{}' alongside a kv pair — \
                         did you mean to quote a multi-word value? \
                         e.g. `border palette=\"{}\"`",
                        verb, verb
                    ));
                }
                return ExecResult::err(unknown_subverb_message(verb));
            }
            _ => {}
        }
    }

    // kv form: collect every recognised key, parse + validate
    // before any mutation. An unknown key aborts with a
    // pointer-style error.
    let mut edits = BorderConfigEdits::default();
    let mut saw_any = false;
    for (k, v) in args.kvs() {
        saw_any = true;
        if let Err(e) = stage_kv(&mut edits, k, v) {
            return ExecResult::err(e);
        }
    }
    if !saw_any {
        return ExecResult::err("usage: border on|off|show|reset | border <key>=<value> …");
    }
    apply_edits(eff, edits)
}

fn apply_set_visible(eff: &mut ConsoleEffects, on: bool) -> ExecResult {
    let ids = match nodes_in_selection(&eff.document.selection, "border") {
        Ok(ids) => ids,
        Err(e) => return e,
    };
    let mut changed = 0usize;
    for id in &ids {
        if eff.document.set_node_border_visible(id, on) {
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

/// `border <typo>` — multi-line error grouping subverbs by
/// kind. API/UX I7: the prior single-line ~155-char enum
/// wrapped mid-quote-list on 80-col terminals. The grouped
/// shape is also load-bearing as a discoverability surface
/// when the user has clearly misspelled a verb.
fn unknown_subverb_message(verb: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!("border: unknown subverb '{}'\n", verb));
    out.push_str("  visibility: on | off | toggle | reset\n");
    out.push_str("  readout:    show [side=…] [verbose]\n");
    out.push_str("  per-field:  preset | color | padding | palette | font\n");
    out.push_str("  glyphs:     side <which> <pattern|reset> | corner <which> <glyph|reset>\n");
    out.push_str("  staged:     preview <kv>=… | preview commit | preview cancel\n");
    out.push_str("  composed:   <key>=<value> [<key>=<value> …]");
    out
}

/// `Some(err)` when the subverb received any unexpected kv or
/// extra positional, otherwise `None`. Lets bare-positional
/// subverbs (`on`/`off`/`toggle`/`reset`) reject typos that
/// would otherwise silently drop arguments.
fn reject_extras(args: &Args, subverb: &'static str, expected_kvs: &[&'static str]) -> Option<ExecResult> {
    let extra_kvs: Vec<&str> = args
        .kvs()
        .filter(|(k, _)| !expected_kvs.contains(k))
        .map(|(k, _)| k)
        .collect();
    let extra_positionals: Vec<&str> = args.positionals().skip(1).collect();
    if extra_kvs.is_empty() && extra_positionals.is_empty() {
        return None;
    }
    let mut bits = Vec::new();
    if !extra_kvs.is_empty() {
        bits.push(format!("kvs: {}", extra_kvs.join(", ")));
    }
    if !extra_positionals.is_empty() {
        bits.push(format!("extras: {}", extra_positionals.join(" ")));
    }
    Some(ExecResult::err(format!(
        "border {}: takes no arguments — got {}. \
         For composed edits use the kv form (`border preset=heavy padding=8`) \
         or stage with `border preview …`.",
        subverb,
        bits.join("; ")
    )))
}

fn apply_reset(eff: &mut ConsoleEffects) -> ExecResult {
    let edits = BorderConfigEdits {
        clear: true,
        ..BorderConfigEdits::default()
    };
    apply_edits(eff, edits)
}

/// First selected node's stored preset, falling back to the
/// canvas-default's preset, then `"light"`. Shared by the
/// `border preset cycle` resolver and the `border side|corner
/// reset` resolver.
fn first_selection_preset(eff: &ConsoleEffects) -> String {
    let ids = match nodes_in_selection(&eff.document.selection, "border") {
        Ok(ids) => ids,
        Err(_) => return "light".to_string(),
    };
    ids.first()
        .and_then(|id| eff.document.mindmap.nodes.get(id))
        .and_then(|n| n.style.border.as_ref())
        .map(|c| c.preset.clone())
        .or_else(|| {
            eff.document
                .mindmap
                .canvas
                .default_border
                .as_ref()
                .map(|c| c.preset.clone())
        })
        .unwrap_or_else(|| "light".to_string())
}

fn apply_toggle_visible(eff: &mut ConsoleEffects) -> ExecResult {
    let ids = match nodes_in_selection(&eff.document.selection, "border") {
        Ok(ids) => ids,
        Err(e) => return e,
    };
    let mut toggled = 0usize;
    let mut now_on = 0usize;
    let mut now_off = 0usize;
    for id in &ids {
        let cur = eff
            .document
            .mindmap
            .nodes
            .get(id)
            .map(|n| n.style.show_frame)
            .unwrap_or(true);
        if eff.document.set_node_border_visible(id, !cur) {
            toggled += 1;
            if !cur {
                now_on += 1;
            } else {
                now_off += 1;
            }
        }
    }
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

/// `border preset <name|cycle>`. `cycle` samples the first
/// selected node's preset (canvas-default-aware) and advances
/// one in `BORDER_PRESETS` — multi-node selections converge.
fn apply_preset_positional(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let value = match args.positional(1) {
        Some(v) => v,
        None => {
            return ExecResult::err("usage: border preset <light|heavy|double|rounded|custom|cycle>");
        }
    };
    if let Some(extra) = args.positionals().nth(2) {
        return ExecResult::err(format!(
            "border preset {}: unexpected extra positional '{}'. \
             Compose multiple edits via the kv form \
             (`border preset=heavy padding=8`) or stage with `border preview …`.",
            value, extra
        ));
    }
    let name_lc = value.to_ascii_lowercase();
    let was_cycle = name_lc == "cycle";
    let target = if was_cycle {
        let current = first_selection_preset(eff);
        baumhard::mindmap::border::next_border_preset(&current).to_string()
    } else {
        if !super::PRESETS.iter().any(|p| *p == name_lc) {
            return ExecResult::err(format!(
                "preset '{}' unknown; pick one of {} | cycle",
                value,
                super::PRESETS.join(" | ")
            ));
        }
        name_lc
    };
    let mut edits = BorderConfigEdits::default();
    edits.preset = OptionEdit::Set(target.clone());
    let outcome = apply_edits(eff, edits);
    // On `cycle`, prepend the resolved preset so heterogeneous
    // Multi selections see what they converged to.
    if was_cycle {
        prepend_line(outcome, format!("border preset → '{}' (cycle)", target))
    } else {
        outcome
    }
}

/// Prepend a synthetic header line to an `ExecResult`. `Err`
/// passes through unchanged. `Ok(_)` lifts to `Lines` so the
/// header survives.
fn prepend_line(result: ExecResult, header: String) -> ExecResult {
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

fn apply_color_positional(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let value = match args.positional(1) {
        Some(v) => v,
        None => {
            return ExecResult::err("usage: border color <#hex|var(--name)|preset|reset>");
        }
    };
    if let Some(extra) = args.positionals().nth(2) {
        return ExecResult::err(format!(
            "border color: unexpected extra positional '{}'. \
             Compose via the kv form (`border color=#fff padding=8`).",
            extra
        ));
    }
    let mut edits = BorderConfigEdits::default();
    if let Err(e) = stage_color(&mut edits, value) {
        return ExecResult::err(e);
    }
    apply_edits(eff, edits)
}

fn apply_padding_positional(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let value = match args.positional(1) {
        Some(v) => v,
        None => return ExecResult::err("usage: border padding <px>"),
    };
    if let Some(extra) = args.positionals().nth(2) {
        return ExecResult::err(format!(
            "border padding: unexpected extra positional '{}'. \
             Compose via the kv form (`border padding=8 color=#fff`).",
            extra
        ));
    }
    let mut edits = BorderConfigEdits::default();
    if let Err(e) = stage_padding(&mut edits, value) {
        return ExecResult::err(e);
    }
    apply_edits(eff, edits)
}

/// `border palette <name|off> [field=<frame|background|text|title>]`
/// — `name` writes the palette; `off` clears it. Optional
/// `field=` kv routes through the same `stage_field` parser
/// `palette field=` does on the kv form.
fn apply_palette_positional(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let value = match args.positional(1) {
        Some(v) => v,
        None => {
            return ExecResult::err("usage: border palette <name|off> [field=<frame|background|text|title>]");
        }
    };
    let mut edits = BorderConfigEdits::default();
    if let Err(e) = stage_palette(&mut edits, value) {
        return ExecResult::err(e);
    }
    if let Some((_, fv)) = args.kvs().find(|(k, _)| *k == "field") {
        if let Err(e) = stage_field(&mut edits, fv) {
            return ExecResult::err(e);
        }
    }
    apply_edits(eff, edits)
}

/// `border font <family|off> [size=<pt>]`. Optional `size=` kv
/// routes through `stage_size`.
fn apply_font_positional(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let value = match args.positional(1) {
        Some(v) => v,
        None => return ExecResult::err("usage: border font <family|off> [size=<pt>]"),
    };
    let mut edits = BorderConfigEdits::default();
    if let Err(e) = stage_font(&mut edits, value) {
        return ExecResult::err(e);
    }
    if let Some((_, sv)) = args.kvs().find(|(k, _)| *k == "size") {
        if let Err(e) = stage_size(&mut edits, sv) {
            return ExecResult::err(e);
        }
    }
    apply_edits(eff, edits)
}

/// `border side <which> <pattern|reset>`. `all` fans; `reset`
/// writes the current preset's default glyph for the named
/// side(s) (the schema's per-side fields are plain Strings, so
/// `OptionEdit::Clear` is a no-op — restoring the preset's own
/// default is the user-meaningful semantics). Errors when the
/// resolved preset isn't `custom` so the user picks `custom`
/// explicitly before glyph writes (no silent auto-promote).
fn apply_side_positional(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let which = match args.positional(1) {
        Some(v) => v,
        None => {
            return ExecResult::err("usage: border side <top|bottom|left|right|all> <pattern|reset>");
        }
    };
    let pattern = match args.positional(2) {
        Some(v) => v,
        None => {
            return ExecResult::err(format!(
                "border side {}: missing pattern (or 'reset' to clear)",
                which
            ));
        }
    };
    let sides = match parse_side_selector(which) {
        Some(s) => s,
        None => {
            return ExecResult::err(format!(
                "border side: '{}' unknown; pick top | bottom | left | right | all",
                which
            ));
        }
    };
    if !pattern.eq_ignore_ascii_case("reset") {
        if let Some(non_custom) = first_non_custom_preset(eff) {
            return ExecResult::err(format!(
                "border side {}: cannot set side glyph against preset '{}'. \
                 run `border preset custom` first, then set the side.",
                which, non_custom
            ));
        }
    }
    let mut edits = BorderConfigEdits::default();
    let reset = pattern.eq_ignore_ascii_case("reset");
    if reset {
        // CustomBorderGlyphs stores per-side glyphs as plain
        // Strings, so `OptionEdit::Clear` is a no-op. Restoring
        // means writing the preset's own default glyph back.
        let glyph_set = baumhard::mindmap::border::preset_glyph_set(&first_selection_preset(eff));
        for side in sides {
            let ch = match side {
                BorderSide::Top => glyph_set.top,
                BorderSide::Bottom => glyph_set.bottom,
                BorderSide::Left => glyph_set.left,
                BorderSide::Right => glyph_set.right,
            };
            if let Err(e) = edits.with_side_pattern(side, &ch.to_string()) {
                return ExecResult::err(e);
            }
        }
    } else {
        for side in sides {
            if let Err(e) = edits.with_side_pattern(side, pattern) {
                return ExecResult::err(e);
            }
        }
    }
    apply_edits(eff, edits)
}

/// First selected node whose resolved preset isn't `custom`,
/// or `None` if every node is already on custom. Walks the
/// whole selection so heterogeneous Multi trips the gate too.
fn first_non_custom_preset(eff: &ConsoleEffects) -> Option<String> {
    let ids = nodes_in_selection(&eff.document.selection, "border").ok()?;
    for id in &ids {
        let preset = eff
            .document
            .mindmap
            .nodes
            .get(id)
            .and_then(|n| n.style.border.as_ref())
            .map(|c| c.preset.as_str())
            .or_else(|| {
                eff.document
                    .mindmap
                    .canvas
                    .default_border
                    .as_ref()
                    .map(|c| c.preset.as_str())
            })
            .unwrap_or("light");
        if !preset.eq_ignore_ascii_case("custom") {
            return Some(preset.to_string());
        }
    }
    None
}

fn parse_side_selector(s: &str) -> Option<Vec<BorderSide>> {
    match s.to_ascii_lowercase().as_str() {
        "top" => Some(vec![BorderSide::Top]),
        "bottom" => Some(vec![BorderSide::Bottom]),
        "left" => Some(vec![BorderSide::Left]),
        "right" => Some(vec![BorderSide::Right]),
        "all" => Some(vec![
            BorderSide::Top,
            BorderSide::Bottom,
            BorderSide::Left,
            BorderSide::Right,
        ]),
        _ => None,
    }
}

/// `border corner <which> <glyph|reset>`. Same shape as
/// [`apply_side_positional`].
fn apply_corner_positional(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let which = match args.positional(1) {
        Some(v) => v,
        None => return ExecResult::err("usage: border corner <tl|tr|bl|br|all> <glyph|reset>"),
    };
    let glyph = match args.positional(2) {
        Some(v) => v,
        None => {
            return ExecResult::err(format!(
                "border corner {}: missing glyph (or 'reset' to clear)",
                which
            ));
        }
    };
    let corners = match parse_corner_selector(which) {
        Some(c) => c,
        None => {
            return ExecResult::err(format!(
                "border corner: '{}' unknown; pick tl | tr | bl | br | all",
                which
            ));
        }
    };
    if !glyph.eq_ignore_ascii_case("reset") {
        if let Some(non_custom) = first_non_custom_preset(eff) {
            return ExecResult::err(format!(
                "border corner {}: cannot set corner glyph against preset '{}'. \
                 run `border preset custom` first, then set the corner.",
                which, non_custom
            ));
        }
    }
    let mut edits = BorderConfigEdits::default();
    let reset = glyph.eq_ignore_ascii_case("reset");
    let glyph_set = reset.then(|| baumhard::mindmap::border::preset_glyph_set(&first_selection_preset(eff)));
    for corner in corners {
        // CODE_CONVENTIONS §9: interactive paths must not panic.
        // `parse_corner_selector` currently only emits the four
        // corners, but a future extension shouldn't crash an
        // interactive session.
        let slot = match corner {
            "tl" => &mut edits.corner_top_left,
            "tr" => &mut edits.corner_top_right,
            "bl" => &mut edits.corner_bottom_left,
            "br" => &mut edits.corner_bottom_right,
            _ => return ExecResult::err(format!("internal: unrecognised corner '{}'", corner)),
        };
        if let Some(ref gs) = glyph_set {
            let ch = match corner {
                "tl" => gs.top_left,
                "tr" => gs.top_right,
                "bl" => gs.bottom_left,
                "br" => gs.bottom_right,
                _ => return ExecResult::err(format!("internal: unrecognised corner '{}'", corner)),
            };
            if let Err(e) = stage_corner_or_err(slot, corner, &ch.to_string()) {
                return ExecResult::err(e);
            }
        } else if let Err(e) = stage_corner_or_err(slot, corner, glyph) {
            return ExecResult::err(e);
        }
    }
    apply_edits(eff, edits)
}

fn parse_corner_selector(s: &str) -> Option<Vec<&'static str>> {
    match s.to_ascii_lowercase().as_str() {
        "tl" => Some(vec!["tl"]),
        "tr" => Some(vec!["tr"]),
        "bl" => Some(vec!["bl"]),
        "br" => Some(vec!["br"]),
        "all" => Some(vec!["tl", "tr", "bl", "br"]),
        _ => None,
    }
}

fn apply_edits(eff: &mut ConsoleEffects, edits: BorderConfigEdits) -> ExecResult {
    let ids = match nodes_in_selection(&eff.document.selection, "border") {
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
    for id in &ids {
        let outcome: BorderEditOutcome = eff.document.set_node_border_config(id, edits.clone());
        if outcome.changed {
            changed += 1;
        }
        if outcome.preset_auto_promoted && auto_promoted.is_none() {
            auto_promoted = outcome.requested_preset.clone();
        }
    }
    let mut lines: Vec<String> = Vec::new();
    if changed == 0 {
        // A `preset=custom`-only edit on a node that already records
        // `preset: custom` is a no-op at the data-model level, but
        // the user still benefits from the same orientation message
        // as the changed-path branch. Emit it instead of the bare
        // "no change" line so the input doesn't feel ignored.
        if bare_custom {
            lines.push("border: preset=custom set; no glyph fields were given".into());
            lines.push(custom_preset_hint("border"));
            return ExecResult::lines(lines);
        }
        return ExecResult::ok_msg("border: no change");
    }
    // Surface auto-promotion exactly once per command invocation,
    // not once per affected node — the same edit applies to every
    // selected node so the message would be redundant. Only the
    // first promoted node's `requested_preset` is reported; every
    // other node received the same edit struct, so the value is
    // necessarily the same.
    lines.push(format!("border applied to {} node(s)", changed));
    if let Some(name) = auto_promoted {
        lines.push(format!(
            "note: preset='{}' auto-promoted to 'custom' \
             (a side or corner glyph was set; non-custom presets \
             ignore the per-node glyph override)",
            name
        ));
    }
    if bare_custom {
        lines.push(custom_preset_hint("border"));
    }
    if lines.len() == 1 {
        ExecResult::ok_msg(lines.into_iter().next().expect("len==1"))
    } else {
        ExecResult::lines(lines)
    }
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
/// stage, or every selected node was already at the requested
/// value. The Action arm uses the bool to decide whether to trigger
/// a scene rebuild.
#[must_use = "the bool gates the scene rebuild — drop it explicitly with `let _ = …` if you don't care"]
pub(crate) fn apply_border_field_to_selection(
    doc: &mut crate::application::document::MindMapDocument,
    field: &str,
    value: &str,
) -> bool {
    let mut edits = BorderConfigEdits::default();
    if stage_kv(&mut edits, field, value).is_err() {
        return false;
    }
    let ids = match nodes_in_selection(&doc.selection, "border") {
        Ok(ids) => ids,
        Err(_) => return false,
    };
    let mut changed = false;
    for id in &ids {
        let outcome = doc.set_node_border_config(id, edits.clone());
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

/// Per-key hint string for the shared `border` kv vocabulary.
/// `border …`, `section frame …`, and `canvas …` all surface the
/// same hints in completion popups; this is the single source of
/// truth.
pub(crate) fn kv_hint(key: &str) -> Option<&'static str> {
    match key {
        "preset" => Some("light | heavy | double | rounded | custom"),
        "font" => Some("font family for border glyphs (use `font list` for names)"),
        "size" => Some("border glyph size in points"),
        "color" => Some("#hex, var(--name), preset, or 'reset'"),
        "palette" => Some("palette name to cycle per-glyph colours, or 'off'"),
        "field" => Some("frame | background | text | title"),
        "padding" => Some("border-to-content padding in pixels"),
        "top" | "bottom" | "left" | "right" => Some("side pattern: `prefix(fill)suffix` or atomic"),
        "tl" | "tr" | "bl" | "br" => Some("single corner glyph (escapes apply)"),
        _ => None,
    }
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
    if value == "off" || value.is_empty() {
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
        other => {
            return Err(format!(
                "field '{}' unknown; pick one of {}",
                other,
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
        // `SidePattern` is `#[non_exhaustive]` so an unrecognised
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
