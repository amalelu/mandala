// SPDX-License-Identifier: MPL-2.0

//! HTTP route handlers + the main-thread dispatcher that completes
//! requests which must touch live state.
//!
//! ## Two halves
//!
//! - `async fn` handlers: run on the tokio IPC thread, axum-managed.
//!   Snapshot-derived GETs read the `Arc<RwLock<StateSnapshot>>` and
//!   return JSON directly. Routes that must mutate state — or that
//!   require renderer access — post an [`super::IpcRequest`] to the
//!   main thread via [`super::RequestSink`] and await the reply on a
//!   bounded oneshot.
//!
//! - `handle_main` / `handle_main_stub`: invoked from the main
//!   thread (winit's `user_event` / the headless tick loop) to
//!   service a single [`super::IpcRequestPayload`]. Returns the
//!   [`super::IpcResponse`] the HTTP handler is awaiting.

use std::time::Duration;

use super::error::ApiError;
use super::{IpcHandle, IpcRequest, IpcRequestPayload, IpcResponse, RunMode};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

/// Maximum time the HTTP handler waits for the main thread to fulfil
/// an [`IpcRequest`]. Five seconds is generous for snapshot/dispatch
/// work and short enough to surface main-thread hangs as 504s rather
/// than slowly-leaking connections.
const MAIN_THREAD_TIMEOUT: Duration = Duration::from_secs(5);

/// `GET /healthz` — constant response, never touches the main thread.
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

/// Read the latest snapshot under a poison-tolerant lock guard. A
/// poisoned `RwLock` means a previous writer panicked; the snapshot
/// is replaced wholesale each frame so reading through poison is
/// safe (`PoisonError::into_inner()`).
fn read_snapshot(handle: &IpcHandle) -> super::state_view::StateSnapshot {
    match handle.snapshot.read() {
        Ok(g) => (*g).clone(),
        Err(poisoned) => (*poisoned.into_inner()).clone(),
    }
}

/// `GET /state` — full snapshot.
pub async fn get_state(
    State(handle): State<IpcHandle>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let snap = read_snapshot(&handle);
    serde_json::to_value(&snap)
        .map(Json)
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("serialise: {e}")))
}

/// `GET /state/document` — just the `MindMap` JSON, read from the
/// cached snapshot.
pub async fn get_document(
    State(handle): State<IpcHandle>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let snap = read_snapshot(&handle);
    match snap.document.mindmap {
        Some(mm) => Ok(Json(mm)),
        None => Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "no document loaded".into(),
        )),
    }
}

/// `GET /state/selection` — selection projection from the snapshot.
pub async fn get_selection(
    State(handle): State<IpcHandle>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let snap = read_snapshot(&handle);
    serde_json::to_value(&snap.selection).map(Json).map_err(|e| {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("serialise: {e}"))
    })
}

/// `GET /state/camera` — camera position + zoom from the snapshot.
/// In headless mode this is the virtual camera; in windowed mode
/// it will read from the live renderer once windowed IPC lands.
pub async fn get_camera(
    State(handle): State<IpcHandle>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let snap = read_snapshot(&handle);
    serde_json::to_value(&snap.camera).map(Json).map_err(|e| {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("serialise: {e}"))
    })
}

/// `GET /state/interaction_mode` — current modal state.
pub async fn get_interaction_mode(
    State(handle): State<IpcHandle>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let snap = read_snapshot(&handle);
    serde_json::to_value(&snap.interaction_mode)
        .map(Json)
        .map_err(|e| {
            ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("serialise: {e}"))
        })
}

/// `GET /actions` — list every Action variant with its classifier
/// metadata. Wire names come from `strum::IntoStaticStr`, NOT from
/// `Debug` — variant renames stay caught by the compiler and the
/// wire format is declarative.
pub async fn list_actions() -> Json<serde_json::Value> {
    Json(serde_json::Value::Array(action_listing()))
}

