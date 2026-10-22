// SPDX-License-Identifier: MPL-2.0

//! Field-level delta types for `GlyphArea` — the vocabulary the mutation
//! pipeline uses to describe which field to touch and how.

use crate::core::primitives::{ApplyOperation, ColorFontRegions};
use crate::gfx_structs::delta::DeltaField;
use crate::gfx_structs::shape::NodeShape;
use crate::gfx_structs::zoom_visibility::ZoomVisibility;
use crate::util::ordered_vec2::OrderedVec2;
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::ops::Add;
use strum_macros::{Display, EnumDiscriminants, EnumIter};

/// Per-glyph halo style. When set on a [`crate::gfx_structs::area::GlyphArea`], the renderer
/// emits 8 extra shaped buffers behind the area's text — each at the
/// same metrics, family pinning, and alignment, but recolored to
/// `color` and positioned at the offsets yielded by
/// [`OutlineStyle::offsets`]. Used to keep colored glyphs legible
/// against arbitrary (or transparent) backgrounds where a per-pass
/// background fill is not on the table.
///
/// # Technique
///
/// We stamp the glyph 8 times — 4 cardinals at `(±px, 0)` / `(0, ±px)`
/// and 4 diagonals at `(±px/√2, ±px/√2)` — then draw the main glyph
/// on top. Every stamp sits on a circle of radius `px`, and adjacent
/// stamp centers are `~0.77·px` apart. Because each stamp is an
/// entire glyph, the stamps visually merge into a continuous outline
/// as long as `px` is no larger than the glyph's stroke width; a
/// halo wider than a stroke starts reading as ghost letter copies
/// rather than a border.
///
/// # Costs
///
/// Each outlined area costs 9 cosmic-text buffer shapings instead
/// of one. The stamp count is canonical (chosen inside this crate,
/// not a caller knob) so every consumer gets the same outline
/// quality without having to tune it. Hot-path work (§B7) — enable only when
/// the background legibility problem actually needs it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutlineStyle {
    /// RGBA halo color, applied to every glyph in every halo copy.
    pub color: [u8; 4],
    /// Halo thickness in screen-space pixels — the radius of the
    /// stamp circle, which sets the final outline thickness at 1:1.
    /// Keep it at or below the glyph's thinnest stroke for a
    /// continuous border; above that the stamps stop merging and the
    /// halo reads as ghost copies (see the type-level `# Technique`
    /// note). Picker scales this with its `font_size` so a shrunk
    /// widget gets a proportionally smaller halo; consumers without
    /// that need pass a fixed value.
    pub px: f32,
}

impl OutlineStyle {
    /// Yields the 8 stamp offsets (in pixels, relative to the main
    /// glyph's anchor) that the renderer must shape to produce the
    /// halo. Single source of truth for the outline technique — see
    /// the type-level `# Technique` note for the rationale.
    #[inline]
    pub fn offsets(&self) -> impl Iterator<Item = (f32, f32)> {
        // 4 cardinals + 4 diagonals, all at distance `px` from the
        // origin. `FRAC_1_SQRT_2` = 1/√2 ≈ 0.7071; the diagonals sit
        // on the same circle as the cardinals so the outline
        // thickness is uniform in every direction.
        let d = self.px * std::f32::consts::FRAC_1_SQRT_2;
        let p = self.px;
        [
            (p, 0.0),
            (-p, 0.0),
            (0.0, p),
            (0.0, -p),
            (d, d),
            (d, -d),
            (-d, d),
            (-d, -d),
        ]
        .into_iter()
    }
}

/// `Eq` is asserted manually because `f32` is only `PartialEq`. The
/// invariant — `OutlineStyle::px` is always finite — holds for every
/// constructor in this codebase, so reflexivity (`a == a`) is true.
/// If a future caller stores `f32::NAN` here that's a bug at the
/// construction site, not a soundness issue at this assert.
impl Eq for OutlineStyle {}

impl Hash for OutlineStyle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.color.hash(state);
        // `f32` does not implement `Hash`; round-trip through bits
        // for stable hashing (mirrors the `OrderedFloat` pattern
        // used elsewhere in this file).
        self.px.to_bits().hash(state);
    }
}

