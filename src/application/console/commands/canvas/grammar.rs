// SPDX-License-Identifier: MPL-2.0

//! `canvas …`'s grammar — six levels, none of which owns a
//! vocabulary.
//!
//! Every key and every per-field subverb here is the *border*
//! verb's, named rather than transcribed:
//! [`Grammar::key_sets`] and [`Grammar::subverb_sets`] are slices of
//! slices precisely so `canvas border` can be border's fifteen keys
//! plus border's seven per-field subverbs plus its own three
//! readout / staging ones. `canvas.rs` used to carry its own copy of
//! that dispatcher, and the copies drifted twice before they were
//! merged.
//!
//! The `focused` modifier is a level rather than a flag. It shifts
//! the whole subverb tree one positional right, and expressing that
//! as a nested [`Grammar`] is what makes every slot past it fall out
//! of the same descent — the hand-written completer had to
//! re-derive `verb_at` per arm, and each arm written for one depth
//! left the next depth answering with kv keys.

use crate::application::console::commands::border::grammar::{
    COMPOSED, KEYS as BORDER_KEYS, POSITIONAL_SUBVERBS,
};
use crate::application::console::spec::{Bare, Grammar, Subverb};

/// The three subverbs both canvas subjects match *ahead* of the
/// positional-vs-kv discriminator, so they stay on offer at a
/// kv-form slot: `canvas border color=#fff show` prints the readout
/// and `canvas border color=#fff preview commit` terminates a
/// preview.
const CANVAS_BORDER_SUBVERBS: &[Subverb] = &[
    Subverb::bare("show", "readout", "print the resolved canvas default"),
    Subverb::bare("reset", "override", "drop the canvas-level default"),
    Subverb::nested(
        "preview",
        "staged",
        "stage a preview without writing the model (commit/cancel terminates)",
        &CANVAS_BORDER_PREVIEW,
    ),
];

const SECTION_FRAME_SUBVERBS: &[Subverb] = &[
    Subverb::bare("show", "readout", "print the resolved canvas default"),
    Subverb::bare("reset", "override", "drop the canvas-level default"),
    Subverb::nested(
        "preview",
        "staged",
        "stage a preview without writing the model (commit/cancel terminates)",
        &CANVAS_SECTION_FRAME_PREVIEW,
    ),
];

const FOCUSED_SUBVERBS: &[Subverb] = &[
    Subverb::bare("show", "readout", "print the resolved canvas default"),
    Subverb::bare("reset", "override", "drop the canvas-level default"),
    Subverb::nested(
        "preview",
        "staged",
        "stage a preview without writing the model (commit/cancel terminates)",
        &CANVAS_FOCUSED_PREVIEW,
    ),
];

/// The modifier under `section-frame`. Ungated with the readout
/// subverbs and for the same reason: the subject reads it before the
/// discriminator runs, so `canvas section-frame color=#fff focused
/// show` still reaches the focused slot's readout.
const FOCUSED_MODIFIER: &[Subverb] = &[Subverb::nested(
    "focused",
    "modifier",
    "target the focused section's frame rather than the unfocused default",
    &CANVAS_SECTION_FRAME_FOCUSED,
)];

/// The two canvas subjects.
const SUBJECTS: &[Subverb] = &[
    Subverb::nested(
        "border",
        "subject",
        "the map-wide default node border",
        &CANVAS_BORDER,
    ),
    Subverb::nested(
        "section-frame",
        "subject",
        "the map-wide default section-frame border",
        &CANVAS_SECTION_FRAME,
    ),
];

/// The staging levels. Three of them, one per canvas slot, because
/// each writes a different `BorderPreviewTarget` — but all three
/// read border's keyset and border's terminator pair.
pub static CANVAS_BORDER_PREVIEW: Grammar = Grammar {
    label: "canvas border preview",
    subverb_sets: &[crate::application::console::commands::border::grammar::PREVIEW_TERMINATORS],
    key_sets: &[BORDER_KEYS],
    bare: Some(Bare::new("composed", COMPOSED)),
};

pub static CANVAS_SECTION_FRAME_PREVIEW: Grammar = Grammar {
    label: "canvas section-frame preview",
    subverb_sets: &[crate::application::console::commands::border::grammar::PREVIEW_TERMINATORS],
    key_sets: &[BORDER_KEYS],
    bare: Some(Bare::new("composed", COMPOSED)),
};

pub static CANVAS_FOCUSED_PREVIEW: Grammar = Grammar {
    label: "canvas section-frame focused preview",
    subverb_sets: &[crate::application::console::commands::border::grammar::PREVIEW_TERMINATORS],
    key_sets: &[BORDER_KEYS],
    bare: Some(Bare::new("composed", COMPOSED)),
};

pub static CANVAS_BORDER: Grammar = Grammar {
    label: "canvas border",
    subverb_sets: &[CANVAS_BORDER_SUBVERBS, POSITIONAL_SUBVERBS],
    key_sets: &[BORDER_KEYS],
    bare: Some(Bare::new("composed", COMPOSED)),
};

pub static CANVAS_SECTION_FRAME_FOCUSED: Grammar = Grammar {
    label: "canvas section-frame focused",
    subverb_sets: &[FOCUSED_SUBVERBS, POSITIONAL_SUBVERBS],
    key_sets: &[BORDER_KEYS],
    bare: Some(Bare::new("composed", COMPOSED)),
};

pub static CANVAS_SECTION_FRAME: Grammar = Grammar {
    label: "canvas section-frame",
    subverb_sets: &[FOCUSED_MODIFIER, SECTION_FRAME_SUBVERBS, POSITIONAL_SUBVERBS],
    key_sets: &[BORDER_KEYS],
    bare: Some(Bare::new("composed", COMPOSED)),
};

/// `canvas …` — the verb's own level. It owns no keys: everything
/// it edits lives one subject deeper, which is why a bare
/// `canvas <key>=<value>` is not a form and the level declares no
/// [`Bare`].
pub static CANVAS: Grammar = Grammar {
    label: "canvas",
    subverb_sets: &[SUBJECTS],
    key_sets: &[],
    bare: None,
};
