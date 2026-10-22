// SPDX-License-Identifier: MPL-2.0

//! Tree-structure invariants: parent_id references and cycle detection.

use baumhard::mindmap::model::MindMap;
use std::collections::HashSet;

use super::Violation;

pub fn check(map: &MindMap) -> Vec<Violation> {
    let mut out = Vec::new();

    // `nodes.values()`, not `node_locations()`: the location stamp
    // is `node.id` cloned into a fresh `String`, and this loop only
    // ever reads `node.id` back off the node.
    for node in map.nodes.values() {
        if let Some(ref pid) = node.parent_id {
            if !map.nodes.contains_key(pid) {
                out.push(Violation::node(
                    "tree",
                    node,
                    format!("parent_id {:?} references a node that does not exist", pid),
                ));
                continue;
            }
        }
    }

    // Cycle detection. **One violation per cycle, not per node whose
    // chain reaches one.** Walking every node's parent chain and
    // reporting each revisit meant an N-node cycle produced N
    // identical-looking violations, plus one more for every node
    // hanging off it — a three-node loop under a wide subtree could
    // fill the output with dozens of lines describing one mistake.
    //
    // A node has at most one parent, so a cycle is a single directed
    // loop and its member set is fixed. Keying the report on the
    // smallest member id gives each loop exactly one canonical name,
    // regardless of which node's walk found it first — which also
    // makes the output independent of `HashMap` iteration order.
    let mut reported: HashSet<&str> = HashSet::new();
    for node in map.nodes.values() {
        let mut seen: HashSet<&str> = HashSet::new();
        seen.insert(node.id.as_str());
        let mut current = node.parent_id.as_deref();
        while let Some(pid) = current {
            if !seen.insert(pid) {
                if let Some(cycle) = cycle_from(map, pid) {
                    let anchor = cycle.iter().copied().min().expect("a cycle has a member");
                    if reported.insert(anchor) {
                        let owner = map.nodes.get(anchor).expect("the anchor came from map.nodes");
                        out.push(Violation::node(
                            "tree",
                            owner,
                            format!("cycle detected in parent_id chain: {}", render(&cycle, anchor)),
                        ));
                    }
                }
                break;
            }
            current = map.nodes.get(pid).and_then(|n| n.parent_id.as_deref());
        }
    }

    out
}

/// The member ids of the parent-chain cycle that `start` lies on,
/// or `None` if `start` is not itself on a cycle.
///
/// `start` is the id a walk revisited, which is on the loop by
/// construction — the walk reached it twice, and the second arrival
/// came through its own parent chain. Following that chain from
/// `start` until it returns to `start` enumerates the loop and
/// nothing hanging off it.
///
/// The step budget is `map.nodes.len()`: a chain that has not closed
/// after visiting every node in the map is not a loop this function
/// can name, and returning `None` there is better than spinning.
fn cycle_from<'a>(map: &'a MindMap, start: &'a str) -> Option<Vec<&'a str>> {
    let mut members = vec![start];
    let mut current = map.nodes.get(start)?.parent_id.as_deref()?;
    for _ in 0..map.nodes.len() {
        if current == start {
            return Some(members);
        }
        members.push(current);
        current = map.nodes.get(current)?.parent_id.as_deref()?;
    }
    None
}

