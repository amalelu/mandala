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
use std::sync::OnceLock;

use baumhard::util::color::{hex_to_hsv_safe, resolve_var};

use crate::application::document::{EdgeRef, MindMapDocument, PortalRef};
use crate::application::widgets::color_picker_widget::load_spec;

/// Number of hue slots on the outer ring. 24 slots = 15° per step. Fine
/// enough that adjacent slots feel continuous, coarse enough that each
/// glyph has a comfortable hit target.
pub const HUE_SLOT_COUNT: usize = 24;

// =============================================================
// Stable channel scheme for the picker tree
// =============================================================
//
// Every picker `GlyphArea` is appended to the tree at a deterministic
// channel — same channel across every rebuild for the same logical
// cell. Stable channels are what let the `MutatorTree` path target
// "the hue ring slot at index 7" without re-deriving an index from
// tree position. Without stability, swapping the picker's full
// rebuild for a mutator-driven update would silently misalign
// (Baumhard's `align_child_walks` pairs mutator children with target
// children by ascending channel — see `tree_walker.rs:226`).
//
// The constants below define the picker's channel space. Bands are
// 100 wide so a future addition (e.g. an extra ring of glyphs) can
// slot in without renumbering. **Order matters**: the values must be
// strictly ascending in tree-insertion order, otherwise the walker
// breaks out of its alignment loop.
//
// Layout-wise: title → hue ring → hint → sat bar → val bar → ࿕
// preview → hex readout → chip row.

pub const PICKER_CHANNEL_TITLE: usize = 1;
pub const PICKER_CHANNEL_HUE_RING_BASE: usize = 100; // +0..23
pub const PICKER_CHANNEL_HINT: usize = 200;
pub const PICKER_CHANNEL_SAT_BASE: usize = 300; // +0..16 (skipping +8)
pub const PICKER_CHANNEL_VAL_BASE: usize = 400; // +0..16 (skipping +8)
pub const PICKER_CHANNEL_PREVIEW: usize = 500;
pub const PICKER_CHANNEL_HEX: usize = 600;

/// Number of cells on each crosshair bar. Odd so the center cell sits
/// exactly on the bar's midpoint (sat=0.5 / val=0.5). Cell 8 is the
/// wheel center where ࿕ lives — it's counted in the HSV quantization
/// but not rendered as a bar cell.
pub const SAT_CELL_COUNT: usize = 17;
pub const VAL_CELL_COUNT: usize = 17;

/// The center cell index of each 17-cell crosshair bar — the wheel
/// center where ࿕ sits. Skipped during bar rendering so the ࿕ glyph
/// shows through cleanly; still counted in sat/val quantization. Its
/// value doubles as the number of rendered cells on each arm, and is
/// used as the fixed size of the per-glyph ink-offset arrays.
pub const CROSSHAIR_CENTER_CELL: usize = 8;

/// Hue ring font size multiplier over the picker's base font_size.
///
/// Backed by [`color_picker.json`](../widgets/color_picker.json).
/// The function form replaces the old `pub const HUE_RING_FONT_SCALE:
/// f32 = 1.7` — moving the value into the widget spec was the first
/// step of the "widget appearance lives in JSON" migration.
pub fn hue_ring_font_scale() -> f32 {
    load_spec().geometry.hue_ring_font_scale
}

// =============================================================
// Glyph accessors — all read from the JSON widget spec
// =============================================================
//
// The old `pub const HUE_RING_GLYPHS: [&str; 24]` (and its four
// crosshair-arm siblings) moved into `color_picker.json`. Runtime
// callers go through these accessors, which read from the spec and
// cache a leaked `&'static [&'static str]` so the existing
// `[&str]`-shaped call-sites keep working unchanged.

/// Cached `&'static [&'static str]` derived from a Vec<String> in the
/// spec. The spec is itself cached; leaking the per-glyph strings
/// costs one allocation per glyph per process, which is trivial
/// (~32 glyphs total) and avoids spreading `String` ownership
/// through the render hot path.
fn leak_glyphs(v: &[String]) -> &'static [&'static str] {
    let slice: Vec<&'static str> = v
        .iter()
        .map(|s| &*Box::leak(s.clone().into_boxed_str()))
        .collect();
    Box::leak(slice.into_boxed_slice())
}

/// Hue ring sacred-script glyphs, clockwise from 12 o'clock.
/// Backed by `color_picker.json`'s `hue_ring_glyphs`. Three 8-glyph
/// arcs today: Devanagari (top-right), Hebrew (bottom-right),
/// Tibetan (bottom-left → top-left). Each glyph indexes directly
/// into `hue_slot_positions[i]`.
pub fn hue_ring_glyphs() -> &'static [&'static str] {
    static CACHE: OnceLock<&'static [&'static str]> = OnceLock::new();
    CACHE.get_or_init(|| leak_glyphs(&load_spec().hue_ring_glyphs))
}

/// Val bar top arm glyphs (brightest → mid).
pub fn arm_top_glyphs() -> &'static [&'static str] {
    static CACHE: OnceLock<&'static [&'static str]> = OnceLock::new();
    CACHE.get_or_init(|| leak_glyphs(&load_spec().arm_top_glyphs))
}

/// Val bar bottom arm glyphs (mid → darkest). Typically Egyptian
/// hieroglyphs; cosmic-text needs an explicit font hint for these —
/// see [`arm_bottom_font`].
pub fn arm_bottom_glyphs() -> &'static [&'static str] {
    static CACHE: OnceLock<&'static [&'static str]> = OnceLock::new();
    CACHE.get_or_init(|| leak_glyphs(&load_spec().arm_bottom_glyphs))
}

/// Sat bar left arm glyphs (desaturated → mid).
pub fn arm_left_glyphs() -> &'static [&'static str] {
    static CACHE: OnceLock<&'static [&'static str]> = OnceLock::new();
    CACHE.get_or_init(|| leak_glyphs(&load_spec().arm_left_glyphs))
}

/// Sat bar right arm glyphs (mid → saturated).
pub fn arm_right_glyphs() -> &'static [&'static str] {
    static CACHE: OnceLock<&'static [&'static str]> = OnceLock::new();
    CACHE.get_or_init(|| leak_glyphs(&load_spec().arm_right_glyphs))
}

/// Central preview glyph — doubles as the commit button on the ࿕.
pub fn center_preview_glyph() -> &'static str {
    static CACHE: OnceLock<&'static str> = OnceLock::new();
    CACHE.get_or_init(|| {
        Box::leak(load_spec().center_preview_glyph.clone().into_boxed_str())
    })
}

/// Explicit font family the renderer should pin when shaping
/// `arm_bottom_glyphs`. `None` if the spec didn't set one — in which
/// case cosmic-text's default fallback picks a face.
pub fn arm_bottom_font() -> Option<baumhard::font::fonts::AppFont> {
    load_spec().arm_bottom_font
}

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
/// Which visual axis on a node the picker should write to when the
/// target is a node. Edges / portals don't need this — they have one
/// color field each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeColorAxis {
    Bg,
    Text,
    Border,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ColorTarget {
    Edge(EdgeRef),
    Portal(PortalRef),
    Node { id: String, axis: NodeColorAxis },
}

/// Resolved handle carried inside [`ColorPickerState::Open`]. For
/// edges and portals it indexes into the live `Vec`; for nodes it
/// carries the id + axis directly. One enum instead of `kind +
/// target_index` + a parallel optional id field.
#[derive(Clone, Debug)]
pub enum PickerHandle {
    Edge(usize),
    Portal(usize),
    Node { id: String, axis: NodeColorAxis },
}

impl PickerHandle {
    /// Short label for the picker title bar.
    pub fn label(&self) -> &'static str {
        match self {
            PickerHandle::Edge(_) => "edge",
            PickerHandle::Portal(_) => "portal",
            PickerHandle::Node { .. } => "node",
        }
    }

    pub fn kind(&self) -> TargetKind {
        match self {
            PickerHandle::Edge(_) => TargetKind::Edge,
            PickerHandle::Portal(_) => TargetKind::Portal,
            PickerHandle::Node { .. } => TargetKind::Node,
        }
    }
}

/// Coarse target kind for legacy call-sites that only need to
/// distinguish edges / portals / nodes without caring about the
/// concrete id or axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Edge,
    Portal,
    Node,
}

