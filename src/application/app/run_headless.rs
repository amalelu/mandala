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
use crate::application::ipc::{
    self, log_buffer, IpcEvent, IpcRequest, IpcRequestPayload, IpcResponse, RequestSink, RunMode,
    StateSnapshot,
};

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

    log::info!(
        "mandala headless: serving IPC on {addr} (no window, no wgpu); ctrl-c to exit"
    );

    // Tick loop: handle pending requests, then nap up to TICK_MS
    // until the next request arrives. No `MindMapDocument` lives
    // here yet — that lands once CoreState is extracted. For now
    // every request gets `routes::handle_main_stub` (503).
    let tick = Duration::from_millis(16);
    loop {
        match req_rx.recv_timeout(tick) {
            Ok(req) => {
                let resp = handle_request(req.payload);
                let _ = req.responder.send(resp);
                while let Ok(req) = req_rx.try_recv() {
                    let resp = handle_request(req.payload);
                    let _ = req.responder.send(resp);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn handle_request(payload: IpcRequestPayload) -> IpcResponse {
    crate::application::ipc::routes::handle_main_stub(payload)
}
