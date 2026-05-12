// SPDX-License-Identifier: MPL-2.0

//! Section-bounds invariants. The `MindNode` docstring promises
//! `maptool verify` flags out-of-bounds sections; this module
//! delivers on that promise. Checks per
//! [`baumhard::mindmap::model::MindSection`]:
//!
//! - `offset.{x,y}` finite and non-negative,
//! - `size.{width,height}` (when set) finite and strictly positive,
//! - `offset + size` (when size set) inside the parent node's
//!   `size` AABB.
//!
//! Violations are emitted with the parent node's id as location
//! and the offending section index inlined into the message, so
//! a multi-section node still pinpoints which section failed.

use baumhard::mindmap::model::MindMap;

use super::Violation;

const CATEGORY: &str = "sections";

pub fn check(map: &MindMap) -> Vec<Violation> {
    let mut out = Vec::new();

    for (_loc, node) in map.node_locations() {
        // Node-level finite check before walking sections —
        // a NaN/inf at `node.size` poisons every section's
        // AABB-containment math, so flagging here surfaces the
        // root cause before downstream "section overflow" cascades.
        check_node_size_finite(node, &mut out);
        check_section_count_cap(node, &mut out);
        check_section_channel_collisions(node, &mut out);
        for (s_idx, section) in node.sections.iter().enumerate() {
            check_offset_finite(node, s_idx, section, &mut out);
            check_offset_non_negative(node, s_idx, section, &mut out);

            // AABB containment uses the section's *effective*
            // size — `Some(sz)` honours the explicit pin,
            // `None` falls back to `node.size` (fill-parent).
            // Pre-fix the `None` arm skipped the check
            // entirely, leaving fill-parent sections at
            // non-zero offset to overflow the node visually.
            let effective_size = section.effective_size(node.size);
            check_within_node_aabb(node, s_idx, section, &effective_size, &mut out);
            if let Some(size) = section.size.as_ref() {
                check_size_finite(node, s_idx, size, &mut out);
                check_size_positive(node, s_idx, size, &mut out);
                check_size_not_astronomical(node, s_idx, size, &mut out);
            }
        }
    }

    out
}

/// Node-level finite-size check. A NaN or infinity at
/// `node.size.{width,height}` propagates into the tree's
/// `render_bounds`, the renderer's `Buffer::set_size`, and every
/// AABB / hit-test comparison in the document — without
/// panicking. Catching it here surfaces a corrupt save before the
/// renderer turns the node invisible-but-not-crashed.
fn check_node_size_finite(node: &baumhard::mindmap::model::MindNode, out: &mut Vec<Violation>) {
    if !node.size.width.is_finite() || !node.size.height.is_finite() {
        out.push(Violation::node(
            CATEGORY,
            node,
            format!(
                "node.size has non-finite component (width={}, height={})",
                node.size.width, node.size.height
            ),
        ));
    }
    if node.size.width.is_finite() && node.size.width <= 0.0 {
        out.push(Violation::node(
            CATEGORY,
            node,
            format!("node.size.width is not positive ({})", node.size.width),
        ));
    }
    if node.size.height.is_finite() && node.size.height <= 0.0 {
        out.push(Violation::node(
            CATEGORY,
            node,
            format!("node.size.height is not positive ({})", node.size.height),
        ));
    }
}

/// Defends against hostile mindmaps with absurd section counts
/// — a `"sections": [{},{},…10M…]` JSON payload would OOM at
/// load. Mirrors the `add_section`'s runtime cap so the model
/// invariant is checked at every entry point.
fn check_section_count_cap(node: &baumhard::mindmap::model::MindNode, out: &mut Vec<Violation>) {
    const MAX_SECTIONS_PER_NODE: usize = 1024;
    if node.sections.len() > MAX_SECTIONS_PER_NODE {
        out.push(Violation::node(
            CATEGORY,
            node,
            format!(
                "node.sections.len()={} exceeds cap {}",
                node.sections.len(),
                MAX_SECTIONS_PER_NODE
            ),
        ));
    }
}

