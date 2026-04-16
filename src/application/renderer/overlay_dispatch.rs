//! Overlay rebuild / mutator dispatchers — the four `Renderer` entry
//! points that bridge the app's `AppScene` overlay slots (console,
//! color picker) to the renderer's buffer pipeline.
//!
//! Each method either builds a fresh overlay tree (full rebuild) or
//! applies a `MutatorTree` delta in place (§B2 mutator path), then
//! calls `rebuild_overlay_scene_buffers` to refresh the shaped
//! buffers. The choice is dispatched by structural-signature
//! equality — see [`crate::application::scene_host::OverlayDispatch`].

use baumhard::font::fonts;

use super::color_picker;
use super::console_geometry::{compute_console_frame_layout, ConsoleOverlayGeometry};
use super::console_pass::{
    build_console_overlay_mutator, build_console_overlay_tree, console_overlay_signature,
};
use super::Renderer;

impl Renderer {
    /// Rebuild the console overlay buffers. When `geometry` is
    /// `None`, the console is closed — clear the buffer list and the
    /// backdrop, and return. When `Some`, lay out a bottom-anchored
    /// glyph-rendered strip: sacred border, scrollback region,
    /// optional completion popup, and the prompt line with cursor.
    ///
    /// Everything is positioned in screen coordinates (the render
    /// pass draws `console_overlay_buffers` with `scale = 1.0`), so
    /// the console stays a fixed size regardless of canvas zoom.
    pub fn rebuild_console_overlay_buffers(
        &mut self,
        app_scene: &mut crate::application::scene_host::AppScene,
        geometry: Option<&ConsoleOverlayGeometry>,
    ) {
        use crate::application::scene_host::{OverlayDispatch, OverlayRole};

        let Some(geometry) = geometry else {
            // Closed: drop the backdrop, drop the tree, refresh
            // overlay buffers so the console disappears. The
            // structural-signature cache lives on `AppScene` and
            // is cleared inside `unregister_overlay`, so the next
            // reopen starts from a clean slate.
            self.console_backdrop = None;
            app_scene.unregister_overlay(OverlayRole::Console);
            self.rebuild_overlay_scene_buffers(app_scene);
            return;
        };

        let layout = compute_console_frame_layout(
            geometry,
            self.config.width as f32,
            self.config.height as f32,
        );
        self.console_backdrop = Some(layout.backdrop_rect());
        let signature = console_overlay_signature(&layout);

        // §B2 dispatch: if the structural signature
        // (`scrollback_rows` × `completion_rows`) hasn't changed
        // since the last build, the existing tree's slot count
        // still matches and we apply an in-place mutator that
        // overwrites every slot's variable fields. Window resize
        // is the only typical event that shifts the signature, so
        // the mutator path covers every keystroke / scrollback-
        // grow / completion-update / Tab-cycle frame.
        match app_scene.overlay_dispatch(OverlayRole::Console, signature) {
            OverlayDispatch::InPlaceMutator => {
                let mutator = {
                    let mut font_system = fonts::FONT_SYSTEM
                        .write()
                        .expect("Failed to acquire font_system lock");
                    build_console_overlay_mutator(geometry, &layout, &mut font_system)
                };
                app_scene.apply_overlay_mutator(OverlayRole::Console, &mutator);
            }
            OverlayDispatch::FullRebuild => {
                // Build the tree under the FONT_SYSTEM lock — we
                // need it for `measure_max_glyph_advance` only.
                // Tree construction itself doesn't shape; that
                // happens during the overlay-scene walk below.
                let tree = {
                    let mut font_system = fonts::FONT_SYSTEM
                        .write()
                        .expect("Failed to acquire font_system lock");
                    build_console_overlay_tree(geometry, &layout, &mut font_system)
                };
                app_scene.register_overlay(OverlayRole::Console, tree, glam::Vec2::ZERO);
                app_scene.set_overlay_signature(OverlayRole::Console, signature);
            }
        }
        self.rebuild_overlay_scene_buffers(app_scene);
    }


