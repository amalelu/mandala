pub mod animation;
pub mod model;
pub mod loader;
pub mod border;
pub mod connection;
pub mod custom_mutation;
pub mod scene_builder;
pub mod scene_cache;
pub mod tree_builder;

/// Cyan selection-highlight hex used across the baumhard crate
/// wherever selection color needs to be applied at emission time:
/// selected edge body (scene_builder/connection.rs), selected edge
/// handles (scene_builder/edge_handles.rs), selected portal markers
/// (scene_builder/portal.rs), and selected portal mutator output
/// (tree_builder/portal.rs). Previously duplicated as three private
/// constants across those modules; keeping one source of truth
/// avoids the sync-by-convention coupling the old duplicated
/// constants explicitly called out.
///
/// Matches the float-RGBA `HIGHLIGHT_COLOR` constant in the app
/// crate's `document/types.rs`, but the app crate consumes the
/// color in shader-ready `[f32; 4]` form while the baumhard scene
/// path needs it as a hex string for theme-var resolution.
pub const SELECTION_HIGHLIGHT_HEX: &str = "#00E5FF";
