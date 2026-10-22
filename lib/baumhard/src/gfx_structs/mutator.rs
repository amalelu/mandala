// SPDX-License-Identifier: MPL-2.0

//! `GfxMutator` — the top-level mutator variant that rides one node
//! of a `MutatorTree<GfxMutator>`, plus the `Mutation` payload,
//! `Instruction` control-flow directive, and the `GlyphTreeEvent`
//! side channel that the walker threads through `apply_to`.

use crate::core::primitives::{Applicable, Discriminated};
use crate::gfx_structs::area::{DeltaGlyphArea, GlyphArea, GlyphAreaCommand};
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::model::{DeltaGlyphModel, GlyphModel, GlyphModelCommand};
use crate::gfx_structs::mutator::Mutation::{AreaCommand, AreaDelta, Event, ModelCommand, ModelDelta};
use crate::gfx_structs::predicate::Predicate;
use crate::gfx_structs::tree::{BranchChannel, TreeEventConsumer, TreeNode};
use crate::util::ordered_vec2::OrderedVec2;
use log::debug;
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumDiscriminants};

/// A control-flow directive attached to a [`GfxMutator::Instruction`]
/// node. Instructions govern *how* the tree walker processes the
/// mutator's children against the target tree, rather than *what*
/// field to change. Evaluated once per matching target node during
/// [`walk_tree_from`](crate::gfx_structs::tree_walker::walk_tree_from);
/// cost is proportional to the number of matching descendants.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Instruction {
    /// Recursively apply the child mutator nodes of this instruction
    /// on every target descendant for which `Predicate` returns true.
    /// When the predicate fails on a node the branch terminates and
    /// the walker resumes with the default terminator. O(n) in the
    /// number of descendants tested.
    ///
    /// Aligns mutator children to target children by **channel** (via
    /// `tree_walker::align_child_walks`, private). For per-index
    /// targeting that sidesteps channel semantics entirely, see
    /// [`Instruction::MapChildren`].
    RepeatWhile(Predicate),
    /// Rotate every predicate-matching descendant around the pivot
    /// element's position by `f32` degrees. **Stub**: the tree walker
    /// currently no-ops this; the slot exists so adding the
    /// implementation later doesn't break serialized trees.
    RotateWhile(f32, Predicate),
    /// Descend the target tree using per-node subtree AABBs to find
    /// the deepest node whose own AABB contains the given point,
    /// pruning branches whose subtree AABB does not. Applies the
    /// attached mutation (typically a [`Mutation::Event`] carrying
    /// [`MouseEventData`]) to that node; no-op if no node contains.
    ///
    /// Bypasses channel alignment — like [`Instruction::MapChildren`]
    /// but routing on spatial hit-test rather than sibling position.
    /// Tree-walker counterpart of
    /// [`Tree::descendant_at`](crate::gfx_structs::tree::Tree::descendant_at).
    ///
    /// Cost: O(branching × depth) for spatially disjoint subtrees,
    /// O(n) worst case with fully overlapping subtrees.
    SpatialDescend(OrderedVec2),
    /// Pair mutator children with target children **by sibling
    /// position** (zip), ignoring channels on the paired children.
    /// The opt-in alternative to [`Instruction::RepeatWhile`]'s
    /// channel-broadcast semantics, used when each child needs a
    /// distinct mutation (per-index layout). Excess children on
    /// either side are dropped with one `debug!` at termination.
    /// The attached `mutation` field is applied to the current
    /// target before the body runs, same as
    /// [`Instruction::RepeatWhile`]. Composes with the AST `Repeat`
    /// wrapper for runtime-count expansion.
    ///
    /// Cost: O(min(mutator_children, target_children)); no
    /// allocation inside the zip.
    MapChildren,
}

/// A timestamped occurrence of a [`GlyphTreeEvent`] delivered to a
/// target element's event subscribers via
/// [`Mutation::Event`]. Unlike field mutations, events do not change
/// element data directly — they invoke the registered
/// [`EventSubscriber`](crate::gfx_structs::tree::EventSubscriber)
/// callbacks, which may in turn enqueue further mutations.
///
/// Cost: one allocation for the boxed closure dispatch per subscriber;
/// no arena walk.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GlyphTreeEventInstance {
    /// The kind of event being delivered.
    pub event_type: GlyphTreeEvent,
    /// Milliseconds since application launch — used to order and
    /// deduplicate event sequences. Wraps after ~50 days; callers
    /// that sequence events across long sessions should account for
    /// rollover.
    pub event_time_millis: usize,
}

