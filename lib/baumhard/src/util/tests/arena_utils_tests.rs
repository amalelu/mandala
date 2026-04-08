use indextree::Arena;
use crate::util::arena_utils::clone_subtree;

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
   assert_eq!(a_root.descendants(&a).count()-1, 9*6);
}
