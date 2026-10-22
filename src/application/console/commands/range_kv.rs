// SPDX-License-Identifier: MPL-2.0

//! The shared `section=N` / `range=A..B` targeting vocabulary —
//! both halves of it.
//!
//! `range=A..B` lifts the Rust-style half-open form into
//! `(usize, usize)` grapheme indices that the range-aware document
//! setters (`set_section_text_color_range`, `_font_size_range`,
//! `_font_family_range`) consume directly; `section=N` lifts a
//! non-negative integer.
//!
//! Alongside the parsers sit the two [`Key`] declarations the
//! levels that speak them splice into their own key sets, and
//! [`section_idx_completions`] — the popup those same verbs offer
//! on the value side of `section=`. One declaration each, so the
//! four surfaces that carry `section=` describe it one way and
//! answer it one way.
//!
//! They live here for the same reason the parsers do: `color`,
//! `font`, `section` and `section frame` all speak these two kvs.
//! What went wrong was not a copied value list but a *missing*
//! one: each verb's `KvValue` arm matched against its whole `KEYS`
//! list and answered with the single vocabulary it happened to
//! know, so `font section=<TAB>` offered point sizes — the
//! vocabulary of a different key on the same verb — while `color
//! section=<TAB>` offered color presets. Neither verb was wrong
//! about itself; neither had an arm for the key under the cursor.
//! Giving a shared kv one shared answer is what retires the class.

use crate::application::console::completion::Completion;
use crate::application::console::spec::kvs::{self, Pair};
use crate::application::console::spec::{free, Key, Vocabulary};
use crate::application::console::ConsoleContext;
use crate::application::document::GraphemeRange;
use crate::application::document::MindMapDocument;

/// Both shared targeting keys, for the levels that take a grapheme
/// range as well as a section index — `color` and `font`.
pub(super) const TARGET_KEYS: &[Key] = &[SECTION_KEY, RANGE_KEY];

/// The `section=<idx>` targeting key, declared once for every level
/// that speaks it.
///
/// A level splices this slice into its own `key_sets` rather than
/// re-declaring it, which is what stops the four popups that
/// surface `section=` from coming to describe it four ways. It is
/// also why `section=<TAB>` answers with section indices at every
/// one: each verb's hand-written `KvValue` arm used to match its
/// *whole* key list and answer with the single vocabulary it
/// happened to know, so `font section=<TAB>` offered point sizes
/// and `color section=<TAB>` offered color names.
pub(super) const SECTION_KEYS: &[Key] = &[SECTION_KEY];

const RANGE_KEY: Key = Key::new(
    "range",
    "grapheme range A..B inside the targeted section",
    free("A..B"),
);

const SECTION_KEY: Key = Key::new(
    "section",
    "target section index inside a multi-section node",
    Vocabulary::Rows {
        placeholder: "idx",
        rows: section_idx_rows,
        sentinels: &[],
    },
);

/// [`section_idx_completions`] in the shape a [`Vocabulary`] wants.
fn section_idx_rows(ctx: &ConsoleContext, partial: &str) -> Vec<Completion> {
    section_idx_completions(ctx, partial)
}

/// Parse a `range=A..B` kv value into a [`GraphemeRange`] over a
/// section's grapheme clusters. Accepts the Rust-style
/// `usize..usize` half-open form. Rejects empty halves,
/// non-numeric components, and `start >= end` (an empty or
/// inverted range is a usage error — the verb path lifts this to
/// an `ExecResult::err`). Returning the typed range rather than a
/// bare pair keeps the grapheme meaning attached from the parse
/// on, so it cannot be mistaken for section indices downstream
/// (issue #47 part C).
pub(super) fn parse_range_kv(value: &str) -> Result<GraphemeRange, String> {
    let (start_str, end_str) = match value.split_once("..") {
        Some(pair) => pair,
        None => return Err("expected `start..end` (e.g. `range=2..7`)".to_string()),
    };
    if start_str.is_empty() || end_str.is_empty() {
        return Err("both halves of `start..end` must be non-empty".to_string());
    }
    let start: usize = start_str
        .parse()
        .map_err(|_| format!("`{}` is not a non-negative integer", start_str))?;
    let end: usize = end_str
        .parse()
        .map_err(|_| format!("`{}` is not a non-negative integer", end_str))?;
    if start >= end {
        return Err(format!(
            "empty / inverted range — `{}..{}` requires start < end",
            start, end
        ));
    }
    Ok(GraphemeRange::new(start, end))
}

/// Parse a `section=N` kv value into a non-negative integer.
/// Used by `color` / `font` / `section` verbs. Verb name is
/// prepended to the error message so the caller doesn't have to
/// re-format. Returns `Err` on negative or non-numeric input.
pub(super) fn parse_section_kv(verb: &str, value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|_| format!("{}: section='{}' is not a non-negative integer", verb, value))
}

