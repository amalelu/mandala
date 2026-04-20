# Baumhard Conventions

Crate-local conventions for `lib/baumhard`. These rules sit on top of
the workspace-wide [`CODE_CONVENTIONS.md`](../../CODE_CONVENTIONS.md)
and the [`TEST_CONVENTIONS.md`](../../TEST_CONVENTIONS.md) file in the
repo root. When a rule here conflicts with the workspace-wide
document, this document wins *inside the Baumhard crate* — the
foundation has stricter needs than what is built on top of it.

## §B0 Baumhard is ours

This project is dedicated to Lord God (see
[`CODE_CONVENTIONS.md §0`](../../CODE_CONVENTIONS.md)), and Baumhard is
part of it. Baumhard is not a vendored library we accommodate; it is
the foundation we are building together with the application — *our
work*, not a dependency. We maintain and develop both. The most
important consequence: **"I cannot change Baumhard" is never a reason.**
When the application needs something Baumhard does not yet provide, the
answer is to extend Baumhard, not to work around it from the app crate.

The foundation must be pristine. Every primitive that other code rests
on has to be Unicode-correct, panic-free in interactive paths,
`unsafe`-free everywhere, and measurably fast. Code that does not meet
that bar does not belong here — fix it or do not land it. Because the
crate is ours, primitives can be replaced rather than preserved: a
shape that turns out wrong is rewritten, not kept around for
backward-compatibility's sake (see
[`CODE_CONVENTIONS.md §10`](../../CODE_CONVENTIONS.md)).

## §B1 Performance is non-negotiable

Baumhard is the hot path. Every allocation, every tree rebuild, every
cloned subtree, and every lock acquisition shows up in a benchmark
sooner or later — and on the lowest-spec target Mandala runs on (a
mobile browser), it shows up as a dropped frame, a warm device, or a
flat battery.

- **Budget for the worst target.** Acceptable on a desktop is not
  acceptable. Acceptable on a phone in a browser, with thermal
  throttling, is the bar.
- **Touch-input math is hot too.** Anything that runs on every
  pointer event — gesture recognition, hit tests, scroll
  decomposition — is subject to the same rules as `walk_tree_from`.
- **Write Baumhard code that is readable *and* fast.** When they
  genuinely conflict — and they rarely do — prefer fast and leave a
  short comment explaining what the readable version would have cost.
  "Readable" is optimism about future maintenance; "fast" is a
  commitment to the user sitting in front of a 60 Hz frame budget on a
  device the developer is not holding.

## §B2 Mutation, not rebuild

This is the central rule of the crate. The tree is an arena; changing
it should almost never mean allocating a new arena.

- **Use `MutatorTree::apply_to` for changes.** Build a
  `MutatorTree<GfxMutator>` describing the delta, then call its
  `apply_to` method with the target `Tree` as `&mut` argument. The
  `Applicable` trait lives in `lib/baumhard/src/core/primitives.rs`;
  the concrete impl for the `GfxElement` tree lives in
  `lib/baumhard/src/gfx_structs/tree.rs`; the walk itself is
  `walk_tree_from` in `lib/baumhard/src/gfx_structs/tree_walker.rs`.
- **Do not clone a subtree, edit it, and reinsert it.** That is a
  full-arena copy to change one field. Use a mutator with the
  targeted field variant (`GlyphAreaField::*` or
  `GlyphModelField::*`) instead.
- **`arena_utils::clone_subtree` is expensive.** It is benchmarked as
  `arena_utils_clone`. Reach for it only when the semantic unit of
  work is genuinely a copy — detaching a subtree for a drag preview,
  for instance — never as a shortcut around the mutator pipeline.
- **Compose mutators; do not branch into rebuild paths.** A
  conditional mutator (`GfxMutator::Instruction` with a `RepeatWhile`
  predicate — negate it for skip-style flow) is cheaper than cloning
  a subtree, clearing it, and rebuilding. If the mutator language is
  missing an expression
  you need, **extend it** — see
  `lib/baumhard/documents/mutators/mutators.md`. Extending the mutator
  language is exactly the kind of work §B0 is about.

## §B3 Grapheme-aware text

Unicode correctness is a load-bearing invariant. Baumhard renders
arbitrary user text — emoji, combining marks, regional indicators,
zero-width joiners. "Number of characters" is not a well-defined
concept; "number of grapheme clusters" is, and it is what users see.

