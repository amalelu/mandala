# Code Conventions

## §0 What this document is

Mandala is a piece of art first and an engineering project second. It
exists for creative expression — for people drawing maps of ideas,
animating thoughts, exporting documents that look the way they think.
The codebase that builds it should feel the same way: an untouched rain
forest where every component has its place, no two look alike, and the
whole stands in balance. Symbiosis is the design ideal.

This document is the engineering scaffold that lets that art exist. The
rules below are a contract, not a suggestion. They are not aspirational
and they are not a direction of travel — they describe what the code in
this repository must look like at the moment of every commit. Where the
code already deviates, the deviation is to be closed by the next session
that touches the file, not preserved.

See also:
- [`TEST_CONVENTIONS.md`](./TEST_CONVENTIONS.md) — the testing spec.
- [`lib/baumhard/CONVENTIONS.md`](./lib/baumhard/CONVENTIONS.md) — the
  foundation-grade rules that apply inside the Baumhard crate.
- [`CLAUDE.md`](./CLAUDE.md) — orientation notes for new sessions.
  `CLAUDE.md` is descriptive ("how things work"); this document is
  prescriptive ("how to change them").

## §1 Baumhard is the foundation

Baumhard is the text-manipulation, layout, tree-mutation, and rendering
substrate this application is built around. It is not a vendored
dependency we pull from crates.io and accommodate. It is **our work** —
the foundation we are developing together with the application. The two
crates rise and fall together.

The single most important rule in this repository: **never work around
Baumhard. Extend it.**

- **Application code consumes Baumhard's primitives.** Do not
  reimplement what it already provides.
  - **Text** through `baumhard::util::grapheme_chad`:
    `replace_graphemes_until_newline`, `split_off_graphemes`,
    `count_grapheme_clusters`, `find_nth_line_grapheme_range`,
    `delete_back_unicode`, `delete_front_unicode`
    (`lib/baumhard/src/util/grapheme_chad.rs`). Never index or slice a
    `String` by byte offset when the offset comes from user input — you
    will land mid-grapheme on the first emoji.
  - **Tree mutation** through `MutatorTree::apply_to`. Build a
    `MutatorTree<GfxMutator>` describing the delta and apply it to the
    target `Tree` via the `Applicable` trait
    (`lib/baumhard/src/core/primitives.rs`; the impl for the
    `GfxElement` tree lives in `lib/baumhard/src/gfx_structs/tree.rs`).
    Never clone a subtree, edit it, and reinsert it to change one field.
  - **Font** through `baumhard::font::fonts`. Call `fonts::init()` once
    at app startup; read the global `FONT_SYSTEM` through its lock
    guard. Layout goes through `create_cosmic_editor_str` — new code
    does not reach into `cosmic_text` directly from the application
    crate. Existing direct uses in `src/application/renderer.rs`,
    `document.rs`, and `console/visuals.rs` predate this rule and are
    tracked in `ROADMAP.md`'s "What needs work" list as gaps to close
    on the way past, never extended.
  - **Geometry, color, regions** are Baumhard's:
    `baumhard::util::geometry::almost_equal_vec2`,
    `baumhard::util::color::*`, and
    `baumhard::gfx_structs::util::regions::RegionIndexer`. Do not
    redefine them in the app crate.

- **Missing primitives are added to Baumhard, not to `src/application/`.**
  If application code is about to grow a second implementation of
  something Baumhard nearly does, extend Baumhard instead. Because
  Baumhard is ours, "I cannot change the library" is never the reason —
  the library is ours to change. Extending it is part of the work, not
  a detour from it.

## §2 Performance across the targets

Mandala is single-threaded and aims to run on as many devices as
possible. The lowest-spec target sets the budget. If something is
acceptable on a maxed-out desktop but stutters on a mid-range phone in
the browser, the budget is the phone's, and the stutter is a bug.

