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
    /// Theme-variable quick-pick chips shown below the wheel.
    pub chips: Vec<ChipSpec>,
    /// Title template for contextual mode. `{target_label}` is
    /// replaced by "edge" / "portal" / "node" at render time.
    pub title_template_contextual: String,
    /// Title shown verbatim when the picker is in standalone mode.
    pub title_template_standalone: String,
    /// Hint footer text shown inside the backdrop.
    pub hint_text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GeometrySpec {
    /// Base font size in pixels, before the layout fn scales it to
    /// fit small windows.
    pub font_size: f32,
    /// Ring font size as a multiple of `font_size`.
    pub hue_ring_font_scale: f32,
    /// Ring box width as a multiple of `ring_font_size`.
    pub ring_box_scale: f32,
    /// Cell box width as a multiple of `cell_advance`.
    pub cell_box_scale: f32,
    /// Preview glyph size as a multiple of `font_size`.
    pub preview_size_scale: f32,
    /// Bar-tip to ring-edge padding as a multiple of
    /// `ring_font_size`.
    pub bar_to_ring_padding_scale: f32,
    /// Font+bounds multiplier applied to the hovered cell. 1.3×
    /// reads as "this one's hot" without pushing into neighbors.
    pub hover_scale: f32,
}

/// One chip in the theme-variable quick-pick row.
#[derive(Debug, Clone, Deserialize)]
pub struct ChipSpec {
    pub label: String,
    pub action: ChipActionSpec,
}

/// What a chip commits when clicked. Mirrors the runtime
/// [`crate::application::color_picker::ChipAction`] but carries
/// `String` instead of `&'static str` because it's loaded at
/// runtime.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChipActionSpec {
    /// Commit a raw `var(--name)` reference so theme-var resolution
    /// runs at render time. `name` carries the full `var(--foo)`
    /// literal including the parentheses.
    Var { name: String },
    /// Clear the target's color override (for edges), or re-seed to
    /// a per-axis default (for nodes / portals).
    Reset,
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
        assert_eq!(spec.arm_top_glyphs.len(), 10);
        assert_eq!(spec.arm_bottom_glyphs.len(), 10);
        assert_eq!(spec.arm_left_glyphs.len(), 10);
        assert_eq!(spec.arm_right_glyphs.len(), 10);
        assert!(!spec.center_preview_glyph.is_empty());
        assert_eq!(spec.chips.len(), 5);
        assert!(spec.geometry.font_size > 0.0);
        assert!(spec.geometry.hover_scale > 1.0);
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

    /// Chip specs must round-trip cleanly — one Var per theme
    /// variable plus one Reset at the end. Regression guard for
    /// schema drift in the JSON.
    #[test]
    fn spec_chip_shape() {
        let spec = load_spec();
        assert_eq!(spec.chips.len(), 5);
        // Last chip is Reset.
        assert!(matches!(
            spec.chips.last().unwrap().action,
            ChipActionSpec::Reset
        ));
        // First four are Var.
        for c in &spec.chips[..4] {
            assert!(matches!(c.action, ChipActionSpec::Var { .. }));
        }
    }
}
