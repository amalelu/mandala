// SPDX-License-Identifier: MPL-2.0

//! The declarative console grammar, and the one engine that reads it.
//!
//! Every console verb used to hand-write the same four things: a
//! subverb match with its unknown-subverb message, a kv loop with
//! unknown-key rejection, a `usage` literal, and a `complete` fn
//! that re-encoded the first two positionally. Four copies of one
//! grammar per verb, twenty verbs, and nothing holding the copies
//! together — which is how `font range=` came to be parseable,
//! named in the verb's own rejection, offered by no popup and
//! absent from `help font`, with the suite green.
//!
//! A [`Grammar`] replaces all four. It is a **level**, not a verb:
//! `border` is one, `border preview` is another, and
//! `canvas section-frame focused` is a third, each joined to its
//! parent by a `&'static` reference through [`Subverb::child`].
//! [`Grammar::subverb_sets`] and [`Grammar::key_sets`] are slices
//! *of slices* so a level composes its neighbors' vocabulary
//! without copying a row — `canvas border` is border's fifteen keys
//! and border's seven per-field subverbs, named rather than
//! transcribed.
//!
//! # What one declaration answers
//!
//! - **Completion**, through [`complete::completions`]: descend the
//!   positional tokens that name subverbs, stop at the first that
//!   does not, then offer this level's subverbs (at the subverb
//!   slot), the slot vocabulary under the cursor, and the kv keys
//!   of the matched form once its *required* slots sit behind the
//!   cursor. That last clause is what keeps `border preset <TAB>` a
//!   preset list while `section resize <TAB>` offers `fill` beside
//!   `w=` / `h=`.
//! - **Usage and tags**, through [`usage`]. `help` prints derived
//!   forms; a key added to a [`Grammar`] is documented by the same
//!   edit that makes it parseable.
//! - **The kv loop**, through [`kvs::read`]: unknown keys rejected
//!   by name, and keys the *shape being typed* does not read
//!   rejected by name too — `border preset heavy color=#fff` used to
//!   stage the preset and drop the color without a word, and
//!   `section resize fill w=99` used to drop the size.
//! - **The hint surface**: one [`Word::hint`] per vocabulary entry
//!   and one [`Key::hint`] per key, read by the popup and by the
//!   `<…>` a usage form prints.
//!
//! # What stays hand-written
//!
//! Value parsing and document mutation. A [`Key`] declares its
//! name, its sentence and its vocabulary; what `padding=8` *means*
//! is the verb's, and stays in the verb. Bespoke semantics —
//! `section move`'s mutually exclusive delta / absolute forms, the
//! `border side` non-custom-preset gate — stay as handwritten
//! handlers *behind* the table rather than as engine concepts.
//!
//! # No verb sees a raw token index
//!
//! The engine hands a verb a [`descent::Descent`], never an index
//! into the token list. Eight completion sites once did their
//! lookahead with `tokens.get(N)` while the arms they fed were
//! keyed on the *positional* index, and the two disagree the moment
//! a kv pair sits earlier on the line. Descent is the only thing
//! here that touches token order.

use crate::application::console::completion::Completion;
use crate::application::console::ConsoleContext;

pub mod complete;
pub mod descent;
pub mod kvs;
pub mod usage;

#[cfg(test)]
mod tests;

pub use descent::Descent;

/// One entry in a closed vocabulary: the token a popup inserts and
/// the sentence beside it.
///
/// An empty `hint` emits a hint-less row, which is what a
/// vocabulary of bare enum values (`top` / `bottom` / `left`) wants
/// — the word is its own explanation.
#[derive(Clone, Copy)]
pub struct Word {
    pub name: &'static str,
    pub hint: &'static str,
}

impl Word {
    /// A vocabulary entry that explains itself.
    pub const fn bare(name: &'static str) -> Self {
        Self { name, hint: "" }
    }

    /// A vocabulary entry with a sentence beside it.
    pub const fn new(name: &'static str, hint: &'static str) -> Self {
        Self { name, hint }
    }
}

