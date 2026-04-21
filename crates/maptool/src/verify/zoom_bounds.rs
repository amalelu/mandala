//! Zoom-visibility range invariant: whenever both
//! `min_zoom_to_render` and `max_zoom_to_render` are `Some` on a
//! `MindNode`, `MindEdge`, `EdgeLabelConfig`, or
//! `PortalEndpointState`, `min <= max` must hold. A swapped pair
//! is a well-defined but always-invisible window — render-time
//! `ZoomVisibility::contains` still terminates cleanly, but the
//! authoring intent is almost always a typo.

use baumhard::mindmap::model::MindMap;

use super::Violation;

pub fn check(map: &MindMap) -> Vec<Violation> {
    let mut out = Vec::new();

    for node in map.nodes.values() {
        check_pair(
            &mut out,
            &node.id,
            "",
            node.min_zoom_to_render,
            node.max_zoom_to_render,
        );
    }

    for (i, edge) in map.edges.iter().enumerate() {
        let loc = format!("edge[{}]", i);
        check_pair(
            &mut out,
            &loc,
            "",
            edge.min_zoom_to_render,
            edge.max_zoom_to_render,
        );
        if let Some(label_cfg) = edge.label_config.as_ref() {
            check_pair(
                &mut out,
                &loc,
                "label_config.",
                label_cfg.min_zoom_to_render,
                label_cfg.max_zoom_to_render,
            );
        }
        if let Some(from) = edge.portal_from.as_ref() {
            check_pair(
                &mut out,
                &loc,
                "portal_from.",
                from.min_zoom_to_render,
                from.max_zoom_to_render,
            );
        }
        if let Some(to) = edge.portal_to.as_ref() {
            check_pair(
                &mut out,
                &loc,
                "portal_to.",
                to.min_zoom_to_render,
                to.max_zoom_to_render,
            );
        }
    }

    out
}

fn check_pair(
    out: &mut Vec<Violation>,
    location: &str,
    field_prefix: &str,
    min: Option<f32>,
    max: Option<f32>,
) {
    // Non-finite (NaN / ±Inf) fails first. `ZoomVisibility::contains`
    // guards NaN at runtime, but an always-false element sitting in
    // the file on disk is a bug to surface, not a state to accept.
    if let Some(m) = min {
        if !m.is_finite() {
            out.push(Violation {
                category: "zoom_bounds",
                location: location.to_string(),
                message: format!(
                    "{}min_zoom_to_render {} is not finite",
                    field_prefix, m
                ),
            });
        }
    }
    if let Some(m) = max {
        if !m.is_finite() {
            out.push(Violation {
                category: "zoom_bounds",
                location: location.to_string(),
                message: format!(
                    "{}max_zoom_to_render {} is not finite",
                    field_prefix, m
                ),
            });
        }
    }
    if let (Some(min), Some(max)) = (min, max) {
        if min.is_finite() && max.is_finite() && min > max {
            out.push(Violation {
                category: "zoom_bounds",
                location: location.to_string(),
                message: format!(
                    "{}min_zoom_to_render {} > {}max_zoom_to_render {}",
                    field_prefix, min, field_prefix, max
                ),
            });
        }
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
    fn node_min_le_max_is_valid() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.min_zoom_to_render = Some(0.5);
        n.max_zoom_to_render = Some(2.0);
        map.nodes.insert("0".into(), n);
        assert!(check(&map).is_empty());
    }

    #[test]
    fn node_min_equal_max_is_valid() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.min_zoom_to_render = Some(1.0);
        n.max_zoom_to_render = Some(1.0);
        map.nodes.insert("0".into(), n);
        assert!(check(&map).is_empty());
    }

    #[test]
    fn node_min_greater_than_max_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.min_zoom_to_render = Some(2.0);
        n.max_zoom_to_render = Some(0.5);
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].category, "zoom_bounds");
        assert!(v[0].message.contains("min_zoom_to_render 2"));
        assert!(v[0].message.contains("max_zoom_to_render 0.5"));
    }

    #[test]
    fn one_sided_windows_are_valid() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.min_zoom_to_render = Some(1.0);
        n.max_zoom_to_render = None;
        map.nodes.insert("0".into(), n);
        assert!(check(&map).is_empty());

        let mut m = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.min_zoom_to_render = None;
        n.max_zoom_to_render = Some(1.0);
        m.nodes.insert("0".into(), n);
        assert!(check(&m).is_empty());
    }

    #[test]
    fn edge_inverted_pair_flagged() {
        let mut map = MindMap::new_blank("t");
        map.nodes.insert("0".into(), node("0", None));
        map.nodes.insert("1".into(), node("1", None));
        let mut e = edge("0", "1");
        e.min_zoom_to_render = Some(3.0);
        e.max_zoom_to_render = Some(1.0);
        map.edges.push(e);
        let v = check(&map);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].location, "edge[0]");
    }

    #[test]
    fn non_finite_min_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.min_zoom_to_render = Some(f32::NAN);
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert_eq!(v.len(), 1);
        assert!(v[0].message.contains("is not finite"));
    }

    #[test]
    fn non_finite_max_flagged() {
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.max_zoom_to_render = Some(f32::INFINITY);
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert_eq!(v.len(), 1);
        assert!(v[0].message.contains("is not finite"));
    }

    #[test]
    fn non_finite_pair_reports_both_violations() {
        // Both bounds non-finite — one violation per bound,
        // not a single compound message. Keeps the per-field
        // verifier posture consistent with the rest of the
        // `zoom_bounds` category.
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.min_zoom_to_render = Some(f32::NEG_INFINITY);
        n.max_zoom_to_render = Some(f32::NAN);
        map.nodes.insert("0".into(), n);
        let v = check(&map);
        assert_eq!(v.len(), 2);
        assert!(v.iter().all(|x| x.message.contains("is not finite")));
    }
}
