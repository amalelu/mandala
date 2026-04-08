use log::debug;
use serde::{Deserialize, Serialize};
use strum_macros::Display;
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::area::{DeltaGlyphArea, GlyphArea, GlyphAreaCommand};
use crate::gfx_structs::model::{DeltaGlyphModel, GlyphModel, GlyphModelCommand};
use crate::gfx_structs::tree::{BranchChannel, TreeEventConsumer, TreeNode};
use crate::gfx_structs::mutator::Mutation::{AreaCommand, AreaDelta, Event, ModelCommand, ModelDelta};
use crate::gfx_structs::predicate::Predicate;
use crate::core::primitives::Applicable;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Instruction {
   /// Recursively apply the child mutator nodes of this instruction on the target tree
   RepeatWhile(Predicate),
   /// Indicates that the nodes matching the predicate should be rotated
   /// The pivot is the [GfxElement] that matches this instruction in the mutator tree
   RotateWhile(f32, Predicate),
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum MutatorType {
   Single,
   Macro,
   Void,
   Instruction,
}

/// A special type of "mutation" may trigger mutations through callback functions
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GlyphTreeEventInstance {
   pub event_type: GlyphTreeEvent,
   // This will just be millis since the application was launched in order to handle sequences
   // todo but we should handle rollover too, it will not be difficult and will only be relevant after 50 days
   pub event_time_millis: usize,
}

impl GlyphTreeEventInstance {
   pub fn new(event_type: GlyphTreeEvent, event_time_millis: usize) -> Self {
      GlyphTreeEventInstance {
         event_type,
         event_time_millis,
      }
   }
}

#[derive(Clone, Debug, Serialize, Deserialize, Display, Eq, PartialEq)]
pub enum GlyphTreeEvent {
   /// Keyboard input events
   KeyboardEvent,
   /// Mouse input events
   MouseEvent,
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

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MutationType {
   AreaDelta,
   AreaCommand,
   ModelDelta,
   ModelCommand,
   Event,
   None,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Mutation {
   AreaDelta(Box<DeltaGlyphArea>),
   AreaCommand(Box<GlyphAreaCommand>),
   ModelDelta(Box<DeltaGlyphModel>),
   ModelCommand(Box<GlyphModelCommand>),
   Event(GlyphTreeEventInstance),
   None,
}

impl AsRef<Mutation> for Mutation {
   fn as_ref(&self) -> &Mutation {
      self
   }
}

impl Mutation {
   pub fn area_delta(area_delta: DeltaGlyphArea) -> Self {
      AreaDelta(Box::new(area_delta))
   }

   pub fn area_command(area_command: GlyphAreaCommand) -> Self {
      AreaCommand(Box::new(area_command))
   }

   pub fn model_delta(model_delta: DeltaGlyphModel) -> Self {
      ModelDelta(Box::new(model_delta))
   }

   pub fn model_command(model_command: GlyphModelCommand) -> Self {
      ModelCommand(Box::new(model_command))
   }

   pub fn none() -> Self {
      Mutation::None
   }

   pub fn is_some(&self) -> bool {
      !self.is_none()
   }

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

   pub fn is_none(&self) -> bool {
      match self {
         Mutation::None => true,
         _ => false,
      }
   }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum GfxMutator {
   Single {
      mutation: Mutation,
      channel: usize,
   },
   /// A void node is an invisible node. It is a way to mark certain parts of a model as
   /// irrelevant to this particular mutation.
   Void {
      channel: usize,
   },
   /// An instruction node contains instructions on how to proceed with its children nodes
   Instruction {
      instruction: Instruction,
      channel: usize,
      mutation: Mutation,
   },
   /// A [Macro] is just a vec of [Mutation] that will be applied to the target
   /// No guarantee is given that they will be applied sequentially
   Macro {
      channel: usize,
      mutations: Vec<Mutation>,
   },
}

impl GfxMutator {
   pub fn new(mutation: Mutation, channel: usize) -> GfxMutator {
      GfxMutator::Single { mutation, channel }
   }

   pub fn new_macro(commands: Vec<Mutation>, channel: usize) -> GfxMutator {
      GfxMutator::Macro {
         channel,
         mutations: commands,
      }
   }

   pub fn new_void(channel: usize) -> GfxMutator {
      GfxMutator::Void { channel }
   }

   pub fn new_instruction(instruction_type: Instruction) -> GfxMutator {
      GfxMutator::Instruction {
         instruction: instruction_type,
         channel: 0,
         mutation: Mutation::None,
      }
   }

   pub fn get_type(&self) -> MutatorType {
      match self {
         GfxMutator::Single { .. } => MutatorType::Single,
         GfxMutator::Void { .. } => MutatorType::Void,
         GfxMutator::Instruction { .. } => MutatorType::Instruction,
         GfxMutator::Macro { .. } => MutatorType::Macro,
      }
   }

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
