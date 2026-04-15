//! Tree-builder scale / performance regression guards — N=1000 chain, N=500 star, deep chain stack-safety.

use super::fixtures::*;
use super::super::*;

#[test]
fn test_build_tree_scale_1000_node_chain() {
    let map = mk_chain_map(1000);
    let result = build_mindmap_tree(&map);
    assert_eq!(result.node_map.len(), 1000);
    // The spine root is the only root, so the tree's root has one
    // child (the Void -> first chain node).
    let roots: Vec<_> = result.tree.root.children(&result.tree.arena).collect();
    assert_eq!(roots.len(), 1);
    // Every chain node is reachable via the node_map.
    for i in 0..1000 {
        let id = format!("c{}", i);
        assert!(result.node_map.contains_key(&id),
            "missing node {}", id);
    }
}

/// A 500-child star fans out from a single root. Guards the
/// wide-breadth case — a regression that used Vec::insert(0, ...)
/// or otherwise grew quadratically in the child list would still
/// produce a correct node_map, but this test's companion 1000-node
/// chain test plus this one together cover both topology extremes.
#[test]
fn test_build_tree_wide_fan_out_500() {
    let map = mk_star_map(500);
    let result = build_mindmap_tree(&map);
    assert_eq!(result.node_map.len(), 500);
    // Root is "root", all others are direct children.
    let root_tree_id = result.node_map.get("root").unwrap();
    let children: Vec<_> = root_tree_id.children(&result.tree.arena).collect();
    assert_eq!(children.len(), 499);
}

/// A 500-node deep spine must build without a stack overflow. The
/// current `build_mindmap_tree` walks iteratively — this test
/// guards against a future refactor silently introducing recursion
/// over the hierarchy.
#[test]
fn test_build_tree_deep_chain_no_stack_overflow() {
    let map = mk_chain_map(500);
    let result = build_mindmap_tree(&map);
    assert_eq!(result.node_map.len(), 500);
    // Walk from the root down the spine and confirm depth == 500.
    let mut current = *result.node_map.get("c0").unwrap();
    let mut depth = 1;
    while let Some(child) = current.children(&result.tree.arena).next() {
        current = child;
        depth += 1;
    }
    assert_eq!(depth, 500);
}
