# Test Conventions

## First of all

Like `CODE_CONVENTIONS.md`, this document belongs to the *repository* first
and foremost, not to developers. Tests are a living craft — they evolve with
the codebase, and it is unreasonable to expect that every test will always be
in perfect conformity with these conventions. We much prefer a useful,
off-convention test over a missing one. A later session can always refactor
it into shape.

Break a convention if it makes sense to. Leave a short comment at the local
scope explaining why, and move on.

## Why we test

We test to pin down mutation correctness, loader edge cases, and the
visual-invariant math behind the scene builder (cull rects, border layouts,
palette scroll windows, hit tests). We do **not** test to chase a coverage
percentage, to mock our way around wgpu, or to prove the type system to
itself. A green suite is a covenant that the pieces we've cared enough to
encode still behave the way we expected them to — nothing more, nothing less.

## Where tests live

Two patterns coexist in this repo, and both are intentional.

### §3.1 Inline `#[cfg(test)] mod tests` — the default

Most test modules live inline at the bottom of the source file they're
testing, wrapped in `#[cfg(test)] mod tests { ... }`. This is the default
pattern for anything that doesn't need to be called from outside the test
harness.

Representative exemplars:
- `src/application/document.rs` — 104 inline tests covering hit-testing,
  selection, undo stacks, portal mutations.
- `lib/baumhard/src/mindmap/scene_builder.rs` — 41 inline tests covering
  connection and border rendering math.
- `lib/baumhard/src/mindmap/connection.rs` — 36 inline tests covering anchor
  points, path sampling, Bezier curves.

If you're not sure where a new test should go, put it inline.

### §3.2 `pub mod tests;` trees — baumhard-only, benchmark-reusable

In `lib/baumhard`, certain modules expose tests through a dedicated
`tests` subdirectory declared as `pub mod tests;` rather than
`#[cfg(test)] mod tests`. This is deliberate and load-bearing: the
benchmark harness at `lib/baumhard/benches/test_bench.rs` imports the
test bodies so criterion can run them as micro-benchmarks.

If the module were gated with `#[cfg(test)]`, the benchmark binary (which
is not compiled under `cfg(test)`) couldn't reach it. So we make the test
module `pub`, and rely on the `do_*()`/`test_*()` naming split (see §4)
to keep the `#[test]`-annotated wrapper functions out of the benchmark
import path.

Don't "fix" the missing `#[cfg(test)]`. It is the way.

Exemplars:
- `lib/baumhard/src/util/tests/` — geometry, color, grapheme, arena, primes.
- `lib/baumhard/src/gfx_structs/tests/` — region, tree, model, area, walker.
- `lib/baumhard/src/core/tests/` — primitives (ranges, color regions).

### §3.3 Cross-crate rule

The `pub mod tests;` pattern is baumhard-only. The `mandala` crate has no
benchmark harness, so there's no reason to ever reach for it there — every
mandala-side test should live inline.

## Naming

- **Test functions:** `test_<topic>_<specific_case>`. Lowercase snake_case.
  Examples: `test_hit_test_direct_hit`, `test_portal_label_gap_reuse`.

- **Benchmark-reusable bodies:** `pub fn do_<topic>_<case>()`, with a
  one-line `#[test] fn test_<topic>_<case>()` wrapper that calls it. The
  `do_*` function is `pub` so it's reachable from `benches/test_bench.rs`.
  Exemplar: `lib/baumhard/src/util/tests/geometry_tests.rs`:

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

- **Fixture helpers:** free functions inside `mod tests { ... }`, named by
  what they return. See `load_test_doc` / `load_test_tree` / `test_map_path`
  at `src/application/document.rs:1951`.

- **Lazy-static test data:** `TEST_<NOUN>` in SCREAMING_SNAKE. Pattern is
  already in `lib/baumhard/src/gfx_structs/tests/region_tests.rs` and
  `lib/baumhard/src/core/tests/primitives_tests.rs`.

## Fixtures and test data

- `maps/testament.mindmap.json` is the canonical fixture. Prefer loading it
  through the `load_test_doc()` / `load_test_tree()` helpers over
  hand-constructing `MindMap` literals. A test that exercises the loader
  path at the same time as the feature under test catches more regressions
  for free.

- Build fixture paths with `env!("CARGO_MANIFEST_DIR")` so tests work
  regardless of the working directory. `src/application/document.rs:1951`
  is the pattern.

