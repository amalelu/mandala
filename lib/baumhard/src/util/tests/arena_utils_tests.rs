// SPDX-License-Identifier: MPL-2.0

use crate::util::arena_utils::clone_subtree;
use indextree::Arena;

#[test]
fn test_clone() {
    do_clone();
}

pub fn do_clone() {
    let mut a: Arena<usize> = Arena::default();
    let a_root = a.new_node(0);
    let a_a0 = a_root.append_value(1, &mut a);
    let a_a1 = a_a0.append_value(2, &mut a);
    let _a_a2 = a_a1.append_value(3, &mut a);

    let a_b0 = a_root.append_value(4, &mut a);
    let a_b1 = a_b0.append_value(5, &mut a);
    let _a_b2 = a_b1.append_value(6, &mut a);

    let mut b: Arena<usize> = Arena::default();
    let b_root = b.new_node(0);

    let mut c: Arena<usize> = Arena::default();
    let c_root = c.new_node(0);

    // Clone a(1) into b(0)
    clone_subtree(&a, a_root, &mut b, b_root);
    // and now a(1) and b(1) should be identical
    assert_eq!(a, b);
    // clone b(1) back into a(1)
    clone_subtree(&b, b_root, &mut a, a_root);
    // now a(2) is not identical to b(1)
    assert_ne!(a, b);
    // now clone b(1) into c(0)
    clone_subtree(&b, b_root, &mut c, c_root);
    // now c(1) should be identical to b(1)
    assert_eq!(b, c);
    // now clone a(2) into b(1)
    clone_subtree(&a, a_root, &mut b, b_root);
    // c(1) and b(3) are no longer identical
    assert_ne!(b, c);
    // a(2) and b(3) are also not identical
    assert_ne!(a, b);
    // same with c(1) and a(2)
    assert_ne!(a, c);
    // clone c(1) into a(2)
    clone_subtree(&c, c_root, &mut a, a_root);
    // now a(3) and b(3) should be identical
    assert_eq!(a, b);
    assert_ne!(a, c);
    assert_ne!(b, c);
    // clone a(3) into b(3)
    clone_subtree(&a, a_root, &mut b, b_root);
    // b(6), a(3), c(1)
    assert_ne!(b, c);
    assert_ne!(a, b);
    assert_ne!(a, c);
    // clone b(6) back into a(3)
    clone_subtree(&b, b_root, &mut a, a_root);
    // a(9)
    assert_ne!(b, c);
    assert_ne!(a, b);
    assert_ne!(a, c);
    assert_eq!(a_root.descendants(&a).count() - 1, 9 * 6);
}

/// The copy visits depth-first, children left-to-right — the order
/// the recursive form produced. This is not cosmetic: it fixes the
/// order slots are allocated in the destination, and therefore the
/// `NodeId`s the caller gets back. Reading the destination in slot
/// order must yield the source's pre-order.
///
/// Plain `#[test]`, and none of §B8's four opt-out classes covers
/// it: nothing stops this body taking the `do_*()` split, it simply
/// never did. §B8 counts it among the 31 unconverted rather than let
/// "a correctness assertion on a small fixture is not a benchmark
/// candidate" stand as a fifth class — that reason fits most of the
/// tree, which is what makes it the wrong kind of reason.
#[test]
fn test_clone_preserves_depth_first_pre_order() {
    // root
    //  +- 1 -- 11 -- 111
    //  |   +-- 12
    //  +- 2
    //  +- 3 -- 31
    let mut src: Arena<usize> = Arena::default();
    let root = src.new_node(0);
    let n1 = root.append_value(1, &mut src);
    let n11 = n1.append_value(11, &mut src);
    let _n111 = n11.append_value(111, &mut src);
    let _n12 = n1.append_value(12, &mut src);
    let _n2 = root.append_value(2, &mut src);
    let n3 = root.append_value(3, &mut src);
    let _n31 = n3.append_value(31, &mut src);

    let mut dst: Arena<usize> = Arena::default();
    let dst_root = dst.new_node(0);
    clone_subtree(&src, root, &mut dst, dst_root);

    let expected: Vec<usize> = root.descendants(&src).skip(1).map(|id| *src[id].get()).collect();
    assert_eq!(
        expected,
        vec![1, 11, 111, 12, 2, 3, 31],
        "the fixture itself must be in pre-order"
    );

    // Slot order, not traversal order — this is what the recursion
    // fixed and what a breadth-first rewrite would silently change.
    let by_slot: Vec<usize> = dst.iter().skip(1).map(|n| *n.get()).collect();
    assert_eq!(
        by_slot, expected,
        "destination slots must be allocated in the source's pre-order"
    );

    let by_walk: Vec<usize> = dst_root
        .descendants(&dst)
        .skip(1)
        .map(|id| *dst[id].get())
        .collect();
    assert_eq!(by_walk, expected, "and the cloned structure must match");
}

/// A scene tree's depth is set by the `.mindmap.json` it came from,
/// and a long `parent_id` chain is legal, so this walk inherits
/// whatever depth the caller hands it. The recursive form exhausted
/// the stack and killed the process with `SIGABRT` — not a panic, so
/// nothing could catch or log it.
///
/// Runs on a 256 KiB stack for the same reason the loader's
/// deep-chain test does: any reintroduced recursion blows a stack
/// that small long before the chain ends, while the iterative form
/// is indifferent to it.
///
/// Plain `#[test]`: §B8 opt-out class 3, the enumeration is the
/// runtime. What an iteration would measure is 50,000 `append_value`
/// calls and a thread spawn, and the assertion is about the stack
/// the spawn asks for rather than about a value.
#[test]
fn test_clone_deep_chain_does_not_exhaust_the_stack() {
    const DEPTH: usize = 50_000;
    const SMALL_STACK: usize = 256 * 1024;

    let cloned = std::thread::Builder::new()
        .stack_size(SMALL_STACK)
        .spawn(|| {
            let mut src: Arena<usize> = Arena::default();
            let root = src.new_node(0);
            let mut cur = root;
            for i in 1..=DEPTH {
                cur = cur.append_value(i, &mut src);
            }

            let mut dst: Arena<usize> = Arena::default();
            let dst_root = dst.new_node(0);
            clone_subtree(&src, root, &mut dst, dst_root);
            dst_root.descendants(&dst).count() - 1
        })
        .expect("spawn the small-stack clone")
        .join()
        .expect("the clone must not exhaust a 256 KiB stack");

    assert_eq!(cloned, DEPTH, "every node in the chain must be copied");
}
