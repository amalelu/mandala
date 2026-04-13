//! Console palette + glyph constants.
//!
//! Kept in one file so "what does the console look like" is a single
//! grep, and so the commit that reshuffles the look doesn't touch
//! the renderer pipeline code. Colors are authored for a dark canvas
//! (the app's default); a future theme-variant pass may swap them.

use cosmic_text::Color;

// ---- border / chrome --------------------------------------------------

/// Dim desaturated slate-teal — the rounded frame, the side columns,
/// and the corner glyphs. Kept well below full saturation so it reads
/// as chrome, not content.
pub const BORDER_COLOR: Color = Color::rgba(0x3a, 0x55, 0x60, 0xff);

// ---- typography ------------------------------------------------------

/// Default foreground for scrollback output lines — bright enough on
/// a dark background, not so bright that the eye snags.
pub const TEXT_COLOR: Color = Color::rgba(0xd6, 0xd6, 0xd6, 0xff);

/// Input echo line color — same-family-but-dim so the "you typed
/// this" row reads as recent history, not as a directive.
pub const INPUT_ECHO_COLOR: Color = Color::rgba(0x5e, 0x8e, 0x7a, 0xff);

/// Error text + its gutter glyph. Muted red; the gutter does the
/// attention-grabbing, the text itself shouldn't shout.
pub const ERROR_COLOR: Color = Color::rgba(0xe2, 0x8c, 0x8c, 0xff);

/// Accent color — prompt `❯`, selected completion marker, ok-line
/// gutter, kv-key hints. A soft teal-green that plays with the
/// BORDER_COLOR without competing.
pub const ACCENT_COLOR: Color = Color::rgba(0x7f, 0xc3, 0xa5, 0xff);

// ---- glyph constants -------------------------------------------------

/// Prompt glyph — the `❯` at the start of the input line.
pub const PROMPT_GLYPH: &str = "\u{276F}";

/// Cursor block — inserted into the input buffer at the cursor
/// position.
pub const CURSOR_GLYPH: &str = "\u{258C}";

/// Gutter glyph placed in the column immediately right of the left
/// border, colored per-line-kind (accent for Ok, ERROR_COLOR for
/// Err, space for input echo).
pub const GUTTER_GLYPH: &str = "\u{258F}"; // ▏ left one-eighth block

/// Leading marker for the currently-highlighted completion row.
pub const SELECTED_COMPLETION_MARKER: &str = "\u{25B8} ";

/// Padding for unselected completion rows so their text starts at
/// the same x as the selected row's post-marker text.
pub const UNSELECTED_COMPLETION_MARKER: &str = "  ";

// ---- scrollback dimming ----------------------------------------------

/// Oldest-row alpha when we progressively dim older scrollback rows.
/// Trailing row is at full `0xff`; rows further back lerp linearly
/// toward this floor.
pub const SCROLLBACK_MIN_ALPHA: u8 = 0x6e;
