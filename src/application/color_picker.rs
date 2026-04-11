//! Magical glyph-wheel color picker — a custom modal overlay for choosing
//! colors that fits Mandala's "everything is a positioned glyph" aesthetic.
//!
//! Layout: a 24-glyph hue ring forming a mandala, a crosshair sat/value
//! selector formed by two perpendicular glyph bars meeting at the wheel
//! center, a central preview glyph showing the currently-previewed color,
//! and a row of theme-variable quick-pick chips below. Mouse hover updates
//! the preview live; click commits, click outside cancels. Keyboard
//! fallback: h/H nudges hue, s/S sat, v/V value, Tab cycles chips, Enter
//! commits, Esc cancels.
//!
//! v1 wires the picker to two color-bearing fields whose document setters
//! already exist: `MindEdge.color` (via `set_edge_color`) and
//! `PortalPair.color` (via `set_portal_color`). Node colors and
//! theme-variable editing become a follow-up session.
//!
//! Live preview uses direct in-place model mutation during hover —
//! mirroring `apply_edge_handle_drag` in `app.rs`. The pre-picker snapshot
//! is captured at open time, so cancel restores it without touching the
//! undo stack and commit pushes a single `EditEdge` / `EditPortal` entry.
//!
//! Pure-function layout (`compute_color_picker_layout`) and hit-testing
//! (`hit_test_picker`) are extracted so unit tests don't need a GPU.

use std::f32::consts::{FRAC_PI_2, TAU};

use baumhard::mindmap::model::{MindEdge, PortalPair};
use baumhard::util::color::{hex_to_hsv_safe, resolve_var};

use crate::application::document::{EdgeRef, MindMapDocument, PortalRef};

/// Number of hue slots on the outer ring. 24 slots = 15° per step. Fine
/// enough that adjacent slots feel continuous, coarse enough that each
/// glyph has a comfortable hit target.
pub const HUE_SLOT_COUNT: usize = 24;

/// Number of cells on each crosshair bar. Odd so the center cell sits
/// exactly on the bar's midpoint (sat=0.5 / val=0.5).
pub const SAT_CELL_COUNT: usize = 11;
pub const VAL_CELL_COUNT: usize = 11;

/// Theme-variable quick-pick chips shown below the wheel. Each entry is
/// `(display_name, raw_color_string)`. The empty string sentinel maps to
/// `None` (clear override) when committed.
pub const THEME_CHIPS: &[(&str, &str)] = &[
    ("--accent", "var(--accent)"),
    ("--bg", "var(--bg)"),
    ("--fg", "var(--fg)"),
    ("--edge", "var(--edge)"),
    ("reset", ""),
];

// =============================================================
// Target abstraction
// =============================================================

/// What the picker is currently editing. Edge and Portal are the two v1
/// targets — the picker reads/writes through their existing document
/// setters. Adding a node-style target would mean adding a new variant
/// here, an `EditNode` undo variant, and node setters in `document.rs`.
#[derive(Clone, Debug, PartialEq)]
pub enum ColorTarget {
    Edge(EdgeRef),
    Portal(PortalRef),
}

impl ColorTarget {
    /// Read the current color string from the target. For edges this
    /// prefers the per-edge `glyph_connection.color` override, falling
    /// back to `edge.color` (the canvas-level default). For portals it's
    /// just `portal.color`. Returns `None` if the target ref no longer
    /// resolves (edge/portal was deleted between open and now).
    pub fn current_color(&self, doc: &MindMapDocument) -> Option<String> {
        match self {
            ColorTarget::Edge(er) => {
                let edge = doc.mindmap.edges.iter().find(|e| er.matches(e))?;
                Some(
                    edge.glyph_connection
                        .as_ref()
                        .and_then(|gc| gc.color.clone())
                        .unwrap_or_else(|| edge.color.clone()),
                )
            }
            ColorTarget::Portal(pr) => doc
                .mindmap
                .portals
                .iter()
                .find(|p| pr.matches(p))
                .map(|p| p.color.clone()),
        }
    }

