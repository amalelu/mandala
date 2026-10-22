// SPDX-License-Identifier: MPL-2.0

//! Grammar descent — the one place in the console that reads token
//! order.
//!
//! A line is walked once: match the positional at this level's
//! subverb slot, step into that subverb's child level if it has
//! one, stop otherwise. The result is a [`Descent`], which names
//! the level reached, the subverb matched there, and the positional
//! slot the subverb sat in — never a raw token index. Everything a
//! verb wants to know about "what came before me" is answered from
//! that, which is what keeps a `Token { index }` arm and its
//! lookahead counting the same slots.
//!
//! The walk also owns the **positional-vs-kv discriminator**, since
//! it is a question about token order and about nothing else.

use crate::application::console::parser::{is_kv_token, split_kv, Args};

use super::{Grammar, Subverb};

/// Where the walk stopped at the deepest level it reached.
#[derive(Clone, Copy)]
pub enum Stop {
    /// No positional sits at the subverb slot: the level's bare
    /// form is what will run.
    Bare,
    /// A subverb matched. Its slots start one positional later.
    Matched(&'static Subverb),
    /// A positional sits at the subverb slot and names nothing this
    /// level declares.
    Unknown,
    /// A kv already made the subverb slot kv-form, and the
    /// positional sitting there is either unknown or a *gated*
    /// subverb. The tokenizer splits an unquoted multi-word value,
    /// so `border palette=My Palette` arrives as
    /// `["palette=My", "Palette"]` and `Palette` coincidentally
    /// names a subverb — reading it as one would dispatch the
    /// positional grammar with the wrong value. See
    /// [`Descent::quoting_hint`], the rejection every surface
    /// shares for it.
    KvForm,
}

/// How deep the walk can go. The declaration's deepest path today
/// is `canvas section-frame focused preview`, which is three
/// descents; the array is sized past that so a fourth nesting level
/// does not need this constant found and changed, and
/// [`descend`] refuses to go deeper rather than overrunning it.
pub const MAX_DEPTH: usize = 6;

/// The outcome of one grammar descent.
pub struct Descent<'a> {
    /// The deepest level reached. Its `label` is the words every
    /// message from here leads with.
    pub level: &'static Grammar,
    /// What was found at that level's subverb slot.
    pub stop: Stop,
    /// Positional index of that level's subverb slot: `0` for
    /// `border …`, `1` for `canvas border …`, `2` for
    /// `canvas section-frame focused …`.
    pub slot: usize,
    /// The positional token sitting at [`Self::slot`], in the
    /// user's own spelling. Every rejection quotes this rather than
    /// the normalized copy it matched on.
    pub typed: Option<&'a str>,
    /// The subverbs the walk *descended through* — those with a
    /// child level — outermost first. `["border"]` for
    /// `canvas border show`, `["section-frame", "focused"]` for
    /// `canvas section-frame focused preset heavy`. Empty when the
    /// walk never left the verb's own level.
    path: [Option<&'static Subverb>; MAX_DEPTH],
    depth: usize,
}

impl<'a> Descent<'a> {
    /// The subverb that matched, or `None` for every other stop.
    pub fn subverb(&self) -> Option<&'static Subverb> {
        match self.stop {
            Stop::Matched(s) => Some(s),
            _ => None,
        }
    }