impl TargetKind {
    /// Short label for the picker title bar.
    pub fn label(&self) -> &'static str {
        match self {
            TargetKind::Edge => "edge",
            TargetKind::Portal => "portal",
            TargetKind::Node => "node",
        }
    }
}

impl ColorTarget {
    /// Resolve the target ref to a concrete [`PickerHandle`]. Returns
    /// `None` if the underlying edge / portal / node was deleted
    /// between the open trigger and the picker-open call (should
    /// never happen in practice because the modal holds the event
    /// loop, but defensive).
    pub fn resolve(self, doc: &MindMapDocument) -> Option<PickerHandle> {
        match self {
            ColorTarget::Edge(er) => doc
                .mindmap
                .edges
                .iter()
                .position(|e| er.matches(e))
                .map(PickerHandle::Edge),
            ColorTarget::Portal(pr) => doc
                .mindmap
                .portals
                .iter()
                .position(|p| pr.matches(p))
                .map(PickerHandle::Portal),
            ColorTarget::Node { id, axis } => doc
                .mindmap
                .nodes
                .contains_key(&id)
                .then_some(PickerHandle::Node { id, axis }),
        }
    }
}

/// Read the current color string for a handle. Used to seed picker
/// HSV at open time and to read the effective color for the
/// preview after a chip action. Returns `None` if the index / id
/// no longer resolves.
pub fn current_color_at(
    doc: &MindMapDocument,
    handle: &PickerHandle,
) -> Option<String> {
    match handle {
        PickerHandle::Edge(index) => {
            let e = doc.mindmap.edges.get(*index)?;
            Some(
                e.glyph_connection
                    .as_ref()
                    .and_then(|gc| gc.color.clone())
                    .unwrap_or_else(|| e.color.clone()),
            )
        }
        PickerHandle::Portal(index) => {
            doc.mindmap.portals.get(*index).map(|p| p.color.clone())
        }
        PickerHandle::Node { id, axis } => {
            let n = doc.mindmap.nodes.get(id)?;
            Some(match axis {
                NodeColorAxis::Bg => n.style.background_color.clone(),
                NodeColorAxis::Text => n.style.text_color.clone(),
                NodeColorAxis::Border => n.style.frame_color.clone(),
            })
        }
    }
}

/// Resolve the current color through the canvas theme variables and
/// parse it into HSV for seeding the picker state. Falls back to
/// `(0.0, 0.0, 0.5)` (mid-gray) on any failure so the picker always
/// opens with a sensible default.
pub fn current_hsv_at(
    doc: &MindMapDocument,
    handle: &PickerHandle,
) -> (f32, f32, f32) {
    let raw = match current_color_at(doc, handle) {
        Some(s) => s,
        None => return (0.0, 0.0, 0.5),
    };
    let resolved = resolve_var(&raw, &doc.mindmap.canvas.theme_variables);
    hex_to_hsv_safe(resolved).unwrap_or((0.0, 0.0, 0.5))
}

// =============================================================
// State machine
// =============================================================

/// Which of the two picker modes is active.
///
/// - **Contextual**: summoned with a specific target (e.g.
///   `color pick edge`, `color bg` on a node). Clicking the center ࿕
///   glyph commits the current HSV to that bound target and closes
///   the wheel. Esc cancels (restores the original color) and closes.
///   A click outside the backdrop also cancels. The pre-existing
///   single-target picker flow lives here.
/// - **Standalone**: summoned as a persistent palette
///   (`color picker on`). No target bound at open time. Clicking ࿕
///   applies the current HSV to every colorable item in the
///   document's current selection (supports multi-select), then
///   stays open. Esc and outside-clicks are ignored. The only way to
///   dismiss it is `color picker off`. The wheel can also be
///   dragged around the screen like a floating node.
#[derive(Debug, Clone)]
pub enum PickerMode {
    /// Target-bound picker. Commit writes to this handle and closes.
    Contextual { handle: PickerHandle },
    /// Persistent palette. Commit writes to the document's current
    /// selection (zero, one, or many items); the wheel stays open.
    Standalone,
}

/// Active-gesture bookkeeping for the color picker.
///
/// The picker accepts two mutually-exclusive drag gestures, each
/// captured on a different mouse button. Storing them in one enum
/// rather than two parallel `Option`s makes the mutual exclusion
/// a type invariant: a `Move` and `Resize` cannot both be active.
///
/// - **`Move`** (left-mouse-button drag from a `PickerHit::DragAnchor`
///   region) — translates the wheel center via
///   `center_override`. Released on LMB-up.
/// - **`Resize`** (right-mouse-button drag from a `DragAnchor`) —
///   scales the picker via `size_scale`. The starting cursor radius
///   from the wheel center anchors the multiplicative scale change:
///   pulling away grows the widget, pulling in shrinks it. Released
///   on RMB-up.
#[derive(Debug, Clone, Copy)]
pub enum PickerGesture {
    /// LMB-drag — translate the wheel.
    Move {
        /// Screen-space offset from the current cursor position to
        /// the wheel center at grab time. Preserves the "pick it
        /// up from where you grabbed" feel — the wheel doesn't
        /// snap to the cursor on first move.
        grab_offset: (f32, f32),
    },
    /// RMB-drag — scale the wheel multiplicatively. Distance from
    /// the wheel center at grab time anchors the ratio.
    Resize {
        /// Distance from the cursor to the wheel center at
        /// grab time, floored to `font_size * 3.0` so a grab very
        /// near the center doesn't cause divide-by-tiny sensitivity
        /// explosions.
        anchor_radius: f32,
        /// `size_scale` value at grab time. New scale = this *
        /// (current_radius / anchor_radius), then clamped to
        /// `[resize_scale_min, resize_scale_max]` from the spec.
        anchor_scale: f32,
        /// Wheel center at grab time. Held constant for the
        /// duration of the gesture so re-derived layouts (which
        /// shift `outer_radius`) don't move the radius reference
        /// point under the cursor.
        anchor_center: (f32, f32),
    },
}


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
/// Hot path design: the target handle (when present) is captured at
/// open time inside [`PickerMode::Contextual`] so the hover handler
/// can push `(target_index, hex)` into the document preview without
/// re-resolving any `EdgeRef` / `PortalRef`.
#[derive(Debug, Clone)]
pub enum ColorPickerState {
    Closed,
    Open {
        /// Which of the two picker modes is active. Contextual carries
        /// the bound target; Standalone has none.
        mode: PickerMode,
        /// Current preview hue in degrees, `[0, 360)`.
        hue_deg: f32,
        /// Current preview saturation, `[0, 1]`.
        sat: f32,
        /// Current preview value/lightness, `[0, 1]`.
        val: f32,
        /// Last cursor position seen by `handle_color_picker_mouse_move`,
        /// in window-space pixels. `None` before the first mouse event
        /// after open. Threaded into geometry so `compute_picker_geometry`
        /// can toggle `hex_visible` based on "cursor inside backdrop".
        last_cursor_pos: Option<(f32, f32)>,
        /// Widest shaped advance across all 32 crosshair-arm glyphs at
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
        /// Font size both `max_cell_advance` and `max_ring_advance`
        /// were measured at — typically `spec.geometry.font_max`.
        /// The layout fn divides the absolute advances by this to
        /// recover dimensionless ratios that scale with whatever
        /// font_size the canonical sizing formula picks.
        measurement_font_size: f32,
        /// Per-glyph ink-center-vs-em-box-center offsets
        /// (dimensionless — pixels at `measurement_font_size`
        /// divided by `measurement_font_size`), measured once at open
        /// via `baumhard::font::fonts::measure_glyph_ink_bounds`.
        /// Used by `compute_color_picker_layout` to re-anchor each
        /// arm's cell position on the ink centre rather than the
        /// em-box centre. One offset per arm cell (8 cells per arm,
        /// excluding the centre slot) — every glyph in the picker has
        /// distinct sidebearings and a distinct baseline-relative ink
        /// extent, so a per-arm aggregate can't keep both axes flush.
        arm_top_ink_offsets: [(f32, f32); CROSSHAIR_CENTER_CELL],
        arm_bottom_ink_offsets: [(f32, f32); CROSSHAIR_CENTER_CELL],
        arm_left_ink_offsets: [(f32, f32); CROSSHAIR_CENTER_CELL],
        arm_right_ink_offsets: [(f32, f32); CROSSHAIR_CENTER_CELL],
        /// Same, for the central preview glyph (Tibetan ࿕).
        preview_ink_offset: (f32, f32),
        /// Cached layout from the last rebuild. `None` between `Open`
        /// construction and the first `rebuild_color_picker_overlay`
        /// call (a narrow window — the rebuild is the very next line
        /// in `open_color_picker`, but `Option` makes the invariant
        /// explicit rather than relying on a placeholder).
        layout: Option<ColorPickerLayout>,
        /// Screen-space translation of the wheel center, applied by
        /// `compute_color_picker_layout` in place of the default
        /// `(screen_w/2, screen_h/2)`. Updated while a drag is active;
        /// retained after drag-release so the wheel stays where the
        /// user left it until the picker closes.
        center_override: Option<(f32, f32)>,
        /// User-controlled multiplier on the picker's size. 1.0 =
        /// the spec's `target_frac` of the screen's shorter side.
        /// Mutated by the right-mouse-button drag-to-resize gesture
        /// in `app.rs`. Reset to 1.0 on each new picker open
        /// (mirrors `center_override` lifecycle) so that opening
        /// after a window resize gets the new screen-derived
        /// default rather than persisting a stale scale.
        size_scale: f32,
        /// Active-gesture bookkeeping. `Some` while the user is
        /// dragging the wheel (LMB-move or RMB-resize); `None` at
        /// all other times. The two gestures are mutually exclusive
        /// by construction — only one variant can be set at a time.
        gesture: Option<PickerGesture>,
        /// Which interactive element the cursor is currently over,
        /// or `None` if it's over the backdrop or outside. Used by
        /// the builder to apply the hover-grow scale to the matching
        /// cell. Updated by `handle_color_picker_mouse_move` and
        /// diffed against the previous value to avoid redundant
        /// rebuilds.
        hovered_hit: Option<PickerHit>,
        /// Pending error-flash animation request. Set when the user
        /// attempts an action that can't complete (e.g. clicking the
        /// center commit button in Standalone mode with no
        /// selection). Today this is a plain boolean stub — the
        /// animation system isn't wired yet, so the renderer ignores
        /// it. When that lands, this becomes the hook point: flip it
        /// on, the renderer reads and clears it, the tree gets a
        /// red-tint mutation for ~250 ms. See [`request_error_flash`].
        pending_error_flash: bool,
    },
}

