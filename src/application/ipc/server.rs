// SPDX-License-Identifier: MPL-2.0

//! tokio runtime entry point and axum `Router` assembly. Run from
//! the dedicated `mandala-ipc` thread via
//! [`super::boot`].

use std::net::SocketAddr;

use axum::routing::{get, post};
use axum::Router;

use super::routes;
use super::sse;
use super::IpcHandle;

/// Build the route table.
///
/// Two tiers of routes:
///
/// 1. **Implemented today** — snapshot-derived GETs (`/healthz`,
///    `/state`, `/state/document`, `/actions`) and the SSE stream
///    (`/events`).
/// 2. **Deferred** — registered with stub handlers that route through
///    `routes::run_on_main` → `handle_main_stub` and return 503 with a
///    machine-readable `not implemented yet` body so clients see a
///    documented surface rather than 404. Replaced by real handlers
///    in the next milestone. Per CLAUDE.md §3, the deferral is
///    explicit at the route table rather than hidden by absent routes.
pub fn router(handle: IpcHandle) -> Router {
    Router::new()
        // Implemented.
        .route("/healthz", get(routes::healthz))
        .route("/state", get(routes::get_state))
        .route("/state/document", get(routes::get_document))
        .route("/actions", get(routes::list_actions))
        .route("/events", get(sse::events_handler))
        // Substate projections from the live snapshot (no main-thread
        // round-trip; the headless / windowed tick already publishes a
        // fresh snapshot on each frame).
        .route("/state/selection", get(routes::get_selection))
        .route("/state/camera", get(routes::get_camera))
        .route("/state/interaction_mode", get(routes::get_interaction_mode))
        // Deferred — 503 stubs.
        .route("/state/console", get(routes::stub_get))
        .route("/logs", get(routes::stub_get))
        .route("/hit_test", get(routes::stub_hit_test))
        .route("/screenshot", get(routes::stub_screenshot))
        .route("/actions/dispatch", post(routes::stub_dispatch_action))
        .route("/console", post(routes::stub_console))
        .route("/mutations/:id", post(routes::stub_custom_mutation))
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
