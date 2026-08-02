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

use baumhard::mindmap::model::{validate, MindMap};

use super::Violation;

const CATEGORY: &str = "sections";

pub fn check(map: &MindMap) -> Vec<Violation> {
    let mut out = Vec::new();

    for (_loc, node) in map.node_locations() {
        out.extend(
            validate::node_size_violations(node.size)
                .into_iter()
                .map(|message| Violation::node(CATEGORY, node, message)),
        );
        if let Err(message) = validate::section_count(node) {
            out.push(Violation::node(CATEGORY, node, message));
        }
        out.extend(
            validate::section_channel_collisions(node)
                .into_iter()
                .map(|message| Violation::node_warn(CATEGORY, node, message)),
        );
        for (s_idx, section) in node.sections.iter().enumerate() {
            out.extend(
                validate::section_aabb_violations(node.size, s_idx, section.offset, section.size)
                    .into_iter()
                    .map(|message| Violation::node(CATEGORY, node, message)),
            );
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::test_helpers::node;
    use baumhard::mindmap::model::{MindSection, Position, Size, MAX_SECTIONS_PER_NODE};

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

    #[test]
    fn channel_collision_is_warning_not_error() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        let mut s0 = MindSection::new_default(String::new(), Vec::new());
        s0.channel = Some(2);
        let mut s1 = MindSection::new_default(String::new(), Vec::new());
        s1.channel = Some(2);
        n.sections = vec![s0, s1];
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        let collision = v
            .iter()
            .find(|x| x.message.contains("channel 2 shared by sections"))
            .expect("expected collision");
        assert_eq!(collision.severity, super::super::Severity::Warning);
    }

    #[test]
    fn astronomical_node_width_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.size.width = 1e30;
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v
            .iter()
            .any(|x| x.message.contains("exceeds the 1000000 ceiling")));
    }

    #[test]
    fn section_count_cap_uses_shared_constant() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        while n.sections.len() <= MAX_SECTIONS_PER_NODE {
            n.sections
                .push(MindSection::new_default(String::new(), Vec::new()));
        }
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v.iter().any(|x| x.message.contains("exceeds cap 1024")));
    }
}
