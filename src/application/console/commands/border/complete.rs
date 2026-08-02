// SPDX-License-Identifier: MPL-2.0

//! `border` contextual completion. Mirrors `font.rs`'s structure:
//! token 0 surfaces verbs + kv keys; `KvValue { key }` matches a
//! per-key vocabulary (presets, palette names, font families).

use baumhard::mindmap::border::border_preset_hint;

use crate::application::console::completion::{
    prefix_filter, Completion, CompletionContext, CompletionState,
};
use crate::application::console::ConsoleContext;

pub fn complete_border(state: &CompletionState, ctx: &ConsoleContext) -> Vec<Completion> {
    // The engine's `Token { index: N }` indexes positionals
    // *after* the verb name (`border`). The dispatch reads
    // tokens[1..] (tokens[0] is "border" itself); first
    // positional after the verb name is at engine index 0.
    let token1 = state.tokens.get(1).map(String::as_str);
    let token2 = state.tokens.get(2).map(String::as_str);
    let after_preview = token1 == Some("preview");
    match &state.context {
        CompletionContext::Token { index: 0 } => verb_or_key(state.partial),
        //positional-subverb value completion. When
        // tokens[1] is a known positional subverb, the next
        // positional is its value — surface a typed vocabulary
        // instead of falling through to kv keys (which would
        // suggest `preset=` / `color=` etc., the wrong context).
        CompletionContext::Token { index: 1 } if after_preview => {
            let mut out = preview_subverb_completions(state.partial);
            out.extend(key_completions(state.partial));
            out
        }
        CompletionContext::Token { index: 1 } => match token1.map(str::to_ascii_lowercase).as_deref() {
            Some("preset") => preset_value_completions(state.partial),
            Some("color") => prefix_filter(super::COLOR_PRESETS, state.partial),
            Some("palette") => palette_value_completions(state.partial, ctx),
            Some("font") => font_family_completions(state.partial),
            Some("side") => prefix_filter(SIDE_VALUES, state.partial),
            Some("corner") => prefix_filter(CORNER_VALUES, state.partial),
            Some("show") => show_arg_completions(state.partial),
            // padding takes a number — no candidate vocabulary;
            // toggle / on / off / reset take nothing — same.
            _ => Vec::new(),
        },
        //second-positional value: side <TAB> takes
        // pattern (free-form) or `reset`; corner <TAB> takes a
        // single glyph (free-form) or `reset`.
        CompletionContext::Token { index: 2 } => match token1.map(str::to_ascii_lowercase).as_deref() {
            Some("side") => side_pattern_completions(state.partial, token2),
            Some("corner") => corner_glyph_completions(state.partial),
            _ => Vec::new(),
        },
        CompletionContext::Token { .. } => key_completions(state.partial),
        CompletionContext::KvValue { key } => kv_value_completions(key.as_str(), state.partial, ctx),
        _ => Vec::new(),
    }
}

const SIDE_VALUES: &[&str] = &["top", "bottom", "left", "right", "all"];
const CORNER_VALUES: &[&str] = &["tl", "tr", "bl", "br", "all"];

/// `border preset <TAB>` and `border preset=<TAB>` value
/// completion: every entry from `BORDER_PRESETS` plus `cycle`.
///
/// Hints come from `baumhard::mindmap::border::border_preset_hint`,
/// which derives from the same `PRESET_TABLE` `BORDER_PRESETS` does
/// — a preset added to the table arrives here already described,
/// instead of completing with a blank hint.
fn preset_value_completions(partial: &str) -> Vec<Completion> {
    let mut out: Vec<Completion> = super::PRESETS
        .iter()
        .filter(|p| p.starts_with(partial))
        .map(|p| Completion {
            text: p.to_string(),
            display: p.to_string(),
            hint: border_preset_hint(p).map(str::to_string),
            font_family: None,
        })
        .collect();
    if "cycle".starts_with(partial) {
        out.push(Completion {
            text: "cycle".to_string(),
            display: "cycle".to_string(),
            hint: Some("advance to the next preset (wraps)".to_string()),
            font_family: None,
        });
    }
    out
}

