// SPDX-License-Identifier: MPL-2.0

//! The border family's declarative grammar.
//!
//! One declaration, read by four surfaces. `border …` writes the
//! per-node config; `canvas border …`, `canvas section-frame
//! [focused] …` and `section frame …` write the map-wide and
//! per-section slots. The vocabulary is identical at every one of
//! them, which is why the tables below are `&'static` slices the
//! sibling levels *name* rather than transcribe — a key added here
//! reaches all four popups, all four usage blocks and all four parse
//! loops in the same edit.
//!
//! Everything that used to be spelled out per surface — the kv key
//! list, the per-key hint table, the value vocabularies, the subverb
//! list, the grouped unknown-subverb listing, the usage literal —
//! is one entry here.

use baumhard::mindmap::border::BORDER_PRESET_ROWS;

use crate::application::console::completion::Completion;
use crate::application::console::spec::{
    bare_words, free, free_words, Bare, Form, Grammar, Key, Slot, Subverb, Word,
};
use crate::application::console::spec::{Vocabulary, Word as W};
use crate::application::console::ConsoleContext;

// ============================================================
// Value vocabularies
// ============================================================

/// The border presets, each carrying the description baumhard's
/// `PRESET_TABLE` gives it, plus the `cycle` sentinel the positional
/// subverb accepts beside them.
///
/// Built at const-fn time from `BORDER_PRESET_ROWS`, so a fifth
/// preset added to baumhard's table reaches the popup, the usage
/// line and the rejection message with its description already
/// attached — the property the previous hand-rolled completer had
/// and the previous hand-written usage literal did not.
pub(crate) const PRESET_WORDS: &[Word] = &{
    const N: usize = BORDER_PRESET_ROWS.len();
    let mut out: [Word; N + 1] = [Word::bare(""); N + 1];
    let mut i = 0;
    while i < N {
        out[i] = Word::new(BORDER_PRESET_ROWS[i].0, BORDER_PRESET_ROWS[i].1);
        i += 1;
    }
    out[N] = Word::new("cycle", "advance to the next preset (wraps)");
    out
};

/// The four sides plus the `all` fan-out selector.
pub(crate) const SIDE_VALUES: &[&str] = &["top", "bottom", "left", "right", "all"];
pub(crate) const SIDE_WORDS: &[Word] = &bare_words::<5>(SIDE_VALUES);

/// The four corners plus the `all` fan-out selector.
pub(crate) const CORNER_VALUES: &[&str] = &["tl", "tr", "bl", "br", "all"];
pub(crate) const CORNER_WORDS: &[Word] = &bare_words::<5>(CORNER_VALUES);

/// The palette fields a `palette` cycle can be pinned to. Mirrors
/// `PaletteField::ALL`.
pub(crate) const FIELDS: &[&str] = &["frame", "background", "text", "title"];
const FIELD_WORDS: &[Word] = &bare_words::<4>(FIELDS);

/// Color preset names mirrored from the `color` verb so
/// `border color=accent` resolves the same way.
pub(crate) const COLOR_PRESETS: &[&str] = &["accent", "edge", "fg", "reset"];
const COLOR_PRESET_WORDS: &[Word] = &bare_words::<4>(COLOR_PRESETS);

/// The sentinel `side`/`corner` take in place of a glyph: restore
/// the surface's current preset's default. The word no user guesses
/// without seeing it, which is why it carries a sentence while the
/// selectors beside it do not.
const RESET_GLYPH: &[Word] = &[W::new("reset", "restore the slot's preset's default glyph")];

const OFF_PALETTE: &[Word] = &[W::new("off", "clear palette cycling")];

/// `off` at a border font slot drops the override. Appended here
/// rather than inside the shared family completer because
/// `font set off` names a family and there is no such family.
const OFF_FONT: &[Word] = &[W::new("off", "clear the font override")];

const VERBOSE_FLAG: &[Word] = &[W::new(
    "verbose",
    "surface the dual color cascade (frame_color vs border.color)",
)];

/// `border palette <TAB>` / `palette=<TAB>` — the document's own
/// palette names.
///
/// Matched case-insensitively while inserting the palette's own
/// spelling, so `palette=CO<TAB>` finds `coral` and tab-accept still
/// writes the name the map stores.
fn palette_rows(ctx: &ConsoleContext, partial: &str) -> Vec<Completion> {
    let partial = &partial.to_ascii_lowercase();
    let mut names: Vec<&str> = ctx.document.mindmap.palettes.keys().map(String::as_str).collect();
    names.sort();
    names
        .into_iter()
        .filter(|n| n.to_ascii_lowercase().starts_with(partial))
        .map(|n| Completion {
            text: n.to_string(),
            display: n.to_string(),
            hint: None,
            font_family: None,
        })
        .collect()
}

