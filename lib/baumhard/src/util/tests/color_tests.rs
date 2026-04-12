use lazy_static::lazy_static;
use crate::util::color::{from_hex, hex_to_rgba_safe};
use crate::{hex, rgb, rgba};

#[test]
pub fn test_from_hex() {
   do_from_hex();
}

pub fn do_from_hex() {
   let rgba = from_hex(&["f7b267", "f79d65", "f4845f", "f27059", "f25c54"]);
   let control_1 = hex!("f7b267");
   let control_2 = hex!("f79d65");
   let control_3 = hex!("f4845f");
   let control_4 = hex!("f27059");
   let control_5 = hex!("f25c54");
   assert_eq!(rgba.len(), 5);
   assert_eq!(rgba.get(0).unwrap(), &control_1);
   assert_eq!(rgba.get(1).unwrap(), &control_2);
   assert_eq!(rgba.get(2).unwrap(), &control_3);
   assert_eq!(rgba.get(3).unwrap(), &control_4);
   assert_eq!(rgba.get(4).unwrap(), &control_5);
}

lazy_static! {
        pub static ref CONTROL_1: [f32; 4] = hex!("#05638f");
        pub static ref CONTROL_2: [f32; 4] = hex!("ddbffd");
        pub static ref CONTROL_3: [f32; 4] = hex!("#ba084f");
        pub static ref CONTROL_4: [f32; 4] = hex!("#fba2c6");
        pub static ref RGBA_COLORS: Vec<[f32; 4]> =
            from_hex(&["#05638f", "ddbffd", "#ba084f", "#fba2c6"]);
    }

#[test]
fn test_from_hex_lazy_static() {
   do_from_hex_lazy_static();
}

pub fn do_from_hex_lazy_static() {
   assert_eq!(RGBA_COLORS.len(), 4);
   assert_eq!(RGBA_COLORS.get(0).unwrap(), &CONTROL_1.clone());
   assert_eq!(RGBA_COLORS.get(1).unwrap(), &CONTROL_2.clone());
   assert_eq!(RGBA_COLORS.get(2).unwrap(), &CONTROL_3.clone());
   assert_eq!(RGBA_COLORS.get(3).unwrap(), &CONTROL_4.clone());
}

#[test]
fn test_from_hex_garbage_falls_back_to_black() {
   do_from_hex_garbage_falls_back_to_black();
}

/// Regression: bad hex strings must degrade to the fallback instead
/// of crashing. The valid entry in the middle ensures surrounding
/// items still parse correctly. The sentinel fallback `[0.42, …]`
/// distinguishes "returned the fallback" from "hardcoded black".
pub fn do_from_hex_garbage_falls_back_to_black() {
   // from_hex uses opaque-black as fallback internally.
   let rgba = from_hex(&["zzzzzz", "ff0000", "not-a-color", ""]);
   assert_eq!(rgba.len(), 4);
   assert_eq!(rgba[0], [0.0, 0.0, 0.0, 1.0]);
   assert_eq!(rgba[1], [1.0, 0.0, 0.0, 1.0]);
   assert_eq!(rgba[2], [0.0, 0.0, 0.0, 1.0]);
   assert_eq!(rgba[3], [0.0, 0.0, 0.0, 1.0]);
   // Use a sentinel fallback to prove hex_to_rgba_safe actually
   // returns the caller's fallback rather than a hardcoded value.
   let sentinel = [0.42, 0.42, 0.42, 0.42];
   assert_eq!(hex_to_rgba_safe("garbage", sentinel), sentinel);
   assert_eq!(hex_to_rgba_safe("", sentinel), sentinel);
}

#[test]
fn test_rgba_hex_macros() {
   do_rgba_hex_macros();
}

pub fn do_rgba_hex_macros() {
   let color1 = "#05638f";
   let color2 = "ddbffd";
   let rgba_rgba1: [f32; 4] = rgba!([5, 99, 143, 255]);
   let rgba_rgba2: [f32; 4] = rgba!([221, 191, 253, 255]);
   let rgb_rgba1: [f32; 4] = rgb!([5, 99, 143]);
   let rgb_rgba2: [f32; 4] = rgb!([221, 191, 253]);
   let hex_rgba1: [f32; 4] = hex!(color1);
   let hex_rgba2: [f32; 4] = hex!(color2);

   assert_eq!(rgba_rgba1, hex_rgba1);
   assert_eq!(rgba_rgba2, hex_rgba2);
   assert_eq!(rgb_rgba1, hex_rgba1);
   assert_eq!(rgb_rgba2, rgba_rgba2);
}