// SPDX-License-Identifier: MPL-2.0

//! Integration tests that exercise the console against the canonical
//! `testament.mindmap.json` fixture. Split by concern so each file
//! stays focused on one area of the console's surface.
//!
//! - [`fixtures`] — shared `load_test_doc` / `select_first_edge` /
//!   `run` helpers.
//! - [`grapheme`] — grapheme-cluster cursor invariants.
//! - [`state`] — `ConsoleState` shape smoke tests.
//! - [`commands`] — per-command execution.
//! - [`clipboard`] — console-driven cut / copy / paste.
//! - [`wheel_dispatch`] — `AcceptsWheelColor` per-target dispatch.
//! - [`apply_kvs`] — `apply_kvs` aggregation behavior.
//! - [`multi_fanout`] — multi-selection fanout + trait dispatcher.
//! - [`applicability`] — per-command `is_applicable` predicates.
//! - [`completion`] — completion engine.
//! - [`resize_mode_lifecycle`] — end-to-end Default → Resize → Default
//!   driven by `mode resize` / `mode default`. Plan §7.2.4.
//! - [`oracle`] — the differential oracle: signatures, the three
//!   comparisons, and what the signatures deliberately do not see.
//! - [`oracle_corpus`] — the input lines the oracle runs.
//! - [`oracle_expected`] — the answers those lines are pinned to.

mod applicability;
mod apply_kvs;
mod clipboard;
mod commands;
mod completion;
pub(in crate::application::console) mod fixtures;
mod grapheme;
mod multi_fanout;
mod oracle;
mod oracle_corpus;
mod oracle_expected;
mod resize_mode_lifecycle;
mod state;
mod wheel_dispatch;
