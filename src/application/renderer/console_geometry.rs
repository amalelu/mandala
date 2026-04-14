//! Console overlay geometry types and pure-function layout math.
//!
//! These types are the pre-layout interchange between the app event
//! loop and the renderer's `rebuild_console_overlay_buffers`. The
//! `compute_console_frame_layout` helper turns viewport dimensions +
//! console state into a fully-resolved `ConsoleFrameLayout` the
//! shaper consumes. Kept free of cosmic-text and wgpu so unit tests
//! can construct geometries trivially.

/// Pre-layout console data handed from the app event loop to the
/// renderer every time the console state changes. The renderer turns
/// it into cosmic-text buffers in `rebuild_console_overlay_buffers`.
/// Kept as a plain struct (no rendering primitives) so unit tests
/// can construct one trivially.
///
/// Layout shape: a bottom-anchored strip with (bottom → top)
/// prompt line → completion popup → scrollback region. The
/// scrollback shows the most recent N output lines; the completion
/// popup is empty unless the user pressed Tab.
///
/// Styling (`font_family`, `font_size`) is threaded in from the
/// user config. The renderer stays dumb about where those values
/// came from — it just draws what the geometry says.
#[derive(Clone, Debug)]
pub struct ConsoleOverlayGeometry {
    /// Input buffer text, rendered after the `❯ ` prompt glyph.
    pub input: String,
    /// Grapheme-cluster index of the cursor. The renderer converts
    /// this to a byte offset via
    /// `baumhard::util::grapheme_chad::find_byte_index_of_grapheme`
    /// so the prompt-line `split_at` lands on a grapheme boundary
    /// even for ZWJ emoji / combining marks.
    pub cursor_grapheme: usize,
    /// Scrollback lines, oldest first. Only the trailing
    /// `MAX_CONSOLE_SCROLLBACK_ROWS` are drawn; anything above scrolls
    /// off the top.
    pub scrollback: Vec<ConsoleOverlayLine>,
    /// Completion candidates. Empty when the popup is closed.
    pub completions: Vec<ConsoleOverlayCompletion>,
    /// Which completion is highlighted. `None` when `completions` is
    /// empty. Index into `completions` otherwise.
    pub selected_completion: Option<usize>,
    /// Font family name passed to cosmic-text via
    /// `Attrs::new().family(Family::Name(..))`. Empty string means
    /// "use cosmic-text's default family", which lets cosmic-text's
    /// own fallback chain resolve it.
    pub font_family: String,
    /// Font size in pixels. The whole overlay scales with this value;
    /// row height, frame extents, and border repetition counts are
    /// all derived from it.
    pub font_size: f32,
}

/// One line in the scrollback, carrying its kind so the renderer can
/// color input echoes, normal output, and errors differently.
#[derive(Clone, Debug)]
pub struct ConsoleOverlayLine {
    pub text: String,
    pub kind: ConsoleOverlayLineKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsoleOverlayLineKind {
    /// Echo of a user-entered command (e.g. `> anchor set from top`).
    Input,
    /// Normal output line from a successful command.
    Output,
    /// Error output from a failed command.
    Error,
}

/// One completion candidate: the replacement text plus an optional
/// dim hint printed to the right (e.g. the command's summary).
#[derive(Clone, Debug)]
pub struct ConsoleOverlayCompletion {
    pub text: String,
    pub hint: Option<String>,
}

/// Pure-function output of the console-overlay layout pass. Holds
/// the derived screen-space dimensions for the console frame so the
/// backdrop rectangle and the border-glyph positions agree exactly.
/// Extracted to a plain struct so unit tests can verify the
/// alignment invariant without constructing a full `Renderer`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConsoleFrameLayout {
    pub left: f32,
    pub top: f32,
    pub frame_width: f32,
    pub frame_height: f32,
    pub font_size: f32,
    pub char_width: f32,
    pub row_height: f32,
    pub inner_padding: f32,
    /// How many scrollback rows fit inside the frame — clamped to
    /// `MAX_CONSOLE_SCROLLBACK_ROWS` and the available vertical
    /// space.
    pub scrollback_rows: usize,
    /// How many completion rows are drawn. 0 when the popup is
    /// closed. Completions sit directly above the prompt line.
    pub completion_rows: usize,
}

/// Maximum number of scrollback lines rendered. The scrollback
/// vector itself can grow unboundedly in memory, but only the
/// trailing N lines ever reach the screen.
pub const MAX_CONSOLE_SCROLLBACK_ROWS: usize = 12;

/// Maximum number of completion candidates drawn in the popup above
/// the prompt.
pub const MAX_CONSOLE_COMPLETION_ROWS: usize = 8;

// Prompt / cursor / completion-marker glyphs live in
// `console::visuals` with the rest of the palette. Bring them into
// this module's scope via `use` from the renderer body.

/// How many rows of `│` belong in each side column of the console
/// frame. The side column sits between the top border (at
/// `y = top`, height `font_size`) and the bottom border (at
/// `y = top + frame_height`), so it spans `frame_height - font_size`
/// pixels at `row_height` per row. Rounded up so the column always
/// reaches the bottom corner.
pub(super) fn side_row_count(frame_height: f32, font_size: f32, row_height: f32) -> usize {
    let span = (frame_height - font_size).max(0.0);
    (span / row_height).ceil() as usize
}

