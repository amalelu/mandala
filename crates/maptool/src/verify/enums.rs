//! Named-enum invariants: check string fields against the known value sets.

use baumhard::mindmap::model::MindMap;

use super::Violation;

const SHAPES: &[&str] = &[
    "rectangle", "rounded_rectangle", "ellipse", "diamond",
    "parallelogram", "hexagon",
];
const LAYOUT_TYPES: &[&str] = &["map", "tree", "outline"];
const DIRECTIONS: &[&str] = &["auto", "up", "down", "left", "right", "balanced"];
const LINE_STYLES: &[&str] = &["solid", "dashed"];
const ANCHORS: &[&str] = &["auto", "top", "right", "bottom", "left"];
const EDGE_TYPES: &[&str] = &["parent_child", "cross_link"];

pub fn check(map: &MindMap) -> Vec<Violation> {
    let mut out = Vec::new();

    for node in map.nodes.values() {
        check_value(&mut out, &node.id, "style.shape", &node.style.shape, SHAPES);
        check_value(&mut out, &node.id, "layout.type", &node.layout.layout_type, LAYOUT_TYPES);
        check_value(&mut out, &node.id, "layout.direction", &node.layout.direction, DIRECTIONS);
    }

    for (i, edge) in map.edges.iter().enumerate() {
        let loc = format!("edge[{}]", i);
        check_value(&mut out, &loc, "type", &edge.edge_type, EDGE_TYPES);
        check_value(&mut out, &loc, "line_style", &edge.line_style, LINE_STYLES);
        check_value(&mut out, &loc, "anchor_from", &edge.anchor_from, ANCHORS);
        check_value(&mut out, &loc, "anchor_to", &edge.anchor_to, ANCHORS);
    }

    out
}

fn check_value(
    out: &mut Vec<Violation>,
    location: &str,
    field: &str,
    value: &str,
    allowed: &[&str],
) {
    if !allowed.contains(&value) {
        out.push(Violation {
            category: "enums",
            location: location.to_string(),
            message: format!("{} {:?} is not one of {:?}", field, value, allowed),
        });
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
    fn bad_shape_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.style.shape = "oblong".into();
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v.iter().any(|x| x.category == "enums" && x.message.contains("oblong")));
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
        assert!(v.iter().any(|x| x.category == "enums" && x.message.contains("diagonal")));
    }
}
