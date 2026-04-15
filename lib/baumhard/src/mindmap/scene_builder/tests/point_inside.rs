//! `point_inside_any_node` boundary cases: strictly inside, on boundary (not inside), outside, and multi-AABB fan-out.

use super::fixtures::*;
use super::super::*;
use super::super::builder::point_inside_any_node;
use std::collections::HashMap;
use glam::Vec2;

#[test]
fn test_point_inside_any_node_strictly_inside() {
    let aabbs = vec![
        (Vec2::new(0.0, 0.0), Vec2::new(100.0, 50.0)),
    ];
    assert!(point_inside_any_node(Vec2::new(50.0, 25.0), &aabbs));
}

#[test]
fn test_point_inside_any_node_on_boundary_is_not_inside() {
    // A point exactly on the right edge should NOT be considered
    // inside — this is where connection anchor points live.
    let aabbs = vec![
        (Vec2::new(0.0, 0.0), Vec2::new(100.0, 50.0)),
    ];
    assert!(!point_inside_any_node(Vec2::new(100.0, 25.0), &aabbs));
    assert!(!point_inside_any_node(Vec2::new(0.0, 25.0), &aabbs));
    assert!(!point_inside_any_node(Vec2::new(50.0, 0.0), &aabbs));
    assert!(!point_inside_any_node(Vec2::new(50.0, 50.0), &aabbs));
}

#[test]
fn test_point_inside_any_node_outside_returns_false() {
    let aabbs = vec![
        (Vec2::new(0.0, 0.0), Vec2::new(100.0, 50.0)),
    ];
    assert!(!point_inside_any_node(Vec2::new(200.0, 25.0), &aabbs));
    assert!(!point_inside_any_node(Vec2::new(-10.0, 25.0), &aabbs));
}

#[test]
fn test_point_inside_any_node_checks_all_aabbs() {
    let aabbs = vec![
        (Vec2::new(0.0, 0.0), Vec2::new(10.0, 10.0)),
        (Vec2::new(100.0, 100.0), Vec2::new(50.0, 50.0)),
    ];
    // Inside the second box
    assert!(point_inside_any_node(Vec2::new(125.0, 125.0), &aabbs));
}

// Shared helpers for the synthetic-map scene tests below.
use crate::mindmap::model::{
    Canvas, MindEdge, MindMap, MindNode, NodeLayout, NodeStyle, Position, Size,
};
