// SPDX-License-Identifier: MPL-2.0

//! The corpus the differential oracle runs.

use super::oracle::Sel;

/// `(selection fixture, input line)` — every line is executed and
/// its outcome compared against the pinned signature.
pub const EXEC_CORPUS: &[(Sel, &str)] = &[
    // help
    (Sel::Node, "help"),
    (Sel::Node, "help all"),
    (Sel::Node, "help color"),
    (Sel::Node, "help font"),
    (Sel::Node, "help nope"),
    // anchor
    (Sel::Edge, "anchor"),
    (Sel::Edge, "anchor from=top"),
    (Sel::Edge, "anchor from=top to=bottom"),
    (Sel::Edge, "anchor from=diagonal"),
    (Sel::Edge, "anchor bogus=1"),
    (Sel::Node, "anchor from=top"),
    // A bare form's slots are counted from the verb, not from a
    // subverb it does not have, so a stray positional on a kv-only
    // verb is named rather than dropped.
    (Sel::Edge, "anchor sideways"),
    // body
    (Sel::Edge, "body"),
    (Sel::Edge, "body glyph=dash"),
    (Sel::Edge, "body glyph=nope"),
    (Sel::Edge, "body bogus=1"),
    // A repeated key resolves to its **last** occurrence, the shell
    // intuition `spec::kvs::value` implements with `rfind`. Two rows
    // in opposite orders, because one alone is satisfied by a reader
    // that takes the first: these two swap answers under that
    // reading, and `dash`/`dot` are chosen so the fixture's own
    // starting glyph makes the two answers differ in wording as well
    // as in value.
    (Sel::Edge, "body glyph=dash glyph=dot"),
    (Sel::Edge, "body glyph=dot glyph=dash"),
    // border
    (Sel::Node, "border"),
    (Sel::Node, "border on"),
    (Sel::Node, "border off"),
    (Sel::Node, "border toggle"),
    (Sel::Node, "border show side=nope"),
    // The three readouts, pinned whole. They spent a while in
    // `EXEC_PREFIX_CORPUS` on the belief that their bodies were
    // measured off the host's fonts — see the reason recorded
    // there for why they are not, and what a prefix let through.
    (Sel::Node, "border show"),
    (Sel::Node, "border show verbose"),
    (Sel::Node, "border show side=top"),
    (Sel::Node, "border reset"),
    (Sel::Node, "border nope"),
    (Sel::Node, "border NOPE"),
    (Sel::Node, "border preset heavy"),
    (Sel::Node, "border preset nope"),
    (Sel::Node, "border preset cycle"),
    (Sel::Node, "border preset"),
    (Sel::Node, "border color accent"),
    (Sel::Node, "border color"),
    (Sel::Node, "border padding 4"),
    (Sel::Node, "border padding abc"),
    (Sel::Node, "border padding"),
    // The `(composed, staged)` shape of the extra-positional
    // rejection — the one the whole border family reaches, and the
    // one no corpus row covered while a doc comment in
    // `spec/tests.rs` asserted that two of them did. The sentence
    // changed here in the migration's first commit with nothing
    // pinning it; it is pinned now, on two surfaces, so the level
    // it is built from is visible in the diff.
    (Sel::Node, "border padding 12 50"),
    (Sel::Node, "border palette off"),
    (Sel::Node, "border palette"),
    (Sel::Node, "border font off"),
    // `stage_font` compared this sentinel exactly while its two
    // siblings on the same verb — `stage_palette`, `stage_field` —
    // read theirs through `eq_ignore_ascii_case`, so `border
    // palette OFF` cleared and `border font OFF` answered "font
    // 'OFF' is not a loaded font".
    (Sel::Node, "border font OFF"),
    (Sel::Node, "border font"),
    (Sel::Node, "border side top reset"),
    (Sel::Node, "border side nope reset"),
    (Sel::Node, "border side top"),
    (Sel::Node, "border side"),
    (Sel::Node, "border corner tl reset"),
    (Sel::Node, "border corner nope x"),
    (Sel::Node, "border corner tl"),
    (Sel::Node, "border corner"),
    (Sel::Node, "border preset=heavy"),
    (Sel::Node, "border size=12"),
    (Sel::Node, "border size=abc"),
    (Sel::Node, "border bogus=1"),
    (Sel::Node, "border preview preset=heavy"),
    // The shared closing move (`border::BorderEdit`) on all four
    // surfaces that make it: the auto-promotion note and the bare
    // `preset=custom` hint. Nothing pinned either before this,
    // which left the two most-copied branches of the four tails
    // answerable only per-verb.
    (Sel::Node, "border preset=heavy top=###"),
    (Sel::Node, "border preset=custom"),
    (Sel::Node, "border preview preset=heavy top=###"),
    (Sel::Node, "border preview preset=custom"),
    (Sel::Node, "canvas border preset=custom"),
    (Sel::Section, "section frame preset=custom"),
    (Sel::Node, "border preview commit"),
    (Sel::Node, "border preview cancel"),
    (Sel::Node, "border preview nope"),
    // The shared preview dispatcher normalizes the subverb to
    // match on it and used to quote the *normalized* spelling
    // back, so this line answered `unknown subverb 'nope'`. One
    // row per label the dispatcher wears — five, not the three
    // this comment claimed while `canvas section-frame preview`
    // and `canvas section-frame focused preview` had no execute
    // row of any kind — because the wording is the only thing the
    // five verbs do not each own.
    (Sel::Node, "border preview NOPE"),
    (Sel::Node, "border preview"),
    (Sel::Edge, "border on"),
    // The per-node twin of the canvas kv-form row below: seven of
    // the thirteen subverbs the token-0 popup used to offer are
    // refused at a slot a kv already made kv-form, and the other
    // six are not — each for its own reason, which is why the
    // popup withholds only the seven.
    (Sel::Node, "border color=#fff preset heavy"),
    (Sel::Node, "border color=#fff preview"),
    (Sel::Node, "border color=#fff show side=nope"),
    (Sel::Node, "border color=#fff on"),
    // The discriminator reads *position*, and three surfaces asked
    // "is there a kv anywhere" instead. A kv past the subverb slot
    // leaves the slot positional, so each of these is an unknown
    // subverb; each used to answer with the quoting hint naming a
    // word that is not a palette.
    (Sel::Node, "border nope palette=coral"),
    (Sel::Node, "border preview nope palette=coral"),
    (Sel::Node, "canvas border preview nope palette=coral"),
    // …and the lines the hint is actually for, on the two surfaces
    // that now reach it through the shared wording rather than a
    // near-copy of it.
    (Sel::Node, "border preview palette=My Palette"),
    // The hint suggests the line the *user* meant, rebuilt from
    // their own tokens. It used to hardcode `palette` as the key
    // and quote the trailing token as the value, so this line —
    // the single most likely way to hit it, since 43 of this
    // host's 77 families carry a space — answered
    // ``border palette="Sans"``. Host-independent: the
    // discriminator refuses the line before any font lookup.
    (Sel::Node, "border font=DejaVu Sans"),
    // canvas
    (Sel::Node, "canvas"),
    (Sel::Node, "canvas nope"),
    (Sel::Node, "canvas NOPE"),
    (Sel::Node, "canvas border"),
    (Sel::Node, "canvas border show"),
    (Sel::Node, "canvas border reset"),
    (Sel::Node, "canvas border preset heavy"),
    (Sel::Node, "canvas border preset nope"),
    (Sel::Node, "canvas border nope"),
    // The two canvas subjects handed the *lowercased* subverb to
    // `apply_positional`, and `unknown_canvas_subverb_message`
    // quotes whatever it is handed — so these three answered with
    // a word the user never typed, the same defect
    // `dispatch_border_preview` was fixed for one commit earlier.
    // Every subverb rejection in the border family reads the
    // user's own spelling back now.
    (Sel::Node, "canvas border NOPE"),
    (Sel::Node, "canvas border bogus=1"),
    (Sel::Node, "canvas border preset=heavy"),
    (Sel::Node, "canvas border padding abc"),
    // The same shape one level down: the suggestion names `canvas
    // border`, not `border`, because it is read off the level the
    // rejection printed from.
    (Sel::Node, "canvas border padding 12 50"),
    (Sel::Node, "canvas border side top reset"),
    (Sel::Node, "canvas border corner tl reset"),
    (Sel::Node, "canvas border corner nope x"),
    // A kv ahead of the subverb slot means kv form, on the canvas
    // surfaces exactly as on the per-node one — this dispatched
    // positionally and dropped the color without a word.
    (Sel::Node, "canvas border color=#fff preset heavy"),
    // …and the three the discriminator does *not* gate, on both
    // subjects and past the `focused` modifier. These are why the
    // popup withholds seven of ten rather than all ten.
    (Sel::Node, "canvas border color=#fff show"),
    (Sel::Node, "canvas border color=#fff reset"),
    (Sel::Node, "canvas border color=#fff preview commit"),
    (Sel::Node, "canvas section-frame color=#fff focused preset heavy"),
    (Sel::Node, "canvas section-frame color=#fff focused show"),
    (Sel::Node, "canvas section-frame"),
    (Sel::Node, "canvas section-frame show"),
    (Sel::Node, "canvas section-frame reset"),
    (Sel::Node, "canvas section-frame nope"),
    (Sel::Node, "canvas section-frame NOPE"),
    (Sel::Node, "canvas section-frame focused show"),
    (Sel::Node, "canvas section-frame focused reset"),
    (Sel::Node, "canvas section-frame focused nope"),
    (Sel::Node, "canvas section-frame focused NOPE"),
    (Sel::Node, "canvas section-frame side top reset"),
    (Sel::Node, "canvas section-frame focused side top reset"),
    (Sel::Node, "canvas border preview commit"),
    (Sel::Node, "canvas border preview cancel"),
    (Sel::Node, "canvas border preview nope"),
    (Sel::Node, "canvas border preview NOPE"),
    // The shared preview dispatcher's remaining two labels. Both
    // behave correctly and always did; neither had an execute row
    // of any kind, which is how the comment above came to claim
    // three rows "one per label" for a dispatcher wearing five.
    (Sel::Node, "canvas section-frame preview NOPE"),
    (Sel::Node, "canvas section-frame focused preview NOPE"),
    // cap
    (Sel::Edge, "cap"),
    (Sel::Edge, "cap from=arrow"),
    (Sel::Edge, "cap from=arrow to=none"),
    (Sel::Edge, "cap from=nope"),
    (Sel::Edge, "cap bogus=1"),
    // color
    (Sel::Node, "color"),
    (Sel::Node, "color bg=#112233"),
    (Sel::Node, "color bg=nope"),
    (Sel::Node, "color bogus=1"),
    (Sel::Node, "color pick"),
    (Sel::Node, "color bg"),
    (Sel::Node, "color text"),
    (Sel::Node, "color border"),
    (Sel::Node, "color picker on"),
    (Sel::Node, "color picker off"),
    (Sel::Node, "color picker"),
    (Sel::Node, "color picker nope"),
    (Sel::Node, "color nope"),
    (Sel::Section, "color bg"),
    (Sel::Node, "color section=0 bg=#112233"),
    (Sel::Node, "color section=abc bg=#112233"),
    (Sel::TwoSectionNode, "color section=0 range=0..2 text=accent"),
    (Sel::TwoSectionNode, "color range=0..2 text=accent"),
    (Sel::TwoSectionNode, "color section=0 range=zz text=accent"),
    // edge
    (Sel::Edge, "edge"),
    (Sel::Edge, "edge type=cross_link"),
    (Sel::Edge, "edge type=nope"),
    (Sel::Edge, "edge reset=straight"),
    (Sel::Edge, "edge reset=nope"),
    (Sel::Edge, "edge display_mode=portal"),
    (Sel::Edge, "edge display_mode=nope"),
    (Sel::Edge, "edge bogus=1"),
    // font
    (Sel::Node, "font"),
    (Sel::Node, "font set"),
    (Sel::Node, "font set NoSuchFamilyHere"),
    (Sel::Node, "font nope"),
    (Sel::Node, "font size=14"),
    (Sel::Node, "font size=abc"),
    (Sel::Node, "font size=0"),
    (Sel::Node, "font bogus=1"),
    (Sel::Edge, "font min=8 max=4"),
    (Sel::Node, "font range=0..2"),
    (Sel::TwoSectionNode, "font section=0 range=0..2 size=12"),
    (Sel::TwoSectionNode, "font section=0 range=zz size=12"),
    (Sel::TwoSectionNode, "font section=abc size=12"),
    (Sel::TwoSectionNode, "font section=0 min=8"),
    // fps
    (Sel::Node, "fps"),
    (Sel::Node, "fps on"),
    (Sel::Node, "fps off"),
    (Sel::Node, "fps debug"),
    (Sel::Node, "fps nope"),
    // spacing
    (Sel::Edge, "spacing"),
    (Sel::Edge, "spacing value=tight"),
    (Sel::Edge, "spacing value=3.5"),
    (Sel::Edge, "spacing value=nope"),
    (Sel::Edge, "spacing bogus=1"),
    // label
    (Sel::EdgeLabel, "label"),
    (Sel::EdgeLabel, "label clear"),
    (Sel::EdgeLabel, "label text=hi"),
    (Sel::EdgeLabel, "label text=hi position=start"),
    (Sel::EdgeLabel, "label position=nope"),
    (Sel::EdgeLabel, "label position_t=0.5"),
    (Sel::EdgeLabel, "label position_t=abc"),
    (Sel::EdgeLabel, "label perpendicular=2"),
    (Sel::EdgeLabel, "label bogus=1"),
    (Sel::EdgeLabel, "label nope"),
    // mode
    (Sel::Node, "mode"),
    (Sel::Node, "mode default"),
    (Sel::Node, "mode resize"),
    (Sel::Node, "mode node-edit"),
    (Sel::Node, "mode nope"),
    // mutation
    (Sel::Node, "mutation"),
    (Sel::Node, "mutation list"),
    (Sel::Node, "mutation list --all"),
    (Sel::Node, "mutation apply"),
    (Sel::Node, "mutation apply no-such-mutation"),
    (Sel::Node, "mutation help"),
    (Sel::Node, "mutation help no-such-mutation"),
    (Sel::Node, "mutation inspect"),
    (Sel::Node, "mutation inspect no-such-mutation"),
    (Sel::Node, "mutation nope"),
    // A kv ahead of the sub-command slot leaves the slot
    // positional, so this has always run — the popup is what
    // did not know it (see `complete_mutation`).
    (Sel::Node, "mutation x=1 list"),
    // node
    (Sel::Node, "node"),
    (Sel::Node, "node fit"),
    (Sel::Node, "node edit"),
    (Sel::Node, "node resize"),
    (Sel::Node, "node resize 120 60"),
    (Sel::Node, "node resize a b"),
    (Sel::Node, "node nope"),
    // open / new / save
    (Sel::Node, "open"),
    // The same count, one slot along: `open` declares a path and
    // refuses a second one instead of loading the first in silence.
    (Sel::Node, "open a.mindmap.json b.mindmap.json"),
    (Sel::Node, "save"),
    (Sel::Node, "new"),
    // section
    (Sel::TwoSectionNode, "section"),
    (Sel::TwoSectionNode, "section show"),
    (Sel::TwoSectionNode, "section nope"),
    (Sel::TwoSectionNode, "section move"),
    (Sel::Section, "section move dx=1 dy=1"),
    (Sel::Section, "section move dx=1 x=1"),
    (Sel::Section, "section move x=1 y=1"),
    (Sel::Section, "section move dx=abc"),
    (Sel::Section, "section move bogus=1"),
    (Sel::Section, "section resize fill"),
    (Sel::Section, "section resize w=40 h=20"),
    (Sel::Section, "section resize"),
    (Sel::Section, "section resize bogus=1"),
    // `resize` declares two forms — the `fill` literal and the
    // `w=`/`h=` pair — and the engine reads the *union* of a
    // subverb's forms, so the kvs are accepted here and then
    // dropped by a handler that returns on `fill` before it looks
    // at them. Pinned as the defect it is; corrected in the commit
    // that narrows the kv check to the forms the positionals admit.
    (Sel::Section, "section resize fill w=99 h=99"),
    (Sel::Section, "section text hi"),
    (Sel::Section, "section text hi runs=clear"),
    (Sel::Section, "section text hi runs=nope"),
    (Sel::Section, "section text bogus=1"),
    (Sel::TwoSectionNode, "section add"),
    (Sel::TwoSectionNode, "section add at=0"),
    (Sel::TwoSectionNode, "section add at=abc"),
    (Sel::TwoSectionNode, "section add bogus=1"),
    (Sel::Section, "section delete"),
    (Sel::Section, "section delete bogus=1"),
    (Sel::Section, "section split"),
    (Sel::Section, "section split at=1"),
    (Sel::Section, "section split at=abc"),
    (Sel::Section, "section split bogus=1"),
    (Sel::TwoSectionNode, "section show bogus=1"),
    (Sel::TwoSectionNode, "section show section=99"),
    (Sel::TwoSectionNode, "section show section=abc"),
    (Sel::Section, "section edit"),
    (Sel::Section, "section edit bogus=1"),
    (Sel::Section, "section frame"),
    (Sel::Section, "section frame show"),
    (Sel::Section, "section frame reset"),
    (Sel::Section, "section frame nope"),
    (Sel::Section, "section frame NOPE"),
    (Sel::Section, "section frame nope palette=coral"),
    (Sel::Section, "section frame palette=My Palette"),
    (Sel::Section, "section frame preset=heavy"),
    (Sel::Section, "section frame bogus=1"),
    (Sel::Section, "section frame preview preset=heavy"),
    (Sel::Section, "section frame preview commit"),
    (Sel::Section, "section frame preview cancel"),
    (Sel::Section, "section frame preview nope"),
    (Sel::Section, "section frame preview NOPE"),
    (Sel::TwoSectionNode, "section frame section=1 preset=heavy"),
    (Sel::TwoSectionNode, "section frame section=abc preset=heavy"),
    (Sel::Node, "section show"),
    (Sel::None, "section show"),
    (Sel::Multi, "section show"),
    (Sel::Multi, "border on"),
    (Sel::Multi, "font size=14"),
    (Sel::Multi, "font min=8"),
    (Sel::Multi, "color bg=#112233"),
    // zoom
    (Sel::Node, "zoom"),
    (Sel::Node, "zoom clear"),
    (Sel::Node, "zoom min=0.5"),
    (Sel::Node, "zoom min=unset"),
    (Sel::Node, "zoom min=abc"),
    (Sel::Node, "zoom min=0"),
    (Sel::Node, "zoom min=2 max=1"),
    (Sel::Node, "zoom bogus=1"),
    (Sel::Node, "zoom nope"),
    (Sel::None, "zoom min=0.5"),
    // Casing. Command names, aliases and positional subverbs are
    // matched case-insensitively console-wide
    // (`commands/mod.rs` § Casing); every subverb popup already
    // filtered its partial that way and inserted the canonical
    // spelling, but only five verbs honored it on the execute
    // side, so a word the popup listed could still be refused
    // when typed out. One row per verb whose dispatch normalizes,
    // each the upper-case twin of a row above.
    (Sel::Node, "help ALL"),
    (Sel::Node, "help BORDER"),
    (Sel::Node, "border PRESET heavy"),
    (Sel::Node, "border PREVIEW COMMIT"),
    (Sel::Node, "canvas BORDER PRESET heavy"),
    (Sel::Node, "canvas SECTION-FRAME FOCUSED show"),
    (Sel::Node, "color PICKER ON"),
    (Sel::Node, "font SET NoSuchFamilyHere"),
    (Sel::Node, "fps ON"),
    (Sel::EdgeLabel, "label CLEAR"),
    (Sel::Node, "mode DEFAULT"),
    (Sel::Node, "mutation LIST"),
    (Sel::Node, "node FIT"),
    (Sel::TwoSectionNode, "section FRAME SHOW"),
    (Sel::Section, "section frame PREVIEW commit"),
    (Sel::Node, "zoom CLEAR"),
    // …and the two halves that stay exact. A kv key is a field
    // name, not a word the user picks, so `TOP=` is unknown on
    // both sides; a palette name is stored verbatim, so the value
    // is compared as written even though the popup finds it
    // case-insensitively.
    (Sel::Node, "border TOP=x"),
    (Sel::Node, "font SIZE=14"),
];