/// What can stand in one slot or on the value side of one key.
///
/// One declaration answers three questions — the rows a popup
/// offers, the `<…>` a usage form prints, and the word list an
/// error can quote back — so the three cannot come to disagree.
#[derive(Clone, Copy)]
pub enum Vocabulary {
    /// Anything. Usage prints `<placeholder>` — a grapheme range, a
    /// pixel count, a file path.
    Free { placeholder: &'static str },
    /// A closed list. Usage prints `<a|b|c>`, or the bare word when
    /// there is only one — a one-word closed vocabulary is a
    /// literal (`section resize fill`), not a choice.
    Words(&'static [Word]),
    /// Free-form plus named sentinels — a font family *or* `off`, a
    /// side pattern *or* `reset`. Usage prints
    /// `<placeholder|off>`.
    FreeWords {
        placeholder: &'static str,
        words: &'static [Word],
    },
    /// Rows a function produces, plus the fixed `sentinels` offered
    /// beside them.
    ///
    /// Two kinds of vocabulary need this. One is genuinely derived
    /// from the live document — the palettes a map declares, the
    /// sections a node has, the families the host loaded. The other
    /// is a *suggestion* list too long or too open-ended to belong
    /// in a usage line: `zoom min=` offers eight zoom levels and
    /// accepts any positive float, so the popup wants the eight and
    /// the usage line wants `<zoom|unset>`. Only the sentinels are
    /// printed, which is what keeps the two honest at once.
    Rows {
        placeholder: &'static str,
        rows: fn(&ConsoleContext, &str) -> Vec<Completion>,
        sentinels: &'static [Word],
    },
}

impl Vocabulary {
    /// Whether a positional already on the line could be standing in
    /// a slot with this vocabulary.
    ///
    /// Only a closed word list can answer "no". An open vocabulary —
    /// a path, a pattern, a palette name — admits whatever was typed,
    /// and its sentinels are a suggestion list rather than a filter,
    /// so `border palette My-Palette` is still the palette form.
    /// Closed words are matched case-insensitively, the way every
    /// parser in the console reads them (`commands/mod.rs` § Casing).
    pub fn admits(&self, value: &str) -> bool {
        match self {
            Vocabulary::Words(words) => words.iter().any(|w| w.name.eq_ignore_ascii_case(value)),
            Vocabulary::Free { .. } | Vocabulary::FreeWords { .. } | Vocabulary::Rows { .. } => true,
        }
    }
}

/// One positional argument of a subverb or of a level's bare form.
///
/// `optional` decides both the square brackets in usage and whether
/// the kv keys stay on offer while the cursor sits here.
#[derive(Clone, Copy)]
pub struct Slot {
    pub vocab: Vocabulary,
    pub optional: bool,
}

impl Slot {
    /// A required slot.
    pub const fn req(vocab: Vocabulary) -> Self {
        Self {
            vocab,
            optional: false,
        }
    }

    /// An optional slot — usage brackets it, and the kv keys of the
    /// enclosing form stay on offer while the cursor is here.
    pub const fn opt(vocab: Vocabulary) -> Self {
        Self {
            vocab,
            optional: true,
        }
    }
}

/// A free-form vocabulary, spelled `<placeholder>` in usage.
pub const fn free(placeholder: &'static str) -> Vocabulary {
    Vocabulary::Free { placeholder }
}

/// A free-form vocabulary with named sentinels beside it.
pub const fn free_words(placeholder: &'static str, words: &'static [Word]) -> Vocabulary {
    Vocabulary::FreeWords { placeholder, words }
}

/// Lift a plain name list into a hint-less vocabulary at const-fn
/// time.
///
/// A closed vocabulary of bare enum values is its own explanation —
/// `top` needs no sentence beside it — so the `&[&str]` list a
/// verb's parser validates against stays the source and the
/// `&[Word]` the engine reads derives from it. One declaration, two
/// readers.
pub const fn bare_words<const N: usize>(names: &'static [&'static str]) -> [Word; N] {
    let mut out = [Word::bare(""); N];
    let mut i = 0;
    while i < N {
        out[i] = Word::bare(names[i]);
        i += 1;
    }
    out
}

/// One `key=value` the level accepts.
///
/// `hint` is the sentence the popup shows beside `key=`; it is also
/// the only place that sentence is written, so the four surfaces
/// that share `section=` describe it one way.
#[derive(Clone, Copy)]
pub struct Key {
    pub name: &'static str,
    pub hint: &'static str,
    pub vocab: Vocabulary,
}

impl Key {
    pub const fn new(name: &'static str, hint: &'static str, vocab: Vocabulary) -> Self {
        Self { name, hint, vocab }
    }
}

/// One shape of a subverb or of a level's bare form — positional
/// slots plus kv keys, and the unit a usage line is printed from.
///
/// Most subverbs have exactly one. A second exists where the shapes
/// are genuinely alternatives rather than a menu: `section move`
/// takes `dx=`/`dy=` *or* `x=`/`y=`, and
/// `section resize` takes the `fill` literal *or* `w=`/`h=`.
/// Printing either pair on one bracketed line would document a
/// command the verb rejects, so each form on its own is what `help`
/// prints.
///
/// What [`kvs::read`] accepts and what the popup offers is the union
/// across the forms **the line's positionals still admit** — see
/// [`Self::admits_prefix`]. That is what makes the exclusion real on
/// `section resize`: `fill` sits in a slot only one of the two forms
/// declares, so once it is typed the other form's `w=`/`h=` are
/// refused by name instead of being read and dropped.
///
/// It reaches exactly as far as the *slots* do. Two forms that differ
/// only in their keys — `section move`'s `dx=`/`dy=` against its
/// `x=`/`y=` — are equally admitted by an empty positional list, so
/// the engine offers and accepts their union and the exclusion stays
/// the verb's own: `execute_move` refuses the mix by hand, with a
/// message naming both shapes. Deciding it here would mean letting
/// whichever key was typed first pick the form, which is a guess, not
/// a grammar.
///
/// `required` keys print bare, `optional` keys print bracketed.
/// Both are names resolved against the level's
/// [`Grammar::key_sets`], so a form naming a key the level does not
/// declare — or a key no form prints — fails the suite.
#[derive(Clone, Copy)]
pub struct Form {
    pub slots: &'static [Slot],
    pub required: &'static [&'static str],
    pub optional: &'static [&'static str],
}

