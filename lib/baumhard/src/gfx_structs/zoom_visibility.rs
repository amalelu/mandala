//! `ZoomVisibility` — an optional lower/upper bound on the camera
//! zoom level at which a [`crate::gfx_structs::area::GlyphArea`] is
//! allowed to render. Orthogonal to the per-element font-size clamps
//! on connections / portals (which reshape *size* with zoom); this
//! primitive controls *presence* — is the element drawn at all at
//! the current zoom?
//!
//! Default is unbounded (both bounds `None`) — any existing
//! `GlyphArea` with no opinion keeps its historical always-visible
//! behaviour. When either bound is `Some`, the renderer's final
//! cull pass skips the element whenever `camera.zoom` falls outside
//! `[min, max]` (inclusive). This is the layering primitive that
//! lets map authors build Google-Maps-style zoom-dependent detail:
//! "only show this label when zoomed in past 1.5×", "only show this
//! landmark when zoomed out below 0.5×".
//!
//! The primitive lives at the Baumhard level so every downstream
//! target — nodes, edges, labels, portals, borders — gets the same
//! cull without each builder re-implementing a parallel check (per
//! `CODE_CONVENTIONS.md` §3 "everything is glyphs" — one filter at
//! the shared seam).

use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

/// Optional inclusive `[min, max]` window on camera zoom controlling
/// whether a [`crate::gfx_structs::area::GlyphArea`] renders. Each
/// bound is independently optional: `None` means "unbounded on that
/// side". The default `{ min: None, max: None }` renders at every
/// zoom — existing call sites that don't care pay nothing.
///
/// # Costs
///
/// [`ZoomVisibility::contains`] is two branchless float comparisons;
/// safe to call inside the render-loop filter on every frame without
/// measurable overhead (bench: `zoom_visibility_contains`). Clones
/// and hashes are `Copy`-cheap (no heap).
///
/// # Example
///
/// ```
/// use baumhard::gfx_structs::zoom_visibility::ZoomVisibility;
/// let only_zoomed_in = ZoomVisibility { min: Some(1.5), max: None };
/// assert!(only_zoomed_in.contains(2.0));
/// assert!(!only_zoomed_in.contains(1.0));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ZoomVisibility {
    /// Lower bound on `camera.zoom`. `None` = unbounded below; the
    /// element renders at arbitrarily small zoom levels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f32>,
    /// Upper bound on `camera.zoom`. `None` = unbounded above; the
    /// element renders at arbitrarily large zoom levels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f32>,
}

impl Default for ZoomVisibility {
    fn default() -> Self {
        ZoomVisibility { min: None, max: None }
    }
}

/// `Eq` is asserted manually because `f32` is only `PartialEq`. The
/// invariant — both bounds are finite when `Some` — holds for every
/// constructor in this codebase (model loaders, verifier, mutator
/// apply). A future caller stashing `f32::NAN` here would be a bug
/// at the construction site, not a soundness issue at this assert.
impl Eq for ZoomVisibility {}

impl Hash for ZoomVisibility {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // `f32` does not implement `Hash`; round-trip through bits
        // for stable hashing (mirrors the `OutlineStyle::px` pattern
        // in `area_fields.rs`).
        self.min.map(f32::to_bits).hash(state);
        self.max.map(f32::to_bits).hash(state);
    }
}

impl ZoomVisibility {
    /// Unbounded window — renders at every zoom. O(1); no heap.
    pub const fn unbounded() -> Self {
        ZoomVisibility { min: None, max: None }
    }

    /// Whether the default (unbounded) value is set. Used by
    /// `#[serde(skip_serializing_if = "ZoomVisibility::is_default")]`
    /// so existing fixtures roundtrip byte-identical (`CODE_CONVENTIONS.md`
    /// §10). O(1).
    pub const fn is_default(&self) -> bool {
        self.min.is_none() && self.max.is_none()
    }

    /// Inclusive containment check: `true` iff `zoom` falls inside
    /// `[min, max]` with `None` on either side treated as an open
    /// bound. Two chained compares on at most two `Option<f32>`
    /// values. Safe in the render-loop filter.
    ///
    /// `zoom.is_nan()` returns `false` — NaN compares as
    /// `false` against everything else, and accepting a NaN
    /// camera as "always render" would silently leak a bug at
    /// the camera level into the visible frame. A well-formed
    /// `camera.zoom` is always finite
    /// ([`crate::gfx_structs::camera::Camera2D`] clamps it), so
    /// this guard only fires when something upstream is already
    /// broken; keeping the element culled surfaces the bug
    /// instead of hiding it.
    #[inline]
    pub fn contains(&self, zoom: f32) -> bool {
        if zoom.is_nan() {
            return false;
        }
        if let Some(min) = self.min {
            if zoom < min {
                return false;
            }
        }
        if let Some(max) = self.max {
            if zoom > max {
                return false;
            }
        }
        true
    }

    /// Build a window from a pair of optional bounds (the flat
    /// serde shape the mindmap model uses on
    /// [`crate::mindmap::model::MindNode`],
    /// [`crate::mindmap::model::MindEdge`], etc.). Does not
    /// validate the bounds — see [`ZoomVisibility::try_new`]
    /// for the invariant-enforcing constructor. O(1).
    pub const fn from_pair(min: Option<f32>, max: Option<f32>) -> Self {
        ZoomVisibility { min, max }
    }

    /// Invariant-enforcing constructor: returns `Some` when
    /// each `Some` bound is finite **and** `min <= max` whenever
    /// both are set; `None` otherwise. The only call site that
    /// should take the raw struct literal today is serde; every
    /// programmatic build — mutator payloads, plugin surfaces,
    /// future script APIs — should go through this so the
    /// always-invisible window case can't slip past the
    /// construction boundary (§B10 "prefer surfaces that
    /// compose"). O(1).
    ///
    /// The verifier (`maptool verify` / `verify::zoom_bounds`)
    /// enforces the same rules at load time for authored JSON;
    /// this constructor is its programmatic counterpart.
    pub fn try_new(min: Option<f32>, max: Option<f32>) -> Option<Self> {
        if let Some(m) = min {
            if !m.is_finite() {
                return None;
            }
        }
        if let Some(m) = max {
            if !m.is_finite() {
                return None;
            }
        }
        if let (Some(mn), Some(mx)) = (min, max) {
            if mn > mx {
                return None;
            }
        }
        Some(ZoomVisibility { min, max })
    }

    /// Replace-not-intersect cascade: if `override_pair` contains
    /// any `Some`, return a window from `override_pair` as-is;
    /// otherwise inherit `parent` unchanged. Matches the cascade
    /// posture the portal font-clamp resolver already uses for
    /// `PortalEndpointState.text_{min,max}_font_size_pt`. O(1).
    ///
    /// "Replace" rather than "intersect" is the user-facing rule:
    /// setting only a `min` on a label means "override the edge's
    /// window with this single-sided one", not "narrow the
    /// existing window further". Intersection would silently
    /// inherit a bound the author didn't mention.
    pub const fn cascade_replace(
        parent: ZoomVisibility,
        override_min: Option<f32>,
        override_max: Option<f32>,
    ) -> Self {
        match (override_min, override_max) {
            (None, None) => parent,
            (min, max) => ZoomVisibility { min, max },
        }
    }
}