    /// The subverb the walk descended through at step `i`, counting
    /// from the verb's own level. This is how a handler asks "which
    /// branch am I in" without ever seeing a token index.
    pub fn parent(&self, i: usize) -> Option<&'static Subverb> {
        if i >= self.depth {
            return None;
        }
        self.path.get(i).copied().flatten()
    }

    /// The name of [`Self::parent`] at step `i`, for the `match` a
    /// handler writes.
    pub fn parent_name(&self, i: usize) -> Option<&'static str> {
        self.parent(i).map(|s| s.name)
    }

    /// Where the matched form's slot 0 sits among the positionals.
    ///
    /// One past the subverb when a subverb matched; *at* the
    /// subverb slot for a bare form, which has no name of its own
    /// to consume a positional — `open <path>` puts the path in the
    /// very slot `border on` puts its subverb.
    fn slot_base(&self) -> usize {
        match self.stop {
            Stop::Matched(_) => self.slot + 1,
            _ => self.slot,
        }
    }

    /// The values of the matched form's positional slots, indexed
    /// from the form rather than from the line.
    pub fn slot_value<'b>(&self, args: &'b Args) -> SlotReader<'b> {
        SlotReader {
            args,
            base: self.slot_base(),
        }
    }

    /// The first positional past everything the matched form
    /// declares — a stray argument the verb should refuse rather
    /// than drop. `count` is how many slots the form declares.
    pub fn extra_positional<'b>(&self, args: &'b Args, count: usize) -> Option<&'b str> {
        args.positional(self.slot_base() + count)
    }

    /// The positionals already sitting in the matched form's slots,
    /// in slot order, stopping at the first one the line does not
    /// carry. `count` bounds the walk at the widest form the subverb
    /// declares.
    ///
    /// This is the "what has the line already said" a subverb with
    /// two shapes is narrowed by — see [`super::Form::admits_prefix`].
    pub fn committed_slots<'b>(&self, args: &'b Args, count: usize) -> Vec<&'b str> {
        let reader = self.slot_value(args);
        (0..count).map_while(|i| reader.get(i)).collect()
    }

    /// The "you probably meant to quote this" rejection for a
    /// [`Stop::KvForm`], built from this descent's own level, slot
    /// and typed word.
    ///
    /// Seven surfaces answer that stop, and every one of them wants
    /// the same sentence. Reading it off the descent is also what
    /// keeps the raw token slice out of a verb's hands: the only
    /// thing a handler passes is the `args` it already has.
    pub fn quoting_hint(&self, args: &Args) -> String {
        unquoted_multiword_hint(
            self.level.label,
            args.tokens(),
            self.slot,
            self.typed.unwrap_or_default(),
        )
    }
}

/// Reader for a matched subverb's positional slots, indexed from
/// the subverb rather than from the line. A verb asks for "my slot
/// 0", never for "positional 3".
pub struct SlotReader<'b> {
    args: &'b Args<'b>,
    base: usize,
}

impl<'b> SlotReader<'b> {
    /// Slot `i` of the matched subverb.
    pub fn get(&self, i: usize) -> Option<&'b str> {
        self.args.positional(self.base + i)
    }
}

/// Positional token `idx` of a raw token slice — the same view
/// [`Args::positional`] gives the execute path, and the very slots
/// `CompletionContext::Token`'s `index` counts on the popup side.
fn positional_at(tokens: &[String], idx: usize) -> Option<&str> {
    tokens
        .iter()
        .filter(|t| !is_kv_token(t))
        .nth(idx)
        .map(String::as_str)
}

/// Walk `tokens` — the verb's own tokens, command name already
/// stripped — down `root` and report where the grammar ran out.
pub fn descend<'a>(root: &'static Grammar, tokens: &'a [String]) -> Descent<'a> {
    descend_at(root, tokens, 0)
}

