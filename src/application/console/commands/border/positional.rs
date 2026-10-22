// SPDX-License-Identifier: MPL-2.0

//! Positional border subverbs — `preset` / `color` / `padding` /
//! `palette` / `font` / `side` / `corner` — parsed once for every
//! surface that speaks the border vocabulary.
//!
//! `border …` writes the per-node config, `canvas border …` and
//! `canvas section-frame [focused] …` write the map-wide canvas
//! slots. The grammar is identical; only two inputs differ, and
//! both are supplied by [`BorderSurface`]: the preset a `cycle` /
//! `reset` resolves against, and the non-custom-preset gate that
//! guards per-side / per-corner glyph writes.
//!
//! `canvas.rs` used to carry its own copy of this dispatcher. The
//! copies drifted — the canvas `reset` arm hardcoded `"light"`
//! glyphs while the per-node arm resolved the live preset, and the
//! canvas glyph writes silently auto-promoted the slot's preset
//! where the per-node path errored. Both gaps were closed by adding
//! more parallel code; this module removes the parallelism instead
//! (`CODE_CONVENTIONS.md` §5).

use baumhard::mindmap::border::{next_border_preset, preset_glyph_set};

use crate::application::console::parser::Args;
use crate::application::console::spec::{kvs, usage, Descent};
use crate::application::document::{BorderConfigEdits, BorderSide, MindMapDocument, OptionEdit};

use super::execute::{first_non_custom_preset, first_selection_preset, stage_kv};

/// Which border-bearing surface a positional subverb writes to.
///
/// Supplies the two document-dependent inputs the shared parser
/// needs. The canvas variants mirror the three slots on
/// [`baumhard::mindmap::model::Canvas`] (and the corresponding
/// [`crate::application::document::BorderPreviewTarget`] arms).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum BorderSurface {
    /// Per-node `border …` — every node in the current selection.
    Selection,
    /// `canvas border …` → `Canvas.default_border`.
    CanvasDefault,
    /// `canvas section-frame …` → `Canvas.default_section_frame_border`.
    CanvasSectionFrame,
    /// `canvas section-frame focused …` →
    /// `Canvas.default_focused_section_frame_border`.
    CanvasSectionFrameFocused,
}

impl BorderSurface {
    /// The verb prefix the user typed, used in every error message
    /// and in the `preset custom` hint so the suggestion is always
    /// copy-pasteable for the surface being edited.
    pub(crate) fn label(self) -> &'static str {
        match self {
            BorderSurface::Selection => "border",
            BorderSurface::CanvasDefault => "canvas border",
            BorderSurface::CanvasSectionFrame => "canvas section-frame",
            BorderSurface::CanvasSectionFrameFocused => "canvas section-frame focused",
        }
    }

    /// The preset a `preset cycle` advances from and a
    /// `side|corner … reset` restores the default glyph of.
    /// Falls back to `"light"` (the model default) when the
    /// surface has no stored config yet.
    fn current_preset(self, doc: &MindMapDocument) -> String {
        match self {
            BorderSurface::Selection => first_selection_preset(doc),
            _ => self
                .canvas_slot(doc)
                .map(|c| c.preset.clone())
                .unwrap_or_else(|| "light".to_string()),
        }
    }

    /// The preset a `cycle` advances to on this surface: one step
    /// past [`Self::current_preset`], wrapping at the end of
    /// `BORDER_PRESETS`.
    ///
    /// This is the *decision*, and it is made exactly once for the
    /// whole codebase. The positional `… preset cycle` subverb
    /// stages the answer into its atomic edit bundle;
    /// [`super::execute::cycle_border_preset_on_selection`] —
    /// which `Action::CycleBorderPreset` calls — writes it straight
    /// through with `BorderSurface::Selection`. Two delivery
    /// mechanisms, one answer, so the keybind and the verb cannot
    /// disagree about what "next" means.
    pub(super) fn next_preset(self, doc: &MindMapDocument) -> String {
        next_border_preset(&self.current_preset(doc)).to_string()
    }

    /// The first resolved preset on this surface that isn't
    /// `custom`, or `None` when every target is already on custom.
    /// `Some(_)` blocks a per-side / per-corner glyph write: the
    /// user picks `custom` explicitly rather than being
    /// auto-promoted behind their back.
    fn first_non_custom_preset(self, doc: &MindMapDocument) -> Option<String> {
        match self {
            // Walks the whole selection so a heterogeneous `Multi`
            // trips the gate on its first non-custom node.
            BorderSurface::Selection => first_non_custom_preset(doc),
            _ => {
                let preset = self.current_preset(doc);
                (!preset.eq_ignore_ascii_case("custom")).then_some(preset)
            }
        }
    }

    /// Borrow the canvas slot behind a canvas variant. `None` for
    /// [`BorderSurface::Selection`] (which has no single slot) and
    /// for a canvas slot that has never been written.
    fn canvas_slot(self, doc: &MindMapDocument) -> Option<&baumhard::mindmap::model::GlyphBorderConfig> {
        let canvas = &doc.mindmap.canvas;
        match self {
            BorderSurface::Selection => None,
            BorderSurface::CanvasDefault => canvas.default_border.as_ref(),
            BorderSurface::CanvasSectionFrame => canvas.default_section_frame_border.as_ref(),
            BorderSurface::CanvasSectionFrameFocused => canvas.default_focused_section_frame_border.as_ref(),
        }
    }
}

