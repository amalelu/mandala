use std::sync::{Arc, RwLock};
#[cfg(not(target_arch = "wasm32"))]
use std::thread;
#[cfg(not(target_arch = "wasm32"))]
use std::thread::JoinHandle;

use crossbeam_channel::{unbounded, Receiver, Sender};
use enum_map::EnumMap;
use glam::f32::Vec2;
use indextree::Arena;
use log::{debug, error};
#[cfg(not(target_arch = "wasm32"))]
use pollster::block_on;
use rustc_hash::FxHashMap;
use wgpu::{Instance, SurfaceTargetUnsafe};
use winit::dpi::PhysicalSize;
use winit::event::{DeviceId, ElementState, Event, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ControlFlow};
use winit::window::WindowId;
use winit::{event_loop::EventLoop, window::Window};

use crate::application::common::{acknowledge_instruction, AckKey, Decree, HostDecree, InputMode, Instruction,
                                 RenderDecree, UiDecree, UiSetting, WindowMode, KeyPress};

use crate::application::game_concepts::LocalGameHost;
use crate::application::main_menu;
use crate::application::renderer::Renderer;

use baumhard::font::fonts::AppFont;
use baumhard::gfx_structs::element::GfxElement;
use baumhard::gfx_structs::area::GlyphArea;
use baumhard::core::primitives;
use baumhard::core::primitives::{ColorFontRegion};