- **All text primitives live in `lib/baumhard/src/util/grapheme_chad.rs`.**
  If you need to manipulate a `String` or `&str`, call a function from
  that file: `replace_graphemes_until_newline`, `split_off_graphemes`,
  `count_grapheme_clusters`, `find_nth_line_grapheme_range`,
  `delete_back_unicode`, `delete_front_unicode`, or one of their
  neighbours. Do not reach for `str::chars()`, `str::bytes()`, or
  `String::split_off` when the offset comes from a user-facing count.
- **Never slice by byte offset when the offset is user-derived.** If
  you need the position of the 10th cluster, call
  `find_nth_line_grapheme_range` or count clusters explicitly. Slicing
  by `str_idx` lands you mid-grapheme on the first emoji and corrupts
  the string.
- **New text primitives go in `grapheme_chad.rs`.** They ship with a
  `do_*()` test (see `lib/baumhard/src/util/tests/grapheme_chad_tests.rs`)
  and a criterion bench entry in `lib/baumhard/benches/test_bench.rs`
  in the *same commit*.

## §B4 Arena and tree discipline

`Tree` wraps an `indextree::Arena` for O(1) child iteration and O(1)
node access via `NodeId`. The arena is what makes Baumhard fast; treat
it with respect.

- **Iterate with `NodeId::children(&arena)` and `descendants(&arena)`.**
  Do not collect into a `Vec<NodeId>` unless the borrow checker forces
  your hand, and when it does, reuse the vector across iterations if
  you can.
- **Node access is O(1) via `NodeId`.** If you find yourself linear-
  scanning the arena to find a node, you have lost a `NodeId`
  somewhere up the call stack. Find it and thread it through.
- **`GfxElement` and its field enums are the mutation surface.**
  `GlyphAreaField` (in `lib/baumhard/src/gfx_structs/area.rs`) and
  `GlyphModelField` (in `lib/baumhard/src/gfx_structs/model.rs`) are
  where new kinds of change land. A new mutation variant is a new
  field variant plus a branch in `DeltaGlyphArea::apply_to` /
  `DeltaGlyphModel::apply_to`, not a new wrapper struct.
- **Do not rebuild the arena to change one field.** If you catch
  yourself writing `Arena::default()` to fix a typo in a node's text,
  stop and use a mutator.

## §B5 Font and layout

`cosmic-text` owns glyph layout. Baumhard owns the single point of
access to it.

- **`fonts::init()` is called once at app startup.** Library code must
  not call it; the app crate calls it; Baumhard internals assume it
  has been called. The init lives in `lib/baumhard/src/font/fonts.rs`.
- **`FONT_SYSTEM` is a global `RwLock<FontSystem>`.** Acquire the
  guard for the minimum scope needed for a single layout pass. Holding
  the write lock across a long computation serialises the whole
  renderer — which on a single-threaded event loop means stalling the
  entire frame.
- **`create_cosmic_editor_str` is the blessed entry point for text
  layout.** If you need layout behaviour it does not provide, extend
  it rather than reaching into `cosmic_text::Buffer` or
  `cosmic_text::Editor` directly from another module. Every call site
  that does its own cosmic-text dance is a place where the lock
  discipline can drift.

## §B6 Regions and spatial indexing

`RegionIndexer` (`lib/baumhard/src/gfx_structs/util/regions.rs`)
maintains a spatial index over the tree's colour and font regions so
that hit-testing and selection highlighting are O(log n) instead of
O(n). It is maintained as a side-effect of `MutatorTree::apply_to`.

- **Never mutate `ColorFontRegions` outside the mutator pipeline.**
  Direct writes skip the index update, the index drifts, and selection
  starts pointing at the wrong glyphs. Every region change is a
  `GlyphAreaField::ColorFontRegions(...)` mutator.
- **Region math has benchmarks.** `region_indexer_initialise`,
  `region_indexer_insert`,
  `region_params_calculate_pixel_from_region`,
  `region_params_calculate_region_from_pixel`, and
  `region_params_calculate_regions_intersected_by_rectangle` in
  `benches/test_bench.rs`. A change to the region layer ships with a
  bench result in the commit message for any non-trivial edit.

## §B7 Hot-path rules

These are the specific rules that make Baumhard measurably fast on the
worst target.

- **No new allocations in hot loops.** "Hot loops" means anything
  inside `walk_tree_from`, `DeltaGlyphArea::apply_to`,
  `DeltaGlyphModel::apply_to`, gesture / pointer handlers, or any
  function benchmarked in `benches/test_bench.rs`. Do not introduce a
  `Vec::new()` or a `String::new()` on the hot path without a
  benchmark to justify it.
