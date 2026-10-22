// SPDX-License-Identifier: MPL-2.0

//! `ColorFontRegions` ↔ cosmic-text styling bridges.
//!
//! Two shapes, one resolver — bridges baumhard's `ColorFontRegions`
//! (the model-level representation of styled text runs) into either
//! cosmic-text API shape:
//!
//! - [`crate::font::attrs::attrs_list_from_regions`] returns an
//!   `AttrsList` for callers using `Editor::insert_string`.
//! - [`crate::font::attrs::RegionFamilies`] +
//!   [`crate::font::attrs::rich_text_spans_from_regions`] returns
//!   a `Vec<(&str, Attrs)>` for callers using `Buffer::set_rich_text`.
//!   (Full crate paths because rustdoc resolves intra-doc links inside
//!   `//!` inner-module docs against the parent module's namespace,
//!   not this module's own — `self::` and bare names both fail.)
//!
//! Both honor the per-region color, font pin, and grapheme-aware
//! byte slicing — `Range` indices on `ColorFontRegion` carry
//! grapheme-cluster offsets (see `CONCEPTS.md`'s `Range` entry,
//! `format/text-runs.md`, and `lib/baumhard/CONVENTIONS.md §B1`).
//! The shared private `resolve_font_family` keeps the lookup +
//! fallback discipline in one place — see `CODE_CONVENTIONS.md`
//! §1 and `lib/baumhard/CONVENTIONS.md` §B5.
//!
//! ## Consumers
//!
//! This is the single owner of the `(ColorFontRegions, &mut
//! FontSystem) → cosmic-text` bridge. New renderer-side code
//! that needs styled spans MUST route through here rather than
//! reinventing the bridge inline (`CODE_CONVENTIONS.md` §1).
//!
//! Current consumers — keep this list current when adding a new
//! call site:
//!
//! - `src/application/renderer/tree_walker.rs` — main
//!   tree-to-buffer walker for nodes / connections / portals
//!   (Baumhard tree path). Uses the
//!   [`crate::font::attrs::RegionFamilies`] +
//!   [`crate::font::attrs::rich_text_spans_into`] pair.
//!
//! [`crate::font::attrs::attrs_list_from_regions`] has no in-tree
//! consumer today: it is the `Editor::insert_string` half of the
//! bridge, kept as the named seam for the inline text editor
//! (§B10). The list above
//! used to claim `lib/baumhard/src/mindmap/tree_builder/border.rs`
//! as a consumer of this module; that file imports nothing from
//! `font::attrs`, so the claim is removed rather than carried
//! forward (§B9 — a doc comment that lies is worse than none).

use cosmic_text::{Attrs, AttrsList, Color, Family, FontSystem, Metrics, Style};
use log::warn;

use crate::core::primitives::{ColorFontRegion, ColorFontRegions};
use crate::font::fonts::COMPILED_FONT_ID_MAP;
use crate::util::color::{convert_f32_to_u8, FloatRgba};
use crate::util::grapheme_chad;

