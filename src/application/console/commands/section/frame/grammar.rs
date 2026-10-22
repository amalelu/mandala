// SPDX-License-Identifier: MPL-2.0

//! `section frame …` — the per-section border level.
//!
//! It borrows the whole `border` keyset and adds the one key of its
//! own: `section=<idx>`, the target selector its usage line has
//! always documented and `parse_section_target_kv` has always
//! honored, while the completer read `BORDER_KEYS` alone and stayed
//! silent about it — twice, since the `preview` sub-level was silent
//! too.
//!
//! Positional per-field subverbs (`preset heavy`, `side top …`)
//! are *not* declared here. The per-node `border` verb and both
//! canvas subjects accept them; this level does not, and the
//! unknown-subverb listing is derived from the declaration, so what
//! it lists is exactly what the parser claims.

use crate::application::console::commands::border::grammar::PREVIEW_TERMINATORS;
use crate::application::console::commands::border::grammar::{KEYS as BORDER_KEYS, KEY_NAMES};
use crate::application::console::commands::range_kv::SECTION_KEYS;
use crate::application::console::spec::{Bare, Form, Grammar, Subverb};

/// The border keyset plus this level's own `section=` target.
const FRAME_KEY_NAMES: &[&str] = &{
    const N: usize = KEY_NAMES.len();
    let mut out: [&str; N + 1] = [""; N + 1];
    let mut i = 0;
    while i < N {
        out[i] = KEY_NAMES[i];
        i += 1;
    }
    out[N] = "section";
    out
};

const FRAME_COMPOSED: &[Form] = &[Form::opt(FRAME_KEY_NAMES)];

const FRAME_SUBVERBS: &[Subverb] = &[
    Subverb::bare("show", "readout", "print the resolved section-frame style")
        .taking(&[Form::opt(&["section"])]),
    Subverb::bare("reset", "override", "drop the per-section override").taking(&[Form::opt(&["section"])]),
    Subverb::nested(
        "preview",
        "staged",
        "stage a preview without writing the model (commit/cancel terminates)",
        &SECTION_FRAME_PREVIEW,
    ),
];

pub static SECTION_FRAME_PREVIEW: Grammar = Grammar {
    label: "section frame preview",
    subverb_sets: &[PREVIEW_TERMINATORS],
    key_sets: &[BORDER_KEYS, SECTION_KEYS],
    bare: Some(Bare::new("composed", FRAME_COMPOSED)),
};

pub static SECTION_FRAME: Grammar = Grammar {
    label: "section frame",
    subverb_sets: &[FRAME_SUBVERBS],
    key_sets: &[BORDER_KEYS, SECTION_KEYS],
    bare: Some(Bare::new("composed", FRAME_COMPOSED)),
};
