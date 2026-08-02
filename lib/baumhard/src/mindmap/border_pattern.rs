// SPDX-License-Identifier: MPL-2.0

//! Border-side **pattern syntax** — the parser and grapheme-aware
//! fitter for the strings stored in `CustomBorderGlyphs.{top,
//! bottom, left, right}`.
//!
//! ## Syntax
//!
//! A side pattern is one string that fills the gap between a
//! border's two corners. Two shapes:
//!
//! 1. **Atomic-repeat** — no fill region. The whole pattern is one
//!    cluster sequence repeated as many whole times as fits.
//!    Example: `+=##=+` → repeats `+=##=+`.
//!
//! 2. **Prefix + Fill + Suffix** — exactly one fill region
//!    delimited by unescaped `(` and `)`. The prefix and suffix
//!    are placed once at the ends; the fill is repeated atomically
//!    as many whole times as fits between them. A single fill
//!    iteration is *also* atomic — never split.
//!    Example: `###(*)###` → `###`, `*` × N, `###`.
//!    Example: `+=#(\(\))#=+` → `+=#`, `()` × N, `#=+`.
//!
//! ## Escapes
//!
//! Three escape sequences are recognized everywhere in the input:
//!
//! - `\(` → literal `(`
//! - `\)` → literal `)`
//! - `\\` → literal `\`
//!
//! Any other backslash is a parse error rather than silently lost,
//! so a typo doesn't ship as a corrupt model value.
//!
//! ## Grapheme awareness
//!
//! After parsing, each section's string is split into grapheme
//! clusters via `grapheme_chad::split_graphemes_owned` — the same
//! `unicode-segmentation` walk every other Mandala text path uses,
//! reached through the one module allowed to name it
//! (CONVENTIONS §B3). Cluster counts,
//! not codepoint counts, drive fitter math; combining-mark glyphs
//! and ZWJ emoji each occupy one cell.
//!
//! ## Why a separate module
//!
//! Three pipelines (scene builder, tree builder, renderer) all
//! need to render border sides; centralizing the parse + render
//! here lets the call sites stay small refactors and keeps the
//! grammar in one place. Pure data — no cosmic-text, no wgpu —
//! so it compiles for `wasm32` by construction and is easy to
//! unit-test.

use crate::util::grapheme_chad::split_graphemes_owned;

/// A parsed side pattern. The two variants reflect the two
/// well-formed inputs the grammar accepts; everything else
/// surfaces from [`SidePattern::parse`] as `Err(String)`.
///
/// `#[non_exhaustive]` so external callers must construct via
/// [`SidePattern::parse`] — that's the one place the empty-fill
/// invariant (and the future "no nested fill region" invariant)
/// is enforced. Internal construction inside this module is
/// still allowed and is what the parser uses.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
#[non_exhaustive]
pub enum SidePattern {
    /// `+=##=+` — repeat the whole cluster sequence atomically.
    /// The cluster vector preserves grapheme boundaries so the
    /// fitter can speak in cluster columns rather than bytes.
    AtomicRepeat { cluster: Vec<String> },
    /// `prefix(fill)suffix` — fixed ends, repeating fill in
    /// between. Each `Vec<String>` is a list of grapheme clusters,
    /// already escape-resolved. The parser guarantees `fill` is
    /// non-empty (an empty fill region errors with `"empty fill
    /// region"`) and that there is exactly one fill region —
    /// these invariants are not enforced by the type system,
    /// hence `#[non_exhaustive]` on the enum.
    PrefixFillSuffix {
        prefix: Vec<String>,
        fill: Vec<String>,
        suffix: Vec<String>,
    },
}

/// Output of [`SidePattern::render`]. Plain data carrier; cheap
/// to move (`text` is one allocation sized to the cluster total,
/// `cluster_count` is `Copy`).
///
/// `text` is the concatenated grapheme clusters, ready to push
/// onto a [`crate::gfx_structs::area::GlyphArea`]'s `text` field
/// or a cosmic-text buffer. `cluster_count` is the cluster
/// length of `text` — palette-cycling callers use it to size
/// their `ColorFontRegions` without re-walking grapheme
/// boundaries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedSide {
    /// Concatenated grapheme clusters, ready for layout.
    pub text: String,
    /// Cluster count of `text`. Equals
    /// `grapheme_chad::count_grapheme_clusters(text)` by
    /// construction; carried inline so callers don't need to
    /// re-walk the string.
    pub cluster_count: usize,
}