/// Scale alpha linearly between `min` and `max` by `t in [0, 1]`.
/// Used to dim older scrollback rows.
pub(super) fn lerp_alpha(min: u8, max: u8, t: f32) -> u8 {
    let t = t.clamp(0.0, 1.0);
    let v = min as f32 + (max as f32 - min as f32) * t;
    v.round().clamp(0.0, 255.0) as u8
}

/// Rebuild a `cosmic_text::Color` with a new alpha byte, keeping
/// RGB. Cosmic-text's `Color` is `[R, G, B, A]` packed into a u32,
/// so we unpack via the getter accessors and re-pack.
pub(super) fn with_alpha(c: cosmic_text::Color, a: u8) -> cosmic_text::Color {
    cosmic_text::Color::rgba(c.r(), c.g(), c.b(), a)
}

impl ConsoleFrameLayout {
    /// Screen-space rectangle covered by the opaque backdrop. Matches
    /// the border-glyph bounds exactly: the top border row sits at
    /// `y = top`, the bottom border row extends to
    /// `y = top + frame_height + font_size`, and the left / right
    /// columns span `[left, left + frame_width]` horizontally.
    pub fn backdrop_rect(&self) -> (f32, f32, f32, f32) {
        (
            self.left,
            self.top,
            self.frame_width,
            self.frame_height + self.font_size,
        )
    }

    /// Y offset of the prompt line's baseline. Sits directly below
    /// the scrollback and completion regions. Kept consistent with
    /// the scrollback / completion placement in
    /// `rebuild_console_overlay_buffers` so rows never overlap.
    pub fn prompt_y(&self) -> f32 {
        self.top
            + self.font_size
            + self.inner_padding
            + self.row_height * (self.scrollback_rows + self.completion_rows) as f32
    }
}

/// Build the four border strings (top, bottom, left_column,
/// right_column) for the console frame using the rounded
/// `BorderGlyphSet` preset: `╭─...─╮`, `│` stacked, `╰─...─╯`.
///
/// `cols` is the total width of the top/bottom rows in monospace
/// character cells, including both corners — so `cols >= 2` is
/// required for the corners to render. `rows` is the height of each
/// side column in rows, exclusive of the corner rows (which belong
/// to the top/bottom border strings).
///
/// Returns `(top, bottom, left, right)`. The box-drawing presets
/// have `left == right`, so both side strings are the same — the
/// caller positions them at different x offsets.
pub fn build_console_border_strings(
    cols: usize,
    rows: usize,
) -> (String, String, String, String) {
    let glyphs = baumhard::mindmap::border::BorderGlyphSet::box_drawing_rounded();
    let top = glyphs.top_border(cols);
    let bottom = glyphs.bottom_border(cols);
    let side = glyphs.side_border(rows);
    (top, bottom, side.clone(), side)
}

/// Compute the screen-space layout for the console overlay from a
/// `ConsoleOverlayGeometry` and the current screen dimensions. Pure
/// function — no GPU or font-system access. Called by
/// `rebuild_console_overlay_buffers` to derive positions for the
/// backdrop rect, border glyphs, prompt, scrollback, and completion
/// popup. Unit tests use it directly to assert the backdrop-vs-border
/// alignment invariant and the scrollback/completion row math.
///
/// The console is a bottom-anchored strip: rows run (bottom → top)
/// **prompt → completion popup (if any) → scrollback region**. The
/// frame grows upward from the bottom of the window as scrollback or
/// completions accumulate, up to the built-in caps.
pub fn compute_console_frame_layout(
    geometry: &ConsoleOverlayGeometry,
    screen_width: f32,
    screen_height: f32,
) -> ConsoleFrameLayout {
    let font_size = geometry.font_size.max(4.0);
    // `0.6` is a conservative monospace advance — cosmic-text's
    // fallback chain lands on a proportional font by default, but the
    // characters we render (`╭ ─ ╮ │ ╰ ╯ ❯ ▌ ▸ ▏`) all advance by
    // roughly font_size * 0.6. Tweaking this value visibly shifts
    // the column count; keep it in sync with the real advance if
    // you swap the default font.
    let char_width = font_size * 0.6;
    let inner_padding: f32 = 8.0;
    let row_height = font_size + 2.0;

    let scrollback_rows = geometry
        .scrollback
        .len()
        .min(MAX_CONSOLE_SCROLLBACK_ROWS);
    let completion_rows = geometry
        .completions
        .len()
        .min(MAX_CONSOLE_COMPLETION_ROWS);

    let prompt_budget = font_size * 1.4;
    // Frame vertical budget: top border + inner pad + scrollback +
    // completions + prompt row + inner pad. Bottom border sits
    // outside `frame_height`; see `backdrop_rect`.
    let frame_height = font_size
        + inner_padding * 2.0
        + row_height * scrollback_rows as f32
        + row_height * completion_rows as f32
        + prompt_budget;

    // Full-width strip at the bottom of the window. No horizontal
    // clamp: the overlay tracks the window width. An inner margin
    // keeps the border from kissing the screen edge.
    let horizontal_margin = char_width;
    let frame_width = (screen_width - horizontal_margin * 2.0).max(char_width * 4.0);
    let left = horizontal_margin;
    let top = (screen_height - frame_height - inner_padding - font_size)
        .max(inner_padding);

    ConsoleFrameLayout {
        left,
        top,
        frame_width,
        frame_height,
        font_size,
        char_width,
        row_height,
        inner_padding,
        scrollback_rows,
        completion_rows,
    }
}
