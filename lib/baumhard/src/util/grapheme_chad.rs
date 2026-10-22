// SPDX-License-Identifier: MPL-2.0

//! Grapheme-cluster aware text utilities. Mandala's text editing,
//! console scrollback, and label rendering all flow through these
//! helpers so emoji, combining marks, and CJK glyphs round-trip
//! correctly. Reach for these primitives from the application crate
//! rather than indexing a `String` by byte offset — the latter
//! splits a 👨‍👩‍👧 ZWJ sequence on the first edit and the rest of the
//! pipeline silently corrupts.
//!
//! # The line model
//!
//! Several of these helpers answer *where does line `n` begin and
//! end*, and callers pair them: [`count_number_lines`] bounds the
//! loop that [`find_nth_line_grapheme_range`] indexes, and
//! [`line_bounds_at`] reports the same span from a cursor sitting
//! inside it. They therefore have to mean the same thing by "line",
//! and this is what they mean:
//!
//! - **A line terminator is a grapheme cluster ending in `\n`.**
//!   UAX #29 fuses `\r\n` into one cluster, so a CRLF is a single
//!   terminator and its CR belongs to the terminator rather than to
//!   the line in front of it. A CR *not* followed by an LF
//!   terminates nothing and stays in the line as ordinary content —
//!   line 0 of `"a\r\r\nb"` is `"a\r"`.
//! - **A terminator ends a line; it does not start one.** A buffer
//!   ending in a terminator therefore has an empty final line:
//!   `"abc\n"` is two lines and line 1 is the empty span at the end
//!   of the buffer. The empty string is one empty line.
//! - **Every line the count counts is addressable, and only those.**
//!   `find_nth_line_grapheme_range(s, n).is_some()` is exactly
//!   `n < count_number_lines(s)`, so `0..count_number_lines(s)` is a
//!   loop whose every index resolves (issue #16).
//!
//! The crate-private `slice_to_newline` is that same model expressed
//! in byte offsets instead of cluster indices — it is what
//! [`replace_graphemes_until_newline`] writes through, and the one
//! member that has to exclude the CRLF's CR by hand. Each function's
//! own doc carries the detail and the costs; the crate's
//! `do_line_model_is_coherent` test holds all four to this.
//!
//! [`count_number_lines`]: crate::util::grapheme_chad::count_number_lines
//! [`find_nth_line_grapheme_range`]: crate::util::grapheme_chad::find_nth_line_grapheme_range
//! [`line_bounds_at`]: crate::util::grapheme_chad::line_bounds_at
//! [`replace_graphemes_until_newline`]: crate::util::grapheme_chad::replace_graphemes_until_newline

// The four link targets above are spelled out in full rather than
// left to shorthand resolution. `util/mod.rs` carries an outer `///`
// on this module's `pub mod` line, and rustdoc merges that fragment
// with this header and resolves the whole of it in `util`'s scope,
// where these names are not visible. Reference-style definitions
// render as nothing, so the header above reads as if they were not
// there.

use log::error;
use unicode_segmentation::UnicodeSegmentation;

/// Borrow the slice from `byte_index` up to (not including) the next
/// line terminator, or to the end of `s` if no terminator follows.
///
/// The terminator is a cluster ending in `\n` — the same rule
/// [`line_bounds_at`] and [`find_nth_line_grapheme_range`] apply, and
/// this helper is the third of the three that answer *where does the
/// line end* (distinct from the four-helper line model, which adds
/// [`count_number_lines`]): it is what
/// [`replace_graphemes_until_newline`] uses to decide how much of a
/// line it may overwrite. Under UAX #29 `\r\n` is a single cluster, so
/// cutting at the raw `\n` byte would leave the CR inside the returned
/// slice — a slice ending mid-cluster, whose caller then splices the
/// CR away and turns a Windows line ending into a Unix one. A `\r`
/// immediately before the `\n` is therefore excluded too. A CR that is
/// *not* followed by `\n` is not a terminator and stays in the line.
///
/// `byte_index` must land on a UTF-8 char boundary; passing a
/// mid-codepoint byte panics like any other `String` slice. A
/// `byte_index` that lands mid-*cluster* — on the `\n` of a CRLF, say
/// — is outside the contract but yields an empty slice rather than an
/// inverted range.
///
/// Cost: O(n) on the search distance for the `\n`, plus one byte of
/// lookbehind. No allocation.
pub(crate) fn slice_to_newline(s: &str, byte_index: usize) -> &str {
    let end_byte_index = match s[byte_index..].find('\n') {
        Some(i) => {
            let nl = byte_index + i;
            // `\r` is one byte, so the CR of a CRLF starts at `nl - 1`.
            // The `> byte_index` guard keeps a caller that pointed at
            // the LF itself from producing an inverted range.
            if nl > byte_index && s.as_bytes()[nl - 1] == b'\r' {
                nl - 1
            } else {
                nl
            }
        }
        None => s.len(),
    };

    &s[byte_index..end_byte_index]
}

/// What a [`replace_graphemes_until_newline`] call did to its target.
///
/// A caller that keeps grapheme-indexed side tables next to the buffer
/// — `ColorFontRegions` spans, or `GlyphMatrix`'s "line N of the
/// target" row bookkeeping — needs three facts to stay in step with
/// the edit, and the previous `Option<(usize, usize)>` return could
/// only carry two of them. In particular it could not say that the
/// target had *gained lines*, which is why a multi-line source used
/// to reflow the buffer silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LineReplacement {
    /// Grapheme index the replacement actually started at, so a
    /// region shift can be applied straight from this value without
    /// threading the index separately. Normally the caller's
    /// `g_index`; when that index is past the end of the target the
    /// write clamps to the end of the buffer and this clamps with
    /// it — shifting from the caller's index instead would step over
    /// every region between the real end and it.
    pub at: usize,
    /// Net change in the target's grapheme-cluster count — the
    /// buffer's *measured* delta, not `source clusters - overwritten
    /// clusters` arithmetic. Shifting every region that starts after
    /// `at` right by `growth` keeps whole-buffer grapheme indices
    /// correct.
    ///
    /// The distinction is not academic and it is why this is signed:
    /// UAX #29 segmentation is not compositional across a splice
    /// seam, so a source whose first scalar extends a cluster
    /// (a combining mark, a ZWJ continuation, an LF after a CR, a
    /// regional indicator completing a pair) *fuses* with the
    /// character before it. Splicing `"\r"` in front of a `"\n"`
    /// leaves one cluster where the arithmetic predicts two, and
    /// overwriting the one-cluster tail of `"ab"` with a lone
    /// `U+0301` leaves one cluster where there were two — a
    /// **negative** growth, which the caller answers with
    /// `ColorFontRegions::shrink_regions_after` rather than one of the
    /// grow-side primitives.
    pub growth: isize,
    /// One past the last cluster of the *post-edit* target whose base
    /// scalar came from `source`. Together with [`Self::at`] it is the
    /// exact grapheme span the spliced text occupies — `at ..
    /// written_end` — so a caller that pins a `ColorFontRegion` to the
    /// text it just wrote can take the span straight from the
    /// measurement instead of adding up cluster counts that were taken
    /// in isolation.
    ///
    /// Isolated arithmetic is wrong for the same reason [`Self::growth`]
    /// is measured: across a splice seam the source's clusters are not
    /// the source's clusters any more. The rule this field applies is
    /// **a cluster belongs to whichever text contributed its first
    /// scalar**:
    ///
    /// - A source that merely *extends* the cluster before it (a lone
    ///   combining mark, an LF landing after a CR) owns no cell of its
    ///   own, and `written_end == at` — an empty span. The fused cell
    ///   keeps the identity of the character whose base it is.
    /// - A source whose last cluster is extended by the text *after* it
    ///   keeps that shared cell, because its own scalar is the base.
    ///
    /// `at <= written_end <= ` the post-edit cluster count, always, so a
    /// span built from this field can never point past the buffer.
    pub written_end: usize,
    /// How many extra lines the target gained, i.e. the number of
    /// `\n` characters `source` contributed. Non-zero means every
    /// line of the target *after* the edited one has moved down by
    /// this much, and a caller that addresses the target by line
    /// number must compensate.
    pub added_lines: usize,
}