- For heavy repeated data (large region tables, primitive truth tables),
  declare it once per test module via `lazy_static!`. See
  `primitives_tests.rs::OVERLAPS_TEST` and `region_tests.rs` for the shape.

- New fixture mindmaps go in `maps/` with a `*.mindmap.json` suffix so the
  build-script walker and the CLI loader both handle them uniformly.

## Assertions

- **Plain `assert!` / `assert_eq!` are the house style.** No
  `pretty_assertions`, no `insta`, no snapshot testing. If a diff is hard
  to read, improve the values you're comparing, not the assertion macro.

- **Floating-point and glyph-space geometry** use `almost_equal` and
  `almost_equal_vec2` from `lib/baumhard/src/util/geometry.rs`. Pick an
  epsilon that matches the scale of the value under test — the helpers
  default to a reasonable scale-invariant tolerance, but don't trust them
  blindly for very large or very small values.

- **Panics:** `#[should_panic(expected = "...")]` is fine for validation
  error paths where the message is load-bearing (see
  `region_tests.rs`'s RegionParams prime-dimension tests). Otherwise
  prefer `assert!(matches!(result, Err(_)))` or explicit
  `assert_eq!(result.unwrap_err(), ...)`.

## Benchmark-reuse constraint

Any `do_*()` function exported through a `pub mod tests;` tree is part of
`lib/baumhard/benches/test_bench.rs`'s surface. Renaming or removing one is
a two-file change — update the benchmark imports in the same commit. This
is not enforced by the compiler (the benchmark file won't be built unless
you run `cargo bench` or `./test.sh --bench`), so keep them in sync by
convention.

## When to add a regression test

If you're writing code that falls into any of these buckets, write the
test at the same time:

- A new mutation or undo variant (especially if it touches the tree
  structure or reparenting logic).
- A new loader path or loader edge case.
- A scene-builder math path that decides where a glyph lands.
- A reported bug — write the test first, name it after the symptom, then
  fix the bug.
- Anything you catch yourself re-verifying by hand across sessions. That
  manual check is a regression test begging to be written.

## GPU and renderer testing

We do **not** exercise `Renderer::new`, the wgpu device/queue, cosmic-text
rasterization, or any live GPU code path in tests. The renderer is
constructed once at app startup and never in a test harness. Trying to
stand up a headless wgpu instance for tests is a tar pit.

What we *do* test from the renderer is the pure layout math — cull rects,
palette frame sizing, palette scroll windows, sacred-border layout,
backdrop alignment. See `application::renderer::tests` for the exemplar.
If a bug requires a live wgpu device to reproduce, note it in
`ROADMAP.md`'s "What needs work" list rather than building a headless
harness to chase it.

## What we deliberately don't do

These aren't accidental omissions — each is a decision. Don't re-litigate
them without a strong reason.

- **No `pretty_assertions`, no `insta`, no snapshot testing.** Plain
  assertions, always.
- **No `mockall` or hand-rolled trait mocks.** Tests construct real objects
  and real data. The codebase is small enough that this works.
- **No async test harness.** The app is single-threaded (see `CLAUDE.md`).
  Tests should stay that way.
- **No `wasm-bindgen-test`.** Cross-platform logic is tested once on native.
  WASM-specific code paths are validated by `build.sh` compiling for
  `wasm32-unknown-unknown`, not by running tests under wasm.
- **No GPU / live-wgpu test infrastructure.** See the section above.
- **No CI yet.** `./test.sh` is the covenant — run it before committing.

## Running the suite

- `./test.sh` — run the full suite across `baumhard` and `mandala` and
  print a test count at the end.
- `./test.sh --coverage` — run the suite under `cargo-llvm-cov`. Requires
  `cargo install cargo-llvm-cov` once. Produces HTML at
  `target/llvm-cov/html/index.html` and LCOV at
  `target/llvm-cov/lcov.info`.
- `./test.sh --lint` — also run `cargo fmt --check` and
  `cargo clippy --workspace --all-targets`. Both are advisory — they print
  their diagnostics but never fail the run.
- `./test.sh --bench` — also run `cargo bench` after tests pass.
- `cargo test -p baumhard --lib <pattern>` or
  `cargo test -p mandala --lib <pattern>` — run a targeted subset while
  iterating on a single feature.

## Breaking these conventions

If a test genuinely needs a different shape than what's described here,
write it, leave a short comment at the local scope explaining why, and
move on. Conventions serve the codebase; the codebase does not serve the
conventions.
