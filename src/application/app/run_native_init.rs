// SPDX-License-Identifier: MPL-2.0

//! First-run initialization for the native event loop. Called once
//! from `super::run_native::NativeApp::resumed`.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use baumhard::mindmap::tree_builder::MindMapTree;
use pollster::block_on;
use winit::window::Window;

use crate::application::platform::input::Modifiers as ModifiersState;

use super::console_input::load_console_history;
use super::run_native::InitState;
use super::scene_rebuild::{rebuild_all, warm_scene_at_load};
use super::single_line_edit::SingleLineEditor;
use super::startup_load;
use super::text_edit::TextEditState;
use super::{DragState, InteractionMode, Options};
use crate::application::common::RenderDecree;
use crate::application::console::ConsoleState;
use crate::application::keybinds::ResolvedKeybinds;
use crate::application::renderer::Renderer;

/// Build the fully-initialized [`InitState`] around a freshly-created
/// `Window`.
///
/// A load failure is not a missing document: it resolves through
/// [`startup_load::native_startup_document`] to the load-failure
/// placard, which is installed by the same code that installs a real
/// map. Everything
/// below the load therefore runs unconditionally — the window opens
/// with the loader's message on the canvas, the camera fits it,
/// input works, and `open` can be typed into the console without
/// restarting the process.
pub(super) fn build(options: &Options, window: Arc<Window>) -> InitState {
    baumhard::font::fonts::init();

    let mut renderer = block_on(Renderer::bootstrap_native(Arc::clone(&window)));

    // Configure initial surface size.
    let size = window.inner_size();
    renderer.process_decree(RenderDecree::SetSurfaceSize(size.width, size.height));

    // Keyed incremental rebuild: document-side cache of per-edge
    // pre-clip sample geometry. Populated at load by
    // the cache-aware connection pass so first interactions don't pay the
    // full Bezier-sample cost; cleared by `rebuild_all` so any
    // structural change forces a fresh scene build.
    let mut scene_cache = baumhard::mindmap::scene_cache::SceneConnectionCache::new();
    // App-level scene host: owns the canvas-space tree for borders
    // today (registered via `update_border_tree_*`) and hosts the
    // console / color-picker overlays.
    let mut app_scene = crate::application::scene_host::AppScene::new();

    // The initial map load, in one call. A rejection is not a
    // missing document: `startup_load` turns it into the load-failure
    // placard carrying the loader's own message, and everything below
    // installs that placard by the same code that installs a real
    // map. The `Result` never reaches this file, which is what keeps
    // the two targets from reporting the same failure differently —
    // the browser's peer call is `wasm_startup_document`.
    let mut doc = startup_load::native_startup_document(&options.mindmap_path);

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
    let resolved_bg = baumhard::util::color::resolve_var(&doc.mindmap.canvas.background_color, vars);
    renderer.set_clear_color_from_hex(resolved_bg);

    // Nodes: build Baumhard tree from MindMap hierarchy.
    let tree = doc.build_tree();
    renderer.rebuild_buffers_from_tree(&tree.tree);
    renderer.fit_camera_to_tree(&tree.tree);

    // Every canvas role projected once at load plus the
    // handle-tree allocator warm — `warm_scene_at_load`, the
    // body the browser's init runs too. `fit_camera_to_tree`
    // above settled the zoom, which that helper reads.
    warm_scene_at_load(&doc, &mut app_scene, &mut renderer, &mut scene_cache);

    // `InitState.mindmap_tree` is optional (the console's
    // document-replace path drops it), and `rebuild_all` takes it as
    // `&mut Option`. `doc` stays owned to the end of this function —
    // startup no longer has a "no document" state to model.
    let mut mindmap_tree: Option<MindMapTree> = Some(tree);

    // Start rendering.
    renderer.process_decree(RenderDecree::StartRender);

    // Pre-warm allocators on the rebuild_all critical path: the
    // first user-triggered selection / tree-mutating drag runs
    // `rebuild_all` (build_tree → apply_tree_highlights →
    // rebuild_buffers_from_tree → rebuild_scene_only) from a fresh
    // process state, paying cold-allocator costs on every cosmic-
    // text Buffer reshape. Running it once here at load warms the
    // BufferLine pools, the Tree arena, and the per-role canvas-
    // signature stamps so the first user-visible rebuild only
    // pays the diffing-cost portion.
    {
        // Init runs in Default mode — no handles emit. Construct the
        // mode locally rather than threading from the (still-empty)
        // InitState; the post-init InitState will use its own field.
        let init_mode = InteractionMode::Default;
        rebuild_all(
            &doc,
            &init_mode,
            &mut mindmap_tree,
            &mut app_scene,
            &mut renderer,
            &mut scene_cache,
        );
    }

    // Pre-warm the render pipeline: one full render cycle so the
    // wgpu driver compiles pipeline shaders, the swapchain
    // allocates its first backing image, and the glyph atlas is
    // populated before the first user-driven frame. Without this,
    // those costs (commonly 50-300ms total on first Vulkan/Metal
    // pipeline bind) would land on the user's first interaction.
    renderer.prewarm();

    let keybinds: ResolvedKeybinds = options.keybind_config.resolve();
    // Cross-session history loaded from disk on startup; appended
    // to on every Enter; written back on close.
    let console_history: Vec<String> = load_console_history();

    // App + User tiers from the platform loaders, then the
    // document-derived Map and Inline tiers. Body is
    // `macros::loader::build_macro_registry`, which the browser's
    // init calls too — including the `log::info!` shape, so
    // cross-target log triage stays uniform. `doc` is owned here
    // rather than an `Option`: startup always produces a document
    // now, a real map or the load-failure placard.
    let macros = crate::application::macros::loader::build_macro_registry(Some(&doc));

    InitState {
        window,
        renderer,
        document: Some(doc),
        mindmap_tree,
        scene_cache,
        app_scene,
        cursor_pos: (0.0, 0.0),
        drag_state: DragState::None,
        interaction_mode: InteractionMode::Default,
        console_state: ConsoleState::Closed,
        console_history,
        single_line_edit_state: SingleLineEditor::Closed,
        text_edit_state: TextEditState::Closed,
        color_picker_state: crate::application::color_picker::ColorPickerState::Closed,
        last_click: None,
        hovered_node: None,
        modifiers: ModifiersState::empty(),
        // True while the cursor is hovering a node with any trigger
        // bindings (a "button"). Tracked so we only call set_cursor
        // on transitions instead of every CursorMoved event.
        cursor_is_hand: false,
        // Last cursor icon written via Window::set_cursor — used by
        // the cursor_moved handler to skip redundant per-frame
        // set_cursor calls on platforms (Windows, Wayland) where
        // winit doesn't dedup. Initialized to Default to match the
        // as-launched cursor.
        cursor_icon_last: winit::window::CursorIcon::Default,
        // Picker hover gate: cursor-moves into the picker update
        // HSV + preview synchronously (cheap), but scene + overlay
        // rebuild runs through the unified adaptive throttle in
        // `AboutToWait`. Each active drag gets its own
        // `MutationFrequencyThrottle` inside its interaction
        // struct on entry (see `event_cursor_moved`).
        picker_hover: super::throttled_interaction::ColorPickerHoverInteraction::new(),
        keybinds,
        macros,
        anim_pause_start_ms: None,
        // Touch gesture recognizer. State machine starts Idle;
        // first `WindowEvent::Touch` lands a finger.
        touch_recognizer: super::touch_gesture::TouchGestureRecognizer::new(),
    }
}
