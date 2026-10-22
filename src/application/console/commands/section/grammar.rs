// SPDX-License-Identifier: MPL-2.0

//! `section …`'s grammar.
//!
//! Two things here are worth naming. First, `frame` is a nested
//! level rather than a special case: the whole border vocabulary
//! hangs off it, and the descent reaches it without `section`
//! knowing anything about borders.
//!
//! Second, `move` and `resize` each declare **two forms**, because
//! their shapes are genuinely alternatives rather than a menu.
//! `section move` takes `dx=`/`dy=` *or* `x=`/`y=`; `section resize`
//! takes the `fill` literal *or* `w=`/`h=`. Bracketing either pair
//! onto one line would document a command the verb refuses, so
//! `help` prints one line per form.
//!
//! The two are not enforced the same way, and the difference is
//! where the alternative is *written*. `resize`'s lives in a slot —
//! only one of its forms declares the `fill` literal — so the engine
//! narrows on it and `section resize fill w=99` is refused by name
//! (`spec::Form::admits_prefix`). `move`'s lives only in its key
//! lists, which an empty positional list admits equally, so the
//! engine offers and accepts their union and `execute_move` refuses
//! the mix by hand, naming both shapes. That handwritten guard is
//! one of the two bespoke semantics issue #27's fix plan names as
//! staying behind the table.
//!
//! This level is also where `KNOWN_VERBS` used to live: a second
//! nine-entry vocabulary beside `VERBS`, in a different order, that
//! the compiler could not hold against it. There is one vocabulary
//! now, and the unknown-subverb listing is derived from it.

use crate::application::console::commands::range_kv::SECTION_KEYS;
use crate::application::console::spec::{free, Form, Grammar, Key, Slot, Subverb, Vocabulary, Word};

use super::frame::grammar::SECTION_FRAME;

/// The two run-handling modes `section text runs=` accepts.
const RUNS_WORDS: &[Word] = &[
    Word::new(
        "preserve",
        "keep per-grapheme styling where the new text supports it",
    ),
    Word::new("clear", "drop every prior run and inherit the first one's style"),
];

/// The `fill` literal `section resize` takes in place of a size:
/// clear the pin so the tree builder fills the parent's AABB.
///
/// It renamed `none`, which read as "remove the section". One word,
/// so usage prints it bare rather than as a one-item choice.
const FILL_WORD: &[Word] = &[Word::new("fill", "clear the size pin and fill the parent node")];

/// The geometry, text and structure keys. `section=` comes from the
/// shared targeting vocabulary in `commands::range_kv`, so the four
/// levels that speak it describe it one way.
const SECTION_OWN_KEYS: &[Key] = &[
    Key::new("dx", "relative move along x axis (canvas units)", free("f64")),
    Key::new("dy", "relative move along y axis (canvas units)", free("f64")),
    Key::new("x", "absolute x offset within parent node", free("f64")),
    Key::new("y", "absolute y offset within parent node", free("f64")),
    Key::new("w", "section width (canvas units)", free("f64")),
    Key::new("h", "section height (canvas units)", free("f64")),
    Key::new(
        "text",
        "section text payload (quote multi-word values)",
        free("text"),
    ),
    Key::new(
        "runs",
        "preserve|clear — keep or drop per-grapheme styling",
        Vocabulary::Words(RUNS_WORDS),
    ),
    Key::new("at", "insertion / split index", free("idx")),
];

/// Every subverb, in the order the popup offers them.
///
/// All are [`Subverb::gated`]: an unquoted multi-word kv value
/// splits into two tokens whose second can spell a subverb name, so
/// a kv at or before the subverb slot means the line is kv form
/// whatever the positional happens to say. `section text=hello
/// world` is the live instance — it used to answer "unknown subverb
/// 'world'" and now suggests the quoting the user meant.
const SUBVERBS: &[Subverb] = &[
    Subverb::bare(
        "move",
        "geometry",
        "shift section offset (dx/dy delta or x/y absolute)",
    )
    .taking(&[
        Form::keys(&["dx", "dy"], &["section"]),
        Form::keys(&["x", "y"], &["section"]),
    ])
    .gated(),
    Subverb::bare(
        "resize",
        "geometry",
        "pin section size (w/h) or clear to fill-parent",
    )
    .taking(&[
        Form::keys(&["w", "h"], &["section"]),
        Form::slots(&[Slot::req(Vocabulary::Words(FILL_WORD))]).reading(&["section"]),
    ])
    .gated(),
    Subverb::bare("show", "readout", "print the resolved per-section properties")
        .taking(&[Form::opt(&["section"])])
        .gated(),
    Subverb::bare("text", "text", "replace section text (runs=preserve|clear)")
        .taking(&[Form::slots(&[Slot::opt(free("text"))]).reading(&["text", "runs", "section"])])
        .gated(),
    Subverb::bare(
        "edit",
        "editor",
        "open the section text editor on the resolved target",
    )
    .taking(&[Form::opt(&["section"])])
    .gated(),
    Subverb::bare("add", "structure", "insert a new section")
        .taking(&[Form::opt(&["at", "text"])])
        .gated(),
    Subverb::bare(
        "delete",
        "structure",
        "remove the section (errors when only one remains)",
    )
    .taking(&[Form::opt(&["section"])])
    .gated(),
    Subverb::bare(
        "split",
        "structure",
        "split a section in two at a grapheme boundary",
    )
    .taking(&[Form::keys(&["at"], &["section"])])
    .gated(),
    Subverb::nested(
        "frame",
        "subject",
        "configure the section's frame border (subverb tree)",
        &SECTION_FRAME,
    ),
];

/// `section …`. No bare form: every operation names a subverb, which
/// is why an unknown first word answers with the subverb listing
/// rather than with a kv rejection.
pub static SECTION: Grammar = Grammar {
    label: "section",
    subverb_sets: &[SUBVERBS],
    key_sets: &[SECTION_OWN_KEYS, SECTION_KEYS],
    bare: None,
};