- **Targets are equal citizens.** Three deployments are first-class:
  - Desktop native (`cargo run`, the primary developer loop).
  - Browser on desktop (WASM, `trunk serve`).
  - Mobile in the browser (WASM, the same build, opened on a phone).
- **Touch-first input is a peer of mouse and keyboard**, not a fallback
  added after the desktop UX is finished. Every interactive surface
  must work under tap, drag, pinch, and long-press; the WASM input
  path's current lag behind native is a tracked gap, not a model to
  emulate.
- **Mobile budget is the binding budget.** Hot-path work has to be
  cheap enough for a battery-powered, thermally-constrained device
  with a small viewport. Unnecessary allocations, redundant tree
  walks, and held lock guards burn battery as surely as they drop
  frames. The specifics live in
  [`lib/baumhard/CONVENTIONS.md`](./lib/baumhard/CONVENTIONS.md) §B1
  and §B7; this section names the constraint at the workspace level so
  app-crate code never assumes the desktop alone defines acceptable.
- **`cfg`-guard discipline is how cross-platform actually happens.**
  Native-only code lives behind `#[cfg(not(target_arch = "wasm32"))]`;
  WASM-only code behind `#[cfg(target_arch = "wasm32")]`; everything
  else is shared and must compile for both. Platform abstraction uses
  `cfg` guards, not traits.

## §3 Architectural invariants

These shape what the codebase *is*. Changing one is a roadmap-scale
decision that belongs in `ROADMAP.md` before it belongs in code.

- **Single-threaded event loop.** The `Application` struct owns the
  `Renderer` directly. No channels, no worker threads, no `tokio`, no
  `std::thread::spawn` in interactive paths. Concurrency is a
  roadmap-scale proposal.
- **Model / view separation.** `MindMapDocument`
  (`src/application/document.rs`) owns the data model (`MindMap`,
  selection, undo stack). `Renderer` (`src/application/renderer.rs`)
  owns GPU resources. Rendering reads intermediate representations the
  document builds (`Tree<GfxElement, GfxMutator>` for nodes,
  `RenderScene` for edges / borders / portals). The renderer never
  reaches into the document's data model directly; the document never
  holds GPU handles.
- **Two-pipeline render, biased toward Baumhard.** Nodes and
  connections render through the Baumhard tree; borders and portals
  render (for now) through the flat `RenderScene`. New visuals belong
  in the Baumhard tree by default — the flat scene is the exception
  that needs a reason.
- **Mutation-first interaction.** Where a user action can be expressed
  as a `MutatorTree<GfxMutator>` applied to the node tree, express it
  that way. Where it cannot (edges, borders, overlays), reach for the
  scene builder or a targeted document method. Every user-facing
  mutation gets a matching `UndoAction` variant in `document.rs` and a
  matching branch in `undo()`.
- **Single-parent tree.** `MindNode.parent_id: Option<String>` is the
  hierarchy. Non-hierarchical relationships are arbitrary edges with
  `edge_type: "cross_link"` or portal pairs. Do not introduce
  multi-parent shapes.
- **Edges have no stable IDs.** Edges are identified by the triple
  `(from_id, to_id, edge_type)`. Mirror this everywhere — no `Uuid`
  fields, no maintained indices.
- **Everything is glyphs.** Text, borders, and connections all render
  as positioned font glyphs via cosmic-text. There are no rectangle
  shaders, no bitmap UI, no sprite atlases. Express new visuals as
  characters first; a new shader pipeline is roadmap-scale.
- **Platform-shared logic must be reachable without a wgpu instance.**
  Touch-input math, gesture recognition, viewport math, and any logic
  that needs to behave identically on native and WASM lives in
  functions that take their inputs as plain values. This is what makes
  cross-platform testable on native (see
  [`TEST_CONVENTIONS.md`](./TEST_CONVENTIONS.md) §T9) and what keeps
  the WASM input path catchable without a browser.

