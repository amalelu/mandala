// SPDX-License-Identifier: MPL-2.0

//! Per-node background / hit-test shapes.
//!
//! [`crate::gfx_structs::shape::NodeShape`] is the single source of
//! truth for "what shape does this node occupy?". Both the renderer
//! (SDF fragment path) and the BVH hit test (point-in-shape check)
//! consult the same enum, so adding a new shape never drifts between
//! visuals and input.
//!
//! Extending the set is deliberately local:
//!
//! 1. Add a variant to the enum below.
//! 2. Give it its canonical `NodeStyle.shape` spelling(s) in
//!    [`crate::gfx_structs::shape::NodeShape::style_spellings`], and
//!    add that spelling to
//!    [`crate::gfx_structs::shape::KNOWN_SHAPES`] if it is not
//!    already listed. The `match` in `style_spellings` is
//!    exhaustive, so step 1 does not compile until this one is
//!    done — a new variant cannot silently stay on the
//!    quiet-fallback path.
//! 3. Add a `SHAPE_*` constant + a `case` arm to the rect pipeline's
//!    fragment shader (`src/application/renderer/mod.rs`,
//!    `RECT_SHADER_WGSL`). Forced, not remembered:
//!    `test_every_node_shape_has_a_matching_wgsl_constant_and_case_arm`
//!    in the mandala crate reads the shader as text and fails on a
//!    variant with no constant, a constant carrying the wrong id, and
//!    a non-default variant with no `case` arm — the last being the
//!    silent one, where every node of the new shape draws as a
//!    rectangle.
//! 4. Add a branch in
//!    [`crate::gfx_structs::shape::NodeShape::contains_local`] and
//!    [`crate::gfx_structs::shape::NodeShape::intersects_local_aabb`].
//!
//! No new structs, no new mutation surfaces, no new mesh builders.
//!
//! The format's shape vocabulary is deliberately wider than the set
//! this module can draw:
//! [`crate::gfx_structs::shape::KNOWN_SHAPES`] lists every canonical
//! spelling `format/enums.md` publishes and `maptool verify` accepts,
//! while `NodeShape` names only the ones with a shader case behind
//! them. [`crate::gfx_structs::shape::ShapeSpelling`] is the pure
//! classifier that tells those two populations apart, so a canonical
//! spelling awaiting its shader case degrades quietly to a rectangle
//! and only a genuine typo is reported to the author.
//! [`crate::gfx_structs::shape::ShapeReport`] is the second half of
//! that split: the *reporting* decision as a value, so what a load
//! says is data a test can read rather than a shape of `if`s only a
//! log sink could observe.

use crate::util::geometry::aabb_contains;
use glam::Vec2;
use serde::{Deserialize, Serialize};
use strum::IntoEnumIterator;
use strum_macros::EnumIter;

/// The background / hit shape of a node. Stored on
/// [`crate::gfx_structs::area::GlyphArea`] next to `background_color`
/// and read by both the renderer and the BVH hit test.
///
/// The variant is copied out of the area in the hot paths, so it is
/// intentionally `Copy` and allocation-free.
///
/// # Costs
/// O(1) to copy, hash, compare. No heap allocation.
///
/// `EnumIter` is derived rather than hand-maintained so that
/// [`ShapeSpelling::classify`] enumerates exactly the variants that
/// exist — the same reason `GlyphAreaFieldType` derives it. A
/// hand-written `ALL` array would let a new variant be added without
/// ever becoming parseable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default, EnumIter)]
pub enum NodeShape {
    /// Fills the bounding box exactly — the legacy behaviour and the
    /// default for any node that doesn't opt in to a different shape.
    #[default]
    Rectangle,
    /// Axis-aligned ellipse inscribed in the bounding box. A perfect
    /// circle is expressed as an `Ellipse` with `width == height`;
    /// the same variant handles stretched / "conical" cases where
    /// the box is wider than it is tall (or vice versa) without any
    /// extra parameters.
    Ellipse,
}

/// Shader-side id for [`NodeShape::Rectangle`]. Must match the
/// `SHAPE_RECT` constant in the rect pipeline's WGSL fragment shader.
pub const SHAPE_ID_RECTANGLE: u32 = 0;
/// Shader-side id for [`NodeShape::Ellipse`]. Must match the
/// `SHAPE_ELLIPSE` constant in the rect pipeline's WGSL fragment
/// shader.
pub const SHAPE_ID_ELLIPSE: u32 = 1;

