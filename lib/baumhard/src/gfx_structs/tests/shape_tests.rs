// SPDX-License-Identifier: MPL-2.0

//! Tests for [`crate::gfx_structs::shape`] — the `NodeShape` enum
//! and its point-in-shape / shape-vs-AABB primitives.
//!
//! Follows the `do_*()` / `test_*()` split from
//! [`TEST_CONVENTIONS.md §T2.2`] so the criterion bench harness at
//! `lib/baumhard/benches/test_bench.rs` can reuse each body as a
//! micro-benchmark. The shape math sits on the BVH hot path
//! (`bvh_descend`), so the rect-pipeline SDF in the renderer and
//! the lasso / editor hit-tests all pay for it every interaction
//! frame — benching is §B8-mandatory, not optional.

use glam::Vec2;
use strum::IntoEnumIterator;

use crate::gfx_structs::shape::{
    NodeShape, ShapeReport, ShapeSpelling, KNOWN_SHAPES, SHAPE_ID_ELLIPSE, SHAPE_ID_RECTANGLE,
};

/// Parse every documented shape spelling, plus the `"circle"`
/// alias and the empty-string fallback. Pins the author-facing
/// vocabulary: changing a spelling here would silently change
/// which JSON maps still render as the intended shape.
#[test]
pub fn test_shape_from_style_string_known_names() {
    do_shape_from_style_string_known_names();
}

pub fn do_shape_from_style_string_known_names() {
    assert_eq!(NodeShape::from_style_string("rectangle"), NodeShape::Rectangle);
    assert_eq!(NodeShape::from_style_string("Rectangle"), NodeShape::Rectangle);
    assert_eq!(NodeShape::from_style_string("ellipse"), NodeShape::Ellipse);
    assert_eq!(NodeShape::from_style_string("ELLIPSE"), NodeShape::Ellipse);
    // "circle" is accepted as a convenience alias for the
    // Ellipse variant — a `width == height` ellipse *is* a
    // circle, and authors will often type this.
    assert_eq!(NodeShape::from_style_string("circle"), NodeShape::Ellipse);
}

/// An empty or unknown string falls back to the default
/// `Rectangle` variant. Mirrors how `tree_builder/node.rs` treats
/// malformed background hex: survive a typo rather than crash the
/// render. Which of these is *reported* is
/// `test_shape_classification_partitions_by_warning`'s subject —
/// the resolved shape is the same either way.
#[test]
pub fn test_shape_from_style_string_empty_and_unknown_fall_back_to_rectangle() {
    do_shape_from_style_string_empty_and_unknown_fall_back_to_rectangle();
}

pub fn do_shape_from_style_string_empty_and_unknown_fall_back_to_rectangle() {
    assert_eq!(NodeShape::from_style_string(""), NodeShape::Rectangle);
    assert_eq!(NodeShape::from_style_string("diamond"), NodeShape::Rectangle);
    assert_eq!(NodeShape::from_style_string("zigzag"), NodeShape::Rectangle);
}

/// **Issue #118.** Every canonical spelling — iterated from
/// `KNOWN_SHAPES`, never re-spelled here — is a shape the format
/// publishes, `maptool convert --legacy` emits and `maptool verify`
/// accepts. None of them may be reported as unknown, whether or not
/// a `NodeShape` variant draws it yet. `"hexagon"` is the one that
/// produced 242 warnings per load of `maps/testament.mindmap.json`.
///
/// Iterating is the point: a spelling added to `KNOWN_SHAPES`
/// tomorrow is covered by this test the moment it lands, with
/// nobody editing the parser or this file.
#[test]
pub fn test_shape_every_known_spelling_is_non_warning() {
    do_shape_every_known_spelling_is_non_warning();
}

pub fn do_shape_every_known_spelling_is_non_warning() {
    assert!(
        !KNOWN_SHAPES.is_empty(),
        "KNOWN_SHAPES is empty — this test would pass vacuously"
    );
    for known in KNOWN_SHAPES {
        let spelling = ShapeSpelling::classify(known);
        assert!(
            !spelling.is_author_error(),
            "canonical shape {known:?} classified as {spelling:?}, which warns"
        );
        assert_ne!(
            spelling,
            ShapeSpelling::Unspecified,
            "canonical shape {known:?} must not classify as unset"
        );
    }
}

/// The classification of every canonical spelling, derived from the
/// two sources of truth rather than restated: a spelling one of the
/// `NodeShape` variants claims is `Rendered`, and every other
/// `KNOWN_SHAPES` entry is `KnownNotYetRendered`. This is the pin
/// that fails when a `KNOWN_SHAPES` entry is added that the
/// classifier does not account for, and equally when a spelling
/// silently changes sides.
#[test]
pub fn test_shape_classification_partitions_by_warning() {
    do_shape_classification_partitions_by_warning();
}

pub fn do_shape_classification_partitions_by_warning() {
    for known in KNOWN_SHAPES {
        let claimed_by = NodeShape::iter().find(|shape| shape.style_spellings().contains(known));
        let expected = match claimed_by {
            Some(shape) => ShapeSpelling::Rendered(shape),
            None => ShapeSpelling::KnownNotYetRendered,
        };
        assert_eq!(
            ShapeSpelling::classify(known),
            expected,
            "canonical shape {known:?} is not classified the way the \
             NodeShape variant set says it should be"
        );
    }
    // Both halves must be non-empty, or the partition above is
    // asserting nothing about one of them. Today: three rendered
    // spellings, four awaiting a shader case.
    let rendered = KNOWN_SHAPES
        .iter()
        .filter(|known| matches!(ShapeSpelling::classify(known), ShapeSpelling::Rendered(_)))
        .count();
    assert!(rendered > 0, "no canonical spelling maps to a NodeShape variant");
    assert!(
        rendered < KNOWN_SHAPES.len(),
        "every canonical spelling is rendered — the KnownNotYetRendered \
         half of this test is now vacuous and needs rewriting"
    );
}

/// A spelling no `NodeShape` variant claims may still be canonical,
/// but a spelling that claims to be canonical while sitting outside
/// `KNOWN_SHAPES` is a bug in the other direction: `maptool verify`
/// would reject a map the runtime renders correctly. Exhaustive over
/// the variants via `EnumIter`, so a new variant is covered
/// automatically.
#[test]
pub fn test_shape_variant_spellings_are_all_known() {
    do_shape_variant_spellings_are_all_known();
}

pub fn do_shape_variant_spellings_are_all_known() {
    for shape in NodeShape::iter() {
        assert!(
            !shape.style_spellings().is_empty(),
            "{shape:?} declares no canonical spelling, so no map can ask for it"
        );
        for spelling in shape.style_spellings() {
            assert!(
                KNOWN_SHAPES.contains(spelling),
                "{shape:?} claims spelling {spelling:?}, which is not in \
                 KNOWN_SHAPES — maptool verify would reject it"
            );
            assert_eq!(
                ShapeSpelling::classify(spelling),
                ShapeSpelling::Rendered(shape),
                "{spelling:?} does not classify back to {shape:?}"
            );
        }
    }
}

/// `KNOWN_SHAPES` entries are written lowercase, and nothing in the
/// type system says so.
///
/// **Not** because the runtime and `maptool verify` would otherwise
/// disagree — both compare with `eq_ignore_ascii_case` against this
/// same list (`ShapeSpelling::classify` here, `check_value` in
/// `crates/maptool/src/verify/enums.rs`), so they agree on case by
/// construction and `"HEXAGON"` in the list still verifies clean.
///
/// What the invariant protects is the two case-*sensitive*
/// `slice::contains` calls in the sibling tests below:
/// `do_shape_classification_partitions_by_warning` asks whether a
/// variant's `style_spellings` contains the entry, and
/// `do_shape_variant_spellings_are_all_known` asks the mirror
/// question of `KNOWN_SHAPES`. Uppercase a variant-claimed spelling
/// and it silently changes sides of the rendered / not-yet-rendered
/// partition; both go red, and this test is what names the cause
/// rather than leaving two derived failures neither of which says
/// "the list is mixed case".
#[test]
pub fn test_shape_known_shapes_are_lowercase() {
    do_shape_known_shapes_are_lowercase();
}

pub fn do_shape_known_shapes_are_lowercase() {
    for known in KNOWN_SHAPES {
        assert_eq!(
            *known,
            known.to_ascii_lowercase().as_str(),
            "KNOWN_SHAPES entry {known:?} is not lowercase"
        );
    }
}

/// The other half of issue #118: a value that really is unknown
/// stays reported. `"pentagram"` is nowhere in `KNOWN_SHAPES`, so it
/// is a typo or a value from a newer build — exactly the case the
/// warning was written for. The empty string is *not* that case;
/// it means the field was left unset.
///
/// Also pins `is_author_error` one classification at a time, since
/// it is the predicate every other test in this file asks. A blanket
/// `true` or `false` there would otherwise make several of them pass
/// for the wrong reason.
#[test]
pub fn test_shape_unrecognized_spelling_still_warns() {
    do_shape_unrecognized_spelling_still_warns();
}