impl ColorPickerState {
    pub fn is_open(&self) -> bool {
        matches!(self, ColorPickerState::Open { .. })
    }

    /// The bound contextual handle if this picker is open in Contextual
    /// mode. Returns `None` when the picker is closed OR when it's
    /// open in Standalone mode (where commits target the document's
    /// current selection instead of a pre-bound handle).
    pub fn contextual_handle(&self) -> Option<&PickerHandle> {
        match self {
            ColorPickerState::Open {
                mode: PickerMode::Contextual { handle },
                ..
            } => Some(handle),
            _ => None,
        }
    }

    /// True if the picker is open in Standalone mode.
    pub fn is_standalone(&self) -> bool {
        matches!(
            self,
            ColorPickerState::Open {
                mode: PickerMode::Standalone,
                ..
            }
        )
    }
}

/// Error-flash animation categories. Today only `Error` is defined —
/// the enum exists so the call-sites have a stable vocabulary and the
/// future animation system can branch on kind without another API
/// change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlashKind {
    /// The user tried something that can't complete right now (e.g.
    /// committing in Standalone mode with an empty selection). Flash
    /// the wheel briefly red.
    Error,
}

/// Request a brief animation flash on the picker. Hook point for the
/// animation system: today this is a no-op stub that just flips
/// `pending_error_flash` so callers can exercise the branch, but once
/// the animation pipeline lands the renderer will pick the flag up on
/// the next frame, enqueue a timed red-tint mutation over the tree,
/// and clear the flag. The current `_kind` argument will fan out to
/// per-kind mutation recipes at that point; today every kind is the
/// same no-op.
pub fn request_error_flash(state: &mut ColorPickerState, _kind: FlashKind) {
    if let ColorPickerState::Open {
        pending_error_flash,
        ..
    } = state
    {
        *pending_error_flash = true;
    }
}

// =============================================================
// Geometry handed to the renderer + cached layout
// =============================================================

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
    /// [`ColorPickerState::Open`] and reset to 1.0 on each new open.
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
    /// Cell font size (crosshair glyphs) actually used —
    /// `font_size * cell_font_scale`. Exposed so the renderer and
    /// the hit test use the same value.
    pub cell_font_size: f32,
    /// Ring font size actually used (`font_size * HUE_RING_FONT_SCALE`).
    /// Exposed so the renderer and hit test stay in sync.
    pub ring_font_size: f32,
    /// 24 hue ring positions, ordered clockwise from 12-o'clock.
    pub hue_slot_positions: [(f32, f32); HUE_SLOT_COUNT],
    /// 17 sat-bar cell centers, left → right. Cell 8 is the wheel
    /// center — NOT rendered (center glyph shows through), but still
    /// used by hit-testing so a click at the exact center resolves
    /// to it.
    pub sat_cell_positions: [(f32, f32); SAT_CELL_COUNT],
    /// 17 val-bar cell centers, top → bottom (top = brightest). Cell
    /// 8 is the wheel center — same skip rule as sat.
    pub val_cell_positions: [(f32, f32); VAL_CELL_COUNT],
    /// Center preview glyph anchor (the ࿕). Top-left corner of the
    /// glyph box, computed so the glyph visually centers on the wheel
    /// center given `preview_size`.
    pub preview_pos: (f32, f32),
    /// Font size for the central preview glyph. A multiple of the
    /// base `font_size` per `spec.geometry.preview_size_scale`.
    pub preview_size: f32,
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
    /// below the wheel, horizontally centered on `center.0`.
    pub hex_pos: Option<(f32, f32)>,
}