/// A staged positional edit plus the one piece of presentation
/// state the caller needs back: the preset a `cycle` resolved to,
/// so the surface can prepend its `→ 'heavy' (cycle)` header.
pub(crate) struct PositionalEdits {
    pub(crate) edits: BorderConfigEdits,
    pub(crate) cycled_to: Option<String>,
}

/// Translate a positional border subverb into a staged
/// [`BorderConfigEdits`] bundle.
///
/// `descent` names the level, the subverb and the positional slot
/// it sat in, so the value reads at slot 0 of that subverb whether
/// the surface is `border preset heavy`, `canvas border preset
/// heavy` or `canvas section-frame focused preset heavy` — the
/// three used to pass `verb_pos` as a hand-counted `0` / `1` / `2`.
/// The `usage:` line each missing value reports is the very line
/// `help` prints for that form, derived from the same declaration.
///
/// Returns `Ok(None)` when the descent matched no subverb at all —
/// the caller owns the "unknown subverb" message because each
/// surface lists a different set of siblings. Errors are plain
/// strings; the caller lifts them into its own `ExecResult::Err`.
pub(crate) fn positional_subverb_to_edits(
    descent: &Descent,
    args: &Args,
    surface: BorderSurface,
    doc: &MindMapDocument,
) -> Result<Option<PositionalEdits>, String> {
    let Some(subverb) = descent.subverb() else {
        return Ok(None);
    };
    let verb_pos = descent.slot;
    let usage_line = usage::subverb_usage(descent.level, subverb);
    // The two refusals every form owes, asked once for all four
    // surfaces: a kv this subverb does not read, and a positional
    // past its declared slots. `border preset heavy color=#fff`
    // dropped the color without a word until this line existed.
    let pairs = kvs::read_strict(descent, args)?;
    let mut edits = BorderConfigEdits::default();
    let mut cycled_to: Option<String> = None;
    match subverb.name {
        "preset" => {
            let value = required_value(args, verb_pos + 1, usage_line)?;
            let name_lc = value.to_ascii_lowercase();
            if name_lc == "cycle" {
                let target = surface.next_preset(doc);
                edits.preset = OptionEdit::Set(target.clone());
                cycled_to = Some(target);
            } else {
                if !super::PRESETS.iter().any(|p| *p == name_lc) {
                    return Err(format!(
                        "preset '{}' unknown; pick one of {} | cycle",
                        value,
                        super::PRESETS.join(" | ")
                    ));
                }
                stage_kv(&mut edits, "preset", &name_lc)?;
            }
        }
        "color" => {
            let value = required_value(args, verb_pos + 1, usage_line)?;
            stage_kv(&mut edits, "color", value)?;
        }
        "padding" => {
            let value = required_value(args, verb_pos + 1, usage_line)?;
            stage_kv(&mut edits, "padding", value)?;
        }
        "palette" => {
            let value = required_value(args, verb_pos + 1, usage_line)?;
            stage_kv(&mut edits, "palette", value)?;
            // The optional `field=` kv composes with the positional
            // palette name — `border palette coral field=text`.
            if let Some(fv) = kvs::value(&pairs, "field") {
                stage_kv(&mut edits, "field", fv)?;
            }
        }
        "font" => {
            let value = required_value(args, verb_pos + 1, usage_line)?;
            stage_kv(&mut edits, "font", value)?;
            if let Some(sv) = kvs::value(&pairs, "size") {
                stage_kv(&mut edits, "size", sv)?;
            }
        }
        "side" => stage_side(args, verb_pos, usage_line, surface, doc, &mut edits)?,
        "corner" => stage_corner(args, verb_pos, usage_line, surface, doc, &mut edits)?,
        _ => return Ok(None),
    }
    Ok(Some(PositionalEdits { edits, cycled_to }))
}