/// Build a cosmic-text `AttrsList` from a `ColorFontRegions` source
/// over `text`.
///
/// One span is emitted per region. A region with `color = Some(rgba)`
/// gets that color; otherwise the span uses cosmic-text's default. A
/// region with `font = Some(id)` resolves to that font family; an
/// unknown or unresolvable font id falls back to `Family::Monospace`
/// with a warning — this function runs inside the renderer's frame
/// loop and a corrupt save must not abort it.
///
/// `text` is required because `AttrsList::add_span` expects byte
/// ranges into the text it styles, while `Range` on the data layer
/// carries grapheme-cluster indices (the unit baumhard's text
/// primitives speak in — see `lib/baumhard/CONVENTIONS.md §B3` and
/// `CONCEPTS.md`'s `Range` entry). The conversion goes through
/// `region_byte_bounds` so a region whose end lands on a
/// ZWJ-emoji or combining-mark cluster boundary produces a
/// UTF-8-valid byte range that matches the visual glyph.
///
/// Cost: O(n_text + n_regions) **when the regions are sorted**,
/// which the format guarantees and the loader enforces; the whole
/// run table is then converted in a single grapheme walk rather than
/// one walk per boundary. An out-of-order region falls back to its
/// own walk and costs O(n_text) for itself — correct, just not
/// linear. Plus one `font_system.db().face()` lookup per region
/// carrying a font id, and the three `Vec`s `region_byte_bounds`
/// builds — the flattened boundary list and the resolved indices,
/// each twice the region count, plus the returned pairs at the
/// region count.
///
/// The caller is expected to hold the `FONT_SYSTEM` write lock for
/// the same scope it uses the returned list — that's how the
/// renderer wires it today.
pub fn attrs_list_from_regions(
    text: &str,
    source: &ColorFontRegions,
    font_system: &mut FontSystem,
) -> AttrsList {
    let mut attr_list = AttrsList::new(&Attrs::new());
    // One walk for the whole table — see `region_byte_bounds`.
    let bounds = region_byte_bounds(text, source.regions.iter().map(|r| &r.range));
    for (region, (start, end)) in source.regions.iter().zip(bounds) {
        let mut attrs = Attrs::new().style(Style::Normal);

        if let Some(color) = region.color {
            attrs = attrs.color(rgba_to_color(color));
        }

        // Resolve the font family. Both miss paths (compiled-id map
        // miss, fontdb face miss) fall back to Monospace with a
        // warning — consistent with §4's "degrade the frame, not
        // abort the process" rule.
        let family = resolve_font_family(region.font.as_ref(), font_system);
        attrs = match family {
            Some(ref name) => attrs.family(Family::Name(name.as_str())),
            None => attrs.family(Family::Monospace),
        };
        if start >= end {
            continue;
        }
        attr_list.add_span(start..end, &attrs);
    }
    attr_list
}

/// Byte `(start, end)` for every region range, resolved in a single
/// grapheme walk over `text`.
///
/// Grapheme-correct slicing is the contract: a `Range` carries
/// grapheme-cluster indices (CONCEPTS.md, §B3 in
/// `lib/baumhard/CONVENTIONS.md`, `format/text-runs.md`), so the
/// conversion has to count clusters rather than bytes. Doing that
/// per boundary restarts the walk each time, which is O(text ×
/// regions) — quadratic on a section whose run table scales with
/// its text, and reachable from any `.mindmap.json`. Both cosmic-text
/// bridges route through here so neither can regress to the
/// per-boundary form.
///
/// An index past the end resolves to `text.len()`, which the
/// callers' `start >= end` guard then drops.
///
/// Cost: O(text + regions) and three `Vec`s — the flattened
/// boundary list and the resolved indices, both `2 × regions`, plus
/// the returned pairs at `regions`.
fn region_byte_bounds<'r>(
    text: &str,
    ranges: impl Iterator<Item = &'r crate::core::primitives::Range>,
) -> Vec<(usize, usize)> {
    let flat: Vec<usize> = ranges.flat_map(|r| [r.start, r.end]).collect();
    let resolved = grapheme_chad::byte_indices_of_graphemes(text, &flat);
    resolved
        .chunks(2)
        .map(|pair| {
            (
                pair[0].unwrap_or(text.len()),
                pair.get(1).copied().flatten().unwrap_or(text.len()),
            )
        })
        .collect()
}

/// Look up the font-family name for a compiled font id. Returns
/// `None` silently when `font_id` is `None` (the region asked for no
/// pin); returns `None` with a `log::warn!` when `font_id` is
/// `Some` but the lookup misses (corrupt save / build skew).
/// Callers decide what `None` means at the cosmic-text seam:
/// [`attrs_list_from_regions`] forces `Family::Monospace`,
/// [`rich_text_spans_from_regions`] omits the family pin entirely.
fn resolve_font_family(
    font_id: Option<&crate::font::fonts::AppFont>,
    font_system: &mut FontSystem,
) -> Option<String> {
    let font_id = font_id?;
    let face_ids = match COMPILED_FONT_ID_MAP.get(font_id) {
        Some(ids) if !ids.is_empty() => ids,
        _ => {
            warn!("font::attrs: unknown font id {font_id:?}, dropping family pin");
            return None;
        }
    };
    let face = match font_system.db().face(face_ids[0]) {
        Some(face) => face,
        None => {
            warn!("font::attrs: fontdb face miss for {font_id:?}, dropping family pin");
            return None;
        }
    };
    // `face.families` is documented as non-empty by fontdb, but the
    // crate doesn't enforce it via the type system; a corrupt face
    // table (or a ttf with zero name records) would slip through. Use
    // `first()` rather than `[0]` so we degrade with a warn rather
    // than panicking inside the renderer's frame loop (§9).
    match face.families.first() {
        Some((name, _)) => Some(name.clone()),
        None => {
            warn!("font::attrs: face for {font_id:?} has no family records, dropping family pin");
            None
        }
    }
}