/// `border show <TAB>` surfaces the optional `side=` filter
/// kv and the `verbose` positional flag (/ B6.8).
/// Pre-fix neither was discoverable from completion.
fn show_arg_completions(partial: &str) -> Vec<Completion> {
    let mut out = Vec::new();
    if "side=".starts_with(partial) || "side".starts_with(partial) {
        out.push(Completion {
            text: "side=".to_string(),
            display: "side=".to_string(),
            hint: Some("filter readout to one side (top|bottom|left|right|all)".to_string()),
            font_family: None,
        });
    }
    if "verbose".starts_with(partial) {
        out.push(Completion {
            text: "verbose".to_string(),
            display: "verbose".to_string(),
            hint: Some("surface the dual color cascade (frame_color vs border.color)".to_string()),
            font_family: None,
        });
    }
    out
}

/// `border side WHICH <TAB>` — pattern templates plus `reset`.
/// Templates are free-form so we don't surface a glyph
/// catalogue, but `reset` is the discoverability gap (users
/// won't guess "reset" without seeing it).
fn side_pattern_completions(partial: &str, _which: Option<&str>) -> Vec<Completion> {
    let mut out = Vec::new();
    if "reset".starts_with(partial) {
        out.push(Completion {
            text: "reset".to_string(),
            display: "reset".to_string(),
            hint: Some("restore the slot's preset's default glyph".to_string()),
            font_family: None,
        });
    }
    out
}

fn corner_glyph_completions(partial: &str) -> Vec<Completion> {
    let mut out = Vec::new();
    if "reset".starts_with(partial) {
        out.push(Completion {
            text: "reset".to_string(),
            display: "reset".to_string(),
            hint: Some("restore the slot's preset's default glyph".to_string()),
            font_family: None,
        });
    }
    out
}

/// `border preview <TAB>` → `commit` / `cancel` rows with hints.
/// C12 fix: the prior shape used `prefix_filter(PREVIEW_SUBVERBS, …)`
/// which yields hint-less rows; users couldn't tell which subverb
/// did what without reading the source. Re-exported for the
/// section-frame and canvas verbs so all four preview surfaces
/// share the same hint vocabulary.
pub(crate) fn preview_subverb_completions(partial: &str) -> Vec<Completion> {
    super::PREVIEW_SUBVERBS
        .iter()
        .filter(|s| s.starts_with(partial))
        .map(|s| Completion {
            text: s.to_string(),
            display: s.to_string(),
            hint: Some(preview_subverb_hint(s).to_string()),
            font_family: None,
        })
        .collect()
}

fn preview_subverb_hint(s: &str) -> &'static str {
    match s {
        "commit" => "write the staged preview through and clear the slot",
        "cancel" => "discard the staged preview, no model write",
        _ => "",
    }
}

/// Shared per-key value completer for the `border` kv vocabulary.
/// Returns the set of completions to surface inside the popup when
/// the cursor is on the *value* side of `<key>=<value>`. Reused by
/// `section frame …` and `canvas …` so the popup vocabulary is
/// byte-identical regardless of which border surface the user is
/// editing.
pub fn kv_value_completions(key: &str, partial: &str, ctx: &ConsoleContext) -> Vec<Completion> {
    match key {
        "preset" => preset_value_completions(partial),
        "field" => prefix_filter(super::FIELDS, partial),
        "color" => prefix_filter(super::COLOR_PRESETS, partial),
        "palette" => palette_value_completions(partial, ctx),
        "font" => font_family_completions(partial),
        _ => Vec::new(),
    }
}

