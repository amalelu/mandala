//! Grapheme-cluster aware text utilities. Mandala's text editing,
//! console scrollback, and label rendering all flow through these
//! helpers so emoji, combining marks, and CJK glyphs round-trip
//! correctly. Reach for these primitives from the application crate
//! rather than indexing a `String` by byte offset — the latter
//! splits a 👨‍👩‍👧 ZWJ sequence on the first edit and the rest of the
//! pipeline silently corrupts.

use log::error;
use unicode_segmentation::UnicodeSegmentation;

/// Borrow the slice from `byte_index` up to (not including) the next
/// `\n`, or to the end of `s` if no newline follows. `byte_index` must
/// land on a UTF-8 char boundary; passing a mid-codepoint byte panics
/// like any other `String` slice. O(n) on the search distance.
pub(crate) fn slice_to_newline(s: &str, byte_index: usize) -> &str {
    let end_byte_index = s[byte_index..]
        .find('\n')
        .map_or(s.len(), |i| byte_index + i);

    &s[byte_index..end_byte_index]
}

/// Replace `target`'s graphemes from `g_index` up to (and not past) the
/// next newline with the graphemes in `source`. If `source` is longer
/// than the existing line tail, the extras are appended; if shorter,
/// the surplus tail beyond `source`'s length is preserved (only the
/// overlapping prefix is overwritten). Stops at the first `\n` in
/// either string — multi-line replacement is intentionally outside
/// this helper's scope.
///
/// Returns `Some((g_index, extra))` when the replacement *grew* the
/// line by `extra` graphemes (the caller uses this to shift any
/// downstream `ColorFontRegions` ranges). Returns `None` when the
/// replacement fit entirely within the existing line tail.
///
/// Cost: two `count_grapheme_clusters` walks plus one
/// `replace_substring` (which itself allocates a fresh `Vec<u8>` —
/// a known hot-path allocation tracked alongside the rest of the
/// "no-alloc text edit" work).
pub fn replace_graphemes_until_newline(
    target: &mut String,
    g_index: usize,
    source: &str,
) -> Option<(usize, usize)> {
    let insert_num_graphemes = count_grapheme_clusters(source);
    let b_index = find_byte_index_of_grapheme(target, g_index).unwrap_or(target.len());

    let line_section = slice_to_newline(target, b_index);

    let target_line_num_graphemes = count_grapheme_clusters(line_section);
    let end_of_target_line_idx = b_index + line_section.len();

    if insert_num_graphemes >= target_line_num_graphemes {
        // We can basically cut away this whole region and then insert our string
        replace_substring(target, b_index, end_of_target_line_idx, source);
        Some((g_index, insert_num_graphemes - target_line_num_graphemes))
    } else {
        // We need to cut away a part between index..insert_num_graphemes, and then insert our string
        replace_substring(
            target,
            b_index,
            find_byte_index_of_grapheme(target, g_index + insert_num_graphemes).unwrap(),
            source,
        );
        None
    }
}

/// Return the byte offset of the `index`-th *char* (Unicode scalar
/// value). Returns `None` if `index` is out of bounds. O(n) over
/// `s.char_indices()`. Prefer [`find_byte_index_of_grapheme`] unless
/// you are specifically working with scalar offsets — chars split
/// grapheme clusters on emoji.
pub fn find_byte_index_of_char(s: &str, index: usize) -> Option<usize> {
    s.char_indices().nth(index).map(|(byte_idx, _)| byte_idx)
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
    None // Return None if index is out of bounds
}

/// Finds the index immediately after the nth grapheme.
// Named grapheme-boundary utility preserved as a seam alongside
// the live `find_byte_index_of_grapheme` helper above — future
// grapheme-aware text-editing paths (IME runs, cursor motion
// across clusters) want the "after-nth" variant too.
// Per CODE_CONVENTIONS.md §6.
#[allow(dead_code)]
fn find_index_after_nth_grapheme(str: &str, n: usize) -> Option<usize> {
    // Graphemes method provides an iterator over the grapheme clusters
    let mut graphemes = str.graphemes(true);

    // Skip n graphemes and take the next one to find the boundary after the nth grapheme
    let skipped_graphemes = graphemes.by_ref().take(n + 1).collect::<Vec<&str>>();

    // If the number of graphemes collected is less than or equal to n, we've reached or exceeded the end of the string
    if skipped_graphemes.len() <= n {
        return None; // Return None if we cannot move n graphemes into the string
    }
    // Calculate the byte index: the sum of the lengths (in bytes) of all taken graphemes
    let byte_index = skipped_graphemes.iter().map(|g| g.len()).sum();

    Some(byte_index)
}

/// Return the byte offset where the `n`-th grapheme cluster starts.
/// `n = 0` returns `Some(0)` for non-empty strings. Returns `None`
/// when `n` exceeds the grapheme count. O(n) over
/// `s.grapheme_indices(true)`.
///
/// Functionally equivalent to [`find_byte_index_of_grapheme`] but
/// uses `grapheme_indices` for the scan. Exists because early callers
/// predated the other function; both are kept for call-site clarity
/// (one says "byte index of grapheme #n", the other says "nth cluster
/// byte index").
pub fn nth_grapheme_cluster_byte_index(s: &str, n: usize) -> Option<usize> {
    let mut index = 0;
    for (i, (start, _)) in s.grapheme_indices(true).enumerate() {
        if i == n {
            return Some(start);
        }
        index = start;
    }
    if n == 0 {
        return Some(index);
    }
    None
}