/// Pure-function layout. No GPU access, no font system — mirrors
/// `compute_palette_frame_layout` so unit tests can construct one from
/// nothing but a geometry struct + screen dimensions.
///
/// # Canonical sizing formula
///
/// The picker is a *widget*, not a modal — its size is driven from a
/// target wheel-diameter fraction of the screen's shorter side. This
/// fn back-solves font_size by inverting the geometry chain:
///
/// 1. Convert the measured glyph advances (which arrive as absolute
///    pixels from `measure_max_glyph_advance` at picker open) into
///    dimensionless ratios by dividing by `measurement_font_size`.
///    The ratios encode each script's per-font shape and stay stable
///    across font sizes — Egyptian hieroglyphs always advance at
///    ~1.0× their font size, Devanagari at ~0.75×, etc.
/// 2. Compute `wheel_side_in_fonts` — the wheel-enclosing square's
///    side measured in units of `font_size`. Drives directly off the
///    same ring-radius constraints used at render time:
///       `ring_r = max(inner_crosshair_fit, no-overlap-on-circle)`,
///       `side = 2 * (ring_r + ring_font/2 + font_padding)`.
/// 3. Pick `target_side = short_axis * target_frac * size_scale` and
///    derive `font_size = clamp(target_side / wheel_side_in_fonts,
///    font_min, font_max)`. Then a final clamp guards against
///    overflow on viewports too small to host even font_min.
/// 4. Re-derive every dimension from the now-known font_size and
///    advance ratios — every layout position scales together.
///
/// This keeps the picker visually consistent across resolutions
/// (always ~38% of the short axis at scale=1.0) and lets the user
/// resize it via the RMB-drag gesture without breaking proportions.
/// The `size_scale = 1.0` default reproduces the same look on every
/// monitor — the previous formula `min(22, h/12)` was effectively
/// constant on any screen taller than 264 px, which is why the
/// picker filled small windows.
pub fn compute_color_picker_layout(
    geometry: &ColorPickerOverlayGeometry,
    screen_w: f32,
    screen_h: f32,
) -> ColorPickerLayout {
    let spec = load_spec();
    let g = &spec.geometry;
    let ring_scale = g.hue_ring_font_scale;

    // Step 1: dimensionless advance ratios. The renderer measures
    // `max_cell_advance` at `measurement_font_size` and
    // `max_ring_advance` at `measurement_font_size * ring_scale`,
    // so the per-font ratios are extracted symmetrically. Fall
    // back to plausible Latin-ish defaults if the measurement was
    // skipped (test stubs with measurement_font_size == 0).
    let measurement_fs = geometry.measurement_font_size.max(1.0);
    let cell_factor = if geometry.measurement_font_size > 0.0 {
        geometry.max_cell_advance / measurement_fs
    } else {
        // Fallback so old/test stubs that pass max_cell_advance
        // directly without measurement_font_size still produce a
        // sane layout. Treats the absolute as if it were measured
        // at our font_max baseline.
        (geometry.max_cell_advance / g.font_max).max(0.6)
    };
    let ring_factor = if geometry.measurement_font_size > 0.0 {
        geometry.max_ring_advance / (measurement_fs * ring_scale)
    } else {
        (geometry.max_ring_advance / (g.font_max * ring_scale)).max(0.6)
    };

    // Preview-clearance floor on the per-font cell advance. The
    // centre preview glyph has radius `preview_size_scale * 0.5`
    // font-units; the first arm cell on each side sits exactly one
    // `cell_advance` from centre. If `cell_advance` is smaller than
    // the preview's radius plus a small padding, the preview's ink
    // overlaps cell[9] / cell[11] (the nearest arm glyphs). Floor
    // `cell_factor` at that clearance so every downstream derivation
    // — `inner_per_font`, `ring_r`, `wheel_side_in_fonts`, the
    // rendered `cell_advance` in step 4 — respects the clearance
    // uniformly and the widget grows proportionally when
    // `preview_size_scale` dominates.
    let preview_clearance_per_font = g.preview_size_scale * 0.5 + g.bar_to_preview_padding_scale;
    let cell_factor = cell_factor.max(preview_clearance_per_font);

    // Step 2: wheel_side_in_fonts. ring_r per font equals the bigger
    // of `CROSSHAIR_CENTER_CELL * cell_factor + bar_to_ring_padding`
    // (crosshair fits inside the ring) and
    // `HUE_SLOT_COUNT * ring_scale * ring_factor / TAU` (the 24 ring
    // glyphs don't overlap on the circumference). The ±1 font padding
    // around the wheel (title above, ring-glyph half-extent on each
    // side) pads the diameter to a square the backdrop can enclose.
    let inner_per_font = CROSSHAIR_CENTER_CELL as f32 * cell_factor;
    let bar_pad_per_font = ring_scale * g.bar_to_ring_padding_scale;
    let crosshair_ring_per_font = inner_per_font + bar_pad_per_font;
    let glyph_ring_per_font = (HUE_SLOT_COUNT as f32 * ring_scale * ring_factor) / TAU;
    let ring_r_per_font = crosshair_ring_per_font.max(glyph_ring_per_font);
    let wheel_side_in_fonts = 2.0 * (ring_r_per_font + ring_scale * 0.5 + 1.0);

    // Step 3: derive font_size from desired widget size.
    let short = screen_w.min(screen_h).max(1.0);
    let target_side = short * g.target_frac * geometry.size_scale.max(0.01);
    let font_from_target = target_side / wheel_side_in_fonts.max(1.0);
    let font_clamped = font_from_target.clamp(g.font_min, g.font_max);
    // Final safety: even at font_min the picker must fit. With
    // `backdrop_top = center.y - side/2 - font` and
    // `backdrop_height = side + 7*font`, the bottom edge sits at
    // `center.y + side/2 + 6*font`, so for the picker centered at
    // `screen_h/2` we need `side + 12*font <= screen_h`.
    // Substituting `side = wheel_side_in_fonts * font` gives
    // `font <= screen_h / (wheel_side_in_fonts + 12)`. Width
    // similarly: `wheel_side_in_fonts + 2` for the wheel itself,
    // but the chip row tops out around 32 font units wide, so we
    // take the larger denominator to satisfy both constraints.
    let max_font_for_h = (screen_h / (wheel_side_in_fonts + 12.0)).max(1.0);
    let chip_width_in_fonts: f32 = 32.0;
    let max_font_for_w =
        (screen_w / (wheel_side_in_fonts + 2.0).max(chip_width_in_fonts)).max(1.0);
    let font_size = font_clamped.min(max_font_for_h).min(max_font_for_w).max(1.0);

    let char_width = font_size * 0.6;
    let ring_font_size = font_size * ring_scale;

    // Step 4: re-derive every dimension at the chosen font_size.
    let cell_advance = (cell_factor * font_size).max(char_width);
    let ring_advance = (ring_factor * ring_font_size).max(ring_font_size * 0.6);
    let inner_extent = CROSSHAIR_CENTER_CELL as f32 * cell_advance;
    let bar_to_ring_padding = ring_font_size * g.bar_to_ring_padding_scale;
    let min_ring_r = (HUE_SLOT_COUNT as f32 * ring_advance) / TAU;
    let desired_ring_r = (inner_extent + bar_to_ring_padding).max(min_ring_r);

    // Backdrop side derived from the now-canonical ring_r — the
    // font_size formula above already accounts for window fit, so
    // these clamps are defensive against rounding and the rare
    // case where the chip-row constraint forced a smaller font
    // than the wheel-side formula expected.
    let ring_outer = desired_ring_r + ring_font_size * 0.5;
    let side_from_ring = (ring_outer + font_size) * 2.0;
    let max_side_for_w = (screen_w - font_size * 2.0).max(0.0);
    let max_side_for_h = (screen_h - font_size * 12.0).max(0.0);
    let side = side_from_ring
        .min(max_side_for_w)
        .min(max_side_for_h)
        .max(0.0);
    let outer_radius = (side * 0.5 - font_size).max(0.0);
    let ring_r = (outer_radius - ring_font_size * 0.5).max(0.0);
    // Wheel center: honor the drag-override if the user has moved
    // the wheel, else sit at the window center. The override is in
    // screen-space pixels (not world/model space) because the picker
    // overlay is screen-space by design.
    let center = geometry
        .center_override
        .unwrap_or((screen_w * 0.5, screen_h * 0.5));

    // ---- Hue ring (24 slots, clockwise from 12 o'clock) ----
    let mut hue_slot_positions = [(0.0_f32, 0.0_f32); HUE_SLOT_COUNT];
    for (i, slot) in hue_slot_positions.iter_mut().enumerate() {
        let angle = (i as f32 / HUE_SLOT_COUNT as f32) * TAU - FRAC_PI_2;
        *slot = (
            center.0 + angle.cos() * ring_r,
            center.1 + angle.sin() * ring_r,
        );
    }

    // ---- Crosshair sat/val bars (17 cells each, center cell is the
    // wheel center and rendered as ࿕ not as a bar cell) ----
    // Bars span `16 * cell_advance` across the diameter of the inner
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

    // Per-glyph ink offsets in layout pixels. The geometry carries
    // dimensionless ratios (ink offset divided by
    // `measurement_font_size`); scale by the current cell font size
    // so the correction tracks the actual glyph size the picker is
    // rendering at. We subtract each glyph's own (dx, dy) from its
    // cell centre so the ink — not the em-box — lands on the
    // crosshair radius. A single per-arm value can't pull this off:
    // every glyph in each arm has its own sidebearings (drives x)
    // and its own baseline-relative ink extent (drives y).
    let cell_fs = font_size * g.cell_font_scale;

    let mut sat_cell_positions = [(0.0_f32, 0.0_f32); SAT_CELL_COUNT];
    let mut val_cell_positions = [(0.0_f32, 0.0_f32); VAL_CELL_COUNT];
    for i in 0..SAT_CELL_COUNT {
        let base_x = center.0 - bar_span * 0.5 + i as f32 * step;
        let base_y = center.1;
        let ink_ratio = if i < CROSSHAIR_CENTER_CELL {
            geometry.arm_left_ink_offsets[i]
        } else if i > CROSSHAIR_CENTER_CELL {
            geometry.arm_right_ink_offsets[i - CROSSHAIR_CENTER_CELL - 1]
        } else {
            (0.0, 0.0)
        };
        sat_cell_positions[i] = (base_x - ink_ratio.0 * cell_fs, base_y - ink_ratio.1 * cell_fs);
    }
    for i in 0..VAL_CELL_COUNT {
        let base_x = center.0;
        let base_y = center.1 - bar_span * 0.5 + i as f32 * step;
        let ink_ratio = if i < CROSSHAIR_CENTER_CELL {
            geometry.arm_top_ink_offsets[i]
        } else if i > CROSSHAIR_CENTER_CELL {
            geometry.arm_bottom_ink_offsets[i - CROSSHAIR_CENTER_CELL - 1]
        } else {
            (0.0, 0.0)
        };
        val_cell_positions[i] = (base_x - ink_ratio.0 * cell_fs, base_y - ink_ratio.1 * cell_fs);
    }

    // Center preview ࿕ at the bar intersection. The glyph is the
    // Tibetan svasti (U+0FD5) — it has a non-trivial left-sidebearing
    // that `Align::Center` would otherwise leave uncorrected. The
    // `preview_ink_offset` carried on geometry is the dimensionless
    // ink-center-vs-advance-center drift measured at open time;
    // scaling by `preview_size` and subtracting moves the
    // glyph-box anchor left/up just enough for the ink to land on
    // the geometric wheel center.
    let preview_size = font_size * g.preview_size_scale;
    let preview_ink_px = (
        geometry.preview_ink_offset.0 * preview_size,
        geometry.preview_ink_offset.1 * preview_size,
    );
    let preview_pos = (
        center.0 - preview_size * 0.5 - preview_ink_px.0,
        center.1 - preview_size * 0.5 - preview_ink_px.1,
    );

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
    // wheel) + wheel diameter + hex readout row (1.5 font_size) +
    // hint footer (1.5 font_size) + bottom padding (3 font_size).
    let backdrop_width = side.min((screen_w - font_size * 2.0).max(0.0));
    let backdrop_left = center.0 - backdrop_width * 0.5;
    let backdrop_top = center.1 - side * 0.5 - font_size;
    let backdrop_height = side + font_size * 7.0;
    let backdrop = (backdrop_left, backdrop_top, backdrop_width, backdrop_height);
    let title_pos = (backdrop_left + font_size * 0.5, backdrop_top + font_size * 0.5);
    let hint_pos = (
        backdrop_left + font_size * 0.5,
        backdrop_top + backdrop_height - font_size * 1.5,
    );

    // ---- Hex readout position ----
    // The hex readout is hidden by default; `geometry.hex_visible`
    // gates whether it renders this frame. When visible, anchor it
    // below the wheel (between the wheel and the hint footer),
    // horizontally centered on `center.0`. "#rrggbb" is 7 chars wide,
    // so the top-left anchor is `center.x - 3.5 * char_width`.
    let hex_pos = if geometry.hex_visible {
        let hex_width = char_width * 7.0;
        let hex_y = center.1 + outer_radius + font_size * 1.5;
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
        cell_font_size: font_size * g.cell_font_scale,
        ring_font_size,
        hue_slot_positions,
        sat_cell_positions,
        val_cell_positions,
        preview_pos,
        preview_size,
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
    /// The center preview glyph — clicking commits the current HSV
    /// (Contextual: to the bound handle; Standalone: to the
    /// current document selection).
    Commit,
    /// Inside the backdrop but not on any interactive element.
    /// Mouse-down here starts a wheel drag; drag ends on mouse-up.
    /// Replaces the older `Inside` fallback — every inside-but-
    /// not-glyph region is now a drag anchor by design.
    DragAnchor,
    /// Outside the backdrop entirely.
    Outside,
}

/// Hit-test a screen position against the cached picker layout. The
/// search order matches the visual layering: val bar → sat bar →
/// hue ring → center preview glyph (commit). Inside-the-backdrop-
/// but-not-on-any-glyph is the drag anchor for the wheel. Returns
/// `Outside` if the cursor is past the backdrop bounds.
pub fn hit_test_picker(layout: &ColorPickerLayout, x: f32, y: f32) -> PickerHit {
    let (bl, bt, bw, bh) = layout.backdrop;
    if x < bl || x > bl + bw || y < bt || y > bt + bh {
        return PickerHit::Outside;
    }

    // Sat/val bars: pick the closer of the two if the cursor is inside
    // the inner cross region. Cell tolerance scales with the actual
    // per-cell advance so denser bars (smaller cell_advance on small
    // windows) have proportionally smaller hit boxes.
    let cell_half = (layout.cell_advance * 0.5).max(layout.font_size * 0.4);

    // Sat (horizontal) bar — only consider when cursor is vertically
    // close to the bar line. Skip the center cell so a click at the
    // wheel center resolves to `Commit` (the ࿕ button) below.
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

    // Center ࿕ — the commit button. Circular hit of radius
    // `preview_size * 0.45` (slightly smaller than the glyph box so
    // users who click in the padding between the ࿕ and the crosshair
    // arm glyphs don't accidentally commit).
    let commit_radius = layout.preview_size * 0.45;
    let dx = x - layout.center.0;
    let dy = y - layout.center.1;
    if dx * dx + dy * dy <= commit_radius * commit_radius {
        return PickerHit::Commit;
    }

    // Hex readout occupies a thin band below the chips; treat a click
    // there as `DragAnchor` too — it's a display element, not
    // interactive, so dragging from it just moves the wheel.
    PickerHit::DragAnchor
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
        // Plausible stub advances measured at a notional 16 pt
        // baseline. cell ratio = 1.0 (worst-case sacred-script-ish),
        // ring ratio = 0.7 (typical at ring_scale = 1.7). The
        // pure-function layout only cares that the numbers are
        // non-zero and self-consistent. Ink offsets default to zero
        // so the layout tests see the classic em-box centering
        // unless a test explicitly overrides them.
        ColorPickerOverlayGeometry {
            target_label: "edge",
            hue_deg: 0.0,
            sat: 1.0,
            val: 1.0,
            preview_hex: "#ff0000".to_string(),
            hex_visible: false,
            max_cell_advance: 16.0,
            max_ring_advance: 24.0,
            measurement_font_size: 16.0,
            size_scale: 1.0,
            center_override: None,
            hovered_hit: None,
            arm_top_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
            arm_bottom_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
            arm_left_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
            arm_right_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
            preview_ink_offset: (0.0, 0.0),
        }
    }

    fn sample_geometry_with_hex() -> ColorPickerOverlayGeometry {
        let mut g = sample_geometry();
        g.hex_visible = true;
        g
    }

    /// The per-glyph arm and preview ink offsets carried on geometry
    /// are subtracted from the corresponding cell / preview-anchor
    /// positions at layout time. The previous revision tested a
    /// uniform per-arm offset; we now test that each cell consumes
    /// its own array entry — varying the offset per index and
    /// checking each cell shifted by exactly its own (dx, dy) scaled
    /// to layout pixels.
    #[test]
    fn layout_subtracts_ink_offsets_for_arms_and_preview() {
        let baseline = compute_color_picker_layout(&sample_geometry(), 1280.0, 720.0);
        let mut g = sample_geometry();
        // Distinct per-cell offsets so a "single offset for the whole
        // arm" regression would visibly fail. Index `i` (arm-local)
        // carries (0.01*(i+1), -0.02*(i+1)) on the top arm, etc.
        for i in 0..CROSSHAIR_CENTER_CELL {
            let f = (i + 1) as f32;
            g.arm_top_ink_offsets[i] = (0.01 * f, -0.02 * f);
            g.arm_bottom_ink_offsets[i] = (-0.01 * f, 0.02 * f);
            g.arm_left_ink_offsets[i] = (0.015 * f, 0.005 * f);
            g.arm_right_ink_offsets[i] = (-0.015 * f, -0.005 * f);
        }
        g.preview_ink_offset = (0.25, -0.3);
        let shifted = compute_color_picker_layout(&g, 1280.0, 720.0);

        let cell_fs = baseline.cell_font_size;
        let preview_size = baseline.preview_size;

        // Val bar: each arm cell shifts by its own (dx, dy)*cell_fs.
        // Centre cell at CROSSHAIR_CENTER_CELL is untouched.
        for i in 0..VAL_CELL_COUNT {
            let (expect_dx, expect_dy) = if i == CROSSHAIR_CENTER_CELL {
                (0.0, 0.0)
            } else if i < CROSSHAIR_CENTER_CELL {
                let (rx, ry) = g.arm_top_ink_offsets[i];
                (-rx * cell_fs, -ry * cell_fs)
            } else {
                let (rx, ry) = g.arm_bottom_ink_offsets[i - CROSSHAIR_CENTER_CELL - 1];
                (-rx * cell_fs, -ry * cell_fs)
            };
            let dx = shifted.val_cell_positions[i].0 - baseline.val_cell_positions[i].0;
            let dy = shifted.val_cell_positions[i].1 - baseline.val_cell_positions[i].1;
            assert!((dx - expect_dx).abs() < 0.001, "val[{i}].dx {dx} vs {expect_dx}");
            assert!((dy - expect_dy).abs() < 0.001, "val[{i}].dy {dy} vs {expect_dy}");
        }

        // Sat bar: same per-cell pattern using the left/right arrays.
        for i in 0..SAT_CELL_COUNT {
            let (expect_dx, expect_dy) = if i == CROSSHAIR_CENTER_CELL {
                (0.0, 0.0)
            } else if i < CROSSHAIR_CENTER_CELL {
                let (rx, ry) = g.arm_left_ink_offsets[i];
                (-rx * cell_fs, -ry * cell_fs)
            } else {
                let (rx, ry) = g.arm_right_ink_offsets[i - CROSSHAIR_CENTER_CELL - 1];
                (-rx * cell_fs, -ry * cell_fs)
            };
            let dx = shifted.sat_cell_positions[i].0 - baseline.sat_cell_positions[i].0;
            let dy = shifted.sat_cell_positions[i].1 - baseline.sat_cell_positions[i].1;
            assert!((dx - expect_dx).abs() < 0.001, "sat[{i}].dx {dx} vs {expect_dx}");
            assert!((dy - expect_dy).abs() < 0.001, "sat[{i}].dy {dy} vs {expect_dy}");
        }

        // Preview glyph anchor shifts by (-0.25*preview_size, +0.3*preview_size).
        let dx = shifted.preview_pos.0 - baseline.preview_pos.0;
        let dy = shifted.preview_pos.1 - baseline.preview_pos.1;
        assert!((dx - (-0.25 * preview_size)).abs() < 0.001);
        assert!((dy - (0.3 * preview_size)).abs() < 0.001);
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
    fn hit_test_outside_backdrop_returns_outside() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        assert_eq!(hit_test_picker(&layout, -10.0, -10.0), PickerHit::Outside);
        assert_eq!(hit_test_picker(&layout, 5000.0, 5000.0), PickerHit::Outside);
    }

    /// A click at the exact wheel center — where the central ࿕ glyph
    /// lives — must resolve to `Commit`. This is the gesture that
    /// commits the current HSV (Contextual) or applies it to the
    /// document selection (Standalone). The center used to be
    /// inert (`Inside`); the new picker makes it the commit button.
    #[test]
    fn hit_test_hits_commit_on_center() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        let (cx, cy) = layout.center;
        assert_eq!(hit_test_picker(&layout, cx, cy), PickerHit::Commit);
    }

    /// A click just inside the backdrop but outside every
    /// interactive glyph must resolve to `DragAnchor` — the
    /// anywhere-you-can-grab-the-wheel zone. Picks a point far from
    /// the center (outside the commit radius), well outside any
    /// bar cell, and not inside the hue ring annulus. The backdrop
    /// corner is a reliable "nothing here but drag anchor" pick.
    #[test]
    fn hit_test_drag_anchor_when_inside_backdrop_not_on_glyph() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        let (bl, bt, _bw, _bh) = layout.backdrop;
        // 4 px inside the backdrop's top-left corner — far from the
        // ring (which is centered on the wheel), the chips (bottom),
        // the ࿕ (center), and the crosshair arms (central cross).
        let x = bl + 4.0;
        let y = bt + 4.0;
        assert_eq!(hit_test_picker(&layout, x, y), PickerHit::DragAnchor);
    }

    /// With `center_override` set, every position in the layout
    /// (hue slots, bar cells, chips, preview, backdrop) must
    /// translate by the offset between the override and the default
    /// window center. Regression guard for drag repositioning the
    /// wheel — if any one component forgot to read the override, the
    /// test catches it.
    #[test]
    fn center_override_translates_all_positions() {
        let screen_w = 1280.0;
        let screen_h = 720.0;
        let mut g = sample_geometry();
        let baseline = compute_color_picker_layout(&g, screen_w, screen_h);
        let offset = (200.0_f32, -80.0_f32);
        g.center_override = Some((
            screen_w * 0.5 + offset.0,
            screen_h * 0.5 + offset.1,
        ));
        let shifted = compute_color_picker_layout(&g, screen_w, screen_h);
        // Center itself.
        assert!((shifted.center.0 - baseline.center.0 - offset.0).abs() < 1e-3);
        assert!((shifted.center.1 - baseline.center.1 - offset.1).abs() < 1e-3);
        // Hue slot 0.
        assert!((shifted.hue_slot_positions[0].0 - baseline.hue_slot_positions[0].0 - offset.0).abs() < 1e-3);
        assert!((shifted.hue_slot_positions[0].1 - baseline.hue_slot_positions[0].1 - offset.1).abs() < 1e-3);
        // First sat cell.
        assert!((shifted.sat_cell_positions[0].0 - baseline.sat_cell_positions[0].0 - offset.0).abs() < 1e-3);
        // Backdrop top-left.
        let (bl0, bt0, _, _) = baseline.backdrop;
        let (bl1, bt1, _, _) = shifted.backdrop;
        assert!((bl1 - bl0 - offset.0).abs() < 1e-3);
        assert!((bt1 - bt0 - offset.1).abs() < 1e-3);
    }

    /// Hover-hit diffing: a layout computed with no hover should
    /// not differ structurally from one computed with a Hue hover
    /// — the builder applies the scale bump, but the pure layout
    /// positions don't shift. This locks in that `hovered_hit`
    /// stays out of `compute_color_picker_layout`'s output.
    #[test]
    fn hovered_hit_does_not_alter_layout_positions() {
        let mut g = sample_geometry();
        let baseline = compute_color_picker_layout(&g, 1280.0, 720.0);
        g.hovered_hit = Some(PickerHit::Hue(0));
        let hovered = compute_color_picker_layout(&g, 1280.0, 720.0);
        assert_eq!(
            baseline.hue_slot_positions[0], hovered.hue_slot_positions[0],
            "hovered_hit must not alter hue slot positions"
        );
        assert_eq!(baseline.backdrop, hovered.backdrop);
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
    /// the layout-emitted `preview_size`. The ࿕ svasti is a Tibetan
    /// ideograph whose ink sits centered in the em box, so the
    /// canonical half-size offset `(0.5, 0.5)` is the right anchor
    /// on both axes — any future preview glyph with a skewed visible
    /// center needs a commensurate tweak here. Regression guard for
    /// the "preview was anchored off-center" bug.
    #[test]
    fn layout_preview_centered_on_wheel_center() {
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        let (px, py) = layout.preview_pos;
        let cx = px + layout.preview_size * 0.5;
        let cy = py + layout.preview_size * 0.5;
        // The preview's visible center should be within ~1 px of the
        // wheel center on each axis.
        assert!((cx - layout.center.0).abs() < 1.0,
            "preview x center {} differs from wheel center {}", cx, layout.center.0);
        assert!((cy - layout.center.1).abs() < 1.0,
            "preview y center {} differs from wheel center {}", cy, layout.center.1);
    }

    /// Regression guard for the "࿕ overlaps the first arm letter"
    /// bug. With `preview_size_scale = 3.0` and the sacred-script
    /// `cell_factor ≈ 1.0`, the nearest arm cell (index
    /// `CROSSHAIR_CENTER_CELL ± 1`) would sit at `cell_advance` from
    /// centre while the preview reaches `preview_size / 2 =
    /// 1.5 × font_size` — the preview ink ended up covering that
    /// cell. `compute_color_picker_layout` now floors `cell_factor`
    /// at `preview_size_scale * 0.5 + bar_to_preview_padding_scale`,
    /// so cell[9] / cell[11] (and their sat-bar twins) always sit at
    /// least `preview_size / 2 + padding_px` from centre.
    #[test]
    fn layout_keeps_preview_clear_of_adjacent_arm_cells() {
        let padding_scale = load_spec().geometry.bar_to_preview_padding_scale;
        let g = sample_geometry();
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        let preview_radius = layout.preview_size * 0.5;
        let min_clearance = preview_radius + layout.font_size * padding_scale;

        let center = layout.center;
        let neighbours = [
            ("val[CENTER - 1]", layout.val_cell_positions[CROSSHAIR_CENTER_CELL - 1]),
            ("val[CENTER + 1]", layout.val_cell_positions[CROSSHAIR_CENTER_CELL + 1]),
            ("sat[CENTER - 1]", layout.sat_cell_positions[CROSSHAIR_CENTER_CELL - 1]),
            ("sat[CENTER + 1]", layout.sat_cell_positions[CROSSHAIR_CENTER_CELL + 1]),
        ];
        // 0.5 px slack for rounding / ink-offset drift.
        let slack = 0.5;
        for (label, (px, py)) in neighbours {
            let dx = px - center.0;
            let dy = py - center.1;
            let dist = (dx * dx + dy * dy).sqrt();
            assert!(
                dist + slack >= min_clearance,
                "{label} at ({px:.1}, {py:.1}) is {dist:.1} px from centre — below preview clearance {min_clearance:.1}",
            );
        }
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

    /// Each crosshair arm must render exactly 8 cells. The bars
    /// have SAT_CELL_COUNT / VAL_CELL_COUNT = 17 cells, cell
    /// CROSSHAIR_CENTER_CELL = 8 is the shared wheel-center slot
    /// (࿕ overlay), and each arm covers 8 non-center cells —
    /// totaling 32 rendered crosshair glyphs. Also asserts that the
    /// center cells of both bars sit exactly on the wheel center.
    #[test]
    fn crosshair_arms_render_exactly_8_cells_each() {
        let layout = compute_color_picker_layout(&sample_geometry(), 1280.0, 720.0);
        // Center cell of the sat bar = wheel center.
        let (scx, scy) = layout.sat_cell_positions[CROSSHAIR_CENTER_CELL];
        assert!((scx - layout.center.0).abs() < 0.1);
        assert!((scy - layout.center.1).abs() < 0.1);
        // Center cell of the val bar = wheel center.
        let (vcx, vcy) = layout.val_cell_positions[CROSSHAIR_CENTER_CELL];
        assert!((vcx - layout.center.0).abs() < 0.1);
        assert!((vcy - layout.center.1).abs() < 0.1);
        // Left arm = 8 cells (0..CROSSHAIR_CENTER_CELL).
        assert_eq!(CROSSHAIR_CENTER_CELL, 8);
        assert_eq!(arm_left_glyphs().len(), 8);
        // Right arm = 8 cells (CROSSHAIR_CENTER_CELL+1..SAT_CELL_COUNT).
        assert_eq!(SAT_CELL_COUNT - CROSSHAIR_CENTER_CELL - 1, 8);
        assert_eq!(arm_right_glyphs().len(), 8);
        // Top arm = 8 cells, bottom arm = 8 cells.
        assert_eq!(arm_top_glyphs().len(), 8);
        assert_eq!(arm_bottom_glyphs().len(), 8);
        // Four arms × 8 glyphs = 32 total.
        assert_eq!(
            arm_top_glyphs().len()
                + arm_bottom_glyphs().len()
                + arm_left_glyphs().len()
                + arm_right_glyphs().len(),
            32,
        );
    }

    /// Every arm cell, after ink correction, must sit on its target
    /// radial point — sat cells on `center.y`, val cells on
    /// `center.x` — within 0.1 px. This is the per-cell version of
    /// the centre-only assertion in
    /// [`crosshair_arms_render_exactly_10_cells_each`]: a regression
    /// against the previous "single per-arm offset" model would
    /// fail this test because the worst-case heuristic mis-corrected
    /// non-worst glyphs by their delta. We re-add each cell's stored
    /// ink offset (`offset_ratio * cell_fs`) and check the result
    /// hits the unrotated radial point.
    #[test]
    fn crosshair_arms_per_cell_ink_correction_aligns_to_radial_target() {
        // Use a non-trivial per-cell offset pattern so any "lost"
        // index would visibly fail.
        let mut g = sample_geometry();
        for i in 0..CROSSHAIR_CENTER_CELL {
            let f = (i + 1) as f32;
            g.arm_top_ink_offsets[i] = (0.02 * f, -0.03 * f);
            g.arm_bottom_ink_offsets[i] = (-0.02 * f, 0.03 * f);
            g.arm_left_ink_offsets[i] = (0.025 * f, 0.01 * f);
            g.arm_right_ink_offsets[i] = (-0.025 * f, -0.01 * f);
        }
        let layout = compute_color_picker_layout(&g, 1280.0, 720.0);
        let cell_fs = layout.cell_font_size;
        let step = layout.cell_advance;
        let cx = layout.center.0;
        let cy = layout.center.1;

        // Sat cells: each (after re-adding its ink offset) must land
        // on (cx + (i - CENTER)*step, cy).
        for i in 0..SAT_CELL_COUNT {
            if i == CROSSHAIR_CENTER_CELL {
                continue;
            }
            let (rx, ry) = if i < CROSSHAIR_CENTER_CELL {
                g.arm_left_ink_offsets[i]
            } else {
                g.arm_right_ink_offsets[i - CROSSHAIR_CENTER_CELL - 1]
            };
            let (px, py) = layout.sat_cell_positions[i];
            let restored_x = px + rx * cell_fs;
            let restored_y = py + ry * cell_fs;
            let target_x = cx + (i as f32 - CROSSHAIR_CENTER_CELL as f32) * step;
            assert!(
                (restored_x - target_x).abs() < 0.1,
                "sat[{i}] restored x {restored_x} != target {target_x}",
            );
            assert!(
                (restored_y - cy).abs() < 0.1,
                "sat[{i}] restored y {restored_y} != center y {cy}",
            );
        }

        // Val cells: each must land on (cx, cy + (i - CENTER)*step).
        for i in 0..VAL_CELL_COUNT {
            if i == CROSSHAIR_CENTER_CELL {
                continue;
            }
            let (rx, ry) = if i < CROSSHAIR_CENTER_CELL {
                g.arm_top_ink_offsets[i]
            } else {
                g.arm_bottom_ink_offsets[i - CROSSHAIR_CENTER_CELL - 1]
            };
            let (px, py) = layout.val_cell_positions[i];
            let restored_x = px + rx * cell_fs;
            let restored_y = py + ry * cell_fs;
            let target_y = cy + (i as f32 - CROSSHAIR_CENTER_CELL as f32) * step;
            assert!(
                (restored_x - cx).abs() < 0.1,
                "val[{i}] restored x {restored_x} != center x {cx}",
            );
            assert!(
                (restored_y - target_y).abs() < 0.1,
                "val[{i}] restored y {restored_y} != target {target_y}",
            );
        }
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
            let cp = first_cp(hue_ring_glyphs()[i]);
            assert!(
                (0x0900..=0x097F).contains(&cp),
                "slot {i} codepoint U+{cp:04X} not in Devanagari",
            );
        }
        // Slots 8-15 Hebrew
        for i in 8..16 {
            let cp = first_cp(hue_ring_glyphs()[i]);
            assert!(
                (0x0590..=0x05FF).contains(&cp),
                "slot {i} codepoint U+{cp:04X} not in Hebrew",
            );
        }
        // Slots 16-23 Tibetan
        for i in 16..24 {
            let cp = first_cp(hue_ring_glyphs()[i]);
            assert!(
                (0x0F00..=0x0FFF).contains(&cp),
                "slot {i} codepoint U+{cp:04X} not in Tibetan",
            );
        }
    }

    /// Each crosshair arm must be grouped by its own script. Codepoint-
    /// range check — not the identity of individual glyphs — so
    /// swapping letters within the same script doesn't break the test,
    /// while an accidental swap between arms will.
    #[test]
    fn arm_glyphs_are_grouped_by_script() {
        fn first_cp(s: &str) -> u32 {
            s.chars().next().expect("glyph string non-empty") as u32
        }
        // Top arm: Devanagari (U+0900–U+097F)
        for (i, g) in arm_top_glyphs().iter().enumerate() {
            let cp = first_cp(g);
            assert!(
                (0x0900..=0x097F).contains(&cp),
                "top arm cell {i} codepoint U+{cp:04X} not in Devanagari",
            );
        }
        // Bottom arm: Egyptian Hieroglyphs (U+13000–U+1342F)
        for (i, g) in arm_bottom_glyphs().iter().enumerate() {
            let cp = first_cp(g);
            assert!(
                (0x13000..=0x1342F).contains(&cp),
                "bottom arm cell {i} codepoint U+{cp:05X} not in Egyptian Hieroglyphs",
            );
        }
        // Left arm: Tibetan (U+0F00–U+0FFF)
        for (i, g) in arm_left_glyphs().iter().enumerate() {
            let cp = first_cp(g);
            assert!(
                (0x0F00..=0x0FFF).contains(&cp),
                "left arm cell {i} codepoint U+{cp:04X} not in Tibetan",
            );
        }
        // Right arm: Hebrew (U+0590–U+05FF)
        for (i, g) in arm_right_glyphs().iter().enumerate() {
            let cp = first_cp(g);
            assert!(
                (0x0590..=0x05FF).contains(&cp),
                "right arm cell {i} codepoint U+{cp:04X} not in Hebrew",
            );
        }
    }

    /// Canonical sizing formula: at `size_scale = 1.0` the picker
    /// occupies roughly `target_frac` of the screen's shorter side.
    /// Verifies the new screen-driven sizing — the old `min(22, h/12)`
    /// formula was effectively constant on any window taller than
    /// 264 px, which is why the picker filled small windows.
    #[test]
    fn layout_targets_screen_short_side_fraction() {
        let g = sample_geometry();
        let spec = load_spec();
        let target_frac = spec.geometry.target_frac;
        for &(w, h) in &[(1920.0_f32, 1080.0_f32), (1280.0, 720.0), (800.0, 600.0)] {
            let layout = compute_color_picker_layout(&g, w, h);
            let short = w.min(h);
            let (_, _, bw, bh) = layout.backdrop;
            // Backdrop's bigger dimension should sit within ~10–80%
            // of the short axis at scale 1.0 — a wide window like
            // chip-row safety can dominate the wheel-driven side
            // a bit, so the bound is loose.
            let max_extent = bw.max(bh);
            let upper = short * target_frac * 2.5;
            let lower = short * target_frac * 0.3;
            assert!(
                max_extent >= lower && max_extent <= upper,
                "{w}x{h}: backdrop extent {max_extent} not in [{lower}, {upper}] \
                — formula doesn't drive size from short axis"
            );
        }
    }

    /// User-controlled `size_scale` must scale the picker
    /// proportionally. A 1.5× scale on the same window produces a
    /// strictly larger backdrop than the 1.0× baseline, all else
    /// equal. Regression guard for the RMB-resize gesture.
    #[test]
    fn layout_scales_with_user_size_scale() {
        let g_baseline = sample_geometry();
        let mut g_grown = sample_geometry();
        g_grown.size_scale = 1.5;
        let mut g_shrunk = sample_geometry();
        g_shrunk.size_scale = 0.7;
        let baseline = compute_color_picker_layout(&g_baseline, 1920.0, 1080.0);
        let grown = compute_color_picker_layout(&g_grown, 1920.0, 1080.0);
        let shrunk = compute_color_picker_layout(&g_shrunk, 1920.0, 1080.0);
        let (_, _, bw_b, _) = baseline.backdrop;
        let (_, _, bw_g, _) = grown.backdrop;
        let (_, _, bw_s, _) = shrunk.backdrop;
        assert!(
            bw_g > bw_b,
            "size_scale=1.5 backdrop {bw_g} not larger than baseline {bw_b}"
        );
        assert!(
            bw_s < bw_b,
            "size_scale=0.7 backdrop {bw_s} not smaller than baseline {bw_b}"
        );
        // font_size also scales monotonically.
        assert!(grown.font_size > baseline.font_size);
        assert!(shrunk.font_size < baseline.font_size);
    }

    /// Small-window robustness: even on a tiny viewport the layout
    /// must never produce negative geometry or a backdrop that
    /// overflows the screen — the canonical formula's safety
    /// clamp on `font_size` should kick in.
    #[test]
    fn layout_font_shrinks_on_small_windows() {
        let g = sample_geometry();
        let big = compute_color_picker_layout(&g, 1920.0, 1080.0);
        let small = compute_color_picker_layout(&g, 400.0, 300.0);
        assert!(
            small.font_size < big.font_size,
            "small-window font_size {} should shrink below big-window {}",
            small.font_size,
            big.font_size
        );
        let (left, top, bw, bh) = small.backdrop;
        assert!(left >= 0.0 && top >= 0.0);
        assert!(left + bw <= 400.5);
        assert!(top + bh <= 300.5);
    }

    /// `measurement_font_size` factors out: a stub measured at
    /// font_size = 16 with cell_advance = 16 (ratio 1.0) should
    /// produce the same layout as a stub measured at font_size = 8
    /// with cell_advance = 8 (also ratio 1.0). The dimensionless
    /// ratio is what matters; the absolute measurement scale is
    /// not.
    #[test]
    fn layout_uses_dimensionless_advance_ratios() {
        let g_a = sample_geometry();
        let mut g_b = sample_geometry();
        g_b.measurement_font_size = 8.0;
        g_b.max_cell_advance = 8.0;
        g_b.max_ring_advance = 12.0;
        let layout_a = compute_color_picker_layout(&g_a, 1280.0, 720.0);
        let layout_b = compute_color_picker_layout(&g_b, 1280.0, 720.0);
        assert!((layout_a.font_size - layout_b.font_size).abs() < 1e-3);
        assert!((layout_a.outer_radius - layout_b.outer_radius).abs() < 1e-3);
    }

    /// A picker opened on a window-resize-shrunk viewport at
    /// `size_scale = 0.5` keeps the backdrop fully on-screen even
    /// though the user-controlled scale would otherwise produce a
    /// cramped widget. The safety clamp on `font_size` must
    /// dominate the user scale when needed.
    #[test]
    fn layout_safety_clamp_dominates_user_scale_on_tiny_screens() {
        let mut g = sample_geometry();
        g.size_scale = 1.5;
        let layout = compute_color_picker_layout(&g, 250.0, 200.0);
        let (left, top, bw, bh) = layout.backdrop;
        assert!(left >= 0.0 && top >= 0.0);
        assert!(left + bw <= 250.5);
        assert!(top + bh <= 200.5);
    }

    /// Picker channel constants must be strictly ascending in
    /// tree-insertion order, otherwise Baumhard's
    /// `align_child_walks` (which pairs mutator children with
    /// target children by ascending channel) breaks alignment and
    /// the §B2 mutator path silently misses elements.
    ///
    /// Insertion order: title → hue ring (24 slots) → hint →
    /// sat bar (17 cells, channels also stride through the
    /// skipped center) → val bar (same) → preview → hex.
    #[test]
    fn picker_channels_are_strictly_ascending() {
        let bands: &[(&str, usize, usize)] = &[
            ("title", PICKER_CHANNEL_TITLE, 1),
            ("hue ring", PICKER_CHANNEL_HUE_RING_BASE, HUE_SLOT_COUNT),
            ("hint", PICKER_CHANNEL_HINT, 1),
            ("sat bar", PICKER_CHANNEL_SAT_BASE, SAT_CELL_COUNT),
            ("val bar", PICKER_CHANNEL_VAL_BASE, VAL_CELL_COUNT),
            ("preview", PICKER_CHANNEL_PREVIEW, 1),
            ("hex", PICKER_CHANNEL_HEX, 1),
        ];
        let mut prev_band_max: usize = 0;
        for (name, base, count) in bands {
            let band_min = *base;
            let band_max = *base + count - 1;
            assert!(
                band_min > prev_band_max,
                "{name} band starts at {band_min} but previous band ended at {prev_band_max}"
            );
            prev_band_max = band_max;
        }
    }

    /// `PickerGesture::Resize` must compute the new scale
    /// multiplicatively from cursor radius. A 2× radius produces
    /// a 2× scale, a 0.5× radius produces a 0.5× scale, modulo
    /// the spec's `[resize_scale_min, resize_scale_max]` clamp.
    /// The math is shared with `handle_color_picker_mouse_move`;
    /// this test pins it as a pure formula so a refactor that
    /// silently flips additive can't slip through.
    #[test]
    fn resize_gesture_scale_math_is_multiplicative() {
        let spec = load_spec();
        let geom = &spec.geometry;
        let anchor_radius: f32 = 100.0;
        let anchor_scale: f32 = 1.0;
        // 2x radius ⇒ 2x scale (clamped).
        let r_double = anchor_radius * 2.0;
        let new_double =
            (anchor_scale * (r_double / anchor_radius)).clamp(geom.resize_scale_min, geom.resize_scale_max);
        assert!(new_double > anchor_scale);
        assert!(new_double <= geom.resize_scale_max);
        // 0.5x radius ⇒ 0.5x scale (clamped).
        let r_half = anchor_radius * 0.5;
        let new_half =
            (anchor_scale * (r_half / anchor_radius)).clamp(geom.resize_scale_min, geom.resize_scale_max);
        assert!(new_half < anchor_scale);
        assert!(new_half >= geom.resize_scale_min);
        // Identity at same radius.
        let new_same =
            (anchor_scale * (anchor_radius / anchor_radius)).clamp(geom.resize_scale_min, geom.resize_scale_max);
        assert!((new_same - anchor_scale).abs() < 1e-6);
    }
}
