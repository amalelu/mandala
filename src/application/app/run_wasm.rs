//! WASM event-loop body for [`super::Application::run`]. Lifted
//! verbatim out of the pre-split `app/mod.rs` so the browser-side
//! event loop has its own file. Every identifier the body reaches
//! is either owned by the `Application` value passed in, imported
//! via `use super::*`, or a fully-qualified `crate::`/`baumhard::`
//! path.

#![cfg(target_arch = "wasm32")]

use super::*;

/// Run the browser event loop against `app`. Called by the
/// `Application::run` dispatcher on wasm32 targets. winit-web's
/// loop takes over the browser thread; on shutdown the closure
/// returns and winit's `.expect(...)` at the bottom propagates any
/// internal failure up the call stack.
pub(super) fn run(mut app: Application) {
use wasm_bindgen::JsCast;
use winit::platform::web::WindowExtWebSys;
use std::rc::Rc;
use std::cell::{Cell, RefCell};
use baumhard::mindmap::tree_builder::MindMapTree;

baumhard::font::fonts::init();

// Load keybindings from the WASM environment (URL query param or
// localStorage) with a defaults fallback. Failure is non-fatal —
// see KeybindConfig::load_for_web().
app.options.keybind_config =
    crate::application::keybinds::KeybindConfig::load_for_web();

// Attach canvas to DOM
let canvas = app.window.canvas().expect("Failed to get canvas");
let web_window = web_sys::window().expect("No global window");
let document = web_window.document().expect("No document");
let body = document.body().expect("No body");
body.append_child(&canvas).expect("Failed to append canvas");
canvas.set_width(web_window.inner_width().unwrap().as_f64().unwrap() as u32);
canvas.set_height(web_window.inner_height().unwrap().as_f64().unwrap() as u32);

// Canvas must be focusable for keyboard events to reach winit.
// Without tabindex, an HTMLCanvasElement never receives focus.
canvas.set_attribute("tabindex", "0").ok();
let _ = canvas.focus();

// Re-focus on mousedown so clicking the canvas after tabbing
// to another element restores keyboard input.
{
    let canvas_for_focus = canvas.clone();
    let focus_cb = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(
        move |_: web_sys::Event| {
            let _ = canvas_for_focus.focus();
        },
    );
    canvas
        .add_event_listener_with_callback("mousedown", focus_cb.as_ref().unchecked_ref())
        .ok();
    focus_cb.forget(); // leak — lives for the page lifetime
}

// preventDefault on keydown while the text editor is open so
// Tab/Enter/Backspace/arrows don't fire browser defaults
// (tab-navigation, history-back, page-scroll).
let suppress_keys: Rc<Cell<bool>> = Rc::new(Cell::new(false));
{
    let suppress = suppress_keys.clone();
    let pd_cb = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(
        move |evt: web_sys::Event| {
            if suppress.get() {
                evt.prevent_default();
            }
        },
    );
    canvas
        .add_event_listener_with_callback("keydown", pd_cb.as_ref().unchecked_ref())
        .ok();
    pd_cb.forget();
}

let renderer_window = Arc::clone(&app.window);

// On WASM, check for ?map= query parameter to override the default path
let mindmap_path = {
    let web_window = web_sys::window().expect("No global window");
    let search = web_window.location().search().unwrap_or_default();
    let mut map_path: Option<String> = None;
    let trimmed = search.trim_start_matches('?');
    for pair in trimmed.split('&') {
        if let Some(val) = pair.strip_prefix("map=") {
            map_path = Some(val.to_string());
        }
    }
    map_path.unwrap_or_else(|| app.options.mindmap_path.clone())
};

// Shared state between the rAF render loop and the winit event
// loop. Two RefCells so input handlers can borrow InputState
// and Renderer simultaneously without conflict.
/// Pending left-click awaiting a release. `None` on init and after
/// release consumed; `Empty` after a click-down on empty canvas;
/// `Node(id)` after a click-down on a node. Full drag machine
/// (pan, move, reparent, connect) deferred to a later
/// WASM-parity session.
enum PendingClick {
    None,
    Empty,
    Node(String),
}
struct WasmInputState {
    document: MindMapDocument,
    mindmap_tree: Option<MindMapTree>,
    text_edit_state: TextEditState,
    last_click: Option<LastClick>,
    cursor_pos: (f64, f64),
    pending_click: PendingClick,
    modifiers: winit::keyboard::ModifiersState,
    /// Mirror of native's `app_scene` so the canvas-scene
    /// tree path (borders, eventually connections/portals)
    /// works identically on WASM. Threaded into every
    /// `rebuild_all` / `rebuild_scene_only` call below.
    app_scene: crate::application::scene_host::AppScene,
}

let renderer_rc: Rc<RefCell<Option<Renderer>>> = Rc::new(RefCell::new(None));
let input_rc: Rc<RefCell<Option<WasmInputState>>> = Rc::new(RefCell::new(None));

// Clone Rcs for the spawn_local init future
let renderer_for_init = renderer_rc.clone();
let input_for_init = input_rc.clone();

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
    )
    .await;

    let size = canvas.width();
    let height = canvas.height();
    renderer.process_decree(RenderDecree::SetSurfaceSize(size, height));

    // Load mindmap through Document -> Tree + Scene -> Renderer flow
    let mut doc_opt: Option<MindMapDocument> = None;
    let mut tree_opt: Option<MindMapTree> = None;
    // Local AppScene used only for the initial border tree
    // build; it's then dropped, and `WasmInputState`'s own
    // `app_scene` takes over for the live event loop.
    let mut init_app_scene =
        crate::application::scene_host::AppScene::new();
    if let Ok(doc) = MindMapDocument::load(&mindmap_path) {
        let mindmap_tree = doc.build_tree();
        renderer.rebuild_buffers_from_tree(&mindmap_tree.tree);
        renderer.fit_camera_to_tree(&mindmap_tree.tree);

        let scene = doc.build_scene(renderer.camera_zoom());
        update_connection_tree(&scene, &mut init_app_scene);
        update_border_tree_static(&doc, &mut init_app_scene);
        update_portal_tree(
            &doc,
            &std::collections::HashMap::new(),
            &mut init_app_scene,
            renderer,
        );
        update_connection_label_tree(&scene, &mut init_app_scene, renderer);
        flush_canvas_scene_buffers(&mut init_app_scene, renderer);
        tree_opt = Some(mindmap_tree);
        doc_opt = Some(doc);
    }

    renderer.process_decree(RenderDecree::StartRender);

    // Populate the shared state now that init is complete.
    *renderer_for_init.borrow_mut() = Some(renderer);

    if let Some(doc) = doc_opt {
        *input_for_init.borrow_mut() = Some(WasmInputState {
            document: doc,
            mindmap_tree: tree_opt,
            text_edit_state: TextEditState::Closed,
            last_click: None,
            cursor_pos: (0.0, 0.0),
            pending_click: PendingClick::None,
            modifiers: winit::keyboard::ModifiersState::empty(),
            app_scene: crate::application::scene_host::AppScene::new(),
        });
    }

    // WASM render loop via requestAnimationFrame
    use wasm_bindgen::closure::Closure;
    let renderer_for_raf = renderer_for_init.clone();
    let f: Rc<RefCell<Option<Closure<dyn FnMut()>>>> =
        Rc::new(RefCell::new(None));
    let g = f.clone();

    *g.borrow_mut() = Some(Closure::new(move || {
        if let Some(r) = renderer_for_raf.borrow_mut().as_mut() {
            r.process();
        }
        request_animation_frame(f.borrow().as_ref().unwrap());
    }));
    request_animation_frame(g.borrow().as_ref().unwrap());
});