/// Replace `target`'s graphemes from `g_index` up to (and not past)
/// the next newline **in `target`** with the graphemes in `source`.
/// If `source` is longer than the existing line tail, the extras are
/// appended; if shorter, the surplus tail beyond `source`'s length is
/// preserved (only the overlapping prefix is overwritten).
///
/// The newline bound applies to the *target only*. `source` is
/// inserted whole, newlines and all. The doc used to claim the helper
/// "stops at the first `\n` in either string"; it never did, and the
/// old `Option<(usize, usize)>` return had no way to tell the caller
/// that the buffer had grown a line. A multi-line `source` therefore
/// **reflows** `target`: the surviving tail of the edited line ends up
/// on the source's last line, and every following line moves down.
/// [`LineReplacement::added_lines`] reports exactly that drift, so a
/// line-addressed caller such as `GlyphMatrix::place_in` can
/// compensate instead of painting its next row into the middle of the
/// text it just inserted.
///
/// The line bound is the same one every other line helper in this
/// module uses: a cluster ending in `\n` terminates the line, and a
/// CRLF is one such cluster whose CR belongs to the terminator. The
/// tail this function may overwrite therefore stops **before** the CR,
/// and a Windows line ending survives being painted over. The bound
/// itself is computed by the crate-private `slice_to_newline`.
///
/// Returns a [`LineReplacement`] describing the edit.
/// [`LineReplacement::growth`] is the buffer's measured
/// grapheme-cluster delta and may be negative; see its doc for why
/// arithmetic on the two cluster counts is not the same thing.
/// [`LineReplacement::at`] and [`LineReplacement::written_end`] are
/// likewise measured, and bound the span the source actually occupies
/// in the post-edit buffer — a caller pinning a region to the text it
/// wrote must use them rather than adding up the source's clusters,
/// which are counted in isolation and stop being true across the seam.
///
/// # Costs
///
/// Two whole-buffer grapheme walks — one before the splice, one after —
/// because `growth` is measured rather than computed and UAX #29 gives
/// no cheaper exact answer across a splice seam (a regional-indicator
/// run makes the required lookbehind unbounded). The second walk
/// yields `written_end` at the same time, so the span costs nothing on
/// top of the delta. Debug builds pay one further walk for the
/// `at`-consistency `debug_assert`. On top of that: one bounded `find_byte_index_of_grapheme`
/// walk for the write position, one `count_grapheme_clusters` of the
/// line tail, one `count_number_lines` byte scan over `source`, and one
/// crate-private `replace_substring`, which splices in place. Every
/// term is O(target length); the function was already in that class
/// before the measurement was added.
pub fn replace_graphemes_until_newline(target: &mut String, g_index: usize, source: &str) -> LineReplacement {
    let insert_num_graphemes = count_grapheme_clusters(source);
    // `count_number_lines` is "newlines + 1", so the count of `\n`
    // characters the source contributes is one less.
    let added_lines = count_number_lines(source) - 1;
    let clusters_before = count_grapheme_clusters(target);
    // A `g_index` past the end of the target clamps the write to the
    // end of the buffer, so the position we report has to clamp with
    // it (see [`LineReplacement::at`]).
    let (b_index, at) = match find_byte_index_of_grapheme(target, g_index) {
        Some(b) => (b, g_index),
        None => (target.len(), clusters_before),
    };

    let line_section = slice_to_newline(target, b_index);

    let target_line_num_graphemes = count_grapheme_clusters(line_section);
    let end_of_target_line_idx = b_index + line_section.len();

    if insert_num_graphemes >= target_line_num_graphemes {
        // The source covers the whole line tail: cut the tail away
        // (never the terminator — `slice_to_newline` stops short of
        // the whole cluster, CR included) and splice the source in.
        replace_substring(target, b_index, end_of_target_line_idx, source);
    } else {
        // The source is shorter than the line tail: overwrite only
        // the overlapping prefix and leave the surplus in place.
        // The bound is in range by construction (`g_index +
        // insert_num_graphemes` is strictly inside the line — this
        // branch only runs when `g_index` itself resolved), but fall
        // back to the line end rather than panic in a text-edit hot
        // path.
        let overlap_end = find_byte_index_of_grapheme(target, g_index + insert_num_graphemes)
            .unwrap_or(end_of_target_line_idx);
        replace_substring(target, b_index, overlap_end, source);
    }

    // One post-splice walk answers both measured questions: the new
    // cluster count (for `growth`) and how many clusters start before
    // the end of the text we just wrote (for `written_end`).
    let (clusters_after, written_end) = count_clusters_and_starts_before(target, b_index + source.len());

    debug_assert_eq!(
        at,
        count_clusters_and_starts_before(target, b_index).1,
        "`at` must equal the post-edit count of clusters starting before the write position"
    );

    LineReplacement {
        at,
        // Measured, not derived: see `LineReplacement::growth`.
        growth: clusters_after as isize - clusters_before as isize,
        written_end,
        added_lines,
    }
}

/// Total grapheme-cluster count of `s`, and how many of those clusters
/// *start* strictly before `byte_limit`, in one walk.
///
/// The second number is the span arithmetic `replace_graphemes_until_newline`
/// needs on both ends of a splice: counted at the write position it is
/// the first cluster the source can claim, and counted at the end of
/// the written bytes it is one past the last. Counting cluster *starts*
/// rather than clusters *contained* is what implements the
/// "a cluster belongs to whichever text contributed its first scalar"
/// rule documented on [`LineReplacement::written_end`]. A `byte_limit`
/// past the end of `s` simply yields the total.
///
/// Cost: one O(n) grapheme walk. No allocation.
fn count_clusters_and_starts_before(s: &str, byte_limit: usize) -> (usize, usize) {
    let mut total = 0;
    let mut starts_before = 0;
    let mut byte = 0;
    for g in s.graphemes(true) {
        if byte < byte_limit {
            starts_before += 1;
        }
        byte += g.len();
        total += 1;
    }
    (total, starts_before)
}

/// Return the byte offset of the `index`-th grapheme cluster in `s`.
/// Returns `None` if `index` is out of bounds. O(n) over
/// `s.graphemes(true)`. This is the grapheme-correct counterpart to
/// `char_indices().nth(index)`.
pub fn find_byte_index_of_grapheme(s: &str, index: usize) -> Option<usize> {
    let mut byte_index = 0;
    for (i, grapheme) in s.graphemes(true).enumerate() {
        if i == index {
            return Some(byte_index);
        }
        byte_index += grapheme.len();
    }
    None
}

