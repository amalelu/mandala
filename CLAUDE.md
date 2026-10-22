# CLAUDE.md
§1 When launching sub-agents for investigation or reviews, always use the most powerful agent you have available, 
not whatever is the default. Opus or if available Mythos

§2 NEVER skip changes because they are "merely cosmetic". 

§3 When proposing multiple options, if any of those options strays from the original task then make that absolutely clear

§4 NEVER use "Not introduced by me" as excuse. No one cares, just address it.

§5 NEVER defer the hard parts until later, and then proceed to ship a "good enough" or "approximate" now unless specifically instructed to. The hard parts are the work, there is nothing else.

§6 Use American English for consistency, not British English

§7 NEVER run benchmarks — no `cargo bench`, no `./bench.sh`, no `./test.sh --bench`. Run the tests.
`./test.sh` is the gate. Changing benchmark code is fine (§B3 requires a bench entry alongside a new
primitive); executing it is not — `cargo check --workspace --benches` proves the target still builds.
Say `--workspace`, and say it every time: without it cargo selects only the root package, which
declares `autobenches = false` and owns no bench target, so the bare form exits 0 on a bench file that
does not compile. `test_no_bench_check_selects_a_package_that_owns_no_bench` holds the flag to that
spelling in every Markdown file, shell script, CI workflow and Rust comment here — `test.sh`, where
the command is run rather than described, included. Make no performance claims: §B7 wants a
main-against-main control row you will not have, and control runs on identical code swing ±10–25% at
p=0.00 on this hardware, so an uncontrolled number is indistinguishable from noise. State work
removed as a structural fact visible in the diff, never as a measured win. See `AGENTS.md`, which
carries this same rule for agents on other harnesses.

"API error: Stream idle timeout - partial response received" is an error that occurs regularly these days. 
To avoid it, please make sure that any large files such as (but not limited to) plan files are written in 
smaller pieces first, and then finally combined into the full file.

## What this is

Mandala is a Rust mindmap application built on wgpu and cosmic-text, using
the Baumhard glyph-animation library under `lib/baumhard`. It runs on both
native desktop and as a WebAssembly build. `.mindmap.json` files are loaded
and rendered as interactive canvases where every visual element — text,
borders, connection paths — is laid out as positioned font glyphs.

## Important references

- **`CONCEPTS.md`** — the conceptual building-blocks reference: what
  each named concept (`GlyphArea`, `MutatorTree`, `Channel`, `Portal`,
  `ZoomVisibility`, `CustomMutation`, ...) is, what problem it solves,
  and where it lives. Start here when a term is unfamiliar or for a
  top-down orientation across both crates.
- **`CODE_CONVENTIONS.md`** — the workspace-wide coding conventions and
  philosophy. Mandatory read.
- **`lib/baumhard/CONVENTIONS.md`** — crate-local rules for baumhard:
  mutation-not-rebuild, grapheme-aware text, arena discipline,
  benchmark-reuse, no-unsafe policy, and performance rules. Read this
  before touching anything under `lib/baumhard/`.
- **`TEST_CONVENTIONS.md`** — testing philosophy, where to put tests, the
  `do_*()` benchmark-reuse pattern, and what we deliberately don't do
  (no mocks, no snapshots, no GPU tests).
- **`format/`** — the `.mindmap.json` format specification.
  `format/schema.md` is the primary reference; per-concept docs cover
  Dewey-decimal IDs, named enums, palettes, channels, text runs,
  validation invariants, portal labels, mutations, and migration from
  legacy. Read this before changing the data model.
- **`crates/maptool/`** — CLI tool for working with `.mindmap.json`
  files: `show`, `grep`, `apply`, `export`, `convert --legacy`
  (migration from miMind-derived format), and `verify` (structural
  validation).
- **`lib/baumhard/src/mindmap/`** — the data model, loaders, scene
  builders, and the tree bridge. Most interesting logic lives here.
- **`src/application/`** — the app shell: event loop, document state,
  rendering pipeline, and input handling.

## Dual-target status

Native desktop and the browser build are first-class peers
(CODE_CONVENTIONS §4). This section is the live registry of
sanctioned native-only carve-outs; each entry names the reason and
the parity trajectory (or why none is owed):

