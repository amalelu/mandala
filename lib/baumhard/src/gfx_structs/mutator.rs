use log::debug;
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};
use strum_macros::Display;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::area::{DeltaGlyphArea, GlyphArea, GlyphAreaCommand};
use crate::gfx_structs::model::{DeltaGlyphModel, GlyphModel, GlyphModelCommand};
use crate::gfx_structs::tree::{BranchChannel, TreeEventConsumer, TreeNode};
use crate::gfx_structs::mutator::Mutation::{AreaCommand, AreaDelta, Event, ModelCommand, ModelDelta};
use crate::gfx_structs::predicate::Predicate;
use crate::core::primitives::Applicable;
use crate::util::ordered_vec2::OrderedVec2;

/// A control-flow directive attached to a [`GfxMutator::Instruction`]
/// node. Instructions govern *how* the tree walker processes the
/// mutator's children against the target tree, rather than *what*
/// field to change. Evaluated once per matching target node during
/// [`walk_tree_from`](crate::gfx_structs::tree_walker::walk_tree_from);
/// cost is proportional to the number of matching descendants.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Instruction {
   /// Recursively apply the child mutator nodes of this instruction
   /// on every target descendant for which `Predicate` returns true.
   /// When the predicate fails on a node the branch terminates and
   /// the walker resumes with the default terminator. O(n) in the
   /// number of descendants tested.
   RepeatWhile(Predicate),
   /// Rotate every descendant that satisfies the predicate around the
   /// pivot element's position by the given degrees. The `f32` is the
   /// rotation angle in degrees; the [`Predicate`] selects which
   /// descendants participate. Currently a stub in the tree walker
   /// (no-op); the variant exists so the mutator language can be
   /// extended without breaking serialised trees.
   RotateWhile(f32, Predicate),
   /// Descend the target tree using per-node subtree AABBs to find
   /// the deepest node whose own AABB contains the given point.
   /// Prunes branches whose subtree AABB does not contain the point.
   /// When the target node is found, the instruction's attached
   /// mutation (typically a [`Mutation::Event`] carrying a
   /// [`MouseEventData`]) is applied to it. If no node contains the
   /// point, the instruction is a no-op.
   ///
   /// This is the tree-walker counterpart of
   /// [`Tree::descendant_at`](crate::gfx_structs::tree::Tree::descendant_at):
   /// where `descendant_at` returns a `NodeId`, `SpatialDescend`
   /// delivers a mutation to the hit node through the mutator
   /// pipeline.
   ///
   /// Costs: O(branching_factor × depth) when subtrees are spatially
   /// disjoint; O(n) worst case with fully overlapping subtrees.
   SpatialDescend(OrderedVec2),
}

/// Discriminant returned by [`GfxMutator::get_type`] for fast
/// variant-matching without destructuring the full enum. No heap
/// cost — `Copy` and comparison is a single-byte test.
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum MutatorType {
   /// A single field-level mutation.
   Single,
   /// A batch of mutations applied to the same target element.
   Macro,
   /// A placeholder node that occupies a tree position without
   /// carrying a mutation — used to align channels.
   Void,
   /// A control-flow node whose [`Instruction`] governs child
   /// traversal.
   Instruction,
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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GlyphTreeEventInstance {
   /// The kind of event being delivered.
   pub event_type: GlyphTreeEvent,
   // This will just be millis since the application was launched in order to handle sequences
   // todo but we should handle rollover too, it will not be difficult and will only be relevant after 50 days
   /// Milliseconds since application launch — used to order and
   /// deduplicate event sequences.
   pub event_time_millis: usize,
}

impl GlyphTreeEventInstance {
   /// Create a new event instance with the given type and timestamp.
   /// No allocation; all fields are stack-sized.
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
    /// Create a new payload from raw `f32` coordinates.
    /// O(1), no allocation.
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
   // The recipient must call the provided function with its info
   //CallbackEvent(Box<dyn Fn(GlyphNodeInfo)>), impl only if needed
   /// This is used for testing mainly
   NoopEvent(usize),
}

/// Discriminant returned by [`Mutation::get_type`] for lightweight
/// variant inspection without destructuring. `Copy` — no heap cost.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MutationType {
   /// Field-level delta targeting a [`GlyphArea`].
   AreaDelta,
   /// Imperative command targeting a [`GlyphArea`].
   AreaCommand,
   /// Field-level delta targeting a [`GlyphModel`].
   ModelDelta,
   /// Imperative command targeting a [`GlyphModel`].
   ModelCommand,
   /// An event delivered to subscribers.
   Event,
   /// The no-op sentinel.
   None,
}

