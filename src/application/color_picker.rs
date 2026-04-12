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
/// exactly on the bar's midpoint (sat=0.5 / val=0.5). Cell 10 is the
/// wheel center where ॐ lives — it's counted in the HSV quantization
/// but not rendered as a bar cell.
pub const SAT_CELL_COUNT: usize = 21;
pub const VAL_CELL_COUNT: usize = 21;

/// The center cell index of each 21-cell crosshair bar — the wheel
/// center where ॐ sits. Skipped during bar rendering so the ॐ glyph
/// shows through cleanly; still counted in sat/val quantization.
pub const CROSSHAIR_CENTER_CELL: usize = 10;

/// Hue ring font size multiplier over the picker's base font_size. The
/// ring is the dominant visual element of the mandala-shaped picker, so
/// it renders larger than the bars, chips, and title. 1.5× strikes a
/// balance: visibly ornate without overflowing the backdrop on a
/// normally-sized window.
pub const HUE_RING_FONT_SCALE: f32 = 1.5;

/// Hue ring sacred-script glyphs, clockwise from 12 o'clock. Three
/// 8-glyph arcs: Devanagari (top-right), Hebrew (bottom-right), Tibetan
/// (bottom-left → top-left). Each glyph indexes directly into
/// `hue_slot_positions[i]`.
pub const HUE_RING_GLYPHS: [&str; HUE_SLOT_COUNT] = [
    // Slots 0-7 — Devanagari consonants
    "\u{0915}", // क KA
    "\u{0916}", // ख KHA
    "\u{0917}", // ग GA
    "\u{0918}", // घ GHA
    "\u{091A}", // च CA
    "\u{091C}", // ज JA
    "\u{091F}", // ट TTA
    "\u{0921}", // ड DDA
    // Slots 8-15 — Hebrew alefbet (first 8 letters)
    "\u{05D0}", // א ALEF
    "\u{05D1}", // ב BET
    "\u{05D2}", // ג GIMEL
    "\u{05D3}", // ד DALET
    "\u{05D4}", // ה HE
    "\u{05D5}", // ו VAV
    "\u{05D6}", // ז ZAYIN
    "\u{05D7}", // ח HET
    // Slots 16-23 — Tibetan consonants
    "\u{0F40}", // ཀ KA
    "\u{0F41}", // ཁ KHA
    "\u{0F42}", // ག GA
    "\u{0F44}", // ང NGA
    "\u{0F45}", // ཅ CA
    "\u{0F4F}", // ཏ TA
    "\u{0F54}", // པ PA
    "\u{0F58}", // མ MA
];

/// Val bar top arm (cells 0..CROSSHAIR_CENTER_CELL, brightest → mid).
/// Devanagari independent vowels — the script sits on the top arm so
/// the cross reads as a typographic compass with one script per arm.
pub const ARM_TOP_GLYPHS: [&str; CROSSHAIR_CENTER_CELL] = [
    "\u{0905}", // अ A
    "\u{0906}", // आ AA
    "\u{0907}", // इ I
    "\u{0908}", // ई II
    "\u{0909}", // उ U
    "\u{090A}", // ऊ UU
    "\u{090B}", // ऋ R-VOCALIC
    "\u{090F}", // ए E
    "\u{0910}", // ऐ AI
    "\u{0913}", // ओ O
];

/// Val bar bottom arm (cells CROSSHAIR_CENTER_CELL+1..SAT_CELL_COUNT,
/// mid → darkest). Hebrew letters beyond the ring's first 8 — script
/// contrast with the Devanagari top arm across the wheel's vertical
/// axis.
pub const ARM_BOTTOM_GLYPHS: [&str; CROSSHAIR_CENTER_CELL] = [
    "\u{05D8}", // ט TET
    "\u{05D9}", // י YOD
    "\u{05DB}", // כ KAF
    "\u{05DC}", // ל LAMED
    "\u{05DE}", // מ MEM
    "\u{05E0}", // נ NUN
    "\u{05E1}", // ס SAMEKH
    "\u{05E2}", // ע AYIN
    "\u{05E4}", // פ PE
    "\u{05E6}", // צ TSADE
];

