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
use crate::application::ipc::state_view::{DocumentView, StateSnapshot};
use crate::application::ipc::{
    self, log_buffer, IpcEvent, IpcRequest, IpcRequestPayload, IpcResponse, RequestSink, RunMode,
};

/// Headless-mode application state. Owns the document and any
/// future cross-platform model state. No window, no renderer.
struct HeadlessState {
    document: Option<MindMapDocument>,
}

impl HeadlessState {
    fn new(mindmap_path: &str) -> Self {
        let document = match MindMapDocument::load(mindmap_path) {
            Ok(mut doc) => {
                let (app_mutations, user_mutations) =
                    crate::application::document::mutations_loader::load_app_and_user(None);
                doc.build_mutation_registry_with_app_and_user(&app_mutations, &user_mutations);
                crate::application::document::mutations::register_builtin_handlers(&mut doc);
                Some(doc)
            }
            Err(e) => {
                log::error!("headless: failed to load `{mindmap_path}`: {e}");
                None
            }
        };
        Self { document }
    }

    fn build_snapshot(&self) -> StateSnapshot {
        let document = match &self.document {
            Some(doc) => DocumentView {
                mindmap: serde_json::to_value(&doc.mindmap).ok(),
                file_path: doc.file_path.clone(),
                dirty: doc.dirty,
                undo_depth: doc.undo_stack.len(),
                active_animation_count: 0,
            },
            None => DocumentView::default(),
        };
        StateSnapshot {
            document,
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
    let (event_tx, _event_rx_dropper) =
        tokio::sync::broadcast::channel::<IpcEvent>(1024);
    let log_buf = Arc::new(Mutex::new(VecDeque::with_capacity(
        log_buffer::LOG_RING_CAPACITY,
    )));

    // Install the tee logger before the IPC thread starts so the
    // first server log lines are captured.
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

    // Tick loop: handle pending requests, then re-publish the
    // snapshot so /state reflects any mutations. Naps up to one
    // tick when idle.
    let tick = Duration::from_millis(16);
    loop {
        let mut had_request = false;
        match req_rx.recv_timeout(tick) {
            Ok(req) => {
                had_request = true;
                let resp = handle_request(&mut state, req.payload);
                let _ = req.responder.send(resp);
                while let Ok(req) = req_rx.try_recv() {
                    let resp = handle_request(&mut state, req.payload);
                    let _ = req.responder.send(resp);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if had_request {
            *snapshot.write().expect("snapshot lock") = state.build_snapshot();
        }
    }
}

fn handle_request(state: &mut HeadlessState, payload: IpcRequestPayload) -> IpcResponse {
    use axum::http::StatusCode;
    match payload {
        IpcRequestPayload::FullState => IpcResponse::Ok(
            serde_json::to_value(state.build_snapshot()).unwrap_or(serde_json::Value::Null),
        ),
        IpcRequestPayload::Scene => match state.document.as_ref() {
            Some(doc) => match serde_json::to_value(&doc.mindmap) {
                Ok(v) => IpcResponse::Ok(v),
                Err(e) => IpcResponse::Err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("scene serialise: {e}"),
                ),
            },
            None => IpcResponse::Err(
                StatusCode::SERVICE_UNAVAILABLE,
                "no document loaded".into(),
            ),
        },
        IpcRequestPayload::Screenshot { .. } => IpcResponse::Err(
            StatusCode::NOT_IMPLEMENTED,
            "screenshot unavailable in headless mode; use /scene for the document JSON".into(),
        ),
        other => crate::application::ipc::routes::handle_main_stub(other),
    }
}
