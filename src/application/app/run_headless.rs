// SPDX-License-Identifier: MPL-2.0

//! Headless run loop — no winit, no wgpu, IPC-only. Entered when
//! the binary was launched with `--headless --ipc-port=N`.
//!
//! Driven by a single tick loop on the main thread that reads
//! `IpcRequest`s from a `crossbeam_channel::Receiver` and dispatches
//! them through the cross-platform action funnel (the same
//! `dispatch_compatible` path WASM uses), then rebuilds the
//! `StateSnapshot` for the IPC thread to read.
//!
//! Currently a minimal-viable skeleton: the loop boots the IPC
//! thread, parks, and replies `503` to every request. Real
//! dispatch lands in a later milestone after `CoreState` is
//! extracted from `InitState`.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use crossbeam_channel::RecvTimeoutError;

use super::Application;
use crate::application::document::MindMapDocument;
use crate::application::ipc::state_view::{CameraView, DocumentView, SelectionView, StateSnapshot};
use crate::application::ipc::{
    self, log_buffer, IpcEvent, IpcRequest, IpcRequestPayload, IpcResponse, RequestSink, RunMode,
};

/// ~60 Hz idle wake-up. Each tick the loop processes any queued IPC
/// requests and republishes the state snapshot. The cadence matches
/// the windowed renderer's redraw frequency so future animations
/// step at a comparable rate when driven through IPC.
const HEADLESS_TICK: Duration = Duration::from_millis(16);

/// Headless-mode application state. Owns the document and any
/// future cross-platform model state. No window, no renderer.
struct HeadlessState {
    document: Option<MindMapDocument>,
    /// Virtual camera. Headless has no canvas so this isn't a real
    /// viewport; agents can still drive it via `Action::ZoomIn` /
    /// `PanCameraNorth` / etc. The state is read by `/state/camera`
    /// so an agent can build a consistent mental model of "where is
    /// the camera now." Defaults to the same `Camera2D::default()`
    /// shape the renderer boots with (centre at origin, zoom 1.0).
    camera: baumhard::gfx_structs::camera::Camera2D,
    /// Mirror of `Renderer.fps_display_mode` so `Action::ToggleFps` /
    /// `ToggleFpsDebug` round-trip through the dispatcher cleanly.
    /// Has no visible effect in headless; future `/state` consumers
    /// can read it.
    fps_mode: crate::application::common::FpsDisplayMode,
    /// Cached `mindmap_tree` projection — rebuilt on each dispatch
    /// via `rebuild_all`. `Option` to match the windowed shape
    /// (`InitState.mindmap_tree`) and the `InputContextCore`
    /// signature.
    mindmap_tree: Option<baumhard::mindmap::tree_builder::MindMapTree>,
    /// Per-edge connection sample cache. Real wgpu builds clear /
    /// repopulate this every rebuild; in headless it's purely
    /// internal state the dispatcher's `apply_*` arms touch.
    scene_cache: baumhard::mindmap::scene_cache::SceneConnectionCache,
    /// App-scene slot table. Same posture as `scene_cache`: shape-
    /// only state, no GPU resources.
    app_scene: crate::application::scene_host::AppScene,
    /// Cross-platform interaction mode. Headless will rarely leave
    /// `Default`, but `Action::EnterReparentMode` etc. need somewhere
    /// to write — and `/state/interaction_mode` reads from here.
    interaction_mode: crate::application::app::InteractionMode,
    /// Cross-platform text-edit modal state. Mirrors
    /// `InputContextCore.text_edit_state` so `Action::TextEditCancel`
    /// / `TextEditCommit` find a target. Headless never has an open
    /// text editor — the dispatcher path is essentially a no-op —
    /// but the field has to exist.
    text_edit_state: crate::application::app::text_edit::TextEditState,
    /// Last click — required by `InputContextCore`. Headless emits
    /// none; the field is permanently `None`.
    last_click: Option<crate::application::app::LastClick>,
    /// Cursor position in synthetic screen space. Defaults to (0,0).
    /// `Action::ZoomIn` reads it to compute the zoom anchor; agents
    /// could in principle write to it via a future endpoint.
    cursor_pos: (f64, f64),
    /// Empty resolved keybinds — headless dispatch never resolves
    /// keys from gestures, but the funnel reads `keybinds` for a
    /// handful of binding-existence queries.
    keybinds: crate::application::keybinds::ResolvedKeybinds,
    /// Empty modifier state.
    modifiers: crate::application::platform::input::Modifiers,
    /// Macro registry — populated alongside the document.
    macros: crate::application::macros::MacroRegistry,
}

