#![allow(dead_code)]

use crate::application::app::{Application, Options};
use crate::application::common::{InputMode, WindowMode};
use log::{debug, error, info, trace, warn};
use rustc_hash::FxHashMap;

mod application;

fn create_options() -> Options {
    let default_keybindings = FxHashMap::default();
    Options {
        launch_gpu_prefer_low_power: false,
        should_exit: false,
        window_mode: WindowMode::WindowedFullscreen,
        ui_scale: 0,
        window_title_text: "Mandala",
        input_mode: InputMode::MappedToInstruction,
        key_bindings: default_keybindings,
        #[cfg(not(target_arch = "wasm32"))]
        avail_cores: num_cpus::get(),
        #[cfg(target_arch = "wasm32")]
        avail_cores: 1,
        render_must_be_main: false,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    env_logger::init();
    info!("Starting Mandala (native)");

    let app = Application::new(create_options());
    app.run();
}

#[cfg(target_arch = "wasm32")]
fn main() {
    std::panic::set_hook(Box::new(console_error_panic_hook::hook));
    console_log::init_with_level(log::Level::Info).expect("Failed to init logger");
    info!("Starting Mandala (WASM)");

    let app = Application::new(create_options());
    app.run();
}