/// Sat bar left arm (cells 0..CROSSHAIR_CENTER_CELL, desaturated →
/// mid). Tibetan consonants not used in the hue ring — gives the
/// left-of-center arm its own distinct script.
pub const ARM_LEFT_GLYPHS: [&str; CROSSHAIR_CENTER_CELL] = [
    "\u{0F49}", // ཉ NYA
    "\u{0F50}", // ཐ THA
    "\u{0F51}", // ད DA
    "\u{0F53}", // ན NA
    "\u{0F55}", // ཕ PHA
    "\u{0F56}", // བ BA
    "\u{0F59}", // ཙ TSA
    "\u{0F5E}", // ཞ ZHA
    "\u{0F62}", // ར RA
    "\u{0F66}", // ས SA
];

/// Sat bar right arm (cells CROSSHAIR_CENTER_CELL+1..SAT_CELL_COUNT,
/// mid → saturated). Egyptian Hieroglyph narrow uniliterals from
/// Gardiner's alphabet — the hieroglyphic "alphabet" of single-
/// consonant signs, picked to match the visual weight of the other
/// three arms (no wide biliterals / triliterals).
pub const ARM_RIGHT_GLYPHS: [&str; CROSSHAIR_CENTER_CELL] = [
    "\u{131CB}", // 𓇋 i — reed flower
    "\u{13171}", // 𓅱 w — quail chick
    "\u{130C0}", // 𓃀 b — leg
    "\u{132AA}", // 𓊪 p — stool
    "\u{13191}", // 𓆑 f — horned viper
    "\u{13216}", // 𓈖 n — water ripple
    "\u{1308B}", // 𓂋 r — mouth
    "\u{13254}", // 𓉔 h — reed shelter
    "\u{132F4}", // 𓋴 s — folded cloth
    "\u{133CF}", // 𓏏 t — bread loaf
];

/// Center wheel preview glyph — ॐ (U+0950, Devanagari Om). Replaces
/// the earlier ✦ dingbat as the focal point of the mandala-shaped
/// picker. Rendered at `layout.preview_size = font_size * 2.0`.
pub const CENTER_PREVIEW_GLYPH: &str = "\u{0950}";

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
        /// Last cursor position seen by `handle_color_picker_mouse_move`,
        /// in window-space pixels. `None` before the first mouse event
        /// after open. Threaded into geometry so `compute_picker_geometry`
        /// can toggle `hex_visible` based on "cursor inside backdrop".
        last_cursor_pos: Option<(f32, f32)>,
        /// Widest shaped advance across all 40 crosshair-arm glyphs at
        /// base `font_size`. Measured once at picker-open via
        /// cosmic-text in `open_color_picker`, cached here so every
        /// subsequent `compute_picker_geometry` call can forward it to
        /// the pure layout fn as the cell-spacing unit. Keeps the four
        /// arms symmetric even when one script shapes wider than the
        /// others.
        max_cell_advance: f32,
        /// Same, for the 24 hue ring glyphs at
        /// `font_size * HUE_RING_FONT_SCALE`. Measured once at open.
        max_ring_advance: f32,
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
    /// Whether the hex readout should render this frame. `true` when
    /// the cursor is inside the backdrop OR a chip is focused; `false`
    /// otherwise. The readout was previously always-on and collided
    /// with the lower val bar cells — now it appears only when the
    /// user is actively engaging with the picker.
    pub hex_visible: bool,
    /// Widest shaped advance across the 40 crosshair-arm glyphs at
    /// base `font_size`. Measured by the renderer via cosmic-text at
    /// picker open. Used by `compute_color_picker_layout` as the
    /// cell-spacing unit for both bars so all four arms stay symmetric
    /// regardless of per-script shaping width.
    pub max_cell_advance: f32,
    /// Same, for the 24 hue ring glyphs at
    /// `font_size * HUE_RING_FONT_SCALE`. Used as the ring's
    /// tangential slot-spacing baseline so slots never overlap at the
    /// new larger font size.
    pub max_ring_advance: f32,
}