/// Section channel-collision check. Two sections sharing a
/// channel under the same parent broadcast a single mutation to
/// both — occasionally the intent, more often a typo. Surfaced
/// as a violation so authors notice; the apply path doesn't
/// reject the map (the broadcast is still well-defined).
///
/// Closes the docstring promise on `MindNode.sections` that
/// `verify` flags channel collisions; pre-fix the docstring
/// promised this but no code did the check.
fn check_section_channel_collisions(node: &baumhard::mindmap::model::MindNode, out: &mut Vec<Violation>) {
    use std::collections::HashMap;
    let mut by_channel: HashMap<usize, Vec<usize>> = HashMap::new();
    for (idx, section) in node.sections.iter().enumerate() {
        // Apply the same effective-channel rule the tree builder
        // uses (`section.channel.unwrap_or(idx)`) so a None on
        // section idx 0 is logically channel 0, etc. — the
        // collision the dispatcher will actually see.
        let channel = section.channel.unwrap_or(idx);
        by_channel.entry(channel).or_default().push(idx);
    }
    for (channel, indices) in by_channel.iter() {
        if indices.len() > 1 {
            out.push(Violation::node(
                CATEGORY,
                node,
                format!(
                    "channel {} shared by sections {:?}; mutations targeting that channel \
                     broadcast to all listed sections — usually unintentional",
                    channel, indices
                ),
            ));
        }
    }
}

/// Catches astronomical-but-finite section sizes (e.g.
/// `1e30` typos) that pass the finite + positive guards but
/// would distort cosmic-text shaping and AABB math downstream.
/// Threshold: 100× the parent node's size. Authors who genuinely
/// want a huge intentionally-overflow section can ignore the
/// warning; a typo surfaces immediately.
fn check_size_not_astronomical(
    node: &baumhard::mindmap::model::MindNode,
    s_idx: usize,
    size: &baumhard::mindmap::model::Size,
    out: &mut Vec<Violation>,
) {
    if !node.size.width.is_finite() || !node.size.height.is_finite() {
        return;
    }
    let max_w = node.size.width * 100.0;
    let max_h = node.size.height * 100.0;
    if size.width > max_w {
        out.push(Violation::node(
            CATEGORY,
            node,
            format!(
                "section[{}].size.width ({}) is over 100× the node's width ({}); \
                 likely a typo (e.g. an extra zero)",
                s_idx, size.width, node.size.width
            ),
        ));
    }
    if size.height > max_h {
        out.push(Violation::node(
            CATEGORY,
            node,
            format!(
                "section[{}].size.height ({}) is over 100× the node's height ({}); \
                 likely a typo (e.g. an extra zero)",
                s_idx, size.height, node.size.height
            ),
        ));
    }
}

fn check_offset_finite(
    node: &baumhard::mindmap::model::MindNode,
    s_idx: usize,
    section: &baumhard::mindmap::model::MindSection,
    out: &mut Vec<Violation>,
) {
    if !section.offset.x.is_finite() || !section.offset.y.is_finite() {
        out.push(Violation::node(
            CATEGORY,
            node,
            format!(
                "section[{}].offset has non-finite component (x={}, y={})",
                s_idx, section.offset.x, section.offset.y
            ),
        ));
    }
}

fn check_offset_non_negative(
    node: &baumhard::mindmap::model::MindNode,
    s_idx: usize,
    section: &baumhard::mindmap::model::MindSection,
    out: &mut Vec<Violation>,
) {
    if section.offset.x.is_finite() && section.offset.x < 0.0 {
        out.push(Violation::node(
            CATEGORY,
            node,
            format!("section[{}].offset.x is negative ({})", s_idx, section.offset.x),
        ));
    }
    if section.offset.y.is_finite() && section.offset.y < 0.0 {
        out.push(Violation::node(
            CATEGORY,
            node,
            format!("section[{}].offset.y is negative ({})", s_idx, section.offset.y),
        ));
    }
}

fn check_size_finite(
    node: &baumhard::mindmap::model::MindNode,
    s_idx: usize,
    size: &baumhard::mindmap::model::Size,
    out: &mut Vec<Violation>,
) {
    if !size.width.is_finite() || !size.height.is_finite() {
        out.push(Violation::node(
            CATEGORY,
            node,
            format!(
                "section[{}].size has non-finite component (width={}, height={})",
                s_idx, size.width, size.height
            ),
        ));
    }
}

fn check_size_positive(
    node: &baumhard::mindmap::model::MindNode,
    s_idx: usize,
    size: &baumhard::mindmap::model::Size,
    out: &mut Vec<Violation>,
) {
    if size.width.is_finite() && size.width <= 0.0 {
        out.push(Violation::node(
            CATEGORY,
            node,
            format!("section[{}].size.width is not positive ({})", s_idx, size.width),
        ));
    }
    if size.height.is_finite() && size.height <= 0.0 {
        out.push(Violation::node(
            CATEGORY,
            node,
            format!("section[{}].size.height is not positive ({})", s_idx, size.height),
        ));
    }
}

