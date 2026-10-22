// SPDX-License-Identifier: MPL-2.0

//! The Unix-socket transport: the boundary threads that own the
//! socket, and [`IpcServer`], the main thread's handle onto them.
//!
//! **Who owns what** (`work_plans/LLM_IPC.md` §D2). A persistent
//! *acceptor* always waits in `accept()`, so a second client learns it
//! is refused at once. Each controller connection gets a *reader*
//! (bytes → lines → [`IpcRequest`]s) and an *outbound* writer
//! (frames → bytes), spawned together and torn down together. None of
//! them sees `InitState`, the document or the renderer; they exchange
//! protocol values with the main thread over `std::sync::mpsc` and
//! wake it through the closure [`IpcServer::start`] is handed — a
//! winit `EventLoopProxy` send in the app.
//!
//! **One ordered queue in.** Requests *and* the controller lifecycle
//! notices (connected, gone) travel on the same FIFO, so the main
//! thread sees them in the order they happened. That is what makes
//! the generation stamp exact: a request is executed only while its
//! controller is the live one, a controller's requests are always
//! seen after its `Connected` and before its `Gone`, and a client
//! that sends a request and then closes its write half still gets
//! every reply (the classic `printf … | nc -U` round trip), because
//! its `Gone` cannot overtake its requests. The request cap is
//! counted separately so lifecycle notices are never refused.
//!
//! **Nothing blocks the main thread.** Delivery to a controller is a
//! `try_send` into its bounded outbound queue. A full queue does not
//! mean the client is gone — the main thread can execute a burst of
//! pipelined requests faster than the writer thread is scheduled to
//! drain them, even though the writer sends everything queued in one
//! write — so it *pauses* that controller instead: the reply that did not fit
//! is held, no further request is executed, and the outbound thread
//! wakes the loop as soon as it has made room. A client that has
//! genuinely stopped reading is detected where it can be, at the
//! socket: a write that cannot make progress for
//! `STALLED_WRITE_TIMEOUT` ends the connection (`format/ipc.md`
//! §Threading contract). The only bounded wait on the main thread is
//! at exit, after the event loop has stopped, for the `shutdown` event
//! to flush.

mod connection;
mod socket;

