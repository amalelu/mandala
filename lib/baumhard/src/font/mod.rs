// SPDX-License-Identifier: MPL-2.0

//! Cosmic-text font integration — the single blessed boundary
//! between baumhard's styled-region data model and the underlying
//! font system. `fonts` owns the compiled font table and the shared
//! `FONT_SYSTEM`; `attrs` translates `ColorFontRegions` into
//! cosmic-text `AttrsList`s.

/// `ColorFontRegions` → cosmic-text bridges
/// ([`attrs::attrs_list_from_regions`] for `Editor::insert_string`,
/// [`attrs::RegionFamilies`] +
/// [`attrs::rich_text_spans_from_regions`] for
/// `Buffer::set_rich_text`).
pub mod attrs;
/// Float-RGBA ↔ `cosmic_text::Color` bridge — the boundary helpers
/// every render path uses to hand cosmic-text a color or read one
/// back. Single source of byte↔float quantization at the cosmic-text
/// wall.
pub mod color;
/// Compiled-in font table, shared `FONT_SYSTEM`, cosmic-text editor
/// factories, and the text-measurement primitives.
pub mod fonts;
/// Hex-string → `cosmic_text::Color` bridge — the single entry point
/// renderer code uses to resolve a theme-variable hex into the
/// cosmic-text color type without importing `cosmic_text` itself.
/// Routes through [`color::cosmic_color_from_rgba`] for the actual
/// quantization.
pub mod hex;
/// Font-metric approximations (`monospace_advance` + the underlying
/// `MONOSPACE_ADVANCE_RATIO`) usable without a live `FontSystem`.
/// Deprecated in favor of [`metric_cache::glyph_advance`] for any
/// caller that has an `AppFont` in scope — the cache returns the
/// measured advance from cosmic-text and the ratio's static
/// calibration drift goes away.
pub mod metrics;
/// Per-`(face, size, grapheme)` measured glyph advances + ink
/// heights, cached. Replaces `metrics::monospace_advance` at every
/// border-rail callsite.
pub mod metric_cache;
/// Naming and ordering rules for the compiled-in font table.
///
/// `build.rs` scans `src/font/fonts/` and emits the `AppFont` enum
/// plus the `FONT_DATA` byte table into `OUT_DIR`. Every decision
/// that turns a directory listing into that generated source —
/// which files count as fonts, how a font's `name` table becomes a
/// Rust identifier, which of two files wins a collision, and what
/// order the variants are emitted in — lives here rather than in
/// the build script, for two reasons:
///
/// 1. **It is testable.** Cargo never compiles a build script under
///    `cfg(test)`, so logic that lives only in `build.rs` cannot be
///    unit-tested. This module is `include!`d by `build.rs` *and*
///    compiled into the library, so the same code the build script
///    runs is the code the test suite exercises.
/// 2. **It is the documented contract.** "Drop a font file into
///    `src/font/fonts/`, recompile, and the variant appears"
///    (CONCEPTS §2) is a promise about these rules; a reader
///    looking for what variant name their file will produce should
///    find the answer in the library, not in a build script.
///
/// Everything here is a pure function over plain values: no
/// filesystem access, no `ttf-parser`, no allocations beyond the
/// returned `String`s. The build script owns the I/O and the
/// OpenType parsing and feeds the results in.
///
/// # Determinism
///
/// [`name_rules::select_font_variants`] is a total function of its
/// input *set*: the order candidates are discovered in cannot
/// affect the result, because every tie is broken by an explicit
/// comparator rather than by insertion order. That is what makes
/// two clean builds emit byte-identical generated source.
pub mod name_rules;
/// Test bodies exposed via `pub mod tests` so `benches/test_bench.rs`
/// can reuse the `do_*()` functions as micro-benchmarks (§B8).
pub mod tests;

// ── cosmic-text re-exports ───────────────────────────────────────
//
// Renderer code reaches a `cosmic_text::Buffer` (and its companion
// types) through `baumhard::font::*` instead of `use cosmic_text`
// directly. The crate boundary stays here; a future swap to a
// different shaper / layout engine is one edit per site below.

/// Cosmic-text `Attrs` — span-level styling threaded into
/// `Buffer::set_rich_text` and `Buffer::set_text`.
pub use cosmic_text::Attrs;
/// Cosmic-text `Align` — text alignment passed to the rich-text /
/// set-text API.
pub use cosmic_text::Align;
/// Cosmic-text `Buffer` — the shaped glyph cache the renderer hands
/// to glyphon. Constructors are wrapped in [`buffer::create`] so the
/// `Metrics::new(...)` boilerplate doesn't repeat at every callsite.
pub use cosmic_text::Buffer;
/// Packed-RGBA color. See [`COLOR_WHITE`] / [`COLOR_BLACK`] for the
/// common defaults the renderer used to write inline.
pub use cosmic_text::Color;
/// Long-lived font system (database + atlas). One per process;
/// owned by [`fonts::FONT_SYSTEM`].
pub use cosmic_text::FontSystem;
/// Per-buffer font / line-height metrics — one `Metrics::new(...)`
/// per [`Buffer`] construction.
pub use cosmic_text::Metrics;
/// Shaping mode passed to `Buffer::set_rich_text`. The blessed
/// default is [`SHAPING_ADVANCED`] — every Mandala render path uses
/// it for non-Latin fallback support.
pub use cosmic_text::Shaping;
/// Glyph-rasterization cache. Owned by the caller — one per
/// measurement pass — so repeated calls share rasterization work.
pub use cosmic_text::SwashCache;

/// Opaque white in cosmic-text's packed-RGBA representation. Used
/// as the FPS-overlay default and the unbranded text-renderer
/// fallback color.
pub const COLOR_WHITE: Color = Color::rgba(255, 255, 255, 255);
/// Opaque black in cosmic-text's packed-RGBA representation. Used
/// as the universal fallback when a theme-variable lookup fails.
pub const COLOR_BLACK: Color = Color::rgba(0, 0, 0, 255);
/// The shaping mode every Mandala render path uses. `Advanced`
/// enables script-aware shaping (cluster boundaries, complex
/// scripts) at the cost of a slower fast path; pinned here so a
/// future tweak is one edit.
pub const SHAPING_ADVANCED: Shaping = Shaping::Advanced;

/// `Buffer` constructor wrappers. The renderer used to inline the
/// `Buffer::new(font_system, Metrics::new(font_size, line_height))`
/// pair at four call sites; this submodule pins the pair once.
pub mod buffer {
    use super::{Buffer, FontSystem, Metrics};

    /// Create a fresh shaped-text buffer at the given font size and
    /// line height. Inlines the `Metrics::new(...)` step the
    /// renderer used to repeat per call site.
    #[inline]
    pub fn create(font_system: &mut FontSystem, font_size: f32, line_height: f32) -> Buffer {
        Buffer::new(font_system, Metrics::new(font_size, line_height))
    }

    /// Create a fresh shaped-text buffer with `font_size == line_height`
    /// (the common case for square-cell glyph rendering).
    #[inline]
    pub fn create_square(font_system: &mut FontSystem, font_size: f32) -> Buffer {
        create(font_system, font_size, font_size)
    }
}
