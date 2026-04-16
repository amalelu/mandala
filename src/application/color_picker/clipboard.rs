//! Clipboard trait implementations for `ColorPickerState`. The picker
//! is the first GUI widget to support clipboard operations — copy
//! produces the current hex color, paste accepts a hex string and
//! sets the HSV state, cut behaves identically to copy (the picker
//! always shows a color; there is nothing to "remove").

use baumhard::util::color::{hex_to_hsv_safe, hsv_to_hex};

use super::state::ColorPickerState;
use crate::application::console::traits::{
    ClipboardContent, HandlesCopy, HandlesCut, HandlesPaste, Outcome,
};

impl HandlesCopy for ColorPickerState {
    fn clipboard_copy(&self) -> ClipboardContent {
        match self {
            ColorPickerState::Open {
                hue_deg, sat, val, ..
            } => ClipboardContent::Text(hsv_to_hex(*hue_deg, *sat, *val)),
            ColorPickerState::Closed => ClipboardContent::NotApplicable,
        }
    }
}

impl HandlesPaste for ColorPickerState {
    fn clipboard_paste(&mut self, content: &str) -> Outcome {
        let ColorPickerState::Open {
            hue_deg,
            sat,
            val,
            hover_preview,
            ..
        } = self
        else {
            return Outcome::NotApplicable;
        };

        let trimmed = content.trim();
        let Some((h, s, v)) = hex_to_hsv_safe(trimmed) else {
            return Outcome::Invalid(format!("not a hex color: {}", trimmed));
        };

        let changed = hue_deg.to_bits() != h.to_bits()
            || sat.to_bits() != s.to_bits()
            || val.to_bits() != v.to_bits();

        *hue_deg = h;
        *sat = s;
        *val = v;
        *hover_preview = None;

        Outcome::applied(changed)
    }
}

impl HandlesCut for ColorPickerState {
    fn clipboard_cut(&mut self) -> ClipboardContent {
        // The picker always displays a color — "cutting" it has no
        // clearable source state. Behave identically to copy.
        self.clipboard_copy()
    }
}
