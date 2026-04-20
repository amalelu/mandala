//! Reference palettes — internal seeds and example constants.
//! Application code should define its own palettes rather than
//! importing these directly; they exist to keep the colour-math
//! tests and demo paths from open-coding magic numbers.

use crate::util::color::Color;

/// Opaque RGB white.
pub const TOTAL_WHITE: Color = Color {
    rgba: [255, 255, 255, 255],
};

/// Opaque RGB black.
pub const TOTAL_BLACK: Color = Color {
    rgba: [0, 0, 0, 255],
};

/// Earth-toned five-colour palette.
pub mod light_forest {
    use crate::util::color::FloatRgba;
    use crate::rgba;

    /// The full palette as an ordered array.
    pub const LIGHT_FOREST: [FloatRgba; 5] =
        [DARK_BROWN, LIGHT_BROWN, LIGHT_GREEN, GREEN, DARK_GREEN];

    pub const DARK_BROWN: FloatRgba = rgba!([72, 60, 31, 255]);
    pub const LIGHT_BROWN: FloatRgba = rgba!([160, 149, 116, 255]);
    pub const LIGHT_GREEN: FloatRgba = rgba!([231, 239, 205, 255]);
    pub const GREEN: FloatRgba = rgba!([198, 200, 108, 255]);
    pub const DARK_GREEN: FloatRgba = rgba!([126, 137, 37, 255]);
}

/// Cool-toned five-colour palette.
pub mod smooth_ocean {
    use crate::util::color::FloatRgba;
    use crate::rgba;

    /// The full palette as an ordered array.
    pub const SMOOTH_OCEAN: [FloatRgba; 5] = [DARK_TURQUOISE, TURQUOISE, GREY, ALMOST_WHITE, BEIGE];

    pub const DARK_TURQUOISE: FloatRgba = rgba!([70, 149, 151, 255]);
    pub const TURQUOISE: FloatRgba = rgba!([91, 161, 153, 255]);
    pub const GREY: FloatRgba = rgba!([187, 198, 200, 255]);
    pub const ALMOST_WHITE: FloatRgba = rgba!([229, 227, 228, 255]);
    pub const BEIGE: FloatRgba = rgba!([221, 190, 170, 255]);
}

/// Named palettes defined by hex literals, parsed once at program
/// start via `lazy_static`.
pub mod hex_palettes {
    use lazy_static::lazy_static;

    use crate::util::color::from_hex;

    lazy_static! {
        pub static ref SUNRISE: Vec<[f32; 4]> =
            from_hex(&["f7b267", "f79d65", "f4845f", "f27059", "f25c54"]);
        pub static ref ALIEN_GOO: Vec<[f32; 4]> =
            from_hex(&["82ff9e", "a9fbc3", "b594b6", "935fa7", "9b489b"]);
        pub static ref STATUS: Vec<[f32; 4]> =
            from_hex(&["1e1e24", "fb9f89", "c4af9a", "81ae9d", "21a179"]);
        pub static ref DIVERSITY_STATION: Vec<[f32; 4]> =
            from_hex(&["0a1045", "00c2d1", "f9e900", "f6af65", "ed33b9"]);
        pub static ref GAY_LIBRARY: Vec<[f32; 4]> =
            from_hex(&["003844", "006c67", "f194b4", "ffb100", "ffebc6"]);
        pub static ref PASTEL_ICECREAM: Vec<[f32; 4]> =
            from_hex(&["90f1ef", "ffd6e0", "ffef9f", "c1fba4", "7bf1a8"]);
        pub static ref LAVA_JUNGLE: Vec<[f32; 4]> =
            from_hex(&["b8b42d", "697a21", "fffce8", "3e363f", "dd403a"]);
        pub static ref TASTEFUL_INTERIOR: Vec<[f32; 4]> =
            from_hex(&["96bbbb", "618985", "414535", "f2e3bc", "c19875"]);
        pub static ref SUNSET_WORLD: Vec<[f32; 4]> =
            from_hex(&["2e86ab", "a23b72", "f18f01", "c73e1d", "3b1f2b"]);
        pub static ref SWEET_BLOSSOMS: Vec<[f32; 4]> =
            from_hex(&["d8e2dc", "ffffff", "ffcad4", "f4acb7", "9d8189"]);
        pub static ref TROPICAL_SEA: Vec<[f32; 4]> =
            from_hex(&["7cfef0", "6bffb8", "2ceaa3", "28965a", "2a6041"]);
        pub static ref FANCY_SOIL: Vec<[f32; 4]> =
            from_hex(&["7a7265", "c0b7b1", "8e6e53", "c69c72", "433e3f"]);
        pub static ref FIELDS_OF_GOLD: Vec<[f32; 4]> = from_hex(&[
            "569d4e", "5fad56", "a9b752", "cebc50", "f2c14e", "f5a151", "f78154", "a28966",
            "4d9078", "5d9a84"
        ]);
    }
}