/// Resolve a whole *table* of grapheme-cluster indices to byte
/// offsets in one left-to-right walk.
///
/// Each entry of the result answers the same question
/// [`find_byte_index_of_grapheme`] does for a single index:
/// `Some(byte_offset)` when a cluster exists at that index, `None`
/// when the index is at or past the end of `s`.
///
/// **Why this exists.** Converting a run table — text runs, color /
/// font regions — one boundary at a time restarts the grapheme walk
/// per boundary, so the conversion costs O(len × boundaries). That
/// is quadratic whenever the run table scales with the text, which
/// a `.mindmap.json` is free to make it do: a section carrying N
/// clusters may legally carry N non-overlapping runs over them.
/// This pays for the walk once instead.
///
/// `ascending` is expected sorted non-decreasing, which is what the
/// format's run ordering already guarantees
/// (`format/text-runs.md`, enforced at load by
/// `mindmap::model::validate`). An out-of-order index is still
/// answered correctly — it falls back to an independent walk for
/// that entry — so a caller that breaks the expectation gets a slow
/// answer rather than a wrong one.
///
/// Cost: O(len(s) + ascending.len()) for sorted input, plus one
/// `Vec` of the returned offsets. An out-of-order entry adds one
/// O(len(s)) walk for itself.
pub fn byte_indices_of_graphemes(s: &str, ascending: &[usize]) -> Vec<Option<usize>> {
    let mut out = Vec::with_capacity(ascending.len());
    let mut clusters = s.graphemes(true).peekable();
    let mut index = 0usize;
    let mut byte_index = 0usize;

    for &target in ascending {
        if target < index {
            out.push(find_byte_index_of_grapheme(s, target));
            continue;
        }
        while index < target {
            match clusters.next() {
                Some(cluster) => {
                    byte_index += cluster.len();
                    index += 1;
                }
                None => break,
            }
        }
        // `peek` rather than position alone: an index exactly at the
        // cluster count has a byte offset but no cluster living
        // there, and the single-index helper reports that as `None`.
        if index == target && clusters.peek().is_some() {
            out.push(Some(byte_index));
        } else {
            out.push(None);
        }
    }
    out
}

/// Replace the bytes `i..n` of `s` with `source`, in place.
///
/// `i` and `n` are **byte** offsets. The contract is that both land
/// on a UTF-8 character boundary of `s`, with `i <= n <= s.len()`.
///
/// **The two production call sites satisfy it by construction** —
/// the two branches of [`replace_graphemes_until_newline`], in this
/// module. `i` comes from [`find_byte_index_of_grapheme`] (a
/// cumulative sum of cluster lengths, and a cluster boundary is
/// always a character boundary) or from `s.len()`; `n` is either
/// another such cluster offset or the end of `slice_to_newline`'s
/// borrowed subslice of `s`, which is a character boundary because it
/// is the end of a `&str`. `s` is not mutated between computing them
/// and arriving here, and `i <= n` holds because cluster offsets rise
/// with cluster index.
///
/// **That is an argument about two callers, not about this
/// function.** It is `pub(crate)`, so the precondition is not
/// enclosed by this module: the crate's own tests reach it, and four
/// of their call sites pass out-of-contract offsets *on purpose*,
/// because driving the degrade is what they exist to do. Any future
/// in-crate caller reaches it on the same terms. The guard below is
/// therefore the contract's enforcement rather than a formality
/// about today's two production callers, and the argument above is
/// what says those two never reach it.
///
/// **Out of contract it degrades rather than panics.**
/// `String::replace_range` has exactly three panic conditions — a
/// start off a character boundary, an end off one (which subsumes an
/// end past the length, since `is_char_boundary` is false there), and
/// a start after the end — and the check below covers all three. This
/// is the per-keystroke edit path, where CODE_CONVENTIONS §9 does not
/// allow a panic, so a bad triple logs and leaves `s` untouched
/// instead.
///
/// That check is **sharper** than the `String::from_utf8` it
/// replaces, not a weaker stand-in for it. The old body copied `s`
/// into a `Vec<u8>`, spliced the bytes, and re-validated the result
/// — a check that runs after the damage and that a mid-character cut
/// can pass: draining bytes `1..3` of `"éé"` leaves `[C3 A9]`, which
/// is valid UTF-8 and is the wrong string. `is_char_boundary`
/// refuses that same call before `s` is touched.
/// `do_replace_substring_refuses_a_mid_character_range` pins both
/// halves of that comparison.
///
/// Cost: two O(1) boundary probes and one `String::replace_range`,
/// which shifts the tail in place and grows the buffer only when
/// `source` is longer than the range it replaces. No copy of `s`, no
/// revalidation.
///
/// `pub(crate)` rather than private so the out-of-contract
/// regression test can drive the degrade.
pub(crate) fn replace_substring(s: &mut String, i: usize, n: usize, source: &str) {
    if i > n || !s.is_char_boundary(i) || !s.is_char_boundary(n) {
        error!(
            "grapheme_chad: replace_substring got byte range {i}..{n}, which is not a \
             character-boundary range of a {}-byte buffer; leaving it unedited",
            s.len()
        );
        return;
    }
    s.replace_range(i..n, source);
}

/// Grapheme-aware analogue of `String::split_off`. Splits `original`
/// at grapheme cluster index `at`, leaving the prefix in `original`
/// and returning the suffix as an owned `String`. If `at` reaches or
/// exceeds the grapheme count, returns an empty `String` and leaves
/// `original` unchanged.
///
/// Cost: one O(n) grapheme walk to find the split offset — short-
/// circuiting at cluster `at` — then one `String::split_off`. That is
/// a single allocation, for the returned suffix; `original` keeps its
/// own buffer and truncates in place. The implementation this
/// replaced collected every cluster into a `Vec<&str>` and rebuilt
/// *both* halves with `concat()`, so it allocated three times and
/// reassigned `original` to a fresh buffer.
pub fn split_off_graphemes(original: &mut String, at: usize) -> String {
    match find_byte_index_of_grapheme(original, at) {
        Some(byte_index) => original.split_off(byte_index),
        // `at` is at or past the cluster count, so there is no
        // cluster to split at and nothing to hand back.
        None => String::new(),
    }
}

/// Grapheme-cluster index of the first cluster in `s` that carries any
/// non-whitespace content, or `None` when `s` is empty or entirely
/// whitespace.
///
/// A cluster counts as whitespace only when *every* scalar in it is
/// `char::is_whitespace`, so a base character with a combining mark is
/// content even though the mark alone would not be.
///
/// This is the grapheme-unit answer to "where does the leading indent
/// end". The `char`-ordinal answer is a trap: it disagrees with the
/// byte offset the moment the indent contains a multi-byte space such
/// as U+3000 IDEOGRAPHIC SPACE, and it disagrees with the grapheme
/// offset the moment the text contains a ZWJ emoji or a combining
/// mark. Callers that then feed the result to a slice, a
/// [`split_off_graphemes`], and a cluster-indexed insert are mixing
/// three units (CONVENTIONS §B3).
///
/// Cost: O(n) grapheme walk, short-circuiting at the first content
/// cluster. No allocation.
pub fn first_non_whitespace_grapheme(s: &str) -> Option<usize> {
    s.graphemes(true)
        .position(|g| g.chars().any(|c| !c.is_whitespace()))
}

/// Number of newline-separated lines in `s`. The trailing line counts
/// even when `s` does not end in `\n`, so an empty string yields 1 and
/// `"abc\n"` yields 2 — the second line being the empty one after the
/// terminator.
///
/// That trailing line is addressable:
/// [`find_nth_line_grapheme_range`] resolves every index in
/// `0..count_number_lines(s)` and nothing outside it, so this count is
/// a valid bound for a line-addressing loop. It was not always — see
/// that function's doc for what the two disagreed about.
///
/// Cost: O(n) byte scan; no allocation.
pub fn count_number_lines(s: &str) -> usize {
    s.as_bytes().iter().filter(|&&c| c == b'\n').count() + 1
}

