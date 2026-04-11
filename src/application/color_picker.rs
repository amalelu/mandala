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
//!
//! WASM status: this module compiles on wasm32 (it's pure Rust + data
//! types, no native-only deps), but the `open_*` / `handle_*` entry
//! points in `app.rs` are gated behind `#[cfg(not(target_arch =
//! "wasm32"))]` like the palette and label-edit modals. Picker keyboard
//! / mouse dispatch for WASM is deferred as part of the broader WASM
//! input gap tracked in the roadmap.

use std::f32::consts::{FRAC_PI_2, TAU};

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

/// What a theme-variable quick-pick chip commits when clicked or
/// Enter-activated with focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChipAction {
    /// Store the given `var(--name)` reference on the target so the
    /// canvas theme map resolves the color at render time.
    Var(&'static str),
    /// Clear the target's color override, falling back to whatever
    /// the canvas-level default is (the `reset` chip).
    Reset,
}

/// One theme-variable chip shown below the wheel.
#[derive(Debug, Clone, Copy)]
pub struct ThemeChip {
    pub label: &'static str,
    pub action: ChipAction,
}

/// Theme-variable quick-pick chips shown below the wheel. Replaces the
/// prior `(&str, &str)` tuple where an empty string was a stringly-typed
/// "reset" sentinel — `ChipAction` makes the intent explicit at the
/// type level.
pub const THEME_CHIPS: &[ThemeChip] = &[
    ThemeChip { label: "--accent", action: ChipAction::Var("var(--accent)") },
    ThemeChip { label: "--bg", action: ChipAction::Var("var(--bg)") },
    ThemeChip { label: "--fg", action: ChipAction::Var("var(--fg)") },
    ThemeChip { label: "--edge", action: ChipAction::Var("var(--edge)") },
    ThemeChip { label: "reset", action: ChipAction::Reset },
];

// =============================================================
// Target abstraction
// =============================================================

/// What the picker is currently editing. `Edge` / `Portal` are the two
/// v1 targets — the picker reads/writes through their existing document
/// setters. Adding a node-style target would mean adding a new variant
/// here, an `EditNode` undo variant, and node setters in `document.rs`.
///
/// Used only at the palette-to-picker handoff. Once the picker is
/// actually open, the hot hover path uses `TargetKind + target_index`
/// instead (captured once at open time) to avoid re-resolving the ref
/// on every mouse move — see `ColorPickerState::Open`.
#[derive(Clone, Debug, PartialEq)]
pub enum ColorTarget {
    Edge(EdgeRef),
    Portal(PortalRef),
}

/// Kind of target currently open. Cheaper to match on during hover
/// than carrying the full `ColorTarget` with its owned strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Edge,
    Portal,
}

impl TargetKind {
    /// Short label for the picker title bar — "edge" or "portal".
    pub fn label(&self) -> &'static str {
        match self {
            TargetKind::Edge => "edge",
            TargetKind::Portal => "portal",
        }
    }
}

impl ColorTarget {
    /// Resolve the target ref to a concrete index into
    /// `doc.mindmap.edges` or `doc.mindmap.portals`. Returns `None` if
    /// the underlying edge/portal was deleted between the palette
    /// closing and the picker opening (should never happen in practice
    /// because the palette holds the event loop, but the defensive
    /// check protects against a later refactor that relaxes that).
    pub fn resolve(&self, doc: &MindMapDocument) -> Option<(TargetKind, usize)> {
        match self {
            ColorTarget::Edge(er) => doc
                .mindmap
                .edges
                .iter()
                .position(|e| er.matches(e))
                .map(|i| (TargetKind::Edge, i)),
            ColorTarget::Portal(pr) => doc
                .mindmap
                .portals
                .iter()
                .position(|p| pr.matches(p))
                .map(|i| (TargetKind::Portal, i)),
        }
    }
}

