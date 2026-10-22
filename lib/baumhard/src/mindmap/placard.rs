// SPDX-License-Identifier: MPL-2.0

//! The map that gets shown when a map could not be loaded.
//!
//! A load failure has to reach the person holding the file, and the
//! only surface guaranteed to exist at that moment is the canvas: on
//! a desktop launch — double-click, `.desktop` entry, file
//! association — there is no terminal attached, so `stderr` is not
//! merely unread, it is gone. The answer here is to hand the shell a
//! *map* rather than a special-cased overlay: a placard is an
//! ordinary one-node [`MindMap`](crate::mindmap::model::MindMap)
//! carrying the loader's own message as
//! its text, so it renders through the same tree projection as any
//! other document (CODE_CONVENTIONS §3 "Render document content
//! through the Baumhard tree") and needs no second pipeline, no
//! renderer pass, and no GPU to test.
//!
//! **A placard is deliberately not the user's map.** It has its own
//! name, and the application binds it to no file path, so a reflexive
//! `Ctrl+S` cannot write it over the file that failed to parse. That
//! property lives at the application boundary — this module only
//! promises that the placard is a valid, self-contained map.
//!
//! ## The message is reproduced whole, and where it stops being whole
//!
//! Truncating a loader message defeats the point: the messages name
//! the offending node id, the offending key and the migration verb,
//! and the tail is where the actionable part usually is — serde puts
//! `at line L column C` last of all. So every message the loader
//! realistically emits reaches the canvas verbatim. The longest of
//! them is the parent-cycle report, which spells the whole chain out:
//! a 400-node cycle is ~3 150 clusters and 56 wrapped lines, well
//! inside the budget below.
//!
//! **A loader message is not bounded, though, and the unbounded part
//! is supplied by the file.** serde's ``invalid type: string
//! "<value>", expected f64`` quotes the offending JSON string back at
//! whatever length it was given: a 4 MB string value in a
//! `.mindmap.json` — or in a `?map=` URL a browser was handed —
//! produces a 4 MB message. Reproduced in full that is 62 508 wrapped
//! lines and a node 1 950 250 canvas units tall, and every part of
//! that is bad. The map stops round-tripping, because
//! [`MAX_NODE_AXIS`](crate::mindmap::model::MAX_NODE_AXIS) is
//! 1 000 000 and `load_from_str` refuses a node above it; the
//! application's grow-to-fit clamp pins the *box* to that ceiling and
//! leaves the *text* at 62 508 lines, so the message this module
//! exists to show is clipped by the box meant to hold it; and nobody
//! reads 62 508 lines regardless. The breach starts at 32 052 wrapped
//! lines, which is ~2 051 300 message bytes at
//! [`PLACARD_COLUMNS`](crate::mindmap::placard::PLACARD_COLUMNS).
//!
//! `MAX_NODE_AXIS` is therefore **not** the backstop this module used
//! to name. It sits downstream of the placard and clamps the wrong
//! quantity. The backstop is here:
//! [`load_failure_text`](crate::mindmap::placard::load_failure_text)
//! elides the *middle* of anything longer than
//! [`PLACARD_HEAD_CLUSTERS`](crate::mindmap::placard::PLACARD_HEAD_CLUSTERS)
//! +
//! [`PLACARD_TAIL_CLUSTERS`](crate::mindmap::placard::PLACARD_TAIL_CLUSTERS),
//! keeping both ends because both carry
//! signal, and those budgets are chosen so the tallest placard they
//! can produce stays under the ceiling by construction rather than by
//! luck.
//!
//! The worst case this module used to name — ``unknown field `x`,
//! expected one of …`` — is not merely no longer the longest, it can
//! no longer occur: #117 removed every `deny_unknown_fields` from the
//! model, so an unknown field is a `warn!` and not a load failure at
//! all.

use crate::mindmap::model::{MindMap, MindNode, MindSection, NodeLayout, NodeStyle, Position, Size, TextRun};
use crate::util::grapheme_chad::{
    count_grapheme_clusters, find_byte_index_of_grapheme, grapheme_display_width, take_graphemes,
    wrap_to_display_width,
};
use std::borrow::Cow;

/// `MindMap::name` every placard carries. Distinct from any name a
/// real map is likely to hold, so a placard is recognizable in a log
/// line or a window title without inspecting its nodes.
pub const PLACARD_MAP_NAME: &str = "load-failure";

/// Node id of a placard's single node. Dewey-decimal root, matching
/// what [`MindMap::new_blank`] would produce for a first node.
pub const PLACARD_NODE_ID: &str = "0";

/// Column count the message is wrapped to. Wide enough that a
/// typical loader message reads as two or three lines rather than a
/// column of fragments, narrow enough that a long one does not
/// become a single unreadable strip.
pub const PLACARD_COLUMNS: usize = 64;

