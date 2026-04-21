# CLAUDE.md

Durable orientation notes for Claude Code sessions working on this repo.
Not a spec, not a recipe — just the stuff a new session tends to benefit
from knowing before diving in.

## What this is

Mandala is a Rust mindmap application built on wgpu and cosmic-text, using
the Baumhard glyph-animation library under `lib/baumhard`. It runs on both
native desktop and as a WebAssembly build. `.mindmap.json` files are loaded
and rendered as interactive canvases where every visual element — text,
borders, connection paths — is laid out as positioned font glyphs.

## Where to look first

- **`CODE_CONVENTIONS.md`** — the workspace-wide coding spec:
  architectural invariants, how to use baumhard, complexity and KISS
  heuristics, error-handling posture, and documentation standards.
  Prescriptive where `CLAUDE.md` is descriptive.
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

## Architectural shape (high level)

Invariants live in `CODE_CONVENTIONS.md §3` — this section is the shape,
not the rules.

- **Single-threaded, with a clean model/view split.** The event loop
  owns the `Renderer` directly; `MindMapDocument` owns the data model
  (`MindMap`, selection, undo stack) and hands the renderer intermediate
  representations to draw from. No channels, no worker threads for app
  logic. One disciplined exception: native builds run a freeze
  watchdog on a background thread
  (`src/application/app/freeze_watchdog.rs`). The watchdog only
  reads a shared `AtomicU64` timestamp the main loop pings — it
  never touches app state — so the single-threaded invariant for
  the model/view pipeline is preserved.
- **Dual rendering pipeline.** Nodes render through a Baumhard
  `Tree<GfxElement, GfxMutator>`; connections, borders, and portals
  render through a flat `RenderScene` wired via
  `src/application/scene_host.rs`. Be aware which pipeline you're
  touching.
- **Mutation-first interaction model.** Where a user action can be
  expressed as a mutation cascading through the Baumhard tree, prefer
  that. Things outside the tree — edges, borders, overlays — reach the
  scene builder or renderer directly.