/// Read the current color string for a target addressed by kind +
/// index. Used to seed picker HSV at open time and to read the
/// effective color for the preview after a chip action. Returns
/// `None` if the index is out of bounds.
pub fn current_color_at(
    doc: &MindMapDocument,
    kind: TargetKind,
    index: usize,
) -> Option<String> {
    match kind {
        TargetKind::Edge => {
            let e = doc.mindmap.edges.get(index)?;
            Some(
                e.glyph_connection
                    .as_ref()
                    .and_then(|gc| gc.color.clone())
                    .unwrap_or_else(|| e.color.clone()),
            )
        }
        TargetKind::Portal => doc.mindmap.portals.get(index).map(|p| p.color.clone()),
    }
}

/// Resolve the current color through the canvas theme variables and
/// parse it into HSV for seeding the picker state. Falls back to
/// `(0.0, 0.0, 0.5)` (mid-gray) on any failure so the picker always
/// opens with a sensible default.
pub fn current_hsv_at(
    doc: &MindMapDocument,
    kind: TargetKind,
    index: usize,
) -> (f32, f32, f32) {
    let raw = match current_color_at(doc, kind, index) {
        Some(s) => s,
        None => return (0.0, 0.0, 0.5),
    };
    let resolved = resolve_var(&raw, &doc.mindmap.canvas.theme_variables);
    hex_to_hsv_safe(resolved).unwrap_or((0.0, 0.0, 0.5))
}

// =============================================================
// State machine
// =============================================================

/// Modal state for the glyph-wheel color picker.
///
/// The previous revision of this struct also stored a
/// `snapshot: UndoAction` so a pre-picker clone of the edited
/// edge/portal could be restored on cancel. That snapshot is no
/// longer needed: preview is now a purely visual substitution via
/// `MindMapDocument::color_picker_preview`, so cancel just clears
/// the preview and commit calls `set_edge_color` /
/// `set_portal_color` once on the final HSV — the committed model
/// is untouched during hover and the fork-on-first-edit semantics
/// of `ensure_glyph_connection` only fire on commit.
///
/// Hot path design: `kind` and `target_index` are captured at open
/// time so the hover handler can push `(target_index, hex)` into
/// the document preview without re-resolving any `EdgeRef` /
/// `PortalRef`.
#[derive(Debug, Clone)]
pub enum ColorPickerState {
    Closed,
    Open {
        /// Which kind of target the picker is editing.
        kind: TargetKind,
        /// Direct index into `doc.mindmap.edges` or `doc.mindmap.portals`,
        /// captured at open time. Stable for the picker's lifetime because
        /// the modal suppresses all other document edits.
        target_index: usize,
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
        /// Tracks what the commit should do. `Hsv` → commit the
        /// current HSV hex as a per-edge/portal override. `Var(raw)`
        /// → commit the `var(--...)` string so theme-var resolution
        /// runs at render time. `ResetToInherited` → clear the edge
        /// override (cfg.color = None) for edges, or re-seed to the
        /// --accent fallback for portals. Set by `apply_picker_chip`;
        /// HSV nudges and mouse hover on the wheel reset it back to
        /// `Hsv`.
        commit_mode: CommitMode,
        /// Cached layout from the last rebuild. `None` between `Open`
        /// construction and the first `rebuild_color_picker_overlay`
        /// call (a narrow window — the rebuild is the very next line
        /// in `open_color_picker`, but `Option` makes the invariant
        /// explicit rather than relying on a placeholder).
        layout: Option<ColorPickerLayout>,
    },
}

/// What the picker will commit to the model on Enter / click-commit.
#[derive(Debug, Clone)]
pub enum CommitMode {
    /// Commit the current HSV value as a concrete hex.
    Hsv,
    /// Commit a raw `var(--name)` reference. Set by a theme chip.
    Var(String),
    /// Commit a "clear override" — `set_edge_color(None)` for edges;
    /// re-seed to `--accent` for portals (which have a non-optional
    /// color field).
    ResetToInherited,
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
    /// Static label ("edge" / "portal") — held as a `&'static str` so
    /// the picker render path doesn't allocate a fresh `String` per
    /// rebuild.
    pub target_label: &'static str,
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
    /// Center preview glyph anchor (the `✦`). Top-left corner of the
    /// glyph box, computed so the glyph visually centers on the wheel
    /// center given `preview_size`.
    pub preview_pos: (f32, f32),
    /// Font size for the central `✦` preview glyph. 2× the base
    /// `font_size` so the preview reads as a focal point.
    pub preview_size: f32,
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

