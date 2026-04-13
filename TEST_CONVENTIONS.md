# Test Conventions

## §T0 Why we test

The test suite is how we know the foundation is intact. When the
fundamentals — mutation correctness, Unicode handling, geometry,
region indexing, undo round-trips, loader edges — pass on every
commit, we can move quickly and confidently on top of them. When they
do not, every change above them is suspect.

We test heavily, and we test fundamentals first. We do not test to
chase a coverage percentage, to mock our way around wgpu, or to prove
the type system to itself. A green suite is a covenant: the pieces we
have cared enough to encode still behave the way we expected them to.

## §T1 Test fundamentals first, and test them heavily

Fundamentals get the heaviest coverage in the repository. These are
the surfaces every other piece of code rests on, and a regression in
one of them ripples into everything above it.

The fundamentals are:

- **Mutations and undo round-trips** — every `GfxMutator` variant,
  every `UndoAction` variant, every forward-and-back. A mutation that
  cannot be undone correctly is a corruption waiting to happen.
- **Unicode and grapheme handling** — every primitive in
  `lib/baumhard/src/util/grapheme_chad.rs`. Emoji, combining marks,
  regional indicators, ZWJ sequences. Test the surprising inputs
  before the obvious ones.
- **Geometry and region indexing** — `almost_equal` /
  `almost_equal_vec2`, `RegionIndexer`, the region params math.
  Every spatial-index assertion is a frame that does not stutter under
  selection.
- **Loader edges** — every shape `.mindmap.json` can take, including
  malformed ones, missing fields, and unknown `edge_type` values.
- **Platform-shared logic** — gesture math, viewport math, anything
  that has to behave identically on native and WASM.

Features built on top of the fundamentals get coverage proportional to
their user impact, but **never less than the happy path plus each
distinct error path**. When in doubt, write the test (§T12).

## §T2 Where tests live

Two patterns coexist, and both are intentional.

### §T2.1 Inline `#[cfg(test)] mod tests` — the default

Most test modules live inline at the bottom of the source file they
test, wrapped in `#[cfg(test)] mod tests { ... }`. This is the default
pattern for anything that does not need to be called from outside the
test harness.

Representative exemplars:
- `src/application/document.rs` — fundamentals coverage of
  hit-testing, selection, undo stacks, portal mutations.
- `lib/baumhard/src/mindmap/scene_builder.rs` — connection and border
  rendering math.
- `lib/baumhard/src/mindmap/connection.rs` — anchor points, path
  sampling, Bezier curves.

If you are not sure where a new test should go, put it inline.

### §T2.2 `pub mod tests;` trees — Baumhard-only, benchmark-reusable

In `lib/baumhard`, certain modules expose tests through a dedicated
`tests` subdirectory declared as `pub mod tests;` rather than
`#[cfg(test)] mod tests`. This is deliberate and load-bearing: the
benchmark harness at `lib/baumhard/benches/test_bench.rs` imports the
test bodies so criterion can run them as micro-benchmarks.

If the module were gated with `#[cfg(test)]`, the benchmark binary
(which is not compiled under `cfg(test)`) could not reach it. So we
make the test module `pub` and rely on the `do_*()` / `test_*()`
naming split (see §T3) to keep `#[test]`-annotated wrapper functions
out of the benchmark import path.

Do not "fix" the missing `#[cfg(test)]`. It is the way.

Exemplars:
- `lib/baumhard/src/util/tests/` — geometry, color, grapheme, arena,
  primes.
- `lib/baumhard/src/gfx_structs/tests/` — region, tree, model, area,
  walker.
- `lib/baumhard/src/core/tests/` — primitives (ranges, color regions).

### §T2.3 Cross-crate rule

The `pub mod tests;` pattern is Baumhard-only. The `mandala` crate has
no benchmark harness, so there is no reason to ever reach for it
there — every mandala-side test lives inline.

## §T3 Naming

- **Test functions:** `test_<topic>_<specific_case>`. Lowercase
  snake_case. Examples: `test_hit_test_direct_hit`,
  `test_portal_label_gap_reuse`.

