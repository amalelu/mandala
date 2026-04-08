use std::rc::Rc;

pub trait Mutator<T: Mutable> {
    fn mutate(&self, value: &mut T);
}

pub trait AnimationMutator<T: Mutable> {
    fn update(instance: AnimationInstance<T>);
}

pub struct AnimationDef<T: Mutable> {
    pub timeline: Timeline,
    pub mutators: Vec<Box<T>>,
}

impl<T: Mutable> AnimationDef<T> {
    pub fn new(timeline: Timeline, mutators: Vec<Box<T>>) -> Rc<Self> {
        Rc::new(Self { timeline, mutators })
    }

    pub fn empty() -> Rc<Self> {
        Rc::new(Self {
            timeline: Vec::new(),
            mutators: Vec::new(),
        })
    }
}

#[derive(Clone)]
pub struct AnimationInstance<T: Mutable> {
    pub def: Rc<AnimationDef<T>>,
    pub speed: usize,
    pub play_num_times: usize,
    pub current_frame: u16,
    pub frame_elapsed_time: usize,
}

pub trait Mutable {}

pub type Timeline = Vec<TimelineEvent>;

#[derive(Clone, Eq, Hash, PartialEq)]
pub struct TimelineBuilder {
    pub events: Vec<TimelineEvent>,
}

impl TimelineBuilder {
    pub fn begin() -> Self {
        Self { events: Vec::new() }
    }
    fn build(self) -> Timeline {
        self.events
    }
    pub fn terminate(mut self) -> Timeline {
        self.events.push(TimelineEvent::Terminate);
        self.build()
    }
    pub fn goto(mut self, label: usize) -> Timeline {
        self.events.push(TimelineEvent::Goto(label));
        self.build()
    }
    pub fn wait_millis(mut self, millis: usize) -> Self {
        self.events.push(TimelineEvent::WaitMillis(millis));
        self
    }
    pub fn mutator(mut self, mutator: u16) -> Self {
        self.events.push(TimelineEvent::Mutator(mutator));
        self
    }
    pub fn interpolation(mut self, mutator: u16, num_frames: u16, duration: usize) -> Self {
        self.events.push(TimelineEvent::Interpolation {
            mutator,
            num_frames,
            duration,
        });
        self
    }
}

#[derive(Copy, Clone, Eq, Hash, PartialEq)]
pub enum TimelineEvent {
    Terminate,
    Goto(usize),
    WaitMillis(usize),
    Mutator(u16),
    Interpolation {
        mutator: u16,
        num_frames: u16,
        duration: usize,
    },
}