// Resolve the keybind config once. `action_for(key, ctrl, shift, alt)`
// answers the dispatch question for every keydown.
let keybinds: ResolvedKeybinds = app.options.keybind_config.resolve();

// Clone Rcs for the event loop closure
let renderer_for_events = renderer_rc.clone();
let input_for_events = input_rc.clone();
let suppress_for_events = suppress_keys.clone();

app.event_loop.run(move |event, _window_target| {
    _ = (&app.window, &mut app.options);

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

        // --- Modifier tracking ---
        Event::WindowEvent {
            event: WindowEvent::ModifiersChanged(mods), ..
        } => {
            if let Some(input) = input_for_events.borrow_mut().as_mut() {
                input.modifiers = mods.state();
            }
        }

        // --- Keyboard input ---
        Event::WindowEvent {
            event: WindowEvent::KeyboardInput {
                event: KeyEvent {
                    state: ElementState::Pressed,
                    logical_key: ref logical_key,
                    ..
                },
                ..
            },
            ..
        } => {
            let key_name = crate::application::keybinds::key_to_name(logical_key);

            let mut input_borrow = input_for_events.borrow_mut();
            let mut renderer_borrow = renderer_for_events.borrow_mut();
            let (Some(input), Some(renderer)) =
                (input_borrow.as_mut(), renderer_borrow.as_mut())
            else { return; };

            // Editor keyboard-steal: if open, route all keys
            // to the editor so hotkeys don't collide with typed text.
            if input.text_edit_state.is_open() {
                handle_text_edit_key(
                    &key_name,
                    logical_key,
                    &mut input.text_edit_state,
                    &mut input.document,
                    &mut input.mindmap_tree,
                    renderer,
                );
                suppress_for_events.set(input.text_edit_state.is_open());
                return;
            }

            // Hotkey dispatch via keybinds.
            let action = key_name.as_deref().and_then(|k| {
                keybinds.action_for(
                    k,
                    input.modifiers.control_key(),
                    input.modifiers.shift_key(),
                    input.modifiers.alt_key(),
                )
            });
            match action {
                Some(Action::Undo) => {
                    if input.document.undo() {
                        rebuild_all(&input.document, &mut input.mindmap_tree, &mut input.app_scene, renderer);
                    }
                }
                Some(Action::CreateOrphanNode) => {
                    let canvas_pos = renderer.screen_to_canvas(
                        input.cursor_pos.0 as f32,
                        input.cursor_pos.1 as f32,
                    );
                    input.document.create_orphan_and_select(canvas_pos);
                    rebuild_all(&input.document, &mut input.mindmap_tree, &mut input.app_scene, renderer);
                }
                Some(Action::OrphanSelection) => {
                    if input.document.apply_orphan_selection_with_undo() {
                        rebuild_all(&input.document, &mut input.mindmap_tree, &mut input.app_scene, renderer);
                    }
                }
                Some(Action::DeleteSelection) => {
                    if input.document.apply_delete_selection() {
                        rebuild_all(&input.document, &mut input.mindmap_tree, &mut input.app_scene, renderer);
                    }
                }
                Some(a @ (Action::EditSelection | Action::EditSelectionClean)) => {
                    let clean = matches!(a, Action::EditSelectionClean);
                    if let SelectionState::Single(id) = &input.document.selection {
                        let nid = id.clone();
                        open_text_edit(
                            &nid, clean,
                            &mut input.document,
                            &mut input.text_edit_state,
                            &mut input.mindmap_tree,
                            renderer,
                        );
                        suppress_for_events.set(input.text_edit_state.is_open());
                    }
                }
                Some(Action::CancelMode) => {
                    // No AppMode on WASM yet; clear any pending click
                    // so a post-Esc click isn't retroactively paired
                    // with a pre-Esc click.
                    input.last_click = None;
                }
                Some(a @ (Action::EnterReparentMode | Action::EnterConnectMode)) => {
                    // AppMode state is still native-only; these
                    // actions are deferred on WASM (would require
                    // de-gating AppMode, hovered_node, and the
                    // reparent/connect click handlers).
                    log::debug!("WASM: mode-based action {:?} deferred", a);
                }
                Some(a) => {
                    // Actions whose backing surface is native-only
                    // (console, filesystem-backed save). On WASM
                    // they're acknowledged in the log and ignored.
                    log::debug!("WASM: action {:?} not supported", a);
                }
                None => {}
            }
        }

        // --- Mouse input ---
        Event::WindowEvent {
            event: WindowEvent::CursorMoved { position, .. }, ..
        } => {
            if let Some(input) = input_for_events.borrow_mut().as_mut() {
                input.cursor_pos = (position.x, position.y);
            }
        }

        Event::WindowEvent {
            event: WindowEvent::MouseInput {
                state: btn_state,
                button: MouseButton::Left,
                ..
            },
            ..
        } => {
            if btn_state == ElementState::Pressed {
                // --- Left mouse Pressed ---
                let mut input_borrow = input_for_events.borrow_mut();
                let Some(input) = input_borrow.as_mut() else { return; };

                // Compute canvas position via renderer
                let canvas_pos = {
                    let renderer_borrow = renderer_for_events.borrow();
                    match renderer_borrow.as_ref() {
                        Some(r) => r.screen_to_canvas(
                            input.cursor_pos.0 as f32,
                            input.cursor_pos.1 as f32,
                        ),
                        None => return,
                    }
                };

                // Hit test against nodes
                let hit_node: Option<String> = input.mindmap_tree
                    .as_mut()
                    .and_then(|tree| {
                        crate::application::document::hit_test(canvas_pos, tree)
                    });

                // Double-click detection
                let now = now_ms();
                let already_editing_same_target = input.text_edit_state
                    .node_id()
                    .map(|id| hit_node.as_deref() == Some(id))
                    .unwrap_or(false);
                let is_dblclick = !already_editing_same_target
                    && input.last_click
                        .as_ref()
                        .map(|prev| is_double_click(prev, now, input.cursor_pos, &hit_node))
                        .unwrap_or(false);

                if is_dblclick {
                    input.last_click = None;

                    let mut renderer_borrow = renderer_for_events.borrow_mut();
                    let Some(renderer) = renderer_borrow.as_mut() else { return; };

                    if let Some(ref node_id) = hit_node {
                        let nid = node_id.clone();
                        input.document.selection = SelectionState::Single(nid.clone());
                        rebuild_all(&input.document, &mut input.mindmap_tree, &mut input.app_scene, renderer);
                        open_text_edit(
                            &nid, false,
                            &mut input.document,
                            &mut input.text_edit_state,
                            &mut input.mindmap_tree,
                            renderer,
                        );
                    } else {
                        let allow_create = !matches!(
                            input.document.selection,
                            SelectionState::Edge(_) | SelectionState::Portal(_)
                        );
                        if allow_create {
                            let new_id = input.document.create_orphan_and_select(canvas_pos);
                            rebuild_all(&input.document, &mut input.mindmap_tree, &mut input.app_scene, renderer);
                            open_text_edit(
                                &new_id, true,
                                &mut input.document,
                                &mut input.text_edit_state,
                                &mut input.mindmap_tree,
                                renderer,
                            );
                        }
                    }
                    suppress_for_events.set(input.text_edit_state.is_open());
                    return;
                }

                input.pending_click = match hit_node.clone() {
                    Some(id) => PendingClick::Node(id),
                    None => PendingClick::Empty,
                };
                input.last_click = Some(LastClick {
                    time: now,
                    screen_pos: input.cursor_pos,
                    hit: hit_node,
                });
            } else {
                // --- Left mouse Released ---
                let mut input_borrow = input_for_events.borrow_mut();
                let Some(input) = input_borrow.as_mut() else { return; };

                let pending = std::mem::replace(&mut input.pending_click, PendingClick::None);
                if matches!(pending, PendingClick::None) { return; }

                if input.text_edit_state.is_open() {
                    let mut renderer_borrow = renderer_for_events.borrow_mut();
                    let Some(renderer) = renderer_borrow.as_mut() else { return; };
                    let release_canvas = renderer.screen_to_canvas(
                        input.cursor_pos.0 as f32,
                        input.cursor_pos.1 as f32,
                    );

                    let inside_edit_node = input.text_edit_state
                        .node_id()
                        .zip(input.mindmap_tree.as_ref())
                        .map(|(id, tree)| {
                            crate::application::document::point_in_node_aabb(
                                release_canvas, id, tree,
                            )
                        })
                        .unwrap_or(false);

                    if inside_edit_node {
                        return;
                    }

                    close_text_edit(
                        true,
                        &mut input.document,
                        &mut input.text_edit_state,
                        &mut input.mindmap_tree,
                        renderer,
                    );
                    suppress_for_events.set(false);
                    return;
                }

                // Plain selection click
                input.document.selection = match pending {
                    PendingClick::Node(node_id) => SelectionState::Single(node_id),
                    _ => SelectionState::None,
                };
                let mut renderer_borrow = renderer_for_events.borrow_mut();
                if let Some(renderer) = renderer_borrow.as_mut() {
                    rebuild_all(&input.document, &mut input.mindmap_tree, &mut input.app_scene, renderer);
                }
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
            let mut input_borrow = input_for_events.borrow_mut();
            let mut renderer_borrow = renderer_for_events.borrow_mut();
            if let (Some(input), Some(renderer)) =
                (input_borrow.as_mut(), renderer_borrow.as_mut())
            {
                renderer.process_decree(RenderDecree::CameraZoom {
                    screen_x: input.cursor_pos.0 as f32,
                    screen_y: input.cursor_pos.1 as f32,
                    factor: factor as f32,
                });
                // Zoom touches scene geometry (connection glyph
                // sample spacing, viewport cull rect) but not the
                // node text tree — scene-only rebuild is enough.
                rebuild_scene_only(&input.document, &mut input.app_scene, renderer);
            }
        }

        _ => {}
    }
}).expect("Event loop error");
}

/// Schedule `f` on the next browser animation frame — the
/// `requestAnimationFrame` handshake winit-web uses to drive its
/// render ticks. Kept next to the event-loop body because that's
/// its sole caller.
fn request_animation_frame(f: &wasm_bindgen::closure::Closure<dyn FnMut()>) {
    use wasm_bindgen::JsCast;
    web_sys::window()
        .unwrap()
        .request_animation_frame(f.as_ref().unchecked_ref())
        .unwrap();
}