/// A single atomic change that can be applied to a [`GfxElement`].
///
/// Mutations are the leaf payload of the mutator pipeline: a
/// [`GfxMutator::Single`] carries one, a [`GfxMutator::Macro`]
/// carries a `Vec` of them. They are dispatched through
/// [`Mutation::apply_to`], which routes to the appropriate
/// element-type method. Boxed variants keep the enum size uniform
/// (one pointer + discriminant) regardless of inner payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
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
   /// mirror [`AreaDelta`](Mutation::AreaDelta) but target model
   /// fields (glyph matrix, layer, position).
   ModelDelta(Box<DeltaGlyphModel>),
   /// An imperative command applied to a [`GlyphModel`] (nudge,
   /// rotate, insert line, etc.). Mirrors
   /// [`AreaCommand`](Mutation::AreaCommand) for model elements.
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
      match self {
         // If this is an event, skip everything else and apply it
         Event(event) => {
            target.accept_event(event);
            return;
         }
         _ => {}
      }
      // Otherwise apply normally
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

   /// Return the [`MutationType`] discriminant for this mutation.
   /// O(1), no allocation.
   pub fn get_type(&self) -> MutationType {
      match self {
         AreaDelta(_) => MutationType::AreaDelta,
         AreaCommand(_) => MutationType::AreaCommand,
         ModelDelta(_) => MutationType::ModelDelta,
         ModelCommand(_) => MutationType::ModelCommand,
         Event(_) => MutationType::Event,
         Mutation::None => MutationType::None,
      }
   }

   /// Apply this mutation directly to a [`GlyphArea`]. Panics if
   /// called with a `Mutation::Event` (events must go through
   /// [`apply_to`](Mutation::apply_to) on a full element). A model
   /// variant is silently ignored (debug-logged). Cost: O(k) for
   /// deltas, O(1) for commands.
   pub fn apply_to_area(&self, area: &mut GlyphArea) {
      match self {
         AreaDelta(mutation) => mutation.apply_to(area),
         AreaCommand(mutation) => mutation.apply_to(area),
         ModelDelta(_) | ModelCommand(_) => {
            debug!("Tried to apply a model mutation to an area, ignoring.")
         }
         Mutation::None => {}
         Event(_) => {
            panic!("Events should not be applied directly to a GlyphArea!")
         }
      }
   }

   /// Apply this mutation directly to a [`GlyphModel`]. Panics if
   /// called with a `Mutation::Event`. An area variant is silently
   /// ignored (debug-logged). Cost: O(k) for deltas, O(1) for
   /// commands.
   pub fn apply_to_model(&self, model: &mut GlyphModel) {
      match self {
         ModelDelta(mutation) => mutation.apply_to(model),
         ModelCommand(mutation) => mutation.apply_to(model),
         AreaDelta(_) | AreaCommand(_) => {
            debug!("Tried to apply an area mutation to a model, ignoring.");
         }
         Mutation::None => {}
         Event(_) => {
            panic!("Events should not be applied directly to a GlyphModel!")
         }
      }
   }

   /// Returns `true` when this is the `Mutation::None` no-op. O(1).
   pub fn is_none(&self) -> bool {
      match self {
         Mutation::None => true,
         _ => false,
      }
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
#[derive(Clone, Debug, Serialize, Deserialize)]
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

   /// Return the [`MutatorType`] discriminant without destructuring.
   /// O(1), no allocation.
   pub fn get_type(&self) -> MutatorType {
      match self {
         GfxMutator::Single { .. } => MutatorType::Single,
         GfxMutator::Void { .. } => MutatorType::Void,
         GfxMutator::Instruction { .. } => MutatorType::Instruction,
         GfxMutator::Macro { .. } => MutatorType::Macro,
      }
   }

   /// Test whether this mutator is of the given [`MutatorType`].
   /// O(1), no allocation.
   pub fn is(&self, mutator_type: MutatorType) -> bool {
      self.get_type() == mutator_type
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