    /// Resolve the current color through the canvas theme variables and
    /// parse it into HSV for seeding the picker state. Falls back to
    /// `(0.0, 0.0, 0.5)` (mid-gray) on any failure so the picker always
    /// opens with a sensible default.
    pub fn current_hsv(&self, doc: &MindMapDocument) -> (f32, f32, f32) {
        let raw = match self.current_color(doc) {
            Some(s) => s,
            None => return (0.0, 0.0, 0.5),
        };
        let resolved = resolve_var(&raw, &doc.mindmap.canvas.theme_variables);
        hex_to_hsv_safe(resolved).unwrap_or((0.0, 0.0, 0.5))
    }

    /// Short label for the picker title bar — "edge" or "portal".
    pub fn kind_label(&self) -> &'static str {
        match self {
            ColorTarget::Edge(_) => "edge",
            ColorTarget::Portal(_) => "portal",
        }
    }
}

// =============================================================
// Pre-picker snapshot (for cancel + commit undo entries)
// =============================================================

/// Captured at picker open time so cancel can restore in-place without
/// touching the undo stack, and commit can push a single
/// `UndoAction::EditEdge` / `EditPortal` with this `before` state.
/// Mirrors the snapshot pattern from `apply_edge_handle_drag` in
/// `app.rs:505-537`.
#[derive(Debug, Clone)]
pub enum ColorPickerSnapshot {
    Edge { index: usize, before: MindEdge },
    Portal { index: usize, before: PortalPair },
}

// =============================================================
// State machine
// =============================================================

#[derive(Debug, Clone)]
pub enum ColorPickerState {
    Closed,
    Open {
        target: ColorTarget,
        snapshot: ColorPickerSnapshot,
        /// Current preview hue in degrees, `[0, 360)`.
        hue_deg: f32,
        /// Current preview saturation, `[0, 1]`.
        sat: f32,
        /// Current preview value/lightness, `[0, 1]`.
        val: f32,
        /// Index of the focused theme chip, or `None` while picking
        /// HSV. Tab cycles through chips; Enter on a focused chip
        /// commits the chip's raw color string instead of the HSV hex.
        chip_focus: Option<usize>,
        /// Cached layout from the last rebuild. Mouse hit testing reads
        /// from this so we don't re-run the layout pure fn on every
        /// CursorMoved event.
        layout: ColorPickerLayout,
    },
}

impl ColorPickerState {
    pub fn is_open(&self) -> bool {
        matches!(self, ColorPickerState::Open { .. })
    }
}

// =============================================================
// Geometry handed to the renderer + cached layout
// =============================================================

/// Pre-render geometry pushed from the app to the renderer. Plain data,
/// no rendering primitives — mirrors `PaletteOverlayGeometry`.
pub struct ColorPickerOverlayGeometry {
    pub target_label: String,
    pub hue_deg: f32,
    pub sat: f32,
    pub val: f32,
    pub preview_hex: String,
    pub chip_focus: Option<usize>,
}

/// Pure-function output of the color-picker layout pass. All positions
/// are in screen-space pixels.
#[derive(Debug, Clone)]
pub struct ColorPickerLayout {
    pub center: (f32, f32),
    pub outer_radius: f32,
    pub font_size: f32,
    pub char_width: f32,
    /// 24 hue ring positions, ordered clockwise from 12-o'clock.
    pub hue_slot_positions: [(f32, f32); HUE_SLOT_COUNT],
    /// 11 sat-bar cell centers, left → right.
    pub sat_cell_positions: [(f32, f32); SAT_CELL_COUNT],
    /// 11 val-bar cell centers, top → bottom (top = brightest).
    pub val_cell_positions: [(f32, f32); VAL_CELL_COUNT],
    /// Center preview glyph anchor (the `✦`).
    pub preview_pos: (f32, f32),
    /// One `(x, y, width)` per chip, ordered as `THEME_CHIPS`.
    pub chip_positions: Vec<(f32, f32, f32)>,
    pub chip_height: f32,
    /// `(left, top, width, height)` of the opaque backdrop rect that
    /// the renderer draws under the overlay text pass.
    pub backdrop: (f32, f32, f32, f32),
    /// Title text anchor (top of frame).
    pub title_pos: (f32, f32),
    /// Hint footer text anchor.
    pub hint_pos: (f32, f32),
}