impl Form {
    /// A form whose keys are all optional and which takes no
    /// positional arguments.
    pub const fn opt(optional: &'static [&'static str]) -> Self {
        Self {
            slots: &[],
            required: &[],
            optional,
        }
    }

    /// A form with required keys and optional ones beside them.
    pub const fn keys(required: &'static [&'static str], optional: &'static [&'static str]) -> Self {
        Self {
            slots: &[],
            required,
            optional,
        }
    }

    /// A form that is only positional arguments.
    pub const fn slots(slots: &'static [Slot]) -> Self {
        Self {
            slots,
            required: &[],
            optional: &[],
        }
    }

    /// The same form, with the named optional keys beside its slots.
    pub const fn reading(mut self, optional: &'static [&'static str]) -> Self {
        self.optional = optional;
        self
    }

    /// Every key this form names, required first.
    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.required.iter().copied().chain(self.optional.iter().copied())
    }

    /// Whether every required slot of this form sits strictly before
    /// index `i` — the condition for its kv keys to stay on offer
    /// while the cursor is at slot `i`.
    pub fn required_slots_behind(&self, i: usize) -> bool {
        !self.slots.iter().skip(i).any(|s| !s.optional)
    }

    /// Whether the positionals already on the line could be standing
    /// in *this* form's slots.
    ///
    /// The question a subverb with two shapes has to answer before it
    /// can say which keys it reads. `section resize` declares the
    /// `fill` literal in one form and the `w=`/`h=` pair in another;
    /// once `fill` is on the line, only the first form is still being
    /// typed, and the second one's keys are not this line's to fill.
    ///
    /// More positionals than slots is a no, and a closed slot
    /// vocabulary that does not contain the word is a no. Nothing
    /// else is: a form is narrowed by what the line already says, not
    /// by what it has yet to say, which is why a missing required
    /// slot does not disqualify one — the popup at that very slot is
    /// what offers it.
    pub fn admits_prefix(&self, committed: &[&str]) -> bool {
        committed.len() <= self.slots.len()
            && self
                .slots
                .iter()
                .zip(committed)
                .all(|(slot, value)| slot.vocab.admits(value))
    }
}