/**
Represents the root container of the application
Manages the winit window and event_loop, and launches application-loop and GPU thread
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

    fn check_for_decrees(
        receiver: &Receiver<Instruction>,
        ack_map: &mut EnumMap<AckKey, usize>,
        options: &mut Options,
    ) {
        while !receiver.is_empty() {
            debug!("Receiver is not empty");
            let result = receiver.recv();
            if result.is_err() {
                error!("Failed to receive instruction");
            } else {
                let instruction = result.unwrap();
                let acknowledge = instruction.acknowledge;
                let ack_sender = instruction.ack_sender;
                match instruction.decree {
                    Decree::Ui(ui_decree) => match ui_decree {
                        UiDecree::ExitApplication => {
                            options.should_exit = true;
                        }
                        UiDecree::UpdateSetting(setting) => {
                            Self::update_setting(setting, options);
                        }
                        UiDecree::Noop => {
                            panic!("Noop decree received")
                        }
                    },
                    Decree::Acknowledge(ack_key, ack) => {
                        AckKey::check_ack(ack_key, ack_map, ack);
                    }
                    _ => {}
                }
                acknowledge_instruction(acknowledge.0, acknowledge.1, ack_sender);
            }
        }
    }

    fn update_setting(setting: UiSetting, options: &mut Options) {
        match setting {
            UiSetting::InputMode(input_mode) => options.input_mode = input_mode,
            UiSetting::KeyboardBindings(bindings) => {
                options.key_bindings = bindings;
            }
            UiSetting::WindowMode(window_mode) => {
                options.window_mode = window_mode;
            }
        }
    }

    fn respond_to_window_resize(render_sender: &Sender<Instruction>, size: PhysicalSize<u32>) {
        let result = render_sender.send(Instruction::new(Decree::Render(
            RenderDecree::SetSurfaceSize(size.width, size.height),
        )));

        if result.is_err() {
            error!("failed to update renderer on window resize");
        }
    }

    fn respond_to_key_event(
        event: KeyEvent,
        window_id: WindowId,
        device_id: DeviceId,
        is_synthetic: bool,
        options: &Options,
        game_sender: &Sender<Instruction>,
    ) {
        match options.input_mode {
            InputMode::Direct => {
                if game_sender
                    .send(Instruction::new(Decree::Host(
                        HostDecree::DirectKeyboardEvent {
                            window_id,
                            device_id,
                            key_press: KeyPress::placeholder(),
                            is_synthetic,
                        },
                    )))
                    .is_err()
                {
                    error!("Failed to send keyboard event to application thread");
                }
            }
            InputMode::MappedToInstruction => {

            }
        }
    }

    fn respond_to_application_exit(
        render_sender: &Sender<Instruction>,
        game_sender: &Sender<Instruction>,
    ) {
        let result = render_sender.send(Instruction::new(Decree::Render(RenderDecree::Terminate)));

        if result.is_err() {
            error!("Failed to send exit signal to render thread");
        }
        let result = game_sender.send(Instruction::new(Decree::Host(HostDecree::ExitInstance)));

        if result.is_err() {
            error!("Failed to send exit signal to application thread");
        }
    }

    fn respond_to_window_destroyed(
        render_sender: &Sender<Instruction>,
        game_sender: &Sender<Instruction>,
    ) {
        // Not quite sure what we should do in this case. Try to spawn a new window, or close the application?
    }

    fn stop_rendering(render_sender: &Sender<Instruction>) -> bool {
        render_sender
            .send(Instruction::new(Decree::Render(RenderDecree::StopRender)))
            .expect("failed to send stop render command to renderer");
        false
    }

    fn start_rendering(render_sender: &Sender<Instruction>) -> bool {
        render_sender
            .send(Instruction::new(Decree::Render(RenderDecree::StartRender)))
            .expect("failed to send start render command to renderer");
        true
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn run(mut self) {
        // Bootstrap - initialize necessary resources, application threads, etc..
        // This thread will be the UI and winit eventloop thread
        // This thread will signal the rendering thread to stop rendering, until it has updated
        // its surface config.
        // This thread will signal the third application thread on user input
        // The application thread and renderer will also have two-way communication
        //
        // The application thread is the host of the content, and decides what should be shown and
        // what response is reasonable according to the given input

        // This is necessary to initialize the lazy statics
        baumhard::font::fonts::init();
        let mut acknowledge: EnumMap<AckKey, usize> = EnumMap::default();

        // This element init is just temporary for testing
        let mut element = GfxElement::new_area_non_indexed_with_id(
            GlyphArea::new(40.0, 40.0, Vec2::new(20.0, 20.0), Vec2::new(1000.0, 800.0)),
            0,
            1,
        );
        element.glyph_area_mut().unwrap().text = "Meat Jackson sex🦅 glyphon 🦁\n".to_string();
        element
            .glyph_area_mut().unwrap()
            .regions
            .submit_region(ColorFontRegion::new(
               primitives::Range::new(0, 10),
               Some(AppFont::Evilz),
               Some([0.4, 0.2, 0.2, 1.0]),
            ));

        element
            .glyph_area_mut().unwrap()
            .regions
            .submit_region(ColorFontRegion::new(
               primitives::Range::new(11, 500),
               Some(AppFont::NIGHTCROW),
               Some([0.0, 0.6, 0.4, 1.0]),
            ));
        // Initialize graphics arena
        let gfx_arena: Arc<RwLock<Arena<GfxElement>>> = Arc::new(RwLock::new(Arena::new()));
        gfx_arena
            .try_write()
            .expect(
                "Failed to acquire exclusive write lock for gfx_arena, \
                but that doesn't make sense, the code should speak for itself..",
            )
            .new_node(element);
        let unsafe_target = unsafe {SurfaceTargetUnsafe::from_window(self.window.as_ref())}
           .expect("Failed to create a SurfaceTargetUnsafe");
        let mut ack_index: Box<usize> = Box::new(1);
        let instance = Instance::default();
        let surface = unsafe { instance.create_surface_unsafe(unsafe_target) }.unwrap();
        let (this_sender, this_receiver) = unbounded();
        let (render_sender, renderer_receiver) = unbounded();
        let (game_sender, game_receiver) = unbounded();

        // Spawn game-controller thread
        let game_handle: JoinHandle<()>;
        let render_sender_for_game = render_sender.clone();
        let ui_sender_for_game = this_sender.clone();
        let game_sender_for_game = game_sender.clone();

        game_handle = thread::spawn(move || {
            let mut game_host = LocalGameHost::new(
                None,
                game_sender_for_game,
                game_receiver,
                ui_sender_for_game,
                render_sender_for_game,
                Arc::new(Default::default()),
            );
            main_menu::launch_main_menu(&mut game_host);
            let mut run = true;
            while run {
                run = game_host.process();
            }
        });

        // Spawn renderer thread
        let renderer_handle: JoinHandle<()>;
        let renderer_sender_clone = render_sender.clone();
        let ui_sender_for_renderer = this_sender.clone();
        let game_sender_for_renderer = game_sender.clone();
        let renderer_window = Arc::clone(&self.window);
        renderer_handle = thread::spawn(move || {
            let mut renderer = block_on(Renderer::new(
                instance,
                surface,
                renderer_window,
                renderer_sender_clone,
                renderer_receiver,
                ui_sender_for_renderer,
                game_sender_for_renderer,
                gfx_arena.clone(),
            ));
            let mut run = true;
            while run {
                run = renderer.process();
            }
        });
        debug!("Spun up renderer thread");
        // now give the renderer its initial surface update
        let size = self.window.inner_size();
        let result = render_sender.send(Instruction::new_acknowledge(
            Decree::Render(RenderDecree::SetSurfaceSize(size.width, size.height)),
            AckKey::Render,
            *ack_index.clone(),
            Some(this_sender.clone()),
        ));

        if result.is_err() {
            error!("Failed to send initial window config to renderer")
        }
        debug!("notified renderer of window size");
        *ack_index += 1;
        let mut render_switch = false;
        let mut cursor_pos: (f64, f64) = (0.0, 0.0);
        let mut is_panning = false;

        render_sender
            .send(Instruction::new(Decree::Render(RenderDecree::ArenaUpdate)))
            .expect("Could not update arena");

        // Load the default mindmap
        render_sender
            .send(Instruction::new(Decree::Render(RenderDecree::LoadMindMap(
                "maps/testament.mindmap.json".to_string(),
            ))))
            .expect("Could not send mindmap load command");

        // Have the closure take ownership of the application
        // `event_loop.run` never returns, therefore we must do this to ensure
        // the resources are properly cleaned up.
        self.event_loop.run(move |event, window_target| {
            // by referencing these immediately within the event_loop, we ensure that they are
            // moved into the event_loop, rather than dropped
            _ = (
                &renderer_handle,
                &this_sender,
                &this_receiver,
                &render_sender,
                &game_sender,
                &self.window,
                &mut self.options,
            );

            // The following loop is going to be long, but it is so well-structured due to the match blocks that
            // it should be no problem for now at least
            window_target.set_control_flow(ControlFlow::Poll);

            Self::check_for_decrees(&this_receiver, &mut acknowledge, &mut self.options);

            if !render_switch && acknowledge[AckKey::Render] == 0 {
                // The lock has been released by an acknowledgement, so start rendering
                render_switch = Self::start_rendering(&render_sender);
            }
            match event {
                //////////////////////////
                //// WINDOW SPECIFIC ////
                ////////////////////////
                Event::WindowEvent {
                    event: WindowEvent::Resized(size),
                    ..
                } => {
                    // update the renderer about the new size
                    Self::respond_to_window_resize(&render_sender, size);
                }
                Event::WindowEvent {
                    event: WindowEvent::Focused(active),
                    ..
                } => {
                    // Alert application stack that the window is now focused
                }
                Event::WindowEvent {
                    window_id,
                    event: WindowEvent::Moved(position),
                } => {
                    // Alert application stack of window move
                }
                Event::WindowEvent {
                    event:
                        WindowEvent::ScaleFactorChanged {
                            scale_factor,
                            inner_size_writer,
                        },
                    ..
                } => {
                    // Update application scale factor
                }
                // It seems the user has requested the application to close
                Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    ..
                } => {
                    Self::respond_to_application_exit(&render_sender, &game_sender);
                }
                // The window has been destroyed. That's an ambiguous message.
                Event::WindowEvent {
                    window_id,
                    event: WindowEvent::Destroyed,
                    ..
                } => {
                    Self::respond_to_window_destroyed(&render_sender, &game_sender);
                }
                //// FILE DRAG AND DROP ////
                Event::WindowEvent {
                    window_id,
                    event: WindowEvent::HoveredFile(path),
                } => {
                    // Handle file hover
                }
                Event::WindowEvent {
                    window_id,
                    event: WindowEvent::HoveredFileCancelled,
                } => {
                    // Handle file hover cancelled
                }
                Event::WindowEvent {
                    window_id,
                    event: WindowEvent::DroppedFile(path),
                } => {
                    // Handle file drop into window
                }
                ///////////////////
                //// KEYBOARD ////
                /////////////////
                Event::WindowEvent {
                    window_id,
                    event:
                        WindowEvent::KeyboardInput {
                           event,
                            device_id,
                            is_synthetic,
                        },
                } => {
                    Self::respond_to_key_event(
                        event,
                        window_id,
                        device_id,
                        is_synthetic,
                        &self.options,
                        &game_sender,
                    );
                }
                Event::WindowEvent {
                    window_id,
                    event: WindowEvent::ModifiersChanged(modifier_state),
                } => {
                    // Alert application stack of modifier change
                }
                ////////////////
                //// MOUSE ////
                //////////////
                Event::WindowEvent {
                    event:
                        WindowEvent::MouseInput {
                            state,
                            button,
                            ..
                        },
                    ..
                } => {
                    // Left or middle mouse button for panning
                    if button == MouseButton::Left || button == MouseButton::Middle {
                        is_panning = state == ElementState::Pressed;
                    }
                }
                Event::WindowEvent {
                    event:
                        WindowEvent::MouseWheel {
                            delta,
                            ..
                        },
                    ..
                } => {
                    let scroll_y = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y as f64,
                        MouseScrollDelta::PixelDelta(pos) => pos.y / 50.0,
                    };
                    let factor = if scroll_y > 0.0 { 1.1 } else { 1.0 / 1.1 };
                    let _ = render_sender.send(Instruction::new(Decree::Render(
                        RenderDecree::CameraZoom {
                            screen_x: cursor_pos.0 as f32,
                            screen_y: cursor_pos.1 as f32,
                            factor: factor as f32,
                        },
                    )));
                }
                //// CURSOR ////
                Event::WindowEvent {
                    window_id,
                    event: WindowEvent::CursorEntered { device_id },
                } => {
                    // Alert application stack that cursor has entered window
                }
                Event::WindowEvent {
                    event:
                        WindowEvent::CursorMoved {
                            position,
                            ..
                        },
                    ..
                } => {
                    let dx = position.x - cursor_pos.0;
                    let dy = position.y - cursor_pos.1;
                    cursor_pos = (position.x, position.y);
                    if is_panning {
                        let _ = render_sender.send(Instruction::new(Decree::Render(
                            RenderDecree::CameraPan(dx as f32, dy as f32),
                        )));
                    }
                }
                Event::WindowEvent {
                    window_id,
                    event: WindowEvent::CursorLeft { device_id },
                } => {
                    // Alert application stack that cursor has left the window
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

        let (render_sender, renderer_receiver) = unbounded();
        let (game_sender, _game_receiver) = unbounded();
        let (this_sender, this_receiver) = unbounded();

        let render_sender_for_init = render_sender.clone();
        let this_sender_clone = this_sender.clone();
        let game_sender_clone = game_sender.clone();
        let gfx_arena: Arc<RwLock<Arena<GfxElement>>> = Arc::new(RwLock::new(Arena::new()));
        let renderer_window = Arc::clone(&self.window);

        // On WASM we run single-threaded — spawn the renderer init as a future
        wasm_bindgen_futures::spawn_local(async move {
            let instance = Instance::default();
            let surface = instance
                .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
                .expect("Failed to create surface");
            let mut renderer = Renderer::new(
                instance,
                surface,
                renderer_window,
                render_sender_for_init,
                renderer_receiver,
                this_sender_clone,
                game_sender_clone,
                gfx_arena.clone(),
            )
            .await;

            let size = canvas.width();
            let height = canvas.height();
            renderer.process_decree(Decree::Render(RenderDecree::SetSurfaceSize(size, height)));
            renderer.process_decree(Decree::Render(RenderDecree::LoadMindMap(
                "maps/testament.mindmap.json".to_string(),
            )));
            renderer.process_decree(Decree::Render(RenderDecree::StartRender));

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
        let mut acknowledge: EnumMap<AckKey, usize> = EnumMap::default();

        self.event_loop.run(move |event, _window_target| {
            _ = (&self.window, &mut self.options);

            match event {
                Event::WindowEvent {
                    event: WindowEvent::Resized(size), ..
                } => {
                    let _ = render_sender.send(Instruction::new(Decree::Render(
                        RenderDecree::SetSurfaceSize(size.width, size.height),
                    )));
                }
                Event::WindowEvent {
                    event: WindowEvent::CloseRequested, ..
                } => {
                    let _ = render_sender.send(Instruction::new(Decree::Render(
                        RenderDecree::Terminate,
                    )));
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
                    let scroll_y = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y as f64,
                        MouseScrollDelta::PixelDelta(pos) => pos.y / 50.0,
                    };
                    let factor = if scroll_y > 0.0 { 1.1 } else { 1.0 / 1.1 };
                    let _ = render_sender.send(Instruction::new(Decree::Render(
                        RenderDecree::CameraZoom {
                            screen_x: cursor_pos.0 as f32,
                            screen_y: cursor_pos.1 as f32,
                            factor: factor as f32,
                        },
                    )));
                }
                Event::WindowEvent {
                    event: WindowEvent::CursorMoved { position, .. }, ..
                } => {
                    let dx = position.x - cursor_pos.0;
                    let dy = position.y - cursor_pos.1;
                    cursor_pos = (position.x, position.y);
                    if is_panning {
                        let _ = render_sender.send(Instruction::new(Decree::Render(
                            RenderDecree::CameraPan(dx as f32, dy as f32),
                        )));
                    }
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
    // A key is mapped directly to a decree, which can then be sent directly to the application/game thread
    pub key_bindings: FxHashMap<KeyPress, HostDecree>,
    pub avail_cores: usize,
    // The rest of the IO is already on the main thread
    pub render_must_be_main: bool,
}
