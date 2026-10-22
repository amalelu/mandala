// SPDX-License-Identifier: MPL-2.0

//! What the **derive macro** says the on-disk shape is, asked of the
//! generated code rather than of the source text that produced it.
//!
//! [`crate::util::serde_coverage`] answers the same questions by
//! parsing baumhard's own `.rs` files with `syn`. That walk is the
//! only thing `format/schema.md`'s published array list rests on, and
//! a walk is a *reading* of the source — it can be wrong in ways the
//! document it feeds is then wrong in too. The two agree because they
//! share a derivation, not because the derivation is right, and
//! [issue #122](https://github.com/Svendsys/mandala/issues/122) is the
//! record of that: a `Vec` behind a type alias, a `VecDeque`, a
//! `#[serde(rename(deserialize = "…"))]`, a container `rename_all` —
//! each one produces a wrong published name or a missing array, and
//! nothing disagrees.
//!
//! So something has to disagree, and it has to get its answer from
//! somewhere else. `serde_derive` already wrote the truth down: the
//! `Deserialize` impl it generates for a struct calls
//! `Deserializer::deserialize_struct(name, FIELDS, visitor)` where
//! `FIELDS` is the exact list of JSON member names — every `rename`
//! spelling resolved, `rename_all` applied, the identifier used only
//! where nothing overrode it. An enum's impl names its variants the
//! same way. A `Vec` field asks for [`Deserializer::deserialize_seq`]
//! whatever it is spelled or aliased as, and a tuple asks for
//! [`Deserializer::deserialize_tuple`], which is the distinction
//! between "an index an edit can move" and "a position the format
//! fixes".
//!
//! [`derived_shape`] hands the derived impls a `Deserializer` that
//! answers every request with the emptiest value that satisfies it and
//! writes down what was asked. Nothing is parsed, nothing is listed,
//! and no `.mindmap.json` is involved: the walk is the compiler's own
//! expansion of the model, driven from `MindMap::deserialize` outward.
//!
//! `#[serde(alias = "…")]` is in that truth too, and reading it the
//! other way round was the module's first real defect. The derive
//! puts a field's aliases into the same `FIELDS` list as its name and
//! a variant's into `VARIANTS`, grouped with the name they alias and
//! sorted inside the group — so the probe sees every spelling, and
//! the published list carries all of them. What the list does *not*
//! say is where one field's group ends, and offering it blindly hands
//! the derived visitor the same field twice, which it refuses. That
//! refusal is where a group boundary gets learned; see
//! [`fold_a_rejected_spelling`].
//!
//! **It is not a replacement for the source walk and must not become
//! one.** It cannot see a type no value reaches (a serialize-only
//! proxy), and it says nothing about `deny_unknown_fields`. Three
//! shapes stop it outright rather than being seen: a
//! `#[serde(flatten)]` field, an internally or adjacently tagged
//! enum, and a `deserialize_with` that validates what it is handed.
//! One more is worse than any of those, and it is the only one that
//! is not loud: **`#[serde(untagged)]`**, which decides its variant
//! by replaying a buffered `Content` and so quietly matches whichever
//! variant accepts the probe's unit, losing every other variant's
//! shape without an error. That is why [`DerivedShape::opaque_sites`]
//! exists — the silence is reported as a place rather than left as an
//! absence. What the module is good for is exactly one thing: being a
//! **second, independent** answer to "which JSON members hold
//! growable arrays of key-bearing values", so
//! `mindmap::unknown_keys::tests::test_the_derived_positional_arrays_survive_an_independent_derivation`
//! has something that can disagree.
//!
//! Test-only: nothing in a shipped build deserializes a model type
//! from a probe. Unlike its counterpart it reads no files and pulls in
//! no `syn`, so nothing about it is native-only — and it carries the
//! same `not(target_arch = "wasm32")` gate regardless, because every
//! consumer of it does. A module compiled into a build where nothing
//! calls it is a pile of dead-code warnings claiming a portability
//! that no gate exercises.

use serde::de::{
    DeserializeOwned, DeserializeSeed, EnumAccess, IntoDeserializer, MapAccess, SeqAccess, VariantAccess,
    Visitor,
};
use serde::Deserializer;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// The probe's error type.
///
/// `serde_json::Error` rather than an enum of our own:
/// [`CODE_CONVENTIONS.md §9`](../../../../CODE_CONVENTIONS.md) forbids
/// custom error types, `serde::de::Error` demands *some* type with a
/// `custom` constructor, and the one this workspace already
/// deserializes every map through satisfies it. The probe only ever
/// raises one error — a walk that would not terminate — so the type's
/// JSON-shaped extras cost nothing.
type Error = serde_json::Error;

/// How deep the probe will follow a chain of required values before
/// it decides the walk does not terminate.
///
/// A recursion reaches bottom one of two ways, and the probe has to
/// do both. Through an `Option`, a sequence or a map there is an
/// emptier value to answer with, and [`Mode::Terminate`] answers it.
/// Through a **required enum payload** there is not — `Repeat {
/// template: Box<MutatorNode> }` has no empty form — so terminating
/// means picking a different variant, which is what
/// [`terminating_variant`] does. What is left over is a type with no
/// bottoming variant anywhere on the cycle; that one really cannot be
/// deserialized, and the depth is capped so the run stops by name
/// rather than spinning.
const MAX_DEPTH: usize = 64;

/// How many passes the variant sweep will make before it gives up.
///
/// One pass per enum variant that needs reaching is the shape of the
/// bound; the cap is far above the model's variant count so that
/// crossing it means the choice tree grew a dimension nobody
/// intended, not that a variant was added.
const MAX_RUNS: usize = 4096;

/// Whether the walk is still discovering shape or only trying to
/// bottom out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Sequences yield one element, `Option`s yield `Some`, maps
    /// yield one entry — every container is opened so what is inside
    /// it is recorded.
    Explore,
    /// Sequences, `Option`s and maps come back empty. Entered when a
    /// container reappears on its own path, which is the only way a
    /// deserializable type recurses.
    Terminate,
}

/// One growable array the derived impls asked for.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DerivedSequence {
    /// The JSON member the array sits under — a struct field's
    /// serde name, or the variant name of an externally tagged
    /// newtype variant, which is the member an array payload is
    /// written under.
    pub member: String,
    /// `true` when an unrecognized key can be captured at or below an
    /// element: somewhere inside one, serde asks for a struct or a
    /// struct variant, which is the shape whose unclaimed members
    /// reach `deserialize_ignored_any`.
    pub key_bearing: bool,
    /// Where the probe met it, as `Container.member`, for a failure
    /// message a reader can act on.
    pub site: String,
}

/// Everything one [`derived_shape`] sweep recorded.
#[derive(Debug, Clone, Default)]
pub struct DerivedShape {
    members: BTreeMap<String, BTreeSet<String>>,
    variants: BTreeMap<String, Vec<String>>,
    sequences: BTreeMap<String, DerivedSequence>,
    unnamed_sequences: BTreeSet<String>,
    opaque_sites: BTreeSet<String>,
}

impl DerivedShape {
    /// Every JSON member name that holds a growable array whose
    /// elements can carry named keys — the compiler's answer to the
    /// question `format/schema.md` publishes.
    ///
    /// Cost: one pass over the recorded sequences; allocates the set.
    pub fn key_bearing_sequences(&self) -> BTreeSet<String> {
        self.sequences
            .values()
            .filter(|seq| seq.key_bearing)
            .map(|seq| seq.member.clone())
            .collect()
    }

