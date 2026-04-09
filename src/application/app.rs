use std::sync::{Arc, RwLock};

#[cfg(not(target_arch = "wasm32"))]
use pollster::block_on;
use indextree::Arena;
use wgpu::{Instance, SurfaceTargetUnsafe};
use winit::event::{ElementState, Event, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ControlFlow;
use winit::{event_loop::EventLoop, window::Window};

use crate::application::common::{InputMode, RenderDecree, WindowMode};
use crate::application::document::MindMapDocument;
use crate::application::renderer::Renderer;

use baumhard::gfx_structs::element::GfxElement;

/**
Represents the root container of the application
Manages the winit window and event_loop, and launches the rendering pipeline
 **/
pub struct Application {
    options: Options,
    event_loop: EventLoop<()>,
    window: Arc<Window>,
}

impl Application {
    pub fn new(options: Options) -> Self {
        let event_loop = EventLoop::new().expect("Could not create an EventLoop");

        let window = event_loop.create_window(Window::default_attributes()).expect("Failed to create application window");

        Application {
            options,
            event_loop,
            window: Arc::new(window),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn run(mut self) {
        // Single-threaded architecture: App owns the Renderer directly
        baumhard::font::fonts::init();

        // Initialize graphics arena (core GfxElement infrastructure)
        let gfx_arena: Arc<RwLock<Arena<GfxElement>>> = Arc::new(RwLock::new(Arena::new()));

        let unsafe_target = unsafe { SurfaceTargetUnsafe::from_window(self.window.as_ref()) }
            .expect("Failed to create a SurfaceTargetUnsafe");
        let instance = Instance::default();
        let surface = unsafe { instance.create_surface_unsafe(unsafe_target) }.unwrap();

        let mut renderer = block_on(Renderer::new(
            instance,
            surface,
            Arc::clone(&self.window),
            gfx_arena.clone(),
        ));

        // Configure initial surface size
        let size = self.window.inner_size();
        renderer.process_decree(RenderDecree::SetSurfaceSize(size.width, size.height));

        // Update arena buffers
        renderer.process_decree(RenderDecree::ArenaUpdate);

        // Load mindmap through Document -> Tree + Scene -> Renderer flow
        match MindMapDocument::load(&self.options.mindmap_path) {
            Ok(document) => {
                // Nodes: build Baumhard tree from MindMap hierarchy
                let mindmap_tree = document.build_tree();
                renderer.rebuild_buffers_from_tree(&mindmap_tree.tree);
                renderer.fit_camera_to_tree(&mindmap_tree.tree);

                // Connections + borders: flat pipeline from RenderScene
                let scene = document.build_scene();
                renderer.rebuild_connection_buffers(&scene.connection_elements);
                renderer.rebuild_border_buffers(&scene.border_elements);
            }
            Err(e) => {
                log::error!("{}", e);
            }
        }

        // Start rendering
        renderer.process_decree(RenderDecree::StartRender);

        let mut cursor_pos: (f64, f64) = (0.0, 0.0);
        let mut is_panning = false;

        self.event_loop.run(move |event, _window_target| {
            _ = (&self.window, &mut self.options);

            _window_target.set_control_flow(ControlFlow::Poll);

            match event {
                //////////////////////////
                //// WINDOW SPECIFIC ////
                ////////////////////////
                Event::WindowEvent {
                    event: WindowEvent::Resized(size),
                    ..
                } => {
                    renderer.process_decree(RenderDecree::SetSurfaceSize(size.width, size.height));
                }
                Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    ..
                } => {
                    renderer.process_decree(RenderDecree::Terminate);
                    std::process::exit(0);
                }
                ////////////////
                //// MOUSE ////
                //////////////
                Event::WindowEvent {
                    event: WindowEvent::MouseInput { state, button, .. },
                    ..
                } => {
                    if button == MouseButton::Left || button == MouseButton::Middle {
                        is_panning = state == ElementState::Pressed;
                    }
                }
                Event::WindowEvent {
                    event: WindowEvent::MouseWheel { delta, .. },
                    ..
                } => {
                    let scroll_y = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y as f64,
                        MouseScrollDelta::PixelDelta(pos) => pos.y / 50.0,
                    };
                    let factor = if scroll_y > 0.0 { 1.1 } else { 1.0 / 1.1 };
                    renderer.process_decree(RenderDecree::CameraZoom {
                        screen_x: cursor_pos.0 as f32,
                        screen_y: cursor_pos.1 as f32,
                        factor: factor as f32,
                    });
                }
                Event::WindowEvent {
                    event: WindowEvent::CursorMoved { position, .. },
                    ..
                } => {
                    let dx = position.x - cursor_pos.0;
                    let dy = position.y - cursor_pos.1;
                    cursor_pos = (position.x, position.y);
                    if is_panning {
                        renderer.process_decree(RenderDecree::CameraPan(dx as f32, dy as f32));
                    }
                }
                Event::AboutToWait => {
                    // Drive the render loop each frame
                    renderer.process();
                }
                _ => {}
            }
        }).expect("Some kind of unexpected error appears to have taken place")
    }

    #[cfg(target_arch = "wasm32")]
    pub fn run(mut self) {
        use wasm_bindgen::JsCast;
        use winit::platform::web::WindowExtWebSys;

        baumhard::font::fonts::init();

        // Attach canvas to DOM
        let canvas = self.window.canvas().expect("Failed to get canvas");
        let web_window = web_sys::window().expect("No global window");
        let document = web_window.document().expect("No document");
        let body = document.body().expect("No body");
        body.append_child(&canvas).expect("Failed to append canvas");
        canvas.set_width(web_window.inner_width().unwrap().as_f64().unwrap() as u32);
        canvas.set_height(web_window.inner_height().unwrap().as_f64().unwrap() as u32);

        let gfx_arena: Arc<RwLock<Arena<GfxElement>>> = Arc::new(RwLock::new(Arena::new()));
        let renderer_window = Arc::clone(&self.window);

        // On WASM, check for ?map= query parameter to override the default path
        let mindmap_path = {
            let web_window = web_sys::window().expect("No global window");
            let search = web_window.location().search().unwrap_or_default();
            if let Some(map_param) = search.strip_prefix("?map=") {
                map_param.to_string()
            } else {
                self.options.mindmap_path.clone()
            }
        };

        // On WASM we run single-threaded -- spawn the renderer init as a future
        wasm_bindgen_futures::spawn_local(async move {
            let instance = Instance::default();
            let surface = instance
                .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
                .expect("Failed to create surface");
            let mut renderer = Renderer::new(
                instance,
                surface,
                renderer_window,
                gfx_arena.clone(),
            )
            .await;

            let size = canvas.width();
            let height = canvas.height();
            renderer.process_decree(RenderDecree::SetSurfaceSize(size, height));

            // Load mindmap through Document -> Tree + Scene -> Renderer flow
            if let Ok(document) = MindMapDocument::load(&mindmap_path) {
                // Nodes: build Baumhard tree from MindMap hierarchy
                let mindmap_tree = document.build_tree();
                renderer.rebuild_buffers_from_tree(&mindmap_tree.tree);
                renderer.fit_camera_to_tree(&mindmap_tree.tree);

                // Connections + borders: flat pipeline from RenderScene
                let scene = document.build_scene();
                renderer.rebuild_connection_buffers(&scene.connection_elements);
                renderer.rebuild_border_buffers(&scene.border_elements);
            }

            renderer.process_decree(RenderDecree::StartRender);

            // Store renderer in a RefCell for the event loop
            let renderer = std::cell::RefCell::new(renderer);

            // WASM render loop via requestAnimationFrame
            use wasm_bindgen::closure::Closure;
            let f: std::rc::Rc<std::cell::RefCell<Option<Closure<dyn FnMut()>>>> =
                std::rc::Rc::new(std::cell::RefCell::new(None));
            let g = f.clone();

            *g.borrow_mut() = Some(Closure::new(move || {
                renderer.borrow_mut().process();
                request_animation_frame(f.borrow().as_ref().unwrap());
            }));
            request_animation_frame(g.borrow().as_ref().unwrap());
        });

        // The event loop handles input but doesn't block on WASM
        let mut cursor_pos: (f64, f64) = (0.0, 0.0);
        let mut is_panning = false;

        self.event_loop.run(move |event, _window_target| {
            _ = (&self.window, &mut self.options);

            match event {
                Event::WindowEvent {
                    event: WindowEvent::Resized(_size), ..
                } => {
                    // On WASM, resize is handled by the renderer's own loop
                }
                Event::WindowEvent {
                    event: WindowEvent::CloseRequested, ..
                } => {
                    // WASM doesn't really close
                }
                Event::WindowEvent {
                    event: WindowEvent::MouseInput { state, button, .. }, ..
                } => {
                    if button == MouseButton::Left || button == MouseButton::Middle {
                        is_panning = state == ElementState::Pressed;
                    }
                }
                Event::WindowEvent {
                    event: WindowEvent::MouseWheel { delta, .. }, ..
                } => {
                    let _scroll_y = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y as f64,
                        MouseScrollDelta::PixelDelta(pos) => pos.y / 50.0,
                    };
                    // TODO: WASM input needs to be forwarded to the renderer
                }
                Event::WindowEvent {
                    event: WindowEvent::CursorMoved { position, .. }, ..
                } => {
                    let _dx = position.x - cursor_pos.0;
                    let _dy = position.y - cursor_pos.1;
                    cursor_pos = (position.x, position.y);
                    // TODO: WASM input needs to be forwarded to the renderer
                }
                _ => {}
            }
        }).expect("Event loop error");
    }
}

#[cfg(target_arch = "wasm32")]
fn request_animation_frame(f: &wasm_bindgen::closure::Closure<dyn FnMut()>) {
    use wasm_bindgen::JsCast;
    web_sys::window()
        .unwrap()
        .request_animation_frame(f.as_ref().unchecked_ref())
        .unwrap();
}

/**
Launch and run options for the application and the application instance
 **/
#[derive(Clone)]
pub struct Options {
    pub launch_gpu_prefer_low_power: bool,
    pub should_exit: bool,
    pub window_mode: WindowMode,
    pub ui_scale: i8,
    pub window_title_text: &'static str,
    pub input_mode: InputMode,
    pub avail_cores: usize,
    pub render_must_be_main: bool,
    pub mindmap_path: String,
}