/// [`descend`], entered at a level that is not the verb's own root.
///
/// `start_slot` is the positional index `root`'s subverb slot sits
/// at. It is `0` for a verb whose grammar *is* its root, and `1` for
/// a level reached through a token the caller has already consumed —
/// `section frame …`, whose parent verb dispatches on
/// `positional(0)` before the border vocabulary starts. Every index
/// the walk reports stays absolute, so a `Token { index }` arm and
/// the descent still count the same slots.
pub fn descend_at<'a>(root: &'static Grammar, tokens: &'a [String], start_slot: usize) -> Descent<'a> {
    let mut level = root;
    let mut slot = start_slot;
    let mut path: [Option<&'static Subverb>; MAX_DEPTH] = [None; MAX_DEPTH];
    let mut depth = 0usize;
    loop {
        let Some(word) = positional_at(tokens, slot) else {
            return Descent {
                level,
                stop: Stop::Bare,
                slot,
                typed: None,
                path,
                depth,
            };
        };
        // A level that declares no subverbs cannot report an
        // unknown one: the positional sitting here is its bare
        // form's first slot, not a word it failed to recognize.
        // `open <path>` is the shape — the path occupies the very
        // slot `border on` puts its subverb in.
        if level.subverb_sets.iter().all(|set| set.is_empty()) {
            return Descent {
                level,
                stop: Stop::Bare,
                slot,
                typed: Some(word),
                path,
                depth,
            };
        }
        let positional_form = subverb_slot_is_positional(tokens, slot);
        let stop = match level.subverb(word) {
            // Ungated subverbs are matched ahead of the
            // discriminator, which is why `border color=#fff show`
            // really does print and the popup keeps offering it at
            // a kv-form slot.
            Some(sv) if !sv.gated => Stop::Matched(sv),
            Some(sv) if positional_form => Stop::Matched(sv),
            Some(_) => Stop::KvForm,
            None if positional_form => Stop::Unknown,
            None => Stop::KvForm,
        };
        match stop {
            // A child level is a step deeper — unless the
            // declaration nests past what the path array can hold,
            // in which case the walk stops here and the level
            // answers as if the subverb were its own terminal. The
            // guard is unreachable at `MAX_DEPTH` = 6 against a
            // grammar three deep, and it is what makes the fixed
            // array safe rather than merely large enough today.
            Stop::Matched(sv) if sv.child.is_some() && depth < MAX_DEPTH => {
                path[depth] = Some(sv);
                depth += 1;
                if let Some(child) = sv.child {
                    level = child;
                }
                slot += 1;
                continue;
            }
            _ => {
                return Descent {
                    level,
                    stop,
                    slot,
                    typed: Some(word),
                    path,
                    depth,
                }
            }
        }
    }
}

/// Whether the subverb slot at positional index `verb_pos` is
/// genuinely positional — no kv token sits at or before it on the
/// line.
///
/// The discriminator exists because the tokenizer splits an
/// unquoted multi-word value: `border palette=My Palette` becomes
/// `["palette=My", "Palette"]`, so positional 0 reads `"Palette"`
/// and coincidentally matches a subverb name. A kv ahead of the
/// subverb slot means the user is writing kv form, whatever the
/// later positional happens to spell — so the caller routes to
/// [`Descent::quoting_hint`] instead of dispatching the
/// positional grammar with the wrong value.
///
/// It is asked once per level inside [`descend`], for the subverbs
/// a level declares as [`Subverb::gated`]. Every surface that used
/// to ask it by hand — five execute dispatchers and three
/// completion slots, three of them asking the subtly different
/// "is there a kv *anywhere* on the line" — now asks it by
/// declaring the gate on the subverb instead.
pub fn subverb_slot_is_positional(tokens: &[String], verb_pos: usize) -> bool {
    tokens.iter().take(verb_pos + 1).all(|t| !is_kv_token(t))
}

/// The "you probably meant to quote this" rejection every level
/// shares. `label` is the level's own prefix (`border`,
/// `canvas border`, `section frame preview`, …) so the suggested
/// line is copy-pasteable for the surface that printed it, and
/// `tokens` / `verb_pos` are the pair [`subverb_slot_is_positional`]
/// was asked, so the suggestion is built from the line that
/// actually reached here.
///
/// It used to be built from neither: the key was the literal
/// `palette` and the value was the offending positional, so
/// `border font=DejaVu Sans` — a real instance of the mistake this
/// message exists for — suggested ``border palette="Sans"``, naming
/// a key the user had not typed and quoting the tail of the value
/// rather than the value.
fn unquoted_multiword_hint(label: &str, tokens: &[String], verb_pos: usize, verb: &str) -> String {
    let suggestion = split_kv_suggestion(tokens, verb_pos).unwrap_or_else(|| format!("<key>=\"{}\"", verb));
    format!(
        "{}: unexpected positional '{}' alongside a kv pair — \
         did you mean to quote a multi-word value? \
         e.g. `{} {}`",
        label, verb, label, suggestion
    )
}

/// Rebuild the kv the tokenizer is presumed to have split, as
/// `key="value words"`.
///
/// The offending positional sits at positional index `verb_pos`;
/// the kv that made that slot kv-form is the last kv token ahead of
/// it, and everything between the two is what a quote would have
/// held together. So `border palette=My Palette` — tokens
/// `["palette=My", "Palette"]` — rebuilds as `palette="My Palette"`,
/// which is exactly the line the user meant.
///
/// A line that reaches the hint for some *other* reason still gets
/// a suggestion made only of words it contains: `border color=#fff
/// preset heavy` rebuilds as `color="#fff preset"`, which is what
/// the message's hypothesis implies rather than a good guess at the
/// user's intent. That is the honest limit of a single wording
/// serving both shapes, and it beats inventing a key.
///
/// `None` only if no kv precedes the positional — impossible at
/// every call site, since the hint fires precisely when one does.
fn split_kv_suggestion(tokens: &[String], verb_pos: usize) -> Option<String> {
    let end = tokens
        .iter()
        .enumerate()
        .filter(|(_, t)| !is_kv_token(t))
        .map(|(i, _)| i)
        .nth(verb_pos)?;
    let kv_at = tokens[..end].iter().rposition(|t| is_kv_token(t))?;
    let (key, head) = split_kv(&tokens[kv_at])?;
    // `kv_at` is the *last* kv before `end`, so the tail is all
    // positional — the run one pair of quotes would have kept whole.
    let mut value = String::from(head);
    for t in &tokens[kv_at + 1..=end] {
        value.push(' ');
        value.push_str(t);
    }
    Some(format!("{}=\"{}\"", key, value))
}