/// The `section=` / `range=` pair as a verb reads it, with the one
/// rule that binds them already applied.
pub(super) struct SectionRange {
    /// The section the edit targets, when the line named one.
    pub(super) section: Option<usize>,
    /// The grapheme range inside that section, when the line named
    /// one.
    pub(super) range: Option<GraphemeRange>,
}

/// Lift the two shared targeting kvs out of an already-read pair
/// list.
///
/// The rule they carry: a range indexes graphemes *inside* one
/// section, so `range=` without `section=` is a usage error. Three
/// copies of this loop lived in `color` and `font` — two of them in
/// the same file — each with its own spelling of the same three
/// messages.
pub(super) fn extract_section_range_kvs(pairs: &[Pair], verb: &str) -> Result<SectionRange, String> {
    let mut out = SectionRange {
        section: None,
        range: None,
    };
    if let Some(v) = kvs::value(pairs, "section") {
        out.section = Some(parse_section_kv(verb, v)?);
    }
    if let Some(v) = kvs::value(pairs, "range") {
        out.range = Some(parse_range_kv(v).map_err(|msg| format!("{}: range='{}' — {}", verb, v, msg))?);
    }
    if out.range.is_some() && out.section.is_none() {
        return Err(format!(
            "{}: range=A..B requires section=N — ranges target grapheme indices inside one section",
            verb
        ));
    }
    Ok(out)
}

/// Reject a range whose start sits past the target section's
/// grapheme count.
///
/// Without this pre-flight the range-aware setters silently no-op
/// and the verb prints "no change", which is indistinguishable from
/// "you set red on already-red text". `color` and `font` each
/// carried a copy, worded the same and reached from different
/// depths.
pub(super) fn preflight_range(
    doc: &MindMapDocument,
    node_id: &str,
    section_idx: usize,
    range: Option<GraphemeRange>,
    verb: &str,
) -> Result<(), String> {
    let Some(start) = range.map(|r| r.start()) else {
        return Ok(());
    };
    let Some(section) = doc
        .mindmap
        .nodes
        .get(node_id)
        .and_then(|n| n.sections.get(section_idx))
    else {
        return Ok(());
    };
    let total = baumhard::util::grapheme_chad::count_grapheme_clusters(&section.text);
    if start >= total {
        return Err(format!(
            "{}: range_start={} is past the section's grapheme count ({})",
            verb, start, total
        ));
    }
    Ok(())
}

/// The popup for `section=<TAB>`: one row per section on the
/// selection's primary node, each hinted with a short preview of
/// that section's text so the user can tell which is which.
///
/// Grapheme-aware truncation via `take_graphemes` — a preview cut
/// at 20 *bytes* would slice a multi-codepoint emoji in half.
pub(super) fn section_idx_completions(ctx: &ConsoleContext, partial: &str) -> Vec<Completion> {
    use baumhard::util::grapheme_chad::take_graphemes;
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
            // One walk, no prefix allocation — `take_graphemes`
            // returns the borrowed prefix and the overflow flag
            // together. Empty sections render `(empty)` so the row
            // isn't a bare bullet.
            let (preview, overflow) = take_graphemes(&section.text, 20);
            let hint = if preview.is_empty() {
                "(empty)".to_string()
            } else if overflow {
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

#[cfg(test)]
mod tests {
    use super::{parse_range_kv, GraphemeRange};

    #[test]
    fn test_parse_range_kv_happy_path() {
        assert_eq!(parse_range_kv("2..7"), Ok(GraphemeRange::new(2, 7)));
        assert_eq!(parse_range_kv("0..1"), Ok(GraphemeRange::new(0, 1)));
        assert_eq!(parse_range_kv("100..1000"), Ok(GraphemeRange::new(100, 1000)));
    }

    #[test]
    fn test_parse_range_kv_missing_separator() {
        assert!(parse_range_kv("27").is_err());
        assert!(parse_range_kv("").is_err());
    }

    #[test]
    fn test_parse_range_kv_empty_halves() {
        assert!(parse_range_kv("..7").is_err());
        assert!(parse_range_kv("2..").is_err());
        assert!(parse_range_kv("..").is_err());
    }

    #[test]
    fn test_parse_range_kv_non_numeric() {
        assert!(parse_range_kv("foo..bar").is_err());
        assert!(parse_range_kv("2..bar").is_err());
        assert!(parse_range_kv("foo..7").is_err());
    }

    #[test]
    fn test_parse_range_kv_inverted_or_empty() {
        // `start >= end` rejected — empty or inverted range is
        // a usage error rather than a silent no-op.
        assert!(parse_range_kv("5..5").is_err());
        assert!(parse_range_kv("7..3").is_err());
    }

    #[test]
    fn test_parse_range_kv_negative_rejected() {
        // `usize::parse` rejects negative integers — surface as
        // a clear error message rather than silent overflow.
        assert!(parse_range_kv("-1..5").is_err());
    }
}
