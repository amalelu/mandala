// SPDX-License-Identifier: MPL-2.0

//! HTTP route handlers + the main-thread dispatcher that completes
//! requests that need to touch live state.
//!
//! ## Two halves
//!
//! - `async fn` handlers: run on the tokio IPC thread, axum-managed.
//!   For read-only routes (`/state*`, `/logs`, `/healthz`) they read
//!   the `Arc<RwLock<StateSnapshot>>` and return JSON directly. For
//!   routes that must mutate state, they post an
//!   [`super::IpcRequest`] to the main thread via
//!   [`super::RequestSink`] and await the reply on a oneshot.
//!
//! - `handle_main` / `handle_main_headless`: invoked from the main
//!   thread (winit's `user_event` / the headless tick loop) to
//!   service a single [`super::IpcRequestPayload`]. Returns the
//!   [`super::IpcResponse`] that the HTTP handler is awaiting.
//!
//! Until [`super::server::serve`] wires this up the body is just
//! the dispatcher half — enough to compile and unit-test.

use super::error::ApiError;
use super::{IpcHandle, IpcRequest, IpcRequestPayload, IpcResponse, RunMode};
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;

/// `GET /healthz` — constant response, never touches the main
/// thread. Cheap probe for clients that want to confirm the IPC
/// server is up before issuing slower calls.
pub async fn healthz(State(handle): State<IpcHandle>) -> Json<serde_json::Value> {
    let mode = match handle.mode {
        RunMode::Windowed => "windowed",
        RunMode::Headless => "headless",
    };
    Json(serde_json::json!({
        "ok": true,
        "pid": std::process::id(),
        "version": env!("CARGO_PKG_VERSION"),
        "mode": mode,
    }))
}

/// `GET /state` — full snapshot.
pub async fn get_state(
    State(handle): State<IpcHandle>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let snap = handle
        .snapshot
        .read()
        .map_err(|_| ApiError::unavailable("snapshot lock poisoned"))?;
    serde_json::to_value(&*snap)
        .map(Json)
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("serialise: {e}")))
}

/// Dispatch helper that posts a payload to the main thread and
/// awaits the response on a fresh oneshot. Used by every mutating
/// route handler.
pub async fn run_on_main(
    handle: &IpcHandle,
    payload: IpcRequestPayload,
) -> Result<IpcResponse, ApiError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    handle
        .sink
        .send(IpcRequest { payload, responder: tx })
        .map_err(|e| ApiError::unavailable(e))?;
    rx.await
        .map_err(|_| ApiError::unavailable("main thread dropped responder"))
}

/// Convert an [`IpcResponse`] into an axum response shape suitable
/// for `Result<...>` propagation in route handlers.
pub fn response_into_json(resp: IpcResponse) -> Result<Json<serde_json::Value>, ApiError> {
    match resp {
        IpcResponse::Ok(v) => Ok(Json(v)),
        IpcResponse::Bytes(_, _) => Err(ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "expected JSON, got bytes".into(),
        )),
        IpcResponse::Err(code, msg) => Err(ApiError(code, msg)),
    }
}

/// Stub for the windowed main-thread dispatcher. Filled in once
/// `InitState` plumbing lands; for now every request returns
/// 503 so the server can boot and routes can be wired without
/// blocking on `InitState` access.
#[cfg(not(target_arch = "wasm32"))]
pub fn handle_main_stub(payload: IpcRequestPayload) -> IpcResponse {
    let kind = match payload {
        IpcRequestPayload::DispatchAction(_) => "DispatchAction",
        IpcRequestPayload::ConsoleLine(_) => "ConsoleLine",
        IpcRequestPayload::CustomMutation { .. } => "CustomMutation",
        IpcRequestPayload::Screenshot { .. } => "Screenshot",
        IpcRequestPayload::Scene => "Scene",
        IpcRequestPayload::HitTest { .. } => "HitTest",
        IpcRequestPayload::FullState => "FullState",
    };
    IpcResponse::Err(
        StatusCode::SERVICE_UNAVAILABLE,
        format!("main-thread dispatch for {kind} not implemented yet"),
    )
}
