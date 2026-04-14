//! Picker geometry — the plain-data struct the app builds each
//! frame and hands to the layout fn. No rendering primitives, no
//! GPU references; mirrors `PaletteOverlayGeometry` in shape so
//! the two overlays feel similar at the call sites.

use super::glyph_tables::CROSSHAIR_CENTER_CELL;
use super::hit::PickerHit;

/// Pre-render geometry pushed from the app to the renderer. Plain data,
/// no rendering primitives — mirrors `PaletteOverlayGeometry`.
pub struct ColorPickerOverlayGeometry {
    /// Static label ("edge" / "portal" / "node") — held as a
    /// `&'static str` so the picker render path doesn't allocate a
    /// fresh `String` per rebuild. Empty string `""` signals
    /// Standalone mode: the builder renders a generic "palette"
    /// title instead of "࿕ {label} color".
    pub target_label: &'static str,
    pub hue_deg: f32,
    pub sat: f32,
    pub val: f32,
    pub preview_hex: String,
    /// Whether the hex readout should render this frame. `true`
    /// when the cursor is inside the backdrop; `false` otherwise.
    /// The readout appears only when the user is actively engaging
    /// with the picker so it doesn't collide with the lower val bar
    /// cells.
    pub hex_visible: bool,
    /// Widest shaped advance across the 32 crosshair-arm glyphs,
    /// measured by the renderer via cosmic-text at picker open. The
    /// layout fn divides by [`measurement_font_size`] to recover a
    /// dimensionless ratio it can scale with whatever font_size the
    /// new sizing formula derives — so the picker can shrink to a
    /// font_size below the measurement baseline without re-measuring.
    pub max_cell_advance: f32,
    /// Same, for the 24 hue ring glyphs (measured at
    /// `measurement_font_size * hue_ring_font_scale`). Combined with
    /// `measurement_font_size` it gives the ring's tangential
    /// slot-spacing ratio that scales with font_size.
    pub max_ring_advance: f32,
    /// Per-glyph ink-center offset from the advance/em-box center,
    /// measured via Baumhard's `measure_glyph_ink_bounds` primitive
    /// at picker open. Each arm's ten glyphs use the same script but
    /// have distinct sidebearings and distinct baseline-relative ink
    /// extents — Devanagari vowels in the top arm differ glyph-to-
    /// glyph, Egyptian hieroglyphs in the bottom arm differ
    /// glyph-to-glyph, etc. cosmic-text's `Align::Center` centers
    /// the em-box (not the ink) along x, and offers no vertical
    /// centering at all — so without a per-glyph correction every
    /// cell drifts a different amount off the crosshair line.
    /// `compute_color_picker_layout` subtracts the scaled offset
    /// from each cell position so the ink lands on the intended
    /// visual radius.
    ///
    /// Dimensionless ratios: multiply by the layout's chosen cell
    /// font size to get pixels. Stored as `(dx, dy)` in the
    /// measurement font's pixel units divided by
    /// `measurement_font_size`. `dx` carries
    /// [`baumhard::font::fonts::InkBounds::x_offset_from_advance_center`];
    /// `dy` carries
    /// [`baumhard::font::fonts::InkBounds::y_offset_from_box_center`]
    /// at the picker's `1.5` line-height multiplier.
    ///
    /// One entry per arm cell (8 per arm, excluding the centre
    /// slot which renders the preview glyph instead).
    pub arm_top_ink_offsets: [(f32, f32); CROSSHAIR_CENTER_CELL],
    pub arm_bottom_ink_offsets: [(f32, f32); CROSSHAIR_CENTER_CELL],
    pub arm_left_ink_offsets: [(f32, f32); CROSSHAIR_CENTER_CELL],
    pub arm_right_ink_offsets: [(f32, f32); CROSSHAIR_CENTER_CELL],
    /// Same, for the central preview glyph (Tibetan ࿕ U+0FD5).
    /// Applied at the `preview_size` scale rather than the cell
    /// scale so a large ࿕ drifts proportionally more pixels than
    /// the arm glyphs would — the ink-center drift is a constant
    /// fraction of the glyph's font size.
    pub preview_ink_offset: (f32, f32),
    /// The font_size both `max_cell_advance` and (after dividing by
    /// `hue_ring_font_scale`) `max_ring_advance` were measured at.
    /// Used to recover dimensionless advance ratios so the layout
    /// can drive font_size from window size, not the other way
    /// around. Open path measures at `font_max` so the ratio is
    /// stable across the picker's whole [`font_min`, `font_max`]
    /// range.
    pub measurement_font_size: f32,
    /// User-controlled scale multiplier applied to the picker's
    /// overall size. 1.0 = the spec's `target_frac` of the screen's
    /// shorter side; values >1 grow the widget, <1 shrink it.
    /// Mutated by the right-mouse-button drag-to-resize gesture in
    /// app.rs; stored alongside `center_override` on
    /// `ColorPickerState::Open` and reset to 1.0 on each new open.
    pub size_scale: f32,
    /// Wheel center override in screen-space pixels, set by the drag
    /// handler. When `None`, the layout centers the wheel at
    /// `(screen_w/2, screen_h/2)` (today's behavior). When `Some`,
    /// every position in the layout — hue slots, bar cells, chips,
    /// title, hint, backdrop, everything — is translated so the
    /// geometric wheel center lands on the override.
    pub center_override: Option<(f32, f32)>,
    /// Which interactive element the cursor is currently over, if
    /// any. Threaded into the builder so the matching glyph renders
    /// with hover-grow scale + brighter color. Diffed by
    /// `handle_color_picker_mouse_move` so only a change triggers a
    /// rebuild.
    pub hovered_hit: Option<PickerHit>,
}