/// The forms of a subverb (or of a level's bare form) that the
/// positionals already on the line admit.
///
/// Falls back to *every* form when the line admits none of them: a
/// positional that fits no shape is a structural error the caller
/// reports on its own terms (`section resize bogus` prints both usage
/// lines), and narrowing the key vocabulary to nothing there would
/// answer a question the user did not ask.
pub fn eligible_forms(forms: &'static [Form], committed: &[&str]) -> Vec<&'static Form> {
    let narrowed: Vec<&'static Form> = forms.iter().filter(|f| f.admits_prefix(committed)).collect();
    if narrowed.is_empty() {
        forms.iter().collect()
    } else {
        narrowed
    }
}

/// A subverb or bare form that accepts nothing at all — no slots,
/// no keys — so that "declares no shapes" and "declares one empty
/// shape" are the same thing to every reader.
pub const NO_FORMS: &[Form] = &[];

/// One named form at a level: `show`, `preset <name>`, `preview …`.
///
/// A subverb either descends into a [`Self::child`] level or
/// declares [`Self::forms`] of its own — never both. That is what
/// lets [`descent`] walk a line without a lookahead table: a subverb
/// with a child is a step deeper, a subverb with forms is the end of
/// the descent.
#[derive(Clone, Copy)]
pub struct Subverb {
    pub name: &'static str,
    /// Heading this subverb sits under in the level's
    /// unknown-subverb listing. Subverbs sharing a group print on
    /// one line, in declaration order.
    pub group: &'static str,
    pub hint: &'static str,
    /// The shapes this subverb accepts — positional slots and kv
    /// keys together. A kv no form names is rejected by name rather
    /// than dropped: `border preset heavy color=#fff` used to stage
    /// the preset and discard the color without a word.
    pub forms: &'static [Form],
    /// The level this subverb opens, for a subverb that nests
    /// rather than taking arguments.
    pub child: Option<&'static Grammar>,
    /// Whether the positional-vs-kv discriminator gates this
    /// subverb. A gated subverb is matched only when no kv sits at
    /// or before its slot, so an unquoted `palette=My Palette` is
    /// not read as a `Palette` subverb — and the popup withholds it
    /// at exactly the slots the verb would refuse it.
    pub gated: bool,
}

impl Subverb {
    /// A subverb with no arguments and no keys.
    pub const fn bare(name: &'static str, group: &'static str, hint: &'static str) -> Self {
        Self {
            name,
            group,
            hint,
            forms: NO_FORMS,
            child: None,
            gated: false,
        }
    }

    /// A subverb opening a nested level.
    pub const fn nested(
        name: &'static str,
        group: &'static str,
        hint: &'static str,
        child: &'static Grammar,
    ) -> Self {
        Self {
            name,
            group,
            hint,
            forms: NO_FORMS,
            child: Some(child),
            gated: false,
        }
    }

    /// The same subverb, accepting the given shapes.
    pub const fn taking(mut self, forms: &'static [Form]) -> Self {
        self.forms = forms;
        self
    }

    /// The same subverb, subject to the positional-vs-kv
    /// discriminator. See [`Self::gated`].
    pub const fn gated(mut self) -> Self {
        self.gated = true;
        self
    }

    /// Every kv key the forms `committed` still admits read — the
    /// keys *this* line may carry. See [`Form::admits_prefix`].
    ///
    /// An empty `committed` is the union — every key the subverb
    /// reads in *some* shape — and both halves of [`eligible_forms`]
    /// have to say so for that to hold. The filter does:
    /// [`Form::admits_prefix`] is unconditionally true against an
    /// empty prefix — `0 <= slots.len()`, and the `all` runs over a
    /// zip with an empty slice — so nothing is narrowed away. The
    /// `narrowed.is_empty()` fallback beneath it does too, and only
    /// reachably in one case: an empty prefix can leave `narrowed`
    /// empty only when `forms` was empty to begin with ([`NO_FORMS`]),
    /// where the fallback hands back the very slice the filter just
    /// produced. Ask for the union that way rather than by a second
    /// method; there used to be one, and it computed the same list
    /// under a name that made it look like the other half of a
    /// pair.
    pub fn readable_keys_for(&self, committed: &[&str]) -> Vec<&'static str> {
        dedup_names(&eligible_forms(self.forms, committed))
    }

    /// How many positionals the subverb declares — the widest of
    /// its forms, since a stray extra positional is only stray past
    /// every shape the subverb accepts.
    pub fn slot_count(&self) -> usize {
        self.forms.iter().map(|f| f.slots.len()).max().unwrap_or(0)
    }
}