/// Canonical named-enum spellings for `NodeStyle.shape`, as used by
/// `format/enums.md` and by `maptool verify`. Both sides compare the
/// same way and therefore agree by construction:
/// [`ShapeSpelling::classify`] here and `check_value` in
/// `crates/maptool/src/verify/enums.rs` each reach for
/// `eq_ignore_ascii_case` against this list, so `"HEXAGON"` loads
/// quietly *and* verifies clean. `"circle"` is the runtime's one
/// alias for `"ellipse"`; verify has no alias machinery and simply
/// lists it.
///
/// This is the **only** list of canonical spellings any parser
/// consults, and deliberately so: [`ShapeSpelling::classify`] reads
/// it rather than repeating a set of literals, so adding a spelling
/// here makes it a non-warning value at load time and an accepted one
/// under `maptool verify` with nobody editing a parser.
///
/// **Five restatements** of it exist elsewhere, across three files,
/// and none of them is trusted — each is read back and checked
/// against this constant:
///
/// | # | Restatement | Pinned by |
/// | --- | --- | --- |
/// | 1 | the fenced spelling list in `format/enums.md` | `test_shape_format_doc_publishes_exactly_known_shapes` |
/// | 2 | its "live shapes" sentence | the same test |
/// | 3 | its "remaining values" parenthesis | the same test |
/// | 4 | `LEGACY_SHAPE_ORDINALS` in `crates/maptool/src/convert/enums.rs` | maptool's `legacy_shape_ordinals_are_canonical_spellings` |
/// | 5 | the `shape_type` ordinal table in `format/migration.md` | maptool's `legacy_shape_ordinals_match_the_published_table`, against row 4 |
///
/// Row 5 is pinned to row 4 rather than to this constant directly,
/// and that is sufficient because row 4 is pinned here: the two
/// together mean a rename in this list cannot leave the converter
/// writing a spelling `maptool verify` rejects, and cannot leave the
/// published ordinal table describing a conversion the converter no
/// longer performs. `format/enums.md` and `format/migration.md` state
/// the same counts; `test_shape_format_doc_publishes_exactly_known_shapes`
/// and the maptool pair are what make the claim checkable rather than
/// a comment.
///
/// Entries are written lowercase and
/// `test_shape_known_shapes_are_lowercase` pins that — **not**
/// because the runtime and verify would otherwise disagree, since
/// the paragraph above is why they cannot, but because two sibling
/// tests compare with `slice::contains`, which is case-*sensitive*.
/// `do_shape_classification_partitions_by_warning` asks whether a
/// variant's [`NodeShape::style_spellings`] contains the entry, and
/// `do_shape_variant_spellings_are_all_known` asks the mirror
/// question of this list. An uppercase entry moves a spelling across
/// the rendered / not-yet-rendered partition and sends both of them
/// red; the lowercase pin is what names the cause instead of leaving
/// two derived tests failing for a reason neither states.
///
/// Membership here is a claim about the *format*, not about the
/// renderer: most of these have no [`NodeShape`] variant yet and
/// draw as a rectangle until one lands.
pub const KNOWN_SHAPES: &[&str] = &[
    "rectangle",
    "rounded_rectangle",
    "ellipse",
    "circle",
    "diamond",
    "parallelogram",
    "hexagon",
];