    /// Every growable array the sweep met, key-bearing or not, keyed
    /// by member name — so a caller can say *why* a member is absent
    /// from [`Self::key_bearing_sequences`] rather than only that it
    /// is.
    pub fn sequences(&self) -> &BTreeMap<String, DerivedSequence> {
        &self.sequences
    }

    /// The JSON member names the derived `Deserialize` impl for
    /// `container` accepts, or `None` when no value of that type was
    /// reached. Struct variants are keyed `"Enum::Variant"`.
    pub fn members_of(&self, container: &str) -> Option<&BTreeSet<String>> {
        self.members.get(container)
    }

    /// Every container name the sweep reached, in sorted order.
    pub fn containers(&self) -> impl Iterator<Item = &str> {
        self.members.keys().map(String::as_str)
    }

    /// The variant names the derived impl for `container` accepts.
    pub fn variants_of(&self, container: &str) -> Option<&[String]> {
        self.variants.get(container).map(Vec::as_slice)
    }

    /// Growable arrays the sweep met at a place with **no** JSON
    /// member name to publish them under — inside a tuple, as the
    /// value of a map whose keys are data, or nested directly inside
    /// another array, as `Vec<Vec<T>>` is.
    ///
    /// Empty for the model today. It is reported rather than dropped
    /// for the same reason [`crate::util::serde_coverage`] reports a
    /// name it cannot resolve: an array with no publishable name is a
    /// positional route the document cannot warn anybody about, and
    /// silence about it reads as absence.
    ///
    /// The nesting case is one only this derivation can see.
    /// [`crate::util::serde_coverage::TypeGraph::unnamed_sequences_from`]
    /// inspects *fields*, and `Vec<Vec<T>>` is one field, so the walk
    /// classifies the outer array and never asks about the inner one.
    pub fn unnamed_sequences(&self) -> &BTreeSet<String> {
        &self.unnamed_sequences
    }

    /// Every place the walk answered a `deserialize_any` — where it
    /// handed back a value without being told what shape was wanted,
    /// and so learned nothing about what is below.
    ///
    /// Two things ask for one. A self-describing value
    /// (`serde_json::Value`) is deliberately opaque and the model has
    /// two of them. An `#[serde(untagged)]` enum is not: serde
    /// buffers the object into a `Content` and replays it through
    /// each variant, so a `Content::Unit` matches whichever variant
    /// accepts a unit and every other variant's shape disappears
    /// **without an error**. That would be the only blind spot here
    /// that says nothing at all — `flatten` and a validating
    /// `deserialize_with` both stop the run — so it is reported
    /// instead, and a test that pins this set to the members the
    /// model means opaque turns the silence into a red.
    ///
    /// Cost: none; the set is accumulated during the sweep.
    pub fn opaque_sites(&self) -> &BTreeSet<String> {
        &self.opaque_sites
    }
}

/// One enum the walk had to pick a variant at.
#[derive(Debug, Clone)]
struct Decision {
    /// The enum's serde container name.
    container: String,
    /// How many variants it declared.
    options: usize,
    /// Which one this pass took.
    chosen: usize,
}

/// Whether a variant carries anything the walk has to go into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VariantKind {
    /// A unit variant. Nothing below it, so it always bottoms out.
    Unit,
    /// A newtype, tuple or struct variant. Whether it bottoms out
    /// depends on what is inside it.
    Payload,
}

/// What one enum variant has turned out to be, accumulated over the
/// whole sweep rather than one pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct VariantFacts {
    /// `true` once an [`Mode::Explore`] pass has taken this variant —
    /// the coverage question the sweep's queue drains against. A
    /// [`Mode::Terminate`] visit deliberately does not set it: it
    /// walks an emptied subtree, so counting it as reached would end
    /// the sweep with shape still unseen.
    reached: bool,
    /// `Some(_)` once any pass has entered the variant and learned
    /// whether it carries a payload.
    kind: Option<VariantKind>,
    /// `true` once a [`Mode::Terminate`] visit of this variant
    /// returned — proof that taking it reaches bottom.
    bottoms_out: bool,
}

/// What the sweep remembers between passes.
///
/// Three questions live here rather than in a [`Run`] because all
/// three outlive a single pass: which variants still need a pass of
/// their own, which variant to take when the walk is only trying to
/// stop, and which `FIELDS` entries are second spellings of a field
/// an earlier entry already named. The last two are why the memory
/// has to *accumulate* rather than just be shared — each is learned
/// by a pass that then has to act on what it learned.
#[derive(Debug, Default)]
struct Memory {
    /// Per enum name, one entry per `VARIANTS` index.
    facts: BTreeMap<String, Vec<VariantFacts>>,
    /// Per container name, the `FIELDS` indices the derived visitor
    /// resolves to a field an earlier index already claimed, each
    /// mapped to that earlier index.
    ///
    /// `FIELDS` is the concatenation of one sorted group per field —
    /// its serde name and every `#[serde(alias = "…")]` spelling — and
    /// nothing in the list says where a group ends. Offering the whole
    /// list hands the derived visitor the same field twice, which it
    /// rejects; the rejection is where a group boundary is *learned*,
    /// and because the groups are contiguous and every index accepted
    /// so far was a distinct field, the rejected index belongs to the
    /// one immediately before it. Empty for a model with no aliases,
    /// which costs the sweep nothing.
    ///
    /// That last step assumes **no alias spells a name some earlier
    /// field already claimed**. If one does, the visitor rejects it
    /// for the earlier field while this attributes it to the most
    /// recently accepted one, and the offending spelling is filed
    /// under the wrong member — nothing is dropped, and every distinct
    /// field is still offered and walked, but `sequences()`'s
    /// per-member map is wrong for that entry. The shape is
    /// self-announcing: rustc emits `unreachable pattern … no value
    /// can reach this` on such an alias, so it cannot be introduced
    /// silently. No type in the model has one.
    folded: BTreeMap<String, BTreeMap<usize, usize>>,
    /// How many boundaries [`Self::fold`] has recorded. A pass that
    /// failed while this grew is a pass worth replaying.
    learned: usize,
}

impl Memory {
    /// The facts for `container`, sized to `options` on first
    /// sighting.
    fn facts_mut(&mut self, container: &str, options: usize) -> &mut Vec<VariantFacts> {
        self.facts
            .entry(container.to_string())
            .or_insert_with(|| vec![VariantFacts::default(); options])
    }

    /// Record that `container`'s `FIELDS` entry number `at` is
    /// another spelling of the field entry `primary` names.
    fn fold(&mut self, container: &str, at: usize, primary: usize) {
        if self
            .folded
            .entry(container.to_string())
            .or_default()
            .insert(at, primary)
            .is_none()
        {
            self.learned += 1;
        }
    }

    /// Whether `container`'s `FIELDS` entry number `at` is a spelling
    /// some earlier entry already offered.
    fn is_folded(&self, container: &str, at: usize) -> bool {
        self.folded
            .get(container)
            .is_some_and(|folded| folded.contains_key(&at))
    }

    /// Every spelling the derive accepts for the field `container`'s
    /// `FIELDS` entry number `at` names, that entry first.
    ///
    /// A route is recorded against the spelling the *file* used, so an
    /// array reached through an aliased field is published under all
    /// of them — the same answer
    /// [`crate::util::serde_coverage::TypeGraph::key_bearing_sequences_from`]
    /// gives from the other side.
    fn spellings(&self, container: &str, fields: &[&str], at: usize) -> Vec<String> {
        let mut out: Vec<String> = fields
            .get(at)
            .map(|name| (*name).to_string())
            .into_iter()
            .collect();
        if let Some(folded) = self.folded.get(container) {
            out.extend(
                folded
                    .iter()
                    .filter(|(_, primary)| **primary == at)
                    .filter_map(|(spelling, _)| fields.get(*spelling))
                    .map(|name| (*name).to_string()),
            );
        }
        out
    }