fn replace_substring(s: &mut String, i: usize, n: usize, source: &str) {
    let mut bytes = s.as_bytes().to_vec(); // Convert the String to a Vec<u8>
    let source_bytes = source.as_bytes();

    // Remove the specified range
    bytes.drain(i..n);

    // Insert the new bytes
    bytes.splice(i..i, source_bytes.iter().cloned());

    // Safely convert Vec<u8> back to String
    if let Ok(modified_string) = String::from_utf8(bytes) {
        *s = modified_string;
    } else {
        // Handle invalid UTF-8 conversion, if necessary
        error!("Failed to convert bytes to UTF-8 String.");
    }
}

/// Grapheme-aware analogue of `String::split_off`. Splits `original`
/// at grapheme cluster index `at`, leaving the prefix in `original`
/// and returning the suffix as an owned `String`. If `at` reaches or
/// exceeds the grapheme count, returns an empty `String` and leaves
/// `original` unchanged.
///
/// Cost: O(n) grapheme walk + two `concat` calls (the implementation
/// collects through a `Vec<&str>` and rebuilds both halves). Allocates
/// the new prefix and the returned suffix; the original buffer is
/// reassigned.
pub fn split_off_graphemes(original: &mut String, at: usize) -> String {
    let graphemes = original.graphemes(true).collect::<Vec<&str>>();

    if at >= graphemes.len() {
        return original.split_off(original.len());
    }

    let (left, right) = graphemes.split_at(at);
    let right_str = right.concat();

    *original = left.concat();
    right_str
}

/// Number of newline-separated lines in `s`. The trailing line counts
/// even when `s` does not end in `\n`, so an empty string yields 1.
/// O(n) byte scan; no allocation.
pub fn count_number_lines(s: &str) -> usize {
    s.as_bytes().iter().filter(|&&c| c == b'\n').count() + 1
}

/// Grapheme-cluster span of the `n`-th newline-separated line in `s`,
/// returned as a half-open `(start_grapheme, end_grapheme)` range.
/// `n = 0` is the first line. Returns `None` if `s` is empty or `n`
/// is past the last line.
///
/// Cost: O(n) grapheme walk plus a final `s.graphemes(true).count()`
/// when the last line is requested.
pub fn find_nth_line_grapheme_range(s: &str, n: usize) -> Option<(usize, usize)> {
    if s.len() == 0 {
        return None;
    }
    let mut line_head = 0;
    let mut last_line_start = 0;
    let mut new_line: bool = true;
    for (idx, graph) in s.graphemes(true).enumerate() {
        if new_line {
            last_line_start = idx;
            new_line = false;
        }
        // Grapheme clusters yielded by `unicode_segmentation` are
        // guaranteed non-empty, so a literal newline is the only
        // line terminator we have to test for.
        if graph == "\n" {
            if line_head == n {
                // We're at the end of the requested line: emit the
                // half-open range [last_line_start, idx).
                return Some((last_line_start, idx));
            }
            new_line = true;
            line_head += 1;
        }
    }
    if line_head < n || (line_head == n && new_line) {
        return None;
    }
    Some((last_line_start, s.graphemes(true).count()))
}

/// Byte span of the `n`-th newline-separated line in `s`, returned as
/// `(start_byte, end_byte)`. `n = 0` is the first line. Returns
/// `None` if `s` is empty or `n` is past the last line.
///
/// Cost: O(n) byte-level walk via `char_indices()`. No allocation.
pub fn find_nth_line_byte_range(s: &str, n: usize) -> Option<(usize, usize)> {
    if s.len() == 0 {
        return None;
    }
    let mut line_head = 0;
    let mut last_line_start = 0;
    let mut new_line: bool = true;
    for (idx, ch) in s.char_indices() {
        if new_line {
            last_line_start = idx;
            new_line = false;
        }
        if ch == '\n' {
            if line_head == n {
                // So it's time to move the line head up one tick
                // but if the head is currently at n, then this is the last
                // index in the line
                return Some((last_line_start, idx));
            }
            // otherwise
            new_line = true;
            line_head += 1;
        }
    }
    if line_head < n || (line_head == n && new_line) {
        return None;
    }
    Some((last_line_start, s.len()))
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

/// Insert `n` spaces at grapheme-cluster index `idx`. If `idx` is
/// past the string's grapheme count the spaces are appended. O(n)
/// grapheme walk + O(len) `String::insert_str` shift.
pub fn insert_spaces(s: &mut String, idx: usize, n: usize) {
    let spaces = " ".repeat(n);
    let maybe = nth_grapheme_cluster_byte_index(s, idx);
    if let Some(byte_offset) = maybe {
        s.insert_str(byte_offset, &spaces);
    } else {
        push_spaces(s, n);
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
        || cp == 0xFEFF                       // BOM / Zero Width No-Break Space
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
        || (0x30000..=0x3FFFD).contains(&cp)   // CJK Extension G+
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
        s.clear(); // If there are fewer or equal graphemes than n, clear the string
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
        char_count += grapheme.len();

        if grapheme_count >= n {
            break;
        }
    }
    if grapheme_count < n {
        s.clear(); // If there are fewer graphemes than n, clear the string
        return;
    }
    // Remove the first n graphemes
    s.drain(0..char_count);
}

#[cfg(test)]
mod test {

}
