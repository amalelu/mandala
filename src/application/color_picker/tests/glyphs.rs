//! Hue-ring + arm glyph script grouping. The ring has Devanagari,
//! Hebrew, and Tibetan arcs; each crosshair arm pins one script.
//! Codepoint-range checks — not the identity of individual glyphs —
//! so swapping letters in the same script doesn't break the test.

use crate::application::color_picker::{
    arm_bottom_glyphs, arm_left_glyphs, arm_right_glyphs, arm_top_glyphs, hue_ring_glyphs,
};

fn first_cp(s: &str) -> u32 {
    s.chars().next().expect("glyph string non-empty") as u32
}

/// The 24-glyph hue ring array must have a Devanagari arc, a
/// Hebrew arc, and a Tibetan arc.
#[test]
fn hue_ring_glyphs_are_grouped_by_script() {
    // Slots 0-7 Devanagari
    for i in 0..8 {
        let cp = first_cp(hue_ring_glyphs()[i]);
        assert!(
            (0x0900..=0x097F).contains(&cp),
            "slot {i} codepoint U+{cp:04X} not in Devanagari",
        );
    }
    // Slots 8-15 Hebrew
    for i in 8..16 {
        let cp = first_cp(hue_ring_glyphs()[i]);
        assert!(
            (0x0590..=0x05FF).contains(&cp),
            "slot {i} codepoint U+{cp:04X} not in Hebrew",
        );
    }
    // Slots 16-23 Tibetan
    for i in 16..24 {
        let cp = first_cp(hue_ring_glyphs()[i]);
        assert!(
            (0x0F00..=0x0FFF).contains(&cp),
            "slot {i} codepoint U+{cp:04X} not in Tibetan",
        );
    }
}

/// Each crosshair arm must be grouped by its own script.
#[test]
fn arm_glyphs_are_grouped_by_script() {
    // Top arm: Devanagari (U+0900–U+097F)
    for (i, g) in arm_top_glyphs().iter().enumerate() {
        let cp = first_cp(g);
        assert!(
            (0x0900..=0x097F).contains(&cp),
            "top arm cell {i} codepoint U+{cp:04X} not in Devanagari",
        );
    }
    // Bottom arm: Egyptian Hieroglyphs (U+13000–U+1342F)
    for (i, g) in arm_bottom_glyphs().iter().enumerate() {
        let cp = first_cp(g);
        assert!(
            (0x13000..=0x1342F).contains(&cp),
            "bottom arm cell {i} codepoint U+{cp:05X} not in Egyptian Hieroglyphs",
        );
    }
    // Left arm: Tibetan (U+0F00–U+0FFF)
    for (i, g) in arm_left_glyphs().iter().enumerate() {
        let cp = first_cp(g);
        assert!(
            (0x0F00..=0x0FFF).contains(&cp),
            "left arm cell {i} codepoint U+{cp:04X} not in Tibetan",
        );
    }
    // Right arm: Hebrew (U+0590–U+05FF)
    for (i, g) in arm_right_glyphs().iter().enumerate() {
        let cp = first_cp(g);
        assert!(
            (0x0590..=0x05FF).contains(&cp),
            "right arm cell {i} codepoint U+{cp:04X} not in Hebrew",
        );
    }
}
