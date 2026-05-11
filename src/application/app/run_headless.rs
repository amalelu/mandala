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
use crate::application::ipc::state_view::{DocumentView, SelectionView, StateSnapshot};
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
}

impl HeadlessState {
    fn new(mindmap_path: &str) -> Self {
        match MindMapDocument::load(mindmap_path) {
            Ok(mut doc) => {
                let (app_mutations, user_mutations) =
                    crate::application::document::mutations_loader::load_app_and_user(None);
                doc.build_mutation_registry_with_app_and_user(&app_mutations, &user_mutations);
                crate::application::document::mutations::register_builtin_handlers(&mut doc);
                Self { document: Some(doc) }
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
        let Some(doc) = &self.document else {
            return StateSnapshot::default();
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
    loop {
        match req_rx.recv_timeout(HEADLESS_TICK) {
            Ok(req) => {
                let resp = handle_request(&mut state, &event_tx, req.payload);
                let _ = req.responder.send(resp);
                while let Ok(req) = req_rx.try_recv() {
                    let resp = handle_request(&mut state, &event_tx, req.payload);
                    let _ = req.responder.send(resp);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        *snapshot.write().expect("snapshot lock") = state.build_snapshot();
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

/// `POST /actions/dispatch` (headless) — runs a curated subset of
/// the [`Action`](crate::application::keybinds::Action) surface
/// that is reachable without a `Renderer` / `AppScene` /
/// `SceneConnectionCache`. Tagged
/// [`MacroSource::Ipc`](crate::application::macros::MacroSource::Ipc).
///
/// **§3 funnel deviation, documented.** The windowed dispatcher
/// is the canonical single funnel for `Action`s. Headless can't
/// reach `dispatch_compatible` because that function takes an
/// `InputContextCore` carrying GPU/scene state we don't have. This
/// handler instead calls the same `pub(in crate::application::app)`
/// `*_in(doc) -> bool` primitives that the dispatcher's `apply_*`
/// helpers wrap — i.e. it shares the document-mutation logic and
/// only skips the scene-rebuild call (headless has no scene to
/// rebuild; the next snapshot tick re-publishes the document).
/// The full windowed-mode wiring is the next milestone — once
/// `EventLoop<UserEvent>` lands, the windowed path will reach
/// `dispatch_compatible` and the curated subset becomes the
/// headless-only fallback.
///
/// Unsupported Actions (everything that needs camera / scene /
/// renderer state, every NativeOnly modal, every clipboard verb)
/// return 501 NOT_IMPLEMENTED with a message naming the variant.
fn handle_dispatch_action(
    state: &mut HeadlessState,
    event_tx: &tokio::sync::broadcast::Sender<IpcEvent>,
    action: crate::application::keybinds::Action,
) -> IpcResponse {
    use axum::http::StatusCode;
    use crate::application::app::dispatch::cross_dispatch as cd;
    use crate::application::keybinds::Action;

    let Some(doc) = state.document.as_mut() else {
        return IpcResponse::Err(
            StatusCode::SERVICE_UNAVAILABLE,
            "no document loaded".into(),
        );
    };

    let changed = match &action {
        Action::Undo => {
            // Fast-forward any in-flight animation snapshot first
            // (mirrors `apply_undo`'s native body so behaviour is
            // platform-uniform even though headless never ticks
            // animations).
            doc.fast_forward_animations(None);
            doc.undo()
        }
        Action::SelectAll => cd::select_all_in(doc),
        Action::DeselectAll => cd::deselect_all_in(doc),
        Action::InvertSelection => cd::invert_selection_in(doc),
        Action::SelectParent => cd::select_parent_in(doc),
        Action::SelectChild => cd::select_child_in(doc),
        Action::SelectNextSibling => cd::select_sibling_in(doc, true),
        Action::SelectPrevSibling => cd::select_sibling_in(doc, false),
        Action::JumpToRoot => {
            // The native body returns the root's canvas position so
            // the camera can re-centre; headless has no camera, so
            // we discard the position. The selection move (root node
            // becomes the single selection) is what an agent reads.
            cd::jump_to_root_in(doc).is_some()
        }
        _ => {
            return IpcResponse::Err(
                StatusCode::NOT_IMPLEMENTED,
                format!(
                    "Action variant not yet reachable in headless mode: {:?}. \
                     Headless dispatch is limited to a curated document-only \
                     subset; the full Action surface lands once windowed-mode \
                     IPC wiring goes through dispatch_compatible.",
                    action
                ),
            );
        }
    };

    let outcome = if changed { "handled" } else { "unchanged" };
    let _ = event_tx.send(IpcEvent::ActionDispatched {
        action: action.clone(),
        outcome: outcome.into(),
    });

    IpcResponse::Ok(serde_json::json!({
        "outcome": outcome,
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
        HeadlessState { document: Some(doc) }
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
    fn test_dispatch_deselect_all_on_empty_selection_is_unchanged() {
        let mut state = test_state();
        let tx = test_event_tx();
        let resp = handle_dispatch_action(&mut state, &tx, Action::DeselectAll);
        // A fresh testament starts with selection=None, so deselect is a no-op.
        assert_eq!(json_outcome(&resp).as_deref(), Some("unchanged"));
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
    fn test_dispatch_unsupported_action_returns_501() {
        let mut state = test_state();
        let tx = test_event_tx();
        // `OpenConsole` requires modal state that headless doesn't carry.
        let resp = handle_dispatch_action(&mut state, &tx, Action::OpenConsole);
        match resp {
            IpcResponse::Err(code, msg) => {
                assert_eq!(code, axum::http::StatusCode::NOT_IMPLEMENTED);
                assert!(
                    msg.contains("OpenConsole"),
                    "error message should name the variant; got: {msg}"
                );
            }
            other => panic!("expected NOT_IMPLEMENTED error, got {other:?}"),
        }
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
    fn test_dispatch_no_document_returns_503() {
        let mut state = HeadlessState { document: None };
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