/// Grapheme-index bounds of the newline-separated line that contains
/// `cursor`, as a half-open `(line_start, line_end)` pair.
///
/// `line_start` is the index just past the most recent line
/// terminator strictly before `cursor` (0 when there is none).
/// `line_end` is the index of the first line terminator at or after
/// `cursor`, or `s`'s cluster count when no terminator follows.
/// Neither bound includes the terminator itself, so
/// `line_end - line_start` is the line's visible length in clusters
/// and `cursor` always satisfies `line_start <= cursor` for any
/// in-range cursor. A `cursor` past the cluster count yields the last
/// line's bounds; an empty `s` yields `(0, 0)`.
///
/// A "line terminator" is any cluster ending in `\n`, which covers
/// both a bare LF and a CRLF pair — UAX #29 fuses `\r\n` into a
/// single cluster, so testing `g == "\n"` would walk straight past a
/// Windows line ending and report one long line. The CR is treated as
/// part of the terminator and therefore falls outside both bounds.
///
/// This is the one implementation of "where does the cursor's line
/// begin and end" (CONVENTIONS §B3); the editor's Home / End /
/// up-line / down-line motions all route through it.
///
/// The line it reports is the same line
/// [`find_nth_line_grapheme_range`] addresses by index, including the
/// empty one a trailing terminator leaves behind: for every `i <
/// count_number_lines(s)` and every cursor inside that line's bounds,
/// the pair returned here equals the range returned there. `"abc\n"`
/// with the cursor at 4 is `(4, 4)`, the same span line 1 has. These
/// functions and the byte-level `slice_to_newline` are one line
/// model — see the module header — checked as one by
/// `do_line_model_is_coherent` in the crate's tests.
///
/// # Costs
///
/// One O(n) grapheme walk that stops at the line's terminator — so
/// O(distance to the end of the cursor's line), degrading to the
/// whole buffer only when the cursor sits on the last line. No
/// allocation.
pub fn line_bounds_at(s: &str, cursor: usize) -> (usize, usize) {
    let mut line_start = 0usize;
    let mut total = 0usize;
    for (i, g) in s.graphemes(true).enumerate() {
        if g.ends_with('\n') {
            if i < cursor {
                line_start = i + 1;
            } else {
                return (line_start, i);
            }
        }
        total = i + 1;
    }
    (line_start, total)
}

/// Grapheme-cluster span of the `n`-th newline-separated line in `s`,
/// returned as a half-open `(start_grapheme, end_grapheme)` range.
/// `n = 0` is the first line.
///
/// **Every line [`count_number_lines`] counts is addressable here**,
/// and only those: `find_nth_line_grapheme_range(s, n).is_some()` is
/// exactly `n < count_number_lines(s)`, for every `s` and every `n`.
/// A caller may therefore iterate `0..count_number_lines(s)` and rely
/// on each index resolving. The two used to disagree about the last
/// line of every newline-terminated string, and about the empty
/// string (issue #16 problem B): `count_number_lines` counts a
/// trailing empty line — `"abc\n"` is two lines, `""` is one — while
/// this finder answered `None` for it, so a `0..count` loop hit a
/// guaranteed `None` on its final iteration and a cursor parked on
/// the empty last line of a text area had no line to sit on. That
/// trailing line now answers with the empty range at the end of the
/// buffer: `"abc\n"` line 1 is `Some((4, 4))`, `""` line 0 is
/// `Some((0, 0))`. An empty range is the honest span of a line with
/// nothing on it, and the `("\n", 0) -> Some((0, 0))` fixture had
/// already set that precedent for the *leading* empty line.
///
/// `None` therefore means one thing only: `n` is past the last line,
/// i.e. the row does not exist. It never means "the row exists but is
/// empty" — a caller that wants *that* distinction compares the two
/// bounds, which costs it nothing.
///
/// A "line terminator" is any cluster ending in `\n` — the same rule
/// [`line_bounds_at`] applies, and for the same reason: UAX #29 fuses
/// `\r\n` into a single cluster, so a `g == "\n"` test walks straight
/// past a Windows line ending and folds two lines into one. The
/// terminator (CR included) falls outside the returned range. Counting
/// lines this way agrees with [`count_number_lines`], which counts
/// `\n` *characters* — a CRLF is one of each — so a caller that sizes
/// a buffer with one and addresses it with the other stays in step.
///
/// Three helpers in this module answer a "where does the line end"
/// question and all three apply that rule: this one,
/// [`line_bounds_at`], and the crate-private `slice_to_newline` that
/// [`replace_graphemes_until_newline`] writes through.
///
/// # Costs
///
/// One O(n) grapheme walk that stops at the requested line's
/// terminator — so O(distance to the end of line `n`), degrading to
/// the whole buffer only for the last line. No allocation. The walk
/// carries its own running cluster count, so the last-line case no
/// longer pays a second `s.graphemes(true).count()` pass on top of
/// it.
pub fn find_nth_line_grapheme_range(s: &str, n: usize) -> Option<(usize, usize)> {
    let mut line_start = 0usize;
    let mut line_index = 0usize;
    let mut total = 0usize;
    for (idx, graph) in s.graphemes(true).enumerate() {
        total = idx + 1;
        if graph.ends_with('\n') {
            if line_index == n {
                // We're at the end of the requested line: emit the
                // half-open range [line_start, idx).
                return Some((line_start, idx));
            }
            line_index += 1;
            line_start = idx + 1;
        }
    }
    // Whatever follows the last terminator is the final line, and it
    // exists whether or not any cluster landed on it — that is the
    // line `count_number_lines`'s `+ 1` counts. An `s` with no
    // terminator at all reaches here with `line_index == 0`, which is
    // why the empty string answers `Some((0, 0))` for line 0 without
    // a special case of its own.
    (line_index == n).then_some((line_start, total))
}

/// Append `n` newline characters to `s`. Convenience wrapper around
/// `str::repeat` + `push_str`; O(n) for the repeat allocation.
pub fn insert_new_lines(s: &mut String, n: usize) {
    let newlines = "\n".repeat(n);
    s.push_str(&newlines);
}

/// Append `n` spaces to `s`. O(n) allocation for the repeat string.
pub fn push_spaces(s: &mut String, n: usize) {
    let spaces = " ".repeat(n);
    s.push_str(&spaces);
}

/// Insert `n` spaces at grapheme-cluster index `idx`.
///
/// `idx` counts clusters, so the spaces always land on a cluster
/// boundary — inserting at index 1 of `"🙏🏻x"` goes after the whole
/// skin-toned emoji, not between its base and its modifier. At or
/// past the string's cluster count the spaces are appended, which
/// makes `idx == count` the natural "at the end" spelling. `n == 0`
/// is a no-op on the contents (the empty repeat is still inserted).
///
/// # Costs
///
/// O(n) `str::repeat` for the space run, an O(idx) grapheme walk to
/// find the boundary, and the O(len) `String::insert_str` shift of
/// everything after it.
pub fn insert_spaces(s: &mut String, idx: usize, n: usize) {
    let spaces = " ".repeat(n);
    match find_byte_index_of_grapheme(s, idx) {
        Some(byte_offset) => s.insert_str(byte_offset, &spaces),
        None => push_spaces(s, n),
    }
}

/// Insert `source` into `s` at grapheme-cluster index `idx`. If `idx`
/// equals or exceeds `s`'s grapheme count the source is appended.
///
/// Cost: O(n) over `s` to walk to the nth grapheme boundary, plus the
/// underlying `String::insert_str` shift. No allocation beyond the
/// string growth.
///
/// This is the grapheme-correct counterpart to `String::insert_str`,
/// and exists so caller code can stop reaching for `char_indices()` —
/// the latter splits emoji and combining marks mid-cluster.
pub fn insert_str_at_grapheme(s: &mut String, idx: usize, source: &str) {
    match find_byte_index_of_grapheme(s, idx) {
        Some(byte) => s.insert_str(byte, source),
        None => s.push_str(source),
    }
}

/// Insert `source` into `s` at grapheme-cluster index `idx`, returning
/// the net change in grapheme-cluster count.
///
/// This is the cursor-safe form for editor payloads that may contain
/// multiple codepoints but fewer user-visible clusters: IME commits,
/// dead-key combining marks, and ZWJ emoji sequences. The returned
/// delta is computed from the buffer's pre/post cluster counts so a
/// combining mark that merges into the previous cluster advances by
/// zero while a multi-codepoint cluster advances by one.
///
/// Cost: one grapheme walk over the inserted prefix to locate the new
/// cursor, plus the work inside [`insert_str_at_grapheme`].
pub fn insert_str_at_grapheme_counted(s: &mut String, idx: usize, source: &str) -> usize {
    let b = find_byte_index_of_grapheme(s, idx).unwrap_or(s.len());
    let source_len = source.len();
    insert_str_at_grapheme(s, idx, source);
    // The cursor must land past the bytes we just inserted. Counting
    // clusters in the buffer prefix up to the end of the insertion
    // handles forward merging: inserting "e" before a standalone
    // combining mark produces one cluster, and the cursor advances by
    // one (the inserted base plus the mark that followed), not zero.
    let new_cursor = count_grapheme_clusters(&s[..b + source_len]);
    new_cursor.saturating_sub(idx)
}

