//! Shared constants used across multiple console command files.
//!
//! String literals used by only one file live at the top of that
//! file — see `renderer::rebuild_console_overlay_buffers` for prompt
//! glyphs and `app::handle_console_key` for key-name strings. Only
//! values genuinely shared across 2+ files end up here so a typo
//! can't drift between them.

// ---------------------------------------------------------------
// CSS theme-variable references
// ---------------------------------------------------------------
//
// Resolved at scene-build time by `baumhard::util::color::resolve_var`.
// Used by the `color` command's chip row and the theme-swap trigger.

pub const VAR_ACCENT: &str = "var(--accent)";
pub const VAR_EDGE: &str = "var(--edge)";
pub const VAR_FG: &str = "var(--fg)";

// ---------------------------------------------------------------
// Edge type names
// ---------------------------------------------------------------
//
// Used in the `edge` command's enum parsing, the `predicates`
// module's applicability checks, and the backward-compat test
// table.

pub const EDGE_TYPE_CROSS_LINK: &str = "cross_link";
pub const EDGE_TYPE_PARENT_CHILD: &str = "parent_child";