    /// Note what `container`'s variant number `at` turned out to be.
    /// Silently ignores an index the enum does not declare, which
    /// cannot happen — the index comes from the same `variants` slice.
    fn note(&mut self, container: &str, at: usize, kind: VariantKind) {
        if let Some(facts) = self.facts.get_mut(container).and_then(|all| all.get_mut(at)) {
            facts.kind = Some(kind);
        }
    }

    /// Note that a [`Mode::Terminate`] visit of `container`'s variant
    /// number `at` reached bottom.
    fn note_bottomed(&mut self, container: &str, at: usize) {
        if let Some(facts) = self.facts.get_mut(container).and_then(|all| all.get_mut(at)) {
            facts.bottoms_out = true;
        }
    }

    /// Whether an [`Mode::Explore`] pass has taken this variant.
    fn reached(&self, container: &str, at: usize) -> bool {
        self.facts
            .get(container)
            .and_then(|all| all.get(at))
            .is_some_and(|facts| facts.reached)
    }
}

/// The variant to take when the walk is no longer discovering shape
/// and only needs to stop, in order of how sure the choice is.
///
/// A unit variant is the emptiest value an enum admits and always
/// bottoms out; one an earlier terminating visit already walked to
/// the end is proven; one nothing has classified yet may be either
/// and is worth trying, since every attempt classifies it and so
/// removes it from the next level's candidates. Variant 0 is the last
/// resort, and reaching it means every variant is known to carry a
/// payload and none has been seen to bottom out — a cycle the depth
/// cap is right to stop.
fn terminating_variant(memory: &Memory, container: &str, options: usize) -> usize {
    let Some(facts) = memory.facts.get(container) else {
        return 0;
    };
    let pick = |wanted: fn(&VariantFacts) -> bool| {
        facts
            .iter()
            .take(options)
            .position(wanted)
            .filter(|at| *at < options)
    };
    pick(|facts| facts.kind == Some(VariantKind::Unit))
        .or_else(|| pick(|facts| facts.bottoms_out))
        .or_else(|| pick(|facts| facts.kind.is_none()))
        .unwrap_or(0)
}

/// A `FIELDS` entry handed to a derived visitor that has not yet
/// asked for its value.
///
/// Set between those two calls and cleared by the second, so it is
/// non-`None` at exactly one moment: after the visitor has resolved a
/// key and before it has accepted it. The only thing the generated
/// code does in that window is reject a field it already has, which
/// is what makes this a duplicate detector rather than a guess.
#[derive(Debug, Clone)]
struct PendingKey {
    /// The container whose [`Fields`] offered it.
    container: String,
    /// Its index in that container's `FIELDS`.
    at: usize,
    /// The last index the same visit offered *and* was asked for the
    /// value of. `FIELDS` groups a field's spellings contiguously, so
    /// a rejection at [`Self::at`] is a second spelling of this one.
    after: Option<usize>,
}

/// State threaded through one pass of the walk.
struct Run<'a> {
    /// Variant index to take at each decision, by decision order. A
    /// decision past the end of the plan takes variant 0.
    plan: Vec<usize>,
    /// The decisions this pass actually met, in order.
    trace: Vec<Decision>,
    /// What the sweep has learned about enum variants and about
    /// `FIELDS` grouping so far.
    memory: &'a mut Memory,
    /// Container names currently open on the walk's path — the
    /// recursion guard.
    active: Vec<String>,
    /// How many times a struct or a struct variant has been asked
    /// for. Compared before and after an element to decide whether
    /// the element can carry named keys.
    named_key_sites: usize,
    /// The key a [`Fields`] map is waiting to be asked the value of.
    pending_key: Option<PendingKey>,
    /// What the sweep has recorded.
    shape: DerivedShape,
}

/// Ask the derived `Deserialize` impls reachable from `T` what shape
/// they expect, and record it.
///
/// The sweep runs the walk repeatedly. Each pass takes one variant at
/// every enum it meets; afterwards every variant it did *not* take,
/// and that no earlier pass took, is queued as a plan of its own —
/// the same choices up to that enum, then the untaken variant. The
/// queue drains when every reachable variant has been inside at least
/// one pass, so a `Vec` that only exists under `Mutation`'s
/// fourteenth variant is still found.
///
/// A pass that ends by learning a `FIELDS` group boundary is replayed
/// rather than counted, which is how an `#[serde(alias = "…")]` is
/// resolved: the derive concatenates a field's spellings into
/// `FIELDS` without saying where one field's group ends, so the only
/// place a boundary can be read off is the visitor's refusal of the
/// second spelling.
///
/// Panics when a pass fails for any other reason, when the walk
/// cannot bottom out ([`MAX_DEPTH`]), or when the sweep exceeds
/// [`MAX_RUNS`]. All three are the probe declining to answer rather
/// than answering short: an under-explored shape is exactly the
/// silent wrongness this module exists to expose.
///
/// Cost: one pass per reachable enum variant plus one per alias
/// spelling, each a full walk of the type graph. No I/O and no
/// parsing.
pub fn derived_shape<T: DeserializeOwned>() -> DerivedShape {
    let mut memory = Memory::default();
    let mut shape = DerivedShape::default();
    let mut targeted: BTreeSet<(String, usize)> = BTreeSet::new();
    let mut queue: VecDeque<Vec<usize>> = VecDeque::new();
    queue.push_back(Vec::new());

    let mut passes = 0usize;
    while let Some(plan) = queue.pop_front() {
        passes += 1;
        assert!(
            passes <= MAX_RUNS,
            "the variant sweep passed {MAX_RUNS} runs without draining its queue. Each \
             queued plan targets one enum variant nothing had reached yet, or one alias \
             spelling whose group boundary the last pass found, so crossing this means \
             the choice tree grew a dimension — raise the cap deliberately rather than \
             by reflex."
        );
        let replay = plan.clone();
        let mut run = Run {
            plan,
            trace: Vec::new(),
            memory: &mut memory,
            active: Vec::new(),
            named_key_sites: 0,
            pending_key: None,
            shape,
        };
        let learned_before = run.memory.learned;
        let probe = Probe {
            run: &mut run,
            member: Vec::new(),
            slot: "<root>".to_string(),
            mode: Mode::Explore,
            depth: 0,
        };
        let outcome = T::deserialize(probe);
        let learned = run.memory.learned;
        let trace = run.trace;
        shape = run.shape;
        if let Err(e) = outcome {
            assert!(
                learned > learned_before,
                "the probe could not deserialize {} from its own answers: {e}. It serves \
                 the emptiest value every request admits, so a failure here is a shape it \
                 does not know how to satisfy. Two shapes it deliberately does not: a \
                 `#[serde(flatten)]` field, which makes the derive ask for a map and then \
                 miss the members it declared, and a `#[serde(tag = \"…\")]` enum, which \
                 buffers through `Content` and refuses a unit. Both are independently \
                 banned from every loadable type by \
                 `loader::tests::test_no_loadable_type_can_swallow_an_unknown_key`, so a \
                 red here from either is that alarm a second time — anything else is a \
                 shape to teach it rather than one to narrow what it walks.",
                std::any::type_name::<T>()
            );
            // A boundary the pass had to fail to find. The same plan
            // now runs with the second spelling folded away, and gets
            // further.
            queue.push_front(replay);
            continue;
        }
        for (at, decision) in trace.iter().enumerate() {
            for option in 0..decision.options {
                let pair = (decision.container.clone(), option);
                // One plan per unreached variant, not one per place
                // it could be reached from. The walk is deterministic
                // for a given plan, so replaying this trace's prefix
                // arrives back at this same decision and takes the
                // untaken option — a second plan for the same variant
                // would only find it again, and enqueueing every
                // (site, option) pair is what turns a sweep bounded
                // by the variant count into one bounded by the size
                // of the choice tree.
                if option == decision.chosen
                    || memory.reached(&decision.container, option)
                    || !targeted.insert(pair)
                {
                    continue;
                }
                let mut next: Vec<usize> = trace[..at].iter().map(|d| d.chosen).collect();
                next.push(option);
                queue.push_back(next);
            }
        }
    }
    shape
}