/// Delete the grapheme cluster at grapheme index `idx`. No-op if `idx`
/// is past the end.
///
/// Cost: O(n) over `s` to walk two grapheme boundaries. No allocation.
pub fn delete_grapheme_at(s: &mut String, idx: usize) {
    let Some(start) = find_byte_index_of_grapheme(s, idx) else {
        return;
    };
    // The end is the start of the *next* grapheme, or the buffer end
    // if `idx` is the last cluster.
    let end = find_byte_index_of_grapheme(s, idx + 1).unwrap_or(s.len());
    s.replace_range(start..end, "");
}

/// Number of extended grapheme clusters in `s`. O(n) walk; no
/// allocation.
pub fn count_grapheme_clusters(s: &str) -> usize {
    s.graphemes(true).count()
}

/// Borrow the first `n` grapheme clusters of `s`, and say whether any
/// clusters were left behind.
///
/// Returns `(prefix, truncated)`. `prefix` covers at most `n`
/// clusters — the whole of `s` when it is shorter — and `truncated`
/// is `true` exactly when `s` holds more than `n` clusters. That flag
/// is what an "…"-appending caller actually wants, and it falls out
/// of the same walk.
///
/// This replaces the `text.graphemes(true).take(n).collect::<String>()`
/// plus `text.graphemes(true).count() > n` idiom, which walks the
/// string twice and allocates the prefix. `n == 0` yields
/// `("", !s.is_empty())`.
///
/// # Costs
///
/// O(min(`n`, cluster count)) grapheme walk plus one extra cluster
/// decode to answer `truncated`. No allocation — the prefix borrows
/// from `s`, and it always ends on a cluster boundary so a ZWJ emoji
/// or a combining mark is never cut in half.
pub fn take_graphemes(s: &str, n: usize) -> (&str, bool) {
    let mut end = 0usize;
    let mut iter = s.graphemes(true);
    for _ in 0..n {
        match iter.next() {
            Some(g) => end += g.len(),
            // Fewer than `n` clusters available: the whole string is
            // the prefix and nothing was left behind.
            None => return (s, false),
        }
    }
    (&s[..end], iter.next().is_some())
}

/// The longest prefix of `s` that is at most `max_bytes` long and
/// ends on a grapheme-cluster boundary, with whether anything was cut.
///
/// The byte-budget sibling of [`take_graphemes`] (a cluster count) and
/// [`truncate_to_display_width`] (a cell count). Use it where the
/// limit is storage or wire size rather than what a reader sees. A
/// diagnostic bounded so a reply frame cannot overflow is the caller
/// it was written for. A cluster that would straddle the budget is
/// left out whole, so a ZWJ emoji or a base plus its combining marks
/// is never split. The prefix can therefore come out shorter than
/// `max_bytes`, and is empty when the first cluster alone exceeds it.
///
/// # Costs
///
/// Free when `s` already fits: one length comparison, no walk.
/// Otherwise an O(prefix) grapheme walk that stops at the budget. No
/// allocation; the prefix borrows from `s`.
pub fn truncate_to_byte_budget(s: &str, max_bytes: usize) -> (&str, bool) {
    if s.len() <= max_bytes {
        return (s, false);
    }
    let mut end = 0usize;
    for g in s.graphemes(true) {
        if end + g.len() > max_bytes {
            break;
        }
        end += g.len();
    }
    (&s[..end], true)
}

/// Split `s` into its grapheme clusters, each owned as a `String`.
///
/// Exists for the cluster-vector layouts that must outlive the source
/// string — `SidePattern`'s parsed border sections are the caller
/// this was lifted from. Prefer borrowing iteration (`take_graphemes`,
/// `find_byte_index_of_grapheme`) wherever the source stays alive:
/// this shape is the expensive one.
///
/// # Costs
///
/// O(n) grapheme walk, one heap allocation per cluster plus one for
/// the vector. On ASCII that is one 1-byte `String` per character.
pub fn split_graphemes_owned(s: &str) -> Vec<String> {
    s.graphemes(true).map(|g| g.to_string()).collect()
}

/// Rebuild `s` with `separator` inserted between every adjacent pair
/// of grapheme clusters, so `join_graphemes("ab", "\n") == "a\nb"`.
/// An empty `s` yields an empty `String`; a single cluster yields
/// itself with no separator.
///
/// Splitting on clusters rather than `char`s is the whole point: a
/// vertical border column built by joining `char`s would put the
/// combining mark of `é` on the row below its base, and would split a
/// ZWJ emoji into its component codepoints.
///
/// # Costs
///
/// Two O(n) grapheme walks — one to count clusters, one to build —
/// and exactly one allocation, sized to the finished string. Counting
/// first is deliberate: reserving only `s.len()` and letting the
/// buffer grow costs a reallocation for every non-empty separator,
/// and the caller is a border column rebuilt per frame.
pub fn join_graphemes(s: &str, separator: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    // One separator per cluster boundary, of which there are
    // `clusters - 1`. The early return above means `s` is non-empty
    // and so `clusters >= 1`, but the saturating form keeps the
    // reservation correct on its own terms rather than on a guard
    // eight lines up: an exact reservation that underflows would ask
    // for a `usize::MAX`-sized allocation.
    let clusters = count_grapheme_clusters(s);
    let mut out = String::with_capacity(s.len() + separator.len() * clusters.saturating_sub(1));
    let mut first = true;
    for g in s.graphemes(true) {
        if !first {
            out.push_str(separator);
        }
        out.push_str(g);
        first = false;
    }
    out
}

/// Monospace display width of `s` in terminal-cell units, counting
/// East-Asian-Wide / Fullwidth graphemes as 2, zero-width / combining
/// marks as 0, and everything else as 1.
///
/// Why this exists: cosmic-text's box-drawing glyphs render at ~1 cell
/// wide in the app's monospace fallback stack, but CJK / fullwidth code
/// points render at ~2 cells. Counting `.chars().count()` or even
/// `count_grapheme_clusters` under-measures a line with `日本語` in it
/// and the right-side console border drifts left. Callers that are
/// laying out a fixed-width frame around a line need the *display*
/// width.
///
/// Cost: O(n) grapheme walk; each grapheme dispatches to a handful of
/// range checks. No allocation.
///
/// The inline range table covers the common East-Asian-Wide blocks
/// (Hangul Jamo, CJK Symbols & Punctuation, Hiragana, Katakana, CJK
/// Unified Ideographs, Yi, Hangul Syllables, CJK Compatibility
/// Ideographs, Vertical Forms, Halfwidth/Fullwidth, CJK Extensions).
/// It is deliberately *not* the full Unicode `East_Asian_Width=W` set
/// — that would be a ~1.5 KB table pulled from `unicode-width`; we
/// keep the crate-dep-free version until a concrete test case proves
/// a gap.
pub fn grapheme_display_width(s: &str) -> usize {
    let mut width = 0usize;
    for g in s.graphemes(true) {
        // A grapheme's display width is the width of its *base*
        // character; combining marks that make up the rest of the
        // cluster add 0. The base is the first scalar.
        let Some(base) = g.chars().next() else { continue };
        width += scalar_display_width(base);
    }
    width
}

