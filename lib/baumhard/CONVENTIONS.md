# Baumhard Conventions

Crate-local conventions for `lib/baumhard`. These rules sit on top of
the workspace-wide [`CODE_CONVENTIONS.md`](../../CODE_CONVENTIONS.md) and
the [`TEST_CONVENTIONS.md`](../../TEST_CONVENTIONS.md) file in the repo
root. When a rule here conflicts with the workspace-wide document, this
document wins *inside the Baumhard crate* — the rationale being that
Baumhard is the performance-critical core and has stricter needs.

## §B0 Scope and philosophy

Baumhard is the hot path. Every allocation, every tree rebuild, every
cloned subtree, and every lock acquisition shows up in a benchmark
sooner or later. The application crate can afford a `Vec::new()` inside
an event handler; Baumhard cannot afford one inside `walk_tree_from`.

Write Baumhard code that is readable *and* fast. When they genuinely
conflict — and they rarely do — prefer fast, and leave a short comment
explaining what the readable version would have cost. "Readable" is a
form of optimism about future maintenance; "fast" is a commitment to
the user sitting in front of a 60 Hz frame budget.

Baumhard code must also be Unicode-correct, panic-free in interactive
paths, and `unsafe`-free everywhere. These are not negotiable.

## §B1 Mutation, not rebuild

This is the central rule of the crate. The tree is an arena; changing
it should almost never mean allocating a new arena.

- **Use `MutatorTree::apply_to` for changes.** Build a
  `MutatorTree<GfxMutator>` describing the delta, then call its
  `apply_to` method with the target `Tree` as `&mut` argument. The
  `Applicable` trait is defined at
  `lib/baumhard/src/core/primitives.rs:269` and the concrete impl for
  the `GfxElement` tree lives at
  `lib/baumhard/src/gfx_structs/tree.rs:55`. The walk itself is
  `walk_tree_from` in `lib/baumhard/src/gfx_structs/tree_walker.rs`.
- **Do not clone a subtree, edit it, and reinsert it.** That is a
  full-arena copy to change one field. Use a mutator with the
  targeted field variant (`GlyphAreaField::*` or
  `GlyphModelField::*`) instead.
- **`arena_utils::clone_subtree` is expensive.** It is benchmarked as
  `arena_utils_clone` in `lib/baumhard/benches/test_bench.rs`. Reach
  for it only when the semantic unit of work is genuinely a copy —
  detaching a subtree for drag preview, for instance — and never as a
  shortcut around the mutator pipeline.
- **Compose mutators; do not branch into rebuild paths.** A
  conditional mutator (`GfxMutator::Instruction` with a
  `RepeatWhile` / `SkipWhile` predicate) is cheaper than cloning a
  subtree, clearing it, and rebuilding. If the mutator language is
  missing an expression you need, extend it — see
  `lib/baumhard/documents/mutators/mutators.md`.

## §B2 Grapheme-aware text

Unicode correctness is a load-bearing invariant. Baumhard renders
arbitrary user text, which means it renders emoji, combining marks,
regional indicators, and zero-width joiners. "Number of characters" is
not a well-defined concept; "number of grapheme clusters" is, and that
is what users see.

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

## §B3 Arena and tree discipline

`Tree` wraps an `indextree::Arena` for O(1) child iteration and O(1)
node access via `NodeId`. The arena is the thing that makes baumhard
fast; treat it with respect.

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

## §B4 Font and layout

`cosmic-text` owns glyph layout. Baumhard owns the single point of
access to it.

- **`fonts::init()` is called once at app startup.** Library code must
  not call it. The app crate calls it; baumhard internals assume it
  has been called. See `lib/baumhard/src/font/fonts.rs:51`.
- **`FONT_SYSTEM` is a global `RwLock<FontSystem>`** at
  `lib/baumhard/src/font/fonts.rs:47`. Acquire the guard for the
  minimum scope needed for a single layout pass. Holding the write
  lock across a long computation serialises the whole renderer.
- **`create_cosmic_editor_str` is the blessed entry point for text
  layout.** If you need layout behaviour it doesn't provide, extend
  it rather than reaching into `cosmic_text::Buffer` or
  `cosmic_text::Editor` directly from some other module. Every call
  site that does its own cosmic-text dance is a place where the lock
  discipline can drift.

## §B5 Regions and spatial indexing

`RegionIndexer` (`lib/baumhard/src/gfx_structs/util/regions.rs:9`)
maintains a spatial index over the tree's colour and font regions so
that hit-testing and selection highlighting are O(log n) instead of
O(n). It is maintained as a side-effect of `MutatorTree::apply_to`.