pub fn do_shape_unrecognized_spelling_still_warns() {
    let spelling = ShapeSpelling::classify("pentagram");
    assert_eq!(spelling, ShapeSpelling::Unrecognized);
    assert!(spelling.is_author_error());
    assert_eq!(spelling.resolve(), NodeShape::Rectangle);
    assert_eq!(NodeShape::from_style_string("pentagram"), NodeShape::Rectangle);

    // Unset is silent, and has been since before #118.
    assert_eq!(ShapeSpelling::classify(""), ShapeSpelling::Unspecified);
    assert!(!ShapeSpelling::classify("").is_author_error());

    assert!(ShapeSpelling::Unrecognized.is_author_error());
    assert!(!ShapeSpelling::Unspecified.is_author_error());
    assert!(!ShapeSpelling::KnownNotYetRendered.is_author_error());
    for shape in NodeShape::iter() {
        assert!(
            !ShapeSpelling::Rendered(shape).is_author_error(),
            "{shape:?} renders as asked, so there is nothing to report"
        );
    }
}

/// The two library-owned reporting predicates, pinned **as a pair**
/// for every classification. `from_style_string` calls
/// `is_author_error` to reach `log::warn!` and `is_quiet_fallback` to
/// reach `log::trace!`, so between them these two booleans are the
/// entire reporting contract of the shipped loader — not a test-only
/// restatement of it.
///
/// Exhaustive by construction: the expectation comes out of a `match`
/// with no `_` arm, so a fifth `ShapeSpelling` variant fails to
/// compile here rather than defaulting into silence at the log site.
///
/// Disjointness is asserted too. A classification answering `true` to
/// both would make what a load actually logs depend on the order
/// `from_style_string` happens to test them in, which is exactly the
/// kind of coupling the split was meant to remove.
#[test]
pub fn test_shape_reporting_predicates_partition() {
    do_shape_reporting_predicates_partition();
}

pub fn do_shape_reporting_predicates_partition() {
    let mut cases = vec![
        ShapeSpelling::Unspecified,
        ShapeSpelling::KnownNotYetRendered,
        ShapeSpelling::Unrecognized,
    ];
    cases.extend(NodeShape::iter().map(ShapeSpelling::Rendered));
    assert!(
        cases.len() > 3,
        "no NodeShape variant produced a Rendered case — the loop below \
         would never exercise one"
    );

    for spelling in cases {
        let expected = match spelling {
            ShapeSpelling::Unrecognized => (true, false),
            ShapeSpelling::KnownNotYetRendered => (false, true),
            ShapeSpelling::Unspecified | ShapeSpelling::Rendered(_) => (false, false),
        };
        assert_eq!(
            (spelling.is_author_error(), spelling.is_quiet_fallback()),
            expected,
            "{spelling:?} answers the wrong (is_author_error, \
             is_quiet_fallback) pair, which is what from_style_string logs on"
        );
        assert!(
            !(spelling.is_author_error() && spelling.is_quiet_fallback()),
            "{spelling:?} is both an author error and a quiet fallback, so \
             what a load logs would depend on branch order"
        );
    }
}

/// **The routing decision, held as data.**
///
/// `ShapeSpelling::report` is the value
/// `NodeShape::from_style_string` matches on, so this test observes
/// the two reporting predicates the way the loader does rather than
/// only asking them the same question directly. That distinction is
/// the round-3 correction to a round-2 claim: before `report`
/// existed, the predicates' one production consumer was a pair of
/// `if`s whose only effect was a log line, and with no log sink in
/// this crate nothing could reach them through the call the loader
/// makes. Hard-wiring `is_author_error` now changes a value this test
/// holds; all that is left over is which macro carries the result,
/// which `log_routing` pins as the source fact it is.
///
/// Exhaustive by construction: the expectation comes out of a `match`
/// with no `_` arm, so a fifth `ShapeSpelling` fails to compile here.
#[test]
pub fn test_shape_report_routes_every_classification() {
    do_shape_report_routes_every_classification();
}

pub fn do_shape_report_routes_every_classification() {
    let mut cases = vec![
        ShapeSpelling::Unspecified,
        ShapeSpelling::KnownNotYetRendered,
        ShapeSpelling::Unrecognized,
    ];
    cases.extend(NodeShape::iter().map(ShapeSpelling::Rendered));
    assert!(
        cases.len() > 3,
        "no NodeShape variant produced a Rendered case — the loop below \
         would never exercise one"
    );

    for spelling in cases {
        let expected = match spelling {
            ShapeSpelling::Unrecognized => Some(ShapeReport::UnknownSpelling),
            ShapeSpelling::KnownNotYetRendered => Some(ShapeReport::RectangleSubstituted),
            ShapeSpelling::Unspecified | ShapeSpelling::Rendered(_) => None,
        };
        assert_eq!(
            spelling.report(),
            expected,
            "{spelling:?} routes to the wrong report, which is the value \
             from_style_string picks its log macro from"
        );
    }

    // Through the strings the loader actually hands it, so a change
    // to `classify` shows up here too and not only in the variant
    // table above. `"hexagon"` is the #118 spelling; `"pentagram"` is
    // the one that must still be reported.
    assert_eq!(
        ShapeSpelling::classify("pentagram").report(),
        Some(ShapeReport::UnknownSpelling)
    );
    assert_eq!(
        ShapeSpelling::classify("hexagon").report(),
        Some(ShapeReport::RectangleSubstituted)
    );
    assert_eq!(ShapeSpelling::classify("").report(), None);
    assert_eq!(ShapeSpelling::classify("ellipse").report(), None);
}

/// **Which level each report is written at, and why those two.**
///
/// `log_routing` holds `from_style_string`'s arms to whatever
/// `ShapeReport::level` says, so `level` is where being *right* has
/// to be asserted — a pin against a wrong definition is faithful and
/// useless.
///
/// The two claims are separate on purpose. The per-variant table is
/// the exact mapping. The two comparisons under it are the reason it
/// is that mapping: CODE_CONVENTIONS §9 caps release builds at
/// `warn`, so a report at or above `Warn` reaches a user's bug report
/// and one below it costs nothing in the binaries `./build.sh` ships.
/// Issue #118 was the second kind filed under the first — 242 lines
/// per load of the demo map telling authors a documented, tool-
/// emitted spelling was unknown.
#[test]
pub fn test_shape_report_levels_are_warn_and_trace() {
    do_shape_report_levels_are_warn_and_trace();
}

pub fn do_shape_report_levels_are_warn_and_trace() {
    assert!(
        ShapeReport::iter().count() > 0,
        "ShapeReport has no variants — the loop below would not run"
    );
    for report in ShapeReport::iter() {
        let expected = match report {
            ShapeReport::UnknownSpelling => log::Level::Warn,
            ShapeReport::RectangleSubstituted => log::Level::Trace,
        };
        assert_eq!(
            report.level(),
            expected,
            "{report:?} is written at the wrong level, and log_routing pins \
             from_style_string's macro to whatever this says"
        );
    }

    // `log::Level` orders by severity, `Error` first, so "at least as
    // severe as `Warn`" is `<=`.
    assert!(
        ShapeReport::UnknownSpelling.level() <= log::Level::Warn,
        "an unknown spelling is something an author can fix, so it has to \
         survive the release cap and reach a bug report"
    );
    assert!(
        ShapeReport::RectangleSubstituted.level() > log::Level::Warn,
        "a canonical spelling awaiting its shader case is the format working \
         as designed; reporting it above the release cap is issue #118"
    );
}

/// The three spellings named in the issue, asserted one at a time
/// because each pins a different property: `"HEXAGON"` that the
/// canonical-but-unrendered path is case-insensitive (uppercase is
/// how the warning was first noticed to be spelling-sensitive),
/// `"Circle"` that the alias survives mixed case, `""` that unset
/// is still silent.
#[test]
pub fn test_shape_classify_case_and_alias() {
    do_shape_classify_case_and_alias();
}

pub fn do_shape_classify_case_and_alias() {
    assert_eq!(
        ShapeSpelling::classify("HEXAGON"),
        ShapeSpelling::KnownNotYetRendered
    );
    assert_eq!(
        ShapeSpelling::classify("hexagon"),
        ShapeSpelling::KnownNotYetRendered
    );
    assert_eq!(NodeShape::from_style_string("HEXAGON"), NodeShape::Rectangle);

    assert_eq!(
        ShapeSpelling::classify("Circle"),
        ShapeSpelling::Rendered(NodeShape::Ellipse)
    );
    assert_eq!(
        ShapeSpelling::classify("CIRCLE"),
        ShapeSpelling::Rendered(NodeShape::Ellipse)
    );
    assert_eq!(NodeShape::from_style_string("Circle"), NodeShape::Ellipse);

    assert_eq!(
        ShapeSpelling::classify("RECTANGLE"),
        ShapeSpelling::Rendered(NodeShape::Rectangle)
    );
    assert_eq!(
        ShapeSpelling::classify("Ellipse"),
        ShapeSpelling::Rendered(NodeShape::Ellipse)
    );

    assert_eq!(ShapeSpelling::classify(""), ShapeSpelling::Unspecified);
    assert_eq!(ShapeSpelling::classify("").resolve(), NodeShape::Rectangle);
}

