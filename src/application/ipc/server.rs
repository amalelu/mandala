// SPDX-License-Identifier: MPL-2.0

//! tokio runtime entry point and axum `Router` assembly. Run from
//! the dedicated `mandala-ipc` thread via
//! [`super::boot`].

use std::net::SocketAddr;

use axum::routing::get;
use axum::Router;

use super::routes;
use super::sse;
use super::IpcHandle;

/// Build the route table. Read-only paths today (`/healthz`,
/// `/state`, `/events`); the mutating routes land in a follow-up.
pub fn router(handle: IpcHandle) -> Router {
    Router::new()
        .route("/healthz", get(routes::healthz))
        .route("/state", get(routes::get_state))
        .route("/state/document", get(routes::get_document))
        .route("/scene", get(routes::get_scene))
        .route("/actions", get(routes::list_actions))
        .route("/events", get(sse::events_handler))
        .with_state(handle)
}

/// Bind and serve forever. Loopback-only by construction —
/// `SocketAddr` is built from `[127, 0, 0, 1]` in the CLI parse,
/// not from user-supplied host strings.
pub async fn serve(addr: SocketAddr, handle: IpcHandle) {
    let app = router(handle);
    match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => {
            log::info!("mandala IPC listening on http://{}", addr);
            if let Err(e) = axum::serve(listener, app).await {
                log::error!("mandala IPC server error: {e}");
            }
        }
        Err(e) => {
            log::error!("mandala IPC bind {} failed: {e}", addr);
        }
    }
}
