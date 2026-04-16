//! Modal state machine for the picker: the two modes (Contextual /
//! Standalone), the two mutually-exclusive drag gestures (Move /
//! Resize), the `ColorPickerState` enum that either of those live
//! on, and the small error-flash stub that app.rs calls when the
//! user attempts something that can't complete.

use super::glyph_tables::CROSSHAIR_CENTER_CELL;
use super::hit::PickerHit;
use super::layout::ColorPickerLayout;
use super::targets::PickerHandle;

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
        /// HSV triple being previewed on hover. `Some` while the
        /// cursor is over a hue/sat/val cell; `None` otherwise. The
        /// rendering pipeline uses this for visual feedback; the real
        /// `hue_deg`/`sat`/`val` only change on click or keyboard
        /// nudge.
        hover_preview: Option<(f32, f32, f32)>,
        /// Pending error-flash animation request. Set when the user
        /// attempts an action that can't complete (e.g. clicking the
        /// center commit button in Standalone mode with no
        /// selection). Today this is a plain boolean stub — the
        /// animation system isn't wired yet, so the renderer ignores
        /// it. When that lands, this becomes the hook point: flip it
        /// on, the renderer reads and clears it, the tree gets a
        /// red-tint mutation for ~250 ms. See [`request_error_flash`].
        pending_error_flash: bool,
        /// `(hue_deg, sat, val, hovered_hit)` of the last
        /// dynamic-phase apply. On layout-stable frames, the
        /// dispatcher compares the current geometry against this
        /// snapshot and skips `apply_color_picker_overlay_dynamic_mutator`
        /// when nothing observable has changed (a mouse move within
        /// the same cell is the common case). `None` between picker
        /// open and the first dynamic apply. Reset (`None`) after
        /// every layout-phase apply because those rewrite the full
        /// cell set and the next dynamic apply should target the
        /// freshly-written state.
        last_dynamic_apply: Option<PickerDynamicApplyKey>,
    },
}

/// Snapshot of the picker state the dynamic phase writes against.
/// Floats are compared bit-exactly (`f32::to_bits()`) so the
/// short-circuit treats two identical geometries as equal even in
/// the pathological NaN case — the op is an optimization, not a
/// correctness boundary, but bit-equality is what we want.
#[derive(Debug, Clone, Copy)]
pub struct PickerDynamicApplyKey {
    pub hue_deg: f32,
    pub sat: f32,
    pub val: f32,
    pub hovered_hit: Option<PickerHit>,
    /// Hex visibility flips change the hex cell's text + regions
    /// without changing the layout, so it's part of the
    /// dynamic-apply key.
    pub hex_visible: bool,
}

impl PartialEq for PickerDynamicApplyKey {
    fn eq(&self, other: &Self) -> bool {
        self.hue_deg.to_bits() == other.hue_deg.to_bits()
            && self.sat.to_bits() == other.sat.to_bits()
            && self.val.to_bits() == other.val.to_bits()
            && self.hovered_hit == other.hovered_hit
            && self.hex_visible == other.hex_visible
    }
}
impl Eq for PickerDynamicApplyKey {}

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