/// `RebuildHost` impl for headless mode. Every method is a no-op
/// except:
/// - camera reads (`camera_zoom`, `surface_width/height`,
///   `screen_to_canvas`) return values derived from the virtual
///   camera so dispatch arms that read these (e.g. `apply_zoom_step`
///   anchors to cursor, `screen_to_canvas` for orphan creation)
///   produce sensible outputs;
/// - `process_decree` interprets `RenderDecree::Camera*` against the
///   virtual camera state — so `Action::ZoomIn` / `PanCameraNorth`
///   etc. actually update something an agent can observe via
///   `/state/camera`;
/// - `fit_camera_to_tree` updates the virtual camera to the tree's
///   bounds;
/// - `set_camera_center` updates the virtual camera's position;
/// - `set_fps_display` / `fps_display_mode` round-trip through a
///   mirror field.
///
/// Methods that touch real GPU resources (`rebuild_buffers_from_tree`,
/// `rebuild_canvas_scene_buffers`, hitbox setters, `reshape_buffer_for`)
/// do nothing — there's no GPU and no on-screen output. The internal
/// scene-cache + app-scene mutations performed *outside* the host
/// (by `rebuild_all`) still happen on the headless fields, so any
/// dispatch arm that consults them will see consistent state.
struct HeadlessHost<'a> {
    camera: &'a mut baumhard::gfx_structs::camera::Camera2D,
    fps_mode: &'a mut crate::application::common::FpsDisplayMode,
}