/// Truncate `s` to at most `max_width` terminal cells of display
/// width, cutting cleanly on grapheme-cluster boundaries. A cluster
/// whose base is width-2 will not be included if it would push past
/// `max_width`.
///
/// Returns the truncated borrowed slice — no allocation. Useful for
/// clipping scrollback lines to a fixed-width console frame without
/// ever landing mid-grapheme (or splitting a wide CJK glyph across
/// the border).
///
/// Cost: O(n) grapheme walk; stops as soon as it would exceed
/// `max_width`.
pub fn truncate_to_display_width(s: &str, max_width: usize) -> &str {
    let mut byte_end = 0usize;
    let mut used = 0usize;
    for g in s.graphemes(true) {
        let base = match g.chars().next() {
            Some(c) => c,
            None => continue,
        };
        let w = scalar_display_width(base);
        if used + w > max_width {
            break;
        }
        used += w;
        byte_end += g.len();
    }
    &s[..byte_end]
}

/// Break `s` into lines no wider than `max_width` display cells,
/// preferring a break at a whitespace run and never splitting a
/// grapheme cluster.
///
/// [`truncate_to_display_width`]'s counterpart: that one clips the
/// overflow away, this one folds it onto the next line. Both measure
/// in the same monospace cells [`grapheme_display_width`] counts, so
/// a caller rendering into a proportional font — the load-failure
/// placard in [`crate::mindmap::placard`] is the one in-tree example
/// — is using a column count as an approximation of a pixel width,
/// deliberately, because the alternative costs a `FONT_SYSTEM`
/// acquisition per candidate break.
///
/// Contract, in the order a caller trips over it:
///
/// - **Hard line breaks in `s` are preserved.** Input is split on
///   `str::lines` first and each line wrapped independently, so
///   `""` yields no lines at all (matching `"".lines()`) and a blank
///   line inside `s` yields an empty `String` that keeps the
///   paragraph gap.
/// - **Leading whitespace survives; trailing whitespace does not.**
///   Indentation is content, so the first wrapped line keeps the
///   spaces it started with. The run that justified a break is
///   consumed by the break rather than dangling off the end or
///   heading the next line, and every emitted line is right-trimmed —
///   which is also what keeps a run sitting on the right margin from
///   pushing the line past `max_width`.
/// - **Indentation at least as wide as `max_width` is consumed by
///   the break its own width forces**, not emitted as a blank first
///   line. `("        x", 4)` is `["x"]`, not `["", "x"]`. This
///   follows from the rule above — the run that justifies a break is
///   consumed by it — and from the next one: an empty `String` in
///   the output means a paragraph gap in the input, so emitting one
///   for indentation would invent a gap that is not there.
/// - **A word wider than `max_width` is split mid-word** at the cell
///   boundary, because the alternative is a line that silently
///   ignores the width it was given. Long paths and serde messages
///   are the reason this is not an error case.
/// - **A single cluster wider than `max_width` still gets its own
///   line**, over-wide, rather than being dropped: this function
///   never loses input. `max_width == 0` is not representable and is
///   clamped to 1 for the same reason.
///
/// Cost: one pass over the graphemes of `s` (widths accumulate as it
/// goes, so a break never rescans), one `Vec` of per-grapheme
/// bookkeeping sized to the longest hard line, and one `String` per
/// emitted line. Cold-path shaped — the callers are error placards
/// and console reflow, not the frame loop.
pub fn wrap_to_display_width(s: &str, max_width: usize) -> Vec<String> {
    let max_width = max_width.max(1);
    let mut out: Vec<String> = Vec::new();
    let mut scratch: Vec<WrapCell> = Vec::new();
    for hard_line in s.lines() {
        wrap_one_line(hard_line, max_width, &mut scratch, &mut out);
    }
    out
}

/// One grapheme of a line being wrapped: the byte range it occupies
/// in the line, the display width of the line up to and including it,
/// and whether it can serve as a break point.
struct WrapCell {
    byte_start: usize,
    byte_end: usize,
    cumulative_width: usize,
    is_whitespace: bool,
}

/// Wrap a single hard line (guaranteed newline-free) into `out`.
/// `scratch` is reused across lines so the per-grapheme bookkeeping
/// is allocated once per [`wrap_to_display_width`] call rather than
/// once per line.
fn wrap_one_line(line: &str, max_width: usize, scratch: &mut Vec<WrapCell>, out: &mut Vec<String>) {
    scratch.clear();
    let mut width = 0usize;
    for (byte_start, g) in line.grapheme_indices(true) {
        width += g.chars().next().map_or(0, scalar_display_width);
        scratch.push(WrapCell {
            byte_start,
            byte_end: byte_start + g.len(),
            cumulative_width: width,
            is_whitespace: !grapheme_is_not_whitespace(g),
        });
    }
    if scratch.is_empty() {
        // A blank line is a paragraph gap, not nothing.
        out.push(String::new());
        return;
    }

    let mut start = 0usize;
    // Display width consumed by everything before `start`, so the
    // width of the current line through cell `i` is one subtraction.
    let mut consumed = 0usize;
    // The most recent *completed* whitespace run beginning after
    // `start`, as (first cell, one past last cell) — the break the
    // wrapper prefers. `open_run` is the run currently being scanned.
    let mut break_run: Option<(usize, usize)> = None;
    let mut open_run: Option<usize> = None;

    let mut i = 0usize;
    while i < scratch.len() {
        if scratch[i].is_whitespace {
            // Whitespace never overflows a line: a run at the right
            // margin is trimmed off when the line is emitted, so
            // letting it "fit" is what keeps `"word  "` from
            // producing a second, blank line.
            open_run.get_or_insert(i);
            i += 1;
            continue;
        }
        if let Some(run_start) = open_run.take() {
            if run_start > start {
                break_run = Some((run_start, i));
            }
        }

        if scratch[i].cumulative_width - consumed > max_width && i > start {
            // Cell `i` does not fit. Break at the last whitespace run
            // when one lies strictly inside the line so far;
            // otherwise split the word at `i`. `i > start` guarantees
            // both cuts leave at least one cell behind, so the loop
            // always makes progress.
            let (cut, next_start) = match break_run {
                Some((run_start, run_end)) if run_start > start && run_end <= i => (run_start, run_end),
                _ => (i, i),
            };
            push_wrapped_segment(line, scratch[start].byte_start, scratch[cut - 1].byte_end, out);
            consumed = scratch[next_start - 1].cumulative_width;
            start = next_start;
            break_run = None;
            // `i` is re-examined against the fresh line rather than
            // advanced past: when the break was a word split it is
            // the first cell of the next line.
            continue;
        }
        i += 1;
    }
    let line_end = scratch[scratch.len() - 1].byte_end;
    push_trimmed(line, scratch[start].byte_start, line_end, out);
}

/// Emit `line[from..to]` with trailing whitespace removed. The right
/// margin is where a wrap discards whitespace; the left margin is
/// where it preserves it, so only the end is trimmed.
///
/// Used for the *last* segment of a hard line, which is emitted
/// whether or not it has content: a blank input line is a paragraph
/// gap and has to survive as one.
fn push_trimmed(line: &str, from: usize, to: usize, out: &mut Vec<String>) {
    out.push(line[from..to].trim_end().to_string());
}

/// [`push_trimmed`] for a segment cut off by a *break*, where an
/// all-whitespace result is dropped rather than emitted.
///
/// The only way to reach one is indentation at least as wide as the
/// column: `("        x", 4)` breaks before the `x` with eight
/// spaces behind it. Emitting `""` there would claim a paragraph gap
/// the input does not contain — the one meaning an empty `String`
/// carries in this function's output — and the caller that noticed
/// was the load-failure placard, whose contract says a blank line
/// separates its parts.
///
/// No content can be lost: the segment is dropped only when it
/// trims to nothing, which means it held nothing but the whitespace
/// the break consumed.
fn push_wrapped_segment(line: &str, from: usize, to: usize, out: &mut Vec<String>) {
    let text = line[from..to].trim_end();
    if !text.is_empty() {
        out.push(text.to_string());
    }
}