/// The `Deserializer` the derived impls are handed. Every method
/// answers with the emptiest value that satisfies the request and
/// writes down what was asked for.
struct Probe<'a, 'r> {
    run: &'r mut Run<'a>,
    /// Every JSON member name this value sits under — one normally,
    /// more where `#[serde(alias = "…")]` makes more than one
    /// spelling reach it. Empty inside a sequence element, a tuple
    /// position, or a map entry: places whose name is data or
    /// position rather than schema.
    member: Vec<String>,
    /// Where this value sits when [`Self::member`] is empty, as a
    /// path through the positions that reach it —
    /// `MindEdge.control_points[0]`, `Tupled::Both.1`. Two unnamed
    /// arrays in one container are two places and have to read as
    /// two.
    slot: String,
    mode: Mode,
    depth: usize,
}

/// One step further down, or the error that says the walk does not
/// bottom out.
fn deeper(depth: usize, what: &str) -> Result<usize, Error> {
    if depth >= MAX_DEPTH {
        return Err(serde::de::Error::custom(format!(
            "the probe reached depth {MAX_DEPTH} at `{what}` without bottoming out. Once a \
             container reappears on its own path the probe empties every `Option`, \
             sequence and map below it, and takes the emptiest variant it can find at \
             every enum — so a cycle that still spins has no bottoming variant on it \
             anywhere, and no document can hold a value of it."
        )));
    }
    Ok(depth + 1)
}

/// `Container.member`, or the slot path where the value has no member
/// name, for a message that names a place in the model.
fn site_of(run: &Run<'_>, member: &[String], slot: &str) -> String {
    let container = run.active.last().map_or("<root>", String::as_str);
    match member.first() {
        Some(name) => format!("{container}.{name}"),
        None => slot.to_string(),
    }
}

/// Record a growable array under every member name that holds it,
/// merging with any earlier sighting: the same member name met twice
/// is key-bearing if it was key-bearing anywhere.
fn record_sequence(run: &mut Run<'_>, member: Vec<String>, site: String, key_bearing: bool) {
    if member.is_empty() {
        run.shape.unnamed_sequences.insert(site);
        return;
    }
    for name in member {
        let entry = run
            .shape
            .sequences
            .entry(name.clone())
            .or_insert_with(|| DerivedSequence {
                member: name,
                key_bearing: false,
                site: site.clone(),
            });
        entry.key_bearing |= key_bearing;
    }
}

/// The scalar answers, each the emptiest value its visitor accepts.
macro_rules! probe_scalar {
    ($($method:ident => $visit:ident($($value:expr),*),)*) => {
        $(
            fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
                visitor.$visit($($value),*)
            }
        )*
    };
}

impl<'de, 'a, 'r> Deserializer<'de> for Probe<'a, 'r> {
    type Error = Error;

    /// The one request the probe answers **without seeing what asked
    /// it**, so it records where it was asked.
    ///
    /// `deserialize_any` is what a self-describing value wants — a
    /// `serde_json::Value` — and it is also what an
    /// `#[serde(untagged)]` enum asks for, because serde buffers the
    /// object into a `Content` and replays it through each variant in
    /// turn. A `Content` built from `visit_unit` matches whichever
    /// variant accepts a unit and drops every other variant's shape
    /// with no error, which would be the module's one *silent* blind
    /// spot. Naming the site turns it into a loud one: an untagged
    /// enum appears here as a place the probe stopped seeing, and
    /// `mindmap::unknown_keys::tests::test_the_derived_positional_arrays_survive_an_independent_derivation`
    /// holds the list of such places to the ones the model means.
    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        let site = site_of(self.run, &self.member, &self.slot);
        self.run.shape.opaque_sites.insert(site);
        visitor.visit_unit()
    }

    probe_scalar! {
        deserialize_ignored_any => visit_unit(),
        deserialize_unit => visit_unit(),
        deserialize_bool => visit_bool(false),
        deserialize_i8 => visit_i8(0),
        deserialize_i16 => visit_i16(0),
        deserialize_i32 => visit_i32(0),
        deserialize_i64 => visit_i64(0),
        deserialize_i128 => visit_i128(0),
        deserialize_u8 => visit_u8(0),
        deserialize_u16 => visit_u16(0),
        deserialize_u32 => visit_u32(0),
        deserialize_u64 => visit_u64(0),
        deserialize_u128 => visit_u128(0),
        deserialize_f32 => visit_f32(0.0),
        deserialize_f64 => visit_f64(0.0),
        deserialize_char => visit_char('a'),
        deserialize_str => visit_str(""),
        deserialize_string => visit_str(""),
        deserialize_bytes => visit_bytes(&[]),
        deserialize_byte_buf => visit_bytes(&[]),
        deserialize_identifier => visit_str(""),
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        visitor.visit_unit()
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        let depth = deeper(self.depth, name)?;
        visitor.visit_newtype_struct(Probe {
            run: self.run,
            member: self.member,
            slot: self.slot,
            mode: self.mode,
            depth,
        })
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        if self.mode == Mode::Terminate {
            return visitor.visit_none();
        }
        let depth = deeper(self.depth, "Option")?;
        visitor.visit_some(Probe {
            run: self.run,
            member: self.member,
            slot: self.slot,
            mode: self.mode,
            depth,
        })
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        let Probe {
            run,
            member,
            slot,
            mode,
            depth,
        } = self;
        let site = site_of(run, &member, &slot);
        if mode == Mode::Terminate {
            let out = visitor.visit_seq(Elements {
                run: &mut *run,
                parent: site.clone(),
                at: 0,
                remaining: 0,
                mode,
                depth,
            })?;
            record_sequence(run, member, site, false);
            return Ok(out);
        }
        let depth = deeper(depth, &site)?;
        // The element is probed inside `visit_seq`, so the count of
        // named-key sites before and after it is exactly "did serde
        // ask for a struct anywhere at or below one element". That is
        // the question the published list turns on, and asking it
        // this way costs no second traversal.
        let before = run.named_key_sites;
        let out = visitor.visit_seq(Elements {
            run: &mut *run,
            parent: site.clone(),
            at: 0,
            remaining: 1,
            mode,
            depth,
        })?;
        let key_bearing = run.named_key_sites > before;
        record_sequence(run, member, site, key_bearing);
        Ok(out)
    }

    fn deserialize_tuple<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value, Error> {
        // A tuple is an array whose arity the format fixes, so an
        // edit cannot slide a route onto a different position. It is
        // walked for what is inside it and never recorded as a
        // growable array.
        let parent = site_of(self.run, &self.member, &self.slot);
        let depth = deeper(self.depth, "tuple")?;
        visitor.visit_seq(Elements {
            run: self.run,
            parent,
            at: 0,
            remaining: len,
            mode: self.mode,
            depth,
        })
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, Error> {
        let depth = deeper(self.depth, name)?;
        visitor.visit_seq(Elements {
            run: self.run,
            parent: name.to_string(),
            at: 0,
            remaining: len,
            mode: self.mode,
            depth,
        })
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        // Not a named-key site: a map's members are data — a palette
        // name, a node id — so none of them is ever "unrecognized".
        let entries = usize::from(self.mode == Mode::Explore);
        let parent = site_of(self.run, &self.member, &self.slot);
        let depth = deeper(self.depth, "map")?;
        visitor
            .visit_map(Entries {
                run: self.run,
                parent: parent.clone(),
                remaining: entries,
                mode: self.mode,
                depth,
            })
            .map_err(|e| flatten_hint(&parent, e))
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        let Probe { run, mode, depth, .. } = self;
        let depth = deeper(depth, name)?;
        record_container(run, name, fields);
        // A struct hands every member it does not claim to
        // `deserialize_ignored_any`, which is what the loader's
        // capture wraps — so this is the shape that makes an element
        // able to carry a key.
        run.named_key_sites += 1;
        let mode = descend_mode(run, name, mode);
        run.active.push(name.to_string());
        let out = visitor.visit_map(Fields {
            run: &mut *run,
            container: name.to_string(),
            fields,
            at: 0,
            accepted: None,
            mode,
            depth,
        });
        run.active.pop();
        fold_a_rejected_spelling(run, name, fields, out)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        let Probe { run, mode, depth, .. } = self;
        let depth = deeper(depth, name)?;
        run.shape
            .variants
            .entry(name.to_string())
            .or_insert_with(|| variants.iter().map(|v| (*v).to_string()).collect());
        let mode = descend_mode(run, name, mode);
        run.memory.facts_mut(name, variants.len());
        // A terminate-mode pass records no decision, and that is the
        // load-bearing half: the sweep's plans are indexed by decision
        // order, so a choice that varied with how deep the recursion
        // happened to be would move every later index out from under
        // the plan that produced it. Which variant it takes is free
        // to depend on what the sweep has learned, because nothing
        // downstream is indexed by it.
        let chosen = if mode == Mode::Terminate {
            terminating_variant(run.memory, name, variants.len())
        } else {
            let want = run.plan.get(run.trace.len()).copied().unwrap_or(0);
            let chosen = want.min(variants.len().saturating_sub(1));
            run.trace.push(Decision {
                container: name.to_string(),
                options: variants.len(),
                chosen,
            });
            if let Some(facts) = run.memory.facts_mut(name, variants.len()).get_mut(chosen) {
                facts.reached = true;
            }
            chosen
        };
        let Some(variant) = variants.get(chosen) else {
            return Err(serde::de::Error::custom(format!(
                "`{name}` declares no variants, so the probe has nothing to hand its \
                 deserializer"
            )));
        };
        run.active.push(name.to_string());
        let out = visitor.visit_enum(Variant {
            run: &mut *run,
            container: name,
            variant,
            at: chosen,
            mode,
            depth,
        });
        run.active.pop();
        if mode == Mode::Terminate && out.is_ok() {
            run.memory.note_bottomed(name, chosen);
        }
        out
    }

    fn is_human_readable(&self) -> bool {
        true
    }
}