/// A single field delta for a [`crate::gfx_structs::area::GlyphArea`].
/// Each variant carries the new value (or addend, depending on the
/// active [`ApplyOperation`]) for one field of the area. Used inside
/// [`crate::gfx_structs::area::DeltaGlyphArea`] and the mutator
/// pipeline — the variant you pick determines which field is touched;
/// all others are left alone.
///
/// The payload-free `GlyphAreaFieldType` tag is *derived* from this
/// enum, so the two can never drift: adding a variant here adds its
/// tag, and the only other edit a new area mutation needs is its
/// branch in
/// [`GlyphArea::apply_operation`](crate::gfx_structs::area::GlyphArea::apply_operation)
/// (`CONVENTIONS.md` §B4).
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, EnumDiscriminants)]
#[strum_discriminants(name(GlyphAreaFieldType))]
#[strum_discriminants(derive(Hash, Serialize, Deserialize, EnumIter, Display))]
#[strum_discriminants(doc = "Payload-free tag for [`GlyphAreaField`], derived by strum so it")]
#[strum_discriminants(doc = "always lists exactly the field variants that exist. Keys the")]
#[strum_discriminants(doc = "[`DeltaGlyphArea`](crate::gfx_structs::area::DeltaGlyphArea) map,")]
#[strum_discriminants(doc = "where using the tag rather than the field avoids hashing the")]
#[strum_discriminants(doc = "payload (text, region sets) on every lookup.")]
pub enum GlyphAreaField {
    /// Replace or append to the area's text content. Under
    /// `ApplyOperation::Assign` the string replaces the current text;
    /// under `Add` it is concatenated; under `Delete` the text is
    /// cleared. `Subtract` and `Multiply` are not defined and are
    /// ignored with a `log::warn!`.
    Text(String),
    /// Font size in points. Under `Add`/`Subtract` the value is added
    /// to / subtracted from the current scale; under `Assign` it
    /// replaces outright.
    Scale(OrderedFloat<f32>),
    /// Vertical spacing multiplier. Arithmetic follows the same
    /// `Add`/`Subtract`/`Assign` contract as `Scale`.
    LineHeight(OrderedFloat<f32>),
    /// World-space position of the area's anchor. Under `Add` the
    /// vector is component-wise added (translation); under `Assign`
    /// it teleports.
    Position(OrderedVec2),
    /// Render bounds (width, height) in pixels. Under `Add` the
    /// components grow; under `Assign` the bounds are replaced.
    Bounds(OrderedVec2),
    /// Character-range color / font runs. Under `Add` each run in
    /// the delta is submitted (merged) into the existing set; under
    /// `Assign` the entire set is replaced; under `Subtract` matching
    /// runs are removed; under `Delete` the set is cleared.
    /// `Multiply` is not defined and is ignored with a `log::warn!`.
    ColorFontRegions(ColorFontRegions),
    /// Replace the area's
    /// [`outline`](crate::gfx_structs::area::GlyphArea#structfield.outline)
    /// field. `None` clears any previously-set halo; `Some(style)`
    /// enables one. Additive merge under `ApplyOperation::Add` is the
    /// rhs (a halo is either on or off; combining two halo styles
    /// isn't meaningful).
    Outline(Option<OutlineStyle>),
    /// Replace the area's
    /// [`shape`](crate::gfx_structs::area::GlyphArea#structfield.shape)
    /// field — the background / hit-test shape. Under
    /// `ApplyOperation::Assign` or `Add` the shape is overwritten
    /// (shapes don't compose arithmetically); under `Subtract` the
    /// shape is reset to [`NodeShape::Rectangle`] as a clean "remove
    /// the custom shape" signal. Additive merge under the `Add`
    /// impl on `GlyphAreaField` is the rhs for the same reason.
    Shape(NodeShape),
    /// Replace the area's
    /// [`zoom_visibility`](crate::gfx_structs::area::GlyphArea#structfield.zoom_visibility)
    /// field — the zoom window controlling whether the element
    /// renders at all. Under `ApplyOperation::Assign` or `Add` the
    /// window is overwritten (windows don't compose arithmetically,
    /// same posture as `Shape` / `Outline`); under `Subtract` the
    /// field is reset to
    /// [`ZoomVisibility::unbounded`](crate::gfx_structs::zoom_visibility::ZoomVisibility::unbounded)
    /// as the clean "remove the window" signal. Additive merge
    /// under the `Add` impl on `GlyphAreaField` is the rhs for the
    /// same reason.
    ZoomVisibility(ZoomVisibility),
    /// Override the arithmetic operation that governs how all sibling
    /// field deltas in the same
    /// [`DeltaGlyphArea`](crate::gfx_structs::area::DeltaGlyphArea)
    /// are applied. Does not modify the area itself — it is a control
    /// variant read by
    /// [`GlyphArea::apply_operation`](crate::gfx_structs::area::GlyphArea::apply_operation).
    Operation(ApplyOperation),
}

impl Add for GlyphAreaField {
    type Output = GlyphAreaField;

