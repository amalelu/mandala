//! Integration tests that exercise the console against the canonical
//! `testament.mindmap.json` fixture. Split by concern so each file
//! stays focused on one area of the console's surface.
//!
//! - [`fixtures`] — shared `load_test_doc` / `select_first_edge` /
//!   `run` helpers.
//! - [`grapheme`] — grapheme-cluster cursor invariants.
//! - [`state`] — `ConsoleState` shape smoke tests.
//! - [`commands`] — per-command execution.
//! - [`wheel_dispatch`] — `AcceptsWheelColor` per-target dispatch.
//! - [`apply_kvs`] — `apply_kvs` aggregation behavior.
//! - [`multi_fanout`] — multi-selection fanout + trait dispatcher.
//! - [`applicability`] — per-command `is_applicable` predicates.
//! - [`completion`] — completion engine.

mod apply_kvs;
mod applicability;
mod clipboard;
mod commands;
mod completion;
mod fixtures;
mod grapheme;
mod multi_fanout;
mod state;
mod wheel_dispatch;
