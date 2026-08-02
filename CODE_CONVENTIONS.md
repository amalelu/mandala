# Code Conventions

## §0 Dedication

This project is dedicated to the highest - in whatever form that takes
for you personally. The codebase we build is a temple,
not an industry building. Cathedral ceilings exist beyond strict need,
and that is the point. The measuring stick is *what is good enough for
the divine?* We know the answer is that we never reach it, and yet we try with
all we have.

Low-effort work is not desired here. The standard
is **canonical** or **exemplary** — nothing less. This is an open source
project with no commercial pressure; do not apply for-profit or
industrial cost/benefit reasoning to it.

These rules are a contract. They describe the expected quality of code that will be merged to main. The current state of the code does not rewrite the
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

**Never work around Baumhard. Extend it.** "I shouldn't touch Baumha-" yes you should.

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
  startup; acquire `FONT_SYSTEM` through `acquire_font_system_write`;
  resolve families through `app_font_by_family` /
  `loaded_families_iter`. Region-styled text reaches cosmic-text only
  through `baumhard::font::attrs` (`attrs_list_from_regions`,
  `RegionFamilies`, `rich_text_spans_from_regions`). New code does
  not reach into `cosmic_text` from the app crate, with the renderer
  (`src/application/renderer/`) as the sole exception — speed is king
  there, but even renderer code uses the baumhard bridges where one
  exists.
- **Geometry, color, regions** are Baumhard's:
  `baumhard::util::geometry`, `baumhard::util::color`,
  `ColorFontRegions` (grapheme-cluster-range runs — see
  `lib/baumhard/CONVENTIONS.md §B1`), and `RegionIndexer`
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
  (same naming, error posture, lock discipline) is the point. Honor
  the idioms; unify the shapes.

## §3 Architectural invariants

These shape what the codebase *is*. Changing one is a project-scale
decision, not a drive-by edit.

- **Single-threaded event loop.** `Application` owns `Renderer` directly.
  No channels, no worker threads, no `tokio`, no `std::thread::spawn` in
  interactive paths.

  **Sanctioned boundary threads.** Two narrow exceptions exist, both
  shaped so that app state (`InitState`, `MindMapDocument`,
  `Renderer`) still has exactly one thread:
  - The native `FreezeWatchdog`
    (`src/application/app/freeze_watchdog.rs`) — reads a liveness
    atomic, never touches app state.
  - The IPC boundary threads (design: `work_plans/LLM_IPC.md` §D2,
    protocol: `format/ipc.md`; lands with IPC-02). When `--ipc` is
    active: a persistent acceptor thread (always `accept()`ing, so a
    second client is rejected immediately with a non-blocking write),
    plus a per-controller reader thread (blocking read → parse →
    enqueue → wake the loop via a winit `EventLoopProxy` user event)
    and a per-controller outbound thread (dequeue → serialize →
    blocking write) that are spawned and torn down together per
    connection. Each connection carries a monotonic generation stamp
    on its requests/replies/events/pending-waits, and the reader's
    disconnect sends a user event so the main thread cancels that
    generation's waits and subscriptions — nothing crosses to a
    successor controller. All see only protocol value types; every
    IPC command executes on the main thread in the `user_event` arm
    with the same access as any input handler, and a command that
    changes pixels requests a redraw just as the winit input handlers
    do. The `std::sync::mpsc` queues at this boundary carry protocol
    values only — they are not a license for channels between app
    components — and blocking IPC I/O on the main thread stays
    forbidden: a stalled client must never trip
    the `FreezeWatchdog`; the bounded outbound queue tears the
    connection down instead.
- **Model / view separation.** `MindMapDocument` owns the data model;
  `Renderer` owns GPU resources. The renderer reads intermediate
  representation (`Tree<GfxElement, GfxMutator>`, one per canvas
  role). The renderer never reaches into the document; the document
  never holds GPU handles.
