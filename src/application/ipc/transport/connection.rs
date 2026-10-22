// SPDX-License-Identifier: MPL-2.0

//! The three boundary-thread bodies — acceptor, reader, outbound —
//! and the capped line reader the reader is built on.
//!
//! Each body is a blocking loop of the form "wait on one socket call,
//! forward the result, repeat": the most auditable shape this
//! boundary can have (`work_plans/LLM_IPC.md` §D2, "Rejected: one
//! thread with a poll(2)/mio readiness loop"). None of them touches
//! app state; what crosses to the main thread is an [`Inbound`]
//! message, and what comes back is an [`Outbound`] frame.

use std::io::{self, BufRead, BufReader, ErrorKind, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{fence, AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::Arc;
use std::time::Duration;

use super::{
    Controller, Inbound, Link, Outbound, OUTBOUND_QUEUE_CAP, REQUEST_QUEUE_CAP, STALLED_WRITE_TIMEOUT,
};
use crate::application::ipc::envelope::{
    decode_line, Decoded, Event, Frame, Reply, INBOUND_LINE_CAP, OUTBOUND_FRAME_CAP,
};

/// The size past which the writer stops adding queued frames to one
/// write. Large enough that a burst of replies goes out in a handful of
/// syscalls; small enough that a batch is not a second unbounded
/// buffer beside the queue.
const WRITE_BATCH_BYTES: usize = 64 << 10;

/// Pause after a failed `accept()` before trying again, so a
/// persistent failure (descriptor exhaustion) cannot spin a core.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

/// The acceptor: install the first connection as controller, refuse
/// every other one while it stays, forever. Runs for the process
/// lifetime on `mandala-ipc-acceptor`.
pub(super) fn accept_loop(listener: UnixListener, link: Arc<Link>) {
    let mut generation = 0u64;
    loop {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(e) => {
                log::warn!("ipc: accept failed: {e}");
                std::thread::sleep(ACCEPT_BACKOFF);
                continue;
            }
        };
        if link.controller_live.load(Ordering::Acquire) {
            reject_busy(&stream);
            continue;
        }
        generation += 1;
        if let Err(e) = install_controller(stream, generation, &link) {
            log::warn!("ipc: could not set up controller connection {generation}: {e}");
        }
    }
}

/// Refuse a connection while a controller is attached: one
/// non-blocking write of `connection_rejected`, then close. A rejectee
/// that never reads cannot stall the acceptor — on a full buffer the
/// write simply fails and the connection is closed without the frame.
fn reject_busy(stream: &UnixStream) {
    let _ = stream.set_nonblocking(true);
    if let Some(bytes) = Frame::Event(Event::connection_rejected_busy()).encode(OUTBOUND_FRAME_CAP) {
        let mut writer = stream;
        let _ = writer.write(&bytes);
    }
}

/// Install `stream` as controller `generation`: greet it, announce it
/// to the main thread, then start its outbound and reader threads.
///
/// The order is load-bearing. `hello` is queued before anything else
/// can be, so it is the first frame on the wire. `Connected` is sent
/// before the reader exists, so it precedes every request on the
/// main thread's queue — a request whose controller the main thread
/// has not met would be dropped as stale.
fn install_controller(stream: UnixStream, generation: u64, link: &Arc<Link>) -> io::Result<()> {
    stream.set_write_timeout(Some(STALLED_WRITE_TIMEOUT))?;
    let read_half = stream.try_clone()?;
    let hangup = stream.try_clone()?;
    let (outbound_tx, outbound_rx) = mpsc::sync_channel(OUTBOUND_QUEUE_CAP);
    let (flushed_tx, flushed_rx) = mpsc::channel();
    let waiting = Arc::new(AtomicBool::new(false));
    // A fresh queue has room; a failure here would only mean a frame
    // the client never asked for went missing.
    let _ = outbound_tx.try_send(Outbound::Frame(Frame::Event(Event::hello(link.map().as_deref()))));

    let writer = Writer {
        stream,
        frames: outbound_rx,
        waiting: Arc::clone(&waiting),
        link: Arc::clone(link),
        _flushed: flushed_tx,
    };
    std::thread::Builder::new()
        .name(format!("mandala-ipc-outbound-{generation}"))
        .spawn(move || writer.run())?;

    link.controller_live.store(true, Ordering::Release);
    link.notify(Inbound::Connected(Controller {
        generation,
        outbound: outbound_tx.clone(),
        waiting,
        hangup,
        flushed: flushed_rx,
    }));

    let reader_link = Arc::clone(link);
    let spawned = std::thread::Builder::new()
        .name(format!("mandala-ipc-reader-{generation}"))
        .spawn(move || reader_loop(read_half, generation, outbound_tx, reader_link));
    if let Err(e) = spawned {
        // The main thread already met this controller; retire it the
        // way a disconnect would, so the next connection can be had.
        link.notify(Inbound::Gone { generation });
        link.controller_live.store(false, Ordering::Release);
        return Err(e);
    }
    Ok(())
}

/// The outbound writer for one controller and everything it owns.
struct Writer {
    stream: UnixStream,
    frames: Receiver<Outbound>,
    /// The main thread's "paused on a full queue" flag, shared with
    /// its `Controller`.
    waiting: Arc<AtomicBool>,
    link: Arc<Link>,
    /// Never sent on. Its drop as [`Writer::run`] returns is the
    /// signal: the main thread's receiver disconnects, which is how a
    /// graceful exit learns the `shutdown` event has been written.
    _flushed: Sender<()>,
}

impl Writer {
    /// Serialize each frame (cap check and `reply_too_large`
    /// substitution included) and write it, gathering every frame
    /// already queued into one write. Ends when every sender is
    /// gone — the reader has exited and the main thread has retired
    /// the controller — on a write error or a write stalled past
    /// `STALLED_WRITE_TIMEOUT`, or after an [`Outbound::Final`] frame;
    /// then closes the connection so a reader still blocked on it
    /// wakes up.
    fn run(self) {
        let Writer {
            stream,
            frames,
            waiting,
            link,
            _flushed,
        } = self;
        // If the main thread paused on a full queue, wake it — it
        // clears the flag first, so one pause costs one wake.
        let resume_main_thread = || {
            if waiting.swap(false, Ordering::SeqCst) {
                link.rouse();
            }
        };
        let mut writer = &stream;
        let mut batch = Vec::new();
        while let Ok(first) = frames.recv() {
            // Everything already queued goes out in one write: a burst
            // costs one syscall rather than one per frame, which keeps
            // the queue from filling while a pipelining client reads
            // normally. The batch stops growing at `WRITE_BATCH_BYTES`
            // so one write never holds an unbounded buffer.
            batch.clear();
            let mut last = false;
            let mut next = Some(first);
            while let Some(message) = next.take() {
                // Taking the frame made room in the queue. The fence
                // pairs with the one in `Controller::offer`.
                fence(Ordering::SeqCst);
                resume_main_thread();
                let frame = match message {
                    Outbound::Frame(frame) => frame,
                    Outbound::Final(frame) => {
                        last = true;
                        frame
                    }
                };
                if !frame.encode_into(&mut batch, OUTBOUND_FRAME_CAP) {
                    log::warn!("ipc: dropped an event that exceeded the outbound frame cap");
                }
                if last || batch.len() >= WRITE_BATCH_BYTES {
                    break;
                }
                next = frames.try_recv().ok();
            }
            if let Err(e) = writer.write_all(&batch) {
                log::debug!("ipc: write to controller failed, closing: {e}");
                break;
            }
            if last {
                break;
            }
        }
        let _ = stream.shutdown(Shutdown::Both);
        // A main thread paused on this connection must learn it is
        // over. The queue goes first, so the retry the wake triggers
        // finds it disconnected rather than still full — waking first
        // would let the main thread pause again on a queue nobody will
        // ever drain, with nobody left to wake it.
        drop(frames);
        resume_main_thread();
    }
}

/// The reader for one controller: lines in, requests out. Rejections
/// that need no app state — malformed frames, oversized lines, a full
/// request queue — are answered from here, straight into the
/// controller's outbound queue. When that queue is full the reader
/// waits for room, which stops it reading: backpressure lands on the
/// client that is not draining its replies, never on the app. On EOF
/// or a read error, announces the disconnect (after every request it
/// queued, by FIFO) and frees the acceptor to install the next
/// controller.
fn reader_loop(stream: UnixStream, generation: u64, outbound: SyncSender<Outbound>, link: Arc<Link>) {
    let mut reader = BufReader::new(&stream);
    let mut line = Vec::new();
    let mut index = 0u64;
    loop {
        let reply = match read_capped_line(&mut reader, &mut line, INBOUND_LINE_CAP) {
            Ok(LineRead::Eof) => break,
            Ok(LineRead::Oversized) => Some(Reply::oversized_line(index)),
            Ok(LineRead::Line) => match decode_line(&line, index, generation) {
                Decoded::Blank => None,
                Decoded::Rejected(reply) => Some(reply),
                Decoded::Request(request) => match link.submit(request) {
                    Ok(()) => None,
                    Err(refused) => Some(Reply::overloaded(refused.id, REQUEST_QUEUE_CAP)),
                },
            },
            Err(e) => {
                log::debug!("ipc: controller {generation} read failed: {e}");
                break;
            }
        };
        if let Some(reply) = reply {
            // Fails only once the outbound thread has exited — the
            // connection is over either way.
            if outbound.send(Outbound::Frame(Frame::Reply(reply))).is_err() {
                break;
            }
        }
        index += 1;
    }
    drop(outbound);
    link.notify(Inbound::Gone { generation });
    link.controller_live.store(false, Ordering::Release);
}

/// What [`read_capped_line`] found.
#[derive(Debug, PartialEq, Eq)]
enum LineRead {
    /// A line (without its newline) is in the buffer. A final line the
    /// peer did not terminate before closing counts as one.
    Line,
    /// A line longer than the cap was consumed through its newline and
    /// discarded; the buffer is empty.
    Oversized,
    /// The peer closed with nothing further to read.
    Eof,
}

/// Read one `\n`-terminated line into `line`, holding it to `cap`
/// bytes. An over-long line is still consumed in full — through its
/// newline — so the stream stays aligned on line boundaries, but its
/// bytes are not kept: memory stays bounded by `cap` whatever the peer
/// sends.
///
/// Cost: one copy of each kept byte out of the reader's buffer.
fn read_capped_line(reader: &mut impl BufRead, line: &mut Vec<u8>, cap: usize) -> io::Result<LineRead> {
    line.clear();
    let mut oversized = false;
    let mut read_any = false;
    loop {
        let available = match reader.fill_buf() {
            Ok(available) => available,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if available.is_empty() {
            return Ok(match (read_any, oversized) {
                (false, _) => LineRead::Eof,
                (true, true) => LineRead::Oversized,
                (true, false) => LineRead::Line,
            });
        }
        read_any = true;
        let newline = available.iter().position(|&b| b == b'\n');
        let chunk = &available[..newline.unwrap_or(available.len())];
        if !oversized {
            if line.len() + chunk.len() > cap {
                oversized = true;
                line.clear();
            } else {
                line.extend_from_slice(chunk);
            }
        }
        let consumed = newline.map_or(available.len(), |at| at + 1);
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(if oversized {
                LineRead::Oversized
            } else {
                LineRead::Line
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every [`read_capped_line`] result over `input`, with the line's
    /// bytes for `Line` results. A tiny `BufReader` capacity forces
    /// lines across buffer refills, which is where a framing bug hides.
    fn frames(input: &[u8], cap: usize, buffer: usize) -> Vec<(LineRead, Vec<u8>)> {
        let mut reader = BufReader::with_capacity(buffer, input);
        let mut line = Vec::new();
        let mut out = Vec::new();
        loop {
            let read = read_capped_line(&mut reader, &mut line, cap).expect("in-memory reads cannot fail");
            if read == LineRead::Eof {
                return out;
            }
            out.push((read, line.clone()));
        }
    }

    #[test]
    fn test_read_capped_line_splits_on_newlines_across_refills() {
        for buffer in [1, 2, 3, 64] {
            assert_eq!(
                frames(b"ab\n\ncde\n", 16, buffer),
                vec![
                    (LineRead::Line, b"ab".to_vec()),
                    (LineRead::Line, b"".to_vec()),
                    (LineRead::Line, b"cde".to_vec()),
                ],
                "buffer {buffer}"
            );
        }
    }

    #[test]
    fn test_read_capped_line_discards_an_oversized_line_and_resynchronizes() {
        for buffer in [1, 4, 64] {
            assert_eq!(
                frames(b"0123456789\nok\n", 4, buffer),
                vec![
                    (LineRead::Oversized, b"".to_vec()),
                    (LineRead::Line, b"ok".to_vec())
                ],
                "buffer {buffer}"
            );
        }
    }

    #[test]
    fn test_read_capped_line_cap_is_inclusive() {
        assert_eq!(frames(b"abcd\n", 4, 2), vec![(LineRead::Line, b"abcd".to_vec())]);
        assert_eq!(
            frames(b"abcde\n", 4, 2),
            vec![(LineRead::Oversized, b"".to_vec())]
        );
    }

    #[test]
    fn test_read_capped_line_keeps_an_unterminated_final_line() {
        assert_eq!(
            frames(b"ab\ncd", 16, 2),
            vec![(LineRead::Line, b"ab".to_vec()), (LineRead::Line, b"cd".to_vec())]
        );
        assert_eq!(frames(b"abcdef", 4, 2), vec![(LineRead::Oversized, b"".to_vec())]);
        assert!(frames(b"", 4, 2).is_empty());
    }
}
