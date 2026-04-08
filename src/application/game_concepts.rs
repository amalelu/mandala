use crate::application::common::{AckKey, Decree, GameActivity, HostDecree, Instruction};

use crossbeam_channel::{Receiver, Sender};
use enum_map::{Enum, EnumMap};
use indextree::Arena;
use log::error;
use std::rc::Rc;
use std::sync::{Arc, RwLock};
use baumhard::util::color::Color;
use baumhard::gfx_structs::element::GfxElement;

pub struct UiStack {
    pub elements: Vec<Box<usize>>,
}

impl UiStack {
    pub fn new() -> Self {
        UiStack { elements: vec![] }
    }
}

/// Root container for a running instance of the game host
pub struct LocalGameHost {
    pub state: Option<LocalGameState>,
    pub ui_stack: Box<UiStack>,
    pub arena: Arc<RwLock<Arena<GfxElement>>>,
    pub this_receiver: Receiver<Instruction>,
    pub this_sender: Sender<Instruction>,
    pub ui_sender: Sender<Instruction>,
    pub render_sender: Sender<Instruction>,
}

impl LocalGameHost {
    pub fn new(
        state: Option<LocalGameState>,
        this_sender: Sender<Instruction>,
        this_receiver: Receiver<Instruction>,
        ui_sender: Sender<Instruction>,
        render_sender: Sender<Instruction>,
        arena: Arc<RwLock<Arena<GfxElement>>>,
    ) -> LocalGameHost {
        LocalGameHost {
            state,
            ui_stack: Box::new(UiStack::new()),
            arena,
            this_receiver,
            this_sender,
            ui_sender,
            render_sender,
        }
    }
    pub fn process(&mut self) -> bool {
        true
    }

    fn handle_decrees(receiver: &Receiver<Instruction>, ack_map: &mut EnumMap<AckKey, usize>) {
        while !receiver.is_empty() {
            let result = receiver.recv();
            if result.is_err() {
                error!("Failed to receive instruction");
            } else {
                let instruction = result.unwrap();
                let acknowledge = instruction.acknowledge;
                let ack_sender = instruction.ack_sender;
                match instruction.decree {
                    Decree::Host(host_decree) => match host_decree {
                        HostDecree::Interact => {}
                        HostDecree::Enter => {}
                        HostDecree::Escape => {}
                        HostDecree::Context => {}
                        HostDecree::Menu => {}
                        HostDecree::MoveUp => {}
                        HostDecree::MoveRight => {}
                        HostDecree::MoveLeft => {}
                        HostDecree::MoveDown => {}
                        HostDecree::Load(_) => {}
                        HostDecree::Save => {}
                        HostDecree::Pause => {}
                        HostDecree::ExitInstance => {}
                        HostDecree::MasterSoundAdjust(_) => {}
                        HostDecree::MasterSoundSetPercentage(_) => {}
                        HostDecree::MasterSoundToggle => {}
                        HostDecree::SetSetting(_) => {}
                        HostDecree::DirectKeyboardEvent { .. } => {}
                        _ => {}
                    },
                    Decree::Acknowledge(ack_key, ack) => {
                        AckKey::check_ack(ack_key, ack_map, ack);
                    }
                    _ => {}
                }
            }
        }
    }
}

/// The current state of an application host, including the state of the world
/// The state also owns the player, and the progress and achievements of the player
pub struct LocalGameState {
    pub worlds: Vec<World>,
    pub active_world: usize,
    pub owned_objects: Vec<GameObject>,
}

impl LocalGameState {
    pub fn new() -> LocalGameState {
        LocalGameState {
            worlds: vec![],
            active_world: 0,
            owned_objects: vec![],
        }
    }
}

/// A world is composed of numerous Scenes, Objects,
pub struct World {
    pub name: String,
    pub scenes: Vec<Scene>,
    pub active_scene: usize,
    pub owned_objects: Vec<GameObject>,
}

impl World {
    pub fn new(name: String) -> Self {
        World {
            name,
            scenes: vec![],
            active_scene: 0,
            owned_objects: vec![],
        }
    }
}

/// A Scene, such as a town or a map view, or an encounter
/// The first thing that gets rendered, according to the scene state values that are set
pub struct Scene {
    pub name: String,
    pub activity: GameActivity,
    pub owned_objects: Vec<GameObject>,
    pub canvas_color: Color,
}

impl Scene {
    pub fn new(name: String, activity: GameActivity, canvas_color: Color) -> Self {
        Scene {
            name,
            activity,
            owned_objects: vec![],
            canvas_color,
        }
    }
}

#[derive(Clone)]
pub enum GameObject {
    Item {
        id: usize,
        properties: EnumMap<ObjectPropertyType, ObjectProperty>,
    },
    Abstract {
        id: usize,
        controller: Option<Rc<dyn ObjectController>>,
        properties: EnumMap<ObjectPropertyType, ObjectProperty>,
    },
    /// A character object
    Character {
        id: usize,
        controller: Option<Rc<dyn ObjectController>>,
        properties: EnumMap<ObjectPropertyType, ObjectProperty>,
    },
    /// A player character object. This variant is like Character, but may be additionally extended to suit the needs
    /// of a player object.
    Player {
        id: usize,
        controller: Option<Rc<dyn ObjectController>>,
        properties: EnumMap<ObjectPropertyType, ObjectProperty>,
    },
}

#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub enum ObjectProperty {
    Health(usize),
}

impl ObjectProperty {
    pub fn get_type(&self) -> ObjectPropertyType {
        match self {
            ObjectProperty::Health(_) => ObjectPropertyType::Health,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq, Hash, Enum)]
pub enum ObjectPropertyType {
    Health,
}

#[derive(Clone)]
pub enum Model {
    /// This model is composed of a tree of nodes, and may be very complex (or not)
    Tree { tree: Arena<GfxElement> },
    /// No model
    Void,
}

impl Model {}

pub trait ObjectController {
    fn tick(&mut self);
}

#[derive(Clone, Copy)]
pub enum GameEvent {}