impl ColorPickerLayout {
    /// A tiny placeholder layout used to construct an `Open` state
    /// before the first rebuild has run. The real layout overwrites
    /// this on the first rebuild_color_picker_overlay call.
    pub fn placeholder() -> Self {
        Self {
            center: (0.0, 0.0),
            outer_radius: 0.0,
            font_size: 16.0,
            char_width: 9.6,
            hue_slot_positions: [(0.0, 0.0); HUE_SLOT_COUNT],
            sat_cell_positions: [(0.0, 0.0); SAT_CELL_COUNT],
            val_cell_positions: [(0.0, 0.0); VAL_CELL_COUNT],
            preview_pos: (0.0, 0.0),
            chip_positions: Vec::new(),
            chip_height: 0.0,
            backdrop: (0.0, 0.0, 0.0, 0.0),
            title_pos: (0.0, 0.0),
            hint_pos: (0.0, 0.0),
        }
    }
}

/// Pure-function layout. No GPU access, no font system — mirrors
/// `compute_palette_frame_layout` so unit tests can construct one from
/// nothing but a geometry struct + screen dimensions.
pub fn compute_color_picker_layout(
    _geometry: &ColorPickerOverlayGeometry,
    screen_w: f32,
    screen_h: f32,
) -> ColorPickerLayout {
    let font_size: f32 = 16.0;
    let char_width = font_size * 0.6;

    // Square frame, centered. The clamp keeps it usable on tiny windows
    // and unobtrusive on huge ones.
    let side = 420f32
        .min(screen_w * 0.6)
        .min(screen_h * 0.8)
        .max(280.0);
    let center = (screen_w * 0.5, screen_h * 0.5);
    let outer_radius = side * 0.45;

    // ---- Hue ring (24 slots, clockwise from 12 o'clock) ----
    let mut hue_slot_positions = [(0.0_f32, 0.0_f32); HUE_SLOT_COUNT];
    let ring_r = outer_radius - font_size * 0.5;
    for (i, slot) in hue_slot_positions.iter_mut().enumerate() {
        let angle = (i as f32 / HUE_SLOT_COUNT as f32) * TAU - FRAC_PI_2;
        *slot = (
            center.0 + angle.cos() * ring_r,
            center.1 + angle.sin() * ring_r,
        );
    }

    // ---- Crosshair sat/val bars, inscribed inside the ring ----
    let inner_extent = outer_radius * 0.55;
    let sat_step = (inner_extent * 2.0) / (SAT_CELL_COUNT as f32 - 1.0);
    let val_step = (inner_extent * 2.0) / (VAL_CELL_COUNT as f32 - 1.0);
    let mut sat_cell_positions = [(0.0_f32, 0.0_f32); SAT_CELL_COUNT];
    let mut val_cell_positions = [(0.0_f32, 0.0_f32); VAL_CELL_COUNT];
    for i in 0..SAT_CELL_COUNT {
        sat_cell_positions[i] = (center.0 - inner_extent + i as f32 * sat_step, center.1);
    }
    for i in 0..VAL_CELL_COUNT {
        val_cell_positions[i] = (center.0, center.1 - inner_extent + i as f32 * val_step);
    }

    // Center preview at the bar intersection.
    let preview_pos = (center.0 - char_width, center.1 - font_size * 0.5);

    // ---- Theme chips row ----
    let chip_row_y = center.1 + outer_radius + font_size * 1.5;
    let chip_height = font_size * 1.4;
    let mut chip_positions: Vec<(f32, f32, f32)> = Vec::with_capacity(THEME_CHIPS.len());
    let total_chip_width: f32 = THEME_CHIPS
        .iter()
        .map(|(name, _)| (name.chars().count() + 4) as f32 * char_width)
        .sum::<f32>()
        + (THEME_CHIPS.len().saturating_sub(1)) as f32 * 6.0;
    let mut x = center.0 - total_chip_width * 0.5;
    for (name, _) in THEME_CHIPS {
        let w = (name.chars().count() + 4) as f32 * char_width;
        chip_positions.push((x, chip_row_y, w));
        x += w + 6.0;
    }

    // ---- Backdrop, title, hint ----
    let backdrop_top = center.1 - side * 0.5 - font_size;
    let backdrop_height = side + font_size * 5.0;
    let backdrop = (
        center.0 - side * 0.5,
        backdrop_top,
        side,
        backdrop_height,
    );
    let title_pos = (center.0 - side * 0.4, backdrop_top + font_size * 0.5);
    let hint_pos = (
        center.0 - side * 0.4,
        backdrop_top + backdrop_height - font_size * 1.5,
    );

    ColorPickerLayout {
        center,
        outer_radius,
        font_size,
        char_width,
        hue_slot_positions,
        sat_cell_positions,
        val_cell_positions,
        preview_pos,
        chip_positions,
        chip_height,
        backdrop,
        title_pos,
        hint_pos,
    }
}