fn check_within_node_aabb(
    node: &baumhard::mindmap::model::MindNode,
    s_idx: usize,
    section: &baumhard::mindmap::model::MindSection,
    size: &baumhard::mindmap::model::Size,
    out: &mut Vec<Violation>,
) {
    if !section.offset.x.is_finite()
        || !section.offset.y.is_finite()
        || !size.width.is_finite()
        || !size.height.is_finite()
    {
        return;
    }

    let right = section.offset.x + size.width;
    let bottom = section.offset.y + size.height;
    if right > node.size.width {
        out.push(Violation::node(
            CATEGORY,
            node,
            format!(
                "section[{}] extends past node right edge ({} > {})",
                s_idx, right, node.size.width
            ),
        ));
    }
    if bottom > node.size.height {
        out.push(Violation::node(
            CATEGORY,
            node,
            format!(
                "section[{}] extends past node bottom edge ({} > {})",
                s_idx, bottom, node.size.height
            ),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::test_helpers::node;
    use baumhard::mindmap::model::{MindSection, Position, Size};

    fn section(offset: Position, size: Option<Size>) -> MindSection {
        MindSection {
            text: String::new(),
            text_runs: Vec::new(),
            offset,
            size,
            channel: None,
            trigger_bindings: Vec::new(),
            frame_border: None,
        }
    }

    #[test]
    fn default_section_clean() {
        let mut map = MindMap::new_blank("t");
        let n = node("0", None);
        map.nodes.insert("0".into(), n);
        assert!(check(&map).is_empty());
    }

    #[test]
    fn explicit_within_aabb_clean() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0] = section(
            Position { x: 5.0, y: 5.0 },
            Some(Size {
                width: 50.0,
                height: 20.0,
            }),
        );
        map.nodes.insert("0".into(), n);
        assert!(check(&map).is_empty());
    }

    #[test]
    fn flush_to_node_aabb_clean() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0] = section(
            Position { x: 0.0, y: 0.0 },
            Some(Size {
                width: 100.0,
                height: 40.0,
            }),
        );
        map.nodes.insert("0".into(), n);
        assert!(check(&map).is_empty());
    }

    #[test]
    fn negative_offset_x_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0] = section(Position { x: -1.0, y: 0.0 }, None);
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v
            .iter()
            .any(|x| x.category == CATEGORY && x.message.contains("offset.x is negative")));
    }

    #[test]
    fn negative_offset_y_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0] = section(Position { x: 0.0, y: -2.0 }, None);
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v
            .iter()
            .any(|x| x.category == CATEGORY && x.message.contains("offset.y is negative")));
    }

    #[test]
    fn nan_offset_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0] = section(Position { x: f64::NAN, y: 0.0 }, None);
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v
            .iter()
            .any(|x| x.category == CATEGORY && x.message.contains("non-finite")));
    }

    #[test]
    fn zero_size_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0] = section(
            Position { x: 0.0, y: 0.0 },
            Some(Size {
                width: 0.0,
                height: 10.0,
            }),
        );
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v
            .iter()
            .any(|x| x.category == CATEGORY && x.message.contains("size.width is not positive")));
    }

    #[test]
    fn negative_size_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0] = section(
            Position { x: 0.0, y: 0.0 },
            Some(Size {
                width: 10.0,
                height: -5.0,
            }),
        );
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v
            .iter()
            .any(|x| x.category == CATEGORY && x.message.contains("size.height is not positive")));
    }

    #[test]
    fn nan_size_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0] = section(
            Position { x: 0.0, y: 0.0 },
            Some(Size {
                width: f64::NAN,
                height: 10.0,
            }),
        );
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v
            .iter()
            .any(|x| x.category == CATEGORY && x.message.contains("non-finite")));
    }

    #[test]
    fn extends_past_right_edge_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0] = section(
            Position { x: 50.0, y: 0.0 },
            Some(Size {
                width: 60.0,
                height: 10.0,
            }),
        );
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v
            .iter()
            .any(|x| x.category == CATEGORY && x.message.contains("past node right edge")));
    }

    #[test]
    fn extends_past_bottom_edge_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0] = section(
            Position { x: 0.0, y: 30.0 },
            Some(Size {
                width: 10.0,
                height: 20.0,
            }),
        );
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v
            .iter()
            .any(|x| x.category == CATEGORY && x.message.contains("past node bottom edge")));
    }

    #[test]
    fn multi_section_pinpoints_offending_index() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections = vec![
            section(Position { x: 0.0, y: 0.0 }, None),
            section(Position { x: -3.0, y: 0.0 }, None),
        ];
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        let off = v
            .iter()
            .find(|x| x.message.contains("offset.x is negative"))
            .expect("missed violation");
        assert!(
            off.message.contains("section[1]"),
            "expected section index 1 in message: {}",
            off.message
        );
    }

    /// `None`-sized sections (fill-parent) are bounds-checked
    /// against the *effective* size = `node.size`. A non-zero
    /// offset on a fill-parent section means the section
    /// stretches past the node's right / bottom edge, so
    /// verify flags it. Pre-fix the `None` arm skipped the
    /// check entirely, leaving fill-parent sections free to
    /// visually escape the parent.
    #[test]
    fn unset_size_at_zero_offset_clean() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0] = section(Position { x: 0.0, y: 0.0 }, None);
        map.nodes.insert("0".into(), n);
        assert!(
            check(&map).is_empty(),
            "fill-parent at (0,0) is the canonical shape"
        );
    }

    #[test]
    fn unset_size_at_nonzero_offset_overflows() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        // node.size is the default (100, 40 — see `node()` helper).
        // Offset (5, 0) + effective size (100, 40) = right 105 > 100.
        n.sections[0] = section(Position { x: 5.0, y: 0.0 }, None);
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(
            v.iter()
                .any(|x| x.message.contains("extends past node right edge")),
            "fill-parent at non-zero offset must flag right-edge overflow, got {:?}",
            v
        );
    }

    #[test]
    fn nan_node_size_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.size.width = f64::NAN;
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(
            v.iter()
                .any(|x| x.category == CATEGORY && x.message.contains("node.size has non-finite")),
            "expected non-finite-node-size violation, got {:?}",
            v
        );
    }

    #[test]
    fn zero_node_size_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.size.width = 0.0;
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v
            .iter()
            .any(|x| x.category == CATEGORY && x.message.contains("node.size.width is not positive")));
    }

    #[test]
    fn channel_collision_between_sections_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        let mut s0 = MindSection::new_default(String::new(), Vec::new());
        s0.channel = Some(2);
        let mut s1 = MindSection::new_default(String::new(), Vec::new());
        s1.channel = Some(2);
        n.sections = vec![s0, s1];
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(
            v.iter()
                .any(|x| x.category == CATEGORY && x.message.contains("channel 2 shared by sections")),
            "expected channel-collision violation, got {:?}",
            v
        );
    }

    #[test]
    fn channel_default_collision_with_explicit_zero_flagged() {
        // Section 0 defaults to channel 0; section 1 with explicit
        // `Some(0)` is honoured as channel 0 → collision. Pre-Tier-E
        // this case was silently overridden by the bare `usize`
        // shape; Option<usize> exposes it for verify to flag.
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        let s0 = MindSection::new_default(String::new(), Vec::new()); // channel = None → effective 0
        let mut s1 = MindSection::new_default(String::new(), Vec::new());
        s1.channel = Some(0);
        n.sections = vec![s0, s1];
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v
            .iter()
            .any(|x| x.category == CATEGORY && x.message.contains("channel 0 shared by sections")));
    }

    #[test]
    fn astronomical_section_width_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0] = section(
            Position { x: 0.0, y: 0.0 },
            Some(Size {
                width: 1e30,
                height: 30.0,
            }),
        );
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v
            .iter()
            .any(|x| x.category == CATEGORY && x.message.contains("over 100× the node's width")));
    }

    #[test]
    fn flush_to_node_aabb_passes_astronomical_guard() {
        // The 100× guard must not trip on legitimate "fill or
        // slightly-larger-than" section sizes. A section flush to
        // the node's AABB is at the AABB-overflow boundary — fine.
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0] = section(
            Position { x: 0.0, y: 0.0 },
            Some(Size {
                width: 100.0,
                height: 40.0,
            }),
        );
        map.nodes.insert("0".into(), n);
        assert!(check(&map).is_empty());
    }
}