/// Pure-function output of the color-picker layout pass. All positions
/// are in screen-space pixels.
#[derive(Debug, Clone)]
pub struct ColorPickerLayout {
    pub center: (f32, f32),
    pub outer_radius: f32,
    pub font_size: f32,
    pub char_width: f32,
    /// Actual per-cell advance used for both bars (derived from
    /// `geometry.max_cell_advance`). Exposed so the hit-test can use
    /// the same tolerance the renderer uses.
    pub cell_advance: f32,
    /// Ring font size actually used (`font_size * HUE_RING_FONT_SCALE`).
    /// Exposed so the renderer and hit test stay in sync.
    pub ring_font_size: f32,
    /// 24 hue ring positions, ordered clockwise from 12-o'clock.
    pub hue_slot_positions: [(f32, f32); HUE_SLOT_COUNT],
    /// 21 sat-bar cell centers, left → right. Cell 10 is the wheel
    /// center — NOT rendered (ॐ shows through), but still used by
    /// hit-testing so a click at the exact center resolves to it.
    pub sat_cell_positions: [(f32, f32); SAT_CELL_COUNT],
    /// 21 val-bar cell centers, top → bottom (top = brightest). Cell
    /// 10 is the wheel center — same skip rule as sat.
    pub val_cell_positions: [(f32, f32); VAL_CELL_COUNT],
    /// Center preview glyph anchor (the ॐ). Top-left corner of the
    /// glyph box, computed so the glyph visually centers on the wheel
    /// center given `preview_size`.
    pub preview_pos: (f32, f32),
    /// Font size for the central ॐ preview glyph. 2× the base
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
    /// `Some((x, y))` top-left anchor for the hex readout when it
    /// should render this frame, `None` otherwise. Derived from
    /// `geometry.hex_visible`. When `Some`, the readout is anchored
    /// below the chip row, horizontally centered on `center.0`.
    pub hex_pos: Option<(f32, f32)>,
}