/// **`format/enums.md` restates `KNOWN_SHAPES`, so the doc is read
/// rather than trusted.**
///
/// `shape.rs` calls its list the only one a parser consults, which is
/// true, and the PR that introduced the classifier went further and
/// called it the only list in the tree, which was not: the format doc
/// publishes the vocabulary three times over in its `style.shape`
/// section — once as the canonical fence, once as the "live shapes"
/// sentence, once as the "remaining values" parenthesis — and nothing
/// tied any of them to the constant. Drop or rename a `KNOWN_SHAPES`
/// entry and the doc keeps publishing the old spelling to authors who
/// will then write maps `maptool verify` rejects.
///
/// Each of the three is checked against a set *derived* from the code
/// rather than restated here: the fence against `KNOWN_SHAPES` in
/// order, the live sentence against the canonical spelling of each
/// `NodeShape` variant, the remaining parenthesis against the entries
/// that classify `KnownNotYetRendered`. The three are then required
/// to cover `KNOWN_SHAPES` between them, so an entry cannot be
/// dropped from the section as a whole either.
///
/// The converter's copy of the same vocabulary is pinned on maptool's
/// side, by `legacy_shape_ordinals_are_canonical_spellings` — it lives
/// there because baumhard does not depend on `maptool`.
#[test]
pub fn test_shape_format_doc_publishes_exactly_known_shapes() {
    do_shape_format_doc_publishes_exactly_known_shapes();
}

pub fn do_shape_format_doc_publishes_exactly_known_shapes() {
    let section = documented_shape_section();

    // 1. The canonical fenced list, in order.
    let fence = fenced_spellings(&section);
    assert_eq!(
        fence,
        KNOWN_SHAPES.to_vec(),
        "the `style.shape` code fence in format/enums.md no longer \
         publishes exactly KNOWN_SHAPES, in order"
    );

    // 2. The "live shapes" sentence: one canonical spelling per
    //    drawable variant, in variant order.
    let live = backticked_spellings(paragraph_before(&section, "are **live shapes**"));
    let expected_live: Vec<&str> = NodeShape::iter()
        .map(|shape| {
            *shape
                .style_spellings()
                .first()
                .expect("every variant declares a canonical spelling")
        })
        .collect();
    assert_eq!(
        live, expected_live,
        "format/enums.md's live-shapes sentence and the NodeShape \
         variant set disagree about what the renderer can draw"
    );

    // 3. The "remaining values" parenthesis: exactly the canonical
    //    spellings with no variant behind them yet.
    let remaining = backticked_spellings(parenthesized_after(&section, "The remaining values ("));
    let expected_remaining: Vec<&str> = KNOWN_SHAPES
        .iter()
        .copied()
        .filter(|known| ShapeSpelling::classify(known) == ShapeSpelling::KnownNotYetRendered)
        .collect();
    assert_eq!(
        remaining, expected_remaining,
        "format/enums.md's \"remaining values\" list and the classifier \
         disagree about which canonical spellings are not drawn yet"
    );

    // 4. Between them the two prose lists plus the aliases must cover
    //    the whole constant, so no entry can quietly leave the
    //    section.
    for known in KNOWN_SHAPES {
        // The alias case (`"circle"`) is named in prose rather than
        // in either list, so a bare code span counts as coverage.
        let code_span = format!("`{known:?}`");
        let covered = live.contains(known) || remaining.contains(known) || section.contains(&code_span);
        assert!(
            covered,
            "canonical spelling {known:?} is published by KNOWN_SHAPES but \
             appears nowhere in format/enums.md's `style.shape` section"
        );
    }
}

/// The body of `format/enums.md`'s `### \`style.shape\`` section.
///
/// Panics rather than returning empty if the heading is gone — a
/// silent fallback would let this whole test pass vacuously, which is
/// the failure mode it exists to prevent.
fn documented_shape_section() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../format/enums.md");
    let doc = std::fs::read_to_string(path).expect("format/enums.md must be readable");
    shape_section_in(&doc)
}

/// [`documented_shape_section`] over supplied text, so where the
/// section starts and where it ends are controllable rather than only
/// exercised against the one document on disk. This is the same split
/// maptool's `shape_ordinals_in` carries, for the same two reasons.
///
/// **The heading has to occur once.** `split_once` takes the first
/// occurrence, so a correct copy of the section planted above the real
/// one shadows it and every assertion below runs against the copy.
///
/// **The section ends at the next heading of any level.** It used to
/// end at the next `### ` — an h3, the level it starts from — so a
/// sibling h2 did not end it and the next chapter's prose would have
/// been read as part of `style.shape`. That is the same asymmetry
/// maptool's reader had in mirror image, and it is fixed here for the
/// same reason: what level the *next* section's heading is at is not
/// something this reader should have an opinion about.
fn shape_section_in(doc: &str) -> String {
    let heading = "### `style.shape`";
    let headings = doc.matches(heading).count();
    assert_eq!(
        headings, 1,
        "format/enums.md must document `style.shape` under exactly one \
         {heading:?} heading; it has {headings}, and this reader would take \
         the first"
    );
    let after = doc.split_once(heading).expect("the heading was just counted").1;
    let mut section = String::new();
    // `skip(1)` drops what is left of the heading's own line.
    for line in after.lines().skip(1) {
        if line.trim_start().starts_with('#') {
            break;
        }
        section.push_str(line);
        section.push('\n');
    }
    section
}

/// **The section start, controlled.** `split_once` takes the first
/// occurrence, so a copy of the heading above the real section shadows
/// it: the copy can publish the vocabulary `KNOWN_SHAPES` has while
/// the real section below publishes anything at all.
///
/// `#[should_panic]` and so no `do_*()` body or bench entry — §B8's
/// class 2 (drives a panic), the same carve-out
/// `region_indexer_tests`'s two out-of-bounds tests take: a
/// benchmark of a panicking body is not a benchmark.
#[test]
#[should_panic(expected = "exactly one")]
fn test_shape_section_rejects_a_duplicated_heading() {
    shape_section_in(
        "\
### `style.shape`

```
\"rectangle\"
```

### `style.shape`

```
\"hexagoon\"
```
",
    );
}

/// **The section boundary, controlled.** A sibling `##` ends the
/// `style.shape` section. Before this was fixed the reader stopped
/// only at an h3 — the level it starts from — so the next *chapter*
/// did not end it, the mirror image of the h2-only stop
/// `shape_ordinals_in` had on maptool's side.
#[test]
pub fn test_shape_section_stops_at_the_next_heading_of_any_level() {
    do_shape_section_stops_at_the_next_heading_of_any_level();
}

pub fn do_shape_section_stops_at_the_next_heading_of_any_level() {
    let doc = "\
### `style.shape`

in the section

## The next chapter

### `style.shape_but_not`

not in the section
";
    assert_eq!(
        shape_section_in(doc),
        "\nin the section\n\n",
        "the reader ran past the sibling h2 and harvested the next chapter"
    );
}

/// The `"quoted"` spellings inside the section's first fenced code
/// block, in source order.
fn fenced_spellings(section: &str) -> Vec<&str> {
    let after = section
        .split_once("```")
        .expect("the `style.shape` section must open a code fence")
        .1;
    let fence = after
        .split_once("```")
        .expect("the `style.shape` code fence must be closed")
        .0;
    quoted_spans(fence)
}

/// The part of the paragraph that runs up to `marker` — everything
/// after the last blank line preceding it. Scoping to one paragraph
/// is what keeps the fenced list and neighboring prose out of a
/// sentence-level assertion.
fn paragraph_before<'a>(section: &'a str, marker: &str) -> &'a str {
    let head = section
        .split_once(marker)
        .unwrap_or_else(|| panic!("format/enums.md must still contain {marker:?}"))
        .0;
    head.rsplit("\n\n")
        .next()
        .expect("rsplit always yields a first element")
}

/// The text between `marker` and the first following `)`.
fn parenthesized_after<'a>(section: &'a str, marker: &str) -> &'a str {
    let after = section
        .split_once(marker)
        .unwrap_or_else(|| panic!("format/enums.md must still contain {marker:?}"))
        .1;
    after
        .split_once(')')
        .expect("the parenthesis opened by that marker must be closed")
        .0
}

/// Spellings written as prose code spans — a backtick, a quoted
/// spelling, a backtick. Prose also contains bare quoted words
/// (`"conical"`), so the backticks are what tell a published spelling
/// from an ordinary one.
fn backticked_spellings(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for span in text.split('`') {
        if let Some(inner) = span.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
            out.push(inner);
        }
    }
    out
}

/// Every `"quoted"` span in `text`, in order.
fn quoted_spans(text: &str) -> Vec<&str> {
    text.split('"').skip(1).step_by(2).collect()
}

/// **Issue #118, against the file that produced it.** Every
/// `style.shape` string in `maps/testament.mindmap.json` — the demo
/// map the app opens by default, 242 of whose nodes are hexagons —
/// must classify as non-warning. Asserted over the map's real shape
/// strings rather than over emitted log records: there is no logger
/// to install from here, and the classification is the thing the
/// warning is a function of.
///
/// The map is loaded through `mindmap::loader`, so a shape string
/// the loader would reject never reaches the assertion — which is
/// the point: this is the load path, not a synthetic list.
#[test]
pub fn test_shape_testament_map_has_no_unknown_shapes() {
    do_shape_testament_map_has_no_unknown_shapes();
}