- **Render document content through the Baumhard tree.** Every
  visual that comes from the model — nodes, connections, borders,
  portals, labels, section frames, handles — converges on
  `Tree<GfxElement, GfxMutator>` and is shaped by
  `src/application/renderer/tree_walker.rs`. There is exactly one
  pipeline for them: a data pass plus a tree projection under
  `lib/baumhard/src/mindmap/tree_builder/`, driven by `CanvasFrame`
  in `src/application/app/scene_rebuild.rs`. Model-derived content
  reaching the GPU any other way is a second pipeline; do not add
  one.

  **Transient chrome is the carve-out**, and the list is small and
  closed: the rubber-band selection rectangle
  (`renderer/selection_overlay.rs`), the console overlay
  (`renderer/console_pass.rs`), the glyph-wheel color picker
  (`renderer/color_picker.rs`), and the FPS / mode-status overlays.
  The discriminator is *what the visual is a projection of*, not
  whether it ever reads the model: each of these belongs to an
  interaction rather than to document content, so it has no model
  element to be the projection *of* — even where it quotes one (the
  mode-status line reports the active `MindNode`'s id and section
  count, but the overlay is a property of the mode, not of the
  node). A new visual qualifies only on that test; if it is a
  rendering of document content, it belongs in the tree.
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
- **Single dispatch funnel.** Every user-driven application-level
  behavior is reified as an `Action` and dispatched via
  `dispatch_action` (`src/application/app/dispatch/native.rs`). New gestures,
  new console verbs that mutate state, new navigation keys: variant,
  default binding, dispatch arm — in that order. No second copy of
  Action body logic in a handler. Mouse handlers synthesize a gesture
  name through `gesture_key_name(MouseGesture::*)` and feed it through
  `action_for_gesture` (mouse — modifier-fallback) or
  `action_for_context` (keyboard — exact). See CONCEPTS.md §5 "Action
  dispatch".

  **Carve-outs that legitimately stay outside the funnel:**
  - **Modal steals.** When console / color-picker / label-edit /
    portal-text-edit / text-edit modals are open, their handlers
    consume the event before the funnel runs. Modals own the literal
    `winit::Key` payload (character insertion, IME sequences) which
    `Action` discards.
  - **Pre-funnel state-machine bookkeeping.** Selection updates on
    single-click, drag-state cleanup on release, double-click time +
    distance + hit detection, and the `last_click` tracker run
    before the funnel. They're not user-named effects; they're
    machinery the funnel rests on.
  - **Per-frame continuous-gesture state.** A drag's per-cursor-move
    delta (e.g. `RenderDecree::CameraPan(dx, dy)` in `event_cursor_moved`)
    legitimately stays inline. The funnel covers the discrete entry
    + exit (the press → `Action::PanCanvas`); the per-frame body
    is not a discrete-action concern.

  Anything else — a new gesture or a new console-verb behavior that
  mutates document state or changes view state — must go through the
  funnel.

  **Macro-tier privilege gates are mandatory.** Macros loaded from
  `~/.config/mandala/macros.json` share
  trust posture with `keybinds.json` — the user owns the file. The
  dispatcher gates two things on `SourceTier`:
  - `MacroStep::ConsoleLine` runs an arbitrary console verb, so it's
    User-tier-only via `SourceTier::allows_console_line`.
  - Destructive / I/O / clipboard `Action` variants
    (`SaveDocument`, `DeleteSelection`, `Cut`, `Paste`, `Copy`,
    `OrphanSelection`, `CreateOrphanNode`, `CreateOrphanNodeAndEdit`,
    `NewDocument`) are User-tier-only via
    `SourceTier::allows_action`.
  Privilege rejections **fail-closed** — the rest of the macro
  aborts so a `[DeleteSelection, ConsoleLine(rejected),
  SaveDocument]` pattern can't sneak its outer steps past the gate.
  All four tiers load today, so the gates are fully active; they
  MUST hold as new dispatch surfaces are added (IPC included —
  `format/ipc.md` §"Trust model").

  `DocumentAction` (carried by `MacroStep::CustomMutation`) is
  `#[non_exhaustive]`. Today every variant is a pure in-memory
  canvas-theme write, safe to expose. **Any new variant that
  performs file I/O, network access, arbitrary content load, or
  cross-process side effects MUST add a parallel gate at the
  `dispatch_macro` site** (look up the existing `allows_*` pattern
  on `SourceTier` and extend it).

  Source-tier assignment is loader-pinned. Each loader call site
  hardcodes the tier; nothing in the on-disk format can affect it.
  Future loaders MUST keep this invariant — only `include_str!` /
  `include_bytes!` content can be tagged `App`; user-modifiable
  paths (XDG data dirs etc.) tag as `User` or below.

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
  contract this repo recognizes. What enforces it: `./test.sh`'s WASM
  gate, which is `cargo check --target wasm32-unknown-unknown
  --workspace` and is the check to reach for while iterating;
  `./build.sh`, which always builds both legs in full; and CI
  (`.github/workflows/test.yml`), which runs `./test.sh`.

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
- **Avoid duplicating logic.** Identical logic copy / pasted throughout the codebase
  becomes a nightmare to maintain. If a function is needed in two or more places, the answer
  is never to copy it, but to use a single function called in two or more places.
- **We do drive-by refactors and fixes.** There are always a million excuses as to why cleaning up something should be postponed. But the quality of our codebase is priority #1, so whenever something not-ideal is spotted we ADDRESS it. It doesn't matter if we get mixed pull requests.
- **Always explicitly look for opportunities to improve the quality of existing code.**
- **Never accept "good enough". No one has asked you for "good enough".** No deferring the "hard parts" until later while shipping a "good enough" now. Do it.

## §6 Modular design by default

This project is about customization, flexibility, and extensibility; the
code should reflect that.

- **Divide and conquer.** Split files by conceptual boundary, not line
  count. Small files, each representing a clear concept, organized in
  modules. A module boundary is a promise: if the concept is
  load-bearing, name it and isolate it. A one-line helper is not a module; a
  private function used once does not need its own file. New files
  should feel justified — but so should monolith files that have
  outgrown their concept.
