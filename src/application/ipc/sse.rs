// SPDX-License-Identifier: MPL-2.0

//! Server-Sent Events surface. The main thread emits
//! [`IpcEvent`]s into a `tokio::sync::broadcast::Sender<IpcEvent>`;
//! each open SSE connection subscribes its own receiver. Capacity
//! 1024 means a slow subscriber tolerates ~16 seconds of typical
//! event traffic before lagging.

use std::convert::Infallible;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::{Stream, StreamExt};
use serde::Serialize;
use tokio_stream::wrappers::BroadcastStream;

use super::log_buffer::LogLine;
use super::state_view::SelectionView;
use super::IpcHandle;

/// One event broadcast to SSE subscribers. Serialised as JSON with
/// an externally-tagged `type` field (e.g.
/// `{"type":"selection_changed", ...}`) so clients can switch on it.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IpcEvent {
    SelectionChanged { selection: SelectionView },
    ActionDispatched { action: serde_json::Value, outcome: String },
    MutationApplied { id: String, target: Option<String> },
    FrameRendered { ts_ms: u64 },
    Log(LogLine),
    Error { source: String, message: String },
}

pub async fn events_handler(
    State(handle): State<IpcHandle>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = handle.event_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|res| async move {
        match res {
            Ok(ev) => match Event::default().json_data(&ev) {
                Ok(e) => Some(Ok(e)),
                Err(_) => None,
            },
            Err(_lag) => Some(Ok(Event::default().event("lagged").data("dropped"))),
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}