impl GlyphTreeEventInstance {
    /// Construct an event instance. O(1), no allocation.
    pub fn new(event_type: GlyphTreeEvent, event_time_millis: usize) -> Self {
        GlyphTreeEventInstance {
            event_type,
            event_time_millis,
        }
    }
}

/// Payload carried by [`GlyphTreeEvent::MouseEvent`]. Contains the
/// canvas-space coordinates of the mouse interaction so that the
/// receiving element (or its [`EventSubscriber`](crate::gfx_structs::tree::EventSubscriber))
/// knows *where* the event occurred.
///
/// Uses [`OrderedFloat`] so the struct is `Eq + Hash`, consistent
/// with other position types in baumhard (`OrderedVec2`, etc.).
///
/// Cost: 8 bytes, `Copy`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct MouseEventData {
    /// Canvas-space X coordinate.
    pub x: OrderedFloat<f32>,
    /// Canvas-space Y coordinate.
    pub y: OrderedFloat<f32>,
}

impl MouseEventData {
    /// Construct from raw `f32` coordinates. O(1), no allocation.
    pub fn new(x: f32, y: f32) -> Self {
        Self {
            x: OrderedFloat(x),
            y: OrderedFloat(y),
        }
    }
}

/// The kind of event a [`GlyphTreeEventInstance`] carries. Each
/// variant represents a category of stimulus that an element's
/// [`EventSubscriber`](crate::gfx_structs::tree::EventSubscriber)
/// may react to. Cheap to clone (`Copy`-like; inner data is at most
/// one `usize`).
#[derive(Clone, Debug, Serialize, Deserialize, Display, Eq, PartialEq)]
pub enum GlyphTreeEvent {
    /// Keyboard input events
    KeyboardEvent,
    /// Mouse input events with canvas-space coordinates.
    MouseEvent(MouseEventData),
    /// Events that are defined by the software application
    AppEvent,
    /// The recipient should start preparing to shut down now
    CloseEvent,
    /// The recipient will be terminated any time
    KillEvent,
    /// A mutation has been performed
    /// This allows EventSubscribers respond to mutations
    MutationEvent,
    /// This is used for testing mainly
    NoopEvent(usize),
}

/// A single atomic change that can be applied to a [`GfxElement`].
///
/// Mutations are the leaf payload of the mutator pipeline: a
/// [`GfxMutator::Single`] carries one, a [`GfxMutator::Macro`]
/// carries a `Vec` of them. They are dispatched through
/// [`Mutation::apply_to`], which routes to the appropriate
/// element-type method. Boxed variants keep the enum size uniform
/// (one pointer + discriminant) regardless of inner payload.
///
/// The payload-free `MutationType` tag is derived from this enum and
/// reached through
/// [`crate::core::primitives::Discriminated::variant`].
///
/// `PartialEq` is IEEE equality over the whole payload, derived. Not
/// `Eq`, and **not reflexive**: the numeric payloads are `f32`, so a
/// mutation carrying a `NaN` is not equal to itself. Use
/// [`Mutation::writes_the_same`] — not `==` — for "would these two
/// write the same thing", which is the question
/// [`flat_mutations`](crate::mindmap::custom_mutation::flat_mutations)
/// asks before collapsing an AST's nested payloads into one list.
/// `==` is still the right operator for an ordinary value
/// comparison; it is only the *agreement* predicate that needs the
/// reflexive form.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, EnumDiscriminants)]
#[strum_discriminants(name(MutationType))]
#[strum_discriminants(derive(Hash, Serialize, Deserialize))]
#[strum_discriminants(doc = "Payload-free tag for [`Mutation`], derived by strum. Lets a")]
#[strum_discriminants(doc = "caller inspect which kind of mutation is carried without")]
#[strum_discriminants(doc = "destructuring it. `Copy` — no heap cost.")]
pub enum Mutation {
    /// A field-level delta applied to a [`GlyphArea`]. The
    /// [`DeltaGlyphArea`] may add, assign, or subtract from one or
    /// more area fields depending on its
    /// [`ApplyOperation`](crate::core::primitives::ApplyOperation).
    /// Cost: O(k) in the number of fields in the delta.
    AreaDelta(Box<DeltaGlyphArea>),
    /// An imperative command applied to a [`GlyphArea`] (nudge,
    /// move-to, pop-text, font resize, etc.). Each
    /// [`GlyphAreaCommand`] variant encodes both the operation and
    /// its parameter. Cost: O(1) per command.
    AreaCommand(Box<GlyphAreaCommand>),
    /// A field-level delta applied to a [`GlyphModel`]. Semantics
    /// mirror [`Mutation::AreaDelta`] but target model fields
    /// (glyph matrix, layer, position).
    ModelDelta(Box<DeltaGlyphModel>),
    /// An imperative command applied to a [`GlyphModel`] (nudge,
    /// rotate, insert line, etc.). Mirrors
    /// [`Mutation::AreaCommand`] for model elements.
    ModelCommand(Box<GlyphModelCommand>),
    /// Delivers a [`GlyphTreeEventInstance`] to the target element's
    /// event subscribers. Does not modify element data directly;
    /// subscribers may enqueue further mutations in response.
    Event(GlyphTreeEventInstance),
    /// A no-op mutation. Applying it leaves the target unchanged.
    /// Useful as a default or placeholder in mutator trees where a
    /// node must exist for structural alignment but carries no work.
    None,
}

