// SPDX-License-Identifier: MPL-2.0

//! Tests for [`crate::gfx_structs::element::GfxElement`] — constructor
//! variants and accessor fundamentals (§T1).
//!
//! Covers the three construction families (`new_area_*`, `new_model_*`,
//! `new_void_*`), the `channel`, `flags`, `unique_id`, and
//! `event_subscribers` accessors, and the stability guarantee on
//! `unique_id`.
//!
//! Follows the `do_*()` / `test_*()` split from §T2.2: every `do_*`
//! body is benchmarkable from `benches/test_bench.rs`.

use glam::Vec2;
use std::cell::RefCell;
use std::rc::Rc;

use crate::core::primitives::{Discriminated, Flag, Flaggable};
use crate::font::fonts;
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::{GfxElement, GfxElementType};
use crate::gfx_structs::mutator::GlyphTreeEventInstance;
use crate::gfx_structs::tree::{BranchChannel, EventSubscriber};

// ── constructor: GlyphArea variant ─────────────────────────────────

#[test]
fn test_new_area_constructs_glyph_area_variant() {
    do_new_area_constructs_glyph_area_variant();
}

/// Constructing via `new_area_non_indexed_with_id` yields a
/// `GlyphArea` variant whose `channel` and `unique_id` match the
/// values passed at construction.
pub fn do_new_area_constructs_glyph_area_variant() {
    fonts::init();
    let area = GlyphArea::new_with_str("hello", 14.0, 14.0, Vec2::new(0.0, 0.0), Vec2::new(100.0, 20.0));
    let elem = GfxElement::new_area_non_indexed_with_id(area, 7, 42);

    assert_eq!(elem.variant(), GfxElementType::GlyphArea);
    assert_eq!(elem.channel(), 7);
    assert_eq!(elem.unique_id(), 42);
}

// ── constructor: Void variant ──────────────────────────────────────

#[test]
fn test_new_void_constructs_void_variant() {
    do_new_void_constructs_void_variant();
}

/// Constructing via `new_void_with_id` yields a `Void` variant.
pub fn do_new_void_constructs_void_variant() {
    let elem = GfxElement::new_void_with_id(3, 99);

    assert_eq!(elem.variant(), GfxElementType::Void);
    assert_eq!(elem.channel(), 3);
    assert_eq!(elem.unique_id(), 99);
}

// ── flags accessor round-trip ──────────────────────────────────────

#[test]
fn test_flags_accessor_round_trips() {
    do_flags_accessor_round_trips();
}

/// Setting a flag via `set_flag` and reading it back via `flag_is_set`
/// round-trips correctly; clearing restores the original state.
pub fn do_flags_accessor_round_trips() {
    let mut elem = GfxElement::new_model_blank(0, 0);

    // Flag should not be set initially.
    assert!(!elem.flag_is_set(Flag::Focused));

    // Set the flag.
    elem.set_flag(Flag::Focused);
    assert!(elem.flag_is_set(Flag::Focused));

    // Clear the flag.
    elem.clear_flag(Flag::Focused);
    assert!(!elem.flag_is_set(Flag::Focused));
}

// ── subtree_aabb cache ─────────────────────────────────────────────

#[test]
fn test_subtree_aabb_set_and_read() {
    do_subtree_aabb_set_and_read();
}

/// Writing a subtree AABB via `set_subtree_aabb` makes it visible
/// through `subtree_aabb()`. `invalidate_subtree_aabb` clears it,
/// and the cache is excluded from `PartialEq` so two elements that
/// differ only in their cache are considered equal.
pub fn do_subtree_aabb_set_and_read() {
    let mut a = GfxElement::new_void_with_id(0, 42);
    let b = GfxElement::new_void_with_id(0, 42);
    let aabb = (Vec2::new(10.0, 20.0), Vec2::new(100.0, 200.0));

    assert!(a.subtree_aabb().is_none());
    a.set_subtree_aabb(Some(aabb));
    assert_eq!(a.subtree_aabb(), Some(aabb));
    // Cache excluded from PartialEq — element identity is unchanged.
    assert_eq!(a, b);

    a.invalidate_subtree_aabb();
    assert!(a.subtree_aabb().is_none());
}

#[test]
fn test_subtree_aabb_clone_resets_cache() {
    do_subtree_aabb_clone_resets_cache();
}