impl<'a> crate::application::app::dispatch::cross_dispatch::RebuildHost for HeadlessHost<'a> {
    fn camera_zoom(&self) -> f32 {
        self.camera.zoom
    }
    fn surface_width(&self) -> u32 {
        // 1920×1080 is a stable synthetic viewport. Agents driving a
        // headless mandala don't have a real surface; this constant
        // keeps zoom/pan math finite and predictable.
        1920
    }
    fn surface_height(&self) -> u32 {
        1080
    }
    fn fps_display_mode(&self) -> crate::application::common::FpsDisplayMode {
        *self.fps_mode
    }
    fn process_decree(&mut self, decree: crate::application::common::RenderDecree) {
        use crate::application::common::RenderDecree as D;
        match decree {
            D::CameraPan(dx, dy) => {
                // Inverse-zoom transform from screen-px to canvas-px,
                // mirroring the real renderer's pan handling.
                let inv = 1.0_f32 / self.camera.zoom.max(f32::EPSILON);
                self.camera.position.x -= dx * inv;
                self.camera.position.y -= dy * inv;
            }
            D::CameraZoom {
                screen_x,
                screen_y,
                factor,
            } => {
                // Zoom around the synthetic screen point. The exact
                // formula matches `Camera2D::zoom_at`'s contract: keep
                // the canvas point under (screen_x, screen_y) fixed.
                let before = self.camera.screen_to_canvas(glam::Vec2::new(screen_x, screen_y));
                self.camera.zoom = (self.camera.zoom * factor).clamp(0.01, 100.0);
                let after = self.camera.screen_to_canvas(glam::Vec2::new(screen_x, screen_y));
                self.camera.position += before - after;
            }
            // Other decrees (resize, terminate, ...) have no headless effect.
            _ => {}
        }
    }
    fn set_camera_center(&mut self, target: glam::Vec2) {
        self.camera.position = target;
    }
    fn fit_camera_to_tree(
        &mut self,
        _tree: &baumhard::gfx_structs::tree::Tree<
            baumhard::gfx_structs::element::GfxElement,
            baumhard::gfx_structs::mutator::GfxMutator,
        >,
    ) {
        // Tree-bound computation lives on Renderer (uses surface
        // dimensions). Headless picks a neutral identity instead —
        // zoom 1.0 at origin — so the camera is at least reset rather
        // than left at a wild value after `Action::ZoomFit`.
        self.camera.zoom = 1.0;
        self.camera.position = glam::Vec2::ZERO;
    }
    fn set_fps_display(&mut self, mode: crate::application::common::FpsDisplayMode) {
        *self.fps_mode = mode;
    }
    fn rebuild_buffers_from_tree(
        &mut self,
        _tree: &baumhard::gfx_structs::tree::Tree<
            baumhard::gfx_structs::element::GfxElement,
            baumhard::gfx_structs::mutator::GfxMutator,
        >,
    ) {
        // No GPU buffers to rebuild.
    }
    fn rebuild_canvas_scene_buffers(
        &mut self,
        _app_scene: &mut crate::application::scene_host::AppScene,
    ) {
        // No GPU buffers.
    }
    fn set_mode_status_text(&mut self, _text: Option<String>) {
        // No overlay.
    }
    fn set_portal_icon_hitboxes(
        &mut self,
        _hitboxes: std::collections::HashMap<
            (baumhard::mindmap::scene_cache::EdgeKey, String),
            (glam::Vec2, glam::Vec2),
        >,
    ) {
    }
    fn set_portal_text_hitboxes(
        &mut self,
        _hitboxes: std::collections::HashMap<
            (baumhard::mindmap::scene_cache::EdgeKey, String),
            (glam::Vec2, glam::Vec2),
        >,
    ) {
    }
    fn set_connection_label_hitboxes(
        &mut self,
        _hitboxes: std::collections::HashMap<
            baumhard::mindmap::scene_cache::EdgeKey,
            (glam::Vec2, glam::Vec2),
        >,
    ) {
    }
    fn screen_to_canvas(&self, screen_x: f32, screen_y: f32) -> glam::Vec2 {
        self.camera.screen_to_canvas(glam::Vec2::new(screen_x, screen_y))
    }
    fn reshape_buffer_for(
        &mut self,
        _arena_id: indextree::NodeId,
        _tree: &baumhard::gfx_structs::tree::Tree<
            baumhard::gfx_structs::element::GfxElement,
            baumhard::gfx_structs::mutator::GfxMutator,
        >,
    ) {
        // No buffers to reshape.
    }
}

impl HeadlessState {
    fn empty() -> Self {
        Self {
            document: None,
            camera: baumhard::gfx_structs::camera::Camera2D::new(1920, 1080),
            fps_mode: crate::application::common::FpsDisplayMode::Off,
            mindmap_tree: None,
            scene_cache: baumhard::mindmap::scene_cache::SceneConnectionCache::default(),
            app_scene: crate::application::scene_host::AppScene::new(),
            interaction_mode: crate::application::app::InteractionMode::Default,
            text_edit_state:
                crate::application::app::text_edit::TextEditState::Closed,
            last_click: None,
            cursor_pos: (0.0, 0.0),
            keybinds: crate::application::keybinds::KeybindConfig::default().resolve(),
            modifiers: crate::application::platform::input::Modifiers::default(),
            macros: crate::application::macros::MacroRegistry::new(),
        }
    }

