// SPDX-License-Identifier: MPL-2.0

//! Numeric-domain invariants — the same rules the loader enforces at
//! the trust boundary, reported here instead of refused.
//!
//! **The two must agree, and this is what makes them.** The loader
//! rejects a map whose numbers would reach an `assert!` inside the
//! text shaper, an inverted `f32::clamp`, or an allocation sized
//! from authored geometry (`baumhard::mindmap::model::validate`).
//! `verify` deliberately loads through the lenient door
//! (`loader::parse_for_inspection`) so it can still inspect those
//! files — but a looser door without the matching rules would mean
//! `verify` printing "valid" for a map the editor refuses to open,
//! which is the most misleading answer the tool could give.
//!
//! So the checks are not restated here. Every rule comes from the
//! same `validate::*_violations` helpers the loader calls, and the
//! only work this module does is stamping each message with the
//! document location `verify` reports in.

use baumhard::mindmap::model::{validate, MindMap};

use super::Violation;

/// Report every numeric-domain violation in `map`, in the order the
/// loader would have hit them: canvas, then map-level mutations,
/// then nodes (sorted by id, with their inline mutations), then
/// edges.
///
/// Cost: one pass over nodes, sections, text runs, edges, control
/// points and mutations — the same walk the loader's sweep makes.
pub fn check(map: &MindMap) -> Vec<Violation> {
    let mut out = Vec::new();

    for message in validate::canvas_numeric_violations(&map.canvas) {
        out.push(Violation::at("numeric", "canvas", message));
    }

    for (idx, mutation) in map.custom_mutations.iter().enumerate() {
        for message in validate::mutation_numeric_violations(mutation) {
            out.push(Violation::at(
                "numeric",
                format!("custom_mutations[{idx}]"),
                message,
            ));
        }
    }

    // Sorted so the report is stable across `HashMap` iteration
    // order — the same reason the loader's sweep sorts.
    let mut nodes: Vec<_> = map.nodes.values().collect();
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    for node in nodes {
        for message in validate::node_numeric_violations(node) {
            out.push(Violation::node("numeric", node, message));
        }
        for (idx, mutation) in node.inline_mutations.iter().enumerate() {
            for message in validate::mutation_numeric_violations(mutation) {
                out.push(Violation::node(
                    "numeric",
                    node,
                    format!("inline_mutations[{idx}]: {message}"),
                ));
            }
        }
    }

    for (idx, edge) in map.edges.iter().enumerate() {
        for message in validate::edge_numeric_violations(edge) {
            out.push(Violation::edge("numeric", idx, message));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::test_helpers::node;

    /// **The tool and the editor must agree about what is valid.**
    /// A map carrying a number the loader refuses must not be
    /// reported clean here, or `verify` sends an author looking for
    /// a problem it just told them they did not have.
    #[test]
    fn test_numeric_check_flags_what_the_loader_refuses() {
        let mut map = MindMap::new_blank("t");
        map.nodes.insert("0".into(), node("0", None));
        let border = baumhard::mindmap::model::GlyphBorderConfig {
            font_size_pt: 0.0,
            ..Default::default()
        };
        map.nodes.get_mut("0").unwrap().style.border = Some(border);

        let violations = check(&map);
        assert!(
            violations.iter().any(|v| v.message.contains("font_size_pt")),
            "a zero border font size must be reported: {violations:?}"
        );
        assert!(
            violations.iter().all(|v| v.category == "numeric"),
            "every violation here is a numeric-domain one"
        );
    }

    /// **The rules this module absorbed.** Text-run ordering and
    /// zoom-window inversion used to live in their own verify
    /// modules, restating rules `validate` already owned — so a
    /// single bad run was reported twice, in two near-identical
    /// wordings. They report through the shared helpers now; these
    /// pin that the coverage came with them.
    #[test]
    fn test_numeric_check_covers_the_rules_it_absorbed() {
        use baumhard::mindmap::model::TextRun;

        let run = |start: usize, end: usize| TextRun {
            start,
            end,
            bold: false,
            italic: false,
            underline: false,
            font: "LiberationSans".into(),
            size_pt: 14.0,
            color: "#ffffff".into(),
            hyperlink: None,
        };

        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.sections[0].text = "abcdef".into();
        n.sections[0].text_runs = vec![run(0, 4), run(2, 6)];
        n.min_zoom_to_render = Some(5.0);
        n.max_zoom_to_render = Some(1.0);
        map.nodes.insert("0".into(), n);

        let violations = check(&map);
        let messages: Vec<&str> = violations.iter().map(|v| v.message.as_str()).collect();
        assert!(
            messages.iter().any(|m| m.contains("overlaps previous run")),
            "overlapping runs must still be reported: {messages:?}"
        );
        assert!(
            messages
                .iter()
                .any(|m| m.contains("no zoom satisfies both bounds")),
            "an inverted zoom window must still be reported: {messages:?}"
        );
        assert_eq!(
            messages
                .iter()
                .filter(|m| m.contains("overlaps previous run"))
                .count(),
            1,
            "and reported exactly once — the duplicate module is what this replaced"
        );
    }

    /// The zoom window is carried by four different objects, and the
    /// absorbed module checked each one. Losing that breadth would be
    /// a silent coverage drop, so it is pinned here instead.
    #[test]
    fn test_numeric_check_reaches_every_zoom_window_carrier() {
        use crate::verify::test_helpers::edge as make_edge;
        use baumhard::mindmap::model::{EdgeLabelConfig, PortalEndpointState};

        let inverted = (Some(5.0_f32), Some(1.0_f32));

        // Node.
        let mut map = MindMap::new_blank("t");
        let mut n = node("0", None);
        n.min_zoom_to_render = inverted.0;
        n.max_zoom_to_render = inverted.1;
        map.nodes.insert("0".into(), n);
        assert!(
            check(&map)
                .iter()
                .any(|v| v.message.contains("no zoom satisfies")),
            "node zoom window"
        );

        // Edge, its label config, and both portal endpoints.
        for label in ["edge", "label_config", "portal_from", "portal_to"] {
            let mut map = MindMap::new_blank("t");
            map.nodes.insert("0".into(), node("0", None));
            map.nodes.insert("0.0".into(), node("0.0", Some("0")));
            let mut e = make_edge("0", "0.0");
            match label {
                "edge" => {
                    e.min_zoom_to_render = inverted.0;
                    e.max_zoom_to_render = inverted.1;
                }
                "label_config" => {
                    e.label_config = Some(EdgeLabelConfig {
                        min_zoom_to_render: inverted.0,
                        max_zoom_to_render: inverted.1,
                        ..Default::default()
                    });
                }
                "portal_from" => {
                    e.portal_from = Some(PortalEndpointState {
                        min_zoom_to_render: inverted.0,
                        max_zoom_to_render: inverted.1,
                        ..Default::default()
                    });
                }
                _ => {
                    e.portal_to = Some(PortalEndpointState {
                        min_zoom_to_render: inverted.0,
                        max_zoom_to_render: inverted.1,
                        ..Default::default()
                    });
                }
            }
            map.edges.push(e);
            let violations = check(&map);
            assert!(
                violations.iter().any(|v| v.message.contains("no zoom satisfies")),
                "{label} zoom window must be reported: {violations:?}"
            );
        }
    }

    /// A well-formed map produces nothing here.
    #[test]
    fn test_numeric_check_is_silent_on_a_valid_map() {
        let mut map = MindMap::new_blank("t");
        map.nodes.insert("0".into(), node("0", None));
        map.nodes.insert("0.0".into(), node("0.0", Some("0")));
        assert!(
            check(&map).is_empty(),
            "a well-formed map has no numeric violations"
        );
    }

    /// The canonical fixture is the real control: hundreds of nodes
    /// with authored borders, connections, palettes and zoom windows,
    /// and it loads through the strict door — so every number in it
    /// is in domain by construction and this must stay silent.
    ///
    /// The synthetic map above cannot make that argument. It has two
    /// bare nodes and exercises almost none of the fields `check`
    /// walks, so it would stay silent under a rule that rejected half
    /// of real authored content. Over-rejection is the failure mode
    /// a shared numeric domain invites, and this is what would catch
    /// it.
    #[test]
    fn test_numeric_check_is_silent_on_the_canonical_fixture() {
        let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.pop();
        p.pop();
        p.push("maps/testament.mindmap.json");
        let map = baumhard::mindmap::loader::load_from_file(&p)
            .expect("the canonical fixture must load through the strict door");
        let violations = check(&map);
        assert!(
            violations.is_empty(),
            "the canonical fixture must produce no numeric violations: {violations:?}"
        );
    }
}