/// Turn a failed struct visit into a learned `FIELDS` group boundary
/// where that is what it was, and leave every other failure alone.
///
/// The derived visitor rejects a field it already holds, and the
/// rejection lands in the one window between resolving a key and
/// asking for its value — which is exactly when [`Run::pending_key`]
/// is set. Nothing else the generated code does happens in that
/// window, so a pending key belonging to *this* container is a
/// duplicate and nothing else. `FIELDS` groups a field's spellings
/// contiguously and every index this visit got a value for was a
/// distinct field, so the rejected spelling belongs to the index
/// immediately before it.
fn fold_a_rejected_spelling<T>(
    run: &mut Run<'_>,
    name: &str,
    fields: &[&str],
    out: Result<T, Error>,
) -> Result<T, Error> {
    if out.is_ok() {
        return out;
    }
    let Some(pending) = run.pending_key.as_ref().filter(|key| key.container == name) else {
        return out;
    };
    let (at, after) = (pending.at, pending.after);
    run.pending_key = None;
    let Some(after) = after else {
        // The first spelling offered was rejected, which needs an
        // earlier one to be a second spelling *of*. Nothing in
        // `serde_derive` produces this; leave the error as it came.
        return out;
    };
    run.memory.fold(name, at, after);
    Err(serde::de::Error::custom(format!(
        "`{name}` accepts `{}` and `{}` as the same field, which the probe learns by \
         being refused and then replays without the second spelling",
        fields.get(after).copied().unwrap_or("<out of range>"),
        fields.get(at).copied().unwrap_or("<out of range>"),
    )))
}

/// Add the one cause of a `missing field` out of a map that a reader
/// would otherwise chase in the wrong place.
///
/// A struct with a `#[serde(flatten)]` field is not offered to
/// `deserialize_struct` at all — the derive asks for a *map* and
/// collects the members itself — so the probe hands it one entry
/// whose key is not any declared name, and the visitor ends up
/// missing every field it declared. The message it raises names the
/// field and says nothing about why the probe could not supply it.
fn flatten_hint(site: &str, error: Error) -> Error {
    if !error.to_string().starts_with("missing field") {
        return error;
    }
    serde::de::Error::custom(format!(
        "{error}, asked for as a map at `{site}`. A `#[serde(flatten)]` field is what \
         turns a struct's `deserialize_struct` into a `deserialize_map`, and the probe \
         has no member names to answer one with — so this is a flattened type, and \
         `loader::tests::test_no_loadable_type_can_swallow_an_unknown_key` is already \
         refusing it for swallowing captured keys."
    ))
}

/// Note the JSON member names a container accepts, merging with any
/// earlier sighting of the same container.
fn record_container(run: &mut Run<'_>, name: &str, fields: &[&str]) {
    run.shape
        .members
        .entry(name.to_string())
        .or_default()
        .extend(fields.iter().map(|field| (*field).to_string()));
}

/// [`Mode::Terminate`] once `name` is already open on the walk's own
/// path — the only way a deserializable type recurses — and never
/// back to [`Mode::Explore`] once it has been left.
fn descend_mode(run: &Run<'_>, name: &str, mode: Mode) -> Mode {
    if mode == Mode::Terminate || run.active.iter().any(|open| open == name) {
        Mode::Terminate
    } else {
        Mode::Explore
    }
}

/// `remaining` sequence or tuple positions, each probed with no
/// member name of its own but with the position it sits at.
struct Elements<'a, 'r> {
    run: &'r mut Run<'a>,
    /// The site of the array or tuple these are positions in.
    parent: String,
    /// How many have been handed out, which is the next one's index.
    at: usize,
    remaining: usize,
    mode: Mode,
    depth: usize,
}

impl<'de, 'a, 'r> SeqAccess<'de> for Elements<'a, 'r> {
    type Error = Error;

    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>, Error> {
        if self.remaining == 0 {
            return Ok(None);
        }
        self.remaining -= 1;
        let slot = format!("{}[{}]", self.parent, self.at);
        self.at += 1;
        seed.deserialize(Probe {
            run: &mut *self.run,
            member: Vec::new(),
            slot,
            mode: self.mode,
            depth: self.depth,
        })
        .map(Some)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.remaining)
    }
}

/// `remaining` entries of a map whose keys are data.
struct Entries<'a, 'r> {
    run: &'r mut Run<'a>,
    /// The site of the map these are entries of.
    parent: String,
    remaining: usize,
    mode: Mode,
    depth: usize,
}