/// Cloning an element with a populated subtree-AABB cache produces
/// a clone whose cache is `None` — the cache is tree-position-
/// dependent and a clone can land at a different position. Pins the
/// invariant against accidental inclusion of the cache field in a
/// future `Clone` derive.
pub fn do_subtree_aabb_clone_resets_cache() {
    let mut elem = GfxElement::new_void(0);
    elem.set_subtree_aabb(Some((Vec2::ZERO, Vec2::new(50.0, 50.0))));
    assert!(elem.clone().subtree_aabb().is_none());
}

// ── event subscribers add and check ────────────────────────────────

#[test]
fn test_event_subscribers_add_and_check() {
    do_event_subscribers_add_and_check();
}

/// Adding an event subscriber via `subscribers_mut().push(...)` makes
/// it visible through `subscribers_as_ref()`. Verifies the list grows
/// and that a freshly-constructed element starts with no subscribers.
pub fn do_event_subscribers_add_and_check() {
    let mut elem = GfxElement::new_void_with_id(0, 0);

    // Starts empty.
    assert!(elem.subscribers_as_ref().is_empty());

    // Add a subscriber (a no-op closure wrapped in Rc<RefCell<...>>).
    let subscriber: EventSubscriber = Rc::new(RefCell::new(
        |_elem: &mut GfxElement, _evt: GlyphTreeEventInstance| {},
    ));
    elem.subscribers_mut().push(subscriber.clone());

    // List should now contain exactly one entry.
    assert_eq!(elem.subscribers_as_ref().len(), 1);

    // The subscriber we pushed should be the same Rc (pointer equality).
    assert!(Rc::ptr_eq(&elem.subscribers_as_ref()[0], &subscriber,));

    // A second subscriber is distinguishable.
    let subscriber2: EventSubscriber = Rc::new(RefCell::new(
        |_elem: &mut GfxElement, _evt: GlyphTreeEventInstance| {},
    ));
    elem.subscribers_mut().push(subscriber2.clone());
    assert_eq!(elem.subscribers_as_ref().len(), 2);
    assert!(!Rc::ptr_eq(
        &elem.subscribers_as_ref()[0],
        &elem.subscribers_as_ref()[1],
    ));
}

#[test]
fn test_event_subscribers_observe_dispatched_event() {
    do_event_subscribers_observe_dispatched_event();
}

/// Drive an event through `accept_event` and assert the
/// subscriber observed it. Pre-strengthening this test only
/// pushed onto the subscriber `Vec`; nothing exercised the
/// dispatch path (`TreeEventConsumer::accept_event` →
/// closure invocation). Now we wire a closure that records
/// what it saw, dispatch a known event, and verify the
/// recording matches.
///
/// Pin: §6.5c (`apply_to(GfxElement)` filters `Event` and
/// routes it via `accept_event`) — this test is the
/// upstream-side validation that subscribers actually fire.
pub fn do_event_subscribers_observe_dispatched_event() {
    use crate::gfx_structs::mutator::GlyphTreeEvent;
    use crate::gfx_structs::tree::TreeEventConsumer;

    let mut elem = GfxElement::new_void_with_id(0, 0);

    // Closure-shared recorder: each invocation pushes the
    // dispatched event-type tag. Rc<RefCell<Vec>> so the
    // closure can mutate observed state from inside the
    // subscriber's RefCell<dyn FnMut>.
    let observed: Rc<RefCell<Vec<GlyphTreeEvent>>> = Rc::new(RefCell::new(Vec::new()));
    let observed_for_subscriber = observed.clone();
    let subscriber: EventSubscriber = Rc::new(RefCell::new(
        move |_elem: &mut GfxElement, evt: GlyphTreeEventInstance| {
            observed_for_subscriber.borrow_mut().push(evt.event_type);
        },
    ));
    elem.subscribers_mut().push(subscriber);

    let event = GlyphTreeEventInstance::new(GlyphTreeEvent::AppEvent, 12345);
    elem.accept_event(&event);

    let recorded = observed.borrow();
    assert_eq!(recorded.len(), 1, "subscriber should fire exactly once");
    assert_eq!(
        recorded[0],
        GlyphTreeEvent::AppEvent,
        "subscriber should observe the event-type that was dispatched",
    );

    // Two subscribers, one event → both fire.
    drop(recorded);
    let observed_for_second = observed.clone();
    let second: EventSubscriber = Rc::new(RefCell::new(
        move |_e: &mut GfxElement, evt: GlyphTreeEventInstance| {
            observed_for_second.borrow_mut().push(evt.event_type);
        },
    ));
    elem.subscribers_mut().push(second);
    let event2 = GlyphTreeEventInstance::new(GlyphTreeEvent::CloseEvent, 67890);
    elem.accept_event(&event2);

    let recorded = observed.borrow();
    assert_eq!(
        recorded.len(),
        3,
        "first dispatch fired 1; second dispatch fires both subscribers"
    );
    // First sub saw AppEvent then CloseEvent; second sub saw CloseEvent only.
    // Combined recording is [AppEvent, CloseEvent, CloseEvent] in dispatch order.
    assert_eq!(recorded[0], GlyphTreeEvent::AppEvent);
    assert_eq!(recorded[1], GlyphTreeEvent::CloseEvent);
    assert_eq!(recorded[2], GlyphTreeEvent::CloseEvent);
}