- **IPC** (`--ipc`, `src/application/ipc/`; transport since
  IPC-02/#62, command registry and `act` bridge since IPC-03/#63)
  — native-only by transport nature: browsers cannot
  serve local sockets, and the consumer is an agent driving a
  desktop/Xvfb instance. Protocol: `format/ipc.md`; design:
  `work_plans/LLM_IPC.md`. The browser trajectory (WebSocket
  transport + browser-side capture, same envelope and commands) is
  parked in IPC-15 (#75). Within native the transport is Unix-only
  for now: `--ipc` on Windows fails at startup naming the gap, until
  the named-pipe adapter lands (#169).
- **Console modal shell** (`src/application/app/console_input/`) —
  the modal shell and its exec path are native-gated; the
  verb/parser layer (`src/application/console/`) is
  cross-platform. Parity is the named next step in CONCEPTS §6
  "Console", not yet scheduled.
- **FreezeWatchdog** (`src/application/app/freeze_watchdog.rs`) —
  native-only by design: browsers ship their own unresponsive-tab
  dialog; no parity owed.
- **`MAX_MAP_BYTES`** (`lib/baumhard/src/mindmap/loader.rs`) —
  `None` on native, `Some(3 GiB)` on wasm32. Divergent by
  platform physics rather than policy: wasm32's `usize` is 32
  bits and its address space is 4 GiB, shared with the runtime,
  the GPU buffers and the browser. Native opens maps as large as
  the machine can hold, which is the product intent; the browser
  cannot, and says so with a message instead of trapping
  mid-parse. No parity is owed — the trajectory is the reverse,
  a 64-bit wasm target, which is not this project's to schedule.
- **`MAX_UNKNOWN_KEYS`**
  (`lib/baumhard/src/mindmap/unknown_keys.rs`) — `None` on
  native, `Some(2_000_000)` on wasm32, for the same reason and
  with the same trajectory as `MAX_MAP_BYTES` above. It began as
  a resource ceiling standing in for a cost defect; once the
  capture and the write-back were both made linear there was
  nothing on native left for it to bound.
- **Clipboard OS layer** (`src/application/clipboard.rs`) — native
  is backed by `arboard`; WASM is stubbed pending async-clipboard
  integration (CONCEPTS §6 "Clipboard").
- **`fps` console verb** — native because the console shell is; the
  render-side FPS plumbing compiles on both targets and browsers
  expose frame timing via DevTools (CONCEPTS §5 "FPS overlay").
- **Single-line editor** (`src/application/app/single_line_edit/`)
  — edge-label and portal-caption inline editing is native-only;
  the browser has no single-line editor yet. Consequence for
  input: an edge-label double-click commits the selection on both
  targets and opens the editor only on native, surfaced as
  `DoubleClickResidual::OpenEdgeLabelEditor` rather than a `cfg`,
  and logged at `debug!` on the browser side so the stop is
  observable. Parity unblocks `Action::DoubleClickActivate` and
  `Action::EditSelection*` flipping to `Compatible`; tracked in
  `work_plans/WASM_CONVERGENCE.md`.
- **`LongPress` → `EnterResizeMode`** (`src/application/keybinds/config.rs`,
  `src/application/app/run_wasm/event_touch.rs`) — the one touch
  gesture that resolves through the keybind table, and the one that
  is native-only. Its default `Action` is `NativeOnly`, so on the
  browser the dispatch returns `Unhandled` and a one-shot `warn!`
  names the remedy; the gesture is *recognized* identically on both
  targets, and only the `Action` it reaches is not. Deliberate rather
  than pending: long-press is the touch peer of the keyboard's `r`,
  so rebinding the browser's long-press to some unrelated
  `Compatible` Action would make one gesture mean two different
  things on two targets — a worse §4 outcome than a gesture that is
  honestly unavailable and says so. **The rest of the touch
  vocabulary is not affected**: tap, one-finger pan and pinch-zoom
  resolve to no `Action` at all and run on both targets through
  `dispatch::apply_touch_effect`, which is what makes the browser
  usable by finger without this entry being closed. Parity is the
  `InteractionMode::Resize` chrome plus its handle-drag reaching the
  browser — the same drag-state machine **Rubber-band select** below
  is parked on — and flips this `Action` to `Compatible` when it
  lands.
- **Rubber-band select** (`DragState::SelectingRect`,
  `src/application/app/drain_frame.rs`,
  `scene_rebuild::rebuild_selection_highlight`,
  `MindMapDocument::{set,take}_rect_select_preview`) —
  native-only because `DragState` and the per-frame drain are:
  the browser has no drag-state machine and no `AboutToWait` to
  hang a per-frame hit-test off, so there is nothing for a
  Shift+drag to enter. **The read is deliberately not gated.**
  `MindMapDocument::rect_select_preview` (the method) and
  `scene_rebuild::highlight_entries_for` are cross-platform,
  because both targets' node-tree rebuilds go through the one
  mapping that answers "which nodes are tinted"; only the
  gesture that fills it is native. On wasm32 the field is
  therefore always `None`, and the tier
  `rebuild_selection_highlight` is not native *by nature* —
  whichever browser-side gesture first needs a highlight-only
  repaint takes the `cfg` off it. Parity for the gesture itself
  rides on the browser gaining a drag-state machine — a press
  needs somewhere to be pending before it can promote — and is
  not yet scheduled. **That is not the same prerequisite the
  per-frame animation drain below names**, which is a rAF-driven
  tick pump; this sentence claimed it was until #35's registry
  work read the two entries side by side. What the two do share
  is the *shape*: a per-frame mechanism native has and the
  browser does not, so either one landing makes the other
  cheaper without supplying it.
- **Per-frame animation drain**
  (`src/application/app/drain_frame.rs`) — native-only, so
  *animated* `CustomMutation`s (`timing.duration_ms > 0`) start on
  the browser and never tick: `drain_animation_tick` is the only
  advance site and there is no browser rAF-driven equivalent
  wired to it. Instant mutations are unaffected and work on both
  targets. Two browser entry points stall, through **two
  different bodies**: the `custom_mutation_bindings` keystroke
  tier (`run_wasm/event_keyboard.rs`) reaches
  `apply_keybind_custom_mutation`'s `start_animation` branch,
  while `click_triggers::fire_onclick_triggers` carries its own
  copy of the animated-vs-instant routing and calls
  `start_animation_at`. That duplication predates this work and
  is named at both sites; unifying it is a behavior decision
  rather than a refactor, because the two copies disagree about
  the no-tree case. Parity for the stall itself is a browser
  drain loop hung off the existing rAF render loop, pumping the
  same `drain_animation_tick` body; not yet scheduled.

## Common tasks

- **Run tests**: `./test.sh` runs `cargo test --workspace` — all four
  members, `mandala`, `baumhard`, `mandala_derive` and `maptool` —
  prints a test count, then type-checks the benchmark targets
  (`cargo check --workspace --benches`, which runs no benchmark) and
  `wasm32-unknown-unknown`, so neither a drifted `do_*()` nor
  cross-platform drift can pass a green run. **With one caveat about
  where**: the wasm32 leg is *skipped with a note* when the target is
  not installed, so a green local run on a machine that has never
  had `rustup target add wasm32-unknown-unknown` has proved nothing
  about the browser build. `rust-toolchain.toml` lists the target, so
  `rustup toolchain install` in the checkout supplies it, and CI runs
  exactly that (`.github/workflows/test.yml`), so the guarantee is
  unconditional there and conditional here. The same
  holds for `lib/baumhard/tests/wasm_font_lock.rs`, which *runs*
  baumhard's font-lock helpers on wasm32 under `node` (TEST_CONVENTIONS
  §T9): it skips with a note without the target or `node`, and CI sets
  `MANDALA_REQUIRE_WASM_PROBE` so it cannot skip there. The
  test count itself is not a gate — a summary-format change makes it
  print "count unavailable" rather than ending the run, which is
  what it used to do, silently, under `set -euo pipefail`. Flags: `--coverage`
  (runs under `cargo-llvm-cov`, outputs
  `target/llvm-cov/html/index.html`), `--lint` (runs
  `cargo fmt --check`, `cargo clippy` and `cargo doc`. `fmt`, both
  `doc` legs and both `clippy` legs are **hard gates** — their
  baselines are zero (#130, #134 for the first two; `clippy` followed
  once its warnings were cleared), a regression prints a `FAILED:`
  line and the run exits non-zero at the end, after every other gate
  has reported. `clippy` runs with `-D warnings`, because `cargo
  clippy` exits 0 on warnings. `fmt` runs
  **once** — rustfmt parses rather than compiles, so it is
  target-independent and there is no wasm32 leg for it. `clippy` and
  `doc` run **twice**, on the host target and on
  `wasm32-unknown-unknown`: a host-target compile drops everything
  under `#[cfg(target_arch = "wasm32")]`, so without the second leg the
  browser half of the app — its lints and its intra-doc links alike —
  is invisible. The wasm32 clippy leg is `--workspace` so baumhard is
  covered too, minus `--all-targets` (criterion and rayon refuse to
  build for wasm32); the host doc leg is `--workspace` with private
  items, while the wasm32 doc leg narrows to `-p mandala`, because
  baumhard cannot be documented standalone for wasm32 — its `rand` →
  `getrandom` edge needs the `wasm_js` feature that only the root
  manifest declares — and maptool and mandala_derive have no wasm32
  half to document. Both wasm32 legs are skipped with a note when the
  target isn't installed. CI runs `./test.sh --lint`, so these gates
  hold on every pull request and not only on the machine of whoever
  remembered the flag. They can be hard because the toolchain is
  pinned in `rust-toolchain.toml`: a new stable release cannot add a
  lint until the pin is bumped, and bumping it is a change of its own
  that fixes what the new lints report), `--bench` (runs the criterion
  benches after tests — **maintainers only**; §7 and `AGENTS.md`
  forbid it to agents, who need the type-check above and nothing
  else).
- **Build releases**: `./build.sh` builds both the native binary
  (`target/release/mandala`) and the WASM bundle (`dist/` via
  `trunk build --release`), replacing prior output for the chosen
  profile — each output directory is set aside and discarded only
  once its replacement exists, so a failed build leaves the artifacts
  you already had intact, at the cost of holding a second copy of the
  output tree for the duration. `--debug` builds dev profile
  on both sides; `--fat` switches native to `release-lto`;
  `--no-keep` deletes the prior output up front instead, for when the
  disk matters more than the safety net. Requires `trunk` on `PATH`
  and the `wasm32-unknown-unknown` target installed.
- **Run the app**: `./run.sh [map.mindmap.json]` launches the release
  binary and `trunk serve --release` in parallel; Ctrl+C stops both.
  It launches rather than builds, so it refuses up front — naming
  which one — when `target/release/mandala` is absent or not
  executable, when the map file does not exist, or when `trunk` is
  not on `PATH`; produce the binary with `./build.sh` first. For
  one-off iteration use `cargo run -- maps/testament.mindmap.json`
  (native) or `trunk serve` (WASM) directly; `cargo run` builds what
  it needs, which makes it the form that works from a fresh clone.
- **Dev-profile build**: `./debug_build.sh [flags]` is `./build.sh
  --debug` with every other flag forwarded — the shorthand for the
  build run most often while iterating.
- **Target a specific test**: `cargo test -p baumhard --lib <pattern>`,
  `cargo test -p mandala <pattern>`,
  `cargo test -p mandala_derive` or `cargo test -p maptool`. `--lib`
  narrows to a library target, so it belongs only to the two members
  that have one — baumhard and mandala_derive. `mandala` and
  `maptool` are binary crates, where adding it narrows nothing and
  exits 101 with `no library targets found in package 'mandala'`
  before a test runs (#148).
  `test_no_documented_command_targets_a_lib_the_package_lacks` holds
  this bullet and TEST_CONVENTIONS §T11's copy of it against the real
  manifests — every code span and every fenced line in those two
  sections — so the flag cannot come back to either of them on a
  member that has no library target. It reads nothing else: the rest
  of this file, and every other document, is on your own care.
- **Load a different mindmap**: the first positional CLI arg is the path
  to a `.mindmap.json` file; WASM reads it from the `?map=` query param.
- **Drive the running app over IPC** (native, Unix): launch with
  `--ipc <socket-path>` — `$XDG_RUNTIME_DIR/mandala.sock` is the
  recommended place — then speak newline-delimited JSON to it, e.g.
  `printf '{"id":1,"cmd":"ping"}\n' | nc -N -U $XDG_RUNTIME_DIR/mandala.sock`
  (`-N` closes the write half after the request; the server answers
  everything it received, then closes).
  `{"id":2,"cmd":"describe"}` lists the commands this build serves;
  `{"id":3,"cmd":"act.action","action":"ZoomIn"}` drives an Action,
  and `act.console` / `act.macro` run a console line or a macro;
  `format/ipc.md` is the full protocol.
