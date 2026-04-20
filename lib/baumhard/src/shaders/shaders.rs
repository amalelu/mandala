//! Shader name constants and the `(name, source)` table consumed by
//! the renderer's shader-loading pass.
//!
//! The renderer registers every `(name, wgsl_source)` pair in
//! `SHADERS` at startup and later looks the module up by the name
//! the application chose — `SHADER_APPLICATION`. Two entries
//! pointing at the same `.wgsl` is a seam for swapping the
//! application shader out per-pass without touching the renderer:
//! change `SHADER_APPLICATION`'s binding, no rebuild needed.

/// Registered shader modules the renderer should load at startup.
/// Each tuple is `(unique name, WGSL source text)`. The name is the
/// key the renderer uses to look a module up again.
pub static SHADERS: [(&'static str, &'static str); 2] = [
    (SHADER_TEST, include_str!("test_shader.wgsl")),
    (SHADER_TEST_TWO, include_str!("test_shader.wgsl")),
];

pub(crate) static SHADER_TEST: &str = "TestShader";
pub(crate) static SHADER_TEST_TWO: &str = "TestShaderTwo";

/// Name of the shader module the main render pipeline binds against.
/// Indirected through a `pub static` so swapping the application
/// shader is a one-line change that doesn't require touching the
/// renderer's lookup site.
pub static SHADER_APPLICATION: &str = SHADER_TEST;