/// `(selection fixture, input line)` — the completion popup for
/// each line (cursor at end of line) is compared against the
/// pinned signature.
pub const COMPLETION_CORPUS: &[(Sel, &str)] = &[
    (Sel::Node, ""),
    (Sel::Node, "co"),
    (Sel::Edge, "b"),
    (Sel::Node, "help "),
    (Sel::Node, "help co"),
    (Sel::Node, "help a"),
    (Sel::Edge, "anchor "),
    (Sel::Edge, "anchor fr"),
    (Sel::Edge, "anchor from="),
    (Sel::Edge, "anchor from=t"),
    (Sel::Edge, "body "),
    (Sel::Edge, "body glyph="),
    (Sel::Edge, "body glyph=d"),
    (Sel::Node, "border "),
    (Sel::Node, "border pre"),
    (Sel::Node, "border preset "),
    (Sel::Node, "border preset h"),
    (Sel::Node, "border color "),
    (Sel::Node, "border palette "),
    (Sel::Node, "border side "),
    (Sel::Node, "border side top "),
    (Sel::Node, "border corner "),
    (Sel::Node, "border corner tl "),
    (Sel::Node, "border show "),
    (Sel::Node, "border show side="),
    (Sel::Node, "border preview "),
    // A kv ahead of the subverb slot puts the line in kv form, so
    // the positional vocabulary is not what would run.
    (Sel::Node, "border color=#fff preset "),
    // …and the slot that *emits* that vocabulary. The gate reached
    // the lookahead above eight lines before it reached this row's
    // arm, so the popup went on offering thirteen subverbs at a
    // slot the verb refuses seven of.
    (Sel::Node, "border color=#fff "),
    // Subverb names are matched case-insensitively on the execute
    // side; the popup follows so the two agree on the same line.
    (Sel::Node, "border PRESET "),
    (Sel::Node, "border preset="),
    (Sel::Node, "border field="),
    (Sel::Node, "border size="),
    (Sel::Node, "border padding "),
    (Sel::Node, "border top="),
    (Sel::Node, "canvas "),
    (Sel::Node, "canvas bo"),
    (Sel::Node, "canvas border "),
    (Sel::Node, "canvas border pre"),
    (Sel::Node, "canvas border preset "),
    (Sel::Node, "canvas border preview "),
    (Sel::Node, "canvas border preset="),
    (Sel::Node, "canvas border show "),
    (Sel::Node, "canvas border color "),
    (Sel::Node, "canvas border palette "),
    (Sel::Node, "canvas border padding "),
    // The depth-2 slots: the value *after* a `side` / `corner`
    // which-arg, on all three canvas surfaces. `EXEC_CORPUS` has
    // pinned `canvas border side top reset` as working since the
    // oracle landed while this table had no counterpart, and that
    // asymmetry is what hid a popup answering with kv keys.
    (Sel::Node, "canvas border side "),
    (Sel::Node, "canvas border side top "),
    (Sel::Node, "canvas border corner "),
    (Sel::Node, "canvas border corner tl "),
    (Sel::Node, "canvas border color=#fff side "),
    (Sel::Node, "canvas Border SIDE top "),
    // The three canvas slots that emit the subverb vocabulary: the
    // `border` subject's, the `section-frame` subject's, and the
    // one past the `focused` modifier. All three offered every
    // positional subverb at a slot `execute_canvas` reads as kv
    // form and rejects seven of.
    (Sel::Node, "canvas color=#fff border "),
    (Sel::Node, "canvas color=#fff section-frame "),
    (Sel::Node, "canvas section-frame color=#fff focused "),
    (Sel::Node, "canvas section-frame "),
    (Sel::Node, "canvas section-frame focused "),
    (Sel::Node, "canvas section-frame focused preview "),
    (Sel::Node, "canvas section-frame preset "),
    (Sel::Node, "canvas section-frame side "),
    (Sel::Node, "canvas section-frame side top "),
    (Sel::Node, "canvas section-frame focused preset "),
    (Sel::Node, "canvas section-frame focused side "),
    (Sel::Node, "canvas section-frame focused side top "),
    (Sel::Node, "canvas section-frame focused corner tl "),
    (Sel::Edge, "cap "),
    (Sel::Edge, "cap from="),
    (Sel::Node, "color "),
    (Sel::Node, "color bg="),
    (Sel::Node, "color picker "),
    (Sel::Node, "color se"),
    (Sel::Node, "color ra"),
    (Sel::TwoSectionNode, "color range="),
    (Sel::Edge, "edge "),
    (Sel::Edge, "edge type="),
    (Sel::Edge, "edge reset="),
    (Sel::Edge, "edge display_mode="),
    (Sel::Node, "font "),
    (Sel::Node, "font se"),
    (Sel::Node, "font size="),
    (Sel::TwoSectionNode, "font section="),
    (Sel::Node, "font ra"),
    (Sel::TwoSectionNode, "font range="),
    (Sel::TwoSectionNode, "color section="),
    (Sel::Node, "fps "),
    (Sel::Node, "fps o"),
    (Sel::Edge, "spacing "),
    (Sel::Edge, "spacing value="),
    (Sel::EdgeLabel, "label "),
    (Sel::EdgeLabel, "label position="),
    (Sel::EdgeLabel, "label text="),
    (Sel::Node, "mode "),
    (Sel::Node, "mode d"),
    (Sel::Node, "mutation "),
    (Sel::Node, "mutation list "),
    (Sel::Node, "mutation apply "),
    // The two lines the positional-slot switch moved on this
    // verb, and the two it moved on `help`. All four are the
    // popup agreeing with the verb where it used not to; the
    // comment that called the switch a no-op is corrected at
    // both sites.
    (Sel::Node, "mutation x=1 "),
    (Sel::Node, "mutation ap=li"),
    (Sel::Node, "help bg=x "),
    (Sel::Node, "help x="),
    (Sel::Node, "node "),
    (Sel::Node, "node r"),
    (Sel::Node, "open "),
    (Sel::Node, "save "),
    (Sel::Node, "new "),
    (Sel::TwoSectionNode, "section "),
    (Sel::TwoSectionNode, "section m"),
    (Sel::TwoSectionNode, "section move "),
    (Sel::TwoSectionNode, "section resize "),
    // The popup half of the same defect: `fill` is behind the
    // cursor, so the only form still in play is the one that reads
    // `section=` alone — and the two size keys are offered anyway.
    (Sel::TwoSectionNode, "section resize fill "),
    (Sel::TwoSectionNode, "section text "),
    (Sel::TwoSectionNode, "section add "),
    (Sel::TwoSectionNode, "section split "),
    (Sel::TwoSectionNode, "section show "),
    (Sel::TwoSectionNode, "section delete "),
    (Sel::TwoSectionNode, "section edit "),
    (Sel::TwoSectionNode, "section show section="),
    (Sel::TwoSectionNode, "section text runs="),
    (Sel::TwoSectionNode, "section frame "),
    (Sel::TwoSectionNode, "section frame preset="),
    (Sel::TwoSectionNode, "section frame preview "),
    (Sel::TwoSectionNode, "section frame preset "),
    (Sel::TwoSectionNode, "section frame se"),
    (Sel::TwoSectionNode, "section frame section="),
    (Sel::TwoSectionNode, "section frame preview section="),
    (Sel::Node, "zoom "),
    (Sel::Node, "zoom c"),
    (Sel::Node, "zoom min="),
    (Sel::Node, "zoom max="),
    // Casing, popup side. Each row is the upper-case twin of a
    // lower-case row above and must answer identically: the
    // popup's filter and the verb's dispatch read the same way
    // (`commands/mod.rs` § Casing). `border PRE` and `section
    // frame PREVIEW ` answered nothing at all before — a hard
    // stop on a line each verb accepts.
    (Sel::Node, "border PRE"),
    (Sel::Node, "border preset=H"),
    (Sel::Node, "border palette=CO"),
    // The `off` sentinel this slot's usage line has always
    // documented and no completer offered. Filtered the way
    // `stage_font` reads it, which is the way its two siblings on
    // this verb already read theirs.
    (Sel::Node, "border font=OF"),
    (Sel::Node, "border side top RE"),
    (Sel::Node, "border show VER"),
    (Sel::Node, "border preview CO"),
    (Sel::Node, "canvas SECTION-FRAME FO"),
    (Sel::Node, "color PICKER "),
    (Sel::Node, "font SE"),
    (Sel::Node, "fps O"),
    (Sel::EdgeLabel, "label CL"),
    (Sel::Node, "mode D"),
    (Sel::Node, "mutation LI"),
    (Sel::Node, "node R"),
    (Sel::TwoSectionNode, "section FRAME "),
    (Sel::TwoSectionNode, "section frame PREVIEW "),
    (Sel::Node, "help AL"),
    (Sel::Node, "zoom C"),
    // The exact half: a kv key filters case-sensitively because
    // `stage_kv` matches it that way, so an upper-case key offers
    // nothing rather than offering a row the verb would reject.
    (Sel::Node, "border TOP"),
];