/// Display width of a single scalar. Exposed for tests; call sites
/// that have a string should use [`grapheme_display_width`] instead so
/// combining marks fold into their base cluster.
pub fn scalar_display_width(c: char) -> usize {
    let cp = c as u32;
    // Zero-width controls, zero-width space, ZWJ, ZWNJ, BOM, and the
    // combining-mark blocks. These never advance the cursor.
    if cp == 0
        || (0x0300..=0x036F).contains(&cp)   // Combining Diacritical Marks
        || (0x1AB0..=0x1AFF).contains(&cp)   // Combining Diacritical Marks Extended
        || (0x1DC0..=0x1DFF).contains(&cp)   // Combining Diacritical Marks Supplement
        || (0x20D0..=0x20FF).contains(&cp)   // Combining Diacritical Marks for Symbols
        || (0xFE20..=0xFE2F).contains(&cp)   // Combining Half Marks
        || cp == 0x200B                       // Zero Width Space
        || cp == 0x200C                       // Zero Width Non-Joiner
        || cp == 0x200D                       // Zero Width Joiner
        || cp == 0xFEFF
    // BOM / Zero Width No-Break Space
    {
        return 0;
    }
    // East Asian Wide / Fullwidth.
    if (0x1100..=0x115F).contains(&cp)         // Hangul Jamo
        || (0x2E80..=0x303E).contains(&cp)     // CJK Radicals Supplement, Kangxi, CJK Symbols & Punctuation
        || (0x3041..=0x33FF).contains(&cp)     // Hiragana, Katakana, Bopomofo, CJK Strokes, Enclosed CJK
        || (0x3400..=0x4DBF).contains(&cp)     // CJK Unified Ideographs Extension A
        || (0x4E00..=0x9FFF).contains(&cp)     // CJK Unified Ideographs
        || (0xA000..=0xA4CF).contains(&cp)     // Yi Syllables, Yi Radicals
        || (0xAC00..=0xD7A3).contains(&cp)     // Hangul Syllables
        || (0xF900..=0xFAFF).contains(&cp)     // CJK Compatibility Ideographs
        || (0xFE30..=0xFE4F).contains(&cp)     // CJK Compatibility Forms
        || (0xFF00..=0xFF60).contains(&cp)     // Fullwidth Forms (pre-halfwidth)
        || (0xFFE0..=0xFFE6).contains(&cp)     // Fullwidth signs
        || (0x20000..=0x2FFFD).contains(&cp)   // CJK Extensions B–F, Compat Supplement
        || (0x30000..=0x3FFFD).contains(&cp)
    // CJK Extension G+
    {
        return 2;
    }
    1
}

/// Remove the last `n` grapheme clusters from `s` (a "Backspace ×n"
/// on the edit cursor at the end of the string). If `s` contains
/// fewer than `n` clusters the string is cleared entirely. O(n)
/// reverse grapheme walk; no allocation (truncates in place).
pub fn delete_back_unicode(s: &mut String, n: usize) {
    let mut char_count = 0;
    let mut grapheme_count = 0;

    for grapheme in UnicodeSegmentation::graphemes(s.as_str(), true).rev() {
        grapheme_count += 1;
        if grapheme_count > n {
            break;
        }
        char_count += grapheme.len();
    }
    if grapheme_count <= n {
        s.clear();
        return;
    }
    let new_len = s.len() - char_count;
    s.truncate(new_len);
}

/// Remove the first `n` grapheme clusters from `s` (a "Delete ×n" at
/// the beginning of the string). If `s` contains fewer than `n`
/// clusters the string is cleared entirely. O(n) forward grapheme
/// walk + O(len) drain. No allocation (edits in place).
pub fn delete_front_unicode(s: &mut String, n: usize) {
    let mut char_count = 0;
    let mut grapheme_count = 0;

    for grapheme in UnicodeSegmentation::graphemes(s.as_str(), true) {
        grapheme_count += 1;
        if grapheme_count > n {
            break;
        }
        char_count += grapheme.len();
    }
    if grapheme_count <= n {
        s.clear();
        return;
    }
    s.drain(0..char_count);
}

/// Move a grapheme-indexed cursor LEFT to the previous word boundary.
/// A "word" is a maximal run of graphemes whose first scalar is
/// alphanumeric (per `char::is_alphanumeric`); punctuation, whitespace,
/// and emoji are treated as word boundaries. Skips backwards past any
/// boundary graphemes immediately before `cursor`, then past the run
/// of word graphemes the boundary skipping reached, returning the
/// grapheme index at the start of that word run.
///
/// `cursor` is interpreted as a count of graphemes (not bytes); a
/// `cursor` of 0 returns 0; a `cursor` greater than the grapheme
/// count is clamped at the buffer's grapheme count.
///
/// **Cost**: one O(n) `grapheme_indices` walk bounded by `cursor`
/// (collects byte offsets into a `Vec<usize>` of capacity
/// `min(cursor, buffer.len()) + 1`), then a backward array-index scan
/// of that vector — the second scan is O(cursor) byte-slice +
/// `is_alphanumeric` checks, no grapheme decoding. Allocates one
/// `usize` per cluster walked; the prior in-app version allocated a
/// `Vec<&str>` over the **whole** buffer (per `CONVENTIONS §B7`
/// hot-path posture).
///
/// A cluster is part of a word when *any* of its scalars is
/// `char::is_alphanumeric`. For ZWJ clusters and combining-mark
/// sequences that is the human-perceived base character; for
/// regional-indicator pairs (flag emoji) no scalar is alphanumeric, so
/// the cluster counts as a boundary. Reading only the *first* scalar
/// gives the same answer everywhere except UAX #29 `Prepend`
/// sequences, where it reads an invisible formatting scalar instead of
/// the character the reader sees — see `grapheme_is_word`.
pub fn word_left(buffer: &str, cursor: usize) -> usize {
    scan_back(buffer, cursor, grapheme_is_word, true)
}

/// Move a grapheme-indexed cursor LEFT past one **whitespace-delimited**
/// word — the `Ctrl+W` / "kill word" motion, as opposed to
/// [`word_left`]'s alphanumeric-run motion.
///
/// A cluster is a separator only when *every* scalar in it is
/// `char::is_whitespace`, so `-`, `=`, `/` and emoji all stay inside
/// the word. That is the difference that matters to a shell-style
/// console: `word_left` stops at each punctuation run inside
/// `key=value`, while this walks over the whole token.
///
/// Skips backwards past any whitespace immediately before `cursor`,
/// then past the run of non-whitespace it reaches, returning the
/// grapheme index at that run's start. `cursor == 0` returns 0; a
/// `cursor` past the buffer's cluster count is clamped to the count.
///
/// U+3000 IDEOGRAPHIC SPACE and the other multi-byte spaces are
/// separators here (`char::is_whitespace` covers them) while U+200B
/// ZERO WIDTH SPACE is *not* — it is `Cf`, not whitespace, so it
/// stays inside the token exactly as it does for
/// [`first_non_whitespace_grapheme`].
///
/// **Cost**: same shape as [`word_left`] — one `grapheme_indices`
/// walk bounded by `cursor` collecting one byte offset per cluster
/// walked, then a backward scan over that vector.
pub fn prev_word_boundary_ws(buffer: &str, cursor: usize) -> usize {
    scan_back(buffer, cursor, grapheme_is_not_whitespace, true)
}

