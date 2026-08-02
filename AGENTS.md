# AGENTS.md

Rules for any automated agent working in this repository, whichever
harness it runs under. Read this first, then `CLAUDE.md` for the
project's working agreements and the reading list in it.

## Never run benchmarks

**Do not run benchmarks. Run the tests.**

That means: no `cargo bench`, no `./bench.sh`, and no `./test.sh
--bench`. There is no task in this repository that requires an agent to
execute a benchmark, and they are expensive on shared hardware.

`./test.sh` is the gate. It runs the full suite across both crates and
then type-checks `wasm32-unknown-unknown`, so cross-platform drift fails
the run.

Three consequences worth spelling out, because each has been reached for
in good faith:

- **Changing benchmark code is fine; executing it is not.** Adding or
  moving a `benches/` entry — which `lib/baumhard/CONVENTIONS.md` §B3
  requires alongside a new primitive — is a static change. `cargo check
  --benches` proves the target still compiles. That is sufficient, and
  `./test.sh` runs it for you (`cargo check --workspace --benches`), so
  a renamed `do_*()` fails the gate rather than waiting for a benchmark
  run nobody here is allowed to do.

- **Do not make performance claims.** `lib/baumhard/CONVENTIONS.md` §B7
  requires a main-against-main control row for any number, and you will
  not have one. State a change that removes work as a structural fact
  visible in the diff — "one parse removed from the load path", "one
  allocation per keypress removed" — never as a measured win, and never
  with a percentage.

- **A number without a control is worse than no number.** Measured on
  this hardware, main-against-main control runs on *identical code*
  swing ±10–25% at p=0.00. Any figure produced without a control row is
  indistinguishable from that noise, so publishing one asserts something
  the measurement cannot support.

If a task appears to genuinely require a benchmark run, stop and ask the
maintainer rather than running one.
