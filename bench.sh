#!/usr/bin/env bash
# The workspace's benchmark harness is baumhard's alone
# (`lib/baumhard/benches/test_bench.rs`); TEST_CONVENTIONS §T2.3.
# Naming `-p mandala` here only bought a bench-profile rebuild of a
# crate with no bench targets.
#
# This is the single definition of "run the benchmarks":
# `./test.sh --bench` calls this script rather than repeating the
# line below.
#
# Maintainers only: AGENTS.md forbids automated agents from running
# this. To prove the bench targets still compile — which is all an
# agent needs — `./test.sh` type-checks them on every run.
set -euo pipefail

cargo bench -p baumhard
