// SPDX-License-Identifier: MPL-2.0

//! HTTP + Server-Sent Events server for the Claude Code feedback
//! loop. Active only when the binary was launched with
//! `--ipc-port=NNNN`. Bound to loopback. Dev-only.
//!
//! Architecture: a dedicated `std::thread` runs a tokio multi-thread
//! runtime hosting axum. HTTP handlers either read an
//! `Arc<RwLock<StateSnapshot>>` (snapshot rebuilt at the end of every
//! frame on the main thread) or send `IpcRequest`s back through a
//! [`RequestSink`] — `EventLoopProxy<UserEvent>` in windowed mode,
//! `crossbeam_channel::Sender<IpcRequest>` in headless mode. The
//! main thread fulfils the request and replies via a
//! `tokio::sync::oneshot::Sender<IpcResponse>` the HTTP handler is
//! awaiting.
//!
//! See `docs/ipc.md` for the wire format and `CONCEPTS.md` (IPC
//! section) for the conceptual model.

#![cfg(not(target_arch = "wasm32"))]

pub mod error;
pub mod log_buffer;
pub mod routes;
pub mod screenshot;
pub mod server;
pub mod sse;
pub mod state_view;

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, RwLock};

pub use log_buffer::LogLine;
pub use sse::IpcEvent;
pub use state_view::StateSnapshot;

/// Dev-only IPC runtime mode. Selected at startup by the CLI flags
/// (`--ipc-port` / `--headless`). Threaded through every IPC handle
/// so handlers know whether the renderer is available, whether
/// camera-bearing actions can run, and whether `/screenshot` is
/// reachable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunMode {
    /// Standard interactive launch with an OS window. `--ipc-port`
    /// was set but `--headless` was not. Renderer is live.
    Windowed,
    /// No window, no wgpu. IPC is the only interface. Reachable
    /// only when `--headless` is set. Anthropic-VM / CI use case.
    Headless,
}

/// Payload of a request posted from the IPC thread to the main
/// thread (winit or headless tick loop). Each variant matches one
/// HTTP route that must touch live application state. The
/// HTTP handler waits on `IpcRequest::responder` for the
/// corresponding `IpcResponse`.
#[derive(Debug)]
pub enum IpcRequestPayload {
    /// `POST /actions/dispatch` — dispatch a single `Action`
    /// through the same funnel the keyboard goes through, tagged
    /// `MacroSource::Ipc`.
    DispatchAction(crate::application::keybinds::Action),
    /// `POST /console` — run one console line.
    ConsoleLine(String),
    /// `POST /mutations/{id}` — apply a CustomMutation.
    CustomMutation {
        id: String,
        target: Option<String>,
    },
    /// `GET /screenshot?width=&height=` — windowed-only; returns
    /// 501 in headless.
    Screenshot { width: u32, height: u32 },
    /// `GET /hit_test?x=&y=` — returns the document hit at canvas
    /// coordinates `(x, y)`.
    HitTest { x: f64, y: f64 },
    /// `GET /state` fallback when the snapshot is stale (e.g. first
    /// frame before the main thread has run a drain). Also used by
    /// the deferred-route stubs (`/state/{selection,camera,...}`,
    /// `/logs`) until they get dedicated dispatcher arms.
    FullState,
}

/// Response from the main thread back to the HTTP handler waiting
/// on the oneshot. The HTTP layer converts this into the final
/// axum `Response` — `Ok` becomes `Json(...)`, `Bytes` sets the
/// content-type, `Err` becomes an [`ApiError`] (status + JSON
/// `{"error": ...}` body).
#[derive(Debug)]
pub enum IpcResponse {
    Ok(serde_json::Value),
    Bytes(Vec<u8>, &'static str),
    Err(axum::http::StatusCode, String),
}

/// One in-flight request from the IPC thread to the main thread.
/// `responder` is a single-use oneshot: the main thread sends
/// exactly one `IpcResponse` and drops its end; the HTTP handler
/// awaits it and converts the result to an axum response.
///
/// Not `Debug` — the responder oneshot isn't `Debug`. `payload` is
/// loggable on its own where useful.
pub struct IpcRequest {
    pub payload: IpcRequestPayload,
    pub responder: tokio::sync::oneshot::Sender<IpcResponse>,
}

/// User-event payload for winit's `EventLoop<UserEvent>` in windowed
/// mode. Only one variant for now (`Ipc`) but a future ScreenReader
/// integration or a clipboard-async completion would land here too.
///
/// Not `Debug` because `IpcRequest::responder` (a oneshot sender)
/// isn't `Debug`. winit's `EventLoopProxy::send_event` doesn't
/// require Debug.
pub enum UserEvent {
    Ipc(IpcRequest),
}

/// Cross-thread request transport. Windowed mode pipes through
/// winit's `EventLoopProxy`; headless mode pipes through a plain
/// crossbeam channel that the tick loop polls. Both ultimately
/// reach the same `routes::handle_main*` dispatcher on the main
/// thread.
#[derive(Clone)]
pub enum RequestSink {
    Windowed(winit::event_loop::EventLoopProxy<UserEvent>),
    Headless(crossbeam_channel::Sender<IpcRequest>),
}

impl RequestSink {
    /// Send a request to the main thread. Returns `Err` if the
    /// receiving side has been dropped (event loop closed, headless
    /// loop exited).
    pub fn send(&self, req: IpcRequest) -> Result<(), &'static str> {
        match self {
            Self::Windowed(p) => p
                .send_event(UserEvent::Ipc(req))
                .map_err(|_| "event loop closed"),
            Self::Headless(s) => s.send(req).map_err(|_| "headless loop closed"),
        }
    }
}

/// Shared handle exposed to both the main thread and the IPC
/// thread. Cheap to clone (everything inside is `Arc` /
/// `broadcast::Sender` / channel sender). The main thread reads /
/// writes `snapshot` at frame boundaries; the IPC thread reads
/// `snapshot` from HTTP handlers and `sink`s requests through it.
#[derive(Clone)]
pub struct IpcHandle {
    pub sink: RequestSink,
    pub snapshot: Arc<RwLock<StateSnapshot>>,
    pub event_tx: tokio::sync::broadcast::Sender<IpcEvent>,
    pub log_buf: Arc<Mutex<VecDeque<LogLine>>>,
    pub mode: RunMode,
}

/// Spawn the dedicated `mandala-ipc` thread, build a tokio
/// multi-thread runtime on it, and run the axum server forever.
/// Called from `run_native::run` (windowed) or `run_headless::run`
/// (headless) when `--ipc-port` is set. Never returns — the
/// thread parks inside `tokio::runtime::Runtime::block_on`.
pub fn boot(
    addr: SocketAddr,
    sink: RequestSink,
    snapshot: Arc<RwLock<StateSnapshot>>,
    event_tx: tokio::sync::broadcast::Sender<IpcEvent>,
    log_buf: Arc<Mutex<VecDeque<LogLine>>>,
    mode: RunMode,
) {
    let handle = IpcHandle {
        sink,
        snapshot,
        event_tx,
        log_buf,
        mode,
    };
    std::thread::Builder::new()
        .name("mandala-ipc".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_io()
                .enable_time()
                .build()
                .expect("ipc tokio runtime");
            rt.block_on(server::serve(addr, handle));
        })
        .expect("spawn mandala-ipc thread");
}
