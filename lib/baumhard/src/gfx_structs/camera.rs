// SPDX-License-Identifier: MPL-2.0

//! 2D pan/zoom camera — the projection between canvas space
//! (where mindmap nodes live) and screen space (pixel coordinates).
//! Single `Camera2D` struct plus its `canvas_to_screen` /
//! `screen_to_canvas` helpers; no scene graph, no matrix stack.
//! The renderer uses this to compute its orthographic view matrix;
//! every hit-test path uses `screen_to_canvas` to land a pointer
//! event on the right glyph.

use glam::Vec2;

/// A 2D camera for navigating large canvas spaces with pan and zoom.
///
/// Coordinates:
/// - Canvas space: absolute positions of mindmap nodes (can be negative)
/// - Screen space: pixel positions on the window (0,0 at top-left)
///
/// The camera's `position` is the canvas coordinate at the center of the viewport.
pub struct Camera2D {
    /// Canvas coordinate at the center of the viewport
    pub position: Vec2,
    /// Zoom factor (1.0 = no zoom, >1.0 = zoomed in, <1.0 = zoomed out)
    pub zoom: f32,
    /// Viewport dimensions in screen pixels
    pub viewport_size: (u32, u32),
}

/// A single camera-state delta — the mutation vocabulary for
/// pan / zoom inputs, theme camera-fits, and the Phase-4
/// animation timeline. Built so input handlers, programmatic
/// fits, and tween instances all funnel through one
/// [`Camera2D::apply_mutation`] entry point — the same shape
/// every other mutation in the codebase respects.
///
/// Variants are *intent*, not raw field writes:
/// - `Pan` shifts the canvas-center by a screen-pixel delta,
///   accounting for current zoom (mouse-drag pan).
/// - `ZoomAt` scales by `factor` while pinning `screen_focus`
///   (cursor-anchored wheel zoom).
/// - `ZoomCenter` scales by `factor` around the viewport
///   center (keyboard zoom, fit-to-bounds tween).
/// - `SetPosition` / `SetZoom` are idempotent assignments,
///   used by fit-to-bounds and animation snapshots.
///
/// Variants compose: an animation timeline emits one
/// `CameraMutation` per tick, and the receiver applies it
/// without caring whether it came from input, a tween, or a
/// scripted gesture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CameraMutation {
    /// Pan by a delta in screen pixels (drag gesture). Mouse-
    /// right means view-right means camera-position-left.
    Pan { screen_delta: Vec2 },
    /// Multiplicative zoom while pinning `screen_focus`. `factor
    /// > 1.0` zooms in, `< 1.0` zooms out. Clamped to
    /// `Camera2D::MIN_ZOOM..=MAX_ZOOM`.
    ZoomAt { screen_focus: Vec2, factor: f32 },
    /// Multiplicative zoom around the viewport centre. Same
    /// clamp as `ZoomAt`.
    ZoomCenter { factor: f32 },
    /// Idempotent assignment of the canvas-space camera centre.
    /// Used by fit-to-bounds and animation snapshots; bypasses
    /// the relative-delta math.
    SetPosition { canvas_pos: Vec2 },
    /// Idempotent assignment of the zoom factor. Clamped to
    /// `MIN_ZOOM..=MAX_ZOOM`.
    SetZoom { factor: f32 },
    /// Recentre and rescale the camera so the canvas-space AABB
    /// `[min, max]` fits inside the viewport, with
    /// `padding_fraction` of the viewport held back as margin on
    /// each side (0.05 = 5% padding). Used by the load-a-mindmap
    /// path and any fit-to-selection gesture. Equivalent to the
    /// existing [`Camera2D::fit_to_bounds`] imperative call —
    /// expressed as a mutation so every camera change has one
    /// dispatch point.
    FitToBounds {
        min: Vec2,
        max: Vec2,
        padding_fraction: f32,
    },
}

impl Camera2D {
    /// Lowest zoom factor allowed; any mutation clamps against this.
    pub const MIN_ZOOM: f32 = 0.05;
    /// Highest zoom factor allowed; any mutation clamps against this.
    pub const MAX_ZOOM: f32 = 5.0;

    /// Construct a camera centred on canvas origin with zoom 1.0.
    /// O(1), no allocation.
    pub fn new(viewport_width: u32, viewport_height: u32) -> Self {
        Camera2D {
            position: Vec2::ZERO,
            zoom: 1.0,
            viewport_size: (viewport_width, viewport_height),
        }
    }