- **Benchmark-reusable bodies:** `pub fn do_<topic>_<case>()`, with a
  one-line `#[test] fn test_<topic>_<case>()` wrapper that calls it.
  The `do_*` function is `pub` so it is reachable from
  `benches/test_bench.rs`. Exemplar from
  `lib/baumhard/src/util/tests/geometry_tests.rs`:

  ```rust
  #[test]
  fn test_90_deg_rotation() {
      do_90_deg_rotation();
  }

  pub fn do_90_deg_rotation() {
      let point = Vec2::new(1.0, 0.0);
      let pivot = Vec2::new(0.0, 0.0);
      let rotated = clockwise_rotation_around_pivot(point, pivot, 90.0);
      let expected = Vec2::new(0.0, -1.0);
      assert!(almost_equal_vec2(rotated, expected));
  }
  ```

- **Fixture helpers:** free functions inside `mod tests { ... }`,
  named by what they return. See `load_test_doc` / `load_test_tree` /
  `test_map_path` in `src/application/document.rs`.

- **Lazy-static test data:** `TEST_<NOUN>` in SCREAMING_SNAKE. Pattern
  is in `lib/baumhard/src/gfx_structs/tests/region_tests.rs` and
  `lib/baumhard/src/core/tests/primitives_tests.rs`.

## §T4 Fixtures and test data

- `maps/testament.mindmap.json` is the canonical fixture. Prefer
  loading it through the `load_test_doc()` / `load_test_tree()`
  helpers over hand-constructing `MindMap` literals. A test that
  exercises the loader path at the same time as the feature under
  test catches more regressions for free.

- Build fixture paths with `env!("CARGO_MANIFEST_DIR")` so tests work
  regardless of working directory. The pattern lives in
  `src/application/document.rs`'s test module.

- For heavy repeated data (large region tables, primitive truth
  tables), declare it once per test module via `lazy_static!`. See
  `primitives_tests.rs::OVERLAPS_TEST` and `region_tests.rs` for the
  shape.

- New fixture mindmaps go in `maps/` with a `*.mindmap.json` suffix so
  the build-script walker and the CLI loader both handle them
  uniformly.

## §T5 Assertions

- **Plain `assert!` / `assert_eq!` are the house style.** No
  `pretty_assertions`, no `insta`, no snapshot testing. If a diff is
  hard to read, improve the values you are comparing, not the
  assertion macro.

- **Floating-point and glyph-space geometry** use `almost_equal` and
  `almost_equal_vec2` from `lib/baumhard/src/util/geometry.rs`. Pick
  an epsilon that matches the scale of the value under test — the
  helpers default to a reasonable scale-invariant tolerance, but do
  not trust them blindly for very large or very small values.