impl AsRef<Mutation> for Mutation {
    fn as_ref(&self) -> &Mutation {
        self
    }
}

impl Mutation {
    /// Wrap a [`DeltaGlyphArea`] into a boxed `Mutation::AreaDelta`.
    /// One heap allocation for the box.
    pub fn area_delta(area_delta: DeltaGlyphArea) -> Self {
        AreaDelta(Box::new(area_delta))
    }

    /// Wrap a [`GlyphAreaCommand`] into a boxed `Mutation::AreaCommand`.
    /// One heap allocation for the box.
    pub fn area_command(area_command: GlyphAreaCommand) -> Self {
        AreaCommand(Box::new(area_command))
    }

    /// Wrap a [`DeltaGlyphModel`] into a boxed `Mutation::ModelDelta`.
    /// One heap allocation for the box.
    pub fn model_delta(model_delta: DeltaGlyphModel) -> Self {
        ModelDelta(Box::new(model_delta))
    }

    /// Wrap a [`GlyphModelCommand`] into a boxed `Mutation::ModelCommand`.
    /// One heap allocation for the box.
    pub fn model_command(model_command: GlyphModelCommand) -> Self {
        ModelCommand(Box::new(model_command))
    }

    /// Return a `Mutation::None` — the no-op sentinel. No allocation.
    pub fn none() -> Self {
        Mutation::None
    }

    /// Whether these two mutations would write the same thing —
    /// the **reflexive** payload-agreement predicate, as opposed to
    /// `==`.
    ///
    /// Derived `PartialEq` over `f32` is not an equivalence relation:
    /// `NaN != NaN`, so a mutation carrying a `NaN` is not `==` to
    /// itself. That matters because
    /// [`flat_mutations`](crate::mindmap::custom_mutation::flat_mutations)
    /// collapses an AST only when every payload beneath the root
    /// agrees with the one it returns, and
    /// [`scope::self_and_descendants`](crate::mindmap::custom_mutation::scope::self_and_descendants)
    /// builds its AST by cloning **one** list into two `Macro` nodes
    /// — so that comparison is a payload against itself. Under `==`
    /// a `NaN` there declines the whole mutator while the identical
    /// payload under
    /// [`scope::self_only`](crate::mindmap::custom_mutation::scope::self_only)
    /// applies: two helpers differing only in scope disagreeing on
    /// whether the mutation runs at all.
    ///
    /// Two payloads agree when they are `==` **or** when they are
    /// structurally identical. The second disjunct is what restores
    /// reflexivity; the first is what keeps `0.0` agreeing with
    /// `-0.0`, which "would write the same thing" requires. The
    /// union is itself an equivalence relation: structural identity
    /// implies equal values except for `NaN` payload bits, so a
    /// chain through either disjunct stays consistent.
    ///
    /// Structural identity is taken over the derived `Debug`
    /// rendering, which is the NaN-normalizing view of these types
    /// that costs no hand-written traversal: `f32`'s `Debug` prints
    /// every `NaN` as `NaN` while distinguishing `inf`, `-0.0` and
    /// every finite value (its shortest round-tripping form is
    /// injective). None of the types reachable from a `Mutation` has
    /// a hand-written `Debug`, so the rendering is total — nothing is
    /// elided, and two mutations rendering alike really are alike.
    ///
    /// The serialized form is **not** usable for this: `serde_json`
    /// writes every non-finite float as `null`, so it would make a
    /// `NaN` payload agree with an `inf` one — and `inf` *is*
    /// reachable from a `.mindmap.json` (an out-of-range literal), so
    /// that would be a silently-dropped payload rather than merely an
    /// over-strict decline.
    ///
    /// Costs: O(1) on the common path — `==` answers, and only a
    /// genuine disagreement or a `NaN` reaches the fallback, which
    /// renders both operands into `String`s.
    pub fn writes_the_same(&self, other: &Self) -> bool {
        self == other || format!("{self:?}") == format!("{other:?}")
    }