pub fn do_shape_testament_map_has_no_unknown_shapes() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../maps/testament.mindmap.json");
    let map = crate::mindmap::loader::load_from_file(std::path::Path::new(path))
        .expect("maps/testament.mindmap.json must load");

    let mut inspected = 0usize;
    for (location, node) in map.node_locations() {
        let spelling = ShapeSpelling::classify(&node.style.shape);
        assert!(
            !spelling.is_author_error(),
            "{location}: style.shape {:?} classified as {spelling:?} — \
             loading the demo map would warn",
            node.style.shape
        );
        inspected += 1;
    }
    assert!(
        inspected > 0,
        "no nodes inspected — the fixture walk found nothing and the \
         assertion above never ran"
    );
}

/// Rectangle `contains_local` matches the classic inclusive-AABB
/// predicate: corners and edges hit, anything outside `[0, bounds]`
/// misses. Locks in the happy path for the legacy behavior that
/// every existing node still depends on.
#[test]
pub fn test_shape_rectangle_contains_local() {
    do_shape_rectangle_contains_local();
}

pub fn do_shape_rectangle_contains_local() {
    let b = Vec2::new(100.0, 50.0);
    assert!(NodeShape::Rectangle.contains_local(Vec2::new(0.0, 0.0), b));
    assert!(NodeShape::Rectangle.contains_local(Vec2::new(100.0, 50.0), b));
    assert!(NodeShape::Rectangle.contains_local(Vec2::new(50.0, 25.0), b));
    assert!(!NodeShape::Rectangle.contains_local(Vec2::new(-0.1, 25.0), b));
    assert!(!NodeShape::Rectangle.contains_local(Vec2::new(100.1, 25.0), b));
}

/// Perfect-circle case: bounds 100×100, radius 50, center
/// `(50, 50)`. Center and the four cardinal rim points all count
/// as inside. Pins the "rim is inclusive" edge-case the BVH hit
/// test relies on for click-on-border behavior.
#[test]
pub fn test_shape_ellipse_contains_center_and_rim() {
    do_shape_ellipse_contains_center_and_rim();
}

pub fn do_shape_ellipse_contains_center_and_rim() {
    let b = Vec2::new(100.0, 100.0);
    assert!(NodeShape::Ellipse.contains_local(Vec2::new(50.0, 50.0), b));
    assert!(NodeShape::Ellipse.contains_local(Vec2::new(0.0, 50.0), b));
    assert!(NodeShape::Ellipse.contains_local(Vec2::new(100.0, 50.0), b));
    assert!(NodeShape::Ellipse.contains_local(Vec2::new(50.0, 0.0), b));
    assert!(NodeShape::Ellipse.contains_local(Vec2::new(50.0, 100.0), b));
}

/// Bounding-box corners sit at distance `√2 · r` from the center
/// of an inscribed circle — comfortably outside. This is the
/// exact case the whole refactor exists to reject: under the
/// pre-change AABB-only hit test, a corner click on an ellipse
/// node would select it; post-change it must miss.
#[test]
pub fn test_shape_ellipse_rejects_aabb_corners() {
    do_shape_ellipse_rejects_aabb_corners();
}

pub fn do_shape_ellipse_rejects_aabb_corners() {
    let b = Vec2::new(100.0, 100.0);
    assert!(!NodeShape::Ellipse.contains_local(Vec2::new(0.0, 0.0), b));
    assert!(!NodeShape::Ellipse.contains_local(Vec2::new(100.0, 0.0), b));
    assert!(!NodeShape::Ellipse.contains_local(Vec2::new(0.0, 100.0), b));
    assert!(!NodeShape::Ellipse.contains_local(Vec2::new(100.0, 100.0), b));
}

/// Stretched conic case: bounds `200 × 50`, radii `(100, 25)`.
/// Center and cardinal rim points still hit; bounding-box corners
/// still miss. Guards the "ellipse handles wider-than-tall without
/// extra parameters" claim from the shape doc comment.
#[test]
pub fn test_shape_ellipse_handles_stretched_conic() {
    do_shape_ellipse_handles_stretched_conic();
}

pub fn do_shape_ellipse_handles_stretched_conic() {
    let b = Vec2::new(200.0, 50.0);
    assert!(NodeShape::Ellipse.contains_local(Vec2::new(100.0, 25.0), b));
    assert!(NodeShape::Ellipse.contains_local(Vec2::new(0.0, 25.0), b));
    assert!(NodeShape::Ellipse.contains_local(Vec2::new(200.0, 25.0), b));
    assert!(!NodeShape::Ellipse.contains_local(Vec2::new(0.0, 0.0), b));
    assert!(!NodeShape::Ellipse.contains_local(Vec2::new(200.0, 50.0), b));
}

/// Degenerate bounds (zero or negative extent on either axis)
/// never hit — guards the division by `bounds / 2` in the ellipse
/// math and mirrors how the BVH's AABB check skips zero-size areas.
/// Rendering a zero-size node is already a no-op upstream, so
/// counting a click as a miss is the internally consistent answer.
#[test]
pub fn test_shape_degenerate_bounds_never_hit() {
    do_shape_degenerate_bounds_never_hit();
}

pub fn do_shape_degenerate_bounds_never_hit() {
    assert!(!NodeShape::Rectangle.contains_local(Vec2::ZERO, Vec2::ZERO));
    assert!(!NodeShape::Ellipse.contains_local(Vec2::ZERO, Vec2::new(0.0, 100.0)));
    assert!(!NodeShape::Ellipse.contains_local(Vec2::ZERO, Vec2::new(100.0, -1.0)));
}

/// Selection rect tucked fully inside the ellipse: the closest
/// point on the rect to the ellipse center is the ellipse center
/// itself, so `distance == 0` and the test registers a hit.
/// Without this branch, the lasso would report "no nodes
/// selected" whenever the user drew a small rectangle inside a
/// circular node — the exact case a user would expect to match.
#[test]
pub fn test_shape_ellipse_intersects_aabb_fully_inside() {
    do_shape_ellipse_intersects_aabb_fully_inside();
}

pub fn do_shape_ellipse_intersects_aabb_fully_inside() {
    let b = Vec2::new(100.0, 100.0);
    let min = Vec2::new(40.0, 40.0);
    let max = Vec2::new(60.0, 60.0);
    assert!(NodeShape::Ellipse.intersects_local_aabb(min, max, b));
}

/// Selection rect tucked into the AABB corner, outside the
/// ellipse. The pre-change AABB-overlap test would have matched
/// this as "node selected"; the shape-aware test must reject it.
/// This is the case the rect-select refactor exists to fix.
#[test]
pub fn test_shape_ellipse_intersects_aabb_corner_only() {
    do_shape_ellipse_intersects_aabb_corner_only();
}

pub fn do_shape_ellipse_intersects_aabb_corner_only() {
    let b = Vec2::new(100.0, 100.0);
    let min = Vec2::new(0.0, 0.0);
    let max = Vec2::new(5.0, 5.0);
    assert!(!NodeShape::Ellipse.intersects_local_aabb(min, max, b));
}

/// Selection rect crossing the ellipse's left rim: the clamp
/// lands on the rect's inside edge (x ≈ 0), which is on the
/// ellipse boundary. Conservative (`<= 1.0`) counts this as a
/// hit — the spirit of a lasso is "any overlap selects".
#[test]
pub fn test_shape_ellipse_intersects_aabb_straddling_rim() {
    do_shape_ellipse_intersects_aabb_straddling_rim();
}

pub fn do_shape_ellipse_intersects_aabb_straddling_rim() {
    let b = Vec2::new(100.0, 100.0);
    let min = Vec2::new(-10.0, 40.0);
    let max = Vec2::new(10.0, 60.0);
    assert!(NodeShape::Ellipse.intersects_local_aabb(min, max, b));
}

/// Selection rect entirely outside the node's bounding box.
/// Early-bails on the AABB–AABB overlap so the shape math isn't
/// even reached. Guards the cheap path every lasso hit-test takes
/// when the user drags far away from any node.
#[test]
pub fn test_shape_ellipse_intersects_aabb_fully_outside() {
    do_shape_ellipse_intersects_aabb_fully_outside();
}

pub fn do_shape_ellipse_intersects_aabb_fully_outside() {
    let b = Vec2::new(100.0, 100.0);
    let min = Vec2::new(200.0, 200.0);
    let max = Vec2::new(300.0, 300.0);
    assert!(!NodeShape::Ellipse.intersects_local_aabb(min, max, b));
}

/// The `shader_id` values are wire-format: the fragment shader
/// matches on the same integers via `SHAPE_RECT` / `SHAPE_ELLIPSE`
/// WGSL constants. Pinning them here catches the silent-breakage
/// case where a future reorder of the enum variants reassigns the
/// ids and every ellipse in every map quietly renders as a
/// rectangle.
#[test]
pub fn test_shape_shader_ids_are_stable() {
    do_shape_shader_ids_are_stable();
}