/// Sync helper behind [`list_actions`]; kept separate so tests can
/// exercise the listing without spinning up a tokio runtime
/// (TEST_CONVENTIONS §T10 forbids an async test harness).
pub fn action_listing() -> Vec<serde_json::Value> {
    use crate::application::keybinds::ActionKind;
    use strum::IntoEnumIterator;

    ActionKind::iter()
        .map(|k| {
            let kind_name: &'static str = k.into();
            let context_name: &'static str = k.context().into();
            let wasm_name: &'static str = k.wasm_compatibility().into();
            serde_json::json!({
                "kind": kind_name,
                "destructive": k.is_destructive(),
                "context": context_name,
                "wasm_compatibility": wasm_name,
            })
        })
        .collect()
}

/// Post a payload to the main thread and await the response, with a
/// [`MAIN_THREAD_TIMEOUT`] cap. A hung main thread surfaces as 504
/// rather than an indefinitely-held tcp connection. Used by every
/// route that must touch live state.
pub async fn run_on_main(
    handle: &IpcHandle,
    payload: IpcRequestPayload,
) -> Result<IpcResponse, ApiError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    handle
        .sink
        .send(IpcRequest { payload, responder: tx })
        .map_err(ApiError::unavailable)?;
    match tokio::time::timeout(MAIN_THREAD_TIMEOUT, rx).await {
        Ok(Ok(resp)) => Ok(resp),
        Ok(Err(_)) => Err(ApiError::unavailable("main thread dropped responder")),
        Err(_) => Err(ApiError(
            StatusCode::GATEWAY_TIMEOUT,
            format!(
                "main thread did not respond within {}s",
                MAIN_THREAD_TIMEOUT.as_secs()
            ),
        )),
    }
}

/// Convert an [`IpcResponse`] into an axum response shape suitable
/// for `Result<...>` propagation in JSON route handlers.
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

// ─────────────────────────── Deferred-route stubs ────────────────────────────
//
// Each stub posts the payload to the main thread (so the wire shape
// stays identical to the eventual real implementation) and returns
// the `handle_main_stub` 503 body. Replaced by real handlers in
// the next milestone — see plan §"Module Layout" and CLAUDE.md §3
// on making deferred work explicit at the route table.

#[derive(Debug, Deserialize)]
pub struct HitTestQuery {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Deserialize)]
pub struct ScreenshotQuery {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Deserialize)]
pub struct DispatchActionBody {
    pub action: crate::application::keybinds::Action,
}

#[derive(Debug, Deserialize)]
pub struct ConsoleBody {
    pub line: String,
}

#[derive(Debug, Deserialize)]
pub struct MutationBody {
    pub target: Option<String>,
}

/// Generic GET stub for `/state/{selection,camera,console,interaction_mode}`
/// and `/logs`. The plan defers these to the snapshot-projection / log-
/// filter slice; for now they route through the main thread and return
/// 503 with a documented body shape.
pub async fn stub_get(
    State(handle): State<IpcHandle>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let resp = run_on_main(&handle, IpcRequestPayload::FullState).await?;
    response_into_json(resp)
}

pub async fn stub_hit_test(
    State(handle): State<IpcHandle>,
    Query(q): Query<HitTestQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let resp = run_on_main(&handle, IpcRequestPayload::HitTest { x: q.x, y: q.y }).await?;
    response_into_json(resp)
}

pub async fn stub_screenshot(
    State(handle): State<IpcHandle>,
    Query(q): Query<ScreenshotQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let resp = run_on_main(
        &handle,
        IpcRequestPayload::Screenshot {
            width: q.width,
            height: q.height,
        },
    )
    .await?;
    response_into_json(resp)
}

pub async fn stub_dispatch_action(
    State(handle): State<IpcHandle>,
    Json(body): Json<DispatchActionBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let resp = run_on_main(&handle, IpcRequestPayload::DispatchAction(body.action)).await?;
    response_into_json(resp)
}

pub async fn stub_console(
    State(handle): State<IpcHandle>,
    Json(body): Json<ConsoleBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let resp = run_on_main(&handle, IpcRequestPayload::ConsoleLine(body.line)).await?;
    response_into_json(resp)
}

pub async fn stub_custom_mutation(
    State(handle): State<IpcHandle>,
    Path(id): Path<String>,
    Json(body): Json<MutationBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let resp = run_on_main(
        &handle,
        IpcRequestPayload::CustomMutation {
            id,
            target: body.target,
        },
    )
    .await?;
    response_into_json(resp)
}