/// The console's one font-family vocabulary, borrowed from the
/// `font` verb — see that function's doc for the quoting rule a
/// second copy here once omitted.
fn font_rows(_ctx: &ConsoleContext, partial: &str) -> Vec<Completion> {
    super::super::font::font_family_completions(partial)
}

const PALETTE_VOCAB: Vocabulary = Vocabulary::Rows {
    placeholder: "name",
    rows: palette_rows,
    sentinels: OFF_PALETTE,
};

const FONT_VOCAB: Vocabulary = Vocabulary::Rows {
    placeholder: "family",
    rows: font_rows,
    sentinels: OFF_FONT,
};

// ============================================================
// Keys
// ============================================================

/// The kv vocabulary every border surface speaks, with the one
/// sentence each key is described by. This is the single source the
/// popup hints, the usage block, the `keys:` listing and the parse
/// loop's rejection all read.
pub(crate) const KEYS: &[Key] = &[
    Key::new(
        "preset",
        "light | heavy | double | rounded | custom",
        Vocabulary::Words(PRESET_STYLE_WORDS),
    ),
    Key::new(
        "font",
        "font family for border glyphs (use `font list` for names)",
        FONT_VOCAB,
    ),
    Key::new("size", "border glyph size in points", free("pt")),
    Key::new(
        "color",
        "#hex, var(--name), accent | edge | fg, or 'reset'",
        free_words("#hex|var(--name)", COLOR_PRESET_WORDS),
    ),
    Key::new(
        "palette",
        "palette name to cycle per-glyph colors, or 'off'",
        PALETTE_VOCAB,
    ),
    Key::new(
        "field",
        "frame | background | text | title",
        Vocabulary::Words(FIELD_WORDS),
    ),
    Key::new("padding", "border-to-content padding in pixels", free("px")),
    Key::new("top", SIDE_PATTERN_HINT, free("pattern")),
    Key::new("bottom", SIDE_PATTERN_HINT, free("pattern")),
    Key::new("left", SIDE_PATTERN_HINT, free("pattern")),
    Key::new("right", SIDE_PATTERN_HINT, free("pattern")),
    Key::new("tl", CORNER_GLYPH_HINT, free("glyph")),
    Key::new("tr", CORNER_GLYPH_HINT, free("glyph")),
    Key::new("bl", CORNER_GLYPH_HINT, free("glyph")),
    Key::new("br", CORNER_GLYPH_HINT, free("glyph")),
];

const SIDE_PATTERN_HINT: &str = "side pattern: `prefix(fill)suffix` or atomic";
const CORNER_GLYPH_HINT: &str = "single corner glyph (escapes apply)";

/// The preset names without `cycle`, which is a positional-only
/// sentinel: `border preset=cycle` is not a thing the kv form
/// accepts, and offering it there would promise a value
/// `stage_preset` rejects.
const PRESET_STYLE_WORDS: &[Word] = &{
    const N: usize = BORDER_PRESET_ROWS.len();
    let mut out: [Word; N] = [Word::bare(""); N];
    let mut i = 0;
    while i < N {
        out[i] = Word::new(BORDER_PRESET_ROWS[i].0, BORDER_PRESET_ROWS[i].1);
        i += 1;
    }
    out
};

/// The `side=` filter of the `show` subverb — a key of the readout,
/// not of the border configuration, which is why it is declared
/// beside `KEYS` rather than inside it. The composed kv form does
/// not read it, and the popup at the composed slot does not offer
/// it.
const SHOW_KEYS: &[Key] = &[Key::new(
    "side",
    "filter readout to one side (top|bottom|left|right|all)",
    Vocabulary::Words(SIDE_WORDS),
)];

/// The key names of [`KEYS`], derived at const-fn time — what the
/// composed form declares it reads.
pub(crate) const KEY_NAMES: &[&str] = &{
    const N: usize = KEYS.len();
    let mut out: [&str; N] = [""; N];
    let mut i = 0;
    while i < N {
        out[i] = KEYS[i].name;
        i += 1;
    }
    out
};

/// The composed kv form: every key, all optional.
pub(crate) const COMPOSED: &[Form] = &[Form::opt(KEY_NAMES)];