    /// Returns `true` when this mutation carries actual work (i.e. is
    /// not `Mutation::None`). O(1).
    pub fn is_some(&self) -> bool {
        !self.is_none()
    }

    /// Apply this mutation to the given [`GfxElement`]. Events are
    /// dispatched to the element's subscribers; field mutations are
    /// routed to the matching element type (area or model). Applying
    /// to a `Void` element or a type mismatch is a silent no-op
    /// (logged at debug level). Cost: O(k) in the number of delta
    /// fields, or O(1) for commands and events.
    pub fn apply_to(&self, target: &mut GfxElement) {
        if let Event(event) = self {
            target.accept_event(event);
            return;
        }
        match target {
            GfxElement::GlyphArea { glyph_area, .. } => {
                self.apply_to_area(glyph_area);
            }
            GfxElement::GlyphModel { glyph_model, .. } => {
                self.apply_to_model(glyph_model);
            }
            GfxElement::Void { .. } => {}
        }
    }

    /// Apply this mutation directly to a [`GlyphArea`]. Panics if
    /// called with a `Mutation::Event` (events go through
    /// [`apply_to`](Mutation::apply_to) on a full element, not
    /// directly to an area). A model variant or event is silently
    /// ignored (debug-logged). Cost: O(k) for deltas, O(1) for
    /// commands.
    ///
    /// `pub(crate)` because the only correct entry point is
    /// [`Self::apply_to`] (which routes `Event` to the element's
    /// subscribers before dispatching to area / model). Direct
    /// callers would bypass `Event` handling — the `unreachable!`
    /// in the `Event` arm below pins this invariant.
    pub(crate) fn apply_to_area(&self, area: &mut GlyphArea) {
        match self {
            AreaDelta(mutation) => mutation.apply_to(area),
            AreaCommand(mutation) => mutation.apply_to(area),
            ModelDelta(_) | ModelCommand(_) => {
                debug!("Tried to apply a model mutation to an area, ignoring.")
            }
            Mutation::None => {}
            // `apply_to` filters `Event` before reaching here.
            // A direct caller hitting this arm is a regression in
            // the dispatch invariant, not a runtime user error.
            Event(_) => {
                unreachable!("Event reached apply_to_area; apply_to() must filter Event before dispatch")
            }
        }
    }

    /// Apply this mutation directly to a [`GlyphModel`]. An area
    /// variant is silently ignored (debug-logged). Cost: O(k) for
    /// deltas, O(1) for commands.
    ///
    /// See [`Self::apply_to_area`] for the `pub(crate)` rationale.
    pub(crate) fn apply_to_model(&self, model: &mut GlyphModel) {
        match self {
            ModelDelta(mutation) => mutation.apply_to(model),
            ModelCommand(mutation) => mutation.apply_to(model),
            AreaDelta(_) | AreaCommand(_) => {
                debug!("Tried to apply an area mutation to a model, ignoring.");
            }
            Mutation::None => {}
            Event(_) => {
                unreachable!("Event reached apply_to_model; apply_to() must filter Event before dispatch")
            }
        }
    }

    /// Returns `true` when this is the `Mutation::None` no-op. O(1).
    pub fn is_none(&self) -> bool {
        matches!(self, Mutation::None)
    }
}