/// Main-thread dispatcher stub. Filled in once `CoreState` / `InitState`
/// plumbing lands. For now every payload returns a documented 503 body
/// so HTTP clients see a stable shape.
#[cfg(not(target_arch = "wasm32"))]
pub fn handle_main_stub(payload: IpcRequestPayload) -> IpcResponse {
    let kind = match payload {
        IpcRequestPayload::DispatchAction(_) => "DispatchAction",
        IpcRequestPayload::ConsoleLine(_) => "ConsoleLine",
        IpcRequestPayload::CustomMutation { .. } => "CustomMutation",
        IpcRequestPayload::Screenshot { .. } => "Screenshot",
        IpcRequestPayload::HitTest { .. } => "HitTest",
        IpcRequestPayload::FullState => "FullState",
    };
    IpcResponse::Err(
        StatusCode::SERVICE_UNAVAILABLE,
        format!("main-thread dispatch for {kind} not implemented yet"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ipc::state_view::{SelectionView, StateSnapshot};
    use std::sync::{Arc, RwLock};

    fn make_handle(snapshot: Arc<RwLock<StateSnapshot>>) -> IpcHandle {
        let (req_tx, _req_rx) = crossbeam_channel::unbounded();
        let (event_tx, _ev_rx) = tokio::sync::broadcast::channel(16);
        let log_buf = Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
        IpcHandle {
            sink: super::super::RequestSink::Headless(req_tx),
            snapshot,
            event_tx,
            log_buf,
            mode: RunMode::Headless,
        }
    }

    #[test]
    fn test_action_listing_emits_strum_names_not_debug() {
        let entries = action_listing();
        // We don't pin specific variant names (the surface is large
        // and evolves), but we do verify the shape:
        //  - every entry has all four required keys,
        //  - `kind` is a string with no debug-formatting artifacts
        //    like surrounding curly braces or parens.
        assert!(!entries.is_empty(), "ActionKind iterator was empty");
        for e in &entries {
            for key in ["kind", "destructive", "context", "wasm_compatibility"] {
                assert!(e.get(key).is_some(), "missing key {key}: {e}");
            }
            let kind = e["kind"].as_str().expect("kind is string");
            assert!(
                !kind.contains('{') && !kind.contains('('),
                "kind looks like Debug output: {kind}"
            );
            let wasm = e["wasm_compatibility"].as_str().expect("wasm is string");
            assert!(
                wasm == "Compatible" || wasm == "NativeOnly",
                "unexpected wasm classification: {wasm}"
            );
        }
    }

    #[test]
    fn test_action_listing_contains_known_action() {
        let entries = action_listing();
        let has_undo = entries
            .iter()
            .any(|e| e["kind"].as_str() == Some("Undo"));
        assert!(has_undo, "expected the Undo variant in the listing");
    }

    #[test]
    fn test_read_snapshot_recovers_from_poisoned_lock() {
        // Construct a poisoned RwLock by panicking inside a write
        // guard on a worker thread.
        let snap = Arc::new(RwLock::new(StateSnapshot::default()));
        {
            let snap_clone = snap.clone();
            let _ = std::thread::spawn(move || {
                let mut g = snap_clone.write().expect("first writer");
                g.hovered_node = Some("x".into());
                panic!("simulated panic while holding write lock");
            })
            .join();
        }
        assert!(snap.read().is_err(), "lock should be poisoned");

        let handle = make_handle(snap);
        // read_snapshot must recover and return the data the panicker left behind.
        let recovered = read_snapshot(&handle);
        assert_eq!(recovered.hovered_node, Some("x".into()));
    }

    #[test]
    fn test_read_snapshot_returns_current_data() {
        let snap = Arc::new(RwLock::new(StateSnapshot::default()));
        {
            let mut g = snap.write().expect("write");
            g.selection = SelectionView::Single { id: "abc".into() };
        }
        let handle = make_handle(snap);
        let r = read_snapshot(&handle);
        assert!(matches!(r.selection, SelectionView::Single { ref id } if id == "abc"));
    }
}