- **Never mutate `ColorFontRegions` outside the mutator pipeline.**
  Direct writes skip the index update, the index drifts, and
  selection starts pointing at the wrong glyphs. Every region change
  is a `GlyphAreaField::ColorFontRegions(...)` mutator.
- **Region math has benchmarks.** See `region_indexer_initialise`,
  `region_indexer_insert`,
  `region_params_calculate_pixel_from_region`, and
  `region_params_calculate_regions_intersected_by_rectangle` in
  `benches/test_bench.rs`. A change to the region layer ships with a
  bench result in the commit message for any non-trivial edit.

## §B6 Performance rules

These are the specific rules that make baumhard measurably fast.

- **No new allocations in hot loops.** "Hot loops" means anything
  inside `walk_tree_from`, `DeltaGlyphArea::apply_to`,
  `DeltaGlyphModel::apply_to`, or any function benchmarked in
  `benches/test_bench.rs`. Do not introduce a `Vec::new()` or a
  `String::new()` on the hot path without a benchmark to justify it.
- **`#[inline]` on true hot paths, not everywhere.** Use it when a
  benchmark demonstrates an improvement. `GlyphModelField::same_type`
  at `lib/baumhard/src/gfx_structs/model.rs:68` is the exemplar:
  tiny, called in the tight loop, inlined on purpose. `#[inline]` on
  a cold function just slows down compilation.
- **`unsafe` is forbidden.** There is currently no `unsafe` in
  baumhard. New `unsafe` is a roadmap-scale decision and needs a
  benchmark plus a review.
- **Every user-visible primitive has a criterion bench.** New
  primitives ship with a new entry in `benches/test_bench.rs`;
  removed primitives drop theirs in the same commit. The bench file
  is not compiled under `cfg(test)`, so the compiler will not catch
  drift — discipline is the only thing that keeps it accurate.
- **Lock guards are held for the minimum scope possible.** See §B4 on
  `FONT_SYSTEM`. The same rule applies to any `RwLock` or `Mutex`
  inside baumhard: take the guard, do the work, drop the guard.

## §B7 Benchmark-reuse and `do_*()` discipline

Baumhard uses the `pub mod tests;` pattern specifically so that
criterion benches can reuse test bodies as micro-benchmarks. See
[`TEST_CONVENTIONS.md §3.2`](../../TEST_CONVENTIONS.md) for the full
rationale.

- **Keep the `do_*()` / `test_*()` split intact.** The `do_*()`
  function is `pub` and benchmark-reachable; the `test_*()` wrapper
  is the thin `#[test]` entry point. Never fold them together.
- **Renaming or deleting a `do_*()` is a two-file change.**
  `benches/test_bench.rs` imports them by path. The bench file is
  not compiled under `cfg(test)`, so `cargo test` will not tell you
  it has drifted — only `cargo bench` or `./test.sh --bench` will.
  Update both files in the same commit.
- **Do not "fix" the missing `#[cfg(test)]` on baumhard test
  modules.** It is load-bearing. Removing it breaks the bench harness.

## §B8 Documentation

Baumhard is a library. Its consumers — currently the Mandala app, but
the design assumes more — read its docs via `cargo doc`. Treat
`cargo doc -p baumhard --no-deps` as a first-class deliverable.

- **Every `pub` item carries a `///` doc comment.** No exceptions.
  Every `pub` function, struct, enum, trait, and module.
- **Doc comments state *purpose, inputs, costs*.** "Costs" is the
  thing that separates a baumhard doc comment from a generic one:
  note an O(n) walk, an allocation, a clone, a lock acquisition, a
  full arena sweep. A consumer reading the doc should be able to
  decide "is this cheap enough to call every frame?" without reading
  the body.
- **Module-level `//!` headers** describe the concept a module
  implements, not the list of items in it. The list is what
  `cargo doc` generates; the concept is what the human needs.
- **Examples in doc comments** are welcome on anything non-trivial,
  especially mutator construction. A two-line example of building a
  `MutatorTree` to change text is worth more than a paragraph.
- **Update doc comments when you change behaviour.** A doc comment
  that lies about its function is worse than no doc comment at all.

## §B9 Breaking these conventions

Same escape hatch as the workspace-wide document: break a rule with a
local comment explaining why, and a reviewer decides whether the
deviation is earned. The bar for deviations is higher here than in the
app crate — baumhard's rules exist because they pay off in the
benchmark — so a deviation should either have a benchmark backing it
up or a clear architectural reason that outweighs the performance
cost.