- **`#[inline]` on true hot paths, not everywhere.** Use it when a
  benchmark demonstrates an improvement. `GlyphModelField::same_type`
  in `lib/baumhard/src/gfx_structs/model.rs` is the exemplar: tiny,
  called in the tight loop, inlined on purpose. `#[inline]` on a cold
  function just slows down compilation.
- **`unsafe` is forbidden.** New `unsafe` is a roadmap-scale decision
  and needs a benchmark plus a review. `unsafe` for lifetime
  laundering, raw pointer arithmetic, or "I know better than the
  borrow checker" is never acceptable. The existing `unsafe` in
  `lib/baumhard/src/util/simd.rs` is legacy scaffolding from an early
  SIMD experiment and is a violation to be cleaned up — not a
  precedent.
- **Every user-visible primitive has a criterion bench.** New
  primitives ship with a new entry in `benches/test_bench.rs`;
  removed primitives drop theirs in the same commit. The bench file
  is not compiled under `cfg(test)`, so the compiler will not catch
  drift — discipline is the only thing that keeps it accurate.
- **Lock guards are held for the minimum scope possible.** See §B5 on
  `FONT_SYSTEM`. The same rule applies to every `RwLock` or `Mutex`
  inside Baumhard: take the guard, do the work, drop the guard.

## §B8 Benchmark-reuse and `do_*()` discipline

Baumhard uses the `pub mod tests;` pattern specifically so that
criterion benches can reuse test bodies as micro-benchmarks. See
[`TEST_CONVENTIONS.md §T2`](../../TEST_CONVENTIONS.md) for the full
rationale.

- **Keep the `do_*()` / `test_*()` split intact.** The `do_*()`
  function is `pub` and benchmark-reachable; the `test_*()` wrapper is
  the thin `#[test]` entry point. Never fold them together.
- **Renaming or deleting a `do_*()` is a two-file change.**
  `benches/test_bench.rs` imports them by path. The bench file is
  not compiled under `cfg(test)`, so `cargo test` will not tell you it
  has drifted — only `cargo bench` or `./test.sh --bench` will.
  Update both files in the same commit.
- **Do not "fix" the missing `#[cfg(test)]` on Baumhard test
  modules.** It is load-bearing. Removing it breaks the bench harness.

## §B9 Library-grade documentation

Baumhard is a library. Its consumers — the Mandala app today, plugins
and a script API tomorrow — read its docs via `cargo doc`. Treat
`cargo doc -p baumhard --no-deps` as a first-class deliverable.

- **Every `pub` item carries a `///` doc comment.** No exceptions.
  Every `pub` function, struct, enum, trait, and module under
  `lib/baumhard/src/`. Existing items missing one is technical debt to
  be closed on the way past, not a precedent to extend.
- **Doc comments state *purpose, inputs, costs*.** "Costs" is what
  separates a Baumhard doc comment from a generic one: note an O(n)
  walk, an allocation, a clone, a lock acquisition, a full arena
  sweep. A consumer reading the doc should be able to decide "is this
  cheap enough to call every frame on a phone?" without reading the
  body.
- **Module-level `//!` headers** describe the concept the module
  implements, not the list of items in it. The list is what
  `cargo doc` generates; the concept is what the human needs.
- **Examples in doc comments** are welcome on anything non-trivial,
  especially mutator construction. A two-line example of building a
  `MutatorTree` to change text is worth more than a paragraph.
- **Update doc comments when you change behaviour.** A doc comment
  that lies about its function is worse than no doc comment at all.

## §B10 Forward-compatible API design

Baumhard's public surface is the substrate the named trajectory rests
on: plugins, a Baumhard script API, complex tree animations, complex
file exports. Design its `pub` shape with that trajectory in mind.

This is distinct from §B0. §B0 says we can replace past shapes that
turned out wrong; §B10 says new shapes account for the consumers we
have already named.

- **`pub` is a commitment to extensibility, not just visibility.** A
  function exposed as `pub` because the app needs it today is also,
  whether you intended it or not, the surface a future plugin will
  reach for. Name it, shape it, and document it (§B9) accordingly.
- **Prefer surfaces that compose.** Mutators compose. Walkers compose.
  A single monolithic "do everything" entry point does not. A script
  API will reach for the composable surface; build that surface as
  you build the feature.
- **Do not leak private invariants through `pub` types.** A `pub`
  struct that exposes raw `Arena` indices is a struct that ties every
  future consumer to the current internal representation. Wrap it.
- **Preserving a seam ≠ preserving a shape.** §B0 lets us replace a
  surface when it turns out wrong. §B10 says to design new surfaces so
  the *category* of consumer (plugin, script, animation, export)
  stays reachable across that replacement. The rewrite changes the
  shape; the seam survives.
