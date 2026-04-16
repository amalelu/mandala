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

- **`DEPRECATED_ROADMAP.md`** — historical record of what each session did,
  kept for reference. The earlier ROADMAP.md has been superseded; the
  living "current state" now lives in this file's **Dual-target status**
  section below (for parity) and in individual commit messages (for the
  detail of what each session landed). Read the deprecated roadmap before
  proposing large changes so you don't re-do something that's already
  landed, but don't expect it to reflect the last few sessions.
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
- **`lib/baumhard/src/mindmap/`** — the data model, loaders, scene
  builders, and the tree bridge. Most interesting logic lives here.
- **`src/application/`** — the app shell: event loop, document state,
  rendering pipeline, and input handling.

## Architectural shape (high level)

- **Single-threaded** — the event loop owns the renderer directly.
  Earlier revisions were multi-threaded with channels; that's gone. Don't
  reintroduce threading without a real reason.
- **Model / View separation** — `MindMapDocument` owns the data model
  (`MindMap`, selection, undo stack). The `Renderer` owns GPU resources
  and rebuilds buffers from intermediate representations the document
  hands it.
- **Dual rendering pipeline** — nodes render through a Baumhard
  `Tree<GfxElement, GfxMutator>`; connections and borders render through a
  flat `RenderScene`. These are two different paths, wired side-by-side in
  the event loop. Be aware which pipeline you're touching.
- **Mutation-first interaction model** — the roadmap's guiding philosophy
  is that user actions should be expressible as mutations cascading
  through the Baumhard tree where possible. Not every interaction fits
  this (anything outside the tree — edges, borders, UI overlays — has to
  reach the scene builder or renderer instead), but when something *can*
  be a mutation, prefer that.
- **Cross-platform reality** — almost everything that works on native
  should also compile for wasm32. Native-only code lives behind
  `#[cfg(not(target_arch = "wasm32"))]`; WASM-only code behind
  `#[cfg(target_arch = "wasm32")]`. The WASM input path currently lags
  behind the native input path — see the **Dual-target status** section
  below for the precise surface.

## Dual-target status

Mandala is built for native desktop (`cargo run`, primary dev loop) and
WASM (`trunk serve` / `trunk build`, the browser build — same binary on
desktop and mobile browsers). Per `CODE_CONVENTIONS.md §2` the two
targets are equal citizens; this section tracks the current parity
surface so a new session doesn't have to trawl `#[cfg]` guards to learn
what works where.

**Runs on both targets:**
- Document model, scene builder, tree bridge — all of `MindMapDocument`
  and `baumhard::mindmap::*`.
- Click to select (nodes); scroll-wheel zoom; undo stack.
- Inline node text editor: double-click / Enter / Backspace to open,
  full keyboard flow (cursor, delete, insert, multi-line), click-outside
  to commit, Esc to cancel. Shared implementation under
  `src/application/app/text_edit/`.
- Action dispatch for `Undo`, `CreateOrphanNode`, `OrphanSelection`,
  `DeleteSelection`, `EditSelection`, `EditSelectionClean`, `CancelMode`.
- Keybind config loading (native: CLI arg + XDG; WASM: `?keybinds=…`
  query param + `localStorage`). Same resolver on both sides —
  `KeybindConfig::load_for_desktop` / `load_for_web` under
  `src/application/keybinds/platform_{desktop,web}.rs` are the reference
  for how platform-split config loading should shape.
- Cross-platform monotonic clock via `now_ms()` in
  `src/application/app/mod.rs` (native: `Instant`; WASM:
  `performance.now()`).

**Native-only today** (each is a roadmap-scale gap, not a style choice):
- Drag gestures: pan, move-node, edge-handle, rect-select — the entire
  `DragState` enum is native-gated.
- `AppMode::{Reparent, Connect}` — the mode state machine plus its
  hover preview and click routing.
- Modals: CLI console (`/` trigger), glyph-wheel color picker, edge
  label editor — state types and rebuild paths all live under
  `#[cfg(not(target_arch = "wasm32"))]`.
- Hover-based UI: `hovered_node` tracking, cursor-change on button
  nodes, OnClick trigger dispatch.
- Clipboard copy/paste — `arboard` on native; WASM `clipboard.rs` stubs
  with `log::warn!` because the browser Clipboard API is async and not
  yet integrated.

**Absent on both targets** (named so they're visible as gaps, not
mistaken for "handled somewhere"):
- Touch gestures (tap / pinch / long-press) — the `PlatformContext::Touch`
  enum variant exists in the custom-mutation registry but no input path
  detects or dispatches on it.
- DPI-aware canvas sizing on WASM (no `devicePixelRatio` handling —
  the canvas buffer tracks CSS pixels 1:1 today).
- `PlatformContext` runtime detection — always uses the compile-time
  `Desktop` / `Web` branch, never `Touch`.

The prescriptive rule that goes with this list — new interactive
features need a cross-platform story from the start — lives in
`CODE_CONVENTIONS.md §2`. The local checks that enforce it
(`./test.sh` WASM gate, `./build.sh --wasm`) are named under
"Common tasks" below.

## Common tasks

- **Run tests**: `./test.sh` (runs the full suite across both crates,
  prints a test count, then type-checks `wasm32-unknown-unknown` so
  cross-platform drift fails the run). Variants: `./test.sh --coverage`
  runs under `cargo-llvm-cov` (install once with
  `cargo install cargo-llvm-cov`, produces
  `target/llvm-cov/html/index.html` and `target/llvm-cov/lcov.info`);
  `./test.sh --lint` adds an advisory `cargo fmt --check` +
  `cargo clippy` pass (never fails the run); `./test.sh --bench` also
  runs the criterion benches after tests pass.
- **Build releases**: `./build.sh` builds the native binary; `./build.sh
  --wasm` builds the WASM bundle via `trunk build --release` (requires
  `trunk` on `PATH`). Both share `--debug` for dev profiles.
- **Run the app**: `cargo run -- maps/testament.mindmap.json` (native) or
  `trunk serve` (WASM).
- **Target a specific test**: `cargo test -p baumhard --lib <pattern>` or
  `cargo test -p mandala --lib <pattern>`.
- **Load a different mindmap**: the first positional CLI arg is the path
  to a `.mindmap.json` file; WASM reads it from the `?map=` query param.

## Conventions worth knowing

- **Everything is glyphs** — text, borders, and connections all render as
  positioned font glyphs via cosmic-text. There are no rectangle shaders;
  if you want a new visual element, think about how to express it as
  characters first.
- **Single-parent tree** — `MindNode.parent_id: Option<String>` is the
  hierarchy. Non-hierarchical relationships use arbitrary edges with
  `edge_type: "cross_link"`. Don't introduce multi-parent shapes.
- **Edges have no stable IDs** — they're identified by the triple
  `(from_id, to_id, edge_type)`. Existing code uses this pattern
  consistently; mirror it when you need to reference edges.
- **Undo lives on the document** — user actions push `UndoAction` variants
  onto a stack, and `undo()` matches on the variant to reverse them. Each
  new user-facing mutation wants a matching undo variant.
- **Something looks unused?** — before deleting it, check whether it's
  deferred WIP from a past milestone or a seam the named trajectory
  (`CODE_CONVENTIONS.md §6`) preserves for future sessions.
  `DEPRECATED_ROADMAP.md`'s "What needs work" list is one reference
  point; the **Dual-target status** section above names parity gaps
  explicitly.

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