/// What a format-level `NodeStyle.shape` string turns out to be,
/// decided without side effects so the decision and the reporting of
/// it can be tested apart.
///
/// The distinction that matters is between a spelling the *format*
/// knows and a spelling nobody knows. [`KNOWN_SHAPES`] is wider than
/// the [`NodeShape`] variant set — `"hexagon"` is published in
/// `format/enums.md`, emitted by `maptool convert --legacy` and
/// accepted by `maptool verify`, yet has no shader case — so
/// collapsing it to `Rectangle` is the intended behavior and there is
/// nothing for an author to fix. Telling them otherwise is noise, and
/// on `maps/testament.mindmap.json` it was 242 lines of it per load
/// (issue #118). A spelling outside `KNOWN_SHAPES` is a different
/// thing entirely: a typo, or a value written by a newer build, and
/// worth a `log::warn!`.
///
/// [`Self::report`] is what turns a classification into the
/// [`ShapeReport`] a load acts on, and
/// [`NodeShape::from_style_string`] is its only caller; anything else
/// that needs to know *why* a shape resolved the way it did — a
/// linter, a script API, an editor surfacing "this shape isn't drawn
/// yet" — reads the classification or the report directly instead of
/// scraping the log.
///
/// # Costs
/// O(1) to copy, hash, compare. Borrows nothing, allocates nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShapeSpelling {
    /// The empty string — the node names no shape at all, which is
    /// not an error and never was. Resolves to the
    /// [`NodeShape::default`] rectangle silently.
    ///
    /// **This is the one place the runtime and `maptool verify`
    /// deliberately disagree, and the disagreement is correct.**
    /// Verify runs `""` through the same `check_value` as every other
    /// spelling, no entry matches, and it is flagged. The two tools
    /// are asked different questions: verify is a linter run before
    /// the map is opened, and an explicit `"shape": ""` is almost
    /// certainly a mistake worth catching there; the loader is on an
    /// interactive path where CODE_CONVENTIONS §9 says degrade and
    /// keep running, and "the field is unset" is the one reading of
    /// an empty string that cannot be an author error. Reaching this
    /// case at all takes an explicit `"shape": ""` in the JSON —
    /// `NodeStyle`'s `#[serde(default = "default_shape")]` fills an
    /// absent key with `"rectangle"` — so the divergence is confined
    /// to a value somebody typed on purpose.
    Unspecified,
    /// A canonical spelling that has a [`NodeShape`] variant behind
    /// it, so the renderer draws what the author asked for.
    Rendered(NodeShape),
    /// Listed in [`KNOWN_SHAPES`], but no [`NodeShape`] variant
    /// exists for it yet. Documented, tool-emitted, round-trips
    /// intact — and draws as a rectangle until a shader case lands.
    /// Not an author error.
    KnownNotYetRendered,
    /// Not in [`KNOWN_SHAPES`] at all: a typo, or a value from a
    /// build that knows more shapes than this one. The only case
    /// that warrants a warning.
    Unrecognized,
}

impl ShapeSpelling {
    /// Classify a `NodeStyle.shape` string. Pure — no logging, no
    /// allocation, no I/O.
    ///
    /// Matching is ASCII-case-insensitive throughout, against the
    /// per-variant spellings from [`NodeShape::style_spellings`]
    /// first and [`KNOWN_SHAPES`] second. The empty string is
    /// [`ShapeSpelling::Unspecified`] rather than unrecognized: it
    /// means "unset", and every `NodeStyle` that omits the field
    /// would otherwise be an error.
    ///
    /// # Costs
    /// At most one `eq_ignore_ascii_case` per canonical spelling —
    /// bounded by `KNOWN_SHAPES.len()` plus the (subset) spellings
    /// of the drawable variants, each O(`s.len()`). No allocation.
    /// Runs once per node per scene rebuild, not per frame.
    pub fn classify(s: &str) -> Self {
        if s.is_empty() {
            return ShapeSpelling::Unspecified;
        }
        for shape in NodeShape::iter() {
            if shape
                .style_spellings()
                .iter()
                .any(|spelling| s.eq_ignore_ascii_case(spelling))
            {
                return ShapeSpelling::Rendered(shape);
            }
        }
        if KNOWN_SHAPES.iter().any(|known| s.eq_ignore_ascii_case(known)) {
            ShapeSpelling::KnownNotYetRendered
        } else {
            ShapeSpelling::Unrecognized
        }
    }

    /// The [`NodeShape`] the renderer and the hit test should use for
    /// this classification. Every non-[`ShapeSpelling::Rendered`]
    /// case degrades to [`NodeShape::Rectangle`] — the bounding box
    /// is the one silhouette that is always correct to fall back to,
    /// because it is what the shape's AABB already occupies.
    ///
    /// # Costs
    /// O(1), branch-only. No allocation. Deliberately not
    /// `#[inline]`: §B7 wants a benchmark that resolves the effect
    /// before the attribute goes on, and this is not a per-frame
    /// call.
    pub const fn resolve(self) -> NodeShape {
        match self {
            ShapeSpelling::Rendered(shape) => shape,
            ShapeSpelling::Unspecified | ShapeSpelling::KnownNotYetRendered | ShapeSpelling::Unrecognized => {
                NodeShape::Rectangle
            }
        }
    }

