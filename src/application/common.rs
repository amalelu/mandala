use std::time::{Duration, Instant};

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

#[derive(Clone, PartialEq)]
pub enum RenderDecree {
    Noop,
    ArenaUpdate,
    DisplayFps,
    StartRender,
    StopRender,
    ReinitAdapter,
    SetSurfaceSize(u32, u32),
    Terminate,
    CameraPan(f32, f32),
    CameraZoom { screen_x: f32, screen_y: f32, factor: f32 },
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