/// Grapheme clusters kept from the **front** of each text the placard
/// reproduces — the source and the loader's message alike.
///
/// Spelled as full lines' worth of [`PLACARD_COLUMNS`] because that
/// is the unit the reader experiences. 96 lines is far past any
/// message the loader realistically emits (the longest, a 400-node
/// parent cycle, is 56) and far short of what stops being readable.
///
/// Together with [`PLACARD_TAIL_CLUSTERS`] this is what makes
/// [`load_failure`]'s round-trip promise unconditional. A text of
/// `head + tail` clusters wraps to at most `head + tail` lines — the
/// pathological input is one cluster per line — so the tallest
/// placard reachable is `2 × (6144 + 2048 + 1) + 5 = 16 391` lines,
/// or 511 399 canvas units at the placard's 24 pt line advance. That
/// is under
/// [`MAX_NODE_AXIS`](crate::mindmap::model::MAX_NODE_AXIS) with the
/// factor of two to spare that a budget nobody re-derives needs.
pub const PLACARD_HEAD_CLUSTERS: usize = 96 * PLACARD_COLUMNS;

/// Grapheme clusters kept from the **back** of each text the placard
/// reproduces. See [`PLACARD_HEAD_CLUSTERS`] for the pair's reason.
///
/// Smaller than the head because the head is where the diagnosis is,
/// but non-zero because the tail is where the *position* is: serde
/// closes with `expected f64` and `at line L column C`, and a placard
/// that dropped those would be answering "what is wrong" without
/// "where".
pub const PLACARD_TAIL_CLUSTERS: usize = 32 * PLACARD_COLUMNS;

/// First line of every placard. States the outcome before the
/// diagnosis, because that is the order the reader needs it in.
pub const PLACARD_HEADLINE: &str = "This map did not load.";

/// Last line of every placard. Answers the question the headline
/// raises — whether anything was damaged.
pub const PLACARD_FOOTER: &str = "Nothing was opened; the file on disk is unchanged.";

/// Point size of the placard's text run.
const PLACARD_FONT_SIZE_PT: f32 = 24.0;

/// Font family of the placard's text run: **empty, meaning no pin**.
///
/// `format/fonts.md` gives the empty string one meaning — "clears the
/// pin (run uses the document default)" — and an unknown family
/// another: a `warn!` at render time and a monospace fallback. The
/// placard wants the first. It shipped with `"LiberationSans"`, which
/// is the second: `app_font_by_family("LiberationSans")` is `None`,
/// because no Liberation face is compiled in and the family the
/// testament map authors under that name has never resolved
/// (`document::custom::sync` documents the same mismatch from the
/// other side).
///
/// Naming any real family would be worse, not better. There is no
/// bundled sans face to name — the Liberation, DejaVu and Noto
/// families a native `FontSystem::new()` reports come from the host's
/// fontconfig and are simply absent in the browser, which is the
/// native-only carve-out `format/fonts.md` opens by describing.
/// Pinning one would make the placard render differently on the two
/// targets (CODE_CONVENTIONS §4) on the one screen a user meets when
/// something has already gone wrong.
const PLACARD_FONT_FAMILY: &str = "";

/// Text and frame color. The red the browser build's DOM overlay
/// used before the placard replaced it, carried over so the visual
/// identity of "your map was rejected" survives the change of
/// mechanism.
const PLACARD_ERROR_COLOR: &str = "#ff6b6b";

/// Node fill. Matches the fill a freshly-created node takes, so the
/// placard sits on the canvas as a node rather than as chrome.
const PLACARD_BACKGROUND_COLOR: &str = "#141414";

/// Advance of one text cell as a fraction of the point size, and the
/// line advance as a multiple of it. Both are estimates: this module
/// has no `FONT_SYSTEM` access and does not want one on a cold error
/// path. The application's grow-to-fit pass re-measures the block
/// with real metrics and can only enlarge the box, so an estimate
/// that runs small is corrected and an estimate that runs large is
/// not — which is why these lean conservative.
const PLACARD_CELL_WIDTH_FRAC: f64 = 0.55;
/// See [`PLACARD_CELL_WIDTH_FRAC`].
const PLACARD_LINE_HEIGHT_FRAC: f64 = 1.3;

/// Build the placard map for a load failure of `source` (the path or
/// URL that was asked for) reporting `message` (the loader's own
/// text, verbatim).
///
/// The result is a complete, valid `.mindmap.json` document: it
/// round-trips through [`crate::mindmap::loader::load_from_str`] and
/// satisfies every invariant that loader enforces, which is what
/// lets the shell treat it as an ordinary document instead of a
/// special case.
///
/// **That holds for every `source` and every `message`, including
/// ones the file controls the length of**, and it holds because
/// [`load_failure_text`] elides past [`PLACARD_HEAD_CLUSTERS`] +
/// [`PLACARD_TAIL_CLUSTERS`]; without that budget a multi-megabyte
/// serde message produces a node taller than
/// [`MAX_NODE_AXIS`](crate::mindmap::model::MAX_NODE_AXIS) and the
/// sentence above is false. See the module docs for the shape that
/// gets there.
///
/// Cost: two grapheme walks of each of `source` and `message` to
/// place the elision, then one wrap pass
/// ([`wrap_to_display_width`]) over the bounded result, one `String`
/// for the joined body, and the map's own allocations. Cold —
/// reached once, on a load that already failed.
pub fn load_failure(source: &str, message: &str) -> MindMap {
    let text = load_failure_text(source, message);
    let mut map = MindMap::new_blank(PLACARD_MAP_NAME);
    map.nodes.insert(PLACARD_NODE_ID.to_string(), placard_node(text));
    map
}

