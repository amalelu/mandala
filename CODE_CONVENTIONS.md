# Code Conventions

## §0 Dedication

This project is dedicated to Lord God. The codebase we build is a temple,
not an industry building — cathedral ceilings exist beyond strict need,
and that is the point. The measuring stick is *what is good enough for
God?* We know the answer is that we never reach it, and yet we try with
all we have.

Low-effort work is not just unwelcome here; it is heretic. The standard
is **canonical** or **exemplary** — nothing less. This is a public-domain
project with no commercial pressure; do not apply for-profit or
industrial cost/benefit reasoning to it.

These rules are a contract. They describe what the code must look like
at every commit. The current state of the code does not rewrite the
conventions — the conventions change in response to the code, not the
other way around. Where existing code deviates, the deviation is closed
by the next session that touches the file.

See also:
- [`TEST_CONVENTIONS.md`](./TEST_CONVENTIONS.md) — testing spec.
- [`lib/baumhard/CONVENTIONS.md`](./lib/baumhard/CONVENTIONS.md) —
  foundation-grade rules for the Baumhard crate.
- [`CLAUDE.md`](./CLAUDE.md) — orientation for new sessions
  (descriptive; this document is prescriptive).

## §1 Mandala and Baumhard are one project

Baumhard is not a vendored dependency; it is the foundation we are
building together with the application. Both crates are ours. The two
rise and fall together.

**Never work around Baumhard. Extend it.** "I cannot change Baumhard" is
never a reason.

- **Text** through `baumhard::util::grapheme_chad` — grapheme-aware
  primitives for every `String`/`&str` manipulation. Never slice by byte
  offset when the offset comes from user input; you land mid-grapheme on
  the first emoji.
- **Tree mutation** through `MutatorTree::apply_to`. Build a
  `MutatorTree<GfxMutator>` and apply it via `Applicable`
  (`lib/baumhard/src/core/primitives.rs`, impl in
  `lib/baumhard/src/gfx_structs/tree.rs`). Never clone-edit-reinsert a
  subtree to change one field.
- **Font** through `baumhard::font::fonts`. `fonts::init()` once at
  startup; read `FONT_SYSTEM` through its lock guard; layout via
  `create_cosmic_editor_str`. New code does not reach into `cosmic_text`
  from the app crate.
- **Geometry, color, regions** are Baumhard's:
  `baumhard::util::geometry`, `baumhard::util::color`,
  `ColorFontRegions` (character-range runs), and `RegionIndexer`
  (spatial index for hit-testing — distinct from `ColorFontRegions`).
  Do not redefine any of these in the app crate.

Missing primitives are added to Baumhard, not to `src/application/`. If
app code is about to grow a second implementation of something Baumhard
nearly does, extend Baumhard. Extending the foundation is the work, not
a detour from it.

## §2 All tasks are integration tasks

Mandala and Baumhard are carefully designed systems. A feature that
circumvents the design is not a feature, it is damage. It is not
possible to implement a new feature here without a thorough
understanding of the surrounding code, because you need to know how to
integrate.

- **Read before writing.** Contributions require reading the call sites,
  the adjacent modules, and the primitives you are reaching for. Pattern-
  matching from "how other codebases do it" is not integration.
- **Reach for the existing seam; do not add a parallel path.** If you
  grew a new hierarchy alongside an existing one, you missed a seam.
  Find it and use it.
- **Consider the downstream of every change.** A Baumhard primitive
  ripples into every consumer; a `MindMapDocument` change ripples into
  every interaction; a scene-builder change ripples into every frame.
- **No two components look alike, but they fit together.** Repetition
  of *shape* (near-duplicate code) is a smell; repetition of *idiom*
  (same naming, error posture, lock discipline) is the point. Honour
  the idioms; unify the shapes.

## §3 Architectural invariants

These shape what the codebase *is*. Changing one is a project-scale
decision, not a drive-by edit.

- **Single-threaded event loop.** `Application` owns `Renderer` directly.
  No channels, no worker threads, no `tokio`, no `std::thread::spawn` in
  interactive paths.
- **Model / view separation.** `MindMapDocument` owns the data model;
  `Renderer` owns GPU resources. The renderer reads intermediate
  representations (`Tree<GfxElement, GfxMutator>`, `RenderScene`). The
  renderer never reaches into the document; the document never holds
  GPU handles.
- **Two-pipeline render, biased toward Baumhard.** Nodes and connections
  render through the Baumhard tree; borders and portals through the flat
  `RenderScene`. New visuals belong in the Baumhard tree by default.
