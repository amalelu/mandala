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
primitive); executing it is not — `cargo check --benches` proves the target still builds. Make no
performance claims: §B7 wants a main-against-main control row you will not have, and control runs on
identical code swing ±10–25% at p=0.00 on this hardware, so an uncontrolled number is indistinguishable
from noise. State work removed as a structural fact visible in the diff, never as a measured win.
See `AGENTS.md`, which carries this same rule for agents on other harnesses.

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

- **IPC** (`--ipc`, `src/application/ipc/`; lands with IPC-02/#62)
  — native-only by transport nature: browsers cannot serve local
  sockets, and the consumer is an agent driving a desktop/Xvfb
  instance. Protocol: `format/ipc.md`; design:
  `work_plans/LLM_IPC.md`. The browser trajectory (WebSocket
  transport + browser-side capture, same envelope and commands) is
  parked in IPC-15 (#75).
- **Console modal shell** (`src/application/app/console_input/`) —
  the modal shell and its exec path are native-gated; the
  verb/parser layer (`src/application/console/`) is
  cross-platform. Parity is the named next step in CONCEPTS §6
  "Console", not yet scheduled.
- **FreezeWatchdog** (`src/application/app/freeze_watchdog.rs`) —
  native-only by design: browsers ship their own unresponsive-tab
  dialog; no parity owed.
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
  cross-platform drift can pass a green run. Flags: `--coverage`
  (runs under `cargo-llvm-cov`, outputs
  `target/llvm-cov/html/index.html`), `--lint` (advisory
  `cargo fmt --check`, `cargo clippy` and `cargo doc`. `fmt` runs
  **once** — rustfmt parses rather than compiles, so it is
  target-independent and there is no wasm32 leg for it. `clippy` and
  `doc` run **twice**, on the host target and on
  `wasm32-unknown-unknown`: a host-target compile drops everything
  under `#[cfg(target_arch = "wasm32")]`, so without the second leg the
  browser half of the app — its lints and its intra-doc links alike —
  is invisible. The wasm32 clippy leg is `--workspace` so baumhard is
  covered too, minus `--all-targets` (criterion and rayon refuse to
  build for wasm32); the wasm32 doc leg is `-p mandala`, the same
  invocation as the host one with only the target changed, because
  baumhard cannot be documented standalone for wasm32 — its `rand` →
  `getrandom` edge needs the `wasm_js` feature that only the root
  manifest declares. Both wasm32 legs are skipped with a note when the
  target isn't installed), `--bench` (runs the criterion
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
  For one-off iteration use `cargo run -- maps/testament.mindmap.json`
  (native) or `trunk serve` (WASM) directly.
- **Target a specific test**: `cargo test -p baumhard --lib <pattern>`,
  `cargo test -p mandala --lib <pattern>`,
  `cargo test -p mandala_derive` or `cargo test -p maptool`.
- **Load a different mindmap**: the first positional CLI arg is the path
  to a `.mindmap.json` file; WASM reads it from the `?map=` query param.