    /// Convert a canvas-space position to screen-space pixels.
    #[inline]
    pub fn canvas_to_screen(&self, canvas_pos: Vec2) -> Vec2 {
        let screen_center = Vec2::new(
            self.viewport_size.0 as f32 / 2.0,
            self.viewport_size.1 as f32 / 2.0,
        );
        (canvas_pos - self.position) * self.zoom + screen_center
    }

    /// Convert a screen-space pixel position to canvas-space.
    #[inline]
    pub fn screen_to_canvas(&self, screen_pos: Vec2) -> Vec2 {
        let screen_center = Vec2::new(
            self.viewport_size.0 as f32 / 2.0,
            self.viewport_size.1 as f32 / 2.0,
        );
        (screen_pos - screen_center) / self.zoom + self.position
    }

    /// Pan by a delta in screen pixels.
    #[inline]
    pub fn pan(&mut self, screen_delta: Vec2) {
        // Moving the mouse right should move the view right,
        // which means the camera position moves left in canvas space
        self.position -= screen_delta / self.zoom;
    }

    /// Zoom centered on a screen-space point (e.g., the cursor).
    /// `factor` > 1.0 zooms in, < 1.0 zooms out.
    pub fn zoom_at(&mut self, screen_focus: Vec2, factor: f32) {
        let canvas_focus = self.screen_to_canvas(screen_focus);

        self.zoom = (self.zoom * factor).clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);

        // After zoom, adjust position so the canvas point under the cursor stays put
        let screen_center = Vec2::new(
            self.viewport_size.0 as f32 / 2.0,
            self.viewport_size.1 as f32 / 2.0,
        );
        self.position = canvas_focus - (screen_focus - screen_center) / self.zoom;
    }

    /// Zoom centered on the viewport center.
    pub fn zoom_center(&mut self, factor: f32) {
        let center = Vec2::new(
            self.viewport_size.0 as f32 / 2.0,
            self.viewport_size.1 as f32 / 2.0,
        );
        self.zoom_at(center, factor);
    }

    /// Update viewport dimensions (e.g., on window resize).
    pub fn set_viewport_size(&mut self, width: u32, height: u32) {
        self.viewport_size = (width, height);
    }

    /// Fit the camera to show a bounding box defined by min/max canvas coordinates.
    /// Adds padding as a fraction of the viewport (e.g., 0.1 = 10% padding on each side).
    pub fn fit_to_bounds(&mut self, min: Vec2, max: Vec2, padding_fraction: f32) {
        let canvas_size = max - min;
        let canvas_center = (min + max) / 2.0;
        self.position = canvas_center;

        let usable_width = self.viewport_size.0 as f32 * (1.0 - 2.0 * padding_fraction);
        let usable_height = self.viewport_size.1 as f32 * (1.0 - 2.0 * padding_fraction);

        if canvas_size.x > 0.0 && canvas_size.y > 0.0 {
            let zoom_x = usable_width / canvas_size.x;
            let zoom_y = usable_height / canvas_size.y;
            self.zoom = zoom_x.min(zoom_y).clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        }
    }

    /// Apply one [`CameraMutation`]. Single dispatch point so
    /// every camera mover — input handlers, fit-to-bounds, and
    /// the Phase-4 animation timeline — funnels through the
    /// same path. Each variant maps to one of the existing
    /// imperative helpers (`pan`, `zoom_at`, `zoom_center`) or
    /// a clamped field assignment.
    pub fn apply_mutation(&mut self, mutation: &CameraMutation) {
        match *mutation {
            CameraMutation::Pan { screen_delta } => self.pan(screen_delta),
            CameraMutation::ZoomAt { screen_focus, factor } => self.zoom_at(screen_focus, factor),
            CameraMutation::ZoomCenter { factor } => self.zoom_center(factor),
            CameraMutation::SetPosition { canvas_pos } => {
                self.position = canvas_pos;
            }
            CameraMutation::SetZoom { factor } => {
                self.zoom = factor.clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
            }
            CameraMutation::FitToBounds {
                min,
                max,
                padding_fraction,
            } => self.fit_to_bounds(min, max, padding_fraction),
        }
    }

    /// Check if a canvas-space axis-aligned rectangle is visible in the viewport.
    /// Used for culling off-screen nodes.
    #[inline]
    pub fn is_visible(&self, canvas_pos: Vec2, canvas_size: Vec2) -> bool {
        let screen_pos = self.canvas_to_screen(canvas_pos);
        let screen_size = canvas_size * self.zoom;

        let right = screen_pos.x + screen_size.x;
        let bottom = screen_pos.y + screen_size.y;

        right > 0.0
            && screen_pos.x < self.viewport_size.0 as f32
            && bottom > 0.0
            && screen_pos.y < self.viewport_size.1 as f32
    }
}
