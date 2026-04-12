# Code Conventions

## §0 How to read this document

These conventions belong to the *repository* first and foremost, not to
developers. They are aspirational in the sense that real code always lags
behind the ideal, and we would rather merge useful off-convention code
than reject it — a later session can refactor it into shape. They are
binding in the sense that every new change should move the codebase
*towards* the conventions, never deliberately away, and that a session
choosing to break a rule should leave a local comment explaining why so
a reviewer can decide whether the deviation is earned.

Coding is an art. We want the code to be easy to understand and maintain,
but we also want it to be beautiful — humorous, poetic, inspiring where
it can be. Art thrives under limitations but not under strict recipes.
The rules below are limitations, not recipes: follow them tightly enough
that the codebase stays coherent, but loosely enough that the art can
still breathe. When a rule is wrong in a specific place, break it on
purpose and say so.

See also:
- [`TEST_CONVENTIONS.md`](./TEST_CONVENTIONS.md) — the testing spec,
  including the `do_*()` / `test_*()` split and what we deliberately
  don't do.
- [`lib/baumhard/CONVENTIONS.md`](./lib/baumhard/CONVENTIONS.md) — the
  performance-critical rules that apply inside the Baumhard crate.
- [`CLAUDE.md`](./CLAUDE.md) — durable orientation notes for new
  sessions. `CLAUDE.md` is descriptive ("how things work"); this
  document is prescriptive ("how to change them").

## §1 Architectural invariants

The invariants below are not negotiable inside a single session. Changing
any of them is a roadmap-scale decision and belongs in `ROADMAP.md`
before it belongs in code.

- **Single-threaded event loop.** The `Application` struct owns the
  `Renderer` directly. There are no channels, no worker threads, no
  `tokio`, no `std::thread::spawn` in interactive paths. An earlier
  revision was multi-threaded; it is gone on purpose. If you think you
  need concurrency, it belongs in a roadmap entry, not a commit.
- **Model / view separation.** `MindMapDocument` in
  `src/application/document.rs` owns the data model (`MindMap`,
  selection, undo stack). The `Renderer` in `src/application/renderer.rs`
  owns GPU resources. Rendering reads from intermediate representations
  the document builds (`Tree<GfxElement, GfxMutator>` for nodes,
  `RenderScene` for edges / borders / portals). The renderer never
  reaches into the document's data model directly, and the document
  never holds GPU handles.
- **Two-pipeline render.** Nodes, and connections render through the Baumhard tree;
  , borders, and portals render (for now) through the flat
  `RenderScene`. These are two independent paths wired side-by-side in
  the event loop. New visuals must choose a pipeline on purpose — but ideally
  everything should use the Baumhard tree unless it has a good reason not to.
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
  `(from_id, to_id, edge_type)`. Mirror this pattern everywhere you
  need to reference an edge — do not invent a `Uuid` field or an index
  you have to maintain.
- **Everything is glyphs.** Text, borders, and connections all render
  as positioned font glyphs via cosmic-text. There are no rectangle
  shaders, no bitmap UI, no sprite atlases. If you need a new visual
  element, think about how to express it as characters first. A new
  shader pipeline is a roadmap-scale proposal.
- **Cross-platform reality.** Almost everything that works on native
  also compiles for `wasm32-unknown-unknown`. Native-only code lives
  behind `#[cfg(not(target_arch = "wasm32"))]`; WASM-only code behind
  `#[cfg(target_arch = "wasm32")]`; everything else is shared. Platform
  abstraction uses `cfg` guards, not traits.

## §2 Using baumhard

Baumhard is not a passive dependency. It is the text-manipulation,
layout, and tree-mutation engine this application is built around.
Application code must consume Baumhard's primitives rather than
reimplement them. This rule exists for two reasons: Baumhard is where
the performance work lives (see
[`lib/baumhard/CONVENTIONS.md`](./lib/baumhard/CONVENTIONS.md)), and
duplicating its primitives in the app crate causes them to drift.

- **Text manipulation goes through `baumhard::util::grapheme_chad`.**
  Use `replace_graphemes_until_newline`, `split_off_graphemes`,
  `count_grapheme_clusters`, `find_nth_line_grapheme_range`,
  `delete_back_unicode`, and `delete_front_unicode`
  (`lib/baumhard/src/util/grapheme_chad.rs`). Never index or slice a
  `String` by byte offset when the offset is derived from something a
  user typed — you will land mid-grapheme on the first emoji.