    fn add(self, rhs: Self) -> Self::Output {
        {
            match self {
                GlyphAreaField::Text(txt) => {
                    if let GlyphAreaField::Text(other) = rhs {
                        return GlyphAreaField::Text(txt + &other);
                    }
                }
                GlyphAreaField::Scale(scale) => {
                    if let GlyphAreaField::Scale(other) = rhs {
                        return GlyphAreaField::Scale(scale + other);
                    }
                }
                GlyphAreaField::Position(this) => {
                    if let GlyphAreaField::Position(other) = rhs {
                        return GlyphAreaField::Position(OrderedVec2::new(
                            this.x + other.x,
                            this.y + other.y,
                        ));
                    }
                }
                GlyphAreaField::Bounds(this) => {
                    if let GlyphAreaField::Bounds(other) = rhs {
                        return GlyphAreaField::Bounds(OrderedVec2::new(this.x + other.x, this.y + other.y));
                    }
                }
                GlyphAreaField::ColorFontRegions(regions) => {
                    if let GlyphAreaField::ColorFontRegions(other) = rhs {
                        let mut color_font_regions = ColorFontRegions::new_empty();
                        for region in regions.regions {
                            color_font_regions.submit_region(region);
                        }
                        for region in other.regions {
                            color_font_regions.submit_region(region);
                        }
                        return GlyphAreaField::ColorFontRegions(color_font_regions);
                    }
                }
                GlyphAreaField::LineHeight(height) => {
                    if let GlyphAreaField::LineHeight(other_height) = rhs {
                        return GlyphAreaField::LineHeight(height + other_height);
                    }
                }
                GlyphAreaField::Outline(_) => {
                    // Outline is on/off — combining two halo styles
                    // additively isn't meaningful (you can't have two
                    // halos at once). The rhs wins; that matches how
                    // a later mutation in a delta sequence overrides
                    // an earlier one for any single-value field.
                    if let GlyphAreaField::Outline(other) = rhs {
                        return GlyphAreaField::Outline(other);
                    }
                }
                GlyphAreaField::Shape(_) => {
                    // Shapes don't compose — "rectangle + ellipse"
                    // has no arithmetic meaning. The rhs wins, same
                    // as outline above.
                    if let GlyphAreaField::Shape(other) = rhs {
                        return GlyphAreaField::Shape(other);
                    }
                }
                GlyphAreaField::ZoomVisibility(_) => {
                    // Zoom windows don't compose additively —
                    // "zoom in only" + "zoom out only" has no
                    // meaningful arithmetic. The rhs wins, matching
                    // Outline / Shape above.
                    if let GlyphAreaField::ZoomVisibility(other) = rhs {
                        return GlyphAreaField::ZoomVisibility(other);
                    }
                }
                GlyphAreaField::Operation(_) => {
                    // Two `Operation` control variants merging is the
                    // *reachable* case, not a mismatch: `Delta::add`
                    // keys by discriminant tag, so the only pairs it
                    // ever hands this impl are same-variant ones. The
                    // arm used to fall through to the mismatch warn
                    // below and log on every legitimate merge of two
                    // deltas that both pin an operation. Last-wins,
                    // matching Outline / Shape / ZoomVisibility.
                    if let GlyphAreaField::Operation(other) = rhs {
                        return GlyphAreaField::Operation(other);
                    }
                }
            }
        }
        // Composing two fields of different variants is not a
        // meaningful additive operation. Mutator chains are reachable
        // from interactive paths (frame render, mutation drains), so
        // CODE_CONVENTIONS §7 forbids panicking here. Warn loudly so
        // the drift is visible in logs, then degrade by returning the
        // rhs unchanged — the same "later mutation wins" rule the
        // single-value-field branches above use for Outline.
        log::warn!(
            "GlyphAreaField::add called with mismatched variants; \
             discarding lhs and returning rhs"
        );
        rhs
    }
}

impl GlyphAreaField {
    /// Wrap an `f32` scale into a `Scale` field, converting through
    /// `OrderedFloat` so the enum remains `Eq`/`Hash`. O(1).
    pub fn scale(s: f32) -> Self {
        GlyphAreaField::Scale(OrderedFloat::from(s))
    }

    /// Wrap an `f32` into a `LineHeight` field. O(1).
    pub fn line_height(line_height: f32) -> Self {
        GlyphAreaField::LineHeight(OrderedFloat::from(line_height))
    }

    /// Construct a `Bounds` field from `(x, y)`. O(1).
    pub fn bounds(x: f32, y: f32) -> Self {
        GlyphAreaField::Bounds(OrderedVec2::new_f32(x, y))
    }

    /// Construct a `Position` field from `(x, y)`. O(1).
    pub fn position(x: f32, y: f32) -> Self {
        GlyphAreaField::Position(OrderedVec2::new_f32(x, y))
    }
}

impl DeltaField for GlyphAreaField {
    const OPERATION_KEY: GlyphAreaFieldType = GlyphAreaFieldType::Operation;

    #[inline]
    fn as_operation(&self) -> Option<ApplyOperation> {
        match self {
            GlyphAreaField::Operation(operation) => Some(*operation),
            _ => None,
        }
    }
}
