// SPDX-License-Identifier: MPL-2.0

//! FPS-overlay toggle apply_* helpers — renderer-only state
//! flips for the on-screen frame-rate readout. No document
//! mutation, no rebuild; both helpers only push to the renderer's
//! `fps_display_mode`.

use super::RebuildHost;

/// Toggle the FPS overlay between `Snapshot` and `Off`. Mirrors
/// `fps on` / `fps off`.
pub(in crate::application::app) fn apply_toggle_fps(host: &mut dyn RebuildHost) {
    use crate::application::common::FpsDisplayMode;
    let next = match host.fps_display_mode() {
        FpsDisplayMode::Snapshot => FpsDisplayMode::Off,
        _ => FpsDisplayMode::Snapshot,
    };
    host.set_fps_display(next);
}

/// Toggle the FPS overlay between `Debug` and `Off`. Mirrors
/// `fps debug` / `fps off`.
pub(in crate::application::app) fn apply_toggle_fps_debug(host: &mut dyn RebuildHost) {
    use crate::application::common::FpsDisplayMode;
    let next = match host.fps_display_mode() {
        FpsDisplayMode::Debug => FpsDisplayMode::Off,
        _ => FpsDisplayMode::Debug,
    };
    host.set_fps_display(next);
}
