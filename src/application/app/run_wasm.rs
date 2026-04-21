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
let cw = canvas.width();
let ch = canvas.height();
log::info!("WASM: canvas sized {}x{}", cw, ch);
if cw == 0 || ch == 0 {
    log::warn!(
        "WASM: canvas has zero dimension — render surface will be empty"
    );
}

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
    /// Cursor landed on a portal **icon** at mouse-down. Committed
    /// at mouse-up into a `SelectionState::PortalLabel`. Carries
    /// both the owning-edge key and the endpoint id the marker
    /// belongs to, matching the native click dispatch surface.
    PortalMarker {
        edge_key: baumhard::mindmap::scene_cache::EdgeKey,
        endpoint_node_id: String,
    },
    /// Cursor landed on a portal **text** at mouse-down.
    /// Committed at mouse-up into a `SelectionState::PortalText`.
    /// Shares the identity shape with `PortalMarker`; only the
    /// mouse-up selection routing differs.
    PortalText {
        edge_key: baumhard::mindmap::scene_cache::EdgeKey,
        endpoint_node_id: String,
    },
    /// Cursor landed on a line-mode edge's label AABB at
    /// mouse-down. Committed at mouse-up into
    /// `SelectionState::EdgeLabel` so per-label color / font /
    /// copy operations target the label instead of the edge
    /// body. Double-click is handled inline by the press-time
    /// dispatcher — WASM doesn't open the inline editor modal
    /// yet so the dbl-click branch falls back to the same
    /// selection commit for parity with single click.
    EdgeLabel(baumhard::mindmap::scene_cache::EdgeKey),
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

