//! Native event-loop first-run initialisation. Lifted out of
//! [`super::run_native::run`] so that the handler struct lives in
//! `run_native.rs` without being drowned by ~150 lines of setup.
//!
//! Called once from
//! [`super::run_native::NativeApp::resumed`][super::run_native::NativeApp]
//! the first time winit fires the resumed callback (i.e. the first
//! time there's an `ActiveEventLoop` to create a window against).
//! Subsequent resume events are guarded against by the `is_none()`
//! check in `resumed`.

#![cfg(not(target_arch = "wasm32"))]

use super::run_native::InitState;
use super::*;

use baumhard::mindmap::tree_builder::MindMapTree;

/// Build the fully-initialised [`InitState`] around a freshly-
/// created `Window`. Performs: GPU surface + renderer init,
/// mindmap load (best-effort; on failure the document stays
/// `None` and the canvas renders an empty scene), mutation-
/// registry wiring, theme / clear-color resolve, initial scene +
/// border + portal tree build, camera fit-to-tree, and finally
/// `StartRender`. All the state that used to live as locals in
/// the old `run_native::run` preamble now rides back out in the
/// returned struct.
pub(super) fn build(options: &Options, window: Arc<Window>) -> InitState {
    // Single-threaded architecture: the handler owns the Renderer directly.
    baumhard::font::fonts::init();

    let unsafe_target = unsafe { SurfaceTargetUnsafe::from_window(window.as_ref()) }
        .expect("Failed to create a SurfaceTargetUnsafe");
    let instance = Instance::default();
    let surface = unsafe { instance.create_surface_unsafe(unsafe_target) }.unwrap();

    let mut renderer = block_on(Renderer::new(instance, surface, Arc::clone(&window)));

    // Configure initial surface size.
    let size = window.inner_size();
    renderer.process_decree(RenderDecree::SetSurfaceSize(size.width, size.height));

    // Load mindmap — document and tree persist for interactive use.
    let mut document: Option<MindMapDocument> = None;
    let mut mindmap_tree: Option<MindMapTree> = None;
    // Phase 4(B) keyed incremental rebuild: document-side cache of
    // per-edge pre-clip sample geometry. Populated lazily by
    // `build_scene_with_cache`; cleared by `rebuild_all` so any
    // structural change forces a fresh scene build.
    let scene_cache = baumhard::mindmap::scene_cache::SceneConnectionCache::new();
    // App-level scene host: owns the canvas-space tree for borders
    // today (registered via `update_border_tree_*`) and hosts the
    // console / color-picker overlays.
    let mut app_scene = crate::application::scene_host::AppScene::new();

    match MindMapDocument::load(&options.mindmap_path) {
        Ok(mut doc) => {
            // Four-source mutation registry: app bundle (shipped in the
            // binary) < user file ($XDG_CONFIG_HOME/mandala/mutations.json)
            // < map (in the .mindmap.json) < inline (on individual nodes).
            let (app_mutations, user_mutations) =
                crate::application::document::mutations_loader::load_app_and_user(None);
            doc.build_mutation_registry_with_app_and_user(&app_mutations, &user_mutations);
            // Rust-backed handlers for mutations too structural for
            // a pure-data `flat_mutations` reach (flower-layout,
            // tree-cascade, …).
            crate::application::document::mutations::register_builtin_handlers(&mut doc);
            // Canvas background: resolve through theme variables so
            // `"var(--bg)"` works, then hand off to the renderer as
            // the render-pass clear color.
            let vars = &doc.mindmap.canvas.theme_variables;
            let resolved_bg = baumhard::util::color::resolve_var(
                &doc.mindmap.canvas.background_color,
                vars,
            );
            renderer.set_clear_color_from_hex(resolved_bg);

            // Nodes: build Baumhard tree from MindMap hierarchy.
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

    // Start rendering.
    renderer.process_decree(RenderDecree::StartRender);

    let keybinds: ResolvedKeybinds = options.keybind_config.resolve();
    // Cross-session history loaded from disk on startup; appended
    // to on every Enter; written back on close.
    let console_history: Vec<String> = load_console_history();

    InitState {
        window,
        renderer,
        document,
        mindmap_tree,
        scene_cache,
        app_scene,
        cursor_pos: (0.0, 0.0),
        drag_state: DragState::None,
        app_mode: AppMode::Normal,
        console_state: ConsoleState::Closed,
        console_history,
        label_edit_state: LabelEditState::Closed,
        portal_text_edit_state: PortalTextEditState::Closed,
        text_edit_state: TextEditState::Closed,
        color_picker_state: crate::application::color_picker::ColorPickerState::Closed,
        last_click: None,
        hovered_node: None,
        modifiers: ModifiersState::empty(),
        // True while the cursor is hovering a node with any trigger
        // bindings (a "button"). Tracked so we only call set_cursor
        // on transitions instead of every CursorMoved event.
        cursor_is_hand: false,
        // Phase 4(E): governing-invariant throttle. Per-frame work
        // in the drag path feeds its measured duration into this
        // tracker; when the moving average crosses the refresh
        // budget, `should_drain()` starts returning false on some
        // frames, coalescing multiple ticks' pending delta into a
        // single drain.
        mutation_throttle: MutationFrequencyThrottle::with_default_budget(),
        // Picker hover gate: cursor-moves into the picker update
        // HSV + preview synchronously (cheap), but scene + overlay
        // rebuild is deferred to the `AboutToWait` drain.
        picker_throttle: MutationFrequencyThrottle::with_default_budget(),
        picker_dirty: false,
        keybinds,
    }
}
