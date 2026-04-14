//! `PickerAreas` table + section enum: the channel-ordered list of
//! `(channel, GlyphArea)` pairs the layout-phase builders fill, plus
//! per-section index arrays so the mutator path can look up an area
//! by `(section, index)` without scanning or doing channel math.

use baumhard::gfx_structs::area::GlyphArea;

use crate::application::color_picker::{HUE_SLOT_COUNT, SAT_CELL_COUNT, VAL_CELL_COUNT};

/// All `GlyphArea`s the picker will emit on one apply cycle.
///
/// `ordered` is the channel-ascending `(channel, area)` list the
/// initial-build path walks to seat each cell at the right channel.
/// The per-section `[Option<usize>; N]` arrays index into `ordered` so
/// the mutator path can resolve
/// `mutator_builder::SectionContext::area(section, index)` calls
/// without scanning or doing channel math. The arrays are inline on
/// the struct (no per-frame heap allocation) — sized at compile time
/// from the per-section constants in `color_picker.rs`. `None` slots
/// mark intentionally-skipped indices (e.g. the centre crosshair cell
/// at `sat_bar[8]` / `val_bar[8]`); calling `area` on a skipped slot
/// is a programming error.
pub(super) struct PickerAreas {
    pub(super) ordered: Vec<(usize, GlyphArea)>,
    title: [Option<usize>; 1],
    hue_ring: [Option<usize>; HUE_SLOT_COUNT],
    hint: [Option<usize>; 1],
    sat_bar: [Option<usize>; SAT_CELL_COUNT],
    val_bar: [Option<usize>; VAL_CELL_COUNT],
    preview: [Option<usize>; 1],
    hex: [Option<usize>; 1],
}

/// Compile-time enum mirror of the picker's JSON section names. Lets
/// `PickerAreas::area` translate the spec's `&str` section key into a
/// branch on a known-shape array without a HashMap. The `from_name`
/// match panics on an unknown section since the JSON / Rust drift
/// would be a programming error, not a recoverable state.
#[derive(Copy, Clone, Debug)]
pub(super) enum PickerSection {
    Title,
    HueRing,
    Hint,
    SatBar,
    ValBar,
    Preview,
    Hex,
}

impl PickerSection {
    fn from_name(name: &str) -> Self {
        match name {
            "title" => PickerSection::Title,
            "hue_ring" => PickerSection::HueRing,
            "hint" => PickerSection::Hint,
            "sat_bar" => PickerSection::SatBar,
            "val_bar" => PickerSection::ValBar,
            "preview" => PickerSection::Preview,
            "hex" => PickerSection::Hex,
            other => panic!("picker area lookup: unknown section {other:?}"),
        }
    }
}

impl PickerAreas {
    /// Allocate an empty table with the channel-ordered vector
    /// pre-sized for the picker's fixed 60-cell payload.
    pub(super) fn new() -> Self {
        Self {
            ordered: Vec::with_capacity(60),
            title: [None; 1],
            hue_ring: [None; HUE_SLOT_COUNT],
            hint: [None; 1],
            sat_bar: [None; SAT_CELL_COUNT],
            val_bar: [None; VAL_CELL_COUNT],
            preview: [None; 1],
            hex: [None; 1],
        }
    }

    /// Push a built area into `ordered` and stamp its position into
    /// the matching per-section index array.
    pub(super) fn push(
        &mut self,
        section: PickerSection,
        index: usize,
        channel: usize,
        area: GlyphArea,
    ) {
        let vec_index = self.ordered.len();
        self.ordered.push((channel, area));
        let slot = match section {
            PickerSection::Title => self.title.get_mut(index),
            PickerSection::HueRing => self.hue_ring.get_mut(index),
            PickerSection::Hint => self.hint.get_mut(index),
            PickerSection::SatBar => self.sat_bar.get_mut(index),
            PickerSection::ValBar => self.val_bar.get_mut(index),
            PickerSection::Preview => self.preview.get_mut(index),
            PickerSection::Hex => self.hex.get_mut(index),
        };
        *slot.expect("picker area builder: index past compile-time section size") =
            Some(vec_index);
    }

    /// Resolve a `(section, index) → &GlyphArea` lookup. Panics if
    /// the section wasn't populated (the spec / builder disagree
    /// on what sections exist) or the requested index was deliberately
    /// skipped (e.g. the crosshair centre slot at sat_bar[8]) —
    /// the picker apply path treats both as a programming error
    /// rather than a recoverable state.
    pub(super) fn area(&self, section: &str, index: usize) -> &GlyphArea {
        let slot: Option<usize> = match PickerSection::from_name(section) {
            PickerSection::Title => self.title.get(index).copied().flatten(),
            PickerSection::HueRing => self.hue_ring.get(index).copied().flatten(),
            PickerSection::Hint => self.hint.get(index).copied().flatten(),
            PickerSection::SatBar => self.sat_bar.get(index).copied().flatten(),
            PickerSection::ValBar => self.val_bar.get(index).copied().flatten(),
            PickerSection::Preview => self.preview.get(index).copied().flatten(),
            PickerSection::Hex => self.hex.get(index).copied().flatten(),
        };
        let vec_index = slot.unwrap_or_else(|| {
            panic!(
                "picker area lookup: section {section:?} index {index} was not populated \
                 (skipped or out-of-range)"
            )
        });
        &self.ordered[vec_index].1
    }
}