- **Reach for a strategy pattern when the shape is plural.** When there
  are clearly multiple reasonable ways to do something — and another
  context is likely to want a different one — use a strategy, trait, or
  equivalent extension point. Do not hardcode one approach into a shape
  that is inherently plural.

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
- **`warn!` and `error!` survive into release; `info!`, `debug!` and
  `trace!` do not.** Both crates build `log` with
  `release_max_level_warn`, so the degrade half of "degrade the frame,
  log, keep running" is real in the binaries users actually run —
  `./build.sh`, `./run.sh`, and the WASM bundle all ship release. That
  fixes which macro to reach for: a condition a user or a bug report
  needs to know about is `warn!`/`error!`; developer instrumentation —
  walker traces, per-frame counters, anything you would be unhappy to
  see at 60 Hz — is `debug!`/`trace!`, which cost nothing in release
  because the call is compiled out entirely. A designed, transient
  degrade that the next frame retries (a contended `try_write`, a
  skipped overlay reshape) is instrumentation, not a warning: log it at
  `debug!` so a persistent condition doesn't flood stderr. The
  compile-time cap is what makes the boundary identical on both
  targets (§4); the runtime filters underneath it differ and are set
  in one place, `util::log::init` — native `env_logger` defaults to
  `warn` (`RUST_LOG` overrides; `RUST_LOG=` set-but-empty counts as
  unset), WASM `console_log` sits at `Info`, looser than native but
  invisible in release because the cap removes `info!` before the
  filter ever sees it. Every binary the workspace ships calls
  `util::log::init` — including `maptool`, which exercises the same
  loader degrade paths.
- **One log-message prefix idiom: `"<area>: message"`.** The area is the
  subsystem, not the enclosing function — `"macros: ..."`,
  `"font::attrs: ..."`, `"keybinds: ..."`, `"console history: ..."`.
  Function names go stale under refactor and tell a user nothing; the
  area is what they can name in a bug report. Bare messages with no
  prefix are not acceptable. Normalize prefixes in files you are
  already editing rather than in a repository-wide sweep.

## §10 No backwards-compatibility assumptions until V1

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
  geometry, loader edges, platform-shared logic). Don't waste tests on 
  "locking in" trivial stuff. 
- **New mutations and undo variants ship with tests in the same commit;
  new Baumhard primitives ship with a `do_*()` test and a criterion
  bench in the same commit.**
- **`./test.sh` green before every commit.**

## §12 Commit hygiene

- **Tests land in the commit that introduces the code they test.**
- **`./test.sh` is green before committing.** It also type-checks the
  benchmark targets and the wasm32 leg, so neither can rot between
  merges. `./test.sh --lint` is advisory; review it.
- **`./build.sh` is green for cross-platform changes.** Anything
  outside an explicit `cfg` guard must build for
  `wasm32-unknown-unknown` before commit.
- **Commit messages explain *why*, not what the diff shows.**
- **Benchmarks are for maintainers.** `AGENTS.md` forbids automated
  agents from running `cargo bench`, `./bench.sh` or
  `./test.sh --bench`, and forbids performance claims made without the
  main-against-main control row
  [`lib/baumhard/CONVENTIONS.md §B7`](./lib/baumhard/CONVENTIONS.md)
  requires. Changing benchmark code is still expected — §B3 wants a
  bench entry alongside a new primitive — and `./test.sh` proves those
  targets compile.

## §13 Cargo manifests

Mechanical repo rules about where a line goes in a `Cargo.toml`.
Unlike §3, obeying these *is* a drive-by edit — it is what you do
while adding a dependency, not a decision to weigh.

- **One *version string* per dependency, written once.** A crate that two or
  more workspace members need is declared in the root manifest's
  `[workspace.dependencies]`, and each member writes
  `dep.workspace = true` — with `features` beside it when that member
  needs more, since features are additive on top of the shared entry.
  A crate only one member uses stays in that member's manifest. The
  reason this is a rule and not a preference: cargo raises no
  objection when two members name the same crate at different
  versions, it simply builds both, and the symptom is two mutually
  incompatible copies of the same types. `strum` sat at 0.27 and 0.28
  simultaneously until it was unified by hand.
  `baumhard::util::manifests` reads the real manifests and checks all
  three clauses, in every spelling in use here — inline,
  `[dependencies.<name>]` sub-table, `dep.workspace = true`, wrapped
  across lines, renamed via `package =`, with or without a trailing
  comment. The spellings cargo accepts that nobody here writes stop
  the run instead: a shape it cannot read is refused by name rather
  than dropping out of the checked set, which is the property that
  makes the check worth citing. The heading says *version string* on
  purpose — a declaration that names no version is outside the first
  two clauses, so `path` and `git` dependencies are exempt and two
  members pinning one crate to two git revisions would not be
  reported. Nothing here uses a `git` dependency; the first one to
  arrive owes that gap a decision.
- **A dependency that exists to turn a feature on says so.** Cargo
  unions features across the workspace, so a manifest entry with no
  call site can still be load-bearing — the root manifest's
  `getrandom` is the live example. Comment it, or the next
  dead-dependency sweep deletes it.
- **Say what a per-target table buys.** A dependency moved under
  `[target.'cfg(...)'.dependencies]` changes the feature union on the
  *other* target too. Note the consequence where the move is made; see
  `env_logger` in `lib/baumhard/Cargo.toml`.