impl<'de, 'a, 'r> MapAccess<'de> for Entries<'a, 'r> {
    type Error = Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, Error> {
        if self.remaining == 0 {
            return Ok(None);
        }
        self.remaining -= 1;
        seed.deserialize(Probe {
            run: &mut *self.run,
            member: Vec::new(),
            slot: format!("{} (a map key)", self.parent),
            mode: self.mode,
            depth: self.depth,
        })
        .map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Error> {
        seed.deserialize(Probe {
            run: &mut *self.run,
            member: Vec::new(),
            slot: format!("{} (a map value)", self.parent),
            mode: self.mode,
            depth: self.depth,
        })
    }
}

/// A struct's or struct variant's members, offered in the order the
/// derived impl declared them.
struct Fields<'a, 'r> {
    run: &'r mut Run<'a>,
    /// The container whose `FIELDS` this is, keying both the pending
    /// key and the learned group boundaries.
    container: String,
    fields: &'static [&'static str],
    /// The next `FIELDS` index to offer.
    at: usize,
    /// The last index this visit was asked the value of — the field a
    /// rejection of the next index would be a second spelling of.
    accepted: Option<usize>,
    mode: Mode,
    depth: usize,
}

impl<'de, 'a, 'r> MapAccess<'de> for Fields<'a, 'r> {
    type Error = Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, Error> {
        // A spelling the sweep already found to be a second name for
        // an earlier field. Offering it again is the duplicate this
        // whole mechanism exists to stop offering.
        while self.run.memory.is_folded(&self.container, self.at) {
            self.at += 1;
        }
        let Some(name) = self.fields.get(self.at) else {
            return Ok(None);
        };
        let key = seed.deserialize(IntoDeserializer::<'de, Error>::into_deserializer(*name))?;
        // Set only once the visitor has resolved the key, so the
        // window this marks holds nothing but the visitor's own
        // accept-or-reject.
        self.run.pending_key = Some(PendingKey {
            container: self.container.clone(),
            at: self.at,
            after: self.accepted,
        });
        Ok(Some(key))
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Error> {
        self.run.pending_key = None;
        let member = self.run.memory.spellings(&self.container, self.fields, self.at);
        let slot = site_of(self.run, &member, &self.container);
        self.accepted = Some(self.at);
        self.at += 1;
        seed.deserialize(Probe {
            run: &mut *self.run,
            member,
            slot,
            mode: self.mode,
            depth: self.depth,
        })
    }
}

/// The one variant this pass takes, and whatever payload it carries.
struct Variant<'a, 'r> {
    run: &'r mut Run<'a>,
    container: &'static str,
    variant: &'static str,
    /// Its index in the enum's `VARIANTS` list, so what it turns out
    /// to be can be written back to [`Memory`].
    at: usize,
    mode: Mode,
    depth: usize,
}

impl<'de, 'a, 'r> EnumAccess<'de> for Variant<'a, 'r> {
    type Error = Error;
    type Variant = Self;

    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Self), Error> {
        let name = self.variant;
        let chosen = seed.deserialize(IntoDeserializer::<'de, Error>::into_deserializer(name))?;
        Ok((chosen, self))
    }
}

impl<'de, 'a, 'r> VariantAccess<'de> for Variant<'a, 'r> {
    type Error = Error;

    fn unit_variant(self) -> Result<(), Error> {
        self.run.memory.note(self.container, self.at, VariantKind::Unit);
        Ok(())
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, Error> {
        // Noted before the payload is walked, not after: the pass
        // that finds `Repeat { template: Box<MutatorNode> }` is the
        // same pass that then has to terminate inside it, and it can
        // only rule the variant out if the fact is already recorded.
        self.run
            .memory
            .note(self.container, self.at, VariantKind::Payload);
        // An externally tagged newtype variant writes its payload as
        // the single member of a wrapper object, so the variant name
        // *is* the JSON member a `Vec` payload sits under — and the
        // member a positional route through it would be published as.
        let member = vec![self.variant.to_string()];
        let slot = site_of(self.run, &member, self.container);
        seed.deserialize(Probe {
            run: self.run,
            member,
            slot,
            mode: self.mode,
            depth: self.depth,
        })
    }

    fn tuple_variant<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value, Error> {
        self.run
            .memory
            .note(self.container, self.at, VariantKind::Payload);
        visitor.visit_seq(Elements {
            parent: format!("{}::{}", self.container, self.variant),
            run: self.run,
            at: 0,
            remaining: len,
            mode: self.mode,
            depth: self.depth,
        })
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.run
            .memory
            .note(self.container, self.at, VariantKind::Payload);
        let key = format!("{}::{}", self.container, self.variant);
        record_container(self.run, &key, fields);
        self.run.named_key_sites += 1;
        let out = visitor.visit_map(Fields {
            run: &mut *self.run,
            container: key.clone(),
            fields,
            at: 0,
            accepted: None,
            mode: self.mode,
            depth: self.depth,
        });
        fold_a_rejected_spelling(self.run, &key, fields, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::collections::BTreeSet as StdBTreeSet;

    /// An object with a member no field claims — the shape whose
    /// presence below an array is what makes the array's indexes
    /// worth publishing.
    #[derive(Deserialize)]
    struct Leaf {
        #[allow(dead_code)]
        x: f64,
    }

    /// The same, orderable, so it can sit in a set.
    #[derive(Deserialize, PartialEq, Eq, PartialOrd, Ord)]
    struct OrderedLeaf {
        #[allow(dead_code)]
        x: i64,
    }

    /// `Vec<Leaf>` wearing a name that says nothing about what it is.
    type Leaves = Vec<Leaf>;

    /// The names the *file* uses, none of which is the identifier.
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Renaming {
        #[allow(dead_code)]
        control_points: Vec<Leaf>,
        #[serde(rename(serialize = "pts", deserialize = "points"))]
        #[allow(dead_code)]
        spare: Vec<Leaf>,
        #[allow(dead_code)]
        plain_scalar: f64,
    }

    /// Three arrays, one of which is not spelled `Vec` and one of
    /// which is not spelled at all, plus one whose elements no key
    /// can hide in.
    #[derive(Deserialize)]
    struct Sequences {
        #[allow(dead_code)]
        aliased: Leaves,
        #[allow(dead_code)]
        ordered: StdBTreeSet<OrderedLeaf>,
        #[allow(dead_code)]
        opaque: Vec<f64>,
    }

    /// A variant whose payload is an array, a variant whose payload
    /// is a fixed-arity tuple, and a fixed-arity array field.
    #[derive(Deserialize)]
    #[allow(dead_code)]
    enum Payloads {
        Literal(Vec<Leaf>),
        Pair(f64, Leaf),
    }

    #[derive(Deserialize)]
    struct CarriesPayload {
        #[allow(dead_code)]
        payload: Payloads,
        #[allow(dead_code)]
        fixed: [Leaf; 2],
    }

    /// An array reachable only through the *second* variant of an
    /// enum, which one pass of the walk cannot see.
    #[derive(Deserialize)]
    enum Branching {
        #[allow(dead_code)]
        First(Shallow),
        #[allow(dead_code)]
        Second(Deep),
    }

    #[derive(Deserialize)]
    struct Shallow {
        #[allow(dead_code)]
        a: f64,
    }

    #[derive(Deserialize)]
    struct Deep {
        #[allow(dead_code)]
        buried: Vec<Leaf>,
    }

    /// A type that contains itself, which every real model does
    /// somewhere.
    #[derive(Deserialize)]
    struct Recursive {
        #[allow(dead_code)]
        children: Vec<Recursive>,
        #[allow(dead_code)]
        name: String,
    }

    /// An array with nowhere to be published: position 0 of a tuple
    /// variant, which no member names.
    #[derive(Deserialize)]
    enum Tupled {
        #[allow(dead_code)]
        Both(Vec<Leaf>, f64),
    }

    /// Two of them in one container, plus a third nested inside a
    /// named one — three distinct places with no member name between
    /// them.
    #[derive(Deserialize)]
    enum TwiceTupled {
        #[allow(dead_code)]
        Both(Vec<Leaf>, Vec<Leaf>),
    }

    #[derive(Deserialize)]
    struct Nesting {
        #[allow(dead_code)]
        pair: TwiceTupled,
        #[allow(dead_code)]
        rows: Vec<Vec<Leaf>>,
    }

    /// A struct serde reads as a *map* rather than a struct, because
    /// one field absorbs everything the others did not claim.
    #[derive(Deserialize)]
    struct Flattened {
        #[allow(dead_code)]
        named: f64,
        #[serde(flatten)]
        #[allow(dead_code)]
        rest: std::collections::BTreeMap<String, f64>,
    }

    /// Recursion that passes through **neither** an `Option`, a
    /// sequence nor a map: a boxed newtype variant, which
    /// [`Mode::Terminate`] has no emptier value to answer with. The
    /// only way out is to take a different variant, and `Stop` is
    /// deliberately declared *second* — the shape only bottoms out by
    /// accident if the walk always takes variant 0.
    #[derive(Deserialize)]
    enum Spinner {
        #[allow(dead_code)]
        Deeper(Box<Holder>),
        Stop,
    }

    #[derive(Deserialize)]
    struct Holder {
        #[allow(dead_code)]
        s: Spinner,
        #[allow(dead_code)]
        leaves: Vec<Leaf>,
    }

    /// An `#[serde(untagged)]` enum whose *first* variant accepts a
    /// unit, which is the shape that used to swallow the probe
    /// whole: serde buffers the object into a `Content`, the probe's
    /// `deserialize_any` answers `visit_unit`, `URoot` matches, and
    /// `Items`'s array and `ULeaf`'s members are gone with no error
    /// raised anywhere.
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Untagged {
        #[allow(dead_code)]
        Nothing(URoot),
        #[allow(dead_code)]
        Items(Vec<ULeaf>),
    }

    #[derive(Deserialize)]
    struct URoot;

    #[derive(Deserialize)]
    struct ULeaf {
        #[allow(dead_code)]
        y: f64,
    }

    #[derive(Deserialize)]
    struct CarriesUntagged {
        #[allow(dead_code)]
        maybe: Untagged,
        #[allow(dead_code)]
        visible: Vec<Leaf>,
    }

    /// `#[serde(alias)]` on both surfaces the derive names: a field,
    /// where the extra spelling joins `FIELDS`, and a variant, where
    /// it joins `VARIANTS`. `waypoints` is the live shape — the name
    /// a `MindEdge::control_points` migration would take.
    #[derive(Deserialize)]
    struct Aliased {
        #[serde(alias = "waypoints")]
        #[allow(dead_code)]
        control_points: Vec<Leaf>,
        #[allow(dead_code)]
        payload: AliasedPayload,
    }

    #[derive(Deserialize)]
    enum AliasedPayload {
        #[serde(alias = "lit")]
        #[allow(dead_code)]
        Literal(Vec<Leaf>),
        #[allow(dead_code)]
        Bare,
    }

    /// **The member names come from the generated code, so every
    /// spelling of `rename` is resolved before the probe ever sees
    /// one.**
    ///
    /// All three names asserted here are ones the Rust identifier
    /// does not supply: `rename_all = "camelCase"` produces
    /// `controlPoints` and `plainScalar`, and the `deserialize` arm of
    /// a list-form `rename` produces `points` while the `serialize`
    /// arm is correctly ignored — a captured route is recorded
    /// against the shape that was read.
    #[test]
    fn test_the_probe_reads_the_member_names_the_derive_generated() {
        let shape = derived_shape::<Renaming>();
        let members = shape
            .members_of("Renaming")
            .expect("the probe must have reached the root type");
        let expected: BTreeSet<String> = ["controlPoints", "plainScalar", "points"]
            .iter()
            .map(|name| (*name).to_string())
            .collect();
        assert_eq!(
            members, &expected,
            "the derived impl's FIELDS list is the on-disk truth, identifiers and \
             `serialize` renames included in neither"
        );
        assert_eq!(
            shape.key_bearing_sequences(),
            ["controlPoints", "points"]
                .iter()
                .map(|name| (*name).to_string())
                .collect::<BTreeSet<String>>(),
            "and the arrays are published under those same names"
        );
    }

    /// **An array is what asks for a sequence, not what is spelled
    /// `Vec`.** A type alias hides the spelling entirely and a
    /// `BTreeSet` never had it; both are JSON arrays whose element
    /// order a save can change. `Vec<f64>` is the control — an array
    /// whose elements have no members for a key to hide in, so no
    /// captured route ever crosses its indexes.
    #[test]
    fn test_the_probe_sees_an_aliased_and_a_non_vec_sequence() {
        let shape = derived_shape::<Sequences>();
        let arrays = shape.key_bearing_sequences();
        assert!(
            arrays.contains("aliased"),
            "a `Vec` behind a type alias is still a `Vec`: {arrays:?}"
        );
        assert!(
            arrays.contains("ordered"),
            "a `BTreeSet` is an array on disk and a save re-sorts it, which is if \
             anything a stronger reason to publish it: {arrays:?}"
        );
        assert!(
            !arrays.contains("opaque"),
            "`Vec<f64>` holds nothing a key can be unrecognized inside: {arrays:?}"
        );
        assert!(
            shape.sequences().contains_key("opaque"),
            "though it is still an array, and the walk has to know that to say why it \
             is absent: {:?}",
            shape.sequences()
        );
    }

    /// **An externally tagged newtype variant publishes its payload
    /// under the variant's name**, which is the member a route
    /// crosses — `{"Literal": [ … ]}`. A tuple variant and a
    /// fixed-arity array are the controls: both are arrays on disk,
    /// neither has a length an edit can change, so neither is a place
    /// a route can be *moved*.
    #[test]
    fn test_the_probe_publishes_a_newtype_variant_payload_under_the_variant_name() {
        let shape = derived_shape::<CarriesPayload>();
        let arrays = shape.key_bearing_sequences();
        assert!(
            arrays.contains("Literal"),
            "the variant name is the JSON member its array payload sits under: {arrays:?}"
        );
        assert!(
            !arrays.contains("Pair"),
            "a tuple variant's arity is fixed by the format: {arrays:?}"
        );
        assert!(!arrays.contains("fixed"), "and so is `[Leaf; 2]`'s: {arrays:?}");
        assert!(
            shape.unnamed_sequences().is_empty(),
            "nothing here is an array without a name: {:?}",
            shape.unnamed_sequences()
        );
    }

    /// **One pass through an enum sees one variant.** The sweep
    /// re-runs the walk with the untaken variant planned in, and
    /// `buried` exists only under the second one — a probe that
    /// stopped after a single pass would report the type as carrying
    /// no arrays at all, which is the silent-under-report this whole
    /// module exists to rule out.
    #[test]
    fn test_the_probe_reaches_an_array_behind_a_second_enum_variant() {
        let arrays = derived_shape::<Branching>().key_bearing_sequences();
        assert!(
            arrays.contains("buried"),
            "the variant sweep has to reach every variant, not the first one: {arrays:?}"
        );
    }

    /// A type that contains itself has to bottom out rather than
    /// spin: the walk empties every container once a name reappears
    /// on its own path, and the array is still recorded from the
    /// outer sighting.
    #[test]
    fn test_the_probe_terminates_on_a_recursive_type() {
        let arrays = derived_shape::<Recursive>().key_bearing_sequences();
        assert!(
            arrays.contains("children"),
            "the recursive array is still an array: {arrays:?}"
        );
    }

    /// **Emptying every container is not enough to bottom out.** A
    /// required enum payload is a recursion [`Mode::Terminate`] has
    /// no emptier answer for, so terminating has to mean *choosing* —
    /// a unit variant where there is one, and otherwise a variant
    /// nothing has yet found a payload behind.
    ///
    /// `MutatorNode::Repeat { template: Box<MutatorNode> }` is this
    /// shape in the live model, and it survives only because `Void`
    /// is declared before it. Declaration order is not a property the
    /// format has an opinion about — an externally tagged enum reads
    /// the same however its variants are ordered, and both
    /// derivations reorder together — so a purely cosmetic reorder
    /// must not be able to stop the sweep. `Stop` is second here for
    /// exactly that reason.
    #[test]
    fn test_the_probe_terminates_through_a_boxed_enum_newtype_variant() {
        let shape = derived_shape::<Holder>();
        assert_eq!(
            shape.variants_of("Spinner"),
            Some(["Deeper".to_string(), "Stop".to_string()].as_slice()),
            "the sweep has to have reached the enum at all"
        );
        assert!(
            shape.key_bearing_sequences().contains("leaves"),
            "and to have walked past it to the rest of the container: {:?}",
            shape.key_bearing_sequences()
        );
    }

    /// **`#[serde(alias = "…")]` is a spelling the derive writes
    /// down, on both surfaces.** A field's aliases join its `FIELDS`
    /// entry and a variant's join `VARIANTS`, sorted, grouped with the
    /// name they alias — so the probe sees every one of them and the
    /// published list has to carry all of them, because a file may
    /// write any and a captured route is recorded against whichever
    /// it wrote.
    ///
    /// The grouping is the part that needs doing rather than reading:
    /// offering `FIELDS` blindly hands the derived visitor the same
    /// field twice, which it rejects as a duplicate. That failure
    /// used to read as "the probe does not know this shape", which
    /// pointed a reader at the model instead of at the probe.
    #[test]
    fn test_the_probe_reads_every_spelling_an_alias_adds() {
        let shape = derived_shape::<Aliased>();
        let members = shape
            .members_of("Aliased")
            .expect("the probe must have reached the root type");
        assert!(
            members.contains("control_points") && members.contains("waypoints"),
            "a field's aliases are in the `FIELDS` list the derive generated: {members:?}"
        );
        assert_eq!(
            shape.variants_of("AliasedPayload"),
            Some(["Literal".to_string(), "lit".to_string(), "Bare".to_string()].as_slice()),
            "and a variant's are in `VARIANTS`, grouped with the name they alias and \
             sorted inside the group — which is the order `serde_derive` emits"
        );
        let arrays = shape.key_bearing_sequences();
        for spelling in ["control_points", "waypoints", "Literal", "lit"] {
            assert!(
                arrays.contains(spelling),
                "a route can cross the array under any spelling the derive accepts, so \
                 every one of them is published: {arrays:?}"
            );
        }
    }

    /// **The one blind spot that says nothing, made to say
    /// something.** `flatten` stops the run and a validating
    /// `deserialize_with` stops the run; `#[serde(untagged)]` does
    /// not. Serde decides an untagged variant by buffering the value
    /// into a `Content` and replaying it, so the probe's
    /// `deserialize_any` answers a unit, the first unit-accepting
    /// variant claims it, and everything behind the others is gone
    /// with nothing raised.
    ///
    /// So the probe records where it answered a `deserialize_any`.
    /// The site is not itself a verdict — a `serde_json::Value` is a
    /// legitimate one and the model has two — but it is a place the
    /// derivation stopped seeing, and a list of those is a thing a
    /// test can pin.
    #[test]
    fn test_the_probe_names_the_place_an_untagged_enum_stopped_it() {
        let shape = derived_shape::<CarriesUntagged>();
        assert!(
            !shape.key_bearing_sequences().contains("maybe"),
            "the untagged enum's array variant really is invisible — this is the blind \
             spot, not a claim that there is none: {:?}",
            shape.key_bearing_sequences()
        );
        assert!(
            shape.key_bearing_sequences().contains("visible"),
            "and the rest of the container is walked as usual, so the report has to be \
             what distinguishes the two: {:?}",
            shape.key_bearing_sequences()
        );
        assert_eq!(
            shape.opaque_sites(),
            &["CarriesUntagged.maybe".to_string()]
                .into_iter()
                .collect::<BTreeSet<String>>(),
            "the member the walk went blind under is named, once, at the place it sits"
        );
    }

    /// An array at a place with no member name is reported rather
    /// than dropped. `format/schema.md` publishes member names, and
    /// there is none here to publish — so the honest answer is to
    /// hand it to a reader, not to leave a positional route the
    /// document cannot mention.
    #[test]
    fn test_the_probe_reports_an_array_that_has_no_member_name() {
        let shape = derived_shape::<Tupled>();
        assert_eq!(
            shape.unnamed_sequences().len(),
            1,
            "position 0 of a tuple variant is an array nothing names: {:?}",
            shape.unnamed_sequences()
        );
        assert!(
            shape.key_bearing_sequences().is_empty(),
            "and it must not be published under a name it does not have: {:?}",
            shape.key_bearing_sequences()
        );
    }

    /// **An unnamed array is a *place*, and two of them are two
    /// places.** The site used to be the enclosing container's name,
    /// so a container holding two of them reported one — and the
    /// report exists precisely so a reader can go and look at each.
    ///
    /// `rows` is the shape only this derivation can see at all. The
    /// source walk classifies fields, and `Vec<Vec<Leaf>>` is one
    /// field: it sees the outer array, names it `rows`, and never
    /// asks what the element is. The inner array has no member name
    /// of its own and is a positional route just the same.
    #[test]
    fn test_two_unnamed_arrays_in_one_container_are_two_reports() {
        let shape = derived_shape::<Nesting>();
        let unnamed = shape.unnamed_sequences();
        assert_eq!(
            unnamed.len(),
            3,
            "both tuple positions and the nested array are separate places: {unnamed:?}"
        );
        for site in ["TwiceTupled::Both[0]", "TwiceTupled::Both[1]", "Nesting.rows[0]"] {
            assert!(
                unnamed.contains(site),
                "and each is named by the path that reaches it: {unnamed:?}"
            );
        }
        assert!(
            shape.key_bearing_sequences().contains("rows"),
            "the outer array keeps its name — the nesting is an extra report, not a \
             replacement: {:?}",
            shape.key_bearing_sequences()
        );
    }

    /// **A `#[serde(flatten)]` field stops the probe, and has to say
    /// why.**
    ///
    /// The derive does not hand a flattened struct to
    /// `deserialize_struct` at all — it asks for a *map* and collects
    /// the members itself — so the probe, which has member names and
    /// no data, cannot answer one and the visitor ends up missing
    /// every field it declared. `missing field \`named\`` on its own
    /// sends a reader looking at the field. It is not a defect this
    /// module has to fix: `flatten` is independently refused on every
    /// loadable type, for hiding captured keys, so this red is that
    /// one arriving from the other side and the message says so.
    #[test]
    #[should_panic(expected = "A `#[serde(flatten)]` field is what")]
    fn test_a_flattened_field_says_what_stopped_the_probe() {
        let _ = derived_shape::<Flattened>();
    }
}