## §4 The quality bar

We want the highest quality code. No compromise. No technical debt.

- **Every commit is a state we would ship.** Tests green, code formatted,
  no broken paths, no half-finished features behind a feature flag.
  "I will fix it in the next commit" is not a contract this repository
  recognises — the next commit is for the next thing.
- **No new technical debt.** Existing gaps (legacy `unsafe` in
  `lib/baumhard/src/util/simd.rs`, `unwrap`/`expect` in interactive
  paths, raw `cosmic_text` use in the app crate, partial doc coverage)
  are tracked in `ROADMAP.md`'s "What needs work" list and closed on
  the way past, never extended. See §7 for the specific posture on
  panics.
- **No deferred `// TODO`, no `// FIXME`, no `// HACK`.** If it needs
  doing, do it; if it does not need doing now, it does not belong in
  the codebase. Roadmap items live in the roadmap.
- **No half-features.** A feature is either complete enough to be used
  by the user it was built for, or it is not in the tree. Stubs that
  "almost work" rot.
- **If you broke a fundamental, fix it in the same commit.** A
  refactor that leaves the test suite red is not a refactor; it is an
  unfinished commit.

## §5 The bigger picture (symbiosis)

A change to one component is a change to the rain forest. Tunnel vision
on the immediate task — without considering the components downstream
of it, the components alongside it, and the application as a whole — is
how the forest fragments into an arboretum.

- **Operational heuristic for new components.** When you add a
  component, name the two existing components it most resembles and
  articulate why it is not one of them. If you cannot articulate the
  difference, you are about to add a duplicate; merge instead. If you
  cannot find two resemblances, your component might be reaching for
  novelty for its own sake; reconsider whether it fits the forest.
- **No two components look alike, but they fit together.** Repetition
  of *shape* (two components that are 80% the same code) is a smell.
  Repetition of *idiom* (two components that follow the same naming
  patterns, the same error posture, the same lock discipline) is the
  point. Honour the idioms; do not duplicate the shapes.
- **Consider the downstream of every change.** A change to a Baumhard
  primitive ripples into every consumer; a change to `MindMapDocument`
  ripples into every interaction; a change to the scene builder
  ripples into every frame. Read the call sites before changing the
  callee.
- **Resist patterns that fragment.** Parallel hierarchies, near-duplicate
  helpers in different modules, two ways to spell the same operation —
  these are how a codebase loses its balance. When you notice one,
  unify it on the way past.

## §6 Simplicity in service of the trajectory

Simplicity is a means, not an end. The end is a codebase that holds the
named trajectory without bending under the weight of speculation.

The trajectory is named: **plugins, a Baumhard script API, complex
tree animations, complex file exports.** Mandala is for creative
expression — exotic creative use cases are the point of the product,
not edge cases to be dismissed.

- **Preserve seams that serve the named trajectory.** A seam is a place
  where a future extension can attach without rewriting what is around
  it: a `pub` boundary on a Baumhard primitive, a mutator variant that
  composes, a scene-builder hook that accepts user-supplied geometry.
  Removing a seam to "simplify" current code is permanent damage.
  When in doubt, preserve the seam — removing one later is harder than
  not adding a flag now.
- **Do not build configurability for callers outside the trajectory.**
  A flag without a named consumer (or a named planned consumer on the
  trajectory) is dead weight. Add the flag when the consumer arrives,
  not when you imagine it might.
- **Never dismiss a use case as niche.** "No one would actually do
  that" is not a design principle when the product is a creative
  expression tool. The user who wants to embed a recursive fractal of
  cross-linked portals in an exported PDF is exactly who Mandala is
  for. If a use case is hard to support, that is a constraint on the
  design, not a reason to pretend the use case does not exist.
- **Prefer editing over creating.** A one-line helper is not a module;
  a private function used once does not need its own file. New files
  should feel justified.