    /// Is this classification something the map's author can fix?
    ///
    /// Only [`ShapeSpelling::Unrecognized`] is — the other three are
    /// the format working as designed, and issue #118 is what
    /// happened when they were reported as if they were not.
    ///
    /// **This is not a description of what
    /// [`NodeShape::from_style_string`] does; it is what
    /// `from_style_string` calls.** The distinction is the whole
    /// point. An earlier draft had the judgment written out twice —
    /// once here and once as a `match` arm at the log site — which is
    /// precisely the twin surface `lib/baumhard/CONVENTIONS.md` §B4
    /// records as the thing that drifts, and it would have been
    /// introduced by the fix for a drift bug. There is one copy, and
    /// the reporting path consults it, so a consumer with a different
    /// surface (a linter, `maptool`, an editor gutter) asking the same
    /// question gets the same answer the loader acted on rather than a
    /// restatement of it.
    ///
    /// Paired with [`Self::is_quiet_fallback`], which answers the
    /// remaining half of the reporting decision. The two are disjoint
    /// and neither uses a `_` arm, so a fifth classification is a
    /// build error in both. [`Self::report`] is the sole production
    /// consumer of the pair and is what
    /// [`NodeShape::from_style_string`] reads, so hard-wiring this
    /// predicate changes a value a test can hold — not only a log
    /// line no test can see.
    ///
    /// # Costs
    /// O(1), branch-only. No allocation.
    pub const fn is_author_error(self) -> bool {
        match self {
            ShapeSpelling::Unrecognized => true,
            // Spelled out rather than `_` so a fifth classification
            // cannot join the quiet set without a decision.
            ShapeSpelling::Unspecified | ShapeSpelling::Rendered(_) | ShapeSpelling::KnownNotYetRendered => {
                false
            }
        }
    }

    /// Did the renderer substitute a rectangle for a spelling that is
    /// perfectly valid, just not drawable yet?
    ///
    /// True for [`ShapeSpelling::KnownNotYetRendered`] alone.
    /// [`ShapeSpelling::Unspecified`] is *not* a substitution — an
    /// unset field asked for nothing — and
    /// [`ShapeSpelling::Unrecognized`] is
    /// [`Self::is_author_error`]'s business, not this one.
    ///
    /// The counterpart to [`Self::is_author_error`]: together the two
    /// carry the entire "should anyone be told, and how loudly?"
    /// decision, which [`Self::report`] composes into one value and
    /// [`NodeShape::from_style_string`] then routes. Also `pub` for
    /// the same reason — an editor that wants to gray out "drawn as a
    /// rectangle until a shader case lands" should ask rather than
    /// re-derive.
    ///
    /// # Costs
    /// O(1), branch-only. No allocation.
    pub const fn is_quiet_fallback(self) -> bool {
        match self {
            ShapeSpelling::KnownNotYetRendered => true,
            // Spelled out rather than `_`, for the reason
            // `is_author_error` spells its own arms out.
            ShapeSpelling::Unspecified | ShapeSpelling::Rendered(_) | ShapeSpelling::Unrecognized => false,
        }
    }

    /// Should this classification be reported, and as what?
    ///
    /// The entire reporting decision as one pure value, and the only
    /// thing [`NodeShape::from_style_string`] consults. Composed from
    /// [`Self::is_author_error`] and [`Self::is_quiet_fallback`]
    /// rather than from a third `match` over the variants: those two
    /// are the semantic questions a consumer asks, this is the
    /// routing they imply, and there is exactly one copy of each.
    ///
    /// **This is what made the reporting testable.** Before it, the
    /// predicates' only production consumer was a pair of `if`s whose
    /// sole effect was a log line, and with no log sink in this crate
    /// nothing could observe them through the call the loader
    /// actually makes; a test could only ask the predicates the same
    /// question directly. `report` is shipped code on the path
    /// `from_style_string` takes, so a test of it observes the
    /// predicates the way the loader does — everything except which
    /// macro carries the result, which is a source fact and is pinned
    /// as one in `shape_tests.rs`'s `log_routing`.
    ///
    /// The order the two predicates are tested in is not
    /// load-bearing: `do_shape_reporting_predicates_partition`
    /// asserts no classification answers `true` to both. It is
    /// written most-serious-first regardless.
    ///
    /// # Costs
    /// O(1), at most two branch-only predicate calls. No allocation.
    pub const fn report(self) -> Option<ShapeReport> {
        if self.is_author_error() {
            Some(ShapeReport::UnknownSpelling)
        } else if self.is_quiet_fallback() {
            Some(ShapeReport::RectangleSubstituted)
        } else {
            None
        }
    }
}

