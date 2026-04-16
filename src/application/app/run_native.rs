//! Native event-loop body for [`super::Application::run`]. Lifted
//! verbatim out of the pre-split `app/mod.rs` so the event-loop
//! logic has its own file without changing any observable
//! behaviour — every identifier the body reaches is either owned by
//! the `Application` value passed in, imported via `use super::*`,
//! or a fully-qualified `crate::`/`baumhard::` path.

#![cfg(not(target_arch = "wasm32"))]

use super::*;

/// Run the native event loop against `app`. Called by the
/// `Application::run` dispatcher on every non-WASM target. Never
/// returns (winit's event loop owns the thread); on shutdown the
/// closure returns which lets winit exit the loop and the
/// `expect(...)` at the bottom propagates any winit-internal
/// failure up through `main`.
pub(super) fn run(mut app: Application) {
use baumhard::mindmap::tree_builder::MindMapTree;

// Single-threaded architecture: App owns the Renderer directly
baumhard::font::fonts::init();

let unsafe_target = unsafe { SurfaceTargetUnsafe::from_window(app.window.as_ref()) }
    .expect("Failed to create a SurfaceTargetUnsafe");
let instance = Instance::default();
let surface = unsafe { instance.create_surface_unsafe(unsafe_target) }.unwrap();

let mut renderer = block_on(Renderer::new(
    instance,
    surface,
    Arc::clone(&app.window),
));

// Configure initial surface size
let size = app.window.inner_size();
renderer.process_decree(RenderDecree::SetSurfaceSize(size.width, size.height));

// Load mindmap — document and tree persist for interactive use
let mut document: Option<MindMapDocument> = None;
let mut mindmap_tree: Option<MindMapTree> = None;
// Phase 4(B) keyed incremental rebuild: the document-side cache
// of per-edge pre-clip sample geometry. Populated lazily by
// `build_scene_with_cache`; cleared by `rebuild_all` so any
// structural change to the map forces a fresh scene build.
// Lives in the event loop state next to `mindmap_tree` so its
// lifetime matches the interactive session. It is a pure view
// optimisation — nothing about the model depends on it.
let mut scene_cache = baumhard::mindmap::scene_cache::SceneConnectionCache::new();
// App-level scene host. Owns the canvas-space tree for
// borders today (registered via `update_border_tree_*`) and
// grows to host the console / color-picker overlays in
// Sessions 3 / 4 of the unified-rendering refactor. Kept
// next to `mindmap_tree` so all tree-shaped components
// share the interactive-session lifetime.
let mut app_scene = crate::application::scene_host::AppScene::new();

match MindMapDocument::load(&app.options.mindmap_path) {
    Ok(mut doc) => {
        doc.build_mutation_registry();
        // Canvas background: resolve through theme
        // variables so `"var(--bg)"` works, then hand off
        // to the renderer as the render-pass clear color.
        // Replaces the previously-hardcoded black clear.
        let vars = &doc.mindmap.canvas.theme_variables;
        let resolved_bg = baumhard::util::color::resolve_var(
            &doc.mindmap.canvas.background_color,
            vars,
        );
        renderer.set_clear_color_from_hex(resolved_bg);

        // Nodes: build Baumhard tree from MindMap hierarchy
        let tree = doc.build_tree();
        renderer.rebuild_buffers_from_tree(&tree.tree);
        renderer.fit_camera_to_tree(&tree.tree);

        // Connections + borders: flat pipeline from RenderScene.
        // `fit_camera_to_tree` above settled the zoom, so pass
        // it through — the scene builder sizes connection
        // glyphs against the actual final zoom rather than the
        // default-init value.
        let scene = doc.build_scene(renderer.camera_zoom());
        update_connection_tree(&scene, &mut app_scene);
        update_border_tree_static(&doc, &mut app_scene);
        update_portal_tree(
            &doc,
            &std::collections::HashMap::new(),
            &mut app_scene,
            &mut renderer,
        );
        update_connection_label_tree(&scene, &mut app_scene, &mut renderer);
        flush_canvas_scene_buffers(&mut app_scene, &mut renderer);

        mindmap_tree = Some(tree);
        document = Some(doc);
    }
    Err(e) => {
        log::error!("{}", e);
    }
}

// Start rendering
renderer.process_decree(RenderDecree::StartRender);

// Input state
let mut cursor_pos: (f64, f64) = (0.0, 0.0);
let mut drag_state = DragState::None;
let mut app_mode = AppMode::Normal;
let mut console_state = ConsoleState::Closed;
// Cross-session history loaded from disk on startup; appended
// to on every Enter; written back on close. See
// `load_console_history` / `save_console_history`.
let mut console_history: Vec<String> = load_console_history();
let mut label_edit_state = LabelEditState::Closed;
let mut text_edit_state = TextEditState::Closed;
let mut color_picker_state =
    crate::application::color_picker::ColorPickerState::Closed;
// Session 7A: tracks the previous left-click-down for
// double-click detection. Cleared after a double-click fires.
let mut last_click: Option<LastClick> = None;
let mut hovered_node: Option<String> = None;
let mut modifiers = ModifiersState::empty();
// True while the cursor is hovering a node with any trigger
// bindings (a "button"). Tracked so we only call set_cursor on
// transitions instead of every CursorMoved event.
let mut cursor_is_hand = false;

// Phase 4(E): the governing-invariant throttle. Per-frame
// work in the drag path feeds its measured duration into this
// tracker; when the moving average crosses the refresh
// budget, `should_drain()` starts returning false on some
// frames, coalescing multiple ticks' worth of pending delta
// into a single drain. Responsiveness is preserved because
// input accumulation keeps running every tick (the mouse
// cursor never lags; only the dragged node briefly does).
// The throttle is reset at drag end so a fresh drag starts
// with n = 1.
let mut mutation_throttle = MutationFrequencyThrottle::with_default_budget();
// Picker hover gate: every cursor-move into the picker
// updates `state.{hue_deg,sat,val}` + `doc.color_picker_preview`
// synchronously (cheap, no shaping), but the actual scene
// + overlay rebuild is deferred to the `AboutToWait`
// drain so it runs at most once per display frame and
// self-throttles further if it falls behind. Same
// pattern as the drag path's `mutation_throttle` /
// `pending_delta` pair.
let mut picker_throttle = MutationFrequencyThrottle::with_default_budget();
let mut picker_dirty: bool = false;

// Resolve keybindings once at startup. Users can rebind any key
// by shipping a `keybinds.json` (see `keybinds.rs` for the format).
let mut keybinds: ResolvedKeybinds = app.options.keybind_config.resolve();

app.event_loop.run(move |event, _window_target| {
    _ = (&app.window, &mut app.options);

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
            // Glyph-wheel color picker caches its layout in
            // ColorPickerState::Open { layout, .. }; the
            // cached values include the screen-space backdrop
            // and per-glyph positions, so a resize would
            // leave hit-tests aimed at the old geometry and
            // the renderer's overlay buffers anchored at the
            // pre-resize coordinates. Re-emit both off the
            // new surface dimensions so picker stays usable
            // after a resize. Mutually exclusive with the
            // palette + label edit modals so only one branch
            // ever fires.
            if color_picker_state.is_open() {
                if let Some(doc) = document.as_ref() {
                    rebuild_color_picker_overlay(
                        &mut color_picker_state,
                        doc,
                        &mut app_scene,
                        &mut renderer,
                    );
                }
            }
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
            event_mouse_click::handle_mouse_input(
                state,
                button,
                cursor_pos,
                &modifiers,
                &mut document,
                &mut mindmap_tree,
                &mut app_scene,
                &mut renderer,
                &mut scene_cache,
                &mut drag_state,
                &mut app_mode,
                &mut console_state,
                &console_history,
                &mut label_edit_state,
                &mut text_edit_state,
                &mut color_picker_state,
                &mut last_click,
                &mut hovered_node,
                &mut mutation_throttle,
                &mut picker_dirty,
            );
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
            let prev_pos = cursor_pos;
            cursor_pos = (position.x, position.y);

            // Glyph-wheel color picker hover preview. Routes
            // mouse-over to the picker hit-test, updates the
            // current HSV in place, and lives-previews the
            // change on the affected edge/portal. In
            // Standalone mode with no active gesture and
            // the cursor outside the backdrop the move
            // falls through so the canvas's own hover
            // (button-node cursor, etc.) keeps working.
            //
            // Guard on `DragState::None`: if a canvas-side
            // drag (pan / node move / edge-handle drag /
            // rubber-band / pending threshold) is already in
            // flight, do not route the move to the picker.
            // The picker would otherwise swallow the move
            // whenever the cursor crossed its UI, freezing
            // the drag until the cursor left again. A
            // press that the picker consumed never sets
            // `drag_state` away from `None`, and an active
            // wheel-move/wheel-resize gesture on the picker
            // also lives entirely in `drag_state == None`,
            // so this guard can't steal events from the
            // picker's own interactions.
            if color_picker_state.is_open()
                && matches!(drag_state, DragState::None)
            {
                let consumed = if let Some(doc) = document.as_mut() {
                    handle_color_picker_mouse_move(
                        cursor_pos,
                        &mut color_picker_state,
                        doc,
                        &mut picker_dirty,
                    )
                } else {
                    true
                };
                if consumed {
                    return;
                }
            }

            // Reparent or Connect mode: hit-test under cursor to update the hover
            // target highlight. Skip the regular drag-state handling (no drag in
            // these modes).
            if matches!(app_mode, AppMode::Reparent { .. } | AppMode::Connect { .. }) {
                let new_hover = mindmap_tree.as_mut().and_then(|tree| {
                    let canvas_pos = renderer.screen_to_canvas(
                        cursor_pos.0 as f32, cursor_pos.1 as f32,
                    );
                    hit_test(canvas_pos, tree)
                });
                if new_hover != hovered_node {
                    hovered_node = new_hover;
                    if let Some(doc) = document.as_ref() {
                        rebuild_all_with_mode(
                            doc, &app_mode, hovered_node.as_deref(),
                            &mut mindmap_tree, &mut app_scene, &mut renderer,
                        );
                    }
                }
                return;
            }

            // Hand cursor over button-like nodes (nodes with any
            // trigger bindings). Only recomputed when idle — during
            // a drag the cursor should stay as-is.
            if matches!(drag_state, DragState::None) {
                let over_button = match (document.as_ref(), mindmap_tree.as_mut()) {
                    (Some(doc), Some(tree)) => {
                        let canvas_pos = renderer.screen_to_canvas(
                            cursor_pos.0 as f32, cursor_pos.1 as f32,
                        );
                        hit_test(canvas_pos, tree)
                            .and_then(|id| doc.mindmap.nodes.get(&id))
                            .map(|n| !n.trigger_bindings.is_empty())
                            .unwrap_or(false)
                    }
                    _ => false,
                };
                if over_button != cursor_is_hand {
                    app.window.set_cursor(if over_button {
                        CursorIcon::Pointer
                    } else {
                        CursorIcon::Default
                    });
                    cursor_is_hand = over_button;
                }
            }

            match &mut drag_state {
                DragState::Panning => {
                    let dx = cursor_pos.0 - prev_pos.0;
                    let dy = cursor_pos.1 - prev_pos.1;
                    renderer.process_decree(RenderDecree::CameraPan(dx as f32, dy as f32));
                }
                DragState::MovingNode { total_delta, pending_delta, .. } => {
                    // Convert screen delta to canvas delta and accumulate.
                    // Actual mutation + rebuild happens in AboutToWait (once per frame).
                    let old_canvas = renderer.screen_to_canvas(prev_pos.0 as f32, prev_pos.1 as f32);
                    let new_canvas = renderer.screen_to_canvas(cursor_pos.0 as f32, cursor_pos.1 as f32);
                    let delta = new_canvas - old_canvas;

                    *total_delta += delta;
                    *pending_delta += delta;
                }
                DragState::DraggingEdgeHandle { total_delta, pending_delta, .. } => {
                    // Same accumulation pattern as MovingNode —
                    // actual edge mutation + buffer rebuild
                    // happens in `AboutToWait`. Keeps the
                    // CursorMoved path cheap so fast mouse
                    // motion doesn't bottleneck on scene builds.
                    let old_canvas = renderer.screen_to_canvas(prev_pos.0 as f32, prev_pos.1 as f32);
                    let new_canvas = renderer.screen_to_canvas(cursor_pos.0 as f32, cursor_pos.1 as f32);
                    let delta = new_canvas - old_canvas;

                    *total_delta += delta;
                    *pending_delta += delta;
                }
                DragState::Pending { start_pos, hit_node, hit_edge_handle } => {
                    let dist_x = cursor_pos.0 - start_pos.0;
                    let dist_y = cursor_pos.1 - start_pos.1;
                    if dist_x * dist_x + dist_y * dist_y > 25.0 {
                        // Past threshold — decide what kind of drag
                        // this is. Handle grabs take precedence over
                        // node hits so that clicking a control-point
                        // handle that happens to overlap a node
                        // still enters the reshape flow.
                        if let Some((edge_ref, handle_kind)) = hit_edge_handle.take() {
                            // Grab the pre-edit snapshot + start
                            // position so the drain loop can do
                            // absolute-positioning math.
                            if let Some(doc) = document.as_mut() {
                                if let Some(original) = doc
                                    .mindmap
                                    .edges
                                    .iter()
                                    .find(|e| edge_ref.matches(e))
                                    .cloned()
                                {
                                    let canvas_pos = renderer.screen_to_canvas(
                                        start_pos.0 as f32,
                                        start_pos.1 as f32,
                                    );
                                    let start_handle_pos = doc
                                        .hit_test_edge_handle(
                                            canvas_pos,
                                            &edge_ref,
                                            f32::INFINITY,
                                        )
                                        .map(|(_, p)| p)
                                        .unwrap_or(canvas_pos);
                                    // Phase 4(B) invariant: a new
                                    // drag starts with a clean
                                    // scene cache — see the
                                    // MovingNode branch for the
                                    // full rationale.
                                    scene_cache.clear();
                                    drag_state = DragState::DraggingEdgeHandle {
                                        edge_ref,
                                        handle: handle_kind,
                                        original,
                                        start_handle_pos,
                                        total_delta: Vec2::ZERO,
                                        pending_delta: Vec2::ZERO,
                                    };
                                    return;
                                }
                            }
                        }
                        if let Some(node_id) = hit_node.take() {
                            // Ensure the dragged node is selected
                            if let Some(doc) = document.as_mut() {
                                if !doc.selection.is_selected(&node_id) {
                                    doc.selection = SelectionState::Single(node_id.clone());
                                    if let Some(tree) = mindmap_tree.as_mut() {
                                        let mut new_tree = doc.build_tree();
                                        apply_tree_highlights(
                                            &mut new_tree,
                                            doc.selection
                                                .selected_ids()
                                                .into_iter()
                                                .map(|id| (id, HIGHLIGHT_COLOR)),
                                        );
                                        renderer.rebuild_buffers_from_tree(&new_tree.tree);
                                        *tree = new_tree;
                                    }
                                }
                            }
                            // Shift+drag: move all selected nodes together
                            let node_ids = if modifiers.shift_key() {
                                if let Some(doc) = document.as_ref() {
                                    let mut ids: Vec<String> = doc.selection.selected_ids()
                                        .iter().map(|s| s.to_string()).collect();
                                    if !ids.contains(&node_id) {
                                        ids.push(node_id);
                                    }
                                    ids
                                } else {
                                    vec![node_id]
                                }
                            } else {
                                vec![node_id]
                            };
                            // Phase 4(B): start each drag with a
                            // clean scene cache. Between drags the
                            // model may have changed structurally
                            // (undo, reparent, edge CRUD, node
                            // edit) and cached entries would not
                            // reflect the current map. Clearing
                            // here means the first drag frame
                            // re-populates the cache from scratch
                            // and subsequent frames get the
                            // incremental-rebuild benefit.
                            scene_cache.clear();
                            drag_state = DragState::MovingNode {
                                node_ids,
                                total_delta: Vec2::ZERO,
                                pending_delta: Vec2::ZERO,
                                individual: modifiers.alt_key(),
                            };
                        } else if modifiers.shift_key() {
                            // Shift+drag on empty space: rubber-band selection
                            let start_canvas = renderer.screen_to_canvas(
                                start_pos.0 as f32, start_pos.1 as f32,
                            );
                            let current_canvas = renderer.screen_to_canvas(
                                cursor_pos.0 as f32, cursor_pos.1 as f32,
                            );
                            drag_state = DragState::SelectingRect {
                                start_canvas,
                                current_canvas,
                            };
                        } else {
                            drag_state = DragState::Panning;
                            let dx = cursor_pos.0 - prev_pos.0;
                            let dy = cursor_pos.1 - prev_pos.1;
                            renderer.process_decree(RenderDecree::CameraPan(dx as f32, dy as f32));
                        }
                    }
                }
                DragState::SelectingRect { ref mut current_canvas, .. } => {
                    *current_canvas = renderer.screen_to_canvas(
                        cursor_pos.0 as f32, cursor_pos.1 as f32,
                    );
                }
                DragState::None => {}
            }
        }
        ////////////////////
        //// KEYBOARD ////
        //////////////////
        Event::WindowEvent {
            event: WindowEvent::ModifiersChanged(mods),
            ..
        } => {
            modifiers = mods.state();
        }
        Event::WindowEvent {
            event: WindowEvent::KeyboardInput {
                event: KeyEvent {
                    logical_key,
                    state: ElementState::Pressed,
                    ..
                },
                ..
            },
            ..
        } => {
            let key_name = crate::application::keybinds::key_to_name(&logical_key);

            // When the console is open, it steals all
            // keyboard input. Character keys insert at the
            // cursor, Tab triggers completion, Up/Down walks
            // history, Enter parses + executes, Escape
            // closes. Regular hotkeys are suppressed until
            // the console closes.
            if console_state.is_open() {
                handle_console_key(
                    &key_name,
                    &logical_key,
                    modifiers.control_key(),
                    &mut console_state,
                    &mut console_history,
                    &mut label_edit_state,
                    &mut color_picker_state,
                    &mut document,
                    &mut mindmap_tree,
                    &mut app_scene,
                    &mut renderer,
                    &mut scene_cache,
                    &mut keybinds,
                );
                return;
            }

            // Glyph-wheel color picker key handling.
            // Mutually exclusive with console and label-edit
            // for the keys it claims (Esc, Enter, h/s/v/
            // H/S/V). Any other key — notably the console
            // trigger `/` — falls through so the Standalone
            // persistent palette doesn't deadlock the user
            // out of the normal keybind dispatch.
            if color_picker_state.is_open() {
                let consumed = if let Some(doc) = document.as_mut() {
                    handle_color_picker_key(
                        &key_name,
                        modifiers.control_key(),
                        modifiers.shift_key(),
                        modifiers.alt_key(),
                        &keybinds,
                        &mut color_picker_state,
                        doc,
                        &mut mindmap_tree,
                        &mut picker_dirty,
                        &mut app_scene,
                        &mut renderer,
                    )
                } else {
                    false
                };
                if consumed {
                    return;
                }
            }

            // Session 6D: inline label edit modal. Steals keys
            // the same way the console does. Escape discards,
            // Enter commits, Backspace pops, character keys
            // append.
            if label_edit_state.is_open() {
                if let Some(doc) = document.as_mut() {
                    handle_label_edit_key(
                        &key_name,
                        &logical_key,
                        &mut label_edit_state,
                        doc,
                        &mut mindmap_tree,
                        &mut app_scene,
                        &mut renderer,
                    );
                }
                return;
            }

            // Session 7A: inline node text editor. Steals keys
            // the same way the console / label-edit modals do.
            // Enter and Tab are literal characters inside the
            // editor — this is a multi-line paragraph editor,
            // not an outliner. Esc cancels; commit is via
            // click-outside in the mouse handler.
            if text_edit_state.is_open() {
                if let Some(doc) = document.as_mut() {
                    handle_text_edit_key(
                        &key_name,
                        &logical_key,
                        &mut text_edit_state,
                        doc,
                        &mut mindmap_tree,
                        &mut app_scene,
                        &mut renderer,
                    );
                }
                return;
            }

            let action = key_name.as_deref().and_then(|k| {
                keybinds.action_for(
                    k,
                    modifiers.control_key(),
                    modifiers.shift_key(),
                    modifiers.alt_key(),
                )
            });

            match action {
                Some(Action::OpenConsole) => {
                    // Toggle: already open → close. Otherwise
                    // construct a fresh state seeded with the
                    // persisted history. Rebuild the overlay
                    // so the frame appears immediately.
                    if console_state.is_open() {
                        save_console_history(&console_history);
                        console_state = ConsoleState::Closed;
                        renderer.rebuild_console_overlay_buffers(&mut app_scene, None);
                    } else {
                        console_state = ConsoleState::open(console_history.clone());
                        if let Some(doc) = document.as_ref() {
                            rebuild_console_overlay(
                                &console_state, doc, &mut app_scene, &mut renderer, &keybinds,
                            );
                        }
                    }
                    return;
                }
                Some(Action::Undo) => {
                    if let Some(doc) = document.as_mut() {
                        // Ctrl+Z during an animation
                        // fast-forwards to completion so the
                        // animation's own undo entry lands on
                        // the stack before we pop — Ctrl+Z
                        // reverses the animation's effect in
                        // one keystroke, matching the
                        // post-completion Ctrl+Z behaviour.
                        if doc.has_active_animations() {
                            doc.fast_forward_animations(mindmap_tree.as_mut());
                        }
                        if doc.undo() {
                            rebuild_all(doc, &mut mindmap_tree, &mut app_scene, &mut renderer);
                        }
                    }
                }
                Some(Action::CancelMode) => {
                    if matches!(app_mode, AppMode::Reparent { .. } | AppMode::Connect { .. }) {
                        app_mode = AppMode::Normal;
                        hovered_node = None;
                        // Session 7A: clear any stale click so a
                        // post-mode click doesn't get retroactively
                        // paired with a pre-mode click into a
                        // spurious double-click.
                        last_click = None;
                        if let Some(doc) = document.as_ref() {
                            rebuild_all_with_mode(
                                doc, &app_mode, hovered_node.as_deref(),
                                &mut mindmap_tree, &mut app_scene, &mut renderer,
                            );
                        }
                    }
                }
                Some(Action::EnterReparentMode) => {
                    if let Some(doc) = document.as_ref() {
                        let sel: Vec<String> = doc.selection.selected_ids()
                            .iter().map(|s| s.to_string()).collect();
                        if !sel.is_empty() {
                            app_mode = AppMode::Reparent { sources: sel };
                            hovered_node = None;
                            last_click = None;
                            rebuild_all_with_mode(
                                doc, &app_mode, hovered_node.as_deref(),
                                &mut mindmap_tree, &mut app_scene, &mut renderer,
                            );
                        }
                    }
                }
                Some(Action::EnterConnectMode) => {
                    if let Some(doc) = document.as_ref() {
                        if let SelectionState::Single(source) = &doc.selection {
                            app_mode = AppMode::Connect { source: source.clone() };
                            hovered_node = None;
                            last_click = None;
                            rebuild_all_with_mode(
                                doc, &app_mode, hovered_node.as_deref(),
                                &mut mindmap_tree, &mut app_scene, &mut renderer,
                            );
                        }
                    }
                }
                Some(Action::DeleteSelection) => {
                    if let Some(doc) = document.as_mut() {
                        if doc.apply_delete_selection() {
                            rebuild_all(doc, &mut mindmap_tree, &mut app_scene, &mut renderer);
                        }
                    }
                }
                Some(Action::CreateOrphanNode) => {
                    if let Some(doc) = document.as_mut() {
                        let canvas_pos = renderer.screen_to_canvas(
                            cursor_pos.0 as f32, cursor_pos.1 as f32,
                        );
                        doc.create_orphan_and_select(canvas_pos);
                        rebuild_all(doc, &mut mindmap_tree, &mut app_scene, &mut renderer);
                    }
                }
                Some(Action::OrphanSelection) => {
                    if let Some(doc) = document.as_mut() {
                        if doc.apply_orphan_selection_with_undo() {
                            rebuild_all(doc, &mut mindmap_tree, &mut app_scene, &mut renderer);
                        }
                    }
                }
                Some(a @ (Action::EditSelection | Action::EditSelectionClean)) => {
                    // Open the text editor on the selected single
                    // node. `EditSelectionClean` opens with an empty
                    // buffer (the "clean slate" retype gesture);
                    // `EditSelection` opens on the node's current
                    // text with cursor at end. The text-editor
                    // steal at the top of keyboard dispatch means
                    // these never fire while the editor is already
                    // open, so Enter/Backspace stay literal inside
                    // the editor.
                    let clean = matches!(a, Action::EditSelectionClean);
                    if let Some(doc) = document.as_mut() {
                        if let SelectionState::Single(id) = &doc.selection {
                            let nid = id.clone();
                            open_text_edit(
                                &nid,
                                clean,
                                doc,
                                &mut text_edit_state,
                                &mut mindmap_tree,
                                &mut app_scene,
                                &mut renderer,
                            );
                        }
                    }
                }
                Some(Action::Copy) | Some(Action::Cut) => {
                    // Dispatch to the current selection's
                    // HandlesCopy / HandlesCut. Today every
                    // TargetView variant returns NotApplicable —
                    // this arm exists so the keybind path is wired
                    // and future impls only need to fill in the
                    // TargetView arms.
                    use crate::application::console::traits::{
                        selection_targets, view_for, ClipboardContent,
                        HandlesCopy, HandlesCut,
                    };
                    let is_cut = matches!(action, Some(Action::Cut));
                    if let Some(doc) = document.as_mut() {
                        let targets = selection_targets(&doc.selection);
                        // First applicable target wins — multi-select
                        // copy takes the first item that produces text.
                        for tid in &targets {
                            let mut view = view_for(doc, tid);
                            let content = if is_cut {
                                view.clipboard_cut()
                            } else {
                                view.clipboard_copy()
                            };
                            if let ClipboardContent::Text(text) = content {
                                crate::application::clipboard::write_clipboard(&text);
                                break;
                            }
                        }
                    }
                }
                Some(Action::Paste) => {
                    // Dispatch to the current selection's
                    // HandlesPaste. Today every TargetView variant
                    // returns NotApplicable — wired for future
                    // impls.
                    use crate::application::console::traits::{
                        selection_targets, view_for, HandlesPaste,
                        Outcome,
                    };
                    if let Some(text) = crate::application::clipboard::read_clipboard() {
                        if let Some(doc) = document.as_mut() {
                            let targets = selection_targets(&doc.selection);
                            let mut any_applied = false;
                            for tid in &targets {
                                let mut view = view_for(doc, tid);
                                if let Outcome::Applied = view.clipboard_paste(&text) {
                                    any_applied = true;
                                }
                            }
                            if any_applied {
                                rebuild_all(doc, &mut mindmap_tree, &mut app_scene, &mut renderer);
                            }
                        }
                    }
                }
                Some(Action::SaveDocument) => {
                    // Quick-save to the document's bound file
                    // path. If no path is bound (e.g. after `new`
                    // without a path), this is a no-op aside from
                    // a status message — the user has to invoke
                    // `save <path>` from the console first.
                    if let Some(doc) = document.as_mut() {
                        save_document_to_bound_path(doc, &mut console_state);
                    }
                }
                Some(_) => {
                    // Component-scoped action resolved at Document
                    // level — should not happen in practice (the
                    // modal handlers above claim their contexts
                    // first). Ignore silently.
                }
                None => {
                    // No built-in action matched — try the
                    // user-defined `custom_mutation_bindings`.
                    // If a combo resolves to a mutation id,
                    // apply it on the current Single
                    // selection. Non-Single selections quietly
                    // skip (no status-bar UI yet).
                    if let Some(id) = key_name.as_deref().and_then(|k| {
                        keybinds
                            .custom_mutation_for(
                                k,
                                modifiers.control_key(),
                                modifiers.shift_key(),
                                modifiers.alt_key(),
                            )
                            .map(|s| s.to_string())
                    }) {
                        if let Some(doc) = document.as_mut() {
                            if let SelectionState::Single(nid) = doc.selection.clone() {
                                let mutation =
                                    doc.mutation_registry.get(&id).cloned();
                                if let (Some(m), Some(tree)) =
                                    (mutation, mindmap_tree.as_mut())
                                {
                                    doc.apply_custom_mutation(&m, &nid, tree);
                                    scene_cache.clear();
                                    rebuild_all(doc, &mut mindmap_tree, &mut app_scene, &mut renderer);
                                }
                            }
                        }
                    }
                }
            }
        }
        Event::AboutToWait => {
            if let DragState::MovingNode { ref node_ids, ref mut pending_delta, ref total_delta, individual, .. } = drag_state {
                drain_frame::drain_moving_node(
                    node_ids,
                    pending_delta,
                    total_delta,
                    individual,
                    &mut mindmap_tree,
                    &document,
                    &mut app_scene,
                    &mut renderer,
                    &mut scene_cache,
                    &mut mutation_throttle,
                );
            }
            if let DragState::DraggingEdgeHandle {
                ref edge_ref,
                ref mut handle,
                ref mut pending_delta,
                ref total_delta,
                ref start_handle_pos,
                ..
            } = drag_state {
                drain_frame::drain_edge_handle(
                    edge_ref,
                    handle,
                    pending_delta,
                    total_delta,
                    start_handle_pos,
                    &mut document,
                    &mut app_scene,
                    &mut renderer,
                    &mut scene_cache,
                    &mut mutation_throttle,
                );
            }
            if let DragState::SelectingRect { start_canvas, current_canvas } = &drag_state {
                drain_frame::drain_selecting_rect(
                    *start_canvas,
                    *current_canvas,
                    &document,
                    &mut mindmap_tree,
                    &mut renderer,
                );
            }

            drain_frame::drain_camera_geometry_rebuild(
                matches!(drag_state, DragState::MovingNode { .. }),
                &document,
                &mut app_scene,
                &mut renderer,
                &mut scene_cache,
            );

            drain_frame::drain_color_picker_hover(
                &mut picker_dirty,
                &mut picker_throttle,
                &mut color_picker_state,
                &mut document,
                &mut app_scene,
                &mut renderer,
            );

            drain_frame::drain_animation_tick(
                &mut document,
                &mut mindmap_tree,
                &mut app_scene,
                &mut renderer,
            );

            // Drive the render loop each frame
            renderer.process();
        }
        _ => {}
    }
}).expect("Some kind of unexpected error appears to have taken place")
}