impl SidePattern {
    /// Parse a side pattern. Errors are user-facing strings —
    /// the console verb surfaces them verbatim.
    ///
    /// Empty input parses as an `AtomicRepeat` with no clusters,
    /// which renders to an empty string at any width. That's the
    /// least-surprising no-op default and lets tests / callers
    /// build a "no side here" pattern without a separate variant.
    pub fn parse(s: &str) -> Result<Self, String> {
        // First pass: walk chars, tracking the fill-region
        // delimiters and the escape state. Two output buffers
        // (`outside`, `inside`) plus a flag for "have we seen a
        // fill region yet". Errors out on a second `(`, an
        // unmatched `)`, or an unrecognized escape.
        let mut outside = String::new();
        let mut inside = String::new();
        let mut have_seen_fill = false;
        let mut in_fill = false;
        let mut fill_closed = false;
        let mut suffix_start: Option<usize> = None;
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    let next = chars
                        .next()
                        .ok_or_else(|| "trailing '\\' (use \\\\ for a literal backslash)".to_string())?;
                    let resolved = match next {
                        '(' => '(',
                        ')' => ')',
                        '\\' => '\\',
                        other => {
                            return Err(format!("unrecognized escape '\\{}' (use \\(, \\), \\\\)", other));
                        }
                    };
                    if in_fill {
                        inside.push(resolved);
                    } else {
                        outside.push(resolved);
                    }
                }
                '(' => {
                    if in_fill {
                        return Err("nested '(' inside fill region; \
                             escape with \\( for a literal"
                            .to_string());
                    }
                    if have_seen_fill {
                        return Err("only one fill region per side; saw 2".to_string());
                    }
                    have_seen_fill = true;
                    in_fill = true;
                    suffix_start = Some(outside.len());
                }
                ')' => {
                    if !in_fill {
                        return Err("unmatched ')'; escape with \\) for a literal".to_string());
                    }
                    in_fill = false;
                    fill_closed = true;
                }
                other => {
                    if in_fill {
                        inside.push(other);
                    } else {
                        outside.push(other);
                    }
                }
            }
        }

        if in_fill {
            return Err("missing ')' to close fill region".to_string());
        }

        if !have_seen_fill {
            // No fill region — the entire input is one atomic
            // cluster sequence to repeat.
            return Ok(SidePattern::AtomicRepeat {
                cluster: split_graphemes_owned(&outside),
            });
        }

        // Only reachable when have_seen_fill && fill_closed.
        debug_assert!(fill_closed);
        let split = suffix_start.expect("suffix_start set when fill opened");
        let prefix_str: String = outside[..split].to_string();
        let suffix_str: String = outside[split..].to_string();
        if inside.is_empty() {
            return Err("empty fill region".to_string());
        }

        Ok(SidePattern::PrefixFillSuffix {
            prefix: split_graphemes_owned(&prefix_str),
            fill: split_graphemes_owned(&inside),
            suffix: split_graphemes_owned(&suffix_str),
        })
    }

    /// Render the pattern at exactly `cluster_width` cluster
    /// columns. Always succeeds; the fitter chooses the largest N
    /// that fits without splitting an atomic unit. When even the
    /// statics don't fit the requested width, the output is
    /// truncated to whole clusters from the start of the static
    /// sequence — every render path that calls this should also
    /// have called [`Self::minimum_cluster_width`] against
    /// auto-resize so the truncation is unreachable in practice.
    ///
    /// # Cost
    ///
    /// O(`cluster_width`) cluster pushes plus one `String`
    /// allocation sized to the rendered byte length. Both hot
    /// border-rebuild paths (the renderer's
    /// the section-frame tree and the border tree's
    /// `build_border_mutator_tree_from_nodes`) call this once per
    /// side per visible node per frame, so the per-glyph push has
    /// to stay branchless — no parser work happens here.
    pub fn render(&self, cluster_width: usize) -> RenderedSide {
        match self {
            SidePattern::AtomicRepeat { cluster } => {
                let unit = cluster.len();
                if unit == 0 || cluster_width == 0 {
                    return RenderedSide {
                        text: String::new(),
                        cluster_count: 0,
                    };
                }
                let copies = cluster_width / unit;
                let mut text = String::with_capacity(unit * copies * 2);
                for _ in 0..copies {
                    for g in cluster {
                        text.push_str(g);
                    }
                }
                RenderedSide {
                    text,
                    cluster_count: copies * unit,
                }
            }
            SidePattern::PrefixFillSuffix { prefix, fill, suffix } => {
                let static_total = prefix.len() + suffix.len();
                if cluster_width < static_total {
                    // Static parts don't fit — emit as much of the
                    // prefix as we can, then as much of the suffix
                    // as still fits. Defensive guard; auto-resize
                    // should make this unreachable.
                    let mut text = String::new();
                    let take_prefix = cluster_width.min(prefix.len());
                    for g in &prefix[..take_prefix] {
                        text.push_str(g);
                    }
                    let leftover = cluster_width.saturating_sub(take_prefix);
                    let take_suffix = leftover.min(suffix.len());
                    let suffix_start = suffix.len() - take_suffix;
                    for g in &suffix[suffix_start..] {
                        text.push_str(g);
                    }
                    return RenderedSide {
                        text,
                        cluster_count: take_prefix + take_suffix,
                    };
                }
                let between = cluster_width - static_total;
                let copies = if fill.is_empty() { 0 } else { between / fill.len() };
                let cluster_count = static_total + copies * fill.len();
                let mut text = String::with_capacity(cluster_count * 2);
                for g in prefix {
                    text.push_str(g);
                }
                for _ in 0..copies {
                    for g in fill {
                        text.push_str(g);
                    }
                }
                for g in suffix {
                    text.push_str(g);
                }
                RenderedSide { text, cluster_count }
            }
        }
    }

    /// Smallest cluster width this pattern needs to render at all
    /// without dropping its static parts. The auto-resize path
    /// uses this as a hard floor.
    ///
    /// - `AtomicRepeat`: `cluster.len()` — one whole copy.
    /// - `PrefixFillSuffix`: `prefix.len() + suffix.len()` — zero
    ///   fill iterations is allowed.
    pub fn minimum_cluster_width(&self) -> usize {
        match self {
            SidePattern::AtomicRepeat { cluster } => cluster.len(),
            SidePattern::PrefixFillSuffix { prefix, suffix, .. } => prefix.len() + suffix.len(),
        }
    }

    /// Smallest cluster width that includes at least one full fill
    /// iteration on a `PrefixFillSuffix` pattern. For
    /// `AtomicRepeat`: same as [`Self::minimum_cluster_width`].
    /// The auto-resize path uses this as a *soft* target so a
    /// short node still shows the fill once.
    pub fn minimum_with_one_fill(&self) -> usize {
        match self {
            SidePattern::AtomicRepeat { cluster } => cluster.len(),
            SidePattern::PrefixFillSuffix { prefix, fill, suffix } => {
                prefix.len() + fill.len() + suffix.len()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_yields_empty_atomic_repeat() {
        let p = SidePattern::parse("").expect("empty parses");
        assert_eq!(p, SidePattern::AtomicRepeat { cluster: Vec::new() });
    }

    #[test]
    fn parse_atomic_repeat_no_parens() {
        let p = SidePattern::parse("+=##=+").expect("parses");
        match p {
            SidePattern::AtomicRepeat { cluster } => {
                assert_eq!(cluster, vec!["+", "=", "#", "#", "=", "+"]);
            }
            _ => panic!("expected AtomicRepeat"),
        }
    }

    #[test]
    fn parse_prefix_fill_suffix_basic() {
        let p = SidePattern::parse("###(*)###").expect("parses");
        match p {
            SidePattern::PrefixFillSuffix { prefix, fill, suffix } => {
                assert_eq!(prefix, vec!["#", "#", "#"]);
                assert_eq!(fill, vec!["*"]);
                assert_eq!(suffix, vec!["#", "#", "#"]);
            }
            _ => panic!("expected PrefixFillSuffix"),
        }
    }

    #[test]
    fn parse_escapes_in_fill() {
        // The user's literal example: outer pattern surrounds a
        // fill that itself contains escaped parens.
        let p = SidePattern::parse(r"+=#(\(\))#=+").expect("parses");
        match p {
            SidePattern::PrefixFillSuffix { prefix, fill, suffix } => {
                assert_eq!(prefix, vec!["+", "=", "#"]);
                assert_eq!(fill, vec!["(", ")"]);
                assert_eq!(suffix, vec!["#", "=", "+"]);
            }
            _ => panic!("expected PrefixFillSuffix"),
        }
    }

    #[test]
    fn parse_escapes_in_static() {
        // A literal `(` outside the fill region — escape it.
        let p = SidePattern::parse(r"\(").expect("parses");
        match p {
            SidePattern::AtomicRepeat { cluster } => {
                assert_eq!(cluster, vec!["("]);
            }
            _ => panic!("expected AtomicRepeat"),
        }
    }

    #[test]
    fn parse_escape_backslash() {
        let p = SidePattern::parse(r"\\").expect("parses");
        match p {
            SidePattern::AtomicRepeat { cluster } => {
                assert_eq!(cluster, vec!["\\"]);
            }
            _ => panic!("expected AtomicRepeat"),
        }
    }

    #[test]
    fn parse_unrecognized_escape_errors() {
        let err = SidePattern::parse(r"\X").expect_err("unrecognized escape errors");
        assert!(err.contains("unrecognized escape"));
        assert!(err.contains(r"\\"));
    }

    #[test]
    fn parse_trailing_backslash_errors() {
        let err = SidePattern::parse("end\\").expect_err("trailing backslash errors");
        assert!(err.contains("trailing"));
    }

    #[test]
    fn parse_two_fill_regions_errors() {
        let err = SidePattern::parse("a(b)c(d)e").expect_err("two fills error");
        assert!(err.contains("only one fill region"));
    }

    #[test]
    fn parse_unbalanced_open_errors() {
        let err = SidePattern::parse("a(b").expect_err("unbalanced ( errors");
        assert!(err.contains("missing ')'"));
    }

    #[test]
    fn parse_unbalanced_close_errors() {
        let err = SidePattern::parse("a)b").expect_err("unbalanced ) errors");
        assert!(err.contains("unmatched ')'"));
    }

    #[test]
    fn parse_empty_fill_errors() {
        let err = SidePattern::parse("pre()suf").expect_err("empty fill errors");
        assert!(err.contains("empty fill"));
    }

    #[test]
    fn parse_grapheme_clusters_combining_marks() {
        // Each of `é` (single codepoint), `🇺🇸` (regional indicator
        // pair = one ZWJ-like grapheme), and `é` again should count
        // as one cluster — 3 clusters total in the prefix.
        let p = SidePattern::parse("é🇺🇸é(👍)é🇺🇸é").expect("parses");
        match p {
            SidePattern::PrefixFillSuffix { prefix, fill, suffix } => {
                assert_eq!(prefix.len(), 3);
                assert_eq!(fill.len(), 1);
                assert_eq!(suffix.len(), 3);
            }
            _ => panic!("expected PrefixFillSuffix"),
        }
    }

    #[test]
    fn render_atomic_repeat_fits_three_iterations() {
        let p = SidePattern::parse("+=##=+").expect("parses");
        let r = p.render(18);
        assert_eq!(r.cluster_count, 18);
        assert_eq!(r.text, "+=##=++=##=++=##=+");
    }

    #[test]
    fn render_atomic_repeat_partial_truncated() {
        // Width 8, cluster 6 → only one whole iteration fits.
        let p = SidePattern::parse("+=##=+").expect("parses");
        let r = p.render(8);
        assert_eq!(r.cluster_count, 6);
        assert_eq!(r.text, "+=##=+");
    }

    #[test]
    fn render_prefix_fill_suffix_zero_iterations_when_only_statics_fit() {
        // Width 6, prefix 3 + suffix 3 + fill 1 → 0 iterations.
        let p = SidePattern::parse("###(*)###").expect("parses");
        let r = p.render(6);
        assert_eq!(r.cluster_count, 6);
        assert_eq!(r.text, "######");
    }

    #[test]
    fn render_prefix_fill_suffix_atomic_fill_drops_partial_iteration() {
        // Width 7, statics = 6, fill = 2 (`()`). One iteration is
        // 2 clusters; a single column wouldn't accommodate it, so
        // 0 iterations.
        let p = SidePattern::parse(r"###(\(\))###").expect("parses");
        let r = p.render(7);
        assert_eq!(r.cluster_count, 6);
        assert_eq!(r.text, "######");
    }

    #[test]
    fn render_prefix_fill_suffix_three_iterations() {
        // Width 12, prefix 3 + suffix 3 + fill 2 → 3 fill iters.
        let p = SidePattern::parse(r"###(\(\))###").expect("parses");
        let r = p.render(12);
        assert_eq!(r.cluster_count, 12);
        assert_eq!(r.text, "###()()()###");
    }

    #[test]
    fn render_below_static_floor_truncates_defensively() {
        // Width 4 against statics totalling 6 — the fitter
        // truncates to whole clusters of (prefix-prefix-prefix +
        // suffix-suffix). Auto-resize is supposed to make this
        // unreachable; this asserts the defensive behavior.
        let p = SidePattern::parse("###(*)###").expect("parses");
        let r = p.render(4);
        // 3 prefix + 1 suffix at the right.
        assert_eq!(r.cluster_count, 4);
        assert_eq!(r.text, "####");
    }

    #[test]
    fn render_atomic_repeat_with_combining_marks() {
        let p = SidePattern::parse("éé").expect("parses");
        let r = p.render(4);
        assert_eq!(r.cluster_count, 4);
        assert_eq!(r.text, "éééé");
    }

    #[test]
    fn render_zero_width_yields_empty() {
        let p = SidePattern::parse("###(*)###").expect("parses");
        let r = p.render(0);
        assert_eq!(r.cluster_count, 0);
        assert_eq!(r.text, "");
    }

    #[test]
    fn minimum_cluster_width_atomic() {
        let p = SidePattern::parse("+=##=+").expect("parses");
        assert_eq!(p.minimum_cluster_width(), 6);
        assert_eq!(p.minimum_with_one_fill(), 6);
    }

    #[test]
    fn minimum_cluster_width_prefix_fill_suffix() {
        let p = SidePattern::parse("###(*)###").expect("parses");
        assert_eq!(p.minimum_cluster_width(), 6);
        assert_eq!(p.minimum_with_one_fill(), 7);
    }

    #[test]
    fn minimum_with_one_fill_long_fill() {
        let p = SidePattern::parse(r"+=#(\(\))#=+").expect("parses");
        // statics = 6, one fill iteration = 2.
        assert_eq!(p.minimum_cluster_width(), 6);
        assert_eq!(p.minimum_with_one_fill(), 8);
    }

    /// `()` (immediately-closed fill region) errors with the
    /// dedicated empty-fill message, not with an "unmatched (" or
    /// trailing-input lurker. Guards the line-182 branch.
    #[test]
    fn parse_open_immediately_closed_errors() {
        let err = SidePattern::parse("()").expect_err("immediately-closed fill errors");
        assert!(
            err.contains("empty fill"),
            "expected empty-fill diagnostic, got: {}",
            err
        );
    }

    /// `(\)` parses as a fill containing a literal `)`. Guards
    /// the case where the fill-region's only content is an
    /// escaped close-paren — easy to mis-handle as "unmatched".
    #[test]
    fn parse_fill_of_only_escaped_close() {
        let p = SidePattern::parse(r"(\))").expect("parses");
        match p {
            SidePattern::PrefixFillSuffix { prefix, fill, suffix } => {
                assert!(prefix.is_empty());
                assert_eq!(fill, vec![")"]);
                assert!(suffix.is_empty());
            }
            other => panic!("expected PrefixFillSuffix, got {:?}", other),
        }
    }

    /// ZWJ-joined emoji as a corner-equivalent atomic-repeat
    /// (corners use `SidePattern::parse` too via the console verb's
    /// `stage_corner_or_err`). Confirms grapheme-cluster awareness
    /// holds for multi-codepoint clusters that the renderer will
    /// shape as a single visual glyph.
    #[test]
    fn parse_zwj_emoji_as_single_cluster() {
        // 👨‍👩‍👧 — five codepoints joined by ZWJ, one cluster.
        let p = SidePattern::parse("\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}").expect("parses");
        match p {
            SidePattern::AtomicRepeat { cluster } => {
                assert_eq!(cluster.len(), 1, "ZWJ-joined emoji must count as one cluster");
            }
            other => panic!("expected AtomicRepeat, got {:?}", other),
        }
    }

    /// Round-trip: parse a simple atomic-repeat string, then
    /// `render` it at exactly its own cluster length, and confirm
    /// the rendered text equals the input. Catches a regression
    /// where atomic-repeat's render somehow re-orders or corrupts
    /// clusters.
    #[test]
    fn parse_render_identity_atomic_repeat() {
        let input = "+=##=+";
        let p = SidePattern::parse(input).expect("parses");
        let r = p.render(6);
        assert_eq!(r.text, input);
        assert_eq!(r.cluster_count, 6);
    }

    /// Very large render width — guards against accidental
    /// quadratic / overflow regressions on the fitter.
    #[test]
    fn render_atomic_repeat_at_very_large_width() {
        let p = SidePattern::parse("ab").expect("parses");
        let r = p.render(20_000);
        assert_eq!(r.cluster_count, 20_000);
        // Spot-check first / last bytes; full string equality
        // would allocate 20k * 2 bytes for the comparison.
        assert!(r.text.starts_with("ab"));
        assert!(r.text.ends_with("ab"));
    }
}
