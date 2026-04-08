use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use enum_map::{Enum, EnumMap};
use log::{debug, error};
use rustc_hash::FxHashMap;
use strum_macros::Display;
use winit::window::WindowId;

use crate::application::common::Decree::Acknowledge;
use crate::application::common::GameResourceAspectType::{
    BaseMaxValue, CurrentValue, EffectiveMaxValue, PassiveRegenRate, ReceiveMultiplier,
    SpendMultiplier,
};

pub type DeviceId = winit::event::DeviceId;

#[derive(Clone)]
pub struct Instruction {
    /// This is the command we wish to have performed
    pub decree: Decree,
    /// An AckKey (None if N/A), and acknowledge serial number, or 0 if N/A
    pub acknowledge: (AckKey, usize),
    /// This is the Sender to use in order to acknowledge, if ack > 0
    pub ack_sender: Option<Sender<Instruction>>,
}

unsafe impl Send for Instruction {}

impl Instruction {
    pub fn new(decree: Decree) -> Self {
        Self {
            decree,
            acknowledge: (AckKey::None, 0),
            ack_sender: None,
        }
    }
    pub fn new_acknowledge(
        decree: Decree,
        ack_key: AckKey,
        check_sum: usize,
        ack_sender: Option<Sender<Instruction>>,
    ) -> Self {
        Self {
            decree,
            acknowledge: (ack_key, check_sum),
            ack_sender,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq, Hash, Enum)]
pub enum RenderSettingType {
    RedrawMode,
}

#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub enum RenderSetting {
    RedrawMode(RedrawMode),
}

impl RenderSetting {
    pub fn get_type(&self) -> RenderSettingType {
        match self {
            RenderSetting::RedrawMode(_) => RenderSettingType::RedrawMode,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq, Hash, Enum)]
pub enum UiSettingType {
    InputMode,
    KeyboardBindings,
    WindowMode,
}

#[derive(Clone)]
pub enum UiSetting {
    WindowMode(WindowMode),
    InputMode(InputMode),
    KeyboardBindings(FxHashMap<KeyPress, HostDecree>),
}

impl UiSetting {
    pub fn get_type(&self) -> UiSettingType {
        match self {
            UiSetting::InputMode(_) => UiSettingType::InputMode,
            UiSetting::KeyboardBindings(_) => UiSettingType::KeyboardBindings,
            UiSetting::WindowMode(_) => UiSettingType::WindowMode,
        }
    }
}

impl PartialEq for UiSetting {
    fn eq(&self, other: &Self) -> bool {
        self.get_type().eq(&other.get_type())
    }

    fn ne(&self, other: &Self) -> bool {
        self.get_type().ne(&other.get_type())
    }
}

impl Eq for UiSetting {}

#[derive(Copy, Clone, Eq, Hash, PartialEq)]
pub enum RedrawMode {
    OnRequest,
    FpsLimit(usize),
    NoLimit,
}

#[derive(Copy, Clone, Eq, Hash, PartialEq)]
pub enum InputMode {
    Direct,
    MappedToInstruction,
}

#[derive(Clone)]
pub enum Decree {
    Noop,
    Render(RenderDecree),
    Host(HostDecree),
    Ui(UiDecree),
    Acknowledge(AckKey, usize),
}

impl Default for Decree {
    fn default() -> Self {
        Decree::Noop
    }
}

#[derive(Copy, Clone, Eq, PartialEq)]
pub enum RenderDecree {
    Noop,
    ArenaUpdate,
    DisplayFps,
    StartRender,
    StopRender,
    ReinitAdapter,
    SetSurfaceSize(u32, u32),
    Terminate,
}

impl Default for RenderDecree {
    fn default() -> Self {
        RenderDecree::Noop
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum UiDecreeType {
    Noop,
    ExitApplication,
    UpdateSetting,
}

#[derive(Clone, Eq, PartialEq)]
pub enum UiDecree {
    Noop,
    ExitApplication,
    UpdateSetting(UiSetting),
}

impl UiDecree {
    pub fn get_type(&self) -> UiDecreeType {
        match self {
            UiDecree::Noop => UiDecreeType::Noop,
            UiDecree::ExitApplication => UiDecreeType::ExitApplication,
            UiDecree::UpdateSetting(_) => UiDecreeType::UpdateSetting,
        }
    }
}

impl Default for UiDecree {
    fn default() -> Self {
        UiDecree::Noop
    }
}

#[derive(Clone, Copy, Eq, PartialEq, Hash, Enum)]
pub enum HostSettingType {
    GameSpeedFactor,
}

#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub enum HostSetting {
    GameSpeedFactor(i8),
}

#[derive(Copy, Clone, Eq, PartialEq)]
pub enum HostDecree {
    Noop,

    Interact,
    Enter,
    Escape,
    Context,
    Menu,

    MoveUp,
    MoveRight,
    MoveLeft,
    MoveDown,

    Load(usize),
    Save,
    Pause,
    ExitInstance,

    MasterSoundAdjust(i8),
    MasterSoundSetPercentage(u8),
    MasterSoundToggle,

    SetSetting(HostSetting),

