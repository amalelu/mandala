// SPDX-License-Identifier: MPL-2.0

//! Local inter-process control: the socket an external process — an
//! LLM agent, a test harness, `mandalactl` — drives the running app
//! through. The wire contract is `format/ipc.md`; the design and its
//! rejected alternatives are `work_plans/LLM_IPC.md`.
//!
//! **The boundary contract** (CODE_CONVENTIONS §3, "Sanctioned
//! boundary threads"). Opt-in per launch with `--ipc <path>`; without
//! the flag nothing in this module runs. With it, dedicated threads
//! own the socket and move *protocol values* — decoded requests,
//! frames to encode — across `std::sync::mpsc`; they never see
//! `InitState`, the document or the renderer. Every command executes
//! on the main thread, between winit events, with the same access an
//! input handler has, so IPC never races the user and a command's
//! effects are ordered with theirs. The threads wake the parked event
//! loop through a winit user event ([`IpcWake`]), which is what lets
//! an idle app — parked in `ControlFlow::Wait`, the state an agent
//! most often finds it in — answer with no polling and no idle cost.
//!
//! Native-only at this module boundary, the `console_input`
//! precedent: browsers cannot serve local sockets (`CLAUDE.md`
//! "Dual-target status"). Within native, the transport is Unix-only
//! until the Windows named-pipe adapter lands; until then `--ipc` on
//! Windows fails at startup rather than doing nothing.

// The envelope is transport-agnostic, but the only code that reads
// what it builds is the Unix transport. Off Unix the module compiles
// with nothing draining it — the same shape as the wasm32 allows in
// `application/mod.rs`, scoped the same way so the Unix lint stays
// fully armed, and gone when the named-pipe transport (#169) lands.
mod commands;
#[cfg_attr(not(unix), allow(dead_code))]
mod envelope;
mod registry;
pub(crate) mod space;
mod target;
#[cfg(unix)]
mod transport;

pub(crate) use registry::execute;
pub(crate) use target::CommandTarget;
#[cfg(unix)]
pub(crate) use transport::IpcServer;

/// The winit user event the boundary threads send to wake the event
/// loop. It carries nothing: the work is on the server's ordered
/// queue, and a wake only says "drain it". Several wakes for one
/// drain are harmless — an empty drain is a no-op.
#[derive(Debug, Clone, Copy)]
pub(crate) struct IpcWake;

/// What executing one request produced.
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) struct Executed {
    pub(crate) reply: envelope::Reply,
    /// The command changed something on screen. The event loop then
    /// requests a redraw, exactly as a winit input handler does after
    /// a mutating event — `about_to_wait`'s continuation check alone
    /// does not schedule a present for a one-shot change.
    pub(crate) repaint: bool,
}

/// What one `IpcServer::drain` did that the event loop must act on.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct DrainOutcome {
    /// Some executed command changed pixels; request a redraw.
    pub(crate) repaint: bool,
    /// The drain budget ran out with messages still queued; keep the
    /// loop iterating rather than parking.
    pub(crate) backlog: bool,
}

/// Stand-in for the transport where it does not exist yet. `start`
/// is the only way to obtain one and always fails, so the other
/// methods are unreachable by construction — the uninhabited field
/// proves it to the compiler.
#[cfg(not(unix))]
pub(crate) struct IpcServer {
    never: std::convert::Infallible,
}

#[cfg(not(unix))]
impl IpcServer {
    pub(crate) fn start(
        _path: &std::path::Path,
        _wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<IpcServer, String> {
        Err(
            "the `--ipc` transport is not available on this platform yet: Windows needs the \
             named-pipe adapter, tracked in #169 (format/ipc.md \"Windows\")"
                .to_string(),
        )
    }

    pub(crate) fn drain(&mut self, _execute: impl FnMut(&envelope::IpcRequest) -> Executed) -> DrainOutcome {
        match self.never {}
    }

    pub(crate) fn publish_map(&mut self, _path: Option<&str>) {
        match self.never {}
    }

    pub(crate) fn shutdown(self) {
        match self.never {}
    }
}
