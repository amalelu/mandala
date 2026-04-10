#![allow(dead_code)]

use crate::application::app::{Application, Options};
use crate::application::common::{InputMode, WindowMode};
use crate::application::keybinds::KeybindConfig;
use log::info;

mod application;

const DEFAULT_MINDMAP: &str = "maps/testament.mindmap.json";

/// Parse the desktop CLI: the first non-flag positional argument is the
/// mindmap path, and `--keybinds <path>` specifies a custom keybinds JSON
/// file. Unknown flags are ignored for forward compatibility.
#[cfg(not(target_arch = "wasm32"))]
fn parse_cli() -> (String, Option<std::path::PathBuf>) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut mindmap_path: Option<String> = None;
    let mut keybinds_path: Option<std::path::PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--keybinds" {
            if let Some(val) = args.get(i + 1) {
                keybinds_path = Some(std::path::PathBuf::from(val));
                i += 2;
                continue;
            }
        } else if let Some(val) = a.strip_prefix("--keybinds=") {
            keybinds_path = Some(std::path::PathBuf::from(val));
        } else if !a.starts_with("--") && mindmap_path.is_none() {
            mindmap_path = Some(a.clone());
        }
        i += 1;
    }
    (
        mindmap_path.unwrap_or_else(|| DEFAULT_MINDMAP.to_string()),
        keybinds_path,
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn create_options() -> Options {
    let (mindmap_path, keybinds_path) = parse_cli();
    let keybind_config = KeybindConfig::load_for_desktop(keybinds_path.as_deref());

    Options {
        launch_gpu_prefer_low_power: false,
        should_exit: false,
        window_mode: WindowMode::WindowedFullscreen,
        ui_scale: 0,
        window_title_text: "Mandala",
        input_mode: InputMode::MappedToInstruction,
        avail_cores: num_cpus::get(),
        render_must_be_main: false,
        mindmap_path,
        keybind_config,
    }
}

#[cfg(target_arch = "wasm32")]
fn create_options() -> Options {
    // WASM: mindmap_path is replaced later by reading ?map=, and keybinds
    // are loaded from ?keybinds= / localStorage inside the WASM run() path.
    // We still seed the Options with sane defaults here.
    Options {
        launch_gpu_prefer_low_power: false,
        should_exit: false,
        window_mode: WindowMode::WindowedFullscreen,
        ui_scale: 0,
        window_title_text: "Mandala",
        input_mode: InputMode::MappedToInstruction,
        avail_cores: 1,
        render_must_be_main: false,
        mindmap_path: DEFAULT_MINDMAP.to_string(),
        keybind_config: KeybindConfig::default(),
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