/// `"a" -> "b" -> "a"`, rotated so the anchor reads first — the
/// same loop found from a different node renders identically.
fn render(cycle: &[&str], anchor: &str) -> String {
    let at = cycle.iter().position(|id| *id == anchor).unwrap_or(0);
    let mut parts: Vec<String> = cycle[at..]
        .iter()
        .chain(cycle[..at].iter())
        .map(|id| format!("{:?}", id))
        .collect();
    parts.push(format!("{:?}", anchor));
    parts.join(" -> ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::test_helpers::node;

    #[test]
    fn empty_map_has_no_violations() {
        assert!(check(&MindMap::new_blank("t")).is_empty());
    }

    #[test]
    fn missing_parent_is_flagged() {
        let mut map = MindMap::new_blank("t");
        map.nodes.insert("0".into(), node("0", Some("ghost")));
        let v = check(&map);
        assert!(v
            .iter()
            .any(|x| x.category == "tree" && x.message.contains("ghost")));
    }

    #[test]
    fn cycle_is_flagged() {
        let mut map = MindMap::new_blank("t");
        // a → b → a
        map.nodes.insert("a".into(), node("a", Some("b")));
        map.nodes.insert("b".into(), node("b", Some("a")));
        let v = check(&map);
        assert!(v
            .iter()
            .any(|x| x.category == "tree" && x.message.contains("cycle")));
    }

    /// **One violation per cycle**, whatever the loop's size and
    /// however many nodes hang off it.
    ///
    /// Fails when: the per-node walk reports every revisit again —
    /// this map produces five lines then, one per node, all
    /// describing the same three-node loop. The two dangling nodes
    /// are what makes the count meaningful: they are not on the
    /// loop, they only reach it, and reporting them says nothing a
    /// reader can act on that the loop's own line does not.
    ///
    /// Control: the anchor and the rendered chain are asserted, so
    /// "exactly one" cannot be satisfied by a check that stopped
    /// reporting cycles and emitted something else.
    #[test]
    fn one_violation_per_cycle_however_many_nodes_reach_it() {
        let mut map = MindMap::new_blank("t");
        // Loop: c → b → a → c.
        map.nodes.insert("a".into(), node("a", Some("c")));
        map.nodes.insert("b".into(), node("b", Some("a")));
        map.nodes.insert("c".into(), node("c", Some("b")));
        // Two nodes hanging off the loop, not part of it.
        map.nodes.insert("d".into(), node("d", Some("a")));
        map.nodes.insert("e".into(), node("e", Some("d")));

        let all = check(&map);
        let cycles: Vec<&Violation> = all.iter().filter(|v| v.message.contains("cycle")).collect();
        assert_eq!(
            cycles.len(),
            1,
            "one loop is one mistake, got: {:?}",
            cycles.iter().map(|v| v.to_string()).collect::<Vec<_>>()
        );
        assert_eq!(cycles[0].location, "a", "the smallest member anchors the report");
        assert_eq!(
            cycles[0].message, "cycle detected in parent_id chain: \"a\" -> \"c\" -> \"b\" -> \"a\"",
            "the message must name the whole loop, starting at the anchor"
        );
    }

    /// Two independent loops in one map are two mistakes.
    ///
    /// Fails when: the dedup key stops distinguishing loops — a
    /// single "already reported a cycle" flag would collapse these
    /// into one line and hide a whole second defect.
    #[test]
    fn two_independent_cycles_are_reported_separately() {
        let mut map = MindMap::new_blank("t");
        map.nodes.insert("a".into(), node("a", Some("b")));
        map.nodes.insert("b".into(), node("b", Some("a")));
        map.nodes.insert("y".into(), node("y", Some("z")));
        map.nodes.insert("z".into(), node("z", Some("y")));

        let v = check(&map);
        let cycles: Vec<&Violation> = v.iter().filter(|x| x.message.contains("cycle")).collect();
        assert_eq!(cycles.len(), 2, "got: {cycles:?}");
        let mut anchors: Vec<&str> = cycles.iter().map(|c| c.location.as_str()).collect();
        anchors.sort_unstable();
        assert_eq!(anchors, vec!["a", "y"]);
    }

    /// A self-parent (`a → a`) is a one-node loop and reads as one.
    ///
    /// Fails when: `cycle_from` assumes a loop has at least two
    /// members — it would then walk past `start` and burn its whole
    /// budget before giving up, reporting nothing.
    #[test]
    fn a_self_parent_is_one_cycle() {
        let mut map = MindMap::new_blank("t");
        map.nodes.insert("a".into(), node("a", Some("a")));
        let v = check(&map);
        let cycles: Vec<&Violation> = v.iter().filter(|x| x.message.contains("cycle")).collect();
        assert_eq!(cycles.len(), 1, "got: {cycles:?}");
        assert_eq!(
            cycles[0].message,
            "cycle detected in parent_id chain: \"a\" -> \"a\""
        );
    }

    #[test]
    fn valid_tree_clean() {
        let mut map = MindMap::new_blank("t");
        map.nodes.insert("0".into(), node("0", None));
        map.nodes.insert("0.0".into(), node("0.0", Some("0")));
        assert!(check(&map).is_empty());
    }
}
