use std::time::Duration;
// `web_time::Instant` is a drop-in for `std::time::Instant` that works
// on wasm32: native re-exports `std`, wasm maps to `performance.now()`.
// Without this swap `PollTimer::new` / `StopWatch::new_start` panic on
// wasm with "time not implemented on this platform".
use web_time::Instant;

#[derive(Copy, Clone, Eq, Hash, PartialEq)]
pub enum RedrawMode {
    OnRequest,
    FpsLimit(usize),
    NoLimit,
}

#[derive(Copy, Clone, Eq, Hash, PartialEq)]
pub enum InputMode {
    Direct,
    MappedToInstruction,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RenderDecree {
    Noop,
    DisplayFps(FpsDisplayMode),
    StartRender,
    StopRender,
    ReinitAdapter,
    SetSurfaceSize(u32, u32),
    Terminate,
    CameraPan(f32, f32),
    CameraZoom { screen_x: f32, screen_y: f32, factor: f32 },
}

/// Which FPS readout the renderer should display, if any.
/// `Snapshot` matches the legacy behavior: one frame's interval,
/// sampled every ~200 frames, held on screen in between.
/// `Debug` maintains a rolling average of the last ~200 frame
/// intervals and updates every frame for diagnostic use.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FpsDisplayMode {
    Off,
    Snapshot,
    Debug,
}

impl Default for RenderDecree {
    fn default() -> Self {
        RenderDecree::Noop
    }
}

// winit::KeyEvent does not derive copy, so we will create our own type that does
#[derive(Copy, Clone, Eq, PartialEq)]
pub enum KeyPress {
    Placeholder,
}

impl KeyPress {
    pub(crate) fn placeholder() -> Self {
        KeyPress::Placeholder
    }
}

#[derive(Copy, Clone)]
pub enum WindowMode {
    Fullscreen,
    WindowedFullscreen,
    Windowed { x: u32, y: u32 },
}

#[derive(Copy, Clone)]
pub struct StopWatch {
    start: Instant,
}

impl StopWatch {
    pub fn new_start() -> StopWatch {
        StopWatch {
            start: Instant::now(),
        }
    }

    pub fn stop(&self) -> Duration {
        Instant::now().duration_since(self.start)
    }
}

#[derive(Copy, Clone)]
pub struct PollTimer {
    instant: Instant,
    duration: Duration,
}

impl PollTimer {
    #[inline]
    pub fn new(duration: Duration) -> PollTimer {
        PollTimer {
            instant: Instant::now(),
            duration,
        }
    }

    #[inline]
    pub fn immediately() -> PollTimer {
        Self::new(Duration::from_millis(0))
    }

    pub fn is_expired(&self) -> bool {
        Instant::now()
            .duration_since(self.instant)
            .ge(&self.duration)
    }
    pub fn expire_in(&mut self, duration: Duration) {
        self.instant = Instant::now();
        self.duration = duration;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_stopwatch_measures_elapsed() {
        let watch = StopWatch::new_start();
        thread::sleep(Duration::from_millis(10));
        let elapsed = watch.stop();
        assert!(
            elapsed >= Duration::from_millis(5),
            "StopWatch should measure at least 5ms after sleeping 10ms; got {:?}",
            elapsed,
        );
    }

    #[test]
    fn test_poll_timer_immediately_is_expired() {
        let timer = PollTimer::immediately();
        assert!(timer.is_expired(), "PollTimer::immediately() should be expired right away");
    }

    #[test]
    fn test_poll_timer_far_future_not_expired() {
        let timer = PollTimer::new(Duration::from_secs(60));
        assert!(!timer.is_expired(), "PollTimer with 60s duration should not expire instantly");
    }

    #[test]
    fn test_poll_timer_expire_in_resets() {
        let mut timer = PollTimer::immediately();
        assert!(timer.is_expired());
        timer.expire_in(Duration::from_secs(60));
        assert!(!timer.is_expired(), "expire_in should reset the timer with a new duration");
    }

    #[test]
    fn test_render_decree_default_is_noop() {
        let decree: RenderDecree = RenderDecree::default();
        assert_eq!(decree, RenderDecree::Noop);
    }
}