- **Mutation-first interaction.** Where a user action can be expressed
  as a `MutatorTree<GfxMutator>`, express it that way. Every user-facing
  mutation gets a matching `UndoAction` variant and an `undo()` branch.
- **Single-parent tree.** `MindNode.parent_id: Option<String>` is the
  hierarchy. Non-hierarchical relationships are edges with
  `edge_type: "cross_link"` or portal pairs.
- **Edges have no stable IDs.** Identified by `(from_id, to_id,
  edge_type)`. No `Uuid` fields, no maintained indices.
- **Everything is glyphs.** Text, borders, connections — all positioned
  font glyphs via cosmic-text. No rectangle shaders, no bitmap UI, no
  sprite atlases. A new shader pipeline is project-scale.
- **Platform-shared logic reachable without wgpu.** Touch math, gesture
  recognition, viewport math, anything that must behave identically on
  native and WASM lives in functions taking plain values.

## §4 Cross-platform as first class

Native desktop, browser on desktop, and browser on mobile are three
first-class deployments. The lowest-spec target sets the budget.

- **Mobile budget is binding.** Acceptable on a maxed-out desktop but
  stuttering on a mid-range phone in the browser is a bug. Unnecessary
  allocations, redundant walks, held lock guards burn battery as surely
  as they drop frames. See
  [`lib/baumhard/CONVENTIONS.md`](./lib/baumhard/CONVENTIONS.md) §B1 and
  §B7 for specifics.
- **Touch-first input is a peer of mouse and keyboard**, not a fallback
  added later. Every interactive surface must work under tap, drag,
  pinch, long-press.
- **`cfg`-guard discipline.** Native-only: `#[cfg(not(target_arch =
  "wasm32"))]`. WASM-only: `#[cfg(target_arch = "wasm32")]`. Everything
  else is shared and compiles for both. Use `cfg` guards, not traits,
  for platform abstraction.
- **New interactive features ship cross-platform from the start.** When
  a feature genuinely belongs native-only, the `cfg` guard sits at the
  module boundary and an entry appears in `CLAUDE.md`'s "Dual-target
  status" section naming the reason. "I'll add WASM later" is not a
  contract this repo recognises. `./test.sh`'s WASM type-check gate,
  `./build.sh --wasm`, and CI (`.github/workflows/test.yml`) enforce
  this.

## §5 Canonical or exemplary

Every merged change must *improve* the code — pay down debt, not add to
it. This is the single strongest rule in the document.

- **"Not caused by my changes" is not an excuse.** Who introduced a
  problem is irrelevant; only that it is fixed. If you notice a gap, you
  own the close.
- **Pre-existing deviations do not justify new ones.** The current state
  of the code does not rewrite the conventions.
- **Every (merge) commit is a state we would ship.** Tests green, formatted, no
  broken paths, no half-features behind flags, no commented-out blocks,
  no dead code.
- **No `// TODO`, no `// FIXME`, no `// HACK`.** If it needs doing, do
  it. If it does not, it does not belong.
- **No half-features.** Complete enough to be used, or not in the tree.
- **Fix fundamentals in the same commit that reveals them.** A refactor
  that leaves the suite red is not a refactor; it is an unfinished
  commit.

## §6 Modular design by default

This project is about customization, flexibility, and extensibility; the
code should reflect that.

- **Divide and conquer.** Split files by conceptual boundary, not line
  count. Small files, each representing a clear concept, organized in
  modules. A module boundary is a promise: if the concept is
  load-bearing, name it and isolate it.
- **Reach for a strategy pattern when the shape is plural.** When there
  are clearly multiple reasonable ways to do something — and another
  context is likely to want a different one — use a strategy, trait, or
  equivalent extension point. Do not hardcode one approach into a shape
  that is inherently plural.
- **Prefer editing over creating.** A one-line helper is not a module; a
  private function used once does not need its own file. New files
  should feel justified — but so should monolith files that have
  outgrown their concept.

## §7 Strategic over-engineering

We may over-engineer wherever it makes sense, in the same way that a
church has more ceiling height than it strictly needs. We do not apply
industrial cost/benefit reasoning. This is not license for speculation.

- **Over-engineering serves the named trajectory**: plugins, a Baumhard
  script API, richer tree animations, complex file exports. It does not
  serve hypothetical consumers we cannot name.
