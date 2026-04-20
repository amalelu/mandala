//! Shader registry: Rust wrappers that name and embed the `.wgsl`
//! source modules the renderer loads at startup. The WGSL files
//! themselves live alongside these wrappers as `include_str!`
//! targets.

/// Shader name constants and the `SHADERS` table consumed by the
/// renderer's shader-loading pass.
pub mod shaders;