/// The placard's body text: headline, the source that was asked for,
/// the loader's message, and the footer, each separated by a blank
/// line and each wrapped to [`PLACARD_COLUMNS`].
///
/// Split out from [`load_failure`] because it is the whole of what a
/// reader sees, and asserting on a `String` beats digging it back
/// out of a `MindMap`.
///
/// **The source appears once.** The line under the headline is
/// dropped when the loader's message already names the map, which
/// [`message_names_source`] decides — the commonest failure of all,
/// a path that is not there, produces `Failed to read file <path>:
/// …` and used to put that path on the placard twice, two lines
/// apart. The message is never edited to make room; it is the source
/// line that goes, because the message is the loader's own words and
/// the line is this module's framing of them.
///
/// **Both texts are elided in the middle past
/// [`PLACARD_HEAD_CLUSTERS`] + [`PLACARD_TAIL_CLUSTERS`]**, which is
/// what keeps [`load_failure`]'s round-trip promise true against a
/// message whose length the file supplies (module docs). Nothing the
/// loader realistically emits reaches the budget, so in practice this
/// is a bound rather than an edit. [`message_names_source`] is asked
/// about the *original* texts, before either is elided: whether the
/// loader named the file is a fact about what it wrote, not about
/// what fits.
///
/// Cost: two grapheme walks per text to place the elision, then one
/// [`wrap_to_display_width`] pass over each bounded result, plus the
/// joined result.
pub fn load_failure_text(source: &str, message: &str) -> String {
    let names_source = message_names_source(source, message);
    let mut lines: Vec<String> = vec![PLACARD_HEADLINE.to_string(), String::new()];
    if !names_source {
        let source = elide_middle(source);
        lines.extend(wrap_to_display_width(&source, PLACARD_COLUMNS));
        lines.push(String::new());
    }
    let message = elide_middle(message);
    lines.extend(wrap_to_display_width(&message, PLACARD_COLUMNS));
    lines.push(String::new());
    lines.push(PLACARD_FOOTER.to_string());
    lines.join("\n")
}

/// `text` with its middle replaced by [`elision_notice`] when it
/// holds more than [`PLACARD_HEAD_CLUSTERS`] +
/// [`PLACARD_TAIL_CLUSTERS`] grapheme clusters, and `text` itself
/// when it does not.
///
/// The middle rather than the tail, because both ends carry signal
/// and neither one alone is a diagnosis: serde opens with the kind of
/// mismatch and closes with the position of it, and the parent-cycle
/// report opens with the node it started from and closes with the
/// remedy. Cutting the middle is also the only cut that cannot be
/// mistaken for the loader having stopped talking — the notice is on
/// its own line, in this module's voice, between two runs of the
/// loader's.
///
/// The notice sits on its own line because [`wrap_to_display_width`]
/// splits on hard newlines first, so it can never be folded into the
/// text on either side of it.
///
/// Cost: two O(n) grapheme walks of `text` — one to count clusters,
/// one to find the byte offset the tail starts at — and, only when it
/// elides, one `String` sized to the budget. No allocation on the
/// path every real loader message takes.
fn elide_middle(text: &str) -> Cow<'_, str> {
    let total = count_grapheme_clusters(text);
    let kept = PLACARD_HEAD_CLUSTERS + PLACARD_TAIL_CLUSTERS;
    if total <= kept {
        return Cow::Borrowed(text);
    }
    let (head, _) = take_graphemes(text, PLACARD_HEAD_CLUSTERS);
    let tail_at = find_byte_index_of_grapheme(text, total - PLACARD_TAIL_CLUSTERS).unwrap_or(text.len());
    Cow::Owned(format!(
        "{head}\n{}\n{}",
        elision_notice(total - kept),
        &text[tail_at..]
    ))
}

/// The line [`elide_middle`] puts where the part it dropped was.
///
/// Short on purpose: it has to fit [`PLACARD_COLUMNS`] on its own
/// line for any `count` a `usize` can hold, or the placard would
/// break the width invariant precisely on the inputs the budget
/// exists to handle. At 20 digits it is 43 columns.
///
/// "Characters" rather than "grapheme clusters" because the reader is
/// someone whose map did not open, and clusters are what they would
/// mean by characters anyway.
fn elision_notice(count: usize) -> String {
    let unit = if count == 1 { "character" } else { "characters" };
    format!("… {count} {unit} elided …")
}