/// Pure-function layout. No GPU access, no font system — mirrors
/// `compute_palette_frame_layout` so unit tests can construct one from
/// nothing but a geometry struct + screen dimensions.
///
/// The cell-spacing unit (`geometry.max_cell_advance`) and ring-slot
/// spacing unit (`geometry.max_ring_advance`) are measured at picker
/// open time by the renderer (see `measure_max_glyph_advance` in
/// `renderer.rs`) and threaded through `ColorPickerOverlayGeometry`.
/// That keeps this fn pure but lets the layout honor the real shaped
/// width of sacred-script glyphs — crucial because Devanagari
/// clusters, Tibetan stacks, and especially Egyptian hieroglyphs
/// shape much wider than `font_size * 0.6`, and all four crosshair
/// arms must share a single cell advance so the cross reads as a
/// symmetric cross.
pub fn compute_color_picker_layout(
    geometry: &ColorPickerOverlayGeometry,
    screen_w: f32,
    screen_h: f32,
) -> ColorPickerLayout {
    let font_size: f32 = 16.0;
    let char_width = font_size * 0.6;
    let ring_font_size = font_size * HUE_RING_FONT_SCALE;

    // Cell-advance units from geometry, with floor fallbacks so the
    // layout still produces sane numbers when called with a stubbed
    // zero (unit tests, or the very first rebuild before the renderer
    // has had a chance to measure).
    let cell_advance = geometry.max_cell_advance.max(char_width);
    let ring_advance = geometry.max_ring_advance.max(ring_font_size * 0.6);

    // Ring radius has to be large enough that adjacent slots don't
    // overlap at the new font size. Tangential spacing between
    // neighbors at radius R is `2*pi*R / 24`, so a minimum radius of
    // `(ring_advance * 24) / (2*pi)` guarantees no overlap. Then we
    // pad by half a ring-font so the glyphs have breathing room from
    // the wheel edge.
    let min_ring_r = (ring_advance * HUE_SLOT_COUNT as f32) / TAU;
    // Inner extent needed to fit `(CROSSHAIR_CENTER_CELL) * cell_advance`
    // between the wheel center and the ring inner edge on each side.
    let inner_extent = CROSSHAIR_CENTER_CELL as f32 * cell_advance;
    // Ring must be big enough to enclose both the crosshair bars and
    // the minimum tangential spacing. Grow `ring_r` to whichever
    // constraint dominates, plus a small padding between the bar tip
    // and the ring glyphs so they don't touch.
    let bar_to_ring_padding = ring_font_size * 0.8;
    let desired_ring_r = (inner_extent + bar_to_ring_padding).max(min_ring_r);

    // Derive the backdrop extent from the actual ring (plus padding
    // for title above, chips + hint below). Clamp to window so small
    // windows still produce a layout that fits.
    let ring_outer = desired_ring_r + ring_font_size * 0.5;
    let side_from_ring = (ring_outer + font_size) * 2.0;
    let max_side_for_w = (screen_w - font_size * 2.0).max(0.0);
    // Vertical budget derivation: with the new `backdrop_height =
    // side + font_size * 7.0` and `backdrop_top = center.y - side/2
    // - font_size`, the backdrop's bottom edge is at
    //     center.y + side/2 + font_size * 6
    // and the top edge is at
    //     center.y - side/2 - font_size.
    // For the top to be >= 0 we need `side <= screen_h - 2*font_size`.
    // For the bottom to be <= screen_h we need
    // `side <= screen_h - 12*font_size`, so the bottom constraint
    // dominates at `center.y = screen_h/2`. A 12 font_size floor
    // leaves enough room for title, wheel, chip row, hex readout,
    // and hint footer even on small windows.
    let max_side_for_h = (screen_h - font_size * 12.0).max(0.0);
    let side = side_from_ring
        .min(max_side_for_w)
        .min(max_side_for_h)
        .max(0.0);
    // Recompute ring_r from the possibly-clamped side so layout is
    // consistent. On unconstrained windows this is a no-op; on small
    // windows it shrinks the ring to fit.
    // Guard against side < 2*font_size producing a negative radius
    // on very small windows. Clamp to 0 so downstream consumers
    // (chip_row_y placement, backdrop math) get a sane value.
    let outer_radius = (side * 0.5 - font_size).max(0.0);
    let ring_r = (outer_radius - ring_font_size * 0.5).max(0.0);
    let center = (screen_w * 0.5, screen_h * 0.5);

    // ---- Hue ring (24 slots, clockwise from 12 o'clock) ----
    let mut hue_slot_positions = [(0.0_f32, 0.0_f32); HUE_SLOT_COUNT];
    for (i, slot) in hue_slot_positions.iter_mut().enumerate() {
        let angle = (i as f32 / HUE_SLOT_COUNT as f32) * TAU - FRAC_PI_2;
        *slot = (
            center.0 + angle.cos() * ring_r,
            center.1 + angle.sin() * ring_r,
        );
    }

    // ---- Crosshair sat/val bars (21 cells each, center cell is the
    // wheel center and rendered as ॐ not as a bar cell) ----
    // Bars span `20 * cell_advance` across the diameter of the inner
    // cross region. If the constrained ring forced the inner extent
    // smaller than `CROSSHAIR_CENTER_CELL * cell_advance`, shrink the
    // actual step so cells still fit — keeps the small-window case
    // from producing overlapping arm glyphs.
    let constrained_inner = ring_r - bar_to_ring_padding;
    let actual_cell_advance = if constrained_inner > 0.0 {
        (constrained_inner / CROSSHAIR_CENTER_CELL as f32).min(cell_advance)
    } else {
        0.0
    };
    let step = actual_cell_advance;
    let bar_span = step * (SAT_CELL_COUNT as f32 - 1.0);
    let mut sat_cell_positions = [(0.0_f32, 0.0_f32); SAT_CELL_COUNT];
    let mut val_cell_positions = [(0.0_f32, 0.0_f32); VAL_CELL_COUNT];
    for i in 0..SAT_CELL_COUNT {
        sat_cell_positions[i] = (center.0 - bar_span * 0.5 + i as f32 * step, center.1);
    }
    for i in 0..VAL_CELL_COUNT {
        val_cell_positions[i] = (center.0, center.1 - bar_span * 0.5 + i as f32 * step);
    }

    // Center preview ॐ at the bar intersection. The glyph renders
    // at 2× font_size; the top-left of its box is offset by half the
    // preview size in each direction so the visible glyph sits on
    // the geometric wheel center. cosmic-text's effective glyph
    // width is ~0.6 of its font size; we use 0.4 horizontally because
    // the ॐ glyph (like the earlier ✦) has whitespace around it that
    // the box includes but the visible mark does not.
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
    //
    // Backdrop height leaves room for title (1 font_size above the
    // wheel) + wheel diameter + chip row (1.5 font_size) + chip row
    // height + hex readout row (1.5 font_size) + hint footer (1.5
    // font_size).
    let backdrop_left = center.0 - side * 0.5;
    let backdrop_top = center.1 - side * 0.5 - font_size;
    let backdrop_height = side + font_size * 7.0;
    let backdrop = (backdrop_left, backdrop_top, side, backdrop_height);
    let title_pos = (backdrop_left + font_size * 0.5, backdrop_top + font_size * 0.5);
    let hint_pos = (
        backdrop_left + font_size * 0.5,
        backdrop_top + backdrop_height - font_size * 1.5,
    );

    // ---- Hex readout position ----
    // The hex readout is hidden by default; `geometry.hex_visible`
    // gates whether it renders this frame. When visible, anchor it
    // below the chip row (between the chips and the hint footer),
    // horizontally centered on `center.0`. "#rrggbb" is 7 chars wide,
    // so the top-left anchor is `center.x - 3.5 * char_width`.
    let hex_pos = if geometry.hex_visible {
        let hex_width = char_width * 7.0;
        let hex_y = chip_row_y + chip_height + font_size * 0.25;
        Some((center.0 - hex_width * 0.5, hex_y))
    } else {
        None
    };

    ColorPickerLayout {
        center,
        outer_radius,
        font_size,
        char_width,
        cell_advance: step,
        ring_font_size,
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
        hex_pos,
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
    // the inner cross region. Cell tolerance scales with the actual
    // per-cell advance so denser bars (smaller cell_advance on small
    // windows) have proportionally smaller hit boxes.
    let cell_half = (layout.cell_advance * 0.5).max(layout.font_size * 0.4);

    // Sat (horizontal) bar — only consider when cursor is vertically
    // close to the bar line. Skip the center cell so a click at the
    // wheel center falls through to the hue ring (or stays Inside) —
    // the center cell is visually occupied by ॐ, not by a bar glyph.
    if (y - layout.center.1).abs() <= cell_half {
        for (i, (cx, _)) in layout.sat_cell_positions.iter().enumerate() {
            if i == CROSSHAIR_CENTER_CELL {
                continue;
            }
            if (x - cx).abs() <= cell_half {
                return PickerHit::SatCell(i);
            }
        }
    }
    // Val (vertical) bar — same in the other axis.
    if (x - layout.center.0).abs() <= cell_half {
        for (i, (_, cy)) in layout.val_cell_positions.iter().enumerate() {
            if i == CROSSHAIR_CENTER_CELL {
                continue;
            }
            if (y - cy).abs() <= cell_half {
                return PickerHit::ValCell(i);
            }
        }
    }

    // Hue ring — annular hit. Only the slot whose glyph contains the
    // cursor counts (ring-scaled tolerance), so the empty space inside
    // the ring stays inert.
    let ring_half = layout.ring_font_size * 0.5;
    for (i, (px, py)) in layout.hue_slot_positions.iter().enumerate() {
        let dx = x - px;
        let dy = y - py;
        if dx * dx + dy * dy <= ring_half * ring_half {
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
            hex_visible: false,
            // Plausible stub advances. 16.0 is the base font_size and
            // 24.0 is font_size * HUE_RING_FONT_SCALE, so these match
            // what the renderer would measure for ordinary Latin text.
            // Real sacred-script measurements will be wider, but the
            // pure-function layout only cares that the numbers are
            // non-zero and self-consistent.
            max_cell_advance: 16.0,
            max_ring_advance: 24.0,
        }
    }

    fn sample_geometry_with_hex() -> ColorPickerOverlayGeometry {
        let mut g = sample_geometry();
        g.hex_visible = true;
        g
    }

    #[test]
    fn layout_emits_24_hue_slots_on_circle() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        assert_eq!(layout.hue_slot_positions.len(), 24);
        // Ring radius is derived from the actual ring font size, not
        // the base font_size, since HUE_RING_FONT_SCALE > 1 makes
        // the ring glyphs larger than the base.
        let r_target = layout.outer_radius - layout.ring_font_size * 0.5;
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
        assert_eq!(layout.sat_cell_positions.len(), SAT_CELL_COUNT);
        for w in layout.sat_cell_positions.windows(2) {
            assert!(w[1].0 > w[0].0, "sat cells must increase in x");
            assert!((w[0].1 - w[1].1).abs() < 0.1, "sat cells share y");
        }
    }

    #[test]
    fn layout_val_bar_monotonic_y_constant_x() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        assert_eq!(layout.val_cell_positions.len(), VAL_CELL_COUNT);
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

    /// Hue ring slots must not overlap at the new 1.5× font scale.
    /// On a full-size window, consecutive slot centers should be at
    /// least `0.9 * max_ring_advance` apart by straight-line (chord)
    /// distance — anything less means the ring radius got clamped too
    /// tight and glyphs will collide visually. Chord distance (not
    /// arc) because that's what matters for glyph collision: the
    /// glyphs sit at the slot centers, and two glyphs collide when
    /// their chord distance falls below their shaped widths.
    #[test]
    fn hue_ring_slots_do_not_overlap_at_new_font_scale() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        for i in 0..HUE_SLOT_COUNT {
            let j = (i + 1) % HUE_SLOT_COUNT;
            let (px, py) = layout.hue_slot_positions[i];
            let (qx, qy) = layout.hue_slot_positions[j];
            let dx = qx - px;
            let dy = qy - py;
            let dist = (dx * dx + dy * dy).sqrt();
            assert!(
                dist >= g.max_ring_advance * 0.9,
                "adjacent hue slots {i} and {j} only {dist} apart, \
                expected >= {}",
                g.max_ring_advance * 0.9,
            );
        }
    }

    /// `hex_pos` must be `Some` when geometry declares it visible and
    /// `None` otherwise. Regression guard against a renderer that
    /// reaches for hex_pos unconditionally.
    #[test]
    fn hex_pos_is_some_iff_hex_visible() {
        let invisible = compute_color_picker_layout(&sample_geometry(), 1280.0, 720.0);
        assert!(
            invisible.hex_pos.is_none(),
            "hex_pos must be None when hex_visible=false",
        );
        let visible = compute_color_picker_layout(&sample_geometry_with_hex(), 1280.0, 720.0);
        assert!(
            visible.hex_pos.is_some(),
            "hex_pos must be Some when hex_visible=true",
        );
    }

    /// When visible, the hex readout must be horizontally centered on
    /// the wheel center. The top-left anchor is offset left by half
    /// the hex text width (7 chars * char_width).
    #[test]
    fn hex_pos_horizontally_centered_on_wheel_center() {
        let layout = compute_color_picker_layout(&sample_geometry_with_hex(), 1280.0, 720.0);
        let (hx, _) = layout.hex_pos.expect("hex_pos should be Some");
        let hex_width = layout.char_width * 7.0;
        let hex_center_x = hx + hex_width * 0.5;
        assert!(
            (hex_center_x - layout.center.0).abs() < 1.0,
            "hex readout center {hex_center_x} not aligned with wheel center {}",
            layout.center.0,
        );
    }

    /// Each crosshair arm must render exactly 10 cells. The bars
    /// have SAT_CELL_COUNT / VAL_CELL_COUNT = 21 cells, cell
    /// CROSSHAIR_CENTER_CELL = 10 is the shared wheel-center slot
    /// (ॐ overlay), and each arm covers 10 non-center cells —
    /// totaling 40 rendered crosshair glyphs. Also asserts that the
    /// center cells of both bars sit exactly on the wheel center.
    #[test]
    fn crosshair_arms_render_exactly_10_cells_each() {
        let layout = compute_color_picker_layout(&sample_geometry(), 1280.0, 720.0);
        // Center cell of the sat bar = wheel center.
        let (scx, scy) = layout.sat_cell_positions[CROSSHAIR_CENTER_CELL];
        assert!((scx - layout.center.0).abs() < 0.1);
        assert!((scy - layout.center.1).abs() < 0.1);
        // Center cell of the val bar = wheel center.
        let (vcx, vcy) = layout.val_cell_positions[CROSSHAIR_CENTER_CELL];
        assert!((vcx - layout.center.0).abs() < 0.1);
        assert!((vcy - layout.center.1).abs() < 0.1);
        // Left arm = 10 cells (0..CROSSHAIR_CENTER_CELL).
        assert_eq!(CROSSHAIR_CENTER_CELL, 10);
        assert_eq!(ARM_LEFT_GLYPHS.len(), 10);
        // Right arm = 10 cells (CROSSHAIR_CENTER_CELL+1..SAT_CELL_COUNT).
        assert_eq!(SAT_CELL_COUNT - CROSSHAIR_CENTER_CELL - 1, 10);
        assert_eq!(ARM_RIGHT_GLYPHS.len(), 10);
        // Top arm = 10 cells, bottom arm = 10 cells.
        assert_eq!(ARM_TOP_GLYPHS.len(), 10);
        assert_eq!(ARM_BOTTOM_GLYPHS.len(), 10);
        // Four arms × 10 glyphs = 40 total.
        assert_eq!(
            ARM_TOP_GLYPHS.len()
                + ARM_BOTTOM_GLYPHS.len()
                + ARM_LEFT_GLYPHS.len()
                + ARM_RIGHT_GLYPHS.len(),
            40,
        );
    }

    /// The four crosshair arms must emit the same per-cell advance so
    /// the cross reads as a symmetric cross, not a plus sign with one
    /// fat arm. Checks that consecutive sat cells and consecutive val
    /// cells have identical step distances.
    #[test]
    fn crosshair_arms_emit_symmetric_cell_advance() {
        let layout = compute_color_picker_layout(&sample_geometry(), 1280.0, 720.0);
        let sat_step = layout.sat_cell_positions[1].0 - layout.sat_cell_positions[0].0;
        let val_step = layout.val_cell_positions[1].1 - layout.val_cell_positions[0].1;
        assert!(
            (sat_step - val_step).abs() < 0.1,
            "sat step {sat_step} differs from val step {val_step} — \
            cross would render asymmetrically",
        );
        // Every step should be equal (not just the first pair).
        for i in 0..SAT_CELL_COUNT - 1 {
            let s = layout.sat_cell_positions[i + 1].0 - layout.sat_cell_positions[i].0;
            let v = layout.val_cell_positions[i + 1].1 - layout.val_cell_positions[i].1;
            assert!((s - sat_step).abs() < 0.1, "sat step {i}→{} drifted", i + 1);
            assert!((v - val_step).abs() < 0.1, "val step {i}→{} drifted", i + 1);
        }
    }

    /// The 24-glyph hue ring array must have a Devanagari arc, a
    /// Hebrew arc, and a Tibetan arc. Codepoint-range check — not
    /// the identity of individual glyphs — so swapping letters in
    /// the same script doesn't break the test.
    #[test]
    fn hue_ring_glyphs_are_grouped_by_script() {
        fn first_cp(s: &str) -> u32 {
            s.chars().next().expect("glyph string non-empty") as u32
        }
        // Slots 0-7 Devanagari
        for i in 0..8 {
            let cp = first_cp(HUE_RING_GLYPHS[i]);
            assert!(
                (0x0900..=0x097F).contains(&cp),
                "slot {i} codepoint U+{cp:04X} not in Devanagari",
            );
        }
        // Slots 8-15 Hebrew
        for i in 8..16 {
            let cp = first_cp(HUE_RING_GLYPHS[i]);
            assert!(
                (0x0590..=0x05FF).contains(&cp),
                "slot {i} codepoint U+{cp:04X} not in Hebrew",
            );
        }
        // Slots 16-23 Tibetan
        for i in 16..24 {
            let cp = first_cp(HUE_RING_GLYPHS[i]);
            assert!(
                (0x0F00..=0x0FFF).contains(&cp),
                "slot {i} codepoint U+{cp:04X} not in Tibetan",
            );
        }
    }
}
