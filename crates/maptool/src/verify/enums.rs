// SPDX-License-Identifier: MPL-2.0

//! Named-enum invariants: check string fields against the known value sets.

use baumhard::gfx_structs::shape::KNOWN_SHAPES;
use baumhard::mindmap::model::{DISPLAY_MODE_LINE, DISPLAY_MODE_PORTAL, MindMap};

use super::Violation;

const LAYOUT_TYPES: &[&str] = &["map", "tree", "outline"];
const DIRECTIONS: &[&str] = &["auto", "up", "down", "left", "right", "balanced"];
const LINE_STYLES: &[&str] = &["solid", "dashed"];
const ANCHORS: &[&str] = &["auto", "top", "right", "bottom", "left"];
const EDGE_TYPES: &[&str] = &["parent_child", "cross_link"];
const DISPLAY_MODES: &[&str] = &[DISPLAY_MODE_LINE, DISPLAY_MODE_PORTAL];

pub fn check(map: &MindMap) -> Vec<Violation> {
    let mut out = Vec::new();

    for (loc, node) in map.node_locations() {
        // The runtime parses `style.shape` case-insensitively (and
        // treats "circle" as an alias for "ellipse"), so verify must
        // match that leniency. Every other named enum is compared
        // case-sensitively against its canonical constant.
        check_value(
            &mut out,
            &loc,
            "style.shape",
            &node.style.shape,
            KNOWN_SHAPES,
            true,
        );
        check_value(
            &mut out,
            &loc,
            "layout.type",
            &node.layout.layout_type,
            LAYOUT_TYPES,
            false,
        );
        check_value(
            &mut out,
            &loc,
            "layout.direction",
            &node.layout.direction,
            DIRECTIONS,
            false,
        );
    }

    for (loc, edge) in map.edge_locations() {
        check_value(&mut out, &loc, "type", &edge.edge_type, EDGE_TYPES, false);
        check_value(
            &mut out,
            &loc,
            "line_style",
            &edge.line_style,
            LINE_STYLES,
            false,
        );
        check_value(
            &mut out,
            &loc,
            "anchor_from",
            &edge.anchor_from,
            ANCHORS,
            false,
        );
        check_value(
            &mut out,
            &loc,
            "anchor_to",
            &edge.anchor_to,
            ANCHORS,
            false,
        );
        if let Some(mode) = edge.display_mode.as_deref() {
            check_value(&mut out, &loc, "display_mode", mode, DISPLAY_MODES, false);
        }
    }

    out
}

fn check_value(
    out: &mut Vec<Violation>,
    location: &str,
    field: &str,
    value: &str,
    allowed: &[&str],
    case_insensitive: bool,
) {
    let matched = if case_insensitive {
        allowed.iter().any(|&known| known.eq_ignore_ascii_case(value))
    } else {
        allowed.contains(&value)
    };
    if !matched {
        out.push(Violation::at(
            "enums",
            location.to_string(),
            format!("{} {:?} is not one of {:?}", field, value, allowed),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::test_helpers::{edge, node};

    #[test]
    fn defaults_are_valid() {
        let mut map = MindMap::new_blank("t");
        map.nodes.insert("0".into(), node("0", None));
        assert!(check(&map).is_empty());
    }

    #[test]
    fn circle_shape_is_valid() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.style.shape = "circle".into();
        map.nodes.insert("0".into(), n);
        assert!(check(&map).is_empty(), "circle is an ellipse alias");
    }

    #[test]
    fn shape_matching_is_case_insensitive() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.style.shape = "Rectangle".into();
        map.nodes.insert("0".into(), n);
        assert!(check(&map).is_empty(), "runtime accepts Rectangle, so verify must too");
    }

    #[test]
    fn bad_shape_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.style.shape = "oblong".into();
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v
            .iter()
            .any(|x| x.category == "enums" && x.message.contains("oblong")));
    }

    #[test]
    fn bad_anchor_flagged() {
        let mut map = MindMap::new_blank("t");
        map.nodes.insert("0".into(), node("0", None));
        map.nodes.insert("1".into(), node("1", None));
        let mut e = edge("0", "1");
        e.anchor_from = "diagonal".into();
        map.edges.push(e);
        let v = check(&map);
        assert!(v
            .iter()
            .any(|x| x.category == "enums" && x.message.contains("diagonal")));
    }

    #[test]
    fn unknown_display_mode_flagged() {
        let mut map = MindMap::new_blank("t");
        map.nodes.insert("0".into(), node("0", None));
        map.nodes.insert("1".into(), node("1", None));
        let mut e = edge("0", "1");
        e.display_mode = Some("projection".into());
        map.edges.push(e);
        let v = check(&map);
        assert!(v
            .iter()
            .any(|x| x.category == "enums" && x.message.contains("projection")));
    }

    #[test]
    fn portal_display_mode_is_valid() {
        let mut map = MindMap::new_blank("t");
        map.nodes.insert("0".into(), node("0", None));
        map.nodes.insert("1".into(), node("1", None));
        let mut e = edge("0", "1");
        e.display_mode = Some("portal".into());
        map.edges.push(e);
        assert!(check(&map).is_empty());
    }

    #[test]
    fn display_mode_is_case_sensitive() {
        // The runtime only recognises the exact constants
        // DISPLAY_MODE_LINE / DISPLAY_MODE_PORTAL, so a mixed-case
        // value would render as a line even though it looks like a
        // portal. Verify must flag it.
        let mut map = MindMap::new_blank("t");
        map.nodes.insert("0".into(), node("0", None));
        map.nodes.insert("1".into(), node("1", None));
        let mut e = edge("0", "1");
        e.display_mode = Some("Portal".into());
        map.edges.push(e);
        let v = check(&map);
        assert!(v
            .iter()
            .any(|x| x.category == "enums" && x.message.contains("Portal")));
    }
}