    fn new(mindmap_path: &str) -> Self {
        match MindMapDocument::load(mindmap_path) {
            Ok(mut doc) => {
                let (app_mutations, user_mutations) =
                    crate::application::document::mutations_loader::load_app_and_user(None);
                doc.build_mutation_registry_with_app_and_user(&app_mutations, &user_mutations);
                crate::application::document::mutations::register_builtin_handlers(&mut doc);
                let mut state = Self::empty();
                state.document = Some(doc);
                state
            }
            Err(e) => {
                // Startup-path: a missing/invalid mindmap is fatal. There is
                // no in-band way to recover today (no `POST /console open ...`
                // route yet). Exit code 2 matches the CLI-error idiom used
                // by `parse_cli` for `--headless`/`--ipc-port` mismatches.
                eprintln!("headless: failed to load `{mindmap_path}`: {e}");
                std::process::exit(2);
            }
        }
    }

    fn build_snapshot(&self) -> StateSnapshot {
        let camera = CameraView {
            x: self.camera.position.x,
            y: self.camera.position.y,
            zoom: self.camera.zoom,
        };
        let Some(doc) = &self.document else {
            return StateSnapshot {
                camera,
                ..Default::default()
            };
        };
        StateSnapshot {
            document: DocumentView {
                mindmap: serde_json::to_value(&doc.mindmap).ok(),
                file_path: doc.file_path.clone(),
                dirty: doc.dirty,
                undo_depth: doc.undo_stack.len(),
                active_animation_count: doc.active_animations.len(),
            },
            selection: SelectionView::from_state(&doc.selection),
            camera,
            ..Default::default()
        }
    }
}

