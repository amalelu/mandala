// SPDX-License-Identifier: MPL-2.0

//! [`CanvasPerPixel`] — a camera's canvas-units-per-screen-pixel
//! ratio, and nothing else.
//!
//! **The module is the point.** A tuple struct's private field is
//! private to the module that *defines* it, not to its `impl`, so a
//! newtype declared beside its consumers is only as strong as the
//! discipline of the file it sits in — any function in that file can
//! still write `CanvasPerPixel(whatever)`. This module contains the
//! type, its three methods, and no consumer, so
//! [`CanvasPerPixel::of`] really is the only constructor a shipped
//! build can reach.
//!
//! That distinction was not academic. The type was first declared in
//! `pointer.rs` under a doc comment claiming the wrong value was "no
//! longer expressible"; a review planted `CanvasPerPixel(
//! EDGE_HIT_TOLERANCE_PX * …)` one screen below the declaration and
//! it compiled clean.

use crate::application::renderer::Renderer;

/// A camera's canvas-units-per-screen-pixel ratio.
///
/// A newtype rather than the `f32` it wraps, because the parameter it
/// occupies **changed meaning without changing type**. Before the
/// shared last rung existed, `handle_click_core`'s fourth argument was
/// `EDGE_HIT_TOLERANCE_PX` *already multiplied by* this ratio — a
/// distance in canvas units. It is now the ratio itself, and the
/// multiplication happens once, inside
/// [`edge_under_pointer`](super::edge_under_pointer).
///
/// Both spellings are `f32`, so passing the old one still compiled and
/// still type-checked, and the only symptom was every click on that
/// target getting an `EDGE_HIT_TOLERANCE_PX`-times-too-large grab
/// radius — silently, because the click tests probe a point far
/// outside the map rather than the boundary. A review planted exactly
/// that and the suite stayed green.
///
/// [`Self::of`] is the only constructor a shipped build can reach, and
/// this module's header is what makes that sentence true rather than
/// aspirational.
#[derive(Debug, Clone, Copy)]
pub(in crate::application::app) struct CanvasPerPixel(f32);

impl CanvasPerPixel {
    /// Read the ratio off the live camera. The only way production
    /// code makes one.
    ///
    /// The body is pinned as source text by
    /// `test_canvas_per_pixel_reads_the_camera_ratio_unscaled`,
    /// because it is the one place a scaling could be smuggled back in
    /// and no runtime test can reach it: checking it that way needs a
    /// live `Renderer`, which `TEST_CONVENTIONS §T8` keeps out of the
    /// harness. Reading the source needs neither.
    pub(in crate::application::app) fn of(renderer: &Renderer) -> Self {
        Self(renderer.canvas_per_pixel())
    }

    /// Test-only constructor, for the cases that pin the scaling
    /// itself and therefore have to name both sides of it.
    #[cfg(test)]
    pub(in crate::application::app) fn from_ratio(ratio: f32) -> Self {
        Self(ratio)
    }

    /// Convert a screen-pixel measurement to canvas units.
    pub(in crate::application::app) fn scale(self, screen_px: f32) -> f32 {
        screen_px * self.0
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use baumhard::util::rust_source::{braced_block_after, production_code};

    /// This module's own path, for the pin that reads it.
    const THIS_FILE: &str = "src/application/app/dispatch/cross_dispatch/canvas_per_pixel.rs";

    /// [`super::CanvasPerPixel::of`] reads the camera's ratio and
    /// applies no scaling of its own.
    ///
    /// A source-level pin, because it is the one place the scaling
    /// this newtype exists to keep out could be smuggled back in, and
    /// no runtime assertion can reach it: calling `of` needs a live
    /// `Renderer`, which `TEST_CONVENTIONS §T8` keeps out of the
    /// harness. Reading the source needs neither a renderer nor a GPU
    /// — the same mechanism
    /// `event_cursor_moved`'s
    /// `test_the_mouse_drag_arms_name_the_shared_pointer_threshold`
    /// uses for the same reason, and the same one
    /// `test_the_browser_canvas_keeps_touch_gestures_from_the_user_agent`
    /// uses over a file the compiler never reads at all.
    ///
    /// A previous revision of this crate claimed *no* test could catch
    /// it. That was false, and this is the test.
    ///
    /// Fails if `of` multiplies by anything, names a `_PX` constant,
    /// or stops reading `canvas_per_pixel()`.
    #[test]
    fn test_canvas_per_pixel_reads_the_camera_ratio_unscaled() {
        let code = production_code(THIS_FILE);
        let body = braced_block_after(&code, "fn of(renderer: &Renderer) -> Self {")
            .unwrap_or_else(|| panic!("{THIS_FILE} must still declare `of`"));
        let flat: String = body.split_whitespace().collect::<Vec<_>>().join(" ");

        assert_eq!(
            flat, "{ Self(renderer.canvas_per_pixel()) }",
            "`of` must forward the camera ratio and nothing else; a scaling here would \
             reinstate the exact hazard `CanvasPerPixel` exists to remove, at the one \
             site no runtime test can see"
        );
    }
}