/// `<surface> side <top|bottom|left|right|all> <pattern|reset>`.
/// `all` fans across the four sides; `reset` writes the surface's
/// *current* preset's default glyph back (the schema stores
/// per-side glyphs as plain `String`s, so `OptionEdit::Clear` is a
/// no-op — restoring the preset's own default is the
/// user-meaningful semantics).
fn stage_side(
    args: &Args,
    verb_pos: usize,
    usage_line: String,
    surface: BorderSurface,
    doc: &MindMapDocument,
    edits: &mut BorderConfigEdits,
) -> Result<(), String> {
    let label = surface.label();
    let which = required_value(args, verb_pos + 1, usage_line)?;
    let pattern = required_value(
        args,
        verb_pos + 2,
        format!("{} side {}: missing pattern (or 'reset' to clear)", label, which),
    )?;
    let sides = parse_side_selector(which).ok_or_else(|| {
        format!(
            "{} side: '{}' unknown; pick top | bottom | left | right | all",
            label, which
        )
    })?;
    let reset = pattern.eq_ignore_ascii_case("reset");
    if !reset {
        if let Some(non_custom) = surface.first_non_custom_preset(doc) {
            return Err(format!(
                "{} side {}: cannot set side glyph against preset '{}'. \
                 run `{} preset custom` first, then set the side.",
                label, which, non_custom, label
            ));
        }
    }
    // Sample the preset's glyph set once, outside the fan loop.
    let glyph_set = reset.then(|| preset_glyph_set(&surface.current_preset(doc)));
    for side in sides {
        match &glyph_set {
            Some(gs) => {
                let ch = match side {
                    BorderSide::Top => gs.top,
                    BorderSide::Bottom => gs.bottom,
                    BorderSide::Left => gs.left,
                    BorderSide::Right => gs.right,
                };
                edits.with_side_pattern(side, &ch.to_string())?;
            }
            None => edits.with_side_pattern(side, pattern)?,
        }
    }
    Ok(())
}

/// `<surface> corner <tl|tr|bl|br|all> <glyph|reset>`. Same shape
/// as [`stage_side`], writing the four corner slots instead.
fn stage_corner(
    args: &Args,
    verb_pos: usize,
    usage_line: String,
    surface: BorderSurface,
    doc: &MindMapDocument,
    edits: &mut BorderConfigEdits,
) -> Result<(), String> {
    let label = surface.label();
    let which = required_value(args, verb_pos + 1, usage_line)?;
    let glyph = required_value(
        args,
        verb_pos + 2,
        format!("{} corner {}: missing glyph (or 'reset' to clear)", label, which),
    )?;
    let corners = parse_corner_selector(which).ok_or_else(|| {
        format!(
            "{} corner: '{}' unknown; pick tl | tr | bl | br | all",
            label, which
        )
    })?;
    let reset = glyph.eq_ignore_ascii_case("reset");
    if !reset {
        if let Some(non_custom) = surface.first_non_custom_preset(doc) {
            return Err(format!(
                "{} corner {}: cannot set corner glyph against preset '{}'. \
                 run `{} preset custom` first, then set the corner.",
                label, which, non_custom, label
            ));
        }
    }
    let glyph_set = reset.then(|| preset_glyph_set(&surface.current_preset(doc)));
    for corner in corners {
        match &glyph_set {
            Some(gs) => {
                let ch = match corner {
                    "tl" => gs.top_left,
                    "tr" => gs.top_right,
                    "bl" => gs.bottom_left,
                    "br" => gs.bottom_right,
                    // `parse_corner_selector` only emits the four
                    // corners today; a future selector extension
                    // degrades to an error rather than a panic
                    // (CODE_CONVENTIONS §9).
                    other => return Err(format!("internal: unrecognized corner '{}'", other)),
                };
                stage_kv(edits, corner, &ch.to_string())?;
            }
            None => stage_kv(edits, corner, glyph)?,
        }
    }
    Ok(())
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

/// Read a required positional, or fail with `usage`.
fn required_value<'a>(args: &'a Args, index: usize, usage: String) -> Result<&'a str, String> {
    args.positional(index).ok_or(usage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::spec::descent::descend;
    use crate::application::document::tests_common::load_test_doc;

    /// Every subverb the border grammar marks as positional is one
    /// [`positional_subverb_to_edits`] actually parses, and a
    /// subverb outside that set is not. The grammar table is what
    /// both popups withhold at a kv-form slot and what all four
    /// execute dispatchers gate, so a subverb added to the parser
    /// and forgotten there would be silently unreachable from every
    /// border popup — with every other test still green.
    #[test]
    fn test_every_positional_subverb_is_parsed() {
        let doc = load_test_doc();
        for subverb in super::super::grammar::POSITIONAL_SUBVERBS {
            // No value token follows, so each recognized subverb
            // answers with its own usage error — the one answer
            // that is neither "not mine" nor a staged edit.
            let tokens = vec![subverb.name.to_string()];
            let args = Args::new(&tokens);
            let descent = descend(&super::super::grammar::BORDER, &tokens);
            assert!(
                positional_subverb_to_edits(&descent, &args, BorderSurface::Selection, &doc).is_err(),
                "'{}' is offered as a positional subverb but the parser does not claim it",
                subverb.name
            );
        }
        // `show` is a subverb of the same level that this parser
        // deliberately does not claim — the caller routes it
        // elsewhere, and the parser answers "not mine" rather than
        // inventing an edit. The failing input is a `show` arm
        // added here, which would make the readout stage a border
        // edit instead of printing.
        let tokens = vec!["show".to_string()];
        let args = Args::new(&tokens);
        let descent = descend(&super::super::grammar::BORDER, &tokens);
        assert!(matches!(
            positional_subverb_to_edits(&descent, &args, BorderSurface::Selection, &doc),
            Ok(None)
        ));
    }
}