/// What [`NodeShape::from_style_string`] tells the log about a
/// classification — the reporting decision lifted out of the log site
/// so it is ordinary data instead of a shape of `if`s only a log sink
/// could observe.
///
/// Two questions were fused here and are now separate. *Whether to
/// report, and why* is a property of the classification, and this
/// enum names it. *Which `log::` macro carries it* is a property of
/// the call site, and stays a **literal** macro there deliberately:
/// `log::trace!` expands to `log::log!(Level::Trace, …)`, whose
/// release compile-out is `lvl <= STATIC_MAX_LEVEL` constant-folding
/// on a level the compiler can see (`log-0.4.34/src/macros.rs:137`
/// and `:356`). A `log::log!(computed_level, …)` driven off
/// [`Self::level`] would defeat that fold and keep the `trace!` arm's
/// formatting in every shipped binary, which is the opposite of what
/// CODE_CONVENTIONS §9's release cap buys. So the level is defined
/// once, here, and the call site restates it as a macro name that a
/// source pin holds to this definition.
///
/// # Costs
/// O(1) to copy, hash, compare. No heap allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, EnumIter)]
pub enum ShapeReport {
    /// The spelling is in no list this repo publishes: a typo, or a
    /// value from a build that knows more shapes than this one.
    /// Reported at [`log::Level::Warn`] because an author can act on
    /// it.
    UnknownSpelling,
    /// A canonical spelling with no shader case yet, drawn as a
    /// rectangle. Reported at [`log::Level::Trace`]: the format is
    /// working as designed, and saying otherwise was issue #118 —
    /// 242 warnings per load of `maps/testament.mindmap.json`.
    RectangleSubstituted,
}

impl ShapeReport {
    /// The [`log::Level`] this report is written at. The single
    /// definition of that mapping.
    ///
    /// [`NodeShape::from_style_string`] does not call this — it
    /// reaches a literal `log::` macro per arm, for the
    /// constant-folding reason in the type doc — and
    /// `log_routing::test_from_style_string_log_routing_is_pinned`
    /// derives its expectation from *this* function, so the arm and
    /// the level cannot drift apart without a red test.
    ///
    /// # Costs
    /// O(1), branch-only. No allocation.
    pub const fn level(self) -> log::Level {
        match self {
            ShapeReport::UnknownSpelling => log::Level::Warn,
            ShapeReport::RectangleSubstituted => log::Level::Trace,
        }
    }
}

impl NodeShape {
    /// Stable id fed to the fragment shader. Must stay in lock-step
    /// with the `SHAPE_*` constants in
    /// `src/application/renderer/mod.rs` — adding a variant without
    /// adding its shader case would render the new shape as a
    /// rectangle. The lock-step is checked rather than asked for:
    /// `test_every_node_shape_has_a_matching_wgsl_constant_and_case_arm`
    /// parses the shader text and holds it against this function over
    /// `NodeShape::iter()`. O(1).
    #[inline]
    pub const fn shader_id(self) -> u32 {
        match self {
            NodeShape::Rectangle => SHAPE_ID_RECTANGLE,
            NodeShape::Ellipse => SHAPE_ID_ELLIPSE,
        }
    }

