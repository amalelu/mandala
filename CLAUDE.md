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

- **`ROADMAP.md`** — the living source of truth for current state and
  planned work. "Current State" at the top lists what works today; the
  Milestones section documents what each session did. Read this before
  proposing large changes so you don't step on in-flight work or re-do
  something that's already landed.
- **`CODE_CONVENTIONS.md`** — the codebase's guiding spirit: KISS,
  readability, code-as-art, and explicit acceptance that the conventions
  are aspirational rather than enforced.
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
  behind the native input path — that's a known gap tracked in the
  roadmap.

## Common tasks

- **Run tests**: `./test.sh` (runs the full suite across both crates and
  prints a test count). Variants: `./test.sh --coverage` runs under
  `cargo-llvm-cov` (install once with `cargo install cargo-llvm-cov`,
  produces `target/llvm-cov/html/index.html` and
  `target/llvm-cov/lcov.info`); `./test.sh --lint` adds an advisory
  `cargo fmt --check` + `cargo clippy` pass (never fails the run);
  `./test.sh --bench` also runs the criterion benches after tests pass.
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
  deferred WIP from a roadmap milestone. The roadmap's "What needs work"
  section is a good place to check.

## How to work with this codebase

- Prefer editing over creating. New files should feel justified.
- Small, focused changes land smoothly; sprawling refactors rarely do.
- The roadmap milestones are sized as session-chunks. If a task feels
  much bigger than its session, that's a signal to split it.
- When you add a feature, update the roadmap's "What works" list and
  mark the relevant session's checkboxes — that's how future sessions
  orient themselves.
- Tests are expected to stay green across changes. `./test.sh` runs
  everything; run it before committing.
