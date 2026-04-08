#![allow(dead_code)]

use crate::application::app::{Application, Options};
use crate::application::common::{InputMode, WindowMode};
use log::{debug, error, info, trace, warn};
use rustc_hash::FxHashMap;

mod application;

fn main() {
    env_logger::init();
    error!("This is the error output");
    warn!("This is the warn output");
    info!("This is the info output");
    debug!("This is the debug output");
    trace!("This is the trace output");

    #[cfg(not(target_arch = "wasm32"))]
    {
        let default_keybindings = FxHashMap::default();
        // Temporarily avoid srgb formats for the swapchain on the web
        let options = Options {
            launch_gpu_prefer_low_power: false,
            should_exit: false,
            window_mode: WindowMode::WindowedFullscreen,
            ui_scale: 0,
            window_title_text: "Goosh Goosh",
            input_mode: InputMode::MappedToInstruction,
            key_bindings: default_keybindings,
            avail_cores: num_cpus::get(),
            render_must_be_main: false,
        };

        let app = Application::new(options);
        app.run();
    }
}