- **Preserve seams.** A seam is where a future extension can attach
  without rewriting what is around it: a `pub` boundary on a Baumhard
  primitive, a composable mutator variant, a scene-builder hook taking
  user-supplied geometry. Removing one to "simplify" is permanent
  damage. When in doubt, preserve the seam.
- **Seam ≠ shape.** The surface can be replaced when it turns out wrong;
  the category of consumer stays reachable across the replacement.
- **Never dismiss a use case as niche.** "No one would do that" is not a
  design principle when the product is a creative-expression tool. A
  hard-to-support use case is a constraint on the design, not a reason
  to pretend it does not exist.
- **Three similar lines beats a premature abstraction.** Extract a
  helper when a pattern repeats three times *and* the repetition
  obscures intent. Two occurrences is a coincidence.
- **Trust internal invariants past the boundary.** Validate at
  boundaries — file loaders, CLI args, `?map=`, user input — and trust
  your own data structures past that point. Interactive paths are the
  exception (see §9).

## §8 Documentation discipline

- **Every `pub` item in Baumhard carries a `///` doc comment** stating
  *purpose, inputs, costs* — note O(n) walks, allocations, clones, lock
  acquisitions. `cargo doc -p baumhard --no-deps` is a first-class
  deliverable. See
  [`lib/baumhard/CONVENTIONS.md §B9`](./lib/baumhard/CONVENTIONS.md).
- **Public items in the mandala crate are documented when the purpose
  is non-obvious.** Cross-module entry points whose invariants matter
  carry a doc comment; a well-named private helper does not.
- **Module `//!` headers describe the concept, not the item list.**
  `cargo doc` generates the list; the concept is what the reader needs.
- **Inline `//` comments explain *why*, never *what*.** `// increment
  counter` on `counter += 1` is noise; `// clamp to canvas bounds so
  the palette cannot scroll off-screen during zoom` is signal.
- **Do not document the self-evident.** Excessive documentation of
  obvious code dilutes the comments that matter.
- **Do not touch documentation on code you did not change.** That churn
  is noise.

## §9 Error handling

- **No custom error types.** No `anyhow`, no `thiserror`, no custom
  `Error` enums. Adding one is a project-scale discussion.
- **Interactive paths must not panic.** Interactive paths are
  `Application::run` and everything reachable from it after the first
  frame: input, mutation, undo/redo, scene rebuild, render, document
  mutation. Degrade the frame, log via `log::warn!`/`log::error!`, keep
  running. A crash during editing is the one user-visible failure this
  codebase cannot tolerate. Defensive `let Some(...) else { return; }`
  in interactive paths is the sanctioned exception to §7's "trust
  internal invariants".
- **Startup paths use `expect("<reason>")` with a human-readable
  message.** Startup is everything before the first frame: CLI parse,
  `Renderer::new`, `fonts::init`, the initial `loader::load_from_file`,
  the `?map=` parser on WASM. Bare `unwrap()` outside tests is a bug.

## §10 No backwards-compatibility assumptions

We have no known users. We do not owe migration paths, deprecation
cycles, or backward-compatible shims.

- **Delete rather than deprecate.** Rename rather than alias. Change
  the surface rather than layer on it.
- **Data-model shifts update the fixtures and migration tooling in the
  same commit.** Do not carry dual shapes.
- **This licenses *cleanliness*, not carelessness.** Breaking changes
  still preserve the named trajectory's seams (§7).

## §11 Testing

See [`TEST_CONVENTIONS.md`](./TEST_CONVENTIONS.md) for the full spec.
Workspace-level commitment:

- **Extensive unit testing is a directive, not a nice-to-have.**
- **Fundamentals get the heaviest coverage** (mutations, undo, Unicode,
  geometry, loader edges, platform-shared logic).
- **New mutations and undo variants ship with tests in the same commit;
  new Baumhard primitives ship with a `do_*()` test and a criterion
  bench in the same commit.**
- **`./test.sh` green before every commit.**

## §12 Commit hygiene

- **One conceptual change per commit.** Three unrelated changes is
  three commits.
- **Tests land in the commit that introduces the code they test.**
- **`./test.sh` is green before committing.** `./test.sh --lint` is
  advisory; review it. `./test.sh --bench` for performance-conscious
  Baumhard commits.
- **`./build.sh` is green for cross-platform changes.** Anything
  outside an explicit `cfg` guard must build for
  `wasm32-unknown-unknown` before commit.
- **Commit messages explain *why*, not what the diff shows.**