fn verb_or_key(partial: &str) -> Vec<Completion> {
    let mut out: Vec<Completion> = Vec::new();
    for v in super::VERBS {
        if v.starts_with(partial) {
            out.push(Completion {
                text: v.to_string(),
                display: v.to_string(),
                hint: Some(verb_hint(v).to_string()),
                font_family: None,
            });
        }
    }
    out.extend(key_completions(partial));
    out
}

fn key_completions(partial: &str) -> Vec<Completion> {
    super::KEYS
        .iter()
        .filter(|k| k.starts_with(partial))
        .map(|k| Completion {
            text: format!("{}=", k),
            display: format!("{}=", k),
            hint: Some(key_hint(k).to_string()),
            font_family: None,
        })
        .collect()
}

fn verb_hint(v: &str) -> &'static str {
    match v {
        "on" => "show the border",
        "off" => "hide the border",
        "toggle" => "flip show_frame per node",
        "show" => "print the resolved config (use [side=…] [verbose])",
        "reset" => "drop the per-node override",
        "preview" => "stage a preview without writing the model (commit/cancel terminates)",
        "preset" => "pick light|heavy|double|rounded|custom or `cycle`",
        "color" => "set border color (#hex|var|preset|reset)",
        "padding" => "set border padding in pixels",
        "palette" => "cycle a palette across glyphs (or `off`)",
        "font" => "set border glyph font family (with optional size=)",
        "side" => "set per-side glyph (top|bottom|left|right|all)",
        "corner" => "set per-corner glyph (tl|tr|bl|br|all)",
        _ => "",
    }
}

/// Per-key hint for the verb-or-key surface (token 0 of `border`).
/// Delegates to the shared [`super::kv_hint`] used across `border`,
/// `section frame`, and `canvas` so the hint table lives in one
/// place. Returns `""` for unknown keys to preserve the
/// `&'static str` return shape this completer's row-emit expects.
fn key_hint(k: &str) -> &'static str {
    super::kv_hint(k).unwrap_or("")
}

fn palette_value_completions(partial: &str, ctx: &ConsoleContext) -> Vec<Completion> {
    let mut names: Vec<&str> = ctx.document.mindmap.palettes.keys().map(String::as_str).collect();
    names.sort();
    let mut out: Vec<Completion> = names
        .into_iter()
        .filter(|n| n.starts_with(partial))
        .map(|n| Completion {
            text: n.to_string(),
            display: n.to_string(),
            hint: None,
            font_family: None,
        })
        .collect();
    if "off".starts_with(partial) {
        out.push(Completion {
            text: "off".to_string(),
            display: "off".to_string(),
            hint: Some("clear palette cycling".to_string()),
            font_family: None,
        });
    }
    out
}

