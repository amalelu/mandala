//! Portals — Session 6E cross-canvas marker pairs. Two matching glyph
//! markers, one placed on each of two distant nodes, act as a
//! lightweight visual link without rendering a connection line across
//! the canvas. This module carries the `PortalPair` record, the
//! glyph preset rotation palette, and the Excel-style column-letter
//! label generator `MindMap::next_portal_label` walks.

use serde::{Deserialize, Serialize};

/// Session 6E: a portal pair — two matching glyph markers placed on
/// two distant nodes so users can visually link them without rendering
/// a connection line across the canvas. Portals are a lightweight
/// alternative to cross-link edges for very far-apart nodes.
///
/// Each pair produces *two* rendered markers (one per endpoint node),
/// both showing the same `glyph` and `color`. The `label` is an
/// auto-assigned identifier ("A", "B", ..., "AA"...) used for stable
/// identity in selection/undo; it is not currently drawn next to the
/// glyph — the glyph alone is the visual cue. Labels are immutable in
/// Session 6E; a rename action is deferred.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortalPair {
    /// Node id of the first endpoint.
    pub endpoint_a: String,
    /// Node id of the second endpoint.
    pub endpoint_b: String,
    /// Auto-assigned stable identifier: "A", "B", ..., "Z", "AA", "AB"...
    /// Picked by `MindMap::next_portal_label` to be the lowest unused
    /// letter in column-letter order at creation time.
    pub label: String,
    /// The visible marker glyph, e.g. "◈", "◆", "⬡". Rotated from
    /// `PORTAL_GLYPH_PRESETS` at creation time, editable via the
    /// Session 6E palette actions.
    pub glyph: String,
    /// Marker color — `#RRGGBB` hex or `var(--name)`. Resolved through
    /// the theme variable map at scene-build time, so `var(--accent)`
    /// auto-restyles on theme swap (see `connection labels` for the
    /// parallel pattern at `scene_builder.rs` Session 6D).
    pub color: String,
    /// Point size of the rendered glyph. Defaults to 16.0 so markers
    /// are a hair larger than body text for legibility without
    /// dominating the node they sit next to.
    #[serde(default = "default_portal_font_size")]
    pub font_size_pt: f32,
    /// Optional font family override. `None` falls back to the
    /// renderer's default font.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
}

fn default_portal_font_size() -> f32 { 16.0 }

/// Convert a 1-indexed ordinal into an Excel-style column letter label:
/// `1 → "A"`, `26 → "Z"`, `27 → "AA"`, `28 → "AB"`, `702 → "ZZ"`,
/// `703 → "AAA"`, and so on. Used by `MindMap::next_portal_label` to
/// walk a lazy sequence of candidate portal labels. Panics on `0`.
pub fn column_letter_label(mut n: u64) -> String {
    assert!(n > 0, "column_letter_label is 1-indexed");
    let mut s = String::new();
    while n > 0 {
        n -= 1;
        s.insert(0, (b'A' + (n % 26) as u8) as char);
        n /= 26;
    }
    s
}

/// Rotation palette used by `MindMapDocument::apply_create_portal`
/// to pick a distinct default glyph for each new portal without
/// requiring the user to choose up front. Indexed by
/// `portals.len() % PORTAL_GLYPH_PRESETS.len()` at creation time.
pub const PORTAL_GLYPH_PRESETS: &[&str] = &[
    "\u{25C8}", // ◈ white diamond containing black small diamond
    "\u{25C6}", // ◆ black diamond
    "\u{2B21}", // ⬡ white hexagon
    "\u{2B22}", // ⬢ black hexagon
    "\u{25C9}", // ◉ fisheye
    "\u{2756}", // ❖ black diamond minus white X
    "\u{2726}", // ✦ black four pointed star
    "\u{2727}", // ✧ white four pointed star
];
