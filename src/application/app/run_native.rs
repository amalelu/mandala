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
            // The console swallows mouse clicks as a close
            // gesture. Clicking anywhere while open dismisses
            // the console without running a command, mirroring
            // Escape.
            if console_state.is_open() && state == ElementState::Pressed {
                save_console_history(&console_history);
                console_state = ConsoleState::Closed;
                renderer.rebuild_console_overlay_buffers(&mut app_scene, None);
                return;
            }

            // Glyph-wheel color picker click handling. The
            // picker captures both left- and right-mouse
            // buttons:
            // - LMB on a `DragAnchor` → wheel-move gesture;
            //   on any other hit → preview / commit / chip
            //   focus.
            // - RMB on a `DragAnchor` → wheel-resize
            //   gesture (drag away to grow, toward to shrink).
            //   RMB elsewhere is currently a no-op — only
            //   the empty backdrop region acts as the resize
            //   handle, mirroring the LMB-move convention.
            // Release of either button ends any active
            // gesture. In **Standalone** (persistent
            // palette) mode, clicks outside the picker
            // backdrop fall through to normal dispatch —
            // otherwise the user couldn't select anything
            // else while the palette was open. In
            // **Contextual** mode the picker captures
            // everything; outside-click cancels.
            if color_picker_state.is_open()
                && matches!(button, MouseButton::Left | MouseButton::Right)
            {
                let consumed = if state == ElementState::Pressed {
                    if let Some(doc) = document.as_mut() {
                        handle_color_picker_click(
                            cursor_pos,
                            button,
                            &mut color_picker_state,
                            doc,
                            &mut mindmap_tree,
                            &mut app_scene,
                            &mut renderer,
                        )
                    } else {
                        true
                    }
                } else {
                    // Release — end any active wheel gesture.
                    // If no gesture was active (e.g.
                    // Standalone + outside-press fell
                    // through), this is a no-op and the
                    // release should also fall through.
                    end_color_picker_gesture(&mut color_picker_state)
                };
                if consumed {
                    return;
                }
            }
            match button {
                MouseButton::Middle => {
                    if state == ElementState::Pressed {
                        drag_state = DragState::Panning;
                    } else {
                        drag_state = DragState::None;
                    }
                }
                MouseButton::Left => {
                    // In reparent or connect mode, left-click (release) is consumed as
                    // a "choose target" gesture and never transitions to Pending/drag.
                    if matches!(app_mode, AppMode::Reparent { .. }) {
                        if state == ElementState::Released {
                            handle_reparent_target_click(
                                cursor_pos,
                                &mut app_mode,
                                &mut hovered_node,
                                &mut document,
                                &mut mindmap_tree,
                                &mut app_scene,
                                &mut renderer,
                            );
                            // Session 7A: mode-exit via target
                            // click — clear any stale click so
                            // the first post-mode click can't
                            // be paired into a double-click.
                            last_click = None;
                        }
                        // Pressed: swallow — do not transition drag state
                    } else if matches!(app_mode, AppMode::Connect { .. }) {
                        if state == ElementState::Released {
                            handle_connect_target_click(
                                cursor_pos,
                                &mut app_mode,
                                &mut hovered_node,
                                &mut document,
                                &mut mindmap_tree,
                                &mut app_scene,
                                &mut renderer,
                            );
                            last_click = None;
                        }
                        // Pressed: swallow
                    } else if state == ElementState::Pressed {
                        // Hit test to determine if clicking on a node
                        let canvas_pos = renderer.screen_to_canvas(
                            cursor_pos.0 as f32,
                            cursor_pos.1 as f32,
                        );
                        let hit_node = mindmap_tree.as_ref().and_then(|tree| {
                            hit_test(canvas_pos, tree)
                        });

                        // Session 7A: double-click detection.
                        // If this press within the double-click
                        // window matches the previous one (same
                        // hit target, within time + distance),
                        // open the node text editor — double-click
                        // on a node edits it; on empty space (and
                        // no edge/portal selected) creates a new
                        // orphan and edits that.
                        //
                        // Guard: if the editor is already open on
                        // the same hit target, DO NOT re-open it
                        // — that would silently discard the
                        // in-progress buffer. Let the press fall
                        // through; the corresponding release
                        // will be swallowed as click-inside.
                        let now = now_ms();
                        let already_editing_same_target = text_edit_state
                            .node_id()
                            .map(|id| hit_node.as_deref() == Some(id))
                            .unwrap_or(false);
                        let is_dblclick = !already_editing_same_target
                            && last_click
                                .as_ref()
                                .map(|prev| is_double_click(prev, now, cursor_pos, &hit_node))
                                .unwrap_or(false);
                        if is_dblclick {
                            last_click = None;
                            if let Some(ref node_id) = hit_node {
                                if let Some(doc) = document.as_mut() {
                                    let nid = node_id.clone();
                                    doc.selection = SelectionState::Single(nid.clone());
                                    // rebuild_all first so the
                                    // selection highlight color
                                    // regions are applied to the
                                    // tree. open_text_edit's
                                    // subsequent apply_text_edit_to_tree
                                    // only touches the Text field
                                    // of the target node's
                                    // GlyphArea (via
                                    // DeltaGlyphArea's selective
                                    // field application) so the
                                    // highlight regions survive
                                    // untouched. If you ever add
                                    // more fields to the caret
                                    // delta, revisit this.
                                    rebuild_all(doc, &mut mindmap_tree, &mut app_scene, &mut renderer);
                                    open_text_edit(
                                        &nid,
                                        false,
                                        doc,
                                        &mut text_edit_state,
                                        &mut mindmap_tree,
                                        &mut app_scene,
                                        &mut renderer,
                                    );
                                }
                                return;
                            } else {
                                // Empty space: only create an
                                // orphan if no edge/portal was
                                // selected (otherwise the user
                                // was probably aiming at the
                                // selected edge/portal).
                                let allow_create = document
                                    .as_ref()
                                    .map(|d| !matches!(
                                        d.selection,
                                        SelectionState::Edge(_) | SelectionState::Portal(_)
                                    ))
                                    .unwrap_or(false);
                                if allow_create {
                                    if let Some(doc) = document.as_mut() {
                                        let new_id = doc.create_orphan_and_select(canvas_pos);
                                        rebuild_all(doc, &mut mindmap_tree, &mut app_scene, &mut renderer);
                                        open_text_edit(
                                            &new_id,
                                            true,
                                            doc,
                                            &mut text_edit_state,
                                            &mut mindmap_tree,
                                            &mut app_scene,
                                            &mut renderer,
                                        );
                                    }
                                    return;
                                }
                            }
                        }
                        last_click = Some(LastClick {
                            time: now,
                            screen_pos: cursor_pos,
                            hit: hit_node.clone(),
                        });

                        // If an edge is currently selected, check
                        // whether the cursor is over one of its
                        // grab-handles. This check has precedence
                        // over the node hit at threshold-cross
                        // time — see the `Pending` → drag
                        // transition below. Returns `None` if no
                        // edge is selected, nothing is in range,
                        // or the hit test infrastructure isn't
                        // ready yet.
                        let hit_edge_handle = match document.as_ref() {
                            Some(doc) => match &doc.selection {
                                SelectionState::Edge(er) => {
                                    let tol = EDGE_HANDLE_HIT_TOLERANCE_PX
                                        * renderer.canvas_per_pixel();
                                    doc.hit_test_edge_handle(canvas_pos, er, tol)
                                        .map(|(kind, _pos)| (er.clone(), kind))
                                }
                                _ => None,
                            },
                            None => None,
                        };
                        drag_state = DragState::Pending {
                            start_pos: cursor_pos,
                            hit_node,
                            hit_edge_handle,
                        };
                    } else {
                        // Released
                        match std::mem::replace(&mut drag_state, DragState::None) {
                            DragState::Pending { hit_node, .. } => {
                                // Session 7A: if the node text
                                // editor is open, the release
                                // decides whether to commit or
                                // swallow. If the release lands
                                // inside the edited node's AABB,
                                // keep editing (no commit, no
                                // selection change). Otherwise
                                // commit and fall through.
                                if text_edit_state.is_open() {
                                    let release_canvas = renderer.screen_to_canvas(
                                        cursor_pos.0 as f32,
                                        cursor_pos.1 as f32,
                                    );
                                    let inside = text_edit_state
                                        .node_id()
                                        .zip(mindmap_tree.as_ref())
                                        .map(|(id, tree)| {
                                            crate::application::document::point_in_node_aabb(
                                                release_canvas, id, tree,
                                            )
                                        })
                                        .unwrap_or(false);
                                    if inside {
                                        // Click-inside: keep
                                        // editing. Do NOT fall
                                        // through to handle_click
                                        // (that would change the
                                        // selection). Also do
                                        // not transition drag
                                        // state — the release
                                        // is fully consumed.
                                        return;
                                    }
                                    // Click-outside: commit the
                                    // edit first, then fall
                                    // through to the regular
                                    // click path so the new
                                    // selection lands.
                                    if let Some(doc) = document.as_mut() {
                                        close_text_edit(
                                            true,
                                            doc,
                                            &mut text_edit_state,
                                            &mut mindmap_tree,
                                            &mut app_scene,
                                            &mut renderer,
                                        );
                                    }
                                }
                                // Session 6D: if an edge is selected and
                                // the cursor hits its label, open the
                                // inline label editor instead of
                                // processing a regular click. Takes
                                // precedence over node / edge selection.
                                let mut entered_label_edit = false;
                                if hit_node.is_none() {
                                    // First, a read-only check to see
                                    // whether we should even call the
                                    // editor (hits the selected edge's
                                    // label AABB). Split from the
                                    // `open_label_edit` call so the
                                    // mutable borrow of `document`
                                    // doesn't conflict with the
                                    // immutable read.
                                    let label_edit_target: Option<crate::application::document::EdgeRef> =
                                        if let Some(doc) = document.as_ref() {
                                            if let SelectionState::Edge(er) = &doc.selection {
                                                let canvas_pos = renderer.screen_to_canvas(
                                                    cursor_pos.0 as f32,
                                                    cursor_pos.1 as f32,
                                                );
                                                let edge_key = baumhard::mindmap::scene_cache::EdgeKey::new(
                                                    &er.from_id,
                                                    &er.to_id,
                                                    &er.edge_type,
                                                );
                                                if renderer.hit_test_edge_label(canvas_pos, &edge_key) {
                                                    Some(er.clone())
                                                } else {
                                                    None
                                                }
                                            } else {
                                                None
                                            }
                                        } else {
                                            None
                                        };
                                    if let Some(er_clone) = label_edit_target {
                                        if let Some(doc) = document.as_mut() {
                                            open_label_edit(
                                                &er_clone,
                                                doc,
                                                &mut label_edit_state,
                                                &mut app_scene,
                                                &mut renderer,
                                            );
                                            entered_label_edit = true;
                                        }
                                    }
                                }
                                if !entered_label_edit {
                                    handle_click(
                                        hit_node,
                                        cursor_pos,
                                        modifiers.shift_key(),
                                        &mut document,
                                        &mut mindmap_tree,
                                        &mut app_scene,
                                        &mut renderer,
                                    );
                                }
                            }
                            DragState::MovingNode { node_ids, total_delta, pending_delta, individual } => {
                                // Flush any remaining pending delta to the tree before drop.
                                // This always runs regardless of the throttle — on release
                                // we want the final position committed in full, even if
                                // the throttle was mid-stretch skipping intermediate drains.
                                if pending_delta != Vec2::ZERO {
                                    if let Some(tree) = mindmap_tree.as_mut() {
                                        for nid in &node_ids {
                                            apply_drag_delta(tree, nid, pending_delta.x, pending_delta.y, !individual);
                                        }
                                    }
                                }
                                // Drop: sync to model, full rebuild, push undo
                                if let Some(doc) = document.as_mut() {
                                    let dx = total_delta.x as f64;
                                    let dy = total_delta.y as f64;
                                    let undo_data = doc.apply_move_multiple(&node_ids, dx, dy, individual);
                                    doc.undo_stack.push(UndoAction::MoveNodes {
                                        original_positions: undo_data,
                                    });
                                    doc.dirty = true;

                                    // Full rebuild from model
                                    rebuild_all(doc, &mut mindmap_tree, &mut app_scene, &mut renderer);
                                }
                                // Drag ended — reset the throttle so the next drag
                                // starts at n = 1 without inheriting any residual
                                // throttling from this one.
                                mutation_throttle.reset();
                            }
                            DragState::DraggingEdgeHandle { edge_ref, handle, original, start_handle_pos, total_delta, pending_delta: _ } => {
                                // The drain loop has been writing
                                // each new edge state directly
                                // into the model. Before release,
                                // flush one last write using the
                                // full `total_delta` (independent
                                // of any throttled pending drain)
                                // so the final committed state
                                // matches the cursor position
                                // exactly. Reaching this branch
                                // means the drag threshold was
                                // crossed, so push an EditEdge
                                // undo with the pre-drag snapshot
                                // unconditionally.
                                if let Some(doc) = document.as_mut() {
                                    apply_edge_handle_drag(
                                        doc,
                                        &edge_ref,
                                        handle,
                                        start_handle_pos,
                                        total_delta,
                                    );
                                    if let Some(idx) = doc.edge_index(&edge_ref) {
                                        doc.undo_stack.push(UndoAction::EditEdge {
                                            index: idx,
                                            before: original,
                                        });
                                        doc.dirty = true;
                                    }
                                    rebuild_all(doc, &mut mindmap_tree, &mut app_scene, &mut renderer);
                                }
                                mutation_throttle.reset();
                            }
                            DragState::SelectingRect { start_canvas, current_canvas } => {
                                // Finalize: select all nodes in the rectangle
                                renderer.clear_overlay_buffers();
                                if let (Some(doc), Some(tree)) = (document.as_mut(), mindmap_tree.as_ref()) {
                                    let hits = rect_select(start_canvas, current_canvas, tree);
                                    doc.selection = match hits.len() {
                                        0 => SelectionState::None,
                                        1 => SelectionState::Single(hits.into_iter().next().unwrap()),
                                        _ => SelectionState::Multi(hits),
                                    };
                                    rebuild_all(doc, &mut mindmap_tree, &mut app_scene, &mut renderer);
                                }
                            }
                            DragState::Panning | DragState::None => {}
                        }
                    }
                }
                _ => {}
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
                let new_hover = mindmap_tree.as_ref().and_then(|tree| {
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
                let over_button = match (document.as_ref(), mindmap_tree.as_ref()) {
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
                        &logical_key,
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
            // Flush any accumulated drag delta (once per frame, not per mouse event),
            // gated by the mutation-frequency throttle. When the moving
            // average of this block's work duration exceeds the refresh
            // budget, `should_drain()` starts returning false on some
            // frames — `pending_delta` stays intact and the next
            // successful drain folds in whatever motion arrived in the
            // meantime. This holds the governing invariant:
            // responsiveness is preserved at the cost of briefer
            // chunking in the visual update cadence.
            if let DragState::MovingNode { ref node_ids, ref mut pending_delta, ref total_delta, individual, .. } = drag_state {
                if *pending_delta != Vec2::ZERO && mutation_throttle.should_drain() {
                    let work_start = Instant::now();

                    if let Some(tree) = mindmap_tree.as_mut() {
                        for nid in node_ids {
                            apply_drag_delta(tree, nid, pending_delta.x, pending_delta.y, !individual);
                        }
                        renderer.rebuild_buffers_from_tree(&tree.tree);
                    }

                    // Rebuild connections and borders with position offsets.
                    //
                    // Phase 4(B): use the cache-aware scene build so
                    // only edges whose endpoints appear in `offsets`
                    // get re-sampled. The renderer's keyed rebuild
                    // methods then only re-shape the buffers for
                    // dirty elements; stable elements have just
                    // their `pos` patched in place.
                    if let Some(doc) = document.as_ref() {
                        let mut offsets: HashMap<String, (f32, f32)> = HashMap::new();
                        let delta = (total_delta.x, total_delta.y);
                        for nid in node_ids {
                            offsets.insert(nid.clone(), delta);
                            if !individual {
                                for desc_id in doc.mindmap.all_descendants(nid) {
                                    offsets.insert(desc_id, delta);
                                }
                            }
                        }

                        // The tree-path connection rebuild
                        // currently re-shapes every edge each
                        // frame. The per-edge incremental
                        // shaping cache that would let us
                        // narrow this to a "dirty" set
                        // (computed from `scene_cache.
                        // edges_touching(nid)` per moved node)
                        // is a known regression tracked
                        // separately; until it lands the full
                        // rebuild below is correct, and we
                        // skip the dead bookkeeping that would
                        // feed it.
                        let scene = doc.build_scene_with_cache(
                            &offsets,
                            &mut scene_cache,
                            renderer.camera_zoom(),
                        );

                        update_connection_tree(&scene, &mut app_scene);
                        // Borders go through the canvas-scene
                        // tree path; drag offsets land on the
                        // tree builder via this helper. The
                        // legacy keyed border path used to
                        // patch positions in place (cheap)
                        // while the tree path re-shapes every
                        // border (more expensive) — that's a
                        // known regression to address with a
                        // tree-side incremental cache, see the
                        // unified-rendering plan Session 2f.
                        update_border_tree_with_offsets(doc, &offsets, &mut app_scene);
                        // Labels are emitted per frame (not
                        // cached) so their positions track the
                        // live drag.
                        update_connection_label_tree(&scene, &mut app_scene, &mut renderer);
                        // Portal markers also track the live drag.
                        update_portal_tree(
                            doc, &offsets, &mut app_scene, &mut renderer,
                        );
                        // Edge handles (anchor / midpoint /
                        // control-point ◆ glyphs on a selected
                        // edge) must also track the live drag.
                        // Without this the handles stay pinned
                        // to the pre-drag positions until mouse
                        // release triggers a full rebuild.
                        update_edge_handle_tree(&scene, &mut app_scene);
                        // Single buffer-walk after the batch.
                        flush_canvas_scene_buffers(&mut app_scene, &mut renderer);
                    }

                    *pending_delta = Vec2::ZERO;
                    mutation_throttle.record_work_duration(work_start.elapsed());
                }
            }
            // Session 6C edge-handle drag drain. Mirrors the
            // MovingNode drain above but writes the edge in
            // place instead of moving nodes. The scene cache
            // is invalidated for the single dirty edge so the
            // next build re-samples just that edge; everything
            // else rides the incremental rebuild path.
            if let DragState::DraggingEdgeHandle {
                ref edge_ref,
                ref mut handle,
                ref mut pending_delta,
                ref total_delta,
                ref start_handle_pos,
                ..
            } = drag_state {
                if *pending_delta != Vec2::ZERO && mutation_throttle.should_drain() {
                    let work_start = Instant::now();
                    if let Some(doc) = document.as_mut() {
                        let new_handle = apply_edge_handle_drag(
                            doc,
                            edge_ref,
                            *handle,
                            *start_handle_pos,
                            *total_delta,
                        );
                        *handle = new_handle;

                        let edge_key = baumhard::mindmap::scene_cache::EdgeKey::new(
                            &edge_ref.from_id,
                            &edge_ref.to_id,
                            &edge_ref.edge_type,
                        );
                        scene_cache.invalidate_edge(&edge_key);

                        // Single dirty edge — the one being
                        // dragged. Once the tree-path gains
                        // an incremental shaping cache (see
                        // the matching note in the MovingNode
                        // drain above), we'll thread this in
                        // via `scene_cache.invalidate_edge`
                        // already called above.
                        let _ = edge_key;
                        let offsets: HashMap<String, (f32, f32)> = HashMap::new();

                        let scene = doc.build_scene_with_cache(
                            &offsets,
                            &mut scene_cache,
                            renderer.camera_zoom(),
                        );
                        update_connection_tree(&scene, &mut app_scene);
                        update_edge_handle_tree(&scene, &mut app_scene);
                        // Labels are rebuilt per frame so a
                        // control-point drag keeps the label
                        // correctly anchored to the live path.
                        update_connection_label_tree(&scene, &mut app_scene, &mut renderer);
                        update_portal_tree(
                            doc,
                            &std::collections::HashMap::new(),
                            &mut app_scene,
                            &mut renderer,
                        );
                        flush_canvas_scene_buffers(&mut app_scene, &mut renderer);
                    }
                    *pending_delta = Vec2::ZERO;
                    mutation_throttle.record_work_duration(work_start.elapsed());
                }
            }
            // Update selection rectangle overlay + preview highlight (once per frame)
            if let DragState::SelectingRect { start_canvas, current_canvas } = &drag_state {
                let sc = *start_canvas;
                let cc = *current_canvas;
                let min = Vec2::new(sc.x.min(cc.x), sc.y.min(cc.y));
                let max = Vec2::new(sc.x.max(cc.x), sc.y.max(cc.y));
                renderer.rebuild_selection_rect_overlay(min, max);

                // Preview: rebuild tree with intersecting nodes highlighted
                if let Some(doc) = document.as_ref() {
                    let mut new_tree = doc.build_tree();
                    let hits = rect_select(sc, cc, &new_tree);
                    let preview_selection = match hits.len() {
                        0 => SelectionState::None,
                        1 => SelectionState::Single(hits.into_iter().next().unwrap()),
                        _ => SelectionState::Multi(hits),
                    };
                    apply_tree_highlights(
                        &mut new_tree,
                        preview_selection
                            .selected_ids()
                            .into_iter()
                            .map(|id| (id, HIGHLIGHT_COLOR)),
                    );
                    renderer.rebuild_buffers_from_tree(&new_tree.tree);
                    mindmap_tree = Some(new_tree);
                }
            }

            // Camera (pan/zoom/resize) changed → rebuild
            // connection buffers against the new viewport. On
            // zoom, the document-side scene cache is also stale
            // because effective font size depends on zoom, so
            // clear it before the rebuild re-samples.
            //
            // Skipped when a node drag is in progress: the
            // MovingNode branch above rebuilds with the drag
            // offsets on its next non-zero `pending_delta` using
            // the current camera, and rebuilding here with
            // empty offsets would flicker dragged connections
            // back to their pre-drag positions for one frame.
            // Wheel-zoom during an active drag with zero
            // `pending_delta` leaves connections stale for one
            // frame until the next mouse-move flush — an
            // acceptable tradeoff to keep the two dirty sources
            // separate. Always take the flags (even when
            // skipped) so they don't leak across drag frames.
            let geometry_dirty = renderer.take_connection_geometry_dirty();
            let viewport_dirty = renderer.take_connection_viewport_dirty();
            if (geometry_dirty || viewport_dirty)
                && !matches!(drag_state, DragState::MovingNode { .. })
            {
                if let Some(doc) = document.as_ref() {
                    if geometry_dirty {
                        // `ensure_zoom` inside
                        // `build_scene_with_cache` would also
                        // catch this, but clearing explicitly
                        // here keeps the ordering readable
                        // next to the rebuild.
                        scene_cache.clear();
                    }
                    let scene = doc.build_scene_with_cache(
                        &HashMap::new(),
                        &mut scene_cache,
                        renderer.camera_zoom(),
                    );
                    update_connection_tree(&scene, &mut app_scene);
                    update_connection_label_tree(&scene, &mut app_scene, &mut renderer);
                    update_portal_tree(
                        doc,
                        &HashMap::new(),
                        &mut app_scene,
                        &mut renderer,
                    );
                    // Edge handles (if an edge is selected) must
                    // also follow camera changes — scroll-wheel
                    // zoom with a selected edge used to leave
                    // the handles pinned to stale screen
                    // positions until the next full rebuild.
                    update_edge_handle_tree(&scene, &mut app_scene);
                    flush_canvas_scene_buffers(&mut app_scene, &mut renderer);
                }
            }

            // Picker hover / chip drain. Mouse-move and
            // chip-focus handlers set `picker_dirty`
            // whenever HSV state changes; this gate runs
            // the actual rebuild at most once per
            // refresh budget. `picker_throttle` self-
            // tunes via the moving-average mechanism so
            // a heavy map (where rebuild_scene_only is
            // expensive) gets fewer drains per second
            // rather than dropping frames.
            if picker_dirty && picker_throttle.should_drain() {
                if let (Some(doc), true) =
                    (document.as_mut(), color_picker_state.is_open())
                {
                    let work_start = std::time::Instant::now();
                    rebuild_scene_only(doc, &mut app_scene, &mut renderer);
                    rebuild_color_picker_overlay(
                        &mut color_picker_state,
                        doc,
                        &mut app_scene,
                        &mut renderer,
                    );
                    picker_dirty = false;
                    picker_throttle.record_work_duration(work_start.elapsed());
                } else {
                    // Picker closed between event and drain — drop the dirty flag.
                    picker_dirty = false;
                }
            }

            // Phase 4: tick any active animations. Each
            // tick lerps the from / to snapshots into the
            // model and (on completion) routes the final
            // state through `apply_custom_mutation` so the
            // standard model-sync + undo-push runs once.
            // Drives `rebuild_all` only when something
            // actually advanced — sleeping in Poll mode
            // when no animations are active is automatic.
            let animation_advanced = match document.as_mut() {
                Some(doc) if doc.has_active_animations() => {
                    doc.tick_animations(now_ms() as u64, mindmap_tree.as_mut())
                }
                _ => false,
            };
            if animation_advanced {
                if let Some(doc) = document.as_ref() {
                    rebuild_all(doc, &mut mindmap_tree, &mut app_scene, &mut renderer);
                }
            }

            // Drive the render loop each frame
            renderer.process();
        }
        _ => {}
    }
}).expect("Some kind of unexpected error appears to have taken place")
}