pub fn do_shape_shader_ids_are_stable() {
    assert_eq!(NodeShape::Rectangle.shader_id(), SHAPE_ID_RECTANGLE);
    assert_eq!(NodeShape::Ellipse.shader_id(), SHAPE_ID_ELLIPSE);
    // The absolute values are also part of the wire format — the
    // WGSL fragment shader hard-codes `0u` / `1u` in its `switch`
    // arms. Keeping the numeric assertion here means a rename
    // alone can't drift them.
    assert_eq!(NodeShape::Rectangle.shader_id(), 0);
    assert_eq!(NodeShape::Ellipse.shader_id(), 1);
}

/// **The reporting half of `from_style_string`, pinned by parsing
/// `shape.rs` itself — as a whitelist, not as a search.**
///
/// Every other test in this file asserts on a returned value, and a
/// returned value cannot see a log call. That left issue #118's
/// *second* requirement — "a value not in `KNOWN_SHAPES`: keep the
/// `warn!`" — untested in a way that mattered: deleting the entire
/// reporting block from `from_style_string` left the suite green, and
/// so did swapping the two macros.
///
/// The closure everyone reaches for first is a test logger, and
/// `util::test_logger` really is on an unmerged branch (#117). It is
/// not the only one. `syn` is already a baumhard dev-dependency for
/// exactly this class of question — `util::serde_coverage` parses the
/// crate's own sources so a contract can be *derived* from the code
/// instead of restated beside it — and which `log::` macro sits in
/// which arm is a fact about source text. So this module reads it out
/// of the source.
///
/// **Deriving from the source is not the same as being right**, and
/// the precedent cited above has since been caught at it: issue #122
/// found `serde_coverage` publishing a list `format/schema.md`
/// carried verbatim, both of them wrong in the same three places
/// because the document was written by reading the walk. The lesson
/// that transfers is the one this module already applies in its own
/// way — a source reader has to be a **whitelist that refuses what it
/// does not model**, never a search that reports what it happens to
/// find, because a search's silence is indistinguishable from
/// approval. `check_routing` below is that whitelist;
/// `util::serde_probe` is the same discipline reached by a different
/// road, a second derivation that can disagree.
///
/// **What changed in round 3, and why it is not three more cases.**
/// The first version of this module *searched* for `log::` calls: it
/// walked the body looking for `if`s, remembered the method name of
/// each condition, and reported the pairs it happened to find. A
/// search over a language model that covers four of Rust's roughly
/// forty expression forms passes vacuously for the other thirty-six.
/// Three spellings proved it: an outer `if` wrapped around the whole
/// reporting block made `from_style_string` log nothing at all and
/// left the pin green; `#[cfg(any())]` on the `warn!` deleted it from
/// every build and left the pin green; a second `warn!` added inside
/// a `for` loop restored issue #118 verbatim and left the pin green.
/// Patching in three more cases would have left the thirty-third.
///
/// So the decision moved out of the source and into a value —
/// [`ShapeSpelling::report`](crate::gfx_structs::shape::ShapeSpelling::report)
/// returns a
/// [`ShapeReport`](crate::gfx_structs::shape::ShapeReport), which
/// `do_shape_report_routes_every_classification` tests as ordinary
/// data — and what is left here is a **whitelist** over what remains:
/// [`check_routing`] states the exact shape the body is allowed to
/// have and returns `Err` for everything else. Three statements, no
/// more and no fewer; each one pinned to the expression it must be;
/// one `match` arm per `ShapeReport` variant plus one `None` arm;
/// each reporting arm a bare `log::<macro>!` whose name is derived
/// from `ShapeReport::level()` rather than restated; and no attribute
/// anywhere in the body but a doc comment. An unmodeled construct
/// fails by not matching, which is the property a search cannot have.
///
/// **Round 4 gave the *selection* the same treatment.** Whitelisting
/// a body is worth nothing if the pin then searches for which body to
/// read: [`from_style_string_fn`] used to take the first
/// `impl NodeShape` that happened to define the name, so a decoy
/// above the real definition — a `#[cfg(any())]` impl, a
/// `#[cfg(target_arch)]` pair, or an ordinary
/// `impl SomeTrait for NodeShape` needing no conditional compilation
/// at all — was what the pin reported on while the real definition
/// went back to warning per node. [`select_from_style_string`] is the
/// requirement that replaced it: every `from_style_string` under a
/// `NodeShape` impl is a candidate, there must be exactly one, its
/// enclosing block must carry no attribute but a doc comment, and
/// that block must be the inherent impl.
///
/// **What it still cannot see**, stated plainly because the round-2
/// disclosure was not:
///
/// - **Anything outside `from_style_string`.** A `log::warn!` added
///   to `ShapeSpelling::classify` would recreate #118's symptom and
///   this pin would not notice. The claim here is about
///   `NodeShape::from_style_string` — the body *and* the choice of
///   it, which is what [`select_from_style_string`] makes provable —
///   and it is complete only about that one function.
/// - **What the macro is handed.** The message string, its format
///   arguments and its `"<area>: "` prefix (CODE_CONVENTIONS §9) are
///   not read. A `log::warn!("")` in the right arm passes.
/// - **What `log::warn!` means at the point it is compiled.** A
///   `macro_rules! warn` shadowing the real one, or a `use` that
///   rebinds `log`, is text this pin accepts and behavior it cannot
///   evaluate. Nothing here does that; the pin does not prove it.
/// - **Whether the level is right.** `ShapeReport::level()` is the
///   definition, so the pin holds the arm to whatever that says. If
///   `level()` itself were wrong, the pin would faithfully hold the
///   arms to the wrong answer —
///   `do_shape_report_levels_are_warn_and_trace` is the separate
///   assertion that the two levels are the ones §9 and #118 call for.
///
/// One escape closed itself in the restructure and is worth naming
/// because it costs nothing: `#[cfg(any())]` on a `match` arm now
/// makes the `match` non-exhaustive, so it is `error[E0004]` before
/// it is ever a test failure. Verified, not assumed.
///
/// §B8 asks for a `do_*()` body and a bench entry per test; the tests
/// in this module have neither, deliberately and with precedent —
/// §B8 opt-out class 1, the body cannot compile into the library.
/// `syn` is a dev-dependency, so a `pub` body in this `pub mod tests`
/// tree would not compile into the library, and `serde_coverage`'s
/// own consumer tests are plain `#[cfg(test)]` tests for the same
/// reason. The `cfg(test)` gate on the module below is what makes
/// that structural rather than a choice: the bench-surface scan does
/// not collect through it. Native only: it reads a file.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod log_routing {
    use strum::IntoEnumIterator;
    use syn::{Attribute, Expr, ImplItem, ImplItemFn, Item, ItemImpl, Pat, Path, PathArguments, Stmt, Type};

    use crate::gfx_structs::shape::ShapeReport;

    /// The name `from_style_string` must bind its classification to.
    const BOUND_NAME: &str = "spelling";
    /// The initializer of that binding, in [`render_expr`]'s grammar.
    const CLASSIFY_CALL: &str = "ShapeSpelling::classify(s)";
    /// The `match` scrutinee. Pinning it is what stops the routing
    /// decision being moved somewhere this module cannot follow: a
    /// `match spelling.is_author_error()` fails here by name.
    const REPORT_CALL: &str = "spelling.report()";
    /// The trailing expression, which is also the whole return value.
    const RESOLVE_TAIL: &str = "spelling.resolve()";
    /// The pattern of the arm that reports nothing.
    const SILENT_ARM: &str = "None";

    /// The pin: the shipped `from_style_string` is exactly the shape
    /// [`check_routing`] models, with one reporting arm per
    /// [`ShapeReport`] variant.
    #[test]
    fn test_from_style_string_log_routing_is_pinned() {
        assert!(
            ShapeReport::iter().count() > 0,
            "ShapeReport has no variants — every derived expectation \
             below would be empty and this test would pass vacuously"
        );

        let function = from_style_string_fn();
        let routing = check_routing(&function).unwrap_or_else(|why| {
            panic!(
                "NodeShape::from_style_string is no longer the shape this pin \
                 models, so what a load logs is once again unobserved: {why}"
            )
        });
        assert_eq!(
            routing.len(),
            ShapeReport::iter().count(),
            "check_routing accepted {routing:?}, which is not one arm per \
             ShapeReport variant"
        );
    }

    /// Positive control. The checker accepts the shape it models, and
    /// the fixture it accepts routes the same way the shipped
    /// function does — so this fixture drifting away from `shape.rs`
    /// is itself a failure rather than a control quietly testing a
    /// shape nobody ships.
    #[test]
    fn test_log_routing_check_accepts_the_modeled_shape() {
        let accepted = check_routing(&fixture(MODELED)).expect("the modeled shape must be accepted");
        let shipped = check_routing(&from_style_string_fn()).expect("the shipped function must be accepted");
        assert_eq!(
            accepted, shipped,
            "the control fixture and the shipped from_style_string route \
             differently, so the controls below are checking a shape this \
             crate does not have"
        );
    }

    /// **Finding 1 of the round-2 review, as a control.** A condition
    /// wrapped *around* the reporting silences `from_style_string`
    /// completely — `"pentagram"` stops warning, issue #118's second
    /// requirement is gone — and the old walker composed the outer
    /// condition away to `None` and stayed green. Here the wrap
    /// displaces the `match` from the one position the pin allows it.
    #[test]
    fn test_log_routing_check_rejects_a_condition_wrapped_around_the_match() {
        let source = MODELED
            .replace(
                "        match spelling.report() {",
                "        if s.len() > 1000 {\n        match spelling.report() {",
            )
            .replace(
                "        }\n        spelling.resolve()",
                "        }\n        }\n        spelling.resolve()",
            );
        let why = check_routing(&fixture(&source)).expect_err("an outer condition must be rejected");
        assert!(
            why.contains("`match`"),
            "the rejection must say the `match` is not where the pin models \
             it; it said {why:?}"
        );
    }

    /// The same silencing spelled as a flag, which is the shape that
    /// reaches the pin as a fourth statement rather than as a
    /// displaced `match`. Both spellings are rejected; they are
    /// rejected by different clauses, so both are controlled.
    #[test]
    fn test_log_routing_check_rejects_a_flag_wrapped_around_the_match() {
        let source = MODELED
            .replace(
                "        match spelling.report() {",
                "        let report = false;\n        if report {\n        match spelling.report() {",
            )
            .replace(
                "        }\n        spelling.resolve()",
                "        }\n        }\n        spelling.resolve()",
            );
        let why = check_routing(&fixture(&source)).expect_err("a flag wrap must be rejected");
        assert!(
            why.contains("statements"),
            "the rejection must name the statement count; it said {why:?}"
        );
    }

    /// **Finding 2, as a control.** `#[cfg(any())]` on an arm strips
    /// the arm from every build. In the shipped function that is a
    /// compile error as well — the `match` stops covering
    /// `Option<ShapeReport>` — but the checker must not need the
    /// compiler's help to say so.
    #[test]
    fn test_log_routing_check_rejects_a_cfg_stripped_arm() {
        let source = MODELED.replace(
            "            Some(ShapeReport::UnknownSpelling)",
            "            #[cfg(any())]\n            Some(ShapeReport::UnknownSpelling)",
        );
        let why = check_routing(&fixture(&source)).expect_err("a cfg-stripped arm must be rejected");
        assert!(
            why.contains("cfg"),
            "the rejection must name the attribute; it said {why:?}"
        );
    }

    /// The other half of finding 2: a `#[cfg]` that hides the macro
    /// rather than the arm. `#[cfg(target_arch = "wasm32")]` here
    /// deletes the warning from the *native* build while this test,
    /// which is itself `not(target_arch = "wasm32")`-gated, keeps
    /// running — a CODE_CONVENTIONS §4 divergence that source text
    /// alone does not show.
    #[test]
    fn test_log_routing_check_rejects_a_cfg_stripped_macro() {
        let source = MODELED.replace(
            "log::warn!(\"unknown\")",
            "{\n                #[cfg(target_arch = \"wasm32\")]\n                log::warn!(\"unknown\");\n            }",
        );
        let why = check_routing(&fixture(&source)).expect_err("a cfg-stripped macro must be rejected");
        assert!(
            why.contains("bare macro invocation"),
            "the rejection must say the arm body is not a bare macro; it said {why:?}"
        );
    }

    /// **Finding 3, as a control.** An *added* `log::warn!` — here in
    /// a `for` loop, the spelling that restores issue #118 verbatim
    /// at 242 lines per load of the demo map — is a fourth statement,
    /// and the pin admits exactly three.
    #[test]
    fn test_log_routing_check_rejects_an_added_log_call() {
        let source = MODELED.replace(
            "        spelling.resolve()",
            "        for _ in 0..1 {\n            log::warn!(\"unknown\");\n        }\n        spelling.resolve()",
        );
        let why = check_routing(&fixture(&source)).expect_err("an added log call must be rejected");
        assert!(
            why.contains("statements"),
            "the rejection must name the statement count; it said {why:?}"
        );
    }

    /// Swapping the two macros is the failure the first version of
    /// this module was written for, and it still fails — now against
    /// `ShapeReport::level()` rather than against a table written
    /// beside the test.
    #[test]
    fn test_log_routing_check_rejects_swapped_macros() {
        let source = MODELED
            .replace("log::warn!(\"unknown\")", "log::swapped!(\"unknown\")")
            .replace("log::trace!(\"substituted\")", "log::warn!(\"substituted\")")
            .replace("log::swapped!(\"unknown\")", "log::trace!(\"unknown\")");
        let why = check_routing(&fixture(&source)).expect_err("swapped macros must be rejected");
        assert!(
            why.contains("log::warn"),
            "the rejection must name the macro the level calls for; it said {why:?}"
        );
    }

    /// The residual disclosed in round 2, now a rejection rather than
    /// a "changed pair": `log::log!(level, …)` with a computed level
    /// is what defeats `log`'s release compile-out, so the pin
    /// refuses it by name instead of recording it and moving on.
    #[test]
    fn test_log_routing_check_rejects_a_computed_level() {
        let source = MODELED.replace(
            "log::warn!(\"unknown\")",
            "log::log!(ShapeReport::UnknownSpelling.level(), \"unknown\")",
        );
        let why = check_routing(&fixture(&source)).expect_err("a computed level must be rejected");
        assert!(
            why.contains("log::log"),
            "the rejection must name what it found; it said {why:?}"
        );
    }

    /// A dropped reporting arm. In the shipped function the compiler
    /// catches this first, since `Option<ShapeReport>` stops being
    /// covered; the checker says so on its own anyway, because a
    /// future variant swept into a widened pattern would compile.
    #[test]
    fn test_log_routing_check_rejects_a_missing_reporting_arm() {
        let source = MODELED.replace(
            "            Some(ShapeReport::RectangleSubstituted) => log::trace!(\"substituted\"),\n",
            "",
        );
        let why = check_routing(&fixture(&source)).expect_err("a missing arm must be rejected");
        assert!(
            why.contains("RectangleSubstituted"),
            "the rejection must name the variant with no arm; it said {why:?}"
        );
    }

    /// Moving the decision somewhere the pin cannot follow. The
    /// scrutinee is pinned by name for this reason: a `match` on
    /// something other than `spelling.report()` is a different
    /// routing, whatever its arms look like.
    #[test]
    fn test_log_routing_check_rejects_a_different_scrutinee() {
        let source = MODELED.replace("match spelling.report()", "match spelling.some_other_report()");
        let why = check_routing(&fixture(&source)).expect_err("a moved decision must be rejected");
        assert!(
            why.contains(REPORT_CALL),
            "the rejection must name the scrutinee the pin models; it said {why:?}"
        );
    }

    /// **Finding B of the round-4 review, as a control.** Statement 3
    /// is pinned to `spelling.resolve()` and not merely to *being* a
    /// trailing expression, because a block is a trailing expression
    /// too — `{ log::warn!(…); spelling.resolve() }` returns the same
    /// value and restores #118 verbatim. Deleting the `expect_expr`
    /// call one line below the `Stmt::Expr` destructure left the whole
    /// suite green, so this is what says the clause is still there.
    #[test]
    fn test_log_routing_check_rejects_a_block_tail() {
        let source = MODELED.replace(
            "        spelling.resolve()\n",
            "        {\n            log::warn!(\"unknown\");\n            spelling.resolve()\n        }\n",
        );
        let why = check_routing(&fixture(&source)).expect_err("a block tail must be rejected");
        assert!(
            why.contains("the trailing expression"),
            "the rejection must name the trailing expression; it said {why:?}"
        );
    }

    /// The same finding one statement up. The `let` initializer is
    /// pinned to `ShapeSpelling::classify(s)` for the same reason:
    /// `let spelling = { log::warn!(…); ShapeSpelling::classify(s) };`
    /// binds the same value, is still statement 1, and warns on every
    /// node.
    #[test]
    fn test_log_routing_check_rejects_a_block_initializer() {
        let source = MODELED.replace(
            "        let spelling = ShapeSpelling::classify(s);\n",
            "        let spelling = {\n            log::warn!(\"unknown\");\n            ShapeSpelling::classify(s)\n        };\n",
        );
        let why = check_routing(&fixture(&source)).expect_err("a block initializer must be rejected");
        assert!(
            why.contains("the `let` binding's initializer"),
            "the rejection must name the initializer; it said {why:?}"
        );
    }

    /// And the third: the `None` arm's body must be an *empty* block,
    /// not merely a block. `None => { log::warn!(…); }` is the arm the
    /// classifier reaches for every node that names no shape and every
    /// node that names a drawable one — which on
    /// `maps/testament.mindmap.json` is most of them.
    #[test]
    fn test_log_routing_check_rejects_a_reporting_silent_arm() {
        let source = MODELED.replace(
            "            None => {}\n",
            "            None => {\n                log::warn!(\"unknown\");\n            }\n",
        );
        let why = check_routing(&fixture(&source)).expect_err("a reporting None arm must be rejected");
        assert!(
            why.contains("statements"),
            "the rejection must say the silent arm's block is not empty; it \
             said {why:?}"
        );
    }

    /// **Finding A of the round-4 review, as a control.** The pin
    /// whitelists the body but must not *search* for it: a second
    /// definition means the body this module reads is not provably the
    /// body the compiler takes. Here the decoy is `#[cfg(any())]`, so
    /// it compiles into nothing while the real definition — free to be
    /// anything at all — is what runs.
    #[test]
    fn test_log_routing_selection_rejects_a_second_candidate() {
        let why = select_from_style_string(&format!(
            "#[cfg(any())]\nimpl NodeShape {{{MODELED}}}\nimpl NodeShape {{{MODELED}}}\n"
        ))
        .err()
        .expect("a second candidate must be rejected");
        assert!(
            why.contains("exactly one"),
            "the rejection must say one definition is all the pin models; it \
             said {why:?}"
        );
    }

    /// The other half of finding A's first exploit: one candidate, but
    /// its `impl` block carries a `#[cfg]`. `only_doc_attrs` already
    /// guarded five positions *inside* the body and none of them
    /// reached the enclosing block, so
    /// `#[cfg(target_arch = "wasm32")]` moved up one level was the same
    /// CODE_CONVENTIONS §4 divergence
    /// `test_log_routing_check_rejects_a_cfg_stripped_macro` exists to
    /// catch, in a position nothing looked at.
    #[test]
    fn test_log_routing_selection_rejects_an_attributed_impl() {
        let why = select_from_style_string(&format!(
            "#[cfg(target_arch = \"wasm32\")]\nimpl NodeShape {{{MODELED}}}\n"
        ))
        .err()
        .expect("an attributed impl must be rejected");
        assert!(
            why.contains("the enclosing `impl` block") && why.contains("cfg"),
            "the rejection must name the block and the attribute; it said {why:?}"
        );
    }

    /// The cheapest exploit of the three, and the only one needing no
    /// conditional compilation at all: an ordinary
    /// `impl SomeTrait for NodeShape` shipped in every build. Its
    /// `self_ty` is `NodeShape`, so it read as the definition; its
    /// body is not the one `NodeShape::from_style_string` resolves to.
    #[test]
    fn test_log_routing_selection_rejects_a_trait_impl() {
        let why = select_from_style_string(&format!("impl ShapeParse for NodeShape {{{MODELED}}}\n"))
            .err()
            .expect("a trait impl must be rejected");
        assert!(
            why.contains("ShapeParse"),
            "the rejection must name the trait it found; it said {why:?}"
        );
    }

    /// Positive control for the three above, so each is a single
    /// deliberate deviation from a source the selector accepts rather
    /// than a rejection that might have come from anywhere.
    #[test]
    fn test_log_routing_selection_accepts_one_unattributed_inherent_impl() {
        let function = select_from_style_string(&format!("impl NodeShape {{{MODELED}}}\n"))
            .expect("one unattributed inherent impl must be accepted");
        assert_eq!(
            check_routing(&function).expect("and its body must be the modeled shape"),
            check_routing(&fixture(MODELED)).expect("as the fixture's is"),
            "the selector returned a different body than the fixture holds"
        );
    }

    /// The shape the pin models, written once so each control above
    /// is a single deliberate deviation from it. Held to the shipped
    /// function by `test_log_routing_check_accepts_the_modeled_shape`
    /// — the message strings are the only part that may differ, since
    /// [`check_routing`] does not read them.
    const MODELED: &str = r#"
    pub fn from_style_string(s: &str) -> Self {
        let spelling = ShapeSpelling::classify(s);
        match spelling.report() {
            Some(ShapeReport::UnknownSpelling) => log::warn!("unknown"),
            Some(ShapeReport::RectangleSubstituted) => log::trace!("substituted"),
            None => {}
        }
        spelling.resolve()
    }