fn font_family_completions(partial: &str) -> Vec<Completion> {
    let lower = partial.to_ascii_lowercase();
    baumhard::font::fonts::loaded_families_iter()
        .filter(|f| f.to_ascii_lowercase().starts_with(&lower))
        .map(|family| Completion {
            text: family.to_string(),
            display: family.to_string(),
            hint: None,
            font_family: Some(family.to_string()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::completion::CompletionContext;
    use crate::application::document::MindMapDocument;

    fn fixture_doc() -> MindMapDocument {
        crate::application::document::tests_common::load_test_doc()
    }

    fn state<'a>(
        partial: &'a str,
        ctx: CompletionContext,
        tokens_owned: &'a [String],
    ) -> CompletionState<'a> {
        CompletionState {
            tokens: tokens_owned,
            cursor_token: 0,
            partial,
            context: ctx,
        }
    }

    /// `border <TAB>` at token 0 surfaces every verb (on / off /
    /// show / reset) plus every kv key — the user gets a
    /// scannable menu of every operation the verb supports.
    #[test]
    fn complete_token_zero_offers_verbs_and_kv_keys() {
        let doc = fixture_doc();
        let ctx = ConsoleContext::from_document(&doc);
        let tokens = Vec::<String>::new();
        let s = state("", CompletionContext::Token { index: 0 }, &tokens);
        let out = complete_border(&s, &ctx);
        let labels: Vec<&str> = out.iter().map(|c| c.display.as_str()).collect();
        // Verbs.
        for v in &["on", "off", "show", "reset"] {
            assert!(
                labels.iter().any(|l| l == v),
                "expected verb '{}' in completions: {:?}",
                v,
                labels
            );
        }
        // A handful of kv keys.
        for k in &["preset=", "font=", "size=", "color=", "palette="] {
            assert!(
                labels.iter().any(|l| l == k),
                "expected kv key '{}' in completions: {:?}",
                k,
                labels
            );
        }
    }

    /// `border preset=<TAB>` lists every preset name. Static
    /// vocabulary; doesn't need the document.
    #[test]
    fn complete_preset_value_lists_five_presets() {
        let doc = fixture_doc();
        let ctx = ConsoleContext::from_document(&doc);
        let tokens = Vec::<String>::new();
        let s = state(
            "",
            CompletionContext::KvValue {
                key: "preset".to_string(),
            },
            &tokens,
        );
        let out = complete_border(&s, &ctx);
        let labels: Vec<&str> = out.iter().map(|c| c.text.as_str()).collect();
        for p in &["light", "heavy", "double", "rounded", "custom"] {
            assert!(
                labels.iter().any(|l| l == p),
                "expected preset '{}' in completions: {:?}",
                p,
                labels
            );
        }
    }

    /// `border palette=<TAB>` lists every palette in the
    /// document plus the `off` sentinel for clearing. Dynamic —
    /// reaches into `doc.mindmap.palettes`.
    #[test]
    fn complete_palette_value_lists_doc_palettes_and_off() {
        let doc = fixture_doc();
        let ctx = ConsoleContext::from_document(&doc);
        let tokens = Vec::<String>::new();
        let s = state(
            "",
            CompletionContext::KvValue {
                key: "palette".to_string(),
            },
            &tokens,
        );
        let out = complete_border(&s, &ctx);
        let labels: Vec<&str> = out.iter().map(|c| c.text.as_str()).collect();
        // Off sentinel must always appear.
        assert!(
            labels.iter().any(|l| l == &"off"),
            "expected 'off' sentinel: {:?}",
            labels
        );
        // Every palette key in the doc must appear.
        for name in doc.mindmap.palettes.keys() {
            assert!(
                labels.iter().any(|l| l == &name.as_str()),
                "expected palette '{}' in completions: {:?}",
                name,
                labels
            );
        }
    }

    /// `border field=<TAB>` lists the four `ColorGroup` channels.
    #[test]
    fn complete_field_value_lists_four_channels() {
        let doc = fixture_doc();
        let ctx = ConsoleContext::from_document(&doc);
        let tokens = Vec::<String>::new();
        let s = state(
            "",
            CompletionContext::KvValue {
                key: "field".to_string(),
            },
            &tokens,
        );
        let out = complete_border(&s, &ctx);
        let labels: Vec<&str> = out.iter().map(|c| c.text.as_str()).collect();
        for f in &["frame", "background", "text", "title"] {
            assert!(
                labels.iter().any(|l| l == f),
                "expected field '{}' in completions: {:?}",
                f,
                labels
            );
        }
    }

    /// `border font=<TAB>` reuses the font-family completer:
    /// every popup row carries `font_family = Some(<name>)` so
    /// the renderer shapes the candidate label in that face.
    /// Mirrors `font.rs::tests::completion_after_set_returns_loaded_families_in_their_face`.
    #[test]
    fn complete_font_value_rows_carry_family_tag() {
        baumhard::font::fonts::init();
        let doc = fixture_doc();
        let ctx = ConsoleContext::from_document(&doc);
        let tokens = Vec::<String>::new();
        let s = state(
            "",
            CompletionContext::KvValue {
                key: "font".to_string(),
            },
            &tokens,
        );
        let out = complete_border(&s, &ctx);
        assert!(!out.is_empty(), "loaded fonts list must not be empty");
        for c in &out {
            assert_eq!(
                c.font_family.as_deref(),
                Some(c.text.as_str()),
                "every font-completion row must tag its display family"
            );
        }
    }
}

#[cfg(test)]
mod plan_5_9_tests {
    //!completion improvements — pin the new
    //! positional-subverb value rows the prior `complete.rs`
    //! never surfaced (Plan Adherence reviewer flagged the
    //! gap as silently-deferred).

    use super::*;
    use crate::application::console::completion::CompletionContext;

    fn fixture_doc() -> crate::application::document::MindMapDocument {
        crate::application::document::tests_common::load_test_doc()
    }

    fn at_token1<'a>(
        partial: &'a str,
        tokens: &'a [String],
    ) -> CompletionState<'a> {
        CompletionState {
            tokens,
            cursor_token: 1,
            partial,
            context: CompletionContext::Token { index: 1 },
        }
    }

    fn at_token2<'a>(
        partial: &'a str,
        tokens: &'a [String],
    ) -> CompletionState<'a> {
        CompletionState {
            tokens,
            cursor_token: 2,
            partial,
            context: CompletionContext::Token { index: 2 },
        }
    }

    /// `border preset <TAB>` surfaces every preset name + `cycle`.
    #[test]
    fn preset_positional_completion_includes_cycle() {
        let doc = fixture_doc();
        let ctx = ConsoleContext::from_document(&doc);
        let tokens = vec!["border".to_string(), "preset".to_string()];
        let s = at_token1("", &tokens);
        let labels: Vec<String> = complete_border(&s, &ctx)
            .into_iter()
            .map(|c| c.display)
            .collect();
        for p in &["light", "heavy", "double", "rounded", "custom", "cycle"] {
            assert!(
                labels.iter().any(|l| l == p),
                "preset completion missing '{}': {:?}",
                p,
                labels
            );
        }
    }

    /// Every preset row the popup renders carries a non-empty
    /// hint — including `cycle`, which is not a preset but shares
    /// the row shape. A blank hint column is the symptom the
    /// baumhard-side table fold exists to prevent, so pin it at
    /// the surface the user actually sees.
    #[test]
    fn preset_completion_rows_all_carry_a_hint() {
        let doc = fixture_doc();
        let ctx = ConsoleContext::from_document(&doc);
        let tokens = vec!["border".to_string(), "preset".to_string()];
        let s = at_token1("", &tokens);
        let rows = complete_border(&s, &ctx);
        assert!(!rows.is_empty(), "preset completion must offer rows");
        for row in &rows {
            let hint = row
                .hint
                .as_deref()
                .unwrap_or_else(|| panic!("preset row '{}' has no hint", row.display));
            assert!(!hint.is_empty(), "preset row '{}' has an empty hint", row.display);
        }
    }

    #[test]
    fn preset_kv_value_completion_includes_cycle() {
        let doc = fixture_doc();
        let ctx = ConsoleContext::from_document(&doc);
        let tokens = vec!["border".to_string()];
        let s = CompletionState {
            tokens: &tokens,
            cursor_token: 1,
            partial: "",
            context: CompletionContext::KvValue {
                key: "preset".to_string(),
            },
        };
        let labels: Vec<String> = complete_border(&s, &ctx)
            .into_iter()
            .map(|c| c.display)
            .collect();
        assert!(
            labels.iter().any(|l| l == "cycle"),
            "preset= completion missing 'cycle': {:?}",
            labels
        );
    }

    #[test]
    fn side_positional_completion_lists_four_sides_and_all() {
        let doc = fixture_doc();
        let ctx = ConsoleContext::from_document(&doc);
        let tokens = vec!["border".to_string(), "side".to_string()];
        let s = at_token1("", &tokens);
        let labels: Vec<String> = complete_border(&s, &ctx)
            .into_iter()
            .map(|c| c.display)
            .collect();
        for v in &["top", "bottom", "left", "right", "all"] {
            assert!(
                labels.iter().any(|l| l == v),
                "side completion missing '{}': {:?}",
                v,
                labels
            );
        }
    }

    #[test]
    fn corner_positional_completion_lists_four_corners_and_all() {
        let doc = fixture_doc();
        let ctx = ConsoleContext::from_document(&doc);
        let tokens = vec!["border".to_string(), "corner".to_string()];
        let s = at_token1("", &tokens);
        let labels: Vec<String> = complete_border(&s, &ctx)
            .into_iter()
            .map(|c| c.display)
            .collect();
        for v in &["tl", "tr", "bl", "br", "all"] {
            assert!(
                labels.iter().any(|l| l == v),
                "corner completion missing '{}': {:?}",
                v,
                labels
            );
        }
    }

    #[test]
    fn side_second_positional_offers_reset() {
        let doc = fixture_doc();
        let ctx = ConsoleContext::from_document(&doc);
        let tokens = vec!["border".to_string(), "side".to_string(), "top".to_string()];
        let s = at_token2("", &tokens);
        let labels: Vec<String> = complete_border(&s, &ctx)
            .into_iter()
            .map(|c| c.display)
            .collect();
        assert!(labels.iter().any(|l| l == "reset"));
    }

    #[test]
    fn corner_second_positional_offers_reset() {
        let doc = fixture_doc();
        let ctx = ConsoleContext::from_document(&doc);
        let tokens = vec!["border".to_string(), "corner".to_string(), "tl".to_string()];
        let s = at_token2("", &tokens);
        let labels: Vec<String> = complete_border(&s, &ctx)
            .into_iter()
            .map(|c| c.display)
            .collect();
        assert!(labels.iter().any(|l| l == "reset"));
    }

    #[test]
    fn show_arg_completion_offers_side_and_verbose() {
        let doc = fixture_doc();
        let ctx = ConsoleContext::from_document(&doc);
        let tokens = vec!["border".to_string(), "show".to_string()];
        let s = at_token1("", &tokens);
        let labels: Vec<String> = complete_border(&s, &ctx)
            .into_iter()
            .map(|c| c.display)
            .collect();
        assert!(labels.iter().any(|l| l == "side="));
        assert!(labels.iter().any(|l| l == "verbose"));
    }

    /// `border palette <TAB>` (positional form) lists palette
    /// names + `off` — same vocabulary the kv `palette=<TAB>`
    /// surfaces.
    #[test]
    fn palette_positional_completion_lists_palettes_and_off() {
        let doc = fixture_doc();
        let ctx = ConsoleContext::from_document(&doc);
        let tokens = vec!["border".to_string(), "palette".to_string()];
        let s = at_token1("", &tokens);
        let labels: Vec<String> = complete_border(&s, &ctx)
            .into_iter()
            .map(|c| c.display)
            .collect();
        assert!(labels.iter().any(|l| l == "off"), "palette positional missing 'off'");
        // At least one palette name should surface (testament fixture has palettes).
        assert!(
            labels.iter().any(|l| l != "off"),
            "palette positional should list palette names: {:?}",
            labels
        );
    }

    #[test]
    fn color_positional_completion_lists_color_presets() {
        let doc = fixture_doc();
        let ctx = ConsoleContext::from_document(&doc);
        let tokens = vec!["border".to_string(), "color".to_string()];
        let s = at_token1("", &tokens);
        let labels: Vec<String> = complete_border(&s, &ctx)
            .into_iter()
            .map(|c| c.display)
            .collect();
        // COLOR_PRESETS includes `accent`, `edge`, `fg`, `reset`.
        assert!(labels.iter().any(|l| l == "accent"));
        assert!(labels.iter().any(|l| l == "reset"));
    }
}
