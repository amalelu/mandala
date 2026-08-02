# LLM IPC — architecture design (IPC-01)

The keystone document for EPIC #60: an LLM-friendly IPC interface so
an agent can drive, inspect, screenshot, and record the running app
— "give Claude a better feedback loop" (README). This file pins the
decisions every downstream issue builds against and carries the
rationale and the rejected alternatives; the wire-level contract
those decisions produce is [`format/ipc.md`](../format/ipc.md).

**If you are implementing IPC-02 … IPC-16**: your issue body plus
`format/ipc.md` plus this file are intended to be sufficient. Read
in this order: your issue → `format/ipc.md` (the sections your
issue owns) → the decision sections here that name your issue →
CONCEPTS §5 "Action dispatch" → CODE_CONVENTIONS §3/§4/§7/§9.

**If you are changing a decision**: decisions here were pinned as a
set — transport, threading, envelope, and trust interlock. Change
the decision section here, ripple `format/ipc.md` (bump `protocol`
if breaking), and check the [amendment ledger](#amendment-ledger)
for the convention text that moves with it.

## What already exists (the seams this design builds on)

The codebase was shaped for this trajectory; IPC adds **no parallel
control path** (CODE_CONVENTIONS §2). The seams, verified at design
time:

- **`dispatch_action`** — the single funnel: "Mouse, keyboard, the
  future macro runtime, and any plugin host all reach the same
  arms" (CONCEPTS §5). `Action` is serde-JSON with a compiled-in
  classification taxonomy (`is_destructive` / `context` /
  `wasm_compatibility` via `mandala_derive::ActionClassify`) that
  new variants cannot dodge.
- **`dispatch_macro`** over the cross-platform `MacroDispatchTarget`
  trait (`src/application/app/dispatch/macro_core.rs`) — the
  privilege-gated step loop whose steps already fan out to
  `dispatch_action` / custom mutations / `execute_console_line`.
- **`execute_console_line`**
  (`src/application/app/console_input/exec.rs`) — a string-in /
  void-out interpreter over ~20 verbs, callable with the console UI
  closed. It runs each verb's typed `ExecResult` internally and
  folds it into the console scrollback rather than returning it — so
  the `act.console` command extracts that result (see D7 and
  `format/ipc.md` §act); the parse → execute → drain-effects path is
  the reusable seam.
- **`SourceTier { App | User | Map | Inline }`** — the
  loader-pinned, fail-closed privilege model (`format/macros.md`).
- **The winit event loop** (`src/application/app/run_native.rs`) —
  `NativeApp: ApplicationHandler`, `about_to_wait` drains and then
  parks in `ControlFlow::Wait` when `needs_continuation()` is
  false. The loop already has an unused wake seam: winit's
  `EventLoopProxy` user events.
- **`FreezeWatchdog`**
  (`src/application/app/freeze_watchdog.rs`) — the sanctioned
  "background thread that never touches app state" precedent §3
  already carries.
- **Pure, GPU-free introspection surfaces** — hit-testing and scene
  building are plain functions; the model layer is fully serde;
  `tick_animations` takes its clock as a parameter with a single
  production call site (the virtualization seam IPC-09 uses).
- **Dormant intent markers** — `Options.should_exit` ("smoke-tests /
  CI captures", never read; IPC-10 territory) and the
  `EventSubscriber` seam ("reserved for the script API", reshaped by
  #43; deliberately *not* consumed by this epic's app-funnel events).

## Non-goals (pinned so they stay out)

Inherited from EPIC #60 and binding for every sub-issue: no remote /
network control, no multi-user sessions, no plugin system (IPC
*consumes* funnels, it does not define a plugin API), no maptool
static rendering, no WASM console parity work, no CI wiring. If an
implementation issue seems to need one of these, the design is being
misread — come back here.

---

## D1 — Transport: NDJSON over a local socket, opt-in by flag

**Decision.** Newline-delimited JSON over a Unix domain socket
(`SOCK_STREAM`) at a user-owned path, enabled per launch by
`--ipc <path>` (mirroring `--keybinds`). Windows: the same byte
protocol over a named pipe restricted to the current user; IPC-02
ships the Unix adapter and `--ipc` fails fast at startup on Windows
until the pipe adapter lands. Single controller at a time; the
listener outlives client connections; no session state survives a
disconnect. Full wire detail: `format/ipc.md` §Transport.

**Why a socket and not stdio.** stdio dies on the epic's central
workflow — *attach after launch, user watches while agent drives*.
The app is launched standalone (`run.sh`, a desktop session, an
already-running instance the user wants help in); a stdio protocol
would exist only for app instances an agent itself spawned, would
allow exactly one controller ever, and would leave protocol framing
one stray `println!` away from corruption on a stream the app has
never treated as an API surface. A socket also gives
reconnect-after-drop for free, which stdio structurally cannot.

**Why not localhost TCP.** No filesystem permissions — on a
multi-user machine any local user could connect; ports collide;
desktop firewalls throw consent dialogs. The socket path *is* the
access-control story (same-user-only, `0600`), and that story is
load-bearing for D4.

**Why NDJSON and not JSON-RPC / length-prefixed frames.** The
consumers are LLM agents and shell tooling; a protocol you can debug
with `socat` and parse with `jq` beats one that needs a codec.
JSON-RPC 2.0's additions (nested envelope, batching, notification
semantics) duplicate what the D3 envelope already provides, worse
(its `error.code` is a number; agents want names). Length-prefixed
binary framing earns its complexity only for large binary payloads —
which D3 deliberately routes to files instead.

**Connection lifecycle.** Pinned in `format/ipc.md`: greeting
(`hello` event) on accept; `connection_rejected` + close for a
second concurrent client; listener re-accepts after disconnect;
stale-socket self-healing at bind; socket unlinked on graceful
exit. Single-controller is a *simplification with a named escape
hatch*: the protocol has no per-connection semantics beyond
subscriptions, so if a real multi-client consumer ever appears
(§7: it must be named, not hypothetical), the change is confined
to the transport module and D2's queues — the envelope does not
change.

## D2 — §3 integration: boundary threads, a queue boundary, and a proxy wake

**The problem.** CODE_CONVENTIONS §3: single-threaded event loop, no
channels, no worker threads in interactive paths. The loop parks in
`ControlFlow::Wait` when idle (`NativeApp::about_to_wait`,
`src/application/app/run_native.rs`), so any poll-per-frame IPC
design starves while the app idles — and idle is precisely when an
agent drives the app. Something must block on the socket, and it
cannot be the main thread.

**Decision.** When `--ipc` is active, dedicated named boundary
threads own the socket. The acceptor is persistent; a reader **and**
an outbound writer are spawned per controller connection and both
exit when that controller disconnects (per-controller outbound so a
departed controller's queued frames are dropped by construction —
see the generation stamp below). None touches app state.

```
            clients (agent / mandalactl)
                      │ unix socket, NDJSON
   ┌──────────────────┴───────────────────────────────┐
   │ mandala-ipc-acceptor  (persistent)                │
   │  loop { accept();                                 │
   │    controller busy? → nonblocking write           │
   │                        connection_rejected, close │
   │    else → gen+=1, install controller, spawn       │
   │           reader+outbound for this gen }          │
   └──────────────────┬────────────────────────────────┘
                      │ hands the stream to
                      ▼
   ┌───────────────────────────────┐   ┌───────────────────────────┐
   │ mandala-ipc-reader            │   │ mandala-ipc-outbound      │
   │  (per controller, gen G)      │   │  (per controller, gen G)  │
   │  blocking read → 1 MiB cap →  │   │  recv value → serialize   │
   │  parse → enqueue IpcRequest   │   │  (8 MiB cap / too_large)  │
   │  {gen:G} → send_event         │   │  → blocking write         │
   └───────┬───────────────────────┘   └──────────▲────────────────┘
           │ mpsc request queue (cap 1024)         │ mpsc outbound queue
           ▼                                       │ (cap 256)
   ┌───────────────────────────────────────────────┴──┐
   │ main thread — winit event loop                   │
   │  user_event(IpcWake): drain request queue,       │
   │  execute each command against InitState (same    │
   │  access as any input handler), push replies,     │
   │  request_redraw if any command changed pixels    │
   └──────────────────────────────────────────────────┘
```

- **The acceptor thread** always waits on `accept()`, independent of
  controller state — this is what makes `connection_rejected`
  *immediate*. A single thread that both accepted and read the
  controller could not honor the single-controller promise: while
  blocked in the controller's `read()`, nothing calls `accept()`, so
  a second client would sit in the OS backlog until the controller
  left instead of being told it's busy. The acceptor rejects a
  second connection with one **non-blocking** write + close (no app
  state, no queue; on `EWOULDBLOCK` it just closes — a rejectee that
  never reads must not stall the acceptor and re-starve the real
  controller, which is the whole reason accept is its own thread) and
  returns to accepting; the first connection it installs as
  controller — bumping a monotonic **generation** counter G — and
  spawns a reader + outbound pair tagged G.
- **The reader thread** (one per controller, gen G) owns that
  connection's read half: reads lines, enforces the 1 MiB cap,
  parses the envelope, and either enqueues a well-formed `IpcRequest`
  stamped `{gen:G}` and wakes the loop via
  `EventLoopProxy::send_event(IpcWake)`, or pushes a pre-formed
  `parse_error` reply straight onto its outbound queue (parse errors
  never need app state, so they never cross into the loop). On
  EOF/error it sends a `send_event(IpcGone{gen:G})` so the main
  thread cancels this generation's pending `clock.wait` and resets
  its subscriptions, then it and its outbound peer exit; the
  acceptor's next accept installs a fresh controller at G+1.
- **The outbound thread** (per controller, gen G) owns the write
  half: receives reply and event *values* (from the main thread;
  plus the reader's pre-formed parse-error replies), serializes them
  — the 8 MiB cap check and the `reply_too_large` substitution live
  here, off the interactive path — and performs every blocking write.
  Its queue dies with the connection, so a reply the main thread
  computed for a departed controller is discarded rather than
  delivered to the successor. Backpressure is **asymmetric**: 256
  undelivered *replies* → teardown, but unsolicited *events* are
  coalesced/dropped-oldest-by-revision rather than triggering
  teardown (a thinking agent that pauses while events flood must keep
  its session). **The app is never backpressured by a slow client**,
  and the `FreezeWatchdog` never sees IPC I/O.
- **The generation stamp** is what keeps a single persistent-looking
  reply/event stream honest across reconnects: every `IpcRequest`,
  reply, event, and pending wait carries the gen it belongs to; the
  main thread only routes to the current gen's outbound and only
  honors an `IpcGone{gen}` for the live gen. This is the mechanism
  behind `format/ipc.md`'s "no session state survives a disconnect."
- **The main thread** is the only place commands execute. IPC-02
  changes the event loop to
  `EventLoop::<IpcUserEvent>::with_user_event()` (the user-event type
  is a small enum — `IpcWake` and `IpcGone{gen}`) and implements
  `ApplicationHandler::user_event` on `NativeApp`: tick the
  watchdog (`unparked`, like every other handler), drain the
  request queue to exhaustion and treat an empty-queue wake as a
  no-op (winit delivers one user event per `send_event`, but an
  earlier wake's drain may already have consumed requests enqueued
  just before a later wake was sent), execute each current-gen
  command via the registry against `InitState` (the same access
  `input_context()` gives an input handler), enqueue replies, and —
  the seam that keeps the watched window and screenshots truthful —
  **request a redraw whenever a command changed pixels**, then fall
  through to the normal `about_to_wait` drain. `hello`'s `map` field
  is live app state, so the main thread publishes the current file
  path into an `Arc`-swapped snapshot on document load and the
  acceptor reads *that* (never `MindMapDocument`) when greeting — the
  state-free-boundary invariant holds. Note the winit event *type*
  changes (`Event<()>` → `Event<IpcUserEvent>`), but
  `handle_event` — which only ever receives `WindowEvent`s — needs
  no signature change; user events land in the separate `user_event`
  method. (The `redraw_after → request_redraw()` pattern the drain
  reuses is already in `handle_event`.)

**Pixels-affecting commands must request a redraw (and bump the
render revision).** The existing winit handlers set
`redraw_after = true` → `Window::request_redraw()` after any
mutating input, because `about_to_wait`'s continuation check alone
does *not* schedule a present for a one-shot change that leaves
`needs_continuation()` false (a selection, a style write, a single
zoom). The `user_event` drain mirrors that exactly: each command
reports whether it touched pixels (document mutation *or* view /
selection change), and the drain requests a redraw and advances the
render revision (IPC-14's counter — a *render*-affecting revision,
not merely a document one) if any did. Without this, an
`act.action` zoom would update buffers that never re-render, and a
following `clock.wait until=settled` + screenshot would capture the
stale frame — the exact race `settled` exists to prevent. This is
why `settled` (D7) keys on the *rendered* render revision (render
pass executed, not swapchain-presented — so headless/occluded
capture still terminates), not the document revision.

**Why this is a §3 amendment and not a violation.** The covenant's
purpose is that *app state has exactly one thread*: no interleaving,
no lock discipline, no cross-thread mutation. That holds unchanged —
the I/O threads see only protocol value types (`IpcRequest`,
serialized frames), never `InitState`, `MindMapDocument`, or
`Renderer`. The queues are a process-boundary adapter, not
inter-component plumbing; the `FreezeWatchdog` ("thread that never
touches app state") is the sanctioned shape this generalizes. The
amendment is recorded in CODE_CONVENTIONS §3 in this PR — same
commit as the design, exactly as §3 demands for project-scale
decisions — and scopes the exception narrowly: IPC boundary only,
`std::sync::mpsc` only (crossbeam is on its way *out* per #43/P2-40),
protocol values only, and never a blocking IPC operation on the
main thread.

**Rejected: `ControlFlow::WaitUntil` polling.** A poll interval is
a fork: short enough for snappy IPC (≤ 5 ms) and the app burns CPU
waking ~200×/s forever once IPC was ever active — §4's
mobile-budget ethos says idle apps sleep; long enough to be cheap
(≥ 100 ms) and every agent command eats up to the interval in
latency, hundreds of times per session. Polling also still needs
either a non-blocking accept/read on the main thread (I/O in the
interactive path — a worse §3 breach than threads that touch
nothing) or the boundary reader thread anyway. The proxy wake gives
zero idle cost *and* sub-millisecond dispatch latency; winit built
`EventLoopProxy` for precisely this. (`WaitUntil` still earns its
keep for the two *timer* cases — a pending `clock.wait` deadline and
an active recording's frame cadence — where the loop must wake at a
known instant, not on socket readiness.)

**Rejected: one thread with a poll(2)/mio readiness loop.** Folding
accept + read + write onto one multiplexed thread is fewer threads
on paper, but it trades dumb blocking loops for a hand-rolled
readiness state machine with partial-write buffering — more code,
more failure modes, identical covenant surface. Threads whose
entire body is "blocking call, forward, repeat" are the most
auditable shape this boundary can have; the single-controller
promise (D-transport) is exactly why accept and read are *separate*
blocking loops rather than one.

**Watchdog interplay (pinned invariants).** `user_event` ticks
`unparked()` on entry like every handler, so a command that wedges
the loop still gets the watchdog's diagnostic abort — IPC gains no
exemption from the 10-second contract. Command execution is
synchronous and bounded; anything that must outlive a heartbeat
(waits, recordings) is a *deferred reply* (D7), never a blocking
loop inside a command handler.

## D3 — Envelope: flat requests, correlated replies, structured errors

**Decision.** Pinned byte-for-byte in `format/ipc.md` §Envelope:
requests `{"id", "cmd", <flat params>}`; replies `{"id", "ok",
"result" | "error", "warnings"?}` with a reserved `revision` key
(activates with IPC-14); events `{"event", "data"}`; a `hello`
greeting carrying `protocol: 1`; stable snake_case error codes;
1 MiB inbound / 8 MiB outbound caps; pixels always to files,
oversized dumps steered to `to_file`.

**Error posture (§9, hostile-input precedent PR #59).** Errors are
structured JSON values — `{"code", "message", "data"?}` — never
Rust error types (no `anyhow`/`thiserror`/custom enums anywhere in
the implementation) and never panics: a malformed, oversized, or
hostile frame degrades to a `parse_error`/`invalid_params` reply
and the app keeps running, exactly as the loader rejects a cyclic
document without dying. Codes are API (agents branch on them);
messages are prose and free to improve.

**Correlation over ordering.** Replies may arrive out of request
order because deferred-reply commands (D7's `clock.wait`) resolve
at heartbeat boundaries while later commands keep executing.
Pinning id-correlation now — rather than FIFO-with-blocking-waits —
is what keeps a pending `wait` from starving a `capture.record_stop`
and keeps the main thread free of any "wait for X inside a command"
loop, which D2 forbids.

**Why flat params.** The primary author of these frames is a
language model. `{"id":1,"cmd":"clock.step","ms":100}` has one
nesting level and reads as a sentence; a `params` wrapper adds a
brace every message to protect against a collision that reserving
two words (`id`, `cmd`) prevents outright. Strict unknown-param
rejection converts agent typos into immediate `invalid_params`
with the offender named, instead of silently ignored options and a
confusing screenshot three commands later.

**Version stance (§10).** One live protocol integer, bumped on any
breaking change, no negotiation, no deprecation shims pre-V1.
Clients tolerate unknown keys (additive growth is free);
`mandalactl` refuses a protocol it doesn't know unless forced. The
SSOT chain — this doc pins semantics, `format/ipc.md` pins bytes,
`describe` mirrors it at runtime, tests keep the mirror honest — is
what lets ~10 issues build in parallel without drift.

## D4 — Trust tier: IPC executes as `User`; no new tier

**Decision.** IPC commands execute at the existing
`SourceTier::User` posture. No `SourceTier::Ipc` variant is
added. `act.macro` runs macros under their own loader-pinned tier —
IPC initiates, it never escalates. `format/macros.md`'s
SOURCE-OF-TRUTH tier list gains a pointer recording this mapping
(done in this PR), so a future tier addition or reorder must
re-evaluate it consciously.

**Rationale — "the user owns the flag."** The trust question is:
*who chose to expose this surface?* For `keybinds.json` and
`macros.json` the answer is "the user, by editing a file they own,"
and the tier is `User`. For IPC the answer is "the user, by passing
`--ipc` at launch" — the same authority exercising the same kind of
consent, enforced by the same mechanism (filesystem permissions on
a user-owned path, D1). The peer process on that socket runs as the
same OS user and could already write `macros.json`, edit
`keybinds.json`, or attach a debugger; pretending in-app tiers can
contain it would be security theater. Meanwhile the epic's charter
is human parity — an agent that cannot save, delete, cut, paste, or
run console verbs cannot give feedback on the app, so any tier
*below* User contradicts the feature's reason to exist.

**Why not a distinct `Ipc` tier anyway (auditability,
future-proofing)?** Everything a tier would buy is available
cheaper and safer:

- *Attribution* — a logging/eventing concern: IPC dispatch sites
  tag their origin in diagnostics and (post IPC-14) events carry
  it. No privilege machinery needed.
- *Future restriction* — would be a product decision to make IPC
  weaker than the human it serves; if that day comes it is a
  protocol-level redesign (this document), not a dormant enum
  variant waiting to confuse someone.
- Against those non-benefits: a fifth variant ripples through every
  entry in the SOURCE-OF-TRUTH list (`SourceTier` order, two
  loader call-site groups, the loader helpers) plus the registry's
  hand-written tier walk beside that list, forces an answer to
  "does `Ipc` allow
  ConsoleLine?" whose only sensible value duplicates `User`, and
  permanently widens the threat-model reference everyone must read.
  §7 over-engineers for *named* trajectories; "maybe IPC should be
  weaker someday" is not one.

**The composition rule (the part that must not drift).** Privilege
is attached to *where instructions were authored*, not *who pulled
the trigger*. A Map/Inline-tier macro fired over IPC is still
Map/Inline-tier — `dispatch_macro`'s fail-closed gates run
untouched. IPC-03's bridge therefore calls the existing dispatchers
and adds **no** gate-bypassing entry points; if a future command
needs a path around a gate, that is a redesign of this section, not
an implementation detail.

## D5 — Command families, registry shape, self-description

**Decision.** Eight families — bootstrap `describe`/`ping`, `act`,
`query`, `scene`, `input`, `capture`, `clock`, `events` — each owned
by exactly one issue and living in exactly one module under
`src/application/ipc/commands/`, assembled by one registry table
(one registrar line per family). The full family → issue → command
map, and the self-description (`describe`) contract, are pinned in
`format/ipc.md` §Command registry / §Command families.

**Why this shape.** It is the console's proven pattern (`COMMANDS`
slice, one module per verb) scaled one level up, and it is §6
modular design serving a scheduling need: Wave 2 lands IPC-04/05/06/
14 as parallel PRs, so the registry must make their diffs disjoint
by construction — new module + one line, no shared files edited.
Command names are namespaced `family.verb` so the registry stays
flat (no routing tree), `describe` output groups naturally, and a
family's docs section is greppable from any frame.

**Self-description as a compile-shape.** A registry entry cannot be
constructed without name, summary, and param/result descriptors —
the same forcing-function philosophy as `ActionClassify` (a variant
cannot exist unclassified). `describe` is rendered *from the
registry*, so the discovery surface cannot drift from the dispatch
surface; drift against `format/ipc.md` is caught by unit tests
asserting the registry matches the documented command list (and
IPC-16 audits at epic close).

**Module layout** (IPC-02 creates the spine, IPC-03 the registry):

```
src/application/ipc/            #[cfg(not(target_arch = "wasm32"))]
├── mod.rs         module doc: the D2 boundary contract
├── transport.rs   acceptor + per-controller reader + outbound threads (IPC-02)
├── envelope.rs    IpcRequest / reply / event value types (IPC-02)
├── registry.rs    CommandSpec, the one assembly table, describe (IPC-03)
└── commands/
    ├── mod.rs     family registrar index (IPC-03)
    ├── meta.rs    describe, ping (IPC-03)
    ├── act.rs     … (IPC-03)
    ├── query.rs   … (IPC-04)
    ├── scene.rs   … (IPC-05)
    ├── input.rs   … (IPC-06)
    ├── capture.rs … (IPC-07, extended by IPC-08)
    ├── clock.rs   … (IPC-09)
    └── events.rs  … (IPC-14)
```

Known micro-overlaps (from EPIC #60's parallel map, unchanged):
the camera read accessor (IPC-04 ∩ IPC-05 — first to land adds it),
the `document/mod.rs` revision field (IPC-14; IPC-09 consumes).

**Crossing the `app` boundary.** `crate::application::ipc` is a
*sibling* of `crate::application::app`, but every funnel a command
reaches — `dispatch_action`, `dispatch_macro`,
`execute_console_line`, `InitState` / `input_context()` — is
`pub(in crate::application::app)`, so a command handler cannot call
them directly. IPC-03 therefore reaches them through an app-side
**execution-context trait** (holding `&mut InitState`, implemented
inside `app`, passed into the `ipc` handlers) — the exact
indirection `dispatch_macro` already uses via `MacroDispatchTarget`
(`app/dispatch/macro_core.rs`), which is why native and WASM share
one macro loop. This keeps the funnels private and the `ipc` modules
free of `app` internals. Widening the seams to
`pub(in crate::application)` is an acceptable equivalent, but the
trait shim is the §2/§7-aligned choice and reuses an existing
precedent; it is IPC-03's to wire and is called out here so the
"reaches the funnels" language in §D5/`format/ipc.md` isn't read as
"calls private items across a module boundary."

## D6 — Cross-platform stance (§4): explicit native carve-out

**Decision.** IPC is native-only at the `src/application/ipc/`
module boundary (`#[cfg(not(target_arch = "wasm32"))]`, the
`console_input` precedent). The §4-mandated carve-out entry now
exists in `CLAUDE.md` "Dual-target status" — a section this PR
creates, since §4, CONCEPTS §1, and `freeze_watchdog.rs` all
referenced it and it had never been written; the already-documented
native-only surfaces (watchdog, console modal, clipboard backend,
`fps` verb) are backfilled there from their existing doc entries so
the section is born truthful.

**Why carve out rather than build dual-target now.** The transport
is *definitionally* native — browsers cannot serve local sockets,
and the epic's consumer (an agent in a desktop/Xvfb container)
is native. The §4 rule exists to stop "I'll add WASM later" drift
on features that *could* be shared; here the shared part — envelope,
command semantics, registry — is exactly what `format/ipc.md` pins
platform-independently, and the WASM path (WebSocket transport +
browser capture, reusing the envelope verbatim) is a *named,
parked* trajectory with an owner: IPC-15. `Action`'s
`wasm_compatibility` taxonomy already gives IPC-15 its per-command
compatibility answers. `./test.sh`'s wasm32 type-check gate keeps
the boundary honest at every merge.

## D7 — Capture and determinism surface: shapes pinned, engines owned downstream

**Decision.** The command shapes for `capture.screenshot`,
`capture.record_start/stop`, `clock.status/set_mode/step`, and
`clock.wait` — params, defaults, clamps, results, the geometry
sidecar schema, the recording manifest schema, and the settling
semantics — are pinned in full in `format/ipc.md` (§capture,
§clock). IPC-07/08/09 implement engines behind stable shapes;
`mandalactl` and the two skills (IPC-12/13) build against the
shapes without waiting.

**The shape-level calls that needed making here:**

- **Offscreen, never window-grab.** `capture.screenshot` renders to
  an offscreen target sized by the request. This makes captures
  compositor-independent (Xvfb container = laptop = future
  headless IPC-10), decouples capture resolution from window size,
  and sidesteps the swapchain-format bug tracked in #7 (offscreen
  target format is IPC-07's to pin, coordinated with #7's SSOT).
- **Sidecar over annotation.** The pixels↔ids map is a JSON sidecar
  (`<path>.geometry.json`), not baked into the image — agents
  hit-test by arithmetic, humans view clean PNGs, and the sidecar
  schema can grow keys additively.
- **Bounded before allocating / running, and write-guarded.** A
  malformed agent frame must degrade, not detonate: `capture.*`
  clamps each side to `1..=16384` and the pixel budget
  (`w × h × scale²`) to 64 Mpx — an offscreen texture is a real GPU
  allocation. `clock.step` clamps to **one** budget: since virtual
  time advances at a pinned 16 ms/heartbeat, `frames = ceil(ms/16)`
  and both spellings clamp to `frames ≤ 100_000` (⇒ `ms ≤ 1_600_000`,
  ~27 min) — the earlier "`ms ≤ 1 h`" let the two params bound
  different work, now reconciled. And the clamp is *not* the
  watchdog-safety mechanism: the step loop **ticks the watchdog per
  internal heartbeat**, so a bounded catch-up is safe by
  construction while a genuinely stuck single heartbeat still trips
  the 10 s threshold; the clamp is a responsiveness/DoS limit.
  Because an LLM clobbers a path by accident more readily than a
  human, `capture.*` also **write-guards** client-supplied `path` /
  `dir` (no symlink-follow; no overwrite outside the artifacts dir
  without `overwrite:true`) and bounds the **server-chosen**
  artifacts dir (ring-buffer, removed on exit) — client-supplied
  paths are the client's to reclaim. Over-bound/guarded requests are
  `invalid_params`. Exact numbers: `format/ipc.md` §capture / §clock.
- **Recording emits frames + manifest; assembly is external.**
  GIF/video encoding in-app would drag an encoder dependency into
  the render path for a job `ffmpeg`/`gifski` do better; the
  `manifest.json` (with per-frame `t_ms` on the capture clock) is
  the contract between IPC-08's recorder and IPC-13's assembly
  skill. An **active recording is a continuation/timer source** —
  on a static scene the loop would park and capture nothing, so
  while recording it parks in `WaitUntil(next frame deadline)` at
  the requested `fps` (under the virtual clock, `clock.step` drives
  frames instead). The **frame-emission rule** is pinned so the
  manifest is deterministic: emit a frame each time the capture
  clock crosses a `1000/fps` boundary — **not** once per heartbeat —
  so `clock.step ms=1000` at `fps=30` yields exactly 30 frames no
  matter how many heartbeats ran. That one rule is the whole IPC-08 ↔
  IPC-09 interlock EPIC #60 warns about.
- **`act.console` reads a typed result, not a live scrollback.**
  `execute_console_line` today returns `()` and pushes verb output
  into a `ConsoleState` scrollback that only exists when the modal
  is `Open`. IPC must not fake an open modal to scrape it; IPC-03
  extracts the parse-and-execute core to return the `ExecResult`
  the command already produces (`Ok`/`Lines`/`Err`), which the
  modal path keeps rendering to scrollback and IPC reads directly.
  Threading the result out is sanctioned §2 seam work; pinned in
  `format/ipc.md` §act so IPC-03 doesn't improvise a UI-scraping
  path.
- **`clock.wait` is a deferred reply** evaluated at heartbeat
  boundaries and at its deadline (D2/D3), with two conditions only:
  `animations_complete` and `settled`, and **exclusive per
  controller** (a second pending wait → `busy`; unbounded pending
  waits would be a memory/scan-cost vector, and there's no named use
  for concurrent waits). **`settled`** means "the rendered scene is
  the final consequence of every command the agent sent" and keys on
  a **render revision**, not the document revision:
  `needs_continuation()` false, injected-input queue empty (the
  one-sample-per-frame queue from D5/§input — pinned so `settled`
  and IPC-06 share one model), no redraw pending, and the frame has
  been **rendered** (render pass ran, buffers current) at the
  current render revision. Two subtleties the review surfaced, both
  now pinned: (1) `settled` keys on *rendered*, **not**
  swapchain-*presented* — an offscreen/headless (IPC-10) or occluded
  capture never presents, so a present-gated `settled` would time
  out in exactly the target environment; and (2) "race-free" is
  scoped to the agent's own commands — a concurrent human's input
  between the wait reply and the screenshot, and especially an inline
  human pan-drag that changes pixels without bumping the revision, is
  not fenced. The counter advances on *every* pixels-affecting
  command, view-only ones (zoom, pan, selection) included; keying on
  the document revision alone would resolve `settled` before a
  view-only repaint rendered (this is why D2 has the IPC drain
  request a redraw and bump the render revision per pixels-affecting
  command). IPC-14 owns the counter (overlapping the
  `document_changed`/`camera_changed`/`selection_changed` emission it
  already builds — the many-site instrumentation is not net-new
  scope); if wave order runs IPC-09 first, IPC-09 carries the field
  itself. `MindMapDocument.dirty` deliberately plays no role: despite
  the name it is the *unsaved-changes* flag — set by document
  setters, cleared only at construction and on `save`, read by the
  `open` / `new` guards — not a rebuild signal. Issue #61's sketch
  assumed otherwise, as did CONCEPTS' "Dirty flag" entry; both
  corrections land with this design, because a settled condition
  gated on `dirty` would never resolve after any unsaved mutation.
  The `timeout_ms` deadline is wall-clock even under a virtual clock
  (it guards client liveness, not simulation time), and a pending
  wait converts an idle park from `ControlFlow::Wait` to
  `WaitUntil(nearest deadline)` — the FPS idle-grace precedent —
  so deadlines fire without polling and without a wake-starved
  hang. A wait is not a barrier: dependent commands are sent after
  its reply (D3). A third `idle` condition was considered and
  dropped: post-settled the loop parks by construction, and the
  only residual timer is the FPS overlay's idle-grace flip — a
  diagnostic that observes behavior and must not become observable
  behavior.
- **Virtual time is a mode, not a per-command flag.**
  `clock.set_mode virtual` freezes app-observed time except via
  `clock.step`, making animation state a pure function of the step
  sequence. How virtual time threads through `now_ms()` and the
  heartbeat is IPC-09's design problem (EPIC #60 rates it Fable
  for a reason); the *surface* above is what it must satisfy, and
  the single-call-site clock parameterization of `tick_animations`
  is the seam it starts from.

## D8 — Observability: replies carry their own diagnostics

**Decision.** Every reply may carry a `warnings` array of
`{"code", "message"}`; every command handler receives a warning
sink; the convention is **dual-emit** — the same site that
`log::warn!`s a degrade pushes the same text into the sink. Defined
once in `format/ipc.md` §Warnings; implementing issues own their
commands' warning completeness, and extending a seam to surface a
degrade upward is sanctioned §2 work.

**Why the envelope and not the log stream.** Release builds compile
logs out entirely (`release_max_level_off` — the #45 decision
gate), so an agent driving a release binary would fly blind on
exactly the §9 degrade paths it most needs to see (font drops,
rejected steps, no-op mutations). The reply envelope is the one
channel that provably reaches the agent in every build profile.
Streaming out-of-band degrades (a `log_line` event class) becomes
possible only under #45's Option A and is parked there — this
design takes no dependency on that decision in either direction.

---

## What each downstream issue consumes from this design

| issue | builds | consumes from IPC-01 |
|---|---|---|
| IPC-02 | transport + loop integration | D1 (socket/flag/lifecycle), D2 (threads, queues, `IpcWake`, watchdog rules), D3 (framing/caps/`hello`), D6 (cfg boundary), §3 amendment text |
| IPC-03 | registry + `act` bridge + gates | D5 (registry/`describe`), D4 (User posture, composition rule), D3 (envelope, error codes), D8 (warning sink plumbing) |
| IPC-04 | `query` family | D5 (module/table), D3, D8; camera accessor micro-overlap note |
| IPC-05 | `scene` family | D5, D3 (`to_file` discipline), coordinate spaces; canonical hit-test rule (never fork) |
| IPC-06 | `input` family | D5, D3; fidelity contract (same handler entry points as winit events); the injected-input queue + one-sample-per-frame pacing (shared with `settled`) |
| IPC-07 | screenshot engine | D7 shapes (params/sidecar/`space` grid/write-guards), offscreen mandate, #7 coordination |
| IPC-08 | recorder | D7 shapes (manifest), external-assembly split, capture-clock `t_ms`, the virtual-clock frame-emission rule |
| IPC-09 | virtual clock + wait | D7 (`clock.*` shapes; `settled` keys on *rendered* not presented; wait exclusivity; watchdog-per-heartbeat; injected-input queue), D2 (deferred replies) |
| IPC-10 | headless | D7 (offscreen capture path; `settled`-on-rendered makes headless capture terminate), `Options.should_exit` seam; app-shell freeze per EPIC #60 |
| IPC-11 | `mandalactl` | D3 (protocol pinning/refusal; enum-widening is breaking), `format/ipc.md` as its command source |
| IPC-12/13 | skills | pinned capture/wait shapes; artifacts + manifest layout |
| IPC-14 | events + revision | D3 (event frames, reserved classes, envelope `revision`); the render-affecting revision counter + event backpressure coalescing; D8 (#45 boundary), #43 boundary (app-funnel only) |
| IPC-15 | WASM parity | D6 (parked trajectory: WebSocket transport reusing the *envelope + commands*; framing/caps re-expressed per transport), `wasm_compatibility` taxonomy |
| IPC-16 | docs sweep | Change-discipline section of `format/ipc.md`; `describe` byte-honesty audit |

## Amendment ledger

Documents moved by this design, in the same PR that pinned it:

- **CODE_CONVENTIONS §3** — the sanctioned IPC boundary: a
  persistent acceptor plus a per-controller reader + outbound pair
  (generation-stamped, torn down together on disconnect),
  `std::sync::mpsc` carrying protocol values only, proxy wake,
  main-thread-only command execution, no blocking IPC I/O on the
  main thread. The covenant's scope line ("in interactive paths")
  now has its second sanctioned boundary case after the watchdog.
- **CLAUDE.md "Dual-target status"** — section created (it was
  referenced by §4, CONCEPTS §1, and `freeze_watchdog.rs` but never
  existed); IPC carve-out entry added; existing documented
  native-only surfaces backfilled.
- **CONCEPTS.md** — §1 "Single-threaded event loop" and §5
  "FreezeWatchdog" both stop claiming the watchdog is the *only*
  sanctioned thread (they now point at the §3 amendment). §5
  "Dirty flag" is rewritten to match code — it is the
  unsaved-changes flag, not a rebuild trigger; drift caught while
  pinning D7's `settled` — and the event-loop step list drops its
  stale dirty-gated-rebuild claim.
- **`format/macros.md`** — SOURCE-OF-TRUTH tier list gains the
  IPC-maps-to-User entry; privilege-model section states the
  composition rule (D4).
- **`format/README.md`** — indexes `ipc.md` under a "Wire protocol"
  heading (it documents a protocol, not the on-disk map format —
  placed per IPC-01's instruction to mirror `macros.md`'s home).

## Change governance

This document and `format/ipc.md` change **together or not at all**:
semantics here, bytes there, `protocol` bump when breaking (D3).
Decisions D1–D8 were reviewed as the epic's human gate (EPIC #60
"Human-decision gates"); a downstream issue that finds a decision
unworkable stops and amends here first — implementation-side
workarounds of a pinned decision are the drift class this document
exists to prevent. No decision in this file is deferred, optional,
or "TBD"; if a section reads ambiguous, the ambiguity is a bug in
this file — fix the file.
