// SPDX-License-Identifier: MPL-2.0

//! Small cross-cutting types shared between the event loop, the
//! renderer, and console verbs. Each type below carries its own
//! invariant; together they form the "configuration" surface the
//! event loop reads on every frame.

// `web_time` maps to `performance.now()` on wasm32; without this swap
// `Instant::now()` panics with "time not implemented on this platform".
#[cfg(not(target_arch = "wasm32"))]
use web_time::Instant;

/// Renderer-side command queue entry. Event loop pushes one
/// for each per-frame intent the renderer should react to; the
/// renderer drains them at frame start. Everything that
/// changes GPU state without changing document state goes
/// through here so the model/view boundary (§3) stays clean.
///
/// Variants:
/// - `Noop` — default sentinel; never actually queued by the
///   event loop, but `RenderDecree::default() = Noop` lets
///   builders compile.
/// - `SetFpsDisplay(mode)` — flip the on-screen FPS readout
///   between off / snapshot / debug. See [`FpsDisplayMode`].
/// - `StartRender` — open the per-frame draw gate. Both targets
///   send it once, from their init path, after the first scene
///   is built; the renderer draws nothing before it.
/// - `SetSurfaceSize(w, h)` — propagate a winit `Resized` to
///   the wgpu surface configuration.
/// - `Terminate` — release GPU resources before the event loop
///   exits.
/// - `CameraPan(dx, dy)` — translate the camera origin by a
///   per-cursor-move delta in canvas pixels (the §3 carve-out
///   for per-frame continuous-gesture state).
/// - `CameraZoom { screen_x, screen_y, factor }` — multiply
///   the camera zoom by `factor`, anchored at the given screen
///   coordinates so the point under the cursor stays put.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum RenderDecree {
    #[default]
    Noop,
    SetFpsDisplay(FpsDisplayMode),
    StartRender,
    SetSurfaceSize(u32, u32),
    // Native-driver-only: `run_native` sends it on window-close;
    // a browser tab has no equivalent teardown event.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Terminate,
    CameraPan(f32, f32),
    CameraZoom {
        screen_x: f32,
        screen_y: f32,
        factor: f32,
    },
}

/// Which FPS readout the renderer should display, if any.
///
/// - `Off` — no overlay; the default.
/// - `Snapshot` — single per-frame FPS number rendered in the
///   corner. Useful for casual monitoring.
/// - `Debug` — extra per-stage timing breakdown (event drain,
///   scene build, GPU submit). Heavier to render; gated behind
///   `Action::ToggleFpsDebug`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FpsDisplayMode {
    Off,
    Snapshot,
    Debug,
}

/// Cross-platform monotonic clock in milliseconds since first call.
/// Native uses `web_time::Instant` (which delegates to
/// `std::time::Instant`); WASM uses `performance.now()` (≥1ms
/// quantised, fine for the 400ms double-click window and the
/// animation-tick rate). Single source for click-time, animation-
/// tick-time, and trigger-binding "now" reads on both targets.
pub fn now_ms() -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::OnceLock;
        static EPOCH: OnceLock<Instant> = OnceLock::new();
        EPOCH.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
    }
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|w| w.performance())
            .map(|p| p.now())
            .unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_decree_default_is_noop() {
        let decree: RenderDecree = RenderDecree::default();
        assert_eq!(decree, RenderDecree::Noop);
    }
}