/// A styled text run resolved down to everything cosmic-text needs
/// except the metrics and the halo color: the text it styles, the
/// borrowed regions, their family-name strings, and their byte
/// ranges into that text.
///
/// Built once per text area and reused across multiple shape passes
/// — typically the renderer's main glyph pass plus eight
/// outline-halo stamps. Reuse across those passes avoids, per pass:
/// - re-allocating the `Vec<&ColorFontRegion>` slice from
///   `all_regions()`.
/// - re-running `font_system.db().face(...)` per region.
/// - re-walking `text`'s grapheme boundaries to turn every region's
///   cluster range into a byte range (`region_byte_bounds`).
///
/// The text is held rather than passed per call on purpose: the
/// cached byte ranges are only meaningful against the string they
/// were resolved from, so making the caller re-supply it would let
/// a mismatched pair slice the wrong bytes.
///
/// Lifetime `'r` ties to both the source `ColorFontRegions` and the
/// text, so neither the cached slice nor the byte ranges can outlive
/// what they describe.
pub struct RegionFamilies<'r> {
    /// The text the regions style, and the string every emitted span
    /// slices out of.
    text: &'r str,
    /// Borrowed regions slice, allocated once at resolve time.
    regions: Vec<&'r ColorFontRegion>,
    /// Resolved family-name string per region; same indexing as
    /// `regions`. `None` means the region had no font pin, or the
    /// pin's lookup missed (logged via `log::warn!` at resolve time).
    names: Vec<Option<String>>,
    /// Byte `(start, end)` into `text` per region; same indexing as
    /// `regions`. Resolved in one grapheme walk at resolve time (see
    /// `region_byte_bounds`) rather than once per shape pass.
    bounds: Vec<(usize, usize)>,
}

impl<'r> RegionFamilies<'r> {
    /// Resolve every region's family-name string and byte range
    /// against `text`, and cache the borrowed regions slice, for
    /// downstream shape passes.
    ///
    /// Empty input (`source.num_regions() == 0`) returns without
    /// touching `font_system` and without allocating any inner
    /// `Vec` — the empty case is the common case for text without
    /// styled runs.
    ///
    /// Cost: O(n_text + n_regions) — one grapheme walk over `text`
    /// for the whole run table — plus one `font_system.db().face()`
    /// lookup per region with a font id. Allocates one
    /// `Vec<&ColorFontRegion>` (from `source.all_regions()`), one
    /// `Vec<Option<String>>`, one `Vec<(usize, usize)>`, and the
    /// three interior `Vec`s `region_byte_bounds` builds. The caller
    /// holds the `font_system` write guard for the duration;
    /// downstream shape passes that consult the result need their
    /// own access to the same guard scope.
    pub fn resolve(text: &'r str, source: &'r ColorFontRegions, font_system: &mut FontSystem) -> Self {
        if source.num_regions() == 0 {
            return Self {
                text,
                regions: Vec::new(),
                names: Vec::new(),
                bounds: Vec::new(),
            };
        }
        let regions = source.all_regions();
        let names = regions
            .iter()
            .map(|region| resolve_font_family(region.font.as_ref(), font_system))
            .collect();
        // One walk for the whole table — see `region_byte_bounds`.
        let bounds = region_byte_bounds(text, regions.iter().map(|r| &r.range));
        Self {
            text,
            regions,
            names,
            bounds,
        }
    }
}