- **Panics:** `#[should_panic(expected = "...")]` is fine for
  validation error paths where the message is load-bearing (see
  `region_tests.rs`'s RegionParams prime-dimension tests). Otherwise
  prefer `assert!(matches!(result, Err(_)))` or explicit
  `assert_eq!(result.unwrap_err(), ...)`.

## §T6 Benchmark-reuse discipline

Any `do_*()` function exported through a `pub mod tests;` tree is part
of `lib/baumhard/benches/test_bench.rs`'s surface. Renaming or
removing one is a two-file change — update the benchmark imports in
the same commit. This is not enforced by the compiler (the benchmark
file is not built unless you run `cargo bench` or `./test.sh
--bench`), so keep them in sync by convention.

## §T7 When to add a regression test

Any of these triggers a test in the same commit:

- A new mutation or undo variant — especially if it touches tree
  structure or reparenting logic.
- A new loader path or loader edge case.
- A scene-builder math path that decides where a glyph lands.
- A reported bug — write the test first, name it after the symptom,
  then fix the bug.
- Anything you catch yourself re-verifying by hand across sessions.
  That manual check is a regression test begging to be written.
- Any change to a fundamental (§T1). Touching a fundamental without
  adding to its test surface is technical debt, and §4 of
  [`CODE_CONVENTIONS.md`](./CODE_CONVENTIONS.md) does not tolerate
  technical debt.

## §T8 GPU and renderer testing

We do **not** exercise `Renderer::new`, the wgpu device/queue,
cosmic-text rasterization, or any live GPU code path in tests. The
renderer is constructed once at app startup and never in a test
harness. Standing up a headless wgpu instance for tests is a tar pit.

What we *do* test from the renderer is the pure layout math — cull
rects, palette frame sizing, palette scroll windows, sacred-border
layout, backdrop alignment. See `application::renderer::tests` for the
exemplar. If a bug requires a live wgpu device to reproduce, note it
in `ROADMAP.md`'s "What needs work" list rather than building a
headless harness to chase it.

## §T9 Mobile and WASM

The cross-platform reality (see
[`CODE_CONVENTIONS.md §2`](./CODE_CONVENTIONS.md) and §3) shapes how
we test for non-native targets.

- **Tests run on native.** `./test.sh` exercises the entire suite
  against the host target. Platform-shared logic that passes on
  native is trusted to pass on WASM, because the logic does not
  depend on the platform.
- **`build.sh` validates the wasm32 cross-compile.** A change that
  builds for native but breaks `wasm32-unknown-unknown` is a
  regression caught at build time, not runtime.
- **Pure platform-shared logic must be reachable without a wgpu
  instance.** Touch-input math, gesture recognition, viewport math,
  hit-test math — anything that has to behave identically on a
  desktop and a phone — lives in functions that take their inputs as
  plain values and return plain values. This is an architectural
  constraint cross-referenced in
  [`CODE_CONVENTIONS.md §3`](./CODE_CONVENTIONS.md): code that only
  works inside a `wgpu::Device` cannot be tested for cross-platform
  correctness, and code that cannot be tested for cross-platform
  correctness will eventually diverge between platforms.
- **No `wasm-bindgen-test`.** Cross-platform logic is tested once on
  native; WASM-specific code paths are validated by `build.sh`
  compiling for `wasm32-unknown-unknown`.

## §T10 What we deliberately don't do

These are not accidental omissions — each is a decision. Do not
re-litigate them without a strong reason.

- **No `pretty_assertions`, no `insta`, no snapshot testing.** Plain
  assertions, always.
- **No `mockall` or hand-rolled trait mocks.** Tests construct real
  objects and real data. The codebase is small enough that this works.
- **No async test harness.** The app is single-threaded (see
  `CLAUDE.md`). Tests stay that way.
- **No `wasm-bindgen-test`.** See §T9.
- **No GPU / live-wgpu test infrastructure.** See §T8.
- **No CI yet.** `./test.sh` is the covenant — run it before
  committing.

## §T11 Running the suite

- `./test.sh` — full suite across `baumhard` and `mandala`; prints a
  test count at the end.
- `./test.sh --coverage` — runs under `cargo-llvm-cov` (install with
  `cargo install cargo-llvm-cov`). HTML at
  `target/llvm-cov/html/index.html`, LCOV at
  `target/llvm-cov/lcov.info`.
- `./test.sh --lint` — also runs `cargo fmt --check` and
  `cargo clippy --workspace --all-targets`. Both advisory; review
  output but they do not fail the run.
- `./test.sh --bench` — also runs `cargo bench` after tests pass.
- `cargo test -p baumhard --lib <pattern>` or
  `cargo test -p mandala --lib <pattern>` — targeted subset while
  iterating.
- `cargo doc -p baumhard --no-deps` — render the library docs and
  spot-check that every `pub` item has the doc comment
  [`lib/baumhard/CONVENTIONS.md §B9`](./lib/baumhard/CONVENTIONS.md)
  requires.

## §T12 Test aggressively

When in doubt, write the test. An untested fundamental is technical
debt (§T1, §T7), and a featureful path with no error-case coverage is
half a feature.

The bias is: more tests, sooner. A test that turns out redundant is
cheap to delete; a regression that ships because no test was written
is expensive to recover from. Test more than feels necessary —
fundamentals especially — and let the suite be the thing that lets
the next session move quickly.