    DirectKeyboardEvent {
        window_id: WindowId,
        device_id: DeviceId,
        key_press: KeyPress,
        is_synthetic: bool,
    },
}

impl Default for HostDecree {
    fn default() -> Self {
        HostDecree::Noop
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Enum, Debug, Display)]
pub enum AckKey {
    None,
    Render,
}

impl AckKey {
    pub fn check_ack(key: AckKey, ack_map: &mut EnumMap<AckKey, usize>, ack: usize) {
        debug!("Received an acknowledgement for key {}", key);
        if ack_map[key] != 0 {
            if ack_map[key] == ack {
                debug!("removing {} lock from lock-table", key);
                ack_map[key] = 0;
            } else {
                debug!("Acknowledgement was not the most recent {} ack value", key);
            }
        }
    }
}

// winit::KeyEvent does not derive copy, so we will create our own type that does
#[derive(Copy, Clone, Eq, PartialEq)]
pub enum KeyPress {
    Placeholder,
}

impl KeyPress {
    pub(crate) fn placeholder() -> Self {
        KeyPress::Placeholder
    }
}

#[derive(Copy, Clone)]
pub enum WindowMode {
    Fullscreen,
    WindowedFullscreen,
    Windowed { x: u32, y: u32 },
}

#[derive(Copy, Clone, Enum)]
pub enum ApplicationActivity {
    Playing,
    Loading,
    Menu,
    Editor,
    Launching,
    Exiting,
}

#[derive(Copy, Clone, Enum, Eq, PartialEq)]
pub enum GameActivity {
    /**
        For the world map kind of travel
    **/
    Travel,
    /**
        The player character is at some location
    **/
    Location,
    /**
        The player character is in an encounter
    **/
    Encounter,
    /**
        There is a scripted scene playing, player has limited control
    **/
    Cutscene,
    /**
        Atypical interactive encounter
    **/
    Interaction,
    /**
        A menu
    **/
    Menu,
    /**
        Development mode
    **/
    Developer,
}

#[derive(Clone, Copy)]
pub struct StatModifier {
    pub target_resource: GameResourceType,
    pub target_modifier: GameStatModifier,
    pub modifier_timing: StatModifierPriority,
}

#[derive(Copy, Clone, Eq, PartialEq)]
pub struct GameObjectResourceInstance {
    pub resource_type: GameResourceType,
    pub aspects: EnumMap<GameResourceAspectType, GameResourceAspect>,
}

#[derive(Clone, Copy, Enum, Eq, PartialEq)]
pub enum GameResourceType {
    Health,
    Armor,
    Mana,
    Energy,
    Experience,
    Level,
    Intelligence,
    Strength,
    Agility,
    Wisdom,
    Charisma,
}

#[derive(Clone, Copy, Enum, Eq, PartialEq)]
pub enum GameResourceAspectType {
    PassiveRegenRate,
    BaseMaxValue,
    EffectiveMaxValue,
    CurrentValue,
    ReceiveMultiplier,
    SpendMultiplier,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum GameResourceAspect {
    PassiveRegenRate(isize),
    BaseMaxValue(isize),
    EffectiveMaxValue(isize),
    CurrentValue(isize),
    ReceiveMultiplier(isize),
    SpendMultiplier(isize),
}

impl GameResourceAspect {
    fn get_type(&self) -> GameResourceAspectType {
        match self {
            GameResourceAspect::PassiveRegenRate(_) => PassiveRegenRate,
            GameResourceAspect::BaseMaxValue(_) => BaseMaxValue,
            GameResourceAspect::EffectiveMaxValue(_) => EffectiveMaxValue,
            GameResourceAspect::CurrentValue(_) => CurrentValue,
            GameResourceAspect::ReceiveMultiplier(_) => ReceiveMultiplier,
            GameResourceAspect::SpendMultiplier(_) => SpendMultiplier,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum StatModifierPriority {
    Base,
    /// The higher the number, the later the modifier will be applied in the chain of modifiers
    Delay(usize),
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum GameStatModifier {
    AddPercentage(GameResourceAspect, isize),
    AddValue(GameResourceAspect, isize),
    MultiplyPercentage(GameResourceAspect, isize),
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum GameObjectFlags {
    Collision,
    Interactive,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum GameItemProperty {
    Icon,
    Slot,
    Requirements,
    Custom,
    CarryEffect,
    EquipEffect,
}

pub(crate) fn acknowledge_instruction(
    ack_key: AckKey,
    ack: usize,
    ack_sender: Option<Sender<Instruction>>,
) {
    // Acknowledge the instruction if requested
    if ack > 0 {
        let ack_result = ack_sender
            .expect("Ack requested, but no Sender provided")
            .send(Instruction::new(Acknowledge(ack_key, ack)));
        debug!("Sent acknowledgement for instruction");
        if ack_result.is_err() {
            error!("Failed to answer ack request");
        }
    }
}

#[derive(Copy, Clone)]
pub struct StopWatch {
    start: Instant,
}

impl StopWatch {
    pub fn new_start() -> StopWatch {
        StopWatch {
            start: Instant::now(),
        }
    }

    pub fn stop(&self) -> Duration {
        Instant::now().duration_since(self.start)
    }
}

#[derive(Copy, Clone)]
pub struct PollTimer {
    instant: Instant,
    duration: Duration,
}

impl PollTimer {
    #[inline]
    pub fn new(duration: Duration) -> PollTimer {
        PollTimer {
            instant: Instant::now(),
            duration,
        }
    }

    #[inline]
    pub fn immediately() -> PollTimer {
        Self::new(Duration::from_millis(0))
    }

    pub fn is_expired(&self) -> bool {
        Instant::now()
            .duration_since(self.instant)
            .ge(&self.duration)
    }
    pub fn expire_in(&mut self, duration: Duration) {
        self.instant = Instant::now();
        self.duration = duration;
    }
}
