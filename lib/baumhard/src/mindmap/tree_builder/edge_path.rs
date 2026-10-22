// SPDX-License-Identifier: MPL-2.0

//! One frame's connection paths, built at most once per edge.
//!
//! Three passes want the same edge's [`ConnectionPath`] in the same
//! rebuild: the sampler in [`super::connection`] when the edge misses
//! the scene cache, the label layout in [`super::connection_label`]
//! for every labeled edge, and [`super::edge_handle`] for whichever
//! edge is selected. Each used to call `build_connection_path`
//! itself, so a selected labeled edge on a cache miss resolved its
//! anchors and promoted its control points three times a frame.
//!
//! [`EdgePathCache`] is what they share instead. It is **lazy**: a
//! path exists only once some pass has asked for that edge, so an
//! edge with no label that hits the scene cache still costs nothing.
//! That is the property worth stating, because the obvious
//! alternative — resolving every edge's path up front and handing
//! out a slice — replaces three builds for a few edges with one
//! build for all of them, which is a different defect wearing this
//! one's clothes.
//!
//! **A stale memo is not spellable.** The cache borrows the `MindMap`
//! and the drag-offset map it resolves against and keys entries by
//! index into `map.edges`, so the borrow checker refuses to let
//! either change while it lives: a frame whose model or offsets moved
//! needs a new cache because the old one is still holding the old
//! ones. Nothing has to remember to invalidate it.

use std::collections::HashMap;

use glam::Vec2;

use crate::mindmap::connection::{self, ConnectionPath};
use crate::mindmap::model::{MindMap, MindNode};

/// A node's canvas-space rectangle with this frame's drag offset
/// applied: `(top_left, size)`.
///
/// Every pass that positions something against a node's live
/// geometry resolves it this way — the sampler, the label layout and
/// the handle emitter all did it inline, in the same three lines,
/// which is how they stayed in agreement by luck rather than by
/// construction.
///
/// Cost: one hash lookup, O(1).
pub fn offset_node_rect(node: &MindNode, offsets: &HashMap<String, (f32, f32)>) -> (Vec2, Vec2) {
    let (dx, dy) = offsets.get(&node.id).copied().unwrap_or((0.0, 0.0));
    (node.pos_vec2() + Vec2::new(dx, dy), node.size_vec2())
}

/// The live endpoint geometry of one edge: both nodes' offset-applied
/// rectangles, in `(from, to)` order.
///
/// `None` when either endpoint id names no node — a dangling edge,
/// which every pass skips.
///
/// Cost: two hash lookups for the nodes plus two for the offsets.
pub fn edge_endpoint_rects(
    map: &MindMap,
    offsets: &HashMap<String, (f32, f32)>,
    edge_index: usize,
) -> Option<((Vec2, Vec2), (Vec2, Vec2))> {
    let edge = map.edges.get(edge_index)?;
    let from_node = map.nodes.get(&edge.from_id)?;
    let to_node = map.nodes.get(&edge.to_id)?;
    Some((
        offset_node_rect(from_node, offsets),
        offset_node_rect(to_node, offsets),
    ))
}

/// Per-frame memo of [`ConnectionPath`]s, keyed by index into
/// `map.edges` and filled on first ask.
///
/// See the module header for why it borrows its inputs rather than
/// copying them.
pub struct EdgePathCache<'a> {
    map: &'a MindMap,
    offsets: &'a HashMap<String, (f32, f32)>,
    /// One slot per edge, `None` until some pass asks for it.
    /// Dangling edges — either endpoint id naming no node — stay
    /// `None` forever and are re-resolved on every ask, which costs
    /// two failed hash lookups and happens for edges no pass emits.
    paths: Vec<Option<ConnectionPath>>,
    filled: usize,
}

impl<'a> EdgePathCache<'a> {
    /// An empty memo sized for `map`'s edge list.
    ///
    /// Cost: one `Vec` of `map.edges.len()` empty slots. No path is
    /// built here — see [`Self::path`].
    pub fn new(map: &'a MindMap, offsets: &'a HashMap<String, (f32, f32)>) -> Self {
        Self {
            map,
            offsets,
            paths: (0..map.edges.len()).map(|_| None).collect(),
            filled: 0,
        }
    }

    /// The path of `map.edges[edge_index]`, resolved on the first ask
    /// and handed back on every later one.
    ///
    /// `None` for an out-of-range index or a dangling edge.
    ///
    /// Cost: O(1) on a hit. On a miss, the anchor resolution and
    /// control-point promotion of
    /// [`connection::build_connection_path`], plus the four hash
    /// lookups [`edge_endpoint_rects`] makes.
    pub fn path(&mut self, edge_index: usize) -> Option<&ConnectionPath> {
        if self.paths.get(edge_index)?.is_none() {
            let edge = self.map.edges.get(edge_index)?;
            let ((from_pos, from_size), (to_pos, to_size)) =
                edge_endpoint_rects(self.map, self.offsets, edge_index)?;
            let built = connection::build_connection_path(
                from_pos,
                from_size,
                &edge.anchor_from,
                to_pos,
                to_size,
                &edge.anchor_to,
                &edge.control_points,
            );
            self.paths[edge_index] = Some(built);
            self.filled += 1;
        }
        self.paths[edge_index].as_ref()
    }

    /// How many distinct edges have had a path built in this memo so
    /// far.
    ///
    /// The number the sharing exists to hold down, exposed so a test
    /// can assert it rather than a reader having to trace three call
    /// sites. O(1).
    pub fn built(&self) -> usize {
        self.filled
    }
}