/// Grapheme index at which the whitespace-delimited token *ending* at
/// `cursor` starts.
///
/// The difference from [`prev_word_boundary_ws`] is the leading skip:
/// this one does **not** step over whitespace sitting immediately
/// before the cursor, so a cursor parked after a space reports the
/// cursor itself — an empty token, which is what a completion popup
/// wants when the user has just typed a separator and is starting a
/// fresh argument. `prev_word_boundary_ws` would instead delete back
/// into the previous word.
///
/// `cursor == 0` returns 0; a `cursor` past the buffer's cluster
/// count is clamped to the count.
///
/// **Cost**: same shape as [`prev_word_boundary_ws`].
pub fn token_start_ws(buffer: &str, cursor: usize) -> usize {
    scan_back(buffer, cursor, grapheme_is_not_whitespace, false)
}

/// Shared backward cursor scan behind [`word_left`],
/// [`prev_word_boundary_ws`], and [`token_start_ws`].
///
/// `in_word` classifies a cluster as part of a word. When
/// `skip_leading_boundary` is set the scan first steps back over any
/// run of non-word clusters, then over the word run it reaches;
/// otherwise it only does the second step.
///
/// Collecting `cursor + 1` grapheme-start offsets is what makes the
/// classifier see *exact* cluster slices: the vector's last entry is
/// the end offset of the cluster just before `cursor`, so a predicate
/// that inspects every scalar (whitespace) is as correct as one that
/// inspects only the first (alphanumeric).
fn scan_back(buffer: &str, cursor: usize, in_word: fn(&str) -> bool, skip_leading_boundary: bool) -> usize {
    if cursor == 0 {
        return 0;
    }
    let starts = grapheme_start_offsets(buffer, cursor);
    let mut i = starts.len() - 1;
    if skip_leading_boundary {
        while i > 0 && !in_word(&buffer[starts[i - 1]..starts[i]]) {
            i -= 1;
        }
    }
    while i > 0 && in_word(&buffer[starts[i - 1]..starts[i]]) {
        i -= 1;
    }
    i
}

/// Byte offsets of the grapheme clusters of `buffer` that lie before
/// `cursor`, plus one trailing sentinel so cluster `i` is exactly
/// `&buffer[out[i]..out[i + 1]]`.
///
/// The returned vector always has at least one element, and its
/// length minus one is the number of clusters strictly before
/// `cursor` (which is fewer than `cursor` when the cursor runs past
/// the end of the buffer).
///
/// Cost: one `grapheme_indices` walk bounded by `cursor`; allocates
/// `min(cursor, buffer.len()) + 1` `usize`s, not the cluster slices
/// themselves. The reservation is clamped by the byte length — which
/// is an upper bound on the cluster count — so an out-of-range
/// `cursor` from a stale caller cannot ask for a huge allocation.
fn grapheme_start_offsets(buffer: &str, cursor: usize) -> Vec<usize> {
    let mut starts: Vec<usize> = Vec::with_capacity(cursor.min(buffer.len()) + 1);
    for (idx, (byte, _)) in buffer.grapheme_indices(true).enumerate() {
        starts.push(byte);
        if idx == cursor {
            // `byte` is the start of the cluster *at* the cursor,
            // which is the end of the cluster before it — the
            // sentinel we need. Stop.
            break;
        }
    }
    if starts.len() <= cursor {
        // The walk ran out of clusters before reaching `cursor`, so
        // no sentinel was collected; the buffer's end is it.
        starts.push(buffer.len());
    }
    starts
}

/// Move a grapheme-indexed cursor RIGHT to the next word boundary.
/// Mirror of [`word_left`]: skip forward past any boundary graphemes
/// at `cursor`, then past the run of word graphemes that follows,
/// returning the grapheme index just past the word's end.
///
/// `cursor` is a grapheme count; a `cursor` at or past the buffer's
/// grapheme count returns it unchanged.
///
/// **Cost**: O(n) grapheme walk; no allocation. Walks the
/// `grapheme_indices` iterator forward once and stops as soon as the
/// next-boundary is found.
pub fn word_right(buffer: &str, cursor: usize) -> usize {
    let mut iter = buffer.grapheme_indices(true);
    // Skip past `cursor` graphemes; if we exhaust before reaching
    // `cursor`, we're already at the end.
    let mut idx = 0usize;
    for _ in 0..cursor {
        if iter.next().is_none() {
            return idx;
        }
        idx += 1;
    }
    // Phase 1: skip non-word graphemes at the cursor.
    let mut peek = iter.next();
    while let Some((_, g)) = peek {
        if grapheme_is_word(g) {
            break;
        }
        idx += 1;
        peek = iter.next();
    }
    // Phase 2: skip word graphemes until non-word or end.
    while let Some((_, g)) = peek {
        if !grapheme_is_word(g) {
            break;
        }
        idx += 1;
        peek = iter.next();
    }
    idx
}

/// Whether a grapheme is part of a "word" for word-boundary cursor
/// motion (`word_left` / `word_right`). A cluster is a word cluster
/// when *any* scalar in it is `char::is_alphanumeric` — the mirror of
/// [`grapheme_is_not_whitespace`]'s "a cluster separates only when
/// every scalar does", and the same reasoning: the cursor moves over
/// what the reader sees, and a cluster is one thing on screen.
///
/// The first-scalar rule this replaces agreed on every cluster whose
/// base is the visible character — ZWJ families, skin tones,
/// combining-mark sequences, VS16 sequences (`"❤\u{FE0F}"`,
/// `"1\u{FE0F}\u{20E3}"`), Indic conjuncts, and regional-indicator
/// pairs (no scalar in a flag is alphanumeric, so a flag stays a
/// boundary under both rules). It disagreed on **two** classes, not
/// one; the sweeps below are over every scalar in Unicode.
///
/// - **UAX #29 `Prepend`** — GB9b glues an *invisible* formatting
///   scalar to the front of the character it applies to. There are
///   **13** such scalars (U+0600–U+0605, U+06DD, U+070F, U+0890,
///   U+0891, U+08E2, U+110BD, U+110CD). U+0600 ARABIC NUMBER SIGN
///   before a digit is the everyday case: the first-scalar rule reads
///   the invisible prefix, calls the cluster a boundary, and walks the
///   cursor straight through a number as though it were punctuation.
///   Reading any scalar finds the digit. This is the class the rule
///   was changed for.
/// - **A non-alphanumeric base carrying an `Other_Alphabetic` mark** —
///   `Mn`/`Mc` codepoints that `char::is_alphanumeric` reports as
///   alphabetic. **1 353** scalars qualify, and every one of them
///   disagrees behind *any* non-alphanumeric base, so the class is a
///   cross product rather than a list: `"-\u{345}"` (hyphen +
///   COMBINING GREEK YPOGEGRAMMENI), `"$\u{093E}"` (dollar +
///   DEVANAGARI VOWEL SIGN AA), `"،\u{064E}"` (Arabic comma + FATHA).
///   Here `any` is the *less* obviously right answer:
///
///   ```text
///   buf = "key-\u{345}val"   clusters ["k","e","y","-\u{345}","v","a","l"]
///   word_left(buf, 8) = 0    // one word — the marked hyphen no longer breaks it
///   ```
///
///   It is marginal under either rule: the cluster is one thing on
///   screen and neither answer is wrong about what the reader sees,
///   the input is vanishingly rare in editor text, and the cost of
///   getting it "wrong" is one cursor hop. `any` is kept because it
///   is the rule [`grapheme_is_not_whitespace`] uses and because the
///   `Prepend` class it fixes is real Arabic text; the second class is
///   the price, and it is named here rather than left for a reader to
///   discover.
fn grapheme_is_word(g: &str) -> bool {
    g.chars().any(char::is_alphanumeric)
}

/// Whether a grapheme is part of a whitespace-delimited token
/// (`prev_word_boundary_ws` / `token_start_ws`). A cluster separates
/// tokens only when *every* scalar in it is whitespace, so a base
/// character carrying a combining mark stays inside the token even
/// though the mark alone would not — the same rule
/// [`first_non_whitespace_grapheme`] applies.
fn grapheme_is_not_whitespace(g: &str) -> bool {
    g.chars().any(|c| !c.is_whitespace())
}
