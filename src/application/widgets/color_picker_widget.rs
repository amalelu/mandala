//! JSON-backed spec for the color picker widget.
//!
//! Loaded once via `include_str!` + `OnceLock` — so the JSON ships
//! inside the binary (works on native AND WASM — `fs::read` is not
//! an option in a browser) and parsing amortizes across every
//! picker-open after the first.
//!
//! The Rust side exposes getters that return `&'static [...]` slices
//! so every call-site in the renderer / app / tests can read glyphs
//! and geometry without knowing a JSON even exists. That separation
//! keeps the hot render path free of serde.

use std::sync::OnceLock;

use baumhard::font::fonts::AppFont;
use serde::Deserialize;

use crate::application::mutator_builder::MutatorNode;

/// Top-level spec — one file describes the whole widget. All
/// `Vec<String>` fields are read as slices by the renderer and never
/// mutated after load; the heap allocation happens exactly once per
/// process.
#[derive(Debug, Clone, Deserialize)]
pub struct ColorPickerWidgetSpec {
    pub geometry: GeometrySpec,
    /// 24 glyphs, one per hue-ring slot, clockwise from 12 o'clock.
    pub hue_ring_glyphs: Vec<String>,
    /// 10 glyphs for the val-bar top arm (brightest → mid).
    pub arm_top_glyphs: Vec<String>,
    /// 10 glyphs for the val-bar bottom arm (mid → darkest).
    /// Typically Egyptian hieroglyphs; `arm_bottom_font` pins the
    /// exact face cosmic-text should shape them with.
    pub arm_bottom_glyphs: Vec<String>,
    /// 10 glyphs for the sat-bar left arm (desaturated → mid).
    pub arm_left_glyphs: Vec<String>,
    /// 10 glyphs for the sat-bar right arm (mid → saturated).
    pub arm_right_glyphs: Vec<String>,
    /// The central glyph on the wheel. Doubles as the commit button.
    pub center_preview_glyph: String,
    /// Explicit font family for `arm_bottom_glyphs`. Needed because
    /// cosmic-text's default fallback doesn't reliably pick a
    /// covering font for SMP-range codepoints like Egyptian
    /// hieroglyphs. `None` to let cosmic-text pick.
    #[serde(default)]
    pub arm_bottom_font: Option<AppFont>,
    /// Title template for contextual mode. `{target_label}` is
    /// replaced by "edge" / "portal" / "node" at render time.
    pub title_template_contextual: String,
    /// Title shown verbatim when the picker is in standalone mode.
    pub title_template_standalone: String,
    /// Hint footer text shown inside the backdrop when the picker
    /// is in contextual mode. Includes the "Esc cancel" affordance
    /// since Esc exits a contextual picker.
    pub hint_text_contextual: String,
    /// Hint footer text shown inside the backdrop when the picker
    /// is in standalone mode. Omits "Esc cancel" — Esc has no
    /// effect on a standalone picker, which only closes via
    /// `color picker off` from the console.
    pub hint_text_standalone: String,
    /// Declarative mutator-tree shape for the picker overlay's
    /// **layout phase** — full per-cell field set, applied on layout
    /// changes only (initial open, viewport resize, RMB size_scale
    /// drag). Walked by `crate::application::mutator_builder` at
    /// apply time to produce a `MutatorTree<GfxMutator>` covering
    /// every variable field on every cell. See the JSON file's
    /// `_mutator_spec_comment` for the on-disk contract.
    pub mutator_spec: MutatorNode,
    /// Declarative mutator-tree shape for the picker overlay's
    /// **dynamic phase** — slim per-section field lists, applied on
    /// every hover / HSV / drag frame. Shares the same channel layout
    /// as `mutator_spec`, but each cell only emits the fields that
    /// genuinely change between frames (color, hover scale, hex text).
    /// Pairs with the layout phase: layout writes static fields once,
    /// dynamic writes per-frame fields on top. See the JSON file's
    /// `_dynamic_mutator_spec_comment` for the on-disk contract.
    pub dynamic_mutator_spec: MutatorNode,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GeometrySpec {
    /// Target wheel-diameter fraction of the screen's shorter side
    /// at `size_scale = 1.0`. The layout fn back-solves
    /// `font_size = target_frac * min(screen_w, screen_h) * size_scale
    /// / wheel_side_in_fonts`, then clamps to `[font_min, font_max]`.
    /// 0.38 keeps the picker comfortably under half the short axis at
    /// every screen size while still big enough to aim glyphs with a
    /// mouse on a 1080p desktop.
    pub target_frac: f32,
    /// Floor for the derived font_size in pixels — below this glyphs
    /// stop being recognizable. Drives the picker's lower-bound size
    /// on tiny phone-class viewports.
    pub font_min: f32,
    /// Ceiling for the derived font_size in pixels — above this the
    /// picker stops feeling like a widget. Drives the upper-bound
    /// size on huge desktop viewports.
    pub font_max: f32,
    /// Ring font size as a multiple of `font_size`.
    pub hue_ring_font_scale: f32,
    /// Cell font size (crosshair arm glyphs) as a multiple of
    /// `font_size`. Bumped above 1.0 so the cross reads as the
    /// focal interactive surface rather than a secondary decoration.
    pub cell_font_scale: f32,
    /// Ring box width as a multiple of `ring_font_size`.
    pub ring_box_scale: f32,
    /// Cell box width as a multiple of `cell_advance`.
    pub cell_box_scale: f32,
    /// Preview glyph size as a multiple of `font_size`.
    pub preview_size_scale: f32,
    /// Bar-tip to ring-edge padding as a multiple of
    /// `ring_font_size`.
    pub bar_to_ring_padding_scale: f32,
    /// Minimum clearance between the centre preview glyph and the
    /// nearest arm cell, expressed as a multiple of `font_size`.
    /// `compute_color_picker_layout` floors `cell_advance` at
    /// `preview_size / 2 + font_size * bar_to_preview_padding_scale`
    /// so the preview's bounding radius never covers cell[9] /
    /// cell[11]. Without this floor, a `preview_size_scale` much
    /// larger than the per-script cell-advance ratio (3.0 vs ~1.0 in
    /// the default spec) produces visible overlap between the ࿕ and
    /// the first arm glyph in each direction.
    pub bar_to_preview_padding_scale: f32,
    /// Font+bounds multiplier applied to the hovered cell. 1.3×
    /// reads as "this one's hot" without pushing into neighbors.
    pub hover_scale: f32,
    /// Outline thickness in pixels at the spec's `font_max` baseline.
    /// Scales linearly with the picker's current font_size so small
    /// pickers get proportionally smaller outlines. Each picker
    /// glyph stamps 8 black copies of itself (cardinals + diagonals,
    /// canonical in baumhard's `OutlineStyle::offsets`) at this
    /// radius, producing a continuous border as long as the value
    /// stays at or below the glyph's stroke width.
    pub outline_px: f32,
    /// When `true`, the picker draws no backdrop fill — canvas
    /// content shows through the gaps between glyphs. Combined with
    /// the glyph halos this makes the picker read as a floating
    /// widget rather than a heavy modal frame.
    pub transparent_backdrop: bool,
    /// Lower clamp for the user-controlled `size_scale` — the RMB
    /// drag gesture can't shrink the picker below this multiple of
    /// the default size.
    pub resize_scale_min: f32,
    /// Upper clamp for the user-controlled `size_scale` — same idea
    /// as `resize_scale_min` but for growth.
    pub resize_scale_max: f32,
}

/// Parsed spec, cached for the life of the process.
static SPEC: OnceLock<ColorPickerWidgetSpec> = OnceLock::new();

/// Return a reference to the parsed spec. First call parses the
/// embedded JSON; subsequent calls hand out the cached handle.
/// Panics only if the JSON is malformed — treated as a build-time
/// contract, not a runtime failure, since the JSON ships inside the
/// binary and is covered by the `spec_loads` test below.
pub fn load_spec() -> &'static ColorPickerWidgetSpec {
    SPEC.get_or_init(|| {
        static SOURCE: &str = include_str!("color_picker.json");
        serde_json::from_str(SOURCE).expect("color_picker.json is malformed")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse-smoke: the embedded JSON must load without panic and
    /// populate every required field. A caught-at-build-time guard
    /// against accidental JSON edits that break the schema.
    #[test]
    fn spec_loads() {
        let spec = load_spec();
        assert_eq!(spec.hue_ring_glyphs.len(), 24);
        assert_eq!(spec.arm_top_glyphs.len(), 8);
        assert_eq!(spec.arm_bottom_glyphs.len(), 8);
        assert_eq!(spec.arm_left_glyphs.len(), 8);
        assert_eq!(spec.arm_right_glyphs.len(), 8);
        assert!(!spec.center_preview_glyph.is_empty());
        assert!(spec.geometry.target_frac > 0.0 && spec.geometry.target_frac < 1.0);
        assert!(spec.geometry.font_min > 0.0);
        assert!(spec.geometry.font_max > spec.geometry.font_min);
        assert!(spec.geometry.hover_scale > 1.0);
        assert!(spec.geometry.cell_font_scale >= 1.0);
        assert!(spec.geometry.outline_px > 0.0);
        assert!(spec.geometry.resize_scale_min > 0.0);
        assert!(spec.geometry.resize_scale_max > spec.geometry.resize_scale_min);
    }

    /// The `mutator_spec` block must parse into a Void root with the
    /// five expected Repeat children (hue_ring, sat_bar, val_bar,
    /// preview, hex) in declaration order, the right channel bases,
    /// and the skip_indices for the crosshair bars. A drift guard —
    /// any JSON renumbering that sneaks past surfaces here rather
    /// than as a silent picker misalignment.
    #[test]
    fn spec_mutator_spec_shape_matches_expectations() {
        use crate::application::mutator_builder::{CountSrc, MutatorNode};
        let spec = load_spec();
        let MutatorNode::Void { channel, children } = &spec.mutator_spec else {
            panic!("mutator_spec root must be a Void");
        };
        assert_eq!(*channel, 0, "picker overlay root channel must be 0");
        let expected: &[(&str, usize, usize, &[usize])] = &[
            ("hue_ring", 100, 24, &[]),
            ("sat_bar", 300, 17, &[8]),
            ("val_bar", 400, 17, &[8]),
            ("preview", 500, 1, &[]),
            ("hex", 600, 1, &[]),
        ];
        assert_eq!(children.len(), expected.len());
        for (child, (exp_section, exp_base, exp_count, exp_skip)) in
            children.iter().zip(expected.iter())
        {
            let MutatorNode::Repeat {
                section,
                channel_base,
                count,
                skip_indices,
                ..
            } = child
            else {
                panic!("mutator_spec children must all be Repeat");
            };
            assert_eq!(section, exp_section);
            assert_eq!(*channel_base, *exp_base);
            match count {
                CountSrc::Literal(n) => assert_eq!(*n, *exp_count),
                _ => panic!("picker sections use literal counts"),
            }
            assert_eq!(skip_indices.as_slice(), *exp_skip);
        }
    }

    /// Total live picker cells = 24 hue + 16 sat + 16 val + 1 preview
    /// + 1 hex = 58. Pins the element-count contract the mutator path
    /// relies on.
    #[test]
    fn spec_mutator_spec_total_live_cells_is_58() {
        use crate::application::mutator_builder::{iter_section_channels, SectionContext};
        struct NoCtx;
        impl SectionContext for NoCtx {}
        let spec = load_spec();
        let mut out = Vec::new();
        iter_section_channels(&spec.mutator_spec, &NoCtx, &mut out);
        assert_eq!(out.len(), 58);
    }

    /// `dynamic_mutator_spec` must mirror `mutator_spec`'s channel
    /// layout exactly — same root, same Repeat sections, same channel
    /// bases / counts / skips. Only the per-section AreaDelta field
    /// list differs (slimmer, per-frame). A drift here means the
    /// dynamic mutator could re-channel a cell that the layout
    /// mutator already registered, silently misaligning the §B2
    /// in-place update.
    #[test]
    fn spec_dynamic_mutator_spec_channel_layout_mirrors_layout_spec() {
        use crate::application::mutator_builder::{iter_section_channels, SectionContext};
        struct NoCtx;
        impl SectionContext for NoCtx {}
        let spec = load_spec();
        let mut layout_out = Vec::new();
        iter_section_channels(&spec.mutator_spec, &NoCtx, &mut layout_out);
        let mut dynamic_out = Vec::new();
        iter_section_channels(&spec.dynamic_mutator_spec, &NoCtx, &mut dynamic_out);
        assert_eq!(
            layout_out, dynamic_out,
            "dynamic spec channel layout must match layout spec"
        );
    }

    /// Per-section dynamic field lists are deliberately slim — only
    /// fields that actually change between hover / HSV / drag frames.
    /// Pins the choice so a future "just add the field, it's harmless"
    /// edit surfaces here as a deliberate design trade rather than a
    /// silent perf regression on the picker hot path.
    #[test]
    fn spec_dynamic_mutator_spec_per_section_fields_are_slim() {
        use crate::application::mutator_builder::{CellField, MutationSrc, MutatorNode};
        let spec = load_spec();
        let MutatorNode::Void { children, .. } = &spec.dynamic_mutator_spec else {
            panic!("dynamic_mutator_spec root must be Void");
        };
        // (section, expected dynamic field count).
        // hue/sat/val/preview = color + scale (2 + Operation).
        // hex = text + color (2 + Operation).
        let expected: &[(&str, usize)] = &[
            ("hue_ring", 3),
            ("sat_bar", 3),
            ("val_bar", 3),
            ("preview", 3),
            ("hex", 3),
        ];
        assert_eq!(children.len(), expected.len());
        for (child, (exp_section, exp_field_count)) in children.iter().zip(expected.iter()) {
            let MutatorNode::Repeat {
                section, template, ..
            } = child
            else {
                panic!("dynamic_mutator_spec children must all be Repeat");
            };
            assert_eq!(section, exp_section);
            let MutatorNode::Single { mutation, .. } = template.as_ref() else {
                panic!("dynamic spec template must be Single");
            };
            let MutationSrc::AreaDelta(fields) = mutation else {
                panic!("dynamic spec mutation must be AreaDelta");
            };
            assert_eq!(
                fields.len(),
                *exp_field_count,
                "section {exp_section}: expected {exp_field_count} dynamic fields, got {}",
                fields.len(),
            );
            // Always last: Operation::Assign sentinel.
            assert!(
                matches!(fields.last(), Some(CellField::Operation(_))),
                "dynamic spec section {exp_section} must end with an Operation field"
            );
        }
    }

    /// The bottom-arm font must be set — without it cosmic-text
    /// falls back to a non-covering face and the hieroglyphs render
    /// as tofu. Explicit test rather than leaving the regression as
    /// a visual one.
    #[test]
    fn spec_pins_bottom_arm_font() {
        let spec = load_spec();
        assert!(
            matches!(
                spec.arm_bottom_font,
                Some(AppFont::NotoSansEgyptianHieroglyphsRegular)
            ),
            "arm_bottom_font must be set to the Egyptian hieroglyph face"
        );
    }
}