- **Tree mutation goes through `MutatorTree::apply_to`.** Build a
  `MutatorTree<GfxMutator>` describing the delta, then call `apply_to`
  on it with the target `Tree` as `&mut` argument (see the
  `Applicable<Tree<GfxElement, GfxMutator>>` impl at
  `lib/baumhard/src/gfx_structs/tree.rs:55`). Do not clone a subtree,
  edit it, and re-insert it to change one field.
- **Font access goes through `baumhard::font::fonts`.** Call
  `fonts::init()` once at app startup. Read the global `FONT_SYSTEM`
  through its lock guard (`lib/baumhard/src/font/fonts.rs:47`). Never
  instantiate a standalone `cosmic_text::FontSystem` in app code.
  Layout goes through `create_cosmic_editor_str` or an extension to
  it — never raw cosmic-text calls from the application crate.
- **Geometry, color, and regions are Baumhard's.**
  `baumhard::util::geometry::almost_equal_vec2`,
  `baumhard::util::color::*`, and
  `baumhard::gfx_structs::util::regions::RegionIndexer` are the
  canonical versions. Do not redefine them in the app crate. If you
  need a new geometry helper, add it to baumhard.
- **Missing primitives go into baumhard, not into `src/application/`.**
  If application code is about to grow a second implementation of
  something baumhard already almost does, extend baumhard instead. The
  maintenance win is real; so is the performance win, because baumhard
  has benchmarks and the app crate does not.

## §3 Complexity and KISS

The cheapest abstraction is the one that does not exist. The cheapest
configurability is the flag you never added. Keep things as simple as
the reality they model, but do not force a complex reality into a
simpler shape than it will tolerate.

- **Prefer editing over creating.** New files should feel justified.
  A one-line helper is not a module; a private function used once does
  not need its own file. If the edit you are about to make can live
  next to the code that already implements the concept, put it there.
- **Three similar lines of code beats a premature abstraction.**
  Extract a helper when a pattern repeats three times *and* the
  repetition obscures intent. Two occurrences is a coincidence; three
  with identical shape and meaning is a duplication. Three with
  different meanings is still two problems.
- **Trust internal invariants.** Do not add defensive `if` branches,
  `Result` wrappers, or fallbacks for states that cannot occur. Validate
  at system boundaries — file loaders, CLI args, user input, the `?map=`
  query parameter — and trust your own data structures past the boundary.
- **Do not design for hypothetical future callers.** Add configurability
  when a second caller actually needs it, not when you imagine it might.
  A flag without a user is dead weight.
- **Delete dead code.** Remove it when you notice it. Do not leave
  `// removed:` stubs, renamed `_placeholder` fields, or "just in case"
  branches. If it is genuinely deferred WIP from a roadmap milestone,
  leave it exactly as the roadmap left it and move on.
- **Function size is a smell, not a rule.** A function the size of the
  screen is fine if it is doing one thing. The same function becomes a
  problem when it mixes concerns — parse *and* validate *and* render.
  Split on the seam, not on the line count.

## §4 Error handling

The codebase's error-handling posture is deliberate and narrow. Do not
introduce `anyhow`, `thiserror`, or custom `Error` enums without
discussing it in a roadmap entry first.

- **No custom error types.** The codebase panics at startup for GPU and
  loader failures and logs at runtime. Matching this posture keeps the
  surface small.
- **`expect("<reason>")` at startup is acceptable** when the failure is
  unrecoverable and the message helps a human understand what went
  wrong (see `src/application/renderer.rs` for the shape:
  `expect("Failed to create device")`). Every `expect` carries a useful
  message. Bare `unwrap()` in non-test code is a bug.
- **Interactive paths must not panic.** Input handling, mutation
  application, frame render, and document mutation — none of these
  may abort the process. Degrade the frame, log via `log::warn!` or
  `log::error!`, and keep running. A crash during editing is the one
  user-visible failure this codebase cannot tolerate. The existing
  app crate is not yet fully up to this bar — there are `expect` and
  `unwrap` calls in interactive paths that predate this rule — and
  new code must not make the situation worse; refactors that remove
  an `unwrap` from an interactive path are welcome drive-bys.
