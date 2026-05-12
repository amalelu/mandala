// SPDX-License-Identifier: MPL-2.0

//! First-run initialisation for the native event loop. Called once
//! from `super::run_native::NativeApp::resumed`.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use baumhard::mindmap::tree_builder::MindMapTree;
use pollster::block_on;
use winit::window::Window;

use crate::application::platform::input::Modifiers as ModifiersState;

use super::console_input::load_console_history;
use super::label_edit::{LabelEditState, PortalTextEditState};
use super::run_native::InitState;
use super::scene_rebuild::{
    flush_canvas_scene_buffers, rebuild_all, update_border_tree_static, update_connection_label_tree,
    update_connection_tree, update_edge_handle_tree, update_node_resize_handle_tree, update_portal_tree,
    update_section_resize_handle_tree, warm_handle_tree_arenas,
};
use super::text_edit::TextEditState;
use super::{DragState, InteractionMode, Options};
use crate::application::common::RenderDecree;
use crate::application::console::ConsoleState;
use crate::application::document::MindMapDocument;
use crate::application::keybinds::ResolvedKeybinds;
use crate::application::renderer::Renderer;

/// Build the fully-initialised [`InitState`] around a freshly-created
/// `Window`. Mindmap load is best-effort (on failure the document
/// stays `None` and the canvas renders empty).
pub(super) fn build(options: &Options, window: Arc<Window>) -> InitState {
    baumhard::font::fonts::init();

    let mut renderer = block_on(Renderer::bootstrap_native(Arc::clone(&window)));

    // Configure initial surface size.
    let size = window.inner_size();
    renderer.process_decree(RenderDecree::SetSurfaceSize(size.width, size.height));

    // Load mindmap — document and tree persist for interactive use.
    let mut document: Option<MindMapDocument> = None;
    let mut mindmap_tree: Option<MindMapTree> = None;
    // Keyed incremental rebuild: document-side cache of per-edge
    // pre-clip sample geometry. Populated at load by
    // `build_scene_with_cache` so first interactions don't pay the
    // full Bezier-sample cost; cleared by `rebuild_all` so any
    // structural change forces a fresh scene build.
    let mut scene_cache = baumhard::mindmap::scene_cache::SceneConnectionCache::new();
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
            let resolved_bg = baumhard::util::color::resolve_var(&doc.mindmap.canvas.background_color, vars);
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
            //
            // Use `build_scene_with_cache` (not `build_scene`) so
            // `scene_cache` is hot before the first interaction; the
            // first drag/zoom no longer pays the full per-edge
            // Bezier-sample cost.
            // Init runs before any interaction — mode is `Default` and
            // no resize handles emit. Pre-warm path uses the explicit
            // `none()` overrides so the warm scene matches the first
            // post-init frame's shape.
            let scene = doc.build_scene_with_cache(
                &std::collections::HashMap::new(),
                &mut scene_cache,
                renderer.camera_zoom(),
                crate::application::document::InteractionModeOverrides::none(),
            );
            update_connection_tree(&scene, &mut app_scene);
            update_border_tree_static(&doc, &mut app_scene);
            update_portal_tree(
                &doc,
                &std::collections::HashMap::new(),
                &mut app_scene,
                &mut renderer,
            );
            update_connection_label_tree(&scene, &mut app_scene, &mut renderer);
            // Register the three handle-tree canvas roles with their
            // fresh-load (empty-slice) signatures. The first real
            // selection still takes `CanvasDispatch::FullRebuild`
            // (its 8-handle signature differs from the empty one),
            // but every subsequent transition back to "nothing
            // selected" hits `InPlaceMutator` instead of
            // FullRebuild because the empty signature is already
            // stamped. The role registration also lets §B2 dispatch
            // find the role at all — without these calls the first
            // interaction would force a register-and-rebuild, the
            // second a rebuild, and only steady-state drags would
            // be cheap.
            update_edge_handle_tree(&scene, &mut app_scene);
            update_section_resize_handle_tree(&scene, &mut app_scene);
            update_node_resize_handle_tree(&scene, &mut app_scene);
            // Synthetic-handle allocator warm: feed the handle-tree
            // dispatch path 8-element slices once so its arena
            // allocates from cold pools at load instead of on the
            // user's first selection. Doesn't help signature
            // matching (the user-state signature still differs),
            // but the cosmic-text BufferLine pools and arena
            // bumpers used inside `build_handle_tree` are warm
            // when the first real selection lands, cutting the
            // FullRebuild cost.
            warm_handle_tree_arenas(&mut app_scene);
            // Restamp the load-time empty signature so the
            // canvas state at load-end is the empty-handles state
            // rather than the synthetic 8-handle one. The later
            // `rebuild_all` would do this again via
            // `rebuild_scene_only`, but we re-stamp here too so
            // correctness doesn't depend on `rebuild_all` running
            // — if a future change makes it conditional or moves
            // it, the canvas state stays well-defined.
            update_edge_handle_tree(&scene, &mut app_scene);
            update_section_resize_handle_tree(&scene, &mut app_scene);
            update_node_resize_handle_tree(&scene, &mut app_scene);
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

    // Pre-warm allocators on the rebuild_all critical path: the
    // first user-triggered selection / tree-mutating drag runs
    // `rebuild_all` (build_tree → apply_tree_highlights →
    // rebuild_buffers_from_tree → rebuild_scene_only) from a fresh
    // process state, paying cold-allocator costs on every cosmic-
    // text Buffer reshape. Running it once here at load warms the
    // BufferLine pools, the Tree arena, and the per-role canvas-
    // signature stamps so the first user-visible rebuild only
    // pays the diffing-cost portion.
    if let Some(doc) = document.as_ref() {
        // Init runs in Default mode — no handles emit. Construct the
        // mode locally rather than threading from the (still-empty)
        // InitState; the post-init InitState will use its own field.
        let init_mode = InteractionMode::Default;
        rebuild_all(
            doc,
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

    // Build the macro registry across all four tiers, in ascending
    // precedence order: App < User at startup; Map < Inline are
    // refreshed via `rebuild_document_macros` whenever a document
    // loads. Higher-tier ids shadow lower-tier ones; clearing a
    // higher tier reveals what's underneath. See
    // `format/macros.md` for the threat model and the SOURCE-OF-
    // TRUTH list of places that must move together when the order
    // changes.
    let mut macros = crate::application::macros::MacroRegistry::new();
    let mut app_count = 0usize;
    for m in crate::application::macros::loader::load_app_macros() {
        macros.insert(m, crate::application::macros::MacroSource::App);
        app_count += 1;
    }
    let mut user_count = 0usize;
    for m in crate::application::macros::loader::load_user_macros() {
        macros.insert(m, crate::application::macros::MacroSource::User);
        user_count += 1;
    }
    if app_count > 0 || user_count > 0 {
        log::info!(
            "loaded {} macro(s): {} app-tier, {} user-tier",
            macros.len(),
            app_count,
            user_count
        );
    }
    // Document-derived macro tiers (Map + Inline). The
    // `rebuild_document_macros` helper is the single entry point
    // shared with the document-replace path in `execute_console_line`
    // so the Map-then-Inline ordering can't drift between sites.
    if let Some(d) = document.as_ref() {
        crate::application::macros::loader::rebuild_document_macros(&mut macros, d);
    }

    InitState {
        window,
        renderer,
        document,
        mindmap_tree,
        scene_cache,
        app_scene,
        cursor_pos: (0.0, 0.0),
        drag_state: DragState::None,
        interaction_mode: InteractionMode::Default,
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
        // Last cursor icon written via Window::set_cursor — used by
        // the cursor_moved handler to skip redundant per-frame
        // set_cursor calls on platforms (Windows, Wayland) where
        // winit doesn't dedup. Initialised to Default to match the
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
        // Touch gesture recogniser. State machine starts Idle;
        // first `WindowEvent::Touch` lands a finger.
        touch_recognizer: super::touch_gesture::TouchGestureRecognizer::new(),
    }
}
