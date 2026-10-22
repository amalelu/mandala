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
  startup; acquire `FONT_SYSTEM` through `acquire_font_system_write`
  (or its `try_` / `_read` siblings — there is no other way in);
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
  - The IPC boundary threads (`src/application/ipc/`; design:
    `work_plans/LLM_IPC.md` §D2, protocol: `format/ipc.md`). Only
    when `--ipc` is passed: a persistent acceptor thread (always
    `accept()`ing, so a second client is rejected immediately with a
    non-blocking write), plus a per-controller reader thread (blocking
    read → parse → enqueue → wake the loop via a winit
    `EventLoopProxy` user event) and a per-controller outbound thread
    (dequeue → serialize → blocking write) that are spawned and torn
    down together per connection. Each connection carries a monotonic
    generation stamp on its requests/replies/events/pending-waits, and
    the reader's disconnect is queued behind that connection's
    requests and wakes the main thread, which retires the generation
    — canceling its waits and subscriptions — so nothing crosses to a
    successor controller. All see only protocol value types; every
    IPC command executes on the main thread, in `about_to_wait` right
    after `drain_inputs` (the `user_event` wake only unparks the
    loop), with the same access as any input handler, and a command that changes pixels requests a
    redraw just as the winit input handlers do. The `std::sync::mpsc`
    queues at this boundary carry protocol values only — they are not
    a license for channels between app components — and blocking IPC
    I/O on the main thread stays forbidden: a stalled client must
    never trip the `FreezeWatchdog`. A full outbound queue pauses that
    controller's execution until its writer makes room, and a write
    that makes no progress for ten seconds tears the connection down.
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
    single-click (and on a touch `Tap`), drag-state cleanup on
    release, double-click time + distance + hit detection, and the
    `last_click` tracker run before the funnel. They're not
    user-named effects; they're machinery the funnel rests on.

    **One thing on that same path is a user-named effect, and it is
    deliberately not an `Action`:** a click or tap that lands on a
    node runs `click_triggers::fire_onclick_triggers`, which applies
    whatever `OnClick` `CustomMutation`s the *map author* bound to
    it, plus their document actions. Those are authored effects by
    definition. They stay outside the funnel because the funnel
    reifies behaviors the *application* names — one `Action` per
    user-facing verb, bindable by key — whereas these are named per
    node in the document and reach the model through the mutation
    pipeline instead.

    **They carry no `SourceTier`, and that is a gap rather than a
    sanction.** An earlier draft of this bullet explained their
    ungated-ness by saying the trust boundary for document content
    is the loader — which contradicts this project's actual stated
    threat model, where a `.mindmap.json` is *untrusted* and a
    "hostile shared mindmap" is named as the adversary
    (`src/application/macros/mod.rs`). The loader validates shape,
    not intent. The correct statement is that `OnClick` triggers
    are outside the *dispatch funnel*, not outside the security
    posture: they are a second route from map content to
    `apply_document_actions` that never passes `dispatch_macro`,
    which is exactly why the gating rule below says *every* apply
    site rather than that one.
  - **Per-frame continuous-gesture state.** A drag's per-cursor-move
    delta (e.g. `RenderDecree::CameraPan(dx, dy)` in `event_cursor_moved`)
    legitimately stays inline. For a *mouse* drag the funnel still
    covers the discrete entry + exit (the press → `Action::PanCanvas`);
    the per-frame body is not a discrete-action concern.

    **A touch pan or pinch has no funnel entry at all**, and that is
    the sanctioned shape rather than an omission. The entry a mouse
    funnels is `Action::PanCanvas`, which does not move the camera —
    it arms `DragState::Panning`, which exists only on native — so
    routing a finger through it would dispatch, return `Unhandled`
    and warn in the browser. There is also nothing left for an entry
    to decide: `TouchGestureRecognizer` is the state machine
    `DragState` is for the mouse, and by the time it emits, the
    gesture is already identified. So the whole of a touch pan /
    pinch is per-frame body, reaching the camera through
    `RenderDecree`s (`dispatch::apply_touch_effect`). A *discrete*
    touch gesture still goes through the funnel — `LongPress`
    resolves through `MouseGesture` and `action_for_gesture` like any
    mouse gesture.

    The line this carve-out draws is therefore "is the effect a
    continuous camera delta?", not "did a funnelled press start it".
    A continuous gesture that *mutates the document* per frame is a
    different question and is not covered here — see the
    `ThrottledDrag` ladder.

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
  cross-process side effects MUST be gated at *every site that
  applies a `DocumentAction`*** (look up the existing `allows_*`
  pattern on `SourceTier` and extend it).

  **"Every site" is the load-bearing phrase, and this paragraph
  used to say "at the `dispatch_macro` site" instead — which is
  wrong in the direction that matters.** `dispatch_macro` is not
  the only route from a `.mindmap.json` to
  `MindMapDocument::apply_document_actions`. There are three, and
  the second is the one a hostile map would take:

  1. `dispatch::apply_keybind_custom_mutation`
     (`cross_dispatch/mod.rs`) — the keybind / macro tier, which
     is the route `dispatch_macro`'s gates cover.
  2. `click_triggers::fire_onclick_triggers` — a node's `OnClick`
     binding, authored **in the map**, fired by a click or a touch
     tap. It carries no `SourceTier` at all, because it is document
     content rather than a dispatch surface. A variant gated only
     at `dispatch_macro` stays fully reachable here.
  3. `console::commands::mutation` — the `mutation` console verb,
     User tier by construction, since the user typed it.

  Nothing turns on this today: both shipped variants
  (`SetThemeVariant`, `SetThemeVariables`) are in-memory theme
  writes and route 2 is harmless for them. The instruction is
  written down correctly now so the first dangerous variant does
  not get gated at one site and shipped as though it were gated.

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
- **Intra-doc links must point at targets that survive `cfg`
  stripping.** Rustdoc compiles with `cfg(test)` off: a doc link from
  live code into a `#[cfg(test)]` module is always broken — the
  target does not exist when rustdoc resolves it — while a broken
  link *inside* a `#[cfg(test)]` module can never fail anything,
  because the module is stripped before doc collection ever sees it.
  The same asymmetry holds per target: a link from cross-platform
  code to a `#[cfg(target_arch = "wasm32")]` item breaks the host
  doc leg, a link to a native-gated item breaks the wasm32 leg, and
  each break is visible to exactly one of `./test.sh --lint`'s two
  doc targets — which is why there are two. Name a cfg-stripped item
  as a plain code-span (backticks, no link): the only form that
  resolves everywhere. The doc gates hold a zero baseline on both
  targets and fail the run (#134); PR #133 established the
  stripped-side half of this rule, the #134 incident the live-side
  half.
- **A module `//!` header whose `mod` item also carries an outer
  `///` writes its intra-doc links crate-absolute.** rustdoc merges
  the two into one doc block, and a *relative* link inside the merged
  block resolves against no module — `[`Foo`]` fails even where `Foo`
  is declared in that very file. Both halves of the pair are the
  house style here (`lib/baumhard/src/mindmap/mod.rs` puts a one-line
  summary on every `pub mod` it declares, and each of those files
  opens with its own `//!`), so the combination is normal and the
  hazard is latent in every one of them. What makes it worth a rule
  rather than a fix-when-it-bites: the error names neither a file nor
  a line, so the only way to locate it is to bisect the doc comments.
  `[`Foo`](crate::path::to::Foo)` resolves.
- **Prose must not name an item the same change deleted, and a
  rename sweep is what catches it — with two blind spots worth
  knowing before you trust one.** A doc that says "which the first
  `insert` overwrites" after `insert` is gone reads as a pointer and
  is a dead end; the same sentence inside a *control instruction* is
  worse, because the next reader runs it. The sweep is: extract every
  `fn` / `struct` / field name the change removed from its own diff,
  grep the tree for each, then triage. It found four such sentences in
  one series. What it does **not** find: (1) removed field names that
  are ordinary English words — `color`, `font`, `position` — where the
  prose is in a file the change never touched, so no qualified
  spelling and no file-locality narrows the grep; (2) a sentence whose
  every identifier still exists but whose *claim* went false, which
  needs reading rather than matching. Both were real misses. A
  deliberate mention of a removed item is fine and sometimes required
  — naming the input that made a regression test fail, or the shape a
  rewrite replaced — but it carries its own disclosure that the thing
  is gone, so a reader is never sent looking.
- **A figure derived from a recorded constant belongs beside the
  constant, not copied into prose.** Name the constant and state the
  relation; a number restated in a doc comment goes stale silently
  while still reading as authority.
  `MEASURED_WORST_ESCAPE_ULPS` in
  `lib/baumhard/src/mindmap/connection/tests.rs` carries the canonical
  statement of this and the incident that produced it. It has since
  been broken once more, in prose about that same constant, which is
  why it is repeated here where the doc rules live.
- **A comment may assert what the standard says and what this
  repository does; a claim about what a *named engine* does at
  runtime needs either a device or a hedge.** The proved-vs-observed
  discipline, applied to prose. "`touch-action: none` is what the
  spec says a canvas needs" and "nothing in this repository sets it"
  are both checkable from a checkout. "Safari ignores
  `user-scalable=no`" is not — no browser runs in CI, so the claim
  can only be inherited from memory, and inherited browser trivia
  goes stale silently and is then quoted as though it were tested.
  Write the checkable half, and where behavior genuinely varies say
  it varies and name who owns confirming it. The rule earned its
  place: `web/index.html` shipped a per-engine claim that a reviewer
  said was backwards, and neither the author nor the reviewer could
  settle it from a checkout.
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
  the winit event loop and its window, the `--ipc` socket when the
  flag is passed, the renderer bootstrap, `fonts::init`, and — in the
  browser — hanging a canvas off the page and reading `?map=` from its
  URL. What can fail says so where it happens, in five groups. **The
  window:** the event loop — `EventLoop::with_user_event` on native,
  whose loop carries the IPC wake, and `EventLoop::new` in the
  browser — then `create_window`, and `run_app`, whose `Err` is the
  loop itself giving out. The browser's loop and window are created
  in `Application::new` rather than in its run module, so
  `create_window` is named on both targets. **The IPC socket, under
  `--ipc` only:** `IpcServer::start`, whose message names the path
  and why it was refused — a parent directory other users can write,
  a path that is not a stale socket of ours, a live listener — so a
  bad `--ipc` ends the launch instead of leaving IPC silently
  missing. **The renderer bootstrap:** `create_surface`, then
  `request_adapter` and `request_device` — no GPU, no program —
  reached from both `Renderer::bootstrap_native` and
  `Renderer::bootstrap_wasm`. **The page, in the browser:**
  `web_sys::window`, the `document`, `body` and `canvas` hung off it,
  the `append_child` that attaches one to the other, and the
  `inner_width` / `inner_height` the first surface is sized from —
  any one of them missing means there is no page to draw on. **The
  font system, behind `fonts::init`:** the `acquire_font_system_read`
  the family index is built under, and the `panic!`s that a
  poisoned, re-entrant or unavailable guard raises — a message rather
  than an `expect` only because the helper has no `Result` to hand
  back: a lock that cannot be had is not something the caller can
  recover from. Two
  things in the phase list cannot fail, and are inside the boundary
  rather than exceptions to it: `fonts::init` itself returns `()`, and
  the query-string extractor is infallible by construction —
  `search().unwrap_or_default()` then `strip_prefix`, with no
  failure to report. What that extractor *feeds* is fallible, and is
  the next bullet — which is also why the map fetch asks
  `web_sys::window` a second time and *degrades* rather than
  aborting: by then there is a shell to put the message in. Bare
  `unwrap()` outside tests is a bug. **This list is checked against
  the code in all three directions, not trusted:** the tests of the
  module the next bullet names fail when it holds a name that has
  moved, when it holds a name that cannot fail, and when a file on
  the startup path grows a panicking call this list does not name.
  It held one of each of the first two — a renamed renderer
  constructor, and a font initializer that returns nothing — for as
  long as nothing looked; and the third direction did not exist at
  all, which is how `request_adapter` and `request_device` came to
  `expect` on every launch of both targets while going unmentioned.
- **"Bare `unwrap()` outside tests is a bug" is enforced twice, and
  the two mechanisms are not redundant.**
  `baumhard::util::unwrap_posture` reads every `.rs` file a shipped
  build compiles — `#[cfg(test)]` items excised in place,
  `#[cfg(test)] mod x;` files and `#![cfg(test)]` files dropped,
  baumhard's ungated `pub mod tests;` trees and `benches/` dropped
  by path — and fails `./test.sh` naming `file:line`. Beside it,
  six of the seven cargo targets carry
  `#![warn(clippy::unwrap_used)]` with
  `#![cfg_attr(test, allow(...))]`, so the same rule reaches an
  editor while the code is being written. The seventh is the
  criterion bench root, which is test code by §T2.2 and which the
  scan drops by path. **Count the targets, not the libraries:**
  `lib/baumhard/src/bin/generate_stress_map.rs` and
  `lib/baumhard/build.rs` are each a crate root of their own, and
  the attribute in `lib.rs` reaches neither — a planted `unwrap()`
  in either drew nothing from `cargo clippy` until that file carried
  the attribute itself. Both were found by writing the count down
  and then checking it against `cargo metadata`, which is the only
  reason this sentence states a number at all. The scan is the gate
  on every `./test.sh`; the lint is the faster feedback — and, under
  `./test.sh --lint`, a second gate, since clippy's warning baseline
  reached zero — and it reads post-macro-expansion code the scan
  cannot. Neither replaces the
  judgment: `expect("<the invariant relied on>")` is the answer when
  a value is provably there, binding at the guard is the answer when
  something already proved it, and degrading is the answer in an
  interactive path. The count this rule was written against was
  twenty-six sites (#42); six weeks later ten of them had moved or
  vanished under unrelated work, leaving sixteen — and the scan
  found seventeen. The extra one was not new:
  `mindmap/connection/mod.rs:90` had been at that line since
  2026-04-15, eleven weeks before the issue was filed, and the hand
  count missed it. A list does not only go stale, it can be wrong
  the day it is written, which is why the posture is checked rather
  than listed.
- **The initial map load is the one startup path that does not
  `expect`, and the reason names the boundary.** `expect` is for a
  *program precondition* that did not hold — no adapter, no fonts,
  nonsense on the command line. There is no window yet and nothing
  sensible to draw into it, so a loud abort is the honest outcome. A
  `.mindmap.json` the loader rejects is not that: it is a user
  *input* the program is supposed to have an opinion about, and
  `expect` turns having an opinion into a crash. On a desktop launch
  — double-click, `.desktop` entry, file association — that crash is
  a process that vanishes leaving no window at all, which tells the
  person holding the file strictly less than the empty window it
  would replace. So the initial `loader::load_from_file` on native
  and the `?map=` fetch + parse in the browser **put the loader's
  message in front of the user and keep the shell alive**: both
  resolve through `app::startup_load`, which logs the message —
  behind the path or URL that was asked for, because most of the
  loader's messages do not name it — and hands the shell
  `baumhard::mindmap::placard`'s one-node map carrying the same
  words, installed by the same code that installs a real document.
  Each target makes exactly one call into that module and is handed a
  document, never a `Result`: a startup-error surface that exists on
  only one of them is not a fix (§4), and a `Result` in an init site's
  hands is where the two start to differ. This paragraph is the
  recorded decision #107 asked for: the code and the convention were
  in conflict, and they are reconciled here rather than at `expect`.
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
- **A log line is also a reply.** Under `--ipc`, every `warn!` /
  `error!` a command's execution emits on the main thread is carried
  in that command's reply as a warning, in the line's own words
  (`baumhard::util::log::capture`; `format/ipc.md` §Warnings). So a
  degrade an agent should be able to *branch* on, rather than read,
  gets a stable code at the site that logs it, as a key-value:
  `log::warn!(code = GATE_REJECTED_CODE; "macros: …")`. The constant
  lives beside the site, and the code is API once it ships. A degrade
  that does not log is invisible there as well as on stderr, which is
  one more reason it must.

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
  merges. `./test.sh --lint`'s fmt, doc and clippy gates are all hard
  at zero baselines (#130, #134 for the first two), so a clippy
  warning fails the run — fix it. CI runs `--lint` on the toolchain
  pinned in `rust-toolchain.toml`, so a pull request that skips it
  locally is still held to it.
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
