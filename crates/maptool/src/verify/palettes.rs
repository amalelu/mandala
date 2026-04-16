//! Palette invariants: every node's color_schema.palette must resolve to
//! a defined palette; every palette must have at least one group.

use baumhard::mindmap::model::MindMap;

use super::Violation;

pub fn check(map: &MindMap) -> Vec<Violation> {
    let mut out = Vec::new();

    for node in map.nodes.values() {
        if let Some(ref schema) = node.color_schema {
            if !map.palettes.contains_key(&schema.palette) {
                out.push(Violation {
                    category: "palettes",
                    location: node.id.clone(),
                    message: format!(
                        "palette {:?} is not defined in map.palettes",
                        schema.palette
                    ),
                });
            }
        }
    }

    for (name, palette) in &map.palettes {
        if palette.groups.is_empty() {
            out.push(Violation {
                category: "palettes",
                location: name.clone(),
                message: "palette has no color groups".to_string(),
            });
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::mindmap::model::{ColorGroup, ColorSchema, Palette};
    use crate::verify::test_helpers::node;

    fn group() -> ColorGroup {
        ColorGroup {
            background: "#000000".into(),
            frame: "#000000".into(),
            text: "#ffffff".into(),
            title: "#ffffff".into(),
        }
    }

    #[test]
    fn valid_palette_ref_clean() {
        let mut map = MindMap::new_blank("t");
        map.palettes.insert("coral".into(), Palette { groups: vec![group()] });
        let mut n = node("0", None);
        n.color_schema = Some(ColorSchema {
            palette: "coral".into(),
            level: 0,
            starts_at_root: true,
            connections_colored: true,
        });
        map.nodes.insert("0".into(), n);
        assert!(check(&map).is_empty());
    }

    #[test]
    fn missing_palette_is_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.color_schema = Some(ColorSchema {
            palette: "sunset".into(),
            level: 0,
            starts_at_root: true,
            connections_colored: true,
        });
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert!(v.iter().any(|x| x.category == "palettes" && x.message.contains("sunset")));
    }

    #[test]
    fn empty_palette_is_flagged() {
        let mut map = MindMap::new_blank("t");
        map.palettes.insert("empty".into(), Palette { groups: vec![] });
        let v = check(&map);
        assert!(v.iter().any(|x| x.category == "palettes" && x.location == "empty"));
    }
}