    // Square frame, centered on the window. The picker draws the
    // wheel inside `side`, then adds a chip row + hint footer below
    // and a title above — see the backdrop computation at the
    // bottom of this function. The vertical budget consumed by the
    // backdrop is `side + 5 * font_size`, and the backdrop is
    // anchored above the wheel by 1 font_size, so the total vertical
    // extent of the backdrop is bounded by:
    //
    //     center.y - side/2 - font_size  ..  center.y + side/2 + 4*font_size
    //
    // For the backdrop to fit inside `[0, screen_h]` we need
    // `side/2 + 4*font_size <= center.y` AND `side/2 + font_size <=
    // center.y` — at center.y = screen_h/2 the first dominates,
    // giving `side <= screen_h - 8*font_size`. Same logic
    // horizontally. Note: the `.min` cascade after the floor would
    // overflow on small windows, so we put the floor on each clamp.
    let max_side_for_w = (screen_w - font_size * 2.0).max(0.0);
    let max_side_for_h = (screen_h - font_size * 8.0).max(0.0);
    let side = 420f32
        .min(max_side_for_w)
        .min(max_side_for_h)
        .max(0.0);
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

    // Center preview at the bar intersection. The ✦ glyph renders
    // at 2× font_size, so the top-left of its box must be offset by
    // half the preview size in each direction so the visible glyph
    // sits on the geometric wheel center. cosmic-text's effective
    // glyph width is ~0.6 of its font size; we use 0.4 horizontally
    // because the ✦ glyph has whitespace around it that the box
    // includes but the visible mark does not.
    let preview_size = font_size * 2.0;
    let preview_pos = (
        center.0 - preview_size * 0.4,
        center.1 - preview_size * 0.5,
    );

    // ---- Theme chips row ----
    let chip_row_y = center.1 + outer_radius + font_size * 1.5;
    let chip_height = font_size * 1.4;
    let mut chip_positions: Vec<(f32, f32, f32)> = Vec::with_capacity(THEME_CHIPS.len());
    let total_chip_width: f32 = THEME_CHIPS
        .iter()
        .map(|c| (c.label.chars().count() + 4) as f32 * char_width)
        .sum::<f32>()
        + (THEME_CHIPS.len().saturating_sub(1)) as f32 * 6.0;
    let mut x = center.0 - total_chip_width * 0.5;
    for chip in THEME_CHIPS {
        let w = (chip.label.chars().count() + 4) as f32 * char_width;
        chip_positions.push((x, chip_row_y, w));
        x += w + 6.0;
    }