/// A node in a [`MutatorTree`](crate::gfx_structs::tree::MutatorTree).
///
/// Each variant pairs a channel index (used by the tree walker to
/// align mutator nodes with target nodes) with a payload that
/// determines what happens when the walker reaches the corresponding
/// target element. The tree walker dispatches through
/// [`Applicable<GfxElement>`](crate::core::primitives::Applicable)
/// which routes to [`Mutation::apply_to`] for `Single` and
/// `Instruction` variants, and iterates the `Vec<Mutation>` for
/// `Macro`.
///
/// Cost of applying: O(1) for `Single`/`Void`, O(k) for `Macro`
/// where k is the number of inner mutations.
///
/// The payload-free `MutatorType` tag is derived from this enum and
/// reached through
/// [`crate::core::primitives::Discriminated::variant`].
#[derive(Clone, Debug, Serialize, Deserialize, EnumDiscriminants)]
#[strum_discriminants(name(MutatorType))]
#[strum_discriminants(derive(Hash, Serialize, Deserialize))]
#[strum_discriminants(doc = "Payload-free tag for [`GfxMutator`], derived by strum. Lets the")]
#[strum_discriminants(doc = "walker match a mutator kind without destructuring the full")]
#[strum_discriminants(doc = "enum. No heap cost — `Copy`, and comparison is a single-byte")]
#[strum_discriminants(doc = "test.")]
pub enum GfxMutator {
    /// A single mutation targeting the element at the matching channel.
    Single {
        /// The mutation payload to apply.
        mutation: Mutation,
        /// Channel index for walker alignment.
        channel: usize,
    },
    /// A placeholder node that occupies a position in the mutator
    /// tree without carrying any mutation. Used to preserve channel
    /// alignment when sibling mutators must skip certain target
    /// positions.
    Void {
        /// Channel index for walker alignment.
        channel: usize,
    },
    /// A control-flow node: the [`Instruction`] governs how the
    /// walker processes this node's children against the target tree
    /// (e.g. repeat-while, rotate-while). The optional `mutation`
    /// field is applied to the matched target before the instruction
    /// body runs.
    Instruction {
        /// The control-flow directive.
        instruction: Instruction,
        /// Channel index for walker alignment.
        channel: usize,
        /// An optional direct mutation applied before the instruction
        /// body. `Mutation::None` when unused.
        mutation: Mutation,
    },
    /// A batch of mutations applied to the same target element in
    /// sequence. No ordering guarantee beyond iteration order of the
    /// `Vec`. Useful for combining several field changes into one
    /// tree node.
    Macro {
        /// Channel index for walker alignment.
        channel: usize,
        /// The mutations to apply, in order.
        mutations: Vec<Mutation>,
    },
}

impl GfxMutator {
    /// Create a `Single` mutator on the given channel. One
    /// allocation (the inner `Mutation` may box its payload).
    pub fn new(mutation: Mutation, channel: usize) -> GfxMutator {
        GfxMutator::Single { mutation, channel }
    }

    /// Create a `Macro` mutator carrying multiple mutations on the
    /// given channel. The `Vec` is moved, not cloned.
    pub fn new_macro(commands: Vec<Mutation>, channel: usize) -> GfxMutator {
        GfxMutator::Macro {
            channel,
            mutations: commands,
        }
    }

    /// Create a `Void` placeholder on the given channel. No payload,
    /// no allocation.
    pub fn new_void(channel: usize) -> GfxMutator {
        GfxMutator::Void { channel }
    }

    /// Create an `Instruction` mutator with channel 0 and no direct
    /// mutation (`Mutation::None`). The instruction type governs
    /// child traversal during the tree walk.
    pub fn new_instruction(instruction_type: Instruction) -> GfxMutator {
        GfxMutator::Instruction {
            instruction: instruction_type,
            channel: 0,
            mutation: Mutation::None,
        }
    }

    /// Test whether this mutator is of the given [`MutatorType`].
    /// O(1), no allocation.
    pub fn is(&self, mutator_type: MutatorType) -> bool {
        self.variant() == mutator_type
    }
}

impl BranchChannel for GfxMutator {
    fn channel(&self) -> usize {
        match self {
            GfxMutator::Single { channel, .. } => *channel,
            GfxMutator::Void { channel, .. } => *channel,
            GfxMutator::Instruction { channel, .. } => *channel,
            GfxMutator::Macro { channel, .. } => *channel,
        }
    }
}

impl Applicable<GfxElement> for GfxMutator {
    fn apply_to(&self, target: &mut GfxElement) {
        match self {
            GfxMutator::Single { mutation, .. } | GfxMutator::Instruction { mutation, .. } => {
                mutation.apply_to(target);
            }
            GfxMutator::Macro { mutations, .. } => {
                for command in mutations {
                    command.apply_to(target);
                }
            }
            _ => {}
        }
    }
}

impl TreeNode for GfxMutator {
    fn void() -> Self {
        Self::new_void(0)
    }
}
