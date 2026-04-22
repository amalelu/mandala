//! Animation traits and timeline primitives.
//!
//! Defines the vocabulary used to sequence `Mutable` changes over
//! time — the `Mutator` / `AnimationMutator` traits, the `Timeline`
//! / `TimelineEvent` event stream, and the `TimelineBuilder` fluent
//! constructor. The scene-level executor lives outside this module;
//! here we only describe the data shapes.

use std::rc::Rc;

/// Something that mutates a [`Mutable`] value in-place. The simplest
/// knob the animation system turns.
pub trait Mutator<T: Mutable> {
    /// Apply this mutator's change to `value` in place. O(impl).
    fn mutate(&self, value: &mut T);
}

/// Tick-driven mutator: advances an [`AnimationInstance`] forward by
/// whatever notion of "update" the implementation implements. Called
/// once per frame by the animation scheduler.
pub trait AnimationMutator<T: Mutable> {
    /// Advance `instance` by one scheduler tick. O(impl).
    fn update(instance: AnimationInstance<T>);
}

/// Immutable blueprint for an animation: a [`Timeline`] of events
/// that reference [`mutators`](AnimationDef::mutators) by u16 index.
/// Shared via `Rc` so many [`AnimationInstance`]s can replay the same
/// definition at different speeds / phases without cloning the event
/// stream.
pub struct AnimationDef<T: Mutable> {
    pub timeline: Timeline,
    pub mutators: Vec<Box<T>>,
}

impl<T: Mutable> AnimationDef<T> {
    /// Wrap the given timeline + mutator table in an `Rc`. O(1) after
    /// the caller's existing allocations.
    pub fn new(timeline: Timeline, mutators: Vec<Box<T>>) -> Rc<Self> {
        Rc::new(Self { timeline, mutators })
    }

    /// An empty, no-op animation def. Useful as a placeholder where
    /// an `Rc<AnimationDef>` is required but no animation should run.
    pub fn empty() -> Rc<Self> {
        Rc::new(Self {
            timeline: Vec::new(),
            mutators: Vec::new(),
        })
    }
}

/// A running animation — a reference to its shared [`AnimationDef`]
/// plus per-instance playback state (speed, loop count, current
/// frame, accumulated time on the frame).
#[derive(Clone)]
pub struct AnimationInstance<T: Mutable> {
    pub def: Rc<AnimationDef<T>>,
    pub speed: usize,
    pub play_num_times: usize,
    pub current_frame: u16,
    pub frame_elapsed_time: usize,
}

/// Marker trait for values an animation system can operate on.
pub trait Mutable {}

/// Alias for the event sequence driving an [`AnimationDef`].
pub type Timeline = Vec<TimelineEvent>;

/// Fluent builder that collects [`TimelineEvent`]s in order, then
/// consumes itself to produce a finalized [`Timeline`]. The
/// terminal verbs ([`Self::terminate`], [`Self::goto`]) return the
/// built vector; every other verb returns `Self` for chaining.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct TimelineBuilder {
    pub events: Vec<TimelineEvent>,
}

impl TimelineBuilder {
    /// Start a new empty timeline builder.
    pub fn begin() -> Self {
        Self { events: Vec::new() }
    }
    fn build(self) -> Timeline {
        self.events
    }
    /// Append a [`TimelineEvent::Terminate`] and finalize the
    /// timeline.
    pub fn terminate(mut self) -> Timeline {
        self.events.push(TimelineEvent::Terminate);
        self.build()
    }
    /// Append a [`TimelineEvent::Goto`] and finalize the timeline.
    pub fn goto(mut self, label: usize) -> Timeline {
        self.events.push(TimelineEvent::Goto(label));
        self.build()
    }
    /// Append a `WaitMillis(millis)` step.
    pub fn wait_millis(mut self, millis: usize) -> Self {
        self.events.push(TimelineEvent::WaitMillis(millis));
        self
    }
    /// Append a single-mutator trigger step referencing the u16
    /// index into the owning [`AnimationDef::mutators`] table.
    pub fn mutator(mut self, mutator: u16) -> Self {
        self.events.push(TimelineEvent::Mutator(mutator));
        self
    }
    /// Append an interpolation step: run mutator `mutator` across
    /// `num_frames` frames over `duration` milliseconds.
    pub fn interpolation(mut self, mutator: u16, num_frames: u16, duration: usize) -> Self {
        self.events.push(TimelineEvent::Interpolation {
            mutator,
            num_frames,
            duration,
        });
        self
    }
}

/// One step in an animation's timeline. Terminate ends playback;
/// Goto jumps the instruction pointer; WaitMillis stalls; Mutator
/// fires a single mutator once; Interpolation fires a mutator across
/// a frame window.
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