/// Fill `out` with the `(text_slice, Attrs)` span list ready for
/// `Buffer::set_rich_text` from a pre-resolved [`RegionFamilies`].
/// One span per region; spans whose `[start, end)` byte slice is
/// empty (out-of-range or zero-width region) are dropped silently —
/// the renderer must not hand cosmic-text zero-width spans.
///
/// `out` is cleared first, so a caller shaping the same run several
/// times in a row (the main glyph plus its halo stamps) can hand the
/// same `Vec` back and pay for its capacity once instead of once per
/// stamp. That is why this, rather than
/// [`rich_text_spans_from_regions`], is the shape the renderer's
/// halo loop calls.
///
/// `color_override = Some(c)` recolors **every** span to `c` — used
/// by outline-halo passes to stamp the same glyphs in the halo
/// color while preserving per-region font pins. `None` keeps each
/// region's own `region.color` (cosmic-text default for `None`).
///
/// Each span carries `Metrics::new(scale, line_height)` so per-area
/// metrics survive cosmic-text shaping. Empty input (the
/// `RegionFamilies` was resolved from a `ColorFontRegions` with no
/// regions) produces a single span over the whole text — the
/// data-model contract that "no regions" means "uniform default
/// styling".
///
/// Cost: O(n_regions) iteration over the cached slice, no grapheme
/// walk (the byte ranges were resolved once at
/// [`RegionFamilies::resolve`] time) and no allocation beyond
/// whatever growth `out` needs. The emitted `Attrs<'a>` borrow
/// strings from `families`, so `families` must outlive `out`.
pub fn rich_text_spans_into<'a>(
    families: &'a RegionFamilies<'a>,
    scale: f32,
    line_height: f32,
    color_override: Option<Color>,
    out: &mut Vec<(&'a str, Attrs<'a>)>,
) {
    out.clear();
    let metrics = Metrics::new(scale, line_height);
    if families.regions.is_empty() {
        let mut attrs = Attrs::new().metrics(metrics);
        if let Some(c) = color_override {
            attrs = attrs.color(c);
        }
        out.push((families.text, attrs));
        return;
    }
    for ((region, family_name), &(start, end)) in families
        .regions
        .iter()
        .zip(families.names.iter())
        .zip(families.bounds.iter())
    {
        if start >= end {
            continue;
        }
        let slice = &families.text[start..end];
        let mut attrs = Attrs::new().metrics(metrics);
        let color = color_override.or_else(|| region.color.map(rgba_to_color));
        if let Some(c) = color {
            attrs = attrs.color(c);
        }
        if let Some(family) = family_name.as_deref() {
            attrs = attrs.family(Family::Name(family));
        }
        out.push((slice, attrs));
    }
}

/// [`rich_text_spans_into`] into a freshly allocated `Vec` — the
/// convenience shape for callers that shape a run exactly once and
/// have no buffer to reuse. Same span semantics; see that function
/// for the contract.
///
/// Cost: as [`rich_text_spans_into`], plus one `Vec` allocation. A
/// caller shaping the same run more than once should reuse a buffer
/// through [`rich_text_spans_into`] instead.
pub fn rich_text_spans_from_regions<'a>(
    families: &'a RegionFamilies<'a>,
    scale: f32,
    line_height: f32,
    color_override: Option<Color>,
) -> Vec<(&'a str, Attrs<'a>)> {
    let mut out = Vec::with_capacity(families.regions.len().max(1));
    rich_text_spans_into(families, scale, line_height, color_override, &mut out);
    out
}

/// Pack a `[f32; 4]` baumhard color into a cosmic-text `Color`.
/// Per-channel `[0.0, 1.0]` → `[0, 255]` via `convert_f32_to_u8`.
fn rgba_to_color(rgba: FloatRgba) -> Color {
    let u8c = convert_f32_to_u8(&rgba);
    Color::rgba(u8c[0], u8c[1], u8c[2], u8c[3])
}

// Tests live in `font/tests/attrs_tests.rs` as `pub mod` so the
// criterion bench harness can reuse `do_*` bodies (§B8 / §T2.2).
