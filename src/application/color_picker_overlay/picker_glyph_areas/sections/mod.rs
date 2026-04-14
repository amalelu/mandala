//! Per-section layout-phase builders. One file per picker section;
//! each `build` fn pushes its cells into the shared `PickerAreas`
//! table the [`super::compute::compute_picker_areas`] dispatcher owns.

pub(super) mod hex;
pub(super) mod hint;
pub(super) mod hue_ring;
pub(super) mod preview;
pub(super) mod sat_bar;
pub(super) mod title;
pub(super) mod val_bar;