use std::path::Path;
use std::sync::atomic::{fence, AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use std::os::unix::net::UnixStream;

use super::envelope::{Event, Frame, IpcRequest};
use super::{DrainOutcome, Executed};

/// Unexecuted requests the main thread may have queued before a reader
/// answers further ones with `overloaded`.
const REQUEST_QUEUE_CAP: usize = 1024;

/// Frames a controller's outbound queue holds before the main thread
/// pauses executing that controller's requests until it drains.
const OUTBOUND_QUEUE_CAP: usize = 256;

/// How long one write may make no progress before the client is
/// declared dead and its connection dropped. A write blocks only once
/// the kernel's socket buffer is full of output the client has not
/// read, so this measures a client that has stopped reading, not one
/// that is merely thinking.
#[cfg(not(test))]
const STALLED_WRITE_TIMEOUT: Duration = Duration::from_secs(10);
/// Shortened under test so the dead-client path runs in seconds rather
/// than ten; the mechanism is the same socket option. Still long enough
/// that a test client descheduled on a loaded runner is not mistaken
/// for a dead one — the paused-reader test pauses for a small,
/// independent fraction of it.
#[cfg(test)]
const STALLED_WRITE_TIMEOUT: Duration = Duration::from_secs(4);

/// Inbound messages one drain handles at most. Bounds the time a
/// single drain can hold the main thread against a client that writes
/// as fast as commands execute; anything left over is reported as
/// backlog, and the event loop keeps iterating until it is gone.
const DRAIN_BUDGET: usize = REQUEST_QUEUE_CAP;

/// How long a graceful exit waits for the `shutdown` event to reach
/// the client. The event loop has already stopped; this only bounds
/// how long a stalled client can delay the process ending.
#[cfg(not(test))]
const SHUTDOWN_FLUSH: Duration = Duration::from_millis(250);
/// Longer under test, so a test client descheduled on a loaded runner
/// is not mistaken for one that stopped reading.
#[cfg(test)]
const SHUTDOWN_FLUSH: Duration = Duration::from_secs(4);

/// How often a graceful exit retries a full queue before its deadline.
const SHUTDOWN_RETRY: Duration = Duration::from_millis(2);

/// A message from the boundary threads to the main thread.
enum Inbound {
    /// A controller was installed. Always precedes its requests.
    Connected(Controller),
    Request(IpcRequest),
    /// The controller of this generation disconnected. Always follows
    /// its requests.
    Gone {
        generation: u64,
    },
}

/// A frame bound for one controller's outbound thread.
enum Outbound {
    Frame(Frame),
    /// Write this frame, then close the connection.
    Final(Frame),
}

/// The main thread's view of the live controller connection.
struct Controller {
    generation: u64,
    outbound: SyncSender<Outbound>,
    /// Set by the main thread when it paused on a full queue; the
    /// outbound thread clears it and wakes the loop once it has taken
    /// a frame off the queue.
    waiting: Arc<AtomicBool>,
    /// A handle on the connection used only to shut it down — the
    /// reader and the outbound thread unblock with an error and exit.
    hangup: UnixStream,
    /// Disconnects when the outbound thread exits; waited on at exit
    /// so the `shutdown` event can flush.
    flushed: Receiver<()>,
}

/// What [`Controller::offer`] did with a frame.
enum Offer {
    Queued,
    /// The queue is full; the frame is handed back to be retried when
    /// the outbound thread wakes the loop.
    Full(Outbound),
    /// The outbound thread is gone: the connection is over.
    Closed,
}

impl Controller {
    /// Queue `message` for delivery without blocking.
    ///
    /// On a full queue the `waiting` flag is raised and the send tried
    /// once more, with a fence between: either the retry sees the room
    /// the outbound thread just made, or the outbound thread — which
    /// fences between taking a frame and reading the flag — sees the
    /// flag and wakes the loop. A wake cannot be lost between the two.
    fn offer(&self, message: Outbound) -> Offer {
        let message = match self.outbound.try_send(message) {
            Ok(()) => return Offer::Queued,
            Err(TrySendError::Disconnected(_)) => return Offer::Closed,
            Err(TrySendError::Full(message)) => message,
        };
        self.waiting.store(true, Ordering::SeqCst);
        fence(Ordering::SeqCst);
        match self.outbound.try_send(message) {
            Ok(()) => Offer::Queued,
            Err(TrySendError::Disconnected(_)) => Offer::Closed,
            Err(TrySendError::Full(message)) => Offer::Full(message),
        }
    }
}

/// State the boundary threads share with the main thread. Only
/// protocol values and counters — never app state.
struct Link {
    inbound: Sender<Inbound>,
    /// Set by the boundary side when it wakes the loop, cleared by the
    /// main thread before it drains, so a burst of messages costs one
    /// wake rather than one per message.
    wake_pending: AtomicBool,
    wake: Box<dyn Fn() + Send + Sync>,
    /// Requests queued and not yet drained — the `overloaded` count.
    pending_requests: AtomicUsize,
    /// Whether a controller is connected; read by the acceptor to
    /// decide between installing and refusing a connection.
    controller_live: AtomicBool,
    /// The document path `hello` reports, published by the main
    /// thread. The mutex is held only to swap or clone an `Arc`, never
    /// across I/O, so neither side can stall the other on it.
    map: Mutex<Option<Arc<str>>>,
}

impl Link {
    /// Hand `message` to the main thread and wake it. A send can only
    /// fail once the server is gone, in which case there is nobody
    /// left to tell.
    fn notify(&self, message: Inbound) {
        if self.inbound.send(message).is_ok() {
            self.rouse();
        }
    }

    /// Wake the main thread unless it is already due to drain.
    fn rouse(&self) {
        if !self.wake_pending.swap(true, Ordering::AcqRel) {
            (self.wake)();
        }
    }

    /// Queue `request` unless [`REQUEST_QUEUE_CAP`] requests already
    /// wait, in which case it is handed back for an `overloaded` reply.
    fn submit(&self, request: IpcRequest) -> Result<(), IpcRequest> {
        if self.pending_requests.fetch_add(1, Ordering::AcqRel) >= REQUEST_QUEUE_CAP {
            self.pending_requests.fetch_sub(1, Ordering::AcqRel);
            return Err(request);
        }
        self.notify(Inbound::Request(request));
        Ok(())
    }

    fn map(&self) -> Option<Arc<str>> {
        self.map
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// The main thread's handle on a running IPC listener. Exists only
/// when `--ipc` was passed; without it, no socket, no thread and no
/// code path of this module runs.
pub(crate) struct IpcServer {
    link: Arc<Link>,
    inbound: Receiver<Inbound>,
    controller: Option<Controller>,
    /// A reply the live controller's full queue could not take. While
    /// it is held, that controller is paused: nothing further is
    /// executed until it is delivered.
    stalled: Option<Outbound>,
    /// The path last published into [`Link::map`], so publishing an
    /// unchanged path costs a comparison and no lock.
    published_map: Option<Arc<str>>,
    socket: socket::BoundSocket,
}

impl IpcServer {
    /// Bind the socket at `path` and start the acceptor. `wake` is
    /// called from boundary threads whenever the main thread has
    /// messages to drain; it must be cheap and must not block.
    ///
    /// Every failure — an untrusted parent directory, a path that is
    /// not a stale socket of ours, a live listener, a bind error — is
    /// returned as a message naming the path, for the caller's startup
    /// `expect`.
    pub(crate) fn start(path: &Path, wake: impl Fn() + Send + Sync + 'static) -> Result<IpcServer, String> {
        let (listener, socket) = socket::bind(path)?;
        let (inbound_tx, inbound) = mpsc::channel();
        let link = Arc::new(Link {
            inbound: inbound_tx,
            wake_pending: AtomicBool::new(false),
            wake: Box::new(wake),
            pending_requests: AtomicUsize::new(0),
            controller_live: AtomicBool::new(false),
            map: Mutex::new(None),
        });
        let acceptor_link = Arc::clone(&link);
        std::thread::Builder::new()
            .name("mandala-ipc-acceptor".into())
            .spawn(move || connection::accept_loop(listener, acceptor_link))
            .map_err(|e| format!("cannot start the IPC acceptor thread: {e}"))?;
        log::info!("ipc: listening on {}", socket.path().display());
        Ok(IpcServer {
            link,
            inbound,
            controller: None,
            stalled: None,
            published_map: None,
            socket,
        })
    }

    /// Run every queued message, up to [`DRAIN_BUDGET`], on the
    /// calling (main) thread. `execute` runs one current-generation
    /// request and produces its reply; requests from a controller that
    /// has since left are dropped unexecuted, and their replies would
    /// have had nowhere to go.
    ///
    /// A paused controller (see the module doc) is resumed first; if
    /// its queue is still full the drain stops there and reports no
    /// backlog — the outbound thread's wake is what brings the loop
    /// back, so a paused controller costs no polling.
    pub(crate) fn drain(&mut self, mut execute: impl FnMut(&IpcRequest) -> Executed) -> DrainOutcome {
        // Cleared *before* draining: a message enqueued after this
        // point re-arms the wake, and one enqueued before it is picked
        // up by the loop below.
        self.link.wake_pending.store(false, Ordering::Release);
        let mut outcome = DrainOutcome::default();
        if let Some(message) = self.stalled.take() {
            if !self.offer_to_controller(message) {
                return outcome;
            }
        }
        for _ in 0..DRAIN_BUDGET {
            match self.inbound.try_recv() {
                Ok(Inbound::Connected(controller)) => self.controller = Some(controller),
                Ok(Inbound::Request(request)) => {
                    self.link.pending_requests.fetch_sub(1, Ordering::AcqRel);
                    if !self
                        .controller
                        .as_ref()
                        .is_some_and(|c| c.generation == request.generation)
                    {
                        continue;
                    }
                    let executed = execute(&request);
                    outcome.repaint |= executed.repaint;
                    if !self.offer_to_controller(Outbound::Frame(Frame::Reply(executed.reply))) {
                        return outcome;
                    }
                }
                Ok(Inbound::Gone { generation }) => self.retire(generation),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return outcome,
            }
        }
        outcome.backlog = true;
        outcome
    }

    /// Offer `message` to the live controller. `false` means the
    /// controller is paused on a full queue and the message is held in
    /// [`Self::stalled`]; a closed connection retires the controller,
    /// and the drain carries on.
    fn offer_to_controller(&mut self, message: Outbound) -> bool {
        let Some(controller) = self.controller.as_ref() else { return true };
        match controller.offer(message) {
            Offer::Queued => true,
            Offer::Full(message) => {
                self.stalled = Some(message);
                false
            }
            Offer::Closed => {
                self.controller = None;
                true
            }
        }
    }

    /// The controller of `generation` disconnected; forget it if it is
    /// still the live one, along with any reply it never collected.
    fn retire(&mut self, generation: u64) {
        if self
            .controller
            .as_ref()
            .is_some_and(|c| c.generation == generation)
        {
            self.controller = None;
            self.stalled = None;
        }
    }

    /// Publish the document path `hello` reports to the next client.
    /// Called by the main thread after it handles events; a no-op
    /// unless the path changed since the last call (a load, a `new`,
    /// a save-as).
    pub(crate) fn publish_map(&mut self, path: Option<&str>) {
        if self.published_map.as_deref() == path {
            return;
        }
        let next: Option<Arc<str>> = path.map(Arc::from);
        *self
            .link
            .map
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = next.clone();
        self.published_map = next;
    }

    /// Graceful exit: tell the controller, if any, that the app is
    /// going away, give that frame until [`SHUTDOWN_FLUSH`] to reach
    /// it, close the connection and remove the socket file. Requests
    /// still queued are not executed — the app is exiting, and the
    /// `shutdown` event is the answer they get.
    ///
    /// A controller paused on a full queue is the one case that needs
    /// care: its held reply is owed first, and neither it nor the
    /// `shutdown` frame fits until the writer makes room. Both are
    /// offered against the same deadline, so a client reading normally
    /// mid-burst still learns the app exited rather than seeing a bare
    /// EOF, and a client that stopped reading costs the exit at most
    /// [`SHUTDOWN_FLUSH`].
    pub(crate) fn shutdown(mut self) {
        while let Ok(message) = self.inbound.try_recv() {
            match message {
                Inbound::Connected(controller) => self.controller = Some(controller),
                Inbound::Gone { generation } => self.retire(generation),
                Inbound::Request(_) => {}
            }
        }
        if let Some(controller) = self.controller.take() {
            let deadline = Instant::now() + SHUTDOWN_FLUSH;
            let Controller {
                outbound,
                hangup,
                flushed,
                ..
            } = controller;
            let held_delivered = self
                .stalled
                .take()
                .is_none_or(|held| send_by(&outbound, held, deadline));
            let goodbye = Outbound::Final(Frame::Event(Event::shutdown()));
            if held_delivered && send_by(&outbound, goodbye, deadline) {
                drop(outbound);
                let _ = flushed.recv_timeout(deadline.saturating_duration_since(Instant::now()));
            }
            let _ = hangup.shutdown(std::net::Shutdown::Both);
        }
        self.socket.unlink();
    }
}

/// Queue `message`, retrying while the queue is full, until
/// `deadline`. Only for the exit path, after the event loop has
/// stopped — anywhere else the main thread never waits on a client.
fn send_by(outbound: &SyncSender<Outbound>, mut message: Outbound, deadline: Instant) -> bool {
    loop {
        match outbound.try_send(message) {
            Ok(()) => return true,
            Err(TrySendError::Disconnected(_)) => return false,
            Err(TrySendError::Full(back)) => {
                if Instant::now() >= deadline {
                    return false;
                }
                message = back;
                std::thread::sleep(SHUTDOWN_RETRY);
            }
        }
    }
}

#[cfg(test)]
mod tests;