- **Three similar lines beats a premature abstraction.** Extract a
  helper when a pattern repeats three times *and* the repetition
  obscures intent. Two occurrences is a coincidence.
- **Trust internal invariants past the boundary.** Validate at
  boundaries — file loaders, CLI args, the `?map=` query parameter,
  user input — and trust your own data structures past that point. The
  exception is interactive paths (see §7).
- **Split on the seam between concerns, not on the line count.** A
  function the size of the screen is fine if it does one thing. The
  same function becomes a problem when it mixes parsing, validation,
  and rendering — split on the concern, not on the length.

## §7 Error handling and documentation

The error-handling posture is narrow on purpose. Documentation is how
future sessions inherit the codebase without asking.

- **No custom error types.** No `anyhow`, no `thiserror`, no custom
  `Error` enums. Adding one is a roadmap-scale discussion.
- **Interactive paths must not panic.** Interactive paths are
  `Application::run` and everything reachable from it after the first
  frame: input handling, mutation application, undo/redo, scene
  rebuild, frame render, and document mutation. None of these may
  abort the process. Degrade the frame, log via `log::warn!` or
  `log::error!`, and keep running. A crash during editing is the one
  user-visible failure this codebase cannot tolerate. Defensive `let
  Some(...) else { return; }` checks in interactive paths are the
  sanctioned exception to §6's "trust internal invariants."
- **Startup paths use `expect("<reason>")`.** Startup is everything
  before the first frame: CLI parse, `Renderer::new`, `fonts::init`,
  the initial `loader::load_from_file`, the `?map=` query parser on
  WASM. When the failure is unrecoverable, `expect` with a
  human-readable message is the rule (`expect("Failed to create
  device")`). Bare `unwrap()` outside tests is a bug. The app crate
  contains pre-existing `unwrap` and `expect` calls in interactive
  paths that predate this rule; they are tracked debt to be closed on
  the way past, never extended.
- **Every `pub` item in Baumhard carries a `///` doc comment.** State
  *purpose, inputs, costs* — note an O(n) walk, an allocation, a
  clone, a lock acquisition. `cargo doc -p baumhard --no-deps` is a
  first-class deliverable. Specifics live in
  [`lib/baumhard/CONVENTIONS.md §B9`](./lib/baumhard/CONVENTIONS.md).
- **Public items in the mandala crate are documented when the purpose
  is non-obvious.** Cross-module entry points whose invariants matter
  carry a doc comment; a private helper with a descriptive name does
  not.
- **Module-level `//!` headers** describe the concept the module
  implements, not the list of items in it.
- **Inline `//` comments explain *why*, never *what*.** `// increment
  counter` on `counter += 1` is noise; `// clamp to canvas bounds so
  the palette cannot scroll off-screen during zoom` is signal.
- **Do not touch documentation on code you did not change.** That
  churn is noise.
- **Testing.** See [`TEST_CONVENTIONS.md`](./TEST_CONVENTIONS.md). New
  mutations and undo variants ship with their tests in the same
  commit; new Baumhard primitives ship with a `do_*()` test and a
  criterion bench in the same commit.

## §8 Commit hygiene

- **One conceptual change per commit.** Three unrelated changes is
  three commits.
- **Tests land in the commit that introduces the code they test.**
  Not before, not after.
- **`./test.sh` is green before committing.** `./test.sh --lint` is
  advisory; review its output. `./test.sh --bench` is for
  performance-conscious commits in Baumhard.
- **`./build.sh` is green for cross-platform changes.** Anything
  outside an explicit `cfg` guard — workspace-shared logic, anything
  in Baumhard that is not native-gated — must build for
  `wasm32-unknown-unknown` before commit. Native-only changes inside
  `#[cfg(not(target_arch = "wasm32"))]` blocks are exempt.
- **Commit messages explain *why*, not what the diff shows.** The
  diff shows what changed. The message explains why the change was
  worth making.
