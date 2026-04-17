pub mod animation;
pub mod model;
pub mod loader;
pub mod border;
pub mod connection;
pub mod custom_mutation;
pub mod portal_geometry;
pub mod scene_builder;
pub mod scene_cache;
pub mod tree_builder;

/// Cyan selection highlight applied at scene / tree emission time
/// (selected edges, edge handles, portal markers, portal mutator
/// output). The app crate's `document::types::HIGHLIGHT_COLOR` is
/// the approximately-matching float-RGBA form used by the selection
/// machinery upstream.
pub(crate) const SELECTION_HIGHLIGHT_HEX: &str = "#00E5FF";