/// Whether `message` already names `source`, so a caller about to put
/// the source in front of it should not.
///
/// One predicate for both surfaces on purpose. The canvas and the log
/// line are the same two pieces of text joined two different ways,
/// and the application's `startup_load::report_line` asks this before
/// prepending exactly as [`load_failure_text`] does — so "the source
/// appears once" is one decision rather than two that can drift.
///
/// The loader is why this is needed at all: exactly one of its
/// failure modes names the file — `load_from_file`'s *read* error,
/// which is what a mistyped path produces — and none of the others
/// do. Asking the message is therefore more honest than classifying
/// the error, and it stays right if a second message ever grows a
/// path.
///
/// **A substring is not a name, and the difference is user-visible.**
/// `mandala map` on a malformed file named `map` produces `Failed to
/// parse mindmap JSON: expected ident at line 1 column 2`, which
/// contains `map` — inside the word *mindmap*. A plain `contains`
/// answered yes, so the placard dropped its source line and the log
/// line dropped its prefix, and neither surface said which file had
/// failed. That is exactly the emptiness this module exists to
/// replace, reached by a shorter path.
///
/// So the occurrence has to be *bounded*: the characters on either
/// side of it must be ones that cannot continue a path. Alphanumerics
/// continue a name (`map` inside `mindmap`), and `_ - . / \ ~`
/// continue a path (`map` inside `map.mindmap.json`, or inside
/// `maps/map` — a different file from `map`, and one the message
/// therefore does not name). Everything else — whitespace, `:`,
/// quotes, the ends of the string — bounds it. A Windows `C:\maps\x`
/// still works: `:` bounds, and the drive letter is inside the source
/// rather than beside it.
///
/// **The residual, stated exactly.** A source whose whole name is a
/// word of the loader's own prose — a file called `mindmap`, `file`,
/// or `JSON` — is still read as named, and its source line is still
/// dropped. No purely textual predicate can do better: at that point
/// the message really does contain the source, bounded, and only
/// knowing that the loader wrote the word rather than interpolated
/// the path would separate them. Erring the other way costs a
/// duplicated path on a screen that already failed; erring this way
/// costs the filename entirely, so the bound is set where the
/// realistic collisions are and the exotic one is written down.
///
/// An empty `source` is never named: `"".contains("")` is true, and a
/// placard built from a blank source must still say what it has.
///
/// Cost: one substring search per occurrence of `source` in
/// `message`, and two character lookups per occurrence.
pub fn message_names_source(source: &str, message: &str) -> bool {
    if source.is_empty() {
        return false;
    }
    let mut from = 0usize;
    while let Some(offset) = message[from..].find(source) {
        let at = from + offset;
        let before = message[..at].chars().next_back();
        let after = message[at + source.len()..].chars().next();
        if !before.is_some_and(continues_a_path) && !after.is_some_and(continues_a_path) {
            return true;
        }
        // Past the whole occurrence, so an *overlapping* one is
        // deliberately not reached. It used to advance one character
        // instead, "so an overlapping one is still reached" — an
        // untested claim, and on inspection the wrong one. An
        // overlapping occurrence starts inside the previous one, so
        // the character before it is a character of `source` itself;
        // reaching it can only turn a rejection into a match, never
        // the reverse. But the loader interpolates the path once, and
        // its prose does not end in a prefix of the path, so no
        // *correct* match is reachable that way — only false ones,
        // which the residual below names as the expensive direction:
        // a false "named" drops the filename off the placard
        // entirely.
        from = at + source.len();
    }
    false
}

/// Whether `c` could be part of the same path as the character next
/// to it — the test [`message_names_source`] applies to the two
/// characters bounding a candidate occurrence.
fn continues_a_path(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '\\' | '~')
}

/// The placard's single node, sized to its own text and styled to
/// read as a rejection rather than as content.
fn placard_node(text: String) -> MindNode {
    let (width, height) = estimated_size(&text);
    let run = TextRun {
        start: 0,
        end: count_grapheme_clusters(&text),
        bold: false,
        italic: false,
        underline: false,
        font: PLACARD_FONT_FAMILY.to_string(),
        size_pt: PLACARD_FONT_SIZE_PT,
        color: PLACARD_ERROR_COLOR.to_string(),
        hyperlink: None,
    };
    MindNode {
        id: PLACARD_NODE_ID.to_string(),
        parent_id: None,
        position: Position { x: 0.0, y: 0.0 },
        size: Size { width, height },
        sections: vec![MindSection::new_default(text, vec![run])],
        style: NodeStyle {
            background_color: PLACARD_BACKGROUND_COLOR.to_string(),
            frame_color: PLACARD_ERROR_COLOR.to_string(),
            text_color: PLACARD_ERROR_COLOR.to_string(),
            shape: "rectangle".to_string(),
            corner_radius_percent: 10.0,
            frame_thickness: 4.0,
            show_frame: true,
            show_shadow: false,
            border: None,
        },
        layout: NodeLayout {
            layout_type: "map".to_string(),
            direction: "auto".to_string(),
            spacing: 50.0,
        },
        folded: false,
        notes: String::new(),
        color_schema: None,
        channel: 0,
        trigger_bindings: Vec::new(),
        inline_mutations: Vec::new(),
        inline_macros: Vec::new(),
        min_zoom_to_render: None,
        max_zoom_to_render: None,
    }
}

