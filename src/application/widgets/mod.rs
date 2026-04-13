//! Widget definitions loaded from embedded JSON.
//!
//! First step toward user-authored widgets: today the glyph-wheel
//! color picker's static structure (glyphs, size scales, chip list,
//! copy) lives in `color_picker.json`, loaded once at startup. The
//! pure-function layout math stays in Rust (it depends on measured
//! glyph advances and screen dimensions that JSON can't express).
//!
//! Future sessions can:
//!   - Move the spec under `lib/baumhard/src/widgets/` once the
//!     widget abstraction matures enough to belong in the library.
//!   - Extend the schema with positional primitives so whole widgets
//!     — not just color pickers — can be defined in JSON.
//!   - Swap the `OnceLock`-cached default spec for a runtime load
//!     path that reads user overrides from disk.

pub mod color_picker_widget;
