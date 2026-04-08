use crate::util::ordered_vec2::OrderedVec2;

pub fn equals() {
    let a = OrderedVec2::new_f32(95.0, 150.0);
    let b = OrderedVec2::new_f32(105.0, 150.0);
    assert_ne!(a, b);
    let result = a == b;
    assert!(!result);
}

#[test]
pub fn test_equals() {
    equals();
}