/// Canvas-unit box for `text` at the placard's point size, from the
/// widest line and the line count. An estimate by construction — see
/// [`PLACARD_CELL_WIDTH_FRAC`].
///
/// Cost: one grapheme walk per line of `text`.
fn estimated_size(text: &str) -> (f64, f64) {
    let widest = text.lines().map(grapheme_display_width).max().unwrap_or(0);
    let line_count = text.lines().count().max(1);
    let pt = f64::from(PLACARD_FONT_SIZE_PT);
    (
        widest as f64 * pt * PLACARD_CELL_WIDTH_FRAC,
        line_count as f64 * pt * PLACARD_LINE_HEIGHT_FRAC,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mindmap::loader;

    const SOURCE: &str = "/home/user/maps/broken.mindmap.json";
    const MESSAGE: &str = "node \"0.3\" ships zero sections — every renderable node needs at \
                           least one. Run `maptool convert --sections <file>` to migrate, or \
                           add an explicit `sections` array.";

    /// The whole reason the placard exists: the reader has to be
    /// able to find the loader's own words on it. Every non-blank
    /// run of the message must survive the wrap, so a reworded
    /// loader cannot quietly stop being reported.
    #[test]
    fn test_placard_reproduces_the_loader_message_in_full() {
        let text = load_failure_text(SOURCE, MESSAGE);
        let flattened = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let expected = MESSAGE.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            flattened.contains(&expected),
            "the placard dropped part of the loader message:\n{text}"
        );
    }

    /// The path the user asked for is on the placard too — "this map
    /// did not load" is not actionable when three maps are open in
    /// three windows.
    ///
    /// **Once**, whichever way the load failed. The read error a
    /// mistyped path produces already names the file, and it is the
    /// likeliest failure of all; a placard that also printed the
    /// source line above it read the path out twice, two lines apart.
    /// Both shapes are checked, so the fix for one cannot silently
    /// drop the other — a parse error names nothing, and dropping the
    /// source line for *it* would leave a placard that does not say
    /// which map died.
    #[test]
    fn test_placard_names_the_source_that_failed() {
        for (shape, message) in [
            ("parse", MESSAGE),
            (
                "read",
                concat!(
                    "Failed to read file /home/user/maps/broken.mindmap.json: ",
                    "No such file or directory (os error 2)"
                ),
            ),
        ] {
            let text = load_failure_text(SOURCE, message);
            let flattened = text.split_whitespace().collect::<Vec<_>>().join(" ");
            assert_eq!(
                flattened.matches(SOURCE).count(),
                1,
                "the {shape} failure names the source {} time(s):\n{text}",
                flattened.matches(SOURCE).count()
            );
            assert!(text.starts_with(PLACARD_HEADLINE));
            assert!(text.ends_with(PLACARD_FOOTER));
        }
    }

    /// The loader's parse message, which names no file. Short enough
    /// to collide with a short source, which is the point.
    const PARSE_MESSAGE: &str = "Failed to parse mindmap JSON: expected ident at line 1 column 2";

    /// **"Names" means bounded, not merely present.**
    ///
    /// The row that put this test here is the second one: `mandala
    /// map` on a malformed file called `map` matched the `map` inside
    /// *mindmap*, so both surfaces dropped the source and neither
    /// said which file had failed — the empty screen #107 is about,
    /// via a shorter route. Every row is a spelling of the same
    /// question, and the three the fix has to get right are a short
    /// source, a source that is a substring of a longer word, and a
    /// source that legitimately appears as its own path component.
    #[test]
    fn test_message_names_source_bounds_the_occurrence() {
        let read_error = concat!(
            "Failed to read file /home/user/maps/broken.mindmap.json: ",
            "No such file or directory (os error 2)"
        );
        let cases: &[(&str, &str, bool, &str)] = &[
            // A source that is its own path component in the message.
            (SOURCE, read_error, true, "the read error names the whole path"),
            // The regression: `map` inside `mindmap`.
            (
                "map",
                PARSE_MESSAGE,
                false,
                "a substring of a longer word is not a name",
            ),
            // The same short source, genuinely named.
            (
                "map",
                "Failed to read file map: No such file or directory (os error 2)",
                true,
                "a short source bounded by a space and a colon is named",
            ),
            // A path component of a *different* path.
            (
                "map",
                "Failed to read file maps/map.mindmap.json: No such file or directory (os error 2)",
                false,
                "`maps/map.mindmap.json` is a different file from `map`",
            ),
            // The same, with the separator as the *only* thing
            // saying so: drop `/` from the bounding set and every
            // other row here stays green while this one flips.
            (
                "map",
                "Failed to read file maps/map: No such file or directory (os error 2)",
                false,
                "`maps/map` is a different file from `map`, and only the `/` says so",
            ),
            // `.` continues a path, so a prefix of a longer filename
            // is not the filename.
            (
                "broken.json",
                "Failed to read file broken.json.bak: No such file or directory (os error 2)",
                false,
                "a prefix of a longer filename is not that filename",
            ),
            // The relative spelling a shell hands over.
            (
                "./map",
                PARSE_MESSAGE,
                false,
                "the parse message names no file at all",
            ),
            (
                "./map",
                "Failed to read file ./map: No such file or directory (os error 2)",
                true,
                "the read error names the relative path as given",
            ),
            // End of string bounds as well as whitespace does.
            (
                "broken.json",
                "cannot open broken.json",
                true,
                "the end of the message bounds",
            ),
            // A drive letter is inside the source, not beside it.
            (
                r"C:\maps\x.mindmap.json",
                r"Failed to read file C:\maps\x.mindmap.json: No such file or directory",
                true,
                "a Windows path is bounded by the space and the colon after it",
            ),
            // An occurrence overlapping a rejected one is not
            // reached, and must not be: the character before it is a
            // character of the source itself, so it can only ever
            // manufacture a match. Synthetic, because only a source
            // that overlaps itself can produce one — but the scan
            // advances by a whole occurrence for this reason, and a
            // scan that advanced by one character would answer
            // `true` here and drop the filename.
            (
                "a a",
                "za a a",
                false,
                "an overlapping occurrence is not a second mention, it is the same one",
            ),
            // A blank source has nothing to be named by.
            ("", PARSE_MESSAGE, false, "an empty source is never named"),
            (
                "",
                "",
                false,
                "an empty source is never named, even by an empty message",
            ),
            // The residual, asserted rather than merely described: a
            // file whose whole name is a word of the loader's prose.
            // No textual predicate separates this from a real mention.
            (
                "mindmap",
                PARSE_MESSAGE,
                true,
                "documented residual — a source named exactly like a word of the message",
            ),
        ];
        for (source, message, expected, why) in cases {
            assert_eq!(
                message_names_source(source, message),
                *expected,
                "message_names_source({source:?}, {message:?}) — {why}"
            );
        }
    }

    /// The end-to-end half of the row above: a short source the
    /// message does not name still reaches the canvas.
    ///
    /// The predicate is shared with `startup_load::report_line`, so
    /// this pins the surface a user sees rather than only the
    /// decision behind it — the regression was visible here first.
    #[test]
    fn test_placard_names_a_short_source_the_message_does_not() {
        let text = load_failure_text("map", PARSE_MESSAGE);
        let lines: Vec<&str> = text.lines().collect();
        assert!(
            lines.contains(&"map"),
            "the placard for a short source dropped it, and nothing on it says which \
             file failed:\n{text}"
        );
        assert!(
            text.contains(PARSE_MESSAGE),
            "the loader's own message must survive alongside the source:\n{text}"
        );
    }

    /// No line of the placard runs past its column, except where a
    /// single indivisible cluster is wider than the whole column —
    /// the one case [`wrap_to_display_width`] documents as
    /// over-running rather than dropping input.
    #[test]
    fn test_placard_respects_its_column_width() {
        let long_path = format!("/home/user/{}/map.mindmap.json", "d".repeat(200));
        for (source, message) in [
            (SOURCE, MESSAGE),
            (
                long_path.as_str(),
                "Failed to parse mindmap JSON: EOF while parsing a value at line 1 column 0",
            ),
            ("", ""),
        ] {
            let text = load_failure_text(source, message);
            for line in text.lines() {
                assert!(
                    grapheme_display_width(line) <= PLACARD_COLUMNS || count_grapheme_clusters(line) == 1,
                    "line {line:?} exceeds {PLACARD_COLUMNS} columns"
                );
            }
        }
    }

    /// **The placard is an ordinary map, not a special case.** It is
    /// serialized and fed back through the real loader, so every
    /// invariant the loader enforces — required keys, the
    /// zero-section rejection, the section cap, the parent-cycle
    /// screen, closed objects — is enforced against it too. Without
    /// this, a model change could turn the surface that reports a
    /// broken map into a second broken map.
    #[test]
    fn test_placard_is_a_map_the_loader_accepts() {
        let map = load_failure(SOURCE, MESSAGE);
        let json = serde_json::to_string(&map).expect("placard serializes");
        let reloaded = loader::load_from_str(&json).unwrap_or_else(|e| {
            panic!("the placard must be a loadable map: {e}\n{json}");
        });
        assert_eq!(reloaded.name, PLACARD_MAP_NAME);
        assert_eq!(reloaded.nodes.len(), 1);
        assert!(reloaded.edges.is_empty());
        let node = reloaded.nodes.get(PLACARD_NODE_ID).expect("placard node");
        assert_eq!(node.sections.len(), 1);
        assert!(node.parent_id.is_none());
    }

    /// A placard with an empty message is still a *renderable* map —
    /// the loader rejects a node whose text is nothing at all only
    /// via the zero-section rule, but a placard that showed a blank
    /// box would be the blank window #107 is about, restored by the
    /// back door.
    #[test]
    fn test_empty_message_still_produces_a_readable_placard() {
        let map = load_failure("", "");
        let node = map.nodes.get(PLACARD_NODE_ID).expect("placard node");
        let text = &node.sections[0].text;
        assert!(text.contains(PLACARD_HEADLINE));
        assert!(text.contains(PLACARD_FOOTER));
        assert!(node.size.width > 0.0 && node.size.height > 0.0);
    }

    /// The text run has to cover the whole text in grapheme units —
    /// a run that stops short leaves the tail unstyled, and the
    /// message is exactly the part that would go missing.
    #[test]
    fn test_placard_run_covers_every_grapheme_of_the_text() {
        let map = load_failure(SOURCE, "node \"0.3\" 👨‍👩‍👧 日本語 ships zero sections");
        let section = &map.nodes[PLACARD_NODE_ID].sections[0];
        assert_eq!(section.text_runs.len(), 1);
        assert_eq!(section.text_runs[0].start, 0);
        assert_eq!(section.text_runs[0].end, count_grapheme_clusters(&section.text));
    }

    /// **The placard's font pin is one the browser can honor too.**
    ///
    /// `format/fonts.md`: "the compiled-in set is the *portable*
    /// set, and anything outside it is a native-only accident." So a
    /// family pin is legitimate only when the face is compiled in —
    /// and the empty string, which clears the pin, always is.
    /// Anything else is a per-render `warn!` and a monospace
    /// fallback, on the one screen a user reaches after something
    /// has already gone wrong.
    ///
    /// This is a real gate rather than a tautology because a native
    /// `FontSystem::new()` also indexes the host's fontconfig: on a
    /// Linux desktop `app_font_by_family("Liberation Sans")` answers
    /// `Some`, and a test written against *that* would wave through
    /// a pin the browser cannot resolve. The roster here is
    /// `FONT_SOURCES` — what the binary carries — not what the
    /// machine running the suite happens to have installed.
    #[test]
    fn test_placard_font_pin_is_one_the_browser_can_honor() {
        use crate::font::fonts::{family_name_of, FONT_SOURCES};

        let map = load_failure(SOURCE, MESSAGE);
        let run = &map.nodes[PLACARD_NODE_ID].sections[0].text_runs[0];
        assert_eq!(run.font, PLACARD_FONT_FAMILY);

        let compiled_in: Vec<&'static str> =
            FONT_SOURCES.keys().copied().filter_map(family_name_of).collect();
        assert!(
            run.font.is_empty() || compiled_in.contains(&run.font.as_str()),
            "the placard pins {:?}, which is not compiled in. Either bundle the face or \
             clear the pin with the empty string — an unresolvable family is a `warn!` \
             per render and a monospace fallback, and it differs between native and the \
             browser. Compiled-in families: {compiled_in:?}",
            run.font
        );
    }

    /// Everything of `text` with the whitespace taken out — the wrap
    /// is free to break a long unbroken run anywhere, so a run that
    /// survived it is only recoverable this way.
    fn without_whitespace(text: &str) -> String {
        text.chars().filter(|c| !c.is_whitespace()).collect()
    }

    /// **[`load_failure`]'s round-trip promise, against the input
    /// that used to falsify it.**
    ///
    /// serde reproduces the offending JSON value verbatim when it
    /// wanted a number and got a string, so the message is exactly as
    /// long as the file made it. At 4 MB the unbounded placard was
    /// 62 508 lines and 1 950 250 canvas units tall — over
    /// `MAX_NODE_AXIS`, so `load_from_str` refused the very map that
    /// exists to report a refusal, and the application's box clamp
    /// then cropped the message rather than the box.
    ///
    /// Both ends have to survive: the head carries what went wrong,
    /// the tail carries where.
    #[test]
    fn test_placard_bounds_a_message_whose_length_the_file_supplies() {
        use crate::mindmap::model::MAX_NODE_AXIS;

        let quoted = "A".repeat(4_000_000);
        let message = format!("node \"0\": invalid type: string \"{quoted}\", expected f64");
        let text = load_failure_text(SOURCE, &message);

        assert!(
            text.contains("invalid type: string"),
            "the head of the message — what went wrong — did not survive:\n{}",
            &text[..200.min(text.len())]
        );
        assert!(
            text.contains("expected f64"),
            "the tail of the message — where it went wrong — did not survive"
        );
        assert!(
            text.contains("characters elided"),
            "the placard elided silently; a reader has to be told the message is not whole"
        );
        for line in text.lines() {
            assert!(
                grapheme_display_width(line) <= PLACARD_COLUMNS || count_grapheme_clusters(line) == 1,
                "line {line:?} exceeds {PLACARD_COLUMNS} columns"
            );
        }

        let map = load_failure(SOURCE, &message);
        let node = &map.nodes[PLACARD_NODE_ID];
        assert!(
            node.size.height <= MAX_NODE_AXIS && node.size.width <= MAX_NODE_AXIS,
            "the placard is {} x {}, past the {MAX_NODE_AXIS} ceiling",
            node.size.width,
            node.size.height
        );
        let json = serde_json::to_string(&map).expect("placard serializes");
        loader::load_from_str(&json).unwrap_or_else(|e| {
            panic!("a 4 MB loader message must still produce a loadable placard: {e}");
        });
    }

    /// **The same hole on the other text.** The source is not this
    /// module's own words either: in the browser it is whatever the
    /// `?map=` of the link said, so a crafted link supplies its
    /// length exactly as a crafted file supplies the message's.
    ///
    /// Separate from the message case rather than folded into it,
    /// because the two are elided by two calls and a test that
    /// exercised only one would leave the other free to be deleted.
    #[test]
    fn test_placard_bounds_a_source_whose_length_the_url_supplies() {
        use crate::mindmap::model::MAX_NODE_AXIS;

        let source = format!("https://example.test/{}.mindmap.json", "u".repeat(4_000_000));
        let text = load_failure_text(&source, PARSE_MESSAGE);
        assert!(text.contains("https://example.test/"), "the URL's head is gone");
        assert!(text.contains(".mindmap.json"), "the URL's tail is gone");
        assert!(text.contains("characters elided"), "the URL was elided silently");
        assert!(
            text.contains(PARSE_MESSAGE),
            "the loader's own message must survive alongside an over-long source"
        );

        let map = load_failure(&source, PARSE_MESSAGE);
        let node = &map.nodes[PLACARD_NODE_ID];
        assert!(
            node.size.height <= MAX_NODE_AXIS,
            "the placard is {} units tall, past the {MAX_NODE_AXIS} ceiling",
            node.size.height
        );
        let json = serde_json::to_string(&map).expect("placard serializes");
        loader::load_from_str(&json).unwrap_or_else(|e| {
            panic!("a 4 MB `?map=` value must still produce a loadable placard: {e}");
        });
    }

    /// The budget is a bound, not an edit: one cluster under it and
    /// the message is whole, one cluster over it and exactly one
    /// cluster is gone.
    ///
    /// The second half is what keeps the first honest — a budget that
    /// never fired would pass the whole-message assertion vacuously.
    #[test]
    fn test_the_message_is_whole_up_to_the_budget_and_elided_one_past_it() {
        let kept = PLACARD_HEAD_CLUSTERS + PLACARD_TAIL_CLUSTERS;

        let exact = "b".repeat(kept);
        let text = load_failure_text("s", &exact);
        assert!(
            without_whitespace(&text).contains(&exact),
            "a message exactly at the budget lost something"
        );
        assert!(
            !text.contains("elided"),
            "a message exactly at the budget was announced as elided"
        );

        let over = format!("{}c", "b".repeat(kept));
        let text = load_failure_text("s", &over);
        assert!(
            text.contains(&elision_notice(1)),
            "one cluster past the budget must report exactly one elided:\n{}",
            text.lines().take(4).collect::<Vec<_>>().join("\n")
        );
        // On its own line, not folded into the loader's words on
        // either side of it — the notice is this module's voice and
        // has to read as an interruption rather than as content.
        //
        // Asked of `elide_middle` rather than of the placard,
        // because the placard's answer is luck: `PLACARD_COLUMNS`
        // divides the head budget exactly, so a notice spliced
        // inline with spaces still lands alone on a wrapped line.
        // The hard newlines are what makes it true for every budget.
        assert!(
            elide_middle(&over).lines().any(|line| line == elision_notice(1)),
            "the notice must be its own line before the wrap ever sees it"
        );
        assert!(
            without_whitespace(&text).ends_with(&format!("c{}", without_whitespace(PLACARD_FOOTER))),
            "the last cluster of an elided message is the one that must not be dropped"
        );
    }

    /// **The budget's whole job, stated as the arithmetic it rests
    /// on.** The tallest placard the budget can produce must stay
    /// under `MAX_NODE_AXIS`, or [`load_failure`]'s promise is
    /// conditional again.
    ///
    /// One cluster per line is the pathological wrap — the only input
    /// whose line count equals its cluster count — so a text of bare
    /// newlines at the budget is the worst case, on both the source
    /// and the message at once.
    ///
    /// The second assertion pins the breach point the module docs
    /// name. It is derived from the point size and the line advance,
    /// so changing either without revisiting the prose fails here
    /// rather than silently making the docs wrong.
    #[test]
    fn test_the_tallest_placard_the_budget_allows_stays_under_the_axis_ceiling() {
        use crate::mindmap::model::MAX_NODE_AXIS;

        let line_advance = f64::from(PLACARD_FONT_SIZE_PT) * PLACARD_LINE_HEIGHT_FRAC;
        let breach_at = (MAX_NODE_AXIS / line_advance).floor() as usize + 1;
        assert_eq!(
            breach_at, 32_052,
            "the module docs name 32 052 as the first line count that breaches the ceiling"
        );
        // And it is the real threshold rather than only the
        // arithmetic: the box a placard of that many lines asks for
        // is over the ceiling, and one line fewer is not.
        let at_breach = "z\n".repeat(breach_at - 1) + "z";
        assert!(estimated_size(&at_breach).1 > MAX_NODE_AXIS);
        let below_breach = "z\n".repeat(breach_at - 2) + "z";
        assert!(estimated_size(&below_breach).1 <= MAX_NODE_AXIS);

        let kept = PLACARD_HEAD_CLUSTERS + PLACARD_TAIL_CLUSTERS;
        // Distinct, so the message does not name the source and both
        // texts land on the placard together.
        let source = "\n".repeat(kept + 1);
        let message = format!("x{}", "\n".repeat(kept));
        let text = load_failure_text(&source, &message);
        assert!(
            text.lines().count() < breach_at,
            "the worst case the budget allows is {} lines, past the {breach_at} the ceiling \
             permits",
            text.lines().count()
        );

        let map = load_failure(&source, &message);
        let node = &map.nodes[PLACARD_NODE_ID];
        assert!(
            node.size.height <= MAX_NODE_AXIS,
            "the worst case is {} units tall, past the {MAX_NODE_AXIS} ceiling",
            node.size.height
        );
        let json = serde_json::to_string(&map).expect("placard serializes");
        loader::load_from_str(&json).unwrap_or_else(|e| {
            panic!("the worst case the budget allows must still be a loadable map: {e}");
        });
    }

    /// The notice is pushed onto the placard as a whole line rather
    /// than wrapped, so it has to fit the column on its own — and for
    /// every count, not just the plausible ones, since the counts
    /// that reach it are the ones a file chose.
    #[test]
    fn test_the_elision_notice_fits_the_placard_column() {
        for count in [0usize, 1, 4_000_000, usize::MAX] {
            let notice = elision_notice(count);
            assert!(
                grapheme_display_width(&notice) <= PLACARD_COLUMNS,
                "the elision notice for {count} is {} columns wide, past {PLACARD_COLUMNS}: \
                 {notice:?}",
                grapheme_display_width(&notice)
            );
        }
        assert!(
            elision_notice(1).contains(" 1 character "),
            "a one-cluster elision reads as one character, not one characters"
        );
    }

    /// The node box grows with the message. A fixed box would clip a
    /// long diagnosis against exactly the failure mode that produces
    /// the longest ones.
    #[test]
    fn test_placard_box_grows_with_the_message() {
        let short = load_failure(SOURCE, "no");
        let long = load_failure(SOURCE, &"a word ".repeat(200));
        let short_node = &short.nodes[PLACARD_NODE_ID];
        let long_node = &long.nodes[PLACARD_NODE_ID];
        assert!(
            long_node.size.height > short_node.size.height,
            "a longer message must produce a taller placard"
        );
    }
}
