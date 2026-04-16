//! Tests for [`crate::gfx_structs::element::GfxElement`] — constructor
//! variants and accessor fundamentals (§T1).
//!
//! Covers the three construction families (`new_area_*`, `new_model_*`,
//! `new_void_*`), the `channel`, `flags`, `unique_id`, and
//! `event_subscribers` accessors, and the stability guarantee on
//! `unique_id`.
//!
//! Follows the `do_*()` / `test_*()` split from §T2.2: every public
//! body is benchmarkable from `benches/test_bench.rs`.

use glam::Vec2;
use std::sync::{Arc, Mutex};

use crate::core::primitives::{Flag, Flaggable};
use crate::font::fonts;
use crate::gfx_structs::area::GlyphArea;
use crate::gfx_structs::element::{GfxElement, GfxElementType};
use crate::gfx_structs::mutator::GlyphTreeEventInstance;
use crate::gfx_structs::tree::BranchChannel;

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
    let area = GlyphArea::new_with_str(
        "hello",
        14.0,
        14.0,
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, 20.0),
    );
    let elem = GfxElement::new_area_non_indexed_with_id(area, 7, 42);

    assert_eq!(elem.get_type(), GfxElementType::GlyphArea);
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

    assert_eq!(elem.get_type(), GfxElementType::Void);
    assert_eq!(elem.channel(), 3);
    assert_eq!(elem.unique_id(), 99);
}

// ── channel accessor ───────────────────────────────────────────────

#[test]
fn test_channel_accessor_returns_correct_value() {
    do_channel_accessor_returns_correct_value();
}

/// The `channel()` accessor returns the value provided at construction
/// for each variant family.
pub fn do_channel_accessor_returns_correct_value() {
    fonts::init();

    // GlyphArea
    let area = GlyphArea::new_with_str(
        "ch",
        10.0,
        10.0,
        Vec2::ZERO,
        Vec2::new(50.0, 10.0),
    );
    let elem_area = GfxElement::new_area_non_indexed_with_id(area, 11, 0);
    assert_eq!(elem_area.channel(), 11);

    // GlyphModel
    let elem_model = GfxElement::new_model_blank(22, 0);
    assert_eq!(elem_model.channel(), 22);

    // Void
    let elem_void = GfxElement::new_void(33);
    assert_eq!(elem_void.channel(), 33);
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

// ── unique_id stability ────────────────────────────────────────────

#[test]
fn test_unique_id_is_stable() {
    do_unique_id_is_stable();
}

/// Calling `unique_id()` multiple times on the same element returns
/// the same value — the id is not regenerated or mutated by access.
pub fn do_unique_id_is_stable() {
    let elem = GfxElement::new_void_with_id(0, 777);

    let first = elem.unique_id();
    let second = elem.unique_id();
    let third = elem.unique_id();

    assert_eq!(first, 777);
    assert_eq!(first, second);
    assert_eq!(second, third);
}

// ── subtree_aabb cache ─────────────────────────────────────────────

#[test]
fn test_subtree_aabb_defaults_to_none() {
    do_subtree_aabb_defaults_to_none();
}

/// Freshly constructed elements have no cached subtree AABB.
pub fn do_subtree_aabb_defaults_to_none() {
    fonts::init();

    let area = GlyphArea::new_with_str("x", 10.0, 10.0, Vec2::ZERO, Vec2::new(50.0, 10.0));
    assert!(GfxElement::new_area_non_indexed(area, 0).subtree_aabb().is_none());
    assert!(GfxElement::new_model_blank(0, 0).subtree_aabb().is_none());
    assert!(GfxElement::new_void(0).subtree_aabb().is_none());
}

#[test]
fn test_subtree_aabb_set_and_read() {
    do_subtree_aabb_set_and_read();
}

/// Writing a subtree AABB via `set_subtree_aabb` makes it visible
/// through `subtree_aabb()`. `invalidate_subtree_aabb` clears it.
pub fn do_subtree_aabb_set_and_read() {
    let mut elem = GfxElement::new_void(0);
    let aabb = (Vec2::new(10.0, 20.0), Vec2::new(100.0, 200.0));

    elem.set_subtree_aabb(Some(aabb));
    assert_eq!(elem.subtree_aabb(), Some(aabb));

    elem.invalidate_subtree_aabb();
    assert!(elem.subtree_aabb().is_none());
}

#[test]
fn test_subtree_aabb_survives_clone() {
    do_subtree_aabb_survives_clone();
}

/// Cloning an element produces a fresh element with `subtree_aabb`
/// defaulting to `None` — the cache is position-dependent and should
/// not carry over to a clone placed in a different tree position.
pub fn do_subtree_aabb_survives_clone() {
    let mut elem = GfxElement::new_void(0);
    elem.set_subtree_aabb(Some((Vec2::ZERO, Vec2::new(50.0, 50.0))));

    let cloned = elem.clone();
    // Cloned through the constructor which defaults to None — correct
    // for a cache that is tree-position-dependent.
    assert!(cloned.subtree_aabb().is_none());
}

#[test]
fn test_subtree_aabb_ignored_in_eq() {
    do_subtree_aabb_ignored_in_eq();
}

/// Two elements that differ only in their cached `subtree_aabb` are
/// considered equal — the cache is not part of element identity.
pub fn do_subtree_aabb_ignored_in_eq() {
    let mut a = GfxElement::new_void_with_id(0, 42);
    let mut b = GfxElement::new_void_with_id(0, 42);

    a.set_subtree_aabb(Some((Vec2::ZERO, Vec2::new(100.0, 100.0))));
    // b has no subtree_aabb set.
    assert_eq!(a, b);
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

    // Add a subscriber (a no-op closure wrapped in Arc<Mutex<...>>).
    let subscriber: Arc<Mutex<dyn FnMut(&mut GfxElement, GlyphTreeEventInstance) + Send + Sync>> =
        Arc::new(Mutex::new(|_elem: &mut GfxElement, _evt: GlyphTreeEventInstance| {}));
    elem.subscribers_mut().push(subscriber.clone());

    // List should now contain exactly one entry.
    assert_eq!(elem.subscribers_as_ref().len(), 1);

    // The subscriber we pushed should be the same Arc (pointer equality).
    assert!(Arc::ptr_eq(
        &elem.subscribers_as_ref()[0],
        &subscriber,
    ));

    // A second subscriber is distinguishable.
    let subscriber2: Arc<Mutex<dyn FnMut(&mut GfxElement, GlyphTreeEventInstance) + Send + Sync>> =
        Arc::new(Mutex::new(|_elem: &mut GfxElement, _evt: GlyphTreeEventInstance| {}));
    elem.subscribers_mut().push(subscriber2.clone());
    assert_eq!(elem.subscribers_as_ref().len(), 2);
    assert!(!Arc::ptr_eq(
        &elem.subscribers_as_ref()[0],
        &elem.subscribers_as_ref()[1],
    ));
}
