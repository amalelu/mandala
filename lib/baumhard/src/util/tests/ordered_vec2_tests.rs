// SPDX-License-Identifier: MPL-2.0

//! Hash and ordering invariants for `OrderedVec2`. Plain `==` /
//! `!=` are derived; the wrapper exists so `Vec2` can participate
//! in `HashMap` / `BTreeSet` / sort orderings — which is what these
//! tests exercise.

use crate::util::ordered_vec2::OrderedVec2;
use std::collections::{HashMap, HashSet};

#[test]
pub fn test_ordered_vec2_round_trips_through_hashmap() {
    do_ordered_vec2_round_trips_through_hashmap();
}

pub fn do_ordered_vec2_round_trips_through_hashmap() {
    let mut map: HashMap<OrderedVec2, &'static str> = HashMap::new();
    map.insert(OrderedVec2::new_f32(1.5, 2.5), "a");
    map.insert(OrderedVec2::new_f32(3.0, 4.0), "b");

    assert_eq!(map.get(&OrderedVec2::new_f32(1.5, 2.5)), Some(&"a"));
    assert_eq!(map.get(&OrderedVec2::new_f32(3.0, 4.0)), Some(&"b"));
    assert!(!map.contains_key(&OrderedVec2::new_f32(1.5, 2.6)));
}

#[test]
pub fn test_ordered_vec2_distinguishes_close_floats_in_hashset() {
    do_ordered_vec2_distinguishes_close_floats_in_hashset();
}

pub fn do_ordered_vec2_distinguishes_close_floats_in_hashset() {
    let mut set: HashSet<OrderedVec2> = HashSet::new();
    set.insert(OrderedVec2::new_f32(0.1, 0.2));
    set.insert(OrderedVec2::new_f32(0.1, 0.2));
    set.insert(OrderedVec2::new_f32(0.1 + f32::EPSILON, 0.2));
    assert_eq!(set.len(), 2);
}