/// Entry point called from `Application::run` when `--headless` was
/// set. Boots the IPC thread, then runs the tick loop on this
/// thread forever.
pub(super) fn run(app: Application) {
    let opts = app.into_options();
    let addr = opts
        .ipc_addr
        .expect("--headless requires --ipc-port (enforced at CLI parse)");

    let (req_tx, req_rx) = crossbeam_channel::unbounded::<IpcRequest>();
    let snapshot = Arc::new(RwLock::new(StateSnapshot::default()));
    // The initial receiver is dropped immediately on purpose — capacity
    // and aliveness are owned by the `Sender`. SSE handlers `subscribe()`
    // their own receiver per connection.
    let (event_tx, _) = tokio::sync::broadcast::channel::<IpcEvent>(1024);
    let log_buf = Arc::new(Mutex::new(VecDeque::with_capacity(
        log_buffer::LOG_RING_CAPACITY,
    )));

    // Install the tee logger as the FIRST and ONLY logger init for the
    // process. `main()` skips `baumhard::util::log::init()` when
    // `--headless` is set so this call wins the global `log` backend
    // unopposed; if it didn't, `/logs` and SSE `Log` events would be
    // silent (the second `set_boxed_logger` would `Err` and the new
    // logger would be dropped). See CODE_CONVENTIONS §9.
    log_buffer::init_with_ipc(log_buf.clone(), event_tx.clone());

    ipc::boot(
        addr,
        RequestSink::Headless(req_tx),
        snapshot.clone(),
        event_tx.clone(),
        log_buf.clone(),
        RunMode::Headless,
    );

    let mut state = HeadlessState::new(&opts.mindmap_path);

    // Publish the initial snapshot so the first /state read after
    // startup sees the loaded document, not the empty default.
    *snapshot.write().expect("snapshot lock") = state.build_snapshot();

    log::info!(
        "mandala headless: serving IPC on {addr} (no window, no wgpu); ctrl-c to exit"
    );

    // Tick loop: handle pending requests then republish the snapshot.
    // We republish every tick — not only after a request — so future
    // animations, timer-driven mutations, and the planned
    // SelectionChanged SSE diff work correctly when the loop is
    // idle-from-HTTP but the model is evolving on its own.
    // Closure: handle one request, rebuild the snapshot, THEN send
    // the response. The order matters — sending the response unblocks
    // the HTTP handler, which may immediately fire a follow-up GET
    // against `Arc<RwLock<StateSnapshot>>`. If the snapshot were
    // rebuilt after the response send, the follow-up GET could race
    // ahead of the rebuild and read a stale snapshot.
    let handle = |state: &mut HeadlessState, req: IpcRequest| {
        let resp = handle_request(state, &event_tx, req.payload);
        *snapshot.write().expect("snapshot lock") = state.build_snapshot();
        let _ = req.responder.send(resp);
    };

    loop {
        match req_rx.recv_timeout(HEADLESS_TICK) {
            Ok(req) => {
                handle(&mut state, req);
                while let Ok(req) = req_rx.try_recv() {
                    handle(&mut state, req);
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                // Idle tick — republish so any tick-driven state
                // (future animations, timer-driven mutations) lands
                // in `/state` without an HTTP nudge.
                *snapshot.write().expect("snapshot lock") = state.build_snapshot();
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn handle_request(
    state: &mut HeadlessState,
    event_tx: &tokio::sync::broadcast::Sender<IpcEvent>,
    payload: IpcRequestPayload,
) -> IpcResponse {
    use axum::http::StatusCode;
    match payload {
        IpcRequestPayload::FullState => IpcResponse::Ok(
            serde_json::to_value(state.build_snapshot()).unwrap_or(serde_json::Value::Null),
        ),
        IpcRequestPayload::CustomMutation { id, target } => {
            handle_custom_mutation(state, event_tx, &id, target.as_deref())
        }
        IpcRequestPayload::DispatchAction(action) => {
            handle_dispatch_action(state, event_tx, action)
        }
        IpcRequestPayload::Screenshot { .. } => IpcResponse::Err(
            StatusCode::NOT_IMPLEMENTED,
            "screenshot unavailable in headless mode; render the document JSON via /state/document"
                .into(),
        ),
        other => crate::application::ipc::routes::handle_main_stub(other),
    }
}

/// `POST /mutations/{id}` — apply a registered custom mutation against
/// the live document. Resolves the target node id from the request
/// body (`{"target": "node-id"}`); falls back to the current
/// `doc.selection` when the request supplies `null` / no target and
/// the selection identifies a single node. Tagged
/// [`MacroSource::Ipc`](crate::application::macros::MacroSource::Ipc)
/// — the privilege gate permits every CustomMutation, including
/// destructive ones, per the dev-only IPC contract.
fn handle_custom_mutation(
    state: &mut HeadlessState,
    event_tx: &tokio::sync::broadcast::Sender<IpcEvent>,
    id: &str,
    target: Option<&str>,
) -> IpcResponse {
    use axum::http::StatusCode;
    use crate::application::macros::MacroSource;

    let Some(doc) = state.document.as_mut() else {
        return IpcResponse::Err(
            StatusCode::SERVICE_UNAVAILABLE,
            "no document loaded".into(),
        );
    };

    let Some(custom) = doc.mutation_registry.get(id).cloned() else {
        return IpcResponse::Err(
            StatusCode::NOT_FOUND,
            format!("unknown custom mutation '{id}'"),
        );
    };

    // Resolve the target node. Body wins; otherwise honour a single
    // selected node. Multi-selection / edge-selection / no-selection
    // without an explicit target is a 400 — the caller has to be
    // explicit when the selection isn't unambiguously a node.
    let resolved_target: String = if let Some(t) = target {
        t.to_string()
    } else {
        match &doc.selection {
            crate::application::document::SelectionState::Single(id) => id.clone(),
            _ => {
                return IpcResponse::Err(
                    StatusCode::BAD_REQUEST,
                    "no target supplied and current selection is not a single node".into(),
                );
            }
        }
    };

    if !doc.mindmap.nodes.contains_key(&resolved_target) {
        return IpcResponse::Err(
            StatusCode::BAD_REQUEST,
            format!("target node '{resolved_target}' does not exist"),
        );
    }

    // Privilege tier: IPC-originated dispatches are tagged with
    // [`MacroSource::Ipc`] — unrestricted by design. The current
    // `apply_custom_mutation` path doesn't take a `MacroSource`
    // argument (no per-tier gating exists yet on this verb), so the
    // tag is informational. When custom-mutation privilege gating
    // lands, plumb `_source` through `apply_custom_mutation` to
    // surface the tier inside the verb.
    let _source = MacroSource::Ipc;

    let mut tree = doc.build_tree();
    doc.apply_custom_mutation(&custom, &resolved_target, Some(&mut tree));

    let _ = event_tx.send(IpcEvent::MutationApplied {
        id: id.to_string(),
        target: Some(resolved_target.clone()),
    });

    IpcResponse::Ok(serde_json::json!({
        "applied": true,
        "id": id,
        "target": resolved_target,
    }))
}

/// `POST /actions/dispatch` (headless) — routes the request through
/// the canonical cross-platform funnel
/// [`dispatch_compatible`](crate::application::app::dispatch::action_core::dispatch_compatible)
/// against an [`InputContextCore`] built from this process's
/// `HeadlessState` + a [`HeadlessHost`] (no-op renderer, virtual
/// camera). Tagged
/// [`MacroSource::Ipc`](crate::application::macros::MacroSource::Ipc).
///
/// The funnel handles **every cross-platform compatible Action**
/// (~80 variants) directly. NativeOnly Actions (modal flows that
/// need `NativeContextExt` state) fall through to the
/// `Unhandled` outcome — see `Action::wasm_compatibility()`. The
/// HTTP response distinguishes:
///   - `"handled"` — dispatch_compatible returned `Handled` and
///     a model mutation may have occurred.
///   - `"unhandled"` — variant is not cross-platform-reachable
///     (NativeOnly). 200 OK with `outcome=unhandled` rather than
///     501 because the variant *is* recognised; it's just that
///     the headless dispatcher has nowhere to deliver it.
fn handle_dispatch_action(
    state: &mut HeadlessState,
    event_tx: &tokio::sync::broadcast::Sender<IpcEvent>,
    action: crate::application::keybinds::Action,
) -> IpcResponse {
    use axum::http::StatusCode;
    use crate::application::app::dispatch::cross_dispatch::DispatchOutcome;

    if state.document.is_none() {
        return IpcResponse::Err(
            StatusCode::SERVICE_UNAVAILABLE,
            "no document loaded".into(),
        );
    }

    let outcome = {
        let mut host = HeadlessHost {
            camera: &mut state.camera,
            fps_mode: &mut state.fps_mode,
        };
        let mut core =
            crate::application::app::input_context_core::InputContextCore {
                document: state.document.as_mut(),
                mindmap_tree: &mut state.mindmap_tree,
                app_scene: &mut state.app_scene,
                host: &mut host,
                scene_cache: &mut state.scene_cache,
                text_edit_state: &mut state.text_edit_state,
                last_click: &mut state.last_click,
                cursor_pos: &mut state.cursor_pos,
                modifiers: &state.modifiers,
                keybinds: &state.keybinds,
                macros: &mut state.macros,
                interaction_mode: &mut state.interaction_mode,
            };
        crate::application::app::dispatch::action_core::dispatch_compatible(
            &action, &mut core,
        )
    };

    let outcome_str = match outcome {
        DispatchOutcome::Handled => "handled",
        DispatchOutcome::Unhandled => "unhandled",
    };
    let _ = event_tx.send(IpcEvent::ActionDispatched {
        action: action.clone(),
        outcome: outcome_str.into(),
    });

    IpcResponse::Ok(serde_json::json!({
        "outcome": outcome_str,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::keybinds::Action;

    /// Build a `HeadlessState` from the canonical test fixture without
    /// going through `HeadlessState::new` (which `process::exit(2)`s
    /// on load failure — fatal in a test runner).
    fn test_state() -> HeadlessState {
        let path = format!(
            "{}/maps/testament.mindmap.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let mut doc = MindMapDocument::load(&path).expect("load testament");
        let (app_mutations, user_mutations) =
            crate::application::document::mutations_loader::load_app_and_user(None);
        doc.build_mutation_registry_with_app_and_user(&app_mutations, &user_mutations);
        crate::application::document::mutations::register_builtin_handlers(&mut doc);
        let mut state = HeadlessState::empty();
        state.document = Some(doc);
        state
    }

    fn test_event_tx() -> tokio::sync::broadcast::Sender<IpcEvent> {
        tokio::sync::broadcast::channel::<IpcEvent>(16).0
    }

    fn json_outcome(resp: &IpcResponse) -> Option<String> {
        match resp {
            IpcResponse::Ok(v) => v.get("outcome").and_then(|s| s.as_str()).map(String::from),
            _ => None,
        }
    }

    #[test]
    fn test_dispatch_select_all_handled_changes_selection() {
        let mut state = test_state();
        let tx = test_event_tx();
        let resp = handle_dispatch_action(&mut state, &tx, Action::SelectAll);
        assert_eq!(json_outcome(&resp).as_deref(), Some("handled"));
        assert!(matches!(
            state.document.as_ref().unwrap().selection,
            crate::application::document::SelectionState::Multi(_),
        ));
    }

    #[test]
    fn test_dispatch_deselect_all_on_empty_selection_is_handled() {
        // Post-refactor: dispatch_compatible always reports `Handled`
        // for cross-platform variants whether or not the inner
        // mutation made a difference. (The funnel doesn't surface
        // the "no-op" granularity — agents read /state/selection to
        // observe model deltas.)
        let mut state = test_state();
        let tx = test_event_tx();
        let resp = handle_dispatch_action(&mut state, &tx, Action::DeselectAll);
        assert_eq!(json_outcome(&resp).as_deref(), Some("handled"));
    }

    #[test]
    fn test_dispatch_jump_to_root_selects_root_node() {
        let mut state = test_state();
        let tx = test_event_tx();
        let resp = handle_dispatch_action(&mut state, &tx, Action::JumpToRoot);
        assert_eq!(json_outcome(&resp).as_deref(), Some("handled"));
        assert!(matches!(
            state.document.as_ref().unwrap().selection,
            crate::application::document::SelectionState::Single(_),
        ));
    }

    #[test]
    fn test_dispatch_native_only_action_returns_unhandled() {
        // NativeOnly variants (modal flows like the console) are
        // recognised but unreachable from the cross-platform funnel.
        // The dispatcher returns 200 with `outcome=unhandled` rather
        // than 501 — the *variant* exists; it's just that headless
        // has nowhere to deliver it.
        let mut state = test_state();
        let tx = test_event_tx();
        let resp = handle_dispatch_action(&mut state, &tx, Action::OpenConsole);
        assert_eq!(json_outcome(&resp).as_deref(), Some("unhandled"));
    }

    #[test]
    fn test_dispatch_undo_after_a_mutation_decrements_undo_stack() {
        let mut state = test_state();
        let tx = test_event_tx();
        // Apply a mutation so there's something to undo. Use the
        // root node "0" since testament's IDs start from 0.
        let custom = state
            .document
            .as_ref()
            .unwrap()
            .mutation_registry
            .get("flower-layout")
            .expect("flower-layout registered")
            .clone();
        let mut tree = state.document.as_ref().unwrap().build_tree();
        state
            .document
            .as_mut()
            .unwrap()
            .apply_custom_mutation(&custom, "0", Some(&mut tree));
        let before = state.document.as_ref().unwrap().undo_stack.len();
        assert!(before >= 1, "mutation should have pushed onto undo stack");

        let resp = handle_dispatch_action(&mut state, &tx, Action::Undo);
        assert_eq!(json_outcome(&resp).as_deref(), Some("handled"));
        let after = state.document.as_ref().unwrap().undo_stack.len();
        assert!(after < before, "undo should pop the stack");
    }

    #[test]
    fn test_dispatch_zoom_in_updates_virtual_camera() {
        let mut state = test_state();
        let tx = test_event_tx();
        let before = state.camera.zoom;
        let resp = handle_dispatch_action(&mut state, &tx, Action::ZoomIn);
        assert_eq!(json_outcome(&resp).as_deref(), Some("handled"));
        assert!(
            state.camera.zoom > before,
            "ZoomIn must increase virtual camera zoom (was {before}, now {})",
            state.camera.zoom
        );
    }

    #[test]
    fn test_dispatch_pan_camera_updates_virtual_position() {
        let mut state = test_state();
        let tx = test_event_tx();
        let before = state.camera.position;
        let _ = handle_dispatch_action(&mut state, &tx, Action::PanCameraNorth);
        let after_north = state.camera.position;
        assert_ne!(
            after_north, before,
            "PanCameraNorth must move the virtual camera"
        );
        // PanCameraNorth uses `dy = -PAN_STEP_PX`. With the inverse-zoom
        // transform, the camera position's y component shifts by
        // `-(-PAN_STEP_PX) * (1/zoom) = +PAN_STEP_PX/zoom`. Sign-only
        // check is enough.
        assert!(after_north.y > before.y, "PanCameraNorth y direction");
    }

    #[test]
    fn test_dispatch_zoom_reset_returns_camera_to_unity_relative() {
        let mut state = test_state();
        let tx = test_event_tx();
        // Zoom in twice then reset.
        let _ = handle_dispatch_action(&mut state, &tx, Action::ZoomIn);
        let _ = handle_dispatch_action(&mut state, &tx, Action::ZoomIn);
        let _ = handle_dispatch_action(&mut state, &tx, Action::ZoomReset);
        assert!(
            (state.camera.zoom - 1.0).abs() < 1e-3,
            "ZoomReset should land near zoom=1.0; got {}",
            state.camera.zoom
        );
    }

    #[test]
    fn test_dispatch_toggle_fps_rotates_through_modes() {
        use crate::application::common::FpsDisplayMode;
        let mut state = test_state();
        let tx = test_event_tx();
        assert_eq!(state.fps_mode, FpsDisplayMode::Off);
        let _ = handle_dispatch_action(&mut state, &tx, Action::ToggleFps);
        assert_eq!(state.fps_mode, FpsDisplayMode::Snapshot);
        let _ = handle_dispatch_action(&mut state, &tx, Action::ToggleFps);
        assert_eq!(state.fps_mode, FpsDisplayMode::Off);
    }

    #[test]
    fn test_dispatch_delete_selection_requires_a_selection() {
        // DeleteSelection on an empty selection is a no-op from the
        // model's perspective, but the funnel reports `Handled`. The
        // important assertion is that node count is unchanged.
        let mut state = test_state();
        let tx = test_event_tx();
        let before = state.document.as_ref().unwrap().mindmap.nodes.len();
        let _ = handle_dispatch_action(&mut state, &tx, Action::DeleteSelection);
        let after = state.document.as_ref().unwrap().mindmap.nodes.len();
        assert_eq!(after, before, "no selection -> no deletion");
    }

    #[test]
    fn test_dispatch_no_document_returns_503() {
        let mut state = HeadlessState::empty();
        let tx = test_event_tx();
        let resp = handle_dispatch_action(&mut state, &tx, Action::Undo);
        match resp {
            IpcResponse::Err(code, msg) => {
                assert_eq!(code, axum::http::StatusCode::SERVICE_UNAVAILABLE);
                assert!(msg.contains("no document"), "msg: {msg}");
            }
            other => panic!("expected 503, got {other:?}"),
        }
    }
}