    /// The canonical `NodeStyle.shape` spellings that resolve to this
    /// variant, in the order [`ShapeSpelling::classify`] tries them.
    /// Every returned spelling must also appear in [`KNOWN_SHAPES`]
    /// — `test_shape_variant_spellings_are_all_known` pins that, so a
    /// variant cannot claim a spelling `maptool verify` would reject.
    ///
    /// The `match` is exhaustive on purpose. It is the guarantee that
    /// adding a `NodeShape` variant is a **build error** rather than
    /// a silent no-op: a new variant with no spelling here would
    /// otherwise be unreachable from the format, and its canonical
    /// name would keep resolving to a quiet `Rectangle` forever.
    ///
    /// `"circle"` is the one alias: a `width == height` ellipse *is*
    /// a circle, and it is the spelling authors reach for first. It
    /// costs nothing to accept and round-trips intact, because
    /// `NodeStyle.shape` stays a free-form `String` at the format
    /// layer and is never written back from here.
    ///
    /// # Costs
    /// O(1) — returns a `'static` slice. No allocation.
    pub const fn style_spellings(self) -> &'static [&'static str] {
        match self {
            NodeShape::Rectangle => &["rectangle"],
            NodeShape::Ellipse => &["ellipse", "circle"],
        }
    }

    /// Parse the format-level `NodeStyle.shape` string into the shape
    /// the renderer and hit test should use, reporting only the case
    /// an author can act on.
    ///
    /// The classification is [`ShapeSpelling::classify`]'s and the
    /// routing is [`ShapeSpelling::report`]'s; both are pure. This
    /// function is the one place that turns a report into a log line,
    /// which is what keeps everything upstream of it testable without
    /// a logger:
    ///
    /// - a canonical spelling with a variant renders as asked;
    /// - the empty string means "unset" and is silent;
    /// - a [`KNOWN_SHAPES`] spelling with no variant yet
    ///   (`"hexagon"`, `"diamond"`, `"parallelogram"`,
    ///   `"rounded_rectangle"`) falls back to
    ///   [`NodeShape::Rectangle`] at `log::trace!`. It is documented,
    ///   tool-emitted and correct; warning about it told authors a
    ///   valid value was unknown, 242 times per load of the demo map
    ///   (issue #118);
    /// - anything else keeps the `log::warn!`, because there it is
    ///   accurate.
    ///
    /// **This function authors no part of that judgment.** It calls
    /// [`ShapeSpelling::report`], which composes
    /// [`ShapeSpelling::is_author_error`] and
    /// [`ShapeSpelling::is_quiet_fallback`] into a
    /// [`ShapeReport`], and then does one thing: pick the literal
    /// `log::` macro that carries each report. Splitting it that way
    /// is what makes the whole decision testable — `report` is
    /// ordinary shipped code returning an ordinary value, so a test
    /// reaches it through the same path the loader takes, and the
    /// only fact left over is which macro sits in which arm.
    ///
    /// That leftover is a fact about *source text*, and
    /// `log_routing::test_from_style_string_log_routing_is_pinned`
    /// reads it as one: it parses this file with `syn` and checks
    /// this body against a **whitelist** — three statements, exactly
    /// the ones written below; one `match` arm per [`ShapeReport`]
    /// variant plus the `None` arm; each reporting arm a bare
    /// `log::<macro>!` whose name equals
    /// [`ShapeReport::level`]'s answer for the variant that arm
    /// matches; and no attribute anywhere in the body but a doc
    /// comment. Anything the whitelist does not model — a condition
    /// wrapped around the `match`, a `#[cfg]` on an arm, an extra
    /// statement that logs from inside a loop — fails it by *not
    /// matching*, rather than passing because the check happened not
    /// to look there. That is the property the round-2 walker did not
    /// have.
    ///
    /// Exhaustiveness is threefold and none of it is a `_` arm: the
    /// `match` below covers `Option<ShapeReport>` completely, both
    /// predicates behind `report` match every [`ShapeSpelling`]
    /// variant, and the pin requires one arm per [`ShapeReport`]
    /// variant. A fifth classification or a third report kind is a
    /// build error or a red test, never a silent default.
    ///
    /// Unknown values stay on disk untouched either way, so a
    /// round-trip through `maptool convert` doesn't lose them.
    ///
    /// # Costs
    /// [`ShapeSpelling::classify`]'s compares plus at most two
    /// branch-only predicate calls. No allocation on any path; the
    /// `trace!` arm compiles out entirely in release
    /// (`release_max_level_warn`), which is why the arm names the
    /// macro literally instead of calling [`ShapeReport::level`].
    pub fn from_style_string(s: &str) -> Self {
        let spelling = ShapeSpelling::classify(s);
        match spelling.report() {
            Some(ShapeReport::UnknownSpelling) => log::warn!(
                "shape: unknown shape {s:?}, \
                 falling back to Rectangle"
            ),
            Some(ShapeReport::RectangleSubstituted) => log::trace!(
                "shape: {s:?} is a canonical shape with no renderer yet, \
                 drawing it as Rectangle"
            ),
            None => {}
        }
        spelling.resolve()
    }

    /// Point-in-shape test in the node's **local** coordinate space,
    /// where the bounding box runs from `(0, 0)` to `bounds`.
    ///
    /// Callers pre-translate into that frame, and **which two values
    /// they subtract matters**: derive `local` and `bounds` from the
    /// same `min` / `max` pair, as
    /// `local = point - min`, `bounds = max - min`. Recomputing
    /// either side independently (say `local = point - position`
    /// against a separately-stored extent) makes the boundary
    /// compare inexact — f32 addition is not associative, so
    /// `(position + extent) - position` can exceed `extent` by an
    /// ULP and a point exactly on the far edge then reports `false`.
    /// `bvh_find` in
    /// [`tree_walker`](crate::gfx_structs::tree_walker) is the
    /// reference caller.
    ///
    /// A degenerate `bounds` (either dimension `<= 0`) always
    /// reports `false`, matching how the BVH skips zero-size areas
    /// at the AABB stage.
    ///
    /// O(1). No allocation.
    #[inline]
    pub fn contains_local(self, local: Vec2, bounds: Vec2) -> bool {
        if bounds.x <= 0.0 || bounds.y <= 0.0 {
            return false;
        }
        match self {
            NodeShape::Rectangle => aabb_contains(local, Vec2::ZERO, bounds),
            NodeShape::Ellipse => {
                // Normalised coordinates in [-1, 1] relative to the
                // ellipse center. A perfect circle is `bounds.x ==
                // bounds.y`; a stretched conic is anything else.
                let rx = bounds.x * 0.5;
                let ry = bounds.y * 0.5;
                let nx = (local.x - rx) / rx;
                let ny = (local.y - ry) / ry;
                nx * nx + ny * ny <= 1.0
            }
        }
    }

    /// Does the shape's filled area overlap the AABB
    /// `[min, max]` (in local coordinates, same frame as
    /// [`Self::contains_local`])? Conservative — false positives
    /// are tolerated, false negatives are not. Used by rect-select.
    ///
    /// For the ellipse variant this clamps the AABB to the
    /// ellipse's bounding box, then checks whether the closest
    /// clamped point sits inside the ellipse. That's conservative
    /// in the "selection rect fully inside the ellipse" corner
    /// (the closest-point test returns true when any corner of the
    /// rect is inside the ellipse, *or* when the ellipse-center is
    /// inside the rect) — which is what we want for a lasso.
    ///
    /// Degenerate bounds (zero or negative extent on either axis)
    /// report `false` even if the AABBs would overlap numerically.
    /// This matches [`Self::contains_local`] and the BVH's
    /// `bounds.x > 0.0 && bounds.y > 0.0` guard in `bvh_descend`:
    /// a zero-size node renders nothing, so selecting nothing for
    /// it is the internally consistent answer. Small behaviour
    /// change from the pre-shape `rect_select` (which would have
    /// matched a point-sized node under the cursor) — considered
    /// an improvement and noted here so a future test author can
    /// find the rationale.
    ///
    /// O(1). No allocation.
    #[inline]
    pub fn intersects_local_aabb(self, min: Vec2, max: Vec2, bounds: Vec2) -> bool {
        if bounds.x <= 0.0 || bounds.y <= 0.0 {
            return false;
        }
        // First, AABB–AABB overlap. Bails on any shape whose bounds
        // don't touch the selection rectangle at all.
        if max.x < 0.0 || min.x > bounds.x || max.y < 0.0 || min.y > bounds.y {
            return false;
        }
        match self {
            NodeShape::Rectangle => true,
            NodeShape::Ellipse => {
                let rx = bounds.x * 0.5;
                let ry = bounds.y * 0.5;
                let cx = rx;
                let cy = ry;
                // Closest point on the AABB to the ellipse center.
                let clamped_x = cx.clamp(min.x, max.x);
                let clamped_y = cy.clamp(min.y, max.y);
                let nx = (clamped_x - cx) / rx;
                let ny = (clamped_y - cy) / ry;
                nx * nx + ny * ny <= 1.0
            }
        }
    }
}

// Tests live out-of-line at
// `lib/baumhard/src/gfx_structs/tests/shape_tests.rs` so the
// criterion bench harness at `lib/baumhard/benches/test_bench.rs`
// can reuse each `do_*()` body as a micro-benchmark — see
// TEST_CONVENTIONS.md §T2.2.