- **Defensive checks in interactive paths are the sanctioned exception
  to §3.** "Trust internal invariants" stops at the edge of code that
  runs sixty times a second in front of a user. A `let Some(node) =
  ... else { return; }` to prevent a panic is earned; the same check
  in a pure function with controlled inputs is noise.
- **`unwrap()` without a message is acceptable only in tests.** Inside
  `#[cfg(test)]` or `pub mod tests;`, it is fine.

## §5 Documentation and comments

Code is read more often than it is written, and by future sessions more
often than by the author. Document accordingly. But documentation is
also a liability: every comment you write is a thing that can become
stale.

- **Every `pub` item in baumhard carries a `///` doc comment.** Every
  `pub` function, type, trait, and module under `lib/baumhard/src/`
  gets a doc comment that explains *what it does, why it exists, and
  what it costs*. "Costs" is specific to baumhard — note an O(n)
  walk, an allocation, a clone, a lock acquisition. See
  [`lib/baumhard/CONVENTIONS.md §B8`](./lib/baumhard/CONVENTIONS.md).
  `cargo doc -p baumhard --no-deps` is a first-class deliverable and
  the consumer's entry point to the library. The current crate is
  not fully documented; the rule is aspirational in the strict sense
  — we are moving towards it, not away from it, and every edit that
  touches a `pub` item closes the gap a little.
- **Public items in the mandala crate are documented when the purpose
  is non-obvious.** An event-handler method with a descriptive name
  does not need a doc comment; a private layout calculation does not
  need one; a cross-module entry point whose invariants matter does.
- **Module-level `//!` headers** are encouraged at the top of files
  that implement a cohesive concept. They are the first thing a new
  session reads; make them count.
- **Inline `//` comments explain *why*, never *what*.** `// increment
  counter` on `counter += 1` is noise. `// clamp to canvas bounds so
  the palette cannot scroll off-screen during zoom` is signal. If your
  comment restates the code, delete one of them.
- **Do not touch documentation on code you did not change.** Do not
  add doc comments, inline comments, or type annotations to unrelated
  code you happen to be reading. That churn is noise, it spoils diffs,
  and the author of that code already decided what was worth saying.
- **`ROADMAP.md` gets updated when a feature lands** — add a bullet to
  the "What works" list, mark the relevant session's checkboxes.
  `CLAUDE.md` gets updated only when an architectural invariant
  changes. This document (`CODE_CONVENTIONS.md`) gets updated only
  when a rule in it is demonstrably wrong.

## §6 Testing

Testing conventions live in [`TEST_CONVENTIONS.md`](./TEST_CONVENTIONS.md).
Read that document before writing tests. The one-sentence summary: the
suite is small, synchronous, mock-free, and exemplar-driven; tests are
expected to stay green across changes; `./test.sh` is the covenant.

Two rules belong here rather than there because they cross the line
from testing discipline into general hygiene:

- **New mutations and undo variants ship with tests in the same commit.**
  A new `UndoAction` variant without a test for its forward-and-back
  round-trip is an incomplete change.
- **New baumhard primitives ship with a `do_*()` test and a criterion
  bench in the same commit.** See
  [`lib/baumhard/CONVENTIONS.md §B6`](./lib/baumhard/CONVENTIONS.md).

## §7 Commit hygiene

- **One conceptual change per commit.** If your diff touches three
  unrelated things, it is three commits.
- **Tests land in the commit that introduces the code they test.** Not
  the commit before, not the commit after.
- **`./test.sh` must be green before committing.** `./test.sh --lint`
  is advisory but its output should be reviewed; `./test.sh --bench`
  is for the performance-conscious commits in baumhard.
- **Commit messages describe *why*, not *what the diff shows*.** The
  diff shows what changed. The message explains why the change was
  worth making.

## §8 Breaking conventions

Sometimes a rule is wrong for a specific place. In that case: leave a
short comment at the local scope explaining why, make the change, and
move on. If the reviewer agrees the deviation is earned, the code is
not refactored to match the rule. If the deviation keeps appearing in
new places, the rule was the thing that was wrong — update this
document.

Conventions serve the codebase; the codebase does not serve the
conventions.
