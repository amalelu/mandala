# Baumhard Conventions

Crate-local conventions for `lib/baumhard`. These rules sit on top of
the workspace-wide [`CODE_CONVENTIONS.md`](../../CODE_CONVENTIONS.md)
and the [`TEST_CONVENTIONS.md`](../../TEST_CONVENTIONS.md) file in the
repo root. When a rule here conflicts with the workspace-wide
document, this document wins *inside the Baumhard crate* — the
foundation has stricter needs than what is built on top of it.

## §B0 Baumhard is ours

Baumhard is not a vendored library we accommodate; it is
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
  neighbors. Do not reach for `str::chars()`, `str::bytes()`, or
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
- **Node access is O(1) via `NodeId`.** 
  If you find yourself linear-scanning the arena to find a node, you have lost a `NodeId`
  somewhere up the call stack. Find it and thread it through.
- **`GfxElement` and its field enums are the mutation surface.**
  `GlyphAreaField` (in
  `lib/baumhard/src/gfx_structs/area_fields.rs`) and
  `GlyphModelField` (in
  `lib/baumhard/src/gfx_structs/model/mutator.rs`) are
  where new kinds of change land. A new mutation variant is a new
  field variant plus a branch in the element's `apply_operation`,
  not a new wrapper struct.

  **Adding a field variant is exactly four edits, and every one of
  them is forced** — the compiler or the suite fails until you make
  it. Nothing here relies on remembering:

  1. the variant on `GlyphAreaField` / `GlyphModelField`;
  2. its arm in `impl Add for GlyphAreaField`
     (`area_fields.rs`) — hand-written and compiler-forced.
     Area-side only: `GlyphModelField` has no `Add` impl;
  3. a representative in `gfx_structs::tests::delta_tests`'s
     `area_field_for` / `model_field_for` — compiler-forced,
     because those match exhaustively over the *derived* tag;
  4. the branch in `GlyphArea::apply_operation` /
     `GlyphModel::apply_operation` — forced by
     `apply_reaches_every_{area,model}_field`, which asserts a delta
     carrying the variant actually changes the target. Without that
     test a variant with no branch deserializes, walks the tree,
     matches its channel and mutates nothing, silently (issue #10-A).

  Everything *else* is generated or shared and must not be
  hand-written: the `*Type` tag comes from
  `#[derive(EnumDiscriminants)]`; `same_type` from the
  `Discriminated` blanket impl; and the delta storage, `new()`,
  `operation_variant()` and the `Delta::add` merge from the shared
  `Delta<F>` container in
  `lib/baumhard/src/gfx_structs/delta.rs`. (Note the split on
  "merge": `Delta::add` is shared, the per-field `Add` arm in edit 2
  is not.) **Do not hand-write a discriminant enum or a second delta
  struct** — the two surfaces were hand-kept twins once and drifted
  (issues #10, #24).

  A field tag is also the **JSON key** of a serialized delta, so
  renaming or removing one is an on-disk format change: see
  `format/mutations.md`.
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
- **`FONT_SYSTEM` is private; the lock is reached through three
  helpers, and a raw `.write()` does not compile.** (1)
  `acquire_font_system_write(site)` — the default, blocking; (2)
  `try_acquire_font_system_write(site)` — non-blocking, for the
  renderer's interactive overlay/prepare paths that skip a frame on
  contention. Contention is the only thing it reports as `None`: a
  re-entrant or *poisoned* lock panics as in the blocking form, because
  neither clears on a later frame — a skip would only hand the panic to
  the same frame's next blocking acquire; (3)
  `acquire_font_system_read(site)`. All three return
  guard types of `fonts.rs`'s own (`FontSystemWriteGuard`,
  `FontSystemReadGuard`), because the helpers record what the
  *current thread* holds. That record is what makes the two failures
  different:
  - **Re-entrancy** (this thread already holds the lock, or holds a
    read guard and wants the write guard) panics *immediately*,
    naming both sites. It is decided from the thread-local record, not
    by waiting out a timeout.
  - **Contention** (another thread holds it) waits, queued fairly, up
    to `FONT_SYSTEM_LOCK_TIMEOUT`, then panics naming the holder. It
    is never an error on its own: parallel tests contend for this lock
    constantly, and the earlier 5 s timeout, which could not tell the
    two cases apart, turned that into spurious crashes.

  Neither case may hang, and neither may be fixed by lengthening a
  timeout that stands in for a decidable question. **The uncontended
  path reads no clock**: `std::time::Instant::now()` traps on
  `wasm32-unknown-unknown`, which is the only path the browser takes.
  Do not add a timing read before the first acquire attempt;
  `tests/wasm_font_lock.rs` runs the helper on wasm32 and fails if it
  does.
- **A caller already holding the guard measures via a `*_with`
  variant, never a nested acquire.** Thread the live
  `&mut FontSystem` into the composable API — `metric_cache::glyph_*_with`,
  `border_run_specs_with`, `measure_glyph_ink_bounds`,
  `measure_text_block_unbounded` — so a cold key shapes through the
  guard the caller holds. A second acquire on the same thread is a
  guaranteed deadlock (issue P0-06). The lock-acquiring wrapper is for
  callers that hold no guard.
- **Cosmic-text usage is concentrated in `lib/baumhard/src/font/`.**
  `font/fonts.rs` owns the `FONT_SYSTEM` lock, font-id table, and
  measurement primitives (`measure_glyph_ink_bounds`,
  `measure_text_block_unbounded`); `font/attrs.rs` owns the
  `ColorFontRegions` → cosmic-text bridges (`attrs_list_from_regions`
  for `Editor::insert_string` callers, `RegionFamilies` +
  `rich_text_spans_from_regions` for `Buffer::set_rich_text` callers).
  Code outside `font/` does not import `cosmic_text` directly — if you
  need a new bridge, add it here. Every call site that does its own
  cosmic-text dance is a place where the lock discipline can drift.

## §B6 Regions and spatial indexing

`RegionIndexer` (`lib/baumhard/src/gfx_structs/util/regions.rs`)
maintains a spatial index over the tree's color and font regions so
that hit-testing and selection highlighting are O(log n) instead of
O(n). The index and its companion `RegionParams` are a tested-but-
unwired subsystem: `RegionParams` and `RegionIndexer` are allocated by
`Tree::new`, but `MutatorTree::apply_to` does **not** currently update
them. Per-tree BVH descent (`Tree::descendant_at`) handles hit-testing
today.

- **Never mutate `ColorFontRegions` outside the mutator pipeline.**
  Direct writes skip whatever index update path eventually lands, the
  index drifts, and selection starts pointing at the wrong glyphs.
  Every region change is a `GlyphAreaField::ColorFontRegions(...)`
  mutator.
- **Region math has benchmarks.** `region_indexer_initialize`,
  `region_indexer_insert_and_remove`,
  `region_params_pixel_to_region`, `region_params_region_to_pixel`,
  and `region_rect_exhaustive_4x4_grid` in `benches/test_bench.rs`,
  among the other `region_*` entries there. A change to the region
  layer keeps those entries current in the same commit (§B8's
  two-file rule). Bench *results* are a maintainer's to produce:
  `AGENTS.md` bars agents from running benchmarks and from
  publishing any number without the §B7 control row.

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
  benchmark demonstrates an improvement.
  `Discriminated::same_type` in
  `lib/baumhard/src/core/primitives.rs` is the exemplar: tiny,
  called in the tight loop, inlined on purpose. (This is the same
  exemplar that used to be named as `GlyphModelField::same_type`;
  the predicate moved into the shared trait carrying its existing
  attribute — the standard did not move with it.) `#[inline]` on a
  cold function just slows down compilation.

  **The corollary binds symmetrically: a *new* `#[inline]` needs a
  benchmark that actually resolves the effect.** Its sibling
  `Discriminated::variant` deliberately has none — it is a trivial
  forwarder to strum's already-`#[inline]` `discriminant()`, and no
  measurement here can resolve it. Neither can
  `ApplyOperation::{apply, apply_ref}`. On a contended machine a
  main-against-main control run has been observed swinging ±10% and
  as far as −23%; below that threshold nothing is demonstrated, and
  the attribute does not go in. Always run the control — a one-sided
  A/B against `main` will happily report `p = 0.00` on identical
  code.
- **A borrowed delta must not clone a payload it will not use.**
  `Applicable::apply_to` takes `&self`, so moving an owned value out
  of a delta costs a clone. Route it through
  `ApplyOperation::apply_ref`, which clones only on the arms that
  consume the payload — `Noop` and `Delete` must never deep-copy a
  `GlyphMatrix` just to discard it.
- **`unsafe` is forbidden.** There is no `unsafe` block anywhere in
  Baumhard today; keep it that way. New `unsafe` is a roadmap-scale
  decision and needs a benchmark plus a review. `unsafe` for lifetime
  laundering, raw pointer arithmetic, or "I know better than the
  borrow checker" is never acceptable.
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
- **Every `do_*()` has exactly one `bench_function` entry, and that
  is checked.** The `do_` prefix is not a naming habit; it is the
  claim that the body is on the bench surface, and
  `test_every_do_body_has_a_bench_entry`
  (`lib/baumhard/src/util/bench_surface.rs`) holds the tree to it —
  it collects every `pub fn do_*()` in a `pub mod tests;` tree,
  parses every `bench_function` call in `benches/test_bench.rs`, and
  fails naming each body with no entry **and** each body with two.
  Its sibling `test_every_bench_entry_id_is_unique` fails on two
  entries under one criterion id, which is one stored row where the
  file declares two. Both read the bench file's *code*: comments are
  stripped and string literals are blanked, so neither a
  commented-out entry nor an id that merely spells a body's name is
  an entry, and a `do_*` written outside every call is a mention
  rather than a row. Nothing else enforces this direction: the
  bench file imports the test modules by glob, so a body that never
  gets an entry upsets no compiler, and `cargo check --workspace
  --benches` catches only the *reverse* drift — an entry whose body
  is gone. Issue #44 recorded what discipline alone had produced by
  the time anyone counted: 173 of 425 bodies unbenched and eight
  whole modules the bench file never imported — seven of which the
  issue found, and three of the eight (camera, predicate, element)
  under a header claiming every public body in them was
  benchmark-reachable.
- **The id names what the row measures, and by default that is the
  body's name minus `do_`.** 505 of the 541 entries are exactly
  that, and a new entry should be. The 36 that are not depart in
  three ways, none of which the default can express:

  1. **A qualifier the flat id space needs.** Criterion has one
     namespace for the whole file, so `do_clone` is
     `arena_utils_clone`, `do_rect_exhaustive_4x4_grid` is
     `region_rect_exhaustive_4x4_grid`, and the five
     `zoom_visibility_*` rows carry their module. Seven entries.
  2. **The call measured, not the claim asserted.** A test name ends
     in the property it checks and the measurement does not:
     `do_strip_comments_removes_only_comments` is `strip_comments`,
     `do_scene_component_in_resolves_overlap_by_smallest_area` is
     `scene_component_in_overlap_smallest_area`. Fifteen entries,
     all of them older than the check.
  3. **There is no single body to name.** Nine entries run one
     bench-local helper at several inputs, so the id names the input
     instead (`subtree_drag_translate_path_zoom_1` / `_zoom_30`),
     and five measure a production path with no `do_*()` at all.

  What every id must do is lead back to what it measures, which is
  why a shortened or qualified one is fine and an unrelated one is
  not. The mechanical half of that is what the two tests above
  check: one entry per body and one entry per id, so a row and its
  source stay a one-to-one pair even where their spellings differ.
  Renaming an existing id is not a free correction — criterion keys
  a row's stored history by it, so the rename discards the history —
  which is why the shapes above stay as they are and #53 owns any
  sweep of them.
- **A test with no benchmark value opts out by not being a
  `do_*()`.** There is no allowlist and no benched subset — the
  opt-out is a plain `#[test] fn` with its body inline, which
  `#[test]` strips from every build without `cfg(test)` and so from
  the surface the check scans. An exemption written in a comment is
  invisible to the reader of the body, which is exactly how two
  bodies once carried the prefix while their own doc comment said
  they were not on the surface.

  **The list below is not a list of permissions.** It is the shapes
  a body *cannot* take the split in, enumerated so that a body which
  could is never left off the surface by argument. Four exist, and
  they are what a census of the tree finds: 65 plain non-wrapper
  `#[test]` fns live in the `pub mod tests;` trees, and 34 of them
  are one of these.

  1. **The body cannot compile into the library.** `syn` is a
     dev-dependency, so a `pub` body in a `pub mod tests;` tree
     cannot call it — the tree is part of the library. The whole
     `shape_tests::log_routing` module is `cfg(test)`-gated for
     that reason and its 18 tests are plain by construction; the
     gate does not even see them. Eighteen.
  2. **The body drives a panic.** Criterion iterates what it is
     given, so a `#[should_panic]` test keeps its body inline —
     twelve of these, among them `fonts_tests`'s re-entrancy
     regression. `metric_cache_tests`'s twin is the thirteenth: it
     catches its own panic under `catch_unwind` instead of
     declaring it, but like its sibling it holds the `FONT_SYSTEM`
     write guard across a deliberate re-entrant acquire, so an
     iteration measures the refusal and never reaches the code.
     Thirteen.
  3. **The enumeration is the runtime, not the primitive.**
     `test_color_to_float_round_trips_through_new_f32` sweeps 16.7
     million byte quads; the number would be the loop.
     `test_clone_deep_chain_does_not_exhaust_the_stack` builds a
     50,000-node chain inside a thread spawned with a 256 KiB
     stack, where the stack size is the assertion. Two.
  4. **The subject is the repository, not a value a function
     returned.** `test_above_test_modules_knows_every_shape_in_this_tree`
     reads several hundred files per call. One.

  **Reading a file is not by itself a reason.**
  `do_production_code_returns_code_without_prose` and
  `do_shape_testament_map_has_no_unknown_shapes` each read one file
  and are benched: after the first iteration what moves is the parse,
  not the disk. Their entries say so, so the number is read as I/O
  rather than mistaken for a classifier's.

  **The other 31 are not opt-outs; they are unconverted.** Nothing
  about them forbids the split — they are ordinary unit tests over
  pure functions, written beside the convention rather than under
  it: 20 in `mutator_builder_tests`, the seven `test_factor_*` in
  `region_params_tests`, the three camera-invariance tests in
  `camera_tests`, and `test_clone_preserves_depth_first_pre_order`
  in `arena_utils_tests`. Converting them is a §B8 two-file change
  each, and a judgment about what earns a criterion row rather than
  unfinished #44 work — #44's subject was the 173 `do_*()` bodies
  with no entry, and that set is closed. Naming them here is what keeps
  the four classes above from quietly growing a fifth that means
  "the author did not think it was worth it". A test that reaches
  for the opt-out and is none of the four is in this bucket, and
  should say so rather than write down a new reason.
- **Renaming or deleting a `do_*()` is a two-file change.**
  `benches/test_bench.rs` imports them by path. The bench file is
  not compiled under `cfg(test)`, so `cargo test` will not tell you it
  has drifted. `./test.sh` will: it ends with
  `cargo check --workspace --benches`, which compiles the bench
  targets without executing anything — the only mechanism left, since
  `AGENTS.md` forbids `cargo bench` and `./test.sh --bench` to the
  agents who write most of this code. Update both files in the same
  commit.
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
- **Update doc comments when you change behavior.** A doc comment
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