- **Cross-platform reality.** Almost everything that works on native
  also compiles for wasm32. Native-only code sits behind
  `#[cfg(not(target_arch = "wasm32"))]`; WASM-only behind
  `#[cfg(target_arch = "wasm32")]`. WASM also falls back to WebGL2
  (via `wgpu`'s `webgl` feature) on browsers without WebGPU. The WASM
  input path currently lags behind native — see **Dual-target status**
  below for the precise surface.

## Dual-target status

Mandala is built for native desktop (`cargo run`, primary dev loop) and
WASM (`trunk serve` / `trunk build` — same binary on desktop and mobile
browsers). Per `CODE_CONVENTIONS.md §4` the two targets are equal
citizens; this section tracks the current parity surface so a new
session doesn't have to trawl `#[cfg]` guards to learn what works where.

**Runs on both targets:**
- Pluggable node shapes (rectangle, ellipse) for background fill +
  hit-testing. One `NodeShape` enum in
  `lib/baumhard/src/gfx_structs/shape.rs` drives both the rect
  pipeline's SDF fragment shader and the BVH descent. Adding a shape
  is localised: one enum variant + one WGSL `case` + one
  `contains_local` arm. See `format/enums.md#styleshape`.
- Document model, scene builder, tree bridge — all of `MindMapDocument`
  and `baumhard::mindmap::*`.
- Inline node text editor: double-click / Enter / Backspace to open,
  full keyboard flow (cursor, delete, insert, multi-line), click-outside
  to commit, Esc to cancel. Shared implementation under
  `src/application/app/text_edit/`.
- Portal-mode edges (`display_mode = "portal"`): two glyph labels, one
  per endpoint, each carrying a `PortalEndpointState` (color override,
  pinned `border_t` position, adjacent text + independent text color /
  font clamps). Single-click on the icon selects
  `SelectionState::PortalLabel`; single-click on the adjacent text
  selects `SelectionState::PortalText`; double-click on either pans
  to the opposite endpoint. Icon vs. text routes through separate
  renderer hitbox maps (`portal_icon_hitboxes` /
  `portal_text_hitboxes`) so per-channel operations (color / font
  via `color text=`, `font min=`, `font max=`) target the clicked
  sub-part. Dispatch wired in `event_mouse_click.rs` (native) and
  `run_wasm.rs` via the shared `ClickHit::{PortalMarker, PortalText}`
  paths. See `format/portal-labels.md`.
- Edge-adjacent selection variants `SelectionState::{EdgeLabel,
  PortalText}` (alongside `Edge` and `PortalLabel`). Each variant
  carries its own clipboard / color / font channel: clipboard
  copy/cut/paste on every edge-adjacent selection operates on the
  resolved color hex of that channel (body, label, icon, or text),
  and `font size= min= max=` writes to the independent font clamps
  on the corresponding config (`glyph_connection.*`,
  `label_config.*`, or `PortalEndpointState.text_*`). Single-click
  on an edge label AABB selects the label; double-click opens the
  inline editor (`open_label_edit` on native; WASM currently just
  commits the selection — editor-on-WASM is a follow-up). Console
  `label position_t=<f32>` / `label perpendicular=<f32>` mirror the
  edge-label drag so WASM users can position labels without drag.
- `connection::closest_point_on_path` — Baumhard primitive backing
  the edge-label drag. Projects a cursor Vec2 onto a
  `ConnectionPath` (straight or cubic) and returns
  `(position_t, perpendicular_offset)`. Straight case is direct
  segment projection; cubic case uses uniform-t sampling + Newton
  refinement. Used native-only today (the drag state it feeds is
  native-only, see below) but available for any future WASM
  gesture that needs the same math.
- Action dispatch: the keybind → action pipeline fires on both targets.
  Representative actions include `Undo`, `CreateOrphanNode`,
  `DeleteSelection`, `EditSelection`, `CancelMode`; the full enum lives
  in `src/application/keybinds/action.rs`.
- Keybind config loading — multi-source resolver with platform splits:
  native reads a CLI arg + XDG; WASM reads `?keybinds=…` + `localStorage`.
  `src/application/keybinds/platform_{desktop,web}.rs` is the reference
  pattern for platform-split config loading.
- Mutation framework — the `CustomMutation` carrier, four-source loader
  (`src/application/document/mutations_loader/` with matching
  `platform_{desktop,web}.rs` split), the `MutatorNode` AST + `build`
  walker (`baumhard::mutator_builder`), the channel-bypassing
  `Instruction::MapChildren` walker primitive, and the imperative
  `DynamicMutationHandler` seam for size-aware layouts
  (`src/application/document/mutations/{flower_layout,tree_cascade}.rs`).
  Both `run_native.rs` and `run_wasm.rs` wire the loader + handler
  registry at document-load time. See `format/mutations.md`.
- Cross-platform monotonic clock via `now_ms()` in
  `src/application/app/mod.rs` (native: `Instant`; WASM:
  `performance.now()`).

**Native-only today** (each is a parity gap, not a style choice):
- Drag gestures: pan, move-node, edge-handle, portal-label, rect-select
  — the entire `DragState` enum is native-gated.
- `AppMode::{Reparent, Connect}` — the mode state machine plus its
  hover preview and click routing.
- Modals: CLI console (`/` trigger), glyph-wheel color picker, edge
  label editor, portal-label text editor. The `mutation` console verb
  (`list` / `apply` / `help` / `inspect`) inherits this scope — the
  loader + registry run on both targets, only the UI shell is native.
- Hover-based UI: `hovered_node` tracking, cursor-change on button
  nodes, `OnClick` trigger dispatch.
- Clipboard copy/paste — `arboard` on native; WASM `clipboard.rs` stubs
  with `log::warn!`.
- Freeze watchdog — native runs a background thread
  (`src/application/app/freeze_watchdog.rs`) that aborts the
  process with a diagnostic banner if the main loop stalls longer
  than its threshold. WASM gets the browser's built-in
  "unresponsive tab" dialog for free, so no equivalent is wired;
  a Worker-based liveness check can slot in when a concrete need
  arises.

**Absent on both targets** (named so they're visible as gaps, not
mistaken for "handled somewhere"):
- Touch gestures (tap / pinch / long-press) — the
  `PlatformContext::Touch` variant exists but no input path dispatches
  on it.
- DPI-aware canvas sizing on WASM — the canvas buffer tracks CSS pixels
  1:1; no `devicePixelRatio` handling.
- `PlatformContext` runtime detection — always the compile-time
  `Desktop` / `Web` branch.

The prescriptive rule that goes with this list — new interactive
features need a cross-platform story from the start — lives in
`CODE_CONVENTIONS.md §4`. `./test.sh`'s WASM type-check gate and
`./build.sh --wasm` are the local checks that keep it honest.

## Common tasks

- **Run tests**: `./test.sh` runs the full suite across both crates,
  prints a test count, then type-checks `wasm32-unknown-unknown` so
  cross-platform drift fails the run. Flags: `--coverage` (runs under
  `cargo-llvm-cov`, outputs `target/llvm-cov/html/index.html`),
  `--lint` (advisory `cargo fmt --check` + `cargo clippy`), `--bench`
  (runs the criterion benches after tests).
- **Build releases**: `./build.sh` cleans prior output and builds both
  the native binary (`target/release/mandala`) and the WASM bundle
  (`dist/` via `trunk build --release`). `--debug` builds dev profile
  on both sides; `--fat` switches native to `release-lto`. Requires
  `trunk` on `PATH` and the `wasm32-unknown-unknown` target installed.
- **Run the app**: `./run.sh [map.mindmap.json]` launches the release
  binary and `trunk serve --release` in parallel; Ctrl+C stops both.
  For one-off iteration use `cargo run -- maps/testament.mindmap.json`
  (native) or `trunk serve` (WASM) directly.
- **Target a specific test**: `cargo test -p baumhard --lib <pattern>` or
  `cargo test -p mandala --lib <pattern>`.
- **Load a different mindmap**: the first positional CLI arg is the path
  to a `.mindmap.json` file; WASM reads it from the `?map=` query param.

## How to work with this codebase

- Prefer editing over creating. New files should feel justified.
- Small, focused changes land smoothly; sprawling refactors rarely do.
- Tasks are sized as session-chunks. If a task feels much bigger than
  its session, that's a signal to split it.
- When you add a user-facing feature, update the **Dual-target status**
  section above so the parity surface stays honest. A cross-platform
  feature adds to "Runs on both targets"; a native-only addition adds
  (with its reason) to "Native-only today" and a path forward.
- Tests are expected to stay green across changes. `./test.sh` runs
  everything including the WASM type-check gate; run it before
  committing.