/// The one outcome whose tail this machine chose rather than the
/// console — pinned as a prefix, with the host-dependent bytes
/// left off the end.
///
/// Rust's `Display` for `io::Error` goes through `strerror_r`, so
/// the words after the path are glibc's and are
/// `LC_MESSAGES`-sensitive: the same `open` of the same missing
/// file reads "No such file or directory" under `LC_ALL=C` and
/// something else under a translated locale. Pinning through the
/// errno string would assert the tester's locale, not the
/// console's behavior. The prefix that stops short of it is not
/// vacuous — it is every byte the console itself contributes, so
/// it pins `open.rs`'s `"open {path}: {e}"` wrapper and the
/// loader's own `"Failed to read file {path}: "` inside it, and a
/// change to either fails this row.
///
/// **Prefix matching is the weaker check, so exactly one row gets
/// it.** The three `border show` readouts sat here too, under a
/// "measured layout" heading claiming their bodies depend on the
/// host's installed fonts. They do not: the cluster count is
/// `node.size.x / (14.0 * BORDER_APPROX_CHAR_WIDTH_FRAC)` — a
/// constant fraction of a constant floor, never a font metric —
/// and the side runs are `SidePattern::render`'s string
/// repetition. Nothing there is measured off a face, so all three
/// are pinned whole in [`EXEC_CORPUS`] instead. Under a prefix
/// they were not merely weaker but *silently* weaker: inserting a
/// whole new line into the readout between `padding:` and `size:`
/// passed every test in the workspace.
pub const EXEC_PREFIX_CORPUS: &[(Sel, &str)] = &[(Sel::Node, "open /nonexistent-dir-xyz/x.mindmap.json")];