// ============================================================
// Subverbs
// ============================================================

/// The seven per-field subverbs, shared verbatim by every border
/// surface. All are [`Subverb::gated`]: an unquoted
/// `palette=My Palette` splits into two tokens whose second
/// coincidentally spells a subverb name, so a kv at or before the
/// subverb slot means the line is kv form whatever the positional
/// happens to say.
pub(crate) const POSITIONAL_SUBVERBS: &[Subverb] = &[
    Subverb::bare(
        "preset",
        "per-field",
        "pick light|heavy|double|rounded|custom or `cycle`",
    )
    .taking(&[Form::slots(&[Slot::req(Vocabulary::Words(PRESET_WORDS))])])
    .gated(),
    Subverb::bare(
        "color",
        "per-field",
        "set border color (#hex|var|accent|edge|fg|reset)",
    )
    .taking(&[Form::slots(&[Slot::req(free_words(
        "#hex|var(--name)",
        COLOR_PRESET_WORDS,
    ))])])
    .gated(),
    Subverb::bare("padding", "per-field", "set border padding in pixels")
        .taking(&[Form::slots(&[Slot::req(free("px"))])])
        .gated(),
    Subverb::bare("palette", "per-field", "cycle a palette across glyphs (or `off`)")
        .taking(&[Form::slots(&[Slot::req(PALETTE_VOCAB)]).reading(&["field"])])
        .gated(),
    Subverb::bare(
        "font",
        "per-field",
        "set border glyph font family (with optional size=)",
    )
    .taking(&[Form::slots(&[Slot::req(FONT_VOCAB)]).reading(&["size"])])
    .gated(),
    Subverb::bare("side", "glyphs", "set per-side glyph (top|bottom|left|right|all)")
        .taking(&[Form::slots(&[
            Slot::req(Vocabulary::Words(SIDE_WORDS)),
            Slot::req(free_words("pattern", RESET_GLYPH)),
        ])])
        .gated(),
    Subverb::bare("corner", "glyphs", "set per-corner glyph (tl|tr|bl|br|all)")
        .taking(&[Form::slots(&[
            Slot::req(Vocabulary::Words(CORNER_WORDS)),
            Slot::req(free_words("glyph", RESET_GLYPH)),
        ])])
        .gated(),
];

/// The `preview` staging level's terminators. Siblings of the kv
/// keys: `preview <kv>=…` and `preview commit` / `preview cancel`
/// are the two ways to end a staged edit.
pub(crate) const PREVIEW_TERMINATORS: &[Subverb] = &[
    Subverb::bare(
        "commit",
        "terminator",
        "write the staged preview through and clear the slot",
    ),
    Subverb::bare(
        "cancel",
        "terminator",
        "discard the staged preview, no model write",
    ),
];

/// The per-node verb's own subverbs — the six matched *ahead* of
/// the discriminator, so they stay on offer at a kv-form slot:
/// `border color=#fff preview` really does stage a preview carrying
/// that color, and `border color=#fff on` is refused by `on`'s own
/// message rather than by the discriminator.
const BORDER_OWN_SUBVERBS: &[Subverb] = &[
    Subverb::bare("on", "visibility", "show the border"),
    Subverb::bare("off", "visibility", "hide the border"),
    Subverb::bare("toggle", "visibility", "flip show_frame per node"),
    Subverb::bare(
        "show",
        "readout",
        "print the resolved config (use [side=…] [verbose])",
    )
    .taking(&[Form::slots(&[Slot::opt(Vocabulary::Words(VERBOSE_FLAG))]).reading(&["side"])]),
    Subverb::bare("reset", "visibility", "drop the per-node override"),
    Subverb::nested(
        "preview",
        "staged",
        "stage a preview without writing the model (commit/cancel terminates)",
        &BORDER_PREVIEW,
    ),
];

// ============================================================
// Levels
// ============================================================

/// `border preview …` — the staging level.
pub static BORDER_PREVIEW: Grammar = Grammar {
    label: "border preview",
    subverb_sets: &[PREVIEW_TERMINATORS],
    key_sets: &[KEYS],
    bare: Some(Bare::new("composed", COMPOSED)),
};

/// `border …` — the per-node level.
pub static BORDER: Grammar = Grammar {
    label: "border",
    subverb_sets: &[BORDER_OWN_SUBVERBS, POSITIONAL_SUBVERBS],
    key_sets: &[KEYS, SHOW_KEYS],
    bare: Some(Bare::new("composed", COMPOSED)),
};