// =============================================================
// Hit testing — used by mouse-move and click handlers in app.rs
// =============================================================

/// Result of a hit test against a `ColorPickerLayout`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerHit {
    /// Index into `hue_slot_positions`.
    Hue(usize),
    /// Index into `sat_cell_positions`.
    SatCell(usize),
    /// Index into `val_cell_positions`.
    ValCell(usize),
    /// Index into `chip_positions`.
    Chip(usize),
    /// Inside the backdrop but not on any interactive element.
    Inside,
    /// Outside the backdrop entirely.
    Outside,
}

/// Hit-test a screen position against the cached picker layout. The
/// search order matches the visual layering: chips → val bar → sat bar
/// → hue ring (innermost interactive element wins ties). Returns
/// `Outside` if the cursor is past the backdrop bounds, `Inside` if it's
/// in the backdrop padding but not on any element.
pub fn hit_test_picker(layout: &ColorPickerLayout, x: f32, y: f32) -> PickerHit {
    let (bl, bt, bw, bh) = layout.backdrop;
    if x < bl || x > bl + bw || y < bt || y > bt + bh {
        return PickerHit::Outside;
    }

    // Chips first — they sit below the wheel.
    for (i, (cx, cy, cw)) in layout.chip_positions.iter().enumerate() {
        if x >= *cx && x <= *cx + *cw && y >= *cy && y <= *cy + layout.chip_height {
            return PickerHit::Chip(i);
        }
    }

    // Sat/val bars: pick the closer of the two if the cursor is inside
    // the inner cross region. Each cell is a square of `font_size`.
    let cell_half = layout.font_size * 0.6;

    // Sat (horizontal) bar — only consider when cursor is vertically
    // close to the bar line.
    if (y - layout.center.1).abs() <= cell_half {
        for (i, (cx, _)) in layout.sat_cell_positions.iter().enumerate() {
            if (x - cx).abs() <= cell_half {
                return PickerHit::SatCell(i);
            }
        }
    }
    // Val (vertical) bar — same in the other axis.
    if (x - layout.center.0).abs() <= cell_half {
        for (i, (_, cy)) in layout.val_cell_positions.iter().enumerate() {
            if (y - cy).abs() <= cell_half {
                return PickerHit::ValCell(i);
            }
        }
    }

    // Hue ring — annular hit. Only the slot whose glyph contains the
    // cursor counts (cell_half tolerance), so the empty space inside
    // the ring stays inert.
    for (i, (px, py)) in layout.hue_slot_positions.iter().enumerate() {
        let dx = x - px;
        let dy = y - py;
        if dx * dx + dy * dy <= cell_half * cell_half {
            return PickerHit::Hue(i);
        }
    }

    PickerHit::Inside
}

/// Convert a hue-ring slot index to its degrees value (0..360).
pub fn hue_slot_to_degrees(slot: usize) -> f32 {
    (slot as f32 / HUE_SLOT_COUNT as f32) * 360.0
}

/// Quantize a degrees value to the nearest hue slot.
pub fn degrees_to_hue_slot(deg: f32) -> usize {
    let normalized = deg.rem_euclid(360.0) / 360.0;
    let slot = (normalized * HUE_SLOT_COUNT as f32).round() as usize;
    slot % HUE_SLOT_COUNT
}

/// Convert a saturation-bar cell index to its `[0, 1]` value.
pub fn sat_cell_to_value(cell: usize) -> f32 {
    cell as f32 / (SAT_CELL_COUNT as f32 - 1.0)
}

/// Convert a value-bar cell index to its `[0, 1]` value. Top of the bar
/// (cell 0) is brightest (val=1.0); bottom is darkest (val=0.0).
pub fn val_cell_to_value(cell: usize) -> f32 {
    1.0 - cell as f32 / (VAL_CELL_COUNT as f32 - 1.0)
}