/// The key names across `forms`, each once: every form's required
/// keys first, then every form's optional ones.
///
/// Two passes rather than one, so a subverb with two forms reads as
/// `dx | dy | x | y | section` rather than interleaving the shared
/// optional key between the two required pairs. Short lists (at most
/// a dozen), so the linear `contains` is cheaper than building a set.
fn dedup_names(forms: &[&'static Form]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    let mut push = |name: &'static str| {
        if !out.contains(&name) {
            out.push(name);
        }
    };
    for form in forms {
        for name in form.required {
            push(name);
        }
    }
    for form in forms {
        for name in form.optional {
            push(name);
        }
    }
    out
}

/// The form a level accepts with no subverb named: `border
/// <key>=<value> …`, `open <path>`, `font size=<pt> …`.
///
/// `None` on [`Grammar::bare`] means a subverb is mandatory, which
/// is what makes `section` answer an unknown first word with its
/// subverb listing rather than with a kv rejection.
#[derive(Clone, Copy)]
pub struct Bare {
    /// Heading the bare form sits under in the level's
    /// unknown-subverb listing.
    pub group: &'static str,
    pub forms: &'static [Form],
}

impl Bare {
    /// The shapes a level accepts with no subverb named.
    pub const fn new(group: &'static str, forms: &'static [Form]) -> Self {
        Self { group, forms }
    }

    /// Every kv key the bare form reads, across all its forms.
    ///
    /// Delegates rather than collecting `forms` itself:
    /// [`Self::readable_keys_for`] with nothing committed *is* the
    /// union, for the reason spelled out on
    /// [`Subverb::readable_keys_for`], and two bodies computing one
    /// list is how a synonym starts looking like a distinction.
    pub fn readable_keys(&self) -> Vec<&'static str> {
        self.readable_keys_for(&[])
    }

    /// Every kv key the bare forms `committed` still admits read.
    pub fn readable_keys_for(&self, committed: &[&str]) -> Vec<&'static str> {
        dedup_names(&eligible_forms(self.forms, committed))
    }

    /// How many positionals the bare form declares.
    pub fn slot_count(&self) -> usize {
        self.forms.iter().map(|f| f.slots.len()).max().unwrap_or(0)
    }
}

/// One level of the console grammar.
///
/// `label` is the words that lead every usage form and every
/// message this level prints — `"border"`, `"canvas section-frame
/// focused"` — so a rejection is always copy-pasteable for the
/// surface that printed it.
pub struct Grammar {
    pub label: &'static str,
    /// Subverb vocabularies, composed. Order across the sets is the
    /// order popups and usage listings use.
    pub subverb_sets: &'static [&'static [Subverb]],
    /// Key vocabularies, composed. A key named by a subverb or by
    /// [`Self::bare`] is resolved here.
    pub key_sets: &'static [&'static [Key]],
    /// What the level accepts with no subverb named.
    pub bare: Option<Bare>,
}

impl Grammar {
    /// Every subverb this level declares, in declaration order
    /// across the composed sets.
    pub fn subverbs(&self) -> impl Iterator<Item = &'static Subverb> {
        self.subverb_sets.iter().copied().flatten()
    }

    /// Every key this level declares, in declaration order across
    /// the composed sets.
    pub fn keys(&self) -> impl Iterator<Item = &'static Key> {
        self.key_sets.iter().copied().flatten()
    }

    /// Look up a key by its exact name. Kv keys are matched
    /// case-sensitively console-wide — a key is a field name, not a
    /// word the user picks (`commands/mod.rs` § Casing).
    pub fn key(&self, name: &str) -> Option<&'static Key> {
        self.keys().find(|k| k.name == name)
    }

    /// Look up a subverb by name, case-insensitively — subverb
    /// names are matched that way console-wide.
    pub fn subverb(&self, name: &str) -> Option<&'static Subverb> {
        self.subverbs().find(|s| s.name.eq_ignore_ascii_case(name))
    }
}