// Renderer init is async on the browser (adapter + surface setup
// are Promise-backed). Spawn as a future so the event loop doesn't
// block waiting for wgpu.
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
    log::info!("WASM: adapter + surface + renderer ready");

    let size = canvas.width();
    let height = canvas.height();
    renderer.process_decree(RenderDecree::SetSurfaceSize(size, height));
    log::info!("WASM: surface configured {}x{}", size, height);

    // std::fs is unavailable in the browser; fetch over the page origin instead.
    let mut doc_opt: Option<MindMapDocument> = None;
    let mut tree_opt: Option<MindMapTree> = None;
    // Local AppScene used only for the initial border tree
    // build; it's then dropped, and `WasmInputState`'s own
    // `app_scene` takes over for the live event loop.
    let mut init_app_scene =
        crate::application::scene_host::AppScene::new();
    match fetch_map_json(&mindmap_path).await {
        Ok(json) => match MindMapDocument::from_json_str(&json, Some(mindmap_path.clone())) {
            Ok(mut doc) => {
                // Canvas background: resolve through theme variables
                // so `"var(--bg)"` works, then hand off to the
                // renderer as the render-pass clear color. Mirrors
                // run_native.rs so the WASM canvas paints against
                // the doc's configured background instead of the
                // default pitch black.
                let vars = &doc.mindmap.canvas.theme_variables;
                let resolved_bg = baumhard::util::color::resolve_var(
                    &doc.mindmap.canvas.background_color,
                    vars,
                );
                renderer.set_clear_color_from_hex(resolved_bg);

                // Four-source mutation registry, matching the native
                // path: app bundle (shipped in the binary) < user
                // source (?mutations= query param + localStorage) <
                // map (custom_mutations in the .mindmap.json) <
                // inline (on individual nodes). Plus the Rust-backed
                // handlers for layouts too structural for pure data.
                let (app_mutations, user_mutations) =
                    crate::application::document::mutations_loader::load_app_and_user();
                doc.build_mutation_registry_with_app_and_user(
                    &app_mutations,
                    &user_mutations,
                );
                crate::application::document::mutations::register_builtin_handlers(
                    &mut doc,
                );

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
                    &mut renderer,
                );
                update_connection_label_tree(&scene, &mut init_app_scene, &mut renderer);
                flush_canvas_scene_buffers(&mut init_app_scene, &mut renderer);
                tree_opt = Some(mindmap_tree);
                doc_opt = Some(doc);
            }
            Err(e) => log::error!(
                "WASM: failed to construct document from '{}': {}",
                mindmap_path, e
            ),
        },
        Err(e) => log::error!("WASM: failed to fetch '{}': {}", mindmap_path, e),
    }

    renderer.process_decree(RenderDecree::StartRender);
    log::info!("WASM: StartRender dispatched, rAF loop starting");

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
            event: WindowEvent::Resized(size), ..
        } => {
            if let Some(renderer) = renderer_for_events.borrow_mut().as_mut() {
                renderer.process_decree(
                    RenderDecree::SetSurfaceSize(size.width, size.height),
                );
            }
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
                    input.modifiers.control_key(),
                    input.modifiers.shift_key(),
                    input.modifiers.alt_key(),
                    &keybinds,
                    &mut input.text_edit_state,
                    &mut input.document,
                    &mut input.mindmap_tree,
                    &mut input.app_scene,
                    renderer,
                );
                suppress_for_events.set(input.text_edit_state.is_open());
                return;
            }

            // Hotkey dispatch via keybinds.
            let action = key_name.as_deref().and_then(|k| {
                keybinds.action_for_context(
                    crate::application::keybinds::InputContext::Document,
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
                            &mut input.app_scene,
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
                // WASM mirrors the native dispatch: build a ClickHit
                // that covers node / portal-marker / empty, and
                // route double-click on a portal marker to a camera
                // jump. The portal hit test runs only when the node
                // hit test misses, same as native.
                let (portal_text_hit, portal_icon_hit, edge_label_hit) =
                    if hit_node.is_none() {
                        let renderer_borrow = renderer_for_events.borrow();
                        let t = renderer_borrow
                            .as_ref()
                            .and_then(|r| r.hit_test_portal_text(canvas_pos));
                        let i = if t.is_none() {
                            renderer_borrow
                                .as_ref()
                                .and_then(|r| r.hit_test_portal(canvas_pos))
                        } else {
                            None
                        };
                        let l = if t.is_none() && i.is_none() {
                            renderer_borrow
                                .as_ref()
                                .and_then(|r| r.hit_test_any_edge_label(canvas_pos))
                        } else {
                            None
                        };
                        (t, i, l)
                    } else {
                        (None, None, None)
                    };
                let click_hit: ClickHit = if let Some(id) = &hit_node {
                    ClickHit::Node(id.clone())
                } else if let Some((key, ep)) = &portal_text_hit {
                    ClickHit::PortalText {
                        edge: key.clone(),
                        endpoint: ep.clone(),
                    }
                } else if let Some((key, ep)) = &portal_icon_hit {
                    ClickHit::PortalMarker {
                        edge: key.clone(),
                        endpoint: ep.clone(),
                    }
                } else if let Some(key) = &edge_label_hit {
                    ClickHit::EdgeLabel(key.clone())
                } else {
                    ClickHit::Empty
                };
                let is_dblclick = !already_editing_same_target
                    && input.last_click
                        .as_ref()
                        .map(|prev| is_double_click(prev, now, input.cursor_pos, &click_hit))
                        .unwrap_or(false);

                if is_dblclick {
                    input.last_click = None;

                    let mut renderer_borrow = renderer_for_events.borrow_mut();
                    let Some(renderer) = renderer_borrow.as_mut() else { return; };

                    match &click_hit {
                        ClickHit::Node(node_id) => {
                            let nid = node_id.clone();
                            input.document.selection = SelectionState::Single(nid.clone());
                            rebuild_all(&input.document, &mut input.mindmap_tree, &mut input.app_scene, renderer);
                            open_text_edit(
                                &nid, false,
                                &mut input.document,
                                &mut input.text_edit_state,
                                &mut input.mindmap_tree,
                                &mut input.app_scene,
                                renderer,
                            );
                        }
                        ClickHit::PortalMarker { edge, endpoint }
                        | ClickHit::PortalText { edge, endpoint } => {
                            // Double-click on icon or text both
                            // jump to the partner endpoint — they
                            // share the same endpoint identity
                            // and the same "navigate" intent.
                            let other_id = if *endpoint == edge.from_id {
                                edge.to_id.clone()
                            } else {
                                edge.from_id.clone()
                            };
                            if let Some(node) = input.document.mindmap.nodes.get(&other_id) {
                                let target = glam::Vec2::new(
                                    node.position.x as f32
                                        + node.size.width as f32 * 0.5,
                                    node.position.y as f32
                                        + node.size.height as f32 * 0.5,
                                );
                                renderer.set_camera_center(target);
                            }
                            input.document.selection = SelectionState::Edge(
                                crate::application::document::EdgeRef::new(
                                    &edge.from_id,
                                    &edge.to_id,
                                    &edge.edge_type,
                                ),
                            );
                            rebuild_all(&input.document, &mut input.mindmap_tree, &mut input.app_scene, renderer);
                        }
                        ClickHit::EdgeLabel(edge_key) => {
                            // Edge label double-click is a parity
                            // placeholder on WASM. Native opens the
                            // inline label editor; WASM's modal
                            // editor path isn't available here yet,
                            // so for now we commit the
                            // `EdgeLabel` selection and let the
                            // user fall back to the `/label edit`
                            // console verb. Single-click already
                            // produces the same selection below,
                            // so dbl-click effectively matches
                            // single-click behaviour on WASM until
                            // the modal lands.
                            let er = crate::application::document::EdgeRef::new(
                                edge_key.from_id.as_str(),
                                edge_key.to_id.as_str(),
                                edge_key.edge_type.as_str(),
                            );
                            input.document.selection = SelectionState::EdgeLabel(
                                crate::application::document::EdgeLabelSel::new(er),
                            );
                            rebuild_all(&input.document, &mut input.mindmap_tree, &mut input.app_scene, renderer);
                        }
                        ClickHit::Empty => {
                            let allow_create = !matches!(
                                input.document.selection,
                                SelectionState::Edge(_)
                            );
                            if allow_create {
                                let new_id = input.document.create_orphan_and_select(canvas_pos);
                                rebuild_all(&input.document, &mut input.mindmap_tree, &mut input.app_scene, renderer);
                                open_text_edit(
                                    &new_id, true,
                                    &mut input.document,
                                    &mut input.text_edit_state,
                                    &mut input.mindmap_tree,
                                    &mut input.app_scene,
                                    renderer,
                                );
                            }
                        }
                    }
                    suppress_for_events.set(input.text_edit_state.is_open());
                    return;
                }

                input.pending_click = if let Some(id) = hit_node.clone() {
                    PendingClick::Node(id)
                } else if let Some((key, endpoint)) = portal_text_hit.clone() {
                    // Portal **text** click — committed to
                    // `SelectionState::PortalText` on mouse-up.
                    PendingClick::PortalText {
                        edge_key: key,
                        endpoint_node_id: endpoint,
                    }
                } else if let Some((key, endpoint)) = portal_icon_hit.clone() {
                    // Portal **icon** click — committed to
                    // `SelectionState::PortalLabel` on mouse-up.
                    // Double-click already fired above so a
                    // pending marker click can only mean "select
                    // this label".
                    PendingClick::PortalMarker {
                        edge_key: key,
                        endpoint_node_id: endpoint,
                    }
                } else if let Some(key) = edge_label_hit.clone() {
                    // Edge label click — committed to
                    // `SelectionState::EdgeLabel` on mouse-up.
                    PendingClick::EdgeLabel(key)
                } else {
                    PendingClick::Empty
                };
                input.last_click = Some(LastClick {
                    time: now,
                    screen_pos: input.cursor_pos,
                    hit: click_hit,
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
                        &mut input.app_scene,
                        renderer,
                    );
                    suppress_for_events.set(false);
                    return;
                }

                // Plain selection click
                input.document.selection = match pending {
                    PendingClick::Node(node_id) => SelectionState::Single(node_id),
                    PendingClick::PortalMarker {
                        edge_key,
                        endpoint_node_id,
                    } => SelectionState::PortalLabel(
                        crate::application::document::PortalLabelSel {
                            edge_key,
                            endpoint_node_id,
                        },
                    ),
                    PendingClick::PortalText {
                        edge_key,
                        endpoint_node_id,
                    } => SelectionState::PortalText(
                        crate::application::document::PortalLabelSel {
                            edge_key,
                            endpoint_node_id,
                        },
                    ),
                    PendingClick::EdgeLabel(edge_key) => {
                        let er = crate::application::document::EdgeRef::new(
                            edge_key.from_id.as_str(),
                            edge_key.to_id.as_str(),
                            edge_key.edge_type.as_str(),
                        );
                        SelectionState::EdgeLabel(
                            crate::application::document::EdgeLabelSel::new(er),
                        )
                    }
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
                // A zoom mid-click invalidates the pending selection:
                // the canvas coord the user pressed over has shifted
                // to a new screen position, so committing the pending
                // click on the eventual mouse-up would select whatever
                // now sits under the release cursor — not what the
                // user pressed on. Clear it so release falls through
                // to empty-click handling.
                input.pending_click = PendingClick::None;
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

/// HTTP-fetch a mindmap JSON file. Maps are bundled into the page
/// origin by trunk's `copy-dir` directive in `web/index.html`.
async fn fetch_map_json(url: &str) -> Result<String, String> {
    use wasm_bindgen::JsCast;
    let window = web_sys::window().ok_or("no global window")?;
    let promise = window.fetch_with_str(url);
    let resp_value = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(|e| format!("fetch failed: {:?}", e))?;
    let resp: web_sys::Response = resp_value
        .dyn_into()
        .map_err(|_| "fetch did not return a Response".to_string())?;
    if !resp.ok() {
        return Err(format!("HTTP {} {}", resp.status(), resp.status_text()));
    }
    let text_promise = resp
        .text()
        .map_err(|e| format!("Response::text() failed: {:?}", e))?;
    wasm_bindgen_futures::JsFuture::from(text_promise)
        .await
        .map_err(|e| format!("reading response body failed: {:?}", e))?
        .as_string()
        .ok_or_else(|| "response body was not a string".to_string())
}