#[test]
fn test_event_subscriber_can_capture_rc_refcell_state() {
    do_event_subscriber_can_capture_rc_refcell_state();
}

/// A plugin-shaped closure can capture `Rc<RefCell<...>>` app state and
/// subscribe to element events. This is the shape future script/plugin
/// consumers need: single-threaded (CODE_CONVENTIONS.md §3), no
/// `Send + Sync` requirement.
pub fn do_event_subscriber_can_capture_rc_refcell_state() {
    use crate::gfx_structs::mutator::GlyphTreeEvent;
    use crate::gfx_structs::tree::TreeEventConsumer;

    let mut elem = GfxElement::new_void_with_id(0, 0);

    // App state captured as Rc<RefCell<T>> — the plugin/script trajectory.
    let app_state: Rc<RefCell<Vec<GlyphTreeEvent>>> = Rc::new(RefCell::new(Vec::new()));
    let captured = app_state.clone();
    let subscriber: EventSubscriber = Rc::new(RefCell::new(
        move |_elem: &mut GfxElement, evt: GlyphTreeEventInstance| {
            captured.borrow_mut().push(evt.event_type);
        },
    ));
    elem.subscribers_mut().push(subscriber);

    elem.accept_event(&GlyphTreeEventInstance::new(GlyphTreeEvent::AppEvent, 1));
    elem.accept_event(&GlyphTreeEventInstance::new(
        GlyphTreeEvent::MouseEvent(crate::gfx_structs::mutator::MouseEventData::new(10.0, 20.0)),
        2,
    ));

    let recorded = app_state.borrow();
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0], GlyphTreeEvent::AppEvent);
    assert!(matches!(recorded[1], GlyphTreeEvent::MouseEvent(_)));
}

#[test]
fn test_event_subscriber_can_mutate_subscriber_list_during_delivery() {
    do_event_subscriber_can_mutate_subscriber_list_during_delivery();
}

/// A subscriber that mutates the subscriber list during delivery
/// (e.g. removes itself) must not panic or dispatch the wrong
/// subscribers. Regression for the review on #90.
pub fn do_event_subscriber_can_mutate_subscriber_list_during_delivery() {
    use crate::gfx_structs::mutator::GlyphTreeEvent;
    use crate::gfx_structs::tree::TreeEventConsumer;

    let mut elem = GfxElement::new_void_with_id(0, 0);

    let fired = Rc::new(RefCell::new(Vec::new()));

    // First subscriber removes itself and the second subscriber from
    // the list during delivery.
    let fired_a = fired.clone();
    let removable: EventSubscriber = Rc::new(RefCell::new(
        move |elem: &mut GfxElement, _evt: GlyphTreeEventInstance| {
            fired_a.borrow_mut().push('a');
            elem.subscribers_mut().clear();
        },
    ));

    let fired_b = fired.clone();
    let second: EventSubscriber = Rc::new(RefCell::new(
        move |_elem: &mut GfxElement, _evt: GlyphTreeEventInstance| {
            fired_b.borrow_mut().push('b');
        },
    ));

    elem.subscribers_mut().push(removable);
    elem.subscribers_mut().push(second);

    elem.accept_event(&GlyphTreeEventInstance::new(GlyphTreeEvent::AppEvent, 1));

    let recorded = fired.borrow();
    // Snapshotting the Rc handles before delivery means both
    // subscribers in the snapshot fire, even though the first
    // subscriber cleared the live list. The important thing is that
    // we do not panic or index the mutated list.
    assert_eq!(*recorded, vec!['a', 'b']);
    assert!(elem.subscribers_as_ref().is_empty());
}