    // ---- Backdrop, title, hint ----
    // Title and hint are anchored RELATIVE to the backdrop's left
    // edge, not the window center. On small windows the frame
    // shrinks; a window-centered anchor would push the hint text
    // off the right edge when the frame is narrow. Anchoring to the
    // backdrop keeps both strings inside the frame at any size —
    // cosmic-text still clips anything past the bounds, which is
    // correct behavior for a text-too-long situation.
    let backdrop_left = center.0 - side * 0.5;
    let backdrop_top = center.1 - side * 0.5 - font_size;
    let backdrop_height = side + font_size * 5.0;
    let backdrop = (backdrop_left, backdrop_top, side, backdrop_height);
    let title_pos = (backdrop_left + font_size * 0.5, backdrop_top + font_size * 0.5);
    let hint_pos = (
        backdrop_left + font_size * 0.5,
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
        preview_size,
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
            target_label: "edge",
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

    /// Layout must fit inside the window even on small windows. The
    /// backdrop's vertical extent (top + height) must not exceed the
    /// screen height; same for horizontal. Regression guard for the
    /// "side defaulted to 280 even on a 200×200 window" overflow.
    #[test]
    fn layout_backdrop_fits_inside_small_window() {
        let g = sample_geometry();
        for &(w, h) in &[(320.0_f32, 240.0_f32), (400.0, 300.0), (200.0, 200.0)] {
            let layout = compute_color_picker_layout(&g, w, h);
            let (left, top, bw, bh) = layout.backdrop;
            assert!(left >= 0.0, "backdrop left underflows on {}x{}", w, h);
            assert!(top >= 0.0, "backdrop top underflows on {}x{}", w, h);
            assert!(left + bw <= w + 0.5,
                "backdrop right overflows on {}x{}: left={} bw={} w={}",
                w, h, left, bw, w);
            assert!(top + bh <= h + 0.5,
                "backdrop bottom overflows on {}x{}: top={} bh={} h={}",
                w, h, top, bh, h);
        }
    }

    /// Preview glyph must center on the geometric wheel center given
    /// the layout-emitted preview_size. Regression guard for the
    /// "preview was anchored low-right of center" bug.
    #[test]
    fn layout_preview_centered_on_wheel_center() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        let (px, py) = layout.preview_pos;
        let cx = px + layout.preview_size * 0.4;
        let cy = py + layout.preview_size * 0.5;
        // The preview's visible center should be within ~1 px of the
        // wheel center on each axis.
        assert!((cx - layout.center.0).abs() < 1.0,
            "preview x center {} differs from wheel center {}", cx, layout.center.0);
        assert!((cy - layout.center.1).abs() < 1.0,
            "preview y center {} differs from wheel center {}", cy, layout.center.1);
    }

    /// Hue wrap: degrees_to_hue_slot must wrap correctly across the
    /// 0/360 boundary in both directions, and slots near the
    /// boundary must round to slot 0 (not slot 24).
    #[test]
    fn degrees_to_hue_slot_wraps_at_boundary() {
        assert_eq!(degrees_to_hue_slot(0.0), 0);
        assert_eq!(degrees_to_hue_slot(360.0), 0);
        assert_eq!(degrees_to_hue_slot(720.0), 0);
        assert_eq!(degrees_to_hue_slot(-15.0), 23);
        assert_eq!(degrees_to_hue_slot(-360.0), 0);
        // 357° rounds to slot 0 (since 24 % 24 = 0).
        assert_eq!(degrees_to_hue_slot(357.0), 0);
        // 352° rounds to slot 23.
        assert_eq!(degrees_to_hue_slot(352.0), 23);
    }

    /// Quantization: every input degree in `[0, 360)` must fall into
    /// the slot whose center is closest. Walk the range in 1° steps
    /// and check that no input is more than 7.5° (half a slot) from
    /// its resolved slot's canonical degree. Guards against a future
    /// refactor that silently shifts the quantization phase or
    /// introduces floor-vs-round inconsistencies.
    #[test]
    fn degrees_to_hue_slot_quantizes_to_nearest() {
        for d in 0..360 {
            let deg = d as f32;
            let slot = degrees_to_hue_slot(deg);
            let canonical = hue_slot_to_degrees(slot);
            // Circular distance from `deg` to `canonical`, taking the
            // shorter arc of the two directions.
            let diff = ((deg - canonical).rem_euclid(360.0)).min(
                (canonical - deg).rem_euclid(360.0),
            );
            assert!(diff <= 7.5 + 1e-4,
                "deg {} → slot {} (canonical {}°) distance {} > 7.5",
                deg, slot, canonical, diff);
        }
    }

    /// Boundary rounding: 7.4° rounds to slot 0 (closer), 7.6°
    /// rounds to slot 1 (closer). Explicit test guarding the
    /// round-half-to-even-or-away edge case.
    #[test]
    fn degrees_to_hue_slot_mid_slot_rounding() {
        assert_eq!(degrees_to_hue_slot(7.4), 0);
        assert_eq!(degrees_to_hue_slot(7.6), 1);
        assert_eq!(degrees_to_hue_slot(22.4), 1);
        assert_eq!(degrees_to_hue_slot(22.6), 2);
    }
}