    /// Build the picker's overlay tree from `geometry`, register
    /// it under [`OverlayRole::ColorPicker`](crate::application::scene_host::OverlayRole),
    /// and walk the overlay sub-scene into
    /// `overlay_scene_buffers`. `None` means the picker is closed
    /// — drops the backdrop, unregisters the tree, refreshes
    /// overlay buffers so it disappears.
    ///
    /// Called by `open_color_picker`, the `Resized` handler, and
    /// the hover / chip-focus / commit / cancel paths in
    /// `app::rebuild_color_picker_overlay`.
    ///
    /// **Performance note**: every invocation re-shapes every
    /// glyph in the picker (~64 cells). The legacy split that
    /// skipped re-shaping the static hue ring on hover is gone;
    /// the planned `MutatorTree`-based hover path will mutate
    /// only changed cell colors and the indicator's position
    /// per §B1 of `lib/baumhard/CONVENTIONS.md`.
    pub fn rebuild_color_picker_overlay_buffers(
        &mut self,
        app_scene: &mut crate::application::scene_host::AppScene,
        geometry_and_layout: Option<(
            &crate::application::color_picker::ColorPickerOverlayGeometry,
            &crate::application::color_picker::ColorPickerLayout,
        )>,
    ) {
        self.color_picker_backdrop =
            color_picker::prepare_overlay_for_rebuild(app_scene, geometry_and_layout);
        self.rebuild_overlay_scene_buffers(app_scene);
    }

    /// §B2 mutation path — apply the **layout-phase** delta to the
    /// picker overlay tree without rebuilding the arena. Pairs with
    /// [`crate::application::color_picker_overlay::build_mutator`]:
    /// every variable field on every picker GlyphArea is overwritten
    /// via an `Assign` `DeltaGlyphArea` keyed by stable channel.
    ///
    /// Use this only when something the layout depends on actually
    /// changed (viewport resize, RMB size_scale drag, drag-move
    /// repositioning the wheel). Per-frame hover/HSV/chip updates
    /// should call [`Self::apply_color_picker_overlay_dynamic_mutator`]
    /// instead — same arena, slimmer per-cell delta. Open / close
    /// still use [`Self::rebuild_color_picker_overlay_buffers`]
    /// because the arena needs to be created or torn down. Calls
    /// `rebuild_overlay_scene_buffers` afterward to refresh the
    /// shaped buffers — the cosmic-text shape pass is still per-
    /// element, which is the §B1 perf gap tracked in `ROADMAP.md`.
    pub fn apply_color_picker_overlay_mutator(
        &mut self,
        app_scene: &mut crate::application::scene_host::AppScene,
        geometry: &crate::application::color_picker::ColorPickerOverlayGeometry,
        layout: &crate::application::color_picker::ColorPickerLayout,
    ) {
        color_picker::apply_layout_mutator(app_scene, geometry, layout);
        self.rebuild_overlay_scene_buffers(app_scene);
    }

    /// §B2 mutation path — apply the **dynamic-phase** delta to the
    /// picker overlay tree. Pairs with
    /// [`crate::application::color_picker_overlay::build_dynamic_mutator`]:
    /// only the per-frame fields (color regions, hover scale, hex
    /// text) are written; layout-phase fields stay as the previous
    /// layout-mutator wrote them.
    ///
    /// This is the per-frame hot path for hover / HSV / chip-focus
    /// updates — the picker's element set, position, and bounds are
    /// unchanged. Calls `rebuild_overlay_scene_buffers` afterward to
    /// refresh the shaped buffers.
    pub fn apply_color_picker_overlay_dynamic_mutator(
        &mut self,
        app_scene: &mut crate::application::scene_host::AppScene,
        geometry: &crate::application::color_picker::ColorPickerOverlayGeometry,
        layout: &crate::application::color_picker::ColorPickerLayout,
    ) {
        color_picker::apply_dynamic_mutator(app_scene, geometry, layout);
        self.rebuild_overlay_scene_buffers(app_scene);
    }
}