// =============================================================
// Tests
// =============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_geometry() -> ColorPickerOverlayGeometry {
        ColorPickerOverlayGeometry {
            target_label: "edge".to_string(),
            hue_deg: 0.0,
            sat: 1.0,
            val: 1.0,
            preview_hex: "#ff0000".to_string(),
            chip_focus: None,
        }
    }

    #[test]
    fn layout_emits_24_hue_slots_on_circle() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        assert_eq!(layout.hue_slot_positions.len(), 24);
        let r_target = layout.outer_radius - layout.font_size * 0.5;
        for (i, (px, py)) in layout.hue_slot_positions.iter().enumerate() {
            let dx = px - layout.center.0;
            let dy = py - layout.center.1;
            let r = (dx * dx + dy * dy).sqrt();
            assert!(
                (r - r_target).abs() < 0.5,
                "slot {i} radius {r} differs from {r_target}",
            );
        }
    }

    #[test]
    fn layout_first_hue_slot_is_at_top() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        let (px, py) = layout.hue_slot_positions[0];
        // Slot 0 sits at 12 o'clock — same x as center, smaller y.
        assert!((px - layout.center.0).abs() < 0.5);
        assert!(py < layout.center.1);
    }

    #[test]
    fn layout_sat_bar_monotonic_x_constant_y() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        assert_eq!(layout.sat_cell_positions.len(), 11);
        for w in layout.sat_cell_positions.windows(2) {
            assert!(w[1].0 > w[0].0, "sat cells must increase in x");
            assert!((w[0].1 - w[1].1).abs() < 0.1, "sat cells share y");
        }
    }

    #[test]
    fn layout_val_bar_monotonic_y_constant_x() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        assert_eq!(layout.val_cell_positions.len(), 11);
        for w in layout.val_cell_positions.windows(2) {
            assert!(w[1].1 > w[0].1, "val cells must increase in y");
            assert!((w[0].0 - w[1].0).abs() < 0.1, "val cells share x");
        }
    }

    #[test]
    fn layout_includes_one_chip_per_theme_entry() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        assert_eq!(layout.chip_positions.len(), THEME_CHIPS.len());
    }

    #[test]
    fn hit_test_outside_backdrop_returns_outside() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        assert_eq!(hit_test_picker(&layout, -10.0, -10.0), PickerHit::Outside);
        assert_eq!(hit_test_picker(&layout, 5000.0, 5000.0), PickerHit::Outside);
    }

    #[test]
    fn hit_test_hits_first_hue_slot() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        let (px, py) = layout.hue_slot_positions[0];
        assert_eq!(hit_test_picker(&layout, px, py), PickerHit::Hue(0));
    }

    #[test]
    fn hit_test_hits_off_center_sat_cell() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        // Pick a sat cell offset from the center so the val-bar branch
        // is not a candidate (the val bar shares the center column).
        let off = SAT_CELL_COUNT / 2 + 2;
        let (cx, cy) = layout.sat_cell_positions[off];
        assert_eq!(hit_test_picker(&layout, cx, cy), PickerHit::SatCell(off));
    }

    #[test]
    fn hit_test_hits_chip_row() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        let (cx, cy, cw) = layout.chip_positions[0];
        // Hit the middle of the chip rect.
        assert_eq!(
            hit_test_picker(&layout, cx + cw * 0.5, cy + layout.chip_height * 0.5),
            PickerHit::Chip(0)
        );
    }

    #[test]
    fn hue_slot_to_degrees_round_trip() {
        for slot in 0..HUE_SLOT_COUNT {
            let deg = hue_slot_to_degrees(slot);
            assert_eq!(degrees_to_hue_slot(deg), slot);
        }
    }

    #[test]
    fn sat_and_val_cell_value_endpoints() {
        assert!((sat_cell_to_value(0) - 0.0).abs() < 1e-6);
        assert!((sat_cell_to_value(SAT_CELL_COUNT - 1) - 1.0).abs() < 1e-6);
        // Val bar inverted: top cell = brightest.
        assert!((val_cell_to_value(0) - 1.0).abs() < 1e-6);
        assert!((val_cell_to_value(VAL_CELL_COUNT - 1) - 0.0).abs() < 1e-6);
    }
}