/// Lines run **in order against one document**, for the outcomes a
/// fresh-document row cannot reach.
///
/// Every row of [`EXEC_CORPUS`] runs on a document of its own, which
/// is what makes those rows independent and reorderable — and what
/// puts a whole branch of the border family outside the oracle's
/// reach. `border preview commit` only prints its auto-promotion
/// note when a preview is *active*, so no single line can pin it, and
/// `commit_border_preview_verb` accordingly carried a fifth,
/// hand-written copy of the note the four committing surfaces share
/// through `border::finish` — through a commit whose stated control
/// ("perturb the shared note and all six new rows move") could not
/// reach it, because none of those six rows is a sequence.
///
/// The cost of a stateful row is that it depends on its
/// predecessors, so a row here is a *whole* interaction rather than a
/// step in one, and the signature pins every line of it. That keeps
/// the failure output readable: a moved row shows which line of the
/// sequence changed.
pub const EXEC_SEQ_CORPUS: &[(Sel, &[&str])] = &[
    // The auto-promotion note on the commit line — the branch no
    // fresh-document row can reach, on all three surfaces that stage
    // a preview.
    (
        Sel::Node,
        &["border preview preset=heavy top=###", "border preview commit"],
    ),
    (
        Sel::Section,
        &[
            "section frame preview preset=heavy top=###",
            "section frame preview commit",
        ],
    ),
    (
        Sel::Node,
        &[
            "canvas border preview preset=heavy top=###",
            "canvas border preview commit",
        ],
    ),
    // The same tail with nothing to promote: one line, not `Lines`.
    (
        Sel::Node,
        &["border preview preset=heavy", "border preview commit"],
    ),
    // `preset=custom` stages the orientation hint and commits without
    // it — the hint belongs to the line that carried the kv.
    (
        Sel::Node,
        &["border preview preset=custom", "border preview commit"],
    ),
    // Terminators against no preview, and the cancel path.
    (Sel::Node, &["border preview commit"]),
    (
        Sel::Node,
        &["border preview preset=heavy", "border preview cancel"],
    ),
];