"#;

    /// Parse a control fixture. Panics rather than returning `Err`:
    /// a fixture that does not parse is a bug in this file, not a
    /// finding about `shape.rs`.
    fn fixture(source: &str) -> ImplItemFn {
        syn::parse_str(source).expect("a control fixture must parse as an impl method")
    }

    /// **The whitelist.** `Ok` carries the `(arm pattern, macro path)`
    /// pairs the body was found to contain, in source order; `Err`
    /// carries the first way the body departs from the modeled shape.
    ///
    /// Every clause below is a *requirement*, never a search. The
    /// difference is the whole point of the round-3 rewrite: a search
    /// reports what it happens to find and says nothing about the
    /// rest, so a construct outside its model passes; a requirement
    /// that cannot be satisfied fails.
    ///
    /// **Four clauses here are deliberately redundant, and are kept
    /// anyway.** Deleting any one of them leaves the suite green, so
    /// naming them is what stops a reader mistaking a control gap for
    /// them being load-bearing:
    ///
    /// - `arm.guard.is_some()` — a guard makes the `match`
    ///   non-exhaustive, so the shipped function is `error[E0004]`
    ///   first. The clause is what lets this checker say so without
    ///   the compiler's help, the same reason
    ///   `test_log_routing_check_rejects_a_cfg_stripped_arm` exists.
    /// - `silent_arms != 1` — a second `None` arm is unreachable, and
    ///   each one still has to be an empty block to get here at all.
    /// - `routing.len()` in the pin's own assertion — subsumed by the
    ///   `covered != wanted` compare below, which is per-variant.
    /// - [`render_expr`]'s `_ => None` catch-all — every caller runs
    ///   the result through [`expect_expr`], which fails an unmodeled
    ///   form on the string compare regardless.
    ///
    /// The clauses that are *not* redundant have a control each: the
    /// three-statement count, the `let` binding's name and
    /// initializer, the `match` scrutinee, the trailing expression,
    /// the silent arm's empty block, the bare-macro arm body, the
    /// derived macro name, and `only_doc_attrs` at every position.
    fn check_routing(function: &ImplItemFn) -> Result<Vec<(String, String)>, String> {
        only_doc_attrs(&function.attrs, "the function")?;

        let stmts = &function.block.stmts;
        if stmts.len() != 3 {
            return Err(format!(
                "the body has {} statements; the pin models exactly three — \
                 bind the classification, match on its report, return the \
                 resolved shape. An added `log::` call, or a flag wrapped \
                 around the reporting, arrives here as a fourth",
                stmts.len()
            ));
        }

        let Stmt::Local(local) = &stmts[0] else {
            return Err("statement 1 is not a `let` binding".to_string());
        };
        only_doc_attrs(&local.attrs, "the `let` binding")?;
        let Pat::Ident(bound) = &local.pat else {
            return Err("the `let` binding does not bind a plain name".to_string());
        };
        if bound.ident != BOUND_NAME {
            return Err(format!(
                "the `let` binding names `{}`; the pin models `{BOUND_NAME}`",
                bound.ident
            ));
        }
        let Some(init) = &local.init else {
            return Err(format!("`{BOUND_NAME}` is declared without an initializer"));
        };
        if init.diverge.is_some() {
            return Err("the `let` binding grew an `else` branch".to_string());
        }
        expect_expr(&init.expr, CLASSIFY_CALL, "the `let` binding's initializer")?;

        let Stmt::Expr(Expr::Match(reporting), _) = &stmts[1] else {
            return Err(format!(
                "statement 2 of the body is not a `match`; the pin models \
                 `match {REPORT_CALL} {{ … }}` there, and a condition wrapped \
                 around the reporting displaces it"
            ));
        };
        only_doc_attrs(&reporting.attrs, "the `match`")?;
        expect_expr(&reporting.expr, REPORT_CALL, "the `match` scrutinee")?;

        let Stmt::Expr(tail, None) = &stmts[2] else {
            return Err("statement 3 is not a trailing expression".to_string());
        };
        expect_expr(tail, RESOLVE_TAIL, "the trailing expression")?;

        let mut routing = Vec::new();
        let mut silent_arms = 0usize;
        for (index, arm) in reporting.arms.iter().enumerate() {
            let position = format!("`match` arm {}", index + 1);
            only_doc_attrs(&arm.attrs, &position)?;
            // syn 3 carries a guard inside the arm's pattern.
            if matches!(arm.pat, Pat::Guard(_)) {
                return Err(format!("{position} carries an `if` guard"));
            }
            let Some(pattern) = render_pat(&arm.pat) else {
                return Err(format!(
                    "{position} has a pattern outside the forms this pin \
                     models — a name, a path, or a tuple struct over those"
                ));
            };
            if pattern == SILENT_ARM {
                expect_empty_block(&arm.body, &position)?;
                silent_arms += 1;
                continue;
            }
            let Some(report) = report_for_pattern(&pattern) else {
                return Err(format!(
                    "{position} matches `{pattern}`, which is neither \
                     `{SILENT_ARM}` nor `Some(ShapeReport::…)`"
                ));
            };
            let Expr::Macro(invocation) = &*arm.body else {
                return Err(format!(
                    "{position}'s body is not a bare macro invocation. A block \
                     is the shape a `#[cfg]` on the macro needs, and a `#[cfg]` \
                     can delete the call from a build the source text still shows"
                ));
            };
            only_doc_attrs(&invocation.attrs, &format!("{position}'s macro"))?;
            let called = path_string(&invocation.mac.path);
            let wanted = format!("log::{}", macro_for(report.level()));
            if called != wanted {
                return Err(format!(
                    "{position} matches `{pattern}`, whose \
                     `ShapeReport::level()` is `{:?}`, so its body must be \
                     `{wanted}!`; it is `{called}!`",
                    report.level()
                ));
            }
            routing.push((pattern, called));
        }

        if silent_arms != 1 {
            return Err(format!(
                "the `match` has {silent_arms} `{SILENT_ARM}` arms; it must \
                 have exactly one, or a classification nobody reports would \
                 have nowhere to land"
            ));
        }
        let mut covered: Vec<String> = routing.iter().map(|(pattern, _)| pattern.clone()).collect();
        covered.sort();
        let mut wanted: Vec<String> = ShapeReport::iter().map(arm_pattern_for).collect();
        wanted.sort();
        if covered != wanted {
            return Err(format!(
                "the reporting arms are {covered:?}; there must be exactly one \
                 per ShapeReport variant, which is {wanted:?}"
            ));
        }
        Ok(routing)
    }

    /// The `log::` macro that writes at `level`. Total over
    /// [`log::Level`] with no `_` arm, so a sixth level would be a
    /// build error here rather than a silently unroutable report.
    fn macro_for(level: log::Level) -> &'static str {
        match level {
            log::Level::Error => "error",
            log::Level::Warn => "warn",
            log::Level::Info => "info",
            log::Level::Debug => "debug",
            log::Level::Trace => "trace",
        }
    }

    /// The arm pattern that reports `report`, derived from the
    /// variant name rather than written out.
    fn arm_pattern_for(report: ShapeReport) -> String {
        format!("Some(ShapeReport::{report:?})")
    }

    /// The inverse of [`arm_pattern_for`], over the variants that
    /// exist.
    fn report_for_pattern(pattern: &str) -> Option<ShapeReport> {
        ShapeReport::iter().find(|report| arm_pattern_for(*report) == pattern)
    }

    /// Reject any attribute but a doc comment.
    ///
    /// Conditional compilation is why this is a whitelist and not a
    /// `cfg` denylist: `#[cfg(any())]` deletes the item from every
    /// build, `#[cfg(target_arch = "wasm32")]` from the native one
    /// only, and `#[cfg_attr(all(), cfg(any()))]` spells the first
    /// without the word `cfg` reaching the attribute's path in the
    /// obvious place. All three leave source text this module would
    /// otherwise read as present.
    fn only_doc_attrs(attrs: &[Attribute], position: &str) -> Result<(), String> {
        for attr in attrs {
            if !attr.path().is_ident("doc") {
                return Err(format!(
                    "{position} carries `#[{}]`. The pin models doc comments \
                     and nothing else, because an attribute can remove from a \
                     build what the source still shows",
                    path_string(attr.path())
                ));
            }
        }
        Ok(())
    }

    /// The silent arm's body must be an empty, unlabeled,
    /// unattributed block — there is nothing for it to do.
    fn expect_empty_block(body: &Expr, position: &str) -> Result<(), String> {
        let Expr::Block(node) = body else {
            return Err(format!(
                "{position} is the `{SILENT_ARM}` arm, so its body must be an \
                 empty block"
            ));
        };
        only_doc_attrs(&node.attrs, &format!("{position}'s block"))?;
        if node.label.is_some() {
            return Err(format!("{position}'s block carries a label"));
        }
        if !node.block.stmts.is_empty() {
            return Err(format!(
                "{position} is the `{SILENT_ARM}` arm, but its block has {} \
                 statements",
                node.block.stmts.len()
            ));
        }
        Ok(())
    }

    /// Require `expr` to be exactly `wanted`, in [`render_expr`]'s
    /// grammar.
    fn expect_expr(expr: &Expr, wanted: &str, position: &str) -> Result<(), String> {
        match render_expr(expr) {
            Some(actual) if actual == wanted => Ok(()),
            Some(actual) => Err(format!("{position} is `{actual}`; the pin models `{wanted}`")),
            None => Err(format!(
                "{position} is an expression form outside the grammar this pin \
                 models — a path, a call, or a method call over those; the pin \
                 models `{wanted}` there"
            )),
        }
    }

    /// Render the small expression grammar this module models: a
    /// path, a call, or a method call whose receiver and arguments
    /// are themselves in the grammar. `None` for everything else,
    /// which is what turns an unmodeled construct into a failure
    /// rather than a silent pass.
    fn render_expr(expr: &Expr) -> Option<String> {
        match expr {
            Expr::Path(node) if node.attrs.is_empty() && node.qself.is_none() => {
                Some(path_string(&node.path))
            }
            Expr::Call(node) if node.attrs.is_empty() => {
                let args: Vec<String> = node.args.iter().map(render_expr).collect::<Option<_>>()?;
                Some(format!("{}({})", render_expr(&node.func)?, args.join(", ")))
            }
            Expr::MethodCall(node) if node.attrs.is_empty() && node.turbofish.is_none() => {
                let args: Vec<String> = node.args.iter().map(render_expr).collect::<Option<_>>()?;
                Some(format!(
                    "{}.{}({})",
                    render_expr(&node.receiver)?,
                    node.method,
                    args.join(", ")
                ))
            }
            _ => None,
        }
    }

    /// The pattern counterpart of [`render_expr`], over the three
    /// forms an arm here is allowed to take.
    fn render_pat(pat: &Pat) -> Option<String> {
        match pat {
            Pat::Ident(node)
                if node.attrs.is_empty()
                    && node.by_ref.is_none()
                    && node.mutability.is_none()
                    && node.subpat.is_none() =>
            {
                Some(node.ident.to_string())
            }
            Pat::Path(node) if node.attrs.is_empty() && node.qself.is_none() => Some(path_string(&node.path)),
            Pat::TupleStruct(node) if node.attrs.is_empty() && node.qself.is_none() => {
                let elems: Vec<String> = node.elems.iter().map(render_pat).collect::<Option<_>>()?;
                Some(format!("{}({})", path_string(&node.path), elems.join(", ")))
            }
            _ => None,
        }
    }

    /// `a::b::c`. A segment carrying generic arguments renders with a
    /// marker no modeled string contains, so it fails the compare
    /// rather than being silently discarded.
    fn path_string(path: &Path) -> String {
        let mut out = String::new();
        if path.leading_colon.is_some() {
            out.push_str("::");
        }
        for (index, segment) in path.segments.iter().enumerate() {
            if index > 0 {
                out.push_str("::");
            }
            out.push_str(&segment.ident.to_string());
            if !matches!(segment.arguments, PathArguments::None) {
                out.push_str("<generic arguments>");
            }
        }
        out
    }

    /// `NodeShape::from_style_string`, parsed out of `shape.rs`.
    /// Panics if the selection is not unambiguous: a silent skip, or a
    /// body picked out of several, would turn the pin into a no-op.
    fn from_style_string_fn() -> ImplItemFn {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/gfx_structs/shape.rs");
        let text = std::fs::read_to_string(path).expect("shape.rs must be readable");
        select_from_style_string(&text).unwrap_or_else(|why| {
            panic!("shape.rs no longer offers one unambiguous NodeShape::from_style_string: {why}")
        })
    }

    /// **The selection, as a requirement rather than a search.**
    /// [`from_style_string_fn`] over supplied source, so what happens
    /// to a second candidate is controllable instead of only ever
    /// exercised against the one file on disk.
    ///
    /// [`check_routing`] is a whitelist over a body; this is the
    /// whitelist over *which* body. Reading the first `impl NodeShape`
    /// that happens to define the name is a search, with a search's
    /// weakness: a decoy above the real definition is what the pin
    /// then reports on. So every `from_style_string` under a
    /// `NodeShape` impl is a candidate — attributed or not, inherent
    /// or trait — and there must be exactly one of them, carrying no
    /// attribute but a doc comment, in an inherent impl. That is what
    /// makes the body this module reads provably the body the compiler
    /// takes.
    fn select_from_style_string(source: &str) -> Result<ImplItemFn, String> {
        let file = syn::parse_file(source).expect("the source must parse as Rust");
        let mut candidates: Vec<(&ItemImpl, &ImplItemFn)> = Vec::new();
        for item in &file.items {
            let Item::Impl(block) = item else { continue };
            if !is_impl_for(&block.self_ty, "NodeShape") {
                continue;
            }
            for member in &block.items {
                let ImplItem::Fn(function) = member else { continue };
                if function.sig.ident == "from_style_string" {
                    candidates.push((block, function));
                }
            }
        }
        let [(block, function)] = candidates[..] else {
            return Err(format!(
                "{} definitions of `from_style_string` sit under a `NodeShape` \
                 impl; the pin models exactly one, because with two it would \
                 be reading a body it cannot show is the one that compiles",
                candidates.len()
            ));
        };
        only_doc_attrs(&block.attrs, "the enclosing `impl` block")?;
        if let Some((path, _)) = &block.trait_ {
            return Err(format!(
                "`from_style_string` is defined in `impl {} for NodeShape`; \
                 the pin models the inherent impl, and a trait impl is a \
                 second definition of the name that a call site can resolve \
                 to instead",
                path_string(path)
            ));
        }
        Ok(function.clone())
    }

    fn is_impl_for(ty: &Type, name: &str) -> bool {
        match ty {
            Type::Path(path) => path
                .path
                .segments
                .last()
                .is_some_and(|segment| segment.ident == name),
            _ => false,
        }
    }
}
